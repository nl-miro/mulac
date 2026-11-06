#![cfg(feature = "diesel")]
mod common;

use common::corpus;

use common::*;
use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use diesel::r2d2::{ConnectionManager, Pool};
use mulac_outbox::prelude::*;
use std::sync::Arc;
use uuid::Uuid;

fn unreachable_repo() -> DieselPgOutboxRepository {
    DieselPgOutboxRepository::new(
        Pool::builder()
            .connection_timeout(std::time::Duration::from_millis(300))
            .max_size(1)
            .build_unchecked(ConnectionManager::<PgConnection>::new(
                "postgres://nobody:none@127.0.0.1:1/none",
            )),
    )
}

fn dead_connection(db: &TestDb) -> PgConnection {
    #[derive(QueryableByName)]
    struct Pid {
        #[diesel(sql_type = diesel::sql_types::Integer)]
        pid: i32,
    }
    let mut conn = db.conn();
    conn.batch_execute("BEGIN").unwrap();
    let pid = diesel::sql_query("SELECT pg_backend_pid() AS pid")
        .get_result::<Pid>(&mut conn)
        .unwrap()
        .pid;
    db.conn()
        .batch_execute(&format!("SELECT pg_terminate_backend({pid})"))
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(150));
    conn
}

/// Records without any reachable database: only validation can answer.
fn offline(api: Api, db: &TestDb, batch: &[NewOutboxEnvelope]) -> RecordingError {
    assert!(
        !api.is_single() || batch.len() == 1,
        "{api:?} takes one event"
    );
    match api {
        Api::RecorderSingle => OutboxRecorder::new(Arc::new(unreachable_repo()))
            .record(&batch[0])
            .unwrap_err(),
        Api::PortSingle => unreachable_repo()
            .record(&NewOutboxEntry::from(&batch[0]))
            .unwrap_err(),
        Api::Recorder => OutboxRecorder::new(Arc::new(unreachable_repo()))
            .record_batch(batch)
            .unwrap_err(),
        Api::Port => {
            let entries: Vec<_> = batch.iter().map(NewOutboxEntry::from).collect();
            unreachable_repo().record_batch(&entries).unwrap_err()
        }
        Api::Caller => record_batch_on_connection(&mut dead_connection(db), batch).unwrap_err(),
    }
}

fn with_payload(payload: &str) -> NewOutboxEnvelope {
    let mut e = envelope(Uuid::now_v7());
    e.payload = payload.to_string();
    e
}

#[test]
fn the_shared_corpus_really_contains_the_escapes_it_claims_to_test() {
    corpus::assert_corpus_exercises_escapes();
}

#[test]
#[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
fn invalid_payloads_are_rejected_without_reaching_a_database() {
    let rejected = corpus::rejected();
    let db = TestDb::new();
    for api in APIS {
        for (name, payload) in &rejected {
            let err = offline(api, &db, &[with_payload(payload)]);
            assert!(
                matches!(err, RecordingError::InvalidPayload(_)),
                "{api:?} {name}: {err:?}"
            );
            assert_eq!(err.outcome(), RecordingOutcome::NoRowsAdded);
        }
    }
    assert_eq!(db.count(), 0);
}

#[test]
#[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
fn accepted_payloads_are_stored_exactly_as_supplied() {
    let accepted = corpus::accepted();
    for api in APIS {
        let db = TestDb::new();
        for (name, payload) in &accepted {
            let id = Uuid::now_v7();
            let mut e = envelope(id);
            e.payload = payload.clone();
            api.record(&db, &[e.clone()])
                .unwrap_or_else(|err| panic!("{api:?} {name}: {err:?}"));
            assert_eq!(
                stored(&mut db.conn(), id).unwrap().payload,
                *payload,
                "{api:?} {name}: byte-identical"
            );
            api.record(&db, &[e])
                .unwrap_or_else(|err| panic!("{api:?} {name} replay: {err:?}"));
        }
    }
}

#[test]
#[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
fn payload_equivalence_ignores_layout_but_compares_numbers_by_written_form() {
    let equivalent = corpus::equivalent();
    let different = corpus::different();
    for api in APIS {
        for &(first, second) in &equivalent {
            let db = TestDb::new();
            let id = Uuid::now_v7();
            let (mut a, mut b) = (envelope(id), envelope(id));
            (a.payload, b.payload) = (first.into(), second.into());
            api.record(&db, &[a]).unwrap();
            api.record(&db, &[b])
                .unwrap_or_else(|e| panic!("{api:?} {first} ~ {second}: {e:?}"));
            assert_eq!(
                stored(&mut db.conn(), id).unwrap().payload,
                first,
                "first writer's bytes kept"
            );
        }
        for &(first, second) in &different {
            let db = TestDb::new();
            let id = Uuid::now_v7();
            let (mut a, mut b) = (envelope(id), envelope(id));
            (a.payload, b.payload) = (first.into(), second.into());
            api.record(&db, &[a]).unwrap();
            let err = api.record(&db, &[b]).expect_err("must conflict");
            assert!(
                matches!(
                    err,
                    RecordingError::ReplayConflict {
                        field: "payload",
                        ..
                    }
                ),
                "{api:?} {first} vs {second}: {err:?}"
            );
        }
    }
}

#[test]
#[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
fn metadata_and_batch_rules_hold_on_every_path() {
    let db = TestDb::new();
    // Duplicate IDs need a batch, so only the batch APIs can be given one, even
    // when the two events are identical.
    for api in BATCH_APIS {
        let id = Uuid::now_v7();
        let err = offline(api, &db, &[envelope(id), envelope(id)]);
        assert!(
            matches!(err, RecordingError::DuplicateEventId(d) if d == id),
            "{api:?}"
        );
    }

    // Every other rule applies to a single event, through every API.
    for api in APIS {
        // NUL in each metadata string.
        for field in ["routing_key", "event_type", "source", "content_type"] {
            let mut e = envelope(Uuid::now_v7());
            let nul = Some("a\0b".to_string());
            match field {
                "routing_key" => e.metadata.routing_key = "a\0b".into(),
                "event_type" => e.metadata.event_type = "a\0b".into(),
                "source" => e.metadata.source = nul,
                _ => e.metadata.content_type = nul,
            }
            let err = offline(api, &db, &[e]);
            assert!(
                matches!(&err, RecordingError::InvalidMetadata { field: f, .. } if *f == field),
                "{api:?} {field}: {err:?}"
            );
        }

        // Blank routing keys are rejected, including Unicode whitespace.
        for blank in ["", " ", "\t\n", "\u{a0}", "\u{2003}"] {
            let mut e = envelope(Uuid::now_v7());
            e.metadata.routing_key = blank.into();
            let err = offline(api, &db, &[e]);
            assert!(
                matches!(
                    err,
                    RecordingError::InvalidMetadata {
                        field: "routing_key",
                        ..
                    }
                ),
                "{api:?} {blank:?}"
            );
        }

        // Sequence bounds.
        for bad in [0, i64::MAX as u64 + 1, u64::MAX] {
            let mut e = envelope(Uuid::now_v7());
            e.metadata.sequence = Some(bad);
            let err = offline(api, &db, &[e]);
            assert!(
                matches!(
                    err,
                    RecordingError::InvalidMetadata {
                        field: "sequence",
                        ..
                    }
                ),
                "{api:?} {bad}"
            );
        }
    }

    // Accepted: routing keys are stored untrimmed; sequences 1 and i64::MAX; no sequence.
    for api in APIS {
        let db = TestDb::new();
        for sequence in [None, Some(1), Some(i64::MAX as u64)] {
            let id = Uuid::now_v7();
            let mut e = envelope(id);
            e.metadata.sequence = sequence;
            e.metadata.routing_key = " padded.key ".into();
            api.record(&db, &[e])
                .unwrap_or_else(|err| panic!("{api:?} {sequence:?}: {err:?}"));
            let meta: OutboxEntryMetadata =
                serde_json::from_str(&stored(&mut db.conn(), id).unwrap().meta).unwrap();
            assert_eq!(meta.routing_key, " padded.key ");
            assert_eq!(meta.sequence, sequence);
        }
    }
}

#[test]
#[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
fn mismatched_entry_and_metadata_ids_are_rejected_through_the_storage_port() {
    let (entry_id, event_id) = (Uuid::now_v7(), Uuid::now_v7());
    let mut mismatched = entry(entry_id);
    mismatched.meta.event_id = event_id;
    let err = unreachable_repo()
        .record_batch(&[mismatched.clone()])
        .unwrap_err();
    assert!(matches!(
        err,
        RecordingError::MismatchedEventId { entry_id: e, event_id: m } if e == entry_id && m == event_id
    ));
    assert_eq!(err.outcome(), RecordingOutcome::NoRowsAdded);
    // The single-entry port method reports the same error.
    let err = unreachable_repo().record(&mismatched).unwrap_err();
    assert!(matches!(
        err,
        RecordingError::MismatchedEventId { entry_id: e, event_id: m } if e == entry_id && m == event_id
    ));
    // Also with a reachable database: nothing is written.
    let db = TestDb::new();
    assert!(db.repo().record_batch(&[mismatched.clone()]).is_err());
    assert!(db.repo().record(&mismatched).is_err());
    assert_eq!(db.count(), 0);
}
