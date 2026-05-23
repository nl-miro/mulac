mod utils;

use reqwest::Client;
use serde_json::json;
use utils::{
    STATUS_COMPLETED, fetch_command_entries, fetch_event_entries, fetch_inbox, fetch_outbox,
    start_test_app,
};
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread")]
async fn inbound_command_records_inbox_lifecycle() {
    let (base_url, pool, _guard) = start_test_app().await;
    let client = Client::new();
    let message_id = Uuid::now_v7();
    let todo_id = Uuid::now_v7();

    let response = client
        .post(format!("{base_url}/api/messages/commands"))
        .json(&json!({
            "id": message_id,
            "command": {
                "type": "CreateTodo",
                "todo_id": todo_id,
                "title": "Inbox-created todo",
                "description": "Created through inbound messages"
            }
        }))
        .send()
        .await
        .unwrap();

    let status = response.status();
    let text = response.text().await.unwrap();
    assert_eq!(status, 200);

    let body: serde_json::Value = serde_json::from_str(&text).unwrap();
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
    let client = Client::new();

    let todo_id: Uuid = client
        .post(format!("{base_url}/api/todos"))
        .json(&json!({"title": "Inbox due date"}))
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

    let message_id = Uuid::now_v7();
    let due_date = "2026-12-31T23:59:59Z";
    let response = client
        .post(format!("{base_url}/api/messages/commands"))
        .json(&json!({
            "id": message_id,
            "command": {
                "type": "UpdateDueDate",
                "todo_id": todo_id,
                "due_at": due_date
            }
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
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

    let outbox = fetch_outbox(&pool).await;
    assert!(
        outbox
            .iter()
            .any(|event| event.event_type == "TodoDueDateChanged")
    );

    let commands = fetch_command_entries(&pool).await;
    let due_cmd = commands
        .iter()
        .find(|command| command.command_type == "UpdateDueDate")
        .expect("UpdateDueDate command entry missing");
    assert_eq!(due_cmd.status, STATUS_COMPLETED);

    let events = fetch_event_entries(&pool).await;
    let due_event = events
        .iter()
        .find(|event| event.event_type == "TodoDueDateChanged")
        .expect("TodoDueDateChanged event entry missing");
    assert_eq!(due_event.status, STATUS_COMPLETED);
}

#[tokio::test(flavor = "multi_thread")]
async fn duplicate_inbox_message_id_returns_409() {
    let (base_url, _pool, _guard) = start_test_app().await;
    let client = Client::new();
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

    let first = client
        .post(format!("{base_url}/api/messages/commands"))
        .json(&payload)
        .send()
        .await
        .unwrap();
    assert_eq!(first.status(), 200);

    let second = client
        .post(format!("{base_url}/api/messages/commands"))
        .json(&payload)
        .send()
        .await
        .unwrap();
    assert_eq!(second.status(), 409);
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
    let client = Client::new();

    let response = client
        .post(format!("{base_url}/api/messages/commands"))
        .json(&json!({"id": Uuid::now_v7()}))
        .send()
        .await
        .unwrap();
    assert!(
        response.status().is_client_error(),
        "missing command field should return 4xx, got {}",
        response.status()
    );

    let response = client
        .post(format!("{base_url}/api/messages/commands"))
        .json(&json!({
            "id": Uuid::now_v7(),
            "command": {
                "type": "UnknownCommand",
                "todo_id": Uuid::now_v7()
            }
        }))
        .send()
        .await
        .unwrap();
    assert!(
        response.status().is_client_error(),
        "unknown command type should return 4xx, got {}",
        response.status()
    );
}
