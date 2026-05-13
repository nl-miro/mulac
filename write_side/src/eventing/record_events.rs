pub mod io {
    pub use super::recorder::EventRecorder;
    pub use super::repository::EventRecorderRepository;
}

mod repository {
    use std::sync::Arc;

    use crate::eventing::assembly::io::{
        EventError,
        EventStorePort,
        NewEventEnvelope, //
    };

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
    use std::sync::Arc;

    use crate::eventing::assembly::io::{EventError, NewEventEnvelope};

    use super::repository::EventRecorderRepository;

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
