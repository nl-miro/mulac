//! Axum adapter for the shared Mulac HTTP APIs.
//!
//! Applications declare the routes and delegate to [`prelude::InboxAdapter`] or,
//! with the `commanding`, `eventing` and `outbox` features,
//! `prelude::CommandingAdapter`, `prelude::EventingAdapter` and `prelude::OutboxAdapter`;
//! parsing, validation and execution live in `mulac_inbox_api`, `mulac_commanding_api`,
//! `mulac_eventing_api` and `mulac_outbox_api`.
pub mod prelude {
    pub use crate::adapter::prelude::{
        InboxAdapter,
        InboxRunResponse,
        InboxSubmission,
        //
    };
    #[cfg(feature = "commanding")]
    pub use crate::commanding::prelude::{
        CommandingAdapter,
        CommandingHttpError,
        CommandsRunResponse,
        RunCommands,
        //
    };
    #[cfg(feature = "eventing")]
    pub use crate::eventing::prelude::{
        EventingAdapter,
        EventingHttpError,
        EventsRunResponse,
        RunEvents,
        //
    };
    pub use crate::extract::prelude::{
        InboxHttpError,
        RunInbox,
        SubmitInbox,
        //
    };
    #[cfg(feature = "outbox")]
    pub use crate::outbox::prelude::{
        OutboxAdapter,
        OutboxHttpError,
        OutboxRunResponse,
        RunOutbox,
        //
    };
}

mod adapter;
mod body;
#[cfg(feature = "commanding")]
mod commanding;
#[cfg(feature = "eventing")]
mod eventing;
mod extract;
#[cfg(feature = "outbox")]
mod outbox;
#[cfg(feature = "utoipa")]
mod schema;
