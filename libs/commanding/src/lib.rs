mod assembly;
mod command_consumer;
mod dispatcher;
mod gateway;
mod record_commands;
mod stale_command_sweep;

pub mod prelude {
    // Gateway
    pub use crate::gateway::prelude::CommandGateway;

    // Dispatcher
    pub use crate::dispatcher::CommandDispatcher;
    pub use crate::dispatcher::prelude::CommandHandlerPort;

    // Record
    pub use crate::record_commands::prelude::{
        CommandRecorder,
        CommandRecorderRepository,
        //
    };

    // Consumer
    pub use crate::command_consumer::prelude::{
        CommandConsumer,
        CommandConsumerRepository,
        CommandReservePort,
        ReservableCommandSpec,
        //
    };

    // Sweep
    pub use crate::stale_command_sweep::prelude::{
        CommandSweepPort,
        CommandSweeper,
        StaleCommandSpec,
        //
    };

    // Application types and ports — commanding
    pub use crate::assembly::prelude::{
        Command,
        CommandEnvelope,
        CommandError,
        CommandMetadata,
        CommandProcessPort,
        CommandStatus,
        CommandStorePort,
        ExtraInfo,
        NewCommand,
        NewCommandEnvelope,
        NewCommandMetadata,
        UnknownCommandStatus,
        //
    };
    pub use crate::dispatcher::prelude::{
        ApplicationEvent,
        ErasedCommandHandler,
        wrap_handler,
        //
    };

    // Diesel infra — commanding (feature-gated)
    #[cfg(feature = "diesel")]
    pub use crate::assembly::prelude::{
        CommandConsumerStorage,
        CommandEntry,
        CommandStoreStorage,
        NewCommandEntry,
        //
    };
}
