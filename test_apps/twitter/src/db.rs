// Re-exports from assembly::infra_diesel. Direct users should migrate to assembly::io.
pub use crate::assembly::io::{DbPool, MIGRATIONS, build_pool, run_migrations};
