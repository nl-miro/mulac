pub mod prelude {
    pub use super::recorder::EventRecorder;
    pub use super::repository::EventRecorderRepository;
}

mod repository {
    use crate::assembly::prelude::{
        EventError,
        EventStorePort,
        NewEventEnvelope,
        //
    };
    use std::sync::Arc;

    pub struct EventRecorderRepository {
        pub(super) store: Arc<dyn EventStorePort>,
    }

    impl EventRecorderRepository {
        pub fn new(store: Arc<dyn EventStorePort>) -> Self {
            Self { store }
        }

        pub(super) fn insert(&self, envelope: &NewEventEnvelope) -> Result<(), EventError> {
            self.store.record(envelope)
        }
    }
}

mod recorder {
    use super::repository::EventRecorderRepository;
    use crate::assembly::prelude::EventError;
    use crate::assembly::prelude::NewEventEnvelope;
    use std::sync::Arc;

    pub struct EventRecorder {
        repo: Arc<EventRecorderRepository>,
    }

    impl EventRecorder {
        pub fn new(repo: Arc<EventRecorderRepository>) -> Self {
            Self { repo }
        }

        pub fn record(&self, envelope: &NewEventEnvelope) -> Result<(), EventError> {
            self.repo.insert(envelope)
        }
    }
}
