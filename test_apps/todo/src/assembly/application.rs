use super::domain::{TodoDto, TodoStatus};
use super::infra_sqlx_pg::entity::TodoRow;
use crate::task_complete::io::{COMPLETE_TODO_COMMAND, CompleteTodoCommand};
use crate::task_create::io::{CREATE_TODO_COMMAND, CreateTodoCommand};
use crate::task_delete::io::{DELETE_TODO_COMMAND, DeleteTodoCommand};
use crate::task_reopen::io::{REOPEN_TODO_COMMAND, ReopenTodoCommand};
use crate::task_schedule_due_dates::io::{UPDATE_DUE_DATE_COMMAND, UpdateDueDateCommand};
use crate::task_update::io::{UPDATE_TODO_COMMAND, UpdateTodoCommand};
use ::commanding::io::CommandError as MulacCommandError;
use ::commanding::io::NewCommandMetadata;
use poem::{IntoResponse, Response, http::StatusCode};
use poem_openapi::Object;
use serde::{Deserialize, Serialize};
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
            AppError::Storage(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        poem::Error::from_response(error_response(status, error.to_string()))
    }
}

fn error_response(status: StatusCode, message: String) -> Response {
    (status, poem::web::Json(ErrorBody { error: message })).into_response()
}

impl TryFrom<TodoRow> for TodoDto {
    type Error = AppError;

    fn try_from(row: TodoRow) -> Result<Self, Self::Error> {
        let status = match row.status.as_str() {
            "active" => TodoStatus::Active,
            "completed" => TodoStatus::Completed,
            "archived" => TodoStatus::Archived,
            other => {
                return Err(AppError::Storage(anyhow::anyhow!(
                    "unknown todo status `{other}`"
                )));
            }
        };
        Ok(TodoDto {
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
    if let kernel::KernelError::Command(MulacCommandError::HandlerExecution(message)) = &error {
        if message.contains("todo not found") {
            return AppError::NotFound;
        }

        if let Some(message) = message.strip_prefix("validation failed: ") {
            return AppError::Validation(message.to_string());
        }
    }

    AppError::Storage(anyhow::anyhow!("command dispatch failed: {error}"))
}

pub trait Command: kernel::ApplicationCommand {
    fn todo_id(&self) -> Uuid;
}

#[derive(Debug, Clone)]
pub enum AppCommand {
    CreateTodo(CreateTodoCommand),
    CompleteTodo(CompleteTodoCommand),
    ReopenTodo(ReopenTodoCommand),
    UpdateTodo(UpdateTodoCommand),
    DeleteTodo(DeleteTodoCommand),
    UpdateDueDate(UpdateDueDateCommand),
}

impl serde::Serialize for AppCommand {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::CreateTodo(c) => c.serialize(serializer),
            Self::CompleteTodo(c) => c.serialize(serializer),
            Self::ReopenTodo(c) => c.serialize(serializer),
            Self::UpdateTodo(c) => c.serialize(serializer),
            Self::DeleteTodo(c) => c.serialize(serializer),
            Self::UpdateDueDate(c) => c.serialize(serializer),
        }
    }
}

impl kernel::ApplicationCommand for AppCommand {
    fn command_type(&self) -> &'static str {
        match self {
            Self::CreateTodo(_) => CREATE_TODO_COMMAND,
            Self::CompleteTodo(_) => COMPLETE_TODO_COMMAND,
            Self::ReopenTodo(_) => REOPEN_TODO_COMMAND,
            Self::UpdateTodo(_) => UPDATE_TODO_COMMAND,
            Self::DeleteTodo(_) => DELETE_TODO_COMMAND,
            Self::UpdateDueDate(_) => UPDATE_DUE_DATE_COMMAND,
        }
    }
}

impl Command for AppCommand {
    fn todo_id(&self) -> Uuid {
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

pub struct NewCommandEnvelope {
    pub command: AppCommand,
    pub metadata: NewCommandMetadata,
}

mod eventing {
    use eventing::io::{EventConsumer, ReservableEventSpec};
    use kernel::{EventError, EventSubscriberPort};
    use std::{collections::HashMap, sync::Arc, time::Duration};
    use tokio_util::sync::CancellationToken;

    pub struct EventSubscriberRegistry {
        subscribers: HashMap<String, Vec<(String, Arc<dyn EventSubscriberPort>)>>,
    }

    impl EventSubscriberRegistry {
        pub fn from_subscribers(
            subscribers: Vec<(String, String, Arc<dyn EventSubscriberPort>)>,
        ) -> Self {
            let mut by_event: HashMap<String, Vec<(String, Arc<dyn EventSubscriberPort>)>> =
                HashMap::new();

            for (event_type, subscriber_name, subscriber) in subscribers {
                by_event
                    .entry(event_type)
                    .or_default()
                    .push((subscriber_name, subscriber));
            }

            Self {
                subscribers: by_event,
            }
        }
    }

    impl EventSubscriberPort for EventSubscriberRegistry {
        fn handle(&self, envelope: &kernel::NewEventEnvelope) -> Result<(), EventError> {
            let subscribers = self
                .subscribers
                .get(&envelope.event_type)
                .into_iter()
                .flatten();

            for (_, subscriber) in subscribers {
                subscriber.handle(envelope)?;
            }
            Ok(())
        }
    }

    pub async fn run_event_worker(consumer: Arc<EventConsumer>, token: CancellationToken) {
        loop {
            tokio::select! {
                _ = token.cancelled() => break,
                _ = tokio::time::sleep(Duration::from_secs(1)) => {}
            }
            let c = Arc::clone(&consumer);
            match tokio::task::spawn_blocking(move || c.consume(&ReservableEventSpec::new(10)))
                .await
            {
                Ok(Ok(())) => {}
                Ok(Err(errs)) => {
                    for e in &errs {
                        tracing::error!("event worker: {e}");
                    }
                }
                Err(e) => tracing::error!("event worker panicked: {e}"),
            }
        }
    }
}

mod commanding {
    use super::NewCommandEnvelope;
    use super::eventing::EventSubscriberRegistry;
    use crate::assembly::io::OutboxSubscriber;
    use crate::task_complete::io::{
        COMPLETE_TODO_COMMAND, CompleteTodoHandler, TODO_COMPLETED_EVENT,
    };
    use crate::task_create::io::{CREATE_TODO_COMMAND, CreateTodoHandler, TODO_CREATED_EVENT};
    use crate::task_delete::io::{DELETE_TODO_COMMAND, DeleteTodoHandler, TODO_DELETED_EVENT};
    use crate::task_reopen::io::{REOPEN_TODO_COMMAND, ReopenTodoHandler, TODO_REOPENED_EVENT};
    use crate::task_schedule_due_dates::io::{
        TODO_DUE_DATE_CHANGED_EVENT, UPDATE_DUE_DATE_COMMAND, UpdateDueDateHandler,
    };
    use crate::task_update::io::{TODO_UPDATED_EVENT, UPDATE_TODO_COMMAND, UpdateTodoHandler};
    use commanding::io::{
        CommandConsumer,
        CommandConsumerRepository,
        CommandConsumerStorage,
        CommandDispatcher,
        CommandError,
        CommandGateway,
        CommandRecorder,
        CommandRecorderRepository,
        CommandStoreStorage,
        ErasedCommandHandler,
        ReservableCommandSpec,
        wrap_handler, //
    };
    use eventing::io::{
        EventConsumer,
        EventConsumerRepository,
        EventConsumerStorage,
        EventDispatcher,
        EventGateway,
        EventRecorder,
        EventRecorderRepository,
        EventStoreStorage,
        ReservableEventSpec, //
    };
    use kernel::{EventError, EventSubscriberPort, KernelError};
    use mulac_diesel::build_pool;
    use sqlx::PgPool;
    use std::{collections::HashMap, future::Future, sync::Arc};
    use tokio_util::sync::CancellationToken;

    const CONSUMER_BATCH_SIZE: usize = 64;

    #[derive(Clone)]
    pub struct MulacState {
        command_gateway: Arc<CommandGateway>,
        command_consumer: Arc<CommandConsumer>,
        event_consumer: Arc<EventConsumer>,
    }

    impl MulacState {
        pub fn dispatch_command(&self, envelope: NewCommandEnvelope) -> Result<(), KernelError> {
            let envelope = kernel::NewCommandEnvelope {
                command: envelope.command,
                metadata: envelope.metadata,
            }
            .into_gateway_envelope()
            .map_err(|e| KernelError::Command(CommandError::Conversion(e.to_string())))?;

            self.command_gateway.dispatch(envelope)?;

            self.command_consumer
                .consume(&ReservableCommandSpec::new(CONSUMER_BATCH_SIZE))
                .map_err(first_command_error)?;

            self.event_consumer
                .consume(&ReservableEventSpec::new(CONSUMER_BATCH_SIZE))
                .map_err(first_event_error)?;

            Ok(())
        }
    }

    pub struct MulacHandle {
        state: MulacState,
        token: CancellationToken,
    }

    impl MulacHandle {
        pub fn state(&self) -> MulacState {
            self.state.clone()
        }

        pub fn child_token(&self) -> CancellationToken {
            self.token.child_token()
        }

        pub fn command_consumer(&self) -> Arc<CommandConsumer> {
            self.state.command_consumer.clone()
        }

        pub fn event_consumer(&self) -> Arc<EventConsumer> {
            self.state.event_consumer.clone()
        }

        pub fn shutdown(&self) {
            self.token.cancel();
        }

        pub async fn wait(self) -> Result<(), KernelError> {
            self.token.cancel();
            Ok(())
        }
    }

    pub async fn start_mulac(pool: PgPool, database_url: &str) -> Result<MulacHandle, KernelError> {
        let db_pool = build_pool(database_url).map_err(|e| KernelError::Database(e.to_string()))?;

        let command_registry = Arc::new(CommandHandlerRegistry::from_handlers(vec![
            (
                CREATE_TODO_COMMAND.to_string(),
                wrap_handler(Arc::new(CreateTodoHandler::new(pool.clone()))),
            ),
            (
                COMPLETE_TODO_COMMAND.to_string(),
                wrap_handler(Arc::new(CompleteTodoHandler::new(pool.clone()))),
            ),
            (
                REOPEN_TODO_COMMAND.to_string(),
                wrap_handler(Arc::new(ReopenTodoHandler::new(pool.clone()))),
            ),
            (
                UPDATE_TODO_COMMAND.to_string(),
                wrap_handler(Arc::new(UpdateTodoHandler::new(pool.clone()))),
            ),
            (
                DELETE_TODO_COMMAND.to_string(),
                wrap_handler(Arc::new(DeleteTodoHandler::new(pool.clone()))),
            ),
            (
                UPDATE_DUE_DATE_COMMAND.to_string(),
                wrap_handler(Arc::new(UpdateDueDateHandler::new(pool.clone()))),
            ),
        ]));

        let event_registry = Arc::new(EventSubscriberRegistry::from_subscribers(vec![
            (
                TODO_CREATED_EVENT.to_string(),
                "todo-created-outbox".to_string(),
                Arc::new(OutboxSubscriber::new(pool.clone())) as Arc<dyn EventSubscriberPort>,
            ),
            (
                TODO_COMPLETED_EVENT.to_string(),
                "todo-completed-outbox".to_string(),
                Arc::new(OutboxSubscriber::new(pool.clone())) as Arc<dyn EventSubscriberPort>,
            ),
            (
                TODO_REOPENED_EVENT.to_string(),
                "todo-reopened-outbox".to_string(),
                Arc::new(OutboxSubscriber::new(pool.clone())) as Arc<dyn EventSubscriberPort>,
            ),
            (
                TODO_UPDATED_EVENT.to_string(),
                "todo-updated-outbox".to_string(),
                Arc::new(OutboxSubscriber::new(pool.clone())) as Arc<dyn EventSubscriberPort>,
            ),
            (
                TODO_DUE_DATE_CHANGED_EVENT.to_string(),
                "todo-due-date-changed-outbox".to_string(),
                Arc::new(OutboxSubscriber::new(pool.clone())) as Arc<dyn EventSubscriberPort>,
            ),
            (
                TODO_DELETED_EVENT.to_string(),
                "todo-deleted-outbox".to_string(),
                Arc::new(OutboxSubscriber::new(pool.clone())) as Arc<dyn EventSubscriberPort>,
            ),
        ]));

        let event_dispatcher = Arc::new(EventDispatcher::new(event_registry));

        let event_store = Arc::new(EventStoreStorage::new(db_pool.clone()));
        let event_recorder = Arc::new(EventRecorder::new(Arc::new(EventRecorderRepository::new(
            event_store,
        ))));
        let event_gateway = Arc::new(EventGateway::two_phased(event_recorder));

        let command_dispatcher = Arc::new(CommandDispatcher::new(command_registry, event_gateway));

        let command_store = Arc::new(CommandStoreStorage::new(db_pool.clone()));
        let command_recorder = Arc::new(CommandRecorder::new(Arc::new(
            CommandRecorderRepository::new(command_store),
        )));
        let command_gateway = Arc::new(CommandGateway::two_phased(command_recorder));

        let command_storage = Arc::new(CommandConsumerStorage::new(db_pool.clone()));
        let command_consumer_repository =
            CommandConsumerRepository::new(command_storage.clone(), command_storage);
        let command_consumer = Arc::new(CommandConsumer::new(
            command_consumer_repository,
            command_dispatcher,
        ));

        let event_storage = Arc::new(EventConsumerStorage::new(db_pool));
        let event_consumer_repository =
            EventConsumerRepository::new(event_storage.clone(), event_storage);
        let event_consumer = Arc::new(EventConsumer::new(
            event_consumer_repository,
            event_dispatcher,
        ));

        Ok(MulacHandle {
            state: MulacState {
                command_gateway,
                command_consumer,
                event_consumer,
            },
            token: CancellationToken::new(),
        })
    }

    fn first_command_error(errors: Vec<CommandError>) -> KernelError {
        match errors.into_iter().next() {
            Some(error) => KernelError::Command(error),
            None => KernelError::Worker("command consumer failed without an error".to_string()),
        }
    }

    fn first_event_error(errors: Vec<EventError>) -> KernelError {
        match errors.into_iter().next() {
            Some(error) => KernelError::Event(error),
            None => KernelError::Worker("event consumer failed without an error".to_string()),
        }
    }

    struct CommandHandlerRegistry {
        handlers: HashMap<String, Arc<dyn ErasedCommandHandler>>,
    }

    impl CommandHandlerRegistry {
        fn from_handlers(handlers: Vec<(String, Arc<dyn ErasedCommandHandler>)>) -> Self {
            Self {
                handlers: handlers.into_iter().collect(),
            }
        }
    }

    impl ErasedCommandHandler for CommandHandlerRegistry {
        fn execute(
            &self,
            envelope: &kernel::GatewayNewCommandEnvelope,
        ) -> Result<Vec<kernel::NewEventEnvelope>, CommandError> {
            let handler = self
                .handlers
                .get(&envelope.command.command_type)
                .ok_or_else(|| {
                    CommandError::HandlerNotFound(envelope.command.command_type.clone())
                })?;
            handler.execute(envelope)
        }
    }

    pub fn block_on_blocking<F>(future: F) -> F::Output
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(future))
    }

    pub async fn run_command_worker(consumer: Arc<CommandConsumer>, token: CancellationToken) {
        loop {
            tokio::select! {
                _ = token.cancelled() => break,
                _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {}
            }
            let c = Arc::clone(&consumer);
            match tokio::task::spawn_blocking(move || c.consume(&ReservableCommandSpec::new(10)))
                .await
            {
                Ok(Ok(())) => {}
                Ok(Err(errs)) => {
                    for e in &errs {
                        tracing::error!("command worker: {e}");
                    }
                }
                Err(e) => tracing::error!("command worker panicked: {e}"),
            }
        }
    }
}

pub use commanding::{MulacHandle, MulacState, block_on_blocking, run_command_worker, start_mulac};
pub use eventing::run_event_worker;
