use commanding::io::{CommandError, ErasedCommandHandler};
use std::{collections::HashMap, sync::Arc};

use crate::NewEventEnvelope;
use crate::{CommandHandlers, GatewayNewCommandEnvelope};

pub struct CommandHandlerRegistry {
    handlers: HashMap<String, Arc<dyn ErasedCommandHandler>>,
}

impl CommandHandlerRegistry {
    pub fn from_handlers(handlers: CommandHandlers) -> Self {
        Self {
            handlers: handlers.into_items().into_iter().collect(),
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
