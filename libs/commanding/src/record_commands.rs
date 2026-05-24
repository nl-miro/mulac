pub mod io {
    pub use super::recorder::CommandRecorder;
    pub use super::repository::CommandRecorderRepository;
}

mod repository {
    use crate::assembly::io::{
        CommandError,
        CommandStorePort,
        NewCommandEnvelope,
        //
    };
    use std::sync::Arc;

    pub struct CommandRecorderRepository {
        pub(super) store: Arc<dyn CommandStorePort>,
    }

    impl CommandRecorderRepository {
        pub fn new(store: Arc<dyn CommandStorePort>) -> Self {
            Self { store }
        }

        pub(super) fn insert(&self, envelope: &NewCommandEnvelope) -> Result<(), CommandError> {
            self.store.record(envelope)
        }
    }
}

mod recorder {
    use super::repository::CommandRecorderRepository;
    use crate::assembly::io::{CommandError, NewCommandEnvelope};
    use std::sync::Arc;

    pub struct CommandRecorder {
        repo: Arc<CommandRecorderRepository>,
    }

    impl CommandRecorder {
        pub fn new(repo: Arc<CommandRecorderRepository>) -> Self {
            Self { repo }
        }

        pub fn record(&self, envelope: &NewCommandEnvelope) -> Result<(), CommandError> {
            self.repo.insert(envelope)
        }
    }
}

#[cfg(feature = "diesel")]
mod infra_diesel_pg {
    use crate::assembly::io::{
        CommandError,
        CommandStorePort,
        CommandStoreStorage,
        NewCommandEntry,
        NewCommandEnvelope,
        command_entries,
        //
    };
    use diesel::prelude::*;

    impl CommandStorePort for CommandStoreStorage {
        fn record(&self, envelope: &NewCommandEnvelope) -> Result<(), CommandError> {
            let mut conn = self
                .pool
                .get()
                .map_err(|e| CommandError::Storage(e.to_string()))?;

            let entry = NewCommandEntry::try_from(envelope)
                .map_err(|e| CommandError::Conversion(e.to_string()))?;

            diesel::insert_into(command_entries::table)
                .values(&entry)
                .on_conflict_do_nothing()
                .execute(&mut conn)
                .map_err(|e| CommandError::Storage(e.to_string()))?;

            Ok(())
        }
    }
}
