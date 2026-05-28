pub mod io {
    pub use super::implementation::ReopenTodoHandler;
    pub use super::intention::{ReopenTodo, TodoReopened};
    pub use super::ui::Api;
}

mod intention {
    use crate::assembly::io::TodoStatus;
    use chrono::{DateTime, Utc};
    use kernel::{ApplicationCommand, ApplicationEvent};
    use poem_openapi::Object;
    use serde::{Deserialize, Serialize};
    use uuid::Uuid;

    /// Asks the system to return a finished todo to active work.
    #[derive(Debug, Clone, Serialize, Deserialize, Object, ApplicationCommand)]
    #[command_type = "ReopenTodo"]
    pub struct ReopenTodo {
        pub todo_id: Uuid,
    }

    impl ReopenTodo {
        pub fn reopen(todo_id: Uuid) -> Self {
            Self { todo_id }
        }

        /// Reopening a todo transitions it back to `Active`. The transition rule
        /// lives here, in plain language, rather than in the SQL mechanics.
        pub fn resulting_status(&self) -> TodoStatus {
            TodoStatus::Active
        }
    }

    /// States that a todo was reopened, carrying its resulting snapshot.
    #[derive(Debug, Clone, Serialize, Deserialize, Object, ApplicationEvent)]
    #[event_type = "TodoReopened"]
    pub struct TodoReopened {
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
    use super::intention::ReopenTodo;
    use crate::AppState;
    use crate::assembly::io::{ApiError, TodoEntry, dispatch_command, fetch_todo};
    use poem::web::Data;
    use poem_openapi::{OpenApi, param::Path, payload::Json};
    use uuid::Uuid;

    pub struct Api;

    #[OpenApi]
    impl Api {
        #[oai(path = "/todos/:id/reopen", method = "post")]
        async fn reopen_todo(&self, state: Data<&AppState>, id: Path<Uuid>) -> Result<Json<TodoEntry>, ApiError> {
            let cmd = ReopenTodo::reopen(id.0);

            dispatch_command(&state.mulac, cmd)?;

            Ok(Json(fetch_todo(&state.pool, id.0).await?))
        }
    }
}

mod implementation {
    use super::intention::{ReopenTodo, TodoReopened};
    use crate::assembly::io::{AppError, Clock, TodoEntry, TodoEvent, TodoRow, TodoStatus, block_on_blocking};
    use derive_new::new;
    use kernel::{CommandError, CommandHandlerPort};
    use sqlx::PgPool;
    use std::sync::Arc;
    use uuid::Uuid;

    #[derive(new)]
    pub struct ReopenTodoHandler {
        pub(super) pool: Arc<kernel::io::DbPool>,
    }

    impl From<TodoEntry> for TodoReopened {
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
    impl CommandHandlerPort<ReopenTodo, TodoEvent> for ReopenTodoHandler {
        fn execute(&self, command: ReopenTodo) -> Result<Vec<TodoEvent>, CommandError> {
            let status = command.resulting_status();
            let persisted = self.apply_status(command.todo_id, status)?;

            Ok(vec![TodoEvent::TodoReopened(persisted.into())])
        }
    }

    impl ReopenTodoHandler {
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
    use super::intention::{ReopenTodo, TodoReopened};
    use crate::assembly::io::TodoStatus;
    use uuid::Uuid;

    #[test]
    fn reopen_todo_contract_uses_expected_type_names() {
        assert_eq!(ReopenTodo::COMMAND_TYPE, "ReopenTodo");
        assert_eq!(TodoReopened::EVENT_TYPE, "TodoReopened");
    }

    #[test]
    fn reopening_transitions_to_active() {
        let command = ReopenTodo::reopen(Uuid::now_v7());
        assert_eq!(command.resulting_status(), TodoStatus::Active);
    }
}
