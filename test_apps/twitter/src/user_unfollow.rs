pub mod io {
    pub use super::implementation::Handler;
    pub use super::intention::{UnfollowUser, UserUnfollowed};
    pub use super::ui::Api;
}

mod intention {
    use kernel::{ApplicationCommand, ApplicationEvent};
    use poem_openapi::Object;
    use serde::{Deserialize, Serialize};
    use uuid::Uuid;

    /// Asks the system to remove a follow relationship if it exists.
    #[derive(Debug, Clone, Serialize, Deserialize, ApplicationCommand)]
    #[command_type = "UnfollowUser"]
    pub struct UnfollowUser {
        pub follower_id: Uuid,
        pub following_id: Uuid,
    }

    impl UnfollowUser {
        pub fn new(follower_id: Uuid, following_id: Uuid) -> Self {
            Self {
                follower_id,
                following_id,
            }
        }
    }

    /// States that one user stopped following another.
    #[derive(Debug, Clone, Serialize, Deserialize, Object, ApplicationEvent)]
    #[event_type = "UserUnfollowed"]
    pub struct UserUnfollowed {
        pub follower_id: Uuid,
        pub following_id: Uuid,
    }
}

mod ui {
    use super::intention::UnfollowUser;
    use crate::AppState;
    use crate::assembly::io::{ApiError, FollowDto, dispatch_command};
    use poem::web::Data;
    use poem_openapi::{Object, OpenApi, payload::Json};
    use serde::Deserialize;
    use uuid::Uuid;

    pub struct Api;

    #[OpenApi]
    impl Api {
        #[oai(path = "/users/unfollow", method = "post")]
        async fn unfollow_user(
            &self,
            state: Data<&AppState>,
            Json(request): Json<UnfollowUserRequest>,
        ) -> Result<Json<FollowDto>, ApiError> {
            let follower_id = request.follower_id;
            let following_id = request.following_id;
            let command = request.into_command();

            dispatch_command(&state.mulac, command)?;

            Ok(Json(FollowDto {
                follower_id,
                following_id,
                created_at: crate::assembly::io::Clock::now(),
            }))
        }
    }

    #[derive(Debug, Deserialize, Object)]
    pub struct UnfollowUserRequest {
        pub follower_id: Uuid,
        pub following_id: Uuid,
    }

    impl UnfollowUserRequest {
        pub(super) fn into_command(self) -> UnfollowUser {
            UnfollowUser::new(self.follower_id, self.following_id)
        }
    }
}

mod implementation {
    use super::intention::{UnfollowUser, UserUnfollowed};
    use crate::TwitterEvent;
    use crate::assembly::io::DbPool;
    use derive_new::new;
    use kernel::io::{CommandError, CommandHandlerPort};

    #[derive(new)]
    pub struct Handler {
        pub(super) pool: DbPool,
    }

    impl From<UnfollowUser> for UserUnfollowed {
        fn from(command: UnfollowUser) -> Self {
            Self {
                follower_id: command.follower_id,
                following_id: command.following_id,
            }
        }
    }

    impl CommandHandlerPort<UnfollowUser, TwitterEvent> for Handler {
        fn execute(&self, command: UnfollowUser) -> Result<Vec<TwitterEvent>, CommandError> {
            let removed = self.delete_follow(&command)?;

            if !removed {
                return Ok(vec![]);
            }

            Ok(vec![TwitterEvent::UserUnfollowed(command.into())])
        }
    }

    impl Handler {
        fn delete_follow(&self, command: &UnfollowUser) -> Result<bool, CommandError> {
            delete_follow(&self.pool, command)
        }
    }

    /// Deletes the follow relationship. Returns `true` when a row was removed,
    /// `false` when the relationship was absent (idempotent no-op).
    pub(super) fn delete_follow(
        pool: &DbPool,
        command: &UnfollowUser,
    ) -> Result<bool, CommandError> {
        use crate::schema::follows;
        use diesel::prelude::*;

        let mut conn = pool
            .get()
            .map_err(|error| CommandError::Storage(error.to_string()))?;

        let rows = diesel::delete(follows::table.find((command.follower_id, command.following_id)))
            .execute(&mut conn)
            .map_err(|error| CommandError::HandlerExecution(error.to_string()))?;

        Ok(rows > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::intention::{UnfollowUser, UserUnfollowed};
    use super::ui::UnfollowUserRequest;
    use crate::assembly::io::{Clock, FollowDto};
    use uuid::Uuid;

    #[test]
    fn command_and_event_types_match_contract() {
        assert_eq!(UnfollowUser::COMMAND_TYPE, "UnfollowUser");
        assert_eq!(UserUnfollowed::EVENT_TYPE, "UserUnfollowed");
    }

    #[test]
    fn synthesized_dto_echoes_request_parties() {
        let follower_id = Uuid::now_v7();
        let following_id = Uuid::now_v7();
        let _request = UnfollowUserRequest {
            follower_id,
            following_id,
        };
        let dto = FollowDto {
            follower_id,
            following_id,
            created_at: Clock::now(),
        };

        assert_eq!(dto.follower_id, follower_id);
        assert_eq!(dto.following_id, following_id);
    }
}
