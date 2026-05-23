mod utils;

use serde_json::json;
use utils::{fetch_direct_messages, fetch_event_entries, fetch_outbox, start_test_app};
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread")]
async fn send_direct_message_success() {
    let (base_url, pool, _guard) = start_test_app().await;
    let sender = Uuid::now_v7();
    let recipient = Uuid::now_v7();

    let resp = utils::client()
        .post(format!("{base_url}/api/messages/direct"))
        .json(&json!({ "sender_id": sender, "recipient_id": recipient, "content": "Hi there!" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["sender_id"].as_str().unwrap(), sender.to_string());
    assert_eq!(body["content"], "Hi there!");

    let dms = fetch_direct_messages(&pool);
    assert_eq!(dms.len(), 1);
    assert_eq!(dms[0].sender_id, sender);

    let outbox = fetch_outbox(&pool);
    assert!(outbox.iter().any(|r| r.event_type == "DirectMessageSent"));
}

#[tokio::test(flavor = "multi_thread")]
async fn send_direct_message_blank_content_returns_400() {
    let (base_url, _pool, _guard) = start_test_app().await;
    let resp = utils::client()
        .post(format!("{base_url}/api/messages/direct"))
        .json(
            &json!({ "sender_id": Uuid::now_v7(), "recipient_id": Uuid::now_v7(), "content": "" }),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

#[tokio::test(flavor = "multi_thread")]
async fn send_direct_message_duplicate_message_id_returns_409() {
    let (base_url, pool, _guard) = start_test_app().await;
    let client = utils::client();
    let message_id = Uuid::now_v7();

    let first = client
        .post(format!("{base_url}/api/messages/commands"))
        .json(&json!({
            "id": Uuid::now_v7(),
            "command": {
                "type": "SendDirectMessage",
                "message_id": message_id,
                "sender_id": Uuid::now_v7(),
                "recipient_id": Uuid::now_v7(),
                "content": "hello"
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(first.status(), 200);

    let second = client
        .post(format!("{base_url}/api/messages/commands"))
        .json(&json!({
            "id": Uuid::now_v7(),
            "command": {
                "type": "SendDirectMessage",
                "message_id": message_id,
                "sender_id": Uuid::now_v7(),
                "recipient_id": Uuid::now_v7(),
                "content": "duplicate"
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(second.status(), 409);

    assert_eq!(fetch_direct_messages(&pool).len(), 1);
    let events: Vec<_> = fetch_event_entries(&pool)
        .into_iter()
        .filter(|e| e.event_type == "DirectMessageSent")
        .collect();
    assert_eq!(events.len(), 1);
    assert_eq!(fetch_outbox(&pool).len(), 1);
}
