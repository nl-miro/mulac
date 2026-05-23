mod utils;

use reqwest::Client;
use serde_json::json;
use utils::start_test_app;
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread")]
async fn list_todos_returns_all_todos() {
    let (base_url, _pool, _guard) = start_test_app().await;
    let client = Client::new();

    client
        .post(format!("{base_url}/api/todos"))
        .json(&json!({"title": "Todo 1"}))
        .send()
        .await
        .unwrap();

    client
        .post(format!("{base_url}/api/todos"))
        .json(&json!({"title": "Todo 2"}))
        .send()
        .await
        .unwrap();

    let response = client
        .get(format!("{base_url}/api/todos"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body = response.json::<serde_json::Value>().await.unwrap();
    let todos = body["items"].as_array().unwrap();
    assert_eq!(todos.len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn list_todos_filters_by_status() {
    let (base_url, _pool, _guard) = start_test_app().await;
    let client = Client::new();

    let todo_id: Uuid = client
        .post(format!("{base_url}/api/todos"))
        .json(&json!({"title": "Task"}))
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

    client
        .post(format!("{base_url}/api/todos/{todo_id}/complete"))
        .send()
        .await
        .unwrap();

    let active_response = client
        .get(format!("{base_url}/api/todos?status=active"))
        .send()
        .await
        .unwrap();
    let active_body = active_response.json::<serde_json::Value>().await.unwrap();
    let active = active_body["items"].as_array().unwrap();
    assert_eq!(active.len(), 0);

    let completed_response = client
        .get(format!("{base_url}/api/todos?status=completed"))
        .send()
        .await
        .unwrap();
    let completed_body = completed_response
        .json::<serde_json::Value>()
        .await
        .unwrap();
    let completed = completed_body["items"].as_array().unwrap();
    assert_eq!(completed.len(), 1);
}
