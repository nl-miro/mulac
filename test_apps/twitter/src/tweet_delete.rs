pub const COMMAND_NAME: &str = "DeleteTweet";
pub const EVENT_NAME: &str = "TweetDeleted";

mod models {
    use kernel::ApplicationEvent;
    use poem_openapi::Object;
    use serde::{Deserialize, Serialize};
    use uuid::Uuid;

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Command {
        pub tweet_id: Uuid,
        pub author_id: Uuid,
    }

    impl kernel::ApplicationCommand for Command {
        fn command_type(&self) -> &'static str {
            super::COMMAND_NAME
        }
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Object)]
    pub struct Event {
        pub tweet_id: Uuid,
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
    use crate::assembly::io::{Clock, DbPool};
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

            let rows = diesel::update(
                tweets::table.filter(
                    tweets::id
                        .eq(cmd.tweet_id)
                        .and(tweets::author_id.eq(cmd.author_id))
                        .and(tweets::deleted_at.is_null()),
                ),
            )
            .set(tweets::deleted_at.eq(Clock::now()))
            .execute(&mut conn)
            .map_err(|e| CommandError::HandlerExecution(e.to_string()))?;

            if rows == 0 {
                return Err(CommandError::HandlerExecution(
                    "tweet not found".to_string(),
                ));
            }

            Ok(vec![crate::TwitterEvent::TweetDeleted(Event {
                tweet_id: cmd.tweet_id,
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
            interpret_dispatch_error,
            run_blocking,
            //
        },
        //
    };
    use kernel::io::NewCommandMetadata;
    use poem::web::Data;
    use poem_openapi::{ApiResponse, Object, OpenApi, payload::Json};
    use serde::Deserialize;
    use uuid::Uuid;

    #[derive(Debug, Deserialize, Object)]
    pub struct Request {
        pub author_id: Uuid,
    }

    #[derive(ApiResponse)]
    pub enum DeleteResp {
        #[oai(status = 204)]
        NoContent,
    }

    pub struct Api;

    #[OpenApi]
    impl Api {
        #[oai(path = "/tweets/:tweet_id", method = "delete")]
        async fn delete_tweet(
            &self,
            state: Data<&AppState>,
            tweet_id: poem_openapi::param::Path<Uuid>,
            Json(req): Json<Request>,
        ) -> Result<DeleteResp, ApiError> {
            let command_id = Uuid::now_v7();
            let envelope = NewCommandEnvelope {
                command: AppCommand::DeleteTweet(super::models::Command {
                    tweet_id: tweet_id.0,
                    author_id: req.author_id,
                }),
                metadata: NewCommandMetadata {
                    command_id,
                    correlation_id: Some(command_id),
                    causation_id: None,
                    source: Some("test_app_twitter.http".to_string()),
                },
            };
            let mulac = state.mulac.clone();
            run_blocking(move || {
                mulac
                    .dispatch_command(envelope)
                    .map_err(interpret_dispatch_error)
            })
            .await?;
            Ok(DeleteResp::NoContent)
        }
    }
}

pub mod io {
    pub use super::handler::Handler;
    pub use super::http::Api;
    pub use super::models::{Command, Event};
}
