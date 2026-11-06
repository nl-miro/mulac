//! What an event subscriber that records outbox events can expect when the
//! recording hits a permanent replay conflict, under TwoPhased and Direct
//! eventing. The subscriber maps the recording error explicitly; the kernel and
//! the eventing library are used exactly as they are.
#[path = "../../../libs/outbox/tests/common/mod.rs"]
mod common;

use common::TestDb;
use diesel::prelude::*;
use mulac_eventing::prelude::*;
use mulac_kernel::prelude::EventSubscriberRegistry;
use mulac_outbox::prelude::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use uuid::Uuid;

const EVENT_TYPE: &str = "Thing.Happened";
const EVENT_ENTRIES: &str = "CREATE TABLE event_entries (
    id uuid PRIMARY KEY, event_type text NOT NULL, status integer NOT NULL DEFAULT 0,
    payload text NOT NULL, meta jsonb, scheduled_at timestamptz NOT NULL,
    attempts integer NOT NULL DEFAULT 0, reservation_id uuid, reserved_at timestamptz,
    received_at timestamptz NOT NULL, updated_at timestamptz NOT NULL DEFAULT now(),
    processed_at timestamptz, extra_info jsonb)";
const FAILED: i32 = 4;

/// Counts how often it ran.
#[derive(Default)]
struct Counting(AtomicUsize);

impl EventSubscriberPort for Counting {
    fn handle(&self, _: &NewEventEnvelope) -> Result<(), EventError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// Records the delivered event into the outbox, mapping a recording failure
/// explicitly into the subscriber error.
struct Recording {
    recorder: OutboxRecorder,
    runs: AtomicUsize,
}

impl EventSubscriberPort for Recording {
    fn handle(&self, envelope: &NewEventEnvelope) -> Result<(), EventError> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        let event_id = envelope.metadata.as_ref().expect("event id").event_id;
        let outbox_event = NewOutboxEnvelope {
            payload: envelope.payload.clone(),
            metadata: NewOutboxMetadata {
                sequence: None,
                event_id,
                message_id: None,
                correlation_id: None,
                causation_id: None,
                event_type: envelope.event_type.clone(),
                routing_key: "things.happened".into(),
                source: None,
                content_type: Some("application/json".into()),
            },
        };
        self.recorder
            .record(&outbox_event)
            .map_err(|error| EventError::SubscriberExecution(format!("outbox recording: {error}")))
    }
}

struct World {
    db: TestDb,
    earlier: Arc<Counting>,
    recording: Arc<Recording>,
    later: Arc<Counting>,
    registry: Arc<EventSubscriberRegistry>,
    event: NewEventEnvelope,
}

/// Three subscribers in order (earlier, recording, later) and an event whose
/// outbox row already exists with different content: a permanent conflict.
fn world() -> World {
    let db = TestDb::new();
    db.exec(EVENT_ENTRIES);
    let event_id = Uuid::now_v7();
    let payload = r#"{"regenerated_at":"2026-11-06T12:00:01Z"}"#;
    // The earlier attempt recorded different content under the same event ID.
    let mut stored = common::envelope(event_id);
    stored.payload = r#"{"regenerated_at":"2026-11-06T12:00:00Z"}"#.into();
    OutboxRecorder::new(Arc::new(db.repo()))
        .record(&stored)
        .unwrap();

    let earlier = Arc::new(Counting::default());
    let later = Arc::new(Counting::default());
    let recording = Arc::new(Recording {
        recorder: OutboxRecorder::new(Arc::new(db.repo())),
        runs: AtomicUsize::new(0),
    });
    let registry = Arc::new(EventSubscriberRegistry::from_subscribers(vec![
        (EVENT_TYPE.into(), "earlier".into(), earlier.clone()),
        (EVENT_TYPE.into(), "recording".into(), recording.clone()),
        (EVENT_TYPE.into(), "later".into(), later.clone()),
    ]));
    let event = NewEventEnvelope::new(
        EVENT_TYPE.into(),
        payload.into(),
        Some(NewEventMetadata {
            sequence: None,
            event_id,
            correlation_id: None,
            causation_id: None,
            source: None,
        }),
    );
    World {
        db,
        earlier,
        recording,
        later,
        registry,
        event,
    }
}

fn event_status(db: &TestDb) -> (i32, i32) {
    #[derive(QueryableByName)]
    struct S {
        #[diesel(sql_type = diesel::sql_types::Int4)]
        status: i32,
        #[diesel(sql_type = diesel::sql_types::Int4)]
        attempts: i32,
    }
    let row = diesel::sql_query("SELECT status, attempts FROM event_entries")
        .get_result::<S>(&mut db.conn())
        .unwrap();
    (row.status, row.attempts)
}

#[test]
#[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
fn twophased_conflict_repeats_earlier_subscribers_skips_later_ones_and_leaves_the_event_failed() {
    const MAX_ATTEMPTS: i32 = 3;
    let w = world();
    let events = Arc::new(DieselPgEventRepository::new(w.db.pool()));
    events
        .record(&w.event)
        .expect("event stored for later consumption");
    let consumer = EventConsumer::new(events, Arc::new(EventDispatcher::new(w.registry.clone())));
    let spec = ReservableEventSpec::new(10).with_max_attempts(MAX_ATTEMPTS);

    for round in 1..=MAX_ATTEMPTS {
        // Make the backoff elapse so the retry is eligible.
        w.db.exec("UPDATE event_entries SET scheduled_at = now() - interval '1 second'");
        let errors = consumer
            .consume(&spec)
            .expect_err("the conflict fails the event");
        assert_eq!(errors.len(), 1, "round {round}: {errors:?}");
        assert!(
            matches!(&errors[0], EventError::SubscriberExecution(m) if m.contains("replay conflict")),
            "round {round}: {errors:?}"
        );
        assert_eq!(
            w.earlier.0.load(Ordering::SeqCst),
            round as usize,
            "earlier subscribers re-run"
        );
        assert_eq!(w.recording.runs.load(Ordering::SeqCst), round as usize);
        assert_eq!(
            w.later.0.load(Ordering::SeqCst),
            0,
            "later subscribers never run"
        );
    }

    // Exhausted: nothing is reserved any more and the event stays Failed.
    w.db.exec("UPDATE event_entries SET scheduled_at = now() - interval '1 second'");
    assert_eq!(consumer.consume(&spec).unwrap_or(0), 0);
    assert_eq!(w.earlier.0.load(Ordering::SeqCst), MAX_ATTEMPTS as usize);
    assert_eq!(event_status(&w.db), (FAILED, MAX_ATTEMPTS));

    // The conflicting outbox row was never touched and nothing was added.
    assert_eq!(w.db.count(), 1);
    let kept = common::stored(
        &mut w.db.conn(),
        w.event.metadata.as_ref().unwrap().event_id,
    )
    .unwrap();
    assert!(kept.payload.contains("12:00:00Z"));
}

#[test]
#[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
fn direct_dispatch_returns_the_mapped_error_without_automatic_retry() {
    let w = world();
    let gateway = EventGateway::direct(Arc::new(EventDispatcher::new(w.registry.clone())));

    let error = gateway
        .dispatch(w.event.clone())
        .expect_err("conflict surfaces to the caller");
    assert!(
        matches!(&error, EventError::SubscriberExecution(m) if m.contains("replay conflict")),
        "{error:?}"
    );
    assert_eq!(w.earlier.0.load(Ordering::SeqCst), 1);
    assert_eq!(w.recording.runs.load(Ordering::SeqCst), 1);
    assert_eq!(
        w.later.0.load(Ordering::SeqCst),
        0,
        "later subscribers never run"
    );

    // No automatic retry follows, and nothing was stored for one.
    std::thread::sleep(std::time::Duration::from_millis(500));
    assert_eq!(w.earlier.0.load(Ordering::SeqCst), 1);
    assert_eq!(w.recording.runs.load(Ordering::SeqCst), 1);
    assert_eq!(event_count(&w.db), 0);
    assert_eq!(w.db.count(), 1);
}

fn event_count(db: &TestDb) -> i64 {
    #[derive(QueryableByName)]
    struct C {
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        count: i64,
    }
    diesel::sql_query("SELECT count(*) AS count FROM event_entries")
        .get_result::<C>(&mut db.conn())
        .unwrap()
        .count
}

/// Control: with no conflict every subscriber runs and a redelivery of the same
/// event replays cleanly instead of adding another outbox row.
#[test]
#[ignore = "requires OUTBOX_TEST_DATABASE_URL"]
fn a_conflict_free_event_records_once_and_replays_cleanly_on_redelivery() {
    let w = world();
    let mut fresh = w.event.clone();
    fresh.metadata.as_mut().unwrap().event_id = Uuid::now_v7();
    let gateway = EventGateway::direct(Arc::new(EventDispatcher::new(w.registry.clone())));
    gateway
        .dispatch(fresh.clone())
        .expect("first delivery records");
    gateway
        .dispatch(fresh)
        .expect("redelivery of the same event replays cleanly");
    assert_eq!(w.db.count(), 2);
    assert_eq!(w.later.0.load(Ordering::SeqCst), 2);
}
