pub mod io {
    pub use super::handler::CommandHandlerPort;
    pub use crate::eventing::assembly::io::EventDispatchPort;
}

mod handler {
    use crate::commanding::assembly::io::{CommandError, NewCommandEnvelope};
    use crate::eventing::assembly::io::NewEventEnvelope;

    pub trait CommandHandlerPort: Send + Sync {
        fn execute(
            &self,
            envelope: &NewCommandEnvelope,
        ) -> Result<Vec<NewEventEnvelope>, CommandError>;
    }
}

mod dispatcher {
    use std::sync::Arc;

    use crate::commanding::assembly::io::{CommandError, NewCommandEnvelope};
    use crate::eventing::assembly::io::EventDispatchPort;

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
