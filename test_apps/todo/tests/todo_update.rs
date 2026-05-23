mod utils;

use reqwest::Client;
use serde_json::json;
use test_app_todo::io::TodoRow;
use utils::{
    STATUS_COMPLETED, fetch_command_entries, fetch_event_entries, fetch_outbox, start_test_app,
};
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread")]
async fn update_todo_dispatches_command_and_emits_event() {
    let (base_url, pool, _guard) = start_test_app().await;
    let client = Client::new();

    let todo_id: Uuid = client
        .post(format!("{base_url}/api/todos"))
        .json(&json!({"title": "Original", "description": "Original desc"}))
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

    let update_response = client
        .put(format!("{base_url}/api/todos/{todo_id}"))
        .json(&json!({"title": "Updated", "description": "Updated desc"}))
        .send()
        .await
        .unwrap();

    assert_eq!(update_response.status(), 200);
    let body = update_response.json::<serde_json::Value>().await.unwrap();
    assert_eq!(body["title"], "Updated");
    assert_eq!(body["description"], "Updated desc");

    let outbox = fetch_outbox(&pool).await;
    let updated_event = outbox.iter().find(|e| e.event_type == "TodoUpdated");
    assert!(updated_event.is_some());
    assert_eq!(updated_event.unwrap().status, "pending");

    let commands = fetch_command_entries(&pool).await;
    let update_cmd = commands.iter().find(|c| c.command_type == "UpdateTodo");
    assert!(update_cmd.is_some(), "UpdateTodo command entry missing");
    assert_eq!(update_cmd.unwrap().status, STATUS_COMPLETED);

    let events = fetch_event_entries(&pool).await;
    let update_evt = events.iter().find(|e| e.event_type == "TodoUpdated");
    assert!(update_evt.is_some(), "TodoUpdated event entry missing");
    assert_eq!(update_evt.unwrap().status, STATUS_COMPLETED);

    let row = sqlx::query_as::<_, TodoRow>(
        "SELECT id, title, description, status, created_at, updated_at, due_at FROM todos WHERE id = $1",
    )
    .bind(todo_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row.title, "Updated");
    assert_eq!(row.description.as_deref(), Some("Updated desc"));
}

#[tokio::test(flavor = "multi_thread")]
async fn update_todo_with_blank_title_returns_400() {
    let (base_url, _pool, _guard) = start_test_app().await;
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

    let response = client
        .put(format!("{base_url}/api/todos/{todo_id}"))
        .json(&json!({"title": "  ", "description": ""}))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 400);
}
