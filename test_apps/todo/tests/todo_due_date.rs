mod utils;

use reqwest::Client;
use serde_json::json;
use test_app_todo::io::TodoRow;
use utils::{
    STATUS_COMPLETED, fetch_command_entries, fetch_event_entries, fetch_outbox, start_test_app,
};
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread")]
async fn update_due_date_emits_event() {
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

    let due_date = "2026-12-31T23:59:59Z";
    let response = client
        .put(format!("{base_url}/api/todos/{todo_id}/due-date"))
        .json(&json!({"due_at": due_date}))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body = response.json::<serde_json::Value>().await.unwrap();
    assert!(
        body["due_at"]
            .as_str()
            .unwrap()
            .contains("2026-12-31T23:59:59")
    );

    let outbox = fetch_outbox(&pool).await;
    let due_date_event = outbox.iter().find(|e| e.event_type == "TodoDueDateChanged");
    assert!(due_date_event.is_some());
    assert_eq!(due_date_event.unwrap().status, "pending");

    let commands = fetch_command_entries(&pool).await;
    let due_cmd = commands.iter().find(|c| c.command_type == "UpdateDueDate");
    assert!(due_cmd.is_some(), "UpdateDueDate command entry missing");
    let due_cmd = due_cmd.unwrap();
    assert_eq!(due_cmd.status, STATUS_COMPLETED);
    assert_eq!(due_cmd.attempts, 1);

    let events = fetch_event_entries(&pool).await;
    let due_evt = events.iter().find(|e| e.event_type == "TodoDueDateChanged");
    assert!(due_evt.is_some(), "TodoDueDateChanged event entry missing");
    assert_eq!(due_evt.unwrap().status, STATUS_COMPLETED);

    let row = sqlx::query_as::<_, TodoRow>(
        "SELECT id, title, description, status, created_at, updated_at, due_at FROM todos WHERE id = $1",
    )
    .bind(todo_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(row.due_at.is_some());
    assert!(row.due_at.unwrap().to_string().contains("2026-12-31"));
}

#[tokio::test(flavor = "multi_thread")]
async fn update_due_date_nonexistent_todo_returns_404() {
    let (base_url, _pool, _guard) = start_test_app().await;
    let id = Uuid::now_v7();

    let response = Client::new()
        .put(format!("{base_url}/api/todos/{id}/due-date"))
        .json(&json!({"due_at": "2026-12-31T23:59:59Z"}))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 404);
}
