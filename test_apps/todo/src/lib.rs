mod assembly;
#[allow(unused)]
mod schema;
mod task_complete;
mod task_create;
mod task_delete;
mod task_get;
mod task_list;
mod task_reopen;
mod task_schedule_due_dates;
mod task_update;

use kernel::ApplicationEvent;
use poem_openapi::Union;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Union)]
#[oai(discriminator_name = "type")]
#[serde(tag = "type", content = "payload")]
pub enum TodoEvent {
    TodoCreated(task_create::prelude::TodoCreated),
    TodoCompleted(task_complete::prelude::TodoCompleted),
    TodoReopened(task_reopen::prelude::TodoReopened),
    TodoUpdated(task_update::prelude::TodoUpdated),
    TodoDueDateChanged(task_schedule_due_dates::prelude::TodoDueDateChanged),
    TodoDeleted(task_delete::prelude::TodoDeleted),
}

impl ApplicationEvent for TodoEvent {
    fn event_type(&self) -> &'static str {
        match self {
            Self::TodoCreated(_) => task_create::prelude::TODO_CREATED_EVENT,
            Self::TodoCompleted(_) => task_complete::prelude::TODO_COMPLETED_EVENT,
            Self::TodoReopened(_) => task_reopen::prelude::TODO_REOPENED_EVENT,
            Self::TodoUpdated(_) => task_update::prelude::TODO_UPDATED_EVENT,
            Self::TodoDueDateChanged(_) => {
                task_schedule_due_dates::prelude::TODO_DUE_DATE_CHANGED_EVENT
            }
            Self::TodoDeleted(_) => task_delete::prelude::TODO_DELETED_EVENT,
        }
    }
}

#[derive(Clone)]
pub struct AppState {
    pub pool: assembly::prelude::DbPool,
    pub mulac: assembly::prelude::MulacState,
}

impl AppState {
    pub fn new(pool: assembly::prelude::DbPool, mulac: assembly::prelude::MulacState) -> Self {
        Self { pool, mulac }
    }
}

mod inbox {
    mod models {
        use crate::assembly::prelude::TodoDto;
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
            CreateTodo(task_create::prelude::CreateTodoCommand),
            CompleteTodo(task_complete::prelude::CompleteTodoCommand),
            ReopenTodo(task_reopen::prelude::ReopenTodoCommand),
            UpdateTodo(task_update::prelude::UpdateTodoCommand),
            DeleteTodo(task_delete::prelude::DeleteTodoCommand),
            UpdateDueDate(task_schedule_due_dates::prelude::UpdateDueDateCommand),
        }

        impl TodoCommand {
            pub fn message_type(&self) -> &'static str {
                match self {
                    Self::CreateTodo(_) => task_create::prelude::CREATE_TODO_COMMAND,
                    Self::CompleteTodo(_) => task_complete::prelude::COMPLETE_TODO_COMMAND,
                    Self::ReopenTodo(_) => task_reopen::prelude::REOPEN_TODO_COMMAND,
                    Self::UpdateTodo(_) => task_update::prelude::UPDATE_TODO_COMMAND,
                    Self::DeleteTodo(_) => task_delete::prelude::DELETE_TODO_COMMAND,
                    Self::UpdateDueDate(_) => {
                        task_schedule_due_dates::prelude::UPDATE_DUE_DATE_COMMAND
                    }
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
        use crate::assembly::prelude::{
            ApiError,
            AppCommand,
            AppError,
            Clock,
            Command,
            DbPool,
            MulacState,
            NewCommandEnvelope,
            fetch_todo,
            run_blocking,
            //
        };
        use kernel::NewCommandMetadata;
        use poem::web::Data;
        use poem_openapi::{OpenApi, payload::Json};
        use uuid::Uuid;

        fn record_received(pool: &DbPool, envelope: &CommandEnvelope) -> Result<(), AppError> {
            use crate::schema::inbox_messages;
            use diesel::prelude::*;

            let payload =
                serde_json::to_value(&envelope.command).map_err(|e| AppError::Storage(e.into()))?;
            let mut conn = pool.get().map_err(|e| AppError::Storage(e.into()))?;
            let inserted = diesel::insert_into(inbox_messages::table)
                .values((
                    inbox_messages::id.eq(envelope.id),
                    inbox_messages::message_type.eq(envelope.command.message_type()),
                    inbox_messages::payload.eq(payload),
                    inbox_messages::status.eq("received"),
                    inbox_messages::received_at.eq(Clock::now()),
                ))
                .on_conflict_do_nothing()
                .execute(&mut conn)
                .map_err(|e| AppError::Storage(e.into()))?;

            if inserted == 0 {
                return Err(AppError::Conflict(format!(
                    "message {} was already received",
                    envelope.id
                )));
            }
            Ok(())
        }

        fn mark_processed(pool: &DbPool, message_id: Uuid) -> Result<(), AppError> {
            use crate::schema::inbox_messages;
            use diesel::prelude::*;

            let mut conn = pool.get().map_err(|e| AppError::Storage(e.into()))?;
            diesel::update(inbox_messages::table.find(message_id))
                .set((
                    inbox_messages::status.eq("processed"),
                    inbox_messages::processed_at.eq(Clock::now()),
                    inbox_messages::error.eq(None::<String>),
                ))
                .execute(&mut conn)
                .map_err(|e| AppError::Storage(e.into()))?;
            Ok(())
        }

        fn mark_failed(pool: &DbPool, message_id: Uuid, error: &AppError) -> Result<(), AppError> {
            use crate::schema::inbox_messages;
            use diesel::prelude::*;

            let mut conn = pool.get().map_err(|e| AppError::Storage(e.into()))?;
            diesel::update(inbox_messages::table.find(message_id))
                .set((
                    inbox_messages::status.eq("failed"),
                    inbox_messages::processed_at.eq(Clock::now()),
                    inbox_messages::error.eq(error.to_string()),
                ))
                .execute(&mut conn)
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
                .map_err(crate::assembly::prelude::interpret_dispatch_error)
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
                let pool = state.pool.clone();
                let mulac = state.mulac.clone();
                let message_id = command.id;
                let response = run_blocking(move || {
                    record_received(&pool, &command)?;
                    let todo_id = match dispatch_inbound_command(&mulac, &command) {
                        Ok(todo_id) => {
                            mark_processed(&pool, message_id)?;
                            todo_id
                        }
                        Err(error) => {
                            mark_failed(&pool, message_id, &error)?;
                            return Err(error);
                        }
                    };
                    Ok(InboundResponse {
                        message_id,
                        todo: fetch_todo(&pool, todo_id)?,
                    })
                })
                .await?;
                Ok(Json(response))
            }
        }
    }

    pub mod prelude {
        pub use super::http::Api;
    }
}

mod outbox {
    mod models {
        use chrono::{DateTime, Utc};
        use poem_openapi::Object;
        use serde::{Deserialize, Serialize};
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

        #[derive(Debug, diesel::Queryable)]
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

    mod infra_diesel {
        use super::models::{OutboxMessageDto, OutboxRow};
        use crate::assembly::prelude::{AppError, DbPool};
        use crate::schema::outbox_messages;
        use diesel::prelude::*;

        pub fn list(pool: &DbPool) -> Result<Vec<OutboxMessageDto>, AppError> {
            let mut conn = pool.get().map_err(|e| AppError::Storage(e.into()))?;
            let rows = outbox_messages::table
                .order(outbox_messages::created_at.asc())
                .load::<OutboxRow>(&mut conn)
                .map_err(|e| AppError::Storage(e.into()))?;
            Ok(rows.into_iter().map(Into::into).collect())
        }
    }

    mod http {
        use super::infra_diesel::list;
        use super::models::OutboxMessageDto;
        use crate::{
            AppState,
            assembly::prelude::{ApiError, run_blocking},
        };
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
                let pool = state.pool.clone();
                let messages = run_blocking(move || list(&pool)).await?;
                Ok(Json(messages))
            }
        }
    }

    pub mod prelude {
        pub use super::http::Api;
        pub use super::models::OutboxMessageDto;
    }
}

pub mod prelude {
    pub use super::AppState;
    pub use super::assembly::prelude::{
        ApiError,
        AppCommand,
        AppError,
        Command,
        DbPool,
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
        build_pool,
        fetch_todo,
        interpret_dispatch_error,
        record_event_payload,
        run_blocking,
        run_command_worker,
        run_event_worker,
        run_migrations,
        start_mulac,
        validate_title,
        //
    };
    pub use super::inbox::prelude::Api as InboxApi;
    pub use super::outbox::prelude::{Api as OutboxApi, OutboxMessageDto};
    pub use super::task_complete::prelude::Api as CompleteApi;
    pub use super::task_create::prelude::Api as CreateApi;
    pub use super::task_delete::prelude::Api as DeleteApi;
    pub use super::task_get::prelude::Api as GetApi;
    pub use super::task_list::prelude::Api as ListApi;
    pub use super::task_list::prelude::FilterStatus;
    pub use super::task_reopen::prelude::Api as ReopenApi;
    pub use super::task_schedule_due_dates::prelude::Api as DueDatesApi;
    pub use super::task_update::prelude::Api as UpdateApi;
}
