use std::sync::{Arc, OnceLock};

use poem::{EndpointExt, Route, get, handler, middleware::AddData};
use poem_openapi::OpenApiService;
use reqwest::Client;
use sqlx::PgPool;
use test_app_todo::io::{
    AppState, CompleteApi, CreateApi, DeleteApi, DueDatesApi, GetApi, InboxApi, ListApi, OutboxApi, ReopenApi, UpdateApi, connect, migrate,
    start_mulac,
};
use tokio::sync::{Mutex, OwnedMutexGuard};
use uuid::Uuid;

pub const STATUS_COMPLETED: i32 = 5;

#[derive(sqlx::FromRow)]
pub struct OutboxRow {
    pub id: Uuid,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub status: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub published_at: Option<chrono::DateTime<chrono::Utc>>,
    pub attempts: i32,
}

#[derive(sqlx::FromRow)]
pub struct InboxRow {
    pub id: Uuid,
    pub message_type: String,
    pub payload: serde_json::Value,
    pub status: String,
    pub received_at: chrono::DateTime<chrono::Utc>,
    pub processed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub error: Option<String>,
}

#[derive(sqlx::FromRow)]
pub struct CommandEntryRow {
    pub id: Uuid,
    pub command_type: String,
    pub status: i32,
    pub payload: String,
    pub meta: Option<serde_json::Value>,
    pub extra_info: Option<serde_json::Value>,
    pub attempts: i32,
    pub reservation_id: Option<Uuid>,
    pub processed_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(sqlx::FromRow)]
pub struct EventEntryRow {
    pub id: Uuid,
    pub event_type: String,
    pub status: i32,
    pub payload: String,
    pub meta: Option<serde_json::Value>,
    pub extra_info: Option<serde_json::Value>,
    pub attempts: i32,
    pub reservation_id: Option<Uuid>,
    pub processed_at: Option<chrono::DateTime<chrono::Utc>>,
}

pub async fn fetch_outbox(pool: &kernel::io::DbPool) -> Vec<OutboxRow> {
    sqlx::query_as::<_, OutboxRow>("SELECT id, event_type, payload, status, created_at, published_at, attempts FROM outbox_messages")
        .fetch_all(pool)
        .await
        .unwrap()
}

pub async fn fetch_command_entries(pool: &kernel::io::DbPool) -> Vec<CommandEntryRow> {
    sqlx::query_as::<_, CommandEntryRow>(
        "SELECT id, command_type, status, payload, meta, extra_info, attempts, reservation_id, processed_at FROM command_entries ORDER BY received_at ASC",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

pub async fn fetch_event_entries(pool: &kernel::io::DbPool) -> Vec<EventEntryRow> {
    sqlx::query_as::<_, EventEntryRow>(
        "SELECT id, event_type, status, payload, meta, extra_info, attempts, reservation_id, processed_at FROM event_entries ORDER BY received_at ASC",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

pub async fn fetch_inbox(pool: &kernel::io::DbPool) -> Vec<InboxRow> {
    sqlx::query_as::<_, InboxRow>("SELECT id, message_type, payload, status, received_at, processed_at, error FROM inbox_messages")
        .fetch_all(pool)
        .await
        .unwrap()
}

#[handler]
fn health() -> &'static str {
    "ok"
}

fn test_lock() -> Arc<Mutex<()>> {
    static LOCK: OnceLock<Arc<Mutex<()>>> = OnceLock::new();
    LOCK.get_or_init(|| Arc::new(Mutex::new(()))).clone()
}

pub async fn start_test_app() -> (String, kernel::io::DbPool, OwnedMutexGuard<()>) {
    let guard = test_lock().lock_owned().await;
    dotenvy::dotenv().ok();

    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");

    let pool = connect(&database_url).await.unwrap();
    migrate(&pool).await.unwrap();

    for table in &["event_entries", "command_entries", "outbox_messages", "inbox_messages", "todos"] {
        sqlx::query(&format!("DELETE FROM {table}")).execute(&pool).await.unwrap();
    }

    let kernel = start_mulac(pool.clone(), &database_url).await.unwrap();
    let state = AppState::new(pool.clone(), kernel.state());

    let api = OpenApiService::new(
        (CreateApi, ListApi, GetApi, UpdateApi, CompleteApi, ReopenApi, DeleteApi, DueDatesApi, InboxApi, OutboxApi),
        "test_app_todo",
        "0.1.0",
    );

    let app = Route::new().at("/health", get(health)).nest("/api", api).with(AddData::new(state));

    let std_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = std_listener.local_addr().unwrap().port();
    drop(std_listener);

    let base_url = format!("http://127.0.0.1:{port}");
    tokio::spawn(poem::Server::new(poem::listener::TcpListener::bind(format!("127.0.0.1:{port}"))).run(app));

    let client = Client::new();
    for _ in 0..20 {
        if client.get(format!("{base_url}/health")).send().await.is_ok() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    (base_url, pool, guard)
}

pub fn client() -> reqwest::Client {
    reqwest::Client::builder().timeout(std::time::Duration::from_secs(10)).build().unwrap()
}

pub async fn assert_outbox_pending(pool: &kernel::io::DbPool, event_type: &str) {
    let outbox = fetch_outbox(pool).await;
    let matching: Vec<_> = outbox.iter().filter(|r| r.event_type == event_type).collect();
    assert_eq!(matching.len(), 1);
    assert_eq!(matching[0].status, "pending");
}

pub async fn assert_command_completed(pool: &kernel::io::DbPool, command_type: &str) {
    let cmds = fetch_command_entries(pool).await;
    let matching: Vec<_> = cmds.iter().filter(|c| c.command_type == command_type).collect();
    assert_eq!(matching.len(), 1);
    assert_eq!(matching[0].status, STATUS_COMPLETED);
}

pub async fn assert_event_completed(pool: &kernel::io::DbPool, event_type: &str) {
    let events = fetch_event_entries(pool).await;
    let matching: Vec<_> = events.iter().filter(|e| e.event_type == event_type).collect();
    assert_eq!(matching.len(), 1);
    assert_eq!(matching[0].status, STATUS_COMPLETED);
}

macro_rules! assert_ok_response {
    ($resp:expr) => {
        assert_eq!($resp.status(), 200)
    };
}
pub(crate) use assert_ok_response;

macro_rules! assert_bad_request_response {
    ($resp:expr) => {
        assert_eq!($resp.status(), 400)
    };
}
pub(crate) use assert_bad_request_response;

macro_rules! assert_not_found_response {
    ($resp:expr) => {
        assert_eq!($resp.status(), 404)
    };
}
pub(crate) use assert_not_found_response;

macro_rules! assert_no_content_response {
    ($resp:expr) => {
        assert_eq!($resp.status(), 204)
    };
}
pub(crate) use assert_no_content_response;

macro_rules! assert_conflict_response {
    ($resp:expr) => {
        assert_eq!($resp.status(), 409)
    };
}
pub(crate) use assert_conflict_response;
