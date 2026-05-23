mod utils;

use serde_json::json;
use utils::{fetch_inbox, fetch_outbox, start_test_app};
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread")]
async fn inbox_post_tweet_success() {
    let (base_url, pool, _guard) = start_test_app().await;
    let tweet_id = Uuid::now_v7();
    let author_id = Uuid::now_v7();
    let msg_id = Uuid::now_v7();

    let resp = utils::client()
        .post(format!("{base_url}/api/messages/commands"))
        .json(&json!({
            "id": msg_id,
            "command": {
                "type": "PostTweet",
                "tweet_id": tweet_id,
                "author_id": author_id,
                "content": "inbox test tweet"
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["message_id"].as_str().unwrap(), msg_id.to_string());
    assert_eq!(body["entity"]["type"], "Tweet");

    let inbox = fetch_inbox(&pool);
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0].status, "processed");
    assert!(inbox[0].processed_at.is_some());

    let outbox = fetch_outbox(&pool);
    assert!(outbox.iter().any(|r| r.event_type == "TweetPosted"));
}

#[tokio::test(flavor = "multi_thread")]
async fn inbox_duplicate_id_returns_409() {
    let (base_url, _pool, _guard) = start_test_app().await;
    let client = utils::client();
    let msg_id = Uuid::now_v7();
    let payload = json!({
        "id": msg_id,
        "command": {
            "type": "PostTweet",
            "tweet_id": Uuid::now_v7(),
            "author_id": Uuid::now_v7(),
            "content": "first"
        }
    });

    client
        .post(format!("{base_url}/api/messages/commands"))
        .json(&payload)
        .send()
        .await
        .unwrap();

    let resp = client
        .post(format!("{base_url}/api/messages/commands"))
        .json(&payload)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 409);
}

#[tokio::test(flavor = "multi_thread")]
async fn inbox_failed_command_marks_failed() {
    let (base_url, pool, _guard) = start_test_app().await;
    let msg_id = Uuid::now_v7();

    // DeleteTweet for non-existent tweet → handler error → 404
    let resp = utils::client()
        .post(format!("{base_url}/api/messages/commands"))
        .json(&json!({
            "id": msg_id,
            "command": {
                "type": "DeleteTweet",
                "tweet_id": Uuid::now_v7(),
                "author_id": Uuid::now_v7()
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);

    let inbox = fetch_inbox(&pool);
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0].status, "failed");
    assert!(inbox[0].error.is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn inbox_delete_success_returns_no_entity() {
    let (base_url, _pool, _guard) = start_test_app().await;
    let client = utils::client();
    let author_id = Uuid::now_v7();

    let created = client
        .post(format!("{base_url}/api/tweets"))
        .json(&json!({ "author_id": author_id, "content": "delete me" }))
        .send()
        .await
        .unwrap();
    let tweet_id: Uuid = created.json::<serde_json::Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();

    let resp = client
        .post(format!("{base_url}/api/messages/commands"))
        .json(&json!({
            "id": Uuid::now_v7(),
            "command": {
                "type": "DeleteTweet",
                "tweet_id": tweet_id,
                "author_id": author_id
            }
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["entity"]["type"], "NoEntity");
}

#[tokio::test(flavor = "multi_thread")]
async fn inbox_unlike_success_returns_no_entity() {
    let (base_url, _pool, _guard) = start_test_app().await;
    let client = utils::client();
    let author_id = Uuid::now_v7();
    let user_id = Uuid::now_v7();

    let created = client
        .post(format!("{base_url}/api/tweets"))
        .json(&json!({ "author_id": author_id, "content": "like me" }))
        .send()
        .await
        .unwrap();
    let tweet_id: Uuid = created.json::<serde_json::Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();

    client
        .post(format!("{base_url}/api/tweets/{tweet_id}/like"))
        .json(&json!({ "user_id": user_id }))
        .send()
        .await
        .unwrap();

    let resp = client
        .post(format!("{base_url}/api/messages/commands"))
        .json(&json!({
            "id": Uuid::now_v7(),
            "command": {
                "type": "UnlikeTweet",
                "user_id": user_id,
                "tweet_id": tweet_id
            }
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["entity"]["type"], "NoEntity");
}

#[tokio::test(flavor = "multi_thread")]
async fn inbox_malformed_payload_returns_400() {
    let (base_url, pool, _guard) = start_test_app().await;

    let resp = utils::client()
        .post(format!("{base_url}/api/messages/commands"))
        .json(&json!({
            "id": Uuid::now_v7(),
            "command": {
                "type": "PostTweet",
                "author_id": Uuid::now_v7()
            }
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 400);
    assert!(fetch_inbox(&pool).is_empty());
}
