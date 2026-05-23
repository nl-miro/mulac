pub mod io {
    pub use super::consumer::EventConsumer;
    pub use super::ports::EventReservePort;
    pub use super::repository::EventConsumerRepository;
    pub use super::reservable::ReservableEventSpec;
}

mod reservable {
    #[cfg(feature = "diesel")]
    use crate::assembly::io::{Criterion, EventStatus};

    /// Parameters for selecting event entries eligible for consumption.
    pub struct ReservableEventSpec {
        /// Maximum number of entries to reserve in a single call.
        pub limit: usize,
        /// Entries with `attempts >= max_attempts` are excluded from reservation.
        pub max_attempts: i32,
    }

    impl ReservableEventSpec {
        pub const DEFAULT_MAX_ATTEMPTS: i32 = 6;

        pub fn new(limit: usize) -> Self {
            Self {
                limit,
                max_attempts: Self::DEFAULT_MAX_ATTEMPTS,
            }
        }

        pub fn with_max_attempts(mut self, max_attempts: i32) -> Self {
            self.max_attempts = max_attempts;
            self
        }

        #[cfg(feature = "diesel")]
        pub(crate) fn criteria(&self) -> Vec<Criterion> {
            vec![
                Criterion::StatusIn(vec![EventStatus::Received, EventStatus::Failed]),
                Criterion::ScheduledBeforeNow,
                Criterion::MaxAttempts(self.max_attempts),
                Criterion::OrderByScheduledAtAsc,
            ]
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn new_uses_default_max_attempts() {
            let spec = ReservableEventSpec::new(10);
            assert_eq!(spec.limit, 10);
            assert_eq!(spec.max_attempts, ReservableEventSpec::DEFAULT_MAX_ATTEMPTS);
        }

        #[test]
        fn with_max_attempts_overrides_default() {
            let spec = ReservableEventSpec::new(5).with_max_attempts(3);
            assert_eq!(spec.limit, 5);
            assert_eq!(spec.max_attempts, 3);
        }

        #[cfg(feature = "diesel")]
        #[test]
        fn criteria_returns_four_entries_in_order() {
            let spec = ReservableEventSpec::new(10);
            let criteria = spec.criteria();
            assert_eq!(criteria.len(), 4);
            assert!(matches!(criteria[0], Criterion::StatusIn(_)));
            assert!(matches!(criteria[1], Criterion::ScheduledBeforeNow));
            assert!(matches!(criteria[2], Criterion::MaxAttempts(6)));
            assert!(matches!(criteria[3], Criterion::OrderByScheduledAtAsc));
        }

        #[cfg(feature = "diesel")]
        #[test]
        fn criteria_max_attempts_reflects_custom_value() {
            let spec = ReservableEventSpec::new(1).with_max_attempts(3);
            let criteria = spec.criteria();
            assert!(matches!(criteria[2], Criterion::MaxAttempts(3)));
        }

        #[cfg(feature = "diesel")]
        #[test]
        fn criteria_status_in_includes_received_and_failed() {
            let spec = ReservableEventSpec::new(1);
            let criteria = spec.criteria();
            if let Criterion::StatusIn(statuses) = &criteria[0] {
                assert!(statuses.contains(&EventStatus::Received));
                assert!(statuses.contains(&EventStatus::Failed));
                assert_eq!(statuses.len(), 2);
            } else {
                panic!("first criterion should be StatusIn");
            }
        }
    }
}

mod ports {
    use crate::assembly::io::{EventEnvelope, EventError};

    use super::reservable::ReservableEventSpec;

    pub trait EventReservePort: Send + Sync {
        fn reserve(&self, spec: &ReservableEventSpec) -> Result<Vec<EventEnvelope>, EventError>;
    }
}

mod repository {
    use std::sync::Arc;

    use uuid::Uuid;

    use crate::assembly::io::{
        EventEnvelope,
        EventError,
        EventProcessPort, //
    };

    use super::ports::EventReservePort;
    use super::reservable::ReservableEventSpec;

    #[derive(Clone)]
    pub struct EventConsumerRepository {
        reserve: Arc<dyn EventReservePort>,
        process: Arc<dyn EventProcessPort>,
    }

    impl EventConsumerRepository {
        pub fn new(reserve: Arc<dyn EventReservePort>, process: Arc<dyn EventProcessPort>) -> Self {
            Self { reserve, process }
        }

        pub fn reserve(
            &self,
            spec: &ReservableEventSpec,
        ) -> Result<Vec<EventEnvelope>, EventError> {
            self.reserve.reserve(spec)
        }

        pub fn completed(&self, id: Uuid, reservation_id: Uuid) -> Result<(), EventError> {
            self.process.completed(id, reservation_id)
        }

        pub fn failed(
            &self,
            id: Uuid,
            reservation_id: Uuid,
            max_attempts: i32,
        ) -> Result<(), EventError> {
            self.process.failed(id, reservation_id, max_attempts)
        }
    }
}

mod conversions {
    use crate::assembly::io::{
        EventEnvelope,
        EventMetadata, //
        NewEventEnvelope,
        NewEventMetadata,
    };

    impl From<&EventMetadata> for NewEventMetadata {
        fn from(meta: &EventMetadata) -> Self {
            NewEventMetadata {
                event_id: meta.event_id,
                correlation_id: meta.correlation_id,
                causation_id: meta.causation_id,
                source: meta.source.clone(),
            }
        }
    }

    impl From<&EventEnvelope> for NewEventEnvelope {
        fn from(envelope: &EventEnvelope) -> Self {
            NewEventEnvelope {
                event_type: envelope.event_type.clone(),
                payload: envelope.payload.clone(),
                metadata: envelope.metadata.as_ref().map(NewEventMetadata::from),
            }
        }
    }
}

mod consumer {
    use super::repository::EventConsumerRepository;
    use super::reservable::ReservableEventSpec;
    use crate::assembly::io::{EventEnvelope, EventError};
    use crate::io::EventDispatcher;
    use std::sync::Arc;

    pub struct EventConsumer {
        repository: EventConsumerRepository,
        dispatcher: Arc<EventDispatcher>,
    }

    impl EventConsumer {
        pub fn new(repository: EventConsumerRepository, dispatcher: Arc<EventDispatcher>) -> Self {
            Self {
                repository,
                dispatcher,
            }
        }

        pub fn consume(&self, spec: &ReservableEventSpec) -> Result<(), Vec<EventError>> {
            let entries = match self.repository.reserve(spec) {
                Ok(entries) => entries,
                Err(e) => return Err(vec![e]),
            };

            let mut errors: Vec<EventError> = vec![];

            for entry in entries {
                self.process_entry(&entry, spec, &mut errors);
            }

            if errors.is_empty() {
                Ok(())
            } else {
                Err(errors)
            }
        }

        fn process_entry(
            &self,
            entry: &EventEnvelope,
            spec: &ReservableEventSpec,
            errors: &mut Vec<EventError>,
        ) {
            let id = entry.id;
            let reservation_id = entry.reservation_id;
            let envelope = entry.into();

            match self.dispatcher.dispatch(&envelope) {
                Ok(()) => {
                    self.repository
                        .completed(id, reservation_id)
                        .unwrap_or_else(|e| errors.push(e));
                }
                Err(e) => {
                    self.repository
                        .failed(id, reservation_id, spec.max_attempts)
                        .unwrap_or_else(|err| errors.push(err));
                    errors.push(e);
                }
            }
        }
    }
}
