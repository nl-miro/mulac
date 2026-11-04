mod application;
mod domain;

#[cfg(feature = "diesel")]
mod infra_diesel;

pub mod prelude {

    pub use super::application::prelude::{
        EventDispatchPort,
        EventEnvelope,
        EventError,
        EventMetadata,
        EventProcessPort,
        EventStorePort, //
    };

    pub use super::domain::{EventStatus, ExtraInfo, UnknownEventStatus};

    #[cfg(feature = "diesel")]
    pub(crate) use super::domain::Criterion;

    #[cfg(feature = "diesel")]
    pub use super::infra_diesel::prelude::{
        EventConsumerStorage,
        EventEntry,
        EventStoreStorage,
        NewEventEntry, //
    };

    pub use super::application::prelude::{NewEventEnvelope, NewEventMetadata};
}
