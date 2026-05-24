pub const COMMAND_NAME: &str = "UnfollowUser";
pub const EVENT_NAME: &str = "UserUnfollowed";

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
            use crate::schema::follows;
            use diesel::prelude::*;

            let mut conn = self
                .pool
                .get()
                .map_err(|e| CommandError::Storage(e.to_string()))?;

            let rows = diesel::delete(follows::table.find((cmd.follower_id, cmd.following_id)))
                .execute(&mut conn)
                .map_err(|e| CommandError::HandlerExecution(e.to_string()))?;

            // Idempotent: relationship absent is a no-op success.
            if rows == 0 {
                return Ok(vec![]);
            }

            Ok(vec![crate::TwitterEvent::UserUnfollowed(Event {
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
            ApiError,
            AppCommand,
            Clock,
            FollowDto,
            NewCommandEnvelope,
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
        pub follower_id: Uuid,
        pub following_id: Uuid,
    }

    pub struct Api;

    #[OpenApi]
    impl Api {
        #[oai(path = "/users/unfollow", method = "post")]
        async fn unfollow_user(
            &self,
            state: Data<&AppState>,
            Json(req): Json<Request>,
        ) -> Result<Json<FollowDto>, ApiError> {
            let follower_id = req.follower_id;
            let following_id = req.following_id;
            let command_id = Uuid::now_v7();
            let envelope = NewCommandEnvelope {
                command: AppCommand::UnfollowUser(super::models::Command {
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
            let mulac = state.mulac.clone();
            run_blocking(move || {
                mulac
                    .dispatch_command(envelope)
                    .map_err(interpret_dispatch_error)
            })
            .await?;
            // Row may be gone (deleted or never existed); synthesize DTO from command inputs.
            Ok(Json(FollowDto {
                follower_id,
                following_id,
                created_at: Clock::now(),
            }))
        }
    }
}

pub mod io {
    pub use super::handler::Handler;
    pub use super::http::Api;
    pub use super::models::{Command, Event};
}
