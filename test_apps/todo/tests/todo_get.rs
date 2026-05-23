mod utils;

use reqwest::Client;
use serde_json::json;
use utils::start_test_app;
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread")]
async fn get_todo_returns_single_todo() {
    let (base_url, _pool, _guard) = start_test_app().await;
    let client = Client::new();

    let todo_id: Uuid = client
        .post(format!("{base_url}/api/todos"))
        .json(&json!({"title": "Single Task"}))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();

    let response = client
        .get(format!("{base_url}/api/todos/{todo_id}"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body = response.json::<serde_json::Value>().await.unwrap();
    assert_eq!(body["id"], todo_id.to_string());
    assert_eq!(body["title"], "Single Task");
}

#[tokio::test(flavor = "multi_thread")]
async fn get_nonexistent_todo_returns_404() {
    let (base_url, _pool, _guard) = start_test_app().await;
    let client = Client::new();
    let nonexistent_id = Uuid::now_v7();

    let response = client
        .get(format!("{base_url}/api/todos/{nonexistent_id}"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 404);
}
