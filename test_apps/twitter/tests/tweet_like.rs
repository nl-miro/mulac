mod utils;

use serde_json::json;
use utils::{
    assert_not_found_response, assert_ok_response, fetch_event_entries, fetch_likes, fetch_outbox,
    start_test_app,
};
use uuid::Uuid;

async fn create_tweet(base_url: &str, author_id: Uuid) -> Uuid {
    let resp = utils::client()
        .post(format!("{base_url}/api/tweets"))
        .json(&json!({ "author_id": author_id, "content": "likeable" }))
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

#[tokio::test(flavor = "multi_thread")]
async fn like_tweet_success() {
    let (base_url, pool, _guard) = start_test_app().await;
    let author = Uuid::now_v7();
    let tweet_id = create_tweet(&base_url, author).await;
    let user = Uuid::now_v7();

    let resp = like_tweet(&base_url, tweet_id, user).await;
    assert_ok_response!(resp);

    let likes = fetch_likes(&pool);
    assert_eq!(likes.len(), 1);
    assert_eq!(likes[0].tweet_id, tweet_id);
    assert_eq!(likes[0].user_id, user);

    let outbox = fetch_outbox(&pool);
    let like_events: Vec<_> = outbox
        .iter()
        .filter(|r| r.event_type == "TweetLiked")
        .collect();
    assert_eq!(like_events.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn like_missing_tweet_returns_404() {
    let (base_url, _pool, _guard) = start_test_app().await;
    let resp = like_tweet(&base_url, Uuid::now_v7(), Uuid::now_v7()).await;
    assert_not_found_response!(resp);
}

#[tokio::test(flavor = "multi_thread")]
async fn like_duplicate_is_noop_200() {
    let (base_url, pool, _guard) = start_test_app().await;
    let tweet_id = create_tweet(&base_url, Uuid::now_v7()).await;
    let user = Uuid::now_v7();

    like_tweet(&base_url, tweet_id, user).await;
    let resp = like_tweet(&base_url, tweet_id, user).await;
    assert_ok_response!(resp);

    assert_eq!(fetch_likes(&pool).len(), 1);
    let events: Vec<_> = fetch_event_entries(&pool)
        .into_iter()
        .filter(|e| e.event_type == "TweetLiked")
        .collect();
    assert_eq!(events.len(), 1); // only one event from the first like
}
