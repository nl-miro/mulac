pub mod io {
    pub use super::ui::Api;
}

mod intention {
    use super::ui::TwitterCommand;

    impl TwitterCommand {
        pub fn message_type(&self) -> &'static str {
            match self {
                Self::PostTweet(_) => "PostTweet",
                Self::DeleteTweet(_) => "DeleteTweet",
                Self::Retweet(_) => "Retweet",
                Self::FollowUser(_) => "FollowUser",
                Self::UnfollowUser(_) => "UnfollowUser",
                Self::LikeTweet(_) => "LikeTweet",
                Self::UnlikeTweet(_) => "UnlikeTweet",
                Self::SendDirectMessage(_) => "SendDirectMessage",
            }
        }
    }
}

mod ui {
    use super::temporary_adapters::record_inbound_message;
    use crate::{
        AppState,
        assembly::io::{ApiError, InboundResponse},
    };
    use kernel::InboundMessageEnvelope;
    use poem::web::Data;
    use poem_openapi::{Object, OpenApi, Union, payload::Json};
    use serde::{Deserialize, Serialize};
    use uuid::Uuid;

    /// The set of commands the inbox endpoint accepts, tagged by `type`.
    #[derive(Debug, Clone, Serialize, Deserialize, Union)]
    #[oai(discriminator_name = "type")]
    pub enum TwitterCommand {
        PostTweet(PostTweetInput),
        DeleteTweet(DeleteTweetInput),
        Retweet(RetweetInput),
        FollowUser(FollowUserInput),
        UnfollowUser(UnfollowUserInput),
        LikeTweet(LikeTweetInput),
        UnlikeTweet(UnlikeTweetInput),
        SendDirectMessage(SendDirectMessageInput),
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Object)]
    pub struct CommandEnvelope {
        pub id: Uuid,
        pub command: TwitterCommand,
    }

    pub struct Api;

    #[OpenApi]
    impl Api {
        #[oai(path = "/messages/commands", method = "post")]
        async fn process_command(
            &self,
            state: Data<&AppState>,
            Json(envelope): Json<CommandEnvelope>,
        ) -> Result<Json<InboundResponse>, ApiError> {
            //let message_id = envelope.id;

            let envelope2 = InboundMessageEnvelope::from(&envelope);

            record_inbound_message(state.mulac.inbox_recorder.clone(), envelope2).await?;

            todo!("handle duplicate message id");

            // dispatch(&state.mulac, state.pool.clone(), &envelope).await?;
            // let entity = materialize(state.pool.clone(), envelope.command).await?;
            // Ok(Json(InboundResponse { message_id, entity }))
        }
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Object)]
    pub struct PostTweetInput {
        pub tweet_id: Uuid,
        pub author_id: Uuid,
        pub content: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Object)]
    pub struct DeleteTweetInput {
        pub tweet_id: Uuid,
        pub author_id: Uuid,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Object)]
    pub struct RetweetInput {
        pub retweet_id: Uuid,
        pub original_tweet_id: Uuid,
        pub author_id: Uuid,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Object)]
    pub struct FollowUserInput {
        pub follower_id: Uuid,
        pub following_id: Uuid,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Object)]
    pub struct UnfollowUserInput {
        pub follower_id: Uuid,
        pub following_id: Uuid,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Object)]
    pub struct LikeTweetInput {
        pub user_id: Uuid,
        pub tweet_id: Uuid,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Object)]
    pub struct UnlikeTweetInput {
        pub user_id: Uuid,
        pub tweet_id: Uuid,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Object)]
    pub struct SendDirectMessageInput {
        pub message_id: Uuid,
        pub sender_id: Uuid,
        pub recipient_id: Uuid,
        pub content: String,
    }
}

mod implementation {
    use super::ui::TwitterCommand;
    use crate::assembly::io::{
        AppCommand, AppError, Clock, DbPool, FollowDto, InboundEntity, fetch_direct_message,
        fetch_follow, fetch_like, fetch_tweet,
    };
    use crate::direct_message_send::io::SendDirectMessage as SendDirectMessageCommand;
    use crate::tweet_delete::io::DeleteTweet as DeleteTweetCommand;
    use crate::tweet_like::io::LikeTweet as LikeTweetCommand;
    use crate::tweet_post::io::PostTweet as PostTweetCommand;
    use crate::tweet_retweet::io::Retweet as RetweetCommand;
    use crate::tweet_unlike::io::UnlikeTweet as UnlikeTweetCommand;
    use crate::user_follow::io::FollowUser as FollowUserCommand;
    use crate::user_unfollow::io::UnfollowUser as UnfollowUserCommand;
    use diesel::prelude::*;
    use uuid::Uuid;

    /// Translates an external inbox command into the internal `AppCommand`.
    pub(super) fn to_app_command(command: TwitterCommand) -> AppCommand {
        match command {
            TwitterCommand::PostTweet(command) => AppCommand::PostTweet(PostTweetCommand::new(
                command.tweet_id,
                command.author_id,
                command.content,
            )),
            TwitterCommand::DeleteTweet(command) => AppCommand::DeleteTweet(
                DeleteTweetCommand::new(command.tweet_id, command.author_id),
            ),
            TwitterCommand::Retweet(command) => AppCommand::Retweet(RetweetCommand::new(
                command.retweet_id,
                command.original_tweet_id,
                command.author_id,
            )),
            TwitterCommand::FollowUser(command) => AppCommand::FollowUser(FollowUserCommand::new(
                command.follower_id,
                command.following_id,
            )),
            TwitterCommand::UnfollowUser(command) => AppCommand::UnfollowUser(
                UnfollowUserCommand::new(command.follower_id, command.following_id),
            ),
            TwitterCommand::LikeTweet(command) => {
                AppCommand::LikeTweet(LikeTweetCommand::new(command.user_id, command.tweet_id))
            }
            TwitterCommand::UnlikeTweet(command) => {
                AppCommand::UnlikeTweet(UnlikeTweetCommand::new(command.user_id, command.tweet_id))
            }
            TwitterCommand::SendDirectMessage(command) => {
                AppCommand::SendDirectMessage(SendDirectMessageCommand::new(
                    command.message_id,
                    command.sender_id,
                    command.recipient_id,
                    command.content,
                ))
            }
        }
    }

    pub(super) fn record_received(
        pool: &DbPool,
        id: Uuid,
        message_type: &str,
        payload: &serde_json::Value,
    ) -> Result<bool, AppError> {
        let mut conn = pool
            .get()
            .map_err(|error| AppError::Storage(error.into()))?;
        let rows = diesel::sql_query(
            "INSERT INTO inbox_messages (id, message_type, payload, status, received_at) \
             VALUES ($1, $2, $3::jsonb, 'received', $4) \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind::<diesel::sql_types::Uuid, _>(id)
        .bind::<diesel::sql_types::Text, _>(message_type)
        .bind::<diesel::sql_types::Text, _>(payload.to_string())
        .bind::<diesel::sql_types::Timestamptz, _>(Clock::now())
        .execute(&mut conn)
        .map_err(|error| AppError::Storage(error.into()))?;
        Ok(rows > 0)
    }

    pub(super) fn mark_processed(pool: &DbPool, id: Uuid) -> Result<(), AppError> {
        let mut conn = pool
            .get()
            .map_err(|error| AppError::Storage(error.into()))?;
        diesel::sql_query(
            "UPDATE inbox_messages SET status = 'processed', processed_at = $1 WHERE id = $2",
        )
        .bind::<diesel::sql_types::Timestamptz, _>(Clock::now())
        .bind::<diesel::sql_types::Uuid, _>(id)
        .execute(&mut conn)
        .map_err(|error| AppError::Storage(error.into()))?;
        Ok(())
    }

    pub(super) fn mark_failed(pool: &DbPool, id: Uuid, error: &str) -> Result<(), AppError> {
        let mut conn = pool
            .get()
            .map_err(|db_error| AppError::Storage(db_error.into()))?;
        diesel::sql_query(
            "UPDATE inbox_messages SET status = 'failed', processed_at = $1, error = $2 WHERE id = $3",
        )
        .bind::<diesel::sql_types::Timestamptz, _>(Clock::now())
        .bind::<diesel::sql_types::Text, _>(error)
        .bind::<diesel::sql_types::Uuid, _>(id)
        .execute(&mut conn)
        .map_err(|db_error| AppError::Storage(db_error.into()))?;
        Ok(())
    }

    pub(super) async fn materialize_entity(
        pool: &DbPool,
        command: &TwitterCommand,
    ) -> Result<InboundEntity, AppError> {
        match command {
            TwitterCommand::PostTweet(command) => {
                let tweet = fetch_tweet(pool, command.tweet_id)
                    .await
                    .map_err(AppError::Storage)?;
                Ok(InboundEntity::Tweet(tweet))
            }
            TwitterCommand::DeleteTweet(_) => Ok(InboundEntity::no_entity()),
            TwitterCommand::Retweet(command) => {
                let tweet = fetch_tweet(pool, command.retweet_id)
                    .await
                    .map_err(AppError::Storage)?;
                Ok(InboundEntity::Tweet(tweet))
            }
            TwitterCommand::FollowUser(command) => {
                let follow = fetch_follow(pool, command.follower_id, command.following_id)
                    .map_err(AppError::Storage)?;
                Ok(InboundEntity::Follow(follow))
            }
            TwitterCommand::UnfollowUser(command) => Ok(InboundEntity::Follow(FollowDto {
                follower_id: command.follower_id,
                following_id: command.following_id,
                created_at: Clock::now(),
            })),
            TwitterCommand::LikeTweet(command) => {
                let like = fetch_like(pool, command.user_id, command.tweet_id)
                    .map_err(AppError::Storage)?;
                Ok(InboundEntity::Like(like))
            }
            TwitterCommand::UnlikeTweet(_) => Ok(InboundEntity::no_entity()),
            TwitterCommand::SendDirectMessage(command) => {
                let direct_message =
                    fetch_direct_message(pool, command.message_id).map_err(AppError::Storage)?;
                Ok(InboundEntity::DirectMessage(direct_message))
            }
        }
    }
}

mod temporary_adapters {
    use super::implementation;
    use super::ui::{CommandEnvelope, TwitterCommand};
    use crate::assembly::io::{
        AppError,
        DbPool,
        InboundEntity,
        MulacState,
        NewCommandEnvelope,
        interpret_dispatch_error,
        run_blocking,
        //
    };
    use kernel::io::NewCommandMetadata;
    use kernel::{InboundMessageEnvelope, InboxError, InboxRecorder, NewInboxMessageEnvelope};
    use std::sync::Arc;

    impl From<&CommandEnvelope> for InboundMessageEnvelope {
        fn from(envelope: &CommandEnvelope) -> Self {
            Self {
                payload: serde_json::to_string(envelope)
                    .expect("serializing command envelope should be infallible"),
                message_id: Some(envelope.id),
                correlation_id: Some(envelope.id),
                source: Some("test_app_twitter.inbox".to_string()),
                routing_key: Some(envelope.command.message_type().to_string()),
            }
        }
    }

    /// Records the inbound message; a duplicate id surfaces as a 409 Conflict.
    pub async fn record_inbound_message(
        recorder: Arc<InboxRecorder>,
        env: InboundMessageEnvelope,
    ) -> Result<(), AppError> {
        let envelope = NewInboxMessageEnvelope::from(env);

        let recording_result = tokio::task::spawn_blocking(move || recorder.publish(envelope))
            .await
            .map_err(|e| InboxError::Recording(e.to_string()))?;

        match recording_result {
            Ok(_) => println!("RECORDED"),
            Err(_) => {
                todo!("handle duplicate message id");

                // tokio::select! {
                //     _ = token.cancelled() => break,
                //     _ = sleep(interval) => continue,
                // }
            }
        }

        Ok(())

        // let id = envelope.id;
        // let message_type = envelope.command.message_type().to_string();
        // let payload = serde_json::to_value(&envelope.command)
        //     .map_err(|error| AppError::Storage(error.into()))?;
        // let inserted = run_blocking(move || {
        //     implementation::record_received(&pool, id, &message_type, &payload)
        // })
        // .await?;
        // if !inserted {
        //     return Err(AppError::Conflict("duplicate inbox message id".to_string()));
        // }
        // Ok(())
    }

    /// Dispatches the command through the kernel and records the outcome on the
    /// inbox row (processed on success, failed on error).
    pub async fn dispatch(
        mulac: &MulacState,
        pool: DbPool,
        envelope: &CommandEnvelope,
    ) -> Result<(), AppError> {
        let message_id = envelope.id;
        let app_command = implementation::to_app_command(envelope.command.clone());
        let mulac = mulac.clone();
        let dispatched = run_blocking(move || {
            mulac
                .dispatch_command(NewCommandEnvelope {
                    command: app_command,
                    metadata: NewCommandMetadata {
                        command_id: message_id,
                        correlation_id: Some(message_id),
                        causation_id: Some(message_id),
                        source: Some("test_app_twitter.inbox".to_string()),
                    },
                })
                .map_err(interpret_dispatch_error)
        })
        .await;

        match dispatched {
            Ok(()) => run_blocking(move || implementation::mark_processed(&pool, message_id)).await,
            Err(error) => {
                let error_text = error.to_string();
                run_blocking(move || implementation::mark_failed(&pool, message_id, &error_text))
                    .await?;
                Err(error)
            }
        }
    }

    /// Reads back the entity produced by the command for the response.
    pub async fn materialize(
        pool: DbPool,
        command: TwitterCommand,
    ) -> Result<InboundEntity, AppError> {
        implementation::materialize_entity(&pool, &command).await
    }
}

#[cfg(test)]
mod tests {
    use super::implementation::to_app_command;
    use super::ui::{
        CommandEnvelope, DeleteTweetInput, FollowUserInput, LikeTweetInput, PostTweetInput,
        RetweetInput, SendDirectMessageInput, TwitterCommand, UnfollowUserInput, UnlikeTweetInput,
    };
    use crate::assembly::io::AppCommand;
    use kernel::InboundMessageEnvelope;
    use uuid::Uuid;

    #[test]
    fn message_type_matches_every_variant() {
        let id = Uuid::now_v7();
        let cases: Vec<(TwitterCommand, &str)> = vec![
            (
                TwitterCommand::PostTweet(PostTweetInput {
                    tweet_id: id,
                    author_id: id,
                    content: "x".to_string(),
                }),
                "PostTweet",
            ),
            (
                TwitterCommand::DeleteTweet(DeleteTweetInput {
                    tweet_id: id,
                    author_id: id,
                }),
                "DeleteTweet",
            ),
            (
                TwitterCommand::Retweet(RetweetInput {
                    retweet_id: id,
                    original_tweet_id: id,
                    author_id: id,
                }),
                "Retweet",
            ),
            (
                TwitterCommand::FollowUser(FollowUserInput {
                    follower_id: id,
                    following_id: id,
                }),
                "FollowUser",
            ),
            (
                TwitterCommand::UnfollowUser(UnfollowUserInput {
                    follower_id: id,
                    following_id: id,
                }),
                "UnfollowUser",
            ),
            (
                TwitterCommand::LikeTweet(LikeTweetInput {
                    user_id: id,
                    tweet_id: id,
                }),
                "LikeTweet",
            ),
            (
                TwitterCommand::UnlikeTweet(UnlikeTweetInput {
                    user_id: id,
                    tweet_id: id,
                }),
                "UnlikeTweet",
            ),
            (
                TwitterCommand::SendDirectMessage(SendDirectMessageInput {
                    message_id: id,
                    sender_id: id,
                    recipient_id: id,
                    content: "x".to_string(),
                }),
                "SendDirectMessage",
            ),
        ];
        for (command, expected) in cases {
            assert_eq!(command.message_type(), expected);
        }
    }

    #[test]
    fn post_tweet_input_maps_to_app_command() {
        let tweet_id = Uuid::now_v7();
        let author_id = Uuid::now_v7();
        let command = TwitterCommand::PostTweet(PostTweetInput {
            tweet_id,
            author_id,
            content: "hi".to_string(),
        });
        match to_app_command(command) {
            AppCommand::PostTweet(tweet) => {
                assert_eq!(tweet.tweet_id, tweet_id);
                assert_eq!(tweet.author_id, author_id);
                assert_eq!(tweet.content, "hi");
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn like_tweet_input_maps_to_composite_key_command() {
        let user_id = Uuid::now_v7();
        let tweet_id = Uuid::now_v7();
        let command = TwitterCommand::LikeTweet(LikeTweetInput { user_id, tweet_id });
        match to_app_command(command) {
            AppCommand::LikeTweet(like) => {
                assert_eq!(like.user_id, user_id);
                assert_eq!(like.tweet_id, tweet_id);
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn command_envelope_maps_to_inbound_message_envelope() {
        let id = Uuid::now_v7();
        let envelope = CommandEnvelope {
            id,
            command: TwitterCommand::PostTweet(PostTweetInput {
                tweet_id: id,
                author_id: id,
                content: "hello".to_string(),
            }),
        };

        let inbound = InboundMessageEnvelope::from(&envelope);

        assert_eq!(inbound.message_id, Some(id));
        assert_eq!(inbound.correlation_id, Some(id));
        assert_eq!(inbound.source.as_deref(), Some("test_app_twitter.inbox"));
        assert_eq!(inbound.routing_key.as_deref(), Some("PostTweet"));
        assert_eq!(inbound.payload, serde_json::to_string(&envelope).unwrap());
    }
}
