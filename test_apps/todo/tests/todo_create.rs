mod utils;

use reqwest::Client;
use serde_json::json;
use test_app_todo::io::TodoRow;
use utils::{
    STATUS_COMPLETED, fetch_command_entries, fetch_event_entries, fetch_inbox, fetch_outbox,
    start_test_app,
};
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread")]
async fn create_todo_returns_todo() {
    let (base_url, pool, _guard) = start_test_app().await;
    let client = Client::new();

    let response = client
        .post(format!("{base_url}/api/todos"))
        .json(&json!({
            "title": "Buy milk",
            "description": "From the corner store"
        }))
        .send()
        .await
        .unwrap();

    let status = response.status();
    let text = response.text().await.unwrap();
    assert_eq!(status, 200);

    let body: serde_json::Value = serde_json::from_str(&text).unwrap();

    assert_eq!(body["title"], "Buy milk");
    assert_eq!(body["description"], "From the corner store");
    assert_eq!(body["status"], "active");
    assert!(body["id"].as_str().is_some_and(|s| !s.is_empty()));
    assert!(body["created_at"].is_string());

    let todo_id: Uuid = body["id"].as_str().unwrap().parse().unwrap();
    let row = sqlx::query_as::<_, TodoRow>(
        "SELECT id, title, description, status, created_at, updated_at, due_at FROM todos WHERE id = $1",
    )
    .bind(todo_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(row.id, todo_id);
    assert_eq!(row.title, "Buy milk");
    assert_eq!(row.description.as_deref(), Some("From the corner store"));
    assert_eq!(row.status, "active");
    assert!(row.due_at.is_none());

    let outbox = fetch_outbox(&pool).await;

    assert_eq!(outbox.len(), 1);
    let event = &outbox[0];
    assert_eq!(event.event_type, "TodoCreated");
    assert_eq!(event.status, "pending");
    assert_eq!(event.attempts, 0);
    assert!(event.published_at.is_none());
    assert_eq!(event.payload["type"], "TodoCreated");
    let todo_payload = &event.payload["payload"]["todo"];
    assert_eq!(todo_payload["id"], body["id"]);
    assert_eq!(todo_payload["title"], "Buy milk");
    assert_eq!(todo_payload["description"], "From the corner store");
    assert_eq!(todo_payload["status"], "active");
    assert!(todo_payload["created_at"].is_string());
    assert!(todo_payload["updated_at"].is_string());
    assert!(todo_payload["due_at"].is_null());

    let commands = fetch_command_entries(&pool).await;

    assert_eq!(commands.len(), 1);
    let command = &commands[0];
    assert_eq!(command.command_type, "CreateTodo");
    assert_eq!(command.status, STATUS_COMPLETED);
    assert_eq!(command.attempts, 1);
    assert!(command.reservation_id.is_none());
    assert!(command.processed_at.is_some());
    let command_payload: serde_json::Value = serde_json::from_str(&command.payload).unwrap();
    assert_eq!(command_payload["todo_id"], body["id"]);
    assert_eq!(command_payload["title"], "Buy milk");
    assert_eq!(command_payload["description"], "From the corner store");
    let command_meta = command.meta.as_ref().unwrap();
    assert!(command_meta["command_id"].is_string());
    assert!(command_meta["correlation_id"].is_string());
    assert!(command_meta["causation_id"].is_null());
    assert_eq!(command_meta["source"], "test_app_todo.http");

    let events = fetch_event_entries(&pool).await;

    assert_eq!(events.len(), 1);
    let event_entry = &events[0];
    assert_eq!(event_entry.event_type, "TodoCreated");
    assert_eq!(event_entry.status, STATUS_COMPLETED);
    assert_eq!(event_entry.attempts, 1);
    assert!(event_entry.reservation_id.is_none());
    assert!(event_entry.processed_at.is_some());
    let event_payload: serde_json::Value = serde_json::from_str(&event_entry.payload).unwrap();
    assert_eq!(event_payload["type"], "TodoCreated");
    assert_eq!(event_payload["payload"]["todo"]["id"], body["id"]);
    let event_meta = event_entry.meta.as_ref().unwrap();
    assert!(event_meta["event_id"].is_string());
    assert!(event_meta["correlation_id"].is_string());
    assert!(event_meta["causation_id"].is_string());

    let inbox = fetch_inbox(&pool).await;
    assert_eq!(inbox.len(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn create_todo_with_blank_title_returns_400() {
    let (base_url, _pool, _guard) = start_test_app().await;
    let client = Client::new();

    let response = client
        .post(format!("{base_url}/api/todos"))
        .json(&json!({"title": "   "}))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 400);
    let body = response.json::<serde_json::Value>().await.unwrap();
    assert!(body["error"].as_str().unwrap().contains("blank"));
}
