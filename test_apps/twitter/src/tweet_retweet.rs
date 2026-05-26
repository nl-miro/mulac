pub const COMMAND_NAME: &str = "Retweet";
pub const EVENT_NAME: &str = "TweetRetweeted";

mod models {
    use kernel::ApplicationEvent;
    use poem_openapi::Object;
    use serde::{Deserialize, Serialize};
    use uuid::Uuid;

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Command {
        pub retweet_id: Uuid,
        pub original_tweet_id: Uuid,
        pub author_id: Uuid,
    }

    impl kernel::ApplicationCommand for Command {
        fn command_type(&self) -> &'static str {
            super::COMMAND_NAME
        }
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Object)]
    pub struct Event {
        pub retweet_id: Uuid,
        pub original_tweet_id: Uuid,
        pub author_id: Uuid,
    }

    impl ApplicationEvent for Event {
        fn event_type(&self) -> &'static str {
            super::EVENT_NAME
        }
    }
}

mod handler {
    use super::models::{Command, Event};
    use crate::assembly::io::DbPool;
    use kernel::io::{CommandError, CommandHandlerPort};

    pub struct Handler {
        pool: DbPool,
    }

    impl Handler {
        pub fn new(pool: DbPool) -> Self {
            Self { pool }
        }
    }

    impl CommandHandlerPort<Command, crate::TwitterEvent> for Handler {
        fn execute(&self, cmd: Command) -> Result<Vec<crate::TwitterEvent>, CommandError> {
            use crate::schema::tweets;
            use diesel::prelude::*;

            let mut conn = self
                .pool
                .get()
                .map_err(|e| CommandError::Storage(e.to_string()))?;

            // Verify the original tweet exists and is not deleted.
            let original_exists: bool = diesel::dsl::select(diesel::dsl::exists(
                tweets::table.filter(
                    tweets::id
                        .eq(cmd.original_tweet_id)
                        .and(tweets::deleted_at.is_null()),
                ),
            ))
            .get_result(&mut conn)
            .map_err(|e| CommandError::HandlerExecution(e.to_string()))?;

            if !original_exists {
                return Err(CommandError::HandlerExecution(
                    "tweet not found".to_string(),
                ));
            }

            let rows = diesel::insert_into(tweets::table)
                .values((
                    tweets::id.eq(cmd.retweet_id),
                    tweets::author_id.eq(cmd.author_id),
                    tweets::content.eq(""),
                    tweets::retweeted_from.eq(cmd.original_tweet_id),
                ))
                .on_conflict_do_nothing()
                .execute(&mut conn)
                .map_err(|e| CommandError::HandlerExecution(e.to_string()))?;

            if rows == 0 {
                return Err(CommandError::HandlerExecution(
                    "duplicate retweet_id".to_string(),
                ));
            }

            Ok(vec![crate::TwitterEvent::TweetRetweeted(Event {
                retweet_id: cmd.retweet_id,
                original_tweet_id: cmd.original_tweet_id,
                author_id: cmd.author_id,
            })])
        }
    }
}

mod infra_diesel {}

mod http {
    use crate::{
        AppState,
        assembly::io::{
            ApiError,
            AppCommand,
            NewCommandEnvelope,
            TweetDto,
            fetch_tweet,
            interpret_dispatch_error,
            run_blocking,
            //
        },
        //
    };
    use kernel::io::NewCommandMetadata;
    use poem::web::Data;
    use poem_openapi::{Object, OpenApi, payload::Json};
    use serde::Deserialize;
    use uuid::Uuid;

    #[derive(Debug, Deserialize, Object)]
    pub struct Request {
        pub author_id: Uuid,
    }

    pub struct Api;

    #[OpenApi]
    impl Api {
        #[oai(path = "/tweets/:original_tweet_id/retweet", method = "post")]
        async fn retweet(
            &self,
            state: Data<&AppState>,
            original_tweet_id: poem_openapi::param::Path<Uuid>,
            Json(req): Json<Request>,
        ) -> Result<Json<TweetDto>, ApiError> {
            let retweet_id = Uuid::now_v7();
            let command_id = Uuid::now_v7();
            let envelope = NewCommandEnvelope {
                command: AppCommand::Retweet(super::models::Command {
                    retweet_id,
                    original_tweet_id: original_tweet_id.0,
                    author_id: req.author_id,
                }),
                metadata: NewCommandMetadata {
                    command_id,
                    correlation_id: Some(command_id),
                    causation_id: None,
                    source: Some("test_app_twitter.http".to_string()),
                },
            };
            let pool = state.pool.clone();
            let mulac = state.mulac.clone();
            run_blocking(move || {
                mulac
                    .dispatch_command(envelope)
                    .map_err(interpret_dispatch_error)
            })
            .await?;
            let tweet = run_blocking(move || {
                fetch_tweet(&pool, retweet_id)
                    .map_err(|e| crate::assembly::io::AppError::Storage(e))
            })
            .await?;
            Ok(Json(tweet))
        }
    }
}

pub mod io {
    pub use super::handler::Handler;
    pub use super::http::Api;
    pub use super::models::{Command, Event};
}
