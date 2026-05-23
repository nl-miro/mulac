mod utils;

use reqwest::Client;
use serde_json::json;
use test_app_todo::io::TodoRow;
use utils::{
    STATUS_COMPLETED, fetch_command_entries, fetch_event_entries, fetch_outbox, start_test_app,
};
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread")]
async fn complete_todo_emits_event() {
    let (base_url, pool, _guard) = start_test_app().await;
    let client = Client::new();

    let create_response = client
        .post(format!("{base_url}/api/todos"))
        .json(&json!({"title": "Task"}))
        .send()
        .await
        .unwrap();
    let todo_id: Uuid = create_response.json::<serde_json::Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();

    let response = client
        .post(format!("{base_url}/api/todos/{todo_id}/complete"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body = response.json::<serde_json::Value>().await.unwrap();
    assert_eq!(body["status"], "completed");

    let outbox = fetch_outbox(&pool).await;
    let completed_event = outbox.iter().find(|e| e.event_type == "TodoCompleted");
    assert!(completed_event.is_some());
    assert_eq!(completed_event.unwrap().status, "pending");

    let commands = fetch_command_entries(&pool).await;
    let complete_cmd = commands.iter().find(|c| c.command_type == "CompleteTodo");
    assert!(complete_cmd.is_some(), "CompleteTodo command entry missing");
    let complete_cmd = complete_cmd.unwrap();
    assert_eq!(complete_cmd.status, STATUS_COMPLETED);
    assert_eq!(complete_cmd.attempts, 1);
    assert!(complete_cmd.processed_at.is_some());
    let cmd_payload: serde_json::Value = serde_json::from_str(&complete_cmd.payload).unwrap();
    assert_eq!(cmd_payload["todo_id"], todo_id.to_string());

    let events = fetch_event_entries(&pool).await;
    let complete_evt = events.iter().find(|e| e.event_type == "TodoCompleted");
    assert!(complete_evt.is_some(), "TodoCompleted event entry missing");
    let complete_evt = complete_evt.unwrap();
    assert_eq!(complete_evt.status, STATUS_COMPLETED);
    assert_eq!(complete_evt.attempts, 1);
    assert!(complete_evt.processed_at.is_some());

    let row = sqlx::query_as::<_, TodoRow>(
        "SELECT id, title, description, status, created_at, updated_at, due_at FROM todos WHERE id = $1",
    )
    .bind(todo_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row.status, "completed");
}

#[tokio::test(flavor = "multi_thread")]
async fn complete_nonexistent_todo_returns_404() {
    let (base_url, _pool, _guard) = start_test_app().await;
    let id = Uuid::now_v7();

    let response = Client::new()
        .post(format!("{base_url}/api/todos/{id}/complete"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 404);
}
