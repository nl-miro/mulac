pub mod io {
    pub use super::handler::{
        ApplicationEvent, CommandHandlerPort, ErasedCommandHandler, wrap_handler,
    };
}

mod handler {
    use std::sync::Arc;

    use serde::de::DeserializeOwned;
    use uuid::Uuid;

    use crate::assembly::io::{CommandError, NewCommandEnvelope};
    use eventing::io::{NewEventEnvelope, NewEventMetadata};

    pub trait ApplicationEvent: serde::Serialize + Send + Sync {
        fn event_type(&self) -> &'static str;
    }

    pub trait CommandHandlerPort<C, E>: Send + Sync {
        fn execute(&self, command: C) -> Result<Vec<E>, CommandError>;
    }

    pub trait ErasedCommandHandler: Send + Sync {
        fn execute(
            &self,
            envelope: &NewCommandEnvelope,
        ) -> Result<Vec<NewEventEnvelope>, CommandError>;
    }

    struct HandlerWrapper<C, E> {
        inner: Arc<dyn CommandHandlerPort<C, E>>,
    }

    impl<C, E> ErasedCommandHandler for HandlerWrapper<C, E>
    where
        C: DeserializeOwned + Send + Sync,
        E: ApplicationEvent,
    {
        fn execute(
            &self,
            envelope: &NewCommandEnvelope,
        ) -> Result<Vec<NewEventEnvelope>, CommandError> {
            let command: C = serde_json::from_str(&envelope.command.payload)
                .map_err(|e| CommandError::Conversion(e.to_string()))?;

            let command_metadata = envelope.metadata.as_ref();
            let causation_id = command_metadata.map(|m| m.command_id);
            let correlation_id = command_metadata
                .and_then(|m| m.correlation_id)
                .or(causation_id);

            self.inner
                .execute(command)?
                .into_iter()
                .map(|event| {
                    Ok(NewEventEnvelope {
                        event_type: event.event_type().to_string(),
                        payload: serde_json::to_string(&event)
                            .map_err(|e| CommandError::Conversion(e.to_string()))?,
                        metadata: Some(NewEventMetadata {
                            event_id: Uuid::now_v7(),
                            correlation_id,
                            causation_id,
                            source: None,
                        }),
                    })
                })
                .collect()
        }
    }

    pub fn wrap_handler<C, E>(
        handler: Arc<dyn CommandHandlerPort<C, E>>,
    ) -> Arc<dyn ErasedCommandHandler>
    where
        C: DeserializeOwned + Send + Sync + 'static,
        E: ApplicationEvent + 'static,
    {
        Arc::new(HandlerWrapper { inner: handler })
    }
}

mod dispatcher {
    use std::sync::Arc;

    use crate::assembly::io::{CommandError, NewCommandEnvelope};
    use eventing::io::EventDispatchPort;

    use super::handler::ErasedCommandHandler;

    pub struct CommandDispatcher {
        handler: Arc<dyn ErasedCommandHandler>,
        event_dispatcher: Arc<dyn EventDispatchPort>,
    }

    impl CommandDispatcher {
        pub fn new(
            handler: Arc<dyn ErasedCommandHandler>,
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
