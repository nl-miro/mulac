//! Inbox components
pub mod io {
    pub use crate::inbox_consumer::io::{
        InboxConsumer, InboxConsumerRepository, InboxReservePort, ReservableInboxSpec,
    };
    pub use crate::record_messages::io::{InboxRecorder, InboxRecorderRepository, InboxStorePort};
    pub use crate::stale_reservation_sweep::io::{
        InboxSweepPort, ReservationSweeper, StaleReservationSpec,
    };

    #[cfg(feature = "diesel")]
    pub use crate::record_messages::io::repository;

    pub use crate::assembly::io::{
        AcknowledgeHandle, InboundMessageEnvelope, InboxError, InboxMessageEnvelope,
        InboxMessageMetadata, InboxProcessPort, InboxTransportFuture, InboxTransportPort,
        InboxTransportResult,
    };

    #[cfg(feature = "diesel")]
    pub use crate::assembly::io::{DbPool, InboxConsumerStorage, InboxStoreStorage, build_pool};

    #[cfg(feature = "amqp")]
    pub use crate::amqp_consumption::io::{
        AmqpClientError, AmqpTransport, AmqpWorker, Channel, Connection, ConnectionProperties,
        connection,
    };
}

mod amqp_consumption;
mod assembly;
mod inbox_consumer;
mod record_messages;
mod stale_reservation_sweep;
