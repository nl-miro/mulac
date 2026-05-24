mod utils;
use serde_json::json;
use utils::{
    assert_command_completed, assert_conflict_response, assert_event_completed, assert_ok_response,
    assert_outbox_pending, fetch_inbox, start_test_app,
};
use uuid::Uuid;

async fn send_command(base_url: &str, payload: &serde_json::Value) -> reqwest::Response {
    utils::client()
        .post(format!("{base_url}/api/messages/commands"))
        .json(payload)
        .send()
        .await
        .unwrap()
}

async fn create_todo(base_url: &str, title: &str) -> Uuid {
    utils::client()
        .post(format!("{base_url}/api/todos"))
        .json(&json!({"title": title}))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn inbound_command_records_inbox_lifecycle() {
    let (base_url, pool, _guard) = start_test_app().await;
    let message_id = Uuid::now_v7();
    let todo_id = Uuid::now_v7();

    let response = send_command(
        &base_url,
        &json!({
            "id": message_id,
            "command": {
                "type": "CreateTodo",
                "todo_id": todo_id,
                "title": "Inbox-created todo",
                "description": "Created through inbound messages"
            }
        }),
    )
    .await;

    assert_ok_response!(response);
    let body = response.json::<serde_json::Value>().await.unwrap();
    assert_eq!(body["message_id"], message_id.to_string());
    assert_eq!(body["todo"]["id"], todo_id.to_string());
    assert_eq!(body["todo"]["title"], "Inbox-created todo");

    let inbox = fetch_inbox(&pool).await;
    assert_eq!(inbox.len(), 1);

    let row = &inbox[0];
    assert_eq!(row.id, message_id);
    assert_eq!(row.message_type, "CreateTodo");
    assert_eq!(row.status, "processed");
    assert!(row.processed_at.is_some());
    assert!(row.error.is_none());
    assert_eq!(row.payload["type"], "CreateTodo");
    assert_eq!(row.payload["payload"]["todo_id"], todo_id.to_string());
    assert_eq!(row.payload["payload"]["title"], "Inbox-created todo");
    assert_eq!(
        row.payload["payload"]["description"],
        "Created through inbound messages"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn inbound_update_due_date_dispatches_command() {
    let (base_url, pool, _guard) = start_test_app().await;

    let todo_id = create_todo(&base_url, "Inbox due date").await;

    let message_id = Uuid::now_v7();
    let due_date = "2026-12-31T23:59:59Z";
    let response = send_command(
        &base_url,
        &json!({
            "id": message_id,
            "command": {
                "type": "UpdateDueDate",
                "todo_id": todo_id,
                "due_at": due_date
            }
        }),
    )
    .await;

    assert_ok_response!(response);
    let body = response.json::<serde_json::Value>().await.unwrap();
    assert_eq!(body["message_id"], message_id.to_string());
    assert_eq!(body["todo"]["id"], todo_id.to_string());
    assert!(
        body["todo"]["due_at"]
            .as_str()
            .unwrap()
            .contains("2026-12-31T23:59:59")
    );

    let inbox = fetch_inbox(&pool).await;
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0].message_type, "UpdateDueDate");
    assert_eq!(inbox[0].status, "processed");
    assert_eq!(inbox[0].payload["type"], "UpdateDueDate");
    assert_eq!(inbox[0].payload["payload"]["todo_id"], todo_id.to_string());

    assert_outbox_pending(&pool, "TodoDueDateChanged").await;
    assert_command_completed(&pool, "UpdateDueDate").await;
    assert_event_completed(&pool, "TodoDueDateChanged").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn duplicate_inbox_message_id_returns_409() {
    let (base_url, _pool, _guard) = start_test_app().await;
    let message_id = Uuid::now_v7();
    let todo_id = Uuid::now_v7();

    let payload = json!({
        "id": message_id,
        "command": {
            "type": "CreateTodo",
            "todo_id": todo_id,
            "title": "Idempotency test"
        }
    });

    let first = send_command(&base_url, &payload).await;
    assert_ok_response!(first);

    let second = send_command(&base_url, &payload).await;
    assert_conflict_response!(second);
    let body = second.json::<serde_json::Value>().await.unwrap();
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains(&message_id.to_string())
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn malformed_inbox_command_payload_returns_error() {
    let (base_url, _pool, _guard) = start_test_app().await;

    let response = send_command(&base_url, &json!({"id": Uuid::now_v7()})).await;
    assert!(
        response.status().is_client_error(),
        "missing command field should return 4xx, got {}",
        response.status()
    );

    let response = send_command(
        &base_url,
        &json!({
            "id": Uuid::now_v7(),
            "command": {
                "type": "UnknownCommand",
                "todo_id": Uuid::now_v7()
            }
        }),
    )
    .await;
    assert!(
        response.status().is_client_error(),
        "unknown command type should return 4xx, got {}",
        response.status()
    );
}
