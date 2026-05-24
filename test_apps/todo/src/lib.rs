mod assembly;
mod task_complete;
mod task_create;
mod task_delete;
mod task_get;
mod task_list;
mod task_reopen;
mod task_schedule_due_dates;
mod task_update;

use poem_openapi::Union;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Union)]
#[oai(discriminator_name = "type")]
#[serde(tag = "type", content = "payload")]
pub enum TodoEvent {
    TodoCreated(task_create::io::TodoCreated),
    TodoCompleted(task_complete::io::TodoCompleted),
    TodoReopened(task_reopen::io::TodoReopened),
    TodoUpdated(task_update::io::TodoUpdated),
    TodoDueDateChanged(task_schedule_due_dates::io::TodoDueDateChanged),
    TodoDeleted(task_delete::io::TodoDeleted),
}

impl kernel::ApplicationEvent for TodoEvent {
    fn event_type(&self) -> &'static str {
        match self {
            Self::TodoCreated(_) => task_create::io::TODO_CREATED_EVENT,
            Self::TodoCompleted(_) => task_complete::io::TODO_COMPLETED_EVENT,
            Self::TodoReopened(_) => task_reopen::io::TODO_REOPENED_EVENT,
            Self::TodoUpdated(_) => task_update::io::TODO_UPDATED_EVENT,
            Self::TodoDueDateChanged(_) => task_schedule_due_dates::io::TODO_DUE_DATE_CHANGED_EVENT,
            Self::TodoDeleted(_) => task_delete::io::TODO_DELETED_EVENT,
        }
    }
}

use sqlx::PgPool;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub mulac: assembly::io::MulacState,
}

impl AppState {
    pub fn new(pool: PgPool, mulac: assembly::io::MulacState) -> Self {
        Self { pool, mulac }
    }
}

mod inbox {
    mod models {
        use crate::assembly::io::TodoDto;
        use crate::{
            task_complete,
            task_create,
            task_delete,
            task_reopen,
            task_schedule_due_dates,
            task_update,
            //
        };
        use poem_openapi::{Object, Union};
        use serde::{Deserialize, Serialize};
        use uuid::Uuid;

        #[derive(Debug, Clone, Serialize, Deserialize, Union)]
        #[oai(discriminator_name = "type")]
        #[serde(tag = "type", content = "payload")]
        pub enum TodoCommand {
            CreateTodo(task_create::io::CreateTodoCommand),
            CompleteTodo(task_complete::io::CompleteTodoCommand),
            ReopenTodo(task_reopen::io::ReopenTodoCommand),
            UpdateTodo(task_update::io::UpdateTodoCommand),
            DeleteTodo(task_delete::io::DeleteTodoCommand),
            UpdateDueDate(task_schedule_due_dates::io::UpdateDueDateCommand),
        }

        impl TodoCommand {
            pub fn message_type(&self) -> &'static str {
                match self {
                    Self::CreateTodo(_) => task_create::io::CREATE_TODO_COMMAND,
                    Self::CompleteTodo(_) => task_complete::io::COMPLETE_TODO_COMMAND,
                    Self::ReopenTodo(_) => task_reopen::io::REOPEN_TODO_COMMAND,
                    Self::UpdateTodo(_) => task_update::io::UPDATE_TODO_COMMAND,
                    Self::DeleteTodo(_) => task_delete::io::DELETE_TODO_COMMAND,
                    Self::UpdateDueDate(_) => task_schedule_due_dates::io::UPDATE_DUE_DATE_COMMAND,
                }
            }

            pub fn todo_id(&self) -> Uuid {
                match self {
                    Self::CreateTodo(c) => c.todo_id,
                    Self::CompleteTodo(c) => c.todo_id,
                    Self::ReopenTodo(c) => c.todo_id,
                    Self::UpdateTodo(c) => c.todo_id,
                    Self::DeleteTodo(c) => c.todo_id,
                    Self::UpdateDueDate(c) => c.todo_id,
                }
            }
        }

        impl kernel::ApplicationCommand for TodoCommand {
            fn command_type(&self) -> &'static str {
                self.message_type()
            }
        }

        #[derive(Debug, Clone, Serialize, Deserialize, Object)]
        pub struct CommandEnvelope {
            pub id: Uuid,
            pub command: TodoCommand,
        }

        #[derive(Debug, Clone, Serialize, Deserialize, Object)]
        pub struct InboundResponse {
            pub message_id: Uuid,
            pub todo: TodoDto,
        }
    }

    mod http {
        use super::models::{CommandEnvelope, InboundResponse, TodoCommand};
        use crate::AppState;
        use crate::assembly::io::{
            ApiError,
            AppCommand,
            AppError,
            Clock,
            Command,
            MulacState,
            NewCommandEnvelope,
            fetch_todo,
            //
        };
        use kernel::NewCommandMetadata;
        use poem::web::Data;
        use poem_openapi::{OpenApi, payload::Json};
        use sqlx::PgPool;
        use uuid::Uuid;

        async fn record_received(
            pool: &PgPool,
            envelope: &CommandEnvelope,
        ) -> Result<(), AppError> {
            let sql = "INSERT INTO inbox_messages (id, message_type, payload, status, received_at) VALUES ($1, $2, $3, 'received', $4) ON CONFLICT (id) DO NOTHING";

            let result = sqlx::query(sql)
                .bind(envelope.id)
                .bind(envelope.command.message_type())
                .bind(
                    serde_json::to_value(&envelope.command)
                        .map_err(|e| AppError::Storage(e.into()))?,
                )
                .bind(Clock::now())
                .execute(pool)
                .await
                .map_err(|e| AppError::Storage(e.into()))?;

            if result.rows_affected() == 0 {
                return Err(AppError::Conflict(format!(
                    "message {} was already received",
                    envelope.id
                )));
            }
            Ok(())
        }

        async fn mark_processed(pool: &PgPool, message_id: Uuid) -> Result<(), AppError> {
            let sql = "UPDATE inbox_messages SET status = 'processed', processed_at = $2, error = NULL WHERE id = $1";

            sqlx::query(sql)
                .bind(message_id)
                .bind(Clock::now())
                .execute(pool)
                .await
                .map_err(|e| AppError::Storage(e.into()))?;
            Ok(())
        }

        async fn mark_failed(
            pool: &PgPool,
            message_id: Uuid,
            error: &AppError,
        ) -> Result<(), AppError> {
            let sql = "UPDATE inbox_messages SET status = 'failed', processed_at = $2, error = $3 WHERE id = $1";

            sqlx::query(sql)
                .bind(message_id)
                .bind(Clock::now())
                .bind(error.to_string())
                .execute(pool)
                .await
                .map_err(|e| AppError::Storage(e.into()))?;
            Ok(())
        }

        fn dispatch_inbound_command(
            mulac: &MulacState,
            envelope: &CommandEnvelope,
        ) -> Result<Uuid, AppError> {
            let command = to_app_command(envelope.command.clone());
            let todo_id = command.todo_id();
            dispatch(
                mulac,
                command,
                NewCommandMetadata {
                    command_id: envelope.id,
                    correlation_id: Some(envelope.id),
                    causation_id: Some(envelope.id),
                    source: Some("test_app_todo.inbox".to_string()),
                },
            )?;
            Ok(todo_id)
        }

        fn to_app_command(cmd: TodoCommand) -> AppCommand {
            match cmd {
                TodoCommand::CreateTodo(c) => AppCommand::CreateTodo(c),
                TodoCommand::CompleteTodo(c) => AppCommand::CompleteTodo(c),
                TodoCommand::ReopenTodo(c) => AppCommand::ReopenTodo(c),
                TodoCommand::UpdateTodo(c) => AppCommand::UpdateTodo(c),
                TodoCommand::DeleteTodo(c) => AppCommand::DeleteTodo(c),
                TodoCommand::UpdateDueDate(c) => AppCommand::UpdateDueDate(c),
            }
        }

        fn dispatch(
            mulac: &MulacState,
            command: AppCommand,
            metadata: NewCommandMetadata,
        ) -> Result<(), AppError> {
            mulac
                .dispatch_command(NewCommandEnvelope { command, metadata })
                .map_err(crate::assembly::io::interpret_dispatch_error)
        }

        pub struct Api;

        #[OpenApi]
        impl Api {
            #[oai(path = "/messages/commands", method = "post")]
            async fn process_command(
                &self,
                state: Data<&AppState>,
                Json(command): Json<CommandEnvelope>,
            ) -> Result<Json<InboundResponse>, ApiError> {
                record_received(&state.pool, &command).await?;
                let message_id = command.id;
                let todo_id = match dispatch_inbound_command(&state.mulac, &command) {
                    Ok(todo_id) => {
                        mark_processed(&state.pool, message_id).await?;
                        todo_id
                    }
                    Err(error) => {
                        mark_failed(&state.pool, message_id, &error).await?;
                        return Err(error.into());
                    }
                };
                Ok(Json(InboundResponse {
                    message_id,
                    todo: fetch_todo(&state.pool, todo_id).await?,
                }))
            }
        }
    }

    pub mod io {
        pub use super::http::Api;
    }
}

mod outbox {
    mod models {
        use chrono::{DateTime, Utc};
        use poem_openapi::Object;
        use serde::{Deserialize, Serialize};
        use sqlx::FromRow;
        use uuid::Uuid;

        #[derive(Debug, Clone, Serialize, Deserialize, Object)]
        pub struct OutboxMessageDto {
            pub id: Uuid,
            pub event_type: String,
            pub payload: serde_json::Value,
            pub status: String,
            pub created_at: DateTime<Utc>,
            pub published_at: Option<DateTime<Utc>>,
            pub attempts: i32,
        }

        #[derive(Debug, FromRow)]
        pub struct OutboxRow {
            pub id: Uuid,
            pub event_type: String,
            pub payload: serde_json::Value,
            pub status: String,
            pub created_at: DateTime<Utc>,
            pub published_at: Option<DateTime<Utc>>,
            pub attempts: i32,
        }

        impl From<OutboxRow> for OutboxMessageDto {
            fn from(row: OutboxRow) -> Self {
                Self {
                    id: row.id,
                    event_type: row.event_type,
                    payload: row.payload,
                    status: row.status,
                    created_at: row.created_at,
                    published_at: row.published_at,
                    attempts: row.attempts,
                }
            }
        }
    }

    mod infra_sqlx_pg {
        use super::models::{OutboxMessageDto, OutboxRow};
        use crate::assembly::io::AppError;
        use sqlx::PgPool;

        pub async fn list(pool: &PgPool) -> Result<Vec<OutboxMessageDto>, AppError> {
            let sql = "SELECT id, event_type, payload, status, created_at, published_at, attempts FROM outbox_messages ORDER BY created_at ASC";

            let rows = sqlx::query_as::<_, OutboxRow>(sql)
                .fetch_all(pool)
                .await
                .map_err(|e| AppError::Storage(e.into()))?;
            Ok(rows.into_iter().map(Into::into).collect())
        }
    }

    mod http {
        use super::infra_sqlx_pg::list;
        use super::models::OutboxMessageDto;
        use crate::{AppState, assembly::io::ApiError};
        use poem::web::Data;
        use poem_openapi::{OpenApi, payload::Json};

        pub struct Api;

        #[OpenApi]
        impl Api {
            #[oai(path = "/messages/outbox", method = "get")]
            async fn list_outbox(
                &self,
                state: Data<&AppState>,
            ) -> Result<Json<Vec<OutboxMessageDto>>, ApiError> {
                Ok(Json(list(&state.pool).await?))
            }
        }
    }

    pub mod io {
        pub use super::http::Api;
        pub use super::models::OutboxMessageDto;
    }
}

pub mod io {
    pub use super::AppState;
    pub use super::assembly::io::{
        ApiError,
        AppCommand,
        AppError,
        Command,
        ErrorBody,
        MulacHandle,
        MulacState,
        NewCommandEnvelope,
        OutboxSubscriber,
        TodoDto,
        TodoList,
        TodoRow,
        TodoStatus,
        block_on_blocking,
        connect,
        fetch_todo,
        interpret_dispatch_error,
        migrate,
        record_event_payload,
        run_command_worker,
        run_event_worker,
        start_mulac,
        validate_title,
        //
    };
    pub use super::inbox::io::Api as InboxApi;
    pub use super::outbox::io::{Api as OutboxApi, OutboxMessageDto};
    pub use super::task_complete::io::Api as CompleteApi;
    pub use super::task_create::io::Api as CreateApi;
    pub use super::task_delete::io::Api as DeleteApi;
    pub use super::task_get::io::Api as GetApi;
    pub use super::task_list::io::Api as ListApi;
    pub use super::task_list::io::FilterStatus;
    pub use super::task_reopen::io::Api as ReopenApi;
    pub use super::task_schedule_due_dates::io::Api as DueDatesApi;
    pub use super::task_update::io::Api as UpdateApi;
}
