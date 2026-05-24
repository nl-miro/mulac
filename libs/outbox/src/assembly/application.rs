//! Outbox application-layer envelopes, ports, and errors.
//!
//! Application types are added incrementally according to the implementation checklist.

pub mod io {
    pub use super::models::{
        NewOutboxEnvelope,
        NewOutboxMetadata,
        OutboundMessageEnvelope,
        OutboxEntryEnvelope,
        OutboxError,
        //
    };
    pub use super::ports::{
        OutboxProcessPort,
        OutboxPublisherPort,
        OutboxReservePort,
        OutboxStorePort,
        OutboxSweepPort,
        //
    };
}

mod models {
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct NewOutboxEnvelope {
        pub payload: String,
        pub metadata: NewOutboxMetadata,
    }

    #[derive(Debug, thiserror::Error)]
    pub enum OutboxError {
        #[error("storage error: {0}")]
        Storage(String),
        #[error("routing error: {0}")]
        Routing(String),
        #[error("serialization error: {0}")]
        Serialization(String),
        #[error("transport error: {0}")]
        Transport(String),
        #[error("reservation error: {0}")]
        Reservation(String),
        #[error("publish error: {0}")]
        Publish(String),
        #[error("missing reservation for entry {id}")]
        MissingReservation { id: uuid::Uuid },
        #[error("conversion error: {0}")]
        Conversion(String),
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct OutboundMessageEnvelope {
        pub payload: Vec<u8>,
        pub metadata: crate::assembly::domain::OutboxEntryMetadata,
    }

    #[derive(Debug, Clone, PartialEq)]
    pub struct OutboxEntryEnvelope {
        pub message: crate::assembly::domain::OutboxEntry,
        pub metadata: crate::assembly::domain::OutboxEntryMetadata,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct NewOutboxMetadata {
        pub event_id: uuid::Uuid,
        pub message_id: Option<uuid::Uuid>,
        pub correlation_id: Option<uuid::Uuid>,
        pub causation_id: Option<uuid::Uuid>,
        pub event_type: String,
        pub routing_key: String,
        pub source: Option<String>,
        pub content_type: Option<String>,
    }
}

mod conversions {
    use super::models::NewOutboxMetadata;
    use crate::assembly::domain::OutboxEntryMetadata;

    impl From<NewOutboxMetadata> for OutboxEntryMetadata {
        fn from(metadata: NewOutboxMetadata) -> Self {
            let message_id = metadata.message_id.unwrap_or(metadata.event_id);
            OutboxEntryMetadata {
                event_id: metadata.event_id,
                message_id,
                correlation_id: metadata.correlation_id,
                causation_id: metadata.causation_id,
                event_type: metadata.event_type,
                routing_key: metadata.routing_key,
                source: metadata.source,
                content_type: metadata.content_type,
            }
        }
    }
}

mod ports {
    use super::models::OutboxError;
    use crate::assembly::domain::NewOutboxEntry;

    pub trait OutboxStorePort: Send + Sync {
        fn record(&self, entry: &NewOutboxEntry) -> Result<(), OutboxError>;
    }

    pub trait OutboxReservePort: Send + Sync {
        fn reserve(
            &self,
            spec: &crate::outbox_consumer::io::ReservableOutboxSpec,
        ) -> Result<Vec<super::models::OutboxEntryEnvelope>, OutboxError>;
    }

    pub trait OutboxPublisherPort: Send + Sync {
        fn publish(
            &self,
            envelope: super::models::OutboundMessageEnvelope,
        ) -> Result<(), OutboxError>;
    }

    pub trait OutboxSweepPort: Send + Sync {
        fn sweep(
            &self,
            spec: &crate::stale_reservation_sweep::io::StaleReservationSpec,
        ) -> Result<u64, OutboxError>;
    }

    pub trait OutboxProcessPort: Send + Sync {
        fn completed(&self, id: uuid::Uuid, reservation_id: uuid::Uuid) -> Result<(), OutboxError>;

        fn failed(
            &self,
            id: uuid::Uuid,
            reservation_id: uuid::Uuid,
            max_attempts: i32,
            reason: Option<String>,
        ) -> Result<(), OutboxError>;

        fn dead(
            &self,
            id: uuid::Uuid,
            reservation_id: uuid::Uuid,
            reason: Option<String>,
        ) -> Result<(), OutboxError>;
    }
}

#[cfg(test)]
mod tests {
    use super::models::NewOutboxMetadata;
    use crate::assembly::domain::OutboxEntryMetadata;
    use uuid::Uuid;

    fn metadata(event_id: Uuid, message_id: Option<Uuid>) -> NewOutboxMetadata {
        NewOutboxMetadata {
            event_id,
            message_id,
            correlation_id: None,
            causation_id: None,
            event_type: "UserRegistered".into(),
            routing_key: "users.registered".into(),
            source: Some("identity-service".into()),
            content_type: Some("application/json".into()),
        }
    }

    #[test]
    fn missing_message_id_defaults_to_event_id() {
        let event_id = Uuid::now_v7();
        let converted = OutboxEntryMetadata::from(metadata(event_id, None));
        assert_eq!(converted.event_id, event_id);
        assert_eq!(converted.message_id, event_id);
    }

    #[test]
    fn supplied_message_id_is_preserved() {
        let event_id = Uuid::now_v7();
        let message_id = Uuid::now_v7();
        let converted = OutboxEntryMetadata::from(metadata(event_id, Some(message_id)));
        assert_eq!(converted.event_id, event_id);
        assert_eq!(converted.message_id, message_id);
    }
}
