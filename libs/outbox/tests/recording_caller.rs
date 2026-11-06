#![cfg(feature = "diesel")]
mod common;

use common::*;
use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use diesel::sql_types::BigInt;
use mulac_outbox::prelude::*;
use uuid::Uuid;

#[derive(QueryableByName)]
struct Biz {
    #[diesel(sql_type = BigInt)]
    count: i64,
}

fn setup() -> TestDb {
    let db = TestDb::new();
    db.exec("CREATE TABLE business (n INT)");
    db
}

fn business(conn: &mut PgConnection) -> i64 {
    diesel::sql_query("SELECT count(*) AS count FROM business")
        .get_result::<Biz>(conn)
        .unwrap()
        .count
}

fn add_business(conn: &mut PgConnection) {
    conn.batch_execute("INSERT INTO business VALUES (1)")
        .unwrap();
}

fn changed(id: Uuid) -> NewOutboxEnvelope {
    let mut e = envelope(id);
    e.payload = r#"{"changed":true}"#.into();
    e
}

#[test]
#[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
fn commit_keeps_business_and_outbox_rows_and_rollback_removes_both() {
    let db = setup();
    // The connection comes from a DbPool; no outbox pool or repository exists.
    let pool = db.pool();
    let mut conn = pool.get().unwrap();
    let kept = ids(3);
    conn.transaction::<(), diesel::result::Error, _>(|c| {
        add_business(c);
        record_batch_on_connection(c, &envelopes(&kept)).expect("record");
        Ok(())
    })
    .unwrap();
    assert_eq!((business(&mut conn), count(&mut conn)), (1, 3));

    let rolled = ids(2);
    let result = conn.transaction::<(), diesel::result::Error, _>(|c| {
        add_business(c);
        record_batch_on_connection(c, &envelopes(&rolled)).expect("record");
        Err(diesel::result::Error::RollbackTransaction)
    });
    assert!(result.is_err());
    assert_eq!((business(&mut conn), count(&mut conn)), (1, 3));
    for id in rolled {
        assert!(stored(&mut conn, id).is_none());
    }
}

#[test]
#[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
fn handled_validation_error_and_conflict_keep_earlier_writes_and_allow_more_work() {
    let db = setup();
    let mut conn = db.conn();
    let existing = Uuid::now_v7();
    record_batch_on_connection_in_tx(&mut conn, existing);
    let (fresh, later) = (Uuid::now_v7(), Uuid::now_v7());
    conn.transaction::<(), diesel::result::Error, _>(|c| {
        add_business(c);
        // Validation failure: duplicate IDs. Nothing is written.
        let dup = envelopes(&[fresh, fresh]);
        let err = record_batch_on_connection(c, &dup).unwrap_err();
        assert!(matches!(err, RecordingError::DuplicateEventId(id) if id == fresh));
        // Conflict after new-row writes: the new row is rolled back with the savepoint.
        let err = record_batch_on_connection(c, &[envelope(fresh), changed(existing)]).unwrap_err();
        assert!(
            matches!(err, RecordingError::ReplayConflict { event_id, .. } if event_id == existing)
        );
        assert_eq!(err.outcome(), RecordingOutcome::NoRowsAdded);
        assert!(
            stored(c, fresh).is_none(),
            "failed batch leaves no new rows"
        );
        assert_eq!(business(c), 1, "earlier business write survives");
        // The transaction is still usable.
        record_batch_on_connection(c, &[envelope(later)]).expect("more work");
        Ok(())
    })
    .unwrap();
    assert_eq!(business(&mut conn), 1);
    assert!(stored(&mut conn, later).is_some());
    assert!(stored(&mut conn, fresh).is_none());
}

fn record_batch_on_connection_in_tx(conn: &mut PgConnection, id: Uuid) {
    conn.transaction::<(), diesel::result::Error, _>(|c| {
        record_batch_on_connection(c, &[envelope(id)]).unwrap();
        Ok(())
    })
    .unwrap();
}

#[test]
#[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
fn no_open_transaction_is_rejected_even_for_an_empty_batch_and_writes_nothing() {
    let db = setup();
    let mut conn = db.conn();
    for batch in [envelopes(&ids(2)), Vec::new()] {
        let err = record_batch_on_connection(&mut conn, &batch).unwrap_err();
        assert!(matches!(err, RecordingError::NoOpenTransaction));
        assert_eq!(err.outcome(), RecordingOutcome::NoRowsAdded);
    }
    assert_eq!(count(&mut conn), 0);
    // Invalid input is reported before connection state is consulted.
    let dup = envelopes(&[Uuid::nil(), Uuid::nil()]);
    let err = record_batch_on_connection(&mut conn, &dup).unwrap_err();
    assert!(matches!(err, RecordingError::DuplicateEventId(_)));
}

#[test]
#[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
fn raw_begin_as_the_first_statement_is_a_transaction_and_commit_or_rollback_decides() {
    let db = setup();
    let mut conn = db.conn();
    let (a, b, c) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());

    conn.batch_execute("BEGIN").unwrap();
    record_batch_on_connection(&mut conn, &[envelope(a)]).expect("recorded after raw BEGIN");
    let err = record_batch_on_connection(&mut conn, &[envelope(b), changed(a)]).unwrap_err();
    assert!(matches!(err, RecordingError::ReplayConflict { .. }));
    // Handled conflict: the earlier write is kept and commits.
    conn.batch_execute("COMMIT").unwrap();
    assert!(stored(&mut conn, a).is_some() && stored(&mut conn, b).is_none());

    conn.batch_execute("BEGIN").unwrap();
    record_batch_on_connection(&mut conn, &[envelope(c)]).unwrap();
    conn.batch_execute("ROLLBACK").unwrap();
    assert!(
        stored(&mut conn, c).is_none(),
        "a later rollback removes the batch"
    );
    assert_eq!(count(&mut conn), 1);
}

#[test]
#[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
fn nested_caller_savepoints_decide_the_batch_and_a_same_named_savepoint_survives() {
    let db = setup();
    let mut conn = db.conn();
    let (a, b, c) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());

    conn.batch_execute("BEGIN; SAVEPOINT outer_sp").unwrap();
    record_batch_on_connection(&mut conn, &[envelope(a)]).unwrap();
    conn.batch_execute("SAVEPOINT inner_sp").unwrap();
    record_batch_on_connection(&mut conn, &[envelope(b)]).unwrap();
    conn.batch_execute("ROLLBACK TO SAVEPOINT inner_sp")
        .unwrap();
    assert!(stored(&mut conn, b).is_none() && stored(&mut conn, a).is_some());

    // A caller savepoint with the crate's own savepoint name is shadowed, not lost.
    conn.batch_execute("SAVEPOINT mulac_outbox_record").unwrap();
    record_batch_on_connection(&mut conn, &[envelope(c)]).unwrap();
    conn.batch_execute("ROLLBACK TO SAVEPOINT mulac_outbox_record")
        .unwrap();
    assert!(
        stored(&mut conn, c).is_none(),
        "caller savepoint still rolls back"
    );
    conn.batch_execute("ROLLBACK TO SAVEPOINT outer_sp")
        .unwrap();
    assert!(stored(&mut conn, a).is_none());
    conn.batch_execute("COMMIT").unwrap();
    assert_eq!(count(&mut conn), 0);
}

#[derive(QueryableByName)]
struct Setting {
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    value: Option<String>,
}

fn probe_setting(conn: &mut PgConnection) -> Option<String> {
    diesel::sql_query("SELECT current_setting('mulac_outbox.probe', true) AS value")
        .get_result::<Setting>(conn)
        .unwrap()
        .value
}

#[test]
#[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
fn a_preexisting_probe_setting_does_not_make_an_idle_connection_look_transactional() {
    let db = setup();
    let mut conn = db.conn();
    // The value the old fixed probe wrote, set for the whole session.
    conn.batch_execute("SET mulac_outbox.probe = '1'").unwrap();
    for batch in [Vec::new(), envelopes(&ids(2))] {
        let err = record_batch_on_connection(&mut conn, &batch).unwrap_err();
        assert!(matches!(err, RecordingError::NoOpenTransaction), "{err:?}");
        assert_eq!(err.outcome(), RecordingOutcome::NoRowsAdded);
    }
    assert_eq!(count(&mut conn), 0);
    assert_eq!(probe_setting(&mut conn).as_deref(), Some("1"));

    // An empty or marker-like value is no different.
    for value in ["", "probe", "00000000-0000-0000-0000-000000000000"] {
        conn.batch_execute(&format!("SET mulac_outbox.probe = '{value}'"))
            .unwrap();
        let err = record_batch_on_connection(&mut conn, &[]).unwrap_err();
        assert!(matches!(err, RecordingError::NoOpenTransaction), "{err:?}");
    }
}

#[test]
#[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
fn a_preexisting_probe_setting_does_not_stop_an_active_transaction_from_being_recognised() {
    let db = setup();
    let mut conn = db.conn();
    conn.batch_execute("SET mulac_outbox.probe = '1'").unwrap();
    conn.batch_execute("BEGIN").unwrap();
    record_batch_on_connection(&mut conn, &[]).expect("empty batch");
    let id = Uuid::now_v7();
    record_batch_on_connection(&mut conn, &[envelope(id)]).expect("batch");
    conn.batch_execute("COMMIT").unwrap();
    assert!(stored(&mut conn, id).is_some());
    assert_eq!(probe_setting(&mut conn).as_deref(), Some("1"));
}

#[test]
#[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
fn the_probe_leaves_the_callers_value_of_its_setting_untouched() {
    let db = setup();
    let mut conn = db.conn();

    // Never defined by the caller: the probe may leave it empty, never a marker.
    conn.batch_execute("BEGIN").unwrap();
    record_batch_on_connection(&mut conn, &[]).unwrap();
    assert!(matches!(
        probe_setting(&mut conn).as_deref(),
        None | Some("")
    ));
    conn.batch_execute("ROLLBACK").unwrap();

    // A session value and a transaction-local value both survive the call.
    conn.batch_execute("SET mulac_outbox.probe = 'session'")
        .unwrap();
    conn.batch_execute("BEGIN").unwrap();
    record_batch_on_connection(&mut conn, &envelopes(&ids(1))).unwrap();
    assert_eq!(probe_setting(&mut conn).as_deref(), Some("session"));
    conn.batch_execute("SET LOCAL mulac_outbox.probe = 'local'")
        .unwrap();
    record_batch_on_connection(&mut conn, &envelopes(&ids(1))).unwrap();
    assert_eq!(probe_setting(&mut conn).as_deref(), Some("local"));
    conn.batch_execute("COMMIT").unwrap();
    assert_eq!(probe_setting(&mut conn).as_deref(), Some("session"));
}

fn kill_backend(db: &TestDb, conn: &mut PgConnection) {
    #[derive(QueryableByName)]
    struct Pid {
        #[diesel(sql_type = diesel::sql_types::Integer)]
        pid: i32,
    }
    let pid = diesel::sql_query("SELECT pg_backend_pid() AS pid")
        .get_result::<Pid>(conn)
        .unwrap()
        .pid;
    db.conn()
        .batch_execute(&format!("SELECT pg_terminate_backend({pid})"))
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(200));
}

#[test]
#[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
fn empty_and_valid_batches_in_idle_active_aborted_and_disconnected_sessions() {
    let db = setup();

    // Active: an empty batch is fine and writes nothing.
    let mut conn = db.conn();
    conn.batch_execute("BEGIN").unwrap();
    record_batch_on_connection(&mut conn, &[]).expect("empty batch in active transaction");
    conn.batch_execute("ROLLBACK").unwrap();

    // Aborted: a database failure, never "no open transaction", empty or not.
    conn.batch_execute("BEGIN").unwrap();
    assert!(conn.batch_execute("SELECT 1/0").is_err());
    for batch in [Vec::new(), envelopes(&ids(1))] {
        let err = record_batch_on_connection(&mut conn, &batch).unwrap_err();
        assert!(
            matches!(err, RecordingError::DatabaseRolledBack(_)),
            "{err:?}"
        );
        assert_eq!(err.outcome(), RecordingOutcome::NoRowsAdded);
    }
    conn.batch_execute("ROLLBACK").unwrap();

    // Disconnected: likewise a database failure.
    for batch in [Vec::new(), envelopes(&ids(1))] {
        let mut dead = db.conn();
        dead.batch_execute("BEGIN").unwrap();
        kill_backend(&db, &mut dead);
        let err = record_batch_on_connection(&mut dead, &batch).unwrap_err();
        assert!(!matches!(err, RecordingError::NoOpenTransaction), "{err:?}");
        assert!(std::error::Error::source(&err).is_some());
    }
    assert_eq!(db.count(), 0);
}
