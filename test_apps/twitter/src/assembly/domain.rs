use chrono::{DateTime, Utc};
use poem_openapi::{Object, Union};
use serde::{Deserialize, Serialize};
use std::sync::{OnceLock, RwLock};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, Object)]
pub struct TweetDto {
    pub id: Uuid,
    pub author_id: Uuid,
    pub content: String,
    pub created_at: DateTime<Utc>,
    pub retweeted_from: Option<Uuid>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Object)]
pub struct FollowDto {
    pub follower_id: Uuid,
    pub following_id: Uuid,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Object)]
pub struct LikeDto {
    pub user_id: Uuid,
    pub tweet_id: Uuid,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Object)]
pub struct DirectMessageDto {
    pub id: Uuid,
    pub sender_id: Uuid,
    pub recipient_id: Uuid,
    pub content: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Object, Default)]
pub struct NoEntityDto {}

#[derive(Debug, Clone, Serialize, Deserialize, Union)]
#[oai(discriminator_name = "type")]
#[serde(tag = "type", content = "entity")]
pub enum InboundEntity {
    Tweet(TweetDto),
    Follow(FollowDto),
    Like(LikeDto),
    DirectMessage(DirectMessageDto),
    NoEntity(NoEntityDto),
}

impl InboundEntity {
    pub fn no_entity() -> Self {
        Self::NoEntity(NoEntityDto::default())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Object)]
pub struct InboundResponse {
    pub message_id: Uuid,
    pub entity: InboundEntity,
}

static FIXED_NOW: OnceLock<RwLock<Option<DateTime<Utc>>>> = OnceLock::new();

fn clock_lock() -> &'static RwLock<Option<DateTime<Utc>>> {
    FIXED_NOW.get_or_init(|| RwLock::new(None))
}

pub struct Clock;

impl Clock {
    pub fn now() -> DateTime<Utc> {
        clock_lock().read().unwrap().unwrap_or_else(Utc::now)
    }

    pub fn fix(at: DateTime<Utc>) {
        *clock_lock().write().unwrap() = Some(at);
    }

    pub fn reset() {
        *clock_lock().write().unwrap() = None;
    }
}
