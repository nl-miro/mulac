use self::entity::TodoRow;
use super::application::{AppError, block_on_blocking};
use super::domain::{Clock, TodoEntry};
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
        pub due_at: Option<DateTime<Utc>>,
        pub created_at: DateTime<Utc>,
        pub updated_at: DateTime<Utc>,
    }
}

pub async fn connect(database_url: &str) -> anyhow::Result<kernel::io::DbPool> {
    Ok(PgPoolOptions::new().max_connections(10).connect(database_url).await?)
}

pub async fn migrate(pool: &kernel::io::DbPool) -> anyhow::Result<()> {
    sqlx::migrate!("./migrations").run(pool).await?;
    Ok(())
}

pub async fn fetch_todo(pool: &kernel::io::DbPool, id: Uuid) -> Result<TodoEntry, AppError> {
    let sql = "SELECT id, title, description, status, created_at, updated_at, due_at FROM todos WHERE id = $1";

    let row = sqlx::query_as::<_, entity::TodoRow>(sql)
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|e| AppError::Storage(e.into()))?
        .ok_or(AppError::NotFound)?;

    row.try_into()
}

pub async fn insert_todo(pool: &kernel::io::DbPool, row: TodoEntry) -> Result<TodoEntry, AppError> {
    let mut tx = pool.begin().await.map_err(|e| AppError::Storage(e.into()))?;

    let sql = "INSERT INTO todos (id, title, description, status, created_at, updated_at, due_at) VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING id, title, description, status, created_at, updated_at, due_at";

    let row = sqlx::query_as::<_, TodoRow>(sql)
        .bind(row.id)
        .bind(row.title.trim())
        .bind(row.description)
        .bind(row.status)
        .bind(row.created_at)
        .bind(row.updated_at)
        .bind(row.due_at)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| AppError::Storage(e.into()))?;

    let todo: TodoEntry = row.try_into()?;
    tx.commit().await.map_err(|e| AppError::Storage(e.into()))?;
    Ok(todo)
}

pub async fn record_event_payload(pool: &kernel::io::DbPool, event_type: &str, payload: serde_json::Value) -> anyhow::Result<Uuid> {
    let id = Uuid::now_v7();

    let sql = "INSERT INTO outbox_messages (id, event_type, payload, status, created_at) VALUES ($1, $2, $3, 'pending', $4)";

    sqlx::query(sql).bind(id).bind(event_type).bind(payload).bind(Clock::now()).execute(pool).await?;
    Ok(id)
}

pub struct OutboxSubscriber {
    pool: kernel::io::DbPool,
}

impl OutboxSubscriber {
    pub fn new(pool: kernel::io::DbPool) -> Self {
        Self { pool }
    }
}

impl EventSubscriberPort for OutboxSubscriber {
    fn handle(&self, envelope: &NewEventEnvelope) -> Result<(), EventError> {
        let payload = serde_json::from_str(&envelope.payload).map_err(|e| EventError::SubscriberExecution(e.to_string()))?;
        let pool = self.pool.clone();
        let event_type = envelope.event_type.clone();
        block_on_blocking(async move { record_event_payload(&pool, &event_type, payload).await })
            .map(|_| ())
            .map_err(|e| EventError::SubscriberExecution(e.to_string()))
    }
}
