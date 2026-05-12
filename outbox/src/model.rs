use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum OutboxStatus {
    Received = 0,
    Reserved = 2,
    Failed = 4,
    Completed = 5,
    Dead = 7,
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
