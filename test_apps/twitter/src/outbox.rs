pub mod io {
    pub use super::ui::{Api, OutboxMessageDto};
}

mod ui {
    use crate::{
        AppState,
        assembly::io::{ApiError, run_blocking},
    };
    use chrono::{DateTime, Utc};
    use poem::web::Data;
    use poem_openapi::{Object, OpenApi, payload::Json};
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

    pub struct Api;

    #[OpenApi]
    impl Api {
        #[oai(path = "/messages/outbox", method = "get")]
        async fn list_outbox(&self, state: Data<&AppState>) -> Result<Json<OutboxList>, ApiError> {
            let pool = state.pool.clone();
            let result = run_blocking(move || super::implementation::list_outbox(&pool)).await?;
            Ok(Json(result))
        }
    }
}

mod implementation {
    use super::ui::{OutboxList, OutboxMessageDto};
    use crate::assembly::io::{AppError, DbPool};
    use chrono::{DateTime, Utc};
    use diesel::prelude::*;
    use serde_json::Value;
    use uuid::Uuid;

    pub(super) fn list_outbox(pool: &DbPool) -> Result<OutboxList, AppError> {
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

        let mut conn = pool
            .get()
            .map_err(|error| AppError::Storage(error.into()))?;
        let rows: Vec<Row> = diesel::sql_query(
            "SELECT id, event_type, payload::text, status, created_at, published_at, attempts \
             FROM outbox_messages ORDER BY created_at ASC",
        )
        .load(&mut conn)
        .map_err(|error| AppError::Storage(error.into()))?;

        let items = rows
            .into_iter()
            .map(|row| {
                let payload: Value = serde_json::from_str(&row.payload)
                    .map_err(|error| AppError::Storage(error.into()))?;
                Ok(OutboxMessageDto {
                    id: row.id,
                    event_type: row.event_type,
                    payload,
                    status: row.status,
                    created_at: row.created_at,
                    published_at: row.published_at,
                    attempts: row.attempts,
                })
            })
            .collect::<Result<Vec<_>, AppError>>()?;

        Ok(OutboxList { items })
    }
}

#[cfg(test)]
mod tests {
    use super::ui::{OutboxList, OutboxMessageDto};

    #[test]
    fn outbox_dto_round_trips_through_json() {
        let dto = OutboxMessageDto {
            id: uuid::Uuid::now_v7(),
            event_type: "TweetPosted".to_string(),
            payload: serde_json::json!({"tweet_id": "abc"}),
            status: "pending".to_string(),
            created_at: chrono::Utc::now(),
            published_at: None,
            attempts: 0,
        };
        let list = OutboxList {
            items: vec![dto.clone()],
        };
        let json = serde_json::to_string(&list).expect("serialize");
        let back: OutboxList = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.items.len(), 1);
        assert_eq!(back.items[0].event_type, dto.event_type);
        assert_eq!(back.items[0].status, dto.status);
    }
}
