//! Outbox domain models.
//!
//! Domain structs are added incrementally according to the implementation checklist.

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExtraInfo {
    errors: Vec<String>,
}

impl ExtraInfo {
    pub fn with_errors(errors: Vec<String>) -> Self {
        Self { errors }
    }

    pub fn with_error(error: impl Into<String>) -> Self {
        Self::with_errors(vec![error.into()])
    }

    pub fn add_error(&mut self, error: impl Into<String>) {
        self.errors.push(error.into());
    }

    pub fn errors(&self) -> &[String] {
        &self.errors
    }
}

pub fn append_error(extra_info: Option<ExtraInfo>, error: impl Into<String>) -> ExtraInfo {
    let mut extra_info = extra_info.unwrap_or_default();
    extra_info.add_error(error);
    extra_info
}

/// Persistent outbox status codes.
///
/// Values are stored as `Int4` in the database and must remain stable. The gaps
/// at `1`, `3`, and `6` are intentionally reserved for compatibility with
/// existing data and must not be reused without a schema/data migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum OutboxStatus {
    /// Stored and awaiting an outbox consumer.
    Received = 0,
    /// Claimed by a consumer; blocked from re-reservation until released.
    Reserved = 2,
    /// Publication attempt failed; scheduled for retry.
    Failed = 4,
    /// Broker accepted the outbound message.
    Completed = 5,
    /// Retry limit exhausted or non-retriable failure occurred.
    Dead = 7,
    /// Archived.
    Archive = 8,
}

#[derive(Debug, Error)]
#[error("unknown outbox status: {0}")]
pub struct UnknownOutboxStatus(pub i32);

impl TryFrom<i32> for OutboxStatus {
    type Error = UnknownOutboxStatus;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Received),
            2 => Ok(Self::Reserved),
            4 => Ok(Self::Failed),
            5 => Ok(Self::Completed),
            7 => Ok(Self::Dead),
            8 => Ok(Self::Archive),
            _ => Err(UnknownOutboxStatus(value)),
        }
    }
}

impl From<OutboxStatus> for i32 {
    fn from(status: OutboxStatus) -> i32 {
        status as i32
    }
}

impl OutboxStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Received => "received",
            Self::Reserved => "reserved",
            Self::Failed => "failed",
            Self::Completed => "completed",
            Self::Dead => "dead",
            Self::Archive => "archive",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OutboxEntryMetadata {
    pub event_id: uuid::Uuid,
    pub message_id: uuid::Uuid,
    pub correlation_id: Option<uuid::Uuid>,
    pub causation_id: Option<uuid::Uuid>,
    pub event_type: String,
    pub routing_key: String,
    pub source: Option<String>,
    pub content_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OutboxEntry {
    pub id: uuid::Uuid,
    pub status: OutboxStatus,
    pub payload: String,
    pub meta: OutboxEntryMetadata,
    pub scheduled_at: chrono::DateTime<chrono::Utc>,
    pub attempts: i32,
    pub reservation_id: Option<uuid::Uuid>,
    pub reserved_at: Option<chrono::DateTime<chrono::Utc>>,
    pub received_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub processed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_error: Option<String>,
    pub extra_info: Option<ExtraInfo>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewOutboxEntry {
    pub id: uuid::Uuid,
    pub payload: String,
    pub meta: OutboxEntryMetadata,
    pub scheduled_at: chrono::DateTime<chrono::Utc>,
    pub received_at: chrono::DateTime<chrono::Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_round_trips_through_i32() {
        let cases = [
            (0, OutboxStatus::Received),
            (2, OutboxStatus::Reserved),
            (4, OutboxStatus::Failed),
            (5, OutboxStatus::Completed),
            (7, OutboxStatus::Dead),
            (8, OutboxStatus::Archive),
        ];

        for (value, expected) in cases {
            let status = OutboxStatus::try_from(value).expect("valid status");
            assert_eq!(status, expected);
            assert_eq!(i32::from(status), value);
        }
    }

    #[test]
    fn reserved_gaps_are_unknown() {
        for value in [1, 3, 6] {
            assert!(
                OutboxStatus::try_from(value).is_err(),
                "value {value} should be unknown"
            );
        }
    }

    #[test]
    fn append_error_accumulates_errors() {
        let extra_info = append_error(Some(ExtraInfo::with_error("first")), "second");

        assert_eq!(
            extra_info.errors(),
            &["first".to_string(), "second".to_string()]
        );
    }

    #[test]
    fn with_errors_preserves_order() {
        let extra_info = ExtraInfo::with_errors(vec!["first".into(), "second".into()]);

        assert_eq!(
            extra_info.errors(),
            &["first".to_string(), "second".to_string()]
        );
    }

    #[test]
    fn arbitrary_unknown_value_returns_error() {
        assert!(OutboxStatus::try_from(99).is_err());
        assert!(OutboxStatus::try_from(-1).is_err());
    }

    #[test]
    fn as_str_matches_variant() {
        assert_eq!(OutboxStatus::Received.as_str(), "received");
        assert_eq!(OutboxStatus::Reserved.as_str(), "reserved");
        assert_eq!(OutboxStatus::Failed.as_str(), "failed");
        assert_eq!(OutboxStatus::Completed.as_str(), "completed");
        assert_eq!(OutboxStatus::Dead.as_str(), "dead");
        assert_eq!(OutboxStatus::Archive.as_str(), "archive");
    }
}
