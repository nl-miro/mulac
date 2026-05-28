pub mod io {
    pub use super::implementation::DeleteTodoHandler;
    pub use super::intention::{DeleteTodo, TodoDeleted};
    pub use super::ui::Api;
}

mod intention {
    use crate::assembly::io::TodoStatus;
    use chrono::{DateTime, Utc};
    use kernel::{ApplicationCommand, ApplicationEvent};
    use poem_openapi::Object;
    use serde::{Deserialize, Serialize};
    use uuid::Uuid;

    /// Asks the system to permanently remove a todo.
    #[derive(Debug, Clone, Serialize, Deserialize, Object, ApplicationCommand)]
    #[command_type = "DeleteTodo"]
    pub struct DeleteTodo {
        pub todo_id: Uuid,
    }

    impl DeleteTodo {
        pub fn remove(todo_id: Uuid) -> Self {
            Self { todo_id }
        }
    }

    /// States that a todo was deleted, carrying the final snapshot it had before
    /// removal so downstream readers can react with full context.
    #[derive(Debug, Clone, Serialize, Deserialize, Object, ApplicationEvent)]
    #[event_type = "TodoDeleted"]
    pub struct TodoDeleted {
        pub id: Uuid,
        pub title: String,
        pub description: Option<String>,
        pub status: TodoStatus,
        pub created_at: DateTime<Utc>,
        pub updated_at: DateTime<Utc>,
        pub due_at: Option<DateTime<Utc>>,
    }
}

mod ui {
    use super::intention::DeleteTodo;
    use crate::AppState;
    use crate::assembly::io::{ApiError, dispatch_command};
    use poem::web::Data;
    use poem_openapi::{ApiResponse, OpenApi, param::Path};
    use uuid::Uuid;

    #[derive(ApiResponse)]
    enum DeleteResponse {
        #[oai(status = 204)]
        NoContent,
    }

    pub struct Api;

    #[OpenApi]
    impl Api {
        #[oai(path = "/todos/:id", method = "delete")]
        async fn delete_todo(&self, state: Data<&AppState>, id: Path<Uuid>) -> Result<DeleteResponse, ApiError> {
            let cmd = DeleteTodo::remove(id.0);

            dispatch_command(&state.mulac, cmd)?;

            Ok(DeleteResponse::NoContent)
        }
    }
}

mod implementation {
    use super::intention::{DeleteTodo, TodoDeleted};
    use crate::assembly::io::{AppError, TodoEntry, TodoEvent, TodoRow, block_on_blocking};
    use derive_new::new;
    use kernel::{CommandError, CommandHandlerPort};
    use sqlx::PgPool;
    use std::sync::Arc;
    use uuid::Uuid;

    #[derive(new)]
    pub struct DeleteTodoHandler {
        pub(super) pool: Arc<kernel::io::DbPool>,
    }

    impl From<TodoEntry> for TodoDeleted {
        fn from(todo: TodoEntry) -> Self {
            Self {
                id: todo.id,
                title: todo.title,
                description: todo.description,
                status: todo.status,
                created_at: todo.created_at,
                updated_at: todo.updated_at,
                due_at: todo.due_at,
            }
        }
    }

    impl CommandHandlerPort<DeleteTodo, TodoEvent> for DeleteTodoHandler {
        fn execute(&self, command: DeleteTodo) -> Result<Vec<TodoEvent>, CommandError> {
            let removed = self.delete(command.todo_id)?;

            Ok(vec![TodoEvent::TodoDeleted(removed.into())])
        }
    }

    impl DeleteTodoHandler {
        fn delete(&self, id: Uuid) -> Result<TodoEntry, CommandError> {
            let pool = self.pool.clone();
            block_on_blocking(async move { remove(&pool, id).await }).map_err(CommandError::from)
        }
    }

    async fn remove(pool: &kernel::io::DbPool, id: Uuid) -> Result<TodoEntry, AppError> {
        let sql = "DELETE FROM todos WHERE id = $1 RETURNING id, title, description, status, created_at, updated_at, due_at";

        let row = sqlx::query_as::<_, TodoRow>(sql)
            .bind(id)
            .fetch_optional(pool)
            .await
            .map_err(|e| AppError::Storage(e.into()))?
            .ok_or(AppError::NotFound)?;

        row.try_into()
    }
}

#[cfg(test)]
mod tests {
    use super::intention::{DeleteTodo, TodoDeleted};
    use uuid::Uuid;

    #[test]
    fn delete_todo_contract_uses_expected_type_names() {
        assert_eq!(DeleteTodo::COMMAND_TYPE, "DeleteTodo");
        assert_eq!(TodoDeleted::EVENT_TYPE, "TodoDeleted");
    }

    #[test]
    fn remove_targets_the_selected_todo() {
        let id = Uuid::now_v7();
        assert_eq!(DeleteTodo::remove(id).todo_id, id);
    }
}
