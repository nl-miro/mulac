pub mod application;
pub mod domain;

#[cfg(feature = "diesel")]
pub mod infra_diesel;

pub mod io {
    pub use super::application::io::*;
    pub use super::domain::*;
    #[cfg(feature = "diesel")]
    pub(crate) use super::infra_diesel::io::record_entries_on_connection;
}
