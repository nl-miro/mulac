mod assembly;
mod dispatcher;
mod event_consumer;
mod gateway;
mod model;
mod record_events;
mod stale_event_sweep;

pub mod prelude {
    // Gateway
    pub use crate::gateway::prelude::EventGateway;

    // Dispatcher
    pub use crate::dispatcher::prelude::{EventDispatcher, EventSubscriberPort};

    // Record
    pub use crate::record_events::prelude::{EventRecorder, EventRecorderRepository};

    // Consumer
    pub use crate::event_consumer::prelude::{
        EventConsumer,
        EventConsumerRepository,
        EventReservePort,
        ReservableEventSpec,
        //
    };

    // Sweep
    pub use crate::stale_event_sweep::prelude::{
        EventSweepPort,
        EventSweeper,
        StaleEventSpec,
        //
    };

    // Application types and ports — eventing
    pub use crate::assembly::prelude::{
        EventDispatchPort,
        EventEnvelope,
        EventError,
        EventMetadata,
        EventProcessPort,
        EventStatus,
        EventStorePort,
        ExtraInfo,
        UnknownEventStatus,
        //
    };

    pub use super::assembly::prelude::{NewEventEnvelope, NewEventMetadata};

    // Diesel infra — eventing (feature-gated)
    #[cfg(feature = "diesel")]
    pub use crate::assembly::prelude::{
        EventConsumerStorage,
        EventEntry,
        EventStoreStorage,
        NewEventEntry,
        //
    };
}
