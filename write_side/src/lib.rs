pub mod commanding;
pub mod eventing;

pub mod io {
    // Gateway
    pub use crate::commanding::gateway::io::CommandGateway;
    pub use crate::eventing::gateway::io::EventGateway;

    // Dispatcher
    pub use crate::commanding::dispatcher::CommandDispatcher;
    pub use crate::commanding::dispatcher::io::{CommandHandlerPort, EventDispatchPort};
    pub use crate::eventing::dispatcher::io::{EventDispatcher, EventSubscriberPort};

    // Record
    pub use crate::commanding::record_commands::io::{CommandRecorder, CommandRecorderRepository};
    pub use crate::eventing::record_events::io::{EventRecorder, EventRecorderRepository};

    // Consumer
    pub use crate::commanding::command_consumer::io::{
        CommandConsumer, CommandConsumerRepository, CommandReservePort, ReservableCommandSpec,
    };
    pub use crate::eventing::event_consumer::io::{
        EventConsumer, EventConsumerRepository, EventReservePort, ReservableEventSpec,
    };

    // Sweep
    pub use crate::commanding::stale_command_sweep::io::{
        CommandSweepPort, CommandSweeper, StaleCommandSpec,
    };
    pub use crate::eventing::stale_event_sweep::io::{
        EventSweepPort, EventSweeper, StaleEventSpec,
    };

    // Application types and ports — commanding
    pub use crate::commanding::assembly::io::{
        CommandEnvelope, CommandError, CommandMetadata, CommandProcessPort, CommandStatus,
        CommandStorePort, NewCommandEnvelope, NewCommandMetadata, UnknownCommandStatus,
    };

    // Application types and ports — eventing
    pub use crate::eventing::assembly::io::{
        EventEnvelope, EventError, EventMetadata, EventProcessPort, EventStatus, EventStorePort,
        NewEventEnvelope, NewEventMetadata, UnknownEventStatus,
    };

    // Diesel infra — commanding (feature-gated)
    #[cfg(feature = "diesel")]
    pub use crate::commanding::assembly::io::{
        CommandConsumerStorage, CommandEntry, CommandStoreStorage, DbPool, NewCommandEntry,
        build_pool,
    };

    // Diesel infra — eventing (feature-gated)
    #[cfg(feature = "diesel")]
    pub use crate::eventing::assembly::io::{
        EventConsumerStorage, EventEntry, EventStoreStorage, NewEventEntry,
    };
}
