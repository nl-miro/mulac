pub mod io {
    pub use super::ui::Api;
}

mod intention {
    use uuid::Uuid;

    pub struct GetTodoQuery {
        pub todo_id: Uuid,
    }

    impl GetTodoQuery {
        pub fn lookup(todo_id: Uuid) -> Self {
            Self { todo_id }
        }

        pub fn requested_todo(&self) -> Uuid {
            self.todo_id
        }
    }
}

mod ui {
    use super::implementation::get;
    use super::intention::GetTodoQuery;
    use crate::AppState;
    use crate::assembly::io::{ApiError, TodoEntry};
    use poem::web::Data;
    use poem_openapi::{OpenApi, param::Path, payload::Json};
    use uuid::Uuid;

    pub struct Api;

    #[OpenApi]
    impl Api {
        #[oai(path = "/todos/:id", method = "get")]
        async fn get_todo(&self, state: Data<&AppState>, id: Path<Uuid>) -> Result<Json<TodoEntry>, ApiError> {
            Ok(Json(get(&state.pool, query_from_path(id)).await?))
        }
    }

    pub(super) fn query_from_path(id: Path<Uuid>) -> GetTodoQuery {
        GetTodoQuery::lookup(id.0)
    }
}

mod implementation {
    use super::intention::GetTodoQuery;
    use crate::assembly::io::{AppError, TodoEntry, fetch_todo};
    use sqlx::PgPool;

    pub async fn get(pool: &kernel::io::DbPool, query: GetTodoQuery) -> Result<TodoEntry, AppError> {
        fetch_todo(pool, query.requested_todo()).await
    }
}

#[cfg(test)]
mod tests {
    use super::intention::GetTodoQuery;

    #[test]
    fn path_helper_extracts_uuid() {
        let id = uuid::Uuid::now_v7();
        assert_eq!(super::ui::query_from_path(poem_openapi::param::Path(id)).todo_id, id);
    }

    #[test]
    fn lookup_query_tracks_requested_todo() {
        let id = uuid::Uuid::now_v7();
        assert_eq!(GetTodoQuery::lookup(id).requested_todo(), id);
    }
}
