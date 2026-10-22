pub mod io {
    pub use super::gateway::CommandGateway;
}

mod gateway {
    use crate::assembly::io::{CommandError, NewCommandEnvelope};
    use crate::dispatcher::CommandDispatcher;
    use crate::record_commands::io::CommandRecorder;
    use std::sync::{Arc, Weak};

    pub enum CommandGateway {
        Direct {
            dispatcher: Arc<CommandDispatcher>,
        },
        TwoPhased {
            recorder: Arc<CommandRecorder>,
        },
        /// Non-owning handle used to resolve direct subscriber dependency cycles.
        Reference {
            gateway: Weak<CommandGateway>,
        },
    }

    impl CommandGateway {
        pub fn direct(dispatcher: Arc<CommandDispatcher>) -> Self {
            Self::Direct { dispatcher }
        }

        pub fn two_phased(recorder: Arc<CommandRecorder>) -> Self {
            Self::TwoPhased { recorder }
        }

        /// The target must be initialized and alive when this handle dispatches.
        pub fn reference(gateway: Weak<Self>) -> Self {
            Self::Reference { gateway }
        }

        pub fn dispatch(&self, envelope: NewCommandEnvelope) -> Result<(), CommandError> {
            if envelope.metadata.is_none() {
                return Err(CommandError::Conversion(
                    "command_id is required: metadata is missing".into(),
                ));
            }
            match self {
                Self::Direct { dispatcher } => dispatcher.dispatch(&envelope),
                Self::TwoPhased { recorder } => recorder.record(&envelope),
                Self::Reference { gateway } => gateway
                    .upgrade()
                    .ok_or_else(|| {
                        CommandError::HandlerExecution(
                            "command gateway is not initialized or has been dropped".into(),
                        )
                    })?
                    .dispatch(envelope),
            }
        }
    }
}
