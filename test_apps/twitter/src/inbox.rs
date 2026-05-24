mod models {
    use crate::assembly::io::{
        AppError,
        Clock,
        DbPool,
        FollowDto,
        InboundEntity,
        fetch_direct_message,
        fetch_follow,
        fetch_like,
        fetch_tweet,
        //
    };
    use poem_openapi::{Object, Union};
    use serde::{Deserialize, Serialize};
    use uuid::Uuid;

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

    impl kernel::ApplicationCommand for TwitterCommand {
        fn command_type(&self) -> &'static str {
            self.message_type()
        }
    }

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

        pub fn entity_id(&self) -> Option<Uuid> {
            match self {
                Self::PostTweet(c) => Some(c.tweet_id),
                Self::DeleteTweet(c) => Some(c.tweet_id),
                Self::Retweet(c) => Some(c.retweet_id),
                Self::SendDirectMessage(c) => Some(c.message_id),
                Self::FollowUser(_)
                | Self::UnfollowUser(_)
                | Self::LikeTweet(_)
                | Self::UnlikeTweet(_) => None,
            }
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

    #[derive(Debug, Clone, Serialize, Deserialize, Object)]
    pub struct CommandEnvelope {
        pub id: Uuid,
        pub command: TwitterCommand,
    }

    pub fn to_app_command(command: TwitterCommand) -> crate::assembly::io::AppCommand {
        use crate::assembly::io::AppCommand;
        #[allow(unused_imports)]
        use kernel::ApplicationCommand as _;
        match command {
            TwitterCommand::PostTweet(c) => AppCommand::PostTweet(crate::tweet_post::io::Command {
                tweet_id: c.tweet_id,
                author_id: c.author_id,
                content: c.content,
            }),
            TwitterCommand::DeleteTweet(c) => {
                AppCommand::DeleteTweet(crate::tweet_delete::io::Command {
                    tweet_id: c.tweet_id,
                    author_id: c.author_id,
                })
            }
            TwitterCommand::Retweet(c) => AppCommand::Retweet(crate::tweet_retweet::io::Command {
                retweet_id: c.retweet_id,
                original_tweet_id: c.original_tweet_id,
                author_id: c.author_id,
            }),
            TwitterCommand::FollowUser(c) => {
                AppCommand::FollowUser(crate::user_follow::io::Command {
                    follower_id: c.follower_id,
                    following_id: c.following_id,
                })
            }
            TwitterCommand::UnfollowUser(c) => {
                AppCommand::UnfollowUser(crate::user_unfollow::io::Command {
                    follower_id: c.follower_id,
                    following_id: c.following_id,
                })
            }
            TwitterCommand::LikeTweet(c) => AppCommand::LikeTweet(crate::tweet_like::io::Command {
                user_id: c.user_id,
                tweet_id: c.tweet_id,
            }),
            TwitterCommand::UnlikeTweet(c) => {
                AppCommand::UnlikeTweet(crate::tweet_unlike::io::Command {
                    user_id: c.user_id,
                    tweet_id: c.tweet_id,
                })
            }
            TwitterCommand::SendDirectMessage(c) => {
                AppCommand::SendDirectMessage(crate::direct_message_send::io::Command {
                    message_id: c.message_id,
                    sender_id: c.sender_id,
                    recipient_id: c.recipient_id,
                    content: c.content,
                })
            }
        }
    }

    pub fn materialize_entity(
        pool: &DbPool,
        command: &TwitterCommand,
    ) -> Result<InboundEntity, AppError> {
        match command {
            TwitterCommand::PostTweet(c) => {
                let tweet = fetch_tweet(pool, c.tweet_id).map_err(|e| AppError::Storage(e))?;
                Ok(InboundEntity::Tweet(tweet))
            }
            TwitterCommand::DeleteTweet(_) => Ok(InboundEntity::no_entity()),
            TwitterCommand::Retweet(c) => {
                let tweet = fetch_tweet(pool, c.retweet_id).map_err(|e| AppError::Storage(e))?;
                Ok(InboundEntity::Tweet(tweet))
            }
            TwitterCommand::FollowUser(c) => {
                let follow = fetch_follow(pool, c.follower_id, c.following_id)
                    .map_err(|e| AppError::Storage(e))?;
                Ok(InboundEntity::Follow(follow))
            }
            TwitterCommand::UnfollowUser(c) => Ok(InboundEntity::Follow(FollowDto {
                follower_id: c.follower_id,
                following_id: c.following_id,
                created_at: Clock::now(),
            })),
            TwitterCommand::LikeTweet(c) => {
                let like =
                    fetch_like(pool, c.user_id, c.tweet_id).map_err(|e| AppError::Storage(e))?;
                Ok(InboundEntity::Like(like))
            }
            TwitterCommand::UnlikeTweet(_) => Ok(InboundEntity::no_entity()),
            TwitterCommand::SendDirectMessage(c) => {
                let dm =
                    fetch_direct_message(pool, c.message_id).map_err(|e| AppError::Storage(e))?;
                Ok(InboundEntity::DirectMessage(dm))
            }
        }
    }
}

mod infra_diesel {
    use crate::assembly::io::{AppError, Clock, DbPool};
    use diesel::prelude::*;
    use uuid::Uuid;

    pub fn record_received(
        pool: &DbPool,
        id: Uuid,
        message_type: &str,
        payload: &serde_json::Value,
    ) -> Result<bool, AppError> {
        let mut conn = pool.get().map_err(|e| AppError::Storage(e.into()))?;
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
        .map_err(|e| AppError::Storage(e.into()))?;
        Ok(rows > 0)
    }

    pub fn mark_processed(pool: &DbPool, id: Uuid) -> Result<(), AppError> {
        let mut conn = pool.get().map_err(|e| AppError::Storage(e.into()))?;
        diesel::sql_query(
            "UPDATE inbox_messages SET status = 'processed', processed_at = $1 WHERE id = $2",
        )
        .bind::<diesel::sql_types::Timestamptz, _>(Clock::now())
        .bind::<diesel::sql_types::Uuid, _>(id)
        .execute(&mut conn)
        .map_err(|e| AppError::Storage(e.into()))?;
        Ok(())
    }

    pub fn mark_failed(pool: &DbPool, id: Uuid, error: &str) -> Result<(), AppError> {
        let mut conn = pool.get().map_err(|e| AppError::Storage(e.into()))?;
        diesel::sql_query(
            "UPDATE inbox_messages SET status = 'failed', processed_at = $1, error = $2 WHERE id = $3",
        )
        .bind::<diesel::sql_types::Timestamptz, _>(Clock::now())
        .bind::<diesel::sql_types::Text, _>(error)
        .bind::<diesel::sql_types::Uuid, _>(id)
        .execute(&mut conn)
        .map_err(|e| AppError::Storage(e.into()))?;
        Ok(())
    }
}

mod http {
    use super::infra_diesel::{mark_failed, mark_processed, record_received};
    use super::models::{CommandEnvelope, materialize_entity, to_app_command};
    use crate::{
        AppState,
        assembly::io::{
            ApiError,
            AppError,
            InboundResponse,
            NewCommandEnvelope,
            interpret_dispatch_error,
            run_blocking,
            //
        },
        //
    };
    use kernel::io::NewCommandMetadata;
    use poem::web::Data;
    use poem_openapi::{OpenApi, payload::Json};

    pub struct Api;

    #[OpenApi]
    impl Api {
        #[oai(path = "/messages/commands", method = "post")]
        async fn process_command(
            &self,
            state: Data<&AppState>,
            Json(envelope): Json<CommandEnvelope>,
        ) -> Result<Json<InboundResponse>, ApiError> {
            let pool = state.pool.clone();
            let message_id = envelope.id;
            let payload =
                serde_json::to_value(&envelope.command).map_err(|e| AppError::Storage(e.into()))?;
            let message_type = envelope.command.message_type().to_string();

            // 1. Record received. Zero rows → duplicate id → 409.
            {
                let pool = pool.clone();
                let inserted = run_blocking(move || {
                    record_received(&pool, message_id, &message_type, &payload)
                })
                .await?;
                if !inserted {
                    return Err(AppError::Conflict("duplicate inbox message id".to_string()).into());
                }
            }

            // 2 + 3. Dispatch and drain.
            let app_command = to_app_command(envelope.command.clone());
            let app_command_for_materialize = envelope.command.clone();
            let mulac = state.mulac.clone();
            let command_id = message_id;
            let dispatch_result: Result<(), AppError> = {
                run_blocking(move || {
                    mulac
                        .dispatch_command(NewCommandEnvelope {
                            command: app_command,
                            metadata: NewCommandMetadata {
                                command_id,
                                correlation_id: Some(message_id),
                                causation_id: Some(message_id),
                                source: Some("test_app_twitter.inbox".to_string()),
                            },
                        })
                        .map_err(interpret_dispatch_error)
                })
                .await
            };

            match dispatch_result {
                Ok(()) => {
                    let pool_c = pool.clone();
                    run_blocking(move || mark_processed(&pool_c, message_id)).await?;
                }
                Err(err) => {
                    let err_text = err.to_string();
                    let pool_c = pool.clone();
                    run_blocking(move || mark_failed(&pool_c, message_id, &err_text)).await?;
                    return Err(err.into());
                }
            }

            // 4. Materialize entity.
            let entity = {
                let pool_c = pool.clone();
                run_blocking(move || materialize_entity(&pool_c, &app_command_for_materialize))
                    .await?
            };

            let response = InboundResponse { message_id, entity };
            Ok(Json(response))
        }
    }
}

pub mod io {
    pub use super::http::Api;
}
