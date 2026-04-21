//! AMQP worker component

use crate::amqp::AmqpMessage;
use crate::amqp::client::{AmqpClient, AmqpClientError};
use crate::inbox::io::{InboxMessage, InboxPublisher, InboxPublisherError};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AmqpWorkerError {
    #[error("client error: {0}")]
    ClientError(#[from] AmqpClientError),
    #[error("publisher error: {0}")]
    PublisherError(#[from] InboxPublisherError),
}

impl From<AmqpMessage> for InboxMessage {
    fn from(_msg: AmqpMessage) -> Self {
        InboxMessage {}
    }
}

struct AmqpWorkerConfig {}

pub struct AmqpWorker {
    client: AmqpClient,
    next: InboxPublisher,
}

impl AmqpWorker {
    pub fn new(client: AmqpClient, next: InboxPublisher) -> Self {
        Self { client, next }
    }

    pub fn consume(&self) -> Result<bool, AmqpWorkerError> {
        let msg = self.client.consume()?;

        let message: AmqpMessage = match msg {
            Some(msg) => msg,
            None => return Ok(false),
        };

        let inbound_msg = InboxMessage::from(message.clone());
        let res = self.next.publish(inbound_msg);

        match res {
            Err(e) => {
                eprintln!("Failed to publish message: {}", e);
                self.nack(message);
            }
            Ok(_) => self.ack(message),
        }

        Ok(true)
    }

    fn ack(&self, msg: AmqpMessage) {}

    fn nack(&self, msg: AmqpMessage) {}
}
