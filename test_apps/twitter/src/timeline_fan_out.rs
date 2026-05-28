pub mod io {
    pub use super::implementation::{Handler, Subscriber};
    pub use super::intention::FanOutTweet;
}

mod intention {
    use kernel::ApplicationCommand;
    use serde::{Deserialize, Serialize};
    use uuid::Uuid;

    /// Asks the system to materialize a posted tweet into follower timelines.
    #[derive(Debug, Clone, Serialize, Deserialize, ApplicationCommand)]
    #[command_type = "FanOutTweet"]
    pub struct FanOutTweet {
        pub tweet_id: Uuid,
        pub author_id: Uuid,
    }

    impl FanOutTweet {
        pub fn new(tweet_id: Uuid, author_id: Uuid) -> Self {
            Self {
                tweet_id,
                author_id,
            }
        }
    }
}

mod implementation {
    use super::intention::FanOutTweet;
    use super::temporary_adapters::dispatch_fan_out;
    use crate::TwitterEvent;
    use crate::assembly::io::DbPool;
    use derive_new::new;
    use kernel::io::{CommandError, CommandGateway, CommandHandlerPort};
    use kernel::{EventError, EventSubscriberPort, NewEventEnvelope};
    use std::sync::Arc;

    /// Command entry point: materializes a tweet into every follower's timeline.
    #[derive(new)]
    pub struct Handler {
        pub(super) pool: DbPool,
    }

    impl CommandHandlerPort<FanOutTweet, TwitterEvent> for Handler {
        fn execute(&self, command: FanOutTweet) -> Result<Vec<TwitterEvent>, CommandError> {
            self.fan_out(&command)?;
            Ok(vec![])
        }
    }

    impl Handler {
        fn fan_out(&self, command: &FanOutTweet) -> Result<(), CommandError> {
            fan_out(&self.pool, command)
        }
    }

    /// Event entry point: reacts to `TweetPosted` by dispatching a `FanOutTweet`
    /// command back through the command gateway.
    #[derive(new)]
    pub struct Subscriber {
        pub(super) command_gateway: Arc<CommandGateway>,
    }

    impl EventSubscriberPort for Subscriber {
        fn handle(&self, envelope: &NewEventEnvelope) -> Result<(), EventError> {
            dispatch_fan_out(&self.command_gateway, envelope)
        }
    }

    pub(super) fn fan_out(pool: &DbPool, command: &FanOutTweet) -> Result<(), CommandError> {
        use crate::schema::{follows, timelines};
        use diesel::prelude::*;
        use uuid::Uuid;

        let mut conn = pool
            .get()
            .map_err(|error| CommandError::Storage(error.to_string()))?;

        let follower_ids: Vec<Uuid> = follows::table
            .filter(follows::following_id.eq(command.author_id))
            .select(follows::follower_id)
            .load(&mut conn)
            .map_err(|error| CommandError::HandlerExecution(error.to_string()))?;

        for follower_id in follower_ids {
            diesel::insert_into(timelines::table)
                .values((
                    timelines::id.eq(Uuid::now_v7()),
                    timelines::user_id.eq(follower_id),
                    timelines::tweet_id.eq(command.tweet_id),
                    timelines::author_id.eq(command.author_id),
                ))
                .on_conflict_do_nothing()
                .execute(&mut conn)
                .map_err(|error| CommandError::HandlerExecution(error.to_string()))?;
        }

        Ok(())
    }
}

mod temporary_adapters {
    use super::intention::FanOutTweet;
    use crate::TwitterEvent;
    use kernel::io::{CommandGateway, NewCommand, NewCommandEnvelope, NewCommandMetadata};
    use kernel::{EventError, NewEventEnvelope};
    use uuid::Uuid;

    /// Parses a `TweetPosted` event payload and dispatches a `FanOutTweet`
    /// command through the gateway.
    pub fn dispatch_fan_out(
        command_gateway: &CommandGateway,
        envelope: &NewEventEnvelope,
    ) -> Result<(), EventError> {
        let twitter_event: TwitterEvent = serde_json::from_str(&envelope.payload)
            .map_err(|error| EventError::SubscriberExecution(error.to_string()))?;

        let (tweet_id, author_id) = match twitter_event {
            TwitterEvent::TweetPosted(ref event) => (event.tweet_id, event.author_id),
            other => {
                return Err(EventError::SubscriberExecution(format!(
                    "unexpected event for timeline fan-out: {:?}",
                    other,
                )));
            }
        };

        let fan_out_command = FanOutTweet::new(tweet_id, author_id);
        let command_id = Uuid::now_v7();
        let gateway_envelope = NewCommandEnvelope {
            command: NewCommand {
                command_type: FanOutTweet::COMMAND_TYPE.to_string(),
                payload: serde_json::to_string(&fan_out_command)
                    .map_err(|error| EventError::SubscriberExecution(error.to_string()))?,
            },
            metadata: Some(NewCommandMetadata {
                command_id,
                correlation_id: None,
                causation_id: None,
                source: Some("event:TweetPosted".to_string()),
            }),
        };

        command_gateway
            .dispatch(gateway_envelope)
            .map_err(|error| EventError::SubscriberExecution(error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::intention::FanOutTweet;

    #[test]
    fn command_type_matches_contract() {
        assert_eq!(FanOutTweet::COMMAND_TYPE, "FanOutTweet");
    }
}
