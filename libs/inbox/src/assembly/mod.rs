mod application;
mod domain;
#[cfg(feature = "diesel")]
mod infra_diesel;

pub mod io {
    pub use super::application::io::{
        AcknowledgeHandle,
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
    pub(crate) use super::domain::Criterion;
    pub use super::domain::InboxStatus;
    #[cfg(feature = "diesel")]
    pub(crate) use super::infra_diesel::entity::{InboxEntry, NewInboxEntry};
    #[cfg(feature = "diesel")]
    pub use super::infra_diesel::io::{
        DbPool,
        InboxConsumerStorage,
        InboxStoreStorage,
        build_pool,
        //
    };
    #[cfg(feature = "diesel")]
    pub(crate) use super::infra_diesel::schema::inbox_entries;
}
