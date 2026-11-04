pub mod prelude {
    pub use super::dispatcher::EventDispatcher;
    pub use super::ports::EventSubscriberPort;
}

mod ports {
    use crate::assembly::prelude::EventError;
    use crate::assembly::prelude::NewEventEnvelope;

    pub trait EventSubscriberPort: Send + Sync {
        fn handle(&self, envelope: &NewEventEnvelope) -> Result<(), EventError>;
    }
}

mod dispatcher {
    use super::ports::EventSubscriberPort;
    use crate::assembly::prelude::EventError;
    use crate::assembly::prelude::NewEventEnvelope;
    use std::sync::Arc;

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
