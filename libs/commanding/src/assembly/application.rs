pub mod io {
    pub use super::models::{
        Command,
        CommandEnvelope,
        CommandError,
        CommandMetadata,
        NewCommand,
        NewCommandEnvelope,
        NewCommandMetadata,
        //
    };
    pub use super::ports::{CommandProcessPort, CommandStorePort};
}

mod models {
    use crate::assembly::domain::ExtraInfo;
    use uuid::Uuid;

    #[derive(Debug, Clone)]
    pub struct NewCommand {
        pub command_type: String,
        pub payload: String,
    }

    /// Gateway input envelope. The caller must supply `command_id`; no ID
    /// generation occurs inside the system boundary.
    #[derive(Debug, Clone)]
    pub struct NewCommandEnvelope {
        pub command: NewCommand,
        pub metadata: Option<NewCommandMetadata>,
    }

    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
    pub struct NewCommandMetadata {
        pub command_id: Uuid,
        pub correlation_id: Option<Uuid>,
        pub causation_id: Option<Uuid>,
        pub source: Option<String>,
    }

    #[derive(Debug)]
    pub struct Command {
        pub id: Uuid,
        pub reservation_id: Uuid,
        pub command_type: String,
        pub payload: String,
        pub attempts: i32,
        pub extra_info: Option<ExtraInfo>,
    }

    /// Read-side envelope returned to the consumer after reservation.
    ///
    /// Constructed by the infra layer from the stored entry; contains all fields
    /// the consumer needs without exposing infra types.
    #[derive(Debug)]
    pub struct CommandEnvelope {
        pub command: Command,
        pub metadata: Option<CommandMetadata>,
    }

    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
    pub struct CommandMetadata {
        pub command_id: Uuid,
        pub correlation_id: Option<Uuid>,
        pub causation_id: Option<Uuid>,
        pub source: Option<String>,
    }

    #[derive(Debug, thiserror::Error)]
    pub enum CommandError {
        #[error("storage error: {0}")]
        Storage(String),
        #[error("reservation error: {0}")]
        Reservation(String),
        #[error("no handler registered for command type '{0}'")]
        HandlerNotFound(String),
        #[error("handler execution error: {0}")]
        HandlerExecution(String),
        #[error("missing reservation for entry {id}")]
        MissingReservation { id: Uuid },
        #[error("conversion error: {0}")]
        Conversion(String),
        #[error("eventing error: {0}")]
        Eventing(#[from] eventing::io::EventError),
        #[error("Domain error: {0}")]
        Domain(String),
    }
}

mod ports {
    use super::models::{CommandError, NewCommandEnvelope};
    use uuid::Uuid;

    pub trait CommandStorePort: Send + Sync {
        fn record(&self, envelope: &NewCommandEnvelope) -> Result<(), CommandError>;
    }

    pub trait CommandProcessPort: Send + Sync {
        fn completed(&self, id: Uuid, reservation_id: Uuid) -> Result<(), CommandError>;

        fn failed(
            &self,
            id: Uuid,
            reservation_id: Uuid,
            max_attempts: i32,
            reason: Option<String>,
        ) -> Result<(), CommandError>;
    }
}
