pub mod io {
    pub use super::consumer::CommandConsumer;
    pub use super::ports::CommandReservePort;
    pub use super::repository::CommandConsumerRepository;
    pub use super::reservable::ReservableCommandSpec;
}

mod reservable {
    #[cfg(feature = "diesel")]
    use crate::assembly::io::{CommandStatus, Criterion};

    /// Parameters for selecting command entries eligible for consumption.
    pub struct ReservableCommandSpec {
        /// Maximum number of entries to reserve in a single call.
        pub limit: usize,
        /// Entries with `attempts >= max_attempts` are excluded from reservation.
        pub max_attempts: i32,
    }

    impl ReservableCommandSpec {
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
                Criterion::StatusIn(vec![CommandStatus::Received, CommandStatus::Failed]),
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
            let spec = ReservableCommandSpec::new(10);
            assert_eq!(spec.limit, 10);
            assert_eq!(
                spec.max_attempts,
                ReservableCommandSpec::DEFAULT_MAX_ATTEMPTS
            );
        }

        #[test]
        fn with_max_attempts_overrides_default() {
            let spec = ReservableCommandSpec::new(5).with_max_attempts(3);
            assert_eq!(spec.limit, 5);
            assert_eq!(spec.max_attempts, 3);
        }

        #[cfg(feature = "diesel")]
        #[test]
        fn criteria_returns_four_entries_in_order() {
            let spec = ReservableCommandSpec::new(10);
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
            let spec = ReservableCommandSpec::new(1).with_max_attempts(3);
            let criteria = spec.criteria();
            assert!(matches!(criteria[2], Criterion::MaxAttempts(3)));
        }

        #[cfg(feature = "diesel")]
        #[test]
        fn criteria_status_in_includes_received_and_failed() {
            let spec = ReservableCommandSpec::new(1);
            let criteria = spec.criteria();
            if let Criterion::StatusIn(statuses) = &criteria[0] {
                assert!(statuses.contains(&CommandStatus::Received));
                assert!(statuses.contains(&CommandStatus::Failed));
                assert_eq!(statuses.len(), 2);
            } else {
                panic!("first criterion should be StatusIn");
            }
        }
    }
}

mod ports {
    use crate::assembly::io::{CommandEnvelope, CommandError};

    use super::reservable::ReservableCommandSpec;

    pub trait CommandReservePort: Send + Sync {
        fn reserve(
            &self,
            spec: &ReservableCommandSpec,
        ) -> Result<Vec<CommandEnvelope>, CommandError>;
    }
}

mod repository {
    use std::sync::Arc;

    use uuid::Uuid;

    use crate::assembly::io::{
        CommandEnvelope,
        CommandError,
        CommandProcessPort, //
    };

    use super::ports::CommandReservePort;
    use super::reservable::ReservableCommandSpec;

    #[derive(Clone)]
    pub struct CommandConsumerRepository {
        reserve: Arc<dyn CommandReservePort>,
        process: Arc<dyn CommandProcessPort>,
    }

    impl CommandConsumerRepository {
        pub fn new(
            reserve: Arc<dyn CommandReservePort>,
            process: Arc<dyn CommandProcessPort>,
        ) -> Self {
            Self { reserve, process }
        }

        pub fn reserve(
            &self,
            spec: &ReservableCommandSpec,
        ) -> Result<Vec<CommandEnvelope>, CommandError> {
            self.reserve.reserve(spec)
        }

        pub fn completed(&self, id: Uuid, reservation_id: Uuid) -> Result<(), CommandError> {
            self.process.completed(id, reservation_id)
        }

        pub fn failed(
            &self,
            id: Uuid,
            reservation_id: Uuid,
            max_attempts: i32,
        ) -> Result<(), CommandError> {
            self.process.failed(id, reservation_id, max_attempts)
        }
    }
}

mod conversions {
    use crate::assembly::io::{
        CommandEnvelope,
        CommandMetadata,
        NewCommandEnvelope,
        NewCommandMetadata, //
    };

    impl From<&CommandMetadata> for NewCommandMetadata {
        fn from(meta: &CommandMetadata) -> Self {
            NewCommandMetadata {
                command_id: meta.command_id,
                correlation_id: meta.correlation_id,
                causation_id: meta.causation_id,
                source: meta.source.clone(),
            }
        }
    }

    impl From<&CommandEnvelope> for NewCommandEnvelope {
        fn from(envelope: &CommandEnvelope) -> Self {
            NewCommandEnvelope {
                command_type: envelope.command_type.clone(),
                payload: envelope.payload.clone(),
                metadata: envelope.metadata.as_ref().map(NewCommandMetadata::from),
            }
        }
    }
}

mod consumer {
    use std::sync::Arc;

    use crate::assembly::io::{CommandEnvelope, CommandError};
    use crate::dispatcher::CommandDispatcher;

    use super::repository::CommandConsumerRepository;
    use super::reservable::ReservableCommandSpec;

    pub struct CommandConsumer {
        repository: CommandConsumerRepository,
        dispatcher: Arc<CommandDispatcher>,
    }

    impl CommandConsumer {
        pub fn new(
            repository: CommandConsumerRepository,
            dispatcher: Arc<CommandDispatcher>,
        ) -> Self {
            Self {
                repository,
                dispatcher,
            }
        }

        pub fn consume(&self, spec: &ReservableCommandSpec) -> Result<(), Vec<CommandError>> {
            let entries = match self.repository.reserve(spec) {
                Ok(entries) => entries,
                Err(e) => return Err(vec![e]),
            };

            let mut errors: Vec<CommandError> = vec![];

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
            entry: &CommandEnvelope,
            spec: &ReservableCommandSpec,
            errors: &mut Vec<CommandError>,
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

#[cfg(feature = "diesel")]
mod infra_diesel_pg {
    use chrono::{DateTime, Duration, Utc};
    use diesel::prelude::*;
    use diesel::sql_types::{Array, BigInt, Int4, Uuid as SqlUuid};
    use uuid::Uuid;

    use crate::assembly::io::{
        CommandConsumerStorage,
        CommandEntry,
        CommandEnvelope,
        CommandError,
        CommandProcessPort,
        CommandStatus,
        Criterion,
        command_entries, //
    };

    use super::io::{CommandReservePort, ReservableCommandSpec};

    impl CommandReservePort for CommandConsumerStorage {
        fn reserve(
            &self,
            spec: &ReservableCommandSpec,
        ) -> Result<Vec<CommandEnvelope>, CommandError> {
            let mut conn = self
                .pool
                .get()
                .map_err(|e| CommandError::Reservation(e.to_string()))?;

            let criteria = ReservationCriteria::from(spec);
            let reservation_id = Uuid::now_v7();

            let entries = diesel::sql_query(
                r#"
                WITH candidates AS (
                    SELECT id
                    FROM command_entries
                    WHERE status = ANY($1)
                      AND scheduled_at <= now()
                      AND attempts < $2
                    ORDER BY scheduled_at ASC
                    LIMIT $3
                    FOR UPDATE SKIP LOCKED
                )
                UPDATE command_entries AS entries
                SET status = $4,
                    reservation_id = $5,
                    reserved_at = now(),
                    attempts = entries.attempts + 1,
                    updated_at = now()
                FROM candidates
                WHERE entries.id = candidates.id
                RETURNING
                    entries.id,
                    entries.command_type,
                    entries.status,
                    entries.payload,
                    entries.meta,
                    entries.scheduled_at,
                    entries.attempts,
                    entries.reservation_id,
                    entries.reserved_at,
                    entries.received_at,
                    entries.updated_at,
                    entries.processed_at
                "#,
            )
            .bind::<Array<Int4>, _>(criteria.statuses)
            .bind::<Int4, _>(criteria.max_attempts)
            .bind::<BigInt, _>(spec.limit as i64)
            .bind::<Int4, _>(i32::from(CommandStatus::Reserved))
            .bind::<SqlUuid, _>(reservation_id)
            .load::<CommandEntry>(&mut conn)
            .map_err(|e| CommandError::Reservation(e.to_string()))?;

            entries.into_iter().map(CommandEnvelope::try_from).collect()
        }
    }

    impl CommandProcessPort for CommandConsumerStorage {
        fn completed(&self, id: Uuid, reservation_id: Uuid) -> Result<(), CommandError> {
            let mut conn = self
                .pool
                .get()
                .map_err(|e| CommandError::Storage(e.to_string()))?;

            let updated = diesel::update(
                command_entries::table
                    .filter(command_entries::id.eq(id))
                    .filter(command_entries::reservation_id.eq(reservation_id))
                    .filter(command_entries::status.eq(i32::from(CommandStatus::Reserved))),
            )
            .set((
                command_entries::status.eq(i32::from(CommandStatus::Completed)),
                command_entries::processed_at.eq(diesel::dsl::now),
                command_entries::updated_at.eq(diesel::dsl::now),
                command_entries::reservation_id.eq(None::<Uuid>),
                command_entries::reserved_at.eq(None::<DateTime<Utc>>),
            ))
            .execute(&mut conn)
            .map_err(|e| CommandError::Storage(e.to_string()))?;

            if updated == 0 {
                return Err(CommandError::MissingReservation { id });
            }

            Ok(())
        }

        fn failed(
            &self,
            id: Uuid,
            reservation_id: Uuid,
            max_attempts: i32,
        ) -> Result<(), CommandError> {
            let mut conn = self
                .pool
                .get()
                .map_err(|e| CommandError::Storage(e.to_string()))?;

            conn.transaction::<(), diesel::result::Error, _>(|conn| {
                let attempts = diesel::update(
                    command_entries::table
                        .filter(command_entries::id.eq(id))
                        .filter(command_entries::reservation_id.eq(reservation_id))
                        .filter(command_entries::status.eq(i32::from(CommandStatus::Reserved))),
                )
                .set((
                    command_entries::updated_at.eq(diesel::dsl::now),
                    command_entries::reservation_id.eq(None::<Uuid>),
                    command_entries::reserved_at.eq(None::<DateTime<Utc>>),
                ))
                .returning(command_entries::attempts)
                .get_result::<i32>(conn)?;

                let status = if attempts >= max_attempts {
                    CommandStatus::Dead
                } else {
                    CommandStatus::Failed
                };

                let retry_delay_seconds =
                    i64::from(attempts.max(1)) * CommandConsumerStorage::RETRY_BACKOFF_SECONDS;
                let retry_delay_seconds =
                    retry_delay_seconds.min(CommandConsumerStorage::MAX_RETRY_BACKOFF_SECONDS);
                let scheduled_at = Utc::now() + Duration::seconds(retry_delay_seconds);

                diesel::update(command_entries::table.find(id))
                    .set((
                        command_entries::status.eq(i32::from(status)),
                        command_entries::scheduled_at.eq(scheduled_at),
                        command_entries::updated_at.eq(diesel::dsl::now),
                    ))
                    .execute(conn)?;

                Ok(())
            })
            .map_err(|e| match e {
                diesel::result::Error::NotFound => CommandError::MissingReservation { id },
                e => CommandError::Storage(e.to_string()),
            })?;

            Ok(())
        }
    }

    struct ReservationCriteria {
        statuses: Vec<i32>,
        max_attempts: i32,
    }

    impl From<&ReservableCommandSpec> for ReservationCriteria {
        fn from(spec: &ReservableCommandSpec) -> Self {
            let mut criteria = Self {
                statuses: vec![],
                max_attempts: spec.max_attempts,
            };

            for criterion in spec.criteria() {
                match criterion {
                    Criterion::StatusIn(statuses) => {
                        criteria.statuses = statuses.into_iter().map(i32::from).collect();
                    }
                    Criterion::MaxAttempts(max) => {
                        criteria.max_attempts = max;
                    }
                    Criterion::ScheduledBeforeNow
                    | Criterion::OrderByScheduledAtAsc
                    | Criterion::ReservedBefore(_) => {}
                }
            }

            criteria
        }
    }
}
