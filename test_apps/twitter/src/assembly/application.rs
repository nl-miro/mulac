use ::commanding::io::{
    CommandConsumer, CommandConsumerRepository, CommandConsumerStorage, CommandDispatcher,
    CommandError as MulacCommandError, CommandGateway, CommandRecorder, CommandRecorderRepository,
    CommandStoreStorage, ErasedCommandHandler, NewCommandMetadata, ReservableCommandSpec,
    wrap_handler,
};
use ::eventing::io::{
    EventConsumer, EventConsumerRepository, EventConsumerStorage, EventDispatcher, EventGateway,
    EventRecorder, EventRecorderRepository, EventStoreStorage, ReservableEventSpec,
};
use kernel::{
    ApplicationCommand, EventError, EventSubscriberPort, GatewayNewCommandEnvelope, KernelError,
    NewEventEnvelope,
};
use poem::{IntoResponse, http::StatusCode};
use poem_openapi::Object;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::assembly::infra_diesel::DbPool;
use crate::direct_message_send::io::Command as SendDirectMessageCommand;
use crate::timeline_fan_out::io::Command as FanOutTweetCommand;
use crate::tweet_delete::io::Command as DeleteTweetCommand;
use crate::tweet_like::io::Command as LikeTweetCommand;
use crate::tweet_post::io::Command as PostTweetCommand;
use crate::tweet_retweet::io::Command as RetweetCommand;
use crate::tweet_unlike::io::Command as UnlikeTweetCommand;
use crate::user_follow::io::Command as FollowUserCommand;
use crate::user_unfollow::io::Command as UnfollowUserCommand;

// ── Error types ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Object)]
pub struct ErrorBody {
    pub error: String,
}

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("not found")]
    NotFound,
    #[error("validation failed: {0}")]
    Validation(String),
    #[error("{0}")]
    Conflict(String),
    #[error("storage error: {0}")]
    Storage(#[from] anyhow::Error),
}

pub type ApiError = poem::Error;

impl From<AppError> for poem::Error {
    fn from(error: AppError) -> Self {
        let status = match &error {
            AppError::NotFound => StatusCode::NOT_FOUND,
            AppError::Validation(_) => StatusCode::BAD_REQUEST,
            AppError::Conflict(_) => StatusCode::CONFLICT,
            AppError::Storage(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        poem::Error::from_response(
            (
                status,
                poem::web::Json(ErrorBody {
                    error: error.to_string(),
                }),
            )
                .into_response(),
        )
    }
}

pub fn validate_content(content: &str, max_chars: usize) -> Result<(), AppError> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Err(AppError::Validation(
            "content must not be blank".to_string(),
        ));
    }
    if trimmed.chars().count() > max_chars {
        return Err(AppError::Validation(format!(
            "content must be at most {max_chars} characters"
        )));
    }
    Ok(())
}

pub fn interpret_dispatch_error(error: KernelError) -> AppError {
    if let KernelError::Command(MulacCommandError::HandlerExecution(ref message)) = error {
        if message.starts_with("tweet not found") || message.starts_with("user not found") {
            return AppError::NotFound;
        }
        if let Some(rest) = message.strip_prefix("validation failed: ") {
            return AppError::Validation(rest.to_string());
        }
        if message == "cannot follow self" {
            return AppError::Validation(message.clone());
        }
        if message.starts_with("duplicate tweet_id")
            || message.starts_with("duplicate retweet_id")
            || message.starts_with("duplicate message_id")
        {
            return AppError::Conflict(message.clone());
        }
    }
    AppError::Storage(anyhow::anyhow!("command dispatch failed: {error}"))
}

// ── Command types ─────────────────────────────────────────────────────────────

pub trait Command: ApplicationCommand {
    fn entity_id(&self) -> Option<Uuid>;
}

#[derive(Debug)]
pub enum AppCommand {
    PostTweet(PostTweetCommand),
    DeleteTweet(DeleteTweetCommand),
    Retweet(RetweetCommand),
    FollowUser(FollowUserCommand),
    UnfollowUser(UnfollowUserCommand),
    LikeTweet(LikeTweetCommand),
    UnlikeTweet(UnlikeTweetCommand),
    SendDirectMessage(SendDirectMessageCommand),
    FanOutTweet(FanOutTweetCommand),
}

impl serde::Serialize for AppCommand {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::PostTweet(c) => c.serialize(s),
            Self::DeleteTweet(c) => c.serialize(s),
            Self::Retweet(c) => c.serialize(s),
            Self::FollowUser(c) => c.serialize(s),
            Self::UnfollowUser(c) => c.serialize(s),
            Self::LikeTweet(c) => c.serialize(s),
            Self::UnlikeTweet(c) => c.serialize(s),
            Self::SendDirectMessage(c) => c.serialize(s),
            Self::FanOutTweet(c) => c.serialize(s),
        }
    }
}

impl ApplicationCommand for AppCommand {
    fn command_type(&self) -> &'static str {
        match self {
            Self::PostTweet(_) => "PostTweet",
            Self::DeleteTweet(_) => "DeleteTweet",
            Self::Retweet(_) => "Retweet",
            Self::FollowUser(_) => "FollowUser",
            Self::UnfollowUser(_) => "UnfollowUser",
            Self::LikeTweet(_) => "LikeTweet",
            Self::UnlikeTweet(_) => "UnlikeTweet",
            Self::SendDirectMessage(_) => "SendDirectMessage",
            Self::FanOutTweet(_) => "FanOutTweet",
        }
    }
}

impl Command for AppCommand {
    fn entity_id(&self) -> Option<Uuid> {
        match self {
            Self::PostTweet(c) => Some(c.tweet_id),
            Self::Retweet(c) => Some(c.retweet_id),
            Self::SendDirectMessage(c) => Some(c.message_id),
            Self::DeleteTweet(c) => Some(c.tweet_id),
            _ => None,
        }
    }
}

pub struct NewCommandEnvelope {
    pub command: AppCommand,
    pub metadata: NewCommandMetadata,
}

pub async fn run_blocking<F, T>(f: F) -> Result<T, AppError>
where
    F: FnOnce() -> Result<T, AppError> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f).await.map_err(|join_err| {
        AppError::Storage(anyhow::anyhow!("blocking task join failed: {join_err}"))
    })?
}

// ── Mulac state ───────────────────────────────────────────────────────────────

const CONSUMER_BATCH: usize = 64;

#[derive(Clone)]
pub struct MulacState {
    command_gateway: Arc<CommandGateway>,
    command_consumer: Arc<CommandConsumer>,
    event_consumer: Arc<EventConsumer>,
}

impl MulacState {
    pub fn dispatch_command(&self, envelope: NewCommandEnvelope) -> Result<(), KernelError> {
        let gateway_envelope = kernel::NewCommandEnvelope {
            command: envelope.command,
            metadata: envelope.metadata,
        }
        .into_gateway_envelope()
        .map_err(|e| KernelError::Command(MulacCommandError::Conversion(e.to_string())))?;

        self.command_gateway.dispatch(gateway_envelope)?;

        // First round: process the dispatched command + its events.
        self.command_consumer
            .consume(&ReservableCommandSpec::new(CONSUMER_BATCH))
            .map_err(first_command_error)?;
        self.event_consumer
            .consume(&ReservableEventSpec::new(CONSUMER_BATCH))
            .map_err(first_event_error)?;

        // Second round: process any commands dispatched by event subscribers
        // (e.g., FanOutTweet queued by timeline_fan_out when TweetPosted is handled).
        self.command_consumer
            .consume(&ReservableCommandSpec::new(CONSUMER_BATCH))
            .map_err(first_command_error)?;
        self.event_consumer
            .consume(&ReservableEventSpec::new(CONSUMER_BATCH))
            .map_err(first_event_error)?;

        Ok(())
    }

    pub fn command_gateway(&self) -> Arc<CommandGateway> {
        self.command_gateway.clone()
    }
}

pub struct MulacHandle {
    state: MulacState,
    token: CancellationToken,
}

impl MulacHandle {
    pub fn state(&self) -> MulacState {
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
        self.token.cancel();
        Ok(())
    }
}

pub fn start_mulac(pool: DbPool) -> Result<MulacHandle, KernelError> {
    use crate::assembly::infra_diesel::OutboxSubscriber;
    use crate::direct_message_send::io::Handler as SendDmHandler;
    use crate::timeline_fan_out::io::{Handler as FanOutHandler, Subscriber as FanOutSubscriber};
    use crate::tweet_delete::io::Handler as DeleteTweetHandler;
    use crate::tweet_like::io::Handler as LikeTweetHandler;
    use crate::tweet_post::io::Handler as PostTweetHandler;
    use crate::tweet_retweet::io::Handler as RetweetHandler;
    use crate::tweet_unlike::io::Handler as UnlikeTweetHandler;
    use crate::user_follow::io::Handler as FollowUserHandler;
    use crate::user_unfollow::io::Handler as UnfollowUserHandler;

    let command_registry = Arc::new(CommandHandlerRegistry::from_handlers(vec![
        (
            "PostTweet".to_string(),
            wrap_handler(Arc::new(PostTweetHandler::new(pool.clone()))),
        ),
        (
            "DeleteTweet".to_string(),
            wrap_handler(Arc::new(DeleteTweetHandler::new(pool.clone()))),
        ),
        (
            "Retweet".to_string(),
            wrap_handler(Arc::new(RetweetHandler::new(pool.clone()))),
        ),
        (
            "FollowUser".to_string(),
            wrap_handler(Arc::new(FollowUserHandler::new(pool.clone()))),
        ),
        (
            "UnfollowUser".to_string(),
            wrap_handler(Arc::new(UnfollowUserHandler::new(pool.clone()))),
        ),
        (
            "LikeTweet".to_string(),
            wrap_handler(Arc::new(LikeTweetHandler::new(pool.clone()))),
        ),
        (
            "UnlikeTweet".to_string(),
            wrap_handler(Arc::new(UnlikeTweetHandler::new(pool.clone()))),
        ),
        (
            "SendDirectMessage".to_string(),
            wrap_handler(Arc::new(SendDmHandler::new(pool.clone()))),
        ),
        (
            "FanOutTweet".to_string(),
            wrap_handler(Arc::new(FanOutHandler::new(pool.clone()))),
        ),
    ]));

    // Build command gateway first — needed by FanOutSubscriber.
    let command_store = Arc::new(CommandStoreStorage::new(pool.clone()));
    let command_recorder = Arc::new(CommandRecorder::new(Arc::new(
        CommandRecorderRepository::new(command_store),
    )));
    let command_gateway = Arc::new(CommandGateway::two_phased(command_recorder));

    // Build event gateway.
    let event_store = Arc::new(EventStoreStorage::new(pool.clone()));
    let event_recorder = Arc::new(EventRecorder::new(Arc::new(EventRecorderRepository::new(
        event_store,
    ))));
    let event_gateway = Arc::new(EventGateway::two_phased(event_recorder));

    // Build command dispatcher (needs event_gateway).
    let command_dispatcher = Arc::new(CommandDispatcher::new(command_registry, event_gateway));

    // Build event subscriber registry. OutboxSubscribers first, fan-out after.
    let event_registry = Arc::new(EventSubscriberRegistry::from_subscribers(vec![
        (
            "TweetPosted".to_string(),
            "tweet-posted-outbox".to_string(),
            Arc::new(OutboxSubscriber::new(pool.clone())) as Arc<dyn EventSubscriberPort>,
        ),
        (
            "TweetDeleted".to_string(),
            "tweet-deleted-outbox".to_string(),
            Arc::new(OutboxSubscriber::new(pool.clone())) as Arc<dyn EventSubscriberPort>,
        ),
        (
            "TweetRetweeted".to_string(),
            "tweet-retweeted-outbox".to_string(),
            Arc::new(OutboxSubscriber::new(pool.clone())) as Arc<dyn EventSubscriberPort>,
        ),
        (
            "TweetLiked".to_string(),
            "tweet-liked-outbox".to_string(),
            Arc::new(OutboxSubscriber::new(pool.clone())) as Arc<dyn EventSubscriberPort>,
        ),
        (
            "TweetUnliked".to_string(),
            "tweet-unliked-outbox".to_string(),
            Arc::new(OutboxSubscriber::new(pool.clone())) as Arc<dyn EventSubscriberPort>,
        ),
        (
            "UserFollowed".to_string(),
            "user-followed-outbox".to_string(),
            Arc::new(OutboxSubscriber::new(pool.clone())) as Arc<dyn EventSubscriberPort>,
        ),
        (
            "UserUnfollowed".to_string(),
            "user-unfollowed-outbox".to_string(),
            Arc::new(OutboxSubscriber::new(pool.clone())) as Arc<dyn EventSubscriberPort>,
        ),
        (
            "DirectMessageSent".to_string(),
            "direct-message-sent-outbox".to_string(),
            Arc::new(OutboxSubscriber::new(pool.clone())) as Arc<dyn EventSubscriberPort>,
        ),
        // Timeline fan-out after outbox for TweetPosted.
        (
            "TweetPosted".to_string(),
            "timeline-fan-out".to_string(),
            Arc::new(FanOutSubscriber::new(command_gateway.clone()))
                as Arc<dyn EventSubscriberPort>,
        ),
    ]));

    let event_dispatcher = Arc::new(EventDispatcher::new(event_registry));

    let command_storage = Arc::new(CommandConsumerStorage::new(pool.clone()));
    let command_consumer_repository =
        CommandConsumerRepository::new(command_storage.clone(), command_storage);
    let command_consumer = Arc::new(CommandConsumer::new(
        command_consumer_repository,
        command_dispatcher,
    ));

    let event_storage = Arc::new(EventConsumerStorage::new(pool));
    let event_consumer_repository =
        EventConsumerRepository::new(event_storage.clone(), event_storage);
    let event_consumer = Arc::new(EventConsumer::new(
        event_consumer_repository,
        event_dispatcher,
    ));

    Ok(MulacHandle {
        state: MulacState {
            command_gateway,
            command_consumer,
            event_consumer,
        },
        token: CancellationToken::new(),
    })
}

pub async fn run_command_worker(consumer: Arc<CommandConsumer>, token: CancellationToken) {
    loop {
        tokio::select! {
            _ = token.cancelled() => break,
            _ = tokio::time::sleep(Duration::from_secs(1)) => {}
        }
        let c = Arc::clone(&consumer);
        match tokio::task::spawn_blocking(move || c.consume(&ReservableCommandSpec::new(10))).await
        {
            Ok(Ok(())) => {}
            Ok(Err(errs)) => {
                for e in &errs {
                    tracing::error!("command worker: {e}");
                }
            }
            Err(e) => tracing::error!("command worker panicked: {e}"),
        }
    }
}

pub async fn run_event_worker(consumer: Arc<EventConsumer>, token: CancellationToken) {
    loop {
        tokio::select! {
            _ = token.cancelled() => break,
            _ = tokio::time::sleep(Duration::from_secs(1)) => {}
        }
        let c = Arc::clone(&consumer);
        match tokio::task::spawn_blocking(move || c.consume(&ReservableEventSpec::new(10))).await {
            Ok(Ok(())) => {}
            Ok(Err(errs)) => {
                for e in &errs {
                    tracing::error!("event worker: {e}");
                }
            }
            Err(e) => tracing::error!("event worker panicked: {e}"),
        }
    }
}

// ── Internal registries ───────────────────────────────────────────────────────

struct CommandHandlerRegistry {
    handlers: HashMap<String, Arc<dyn ErasedCommandHandler>>,
}

impl CommandHandlerRegistry {
    fn from_handlers(pairs: Vec<(String, Arc<dyn ErasedCommandHandler>)>) -> Self {
        Self {
            handlers: pairs.into_iter().collect(),
        }
    }
}

impl ErasedCommandHandler for CommandHandlerRegistry {
    fn execute(
        &self,
        envelope: &GatewayNewCommandEnvelope,
    ) -> Result<Vec<NewEventEnvelope>, MulacCommandError> {
        let handler = self
            .handlers
            .get(&envelope.command.command_type)
            .ok_or_else(|| {
                MulacCommandError::HandlerNotFound(envelope.command.command_type.clone())
            })?;
        handler.execute(envelope)
    }
}

struct EventSubscriberRegistry {
    subscribers: HashMap<String, Vec<(String, Arc<dyn EventSubscriberPort>)>>,
}

impl EventSubscriberRegistry {
    fn from_subscribers(entries: Vec<(String, String, Arc<dyn EventSubscriberPort>)>) -> Self {
        let mut by_event: HashMap<String, Vec<(String, Arc<dyn EventSubscriberPort>)>> =
            HashMap::new();
        for (event_type, name, subscriber) in entries {
            by_event
                .entry(event_type)
                .or_default()
                .push((name, subscriber));
        }
        Self {
            subscribers: by_event,
        }
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

// ── Helpers ───────────────────────────────────────────────────────────────────

fn first_command_error(errors: Vec<MulacCommandError>) -> KernelError {
    match errors.into_iter().next() {
        Some(e) => KernelError::Command(e),
        None => KernelError::Worker("command consumer failed without an error".to_string()),
    }
}

fn first_event_error(errors: Vec<EventError>) -> KernelError {
    match errors.into_iter().next() {
        Some(e) => KernelError::Event(e),
        None => KernelError::Worker("event consumer failed without an error".to_string()),
    }
}
