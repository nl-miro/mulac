pub mod io {
    pub use super::implementation::UpdateDueDateHandler;
    pub use super::intention::{TodoDueDateChanged, UpdateDueDate};
    pub use super::ui::Api;
}

mod intention {
    use crate::assembly::io::TodoStatus;
    use chrono::{DateTime, Utc};
    use kernel::{ApplicationCommand, ApplicationEvent};
    use poem_openapi::Object;
    use serde::{Deserialize, Serialize};
    use uuid::Uuid;

    /// Asks the system to set or clear a todo's due date. A `None` due date
    /// means the todo has no deadline.
    #[derive(Debug, Clone, Serialize, Deserialize, Object, ApplicationCommand)]
    #[command_type = "UpdateDueDate"]
    pub struct UpdateDueDate {
        pub todo_id: Uuid,
        pub due_at: Option<DateTime<Utc>>,
    }

    #[derive(Debug, Clone)]
    pub struct DueDateSchedule {
        todo_id: Uuid,
        due_at: Option<DateTime<Utc>>,
    }

    impl UpdateDueDate {
        pub fn reschedule(todo_id: Uuid, due_at: Option<DateTime<Utc>>) -> Self {
            Self { todo_id, due_at }
        }

        pub fn schedule(self) -> DueDateSchedule {
            DueDateSchedule { todo_id: self.todo_id, due_at: self.due_at }
        }
    }

    impl DueDateSchedule {
        pub fn into_parts(self) -> (Uuid, Option<DateTime<Utc>>) {
            (self.todo_id, self.due_at)
        }
    }

    /// States that a todo's due date changed, carrying its resulting snapshot.
    #[derive(Debug, Clone, Serialize, Deserialize, Object, ApplicationEvent)]
    #[event_type = "TodoDueDateChanged"]
    pub struct TodoDueDateChanged {
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
    use super::intention::UpdateDueDate;
    use crate::AppState;
    use crate::assembly::io::{ApiError, TodoEntry, dispatch_command, fetch_todo};
    use chrono::{DateTime, Utc};
    use poem::web::Data;
    use poem_openapi::{Object, OpenApi, param::Path, payload::Json};
    use serde::{Deserialize, Serialize};
    use uuid::Uuid;

    pub struct Api;

    #[OpenApi]
    impl Api {
        #[oai(path = "/todos/:id/due-date", method = "put")]
        async fn update_due_date(
            &self,
            state: Data<&AppState>,
            id: Path<Uuid>,
            Json(request): Json<UpdateDueDateRequest>,
        ) -> Result<Json<TodoEntry>, ApiError> {
            let cmd = request.into_command(id.0);

            dispatch_command(&state.mulac, cmd)?;

            Ok(Json(fetch_todo(&state.pool, id.0).await?))
        }
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Object)]
    pub struct UpdateDueDateRequest {
        pub due_at: Option<DateTime<Utc>>,
    }

    impl UpdateDueDateRequest {
        // Boundary adapter: turn an inbound request into the feature's command.
        fn into_command(self, todo_id: Uuid) -> UpdateDueDate {
            UpdateDueDate::reschedule(todo_id, self.due_at)
        }
    }
}

mod implementation {
    use super::intention::{DueDateSchedule, TodoDueDateChanged, UpdateDueDate};
    use crate::assembly::io::{AppError, Clock, TodoEntry, TodoEvent, TodoRow, block_on_blocking};
    use derive_new::new;
    use kernel::{CommandError, CommandHandlerPort};
    use sqlx::PgPool;
    use std::sync::Arc;

    #[derive(new)]
    pub struct UpdateDueDateHandler {
        pub(super) pool: Arc<kernel::io::DbPool>,
    }

    impl From<TodoEntry> for TodoDueDateChanged {
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

    impl CommandHandlerPort<UpdateDueDate, TodoEvent> for UpdateDueDateHandler {
        fn execute(&self, command: UpdateDueDate) -> Result<Vec<TodoEvent>, CommandError> {
            let schedule = command.schedule();
            let persisted = self.apply(schedule)?;

            Ok(vec![TodoEvent::TodoDueDateChanged(persisted.into())])
        }
    }

    impl UpdateDueDateHandler {
        fn apply(&self, schedule: DueDateSchedule) -> Result<TodoEntry, CommandError> {
            let pool = self.pool.clone();
            block_on_blocking(async move { set_due_date(&pool, schedule).await }).map_err(CommandError::from)
        }
    }

    async fn set_due_date(pool: &kernel::io::DbPool, schedule: DueDateSchedule) -> Result<TodoEntry, AppError> {
        let sql = "UPDATE todos SET due_at = $2, updated_at = $3 WHERE id = $1 RETURNING id, title, description, status, created_at, updated_at, due_at";
        let (id, due_at) = schedule.into_parts();

        let row = sqlx::query_as::<_, TodoRow>(sql)
            .bind(id)
            .bind(due_at)
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
    use super::intention::{TodoDueDateChanged, UpdateDueDate};
    use chrono::Utc;
    use uuid::Uuid;

    #[test]
    fn due_date_contract_uses_expected_type_names() {
        assert_eq!(UpdateDueDate::COMMAND_TYPE, "UpdateDueDate");
        assert_eq!(TodoDueDateChanged::EVENT_TYPE, "TodoDueDateChanged");
    }

    #[test]
    fn scheduling_can_clear_a_due_date() {
        let (_, due_at) = UpdateDueDate::reschedule(Uuid::now_v7(), None).schedule().into_parts();
        assert!(due_at.is_none());
    }

    #[test]
    fn scheduling_preserves_the_requested_deadline() {
        let due_at = Utc::now();
        let (_, scheduled_due_at) = UpdateDueDate::reschedule(Uuid::now_v7(), Some(due_at)).schedule().into_parts();
        assert_eq!(scheduled_due_at, Some(due_at));
    }
}
