//! AMQP client component

use crate::amqp::AmqpMessage;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AmqpClientError {
    #[error("todo error: {0}")]
    Todo(String),
}

pub struct AmqpClient {}

impl AmqpClient {
    pub fn consume(&self) -> Result<Option<AmqpMessage>, AmqpClientError> {
        // TODO: implement
        Ok(None)
    }
}
