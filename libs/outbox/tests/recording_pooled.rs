#![cfg(feature = "diesel")]
mod common;

use common::*;
use diesel::r2d2::{ConnectionManager, Pool};
use mulac_outbox::prelude::*;
use std::sync::Arc;
use uuid::Uuid;

fn unreachable_pool() -> DbPool {
    Pool::builder()
        .connection_timeout(std::time::Duration::from_millis(300))
        .max_size(1)
        .build_unchecked(ConnectionManager::<diesel::PgConnection>::new(
            "postgres://nobody:none@127.0.0.1:1/none",
        ))
}

#[test]
#[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
fn batch_records_every_event_and_replay_succeeds_unchanged() {
    let db = TestDb::new();
    let recorder = OutboxRecorder::new(Arc::new(db.repo()));
    let ids = ids(5);
    let batch = envelopes(&ids);

    recorder.record_batch(&batch).expect("first batch");
    assert_eq!(db.count(), 5);
    let before: Vec<_> = ids.iter().map(|i| stored(&mut db.conn(), *i)).collect();

    recorder
        .record_batch(&batch)
        .expect("replay of the same batch");
    assert_eq!(db.count(), 5);
    let after: Vec<_> = ids.iter().map(|i| stored(&mut db.conn(), *i)).collect();
    assert_eq!(before, after);
    // Accepted payloads read back byte-identical.
    for (event, row) in batch.iter().zip(&before) {
        assert_eq!(row.as_ref().unwrap().payload, event.payload);
    }
}

#[test]
#[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
fn batch_above_the_bind_parameter_limit_records_completely_on_every_path() {
    // One more than PostgreSQL's 65,535 bind parameters allow in one statement.
    const LARGE: usize = 65_536;
    for api in BATCH_APIS {
        let db = TestDb::new();
        let batch = envelopes(&ids(LARGE));
        api.record(&db, &batch)
            .unwrap_or_else(|e| panic!("{api:?}: {e:?}"));
        assert_eq!(db.count(), LARGE as i64, "{api:?}");
        api.record(&db, &batch)
            .unwrap_or_else(|e| panic!("{api:?} replay: {e:?}"));
        assert_eq!(db.count(), LARGE as i64, "{api:?} replay");
    }
}

#[test]
#[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
fn replay_conflict_after_new_writes_leaves_no_new_rows() {
    let db = TestDb::new();
    let repo = db.repo();
    let existing = Uuid::now_v7();
    repo.record(&entry(existing)).unwrap();
    let mut changed = entry(existing);
    changed.payload = r#"{"different":true}"#.into();
    let mut batch: Vec<_> = ids(3).into_iter().map(entry).collect();
    batch.push(changed);

    let err = repo.record_batch(&batch).expect_err("conflict");
    assert!(matches!(
        err,
        RecordingError::ReplayConflict { event_id, field: "payload" } if event_id == existing
    ));
    assert_eq!(err.outcome(), RecordingOutcome::NoRowsAdded);
    assert_eq!(db.count(), 1);
}

#[test]
#[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
fn empty_batches_succeed_without_a_connection() {
    let repo = DieselPgOutboxRepository::new(unreachable_pool());
    repo.record_batch(&[]).expect("empty store batch");
    OutboxRecorder::new(Arc::new(repo))
        .record_batch(&[])
        .expect("empty recorder batch");
}
