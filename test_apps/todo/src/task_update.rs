pub mod io {
    pub use super::implementation::UpdateTodoHandler;
    pub use super::intention::{TodoUpdated, UpdateTodo};
    pub use super::ui::Api;
}

mod intention {
    use crate::assembly::io::{AppError, TodoStatus, validate_title};
    use chrono::{DateTime, Utc};
    use kernel::{ApplicationCommand, ApplicationEvent};
    use poem_openapi::Object;
    use serde::{Deserialize, Serialize};
    use uuid::Uuid;

    /// Asks the system to change a todo's title and description.
    #[derive(Debug, Clone, Serialize, Deserialize, Object, ApplicationCommand)]
    #[command_type = "UpdateTodo"]
    pub struct UpdateTodo {
        pub todo_id: Uuid,
        pub title: String,
        pub description: Option<String>,
    }

    #[derive(Debug, Clone)]
    pub struct TodoRevision {
        todo_id: Uuid,
        title: String,
        description: Option<String>,
    }

    impl UpdateTodo {
        pub fn revise(todo_id: Uuid, title: String, description: Option<String>) -> Self {
            Self { todo_id, title, description }
        }

        /// An updated todo must still be named. The rule lives here, in plain
        /// language, rather than in the SQL mechanics.
        pub fn revision(self) -> Result<TodoRevision, AppError> {
            validate_title(&self.title).map(|()| TodoRevision {
                todo_id: self.todo_id,
                title: self.title.trim().to_string(),
                description: self.description,
            })
        }
    }

    impl TodoRevision {
        pub fn into_parts(self) -> (Uuid, String, Option<String>) {
            (self.todo_id, self.title, self.description)
        }
    }

    /// States that a todo was updated, carrying its resulting snapshot.
    #[derive(Debug, Clone, Serialize, Deserialize, Object, ApplicationEvent)]
    #[event_type = "TodoUpdated"]
    pub struct TodoUpdated {
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
    use super::intention::UpdateTodo;
    use crate::AppState;
    use crate::assembly::io::{ApiError, TodoEntry, dispatch_command, fetch_todo};
    use poem::web::Data;
    use poem_openapi::{Object, OpenApi, param::Path, payload::Json};
    use serde::{Deserialize, Serialize};
    use uuid::Uuid;

    pub struct Api;

    #[OpenApi]
    impl Api {
        #[oai(path = "/todos/:id", method = "put")]
        async fn update_todo(
            &self,
            state: Data<&AppState>,
            id: Path<Uuid>,
            Json(request): Json<UpdateTodoRequest>,
        ) -> Result<Json<TodoEntry>, ApiError> {
            let cmd = request.into_command(id.0);

            dispatch_command(&state.mulac, cmd)?;

            Ok(Json(fetch_todo(&state.pool, id.0).await?))
        }
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Object)]
    pub struct UpdateTodoRequest {
        pub title: String,
        pub description: Option<String>,
    }

    impl UpdateTodoRequest {
        // Boundary adapter: turn an inbound request into the feature's command.
        fn into_command(self, todo_id: Uuid) -> UpdateTodo {
            UpdateTodo::revise(todo_id, self.title, self.description)
        }
    }
}

mod implementation {
    use super::intention::{TodoRevision, TodoUpdated, UpdateTodo};
    use crate::assembly::io::{AppError, Clock, TodoEntry, TodoEvent, TodoRow, block_on_blocking};
    use derive_new::new;
    use kernel::{CommandError, CommandHandlerPort};
    use sqlx::PgPool;
    use std::sync::Arc;

    #[derive(new)]
    pub struct UpdateTodoHandler {
        pub(super) pool: Arc<kernel::io::DbPool>,
    }

    impl From<TodoEntry> for TodoUpdated {
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

    // The handler validates the intent, then persists the new field values.
    impl CommandHandlerPort<UpdateTodo, TodoEvent> for UpdateTodoHandler {
        fn execute(&self, command: UpdateTodo) -> Result<Vec<TodoEvent>, CommandError> {
            let revision = command.revision()?;
            let persisted = self.update(revision)?;

            Ok(vec![TodoEvent::TodoUpdated(persisted.into())])
        }
    }

    impl UpdateTodoHandler {
        fn update(&self, revision: TodoRevision) -> Result<TodoEntry, CommandError> {
            let pool = self.pool.clone();
            block_on_blocking(async move { write_fields(&pool, revision).await }).map_err(CommandError::from)
        }
    }

    async fn write_fields(pool: &kernel::io::DbPool, revision: TodoRevision) -> Result<TodoEntry, AppError> {
        let sql = "UPDATE todos SET title = $2, description = $3, updated_at = $4 WHERE id = $1 RETURNING id, title, description, status, created_at, updated_at, due_at";
        let (todo_id, title, description) = revision.into_parts();

        let row = sqlx::query_as::<_, TodoRow>(sql)
            .bind(todo_id)
            .bind(title)
            .bind(description)
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
    use super::intention::{TodoUpdated, UpdateTodo};
    use uuid::Uuid;

    #[test]
    fn update_todo_contract_uses_expected_type_names() {
        assert_eq!(UpdateTodo::COMMAND_TYPE, "UpdateTodo");
        assert_eq!(TodoUpdated::EVENT_TYPE, "TodoUpdated");
    }

    #[test]
    fn revision_trims_title_and_keeps_description() {
        let revision = UpdateTodo::revise(Uuid::now_v7(), "  Updated  ".to_string(), Some("Details".to_string()))
            .revision()
            .expect("a named todo should revise");
        let (_, title, description) = revision.into_parts();

        assert_eq!(title, "Updated");
        assert_eq!(description.as_deref(), Some("Details"));
    }

    #[test]
    fn revision_rejects_blank_title() {
        let result = UpdateTodo::revise(Uuid::now_v7(), "   ".to_string(), None).revision();
        assert!(result.is_err(), "a blank title is not a meaningful update");
    }
}
