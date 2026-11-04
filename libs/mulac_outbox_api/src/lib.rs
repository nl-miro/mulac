//! Framework-free outbox HTTP contract and operations.
//!
//! HTTP adapters such as `mulac_poem` and `mulac_axum` parse request bodies
//! into JSON values and delegate validation and execution here. The publisher
//! is owned by the application; this crate never depends on a broker client.
pub mod prelude {
    pub use crate::contract::prelude::{
        RunOutboxRequest,
        RunOutboxResponse,
        //
    };
    pub use crate::error::prelude::{
        OutboxApiError,
        OutboxApiErrorKind,
        //
    };
    #[cfg(feature = "tokio")]
    pub use crate::runtime::prelude::AsyncOutboxService;
    pub use crate::service::prelude::OutboxService;
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
