pub mod io {
    pub use super::implementation::Handler;
    pub use super::intention::{Retweet, TweetRetweeted};
    pub use super::ui::Api;
}

mod intention {
    use kernel::{ApplicationCommand, ApplicationEvent};
    use poem_openapi::Object;
    use serde::{Deserialize, Serialize};
    use uuid::Uuid;

    /// Asks the system to publish a retweet of an existing tweet.
    #[derive(Debug, Clone, Serialize, Deserialize, ApplicationCommand)]
    #[command_type = "Retweet"]
    pub struct Retweet {
        pub retweet_id: Uuid,
        pub original_tweet_id: Uuid,
        pub author_id: Uuid,
    }

    impl Retweet {
        pub fn new(retweet_id: Uuid, original_tweet_id: Uuid, author_id: Uuid) -> Self {
            Self {
                retweet_id,
                original_tweet_id,
                author_id,
            }
        }
    }

    /// States that a retweet was published.
    #[derive(Debug, Clone, Serialize, Deserialize, Object, ApplicationEvent)]
    #[event_type = "TweetRetweeted"]
    pub struct TweetRetweeted {
        pub retweet_id: Uuid,
        pub original_tweet_id: Uuid,
        pub author_id: Uuid,
    }
}

mod ui {
    use super::intention::Retweet;
    use crate::AppState;
    use crate::assembly::io::{ApiError, AppError, TweetDto, dispatch_command, fetch_tweet};
    use poem::web::Data;
    use poem_openapi::{Object, OpenApi, payload::Json};
    use serde::Deserialize;
    use uuid::Uuid;

    pub struct Api;

    #[OpenApi]
    impl Api {
        #[oai(path = "/tweets/:original_tweet_id/retweet", method = "post")]
        async fn retweet(
            &self,
            state: Data<&AppState>,
            original_tweet_id: poem_openapi::param::Path<Uuid>,
            Json(request): Json<RetweetRequest>,
        ) -> Result<Json<TweetDto>, ApiError> {
            let retweet_id = Uuid::now_v7();
            let command = request.into_command(retweet_id, original_tweet_id.0);

            dispatch_command(&state.mulac, command)?;

            let response = fetch_tweet(&state.pool, retweet_id)
                .await
                .map_err(AppError::Storage)?;

            Ok(Json(response))
        }
    }

    #[derive(Debug, Deserialize, Object)]
    pub struct RetweetRequest {
        pub author_id: Uuid,
    }

    impl RetweetRequest {
        pub(super) fn into_command(self, retweet_id: Uuid, original_tweet_id: Uuid) -> Retweet {
            Retweet::new(retweet_id, original_tweet_id, self.author_id)
        }
    }
}

mod implementation {
    use super::intention::{Retweet, TweetRetweeted};
    use crate::TwitterEvent;
    use crate::assembly::io::DbPool;
    use derive_new::new;
    use kernel::io::{CommandError, CommandHandlerPort};

    #[derive(new)]
    pub struct Handler {
        pub(super) pool: DbPool,
    }

    impl From<Retweet> for TweetRetweeted {
        fn from(command: Retweet) -> Self {
            Self {
                retweet_id: command.retweet_id,
                original_tweet_id: command.original_tweet_id,
                author_id: command.author_id,
            }
        }
    }

    impl CommandHandlerPort<Retweet, TwitterEvent> for Handler {
        fn execute(&self, command: Retweet) -> Result<Vec<TwitterEvent>, CommandError> {
            self.insert_retweet(&command)?;
            Ok(vec![TwitterEvent::TweetRetweeted(command.into())])
        }
    }

    impl Handler {
        fn insert_retweet(&self, command: &Retweet) -> Result<(), CommandError> {
            insert_retweet(&self.pool, command)
        }
    }

    pub(super) fn insert_retweet(pool: &DbPool, command: &Retweet) -> Result<(), CommandError> {
        use crate::schema::tweets;
        use diesel::prelude::*;

        let mut conn = pool
            .get()
            .map_err(|error| CommandError::Storage(error.to_string()))?;

        let original_exists: bool = diesel::dsl::select(diesel::dsl::exists(
            tweets::table.filter(
                tweets::id
                    .eq(command.original_tweet_id)
                    .and(tweets::deleted_at.is_null()),
            ),
        ))
        .get_result(&mut conn)
        .map_err(|error| CommandError::HandlerExecution(error.to_string()))?;

        if !original_exists {
            return Err(CommandError::HandlerExecution(
                "tweet not found".to_string(),
            ));
        }

        let rows = diesel::insert_into(tweets::table)
            .values((
                tweets::id.eq(command.retweet_id),
                tweets::author_id.eq(command.author_id),
                tweets::content.eq(""),
                tweets::retweeted_from.eq(command.original_tweet_id),
            ))
            .on_conflict_do_nothing()
            .execute(&mut conn)
            .map_err(|error| CommandError::HandlerExecution(error.to_string()))?;

        if rows == 0 {
            return Err(CommandError::HandlerExecution(
                "duplicate retweet_id".to_string(),
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::intention::{Retweet, TweetRetweeted};
    use super::ui::RetweetRequest;
    use uuid::Uuid;

    #[test]
    fn command_and_event_types_match_contract() {
        assert_eq!(Retweet::COMMAND_TYPE, "Retweet");
        assert_eq!(TweetRetweeted::EVENT_TYPE, "TweetRetweeted");
    }

    #[test]
    fn request_carries_author() {
        let author_id = Uuid::now_v7();
        let request = RetweetRequest { author_id };
        assert_eq!(request.author_id, author_id);
    }
}
