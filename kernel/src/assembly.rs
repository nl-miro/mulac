use crate::{io, registrations, runtime};

use crate::command_handler_registry::CommandHandlerRegistry;
use crate::event_subscriber_registry::EventSubscriberRegistry;
use commanding::io::{
    ApplicationEvent,
    CommandError,
    CommandHandlerPort,
    NewCommandMetadata, //
};
use commanding::io::{
    CommandConsumer,
    CommandConsumerRepository,
    CommandDispatcher,
    CommandGateway,
    ErasedCommandHandler,
    NewCommand as GatewayNewCommand,
    NewCommandEnvelope as GatewayCommandEnvelope,
    wrap_handler, //
};
use eventing::io::{
    EventConsumer, EventConsumerRepository, EventDispatcher, EventError, EventGateway,
    EventSubscriberPort,
};
use inbox::io::{InboxError, InboxRecorder};
use outbox::io::OutboxError;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

pub type GatewayNewCommandEnvelope = GatewayCommandEnvelope;

pub(crate) const OUTBOX_SUBSCRIBER: &str = "_outbox";

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

/// Direct dispatch has no durable inbox boundary.
///
/// ```compile_fail
/// use mulac_kernel::io::AppState;
/// fn inbox(state: AppState) { let _ = state.inbox_recorder; }
/// ```
#[derive(Clone)]
pub struct AppState {
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
    config: KernelConfig,
    command_handlers: Vec<(String, Arc<dyn ErasedCommandHandler>)>,
    event_subscribers: Vec<EventSubscriberRegistration>,
    outbox_subscribers: Vec<String>,
}

pub(crate) enum EventSubscriberRegistration {
    Direct {
        event_type: String,
        subscriber_name: String,
        subscriber: Arc<dyn EventSubscriberPort>,
    },
    WithCommandGateway {
        event_type: String,
        subscriber_name: String,
        factory: Arc<dyn Fn(Arc<CommandGateway>) -> Arc<dyn EventSubscriberPort> + Send + Sync>,
    },
}

impl EventSubscriberRegistration {
    fn event_type(&self) -> &str {
        match self {
            Self::Direct { event_type, .. } | Self::WithCommandGateway { event_type, .. } => {
                event_type
            }
        }
    }

    fn subscriber_name(&self) -> &str {
        match self {
            Self::Direct {
                subscriber_name, ..
            }
            | Self::WithCommandGateway {
                subscriber_name, ..
            } => subscriber_name,
        }
    }
}

impl KernelBuilder {
    fn new(config: KernelConfig) -> Self {
        Self {
            config,
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
            .push(EventSubscriberRegistration::Direct {
                event_type: event_type.into(),
                subscriber_name: subscriber_name.into(),
                subscriber,
            });
        self
    }

    pub fn event_subscriber_with_command_gateway<F>(
        mut self,
        event_type: impl Into<String>,
        subscriber_name: impl Into<String>,
        factory: F,
    ) -> Self
    where
        F: Fn(Arc<CommandGateway>) -> Arc<dyn EventSubscriberPort> + Send + Sync + 'static,
    {
        self.event_subscribers
            .push(EventSubscriberRegistration::WithCommandGateway {
                event_type: event_type.into(),
                subscriber_name: subscriber_name.into(),
                factory: Arc::new(factory),
            });
        self
    }

    pub fn outbox_subscriber(mut self, event_type: impl Into<String>) -> Self {
        self.outbox_subscribers.push(event_type.into());
        self
    }

    pub async fn start(self) -> Result<KernelHandle, KernelError> {
        self.validate()?;

        if !self.outbox_subscribers.is_empty() {
            return Err(KernelError::Unsupported(
                "outbox subscribers require persistent mode".into(),
            ));
        }

        // Factories receive a non-owning gateway handle. Arc::new_cyclic makes
        // it usable before startup returns, without leaking a registry/gateway cycle.
        let mut dispatchers = None;
        let command_gateway = Arc::new_cyclic(|gateway| {
            let factory_gateway = Arc::new(CommandGateway::reference(gateway.clone()));
            let subscribers = registrations::resolve(self.event_subscribers, factory_gateway);
            let event_registry = Arc::new(EventSubscriberRegistry::from_subscribers(subscribers));
            let event_dispatcher = Arc::new(EventDispatcher::new(event_registry));
            let event_gateway = Arc::new(EventGateway::direct(event_dispatcher.clone()));
            let command_registry =
                Arc::new(CommandHandlerRegistry::from_handlers(self.command_handlers));
            let command_dispatcher = Arc::new(CommandDispatcher::new(
                command_registry,
                event_gateway.clone(),
            ));
            dispatchers = Some((command_dispatcher.clone(), event_dispatcher, event_gateway));
            CommandGateway::direct(command_dispatcher)
        });
        let (command_dispatcher, event_dispatcher, event_gateway) = dispatchers.unwrap();

        let state = AppState {
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

    pub fn start_persistent(
        self,
        db_pool: impl Into<Option<io::DbPool>>,
        drain_rounds: usize,
    ) -> Result<PersistentKernelHandle, KernelError> {
        self.validate()?;
        self.validate_persistent_config()?;
        let db_pool = db_pool.into();
        if db_pool.is_none()
            && self
                .config
                .database_url
                .as_ref()
                .is_none_or(|url| url.trim().is_empty())
        {
            return Err(KernelError::Config(
                "database_url or an explicit pool is required".into(),
            ));
        }
        runtime::require_runtime()?;
        let db_pool = match db_pool {
            Some(pool) => pool,
            None => io::build_pool(self.config.database_url.as_deref().ok_or_else(|| {
                KernelError::Config("database_url or an explicit pool is required".into())
            })?)
            .map_err(|error| KernelError::Database(error.to_string()))?,
        };

        let command_registry =
            Arc::new(CommandHandlerRegistry::from_handlers(self.command_handlers));

        let command_store = Arc::new(io::CommandStoreStorage::new(db_pool.clone()));
        let command_recorder = Arc::new(io::CommandRecorder::new(Arc::new(
            io::CommandRecorderRepository::new(command_store),
        )));
        let command_gateway = Arc::new(CommandGateway::two_phased(command_recorder));

        let mut subscribers =
            registrations::resolve(self.event_subscribers, command_gateway.clone());
        if !self.outbox_subscribers.is_empty() {
            let recorder = Arc::new(outbox::io::OutboxRecorder::new(
                outbox::io::recorder_repository(db_pool.clone()),
            ));
            let subscriber = Arc::new(registrations::OutboxSubscriber::new(
                recorder,
                self.config.outbox_content_type.clone(),
            ));
            for event_type in &self.outbox_subscribers {
                subscribers.push((
                    event_type.clone(),
                    OUTBOX_SUBSCRIBER.into(),
                    subscriber.clone(),
                ));
            }
        }

        let event_registry = Arc::new(EventSubscriberRegistry::from_subscribers(subscribers));
        let event_dispatcher = Arc::new(EventDispatcher::new(event_registry));

        let event_store = Arc::new(io::EventStoreStorage::new(db_pool.clone()));
        let event_recorder = Arc::new(io::EventRecorder::new(Arc::new(
            io::EventRecorderRepository::new(event_store),
        )));
        let event_gateway = Arc::new(EventGateway::two_phased(event_recorder));

        let command_dispatcher = Arc::new(CommandDispatcher::new(
            command_registry,
            event_gateway.clone(),
        ));

        let command_storage = Arc::new(io::CommandConsumerStorage::new(db_pool.clone()));
        let command_consumer_repository =
            CommandConsumerRepository::new(command_storage.clone(), command_storage);
        let command_consumer = Arc::new(CommandConsumer::new(
            command_consumer_repository,
            command_dispatcher,
        ));

        let event_storage = Arc::new(io::EventConsumerStorage::new(db_pool.clone()));
        let event_consumer_repository =
            EventConsumerRepository::new(event_storage.clone(), event_storage);
        let event_consumer = Arc::new(EventConsumer::new(
            event_consumer_repository,
            event_dispatcher,
        ));

        let inbox_recorder = Arc::new(InboxRecorder::new(inbox::io::repository(db_pool.clone())));
        let token = CancellationToken::new();
        let workers = runtime::PersistentRuntime::start(
            &self.config,
            db_pool,
            inbox_recorder.clone(),
            command_consumer.clone(),
            event_consumer.clone(),
            !self.outbox_subscribers.is_empty(),
            token.clone(),
        )?;

        Ok(PersistentKernelHandle {
            state: PersistentKernelState {
                command_gateway,
                command_consumer,
                event_consumer,
                drain_rounds,
                inbox_recorder,
                event_gateway,
            },
            token,
            workers,
        })
    }

    fn validate_persistent_config(&self) -> Result<(), KernelError> {
        let config = &self.config;
        for (name, value) in [
            ("amqp_url", &config.amqp_url),
            ("inbox_queue", &config.inbox_queue),
            ("inbox_exchange", &config.inbox_exchange),
            ("outbox_content_type", &config.outbox_content_type),
        ] {
            if value.as_ref().is_some_and(|value| value.trim().is_empty()) {
                return Err(KernelError::Config(format!("{name} must not be blank")));
            }
        }
        let inbox = config.inbox_queue.is_some() || config.inbox_exchange.is_some();
        let outbox = !self.outbox_subscribers.is_empty();
        if inbox
            && (config.amqp_url.is_none()
                || config.inbox_queue.is_none()
                || config.inbox_exchange.is_none())
        {
            return Err(KernelError::Config(
                "inbox intake requires amqp_url, inbox_queue and inbox_exchange".into(),
            ));
        }
        if outbox && (config.amqp_url.is_none() || config.outbox_exchange.is_none()) {
            return Err(KernelError::Config(
                "outbox subscribers require amqp_url and outbox_exchange".into(),
            ));
        }
        if !outbox && (config.outbox_exchange.is_some() || config.outbox_content_type.is_some()) {
            return Err(KernelError::Config(
                "outbox configuration requires an outbox subscriber".into(),
            ));
        }
        if !inbox && !outbox && config.amqp_url.is_some() {
            return Err(KernelError::Config(
                "amqp_url requires inbox intake or an outbox subscriber".into(),
            ));
        }
        Ok(())
    }

    fn validate(&self) -> Result<(), KernelError> {
        let mut command_types = std::collections::HashSet::new();
        for (command_type, _) in &self.command_handlers {
            if !command_types.insert(command_type.clone()) {
                return Err(KernelError::DuplicateCommandHandler(command_type.clone()));
            }
        }

        let mut event_subscribers = std::collections::HashSet::new();
        for registration in &self.event_subscribers {
            let event_type = registration.event_type();
            let subscriber = registration.subscriber_name();
            if subscriber == OUTBOX_SUBSCRIBER {
                return Err(KernelError::ReservedSubscriberName(subscriber.to_string()));
            }
            if !event_subscribers.insert((event_type.to_string(), subscriber.to_string())) {
                return Err(KernelError::DuplicateEventSubscriber {
                    event_type: event_type.to_string(),
                    subscriber: subscriber.to_string(),
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

#[derive(Clone)]
pub struct PersistentKernelState {
    command_gateway: Arc<CommandGateway>,
    command_consumer: Arc<CommandConsumer>,
    event_consumer: Arc<EventConsumer>,
    drain_rounds: usize,
    pub inbox_recorder: Arc<InboxRecorder>,
    event_gateway: Arc<EventGateway>,
}

impl PersistentKernelState {
    pub fn dispatch_command<C>(&self, envelope: NewCommandEnvelope<C>) -> Result<(), KernelError>
    where
        C: ApplicationCommand,
    {
        let envelope = envelope
            .into_gateway_envelope()
            .map_err(|e| KernelError::Command(CommandError::Conversion(e.to_string())))?;

        self.command_gateway.dispatch(envelope)?;

        for _ in 0..self.drain_rounds {
            self.command_consumer
                .consume(&io::ReservableCommandSpec::new(64))
                .map_err(first_command_error)?;
            self.event_consumer
                .consume(&io::ReservableEventSpec::new(64))
                .map_err(first_event_error)?;
        }

        Ok(())
    }

    pub fn event_gateway(&self) -> Arc<EventGateway> {
        self.event_gateway.clone()
    }

    pub fn command_gateway(&self) -> Arc<CommandGateway> {
        self.command_gateway.clone()
    }
}

pub struct PersistentKernelHandle {
    state: PersistentKernelState,
    token: CancellationToken,
    workers: runtime::PersistentRuntime,
}

impl PersistentKernelHandle {
    pub fn state(&self) -> PersistentKernelState {
        self.state.clone()
    }

    pub fn child_token(&self) -> CancellationToken {
        self.token.child_token()
    }

    pub fn command_consumer(&self) -> Arc<CommandConsumer> {
        self.state.command_consumer.clone()
    }

    pub fn event_consumer(&self) -> Arc<EventConsumer> {
        self.state.event_consumer.clone()
    }

    pub fn shutdown(&self) {
        self.token.cancel();
    }

    pub async fn wait(self) -> Result<(), KernelError> {
        self.workers.wait().await
    }
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

pub fn first_command_error(errors: Vec<CommandError>) -> KernelError {
    match errors.into_iter().next() {
        Some(error) => KernelError::Command(error),
        None => KernelError::Worker("command consumer failed without an error".to_string()),
    }
}

pub fn first_event_error(errors: Vec<EventError>) -> KernelError {
    match errors.into_iter().next() {
        Some(error) => KernelError::Event(error),
        None => KernelError::Worker("event consumer failed without an error".to_string()),
    }
}

pub fn block_on_blocking<F>(future: F) -> F::Output
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(future))
}

#[derive(Debug, thiserror::Error)]
pub enum KernelError {
    #[error("invalid kernel configuration: {0}")]
    Config(String),
    #[error("unsupported kernel capability: {0}")]
    Unsupported(String),
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
