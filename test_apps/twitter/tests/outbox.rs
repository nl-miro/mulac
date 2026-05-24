mod utils;

use serde_json::json;
use utils::{
    assert_ok_response,
    fetch_outbox,
    start_test_app, //
};
use uuid::Uuid;

async fn create_tweet(base_url: &str, author_id: Uuid, content: &str) {
    let resp = utils::client()
        .post(format!("{base_url}/api/tweets"))
        .json(&json!({ "author_id": author_id, "content": content }))
        .send()
        .await
        .unwrap();
    assert_ok_response!(resp);
}

async fn send_command(base_url: &str, payload: serde_json::Value) -> reqwest::Response {
    utils::client()
        .post(format!("{base_url}/api/messages/commands"))
        .json(&payload)
        .send()
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn outbox_lists_events_in_created_at_order() {
    let (base_url, _pool, _guard) = start_test_app().await;
    let author = Uuid::now_v7();

    create_tweet(&base_url, author, "first").await;
    create_tweet(&base_url, author, "second").await;

    let resp = utils::client()
        .get(format!("{base_url}/api/messages/outbox"))
        .send()
        .await
        .unwrap();
    assert_ok_response!(resp);
    let body: serde_json::Value = resp.json().await.unwrap();
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);

    // Verify tagged payload shape.
    for item in items {
        assert_eq!(item["event_type"], "TweetPosted");
        assert_eq!(item["status"], "pending");
        assert!(item["id"].is_string());
        let payload = &item["payload"];
        assert_eq!(payload["type"], "TweetPosted");
        assert!(payload["payload"]["tweet_id"].is_string());
    }

    // Verify created_at ordering.
    let t0 = &items[0]["created_at"].as_str().unwrap();
    let t1 = &items[1]["created_at"].as_str().unwrap();
    assert!(t0 <= t1, "outbox rows should be ordered by created_at ASC");
}

#[tokio::test(flavor = "multi_thread")]
async fn outbox_no_duplicate_row_per_event_id() {
    let (base_url, pool, _guard) = start_test_app().await;
    let msg_id = Uuid::now_v7();
    let tweet_id = Uuid::now_v7();

    // Use inbox to control the event_id (message_id becomes command_id → event correlation).
    send_command(
        &base_url,
        json!({
            "id": msg_id,
            "command": {
                "type": "PostTweet",
                "tweet_id": tweet_id,
                "author_id": Uuid::now_v7(),
                "content": "idempotency test"
            }
        }),
    )
    .await;

    // Outbox should have exactly one row for this tweet's event.
    let rows = fetch_outbox(&pool);
    let count = rows
        .iter()
        .filter(|r| r.event_type == "TweetPosted")
        .count();
    assert_eq!(count, 1, "exactly one outbox row per event");
}
