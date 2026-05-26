pub const REOPEN_TODO_COMMAND: &str = "ReopenTodo";
pub const TODO_REOPENED_EVENT: &str = "TodoReopened";

mod models {
    use crate::assembly::io::TodoDto;
    use kernel::ApplicationEvent;
    use poem_openapi::Object;
    use serde::{Deserialize, Serialize};
    use uuid::Uuid;

    #[derive(Debug, Clone, Serialize, Deserialize, Object)]
    pub struct ReopenTodoCommand {
        pub todo_id: Uuid,
    }

    impl kernel::ApplicationCommand for ReopenTodoCommand {
        fn command_type(&self) -> &'static str {
            super::REOPEN_TODO_COMMAND
        }
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Object)]
    pub struct TodoReopened {
        pub todo: TodoDto,
    }

    impl ApplicationEvent for TodoReopened {
        fn event_type(&self) -> &'static str {
            super::TODO_REOPENED_EVENT
        }
    }
}

mod handler {
    use super::models::{ReopenTodoCommand, TodoReopened};
    use crate::assembly::io::{TodoEvent, block_on_blocking};
    use kernel::{CommandError, CommandHandlerPort};
    use sqlx::PgPool;

    pub struct ReopenTodoHandler {
        pool: PgPool,
    }

    impl ReopenTodoHandler {
        pub fn new(pool: PgPool) -> Self {
            Self { pool }
        }
    }

    impl CommandHandlerPort<ReopenTodoCommand, TodoEvent> for ReopenTodoHandler {
        fn execute(&self, command: ReopenTodoCommand) -> Result<Vec<TodoEvent>, CommandError> {
            let pool = self.pool.clone();
            let todo = block_on_blocking(async move {
                super::infra_sqlx_pg::reopen(&pool, command.todo_id).await
            })
            .map_err(|e| CommandError::HandlerExecution(e.to_string()))?;

            Ok(vec![TodoEvent::TodoReopened(TodoReopened { todo })])
        }
    }
}

mod infra_sqlx_pg {
    use crate::assembly::io::{AppError, Clock, TodoDto, TodoRow, TodoStatus};
    use sqlx::PgPool;
    use uuid::Uuid;
    pub async fn reopen(pool: &PgPool, id: Uuid) -> Result<TodoDto, AppError> {
        let sql = "UPDATE todos SET status = $2, updated_at = $3 WHERE id = $1 RETURNING id, title, description, status, created_at, updated_at, due_at";

        let row = sqlx::query_as::<_, TodoRow>(sql)
            .bind(id)
            .bind(TodoStatus::Active.as_str())
            .bind(Clock::now())
            .fetch_optional(pool)
            .await
            .map_err(|e| AppError::Storage(e.into()))?
            .ok_or(AppError::NotFound)?;
        row.try_into()
    }
}

mod http {
    use super::models::ReopenTodoCommand;
    use crate::{
        AppState,
        assembly::io::{
            ApiError, AppCommand, MulacState, NewCommandEnvelope, TodoDto, fetch_todo,
            interpret_dispatch_error,
        },
        //
    };
    use poem::web::Data;
    use poem_openapi::{OpenApi, param::Path, payload::Json};
    use uuid::Uuid;

    fn dispatch_reopen_todo(mulac: &MulacState, id: Uuid) -> Result<(), ApiError> {
        let command_id = Uuid::now_v7();
        let envelope = NewCommandEnvelope {
            command: AppCommand::ReopenTodo(ReopenTodoCommand { todo_id: id }),
            metadata: kernel::NewCommandMetadata {
                command_id,
                correlation_id: Some(command_id),
                causation_id: None,
                source: Some("test_app_todo.http".to_string()),
            },
        };
        mulac
            .dispatch_command(envelope)
            .map_err(|e| ApiError::from(interpret_dispatch_error(e)))
    }

    pub struct Api;

    #[OpenApi]
    impl Api {
        #[oai(path = "/todos/:id/reopen", method = "post")]
        async fn reopen_todo(
            &self,
            state: Data<&AppState>,
            id: Path<Uuid>,
        ) -> Result<Json<TodoDto>, ApiError> {
            dispatch_reopen_todo(&state.mulac, id.0)?;
            Ok(Json(fetch_todo(&state.pool, id.0).await?))
        }
    }
}

pub mod io {
    pub use super::REOPEN_TODO_COMMAND;
    pub use super::TODO_REOPENED_EVENT;
    pub use super::handler::ReopenTodoHandler;
    pub use super::http::Api;
    pub use super::models::{ReopenTodoCommand, TodoReopened};
}
