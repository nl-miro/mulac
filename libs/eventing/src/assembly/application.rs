pub mod io {
    pub use super::models::{
        EventEnvelope,
        EventError,
        EventMetadata,
        NewEventEnvelope,
        NewEventMetadata,
        //
    };
    pub use super::ports::{EventDispatchPort, EventProcessPort, EventStorePort};
}

mod models {

    fn sequence<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<u64>, D::Error> {
        let value = <Option<u64> as serde::Deserialize>::deserialize(deserializer)?;
        if value.is_some_and(|sequence| sequence == 0 || sequence > i64::MAX as u64) {
            return Err(serde::de::Error::custom(
                "sequence must be between 1 and i64::MAX",
            ));
        }
        Ok(value)
    }

    use crate::assembly::domain::ExtraInfo;
    use uuid::Uuid;

    /// Event envelope produced by a command handler and forwarded to the event
    /// dispatch port. Defined here (in commanding) because both commanding and
    /// eventing depend on this shared type; eventing re-exports it.
    #[derive(Debug, Clone)]
    pub struct NewEventEnvelope {
        pub event_type: String,
        pub payload: String,
        pub metadata: Option<NewEventMetadata>,
    }

    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
    pub struct NewEventMetadata {
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "sequence"
        )]
        pub sequence: Option<u64>,
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
        pub extra_info: Option<ExtraInfo>,
    }

    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
    pub struct EventMetadata {
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "sequence"
        )]
        pub sequence: Option<u64>,
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
        #[error("event dispatch error: {0}")]
        EventDispatch(String),
    }
}

mod ports {
    use super::models::EventError;
    use crate::io::NewEventEnvelope;
    use uuid::Uuid;

    /// Port implemented by the event gateway to dispatch events produced by
    /// command handlers. Defined in commanding so eventing can implement it
    /// while depending on commanding (not the reverse).
    pub trait EventDispatchPort: Send + Sync {
        fn dispatch(&self, event: NewEventEnvelope) -> Result<(), EventError>;
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
            reason: Option<String>,
        ) -> Result<(), EventError>;
    }
}

#[cfg(test)]
mod sequence_tests {
    use super::io::*;
    #[test]
    fn metadata_rejects_invalid_revisions_and_preserves_legacy_omission() {
        let mut value = serde_json::json!({"event_id":uuid::Uuid::now_v7(),"correlation_id":null,"causation_id":null,"source":null});
        let old: NewEventMetadata = serde_json::from_value(value.clone()).unwrap();
        assert!(old.sequence.is_none());
        assert_eq!(serde_json::to_value(old).unwrap(), value);
        for invalid in [
            serde_json::json!(0),
            serde_json::json!(-1),
            serde_json::json!(i64::MAX as u64 + 1),
            serde_json::json!("1"),
        ] {
            value["sequence"] = invalid;
            assert!(serde_json::from_value::<NewEventMetadata>(value.clone()).is_err());
            assert!(serde_json::from_value::<EventMetadata>(value.clone()).is_err());
        }
    }
}
