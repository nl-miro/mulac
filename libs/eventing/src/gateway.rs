pub mod io {
    pub use super::gateway::EventGateway;
}

mod gateway {
    use crate::assembly::io::{EventDispatchPort, EventError, NewEventEnvelope};
    use crate::dispatcher::io::EventDispatcher;
    use crate::record_events::io::EventRecorder;
    use std::sync::Arc;

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
        fn dispatch(&self, event: NewEventEnvelope) -> Result<(), EventError> {
            if event.metadata.is_none() {
                return Err(EventError::EventDispatch(
                    "event_id is required: metadata is missing".into(),
                ));
            }
            match self {
                Self::Direct { dispatcher } => dispatcher
                    .dispatch(&event)
                    .map_err(|e| EventError::EventDispatch(e.to_string())),
                Self::TwoPhased { recorder } => recorder
                    .record(&event)
                    .map_err(|e| EventError::EventDispatch(e.to_string())),
            }
        }
    }
}
