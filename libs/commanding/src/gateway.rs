pub mod io {
    pub use super::gateway::CommandGateway;
}

mod gateway {
    use std::sync::Arc;

    use crate::assembly::io::{CommandError, NewCommandEnvelope};
    use crate::dispatcher::CommandDispatcher;
    use crate::record_commands::io::CommandRecorder;

    pub enum CommandGateway {
        Direct { dispatcher: Arc<CommandDispatcher> },
        TwoPhased { recorder: Arc<CommandRecorder> },
    }

    impl CommandGateway {
        pub fn direct(dispatcher: Arc<CommandDispatcher>) -> Self {
            Self::Direct { dispatcher }
        }

        pub fn two_phased(recorder: Arc<CommandRecorder>) -> Self {
            Self::TwoPhased { recorder }
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
            }
        }
    }
}
