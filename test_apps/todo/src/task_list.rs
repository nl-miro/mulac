pub mod io {
    pub use super::intention::FilterStatus;
    pub use super::ui::Api;
}

mod intention {
    use crate::assembly::io::TodoStatus;
    use poem_openapi::Enum;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Copy, Eq, PartialEq)]
    pub struct TodoListing {
        status: Option<TodoStatus>,
    }

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

        pub fn listing(status: Option<Self>) -> TodoListing {
            TodoListing { status: status.and_then(Self::as_todo_status) }
        }
    }

    impl TodoListing {
        pub fn requested_status(self) -> Option<TodoStatus> {
            self.status
        }
    }
}

mod ui {
    use super::implementation::list;
    use super::intention::FilterStatus;
    use crate::AppState;
    use crate::assembly::io::{ApiError, TodoList};
    use poem::web::Data;
    use poem_openapi::{OpenApi, param::Query, payload::Json};

    pub struct Api;

    #[OpenApi]
    impl Api {
        #[oai(path = "/todos", method = "get")]
        async fn list_todos(&self, state: Data<&AppState>, status: Query<Option<FilterStatus>>) -> Result<Json<TodoList>, ApiError> {
            Ok(Json(list(&state.pool, FilterStatus::listing(status.0)).await?))
        }
    }
}

mod implementation {
    use super::intention::TodoListing;
    use crate::assembly::io::{AppError, TodoEntry, TodoList, TodoRow};
    use sqlx::PgPool;

    pub async fn list(pool: &kernel::io::DbPool, listing: TodoListing) -> Result<TodoList, AppError> {
        let sql_filtered =
            "SELECT id, title, description, status, created_at, updated_at, due_at FROM todos WHERE status = $1 ORDER BY created_at ASC";
        let sql_all = "SELECT id, title, description, status, created_at, updated_at, due_at FROM todos ORDER BY created_at ASC";

        let rows = if let Some(status) = listing.requested_status() {
            sqlx::query_as::<_, TodoRow>(sql_filtered).bind(status.as_str()).fetch_all(pool).await
        } else {
            sqlx::query_as::<_, TodoRow>(sql_all).fetch_all(pool).await
        }
        .map_err(|e| AppError::Storage(e.into()))?;

        let items = rows.into_iter().map(TryInto::try_into).collect::<Result<Vec<TodoEntry>, AppError>>()?;

        Ok(TodoList { items })
    }
}

#[cfg(test)]
mod tests {
    use super::intention::FilterStatus;
    use crate::assembly::io::TodoStatus;

    #[test]
    fn all_filter_requests_all_statuses() {
        assert_eq!(FilterStatus::listing(Some(FilterStatus::All)).requested_status(), None);
    }

    #[test]
    fn completed_filter_requests_completed_status() {
        assert_eq!(FilterStatus::listing(Some(FilterStatus::Completed)).requested_status(), Some(TodoStatus::Completed));
    }
}
