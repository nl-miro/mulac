mod application;
mod domain;

#[cfg(feature = "diesel")]
mod infra_diesel;

pub mod io {
    pub use super::application::io::{
        EventDispatchPort,
        EventEnvelope,
        EventError,
        EventMetadata,
        EventProcessPort,
        EventStorePort,
        NewEventEnvelope,
        NewEventMetadata, //
    };

    pub use super::domain::{EventStatus, UnknownEventStatus};

    #[cfg(feature = "diesel")]
    pub(crate) use super::domain::Criterion;

    #[cfg(feature = "diesel")]
    pub use super::infra_diesel::io::{
        DbPool,
        EventConsumerStorage,
        EventEntry,
        EventStoreStorage,
        NewEventEntry,
        build_pool, //
    };
}
