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
use kernel::io::CommandError as MulacCommandError;
use kernel::{
    ApplicationCommand,
    KernelError, //
};
use poem::{IntoResponse, http::StatusCode};
use poem_openapi::Object;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

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

pub type NewCommandEnvelope = kernel::NewCommandEnvelope<AppCommand>;

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

pub type MulacState = kernel::PersistentKernelState;
pub type MulacHandle = kernel::PersistentKernelHandle;

pub fn start_mulac(pool: DbPool) -> Result<MulacHandle, KernelError> {
    use crate::assembly::infra_diesel::OutboxSubscriber;
    use crate::direct_message_send::io::Handler as SendDmHandler;
    use crate::timeline_fan_out::io::{
        Handler as FanOutHandler,
        Subscriber as FanOutSubscriber,
        //
    };
    use crate::tweet_delete::io::Handler as DeleteTweetHandler;
    use crate::tweet_like::io::Handler as LikeTweetHandler;
    use crate::tweet_post::io::Handler as PostTweetHandler;
    use crate::tweet_retweet::io::Handler as RetweetHandler;
    use crate::tweet_unlike::io::Handler as UnlikeTweetHandler;
    use crate::user_follow::io::Handler as FollowUserHandler;
    use crate::user_unfollow::io::Handler as UnfollowUserHandler;

    kernel::boot(kernel::KernelConfig::default())
        .command_handler("PostTweet", Arc::new(PostTweetHandler::new(pool.clone())))
        .command_handler(
            "DeleteTweet",
            Arc::new(DeleteTweetHandler::new(pool.clone())),
        )
        .command_handler("Retweet", Arc::new(RetweetHandler::new(pool.clone())))
        .command_handler("FollowUser", Arc::new(FollowUserHandler::new(pool.clone())))
        .command_handler(
            "UnfollowUser",
            Arc::new(UnfollowUserHandler::new(pool.clone())),
        )
        .command_handler("LikeTweet", Arc::new(LikeTweetHandler::new(pool.clone())))
        .command_handler(
            "UnlikeTweet",
            Arc::new(UnlikeTweetHandler::new(pool.clone())),
        )
        .command_handler(
            "SendDirectMessage",
            Arc::new(SendDmHandler::new(pool.clone())),
        )
        .command_handler("FanOutTweet", Arc::new(FanOutHandler::new(pool.clone())))
        .event_subscriber(
            "TweetPosted",
            "tweet-posted-outbox",
            Arc::new(OutboxSubscriber::new(pool.clone())) as Arc<dyn kernel::EventSubscriberPort>,
        )
        .event_subscriber(
            "TweetDeleted",
            "tweet-deleted-outbox",
            Arc::new(OutboxSubscriber::new(pool.clone())) as Arc<dyn kernel::EventSubscriberPort>,
        )
        .event_subscriber(
            "TweetRetweeted",
            "tweet-retweeted-outbox",
            Arc::new(OutboxSubscriber::new(pool.clone())) as Arc<dyn kernel::EventSubscriberPort>,
        )
        .event_subscriber(
            "TweetLiked",
            "tweet-liked-outbox",
            Arc::new(OutboxSubscriber::new(pool.clone())) as Arc<dyn kernel::EventSubscriberPort>,
        )
        .event_subscriber(
            "TweetUnliked",
            "tweet-unliked-outbox",
            Arc::new(OutboxSubscriber::new(pool.clone())) as Arc<dyn kernel::EventSubscriberPort>,
        )
        .event_subscriber(
            "UserFollowed",
            "user-followed-outbox",
            Arc::new(OutboxSubscriber::new(pool.clone())) as Arc<dyn kernel::EventSubscriberPort>,
        )
        .event_subscriber(
            "UserUnfollowed",
            "user-unfollowed-outbox",
            Arc::new(OutboxSubscriber::new(pool.clone())) as Arc<dyn kernel::EventSubscriberPort>,
        )
        .event_subscriber(
            "DirectMessageSent",
            "direct-message-sent-outbox",
            Arc::new(OutboxSubscriber::new(pool.clone())) as Arc<dyn kernel::EventSubscriberPort>,
        )
        .event_subscriber_with_command_gateway(
            "TweetPosted",
            "timeline-fan-out",
            move |command_gateway| {
                Arc::new(FanOutSubscriber::new(command_gateway))
                    as Arc<dyn kernel::EventSubscriberPort>
            },
        )
        .start_persistent(pool, 2)
}

pub use kernel::io::{run_command_worker, run_event_worker};
