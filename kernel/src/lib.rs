mod assembly;
mod command_handler_registry;
mod event_subscriber_registry;
mod registrations;
mod runtime;
mod workers;

pub mod io {
    pub use super::assembly::{
        AppState, ApplicationCommand, GatewayNewCommandEnvelope, KernelBuilder, KernelConfig,
        KernelError, KernelHandle, NewCommandEnvelope, PersistentKernelHandle,
        PersistentKernelState, block_on_blocking, boot, first_command_error, first_event_error,
    };
    pub use super::command_handler_registry::CommandHandlerRegistry;
    pub use super::event_subscriber_registry::EventSubscriberRegistry;
    pub use super::workers::{run_command_worker, run_event_worker};
    pub use commanding::io::{
        ApplicationEvent,
        CommandConsumer,
        CommandConsumerRepository,
        CommandConsumerStorage,
        CommandDispatcher,
        CommandError,
        CommandGateway,
        CommandHandlerPort,
        CommandRecorder,
        CommandRecorderRepository,
        CommandStoreStorage,
        NewCommand,
        NewCommandMetadata,
        ReservableCommandSpec,
        wrap_handler, //
    };
    pub use mulac_diesel::{DbPool, build_pool};
    pub use outbox::io::{NewOutboxEnvelope, NewOutboxMetadata, OutboxError};

    pub use eventing::io::{
        EventConsumer,
        EventConsumerRepository,
        EventConsumerStorage,
        EventDispatcher,
        EventEnvelope,
        EventError,
        EventGateway,
        EventMetadata,
        EventProcessPort,
        EventRecorder,
        EventRecorderRepository,
        EventReservePort,
        EventStoreStorage,
        EventSubscriberPort,
        NewEventEnvelope,
        NewEventMetadata,
        ReservableEventSpec, //
    };
    pub use inbox::io::{
        InboundMessageEnvelope,
        InboxConsumer,
        InboxConsumerRepository,
        InboxConsumerStorage,
        InboxError,
        InboxMessageMetadata,
        InboxRecorder,
        InboxRecorderRepository,
        InboxStorePort,
        NewInboxMessageEnvelope,
        ReservableInboxSpec, //
    };
}
