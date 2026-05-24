mod utils;

use serde_json::json;
use utils::{
    assert_bad_request_response,
    assert_event_completed,
    assert_ok_response,
    assert_outbox_pending,
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

#[tokio::test(flavor = "multi_thread")]
async fn follow_user_success() {
    let (base_url, pool, _guard) = start_test_app().await;
    let follower = Uuid::now_v7();
    let following = Uuid::now_v7();

    let resp = follow_user(&base_url, follower, following).await;
    assert_ok_response!(resp);

    let follows = fetch_follows(&pool);
    assert_eq!(follows.len(), 1);
    assert_eq!(follows[0].follower_id, follower);
    assert_eq!(follows[0].following_id, following);

    assert_outbox_pending(&pool, "UserFollowed");
}

#[tokio::test(flavor = "multi_thread")]
async fn follow_self_returns_400() {
    let (base_url, _pool, _guard) = start_test_app().await;
    let user = Uuid::now_v7();

    let resp = follow_user(&base_url, user, user).await;
    assert_bad_request_response!(resp);
}

#[tokio::test(flavor = "multi_thread")]
async fn follow_duplicate_is_noop_200() {
    let (base_url, pool, _guard) = start_test_app().await;
    let follower = Uuid::now_v7();
    let following = Uuid::now_v7();

    // First follow.
    follow_user(&base_url, follower, following).await;

    // Second follow — should be 200 no-op.
    let resp = follow_user(&base_url, follower, following).await;
    assert_ok_response!(resp);

    // Only one follow row.
    assert_eq!(fetch_follows(&pool).len(), 1);
    // Only one outbox row (from first follow only).
    assert_eq!(fetch_outbox(&pool).len(), 1);
    // Only one event entry.
    assert_event_completed(&pool, "UserFollowed");
}
