mod assembly;
mod dispatcher;
mod event_consumer;
mod gateway;
mod model;
mod record_events;
mod stale_event_sweep;

pub mod io {
    // Gateway
    pub use crate::gateway::io::EventGateway;

    // Dispatcher
    pub use crate::dispatcher::io::{EventDispatcher, EventSubscriberPort};

    // Record
    pub use crate::record_events::io::{EventRecorder, EventRecorderRepository};

    // Consumer
    pub use crate::event_consumer::io::{
        EventConsumer, EventConsumerRepository, EventReservePort, ReservableEventSpec,
    };

    // Sweep
    pub use crate::stale_event_sweep::io::{EventSweepPort, EventSweeper, StaleEventSpec};

    // Application types and ports — eventing
    pub use crate::assembly::io::{
        EventDispatchPort, EventEnvelope, EventError, EventMetadata, EventProcessPort, EventStatus,
        EventStorePort, UnknownEventStatus,
    };

    pub use super::assembly::io::{NewEventEnvelope, NewEventMetadata};

    // Diesel infra — eventing (feature-gated)
    #[cfg(feature = "diesel")]
    pub use crate::assembly::io::{
        EventConsumerStorage, EventEntry, EventStoreStorage, NewEventEntry,
    };
}
