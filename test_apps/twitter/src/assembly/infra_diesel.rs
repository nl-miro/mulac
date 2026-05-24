use super::domain::{Clock, DirectMessageDto, FollowDto, LikeDto, TweetDto};
use chrono::{DateTime, Utc};
use diesel::PgConnection;
use diesel::r2d2::{ConnectionManager, Pool};
use diesel_migrations::{
    EmbeddedMigrations,
    MigrationHarness,
    embed_migrations, //
};
use kernel::{EventError, EventSubscriberPort, NewEventEnvelope};
use uuid::Uuid;

pub type DbPool = Pool<ConnectionManager<PgConnection>>;
pub const DEFAULT_DATABASE_URL: &str = "postgres://twitter:twitter@127.0.0.1:5433/twitter";

pub fn build_pool(database_url: &str) -> anyhow::Result<DbPool> {
    let manager = ConnectionManager::<PgConnection>::new(database_url);
    Pool::builder()
        .build(manager)
        .map_err(|e| anyhow::anyhow!("pool error: {e}"))
}

pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations");

pub fn run_migrations(pool: &DbPool) -> anyhow::Result<()> {
    let mut conn = pool.get()?;
    conn.run_pending_migrations(MIGRATIONS)
        .map_err(|e| anyhow::anyhow!("migration error: {e}"))?;
    Ok(())
}

// ── Shared read-after-write fetch helpers ─────────────────────────────────────

pub fn fetch_tweet(pool: &DbPool, tweet_id: Uuid) -> anyhow::Result<TweetDto> {
    use crate::schema::tweets;
    use diesel::prelude::*;

    #[derive(diesel::Queryable)]
    struct Row {
        id: Uuid,
        author_id: Uuid,
        content: String,
        retweeted_from: Option<Uuid>,
        created_at: DateTime<Utc>,
        deleted_at: Option<DateTime<Utc>>,
    }

    let mut conn = pool.get()?;
    let row = tweets::table.find(tweet_id).first::<Row>(&mut conn)?;

    Ok(TweetDto {
        id: row.id,
        author_id: row.author_id,
        content: row.content,
        created_at: row.created_at,
        retweeted_from: row.retweeted_from,
        deleted_at: row.deleted_at,
    })
}

pub fn fetch_follow(
    pool: &DbPool,
    follower_id: Uuid,
    following_id: Uuid,
) -> anyhow::Result<FollowDto> {
    use crate::schema::follows;
    use diesel::prelude::*;

    #[derive(diesel::Queryable)]
    struct Row {
        follower_id: Uuid,
        following_id: Uuid,
        created_at: DateTime<Utc>,
    }

    let mut conn = pool.get()?;
    let row = follows::table
        .find((follower_id, following_id))
        .first::<Row>(&mut conn)?;

    Ok(FollowDto {
        follower_id: row.follower_id,
        following_id: row.following_id,
        created_at: row.created_at,
    })
}

pub fn fetch_like(pool: &DbPool, user_id: Uuid, tweet_id: Uuid) -> anyhow::Result<LikeDto> {
    use crate::schema::likes;
    use diesel::prelude::*;

    #[derive(diesel::Queryable)]
    struct Row {
        user_id: Uuid,
        tweet_id: Uuid,
        created_at: DateTime<Utc>,
    }

    let mut conn = pool.get()?;
    let row = likes::table
        .find((user_id, tweet_id))
        .first::<Row>(&mut conn)?;

    Ok(LikeDto {
        user_id: row.user_id,
        tweet_id: row.tweet_id,
        created_at: row.created_at,
    })
}

pub fn fetch_direct_message(pool: &DbPool, message_id: Uuid) -> anyhow::Result<DirectMessageDto> {
    use crate::schema::direct_messages;
    use diesel::prelude::*;

    #[derive(diesel::Queryable)]
    struct Row {
        id: Uuid,
        sender_id: Uuid,
        recipient_id: Uuid,
        content: String,
        created_at: DateTime<Utc>,
    }

    let mut conn = pool.get()?;
    let row = direct_messages::table
        .find(message_id)
        .first::<Row>(&mut conn)?;

    Ok(DirectMessageDto {
        id: row.id,
        sender_id: row.sender_id,
        recipient_id: row.recipient_id,
        content: row.content,
        created_at: row.created_at,
    })
}

// ── App-level outbox subscriber ───────────────────────────────────────────────

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
        let event_id = envelope
            .metadata
            .as_ref()
            .map(|m| m.event_id)
            .ok_or_else(|| {
                EventError::SubscriberExecution(
                    "event_id is required for outbox idempotency".to_string(),
                )
            })?;

        let payload: serde_json::Value = serde_json::from_str(&envelope.payload)
            .map_err(|e| EventError::SubscriberExecution(e.to_string()))?;

        record_event_payload(&self.pool, event_id, &envelope.event_type, payload)
            .map_err(|e| EventError::SubscriberExecution(e.to_string()))
    }
}

pub fn record_event_payload(
    pool: &DbPool,
    event_id: Uuid,
    event_type: &str,
    payload: serde_json::Value,
) -> anyhow::Result<()> {
    use diesel::prelude::*;

    let mut conn = pool.get()?;
    diesel::sql_query(
        "INSERT INTO outbox_messages (id, event_type, payload, status, created_at) \
         VALUES ($1, $2, $3::jsonb, 'pending', $4) \
         ON CONFLICT (id) DO NOTHING",
    )
    .bind::<diesel::sql_types::Uuid, _>(event_id)
    .bind::<diesel::sql_types::Text, _>(event_type)
    .bind::<diesel::sql_types::Text, _>(payload.to_string())
    .bind::<diesel::sql_types::Timestamptz, _>(Clock::now())
    .execute(&mut conn)?;

    Ok(())
}
