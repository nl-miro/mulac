mod utils;

use serde_json::json;
use utils::{fetch_event_entries, fetch_likes, fetch_outbox, start_test_app};
use uuid::Uuid;

async fn create_tweet(base_url: &str, author_id: Uuid) -> Uuid {
    let resp = utils::client()
        .post(format!("{base_url}/api/tweets"))
        .json(&json!({ "author_id": author_id, "content": "likeable" }))
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
async fn like_tweet_success() {
    let (base_url, pool, _guard) = start_test_app().await;
    let author = Uuid::now_v7();
    let tweet_id = create_tweet(&base_url, author).await;
    let user = Uuid::now_v7();

    let resp = utils::client()
        .post(format!("{base_url}/api/tweets/{tweet_id}/like"))
        .json(&json!({ "user_id": user }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

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
    let resp = utils::client()
        .post(format!("{base_url}/api/tweets/{}/like", Uuid::now_v7()))
        .json(&json!({ "user_id": Uuid::now_v7() }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

#[tokio::test(flavor = "multi_thread")]
async fn like_duplicate_is_noop_200() {
    let (base_url, pool, _guard) = start_test_app().await;
    let client = utils::client();
    let tweet_id = create_tweet(&base_url, Uuid::now_v7()).await;
    let user = Uuid::now_v7();

    client
        .post(format!("{base_url}/api/tweets/{tweet_id}/like"))
        .json(&json!({ "user_id": user }))
        .send()
        .await
        .unwrap();
    let resp = client
        .post(format!("{base_url}/api/tweets/{tweet_id}/like"))
        .json(&json!({ "user_id": user }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    assert_eq!(fetch_likes(&pool).len(), 1);
    let events: Vec<_> = fetch_event_entries(&pool)
        .into_iter()
        .filter(|e| e.event_type == "TweetLiked")
        .collect();
    assert_eq!(events.len(), 1); // only one event from the first like
}
