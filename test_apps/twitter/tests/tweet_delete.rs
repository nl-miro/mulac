mod utils;

use serde_json::json;
use utils::{
    assert_no_content_response,
    assert_not_found_response,
    assert_ok_response,
    assert_outbox_pending,
    fetch_event_entries,
    fetch_outbox,
    fetch_tweet_by_id,
    start_test_app, //
};
use uuid::Uuid;

async fn create_tweet(base_url: &str, author_id: Uuid, content: &str) -> Uuid {
    let resp = utils::client()
        .post(format!("{base_url}/api/tweets"))
        .json(&json!({ "author_id": author_id, "content": content }))
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

async fn delete_tweet(base_url: &str, tweet_id: Uuid, author_id: Uuid) -> reqwest::Response {
    utils::client()
        .delete(format!("{base_url}/api/tweets/{tweet_id}"))
        .json(&json!({ "author_id": author_id }))
        .send()
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_tweet_success() {
    let (base_url, pool, _guard) = start_test_app().await;
    let author_id = Uuid::now_v7();
    let tweet_id = create_tweet(&base_url, author_id, "to be deleted").await;

    let resp = delete_tweet(&base_url, tweet_id, author_id).await;
    assert_no_content_response!(resp);

    let row = fetch_tweet_by_id(&pool, tweet_id).unwrap();
    assert!(row.deleted_at.is_some(), "tweet should have deleted_at set");

    assert_outbox_pending(&pool, "TweetDeleted");
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_tweet_not_found_returns_404() {
    let (base_url, pool, _guard) = start_test_app().await;
    let resp = delete_tweet(&base_url, Uuid::now_v7(), Uuid::now_v7()).await;
    assert_not_found_response!(resp);

    // No events or outbox rows for missing tweet.
    assert!(fetch_event_entries(&pool).is_empty());
    assert!(fetch_outbox(&pool).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_tweet_wrong_author_returns_404() {
    let (base_url, pool, _guard) = start_test_app().await;
    let author_id = Uuid::now_v7();
    let tweet_id = create_tweet(&base_url, author_id, "mine").await;

    // Different author cannot delete.
    let resp = delete_tweet(&base_url, tweet_id, Uuid::now_v7()).await;
    assert_not_found_response!(resp);

    // Tweet is not soft-deleted.
    let row = fetch_tweet_by_id(&pool, tweet_id).unwrap();
    assert!(row.deleted_at.is_none());
}
