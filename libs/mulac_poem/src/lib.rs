//! Poem and poem-openapi adapter for the shared Mulac HTTP APIs.
//!
//! Applications declare the endpoints and delegate to [`prelude::InboxAdapter`] or,
//! with the `commanding`, `eventing` and `outbox` features,
//! `prelude::CommandingAdapter`, `prelude::EventingAdapter` and `prelude::OutboxAdapter`;
//! parsing, validation and execution live in `mulac_inbox_api`, `mulac_commanding_api`,
//! `mulac_eventing_api` and `mulac_outbox_api`.
pub mod prelude {
    pub use crate::adapter::prelude::{
        InboxAdapter,
        InboxSubmission,
        //
    };
    #[cfg(feature = "commanding")]
    pub use crate::commanding::prelude::CommandingAdapter;
    #[cfg(feature = "eventing")]
    pub use crate::eventing::prelude::EventingAdapter;
    #[cfg(feature = "outbox")]
    pub use crate::outbox::prelude::OutboxAdapter;
    pub use crate::types::prelude::{
        RunRequest,
        RunResponse,
        SubmitInboxRequest,
        SubmitInboxResponse,
        //
    };
}

mod adapter;
#[cfg(feature = "commanding")]
mod commanding;
#[cfg(feature = "eventing")]
mod eventing;
#[cfg(feature = "outbox")]
mod outbox;
mod types;
