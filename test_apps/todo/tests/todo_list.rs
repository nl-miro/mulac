mod utils;
use serde_json::json;
use utils::{assert_ok_response, start_test_app};
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

async fn complete_todo(base_url: &str, todo_id: Uuid) {
    let response = utils::client()
        .post(format!("{base_url}/api/todos/{todo_id}/complete"))
        .send()
        .await
        .unwrap();
    assert_ok_response!(response);
}

#[tokio::test(flavor = "multi_thread")]
async fn list_todos_returns_all_todos() {
    let (base_url, _pool, _guard) = start_test_app().await;

    create_todo(&base_url, "Todo 1").await;
    create_todo(&base_url, "Todo 2").await;

    let response = utils::client()
        .get(format!("{base_url}/api/todos"))
        .send()
        .await
        .unwrap();

    assert_ok_response!(response);
    let body = response.json::<serde_json::Value>().await.unwrap();
    let todos = body["items"].as_array().unwrap();
    assert_eq!(todos.len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn list_todos_filters_by_status() {
    let (base_url, _pool, _guard) = start_test_app().await;

    let todo_id = create_todo(&base_url, "Task").await;
    complete_todo(&base_url, todo_id).await;

    let active_response = utils::client()
        .get(format!("{base_url}/api/todos?status=active"))
        .send()
        .await
        .unwrap();
    let active_body = active_response.json::<serde_json::Value>().await.unwrap();
    let active = active_body["items"].as_array().unwrap();
    assert_eq!(active.len(), 0);

    let completed_response = utils::client()
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
