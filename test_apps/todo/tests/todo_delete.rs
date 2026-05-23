mod utils;

use reqwest::Client;
use serde_json::json;
use utils::{
    STATUS_COMPLETED, fetch_command_entries, fetch_event_entries, fetch_outbox, start_test_app,
};
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread")]
async fn delete_todo_emits_event() {
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

    let response = client
        .delete(format!("{base_url}/api/todos/{todo_id}"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 204);

    let todos_response = client
        .get(format!("{base_url}/api/todos"))
        .send()
        .await
        .unwrap();
    let todos_body = todos_response.json::<serde_json::Value>().await.unwrap();
    let todos = todos_body["items"].as_array().unwrap();
    assert!(
        todos
            .iter()
            .all(|t| t["id"].as_str().unwrap() != todo_id.to_string())
    );

    let outbox = fetch_outbox(&pool).await;
    let deleted_event = outbox.iter().find(|e| e.event_type == "TodoDeleted");
    assert!(deleted_event.is_some());
    assert_eq!(deleted_event.unwrap().status, "pending");

    let commands = fetch_command_entries(&pool).await;
    let delete_cmd = commands.iter().find(|c| c.command_type == "DeleteTodo");
    assert!(delete_cmd.is_some(), "DeleteTodo command entry missing");
    assert_eq!(delete_cmd.unwrap().status, STATUS_COMPLETED);

    let events = fetch_event_entries(&pool).await;
    let delete_evt = events.iter().find(|e| e.event_type == "TodoDeleted");
    assert!(delete_evt.is_some(), "TodoDeleted event entry missing");
    assert_eq!(delete_evt.unwrap().status, STATUS_COMPLETED);

    let count = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM todos WHERE id = $1")
        .bind(todo_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0, "todo row should be absent after delete");
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_nonexistent_todo_returns_404() {
    let (base_url, _pool, _guard) = start_test_app().await;
    let id = Uuid::now_v7();

    let response = Client::new()
        .delete(format!("{base_url}/api/todos/{id}"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 404);
}
