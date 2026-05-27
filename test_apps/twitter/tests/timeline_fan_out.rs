mod utils;

use serde_json::json;
use utils::{
    STATUS_COMPLETED,
    assert_command_completed,
    assert_ok_response,
    fetch_command_entries,
    fetch_timelines_for_user,
    retry_fan_out,
    start_test_app, //
};
use uuid::Uuid;

async fn follow(base_url: &str, follower: Uuid, following: Uuid) {
    let resp = utils::client()
        .post(format!("{base_url}/api/users/follow"))
        .json(&json!({ "follower_id": follower, "following_id": following }))
        .send()
        .await
        .unwrap();
    assert_ok_response!(resp);
}

async fn post_tweet(base_url: &str, author_id: Uuid, content: &str) -> Uuid {
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

fn assert_fan_out_commands(pool: &utils::DbPool, expected_count: usize) {
    let cmds: Vec<_> = fetch_command_entries(pool)
        .into_iter()
        .filter(|c| c.command_type == "FanOutTweet")
        .collect();
    assert_eq!(cmds.len(), expected_count);
    assert!(cmds.iter().all(|c| c.status == STATUS_COMPLETED));
    assert!(cmds.iter().all(|c| c.extra_info.is_none()));
}

#[tokio::test(flavor = "multi_thread")]
async fn tweet_posted_fans_out_to_followers() {
    let (base_url, pool, _guard) = start_test_app().await;
    let author = Uuid::now_v7();
    let follower1 = Uuid::now_v7();
    let follower2 = Uuid::now_v7();

    follow(&base_url, follower1, author).await;
    follow(&base_url, follower2, author).await;

    // Give the fan-out event subscriber a moment to process.
    let tweet_id = post_tweet(&base_url, author, "hello followers").await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let tl1 = fetch_timelines_for_user(&pool, follower1);
    let tl2 = fetch_timelines_for_user(&pool, follower2);
    assert!(
        tl1.iter().any(|r| r.tweet_id == tweet_id),
        "follower1 timeline should have tweet"
    );
    assert!(
        tl2.iter().any(|r| r.tweet_id == tweet_id),
        "follower2 timeline should have tweet"
    );

    assert_command_completed(&pool, "FanOutTweet");
}

#[tokio::test(flavor = "multi_thread")]
async fn fan_out_is_idempotent() {
    let (base_url, pool, _guard) = start_test_app().await;
    let author = Uuid::now_v7();
    let follower = Uuid::now_v7();

    follow(&base_url, follower, author).await;
    let tweet_id = post_tweet(&base_url, author, "idempotent test").await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    retry_fan_out(&pool, tweet_id, author);

    let rows = fetch_timelines_for_user(&pool, follower);
    let count = rows.iter().filter(|r| r.tweet_id == tweet_id).count();
    assert_eq!(count, 1, "retry should not produce duplicate timeline rows");

    assert_fan_out_commands(&pool, 2);
}
