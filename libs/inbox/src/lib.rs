//! Inbox components
pub mod prelude {
    #[cfg(feature = "amqp")]
    pub use crate::amqp_consumption::prelude::{
        AmqpClientError,
        AmqpTransport,
        AmqpWorker,
        Channel,
        Connection,
        ConnectionProperties,
        connection,
        //
    };
    pub use crate::assembly::prelude::{
        AcknowledgeHandle,
        ExtraInfo,
        InboundMessageEnvelope,
        InboxError,
        InboxMessageEnvelope,
        InboxMessageMetadata,
        InboxProcessPort,
        InboxTransportFuture,
        InboxTransportPort,
        InboxTransportResult,
        //
    };
    #[cfg(feature = "diesel")]
    pub use crate::assembly::prelude::{
        DbPool,
        InboxConsumerStorage,
        InboxStoreStorage,
        build_pool,
        //
    };
    pub use crate::inbox_consumer::prelude::{
        InboxConsumer,
        InboxConsumerRepository,
        InboxReservePort,
        ReservableInboxSpec,
        //
    };
    #[cfg(feature = "diesel")]
    pub use crate::record_messages::prelude::repository;
    pub use crate::record_messages::prelude::{
        InboxRecorder,
        InboxRecorderRepository,
        InboxStorePort,
        NewInboxMessageEnvelope,
        //
    };
    pub use crate::stale_reservation_sweep::prelude::{
        InboxSweepPort,
        ReservationSweeper,
        StaleReservationSpec,
        //
    };
}

mod amqp_consumption;
mod assembly;
mod inbox_consumer;
mod record_messages;
mod stale_reservation_sweep;
