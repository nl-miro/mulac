pub mod io {
    pub use super::implementation::Handler;
    pub use super::intention::{TweetUnliked, UnlikeTweet};
    pub use super::ui::Api;
}

mod intention {
    use kernel::{ApplicationCommand, ApplicationEvent};
    use poem_openapi::Object;
    use serde::{Deserialize, Serialize};
    use uuid::Uuid;

    /// Asks the system to remove a like from a tweet.
    #[derive(Debug, Clone, Serialize, Deserialize, ApplicationCommand)]
    #[command_type = "UnlikeTweet"]
    pub struct UnlikeTweet {
        pub user_id: Uuid,
        pub tweet_id: Uuid,
    }

    impl UnlikeTweet {
        pub fn new(user_id: Uuid, tweet_id: Uuid) -> Self {
            Self { user_id, tweet_id }
        }
    }

    /// States that a like was removed from a tweet.
    #[derive(Debug, Clone, Serialize, Deserialize, Object, ApplicationEvent)]
    #[event_type = "TweetUnliked"]
    pub struct TweetUnliked {
        pub user_id: Uuid,
        pub tweet_id: Uuid,
    }
}

mod ui {
    use super::intention::UnlikeTweet;
    use crate::AppState;
    use crate::assembly::io::{ApiError, dispatch_command};
    use poem::web::Data;
    use poem_openapi::{ApiResponse, Object, OpenApi, payload::Json};
    use serde::Deserialize;
    use uuid::Uuid;

    pub struct Api;

    #[OpenApi]
    impl Api {
        #[oai(path = "/tweets/:tweet_id/like", method = "delete")]
        async fn unlike_tweet(
            &self,
            state: Data<&AppState>,
            tweet_id: poem_openapi::param::Path<Uuid>,
            Json(request): Json<UnlikeTweetRequest>,
        ) -> Result<UnlikeResp, ApiError> {
            dispatch_command(&state.mulac, request.into_command(tweet_id.0))?;
            Ok(UnlikeResp::NoContent)
        }
    }

    #[derive(ApiResponse)]
    pub enum UnlikeResp {
        #[oai(status = 204)]
        NoContent,
    }

    #[derive(Debug, Deserialize, Object)]
    pub struct UnlikeTweetRequest {
        pub user_id: Uuid,
    }

    impl UnlikeTweetRequest {
        pub(super) fn into_command(self, tweet_id: Uuid) -> UnlikeTweet {
            UnlikeTweet::new(self.user_id, tweet_id)
        }
    }
}

mod implementation {
    use super::intention::{TweetUnliked, UnlikeTweet};
    use crate::TwitterEvent;
    use crate::assembly::io::DbPool;
    use derive_new::new;
    use kernel::io::{CommandError, CommandHandlerPort};

    #[derive(new)]
    pub struct Handler {
        pub(super) pool: DbPool,
    }

    impl From<UnlikeTweet> for TweetUnliked {
        fn from(command: UnlikeTweet) -> Self {
            Self {
                user_id: command.user_id,
                tweet_id: command.tweet_id,
            }
        }
    }

    impl CommandHandlerPort<UnlikeTweet, TwitterEvent> for Handler {
        fn execute(&self, command: UnlikeTweet) -> Result<Vec<TwitterEvent>, CommandError> {
            let deleted = self.delete_like(&command)?;

            if !deleted {
                return Ok(vec![]);
            }

            Ok(vec![TwitterEvent::TweetUnliked(command.into())])
        }
    }

    impl Handler {
        fn delete_like(&self, command: &UnlikeTweet) -> Result<bool, CommandError> {
            delete_like(&self.pool, command)
        }
    }

    /// Deletes the like if the tweet exists. Returns `true` when a like row was
    /// removed, `false` when no like existed (idempotent no-op).
    pub(super) fn delete_like(pool: &DbPool, command: &UnlikeTweet) -> Result<bool, CommandError> {
        use crate::schema::{likes, tweets};
        use diesel::prelude::*;

        let mut conn = pool
            .get()
            .map_err(|error| CommandError::Storage(error.to_string()))?;

        let tweet_exists: bool = diesel::dsl::select(diesel::dsl::exists(
            tweets::table.filter(tweets::id.eq(command.tweet_id)),
        ))
        .get_result(&mut conn)
        .map_err(|error| CommandError::HandlerExecution(error.to_string()))?;

        if !tweet_exists {
            return Err(CommandError::HandlerExecution(
                "tweet not found".to_string(),
            ));
        }

        let rows = diesel::delete(likes::table.find((command.user_id, command.tweet_id)))
            .execute(&mut conn)
            .map_err(|error| CommandError::HandlerExecution(error.to_string()))?;

        Ok(rows > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::intention::{TweetUnliked, UnlikeTweet};
    use super::ui::UnlikeTweetRequest;
    use uuid::Uuid;

    #[test]
    fn command_and_event_types_match_contract() {
        assert_eq!(UnlikeTweet::COMMAND_TYPE, "UnlikeTweet");
        assert_eq!(TweetUnliked::EVENT_TYPE, "TweetUnliked");
    }

    #[test]
    fn request_carries_user() {
        let user_id = Uuid::now_v7();
        let request = UnlikeTweetRequest { user_id };
        assert_eq!(request.user_id, user_id);
    }
}
