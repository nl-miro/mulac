//! Framework-free inbox HTTP contract and operations.
//!
//! HTTP adapters such as `mulac_poem` and `mulac_axum` parse request bodies
//! into JSON values and delegate parsing, validation and execution here.
pub mod prelude {
    pub use crate::contract::prelude::{
        RunRequest,
        RunResponse,
        SubmitInboxRequest,
        SubmitInboxResponse,
        //
    };
    pub use crate::error::prelude::InboxApiError;
    #[cfg(feature = "tokio")]
    pub use crate::runtime::prelude::AsyncInboxService;
    pub use crate::service::prelude::InboxService;
    #[cfg(feature = "testing")]
    pub use crate::testing::prelude as testing;
}

mod contract;
mod error;
#[cfg(feature = "tokio")]
mod runtime;
mod service;
#[cfg(feature = "testing")]
mod testing;
