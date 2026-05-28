pub mod io {
    pub use super::implementation::CompleteTodoHandler;
    pub use super::intention::{CompleteTodo, TodoCompleted};
    pub use super::ui::Api;
}

mod intention {
    use crate::assembly::io::TodoStatus;
    use chrono::{DateTime, Utc};
    use kernel::{ApplicationCommand, ApplicationEvent};
    use poem_openapi::Object;
    use serde::{Deserialize, Serialize};
    use uuid::Uuid;

    /// Asks the system to mark a todo as done.
    #[derive(Debug, Clone, Serialize, Deserialize, Object, ApplicationCommand)]
    #[command_type = "CompleteTodo"]
    pub struct CompleteTodo {
        pub todo_id: Uuid,
    }

    impl CompleteTodo {
        pub fn complete(todo_id: Uuid) -> Self {
            Self { todo_id }
        }

        /// Completing a todo transitions it to `Completed`. The transition rule
        /// lives here, in plain language, rather than in the SQL mechanics.
        pub fn resulting_status(&self) -> TodoStatus {
            TodoStatus::Completed
        }
    }

    /// States that a todo was completed, carrying its resulting snapshot.
    #[derive(Debug, Clone, Serialize, Deserialize, Object, ApplicationEvent)]
    #[event_type = "TodoCompleted"]
    pub struct TodoCompleted {
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
    use super::intention::CompleteTodo;
    use crate::AppState;
    use crate::assembly::io::{ApiError, TodoEntry, dispatch_command, fetch_todo};
    use poem::web::Data;
    use poem_openapi::{OpenApi, param::Path, payload::Json};
    use uuid::Uuid;

    pub struct Api;

    #[OpenApi]
    impl Api {
        #[oai(path = "/todos/:id/complete", method = "post")]
        async fn complete_todo(&self, state: Data<&AppState>, id: Path<Uuid>) -> Result<Json<TodoEntry>, ApiError> {
            let cmd = CompleteTodo::complete(id.0);

            dispatch_command(&state.mulac, cmd)?;

            Ok(Json(fetch_todo(&state.pool, id.0).await?))
        }
    }
}

mod implementation {
    use super::intention::{CompleteTodo, TodoCompleted};
    use crate::assembly::io::{AppError, Clock, TodoEntry, TodoEvent, TodoRow, TodoStatus, block_on_blocking};
    use derive_new::new;
    use kernel::{CommandError, CommandHandlerPort};
    use sqlx::PgPool;
    use std::sync::Arc;
    use uuid::Uuid;

    #[derive(new)]
    pub struct CompleteTodoHandler {
        pub(super) pool: Arc<kernel::io::DbPool>,
    }

    impl From<TodoEntry> for TodoCompleted {
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

    // The handler asks the intention for the resulting status, then persists it.
    impl CommandHandlerPort<CompleteTodo, TodoEvent> for CompleteTodoHandler {
        fn execute(&self, command: CompleteTodo) -> Result<Vec<TodoEvent>, CommandError> {
            let status = command.resulting_status();
            let persisted = self.apply_status(command.todo_id, status)?;

            Ok(vec![TodoEvent::TodoCompleted(persisted.into())])
        }
    }

    impl CompleteTodoHandler {
        fn apply_status(&self, id: Uuid, status: TodoStatus) -> Result<TodoEntry, CommandError> {
            let pool = self.pool.clone();
            block_on_blocking(async move { set_status(&pool, id, status).await }).map_err(CommandError::from)
        }
    }

    async fn set_status(pool: &kernel::io::DbPool, id: Uuid, status: TodoStatus) -> Result<TodoEntry, AppError> {
        let sql = "UPDATE todos SET status = $2, updated_at = $3 WHERE id = $1 RETURNING id, title, description, status, created_at, updated_at, due_at";

        let row = sqlx::query_as::<_, TodoRow>(sql)
            .bind(id)
            .bind(status.as_str())
            .bind(Clock::now())
            .fetch_optional(pool)
            .await
            .map_err(|e| AppError::Storage(e.into()))?
            .ok_or(AppError::NotFound)?;

        row.try_into()
    }
}

#[cfg(test)]
mod tests {
    use super::intention::{CompleteTodo, TodoCompleted};
    use crate::assembly::io::TodoStatus;
    use uuid::Uuid;

    #[test]
    fn complete_todo_contract_uses_expected_type_names() {
        assert_eq!(CompleteTodo::COMMAND_TYPE, "CompleteTodo");
        assert_eq!(TodoCompleted::EVENT_TYPE, "TodoCompleted");
    }

    #[test]
    fn completing_transitions_to_completed() {
        let command = CompleteTodo::complete(Uuid::now_v7());
        assert_eq!(command.resulting_status(), TodoStatus::Completed);
    }
}
