use super::application::AppError;
use super::domain::{Clock, TodoDto};
use diesel_migrations::{EmbeddedMigrations, MigrationHarness, embed_migrations};
use kernel::{EventError, EventSubscriberPort, NewEventEnvelope};
use sqlx::{PgPool, postgres::PgPoolOptions};
use uuid::Uuid;

pub use kernel::io::{DbPool, build_pool};

pub mod entity {
    use chrono::{DateTime, Utc};
    use sqlx::FromRow;
    use uuid::Uuid;

    #[derive(Debug, Clone, FromRow, diesel::Queryable)]
    pub struct TodoRow {
        pub id: Uuid,
        pub title: String,
        pub description: Option<String>,
        pub status: String,
        pub created_at: DateTime<Utc>,
        pub updated_at: DateTime<Utc>,
        pub due_at: Option<DateTime<Utc>>,
    }
}

pub async fn connect(database_url: &str) -> anyhow::Result<PgPool> {
    Ok(PgPoolOptions::new()
        .max_connections(10)
        .connect(database_url)
        .await?)
}

pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations");

pub fn run_migrations(database_url: &str) -> anyhow::Result<()> {
    let pool = build_pool(database_url).map_err(|e| anyhow::anyhow!("pool error: {e}"))?;
    let mut conn = pool.get()?;
    conn.run_pending_migrations(MIGRATIONS)
        .map_err(|e| anyhow::anyhow!("migration error: {e}"))?;
    Ok(())
}

pub async fn fetch_todo(pool: &PgPool, id: Uuid) -> Result<TodoDto, AppError> {
    let sql = "SELECT id, title, description, status, created_at, updated_at, due_at FROM todos WHERE id = $1";

    let row = sqlx::query_as::<_, entity::TodoRow>(sql)
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|e| AppError::Storage(e.into()))?
        .ok_or(AppError::NotFound)?;

    row.try_into()
}

pub fn record_event_payload(
    pool: &DbPool,
    event_type: &str,
    payload: serde_json::Value,
) -> anyhow::Result<Uuid> {
    use crate::schema::outbox_messages;
    use diesel::prelude::*;

    let id = Uuid::now_v7();
    let mut conn = pool.get()?;
    diesel::insert_into(outbox_messages::table)
        .values((
            outbox_messages::id.eq(id),
            outbox_messages::event_type.eq(event_type),
            outbox_messages::payload.eq(payload),
            outbox_messages::status.eq("pending"),
            outbox_messages::created_at.eq(Clock::now()),
        ))
        .execute(&mut conn)?;
    Ok(id)
}

pub struct OutboxSubscriber {
    pool: DbPool,
}

impl OutboxSubscriber {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }
}

impl EventSubscriberPort for OutboxSubscriber {
    fn handle(&self, envelope: &NewEventEnvelope) -> Result<(), EventError> {
        let payload = serde_json::from_str(&envelope.payload)
            .map_err(|e| EventError::SubscriberExecution(e.to_string()))?;
        record_event_payload(&self.pool, &envelope.event_type, payload)
            .map(|_| ())
            .map_err(|e| EventError::SubscriberExecution(e.to_string()))
    }
}
