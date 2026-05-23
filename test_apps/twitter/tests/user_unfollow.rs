mod utils;

use serde_json::json;
use utils::{fetch_event_entries, fetch_follows, fetch_outbox, start_test_app};
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread")]
async fn unfollow_existing_relation_emits_event() {
    let (base_url, pool, _guard) = start_test_app().await;
    let client = utils::client();
    let follower = Uuid::now_v7();
    let following = Uuid::now_v7();

    // Create follow first.
    client
        .post(format!("{base_url}/api/users/follow"))
        .json(&json!({ "follower_id": follower, "following_id": following }))
        .send()
        .await
        .unwrap();

    let resp = client
        .post(format!("{base_url}/api/users/unfollow"))
        .json(&json!({ "follower_id": follower, "following_id": following }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Follow row removed.
    assert!(fetch_follows(&pool).is_empty());

    // Outbox has both UserFollowed and UserUnfollowed.
    let outbox = fetch_outbox(&pool);
    assert!(outbox.iter().any(|r| r.event_type == "UserUnfollowed"));
}

#[tokio::test(flavor = "multi_thread")]
async fn unfollow_absent_relation_is_noop_200() {
    let (base_url, pool, _guard) = start_test_app().await;
    let resp = utils::client()
        .post(format!("{base_url}/api/users/unfollow"))
        .json(&json!({ "follower_id": Uuid::now_v7(), "following_id": Uuid::now_v7() }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // No events emitted.
    let events = fetch_event_entries(&pool);
    assert!(events.iter().all(|e| e.event_type != "UserUnfollowed"));
    assert!(fetch_outbox(&pool).is_empty());
}
