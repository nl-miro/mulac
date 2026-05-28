pub mod io {
    pub use super::implementation::Handler;
    pub use super::intention::{LikeTweet, TweetLiked};
    pub use super::ui::Api;
}

mod intention {
    use kernel::{ApplicationCommand, ApplicationEvent};
    use poem_openapi::Object;
    use serde::{Deserialize, Serialize};
    use uuid::Uuid;

    /// Asks the system to like a tweet.
    #[derive(Debug, Clone, Serialize, Deserialize, ApplicationCommand)]
    #[command_type = "LikeTweet"]
    pub struct LikeTweet {
        pub user_id: Uuid,
        pub tweet_id: Uuid,
    }

    impl LikeTweet {
        pub fn new(user_id: Uuid, tweet_id: Uuid) -> Self {
            Self { user_id, tweet_id }
        }
    }

    /// States that a tweet received a like.
    #[derive(Debug, Clone, Serialize, Deserialize, Object, ApplicationEvent)]
    #[event_type = "TweetLiked"]
    pub struct TweetLiked {
        pub user_id: Uuid,
        pub tweet_id: Uuid,
    }
}

mod ui {
    use super::intention::LikeTweet;
    use crate::AppState;
    use crate::assembly::io::{ApiError, AppError, LikeDto, dispatch_command, fetch_like};
    use poem::web::Data;
    use poem_openapi::{Object, OpenApi, payload::Json};
    use serde::Deserialize;
    use uuid::Uuid;

    pub struct Api;

    #[OpenApi]
    impl Api {
        #[oai(path = "/tweets/:tweet_id/like", method = "post")]
        async fn like_tweet(
            &self,
            state: Data<&AppState>,
            tweet_id: poem_openapi::param::Path<Uuid>,
            Json(request): Json<LikeTweetRequest>,
        ) -> Result<Json<LikeDto>, ApiError> {
            let user_id = request.user_id;
            let command = request.into_command(tweet_id.0);

            dispatch_command(&state.mulac, command)?;

            let response =
                fetch_like(&state.pool, user_id, tweet_id.0).map_err(AppError::Storage)?;

            Ok(Json(response))
        }
    }

    #[derive(Debug, Deserialize, Object)]
    pub struct LikeTweetRequest {
        pub user_id: Uuid,
    }

    impl LikeTweetRequest {
        pub(super) fn into_command(self, tweet_id: Uuid) -> LikeTweet {
            LikeTweet::new(self.user_id, tweet_id)
        }
    }
}

mod implementation {
    use super::intention::{LikeTweet, TweetLiked};
    use crate::TwitterEvent;
    use crate::assembly::io::DbPool;
    use derive_new::new;
    use kernel::io::{CommandError, CommandHandlerPort};

    #[derive(new)]
    pub struct Handler {
        pub(super) pool: DbPool,
    }

    impl From<LikeTweet> for TweetLiked {
        fn from(command: LikeTweet) -> Self {
            Self {
                user_id: command.user_id,
                tweet_id: command.tweet_id,
            }
        }
    }

    impl CommandHandlerPort<LikeTweet, TwitterEvent> for Handler {
        fn execute(&self, command: LikeTweet) -> Result<Vec<TwitterEvent>, CommandError> {
            let inserted = self.insert_like(&command)?;

            if !inserted {
                return Ok(vec![]);
            }

            Ok(vec![TwitterEvent::TweetLiked(command.into())])
        }
    }

    impl Handler {
        fn insert_like(&self, command: &LikeTweet) -> Result<bool, CommandError> {
            insert_like(&self.pool, command)
        }
    }

    /// Inserts the like if the tweet exists. Returns `true` when a new like row
    /// was created, `false` when it already existed (idempotent no-op).
    pub(super) fn insert_like(pool: &DbPool, command: &LikeTweet) -> Result<bool, CommandError> {
        use crate::schema::{likes, tweets};
        use diesel::prelude::*;

        let mut conn = pool
            .get()
            .map_err(|error| CommandError::Storage(error.to_string()))?;

        let tweet_exists: bool = diesel::dsl::select(diesel::dsl::exists(
            tweets::table.filter(
                tweets::id
                    .eq(command.tweet_id)
                    .and(tweets::deleted_at.is_null()),
            ),
        ))
        .get_result(&mut conn)
        .map_err(|error| CommandError::HandlerExecution(error.to_string()))?;

        if !tweet_exists {
            return Err(CommandError::HandlerExecution(
                "tweet not found".to_string(),
            ));
        }

        let rows = diesel::insert_into(likes::table)
            .values((
                likes::user_id.eq(command.user_id),
                likes::tweet_id.eq(command.tweet_id),
            ))
            .on_conflict_do_nothing()
            .execute(&mut conn)
            .map_err(|error| CommandError::HandlerExecution(error.to_string()))?;

        Ok(rows > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::intention::{LikeTweet, TweetLiked};
    use super::ui::LikeTweetRequest;
    use uuid::Uuid;

    #[test]
    fn command_and_event_types_match_contract() {
        assert_eq!(LikeTweet::COMMAND_TYPE, "LikeTweet");
        assert_eq!(TweetLiked::EVENT_TYPE, "TweetLiked");
    }

    #[test]
    fn request_carries_user() {
        let user_id = Uuid::now_v7();
        let request = LikeTweetRequest { user_id };
        assert_eq!(request.user_id, user_id);
    }
}
