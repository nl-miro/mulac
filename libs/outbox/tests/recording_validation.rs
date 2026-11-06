#![cfg(feature = "diesel")]
mod common;

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
    match api {
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

/// A payload with exactly `total` nested containers, counting the outermost.
fn nested(total: usize) -> String {
    let is_object = |i: usize| i % 2 == 0;
    let mut s = if is_object(total - 1) { "{}" } else { "[]" }.to_string();
    for i in (0..total - 1).rev() {
        s = if is_object(i) {
            format!(r#"{{"k":{s}}}"#)
        } else {
            format!("[{s}]")
        };
    }
    s
}

#[test]
#[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
fn invalid_payloads_are_rejected_without_reaching_a_database() {
    let too_deep = nested(128);
    let rejected: Vec<(&str, String)> = vec![
        ("array", "[]".into()),
        ("null", "null".into()),
        ("number", "1".into()),
        ("string", r#""s""#.into()),
        ("true", "true".into()),
        ("empty", String::new()),
        ("whitespace only", " \n\t".into()),
        ("two values", "{} {}".into()),
        ("trailing text", "{}x".into()),
        ("BOM", "\u{feff}{}".into()),
        ("NBSP", "\u{a0}{}".into()),
        ("form feed", "\u{c}{}".into()),
        ("vertical tab", "\u{b}{}".into()),
        ("trailing comma", r#"{"a":1,}"#.into()),
        ("leading comma", r#"{,"a":1}"#.into()),
        ("single quotes", "{'a':1}".into()),
        ("unquoted key", "{a:1}".into()),
        ("comment", r#"{"a":1/*x*/}"#.into()),
        ("duplicate key", r#"{"a":1,"a":2}"#.into()),
        ("escaped duplicate key", r#"{"a":1,"a":2}"#.into()),
        ("duplicate nested", r#"{"o":{"a":1,"a":1}}"#.into()),
        ("duplicate across escapes", r#"{"é":1,"é":2}"#.into()),
        ("lone high surrogate", r#"{"s":"\ud800"}"#.into()),
        ("lone low surrogate", r#"{"s":"\udc00"}"#.into()),
        ("high then non-low", r#"{"s":"\ud800a"}"#.into()),
        ("reversed pair", r#"{"s":"\udc00\ud800"}"#.into()),
        ("lone surrogate key", r#"{"\ud800":1}"#.into()),
        ("leading zero", r#"{"n":01}"#.into()),
        ("trailing dot", r#"{"n":1.}"#.into()),
        ("leading dot", r#"{"n":.5}"#.into()),
        ("plus sign", r#"{"n":+1}"#.into()),
        ("bare exponent", r#"{"n":1e}"#.into()),
        ("signed bare exponent", r#"{"n":1e+}"#.into()),
        ("lone minus", r#"{"n":-}"#.into()),
        ("double minus", r#"{"n":--1}"#.into()),
        ("hex", r#"{"n":0x10}"#.into()),
        ("NaN", r#"{"n":NaN}"#.into()),
        ("Infinity", r#"{"n":Infinity}"#.into()),
        ("two dots", r#"{"n":1.2.3}"#.into()),
        ("bad escape", r#"{"s":"\x"}"#.into()),
        ("short unicode escape", r#"{"s":"\u12"}"#.into()),
        ("raw newline in string", "{\"s\":\"a\nb\"}".into()),
        ("raw NUL in string", "{\"s\":\"a\0b\"}".into()),
        ("raw NUL outside string", "{}\0".into()),
        ("unterminated string", r#"{"s":"abc}"#.into()),
        ("unterminated object", r#"{"a":1"#.into()),
        ("128 containers", too_deep),
    ];
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
    let wide = format!(
        "{{{}}}",
        (0..10_000)
            .map(|i| format!("\"k{i}\":{i}"))
            .collect::<Vec<_>>()
            .join(",")
    );
    let accepted: Vec<(&str, String)> = vec![
        ("empty object", "{}".into()),
        (
            "big integer",
            r#"{"n":123456789012345678901234567890}"#.into(),
        ),
        ("huge exponent", r#"{"n":1e999999999999999999999}"#.into()),
        ("negative zero", r#"{"n":-0}"#.into()),
        ("fraction", r#"{"n":0.0,"m":1E+2,"k":-1.5e-3}"#.into()),
        ("unicode", r#"{"s":"é日本語 😀"}"#.into()),
        ("surrogate pair", r#"{"s":"😀"}"#.into()),
        ("escaped NUL value", r#"{"s":"\u0000"}"#.into()),
        ("escaped NUL key", r#"{"\u0000":"x","a\u0000b":1}"#.into()),
        (
            "sibling repeated keys",
            r#"{"a":{"x":1},"b":{"x":2},"c":[{"x":3},{"x":4}]}"#.into(),
        ),
        ("empty key", r#"{"":1}"#.into()),
        (
            "parser token key",
            r#"{"$serde_json::private::Number":"1"}"#.into(),
        ),
        ("json whitespace", " \t\r\n{ \"a\" :\n1 }\r\n".into()),
        ("127 containers", nested(127)),
        ("wide object", wide),
    ];
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
    let equivalent = [
        (r#"{"a":1,"b":2}"#, r#"{"b":2,"a":1}"#),
        (r#"{"a":1}"#, " { \"a\" :\t1 }\n"),
        (r#"{"a":"x"}"#, r#"{"a":"x"}"#),
        (r#"{"s":"a/b"}"#, r#"{"s":"a\/b"}"#),
        (r#"{"s":"é"}"#, r#"{"s":"é"}"#),
        (r#"{"s":"😀"}"#, r#"{"s":"😀"}"#),
        (
            r#"{"o":{"a":1,"b":[1,{"c":2,"d":3}]}}"#,
            r#"{"o":{"b":[1,{"d":3,"c":2}],"a":1}}"#,
        ),
        (r#"{"n":1.0}"#, r#"{ "n" : 1.0 }"#),
    ];
    let different = [
        (r#"{"n":1}"#, r#"{"n":1.0}"#),
        (r#"{"n":1e2}"#, r#"{"n":1E2}"#),
        (r#"{"n":1e2}"#, r#"{"n":1e+2}"#),
        (r#"{"n":0}"#, r#"{"n":-0}"#),
        (r#"{"n":1.0}"#, r#"{"n":1.00}"#),
        (r#"{"n":100}"#, r#"{"n":1e2}"#),
        (
            r#"{"n":18446744073709551616}"#,
            r#"{"n":18446744073709551617}"#,
        ),
        (r#"{"n":1.0000000000000001}"#, r#"{"n":1.0}"#),
        (r#"{"a":[1,2]}"#, r#"{"a":[2,1]}"#),
        (r#"{"s":"é"}"#, "{\"s\":\"e\u{301}\"}"),
        (r#"{"s":"a"}"#, r#"{"s":"A"}"#),
        (r#"{"v":true}"#, r#"{"v":"true"}"#),
        (r#"{"v":null}"#, r#"{"v":"null"}"#),
        (r#"{"a":1}"#, r#"{"a":1,"b":2}"#),
    ];
    for api in APIS {
        for (first, second) in equivalent {
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
        for (first, second) in different {
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
    for api in APIS {
        // Duplicate IDs, even when the two events are identical.
        let id = Uuid::now_v7();
        let err = offline(api, &db, &[envelope(id), envelope(id)]);
        assert!(
            matches!(err, RecordingError::DuplicateEventId(d) if d == id),
            "{api:?}"
        );

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
    // Also with a reachable database: nothing is written.
    let db = TestDb::new();
    assert!(db.repo().record_batch(&[mismatched]).is_err());
    assert_eq!(db.count(), 0);
}
