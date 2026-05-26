pub(crate) mod assembly;
pub(crate) mod direct_message_send;
mod inbox;
mod outbox;
pub mod schema;
pub(crate) mod timeline_fan_out;
pub(crate) mod tweet_delete;
pub(crate) mod tweet_like;
pub(crate) mod tweet_post;
pub(crate) mod tweet_retweet;
pub(crate) mod tweet_unlike;
pub(crate) mod user_follow;
pub(crate) mod user_unfollow;

use assembly::io::{DbPool, MulacState};
use kernel::ApplicationEvent;
use poem_openapi::Union;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Union)]
#[oai(discriminator_name = "type")]
#[serde(tag = "type", content = "payload")]
pub enum TwitterEvent {
    TweetPosted(tweet_post::io::Event),
    TweetDeleted(tweet_delete::io::Event),
    TweetRetweeted(tweet_retweet::io::Event),
    UserFollowed(user_follow::io::Event),
    UserUnfollowed(user_unfollow::io::Event),
    TweetLiked(tweet_like::io::Event),
    TweetUnliked(tweet_unlike::io::Event),
    DirectMessageSent(direct_message_send::io::Event),
}

impl ApplicationEvent for TwitterEvent {
    fn event_type(&self) -> &'static str {
        match self {
            Self::TweetPosted(e) => e.event_type(),
            Self::TweetDeleted(e) => e.event_type(),
            Self::TweetRetweeted(e) => e.event_type(),
            Self::UserFollowed(e) => e.event_type(),
            Self::UserUnfollowed(e) => e.event_type(),
            Self::TweetLiked(e) => e.event_type(),
            Self::TweetUnliked(e) => e.event_type(),
            Self::DirectMessageSent(e) => e.event_type(),
        }
    }
}

#[derive(Clone)]
pub struct AppState {
    pub pool: DbPool,
    pub mulac: MulacState,
}

impl AppState {
    pub fn new(pool: DbPool, mulac: MulacState) -> Self {
        Self { pool, mulac }
    }
}

pub mod io {
    pub use super::assembly::io::{
        ApiError,
        AppCommand,
        AppError,
        DEFAULT_DATABASE_URL,
        DbPool,
        ErrorBody,
        MulacHandle,
        MulacState,
        NewCommandEnvelope,
        OutboxSubscriber,
        build_pool,
        interpret_dispatch_error,
        run_blocking,
        run_command_worker,
        run_event_worker,
        run_migrations,
        start_mulac,
        validate_content,
        //
    };
    pub use super::direct_message_send::io::{
        Api as DirectMessageSendApi,
        Handler as SendDirectMessageHandler,
        //
    };
    pub use super::inbox::io::Api as InboxApi;
    pub use super::outbox::io::{Api as OutboxApi, OutboxMessageDto};
    pub use super::timeline_fan_out::io::Handler as FanOutTweetHandler;
    pub use super::tweet_delete::io::{
        Api as TweetDeleteApi,
        Handler as DeleteTweetHandler,
        //
    };
    pub use super::tweet_like::io::{
        Api as TweetLikeApi,
        Handler as LikeTweetHandler,
        //
    };
    pub use super::tweet_post::io::{
        Api as TweetPostApi,
        Handler as PostTweetHandler,
        //
    };
    pub use super::tweet_retweet::io::{
        Api as TweetRetweetApi,
        Handler as RetweetHandler,
        //
    };
    pub use super::tweet_unlike::io::{
        Api as TweetUnlikeApi,
        Handler as UnlikeTweetHandler,
        //
    };
    pub use super::user_follow::io::{
        Api as FollowUserApi,
        Handler as FollowUserHandler,
        //
    };
    pub use super::user_unfollow::io::{
        Api as UnfollowUserApi,
        Handler as UnfollowUserHandler,
        //
    };
    pub use super::{AppState, TwitterEvent};
}
