pub mod io {
    pub use super::implementation::Handler;
    pub use super::intention::{FollowUser, UserFollowed};
    pub use super::ui::Api;
}

mod intention {
    use crate::assembly::io::AppError;
    use kernel::{ApplicationCommand, ApplicationEvent};
    use poem_openapi::Object;
    use serde::{Deserialize, Serialize};
    use uuid::Uuid;

    /// Asks the system to create a follow relationship between two users.
    #[derive(Debug, Clone, Serialize, Deserialize, ApplicationCommand)]
    #[command_type = "FollowUser"]
    pub struct FollowUser {
        pub follower_id: Uuid,
        pub following_id: Uuid,
    }

    impl FollowUser {
        pub fn new(follower_id: Uuid, following_id: Uuid) -> Self {
            Self {
                follower_id,
                following_id,
            }
        }

        pub fn validate(self) -> Result<Self, AppError> {
            if self.follower_id == self.following_id {
                return Err(AppError::Validation("cannot follow self".to_string()));
            }

            Ok(self)
        }
    }

    /// States that one user started following another.
    #[derive(Debug, Clone, Serialize, Deserialize, Object, ApplicationEvent)]
    #[event_type = "UserFollowed"]
    pub struct UserFollowed {
        pub follower_id: Uuid,
        pub following_id: Uuid,
    }
}

mod ui {
    use super::intention::FollowUser;
    use crate::AppState;
    use crate::assembly::io::{ApiError, AppError, FollowDto, dispatch_command, fetch_follow};
    use poem::web::Data;
    use poem_openapi::{Object, OpenApi, payload::Json};
    use serde::Deserialize;
    use uuid::Uuid;

    pub struct Api;

    #[OpenApi]
    impl Api {
        #[oai(path = "/users/follow", method = "post")]
        async fn follow_user(
            &self,
            state: Data<&AppState>,
            Json(request): Json<FollowUserRequest>,
        ) -> Result<Json<FollowDto>, ApiError> {
            let follower_id = request.follower_id;
            let following_id = request.following_id;
            let command = request.into_command();

            dispatch_command(&state.mulac, command)?;

            let response =
                fetch_follow(&state.pool, follower_id, following_id).map_err(AppError::Storage)?;

            Ok(Json(response))
        }
    }

    #[derive(Debug, Deserialize, Object)]
    pub struct FollowUserRequest {
        pub follower_id: Uuid,
        pub following_id: Uuid,
    }

    impl FollowUserRequest {
        pub(super) fn into_command(self) -> FollowUser {
            FollowUser::new(self.follower_id, self.following_id)
        }
    }
}

mod implementation {
    use super::intention::{FollowUser, UserFollowed};
    use crate::TwitterEvent;
    use crate::assembly::io::DbPool;
    use derive_new::new;
    use kernel::io::{CommandError, CommandHandlerPort};

    #[derive(new)]
    pub struct Handler {
        pub(super) pool: DbPool,
    }

    impl From<FollowUser> for UserFollowed {
        fn from(command: FollowUser) -> Self {
            Self {
                follower_id: command.follower_id,
                following_id: command.following_id,
            }
        }
    }

    impl CommandHandlerPort<FollowUser, TwitterEvent> for Handler {
        fn execute(&self, command: FollowUser) -> Result<Vec<TwitterEvent>, CommandError> {
            let command = command.validate().map_err(CommandError::from)?;
            let created = self.insert_follow(&command)?;

            if !created {
                return Ok(vec![]);
            }

            Ok(vec![TwitterEvent::UserFollowed(command.into())])
        }
    }

    impl Handler {
        fn insert_follow(&self, command: &FollowUser) -> Result<bool, CommandError> {
            insert_follow(&self.pool, command)
        }
    }

    pub(super) fn insert_follow(pool: &DbPool, command: &FollowUser) -> Result<bool, CommandError> {
        use crate::schema::follows;
        use diesel::prelude::*;

        let mut conn = pool
            .get()
            .map_err(|error| CommandError::Storage(error.to_string()))?;

        let rows = diesel::insert_into(follows::table)
            .values((
                follows::follower_id.eq(command.follower_id),
                follows::following_id.eq(command.following_id),
            ))
            .on_conflict_do_nothing()
            .execute(&mut conn)
            .map_err(|error| CommandError::HandlerExecution(error.to_string()))?;

        Ok(rows > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::intention::{FollowUser, UserFollowed};
    use super::ui::FollowUserRequest;
    use uuid::Uuid;

    #[test]
    fn command_and_event_types_match_contract() {
        assert_eq!(FollowUser::COMMAND_TYPE, "FollowUser");
        assert_eq!(UserFollowed::EVENT_TYPE, "UserFollowed");
    }

    #[test]
    fn validate_rejects_self_follow() {
        let user_id = Uuid::now_v7();
        let result = FollowUser::new(user_id, user_id).validate();

        assert!(result.is_err(), "self-follow should be rejected");
    }

    #[test]
    fn request_carries_both_parties() {
        let follower_id = Uuid::now_v7();
        let following_id = Uuid::now_v7();
        let request = FollowUserRequest {
            follower_id,
            following_id,
        };

        assert_eq!(request.follower_id, follower_id);
        assert_eq!(request.following_id, following_id);
    }
}
