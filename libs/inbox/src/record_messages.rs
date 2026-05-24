pub mod io {
    pub use super::inbox_repository::InboxRecorderRepository;
    pub use super::models::NewInboxMessageEnvelope;
    pub use super::ports::InboxStorePort;
    pub use super::recorder::InboxRecorder;
    #[cfg(feature = "diesel")]
    use crate::io::{DbPool, InboxStoreStorage};
    #[cfg(feature = "diesel")]
    use std::sync::Arc;

    #[cfg(feature = "diesel")]
    pub fn repository(pool: DbPool) -> Arc<InboxRecorderRepository> {
        let store = Arc::new(InboxStoreStorage::new(pool));
        let repository = InboxRecorderRepository::new(store);

        Arc::new(repository)
    }
}

mod models {
    use uuid::Uuid;

    /// Raw payload of a new inbound message before it is stored.
    #[derive(Debug)]
    pub struct NewInboxMessage {
        pub payload: String,
    }

    /// Write-side envelope wrapping a new message and its routing metadata.
    ///
    /// Constructed from an [`InboundMessageEnvelope`] arriving from a transport
    /// adapter (e.g. AMQP). If the transport did not supply a `message_id`, one
    /// is generated as a time-ordered UUID v7 to ensure idempotent inserts.
    pub struct NewInboxMessageEnvelope {
        pub msg: NewInboxMessage,
        pub meta: NewInboxMessageMetadata,
    }

    /// Metadata carried with a new inbound message.
    ///
    /// `message_id` is always present — either forwarded from the transport or
    /// generated at the point of conversion.
    #[derive(Debug, Clone)]
    pub struct NewInboxMessageMetadata {
        pub message_id: Uuid,
        pub correlation_id: Option<Uuid>,
        pub source: Option<String>,
        pub routing_key: Option<String>,
    }

    impl NewInboxMessageEnvelope {
        pub fn id(&self) -> &Uuid {
            &self.meta.message_id
        }

        pub fn payload(&self) -> &str {
            &self.msg.payload
        }
    }
}

mod conversions {
    use super::models::{NewInboxMessage, NewInboxMessageEnvelope, NewInboxMessageMetadata};
    use crate::assembly::io::{InboundMessageEnvelope, InboxMessageMetadata};
    use uuid::Uuid;

    impl From<NewInboxMessageMetadata> for InboxMessageMetadata {
        fn from(message: NewInboxMessageMetadata) -> Self {
            InboxMessageMetadata {
                message_id: Some(message.message_id),
                correlation_id: message.correlation_id,
                source: message.source,
                routing_key: message.routing_key,
            }
        }
    }

    impl From<InboundMessageEnvelope> for NewInboxMessageEnvelope {
        /// Convert an inbound transport envelope into a write-side envelope.
        ///
        /// If the transport did not provide a `message_id`, a new UUID v7 is
        /// generated. All other metadata fields are passed through unchanged.
        fn from(msg: InboundMessageEnvelope) -> Self {
            let meta = NewInboxMessageMetadata {
                message_id: msg.message_id.unwrap_or_else(Uuid::now_v7),
                correlation_id: msg.correlation_id,
                source: msg.source,
                routing_key: msg.routing_key,
            };

            NewInboxMessageEnvelope {
                msg: NewInboxMessage {
                    payload: msg.payload,
                },
                meta,
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::super::models::{
            NewInboxMessageEnvelope,
            NewInboxMessageMetadata, //
        };
        use crate::assembly::io::{InboundMessageEnvelope, InboxMessageMetadata};
        use uuid::Uuid;

        fn inbound(
            payload: &str,
            message_id: Option<Uuid>,
            correlation_id: Option<Uuid>,
            source: Option<&str>,
            routing_key: Option<&str>,
        ) -> InboundMessageEnvelope {
            InboundMessageEnvelope {
                payload: payload.into(),
                message_id,
                correlation_id,
                source: source.map(Into::into),
                routing_key: routing_key.map(Into::into),
            }
        }

        #[test]
        fn preserves_existing_message_id() {
            let id = Uuid::new_v4();
            let envelope =
                NewInboxMessageEnvelope::from(inbound("hello", Some(id), None, None, None));
            assert_eq!(*envelope.id(), id);
        }

        #[test]
        fn generates_message_id_when_absent() {
            let envelope = NewInboxMessageEnvelope::from(inbound("hello", None, None, None, None));
            assert_ne!(*envelope.id(), Uuid::nil());
        }

        #[test]
        fn two_conversions_without_id_produce_distinct_ids() {
            let a = NewInboxMessageEnvelope::from(inbound("x", None, None, None, None));
            let b = NewInboxMessageEnvelope::from(inbound("x", None, None, None, None));
            assert_ne!(a.id(), b.id());
        }

        #[test]
        fn preserves_payload() {
            let envelope =
                NewInboxMessageEnvelope::from(inbound("my-payload", None, None, None, None));
            assert_eq!(envelope.payload(), "my-payload");
        }

        #[test]
        fn preserves_metadata_fields() {
            let corr = Uuid::new_v4();
            let envelope = NewInboxMessageEnvelope::from(inbound(
                "data",
                None,
                Some(corr),
                Some("payments"),
                Some("user.created"),
            ));
            assert_eq!(envelope.meta.correlation_id, Some(corr));
            assert_eq!(envelope.meta.source.as_deref(), Some("payments"));
            assert_eq!(envelope.meta.routing_key.as_deref(), Some("user.created"));
        }

        #[test]
        fn metadata_conversion_wraps_message_id_in_some() {
            let id = Uuid::new_v4();
            let meta = NewInboxMessageMetadata {
                message_id: id,
                correlation_id: None,
                source: None,
                routing_key: None,
            };
            let converted = InboxMessageMetadata::from(meta);
            assert_eq!(converted.message_id, Some(id));
        }
    }
}

mod ports {
    use super::models::NewInboxMessageEnvelope;
    use crate::assembly::io::InboxError;

    pub trait InboxStorePort: Send + Sync {
        fn store(&self, msg: NewInboxMessageEnvelope) -> Result<(), InboxError>;
    }
}

mod inbox_repository {
    use super::models::NewInboxMessageEnvelope;
    use super::ports::InboxStorePort;
    use crate::assembly::io::InboxError;
    use std::sync::Arc;

    #[derive(Clone)]
    pub struct InboxRecorderRepository {
        store: Arc<dyn InboxStorePort>,
    }

    impl InboxRecorderRepository {
        pub fn new(store: Arc<dyn InboxStorePort>) -> Self {
            Self { store }
        }

        pub fn insert(&self, msg: NewInboxMessageEnvelope) -> Result<(), InboxError> {
            self.store.store(msg)
        }
    }
}

mod recorder {
    use super::inbox_repository::InboxRecorderRepository;
    use super::models::NewInboxMessageEnvelope;
    use crate::assembly::io::InboxError;
    use std::sync::Arc;

    #[derive(Clone)]
    pub struct InboxRecorder {
        repo: Arc<InboxRecorderRepository>,
    }

    impl InboxRecorder {
        pub fn new(repo: Arc<InboxRecorderRepository>) -> Self {
            Self { repo }
        }

        pub fn publish(&self, msg: NewInboxMessageEnvelope) -> Result<(), InboxError> {
            self.repo.insert(msg)?;
            Ok(())
        }
    }
}

#[cfg(feature = "diesel")]
mod infra_diesel_pg {
    use super::io::{InboxStorePort, NewInboxMessageEnvelope};
    use crate::assembly::io::InboxError;
    use crate::assembly::io::InboxStoreStorage;
    use crate::assembly::io::NewInboxEntry;
    use crate::assembly::io::inbox_entries;
    use diesel::prelude::*;

    impl InboxStorePort for InboxStoreStorage {
        fn store(&self, msg: NewInboxMessageEnvelope) -> Result<(), InboxError> {
            let mut conn = self
                .pool
                .get()
                .map_err(|e| InboxError::Storage(e.to_string()))?;
            let entry = NewInboxEntry::from(msg);
            diesel::insert_into(inbox_entries::table)
                .values(&entry)
                .on_conflict(inbox_entries::id)
                .do_nothing()
                .execute(&mut conn)
                .map_err(|e| InboxError::Storage(e.to_string()))?;

            Ok(())
        }
    }
}
