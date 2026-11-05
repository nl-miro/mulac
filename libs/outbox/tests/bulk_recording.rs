#![cfg(feature = "diesel")]
use diesel::{connection::SimpleConnection, prelude::*};
use mulac_outbox::io::*;
use uuid::Uuid;

fn connection() -> PgConnection {
    let mut conn =
        PgConnection::establish(&std::env::var("OUTBOX_TEST_DATABASE_URL").unwrap()).unwrap();
    conn.batch_execute(&include_str!("../../../docs/outbox_entries.sql").replace(
        "CREATE TABLE outbox_entries",
        "CREATE TEMP TABLE outbox_entries",
    ))
    .unwrap();
    conn
}

fn envelope(id: u128) -> NewOutboxEnvelope {
    NewOutboxEnvelope {
        payload: "{}".into(),
        metadata: NewOutboxMetadata {
            event_id: Uuid::from_u128(id),
            message_id: None,
            sequence: None,
            correlation_id: None,
            causation_id: None,
            source: None,
            routing_key: "test".into(),
            event_type: "Test".into(),
            content_type: None,
        },
    }
}

// Only the primary key is needed for this test's typed count query.
diesel::table! {
    outbox_entries (id) {
        id -> Uuid,
    }
}

fn count(conn: &mut PgConnection) -> i64 {
    outbox_entries::table
        .count()
        .get_result::<i64>(conn)
        .unwrap()
}

#[test]
#[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
fn chunks_replay_and_cross_chunk_rollback() {
    let mut conn = connection();
    let entries: Vec<_> = (1..=10001).rev().map(envelope).collect();
    record_bulk_on_connection(&mut conn, &entries).unwrap();
    assert_eq!(count(&mut conn), 10001);
    record_bulk_on_connection(&mut conn, &entries).unwrap();
    let mut conflict = envelope(10001);
    conflict.payload = r#"{"changed":true}"#.into();
    let mut batch: Vec<_> = (10002..=20002).map(envelope).collect();
    batch.push(conflict);
    assert!(matches!(
        record_bulk_on_connection(&mut conn, &batch),
        Err(OutboxError::Conflict { .. })
    ));
    assert_eq!(count(&mut conn), 10001);
}

#[test]
#[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
fn caller_rollback_and_savepoint_isolation() {
    let mut conn = connection();
    let result = conn.transaction::<(), OutboxError, _>(|conn| {
        record_bulk_on_connection(conn, &[envelope(1)])?;
        let mut conflict = envelope(1);
        conflict.payload = r#"{"changed":true}"#.into();
        assert!(matches!(
            record_bulk_on_connection(conn, &[envelope(2), conflict]),
            Err(OutboxError::Conflict { .. })
        ));
        assert_eq!(count(conn), 1);
        Err(OutboxError::Database(
            diesel::result::Error::RollbackTransaction,
        ))
    });
    assert!(result.is_err());
    assert_eq!(count(&mut conn), 0);
}
