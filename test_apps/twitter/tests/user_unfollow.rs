mod utils;

use serde_json::json;
use utils::{
    assert_ok_response,
    fetch_event_entries,
    fetch_follows,
    fetch_outbox,
    start_test_app, //
};
use uuid::Uuid;

async fn follow_user(base_url: &str, follower: Uuid, following: Uuid) -> reqwest::Response {
    utils::client()
        .post(format!("{base_url}/api/users/follow"))
        .json(&json!({ "follower_id": follower, "following_id": following }))
        .send()
        .await
        .unwrap()
}

async fn unfollow_user(base_url: &str, follower: Uuid, following: Uuid) -> reqwest::Response {
    utils::client()
        .post(format!("{base_url}/api/users/unfollow"))
        .json(&json!({ "follower_id": follower, "following_id": following }))
        .send()
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn unfollow_existing_relation_emits_event() {
    let (base_url, pool, _guard) = start_test_app().await;
    let follower = Uuid::now_v7();
    let following = Uuid::now_v7();

    // Create follow first.
    follow_user(&base_url, follower, following).await;

    let resp = unfollow_user(&base_url, follower, following).await;
    assert_ok_response!(resp);

    // Follow row removed.
    assert!(fetch_follows(&pool).is_empty());

    // Outbox has both UserFollowed and UserUnfollowed.
    let outbox = fetch_outbox(&pool);
    assert!(outbox.iter().any(|r| r.event_type == "UserUnfollowed"));
}

#[tokio::test(flavor = "multi_thread")]
async fn unfollow_absent_relation_is_noop_200() {
    let (base_url, pool, _guard) = start_test_app().await;
    let resp = unfollow_user(&base_url, Uuid::now_v7(), Uuid::now_v7()).await;
    assert_ok_response!(resp);

    // No events emitted.
    let events = fetch_event_entries(&pool);
    assert!(events.iter().all(|e| e.event_type != "UserUnfollowed"));
    assert!(fetch_outbox(&pool).is_empty());
}
