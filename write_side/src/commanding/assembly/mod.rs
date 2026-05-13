mod application;
mod domain;

#[cfg(feature = "diesel")]
mod infra_diesel;

pub mod io {
    pub use super::application::io::{
        CommandEnvelope,
        CommandError,
        CommandMetadata,
        CommandProcessPort,
        CommandStorePort,
        NewCommandEnvelope,
        NewCommandMetadata, //
    };

    pub use super::domain::{CommandStatus, UnknownCommandStatus};

    #[cfg(feature = "diesel")]
    pub(crate) use super::domain::Criterion;

    #[cfg(feature = "diesel")]
    pub use super::infra_diesel::io::{
        CommandConsumerStorage,
        CommandEntry,
        CommandStoreStorage,
        DbPool,
        NewCommandEntry,
        build_pool, //
    };

    #[cfg(feature = "diesel")]
    pub(crate) use super::infra_diesel::schema::command_entries;
}
