//! Helper for the commit-acknowledgment manual check. It is driven by
//! `scripts/check_outbox_commit_cut.py`, which puts a proxy between this program
//! and PostgreSQL that lets a `COMMIT` reach the server and then drops the
//! connection before the acknowledgment reaches the client.
//!
//! `PROXY_DATABASE_URL` goes through that proxy; `DIRECT_DATABASE_URL` does not
//! and is used to observe what was really committed. Run it only against a
//! disposable database that already has the outbox table and a `business` table.
use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use mulac_outbox::prelude::*;
use std::sync::Arc;
use uuid::Uuid;

#[derive(QueryableByName)]
struct Count {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    count: i64,
}

fn count(conn: &mut PgConnection, table: &str) -> i64 {
    diesel::sql_query(format!("SELECT count(*) AS count FROM {table}"))
        .get_result::<Count>(conn)
        .expect("count")
        .count
}

fn events(n: usize) -> Vec<NewOutboxEnvelope> {
    (0..n)
        .map(|_| {
            let id = Uuid::now_v7();
            NewOutboxEnvelope {
                payload: format!(r#"{{"id":"{id}"}}"#),
                metadata: NewOutboxMetadata {
                    sequence: None,
                    event_id: id,
                    message_id: None,
                    correlation_id: None,
                    causation_id: None,
                    event_type: "Check.Event".into(),
                    routing_key: "check.event".into(),
                    source: None,
                    content_type: Some("application/json".into()),
                },
            }
        })
        .collect()
}

fn main() {
    let via = std::env::var("PROXY_DATABASE_URL").expect("PROXY_DATABASE_URL");
    let direct = std::env::var("DIRECT_DATABASE_URL").expect("DIRECT_DATABASE_URL");
    let mut observer = PgConnection::establish(&direct).expect("observer");
    let direct_recorder = OutboxRecorder::new(Arc::new(DieselPgOutboxRepository::new(
        build_pool(&direct).expect("direct pool"),
    )));

    // Pooled recording: its own COMMIT is the one whose acknowledgment is lost.
    let batch = events(3);
    let pool = build_pool(&via).expect("proxy pool");
    let pooled =
        OutboxRecorder::new(Arc::new(DieselPgOutboxRepository::new(pool))).record_batch(&batch);
    let (kind, outcome) = match &pooled {
        Ok(()) => ("Ok".to_string(), "none".to_string()),
        Err(error) => (
            format!("{:?}", error)
                .split('(')
                .next()
                .unwrap_or("")
                .to_string(),
            format!("{:?}", error.outcome()),
        ),
    };
    let committed = count(&mut observer, "outbox_entries");
    let retry = direct_recorder.record_batch(&batch);
    println!(
        "RESULT pooled error={kind} outcome={outcome} rows_committed={committed} retry_ok={} rows_after_retry={}",
        retry.is_ok(),
        count(&mut observer, "outbox_entries"),
    );

    // Caller-owned transaction: business write and outbox events, committed by
    // the application. A lost acknowledgment is the application's commit error.
    let batch = events(2);
    let mut conn = PgConnection::establish(&via).expect("proxy connection");
    let commit = conn.transaction::<(), diesel::result::Error, _>(|c| {
        c.batch_execute("INSERT INTO business VALUES (1)")?;
        record_batch_on_connection(c, &batch).expect("recording itself succeeds");
        Ok(())
    });
    let before_outbox = count(&mut observer, "outbox_entries");
    let retry = direct_recorder.record_batch(&batch);
    println!(
        "RESULT caller commit_failed={} business_rows={} outbox_rows_committed={} retry_ok={} rows_after_retry={}",
        commit.is_err(),
        count(&mut observer, "business"),
        before_outbox - 3,
        retry.is_ok(),
        count(&mut observer, "outbox_entries") - 3,
    );
}
