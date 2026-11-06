//! Runs the documented handler example (`examples/record_with_retry.rs`) itself.
#![cfg(feature = "diesel")]
#[path = "common/ack_proxy.rs"]
mod ack_proxy;
mod common;
#[allow(dead_code)]
#[path = "../examples/record_with_retry.rs"]
mod example;

use ack_proxy::AckProxy;
use common::*;
use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use example::*;
use mulac_outbox::prelude::*;
use std::num::NonZeroU32;
use uuid::Uuid;

fn attempts(n: u32) -> NonZeroU32 {
    NonZeroU32::new(n).expect("non-zero")
}

/// The naive callback: every run adds a row.
fn business(conn: &mut PgConnection) -> QueryResult<()> {
    diesel::sql_query("INSERT INTO orders (n) VALUES (1)")
        .execute(conn)
        .map(|_| ())
}

/// The repeat-safe callback: keyed by the plan's operation, so running it again
/// finds its own earlier row.
fn keyed_business(operation: Uuid) -> impl Fn(&mut PgConnection) -> QueryResult<()> {
    move |conn| {
        diesel::sql_query(
            "INSERT INTO orders (operation, n) VALUES ($1, 1) ON CONFLICT (operation) DO NOTHING",
        )
        .bind::<diesel::sql_types::Uuid, _>(operation)
        .execute(conn)
        .map(|_| ())
    }
}

fn setup() -> TestDb {
    let db = TestDb::new();
    db.exec("CREATE TABLE orders (operation UUID PRIMARY KEY DEFAULT gen_random_uuid(), n INT)");
    db
}

fn orders(db: &TestDb) -> i64 {
    #[derive(QueryableByName)]
    struct C {
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        count: i64,
    }
    diesel::sql_query("SELECT count(*) AS count FROM orders")
        .get_result::<C>(&mut db.conn())
        .unwrap()
        .count
}

fn plan(ids: &[Uuid]) -> Plan {
    Plan {
        operation_id: Uuid::now_v7(),
        events: envelopes(ids),
    }
}

/// Terminates the backend when `id` is inserted: always, or only the first time
/// (a sequence survives the rolled-back transaction, a table row would not).
fn kill_on_insert(db: &TestDb, id: Uuid, only_first_time: bool) {
    let condition = if only_first_time {
        "nextval('fault_seq') = 1"
    } else {
        "true"
    };
    db.exec(&format!(
        "CREATE SEQUENCE fault_seq; \
         CREATE FUNCTION kill_on_insert() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN IF NEW.id = '{id}' AND {condition} THEN \
         PERFORM pg_terminate_backend(pg_backend_pid()); PERFORM pg_sleep(1); END IF; RETURN NEW; END $$; \
         CREATE TRIGGER kill_on_insert BEFORE INSERT ON outbox_entries \
         FOR EACH ROW EXECUTE FUNCTION kill_on_insert();"
    ));
}

#[test]
fn the_policy_covers_every_recording_case() {
    let id = Uuid::nil();
    let source = || -> Box<dyn std::error::Error + Send + Sync> { "x".into() };
    let cases: Vec<(RecordingError, Policy)> = vec![
        (RecordingError::DuplicateEventId(id), Policy::Stop),
        (
            RecordingError::MismatchedEventId {
                entry_id: id,
                event_id: id,
            },
            Policy::Stop,
        ),
        (RecordingError::InvalidPayload(id), Policy::Stop),
        (
            RecordingError::InvalidMetadata {
                event_id: id,
                field: "routing_key",
            },
            Policy::Stop,
        ),
        (
            RecordingError::ReplayConflict {
                event_id: id,
                field: "payload",
            },
            Policy::Stop,
        ),
        (RecordingError::NoOpenTransaction, Policy::Stop),
        (RecordingError::MissingEvents(vec![id]), Policy::Retry),
        (RecordingError::NoConnection(source()), Policy::Retry),
        (RecordingError::DatabaseRolledBack(source()), Policy::Retry),
        (
            RecordingError::DatabaseUncertain(source()),
            Policy::RetryUncertain,
        ),
    ];
    for (error, expected) in cases {
        let text = error.to_string();
        assert_eq!(policy(&HandlerError::Recording(error)), expected, "{text}");
    }
    let commit = HandlerError::Commit(diesel::result::Error::RollbackTransaction);
    assert_eq!(policy(&commit), Policy::RetryUncertain);
    let business = HandlerError::Business(diesel::result::Error::RollbackTransaction);
    assert_eq!(policy(&business), Policy::Retry);
    // Explicit conversion distinguishes all four kinds.
    assert!(for_subscriber(&commit).contains("commit failed"));
    assert!(for_subscriber(&business).contains("business write failed"));
}

#[test]
#[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
fn business_and_events_commit_together_and_the_history_is_clear() {
    let db = setup();
    let ids = ids(2);
    let mut history = History::default();
    handle(
        &db.pool(),
        &plan(&ids),
        &mut history,
        attempts(3),
        &business,
    )
    .expect("handled");
    assert_eq!((orders(&db), db.count()), (1, 2));
    assert_eq!(history.attempts, [Attempt::Committed]);
    assert!(!history.unresolved);
}

#[test]
#[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
fn a_rejected_input_is_returned_once_and_rolls_the_business_write_back() {
    let db = setup();
    let id = Uuid::now_v7();
    let mut bad = plan(&[id]);
    bad.events[0].payload = "[]".into();
    let mut history = History::default();
    let error = handle(&db.pool(), &bad, &mut history, attempts(3), &business).unwrap_err();
    assert!(matches!(error, HandlerError::Recording(RecordingError::InvalidPayload(i)) if i == id));
    assert_eq!(
        history.attempts,
        [Attempt::NotCommitted],
        "no retry for a fix-input error"
    );
    assert_eq!((orders(&db), db.count()), (0, 0));
}

#[test]
#[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
fn an_uncertain_attempt_is_retried_with_the_same_ids_and_resolves_on_success() {
    let db = setup();
    let ids = ids(1);
    kill_on_insert(&db, ids[0], true);
    let plan = plan(&ids);
    let mut history = History::default();
    handle(&db.pool(), &plan, &mut history, attempts(3), &business)
        .expect("second attempt succeeds");
    assert_eq!(history.attempts, [Attempt::Uncertain, Attempt::Committed]);
    assert!(!history.unresolved);
    assert!(
        history.logged[0].contains(&ids[0].to_string()),
        "event IDs are logged"
    );
    // Same IDs, once; the business write happened once.
    assert_eq!((orders(&db), db.count()), (1, 1));
    assert!(stored(&mut db.conn(), ids[0]).is_some());
}

#[test]
#[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
fn a_conflict_after_an_uncertain_attempt_escalates_and_the_outcome_stays_unresolved() {
    let db = setup();
    let (fresh, existing) = (Uuid::now_v7(), Uuid::now_v7());
    // `existing` is stored with different content than the plan will replay.
    let mut other = envelope(existing);
    other.payload = r#"{"regenerated":true}"#.into();
    Api::Port.record(&db, &[other]).unwrap();
    kill_on_insert(&db, fresh, true);

    let plan = plan(&[fresh, existing]);
    let mut history = History::default();
    let error = handle(&db.pool(), &plan, &mut history, attempts(3), &business).unwrap_err();
    match error {
        HandlerError::Escalate {
            reason,
            event_ids,
            last,
        } => {
            assert_eq!(reason, "failed after an uncertain attempt");
            assert_eq!(
                event_ids,
                [fresh, existing],
                "the plan's IDs, never replacements"
            );
            assert!(matches!(
                *last,
                HandlerError::Recording(RecordingError::ReplayConflict { .. })
            ));
        }
        other => panic!("expected escalation, got {other:?}"),
    }
    assert!(
        history.unresolved,
        "the earlier uncertain attempt is not erased"
    );
    assert_eq!(history.attempts, [Attempt::Uncertain]);
    assert_eq!((orders(&db), db.count()), (0, 1));
}

#[test]
#[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
fn exhausted_retries_after_uncertain_attempts_stay_uncertain_with_the_same_ids() {
    let db = setup();
    let ids = ids(1);
    kill_on_insert(&db, ids[0], false);
    let plan = plan(&ids);
    let mut history = History::default();
    let error = handle(&db.pool(), &plan, &mut history, attempts(3), &business).unwrap_err();
    match error {
        HandlerError::Escalate {
            reason,
            event_ids,
            last,
        } => {
            assert_eq!(reason, "retries exhausted");
            assert_eq!(event_ids, ids);
            assert!(matches!(
                *last,
                HandlerError::Recording(RecordingError::DatabaseUncertain(_))
            ));
        }
        other => panic!("expected escalation, got {other:?}"),
    }
    assert!(history.unresolved);
    assert_eq!(
        history.attempts,
        vec![Attempt::Uncertain, Attempt::Uncertain, Attempt::Uncertain]
    );
    assert_eq!(history.logged.len(), 3);
    assert_eq!(
        plan.events[0].metadata.event_id, ids[0],
        "the plan was never regenerated"
    );
}

#[test]
#[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
fn a_failed_commit_is_a_commit_error_and_never_a_recording_error() {
    let db = setup();
    db.exec(
        "CREATE FUNCTION refuse() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN RAISE EXCEPTION 'commit refused'; END $$; \
         CREATE CONSTRAINT TRIGGER refuse AFTER INSERT ON outbox_entries \
         DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION refuse();",
    );
    let plan = plan(&ids(1));
    let mut history = History::default();
    let error = handle(&db.pool(), &plan, &mut history, attempts(1), &business).unwrap_err();
    match error {
        HandlerError::Escalate { last, .. } => {
            assert!(matches!(*last, HandlerError::Commit(_)), "{last:?}");
        }
        other => panic!("expected escalation, got {other:?}"),
    }
    assert_eq!(history.attempts, [Attempt::Uncertain]);
    assert!(history.unresolved);
}

#[test]
#[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
fn a_failed_callback_followed_by_a_failed_rollback_keeps_the_callback_error() {
    let db = setup();
    // The callback ends its own session, so the rollback that follows fails too.
    let double_failure = |conn: &mut PgConnection| -> QueryResult<()> {
        conn.batch_execute("SELECT pg_terminate_backend(pg_backend_pid()); SELECT pg_sleep(1)")
    };
    let plan = plan(&ids(1));
    let mut history = History::default();
    let error = handle(
        &db.pool(),
        &plan,
        &mut history,
        attempts(1),
        &double_failure,
    )
    .unwrap_err();
    match error {
        HandlerError::Escalate { last, .. } => match *last {
            HandlerError::Business(ref e) => {
                // The callback's failure; the rollback that follows fails with
                // a different kind (UnableToSendCommand) and must not replace it.
                assert!(
                    matches!(
                        e,
                        diesel::result::Error::DatabaseError(
                            diesel::result::DatabaseErrorKind::ClosedConnection,
                            _
                        )
                    ),
                    "expected the callback's ClosedConnection, not the rollback's error: {e:?}"
                );
            }
            other => panic!("expected the callback's business error, got {other:?}"),
        },
        other => panic!("expected escalation, got {other:?}"),
    }
    assert_eq!(db.count(), 0);
}

/// A pool whose connections run through a proxy that loses the first COMMIT
/// acknowledgment: the database commits, the application sees a dead connection.
fn pool_losing_one_commit_acknowledgment(db: &TestDb) -> (AckProxy, DbPool) {
    let proxy = AckProxy::start(&base_url(), 1);
    let pool = build_pool(&proxy.url(&db.url())).expect("pool through the proxy");
    (proxy, pool)
}

#[test]
#[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
fn an_unacknowledged_commit_is_retried_without_repeating_a_keyed_business_write() {
    let db = setup();
    let (proxy, pool) = pool_losing_one_commit_acknowledgment(&db);
    let plan = plan(&ids(2));
    let mut history = History::default();
    let callback = keyed_business(plan.operation_id);

    handle(&pool, &plan, &mut history, attempts(4), &callback).expect("a later attempt succeeds");

    assert_eq!(
        proxy.cuts(),
        1,
        "exactly one commit acknowledgment was lost"
    );
    assert_eq!(history.attempts.first(), Some(&Attempt::Uncertain));
    assert_eq!(history.attempts.last(), Some(&Attempt::Committed));
    assert!(!history.unresolved);
    // The first attempt really committed; the retry added neither business
    // work nor outbox rows.
    assert_eq!((orders(&db), db.count()), (1, 2));
}

#[test]
#[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
fn an_unacknowledged_commit_repeats_a_business_write_that_is_not_keyed() {
    let db = setup();
    let (proxy, pool) = pool_losing_one_commit_acknowledgment(&db);
    let plan = plan(&ids(2));
    let mut history = History::default();

    handle(&pool, &plan, &mut history, attempts(4), &business).expect("a later attempt succeeds");

    assert_eq!(proxy.cuts(), 1);
    // The outbox rows are protected by their IDs; the naive business row is not.
    // This is why the callback must be safe to repeat.
    assert_eq!((orders(&db), db.count()), (2, 2));
}
