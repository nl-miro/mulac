pub mod commanding;
pub mod eventing;

pub mod io {
    // Gateway
    pub use crate::commanding::gateway::io::CommandGateway;

    // Dispatcher
    pub use crate::commanding::dispatcher::CommandDispatcher;
    pub use crate::commanding::dispatcher::io::{CommandHandlerPort, EventDispatchPort};

    // Record
    pub use crate::commanding::record_commands::io::{CommandRecorder, CommandRecorderRepository};

    // Consumer
    pub use crate::commanding::command_consumer::io::{
        CommandConsumer, CommandConsumerRepository, CommandReservePort, ReservableCommandSpec,
    };

    // Sweep
    pub use crate::commanding::stale_command_sweep::io::{
        CommandSweepPort, CommandSweeper, StaleCommandSpec,
    };

    // Application types and ports
    pub use crate::commanding::assembly::io::{
        CommandEnvelope, CommandError, CommandMetadata, CommandProcessPort, CommandStatus,
        CommandStorePort, NewCommandEnvelope, NewCommandMetadata, UnknownCommandStatus,
    };

    // Diesel infra (feature-gated)
    #[cfg(feature = "diesel")]
    pub use crate::commanding::assembly::io::{
        CommandConsumerStorage, CommandEntry, CommandStoreStorage, DbPool, NewCommandEntry,
        build_pool,
    };
}
