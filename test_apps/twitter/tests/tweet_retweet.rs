mod utils;

use serde_json::json;
use utils::{
    assert_not_found_response, assert_ok_response, assert_outbox_pending, fetch_tweet_by_id,
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
    assert_ok_response!(resp);
    resp.json::<serde_json::Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap()
}

async fn retweet(base_url: &str, original_id: Uuid, author_id: Uuid) -> reqwest::Response {
    utils::client()
        .post(format!("{base_url}/api/tweets/{original_id}/retweet"))
        .json(&json!({ "author_id": author_id }))
        .send()
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn retweet_success() {
    let (base_url, pool, _guard) = start_test_app().await;
    let original_author = Uuid::now_v7();
    let original_id = create_tweet(&base_url, original_author, "original").await;

    let rt_author = Uuid::now_v7();
    let resp = retweet(&base_url, original_id, rt_author).await;
    assert_ok_response!(resp);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(
        body["retweeted_from"].as_str().unwrap(),
        original_id.to_string()
    );

    let retweet_id: Uuid = body["id"].as_str().unwrap().parse().unwrap();
    let row = fetch_tweet_by_id(&pool, retweet_id).unwrap();
    assert_eq!(row.retweeted_from.unwrap(), original_id);

    assert_outbox_pending(&pool, "TweetRetweeted");
}

#[tokio::test(flavor = "multi_thread")]
async fn retweet_missing_original_returns_404() {
    let (base_url, _pool, _guard) = start_test_app().await;
    let resp = retweet(&base_url, Uuid::now_v7(), Uuid::now_v7()).await;
    assert_not_found_response!(resp);
}
