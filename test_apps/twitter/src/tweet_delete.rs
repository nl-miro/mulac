pub mod io {
    pub use super::implementation::Handler;
    pub use super::intention::{DeleteTweet, TweetDeleted};
    pub use super::ui::Api;
}

mod intention {
    use kernel::{ApplicationCommand, ApplicationEvent};
    use poem_openapi::Object;
    use serde::{Deserialize, Serialize};
    use uuid::Uuid;

    /// Asks the system to delete a tweet if it exists and belongs to the author.
    #[derive(Debug, Clone, Serialize, Deserialize, ApplicationCommand)]
    #[command_type = "DeleteTweet"]
    pub struct DeleteTweet {
        pub tweet_id: Uuid,
        pub author_id: Uuid,
    }

    impl DeleteTweet {
        pub fn new(tweet_id: Uuid, author_id: Uuid) -> Self {
            Self {
                tweet_id,
                author_id,
            }
        }
    }

    /// States that a tweet was deleted.
    #[derive(Debug, Clone, Serialize, Deserialize, Object, ApplicationEvent)]
    #[event_type = "TweetDeleted"]
    pub struct TweetDeleted {
        pub tweet_id: Uuid,
        pub author_id: Uuid,
    }
}

mod ui {
    use super::intention::DeleteTweet;
    use crate::AppState;
    use crate::assembly::io::{ApiError, dispatch_command};
    use poem::web::Data;
    use poem_openapi::{ApiResponse, Object, OpenApi, payload::Json};
    use serde::Deserialize;
    use uuid::Uuid;

    pub struct Api;

    #[OpenApi]
    impl Api {
        #[oai(path = "/tweets/:tweet_id", method = "delete")]
        async fn delete_tweet(
            &self,
            state: Data<&AppState>,
            tweet_id: poem_openapi::param::Path<Uuid>,
            Json(request): Json<DeleteTweetRequest>,
        ) -> Result<DeleteResp, ApiError> {
            dispatch_command(&state.mulac, request.into_command(tweet_id.0))?;
            Ok(DeleteResp::NoContent)
        }
    }

    #[derive(ApiResponse)]
    pub enum DeleteResp {
        #[oai(status = 204)]
        NoContent,
    }

    #[derive(Debug, Deserialize, Object)]
    pub struct DeleteTweetRequest {
        pub author_id: Uuid,
    }

    impl DeleteTweetRequest {
        pub(super) fn into_command(self, tweet_id: Uuid) -> DeleteTweet {
            DeleteTweet::new(tweet_id, self.author_id)
        }
    }
}

mod implementation {
    use super::intention::{DeleteTweet, TweetDeleted};
    use crate::TwitterEvent;
    use crate::assembly::io::{Clock, DbPool};
    use derive_new::new;
    use kernel::io::{CommandError, CommandHandlerPort};

    #[derive(new)]
    pub struct Handler {
        pub(super) pool: DbPool,
    }

    impl From<DeleteTweet> for TweetDeleted {
        fn from(command: DeleteTweet) -> Self {
            Self {
                tweet_id: command.tweet_id,
                author_id: command.author_id,
            }
        }
    }

    impl CommandHandlerPort<DeleteTweet, TwitterEvent> for Handler {
        fn execute(&self, command: DeleteTweet) -> Result<Vec<TwitterEvent>, CommandError> {
            self.soft_delete_tweet(&command)?;
            Ok(vec![TwitterEvent::TweetDeleted(command.into())])
        }
    }

    impl Handler {
        fn soft_delete_tweet(&self, command: &DeleteTweet) -> Result<(), CommandError> {
            soft_delete_tweet(&self.pool, command)
        }
    }

    pub(super) fn soft_delete_tweet(
        pool: &DbPool,
        command: &DeleteTweet,
    ) -> Result<(), CommandError> {
        use crate::schema::tweets;
        use diesel::prelude::*;

        let mut conn = pool
            .get()
            .map_err(|error| CommandError::Storage(error.to_string()))?;

        let rows = diesel::update(
            tweets::table.filter(
                tweets::id
                    .eq(command.tweet_id)
                    .and(tweets::author_id.eq(command.author_id))
                    .and(tweets::deleted_at.is_null()),
            ),
        )
        .set(tweets::deleted_at.eq(Clock::now()))
        .execute(&mut conn)
        .map_err(|error| CommandError::HandlerExecution(error.to_string()))?;

        if rows == 0 {
            return Err(CommandError::HandlerExecution(
                "tweet not found".to_string(),
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::intention::{DeleteTweet, TweetDeleted};
    use super::ui::DeleteTweetRequest;
    use uuid::Uuid;

    #[test]
    fn command_and_event_types_match_contract() {
        assert_eq!(DeleteTweet::COMMAND_TYPE, "DeleteTweet");
        assert_eq!(TweetDeleted::EVENT_TYPE, "TweetDeleted");
    }

    #[test]
    fn request_carries_author() {
        let author_id = Uuid::now_v7();
        let request = DeleteTweetRequest { author_id };
        assert_eq!(request.author_id, author_id);
    }
}
