pub mod io {
    pub use super::gateway::EventGateway;
}

mod gateway {
    use std::sync::Arc;

    use crate::commanding::assembly::io::CommandError;
    use crate::eventing::assembly::io::{EventDispatchPort, EventError, NewEventEnvelope};
    use crate::eventing::dispatcher::EventDispatcher;
    use crate::eventing::record_events::io::EventRecorder;

    pub enum EventGateway {
        Direct { dispatcher: Arc<EventDispatcher> },
        TwoPhased { recorder: Arc<EventRecorder> },
    }

    impl EventGateway {
        pub fn direct(dispatcher: Arc<EventDispatcher>) -> Self {
            Self::Direct { dispatcher }
        }

        pub fn two_phased(recorder: Arc<EventRecorder>) -> Self {
            Self::TwoPhased { recorder }
        }

        pub fn dispatch(&self, envelope: NewEventEnvelope) -> Result<(), EventError> {
            if envelope.metadata.is_none() {
                return Err(EventError::Conversion(
                    "event_id is required: metadata is missing".into(),
                ));
            }
            match self {
                Self::Direct { dispatcher } => dispatcher.dispatch(&envelope),
                Self::TwoPhased { recorder } => recorder.record(&envelope),
            }
        }
    }

    impl EventDispatchPort for EventGateway {
        fn dispatch(&self, event: NewEventEnvelope) -> Result<(), CommandError> {
            if event.metadata.is_none() {
                return Err(CommandError::EventDispatch(
                    "event_id is required: metadata is missing".into(),
                ));
            }
            match self {
                Self::Direct { dispatcher } => dispatcher
                    .dispatch(&event)
                    .map_err(|e| CommandError::EventDispatch(e.to_string())),
                Self::TwoPhased { recorder } => recorder
                    .record(&event)
                    .map_err(|e| CommandError::EventDispatch(e.to_string())),
            }
        }
    }
}

pub use gateway::EventGateway;
