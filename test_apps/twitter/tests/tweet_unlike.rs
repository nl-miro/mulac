mod utils;

use serde_json::json;
use utils::{fetch_event_entries, fetch_likes, fetch_outbox, start_test_app};
use uuid::Uuid;

async fn create_tweet(base_url: &str) -> Uuid {
    let resp = utils::client()
        .post(format!("{base_url}/api/tweets"))
        .json(&json!({ "author_id": Uuid::now_v7(), "content": "likeable" }))
        .send()
        .await
        .unwrap();
    resp.json::<serde_json::Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn unlike_existing_like_returns_204_with_event() {
    let (base_url, pool, _guard) = start_test_app().await;
    let client = utils::client();
    let tweet_id = create_tweet(&base_url).await;
    let user = Uuid::now_v7();

    client
        .post(format!("{base_url}/api/tweets/{tweet_id}/like"))
        .json(&json!({ "user_id": user }))
        .send()
        .await
        .unwrap();
    let resp = client
        .delete(format!("{base_url}/api/tweets/{tweet_id}/like"))
        .json(&json!({ "user_id": user }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 204);

    assert!(fetch_likes(&pool).is_empty());
    let outbox = fetch_outbox(&pool);
    assert!(outbox.iter().any(|r| r.event_type == "TweetUnliked"));
}

#[tokio::test(flavor = "multi_thread")]
async fn unlike_absent_like_returns_204_no_event() {
    let (base_url, pool, _guard) = start_test_app().await;
    let tweet_id = create_tweet(&base_url).await;

    let resp = utils::client()
        .delete(format!("{base_url}/api/tweets/{tweet_id}/like"))
        .json(&json!({ "user_id": Uuid::now_v7() }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 204);

    let events = fetch_event_entries(&pool);
    assert!(events.iter().all(|e| e.event_type != "TweetUnliked"));
}

#[tokio::test(flavor = "multi_thread")]
async fn unlike_missing_tweet_returns_404() {
    let (base_url, _pool, _guard) = start_test_app().await;
    let resp = utils::client()
        .delete(format!("{base_url}/api/tweets/{}/like", Uuid::now_v7()))
        .json(&json!({ "user_id": Uuid::now_v7() }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}
