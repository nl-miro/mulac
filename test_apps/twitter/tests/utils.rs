use std::sync::{Arc, Mutex, OnceLock};

use commanding::io::{NewCommand, NewCommandEnvelope, NewCommandMetadata, ReservableCommandSpec};
use diesel::prelude::*;
use diesel::r2d2::{ConnectionManager, Pool};
use poem::{EndpointExt, Route, get, handler, middleware::AddData};
use poem_openapi::OpenApiService;
use reqwest::Client;
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

pub use test_app_twitter::db::{build_pool, run_migrations};
pub use test_app_twitter::io::{
    AppState, DEFAULT_DATABASE_URL, DirectMessageSendApi, FollowUserApi, InboxApi, OutboxApi,
    TweetDeleteApi, TweetLikeApi, TweetPostApi, TweetRetweetApi, TweetUnlikeApi, UnfollowUserApi,
    run_command_worker, run_event_worker, start_mulac,
};

pub type DbPool = Pool<ConnectionManager<diesel::PgConnection>>;

pub const STATUS_COMPLETED: i32 = 5;

// ── Row structs (using QueryableByName for raw SQL) ───────────────────────────

#[derive(Debug, diesel::QueryableByName)]
pub struct TweetRow {
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub id: Uuid,
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub author_id: Uuid,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub content: String,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Uuid>)]
    pub retweeted_from: Option<Uuid>,
    #[diesel(sql_type = diesel::sql_types::Timestamptz)]
    pub created_at: chrono::DateTime<chrono::Utc>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Timestamptz>)]
    pub deleted_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, diesel::QueryableByName)]
pub struct FollowRow {
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub follower_id: Uuid,
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub following_id: Uuid,
    #[diesel(sql_type = diesel::sql_types::Timestamptz)]
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, diesel::QueryableByName)]
pub struct LikeRow {
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub user_id: Uuid,
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub tweet_id: Uuid,
    #[diesel(sql_type = diesel::sql_types::Timestamptz)]
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, diesel::QueryableByName)]
pub struct DirectMessageRow {
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub id: Uuid,
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub sender_id: Uuid,
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub recipient_id: Uuid,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub content: String,
    #[diesel(sql_type = diesel::sql_types::Timestamptz)]
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, diesel::QueryableByName)]
pub struct TimelineRow {
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub id: Uuid,
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub user_id: Uuid,
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub tweet_id: Uuid,
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub author_id: Uuid,
    #[diesel(sql_type = diesel::sql_types::Timestamptz)]
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, diesel::QueryableByName)]
pub struct OutboxRow {
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub id: Uuid,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub event_type: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub payload: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub status: String,
    #[diesel(sql_type = diesel::sql_types::Timestamptz)]
    pub created_at: chrono::DateTime<chrono::Utc>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Timestamptz>)]
    pub published_at: Option<chrono::DateTime<chrono::Utc>>,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub attempts: i32,
}

#[derive(Debug, diesel::QueryableByName)]
pub struct InboxRow {
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub id: Uuid,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub message_type: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub payload: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub status: String,
    #[diesel(sql_type = diesel::sql_types::Timestamptz)]
    pub received_at: chrono::DateTime<chrono::Utc>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Timestamptz>)]
    pub processed_at: Option<chrono::DateTime<chrono::Utc>>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    pub error: Option<String>,
}

#[derive(Debug, diesel::QueryableByName)]
pub struct CommandEntryRow {
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub id: Uuid,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub command_type: String,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub status: i32,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub attempts: i32,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Timestamptz>)]
    pub processed_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, diesel::QueryableByName)]
pub struct EventEntryRow {
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub id: Uuid,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub event_type: String,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub status: i32,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub attempts: i32,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Timestamptz>)]
    pub processed_at: Option<chrono::DateTime<chrono::Utc>>,
}

// ── DB helpers ────────────────────────────────────────────────────────────────

pub fn fetch_tweets(pool: &DbPool) -> Vec<TweetRow> {
    let mut conn = pool.get().unwrap();
    diesel::sql_query(
        "SELECT id, author_id, content, retweeted_from, created_at, deleted_at FROM tweets",
    )
    .load(&mut conn)
    .unwrap()
}

pub fn fetch_tweet_by_id(pool: &DbPool, id: Uuid) -> Option<TweetRow> {
    let mut conn = pool.get().unwrap();
    diesel::sql_query(
        "SELECT id, author_id, content, retweeted_from, created_at, deleted_at \
         FROM tweets WHERE id = $1",
    )
    .bind::<diesel::sql_types::Uuid, _>(id)
    .load::<TweetRow>(&mut conn)
    .unwrap()
    .into_iter()
    .next()
}

pub fn fetch_follows(pool: &DbPool) -> Vec<FollowRow> {
    let mut conn = pool.get().unwrap();
    diesel::sql_query("SELECT follower_id, following_id, created_at FROM follows")
        .load(&mut conn)
        .unwrap()
}

pub fn fetch_likes(pool: &DbPool) -> Vec<LikeRow> {
    let mut conn = pool.get().unwrap();
    diesel::sql_query("SELECT user_id, tweet_id, created_at FROM likes")
        .load(&mut conn)
        .unwrap()
}

pub fn fetch_direct_messages(pool: &DbPool) -> Vec<DirectMessageRow> {
    let mut conn = pool.get().unwrap();
    diesel::sql_query(
        "SELECT id, sender_id, recipient_id, content, created_at FROM direct_messages",
    )
    .load(&mut conn)
    .unwrap()
}

pub fn fetch_timelines(pool: &DbPool) -> Vec<TimelineRow> {
    let mut conn = pool.get().unwrap();
    diesel::sql_query("SELECT id, user_id, tweet_id, author_id, created_at FROM timelines")
        .load(&mut conn)
        .unwrap()
}

pub fn fetch_timelines_for_user(pool: &DbPool, user_id: Uuid) -> Vec<TimelineRow> {
    let mut conn = pool.get().unwrap();
    diesel::sql_query(
        "SELECT id, user_id, tweet_id, author_id, created_at \
         FROM timelines WHERE user_id = $1",
    )
    .bind::<diesel::sql_types::Uuid, _>(user_id)
    .load(&mut conn)
    .unwrap()
}

pub fn fetch_outbox(pool: &DbPool) -> Vec<OutboxRow> {
    let mut conn = pool.get().unwrap();
    diesel::sql_query(
        "SELECT id, event_type, payload::text, status, created_at, published_at, attempts \
         FROM outbox_messages ORDER BY created_at ASC",
    )
    .load(&mut conn)
    .unwrap()
}

pub fn fetch_inbox(pool: &DbPool) -> Vec<InboxRow> {
    let mut conn = pool.get().unwrap();
    diesel::sql_query(
        "SELECT id, message_type, payload::text, status, received_at, processed_at, error \
         FROM inbox_messages ORDER BY received_at ASC",
    )
    .load(&mut conn)
    .unwrap()
}

pub fn fetch_command_entries(pool: &DbPool) -> Vec<CommandEntryRow> {
    let mut conn = pool.get().unwrap();
    diesel::sql_query(
        "SELECT id, command_type, status, attempts, processed_at \
         FROM command_entries ORDER BY received_at ASC",
    )
    .load(&mut conn)
    .unwrap()
}

pub fn fetch_event_entries(pool: &DbPool) -> Vec<EventEntryRow> {
    let mut conn = pool.get().unwrap();
    diesel::sql_query(
        "SELECT id, event_type, status, attempts, processed_at \
         FROM event_entries ORDER BY received_at ASC",
    )
    .load(&mut conn)
    .unwrap()
}

pub fn retry_fan_out(pool: &DbPool, tweet_id: Uuid, author_id: Uuid) {
    let kernel = start_mulac(pool.clone()).unwrap();
    let gateway = kernel.state().command_gateway();

    gateway
        .dispatch(NewCommandEnvelope {
            command: NewCommand {
                command_type: "FanOutTweet".to_string(),
                payload: serde_json::json!({
                    "tweet_id": tweet_id,
                    "author_id": author_id,
                })
                .to_string(),
            },
            metadata: Some(NewCommandMetadata {
                command_id: Uuid::now_v7(),
                correlation_id: None,
                causation_id: None,
                source: Some("test_app_twitter.tests".to_string()),
            }),
        })
        .unwrap();

    kernel
        .command_consumer()
        .consume(&ReservableCommandSpec::new(10))
        .unwrap();
}

// ── Test server ───────────────────────────────────────────────────────────────

// One pool shared across all tests in this binary — prevents connection accumulation.
fn shared_pool(database_url: &str) -> DbPool {
    static POOL: OnceLock<DbPool> = OnceLock::new();
    POOL.get_or_init(|| build_pool(database_url).expect("failed to build test pool"))
        .clone()
}

// Previous kernel's cancellation token — cancelled before each new test starts.
fn prev_worker_token() -> &'static Mutex<Option<CancellationToken>> {
    static TOKEN: OnceLock<Mutex<Option<CancellationToken>>> = OnceLock::new();
    TOKEN.get_or_init(|| Mutex::new(None))
}

fn test_lock() -> Arc<AsyncMutex<()>> {
    static LOCK: OnceLock<Arc<AsyncMutex<()>>> = OnceLock::new();
    LOCK.get_or_init(|| Arc::new(AsyncMutex::new(()))).clone()
}

#[handler]
fn health_handler() -> &'static str {
    "ok"
}

fn reset_tables(pool: &DbPool) {
    let mut conn = pool.get().unwrap();
    diesel::sql_query(
        "TRUNCATE TABLE \
            outbox_messages, inbox_messages, \
            event_entries, command_entries, \
            timelines, likes, direct_messages, follows, tweets \
         RESTART IDENTITY CASCADE",
    )
    .execute(&mut conn)
    .unwrap();
}

pub async fn start_test_app() -> (String, DbPool, OwnedMutexGuard<()>) {
    let guard = test_lock().lock_owned().await;
    dotenvy::dotenv().ok();

    let database_url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| DEFAULT_DATABASE_URL.to_string());

    // Cancel previous test's background workers so they stop competing for
    // command/event reservations and connections.
    {
        let mut lock = prev_worker_token().lock().unwrap();
        if let Some(token) = lock.take() {
            token.cancel();
        }
    }
    // Give cancelled workers a tick to exit their current consume cycle.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let pool = shared_pool(&database_url);
    run_migrations(&pool).unwrap();
    reset_tables(&pool);

    let kernel = start_mulac(pool.clone()).unwrap();
    let token = kernel.child_token();
    tokio::spawn(run_command_worker(kernel.command_consumer(), token.clone()));
    tokio::spawn(run_event_worker(kernel.event_consumer(), token.clone()));

    // Store this test's token so the next test can cancel it.
    *prev_worker_token().lock().unwrap() = Some(token);

    let state = AppState::new(pool.clone(), kernel.state());

    let api = OpenApiService::new(
        (
            TweetPostApi,
            TweetDeleteApi,
            TweetRetweetApi,
            FollowUserApi,
            UnfollowUserApi,
            TweetLikeApi,
            TweetUnlikeApi,
            DirectMessageSendApi,
            InboxApi,
            OutboxApi,
        ),
        "test_app_twitter",
        "0.1.0",
    );

    let app = Route::new()
        .at("/health", get(health_handler))
        .nest("/api", api)
        .with(AddData::new(state));

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let base_url = format!("http://127.0.0.1:{port}");

    tokio::spawn(
        poem::Server::new(poem::listener::TcpListener::bind(format!(
            "127.0.0.1:{port}"
        )))
        .run(app),
    );

    // Poll health with a per-attempt timeout to avoid a silent hang on startup.
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap();
    for _ in 0..30 {
        if client
            .get(format!("{base_url}/health"))
            .send()
            .await
            .is_ok()
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    (base_url, pool, guard)
}

/// Returns a reqwest client with a per-request timeout. Use this in tests
/// instead of `Client::new()` so that a stuck server causes a clean failure
/// rather than an indefinite hang.
pub fn client() -> Client {
    Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap()
}
