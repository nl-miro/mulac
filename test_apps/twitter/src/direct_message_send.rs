pub const COMMAND_NAME: &str = "SendDirectMessage";
pub const EVENT_NAME: &str = "DirectMessageSent";

mod models {
    use poem_openapi::Object;
    use serde::{Deserialize, Serialize};
    use uuid::Uuid;

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Command {
        pub message_id: Uuid,
        pub sender_id: Uuid,
        pub recipient_id: Uuid,
        pub content: String,
    }

    impl kernel::ApplicationCommand for Command {
        fn command_type(&self) -> &'static str {
            super::COMMAND_NAME
        }
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Object)]
    pub struct Event {
        pub message_id: Uuid,
        pub sender_id: Uuid,
        pub recipient_id: Uuid,
        pub content: String,
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
            use crate::schema::direct_messages;
            use diesel::prelude::*;

            let mut conn = self
                .pool
                .get()
                .map_err(|e| CommandError::Storage(e.to_string()))?;

            let rows = diesel::insert_into(direct_messages::table)
                .values((
                    direct_messages::id.eq(cmd.message_id),
                    direct_messages::sender_id.eq(cmd.sender_id),
                    direct_messages::recipient_id.eq(cmd.recipient_id),
                    direct_messages::content.eq(&cmd.content),
                ))
                .on_conflict_do_nothing()
                .execute(&mut conn)
                .map_err(|e| CommandError::HandlerExecution(e.to_string()))?;

            if rows == 0 {
                return Err(CommandError::HandlerExecution(
                    "duplicate message_id".to_string(),
                ));
            }

            Ok(vec![crate::TwitterEvent::DirectMessageSent(Event {
                message_id: cmd.message_id,
                sender_id: cmd.sender_id,
                recipient_id: cmd.recipient_id,
                content: cmd.content,
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
            DirectMessageDto,
            NewCommandEnvelope,
            fetch_direct_message,
            interpret_dispatch_error,
            run_blocking,
            validate_content,
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
        pub sender_id: Uuid,
        pub recipient_id: Uuid,
        pub content: String,
    }

    pub struct Api;

    #[OpenApi]
    impl Api {
        #[oai(path = "/messages/direct", method = "post")]
        async fn send_direct_message(
            &self,
            state: Data<&AppState>,
            Json(req): Json<Request>,
        ) -> Result<Json<DirectMessageDto>, ApiError> {
            validate_content(&req.content, 280)?;
            let message_id = Uuid::now_v7();
            let command_id = Uuid::now_v7();
            let envelope = NewCommandEnvelope {
                command: AppCommand::SendDirectMessage(super::models::Command {
                    message_id,
                    sender_id: req.sender_id,
                    recipient_id: req.recipient_id,
                    content: req.content,
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
            let dm = run_blocking(move || {
                fetch_direct_message(&pool, message_id)
                    .map_err(|e| crate::assembly::io::AppError::Storage(e))
            })
            .await?;
            Ok(Json(dm))
        }
    }
}

pub mod io {
    pub use super::handler::Handler;
    pub use super::http::Api;
    pub use super::models::{Command, Event};
}
