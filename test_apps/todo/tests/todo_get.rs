mod utils;
use serde_json::json;
use utils::{assert_not_found_response, assert_ok_response, start_test_app};
use uuid::Uuid;

async fn create_todo(base_url: &str, title: &str) -> Uuid {
    utils::client()
        .post(format!("{base_url}/api/todos"))
        .json(&json!({"title": title}))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn get_todo_returns_single_todo() {
    let (base_url, _pool, _guard) = start_test_app().await;

    let todo_id = create_todo(&base_url, "Single Task").await;

    let response = utils::client().get(format!("{base_url}/api/todos/{todo_id}")).send().await.unwrap();

    assert_ok_response!(response);
    let body = response.json::<serde_json::Value>().await.unwrap();
    assert_eq!(body["id"], todo_id.to_string());
    assert_eq!(body["title"], "Single Task");
}

#[tokio::test(flavor = "multi_thread")]
async fn get_nonexistent_todo_returns_404() {
    let (base_url, _pool, _guard) = start_test_app().await;
    let nonexistent_id = Uuid::now_v7();

    let response = utils::client().get(format!("{base_url}/api/todos/{nonexistent_id}")).send().await.unwrap();

    assert_not_found_response!(response);
}
