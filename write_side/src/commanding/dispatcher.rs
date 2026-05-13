pub mod io {
    pub use super::event_dispatch::EventDispatchPort;
    pub use super::handler::CommandHandlerPort;
}

mod handler {
    use crate::commanding::assembly::io::{CommandError, NewCommandEnvelope};
    use crate::eventing::assembly::application::EventEnvelope;

    pub trait CommandHandlerPort: Send + Sync {
        fn execute(
            &self,
            envelope: &NewCommandEnvelope,
        ) -> Result<Vec<EventEnvelope>, CommandError>;
    }
}

mod event_dispatch {
    use crate::commanding::assembly::io::CommandError;
    use crate::eventing::assembly::application::EventEnvelope;

    pub trait EventDispatchPort: Send + Sync {
        fn dispatch(&self, event: EventEnvelope) -> Result<(), CommandError>;
    }
}

mod dispatcher {
    use std::sync::Arc;

    use crate::commanding::assembly::io::{CommandError, NewCommandEnvelope};

    use super::event_dispatch::EventDispatchPort;
    use super::handler::CommandHandlerPort;

    pub struct CommandDispatcher {
        handler: Arc<dyn CommandHandlerPort>,
        event_dispatcher: Arc<dyn EventDispatchPort>,
    }

    impl CommandDispatcher {
        pub fn new(
            handler: Arc<dyn CommandHandlerPort>,
            event_dispatcher: Arc<dyn EventDispatchPort>,
        ) -> Self {
            Self {
                handler,
                event_dispatcher,
            }
        }

        pub fn dispatch(&self, envelope: &NewCommandEnvelope) -> Result<(), CommandError> {
            let events = self.handler.execute(envelope)?;
            for event in events {
                self.event_dispatcher.dispatch(event)?;
            }
            Ok(())
        }
    }
}

pub use dispatcher::CommandDispatcher;
