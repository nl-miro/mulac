pub mod io {
    #[cfg(feature = "diesel")]
    use std::sync::Arc;

    #[cfg(feature = "diesel")]
    use crate::assembly::infra_diesel::io::{DbPool, OutboxStoreStorage};

    pub use super::recorder::OutboxRecorder;
    pub use super::repository::OutboxRecorderRepository;

    #[cfg(feature = "diesel")]
    pub fn repository(pool: DbPool) -> Arc<OutboxRecorderRepository> {
        let store = Arc::new(OutboxStoreStorage::new(pool));
        Arc::new(OutboxRecorderRepository::new(store))
    }
}

mod repository {
    use std::sync::Arc;

    use crate::assembly::io::{NewOutboxEntry, OutboxError, OutboxStorePort};

    pub struct OutboxRecorderRepository {
        pub(super) store: Arc<dyn OutboxStorePort>,
    }

    impl OutboxRecorderRepository {
        pub fn new(store: Arc<dyn OutboxStorePort>) -> Self {
            Self { store }
        }

        pub(super) fn insert(&self, entry: &NewOutboxEntry) -> Result<(), OutboxError> {
            self.store.record(entry)
        }
    }
}

mod recorder {
    use std::sync::Arc;

    use crate::assembly::io::{NewOutboxEntry, NewOutboxEnvelope, OutboxError};

    use super::repository::OutboxRecorderRepository;

    pub struct OutboxRecorder {
        repo: Arc<OutboxRecorderRepository>,
    }

    impl OutboxRecorder {
        pub fn new(repo: Arc<OutboxRecorderRepository>) -> Self {
            Self { repo }
        }

        pub fn record(&self, envelope: &NewOutboxEnvelope) -> Result<(), OutboxError> {
            validate_routing_key(envelope)?;
            let entry = NewOutboxEntry::from(envelope);
            self.repo.insert(&entry)
        }
    }

    fn validate_routing_key(envelope: &NewOutboxEnvelope) -> Result<(), OutboxError> {
        if envelope.metadata.routing_key.trim().is_empty() {
            return Err(OutboxError::Routing("routing_key is required".into()));
        }
        Ok(())
    }
}

mod conversions {
    use chrono::Utc;

    use crate::assembly::io::{NewOutboxEntry, NewOutboxEnvelope, OutboxEntryMetadata};

    impl From<&NewOutboxEnvelope> for NewOutboxEntry {
        fn from(envelope: &NewOutboxEnvelope) -> Self {
            let now = Utc::now();
            let meta = OutboxEntryMetadata::from(envelope.metadata.clone());
            NewOutboxEntry {
                id: meta.event_id,
                payload: envelope.payload.clone(),
                meta,
                scheduled_at: now,
                received_at: now,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use uuid::Uuid;

    use crate::assembly::io::{
        NewOutboxEntry, NewOutboxEnvelope, NewOutboxMetadata, OutboxError, OutboxStorePort,
    };

    use super::io::{OutboxRecorder, OutboxRecorderRepository};

    #[derive(Default)]
    struct FakeStore {
        entries: Mutex<Vec<NewOutboxEntry>>,
    }

    impl OutboxStorePort for FakeStore {
        fn record(&self, entry: &NewOutboxEntry) -> Result<(), OutboxError> {
            self.entries.lock().unwrap().push(entry.clone());
            Ok(())
        }
    }

    fn envelope(event_id: Uuid) -> NewOutboxEnvelope {
        NewOutboxEnvelope {
            payload: "{}".into(),
            metadata: NewOutboxMetadata {
                event_id,
                message_id: None,
                correlation_id: None,
                causation_id: None,
                event_type: "UserRegistered".into(),
                routing_key: "users.registered".into(),
                source: None,
                content_type: Some("application/json".into()),
            },
        }
    }

    #[test]
    fn conversion_uses_event_id_as_entry_id() {
        let event_id = Uuid::now_v7();
        let entry = NewOutboxEntry::from(&envelope(event_id));
        assert_eq!(entry.id, event_id);
    }

    #[test]
    fn record_stores_valid_envelope() {
        let store = Arc::new(FakeStore::default());
        let repo = Arc::new(OutboxRecorderRepository::new(store.clone()));
        let recorder = OutboxRecorder::new(repo);
        let event_id = Uuid::now_v7();

        recorder
            .record(&envelope(event_id))
            .expect("record succeeds");

        let entries = store.entries.lock().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, event_id);
    }

    #[test]
    fn record_rejects_blank_routing_key_before_storage() {
        let store = Arc::new(FakeStore::default());
        let repo = Arc::new(OutboxRecorderRepository::new(store.clone()));
        let recorder = OutboxRecorder::new(repo);
        let mut envelope = envelope(Uuid::now_v7());
        envelope.metadata.routing_key = "  ".into();

        let err = recorder
            .record(&envelope)
            .expect_err("routing validation fails");

        assert!(
            matches!(err, OutboxError::Routing(message) if message == "routing_key is required")
        );
        assert!(store.entries.lock().unwrap().is_empty());
    }

    #[test]
    fn record_preserves_event_id_in_stored_entry() {
        let store = Arc::new(FakeStore::default());
        let repo = Arc::new(OutboxRecorderRepository::new(store.clone()));
        let recorder = OutboxRecorder::new(repo);
        let event_id = Uuid::now_v7();
        let mut envelope = envelope(event_id);
        envelope.metadata.message_id = Some(Uuid::now_v7());

        recorder.record(&envelope).expect("record succeeds");

        let entries = store.entries.lock().unwrap();
        assert_eq!(entries[0].id, event_id);
        assert_eq!(entries[0].meta.event_id, event_id);
    }
}
