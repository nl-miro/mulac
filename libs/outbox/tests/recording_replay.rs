#![cfg(feature = "diesel")]
mod common;

use chrono::Duration;
use common::*;
use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use mulac_outbox::prelude::*;
use std::thread;
use uuid::Uuid;

fn base(id: Uuid) -> NewOutboxEnvelope {
    let mut e = envelope(id);
    e.metadata.sequence = Some(5);
    e.metadata.correlation_id = Some(Uuid::now_v7());
    e.metadata.causation_id = Some(Uuid::now_v7());
    e
}

type Change = fn(&mut NewOutboxEnvelope);

#[test]
#[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
fn each_compared_field_conflicts_alone_and_leaves_no_new_rows() {
    let cases: Vec<(&str, Change)> = vec![
        ("payload", |e| e.payload = r#"{"other":1}"#.into()),
        ("message_id", |e| {
            e.metadata.message_id = Some(Uuid::now_v7())
        }),
        ("routing_key", |e| {
            e.metadata.routing_key = "other.key".into()
        }),
        ("routing_key", |e| {
            e.metadata.routing_key = " things.happened".into()
        }),
        ("event_type", |e| e.metadata.event_type = "Other".into()),
        ("content_type", |e| e.metadata.content_type = None),
        ("content_type", |e| {
            e.metadata.content_type = Some(String::new())
        }),
        ("content_type", |e| {
            e.metadata.content_type = Some("text/plain".into())
        }),
        ("sequence", |e| e.metadata.sequence = Some(6)),
        ("sequence", |e| e.metadata.sequence = None),
    ];
    for api in APIS {
        for (field, change) in &cases {
            let db = TestDb::new();
            let id = Uuid::now_v7();
            api.record(&db, &[base(id)]).unwrap();
            let before = snapshot(&mut db.conn(), id);

            let mut replay = base(id);
            // Same correlation/causation as stored so only `field` differs.
            replay.metadata = {
                let mut m = replay.metadata;
                let stored: OutboxEntryMetadata =
                    serde_json::from_str(&stored(&mut db.conn(), id).unwrap().meta).unwrap();
                m.correlation_id = stored.correlation_id;
                m.causation_id = stored.causation_id;
                m
            };
            change(&mut replay);
            // Batch APIs also carry a fresh event, to show it is rolled back with
            // the conflict; a single-event API can only replay the one event.
            let attempt = if api.is_single() {
                vec![replay]
            } else {
                vec![envelope(Uuid::now_v7()), replay]
            };
            let err = api.record(&db, &attempt).expect_err("conflict");
            assert!(
                matches!(err, RecordingError::ReplayConflict { event_id, field: f } if event_id == id && f == *field),
                "{api:?} {field}: {err:?}"
            );
            assert_eq!(err.outcome(), RecordingOutcome::NoRowsAdded);
            assert_eq!(db.count(), 1, "{api:?} {field}: no new rows");
            assert_eq!(snapshot(&mut db.conn(), id), before, "stored row untouched");
        }
    }
}

#[test]
#[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
fn tolerated_fields_may_differ_and_keep_their_stored_values() {
    for api in APIS {
        let db = TestDb::new();
        let id = Uuid::now_v7();
        api.record(&db, &[base(id)]).unwrap();
        let before = snapshot(&mut db.conn(), id);

        let mut replay = base(id);
        replay.metadata.correlation_id = Some(Uuid::now_v7());
        replay.metadata.causation_id = None;
        replay.metadata.source = Some("another-source".into());
        replay.metadata.sequence = Some(5);
        // The stored row's compared fields are all unchanged.
        let stored_meta: OutboxEntryMetadata =
            serde_json::from_str(&stored(&mut db.conn(), id).unwrap().meta).unwrap();
        replay.metadata.sequence = stored_meta.sequence;
        api.record(&db, &[replay])
            .expect("tolerated differences replay");
        assert_eq!(
            snapshot(&mut db.conn(), id),
            before,
            "{api:?}: stored values kept"
        );
    }
}

#[test]
#[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
fn omitted_and_explicit_default_message_ids_replay_equivalently_and_reused_message_ids_insert() {
    for api in APIS {
        let db = TestDb::new();
        let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
        let mut explicit = envelope(a);
        explicit.metadata.message_id = Some(a);
        api.record(&db, &[explicit.clone()]).unwrap();
        api.record(&db, &[envelope(a)])
            .expect("omitted equals explicit default");

        api.record(&db, &[envelope(b)]).unwrap();
        let mut explicit_b = envelope(b);
        explicit_b.metadata.message_id = Some(b);
        api.record(&db, &[explicit_b])
            .expect("explicit equals omitted default");

        // A new event ID with a reused message ID creates a row.
        let mut reused = envelope(Uuid::now_v7());
        reused.metadata.message_id = Some(a);
        api.record(&db, &[reused]).expect("new row");
        assert_eq!(db.count(), 3);
    }
}

#[test]
#[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
fn replay_changes_nothing_in_any_delivery_state() {
    for api in APIS {
        let db = TestDb::new();
        let states = [0, 2, 4, 5, 7, 8];
        let rows: Vec<_> = states.iter().map(|s| (Uuid::now_v7(), *s)).collect();
        for (id, state) in &rows {
            api.record(&db, &[envelope(*id)]).unwrap();
            db.exec(&format!(
                "UPDATE outbox_entries SET status = {state}, attempts = 3, last_error = 'old', \
                 reservation_id = CASE WHEN {state} = 2 THEN gen_random_uuid() END, \
                 reserved_at = CASE WHEN {state} = 2 THEN now() END WHERE id = '{id}'"
            ));
        }
        let before: Vec<_> = rows
            .iter()
            .map(|(id, _)| snapshot(&mut db.conn(), *id))
            .collect();
        let batch: Vec<_> = rows.iter().map(|(id, _)| envelope(*id)).collect();
        api.record_each(&db, &batch)
            .expect("replay in every delivery state");
        let after: Vec<_> = rows
            .iter()
            .map(|(id, _)| snapshot(&mut db.conn(), *id))
            .collect();
        assert_eq!(before, after, "{api:?}");
    }
}

/// Rows that break the new rules, seeded with explicit SQL: recording would now
/// refuse to create them.
fn seed_corrupt(db: &TestDb) -> Vec<(Uuid, &'static str)> {
    let valid_meta = |id: Uuid| {
        format!(
            r#"{{"event_id":"{id}","message_id":"{id}","event_type":"Thing.Happened","routing_key":"things.happened","content_type":"application/json","source":"test","correlation_id":null,"causation_id":null}}"#
        )
    };
    let rows = [
        (Uuid::now_v7(), "payload", "[1,2]".to_string(), None),
        (
            Uuid::now_v7(),
            "payload",
            r#"{"a":1,"a":2}"#.to_string(),
            None,
        ),
        (
            Uuid::now_v7(),
            "stored_metadata",
            "{}".to_string(),
            Some(r#"{"bad":true}"#.to_string()),
        ),
        (
            Uuid::now_v7(),
            "stored_event_id",
            "{}".to_string(),
            Some(valid_meta(Uuid::now_v7())),
        ),
        (
            Uuid::now_v7(),
            "stored_metadata",
            "{}".to_string(),
            Some(
                valid_meta(Uuid::nil()).replace("\"event_type\"", "\"sequence\":0,\"event_type\""),
            ),
        ),
    ];
    rows.into_iter()
        .map(|(id, field, payload, meta)| {
            let meta = meta.unwrap_or_else(|| valid_meta(id));
            // The mismatched-event-ID row stores some other ID in its metadata.
            db.exec(&format!(
                "INSERT INTO outbox_entries (id, status, payload, meta) VALUES ('{id}', 0, '{}', '{}')",
                payload.replace('\'', "''"),
                meta.replace('\'', "''")
            ));
            (id, field)
        })
        .collect()
}

#[test]
#[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
fn rule_breaking_stored_rows_conflict_stay_unchanged_and_keep_their_delivery_behavior() {
    let spec = ReservableOutboxSpec::new(50);
    let outcomes = |db: &TestDb| {
        let repo = db.repo();
        let _ = repo.reserve(&spec).map(|healthy| healthy.len());
        let mut conn = db.conn();
        diesel::sql_query(
            "SELECT id, status, attempts FROM outbox_entries ORDER BY payload, meta::text",
        )
        .load::<Row>(&mut conn)
        .unwrap()
    };

    #[derive(QueryableByName, Debug, PartialEq)]
    struct Row {
        #[diesel(sql_type = diesel::sql_types::Uuid)]
        id: Uuid,
        #[diesel(sql_type = diesel::sql_types::Int4)]
        status: i32,
        #[diesel(sql_type = diesel::sql_types::Int4)]
        attempts: i32,
    }

    // Reference: the same rows, never replayed.
    let reference = TestDb::new();
    let ref_rows = seed_corrupt(&reference);
    let reference_outcome: Vec<_> = outcomes(&reference)
        .iter()
        .map(|r| (r.status, r.attempts))
        .collect();

    for api in APIS {
        let db = TestDb::new();
        let seeded = seed_corrupt(&db);
        for (id, field) in &seeded {
            let before = snapshot(&mut db.conn(), *id);
            let err = api.record(&db, &[envelope(*id)]).expect_err("conflict");
            assert!(
                matches!(&err, RecordingError::ReplayConflict { event_id, field: f } if event_id == id && f == field),
                "{api:?} {field}: {err:?}"
            );
            assert_eq!(snapshot(&mut db.conn(), *id), before, "row unchanged");
        }
        // An invalid batch is rejected by validation before any stored row is read.
        let mut invalid = envelope(seeded[0].0);
        invalid.payload = "[]".into();
        assert!(matches!(
            api.record(&db, &[invalid]).unwrap_err(),
            RecordingError::InvalidPayload(_)
        ));
        let after: Vec<_> = outcomes(&db)
            .iter()
            .map(|r| (r.status, r.attempts))
            .collect();
        assert_eq!(
            after, reference_outcome,
            "{api:?}: reservation behavior unchanged"
        );
    }
    assert_eq!(ref_rows.len(), 5);
}

/// Committed fixtures in the three delivery states a consumer touches.
struct Fixtures {
    received: Uuid,
    reserved: Uuid,
    token: Uuid,
    stale: Uuid,
}

fn fixtures(db: &TestDb) -> Fixtures {
    let f = Fixtures {
        received: Uuid::now_v7(),
        reserved: Uuid::now_v7(),
        token: Uuid::now_v7(),
        stale: Uuid::now_v7(),
    };
    for id in [f.received, f.reserved, f.stale] {
        db.repo().record(&entry(id)).unwrap();
    }
    db.exec(&format!(
        "UPDATE outbox_entries SET status = 2, reservation_id = '{}', reserved_at = now(), attempts = 1 WHERE id = '{}';
         UPDATE outbox_entries SET status = 2, reservation_id = gen_random_uuid(), reserved_at = now() - interval '1 hour', attempts = 1 WHERE id = '{}'",
        f.token, f.reserved, f.stale
    ));
    f
}

fn consumers_proceed_while_a_recording_transaction_is_open(mixed: bool) {
    let db = TestDb::new();
    let f = fixtures(&db);
    let mut batch = envelopes(&[f.received, f.reserved, f.stale]);
    let new_id = Uuid::now_v7();
    if mixed {
        batch.push(envelope(new_id));
    }

    // The recording transaction stays open for the whole test.
    let mut recorder = db.conn();
    recorder.batch_execute("BEGIN").unwrap();
    record_batch_on_connection(&mut recorder, &batch).expect("replay in open transaction");

    // Independent connections reserve, complete and sweep the replayed rows.
    let repo = db.repo();
    let worker = thread::spawn(move || {
        let reserved = repo.reserve(&ReservableOutboxSpec::new(10)).unwrap();
        let completed = repo.completed(f.reserved, f.token);
        let swept = repo
            .sweep(&StaleReservationSpec::new(Duration::minutes(5)))
            .unwrap();
        (reserved, completed, swept)
    });
    let (reserved, completed, swept) = join(worker);
    assert_eq!(reserved.len(), 1);
    assert_eq!(
        reserved[0].message.id, f.received,
        "reservation returned the target"
    );
    completed.expect("matching-token completion succeeded");
    assert_eq!(swept, 1, "the stale sweep transitioned its target");

    let mut observer = db.conn();
    assert_eq!(status(&mut observer, f.received), 2);
    assert_eq!(status(&mut observer, f.reserved), 5);
    assert_eq!(status(&mut observer, f.stale), 4);
    assert_eq!(blocked_backends(&db, &mut observer), 0);
    if mixed {
        assert!(
            stored(&mut observer, new_id).is_none(),
            "new row invisible until commit"
        );
    }
    recorder.batch_execute("COMMIT").unwrap();
    if mixed {
        assert!(stored(&mut observer, new_id).is_some());
    }
}

#[test]
#[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
fn replay_of_committed_rows_does_not_block_reservation_completion_or_sweep() {
    consumers_proceed_while_a_recording_transaction_is_open(false);
}

#[test]
#[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
fn mixed_batch_replay_does_not_block_consumers_and_hides_new_rows_until_commit() {
    consumers_proceed_while_a_recording_transaction_is_open(true);
}
