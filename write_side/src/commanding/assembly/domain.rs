use chrono::{DateTime, Utc};
use thiserror::Error;

/// Persistent command status codes.
///
/// Values are stored as `Int4` in the database and must remain stable. The gaps
/// at `1`, `3`, and `6` are intentionally reserved for compatibility with
/// existing data and must not be reused without a schema/data migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum CommandStatus {
    /// Stored and awaiting a consumer.
    Received = 0,
    /// Claimed by a consumer; blocked from re-reservation until released.
    Reserved = 2,
    /// Execution attempt failed; scheduled for retry.
    Failed = 4,
    /// Executed successfully and events handed off.
    Completed = 5,
    /// Retry limit exhausted; will not be retried automatically.
    Dead = 7,
    /// Archived.
    Archive = 8,
}

#[derive(Debug, Error)]
#[error("unknown command status: {0}")]
pub struct UnknownCommandStatus(pub i32);

impl TryFrom<i32> for CommandStatus {
    type Error = UnknownCommandStatus;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Received),
            2 => Ok(Self::Reserved),
            4 => Ok(Self::Failed),
            5 => Ok(Self::Completed),
            7 => Ok(Self::Dead),
            8 => Ok(Self::Archive),
            _ => Err(UnknownCommandStatus(value)),
        }
    }
}

impl From<CommandStatus> for i32 {
    fn from(status: CommandStatus) -> i32 {
        status as i32
    }
}

impl CommandStatus {
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

#[cfg(feature = "diesel")]
pub(crate) enum Criterion {
    StatusIn(Vec<CommandStatus>),
    ScheduledBeforeNow,
    MaxAttempts(i32),
    ReservedBefore(DateTime<Utc>),
    OrderByScheduledAtAsc,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_round_trips_through_i32() {
        let cases = [
            (0, CommandStatus::Received),
            (2, CommandStatus::Reserved),
            (4, CommandStatus::Failed),
            (5, CommandStatus::Completed),
            (7, CommandStatus::Dead),
            (8, CommandStatus::Archive),
        ];
        for (value, expected) in cases {
            let status = CommandStatus::try_from(value).expect("valid status");
            assert_eq!(status, expected);
            assert_eq!(i32::from(status), value);
        }
    }

    #[test]
    fn reserved_gaps_are_unknown() {
        for value in [1, 3, 6] {
            assert!(
                CommandStatus::try_from(value).is_err(),
                "value {value} should be unknown"
            );
        }
    }

    #[test]
    fn arbitrary_unknown_value_returns_error() {
        assert!(CommandStatus::try_from(99).is_err());
        assert!(CommandStatus::try_from(-1).is_err());
    }

    #[test]
    fn as_str_matches_variant() {
        assert_eq!(CommandStatus::Received.as_str(), "received");
        assert_eq!(CommandStatus::Reserved.as_str(), "reserved");
        assert_eq!(CommandStatus::Failed.as_str(), "failed");
        assert_eq!(CommandStatus::Completed.as_str(), "completed");
        assert_eq!(CommandStatus::Dead.as_str(), "dead");
        assert_eq!(CommandStatus::Archive.as_str(), "archive");
    }
}
