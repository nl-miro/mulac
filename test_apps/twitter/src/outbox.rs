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

    #[derive(Debug, Clone, Serialize, Deserialize, Object)]
    pub struct OutboxList {
        pub items: Vec<OutboxMessageDto>,
    }
}

mod infra_diesel {
    use super::models::{OutboxList, OutboxMessageDto};
    use crate::assembly::io::{AppError, DbPool};
    use chrono::{DateTime, Utc};
    use diesel::prelude::*;
    use serde_json::Value;
    use uuid::Uuid;

    pub fn list_outbox(pool: &DbPool) -> Result<OutboxList, AppError> {
        #[derive(diesel::QueryableByName)]
        struct Row {
            #[diesel(sql_type = diesel::sql_types::Uuid)]
            id: Uuid,
            #[diesel(sql_type = diesel::sql_types::Text)]
            event_type: String,
            #[diesel(sql_type = diesel::sql_types::Text)]
            payload: String,
            #[diesel(sql_type = diesel::sql_types::Text)]
            status: String,
            #[diesel(sql_type = diesel::sql_types::Timestamptz)]
            created_at: DateTime<Utc>,
            #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Timestamptz>)]
            published_at: Option<DateTime<Utc>>,
            #[diesel(sql_type = diesel::sql_types::Integer)]
            attempts: i32,
        }

        let mut conn = pool.get().map_err(|e| AppError::Storage(e.into()))?;
        let rows: Vec<Row> = diesel::sql_query(
            "SELECT id, event_type, payload::text, status, created_at, published_at, attempts \
             FROM outbox_messages ORDER BY created_at ASC",
        )
        .load(&mut conn)
        .map_err(|e| AppError::Storage(e.into()))?;

        let items = rows
            .into_iter()
            .map(|r| {
                let payload: Value =
                    serde_json::from_str(&r.payload).map_err(|e| AppError::Storage(e.into()))?;
                Ok(OutboxMessageDto {
                    id: r.id,
                    event_type: r.event_type,
                    payload,
                    status: r.status,
                    created_at: r.created_at,
                    published_at: r.published_at,
                    attempts: r.attempts,
                })
            })
            .collect::<Result<Vec<_>, AppError>>()?;

        Ok(OutboxList { items })
    }
}

mod http {
    use super::infra_diesel::list_outbox;
    use super::models::OutboxList;
    use crate::{
        AppState,
        assembly::io::{ApiError, run_blocking},
    };
    use poem::web::Data;
    use poem_openapi::{OpenApi, payload::Json};

    pub struct Api;

    #[OpenApi]
    impl Api {
        #[oai(path = "/messages/outbox", method = "get")]
        async fn list_outbox(&self, state: Data<&AppState>) -> Result<Json<OutboxList>, ApiError> {
            let pool = state.pool.clone();
            let result = run_blocking(move || list_outbox(&pool)).await?;
            Ok(Json(result))
        }
    }
}

pub mod io {
    pub use super::http::Api;
    pub use super::models::OutboxMessageDto;
}
