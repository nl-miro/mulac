use chrono::{DateTime, Utc};
use poem_openapi::{Enum, Object};
use serde::{Deserialize, Serialize};
use std::sync::{OnceLock, RwLock};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize, Enum, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[oai(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum TodoStatus {
    Active,
    Completed,
    Archived,
}

impl TodoStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Completed => "completed",
            Self::Archived => "archived",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Object)]
pub struct TodoEntry {
    pub id: Uuid,
    pub title: String,
    pub description: Option<String>,
    pub status: TodoStatus,
    pub due_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Object)]
pub struct TodoList {
    pub items: Vec<TodoEntry>,
}

static FIXED_NOW: OnceLock<RwLock<Option<DateTime<Utc>>>> = OnceLock::new();

fn lock() -> &'static RwLock<Option<DateTime<Utc>>> {
    FIXED_NOW.get_or_init(|| RwLock::new(None))
}

pub struct Clock;

impl Clock {
    pub fn now() -> DateTime<Utc> {
        lock().read().unwrap().unwrap_or_else(Utc::now)
    }

    pub fn fix(at: DateTime<Utc>) {
        *lock().write().unwrap() = Some(at);
    }

    pub fn reset() {
        *lock().write().unwrap() = None;
    }
}
