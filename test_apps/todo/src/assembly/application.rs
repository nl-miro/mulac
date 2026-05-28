use super::domain::{TodoEntry, TodoStatus};
use super::infra_sqlx_pg::entity::TodoRow;
use crate::task_complete::io::{CompleteTodo, CompleteTodoHandler, TodoCompleted};
use crate::task_create::io::{CreateTodo, CreateTodoHandler, TodoCreated};
use crate::task_delete::io::{DeleteTodo, DeleteTodoHandler, TodoDeleted};
use crate::task_reopen::io::{ReopenTodo, ReopenTodoHandler, TodoReopened};
use crate::task_schedule_due_dates::io::{TodoDueDateChanged, UpdateDueDate, UpdateDueDateHandler};
use crate::task_update::io::{TodoUpdated, UpdateTodo, UpdateTodoHandler};
use kernel::io::{CommandError as MulacCommandError, DbPool, build_pool};
use kernel::{
    ApplicationCommand, CommandHandlers, EventSubscriberPort, EventSubscribers, KernelBuilder, KernelError, NewCommandEnvelope,
    NewCommandMetadata,
};
use poem::{IntoResponse, Response, http::StatusCode};
use poem_openapi::Object;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, Object)]
pub struct ErrorBody {
    pub error: String,
}

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("todo not found")]
    NotFound,
    #[error("validation failed: {0}")]
    Validation(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    Domain(String),
    #[error("storage error: {0}")]
    Storage(#[from] anyhow::Error),
}

pub type ApiError = poem::Error;

impl From<AppError> for poem::Error {
    fn from(error: AppError) -> Self {
        let status = match error {
            AppError::NotFound => StatusCode::NOT_FOUND,
            AppError::Validation(_) => StatusCode::BAD_REQUEST,
            AppError::Conflict(_) => StatusCode::CONFLICT,
            AppError::Domain(_) => StatusCode::BAD_REQUEST,
            AppError::Storage(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        poem::Error::from_response(error_response(status, error.to_string()))
    }
}

fn error_response(status: StatusCode, message: String) -> Response {
    (status, poem::web::Json(ErrorBody { error: message })).into_response()
}

// Translate the app's error vocabulary into the command layer's: business
// conditions become `Domain`, storage failures stay storage failures. Lives in
// `assembly` because it is a crate-wide boundary shared by every feature's
// command handler, not the property of any single feature.
impl From<AppError> for kernel::CommandError {
    fn from(error: AppError) -> Self {
        match error {
            AppError::Storage(e) => kernel::CommandError::Storage(e.to_string()),
            AppError::NotFound | AppError::Validation(_) | AppError::Conflict(_) | AppError::Domain(_) => {
                kernel::CommandError::Domain(error.to_string())
            }
        }
    }
}

impl TryFrom<TodoRow> for TodoEntry {
    type Error = AppError;

    fn try_from(row: TodoRow) -> Result<Self, Self::Error> {
        let status = match row.status.as_str() {
            "active" => TodoStatus::Active,
            "completed" => TodoStatus::Completed,
            "archived" => TodoStatus::Archived,
            other => {
                return Err(AppError::Storage(anyhow::anyhow!("unknown todo status `{other}`")));
            }
        };
        Ok(TodoEntry {
            id: row.id,
            title: row.title,
            description: row.description,
            status,
            created_at: row.created_at,
            updated_at: row.updated_at,
            due_at: row.due_at,
        })
    }
}

pub fn validate_title(title: &str) -> Result<(), AppError> {
    if title.trim().is_empty() {
        return Err(AppError::Validation("title must not be blank".to_string()));
    }
    Ok(())
}

pub fn interpret_dispatch_error(error: kernel::KernelError) -> AppError {
    if let kernel::KernelError::Command(command_error) = &error {
        match command_error {
            MulacCommandError::Domain(message) => {
                if message == "todo not found" {
                    return AppError::NotFound;
                }

                if let Some(message) = message.strip_prefix("validation failed: ") {
                    return AppError::Validation(message.to_string());
                }

                return AppError::Domain(message.clone());
            }
            MulacCommandError::HandlerExecution(message) => {
                if message.contains("todo not found") {
                    return AppError::NotFound;
                }

                if let Some(message) = message.strip_prefix("validation failed: ") {
                    return AppError::Validation(message.to_string());
                }
            }
            _ => {}
        }
    }

    AppError::Storage(anyhow::anyhow!("command dispatch failed: {error}"))
}

pub fn dispatch_command<C: ApplicationCommand>(mulac: &kernel::PersistentKernelState, command: C) -> Result<(), AppError> {
    let command_id = Uuid::now_v7();

    let metadata = NewCommandMetadata {
        command_id,
        correlation_id: Some(command_id),
        causation_id: None,
        source: Some("test_app_todo.http".to_string()),
    };

    let envelope = NewCommandEnvelope { command, metadata };

    mulac.dispatch_command(envelope).map_err(interpret_dispatch_error)
}

pub type MulacState = kernel::PersistentKernelState;
pub type MulacHandle = kernel::PersistentKernelHandle;

pub async fn start_mulac(pool: kernel::io::DbPool, database_url: &str) -> Result<MulacHandle, kernel::KernelError> {
    use crate::assembly::io::OutboxSubscriber;
    use crate::task_complete::io::{CompleteTodoHandler, TodoCompleted};
    use crate::task_create::io::{CreateTodoHandler, TodoCreated};
    use crate::task_delete::io::{DeleteTodoHandler, TodoDeleted};
    use crate::task_reopen::io::{ReopenTodoHandler, TodoReopened};
    use crate::task_schedule_due_dates::io::{TodoDueDateChanged, UpdateDueDateHandler};
    use crate::task_update::io::{TodoUpdated, UpdateTodoHandler};

    let db_pool = build_pool(database_url).map_err(|e| kernel::KernelError::Database(e.to_string()))?;
    let arced_db_pool = Arc::new(db_pool.clone());

    let outbox_subscriber = Arc::new(OutboxSubscriber::new(pool.clone())) as Arc<dyn kernel::EventSubscriberPort>;

    let command_handlers = command_list(arced_db_pool.clone());

    let event_subscribers = EventSubscribers::new()
        .register(TodoCreated::EVENT_TYPE, "todo-created-outbox", outbox_subscriber.clone())
        .register(TodoCompleted::EVENT_TYPE, "todo-completed-outbox", outbox_subscriber.clone())
        .register(TodoReopened::EVENT_TYPE, "todo-reopened-outbox", outbox_subscriber.clone())
        .register(TodoUpdated::EVENT_TYPE, "todo-updated-outbox", outbox_subscriber.clone())
        .register(TodoDueDateChanged::EVENT_TYPE, "todo-due-date-changed-outbox", outbox_subscriber.clone())
        .register(TodoDeleted::EVENT_TYPE, "todo-deleted-outbox", outbox_subscriber.clone());

    let kernel = kernel::boot(kernel::KernelConfig::default())
        .command_handlers(command_handlers)
        .event_subscribers(event_subscribers)
        .start_persistent(db_pool, 1)?;
    //
    // kernel::boot(kernel::KernelConfig::default())
    //     .command_handler(CreateTodo::COMMAND_TYPE, Arc::new(CreateTodoHandler::new(pool.clone())))
    //     .command_handler(CompleteTodo::COMMAND_TYPE, Arc::new(CompleteTodoHandler::new(pool.clone())))
    //     .command_handler(ReopenTodo::COMMAND_TYPE, Arc::new(ReopenTodoHandler::new(pool.clone())))
    //     .command_handler(UpdateTodo::COMMAND_TYPE, Arc::new(UpdateTodoHandler::new(pool.clone())))
    //     .command_handler(DeleteTodo::COMMAND_TYPE, Arc::new(DeleteTodoHandler::new(pool.clone())))
    //     .command_handler(UpdateDueDate::COMMAND_TYPE, Arc::new(UpdateDueDateHandler::new(pool.clone())))
    //     .event_subscriber(TodoCreated::EVENT_TYPE, "todo-created-outbox", outbox_subscriber.clone())
    //     .event_subscriber(TodoCompleted::EVENT_TYPE, "todo-completed-outbox", outbox_subscriber.clone())
    //     .event_subscriber(TodoReopened::EVENT_TYPE, "todo-reopened-outbox", outbox_subscriber.clone())
    //     .event_subscriber(TodoUpdated::EVENT_TYPE, "todo-updated-outbox", outbox_subscriber.clone())
    //     .event_subscriber(TodoDueDateChanged::EVENT_TYPE, "todo-due-date-changed-outbox", outbox_subscriber.clone())
    //     .event_subscriber(TodoDeleted::EVENT_TYPE, "todo-deleted-outbox", outbox_subscriber.clone())
    //     .start_persistent(db_pool, 1)

    Ok(kernel)
}

fn command_list(pool: Arc<DbPool>) -> CommandHandlers {
    CommandHandlers::new()
        .register(CreateTodo::COMMAND_TYPE, Arc::new(CreateTodoHandler::new(pool.clone())))
        .register(CompleteTodo::COMMAND_TYPE, Arc::new(CompleteTodoHandler::new(pool.clone())))
        .register(ReopenTodo::COMMAND_TYPE, Arc::new(ReopenTodoHandler::new(pool.clone())))
        .register(UpdateTodo::COMMAND_TYPE, Arc::new(UpdateTodoHandler::new(pool.clone())))
        .register(DeleteTodo::COMMAND_TYPE, Arc::new(DeleteTodoHandler::new(pool.clone())))
        .register(UpdateDueDate::COMMAND_TYPE, Arc::new(UpdateDueDateHandler::new(pool.clone())))
}

pub use kernel::io::{block_on_blocking, run_command_worker, run_event_worker};
