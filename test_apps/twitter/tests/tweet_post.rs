mod utils;

use serde_json::json;
use utils::{
    STATUS_COMPLETED, fetch_command_entries, fetch_event_entries, fetch_outbox, fetch_tweet_by_id,
    fetch_tweets, start_test_app,
};
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread")]
async fn post_tweet_success() {
    let (base_url, pool, _guard) = start_test_app().await;
    let client = utils::client();
    let author_id = Uuid::now_v7();

    let resp = client
        .post(format!("{base_url}/api/tweets"))
        .json(&json!({ "author_id": author_id, "content": "Hello world!" }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["author_id"].as_str().unwrap(), author_id.to_string());
    assert_eq!(body["content"], "Hello world!");
    assert!(
        body["id"]
            .as_str()
            .is_some_and(|s| Uuid::parse_str(s).is_ok())
    );

    let tweet_id: Uuid = body["id"].as_str().unwrap().parse().unwrap();
    let row = fetch_tweet_by_id(&pool, tweet_id).expect("tweet row should exist");
    assert_eq!(row.author_id, author_id);
    assert_eq!(row.content, "Hello world!");
    assert!(row.deleted_at.is_none());

    let cmds = fetch_command_entries(&pool);
    let post_tweet_cmds: Vec<_> = cmds
        .iter()
        .filter(|c| c.command_type == "PostTweet")
        .collect();
    assert_eq!(post_tweet_cmds.len(), 1);
    assert_eq!(post_tweet_cmds[0].status, STATUS_COMPLETED);

    let events = fetch_event_entries(&pool);
    let tweet_posted: Vec<_> = events
        .iter()
        .filter(|e| e.event_type == "TweetPosted")
        .collect();
    assert_eq!(tweet_posted.len(), 1);
    assert_eq!(tweet_posted[0].status, STATUS_COMPLETED);

    let outbox = fetch_outbox(&pool);
    assert_eq!(outbox.len(), 1);
    let ob = &outbox[0];
    assert_eq!(ob.event_type, "TweetPosted");
    assert_eq!(ob.status, "pending");
    let payload: serde_json::Value = serde_json::from_str(&ob.payload).unwrap();
    assert_eq!(payload["type"], "TweetPosted");
    assert_eq!(
        payload["payload"]["tweet_id"].as_str().unwrap(),
        tweet_id.to_string()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn post_tweet_blank_content_returns_400() {
    let (base_url, _pool, _guard) = start_test_app().await;
    let resp = utils::client()
        .post(format!("{base_url}/api/tweets"))
        .json(&json!({ "author_id": Uuid::now_v7(), "content": "   " }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

#[tokio::test(flavor = "multi_thread")]
async fn post_tweet_too_long_content_returns_400() {
    let (base_url, _pool, _guard) = start_test_app().await;
    let resp = utils::client()
        .post(format!("{base_url}/api/tweets"))
        .json(&json!({ "author_id": Uuid::now_v7(), "content": "x".repeat(281) }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

#[tokio::test(flavor = "multi_thread")]
async fn post_tweet_unicode_content_within_limit_succeeds() {
    let (base_url, _pool, _guard) = start_test_app().await;
    let content = "é".repeat(280); // 280 Unicode scalars, > 280 bytes
    let resp = utils::client()
        .post(format!("{base_url}/api/tweets"))
        .json(&json!({ "author_id": Uuid::now_v7(), "content": content }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test(flavor = "multi_thread")]
async fn post_tweet_duplicate_tweet_id_returns_409() {
    let (base_url, pool, _guard) = start_test_app().await;
    let client = utils::client();
    let author_id = Uuid::now_v7();
    let tweet_id = Uuid::now_v7();
    let first = client
        .post(format!("{base_url}/api/messages/commands"))
        .json(&json!({
            "id": Uuid::now_v7(),
            "command": {
                "type": "PostTweet",
                "tweet_id": tweet_id,
                "author_id": author_id,
                "content": "first"
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
                "type": "PostTweet",
                "tweet_id": tweet_id,
                "author_id": author_id,
                "content": "second"
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(second.status(), 409);

    let tweets = fetch_tweets(&pool);
    assert_eq!(tweets.len(), 1);
    assert_eq!(tweets[0].id, tweet_id);

    let events: Vec<_> = fetch_event_entries(&pool)
        .into_iter()
        .filter(|e| e.event_type == "TweetPosted")
        .collect();
    assert_eq!(events.len(), 1);
    assert_eq!(fetch_outbox(&pool).len(), 1);
}
