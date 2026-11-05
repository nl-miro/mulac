pub mod io {
    #[cfg(feature = "diesel")]
    pub use super::postgres::record_bulk_on_connection;
    pub use super::recorder::OutboxRecorder;
    pub use super::repository::OutboxRecorderRepository;
    #[cfg(feature = "diesel")]
    pub(crate) use super::validation::validate_entries;
    #[cfg(feature = "diesel")]
    use crate::assembly::infra_diesel::io::{DbPool, OutboxStoreStorage};
    #[cfg(feature = "diesel")]
    use std::sync::Arc;
    #[cfg(feature = "diesel")]
    pub fn repository(pool: DbPool) -> Arc<OutboxRecorderRepository> {
        let store = Arc::new(OutboxStoreStorage::new(pool));
        Arc::new(OutboxRecorderRepository::new(store))
    }
}

mod repository {
    use crate::assembly::io::{NewOutboxEntry, OutboxError, OutboxStorePort};
    use std::sync::Arc;

    pub struct OutboxRecorderRepository {
        pub(super) store: Arc<dyn OutboxStorePort>,
    }

    impl OutboxRecorderRepository {
        pub fn new(store: Arc<dyn OutboxStorePort>) -> Self {
            Self { store }
        }

        /// Insert a batch using the storage adapter's bulk operation.
        /// Atomicity depends on the adapter; PostgreSQL inserts atomically.
        pub(super) fn insert_bulk(&self, entries: &[NewOutboxEntry]) -> Result<(), OutboxError> {
            if entries.is_empty() {
                return Ok(());
            }
            super::validation::validate_entries(entries)?;
            self.store.record_bulk(entries)
        }

        pub(super) fn insert(&self, entry: &NewOutboxEntry) -> Result<(), OutboxError> {
            super::validation::validate_entries(std::slice::from_ref(entry))?;
            self.store.record(entry)
        }
    }
}

mod recorder {
    use super::repository::OutboxRecorderRepository;
    use crate::assembly::io::{NewOutboxEntry, NewOutboxEnvelope, OutboxError};
    use std::sync::Arc;

    pub struct OutboxRecorder {
        repo: Arc<OutboxRecorderRepository>,
    }

    impl OutboxRecorder {
        pub fn new(repo: Arc<OutboxRecorderRepository>) -> Self {
            Self { repo }
        }

        /// Validate the entire batch before storing any entries.
        pub fn record_bulk(&self, envelopes: &[NewOutboxEnvelope]) -> Result<(), OutboxError> {
            if envelopes.is_empty() {
                return Ok(());
            }
            for envelope in envelopes {
                validate_routing_key(envelope)?;
            }
            let entries: Vec<_> = envelopes.iter().map(NewOutboxEntry::from).collect();
            self.repo.insert_bulk(&entries)
        }

        pub fn record(&self, envelope: &NewOutboxEnvelope) -> Result<(), OutboxError> {
            validate_routing_key(envelope)?;
            let entry = NewOutboxEntry::from(envelope);
            self.repo.insert(&entry)
        }
    }

    fn validate_routing_key(envelope: &NewOutboxEnvelope) -> Result<(), OutboxError> {
        if envelope
            .metadata
            .sequence
            .is_some_and(|value| value == 0 || value > i64::MAX as u64)
        {
            return Err(OutboxError::Routing(
                "sequence must be between 1 and i64::MAX".into(),
            ));
        }
        if envelope.metadata.routing_key.trim().is_empty() {
            return Err(OutboxError::Routing("routing_key is required".into()));
        }
        Ok(())
    }
}

mod conversions {
    use crate::assembly::io::{
        NewOutboxEntry,
        NewOutboxEnvelope,
        OutboxEntryMetadata,
        //
    };
    use chrono::Utc;

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
    use super::io::{OutboxRecorder, OutboxRecorderRepository};
    use crate::assembly::io::{
        NewOutboxEntry,
        NewOutboxEnvelope,
        NewOutboxMetadata,
        OutboxError,
        OutboxStorePort,
        //
    };
    use std::sync::{Arc, Mutex};
    use uuid::Uuid;

    #[derive(Default)]
    struct FakeStore {
        entries: Mutex<Vec<NewOutboxEntry>>,
    }

    impl OutboxStorePort for FakeStore {
        fn record_bulk(&self, entries: &[NewOutboxEntry]) -> Result<(), OutboxError> {
            self.entries.lock().unwrap().extend_from_slice(entries);
            Ok(())
        }

        fn record(&self, entry: &NewOutboxEntry) -> Result<(), OutboxError> {
            self.entries.lock().unwrap().push(entry.clone());
            Ok(())
        }
    }

    fn envelope(event_id: Uuid) -> NewOutboxEnvelope {
        NewOutboxEnvelope {
            payload: "{}".into(),
            metadata: NewOutboxMetadata {
                sequence: None,
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
    fn bulk_records_all_entries_and_accepts_empty_batch() {
        let store = Arc::new(FakeStore::default());
        let repo = Arc::new(OutboxRecorderRepository::new(store.clone()));
        let recorder = OutboxRecorder::new(repo);
        let ids = [Uuid::now_v7(), Uuid::now_v7()];
        recorder.record_bulk(&[]).unwrap();
        recorder.record_bulk(&ids.map(envelope)).unwrap();
        let entries = store.entries.lock().unwrap();
        assert_eq!(
            entries.iter().map(|entry| entry.id).collect::<Vec<_>>(),
            ids
        );
    }

    #[test]
    fn bulk_validates_all_envelopes_before_storage() {
        let store = Arc::new(FakeStore::default());
        let recorder = OutboxRecorder::new(Arc::new(OutboxRecorderRepository::new(store.clone())));
        let valid = envelope(Uuid::now_v7());
        let mut invalid = envelope(Uuid::now_v7());
        invalid.metadata.routing_key = " ".into();
        assert!(matches!(
            recorder.record_bulk(&[valid, invalid]),
            Err(OutboxError::Routing(_))
        ));
        assert!(store.entries.lock().unwrap().is_empty());
    }

    #[test]
    fn rejects_duplicates_and_non_object_payloads_before_storage() {
        let store = Arc::new(FakeStore::default());
        let recorder = OutboxRecorder::new(Arc::new(OutboxRecorderRepository::new(store.clone())));
        let e = envelope(Uuid::now_v7());
        assert!(matches!(
            recorder.record_bulk(&[e.clone(), e.clone()]),
            Err(OutboxError::Duplicate { .. })
        ));
        for payload in ["invalid", "[]", "null", "1", "\"text\""] {
            let mut invalid = e.clone();
            invalid.payload = payload.into();
            assert!(matches!(
                recorder.record(&invalid),
                Err(OutboxError::InvalidPayload { .. })
            ));
            assert!(matches!(
                recorder.record_bulk(&[invalid]),
                Err(OutboxError::InvalidPayload { .. })
            ));
        }
        assert!(store.entries.lock().unwrap().is_empty());
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

mod validation {
    use crate::assembly::io::{NewOutboxEntry, OutboxError};

    pub(crate) fn validate_entries(entries: &[NewOutboxEntry]) -> Result<(), OutboxError> {
        let mut sorted: Vec<_> = entries.iter().collect();
        sorted.sort_by_key(|entry| entry.id);
        for pair in sorted.windows(2) {
            if pair[0].id == pair[1].id {
                return Err(OutboxError::Duplicate { id: pair[0].id });
            }
        }
        for entry in sorted {
            if entry.meta.routing_key.trim().is_empty() {
                return Err(OutboxError::Routing("routing_key is required".into()));
            }
            if entry
                .meta
                .sequence
                .is_some_and(|v| v == 0 || v > i64::MAX as u64)
            {
                return Err(OutboxError::Routing(
                    "sequence must be between 1 and i64::MAX".into(),
                ));
            }
            if !matches!(
                serde_json::from_str::<serde_json::Value>(&entry.payload),
                Ok(serde_json::Value::Object(_))
            ) {
                return Err(OutboxError::InvalidPayload { id: entry.id });
            }
        }
        Ok(())
    }
}

#[cfg(feature = "diesel")]
mod postgres {
    use crate::assembly::io::{NewOutboxEntry, NewOutboxEnvelope, OutboxError};

    /// Record atomically on a caller-owned connection (a savepoint in an outer transaction).
    pub fn record_bulk_on_connection(
        conn: &mut diesel::PgConnection,
        envelopes: &[NewOutboxEnvelope],
    ) -> Result<(), OutboxError> {
        let entries: Vec<_> = envelopes.iter().map(NewOutboxEntry::from).collect();
        crate::assembly::io::record_entries_on_connection(conn, &entries)
    }
}
