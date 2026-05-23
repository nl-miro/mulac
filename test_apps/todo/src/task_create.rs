pub const CREATE_TODO_COMMAND: &str = "CreateTodo";
pub const TODO_CREATED_EVENT: &str = "TodoCreated";

mod models {
    use chrono::{DateTime, Utc};
    use poem_openapi::Object;
    use serde::{Deserialize, Serialize};
    use uuid::Uuid;

    use crate::assembly::io::TodoDto;

    #[derive(Debug, Clone, Serialize, Deserialize, Object)]
    pub struct CreateTodoCommand {
        pub todo_id: Uuid,
        pub title: String,
        pub description: Option<String>,
        pub due_at: Option<DateTime<Utc>>,
    }

    impl kernel::ApplicationCommand for CreateTodoCommand {
        fn command_type(&self) -> &'static str {
            super::CREATE_TODO_COMMAND
        }
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Object)]
    pub struct TodoCreated {
        pub todo: TodoDto,
    }

    impl kernel::ApplicationEvent for TodoCreated {
        fn event_type(&self) -> &'static str {
            super::TODO_CREATED_EVENT
        }
    }
}

mod handler {
    use crate::assembly::io::{TodoEvent, block_on_blocking};
    use kernel::{CommandError, CommandHandlerPort};
    use sqlx::PgPool;

    use super::models::{CreateTodoCommand, TodoCreated};

    pub struct CreateTodoHandler {
        pool: PgPool,
    }

    impl CreateTodoHandler {
        pub fn new(pool: PgPool) -> Self {
            Self { pool }
        }
    }

    impl CommandHandlerPort<CreateTodoCommand, TodoEvent> for CreateTodoHandler {
        fn execute(&self, command: CreateTodoCommand) -> Result<Vec<TodoEvent>, CommandError> {
            let pool = self.pool.clone();
            let todo = block_on_blocking(async move {
                super::infra_sqlx_pg::create_from_command(&pool, command).await
            })
            .map_err(|e| CommandError::HandlerExecution(e.to_string()))?;

            Ok(vec![TodoEvent::TodoCreated(TodoCreated { todo })])
        }
    }
}

mod infra_sqlx_pg {
    use crate::assembly::io::{AppError, Clock, TodoDto, TodoRow, TodoStatus, validate_title};
    use sqlx::PgPool;

    use super::models::CreateTodoCommand;

    pub async fn create_from_command(
        pool: &PgPool,
        command: CreateTodoCommand,
    ) -> Result<TodoDto, AppError> {
        validate_title(&command.title)?;
        let id = command.todo_id;
        let now = Clock::now();
        let mut tx = pool
            .begin()
            .await
            .map_err(|e| AppError::Storage(e.into()))?;
        let sql = "INSERT INTO todos (id, title, description, status, created_at, updated_at, due_at) VALUES ($1, $2, $3, $4, $5, $5, $6) RETURNING id, title, description, status, created_at, updated_at, due_at";

        let row = sqlx::query_as::<_, TodoRow>(sql)
            .bind(id)
            .bind(command.title.trim())
            .bind(command.description)
            .bind(TodoStatus::Active.as_str())
            .bind(now)
            .bind(command.due_at)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| AppError::Storage(e.into()))?;
        let todo: TodoDto = row.try_into()?;
        tx.commit().await.map_err(|e| AppError::Storage(e.into()))?;
        Ok(todo)
    }
}

mod http {
    use crate::assembly::io::{AppCommand, AppError, NewCommandEnvelope};
    use crate::{
        AppState,
        assembly::io::{ApiError, Command, TodoDto, fetch_todo, interpret_dispatch_error},
    };
    use chrono::{DateTime, Utc};
    use kernel::NewCommandMetadata;
    use poem::web::Data;
    use poem_openapi::{Object, OpenApi, payload::Json};
    use serde::{Deserialize, Serialize};
    use uuid::Uuid;

    use super::models::CreateTodoCommand;

    #[derive(Debug, Clone, Serialize, Deserialize, Object)]
    pub struct CreateTodoRequest {
        pub title: String,
        pub description: Option<String>,
        pub due_at: Option<DateTime<Utc>>,
    }

    impl TryFrom<CreateTodoRequest> for NewCommandEnvelope {
        type Error = AppError;

        fn try_from(request: CreateTodoRequest) -> Result<Self, Self::Error> {
            let todo_id = Uuid::now_v7();
            let command_id = Uuid::now_v7();
            Ok(NewCommandEnvelope {
                command: AppCommand::CreateTodo(CreateTodoCommand {
                    todo_id,
                    title: request.title,
                    description: request.description,
                    due_at: request.due_at,
                }),
                metadata: NewCommandMetadata {
                    command_id,
                    correlation_id: Some(command_id),
                    causation_id: None,
                    source: Some("test_app_todo.http".to_string()),
                },
            })
        }
    }

    pub struct Api;

    #[OpenApi]
    impl Api {
        #[oai(path = "/todos", method = "post")]
        async fn create_todo(
            &self,
            state: Data<&AppState>,
            Json(request): Json<CreateTodoRequest>,
        ) -> Result<Json<TodoDto>, ApiError> {
            let envelope: NewCommandEnvelope = request.try_into()?;
            let todo_id = envelope.command.todo_id();
            state
                .mulac
                .dispatch_command(envelope)
                .map_err(interpret_dispatch_error)?;
            let todo = fetch_todo(&state.pool, todo_id).await?;
            Ok(Json(todo))
        }
    }
}

pub mod io {
    pub use super::CREATE_TODO_COMMAND;
    pub use super::TODO_CREATED_EVENT;
    pub use super::handler::CreateTodoHandler;
    pub use super::http::Api;
    pub use super::models::{CreateTodoCommand, TodoCreated};
}
