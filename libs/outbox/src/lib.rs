pub mod prelude {
    pub use crate::assembly::prelude::*;
    pub use crate::outbox_consumer::prelude::{
        OutboxConsumer,
        OutboxConsumerRepository,
        ReservableOutboxSpec,
        //
    };
    pub use crate::record_events::prelude::{
        OutboxRecorder,
        OutboxRecorderRepository,
        //
    };
    pub use crate::stale_reservation_sweep::prelude::{
        ReservationSweeper,
        StaleReservationSpec,
        //
    };

    #[cfg(feature = "diesel")]
    pub use crate::assembly::infra_diesel::prelude::{
        DbPool,
        OutboxConsumerStorage,
        OutboxStoreStorage,
        build_pool,
        //
    };
    #[cfg(feature = "diesel")]
    pub use crate::outbox_consumer::prelude::repository as consumer_repository;
    #[cfg(feature = "diesel")]
    pub use crate::record_events::prelude::repository as recorder_repository;
    #[cfg(feature = "diesel")]
    pub use crate::stale_reservation_sweep::prelude::sweeper as reservation_sweeper;

    #[cfg(feature = "amqp")]
    pub use crate::amqp_publisher::prelude::{AmqpPublishConfig, AmqpPublisher};
    #[cfg(feature = "amqp")]
    pub use crate::confirmed_amqp_publisher::prelude::ConfirmedAmqpPublisher;
}

#[cfg(feature = "amqp")]
mod amqp_publisher;
mod assembly;
#[cfg(feature = "amqp")]
mod confirmed_amqp_publisher;
mod outbox_consumer;
mod record_events;
mod stale_reservation_sweep;
