mod assembly;
mod command_consumer;
mod dispatcher;
mod gateway;
mod record_commands;
mod stale_command_sweep;

pub mod io {
    // Gateway
    pub use crate::gateway::io::CommandGateway;

    // Dispatcher
    pub use crate::dispatcher::CommandDispatcher;
    pub use crate::dispatcher::io::CommandHandlerPort;

    // Record
    pub use crate::record_commands::io::{CommandRecorder, CommandRecorderRepository};

    // Consumer
    pub use crate::command_consumer::io::{
        CommandConsumer, CommandConsumerRepository, CommandReservePort, ReservableCommandSpec,
    };

    // Sweep
    pub use crate::stale_command_sweep::io::{CommandSweepPort, CommandSweeper, StaleCommandSpec};

    // Application types and ports — commanding
    pub use crate::assembly::io::{
        CommandEnvelope, CommandError, CommandMetadata, CommandProcessPort, CommandStatus,
        CommandStorePort, NewCommandEnvelope, NewCommandMetadata, UnknownCommandStatus,
    };

    // // Shared event envelope types (produced by command handlers, consumed by event dispatch)
    // pub use crate::assembly::io::{NewEventEnvelope, NewEventMetadata};

    // Diesel infra — commanding (feature-gated)
    #[cfg(feature = "diesel")]
    pub use crate::assembly::io::{
        CommandConsumerStorage, CommandEntry, CommandStoreStorage, DbPool, NewCommandEntry,
        build_pool,
    };
}
