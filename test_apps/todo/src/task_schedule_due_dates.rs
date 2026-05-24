pub const UPDATE_DUE_DATE_COMMAND: &str = "UpdateDueDate";
pub const TODO_DUE_DATE_CHANGED_EVENT: &str = "TodoDueDateChanged";

mod models {
    use crate::assembly::io::TodoDto;
    use chrono::{DateTime, Utc};
    use poem_openapi::Object;
    use serde::{Deserialize, Serialize};
    use uuid::Uuid;

    #[derive(Debug, Clone, Serialize, Deserialize, Object)]
    pub struct UpdateDueDateCommand {
        pub todo_id: Uuid,
        pub due_at: Option<DateTime<Utc>>,
    }

    impl kernel::ApplicationCommand for UpdateDueDateCommand {
        fn command_type(&self) -> &'static str {
            super::UPDATE_DUE_DATE_COMMAND
        }
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Object)]
    pub struct TodoDueDateChanged {
        pub todo: TodoDto,
    }

    impl kernel::ApplicationEvent for TodoDueDateChanged {
        fn event_type(&self) -> &'static str {
            super::TODO_DUE_DATE_CHANGED_EVENT
        }
    }
}

mod handler {
    use super::models::{TodoDueDateChanged, UpdateDueDateCommand};
    use crate::assembly::io::{TodoEvent, block_on_blocking};
    use kernel::{CommandError, CommandHandlerPort};
    use sqlx::PgPool;

    pub struct UpdateDueDateHandler {
        pool: PgPool,
    }

    impl UpdateDueDateHandler {
        pub fn new(pool: PgPool) -> Self {
            Self { pool }
        }
    }

    impl CommandHandlerPort<UpdateDueDateCommand, TodoEvent> for UpdateDueDateHandler {
        fn execute(&self, command: UpdateDueDateCommand) -> Result<Vec<TodoEvent>, CommandError> {
            let pool = self.pool.clone();
            let todo = block_on_blocking(async move {
                super::infra_sqlx_pg::set_due_date(&pool, command.todo_id, command.due_at).await
            })
            .map_err(|e| CommandError::HandlerExecution(e.to_string()))?;

            Ok(vec![TodoEvent::TodoDueDateChanged(TodoDueDateChanged {
                todo,
            })])
        }
    }
}

mod infra_sqlx_pg {
    use crate::assembly::io::{AppError, Clock, TodoDto, TodoRow};
    use chrono::{DateTime, Utc};
    use sqlx::PgPool;
    use uuid::Uuid;
    pub async fn set_due_date(
        pool: &PgPool,
        id: Uuid,
        due_at: Option<DateTime<Utc>>,
    ) -> Result<TodoDto, AppError> {
        let sql = "UPDATE todos SET due_at = $2, updated_at = $3 WHERE id = $1 RETURNING id, title, description, status, created_at, updated_at, due_at";

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

mod http {
    use super::models::UpdateDueDateCommand;
    use crate::{
        AppState,
        assembly::io::{
            ApiError, AppCommand, MulacState, NewCommandEnvelope, TodoDto, fetch_todo,
            interpret_dispatch_error,
        },
        //
    };
    use chrono::{DateTime, Utc};
    use poem::web::Data;
    use poem_openapi::{Object, OpenApi, param::Path, payload::Json};
    use serde::{Deserialize, Serialize};
    use uuid::Uuid;

    #[derive(Debug, Clone, Serialize, Deserialize, Object)]
    pub struct UpdateDueDateRequest {
        pub due_at: Option<DateTime<Utc>>,
    }

    fn dispatch_update_due_date(
        mulac: &MulacState,
        id: Uuid,
        due_at: Option<DateTime<Utc>>,
    ) -> Result<(), ApiError> {
        let command_id = Uuid::now_v7();
        let envelope = NewCommandEnvelope {
            command: AppCommand::UpdateDueDate(UpdateDueDateCommand {
                todo_id: id,
                due_at,
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
            .map_err(|e| ApiError::from(interpret_dispatch_error(e)))
    }

    pub struct Api;

    #[OpenApi]
    impl Api {
        #[oai(path = "/todos/:id/due-date", method = "put")]
        async fn update_due_date(
            &self,
            state: Data<&AppState>,
            id: Path<Uuid>,
            Json(request): Json<UpdateDueDateRequest>,
        ) -> Result<Json<TodoDto>, ApiError> {
            dispatch_update_due_date(&state.mulac, id.0, request.due_at)?;
            Ok(Json(fetch_todo(&state.pool, id.0).await?))
        }
    }
}

pub mod io {
    pub use super::TODO_DUE_DATE_CHANGED_EVENT;
    pub use super::UPDATE_DUE_DATE_COMMAND;
    pub use super::handler::UpdateDueDateHandler;
    pub use super::http::Api;
    pub use super::models::{TodoDueDateChanged, UpdateDueDateCommand};
}
