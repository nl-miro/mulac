pub const DELETE_TODO_COMMAND: &str = "DeleteTodo";
pub const TODO_DELETED_EVENT: &str = "TodoDeleted";

mod models {
    use crate::assembly::io::TodoDto;
    use poem_openapi::Object;
    use serde::{Deserialize, Serialize};
    use uuid::Uuid;

    #[derive(Debug, Clone, Serialize, Deserialize, Object)]
    pub struct DeleteTodoCommand {
        pub todo_id: Uuid,
    }

    impl kernel::ApplicationCommand for DeleteTodoCommand {
        fn command_type(&self) -> &'static str {
            super::DELETE_TODO_COMMAND
        }
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Object)]
    pub struct TodoDeleted {
        pub todo: TodoDto,
    }

    impl kernel::ApplicationEvent for TodoDeleted {
        fn event_type(&self) -> &'static str {
            super::TODO_DELETED_EVENT
        }
    }
}

mod handler {
    use super::models::{DeleteTodoCommand, TodoDeleted};
    use crate::assembly::io::{TodoEvent, block_on_blocking};
    use kernel::{CommandError, CommandHandlerPort};
    use sqlx::PgPool;

    pub struct DeleteTodoHandler {
        pool: PgPool,
    }

    impl DeleteTodoHandler {
        pub fn new(pool: PgPool) -> Self {
            Self { pool }
        }
    }

    impl CommandHandlerPort<DeleteTodoCommand, TodoEvent> for DeleteTodoHandler {
        fn execute(&self, command: DeleteTodoCommand) -> Result<Vec<TodoEvent>, CommandError> {
            let pool = self.pool.clone();
            let todo = block_on_blocking(async move {
                super::infra_sqlx_pg::delete_from_command(&pool, command).await
            })
            .map_err(|e| CommandError::HandlerExecution(e.to_string()))?;

            Ok(vec![TodoEvent::TodoDeleted(TodoDeleted { todo })])
        }
    }
}

mod infra_sqlx_pg {
    use super::models::DeleteTodoCommand;
    use crate::assembly::io::{AppError, TodoDto, TodoRow};
    use sqlx::PgPool;

    pub async fn delete_from_command(
        pool: &PgPool,
        command: DeleteTodoCommand,
    ) -> Result<TodoDto, AppError> {
        let sql = "DELETE FROM todos WHERE id = $1 RETURNING id, title, description, status, created_at, updated_at, due_at";

        let row = sqlx::query_as::<_, TodoRow>(sql)
            .bind(command.todo_id)
            .fetch_optional(pool)
            .await
            .map_err(|e| AppError::Storage(e.into()))?
            .ok_or(AppError::NotFound)?;
        row.try_into()
    }
}

mod http {
    use super::models::DeleteTodoCommand;
    use crate::{
        AppState,
        assembly::io::{
            ApiError, AppCommand, AppError, MulacState, NewCommandEnvelope,
            interpret_dispatch_error,
        },
        //
    };
    use poem::web::Data;
    use poem_openapi::{ApiResponse, OpenApi, param::Path};
    use uuid::Uuid;

    #[derive(ApiResponse)]
    enum DeleteResponse {
        #[oai(status = 204)]
        NoContent,
    }

    fn dispatch_delete_todo(mulac: &MulacState, id: Uuid) -> Result<(), AppError> {
        let command_id = Uuid::now_v7();
        let envelope = NewCommandEnvelope {
            command: AppCommand::DeleteTodo(DeleteTodoCommand { todo_id: id }),
            metadata: kernel::NewCommandMetadata {
                command_id,
                correlation_id: Some(command_id),
                causation_id: None,
                source: Some("test_app_todo.http".to_string()),
            },
        };
        mulac
            .dispatch_command(envelope)
            .map_err(interpret_dispatch_error)
    }

    pub struct Api;

    #[OpenApi]
    impl Api {
        #[oai(path = "/todos/:id", method = "delete")]
        async fn delete_todo(
            &self,
            state: Data<&AppState>,
            id: Path<Uuid>,
        ) -> Result<DeleteResponse, ApiError> {
            dispatch_delete_todo(&state.mulac, id.0)?;
            Ok(DeleteResponse::NoContent)
        }
    }
}

pub mod io {
    pub use super::DELETE_TODO_COMMAND;
    pub use super::TODO_DELETED_EVENT;
    pub use super::handler::DeleteTodoHandler;
    pub use super::http::Api;
    pub use super::models::{DeleteTodoCommand, TodoDeleted};
}
