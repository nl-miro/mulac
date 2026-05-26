pub const COMMAND_NAME: &str = "LikeTweet";
pub const EVENT_NAME: &str = "TweetLiked";

mod models {
    use kernel::ApplicationEvent;
    use poem_openapi::Object;
    use serde::{Deserialize, Serialize};
    use uuid::Uuid;

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Command {
        pub user_id: Uuid,
        pub tweet_id: Uuid,
    }

    impl kernel::ApplicationCommand for Command {
        fn command_type(&self) -> &'static str {
            super::COMMAND_NAME
        }
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Object)]
    pub struct Event {
        pub user_id: Uuid,
        pub tweet_id: Uuid,
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
            use crate::schema::{likes, tweets};
            use diesel::prelude::*;

            let mut conn = self
                .pool
                .get()
                .map_err(|e| CommandError::Storage(e.to_string()))?;

            let tweet_exists: bool = diesel::dsl::select(diesel::dsl::exists(
                tweets::table.filter(
                    tweets::id
                        .eq(cmd.tweet_id)
                        .and(tweets::deleted_at.is_null()),
                ),
            ))
            .get_result(&mut conn)
            .map_err(|e| CommandError::HandlerExecution(e.to_string()))?;

            if !tweet_exists {
                return Err(CommandError::HandlerExecution(
                    "tweet not found".to_string(),
                ));
            }

            let rows = diesel::insert_into(likes::table)
                .values((
                    likes::user_id.eq(cmd.user_id),
                    likes::tweet_id.eq(cmd.tweet_id),
                ))
                .on_conflict_do_nothing()
                .execute(&mut conn)
                .map_err(|e| CommandError::HandlerExecution(e.to_string()))?;

            // Idempotent: already liked is a no-op success.
            if rows == 0 {
                return Ok(vec![]);
            }

            Ok(vec![crate::TwitterEvent::TweetLiked(Event {
                user_id: cmd.user_id,
                tweet_id: cmd.tweet_id,
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
            LikeDto,
            NewCommandEnvelope,
            fetch_like,
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
        pub user_id: Uuid,
    }

    pub struct Api;

    #[OpenApi]
    impl Api {
        #[oai(path = "/tweets/:tweet_id/like", method = "post")]
        async fn like_tweet(
            &self,
            state: Data<&AppState>,
            tweet_id: poem_openapi::param::Path<Uuid>,
            Json(req): Json<Request>,
        ) -> Result<Json<LikeDto>, ApiError> {
            let user_id = req.user_id;
            let tweet_id_val = tweet_id.0;
            let command_id = Uuid::now_v7();
            let envelope = NewCommandEnvelope {
                command: AppCommand::LikeTweet(super::models::Command {
                    user_id,
                    tweet_id: tweet_id_val,
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
            let like = run_blocking(move || {
                fetch_like(&pool, user_id, tweet_id_val)
                    .map_err(|e| crate::assembly::io::AppError::Storage(e))
            })
            .await?;
            Ok(Json(like))
        }
    }
}

pub mod io {
    pub use super::handler::Handler;
    pub use super::http::Api;
    pub use super::models::{Command, Event};
}
