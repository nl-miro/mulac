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

use assembly::prelude::{DbPool, MulacState};
use kernel::ApplicationEvent;
use poem_openapi::Union;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Union)]
#[oai(discriminator_name = "type")]
#[serde(tag = "type", content = "payload")]
pub enum TwitterEvent {
    TweetPosted(tweet_post::prelude::Event),
    TweetDeleted(tweet_delete::prelude::Event),
    TweetRetweeted(tweet_retweet::prelude::Event),
    UserFollowed(user_follow::prelude::Event),
    UserUnfollowed(user_unfollow::prelude::Event),
    TweetLiked(tweet_like::prelude::Event),
    TweetUnliked(tweet_unlike::prelude::Event),
    DirectMessageSent(direct_message_send::prelude::Event),
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

pub mod prelude {
    pub use super::assembly::prelude::{
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
    pub use super::direct_message_send::prelude::{
        Api as DirectMessageSendApi,
        Handler as SendDirectMessageHandler,
        //
    };
    pub use super::inbox::prelude::Api as InboxApi;
    pub use super::outbox::prelude::{Api as OutboxApi, OutboxMessageDto};
    pub use super::timeline_fan_out::prelude::Handler as FanOutTweetHandler;
    pub use super::tweet_delete::prelude::{
        Api as TweetDeleteApi,
        Handler as DeleteTweetHandler,
        //
    };
    pub use super::tweet_like::prelude::{
        Api as TweetLikeApi,
        Handler as LikeTweetHandler,
        //
    };
    pub use super::tweet_post::prelude::{
        Api as TweetPostApi,
        Handler as PostTweetHandler,
        //
    };
    pub use super::tweet_retweet::prelude::{
        Api as TweetRetweetApi,
        Handler as RetweetHandler,
        //
    };
    pub use super::tweet_unlike::prelude::{
        Api as TweetUnlikeApi,
        Handler as UnlikeTweetHandler,
        //
    };
    pub use super::user_follow::prelude::{
        Api as FollowUserApi,
        Handler as FollowUserHandler,
        //
    };
    pub use super::user_unfollow::prelude::{
        Api as UnfollowUserApi,
        Handler as UnfollowUserHandler,
        //
    };
    pub use super::{AppState, TwitterEvent};
}
