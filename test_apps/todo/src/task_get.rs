mod infra_sqlx_pg {
    use crate::assembly::io::{AppError, TodoDto, fetch_todo};
    use sqlx::PgPool;
    use uuid::Uuid;

    pub async fn get(pool: &PgPool, id: Uuid) -> Result<TodoDto, AppError> {
        fetch_todo(pool, id).await
    }
}

mod http {
    use crate::{
        AppState,
        assembly::io::{ApiError, TodoDto},
        //
    };
    use poem::web::Data;
    use poem_openapi::{OpenApi, param::Path, payload::Json};
    use uuid::Uuid;

    pub struct Api;

    #[OpenApi]
    impl Api {
        #[oai(path = "/todos/:id", method = "get")]
        async fn get_todo(
            &self,
            state: Data<&AppState>,
            id: Path<Uuid>,
        ) -> Result<Json<TodoDto>, ApiError> {
            Ok(Json(super::infra_sqlx_pg::get(&state.pool, id.0).await?))
        }
    }
}

pub mod io {
    pub use super::http::Api;
}
