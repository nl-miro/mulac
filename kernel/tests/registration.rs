mod support;
use mulac_kernel::io::*;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use support::*;

#[tokio::test]
async fn direct_subscribers_receive_only_matching_events_and_command_outputs() {
    let received = Arc::new(Mutex::new(vec![]));
    let other = Arc::new(Mutex::new(vec![]));
    let kernel = boot(KernelConfig::default())
        .command_handler("start", Arc::new(StartHandler))
        .event_subscriber("started", "capture", Arc::new(Capture(received.clone())))
        .event_subscriber("other", "capture", Arc::new(Capture(other.clone())))
        .start()
        .await
        .unwrap();
    let state = kernel.state();
    state.dispatch_command(envelope(Start)).unwrap();
    let sent = event("started");
    state.event_gateway.dispatch(sent.clone()).unwrap();
    assert_eq!(received.lock().unwrap().len(), 2);
    assert_eq!(
        received.lock().unwrap()[0]
            .metadata
            .as_ref()
            .unwrap()
            .routing_key
            .as_deref(),
        Some("qa01.started")
    );
    assert_eq!(
        received.lock().unwrap()[1]
            .metadata
            .as_ref()
            .unwrap()
            .event_id,
        sent.metadata.unwrap().event_id
    );
    assert!(other.lock().unwrap().is_empty());
    state.event_gateway.dispatch(event("other")).unwrap();
    assert_eq!(other.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn direct_factory_runs_once_and_can_dispatch_follow_up_commands() {
    let calls = Arc::new(AtomicUsize::new(0));
    let factories = Arc::new(AtomicUsize::new(0));
    let count = factories.clone();
    let kernel = boot(KernelConfig::default())
        .command_handler("start", Arc::new(StartHandler))
        .command_handler("follow", Arc::new(FollowHandler(calls.clone())))
        .event_subscriber_with_command_gateway("started", "follow", move |gateway| {
            count.fetch_add(1, Ordering::SeqCst);
            Arc::new(FollowSubscriber(gateway))
        })
        .start()
        .await
        .unwrap();
    assert_eq!(factories.load(Ordering::SeqCst), 1);
    let state = kernel.state();
    let gateway = Arc::downgrade(&state.command_gateway);
    state.dispatch_command(envelope(Start)).unwrap();
    state.dispatch_command(envelope(Start)).unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(factories.load(Ordering::SeqCst), 1);
    drop(state);
    drop(kernel);
    assert!(
        gateway.upgrade().is_none(),
        "subscriber wiring must not create an Arc cycle"
    );
}

struct Fail;
impl EventSubscriberPort for Fail {
    fn handle(&self, _: &NewEventEnvelope) -> Result<(), EventError> {
        Err(EventError::SubscriberExecution("subscriber failed".into()))
    }
}
#[tokio::test]
async fn subscriber_and_follow_up_dispatch_failures_are_not_reported_as_success() {
    let kernel = boot(KernelConfig::default())
        .event_subscriber("failed", "fail", Arc::new(Fail))
        .event_subscriber_with_command_gateway("started", "follow", |gateway| {
            Arc::new(FollowSubscriber(gateway))
        })
        .start()
        .await
        .unwrap();
    assert!(
        matches!(kernel.state().event_gateway.dispatch(event("failed")),
        Err(EventError::SubscriberExecution(message)) if message == "subscriber failed")
    );
    assert!(
        matches!(kernel.state().event_gateway.dispatch(event("started")),
        Err(EventError::SubscriberExecution(message)) if message.contains("follow"))
    );
}

#[tokio::test]
async fn direct_outbox_is_explicitly_unsupported_before_factories_run() {
    let factories = Arc::new(AtomicUsize::new(0));
    let count = factories.clone();
    let result = boot(KernelConfig::default())
        .event_subscriber_with_command_gateway("started", "follow", move |gateway| {
            count.fetch_add(1, Ordering::SeqCst);
            Arc::new(FollowSubscriber(gateway))
        })
        .outbox_subscriber("started")
        .start()
        .await;
    assert!(matches!(result, Err(KernelError::Unsupported(_))));
    assert_eq!(factories.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn direct_mode_intentionally_ignores_persistent_configuration() {
    let kernel = boot(KernelConfig {
        database_url: Some("invalid".into()),
        amqp_url: Some("invalid".into()),
        inbox_queue: Some("queue".into()),
        inbox_exchange: None,
        outbox_exchange: Some("exchange".into()),
        outbox_content_type: Some("".into()),
    })
    .command_handler("start", Arc::new(StartHandler))
    .start()
    .await
    .unwrap();
    kernel.state().dispatch_command(envelope(Start)).unwrap();
}

#[tokio::test]
async fn duplicate_and_reserved_registrations_are_rejected() {
    assert!(matches!(boot(KernelConfig::default())
        .command_handler("start", Arc::new(StartHandler))
        .command_handler("start", Arc::new(StartHandler)).start().await,
        Err(KernelError::DuplicateCommandHandler(name)) if name == "start"));
    let received = Arc::new(Mutex::new(vec![]));
    assert!(matches!(
        boot(KernelConfig::default())
            .event_subscriber("started", "same", Arc::new(Capture(received.clone())))
            .event_subscriber_with_command_gateway("started", "same", |gateway| Arc::new(
                FollowSubscriber(gateway)
            ))
            .start()
            .await,
        Err(KernelError::DuplicateEventSubscriber { .. })
    ));
    assert!(matches!(
        boot(KernelConfig::default())
            .event_subscriber("started", "_outbox", Arc::new(Capture(received)))
            .start()
            .await,
        Err(KernelError::ReservedSubscriberName(_))
    ));
    assert!(matches!(
        boot(KernelConfig::default())
            .outbox_subscriber("started")
            .outbox_subscriber("started")
            .start()
            .await,
        Err(KernelError::ReservedSubscriberName(_))
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn persistent_configuration_errors_precede_storage_and_factory_side_effects() {
    for config in [
        KernelConfig::default(),
        KernelConfig {
            inbox_queue: Some("queue".into()),
            ..Default::default()
        },
        KernelConfig {
            inbox_exchange: Some("exchange".into()),
            ..Default::default()
        },
        KernelConfig {
            amqp_url: Some("amqp://unused".into()),
            ..Default::default()
        },
        KernelConfig {
            outbox_content_type: Some("application/json".into()),
            ..Default::default()
        },
        KernelConfig {
            database_url: Some(" ".into()),
            ..Default::default()
        },
    ] {
        let result = boot(config).start_persistent(None, 0);
        assert!(matches!(result, Err(KernelError::Config(_))));
    }
    for config in [
        KernelConfig::default(),
        KernelConfig {
            amqp_url: Some("amqp://unused".into()),
            ..Default::default()
        },
        KernelConfig {
            outbox_exchange: Some("events".into()),
            ..Default::default()
        },
    ] {
        let factories = Arc::new(AtomicUsize::new(0));
        let count = factories.clone();
        let result = boot(config)
            .outbox_subscriber("started")
            .event_subscriber_with_command_gateway("started", "follow", move |gateway| {
                count.fetch_add(1, Ordering::SeqCst);
                Arc::new(FollowSubscriber(gateway))
            })
            .start_persistent(None, 0);
        assert!(matches!(result, Err(KernelError::Config(_))));
        assert_eq!(factories.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn unsupported_persistent_runtime_has_an_explicit_outcome() {
    assert!(matches!(
        boot(KernelConfig {
            database_url: Some("unused".into()),
            ..Default::default()
        })
        .start_persistent(None, 0),
        Err(KernelError::Unsupported(_))
    ));
}
