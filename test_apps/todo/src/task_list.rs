use crate::io::TodoStatus;
use poem_openapi::Enum;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize, Enum)]
#[serde(rename_all = "snake_case")]
#[oai(rename_all = "snake_case")]
pub enum FilterStatus {
    Active,
    Completed,
    Archived,
    All,
}

impl FilterStatus {
    pub fn as_todo_status(self) -> Option<TodoStatus> {
        match self {
            Self::Active => Some(TodoStatus::Active),
            Self::Completed => Some(TodoStatus::Completed),
            Self::Archived => Some(TodoStatus::Archived),
            Self::All => None,
        }
    }
}

mod infra_sqlx_pg {
    use crate::assembly::io::{AppError, TodoDto, TodoList, TodoRow};
    use crate::task_list::FilterStatus;
    use sqlx::PgPool;

    pub async fn list(pool: &PgPool, status: Option<FilterStatus>) -> Result<TodoList, AppError> {
        let sql_filtered = "SELECT id, title, description, status, created_at, updated_at, due_at FROM todos WHERE status = $1 ORDER BY created_at ASC";
        let sql_all = "SELECT id, title, description, status, created_at, updated_at, due_at FROM todos ORDER BY created_at ASC";

        let rows = if let Some(status) = status.and_then(FilterStatus::as_todo_status) {
            sqlx::query_as::<_, TodoRow>(sql_filtered)
                .bind(status.as_str())
                .fetch_all(pool)
                .await
        } else {
            sqlx::query_as::<_, TodoRow>(sql_all).fetch_all(pool).await
        }
        .map_err(|e| AppError::Storage(e.into()))?;

        let items = rows
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<TodoDto>, AppError>>()?;
        Ok(TodoList { items })
    }
}

mod http {
    use crate::AppState;
    use crate::assembly::io::{ApiError, TodoList};
    use crate::task_list::FilterStatus;
    use poem::web::Data;
    use poem_openapi::{OpenApi, param::Query, payload::Json};

    pub struct Api;

    #[OpenApi]
    impl Api {
        #[oai(path = "/todos", method = "get")]
        async fn list_todos(
            &self,
            state: Data<&AppState>,
            status: Query<Option<FilterStatus>>,
        ) -> Result<Json<TodoList>, ApiError> {
            Ok(Json(
                super::infra_sqlx_pg::list(&state.pool, status.0).await?,
            ))
        }
    }
}

pub mod io {
    pub use super::FilterStatus;
    pub use super::http::Api;
}
