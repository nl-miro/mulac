//! The outbox payload corpus, run unchanged under whatever `serde_json`
//! features this package was built with. Results must not depend on them.
#[path = "../../../libs/outbox/tests/common/mod.rs"]
mod common;

use common::corpus;
use common::*;
use mulac_outbox::prelude::*;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

#[derive(Default)]
struct Capture(Mutex<Vec<NewOutboxEntry>>);

impl OutboxStorePort for Capture {
    fn record_batch(&self, entries: &[NewOutboxEntry]) -> Result<(), RecordingError> {
        self.0.lock().unwrap().extend_from_slice(entries);
        Ok(())
    }
}

/// Proves which configuration this run really has, so a baseline can never
/// silently run with an augmented `serde_json`.
#[test]
fn the_runtime_configuration_matches_the_requested_features() {
    let huge = serde_json::from_str::<serde_json::Value>("1e999").is_ok();
    assert_eq!(huge, cfg!(feature = "arbitrary_precision"));
    let first_key = serde_json::from_str::<serde_json::Value>(r#"{"b":1,"a":2}"#)
        .unwrap()
        .as_object()
        .unwrap()
        .keys()
        .next()
        .cloned();
    assert_eq!(
        first_key.as_deref() == Some("b"),
        cfg!(feature = "preserve_order")
    );
    #[cfg(feature = "raw_value")]
    let _: Option<&serde_json::value::RawValue> = None;
    eprintln!(
        "serde_json features under test: arbitrary_precision={} preserve_order={} raw_value={}",
        cfg!(feature = "arbitrary_precision"),
        cfg!(feature = "preserve_order"),
        cfg!(feature = "raw_value"),
    );
}

#[test]
fn the_shared_corpus_really_contains_the_escapes_it_claims_to_test() {
    corpus::assert_corpus_exercises_escapes();
}

#[test]
fn validation_corpus_gives_the_same_results_without_a_database() {
    for (name, payload) in corpus::rejected() {
        let capture = Arc::new(Capture::default());
        let recorder = OutboxRecorder::new(capture.clone());
        let mut e = envelope(Uuid::now_v7());
        e.payload = payload;
        assert!(
            matches!(recorder.record(&e), Err(RecordingError::InvalidPayload(_))),
            "{name}"
        );
        assert!(
            capture.0.lock().unwrap().is_empty(),
            "{name}: reached storage"
        );
    }
    for (name, payload) in corpus::accepted() {
        let capture = Arc::new(Capture::default());
        let recorder = OutboxRecorder::new(capture.clone());
        let mut e = envelope(Uuid::now_v7());
        e.payload = payload.clone();
        recorder
            .record(&e)
            .unwrap_or_else(|err| panic!("{name}: {err:?}"));
        assert_eq!(capture.0.lock().unwrap()[0].payload, payload, "{name}");
    }
}

#[test]
#[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
fn equivalence_corpus_gives_the_same_results_in_the_database() {
    for (first, second) in corpus::equivalent() {
        let db = TestDb::new();
        let id = Uuid::now_v7();
        let (mut a, mut b) = (envelope(id), envelope(id));
        (a.payload, b.payload) = (first.into(), second.into());
        Api::Port.record(&db, &[a]).unwrap();
        Api::Port
            .record(&db, &[b])
            .unwrap_or_else(|e| panic!("{first} ~ {second}: {e:?}"));
    }
    for (first, second) in corpus::different() {
        let db = TestDb::new();
        let id = Uuid::now_v7();
        let (mut a, mut b) = (envelope(id), envelope(id));
        (a.payload, b.payload) = (first.into(), second.into());
        Api::Port.record(&db, &[a]).unwrap();
        let err = Api::Port.record(&db, &[b]).expect_err("must conflict");
        assert!(
            matches!(
                err,
                RecordingError::ReplayConflict {
                    field: "payload",
                    ..
                }
            ),
            "{first} vs {second}: {err:?}"
        );
    }
}
