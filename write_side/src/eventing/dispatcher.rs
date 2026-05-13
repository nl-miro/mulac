pub mod io {
    pub use super::dispatcher::EventDispatcher;
    pub use super::ports::EventSubscriberPort;
}

mod ports {
    use crate::eventing::assembly::io::{EventError, NewEventEnvelope};

    pub trait EventSubscriberPort: Send + Sync {
        fn handle(&self, envelope: &NewEventEnvelope) -> Result<(), EventError>;
    }
}

mod dispatcher {
    use std::sync::Arc;

    use crate::eventing::assembly::io::{EventError, NewEventEnvelope};

    use super::ports::EventSubscriberPort;

    pub struct EventDispatcher {
        subscriber: Arc<dyn EventSubscriberPort>,
    }

    impl EventDispatcher {
        pub fn new(subscriber: Arc<dyn EventSubscriberPort>) -> Self {
            Self { subscriber }
        }

        pub fn dispatch(&self, envelope: &NewEventEnvelope) -> Result<(), EventError> {
            self.subscriber.handle(envelope)
        }
    }
}

pub use dispatcher::EventDispatcher;
