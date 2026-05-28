pub mod io {
    pub use super::implementation::Handler;
    pub use super::intention::{PostTweet, TweetPosted};
    pub use super::ui::Api;
}

mod intention {
    use crate::assembly::io::{AppError, validate_content};
    use kernel::{ApplicationCommand, ApplicationEvent};
    use poem_openapi::Object;
    use serde::{Deserialize, Serialize};
    use uuid::Uuid;

    const MAX_TWEET_CONTENT_CHARS: usize = 280;

    /// Asks the system to publish a tweet for the given author.
    #[derive(Debug, Clone, Serialize, Deserialize, ApplicationCommand)]
    #[command_type = "PostTweet"]
    pub struct PostTweet {
        pub tweet_id: Uuid,
        pub author_id: Uuid,
        pub content: String,
    }

    impl PostTweet {
        pub fn new(tweet_id: Uuid, author_id: Uuid, content: String) -> Self {
            Self {
                tweet_id,
                author_id,
                content,
            }
        }

        /// A tweet must contain non-blank text and stay within the published
        /// character limit.
        pub fn validate(self) -> Result<Self, AppError> {
            validate_content(&self.content, MAX_TWEET_CONTENT_CHARS)?;
            Ok(self)
        }
    }

    /// States that a tweet was published.
    #[derive(Debug, Clone, Serialize, Deserialize, Object, ApplicationEvent)]
    #[event_type = "TweetPosted"]
    pub struct TweetPosted {
        pub tweet_id: Uuid,
        pub author_id: Uuid,
        pub content: String,
    }
}

mod ui {
    use super::intention::PostTweet;
    use crate::AppState;
    use crate::assembly::io::{ApiError, AppError, TweetDto, dispatch_command, fetch_tweet};
    use poem::web::Data;
    use poem_openapi::{Object, OpenApi, payload::Json};
    use serde::Deserialize;
    use uuid::Uuid;

    pub struct Api;

    #[OpenApi]
    impl Api {
        #[oai(path = "/tweets", method = "post")]
        async fn post_tweet(
            &self,
            state: Data<&AppState>,
            Json(request): Json<PostTweetRequest>,
        ) -> Result<Json<TweetDto>, ApiError> {
            let tweet_id = Uuid::now_v7();
            let cmd = request.into_command(tweet_id);

            dispatch_command(&state.mulac, cmd)?;

            let response = fetch_tweet(&state.pool, tweet_id)
                .await
                .map_err(AppError::Storage)?;

            Ok(Json(response))
        }
    }

    #[derive(Debug, Deserialize, Object)]
    pub struct PostTweetRequest {
        pub author_id: Uuid,
        pub content: String,
    }

    impl PostTweetRequest {
        pub(super) fn into_command(self, tweet_id: Uuid) -> PostTweet {
            PostTweet::new(tweet_id, self.author_id, self.content)
        }
    }
}

mod implementation {
    use super::intention::{PostTweet, TweetPosted};
    use crate::TwitterEvent;
    use crate::assembly::io::DbPool;
    use derive_new::new;
    use kernel::io::{CommandError, CommandHandlerPort};

    #[derive(new)]
    pub struct Handler {
        pub(super) pool: DbPool,
    }

    impl From<PostTweet> for TweetPosted {
        fn from(command: PostTweet) -> Self {
            Self {
                tweet_id: command.tweet_id,
                author_id: command.author_id,
                content: command.content,
            }
        }
    }

    impl CommandHandlerPort<PostTweet, TwitterEvent> for Handler {
        fn execute(&self, command: PostTweet) -> Result<Vec<TwitterEvent>, CommandError> {
            let command = command.validate().map_err(CommandError::from)?;
            self.insert_tweet(&command)?;

            Ok(vec![TwitterEvent::TweetPosted(command.into())])
        }
    }

    impl Handler {
        fn insert_tweet(&self, command: &PostTweet) -> Result<(), CommandError> {
            insert_tweet(&self.pool, command)
        }
    }

    pub(super) fn insert_tweet(pool: &DbPool, cmd: &PostTweet) -> Result<(), CommandError> {
        use crate::schema::tweets;
        use diesel::prelude::*;

        let mut conn = pool
            .get()
            .map_err(|e| CommandError::Storage(e.to_string()))?;

        let rows = diesel::insert_into(tweets::table)
            .values((
                tweets::id.eq(cmd.tweet_id),
                tweets::author_id.eq(cmd.author_id),
                tweets::content.eq(&cmd.content),
            ))
            .on_conflict_do_nothing()
            .execute(&mut conn)
            .map_err(|e| CommandError::HandlerExecution(e.to_string()))?;

        if rows == 0 {
            return Err(CommandError::HandlerExecution(
                "duplicate tweet_id".to_string(),
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::intention::{PostTweet, TweetPosted};
    use super::ui::PostTweetRequest;
    use uuid::Uuid;

    fn sample_command(content: &str) -> PostTweet {
        PostTweet::new(Uuid::now_v7(), Uuid::now_v7(), content.to_string())
    }

    #[test]
    fn command_and_event_types_match_contract() {
        assert_eq!(PostTweet::COMMAND_TYPE, "PostTweet");
        assert_eq!(TweetPosted::EVENT_TYPE, "TweetPosted");
    }

    #[test]
    fn validate_rejects_blank_content() {
        let result = sample_command("   ").validate();

        assert!(result.is_err(), "blank content should be rejected");
    }

    #[test]
    fn validate_accepts_unicode_content_at_limit() {
        let content = "é".repeat(280);
        let validated = sample_command(&content)
            .validate()
            .expect("280 code points should be accepted");

        assert_eq!(validated.content, content);
    }

    #[test]
    fn request_carries_author_and_content() {
        let author_id = Uuid::now_v7();
        let request = PostTweetRequest {
            author_id,
            content: "hello world".to_string(),
        };

        assert_eq!(request.author_id, author_id);
        assert_eq!(request.content, "hello world");
    }
}
