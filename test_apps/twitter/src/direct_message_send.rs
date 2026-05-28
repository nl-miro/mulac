pub mod io {
    pub use super::implementation::Handler;
    pub use super::intention::{DirectMessageSent, SendDirectMessage};
    pub use super::ui::Api;
}

mod intention {
    use crate::assembly::io::{AppError, validate_content};
    use kernel::{ApplicationCommand, ApplicationEvent};
    use poem_openapi::Object;
    use serde::{Deserialize, Serialize};
    use uuid::Uuid;

    const MAX_DIRECT_MESSAGE_CONTENT_CHARS: usize = 280;

    /// Asks the system to send a direct message from one user to another.
    #[derive(Debug, Clone, Serialize, Deserialize, ApplicationCommand)]
    #[command_type = "SendDirectMessage"]
    pub struct SendDirectMessage {
        pub message_id: Uuid,
        pub sender_id: Uuid,
        pub recipient_id: Uuid,
        pub content: String,
    }

    impl SendDirectMessage {
        pub fn new(message_id: Uuid, sender_id: Uuid, recipient_id: Uuid, content: String) -> Self {
            Self {
                message_id,
                sender_id,
                recipient_id,
                content,
            }
        }

        pub fn validate(self) -> Result<Self, AppError> {
            validate_content(&self.content, MAX_DIRECT_MESSAGE_CONTENT_CHARS)?;
            Ok(self)
        }
    }

    /// States that a direct message was sent.
    #[derive(Debug, Clone, Serialize, Deserialize, Object, ApplicationEvent)]
    #[event_type = "DirectMessageSent"]
    pub struct DirectMessageSent {
        pub message_id: Uuid,
        pub sender_id: Uuid,
        pub recipient_id: Uuid,
        pub content: String,
    }
}

mod ui {
    use super::intention::SendDirectMessage;
    use crate::AppState;
    use crate::assembly::io::{
        ApiError, AppError, DirectMessageDto, dispatch_command, fetch_direct_message,
    };
    use poem::web::Data;
    use poem_openapi::{Object, OpenApi, payload::Json};
    use serde::Deserialize;
    use uuid::Uuid;

    pub struct Api;

    #[OpenApi]
    impl Api {
        #[oai(path = "/messages/direct", method = "post")]
        async fn send_direct_message(
            &self,
            state: Data<&AppState>,
            Json(request): Json<SendDirectMessageRequest>,
        ) -> Result<Json<DirectMessageDto>, ApiError> {
            let message_id = Uuid::now_v7();
            let command = request.into_command(message_id);

            dispatch_command(&state.mulac, command)?;

            let response =
                fetch_direct_message(&state.pool, message_id).map_err(AppError::Storage)?;

            Ok(Json(response))
        }
    }

    #[derive(Debug, Deserialize, Object)]
    pub struct SendDirectMessageRequest {
        pub sender_id: Uuid,
        pub recipient_id: Uuid,
        pub content: String,
    }

    impl SendDirectMessageRequest {
        pub(super) fn into_command(self, message_id: Uuid) -> SendDirectMessage {
            SendDirectMessage::new(message_id, self.sender_id, self.recipient_id, self.content)
        }
    }
}

mod implementation {
    use super::intention::{DirectMessageSent, SendDirectMessage};
    use crate::TwitterEvent;
    use crate::assembly::io::DbPool;
    use derive_new::new;
    use kernel::io::{CommandError, CommandHandlerPort};

    #[derive(new)]
    pub struct Handler {
        pub(super) pool: DbPool,
    }

    impl From<SendDirectMessage> for DirectMessageSent {
        fn from(command: SendDirectMessage) -> Self {
            Self {
                message_id: command.message_id,
                sender_id: command.sender_id,
                recipient_id: command.recipient_id,
                content: command.content,
            }
        }
    }

    impl CommandHandlerPort<SendDirectMessage, TwitterEvent> for Handler {
        fn execute(&self, command: SendDirectMessage) -> Result<Vec<TwitterEvent>, CommandError> {
            let command = command.validate().map_err(CommandError::from)?;
            self.insert_message(&command)?;

            Ok(vec![TwitterEvent::DirectMessageSent(command.into())])
        }
    }

    impl Handler {
        fn insert_message(&self, command: &SendDirectMessage) -> Result<(), CommandError> {
            insert_message(&self.pool, command)
        }
    }

    pub(super) fn insert_message(
        pool: &DbPool,
        command: &SendDirectMessage,
    ) -> Result<(), CommandError> {
        use crate::schema::direct_messages;
        use diesel::prelude::*;

        let mut conn = pool
            .get()
            .map_err(|error| CommandError::Storage(error.to_string()))?;

        let rows = diesel::insert_into(direct_messages::table)
            .values((
                direct_messages::id.eq(command.message_id),
                direct_messages::sender_id.eq(command.sender_id),
                direct_messages::recipient_id.eq(command.recipient_id),
                direct_messages::content.eq(&command.content),
            ))
            .on_conflict_do_nothing()
            .execute(&mut conn)
            .map_err(|error| CommandError::HandlerExecution(error.to_string()))?;

        if rows == 0 {
            return Err(CommandError::HandlerExecution(
                "duplicate message_id".to_string(),
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::intention::{DirectMessageSent, SendDirectMessage};
    use super::ui::SendDirectMessageRequest;
    use uuid::Uuid;

    fn sample_command(content: &str) -> SendDirectMessage {
        SendDirectMessage::new(
            Uuid::now_v7(),
            Uuid::now_v7(),
            Uuid::now_v7(),
            content.to_string(),
        )
    }

    #[test]
    fn command_and_event_types_match_contract() {
        assert_eq!(SendDirectMessage::COMMAND_TYPE, "SendDirectMessage");
        assert_eq!(DirectMessageSent::EVENT_TYPE, "DirectMessageSent");
    }

    #[test]
    fn validate_rejects_blank_content() {
        let result = sample_command("   ").validate();

        assert!(result.is_err(), "blank content should be rejected");
    }

    #[test]
    fn request_carries_message_fields() {
        let sender_id = Uuid::now_v7();
        let recipient_id = Uuid::now_v7();
        let request = SendDirectMessageRequest {
            sender_id,
            recipient_id,
            content: "hi".to_string(),
        };

        assert_eq!(request.sender_id, sender_id);
        assert_eq!(request.recipient_id, recipient_id);
        assert_eq!(request.content, "hi");
    }
}
