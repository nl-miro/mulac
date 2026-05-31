use super::application::{AppError, block_on_blocking};
use super::domain::{Clock, TodoDto};
use diesel_migrations::{EmbeddedMigrations, MigrationHarness, embed_migrations};
use kernel::{EventError, EventSubscriberPort, NewEventEnvelope};
use sqlx::{PgPool, postgres::PgPoolOptions};
use uuid::Uuid;

pub mod entity {
    use chrono::{DateTime, Utc};
    use sqlx::FromRow;
    use uuid::Uuid;

    #[derive(Debug, Clone, FromRow)]
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
    let pool = kernel::io::build_pool(database_url)
        .map_err(|e| anyhow::anyhow!("pool error: {e}"))?;
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

pub async fn record_event_payload(
    pool: &PgPool,
    event_type: &str,
    payload: serde_json::Value,
) -> anyhow::Result<Uuid> {
    let id = Uuid::now_v7();

    let sql = "INSERT INTO outbox_messages (id, event_type, payload, status, created_at) VALUES ($1, $2, $3, 'pending', $4)";

    sqlx::query(sql)
        .bind(id)
        .bind(event_type)
        .bind(payload)
        .bind(Clock::now())
        .execute(pool)
        .await?;
    Ok(id)
}

pub struct OutboxSubscriber {
    pool: PgPool,
}

impl OutboxSubscriber {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl EventSubscriberPort for OutboxSubscriber {
    fn handle(&self, envelope: &NewEventEnvelope) -> Result<(), EventError> {
        let payload = serde_json::from_str(&envelope.payload)
            .map_err(|e| EventError::SubscriberExecution(e.to_string()))?;
        let pool = self.pool.clone();
        let event_type = envelope.event_type.clone();
        block_on_blocking(async move { record_event_payload(&pool, &event_type, payload).await })
            .map(|_| ())
            .map_err(|e| EventError::SubscriberExecution(e.to_string()))
    }
}
