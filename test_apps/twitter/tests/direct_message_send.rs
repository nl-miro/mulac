mod utils;

use serde_json::json;
use utils::{
    assert_bad_request_response,
    assert_conflict_response,
    assert_event_completed,
    assert_ok_response,
    assert_outbox_pending,
    fetch_direct_messages,
    start_test_app, //
};
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread")]
async fn send_direct_message_success() {
    let (base_url, pool, _guard) = start_test_app().await;
    let sender = Uuid::now_v7();
    let recipient = Uuid::now_v7();

    let resp = send_direct_message(&base_url, sender, recipient, "Hi there!").await;

    assert_ok_response!(resp);

    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["sender_id"].as_str().unwrap(), sender.to_string());
    assert_eq!(body["content"], "Hi there!");

    let dms = fetch_direct_messages(&pool);
    assert_eq!(dms.len(), 1);
    assert_eq!(dms[0].sender_id, sender);

    assert_outbox_pending(&pool, "DirectMessageSent");
}

async fn send_direct_message(
    base_url: &str,
    sender: Uuid,
    recipient: Uuid,
    content: &str,
) -> reqwest::Response {
    utils::client()
        .post(format!("{base_url}/api/messages/direct"))
        .json(&json!({ "sender_id": sender, "recipient_id": recipient, "content": content }))
        .send()
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn send_direct_message_blank_content_returns_400() {
    let (base_url, _pool, _guard) = start_test_app().await;
    let resp = send_direct_message(&base_url, Uuid::now_v7(), Uuid::now_v7(), "").await;

    assert_bad_request_response!(resp);
}

#[tokio::test(flavor = "multi_thread")]
async fn send_direct_message_duplicate_message_id_returns_409() {
    let (base_url, pool, _guard) = start_test_app().await;
    let client = utils::client();
    let message_id = Uuid::now_v7();

    let first = send_direct_message_command(
        &client,
        &base_url,
        message_id,
        Uuid::now_v7(),
        Uuid::now_v7(),
        "hello",
    )
    .await;
    assert_ok_response!(first);

    let second = send_direct_message_command(
        &client,
        &base_url,
        message_id,
        Uuid::now_v7(),
        Uuid::now_v7(),
        "duplicate",
    )
    .await;
    assert_conflict_response!(second);

    assert_eq!(fetch_direct_messages(&pool).len(), 1);
    assert_event_completed(&pool, "DirectMessageSent");
    assert_outbox_pending(&pool, "DirectMessageSent");
}

async fn send_direct_message_command(
    client: &reqwest::Client,
    base_url: &str,
    message_id: Uuid,
    sender_id: Uuid,
    recipient_id: Uuid,
    content: &str,
) -> reqwest::Response {
    client
        .post(format!("{base_url}/api/messages/commands"))
        .json(&json!({
            "id": Uuid::now_v7(),
            "command": {
                "type": "SendDirectMessage",
                "message_id": message_id,
                "sender_id": sender_id,
                "recipient_id": recipient_id,
                "content": content,
            }
        }))
        .send()
        .await
        .unwrap()
}
