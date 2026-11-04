//! Framework-free eventing HTTP contract and operations.
//!
//! HTTP adapters such as `mulac_poem` and `mulac_axum` parse request bodies
//! into JSON values and delegate validation and execution here. The event
//! consumer, with its dispatcher and subscribers, is built by the application or
//! the kernel.
pub mod prelude {
    pub use crate::contract::prelude::{
        RunEventsRequest,
        RunEventsResponse,
        //
    };
    pub use crate::error::prelude::EventingApiError;
    #[cfg(feature = "tokio")]
    pub use crate::runtime::prelude::AsyncEventingService;
    pub use crate::service::prelude::EventingService;
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
