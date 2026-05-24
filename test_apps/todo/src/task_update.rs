pub const UPDATE_TODO_COMMAND: &str = "UpdateTodo";
pub const TODO_UPDATED_EVENT: &str = "TodoUpdated";

mod models {
    use crate::assembly::io::TodoDto;
    use poem_openapi::Object;
    use serde::{Deserialize, Serialize};
    use uuid::Uuid;

    #[derive(Debug, Clone, Serialize, Deserialize, Object)]
    pub struct UpdateTodoCommand {
        pub todo_id: Uuid,
        pub title: String,
        pub description: Option<String>,
    }

    impl kernel::ApplicationCommand for UpdateTodoCommand {
        fn command_type(&self) -> &'static str {
            super::UPDATE_TODO_COMMAND
        }
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Object)]
    pub struct TodoUpdated {
        pub todo: TodoDto,
    }

    impl kernel::ApplicationEvent for TodoUpdated {
        fn event_type(&self) -> &'static str {
            super::TODO_UPDATED_EVENT
        }
    }
}

mod handler {
    use super::models::{TodoUpdated, UpdateTodoCommand};
    use crate::assembly::io::{TodoEvent, block_on_blocking};
    use kernel::{CommandError, CommandHandlerPort};
    use sqlx::PgPool;

    pub struct UpdateTodoHandler {
        pool: PgPool,
    }

    impl UpdateTodoHandler {
        pub fn new(pool: PgPool) -> Self {
            Self { pool }
        }
    }

    impl CommandHandlerPort<UpdateTodoCommand, TodoEvent> for UpdateTodoHandler {
        fn execute(&self, command: UpdateTodoCommand) -> Result<Vec<TodoEvent>, CommandError> {
            let pool = self.pool.clone();
            let todo = block_on_blocking(async move {
                super::infra_sqlx_pg::update_from_command(&pool, command).await
            })
            .map_err(|e| CommandError::HandlerExecution(e.to_string()))?;

            Ok(vec![TodoEvent::TodoUpdated(TodoUpdated { todo })])
        }
    }
}

mod infra_sqlx_pg {
    use super::models::UpdateTodoCommand;
    use crate::assembly::io::{
        AppError,
        Clock,
        TodoDto,
        TodoRow,
        validate_title,
        //
    };
    use sqlx::PgPool;

    pub async fn update_from_command(
        pool: &PgPool,
        command: UpdateTodoCommand,
    ) -> Result<TodoDto, AppError> {
        validate_title(&command.title)?;
        let sql = "UPDATE todos SET title = $2, description = $3, updated_at = $4 WHERE id = $1 RETURNING id, title, description, status, created_at, updated_at, due_at";

        let row = sqlx::query_as::<_, TodoRow>(sql)
            .bind(command.todo_id)
            .bind(command.title.trim())
            .bind(command.description)
            .bind(Clock::now())
            .fetch_optional(pool)
            .await
            .map_err(|e| AppError::Storage(e.into()))?
            .ok_or(AppError::NotFound)?;
        row.try_into()
    }
}

mod http {
    use super::models::UpdateTodoCommand;
    use crate::{
        AppState,
        assembly::io::{
            ApiError, AppCommand, AppError, MulacState, NewCommandEnvelope, TodoDto, fetch_todo,
            interpret_dispatch_error,
        },
        //
    };
    use poem::web::Data;
    use poem_openapi::{Object, OpenApi, param::Path, payload::Json};
    use serde::{Deserialize, Serialize};
    use uuid::Uuid;

    #[derive(Debug, Clone, Serialize, Deserialize, Object)]
    pub struct UpdateTodoRequest {
        pub title: String,
        pub description: Option<String>,
    }

    fn dispatch_update_todo(
        mulac: &MulacState,
        id: Uuid,
        request: UpdateTodoRequest,
    ) -> Result<Uuid, AppError> {
        let command_id = Uuid::now_v7();
        let envelope = NewCommandEnvelope {
            command: AppCommand::UpdateTodo(UpdateTodoCommand {
                todo_id: id,
                title: request.title,
                description: request.description,
            }),
            metadata: kernel::NewCommandMetadata {
                command_id,
                correlation_id: Some(command_id),
                causation_id: None,
                source: Some("test_app_todo.http".to_string()),
            },
        };
        mulac
            .dispatch_command(envelope)
            .map_err(interpret_dispatch_error)?;
        Ok(id)
    }

    pub struct Api;

    #[OpenApi]
    impl Api {
        #[oai(path = "/todos/:id", method = "put")]
        async fn update_todo(
            &self,
            state: Data<&AppState>,
            id: Path<Uuid>,
            Json(request): Json<UpdateTodoRequest>,
        ) -> Result<Json<TodoDto>, ApiError> {
            let todo_id = dispatch_update_todo(&state.mulac, id.0, request)?;
            Ok(Json(fetch_todo(&state.pool, todo_id).await?))
        }
    }
}

pub mod io {
    pub use super::TODO_UPDATED_EVENT;
    pub use super::UPDATE_TODO_COMMAND;
    pub use super::handler::UpdateTodoHandler;
    pub use super::http::Api;
    pub use super::models::{TodoUpdated, UpdateTodoCommand};
}
