pub mod io {
    pub use super::models::{
        EventEnvelope,
        EventError,
        EventMetadata,
        NewEventEnvelope,
        NewEventMetadata, //
    };
    pub use super::ports::{EventDispatchPort, EventProcessPort, EventStorePort};
}

mod models {
    use uuid::Uuid;

    /// Gateway input envelope. The caller must supply `event_id` inside metadata;
    /// no ID generation occurs inside the system boundary.
    #[derive(Debug, Clone)]
    pub struct NewEventEnvelope {
        pub event_type: String,
        pub payload: String,
        pub metadata: Option<NewEventMetadata>,
    }

    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
    pub struct NewEventMetadata {
        pub event_id: Uuid,
        pub correlation_id: Option<Uuid>,
        pub causation_id: Option<Uuid>,
        pub source: Option<String>,
    }

    /// Read-side envelope returned to the consumer after reservation.
    ///
    /// Constructed by the infra layer from the stored entry; contains all fields
    /// the consumer needs without exposing infra types.
    #[derive(Debug)]
    pub struct EventEnvelope {
        pub id: Uuid,
        pub reservation_id: Uuid,
        pub event_type: String,
        pub payload: String,
        pub attempts: i32,
        pub metadata: Option<EventMetadata>,
    }

    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
    pub struct EventMetadata {
        pub event_id: Uuid,
        pub correlation_id: Option<Uuid>,
        pub causation_id: Option<Uuid>,
        pub source: Option<String>,
    }

    #[derive(Debug, thiserror::Error)]
    pub enum EventError {
        #[error("storage error: {0}")]
        Storage(String),
        #[error("reservation error: {0}")]
        Reservation(String),
        #[error("no subscriber registered for event type '{0}'")]
        SubscriberNotFound(String),
        #[error("subscriber execution error: {0}")]
        SubscriberExecution(String),
        #[error("missing reservation for entry {id}")]
        MissingReservation { id: Uuid },
        #[error("conversion error: {0}")]
        Conversion(String),
    }
}

mod ports {
    use uuid::Uuid;

    use crate::commanding::assembly::io::CommandError;

    use super::models::{EventError, NewEventEnvelope};

    pub trait EventDispatchPort: Send + Sync {
        fn dispatch(&self, event: NewEventEnvelope) -> Result<(), CommandError>;
    }

    pub trait EventStorePort: Send + Sync {
        fn record(&self, envelope: &NewEventEnvelope) -> Result<(), EventError>;
    }

    pub trait EventProcessPort: Send + Sync {
        fn completed(&self, id: Uuid, reservation_id: Uuid) -> Result<(), EventError>;

        fn failed(
            &self,
            id: Uuid,
            reservation_id: Uuid,
            max_attempts: i32,
        ) -> Result<(), EventError>;
    }
}
