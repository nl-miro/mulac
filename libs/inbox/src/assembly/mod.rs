mod application;
pub(crate) mod domain;
#[cfg(feature = "diesel")]
pub(crate) mod infra_diesel;

pub mod prelude {
    pub use super::application::prelude::{
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
    pub use super::domain::{ExtraInfo, InboxStatus};
    #[cfg(feature = "diesel")]
    pub(crate) use super::infra_diesel::entity::{InboxEntry, NewInboxEntry};
    #[cfg(feature = "diesel")]
    pub use super::infra_diesel::prelude::{
        DbPool,
        InboxConsumerStorage,
        InboxStoreStorage,
        build_pool,
        //
    };
    #[cfg(feature = "diesel")]
    pub(crate) use super::infra_diesel::schema::inbox_entries;
}
