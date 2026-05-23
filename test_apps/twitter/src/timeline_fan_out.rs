pub const COMMAND_NAME: &str = "FanOutTweet";

mod models {
    use serde::{Deserialize, Serialize};
    use uuid::Uuid;

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Command {
        pub tweet_id: Uuid,
        pub author_id: Uuid,
    }

    impl kernel::ApplicationCommand for Command {
        fn command_type(&self) -> &'static str {
            super::COMMAND_NAME
        }
    }
}

mod handler {
    use super::models::Command;
    use crate::assembly::io::DbPool;
    use commanding::io::{CommandError, CommandHandlerPort};

    pub struct Handler {
        pool: DbPool,
    }

    impl Handler {
        pub fn new(pool: DbPool) -> Self {
            Self { pool }
        }
    }

    impl CommandHandlerPort<Command, crate::TwitterEvent> for Handler {
        fn execute(&self, cmd: Command) -> Result<Vec<crate::TwitterEvent>, CommandError> {
            use crate::schema::{follows, timelines};
            use diesel::prelude::*;
            use uuid::Uuid;

            let mut conn = self
                .pool
                .get()
                .map_err(|e| CommandError::Storage(e.to_string()))?;

            let follower_ids: Vec<Uuid> = follows::table
                .filter(follows::following_id.eq(cmd.author_id))
                .select(follows::follower_id)
                .load(&mut conn)
                .map_err(|e| CommandError::HandlerExecution(e.to_string()))?;

            for follower_id in follower_ids {
                diesel::insert_into(timelines::table)
                    .values((
                        timelines::id.eq(Uuid::now_v7()),
                        timelines::user_id.eq(follower_id),
                        timelines::tweet_id.eq(cmd.tweet_id),
                        timelines::author_id.eq(cmd.author_id),
                    ))
                    .on_conflict_do_nothing()
                    .execute(&mut conn)
                    .map_err(|e| CommandError::HandlerExecution(e.to_string()))?;
            }

            Ok(vec![])
        }
    }
}

mod eventing {
    use commanding::io::{CommandGateway, NewCommand, NewCommandEnvelope, NewCommandMetadata};
    use kernel::{EventError, EventSubscriberPort, NewEventEnvelope};
    use std::sync::Arc;
    use uuid::Uuid;

    pub struct Subscriber {
        command_gateway: Arc<CommandGateway>,
    }

    impl Subscriber {
        pub fn new(command_gateway: Arc<CommandGateway>) -> Self {
            Self { command_gateway }
        }
    }

    impl EventSubscriberPort for Subscriber {
        fn handle(&self, envelope: &NewEventEnvelope) -> Result<(), EventError> {
            let twitter_event: crate::TwitterEvent = serde_json::from_str(&envelope.payload)
                .map_err(|e| EventError::SubscriberExecution(e.to_string()))?;

            let (tweet_id, author_id) = match twitter_event {
                crate::TwitterEvent::TweetPosted(ref ev) => (ev.tweet_id, ev.author_id),
                other => {
                    return Err(EventError::SubscriberExecution(format!(
                        "unexpected event for timeline fan-out: {:?}",
                        other,
                    )));
                }
            };

            let fan_cmd = super::models::Command {
                tweet_id,
                author_id,
            };
            let command_id = Uuid::now_v7();
            let gateway_envelope = NewCommandEnvelope {
                command: NewCommand {
                    command_type: super::COMMAND_NAME.to_string(),
                    payload: serde_json::to_string(&fan_cmd)
                        .map_err(|e| EventError::SubscriberExecution(e.to_string()))?,
                },
                metadata: Some(NewCommandMetadata {
                    command_id,
                    correlation_id: None,
                    causation_id: None,
                    source: Some("event:TweetPosted".to_string()),
                }),
            };

            self.command_gateway
                .dispatch(gateway_envelope)
                .map_err(|e| EventError::SubscriberExecution(e.to_string()))
        }
    }
}

pub mod io {
    pub use super::COMMAND_NAME;
    pub use super::eventing::Subscriber;
    pub use super::handler::Handler;
    pub use super::models::Command;
}
