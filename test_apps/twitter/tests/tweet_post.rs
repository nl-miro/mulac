mod utils;

use serde::Deserialize;
use serde_json::json;
use utils::{
    assert_bad_request_response,
    assert_command_completed,
    assert_conflict_response,
    assert_event_completed,
    assert_ok_response,
    assert_outbox_pending,
    fetch_outbox,
    fetch_tweet_by_id,
    fetch_tweets,
    start_test_app, //
};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
struct PostTweetResponse {
    id: Uuid,
    author_id: Uuid,
    content: String,
}

#[tokio::test(flavor = "multi_thread")]
async fn post_tweet_success() {
    let (base_url, pool, _guard) = start_test_app().await;
    let author_id = Uuid::now_v7();

    let resp = post_tweet(&base_url, author_id, "Hello world!").await;

    assert_ok_response!(resp);

    let body: PostTweetResponse = resp.json().await.unwrap();

    assert_eq!(body.author_id, author_id);
    assert_eq!(body.content, "Hello world!");

    let tweet_id = body.id;
    let row = fetch_tweet_by_id(&pool, tweet_id).expect("tweet row should exist");
    assert_eq!(row.author_id, author_id);
    assert_eq!(row.content, "Hello world!");
    assert!(row.deleted_at.is_none());

    assert_command_completed(&pool, "PostTweet");
    assert_event_completed(&pool, "TweetPosted");

    assert_outbox_pending(&pool, "TweetPosted");

    assert_outbox_tweet_posted_payload(&pool, tweet_id);
}

fn assert_outbox_tweet_posted_payload(pool: &utils::DbPool, tweet_id: Uuid) {
    let outbox = fetch_outbox(pool);
    let payload: serde_json::Value = serde_json::from_str(&outbox[0].payload).unwrap();

    assert_eq!(payload["type"], "TweetPosted");
    assert_eq!(
        payload["payload"]["tweet_id"].as_str().unwrap(),
        tweet_id.to_string()
    );
}

async fn post_tweet(base_url: &str, author_id: Uuid, content: &str) -> reqwest::Response {
    utils::client()
        .post(format!("{base_url}/api/tweets"))
        .json(&json!({ "author_id": author_id, "content": content }))
        .send()
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn post_tweet_blank_content_returns_400() {
    let (base_url, _pool, _guard) = start_test_app().await;
    let resp = post_tweet(&base_url, Uuid::now_v7(), "   ").await;

    assert_bad_request_response!(resp);
}

#[tokio::test(flavor = "multi_thread")]
async fn post_tweet_too_long_content_returns_400() {
    let (base_url, _pool, _guard) = start_test_app().await;
    let content = &"x".repeat(281); // 281 ASCII chars, > 280 bytes
    let resp = post_tweet(&base_url, Uuid::now_v7(), content).await;

    assert_bad_request_response!(resp);
}

#[tokio::test(flavor = "multi_thread")]
async fn post_tweet_unicode_content_within_limit_succeeds() {
    let (base_url, _pool, _guard) = start_test_app().await;
    let content = "é".repeat(280); // 280 Unicode scalars, > 280 bytes
    let resp = post_tweet(&base_url, Uuid::now_v7(), &content).await;

    assert_ok_response!(resp);
}

// #[tokio::test(flavor = "multi_thread")]
// async fn post_tweet_duplicate_tweet_id_returns_409() {
//     let (base_url, pool, _guard) = start_test_app().await;
//     let client = utils::client();
//     let author_id = Uuid::now_v7();
//     let tweet_id = Uuid::now_v7();
//
//     let first = post_tweet_command(&client, &base_url, tweet_id, author_id, "first").await;
//     assert_ok_response!(first);
//
//     let second = post_tweet_command(&client, &base_url, tweet_id, author_id, "second").await;
//     assert_conflict_response!(second);
//
//     let tweets = fetch_tweets(&pool);
//     assert_eq!(tweets.len(), 1);
//     assert_eq!(tweets[0].id, tweet_id);
//
//     assert_event_completed(&pool, "TweetPosted");
//     assert_outbox_pending(&pool, "TweetPosted");
// }

async fn post_tweet_command(
    client: &reqwest::Client,
    base_url: &str,
    tweet_id: Uuid,
    author_id: Uuid,
    content: &str,
) -> reqwest::Response {
    client
        .post(format!("{base_url}/api/messages/commands"))
        .json(&json!({
            "id": Uuid::now_v7(),
            "command": {
                "type": "PostTweet",
                "tweet_id": tweet_id,
                "author_id": author_id,
                "content": content,
            }
        }))
        .send()
        .await
        .unwrap()
}
