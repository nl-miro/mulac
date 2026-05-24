pub mod io {
    pub use super::models::{
        InboundMessageEnvelope,
        InboxError,
        InboxMessageEnvelope,
        InboxMessageMetadata,
        //
    };
    pub use super::ports::{
        AcknowledgeHandle,
        InboxProcessPort,
        InboxTransportFuture,
        InboxTransportPort,
        InboxTransportResult,
        //
    };
}

mod models {
    use crate::assembly::domain::{InboxMessage, InboxStatus};
    use uuid::Uuid;

    #[derive(Debug, Clone, Default)]
    pub struct InboundMessageEnvelope {
        pub payload: String,
        pub message_id: Option<Uuid>,
        pub correlation_id: Option<Uuid>,
        pub source: Option<String>,
        pub routing_key: Option<String>,
    }

    #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
    pub struct InboxMessageMetadata {
        pub message_id: Option<Uuid>,
        pub correlation_id: Option<Uuid>,
        pub source: Option<String>,
        pub routing_key: Option<String>,
    }

    #[derive(Debug)]
    pub struct InboxMessageEnvelope {
        pub msg: InboxMessage,
        pub meta: InboxMessageMetadata,
    }

    impl InboxMessageEnvelope {
        pub fn id(&self) -> &Uuid {
            &self.msg.id
        }

        pub fn payload(&self) -> &str {
            &self.msg.payload
        }

        pub fn status(&self) -> InboxStatus {
            self.msg.status
        }

        pub fn reservation_id(&self) -> Option<Uuid> {
            self.msg.reservation_id
        }
    }

    #[derive(Debug, thiserror::Error)]
    pub enum InboxError {
        #[error("storage error: {0}")]
        Storage(String),
        #[error("transport error: {0}")]
        Transport(String),
        #[error("acknowledgement error: {0}")]
        Acknowledgement(String),
        #[error("recording error: {0}")]
        Recording(String),
        #[error("inbox entry {id} is not reserved by reservation {reservation_id}")]
        ReservationNotOwned { id: Uuid, reservation_id: Uuid },
        #[error("Publish failed: {0}")]
        PublishFailed(String),
        #[error("Missing reservation for message {id}")]
        MissingReservation { id: Uuid },
        #[error("conversion error: {0}")]
        Conversion(String),
    }
}

mod ports {
    use super::models::{InboundMessageEnvelope, InboxError};
    use std::future::Future;
    use std::pin::Pin;
    use uuid::Uuid;

    pub trait InboxProcessPort: Send + Sync {
        fn completed(&self, id: Uuid, reservation_id: Uuid) -> Result<(), InboxError>;
        fn failed(
            &self,
            id: Uuid,
            reservation_id: Uuid,
            max_attempts: i32,
        ) -> Result<(), InboxError>;
    }

    /// Application-owned acknowledgement boundary for inbound delivery handles.
    ///
    /// The application layer owns this trait because recording an inbound message is
    /// an application use case: the worker must acknowledge the external delivery
    /// only after the recorder accepts the application envelope. Concrete protocol
    /// mechanics, such as AMQP `basic.ack`/`basic.nack`, stay in infra/adapters.
    pub trait AcknowledgeHandle: Send {
        fn ack(self: Box<Self>) -> Pin<Box<dyn Future<Output = Result<(), InboxError>> + Send>>;
        fn nack(self: Box<Self>) -> Pin<Box<dyn Future<Output = Result<(), InboxError>> + Send>>;
    }

    pub type InboxTransportResult =
        Result<Option<(InboundMessageEnvelope, Box<dyn AcknowledgeHandle>)>, InboxError>;

    pub type InboxTransportFuture<'a> =
        Pin<Box<dyn Future<Output = InboxTransportResult> + Send + 'a>>;

    /// Application input port for polling inbound messages from an adapter.
    ///
    /// Runtime concerns such as sleeping, cancellation, reconnecting, and concrete
    /// broker clients remain outside application. This port only describes the
    /// application contract: receive the next inbound envelope with a handle that
    /// can acknowledge the external delivery after the use case succeeds or fails.
    pub trait InboxTransportPort {
        fn next(&mut self) -> InboxTransportFuture<'_>;
    }
}
