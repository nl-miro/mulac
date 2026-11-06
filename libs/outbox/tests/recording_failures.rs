#![cfg(feature = "diesel")]
mod common;

use common::*;
use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use diesel::r2d2::{ConnectionManager, Pool};
use mulac_outbox::prelude::*;
use std::error::Error;
use uuid::Uuid;

fn unreachable_repo() -> DieselPgOutboxRepository {
    DieselPgOutboxRepository::new(
        Pool::builder()
            .max_size(1)
            .build_unchecked(ConnectionManager::<PgConnection>::new(
                "postgres://nobody:none@127.0.0.1:1/none",
            )),
    )
}

/// More rows than one insertion chunk, so the failure comes after earlier
/// statements have really written rows.
const LARGE: usize = 5_010;

/// Installs a trigger that runs `action` when the highest event ID is inserted.
fn trigger_on_last(db: &TestDb, action: &str) -> Vec<Uuid> {
    let mut all = ids(LARGE);
    all.sort();
    let last = all[LARGE - 1];
    db.exec(&format!(
        "CREATE FUNCTION on_last() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN IF NEW.id = '{last}' THEN {action} END IF; RETURN NEW; END $$; \
         CREATE TRIGGER on_last BEFORE INSERT ON outbox_entries \
         FOR EACH ROW EXECUTE FUNCTION on_last();"
    ));
    all
}

#[test]
#[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
fn connection_loss_during_a_batch_reports_uncertain_on_every_path() {
    for api in APIS {
        let db = TestDb::new();
        let all = trigger_on_last(
            &db,
            "PERFORM pg_terminate_backend(pg_backend_pid()); PERFORM pg_sleep(1);",
        );
        let err = api
            .record(&db, &envelopes(&all))
            .expect_err("connection lost");
        // The caller helper cannot roll back a dead session; the others report it.
        assert!(
            matches!(err, RecordingError::DatabaseUncertain(_)),
            "{api:?}: {err:?}"
        );
        assert_eq!(err.outcome(), RecordingOutcome::Uncertain);
        assert!(err.source().is_some());
        // Nothing can have committed: the backend was terminated mid-statement.
        assert_eq!(db.count(), 0, "{api:?}");
    }
}

#[test]
#[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
fn a_rolled_back_database_failure_keeps_the_original_backend_error() {
    for api in APIS {
        let db = TestDb::new();
        let all = trigger_on_last(&db, "RAISE EXCEPTION 'forced failure';");
        let err = api
            .record(&db, &envelopes(&all))
            .expect_err("database failure");
        assert!(
            matches!(err, RecordingError::DatabaseRolledBack(_)),
            "{api:?}: {err:?}"
        );
        assert_eq!(err.outcome(), RecordingOutcome::NoRowsAdded);
        let source = err.source().expect("source");
        assert!(source.downcast_ref::<diesel::result::Error>().is_some());
        assert!(source.to_string().contains("forced failure"));
        assert_eq!(db.count(), 0);
    }
}

#[test]
#[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
fn a_failed_pooled_commit_reports_uncertain_and_a_caller_commit_failure_is_the_callers() {
    let db = TestDb::new();
    db.exec(
        "CREATE FUNCTION fail_commit() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN RAISE EXCEPTION 'commit refused'; END $$; \
         CREATE CONSTRAINT TRIGGER fail_commit AFTER INSERT ON outbox_entries \
         DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION fail_commit();",
    );
    // Pooled: the failure happens at COMMIT, so the outcome is uncertain.
    let err = db.repo().record(&entry(Uuid::now_v7())).unwrap_err();
    assert!(
        matches!(err, RecordingError::DatabaseUncertain(_)),
        "{err:?}"
    );
    assert!(err.source().unwrap().to_string().contains("commit refused"));

    // Caller connection: recording succeeds; the caller's own COMMIT fails and
    // that is an application error, never a recording error.
    let mut conn = db.conn();
    conn.batch_execute("BEGIN").unwrap();
    record_batch_on_connection(&mut conn, &[envelope(Uuid::now_v7())])
        .expect("recording succeeded");
    let commit = conn.batch_execute("COMMIT");
    assert!(commit.unwrap_err().to_string().contains("commit refused"));
    assert_eq!(db.count(), 0);
}

#[test]
#[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
fn no_connection_is_reported_with_its_source_and_no_rows_added() {
    let repo = unreachable_repo();
    let err = repo.record(&entry(Uuid::now_v7())).unwrap_err();
    assert!(matches!(err, RecordingError::NoConnection(_)), "{err:?}");
    assert_eq!(err.outcome(), RecordingOutcome::NoRowsAdded);
    assert!(err.source().is_some());
}

#[test]
#[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
fn every_failures_table_case_carries_its_identifying_data_and_outcome() {
    let db = TestDb::new();
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());

    let dup = db.repo().record_batch(&[entry(a), entry(a)]).unwrap_err();
    assert!(matches!(dup, RecordingError::DuplicateEventId(id) if id == a));

    let mut mismatched = entry(a);
    mismatched.meta.event_id = b;
    let mm = db.repo().record_batch(&[mismatched]).unwrap_err();
    assert!(
        matches!(mm, RecordingError::MismatchedEventId { entry_id, event_id } if entry_id == a && event_id == b)
    );

    let mut bad = entry(a);
    bad.payload = "[]".into();
    let payload = db.repo().record_batch(&[bad]).unwrap_err();
    assert!(matches!(payload, RecordingError::InvalidPayload(id) if id == a));

    let mut blank = entry(a);
    blank.meta.routing_key = " ".into();
    let meta = db.repo().record_batch(&[blank]).unwrap_err();
    assert!(
        matches!(meta, RecordingError::InvalidMetadata { event_id, field: "routing_key" } if event_id == a)
    );

    db.repo().record(&entry(a)).unwrap();
    let mut changed = entry(a);
    changed.payload = r#"{"changed":1}"#.into();
    let conflict = db.repo().record_batch(&[changed]).unwrap_err();
    assert!(
        matches!(conflict, RecordingError::ReplayConflict { event_id, field: "payload" } if event_id == a)
    );

    let none = record_batch_on_connection(&mut db.conn(), &[envelope(b)]).unwrap_err();
    assert!(matches!(none, RecordingError::NoOpenTransaction));

    for err in [&dup, &mm, &payload, &meta, &conflict, &none] {
        assert_eq!(err.outcome(), RecordingOutcome::NoRowsAdded, "{err}");
    }
}

#[test]
fn display_is_bounded_while_ids_and_sources_stay_complete_as_data() {
    let ids: Vec<Uuid> = (0..10_000).map(|_| Uuid::now_v7()).collect();
    let err = RecordingError::MissingEvents(ids.clone());
    assert!(err.to_string().len() < 300, "{}", err.to_string().len());
    assert!(err.to_string().contains("and 9997 more"));
    assert!(matches!(&err, RecordingError::MissingEvents(all) if all == &ids));

    let long = "x".repeat(50_000);
    let source: Box<dyn Error + Send + Sync> = long.clone().into();
    let err = RecordingError::DatabaseUncertain(source);
    assert!(err.to_string().len() < 400, "{}", err.to_string().len());
    assert!(err.to_string().ends_with("outcome uncertain"));
    assert_eq!(
        err.source().unwrap().to_string(),
        long,
        "full source stays inspectable"
    );

    let multibyte: Box<dyn Error + Send + Sync> = "é".repeat(1_000).into();
    let _ = RecordingError::DatabaseRolledBack(multibyte).to_string(); // never splits a char
}

#[test]
#[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
fn crate_authored_diagnostics_never_quote_payload_content() {
    const SECRET: &str = "s3cr3t-card-4111";
    let db = TestDb::new();
    let id = Uuid::now_v7();
    let secret_payload = |extra: &str| format!(r#"{{"{SECRET}":"{SECRET}"{extra}}}"#);
    let all_text = |e: &RecordingError| format!("{e} {e:?}");

    // Invalid payload with the secret in a key (duplicate keys).
    let mut dup_keys = envelope(id);
    dup_keys.payload = secret_payload(&format!(r#","{SECRET}":1"#));
    // Replay conflict where both payloads hold the secret.
    db.repo()
        .record_batch(&[{
            let mut e = entry(id);
            e.payload = secret_payload("");
            e
        }])
        .unwrap();
    let mut changed = envelope(id);
    changed.payload = secret_payload(r#","n":2"#);
    // Invalid metadata next to a secret payload.
    let mut blank = envelope(Uuid::now_v7());
    blank.payload = secret_payload("");
    blank.metadata.routing_key = " ".into();

    for api in APIS {
        for batch in [
            vec![dup_keys.clone()],
            vec![changed.clone()],
            vec![blank.clone()],
        ] {
            let err = api.record(&db, &batch).unwrap_err();
            let text = all_text(&err);
            assert!(!text.contains(SECRET), "{api:?}: {text}");
        }
    }
}
