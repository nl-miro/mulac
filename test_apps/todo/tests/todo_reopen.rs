mod utils;

use reqwest::Client;
use serde_json::json;
use test_app_todo::io::TodoRow;
use utils::{
    STATUS_COMPLETED, fetch_command_entries, fetch_event_entries, fetch_outbox, start_test_app,
};
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread")]
async fn reopen_todo_emits_event() {
    let (base_url, pool, _guard) = start_test_app().await;
    let client = Client::new();

    let todo_id: Uuid = client
        .post(format!("{base_url}/api/todos"))
        .json(&json!({"title": "Task"}))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();

    client
        .post(format!("{base_url}/api/todos/{todo_id}/complete"))
        .send()
        .await
        .unwrap();

    let response = client
        .post(format!("{base_url}/api/todos/{todo_id}/reopen"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body = response.json::<serde_json::Value>().await.unwrap();
    assert_eq!(body["status"], "active");

    let outbox = fetch_outbox(&pool).await;
    let reopened_event = outbox.iter().find(|e| e.event_type == "TodoReopened");
    assert!(reopened_event.is_some());
    assert_eq!(reopened_event.unwrap().status, "pending");

    let commands = fetch_command_entries(&pool).await;
    let reopen_cmd = commands.iter().find(|c| c.command_type == "ReopenTodo");
    assert!(reopen_cmd.is_some(), "ReopenTodo command entry missing");
    let reopen_cmd = reopen_cmd.unwrap();
    assert_eq!(reopen_cmd.status, STATUS_COMPLETED);
    assert_eq!(reopen_cmd.attempts, 1);

    let events = fetch_event_entries(&pool).await;
    let reopen_evt = events.iter().find(|e| e.event_type == "TodoReopened");
    assert!(reopen_evt.is_some(), "TodoReopened event entry missing");
    assert_eq!(reopen_evt.unwrap().status, STATUS_COMPLETED);

    let row = sqlx::query_as::<_, TodoRow>(
        "SELECT id, title, description, status, created_at, updated_at, due_at FROM todos WHERE id = $1",
    )
    .bind(todo_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row.status, "active");
}

#[tokio::test(flavor = "multi_thread")]
async fn reopen_nonexistent_todo_returns_404() {
    let (base_url, _pool, _guard) = start_test_app().await;
    let id = Uuid::now_v7();

    let response = Client::new()
        .post(format!("{base_url}/api/todos/{id}/reopen"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 404);
}
