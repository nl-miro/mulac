pub mod io {
    pub use super::implementation::CreateTodoHandler;
    pub use super::intention::{CreateTodo, TodoCreated};
    pub use super::ui::Api;
}

mod intention {
    use crate::assembly::io::validate_title;
    use crate::assembly::io::{AppError, TodoStatus};
    use chrono::{DateTime, Utc};
    use kernel::{ApplicationCommand, ApplicationEvent};
    use poem_openapi::Object;
    use serde::{Deserialize, Serialize};
    use uuid::Uuid;

    /// Asks the system to create a new todo.
    #[derive(Debug, Clone, Serialize, Deserialize, Object, ApplicationCommand)]
    #[command_type = "CreateTodo"]
    pub struct CreateTodo {
        pub todo_id: Uuid,
        pub title: String,
        pub description: Option<String>,
        pub due_at: Option<DateTime<Utc>>,
    }

    impl CreateTodo {
        pub fn create(todo_id: Uuid, title: String, description: Option<String>, due_at: Option<DateTime<Utc>>) -> Self {
            Self { todo_id, title, description, due_at }
        }

        /// Begin a todo: a created todo must be named, and it always starts
        /// `Active` at the given moment. The business rules of creation live
        /// here, in plain language, rather than in the mechanics.
        pub fn begin(self, now: DateTime<Utc>) -> Result<NewTodo, AppError> {
            validate_title(&self.title)?;

            Ok(NewTodo {
                id: self.todo_id,
                title: self.title,
                description: self.description,
                due_at: self.due_at,
                status: TodoStatus::Active,
                created_at: now,
            })
        }
    }
    /// A todo at the instant of creation: it has been named, it begins `Active`,
    /// and it is stamped with the moment it came into being. This is the
    /// feature's core business act — turning a request to remember something
    /// into a committed, active task.
    #[derive(Debug, Clone)]
    pub struct NewTodo {
        pub id: Uuid,
        pub title: String,
        pub description: Option<String>,
        pub due_at: Option<DateTime<Utc>>,
        pub status: TodoStatus,
        pub created_at: DateTime<Utc>,
    }

    /// States that a todo was created.
    #[derive(Debug, Clone, Serialize, Deserialize, Object, ApplicationEvent)]
    #[event_type = "TodoCreated"]
    pub struct TodoCreated {
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
    use super::intention::CreateTodo;
    use crate::AppState;
    use crate::assembly::io::{ApiError, TodoEntry, dispatch_command, fetch_todo};
    use chrono::{DateTime, Utc};
    use poem::web::Data;
    use poem_openapi::payload::Json;
    use poem_openapi::{Object, OpenApi};
    use serde::{Deserialize, Serialize};
    use uuid::Uuid;

    pub struct Api;

    #[OpenApi]
    impl Api {
        #[oai(path = "/todos", method = "post")]
        async fn create_todo(&self, state: Data<&AppState>, Json(request): Json<CreateTodoRequest>) -> Result<Json<TodoEntry>, ApiError> {
            let todo_id = Uuid::now_v7();
            let cmd = request.into_command(todo_id);

            dispatch_command(&state.mulac, cmd)?;

            Ok(Json(fetch_todo(&state.pool, todo_id).await?))
        }
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Object)]
    pub struct CreateTodoRequest {
        pub title: String,
        pub description: Option<String>,
        pub due_at: Option<DateTime<Utc>>,
    }

    impl CreateTodoRequest {
        // Boundary adapter: turn an inbound request into the feature's command.
        fn into_command(self, todo_id: Uuid) -> CreateTodo {
            CreateTodo::create(todo_id, self.title, self.description, self.due_at)
        }
    }
}

mod implementation {
    use super::intention::{CreateTodo, NewTodo, TodoCreated};
    use crate::assembly::io::{
        Clock,
        TodoEntry,
        TodoEvent,
        block_on_blocking,
        insert_todo, //
    };
    use derive_new::new;
    use kernel::io::DbPool;
    use kernel::{CommandError, CommandHandlerPort};
    use std::sync::Arc;

    #[derive(new)]
    pub struct CreateTodoHandler {
        pub(super) pool: Arc<DbPool>,
    }

    // The mechanics only do structural mapping; the creation rules live in
    // `CreateTodo::begin` over in `intention`.

    impl From<NewTodo> for TodoEntry {
        fn from(todo: NewTodo) -> Self {
            TodoEntry {
                id: todo.id,
                title: todo.title,
                description: todo.description,
                status: todo.status,
                created_at: todo.created_at,
                updated_at: todo.created_at,
                due_at: todo.due_at,
            }
        }
    }

    impl From<TodoEntry> for TodoCreated {
        fn from(todo: TodoEntry) -> Self {
            Self {
                id: todo.id,
                title: todo.title,
                description: todo.description,
                status: todo.status,
                due_at: todo.due_at,
                created_at: todo.created_at,
                updated_at: todo.updated_at,
            }
        }
    }

    // The handler reads as the feature's story: state the business act
    // (`begin`), persist it, announce what happened.
    impl CommandHandlerPort<CreateTodo, TodoEvent> for CreateTodoHandler {
        fn execute(&self, command: CreateTodo) -> Result<Vec<TodoEvent>, CommandError> {
            let new_todo = command.begin(Clock::now())?;

            let persisted_entity = self.persist(new_todo.into())?;

            Ok(vec![TodoEvent::TodoCreated(persisted_entity.into())])
        }
    }

    impl CreateTodoHandler {
        pub fn persist(&self, row: TodoEntry) -> Result<TodoEntry, CommandError> {
            let pool = self.pool.clone();
            let future = async move { insert_todo(&pool, row).await };

            block_on_blocking(future).map_err(CommandError::from)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::intention::{CreateTodo, TodoCreated};
    use crate::assembly::io::TodoStatus;
    use crate::task_create::ui::CreateTodoRequest;
    use uuid::Uuid;

    fn sample_command(title: &str) -> CreateTodo {
        CreateTodo::create(Uuid::now_v7(), title.to_string(), None, None)
    }

    #[test]
    fn begin_starts_the_todo_active_and_stamped() {
        let now = chrono::Utc::now();

        let new_todo = sample_command("Buy milk").begin(now).expect("a named todo should begin");

        assert_eq!(new_todo.status, TodoStatus::Active);
        assert_eq!(new_todo.created_at, now);
    }

    #[test]
    fn begin_refuses_an_unnamed_todo() {
        let result = sample_command("   ").begin(chrono::Utc::now());

        assert!(result.is_err(), "a blank title is not a meaningful task");
    }

    #[test]
    fn command_and_event_types_match_contract() {
        assert_eq!(CreateTodo::COMMAND_TYPE, "CreateTodo");
        assert_eq!(TodoCreated::EVENT_TYPE, "TodoCreated");
    }

    #[test]
    fn request_preserves_due_date_and_description() {
        let due_at = chrono::Utc::now();
        let request =
            CreateTodoRequest { title: "Buy milk".to_string(), description: Some("At the corner store".to_string()), due_at: Some(due_at) };

        assert_eq!(request.title, "Buy milk");
        assert_eq!(request.description.as_deref(), Some("At the corner store"));
        assert_eq!(request.due_at, Some(due_at));
    }
}
