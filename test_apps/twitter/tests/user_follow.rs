mod utils;

use serde_json::json;
use utils::{fetch_event_entries, fetch_follows, fetch_outbox, start_test_app};
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread")]
async fn follow_user_success() {
    let (base_url, pool, _guard) = start_test_app().await;
    let follower = Uuid::now_v7();
    let following = Uuid::now_v7();

    let resp = utils::client()
        .post(format!("{base_url}/api/users/follow"))
        .json(&json!({ "follower_id": follower, "following_id": following }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    let follows = fetch_follows(&pool);
    assert_eq!(follows.len(), 1);
    assert_eq!(follows[0].follower_id, follower);
    assert_eq!(follows[0].following_id, following);

    let outbox = fetch_outbox(&pool);
    assert_eq!(outbox.len(), 1);
    assert_eq!(outbox[0].event_type, "UserFollowed");
}

#[tokio::test(flavor = "multi_thread")]
async fn follow_self_returns_400() {
    let (base_url, _pool, _guard) = start_test_app().await;
    let user = Uuid::now_v7();
    let resp = utils::client()
        .post(format!("{base_url}/api/users/follow"))
        .json(&json!({ "follower_id": user, "following_id": user }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

#[tokio::test(flavor = "multi_thread")]
async fn follow_duplicate_is_noop_200() {
    let (base_url, pool, _guard) = start_test_app().await;
    let client = utils::client();
    let follower = Uuid::now_v7();
    let following = Uuid::now_v7();

    // First follow.
    client
        .post(format!("{base_url}/api/users/follow"))
        .json(&json!({ "follower_id": follower, "following_id": following }))
        .send()
        .await
        .unwrap();

    // Second follow — should be 200 no-op.
    let resp = client
        .post(format!("{base_url}/api/users/follow"))
        .json(&json!({ "follower_id": follower, "following_id": following }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Only one follow row.
    assert_eq!(fetch_follows(&pool).len(), 1);
    // Only one outbox row (from first follow only).
    assert_eq!(fetch_outbox(&pool).len(), 1);
    // Only one event entry.
    let events = fetch_event_entries(&pool);
    let follow_events: Vec<_> = events
        .iter()
        .filter(|e| e.event_type == "UserFollowed")
        .collect();
    assert_eq!(follow_events.len(), 1);
}
