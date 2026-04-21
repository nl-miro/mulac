//! Inbox publisher component

use crate::inbox::InboxMessage;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum InboxPublisherError {
    #[error("publish error: {0}")]
    PublishError(String),
}

pub struct InboxPublisher {}

impl InboxPublisher {
    pub fn publish(&self, _msg: InboxMessage) -> Result<(), InboxPublisherError> {
        // TODO: implement publish logic
        Ok(())
    }
}
