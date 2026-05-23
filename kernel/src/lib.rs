use std::{collections::HashMap, sync::Arc};

use inbox::io::{
    InboxError, InboxRecorder, InboxRecorderRepository, InboxStorePort, NewInboxMessageEnvelope,
};
use outbox::io::OutboxError;
use tokio_util::sync::CancellationToken;
use write_side::io::{
    CommandDispatcher, ErasedCommandHandler, EventDispatcher, NewCommand as GatewayNewCommand,
    NewCommandEnvelope as GatewayCommandEnvelope, wrap_handler,
};

pub use inbox::io::{InboundMessageEnvelope, InboxMessageMetadata};
pub use outbox::io::{NewOutboxEnvelope, NewOutboxMetadata};
pub use write_side::io::{
    ApplicationEvent, Command, CommandConsumer, CommandConsumerRepository, CommandEnvelope,
    CommandError, CommandGateway, CommandHandlerPort, CommandMetadata, CommandProcessPort,
    CommandReservePort, ErasedCommandHandler as GatewayCommandHandler, EventConsumer,
    EventConsumerRepository, EventEnvelope, EventError, EventGateway, EventMetadata,
    EventProcessPort, EventReservePort, EventSubscriberPort, NewCommandMetadata, NewEventEnvelope,
    NewEventMetadata, ReservableCommandSpec, ReservableEventSpec,
};

pub type GatewayNewCommandEnvelope = GatewayCommandEnvelope;

const OUTBOX_SUBSCRIBER: &str = "_outbox";

/// Application-owned command boundary.
///
/// Apps define their own command enum and implement this trait. The kernel stays
/// independent of application command types while still offering typed dispatch
/// ergonomics to HTTP handlers and other app entrypoints.
pub trait ApplicationCommand: serde::Serialize {
    fn command_type(&self) -> &'static str;
}

/// Typed application command envelope.
///
/// The command is app-defined, while metadata is the mulac/kernel routing and
/// correlation metadata needed to cross the command gateway boundary.
pub struct NewCommandEnvelope<C> {
    pub command: C,
    pub metadata: NewCommandMetadata,
}

impl<C> NewCommandEnvelope<C>
where
    C: ApplicationCommand,
{
    pub fn into_gateway_envelope(self) -> Result<GatewayNewCommandEnvelope, serde_json::Error> {
        Ok(GatewayNewCommandEnvelope {
            command: GatewayNewCommand {
                command_type: self.command.command_type().to_string(),
                payload: serde_json::to_string(&self.command)?,
            },
            metadata: Some(self.metadata),
        })
    }
}

pub fn boot(config: KernelConfig) -> KernelBuilder {
    KernelBuilder::new(config)
}

#[derive(Debug, Clone, Default)]
pub struct KernelConfig {
    pub database_url: Option<String>,
    pub amqp_url: Option<String>,
    pub inbox_queue: Option<String>,
    pub inbox_exchange: Option<String>,
    pub outbox_exchange: Option<String>,
    pub outbox_content_type: Option<String>,
}

impl KernelConfig {
    pub fn from_env() -> Result<Self, KernelError> {
        Ok(Self {
            database_url: std::env::var("DATABASE_URL").ok(),
            amqp_url: std::env::var("AMQP_URL").ok(),
            inbox_queue: std::env::var("INBOX_QUEUE").ok(),
            inbox_exchange: std::env::var("INBOX_EXCHANGE").ok(),
            outbox_exchange: std::env::var("OUTBOX_EXCHANGE").ok(),
            outbox_content_type: std::env::var("OUTBOX_CONTENT_TYPE").ok(),
        })
    }
}

#[derive(Clone)]
pub struct AppState {
    pub inbox_recorder: Arc<InboxRecorder>,
    pub command_gateway: Arc<CommandGateway>,
    pub event_gateway: Arc<EventGateway>,
}

impl AppState {
    pub fn dispatch_command<C>(&self, envelope: NewCommandEnvelope<C>) -> Result<(), KernelError>
    where
        C: ApplicationCommand,
    {
        let envelope = envelope
            .into_gateway_envelope()
            .map_err(|e| KernelError::Command(CommandError::Conversion(e.to_string())))?;
        self.command_gateway.dispatch(envelope)?;
        Ok(())
    }
}

pub struct KernelBuilder {
    _config: KernelConfig,
    command_handlers: Vec<(String, Arc<dyn ErasedCommandHandler>)>,
    event_subscribers: Vec<(String, String, Arc<dyn EventSubscriberPort>)>,
    outbox_subscribers: Vec<String>,
}

impl KernelBuilder {
    fn new(config: KernelConfig) -> Self {
        Self {
            _config: config,
            command_handlers: Vec::new(),
            event_subscribers: Vec::new(),
            outbox_subscribers: Vec::new(),
        }
    }

    pub fn command_handler<C, E>(
        mut self,
        command_type: impl Into<String>,
        handler: Arc<dyn CommandHandlerPort<C, E>>,
    ) -> Self
    where
        C: serde::de::DeserializeOwned + Send + Sync + 'static,
        E: ApplicationEvent + 'static,
    {
        self.command_handlers
            .push((command_type.into(), wrap_handler(handler)));
        self
    }

    pub fn event_subscriber(
        mut self,
        event_type: impl Into<String>,
        subscriber_name: impl Into<String>,
        subscriber: Arc<dyn EventSubscriberPort>,
    ) -> Self {
        self.event_subscribers
            .push((event_type.into(), subscriber_name.into(), subscriber));
        self
    }

    pub fn outbox_subscriber(mut self, event_type: impl Into<String>) -> Self {
        self.outbox_subscribers.push(event_type.into());
        self
    }

    pub async fn start(self) -> Result<KernelHandle, KernelError> {
        self.validate()?;

        let event_registry = Arc::new(EventSubscriberRegistry::from_builder(&self));
        let event_dispatcher = Arc::new(EventDispatcher::new(event_registry));
        let event_gateway = Arc::new(EventGateway::direct(event_dispatcher.clone()));

        let command_registry =
            Arc::new(CommandHandlerRegistry::from_handlers(self.command_handlers));
        let command_dispatcher = Arc::new(CommandDispatcher::new(
            command_registry,
            event_gateway.clone(),
        ));
        let command_gateway = Arc::new(CommandGateway::direct(command_dispatcher.clone()));

        let inbox_recorder = Arc::new(InboxRecorder::new(Arc::new(InboxRecorderRepository::new(
            Arc::new(NoopInboxStore),
        ))));

        let state = AppState {
            inbox_recorder,
            command_gateway,
            event_gateway,
        };

        Ok(KernelHandle {
            state,
            token: CancellationToken::new(),
            command_dispatcher,
            event_dispatcher,
        })
    }

    fn validate(&self) -> Result<(), KernelError> {
        let mut command_types = std::collections::HashSet::new();
        for (command_type, _) in &self.command_handlers {
            if !command_types.insert(command_type.clone()) {
                return Err(KernelError::DuplicateCommandHandler(command_type.clone()));
            }
        }

        let mut event_subscribers = std::collections::HashSet::new();
        for (event_type, subscriber, _) in &self.event_subscribers {
            if subscriber == OUTBOX_SUBSCRIBER {
                return Err(KernelError::ReservedSubscriberName(subscriber.clone()));
            }
            if !event_subscribers.insert((event_type.clone(), subscriber.clone())) {
                return Err(KernelError::DuplicateEventSubscriber {
                    event_type: event_type.clone(),
                    subscriber: subscriber.clone(),
                });
            }
        }

        let mut outbox_event_types = std::collections::HashSet::new();
        for event_type in &self.outbox_subscribers {
            if !outbox_event_types.insert(event_type.clone()) {
                return Err(KernelError::ReservedSubscriberName(
                    OUTBOX_SUBSCRIBER.into(),
                ));
            }
        }

        Ok(())
    }
}

pub struct KernelHandle {
    state: AppState,
    token: CancellationToken,
    command_dispatcher: Arc<CommandDispatcher>,
    event_dispatcher: Arc<EventDispatcher>,
}

impl KernelHandle {
    pub fn state(&self) -> AppState {
        self.state.clone()
    }

    pub fn child_token(&self) -> CancellationToken {
        self.token.child_token()
    }

    pub fn command_consumer(&self, repo: CommandConsumerRepository) -> CommandConsumer {
        CommandConsumer::new(repo, self.command_dispatcher.clone())
    }

    pub fn event_consumer(&self, repo: EventConsumerRepository) -> EventConsumer {
        EventConsumer::new(repo, self.event_dispatcher.clone())
    }

    pub fn shutdown(&self) {
        self.token.cancel();
    }

    pub async fn wait(self) -> Result<(), KernelError> {
        Ok(())
    }
}

struct CommandHandlerRegistry {
    handlers: HashMap<String, Arc<dyn ErasedCommandHandler>>,
}

impl CommandHandlerRegistry {
    fn from_handlers(handlers: Vec<(String, Arc<dyn ErasedCommandHandler>)>) -> Self {
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

struct EventSubscriberRegistry {
    subscribers: HashMap<String, Vec<(String, Arc<dyn EventSubscriberPort>)>>,
}

impl EventSubscriberRegistry {
    fn from_builder(builder: &KernelBuilder) -> Self {
        let mut subscribers: HashMap<String, Vec<(String, Arc<dyn EventSubscriberPort>)>> =
            HashMap::new();

        for event_type in &builder.outbox_subscribers {
            subscribers
                .entry(event_type.clone())
                .or_default()
                .push((OUTBOX_SUBSCRIBER.into(), Arc::new(NoopEventSubscriber)));
        }

        for (event_type, subscriber_name, subscriber) in &builder.event_subscribers {
            subscribers
                .entry(event_type.clone())
                .or_default()
                .push((subscriber_name.clone(), subscriber.clone()));
        }

        Self { subscribers }
    }
}

impl EventSubscriberPort for EventSubscriberRegistry {
    fn handle(&self, envelope: &NewEventEnvelope) -> Result<(), EventError> {
        for (_, subscriber) in self
            .subscribers
            .get(&envelope.event_type)
            .into_iter()
            .flatten()
        {
            subscriber.handle(envelope)?;
        }
        Ok(())
    }
}

struct NoopEventSubscriber;

impl EventSubscriberPort for NoopEventSubscriber {
    fn handle(&self, _envelope: &NewEventEnvelope) -> Result<(), EventError> {
        Ok(())
    }
}

struct NoopInboxStore;

impl InboxStorePort for NoopInboxStore {
    fn store(&self, _msg: NewInboxMessageEnvelope) -> Result<(), InboxError> {
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum KernelError {
    #[error("database connection failed: {0}")]
    Database(String),
    #[error("AMQP connection failed: {0}")]
    Amqp(String),
    #[error("database migration failed: {0}")]
    Migration(String),
    #[error("command handler already registered for type '{0}'")]
    DuplicateCommandHandler(String),
    #[error("event subscriber '{subscriber}' already registered for event type '{event_type}'")]
    DuplicateEventSubscriber {
        event_type: String,
        subscriber: String,
    },
    #[error("subscriber name '{0}' is reserved")]
    ReservedSubscriberName(String),
    #[error("worker task failed: {0}")]
    Worker(String),
    #[error("inbox error: {0}")]
    Inbox(#[from] InboxError),
    #[error("command error: {0}")]
    Command(#[from] CommandError),
    #[error("event error: {0}")]
    Event(#[from] EventError),
    #[error("outbox error: {0}")]
    Outbox(#[from] OutboxError),
}
