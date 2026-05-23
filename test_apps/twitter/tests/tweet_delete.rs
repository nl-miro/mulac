mod utils;

use serde_json::json;
use utils::{
    STATUS_COMPLETED, fetch_command_entries, fetch_event_entries, fetch_outbox, fetch_tweet_by_id,
    start_test_app,
};
use uuid::Uuid;

async fn create_tweet(base_url: &str, author_id: Uuid, content: &str) -> Uuid {
    let resp = utils::client()
        .post(format!("{base_url}/api/tweets"))
        .json(&json!({ "author_id": author_id, "content": content }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    resp.json::<serde_json::Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_tweet_success() {
    let (base_url, pool, _guard) = start_test_app().await;
    let author_id = Uuid::now_v7();
    let tweet_id = create_tweet(&base_url, author_id, "to be deleted").await;

    let resp = utils::client()
        .delete(format!("{base_url}/api/tweets/{tweet_id}"))
        .json(&json!({ "author_id": author_id }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 204);

    let row = fetch_tweet_by_id(&pool, tweet_id).unwrap();
    assert!(row.deleted_at.is_some(), "tweet should have deleted_at set");

    let outbox = fetch_outbox(&pool);
    let delete_events: Vec<_> = outbox
        .iter()
        .filter(|r| r.event_type == "TweetDeleted")
        .collect();
    assert_eq!(delete_events.len(), 1);
    assert_eq!(delete_events[0].status, "pending");
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_tweet_not_found_returns_404() {
    let (base_url, pool, _guard) = start_test_app().await;
    let resp = utils::client()
        .delete(format!("{base_url}/api/tweets/{}", Uuid::now_v7()))
        .json(&json!({ "author_id": Uuid::now_v7() }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);

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
    let resp = utils::client()
        .delete(format!("{base_url}/api/tweets/{tweet_id}"))
        .json(&json!({ "author_id": Uuid::now_v7() }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);

    // Tweet is not soft-deleted.
    let row = fetch_tweet_by_id(&pool, tweet_id).unwrap();
    assert!(row.deleted_at.is_none());
}
