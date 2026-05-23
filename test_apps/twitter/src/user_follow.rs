pub const COMMAND_NAME: &str = "FollowUser";
pub const EVENT_NAME: &str = "UserFollowed";

mod models {
    use poem_openapi::Object;
    use serde::{Deserialize, Serialize};
    use uuid::Uuid;

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Command {
        pub follower_id: Uuid,
        pub following_id: Uuid,
    }

    impl kernel::ApplicationCommand for Command {
        fn command_type(&self) -> &'static str {
            super::COMMAND_NAME
        }
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Object)]
    pub struct Event {
        pub follower_id: Uuid,
        pub following_id: Uuid,
    }

    impl kernel::ApplicationEvent for Event {
        fn event_type(&self) -> &'static str {
            super::EVENT_NAME
        }
    }
}

mod handler {
    use super::models::{Command, Event};
    use crate::assembly::io::DbPool;
    use commanding::io::{CommandError, CommandHandlerPort};

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
            use crate::schema::follows;
            use diesel::prelude::*;

            if cmd.follower_id == cmd.following_id {
                return Err(CommandError::HandlerExecution(
                    "cannot follow self".to_string(),
                ));
            }

            let mut conn = self
                .pool
                .get()
                .map_err(|e| CommandError::Storage(e.to_string()))?;

            let rows = diesel::insert_into(follows::table)
                .values((
                    follows::follower_id.eq(cmd.follower_id),
                    follows::following_id.eq(cmd.following_id),
                ))
                .on_conflict_do_nothing()
                .execute(&mut conn)
                .map_err(|e| CommandError::HandlerExecution(e.to_string()))?;

            // Idempotent: already following is a no-op success.
            if rows == 0 {
                return Ok(vec![]);
            }

            Ok(vec![crate::TwitterEvent::UserFollowed(Event {
                follower_id: cmd.follower_id,
                following_id: cmd.following_id,
            })])
        }
    }
}

mod infra_diesel {}

mod http {
    use crate::{
        AppState,
        assembly::io::{
            ApiError, AppCommand, FollowDto, NewCommandEnvelope, fetch_follow,
            interpret_dispatch_error, run_blocking,
        },
    };
    use commanding::io::NewCommandMetadata;
    use poem::web::Data;
    use poem_openapi::{Object, OpenApi, payload::Json};
    use serde::Deserialize;
    use uuid::Uuid;

    #[derive(Debug, Deserialize, Object)]
    pub struct Request {
        pub follower_id: Uuid,
        pub following_id: Uuid,
    }

    pub struct Api;

    #[OpenApi]
    impl Api {
        #[oai(path = "/users/follow", method = "post")]
        async fn follow_user(
            &self,
            state: Data<&AppState>,
            Json(req): Json<Request>,
        ) -> Result<Json<FollowDto>, ApiError> {
            let follower_id = req.follower_id;
            let following_id = req.following_id;
            let command_id = Uuid::now_v7();
            let envelope = NewCommandEnvelope {
                command: AppCommand::FollowUser(super::models::Command {
                    follower_id,
                    following_id,
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
            let follow = run_blocking(move || {
                fetch_follow(&pool, follower_id, following_id)
                    .map_err(|e| crate::assembly::io::AppError::Storage(e))
            })
            .await?;
            Ok(Json(follow))
        }
    }
}

pub mod io {
    pub use super::handler::Handler;
    pub use super::http::Api;
    pub use super::models::{Command, Event};
    pub use super::{COMMAND_NAME, EVENT_NAME};
}
