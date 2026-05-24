use commanding::io::{CommandError, ErasedCommandHandler};
use std::{collections::HashMap, sync::Arc};

use crate::GatewayNewCommandEnvelope;
use crate::NewEventEnvelope;

pub struct CommandHandlerRegistry {
    handlers: HashMap<String, Arc<dyn ErasedCommandHandler>>,
}

impl CommandHandlerRegistry {
    pub fn from_handlers(handlers: Vec<(String, Arc<dyn ErasedCommandHandler>)>) -> Self {
        Self {
            handlers: handlers.into_iter().collect(),
        }
    }
}

impl ErasedCommandHandler for CommandHandlerRegistry {
    fn execute(
        &self,
        envelope: &GatewayNewCommandEnvelope,
    ) -> Result<Vec<NewEventEnvelope>, CommandError> {
        let handler = self
            .handlers
            .get(&envelope.command.command_type)
            .ok_or_else(|| CommandError::HandlerNotFound(envelope.command.command_type.clone()))?;
        handler.execute(envelope)
    }
}
