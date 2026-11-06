#![cfg(feature = "diesel")]
mod common;

use common::*;
use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use mulac_outbox::prelude::*;
use std::sync::{Arc, Barrier};
use std::thread;
use uuid::Uuid;

fn changed(id: Uuid) -> NewOutboxEntry {
    let mut e = entry(id);
    e.payload = r#"{"changed":true}"#.into();
    e
}

/// Holds `ids` uncommitted on a caller connection.
fn holder(db: &TestDb, ids: &[Uuid]) -> PgConnection {
    let mut conn = db.conn();
    conn.batch_execute("BEGIN").unwrap();
    record_batch_on_connection(&mut conn, &envelopes(ids)).expect("holder records");
    conn
}

#[test]
#[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
fn matching_same_id_writers_succeed_with_one_row() {
    let db = TestDb::new();
    let id = Uuid::now_v7();
    let mut first = holder(&db, &[id]);
    let writers: Vec<_> = (0..2)
        .map(|_| {
            let repo = db.repo();
            thread::spawn(move || repo.record(&entry(id)))
        })
        .collect();
    wait_blocked(&db, 2);
    first.batch_execute("COMMIT").unwrap();
    for writer in writers {
        join(writer).expect("matching writer succeeds");
    }
    assert_eq!(db.count(), 1);
}

#[test]
#[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
fn differing_same_id_writers_give_one_commit_and_conflicts() {
    let db = TestDb::new();
    let id = Uuid::now_v7();
    let mut first = holder(&db, &[id]);
    let writers: Vec<_> = (0..2)
        .map(|_| {
            let repo = db.repo();
            thread::spawn(move || repo.record(&changed(id)))
        })
        .collect();
    wait_blocked(&db, 2);
    first.batch_execute("COMMIT").unwrap();
    for writer in writers {
        let err = join(writer).expect_err("differing writer conflicts");
        assert!(matches!(
            err,
            RecordingError::ReplayConflict {
                field: "payload",
                ..
            }
        ));
    }
    assert_eq!(db.count(), 1);
    let kept = stored(&mut db.conn(), id).unwrap();
    assert_eq!(
        kept.payload,
        envelope(id).payload,
        "the committed writer's row is kept"
    );
}

#[test]
#[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
fn overlapping_batches_in_opposite_orders_across_chunk_boundaries_do_not_deadlock() {
    // More rows than one insertion chunk. An uncommitted row past the boundary
    // stalls both batches mid-range: without one global insertion order each
    // would then hold half the range the other needs once it is released.
    const N: usize = 5_100;
    const HELD: usize = 5_050;
    for round in 0..3 {
        let db = TestDb::new();
        let mut all = ids(N);
        all.sort();
        let mut first = holder(&db, &all[HELD..=HELD]);
        let ascending = db.repo();
        let descending = db.repo();
        let up: Vec<_> = all.iter().copied().map(entry).collect();
        let down: Vec<_> = all.iter().rev().copied().map(entry).collect();
        let a = thread::spawn(move || ascending.record_batch(&up));
        let b = thread::spawn(move || descending.record_batch(&down));
        // Both batches are stalled behind a lock: contention is real.
        wait_blocked(&db, 2);
        first.batch_execute("COMMIT").unwrap();
        join(a).unwrap_or_else(|e| panic!("round {round}: ascending batch failed: {e:?}"));
        join(b).unwrap_or_else(|e| panic!("round {round}: descending batch failed: {e:?}"));
        assert_eq!(db.count(), N as i64);
        for id in [all[0], all[HELD], all[N - 1]] {
            assert_eq!(
                stored(&mut db.conn(), id).unwrap().payload,
                envelope(id).payload
            );
        }
    }
}

fn raw_insert(id: Uuid) -> String {
    format!(
        "INSERT INTO outbox_entries (id, status, payload, meta) VALUES ('{id}', 0, '{{}}', '{{}}')"
    )
}

/// A test-only unsorted control: the same shape of contention as the sorted
/// scenario below, but inserting in arrival order, genuinely deadlocks.
#[test]
#[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
fn unsorted_control_deadlocks_where_sorted_recording_does_not() {
    let db = TestDb::new();
    let (mut a, mut b) = (Uuid::now_v7(), Uuid::now_v7());
    if a > b {
        std::mem::swap(&mut a, &mut b);
    }

    // Control: t1 holds `a`, t2 inserts `b` then `a` (arrival order).
    let mut t1 = db.conn();
    t1.batch_execute(&format!("BEGIN; {}", raw_insert(a)))
        .unwrap();
    let mut t2 = db.conn();
    t2.batch_execute(&format!("BEGIN; {}", raw_insert(b)))
        .unwrap();
    let waiting = thread::spawn(move || {
        let result = t2.batch_execute(&raw_insert(a));
        (t2, result)
    });
    wait_blocked(&db, 1);
    let first = t1.batch_execute(&raw_insert(b));
    let (mut t2, second) = join(waiting);
    let failures: Vec<_> = [first, second]
        .into_iter()
        .filter_map(Result::err)
        .collect();
    assert_eq!(
        failures.len(),
        1,
        "exactly one deadlock victim: {failures:?}"
    );
    assert!(
        failures[0].to_string().contains("deadlock detected"),
        "{failures:?}"
    );
    let _ = t1.batch_execute("ROLLBACK");
    let _ = t2.batch_execute("ROLLBACK");
}

#[test]
#[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
fn sorted_recording_does_not_deadlock_in_the_same_shape() {
    let db = TestDb::new();
    let (mut a, mut b) = (Uuid::now_v7(), Uuid::now_v7());
    if a > b {
        std::mem::swap(&mut a, &mut b);
    }
    // t1 holds `a`; t2 supplies [b, a] and so, once sorted, waits on `a` first
    // while holding nothing t1 needs.
    let mut t1 = holder(&db, &[a]);
    let repo = db.repo();
    let t2 = thread::spawn(move || repo.record_batch(&[entry(b), entry(a)]));
    wait_blocked(&db, 1);
    record_batch_on_connection(&mut t1, &[envelope(b)]).expect("t1 is not blocked by t2");
    t1.batch_execute("COMMIT").unwrap();
    join(t2).expect("t2 completes after t1 commits");
    assert_eq!(db.count(), 2);
}

#[test]
#[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
fn deadlock_through_recording_calls_on_both_sides_has_one_victim_and_a_retry_succeeds() {
    let db = TestDb::new();
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    let ready = Arc::new(Barrier::new(2));
    let work = |first: Uuid, second: Uuid| {
        let url = db.url();
        let ready = ready.clone();
        thread::spawn(move || {
            let mut conn = PgConnection::establish(&url).unwrap();
            conn.batch_execute("BEGIN").unwrap();
            record_batch_on_connection(&mut conn, &[envelope(first)]).unwrap();
            ready.wait();
            let result = record_batch_on_connection(&mut conn, &[envelope(second)]);
            if result.is_ok() {
                conn.batch_execute("COMMIT").unwrap();
            } else {
                // The whole transaction is abandoned, as the plan requires.
                conn.batch_execute("ROLLBACK").unwrap();
            }
            result
        })
    };
    let (t1, t2) = (work(a, b), work(b, a));
    let results = [join(t1), join(t2)];
    let victims: Vec<_> = results.iter().filter_map(|r| r.as_ref().err()).collect();
    assert_eq!(victims.len(), 1, "one victim, one commit");
    let victim = victims[0];
    assert!(
        matches!(victim, RecordingError::DatabaseRolledBack(_)),
        "{victim:?}"
    );
    assert_eq!(victim.outcome(), RecordingOutcome::NoRowsAdded);
    let source = std::error::Error::source(victim).unwrap();
    assert!(
        source.downcast_ref::<diesel::result::Error>().is_some(),
        "original error kept"
    );
    assert!(source.to_string().contains("deadlock detected"), "{source}");

    // Retrying the whole transaction succeeds.
    let mut conn = db.conn();
    conn.transaction::<(), diesel::result::Error, _>(|c| {
        record_batch_on_connection(c, &envelopes(&[a, b])).expect("retry");
        Ok(())
    })
    .unwrap();
    assert_eq!(db.count(), 2);
}

#[test]
#[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
fn serialization_failure_above_read_committed_is_retained_and_a_retry_succeeds() {
    let db = TestDb::new();
    let id = Uuid::now_v7();
    let mut conn = db.conn();
    conn.batch_execute("BEGIN ISOLATION LEVEL REPEATABLE READ")
        .unwrap();
    // Establish the snapshot before the competing writer commits.
    assert_eq!(count(&mut conn), 0);
    db.repo()
        .record(&entry(id))
        .expect("competing writer commits");

    let err = record_batch_on_connection(&mut conn, &[envelope(id)]).unwrap_err();
    assert!(
        matches!(err, RecordingError::DatabaseRolledBack(_)),
        "{err:?}"
    );
    let source = std::error::Error::source(&err).unwrap();
    let diesel_error = source
        .downcast_ref::<diesel::result::Error>()
        .expect("diesel error");
    assert!(
        matches!(
            diesel_error,
            diesel::result::Error::DatabaseError(
                diesel::result::DatabaseErrorKind::SerializationFailure,
                _
            )
        ),
        "{diesel_error:?}"
    );
    conn.batch_execute("ROLLBACK").unwrap();

    // A retry of the whole transaction takes a fresh snapshot and succeeds.
    conn.batch_execute("BEGIN ISOLATION LEVEL REPEATABLE READ")
        .unwrap();
    record_batch_on_connection(&mut conn, &[envelope(id)]).expect("retry");
    conn.batch_execute("COMMIT").unwrap();
    assert_eq!(db.count(), 1);
}
