use crate::assembly::{EventSubscriberRegistration, OUTBOX_SUBSCRIBER};
use commanding::io::CommandGateway;
use eventing::io::{EventError, EventSubscriberPort, NewEventEnvelope};
use outbox::io::{NewOutboxEnvelope, NewOutboxMetadata, OutboxRecorder};
use std::sync::Arc;

pub(crate) fn resolve(
    registrations: Vec<EventSubscriberRegistration>,
    gateway: Arc<CommandGateway>,
) -> Vec<(String, String, Arc<dyn EventSubscriberPort>)> {
    registrations
        .into_iter()
        .map(|registration| match registration {
            EventSubscriberRegistration::Direct {
                event_type,
                subscriber_name,
                subscriber,
            } => (event_type, subscriber_name, subscriber),
            EventSubscriberRegistration::WithCommandGateway {
                event_type,
                subscriber_name,
                factory,
            } => (event_type, subscriber_name, factory(gateway.clone())),
        })
        .collect()
}

pub(crate) struct OutboxSubscriber {
    recorder: Arc<OutboxRecorder>,
    content_type: Option<String>,
}

impl OutboxSubscriber {
    pub(crate) fn new(recorder: Arc<OutboxRecorder>, content_type: Option<String>) -> Self {
        Self {
            recorder,
            content_type,
        }
    }
}

impl EventSubscriberPort for OutboxSubscriber {
    fn handle(&self, envelope: &NewEventEnvelope) -> Result<(), EventError> {
        let metadata = envelope.metadata.as_ref().ok_or_else(|| {
            EventError::Conversion(format!("{OUTBOX_SUBSCRIBER} requires event metadata"))
        })?;
        let routing_key = metadata
            .routing_key
            .as_ref()
            .filter(|key| !key.trim().is_empty())
            .ok_or_else(|| {
                EventError::Conversion(format!("{OUTBOX_SUBSCRIBER} requires routing_key"))
            })?;
        self.recorder
            .record(&NewOutboxEnvelope {
                payload: envelope.payload.clone(),
                metadata: NewOutboxMetadata {
                    event_id: metadata.event_id,
                    message_id: Some(metadata.event_id),
                    correlation_id: metadata.correlation_id,
                    causation_id: metadata.causation_id,
                    event_type: envelope.event_type.clone(),
                    routing_key: routing_key.clone(),
                    source: metadata.source.clone(),
                    content_type: self.content_type.clone(),
                },
            })
            .map_err(|error| EventError::SubscriberExecution(error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use outbox::io::{NewOutboxEntry, OutboxError, OutboxRecorderRepository, OutboxStorePort};
    use std::sync::Mutex;
    use uuid::Uuid;

    #[derive(Default)]
    struct Store {
        entries: Mutex<Vec<NewOutboxEntry>>,
        fail: bool,
    }
    impl OutboxStorePort for Store {
        fn record(&self, entry: &NewOutboxEntry) -> Result<(), OutboxError> {
            if self.fail {
                return Err(OutboxError::Storage("database unavailable".into()));
            }
            self.entries.lock().unwrap().push(entry.clone());
            Ok(())
        }
    }
    fn subscriber(store: Arc<Store>) -> OutboxSubscriber {
        OutboxSubscriber::new(
            Arc::new(OutboxRecorder::new(Arc::new(
                OutboxRecorderRepository::new(store),
            ))),
            Some("application/cloudevents+json".into()),
        )
    }
    fn event() -> NewEventEnvelope {
        NewEventEnvelope {
            event_type: "Started".into(),
            payload: "{\"value\":42}".into(),
            metadata: Some(eventing::io::NewEventMetadata {
                event_id: Uuid::now_v7(),
                correlation_id: Some(Uuid::now_v7()),
                causation_id: Some(Uuid::now_v7()),
                source: Some("qa01".into()),
                routing_key: Some("events.started".into()),
            }),
        }
    }
    #[test]
    fn outbox_acceptance_records_real_work_with_identity_routing_and_configured_content_type() {
        let store = Arc::new(Store::default());
        let subscriber = subscriber(store.clone());
        let event = event();
        subscriber.handle(&event).unwrap();
        let entries = store.entries.lock().unwrap();
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        let metadata = event.metadata.unwrap();
        assert_eq!(entry.id, metadata.event_id);
        assert_eq!(entry.meta.event_id, metadata.event_id);
        assert_eq!(entry.meta.message_id, metadata.event_id);
        assert_eq!(entry.meta.correlation_id, metadata.correlation_id);
        assert_eq!(entry.meta.causation_id, metadata.causation_id);
        assert_eq!(entry.meta.source, metadata.source);
        assert_eq!(entry.meta.routing_key, "events.started");
        assert_eq!(entry.meta.event_type, "Started");
        assert_eq!(
            entry.meta.content_type.as_deref(),
            Some("application/cloudevents+json")
        );
        assert_eq!(entry.payload, event.payload);
    }
    #[test]
    fn missing_metadata_or_routing_never_reports_outbox_acceptance() {
        let store = Arc::new(Store::default());
        let subscriber = subscriber(store.clone());
        let mut event = event();
        for routing_key in [None, Some("".into()), Some(" \t".into())] {
            event.metadata.as_mut().unwrap().routing_key = routing_key;
            assert!(matches!(
                subscriber.handle(&event),
                Err(EventError::Conversion(_))
            ));
        }
        event.metadata = None;
        assert!(matches!(
            subscriber.handle(&event),
            Err(EventError::Conversion(_))
        ));
        assert!(store.entries.lock().unwrap().is_empty());
    }
    #[test]
    fn outbox_storage_failure_is_a_subscriber_failure() {
        let store = Arc::new(Store {
            fail: true,
            ..Default::default()
        });
        assert!(matches!(subscriber(store.clone()).handle(&event()),
            Err(EventError::SubscriberExecution(message)) if message.contains("database unavailable")));
        assert!(store.entries.lock().unwrap().is_empty());
    }
}
