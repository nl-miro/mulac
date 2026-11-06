//! Shared PostgreSQL harness for the outbox acceptance suites.
//!
//! Every test gets its own schema built from `docs/outbox_entries.sql`, and
//! every connection it hands out is pinned to that schema, so tests never reset
//! each other's tables. Tests are `#[ignore]`d and need `OUTBOX_TEST_DATABASE_URL`.
#![allow(dead_code)]

pub mod corpus;

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
    /// A fresh, empty schema: the caller applies its own baseline.
    pub fn bare() -> Self {
        let base = base_url();
        let schema = format!("t_{}", Uuid::new_v4().simple());
        let mut admin = PgConnection::establish(&base).expect("connect to test database");
        admin
            .batch_execute(&format!("CREATE SCHEMA {schema}"))
            .expect("create schema");
        Self { base, schema }
    }

    /// A fresh schema holding the table from `docs/outbox_entries.sql`.
    pub fn new() -> Self {
        let db = Self::bare();
        db.conn()
            .batch_execute(SCHEMA_SQL)
            .expect("apply docs/outbox_entries.sql");
        db
    }

    /// Connection URL pinned to this test's schema.
    pub fn url(&self) -> String {
        let sep = if self.base.contains('?') { '&' } else { '?' };
        // `application_name` lets observers tell this test's backends apart.
        format!(
            "{}{sep}options=-csearch_path%3D{}&application_name={}",
            self.base, self.schema, self.schema
        )
    }

    /// The URL up to and including the `options` value, so more `-c` settings can
    /// be appended to it (`%20-cname%3Dvalue`). `application_name` follows it, so
    /// this puts it first.
    pub fn url_with_options(&self) -> String {
        let sep = if self.base.contains('?') { '&' } else { '?' };
        format!(
            "{}{sep}application_name={}&options=-csearch_path%3D{}",
            self.base, self.schema, self.schema
        )
    }

    pub fn conn(&self) -> PgConnection {
        PgConnection::establish(&self.url()).expect("connect to test schema")
    }

    pub fn pool(&self) -> DbPool {
        Pool::builder()
            .max_size(8)
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

/// Backends of this test that are currently waiting on another backend's lock.
pub fn blocked_backends(db: &TestDb, observer: &mut PgConnection) -> i64 {
    diesel::sql_query(
        "SELECT count(*) AS count FROM pg_stat_activity \
         WHERE application_name = $1 AND cardinality(pg_blocking_pids(pid)) > 0",
    )
    .bind::<diesel::sql_types::Text, _>(db.schema.clone())
    .get_result::<Count>(observer)
    .expect("observe blocked backends")
    .count
}

/// Waits, with a watchdog, until at least `n` of this test's backends are
/// blocked on another backend. This proves real contention rather than timing.
pub fn wait_blocked(db: &TestDb, n: i64) {
    let mut observer = db.conn();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while blocked_backends(db, &mut observer) < n {
        assert!(
            std::time::Instant::now() < deadline,
            "expected {n} blocked backend(s) but contention never appeared"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

/// Joins a worker thread under a watchdog.
pub fn join<T: Send + 'static>(handle: std::thread::JoinHandle<T>) -> T {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while !handle.is_finished() {
        assert!(
            std::time::Instant::now() < deadline,
            "worker did not finish"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    handle.join().expect("worker panicked")
}

/// Every public way to record envelopes: the batch methods, the single-event
/// methods, and the caller-connection function. The single-event modes call
/// `OutboxRecorder::record` and `OutboxStorePort::record` themselves, so a rule
/// asserted through them is proven for those methods and not only for a batch
/// of one.
#[derive(Clone, Copy, Debug)]
pub enum Api {
    Recorder,
    Port,
    Caller,
    RecorderSingle,
    PortSingle,
}

/// Every API, for cases that record exactly one event per call.
pub const APIS: [Api; 5] = [
    Api::Recorder,
    Api::Port,
    Api::Caller,
    Api::RecorderSingle,
    Api::PortSingle,
];

/// The APIs that accept a batch of any size, for cases with several events.
pub const BATCH_APIS: [Api; 3] = [Api::Recorder, Api::Port, Api::Caller];

impl Api {
    pub fn is_single(self) -> bool {
        matches!(self, Api::RecorderSingle | Api::PortSingle)
    }

    /// Records `batch` the way this API would; the single-event modes take
    /// exactly one event and fail loudly otherwise. The caller path runs inside
    /// its own raw transaction, committed on success and rolled back on failure.
    pub fn record(self, db: &TestDb, batch: &[NewOutboxEnvelope]) -> Result<(), RecordingError> {
        assert!(
            !self.is_single() || batch.len() == 1,
            "{self:?} records exactly one event, got {}; use BATCH_APIS for batches",
            batch.len()
        );
        match self {
            Api::RecorderSingle => {
                OutboxRecorder::new(std::sync::Arc::new(db.repo())).record(&batch[0])
            }
            Api::PortSingle => db.repo().record(&NewOutboxEntry::from(&batch[0])),
            Api::Recorder => {
                OutboxRecorder::new(std::sync::Arc::new(db.repo())).record_batch(batch)
            }
            Api::Port => {
                let entries: Vec<_> = batch.iter().map(NewOutboxEntry::from).collect();
                db.repo().record_batch(&entries)
            }
            Api::Caller => {
                let mut conn = db.conn();
                conn.batch_execute("BEGIN").unwrap();
                let result = record_batch_on_connection(&mut conn, batch);
                let end = conn.batch_execute(if result.is_ok() { "COMMIT" } else { "ROLLBACK" });
                // A session the failure itself killed cannot be ended cleanly.
                if result.is_ok() {
                    end.unwrap();
                }
                result
            }
        }
    }
}

impl Api {
    /// Records every event of `batch`: at once through a batch API, or one call
    /// per event through a single-event API (stopping at the first error). For
    /// cases where the events are independent, such as replaying distinct rows.
    pub fn record_each(
        self,
        db: &TestDb,
        batch: &[NewOutboxEnvelope],
    ) -> Result<(), RecordingError> {
        if self.is_single() {
            batch
                .iter()
                .try_for_each(|e| self.record(db, std::slice::from_ref(e)))
        } else {
            self.record(db, batch)
        }
    }
}

pub fn status(conn: &mut PgConnection, id: Uuid) -> i32 {
    #[derive(QueryableByName)]
    struct S {
        #[diesel(sql_type = diesel::sql_types::Int4)]
        status: i32,
    }
    diesel::sql_query("SELECT status FROM outbox_entries WHERE id = $1")
        .bind::<diesel::sql_types::Uuid, _>(id)
        .get_result::<S>(conn)
        .expect("row exists")
        .status
}

/// The whole stored row as JSON text, to prove nothing about it changed.
pub fn snapshot(conn: &mut PgConnection, id: Uuid) -> String {
    #[derive(QueryableByName)]
    struct J {
        #[diesel(sql_type = diesel::sql_types::Text)]
        row: String,
    }
    diesel::sql_query("SELECT row_to_json(t)::text AS row FROM outbox_entries t WHERE id = $1")
        .bind::<diesel::sql_types::Uuid, _>(id)
        .get_result::<J>(conn)
        .expect("row exists")
        .row
}
