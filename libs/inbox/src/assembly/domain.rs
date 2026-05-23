use chrono::{DateTime, Utc};
use thiserror::Error;
use uuid::Uuid;

/// A stored inbox message as it exists in the database.
///
/// Carries the full lifecycle state — status, retry counter, reservation
/// ownership — so consumers can make processing decisions without additional
/// queries.
#[derive(Debug)]
pub struct InboxMessage {
    pub id: Uuid,
    pub payload: String,
    pub status: InboxStatus,
    /// Next time this message becomes eligible for reservation.
    /// Advances on each failed attempt according to the retry backoff policy.
    pub scheduled_at: DateTime<Utc>,
    /// Number of times this message has been reserved for processing.
    ///
    /// Incremented on every reservation, including reservations that did not
    /// result in an explicit `completed` or `failed` call (e.g. worker crash
    /// or shutdown). A message is marked [`Dead`] once this reaches the
    /// configured limit regardless of how many reservations actually ran to
    /// completion.
    ///
    /// [`Dead`]: InboxStatus::Dead
    pub attempts: i32,
    /// Identifies the current reservation holder. `None` when not reserved.
    pub reservation_id: Option<Uuid>,
    pub reserved_at: Option<DateTime<Utc>>,
}

/// Persistent inbox status codes.
///
/// Values are stored as `Int4` in the database and must remain stable. The gaps
/// at `1`, `3`, and `6` are intentionally reserved for compatibility with
/// existing data and must not be reused without a schema/data migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum InboxStatus {
    /// Stored and awaiting a consumer.
    Received = 0,
    /// Claimed by a consumer; blocked from re-reservation until released.
    Reserved = 2,
    /// Processing attempt failed; scheduled for retry.
    Failed = 4,
    /// Processed successfully.
    Completed = 5,
    /// Retry limit exhausted; will not be retried automatically.
    Dead = 7,
    /// Archived.
    Archive = 8,
}

impl TryFrom<i32> for InboxStatus {
    type Error = UnknownInboxStatus;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Received),
            2 => Ok(Self::Reserved),
            4 => Ok(Self::Failed),
            5 => Ok(Self::Completed),
            7 => Ok(Self::Dead),
            8 => Ok(Self::Archive),
            _ => Err(UnknownInboxStatus(value)),
        }
    }
}

impl InboxStatus {
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

impl From<InboxStatus> for i32 {
    fn from(status: InboxStatus) -> i32 {
        status as i32
    }
}

#[cfg(feature = "diesel")]
pub(crate) enum Criterion {
    StatusIn(Vec<InboxStatus>),
    ScheduledBeforeNow,
    MaxAttempts(i32),
    OrderByScheduledAtAsc,
}

#[derive(Debug, Error)]
#[error("unknown inbox status: {0}")]
pub struct UnknownInboxStatus(pub i32);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_round_trips_through_i32() {
        let cases = [
            (0, InboxStatus::Received),
            (2, InboxStatus::Reserved),
            (4, InboxStatus::Failed),
            (5, InboxStatus::Completed),
            (7, InboxStatus::Dead),
            (8, InboxStatus::Archive),
        ];
        for (value, expected) in cases {
            let status = InboxStatus::try_from(value).expect("valid status");
            assert_eq!(status, expected);
            assert_eq!(i32::from(status), value);
        }
    }

    #[test]
    fn reserved_gaps_are_unknown() {
        for value in [1, 3, 6] {
            assert!(
                InboxStatus::try_from(value).is_err(),
                "value {value} should be unknown"
            );
        }
    }

    #[test]
    fn arbitrary_unknown_value_returns_error() {
        assert!(InboxStatus::try_from(99).is_err());
        assert!(InboxStatus::try_from(-1).is_err());
    }

    #[test]
    fn as_str_matches_variant() {
        assert_eq!(InboxStatus::Received.as_str(), "received");
        assert_eq!(InboxStatus::Reserved.as_str(), "reserved");
        assert_eq!(InboxStatus::Failed.as_str(), "failed");
        assert_eq!(InboxStatus::Completed.as_str(), "completed");
        assert_eq!(InboxStatus::Dead.as_str(), "dead");
        assert_eq!(InboxStatus::Archive.as_str(), "archive");
    }
}
