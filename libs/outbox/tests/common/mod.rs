//! Shared PostgreSQL harness for the outbox acceptance suites.
//!
//! Every test gets its own schema built from `docs/outbox_entries.sql`, and
//! every connection it hands out is pinned to that schema, so tests never reset
//! each other's tables. Tests are `#[ignore]`d and need `OUTBOX_TEST_DATABASE_URL`.
#![allow(dead_code)]

use diesel::connection::SimpleConnection;
use diesel::pg::PgConnection;
use diesel::prelude::*;
use diesel::r2d2::{ConnectionManager, Pool};
use diesel::sql_types::BigInt;
use mulac_outbox::prelude::*;
use uuid::Uuid;

pub const SCHEMA_SQL: &str = include_str!("../../../../docs/outbox_entries.sql");

pub fn base_url() -> String {
    std::env::var("OUTBOX_TEST_DATABASE_URL")
        .expect("OUTBOX_TEST_DATABASE_URL must be set to run the outbox database tests")
}

pub struct TestDb {
    base: String,
    pub schema: String,
}

impl TestDb {
    pub fn new() -> Self {
        let base = base_url();
        let schema = format!("t_{}", Uuid::new_v4().simple());
        let mut admin = PgConnection::establish(&base).expect("connect to test database");
        admin
            .batch_execute(&format!("CREATE SCHEMA {schema}"))
            .expect("create schema");
        let db = Self { base, schema };
        db.conn()
            .batch_execute(SCHEMA_SQL)
            .expect("apply docs/outbox_entries.sql");
        db
    }

    /// Connection URL pinned to this test's schema.
    pub fn url(&self) -> String {
        let sep = if self.base.contains('?') { '&' } else { '?' };
        format!("{}{sep}options=-csearch_path%3D{}", self.base, self.schema)
    }

    pub fn conn(&self) -> PgConnection {
        PgConnection::establish(&self.url()).expect("connect to test schema")
    }

    pub fn pool(&self) -> DbPool {
        Pool::builder()
            .max_size(4)
            .min_idle(Some(0))
            .build(ConnectionManager::<PgConnection>::new(self.url()))
            .expect("build pool")
    }

    pub fn repo(&self) -> DieselPgOutboxRepository {
        DieselPgOutboxRepository::new(self.pool())
    }

    pub fn exec(&self, sql: &str) {
        self.conn().batch_execute(sql).expect("execute SQL");
    }

    pub fn count(&self) -> i64 {
        count(&mut self.conn())
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        if let Ok(mut admin) = PgConnection::establish(&self.base) {
            let _ = admin.batch_execute(&format!("DROP SCHEMA IF EXISTS {} CASCADE", self.schema));
        }
    }
}

#[derive(QueryableByName)]
struct Count {
    #[diesel(sql_type = BigInt)]
    count: i64,
}

pub fn count(conn: &mut PgConnection) -> i64 {
    diesel::sql_query("SELECT count(*) AS count FROM outbox_entries")
        .get_result::<Count>(conn)
        .expect("count rows")
        .count
}

#[derive(QueryableByName, Debug, PartialEq)]
pub struct Stored {
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub payload: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub meta: String,
}

pub fn stored(conn: &mut PgConnection, id: Uuid) -> Option<Stored> {
    diesel::sql_query("SELECT payload, meta::text AS meta FROM outbox_entries WHERE id = $1")
        .bind::<diesel::sql_types::Uuid, _>(id)
        .get_result::<Stored>(conn)
        .optional()
        .expect("read stored row")
}

pub fn envelope(id: Uuid) -> NewOutboxEnvelope {
    NewOutboxEnvelope {
        payload: format!(r#"{{"id":"{id}","n":1}}"#),
        metadata: NewOutboxMetadata {
            sequence: None,
            event_id: id,
            message_id: None,
            correlation_id: None,
            causation_id: None,
            event_type: "Thing.Happened".into(),
            routing_key: "things.happened".into(),
            source: Some("test".into()),
            content_type: Some("application/json".into()),
        },
    }
}

pub fn entry(id: Uuid) -> NewOutboxEntry {
    NewOutboxEntry::from(&envelope(id))
}

pub fn ids(n: usize) -> Vec<Uuid> {
    (0..n).map(|_| Uuid::now_v7()).collect()
}

pub fn envelopes(ids: &[Uuid]) -> Vec<NewOutboxEnvelope> {
    ids.iter().copied().map(envelope).collect()
}
