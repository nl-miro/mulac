mod utils;

use serde_json::json;
use utils::{
    assert_no_content_response, assert_not_found_response, assert_ok_response, fetch_event_entries,
    fetch_likes, fetch_outbox, start_test_app,
};
use uuid::Uuid;

async fn create_tweet(base_url: &str) -> Uuid {
    let resp = utils::client()
        .post(format!("{base_url}/api/tweets"))
        .json(&json!({ "author_id": Uuid::now_v7(), "content": "likeable" }))
        .send()
        .await
        .unwrap();
    assert_ok_response!(resp);
    resp.json::<serde_json::Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap()
}

async fn like_tweet(base_url: &str, tweet_id: Uuid, user_id: Uuid) -> reqwest::Response {
    utils::client()
        .post(format!("{base_url}/api/tweets/{tweet_id}/like"))
        .json(&json!({ "user_id": user_id }))
        .send()
        .await
        .unwrap()
}

async fn unlike_tweet(base_url: &str, tweet_id: Uuid, user_id: Uuid) -> reqwest::Response {
    utils::client()
        .delete(format!("{base_url}/api/tweets/{tweet_id}/like"))
        .json(&json!({ "user_id": user_id }))
        .send()
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn unlike_existing_like_returns_204_with_event() {
    let (base_url, pool, _guard) = start_test_app().await;
    let tweet_id = create_tweet(&base_url).await;
    let user = Uuid::now_v7();

    like_tweet(&base_url, tweet_id, user).await;
    let resp = unlike_tweet(&base_url, tweet_id, user).await;
    assert_no_content_response!(resp);

    assert!(fetch_likes(&pool).is_empty());
    let outbox = fetch_outbox(&pool);
    assert!(outbox.iter().any(|r| r.event_type == "TweetUnliked"));
}

#[tokio::test(flavor = "multi_thread")]
async fn unlike_absent_like_returns_204_no_event() {
    let (base_url, pool, _guard) = start_test_app().await;
    let tweet_id = create_tweet(&base_url).await;

    let resp = unlike_tweet(&base_url, tweet_id, Uuid::now_v7()).await;
    assert_no_content_response!(resp);

    let events = fetch_event_entries(&pool);
    assert!(events.iter().all(|e| e.event_type != "TweetUnliked"));
}

#[tokio::test(flavor = "multi_thread")]
async fn unlike_missing_tweet_returns_404() {
    let (base_url, _pool, _guard) = start_test_app().await;
    let resp = unlike_tweet(&base_url, Uuid::now_v7(), Uuid::now_v7()).await;
    assert_not_found_response!(resp);
}
