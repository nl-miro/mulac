pub mod io {
    pub use super::consumer::InboxConsumer;
    pub use super::ports::InboxReservePort;
    pub use super::repository::InboxConsumerRepository;
    pub use super::reservable::ReservableInboxSpec;
}

mod reservable {
    #[cfg(feature = "diesel")]
    use crate::assembly::io::{Criterion, InboxStatus};

    /// Parameters for selecting inbox messages eligible for consumption.
    ///
    /// Passed to [`InboxReservePort::reserve`] to express how many messages to
    /// claim and how many prior failures are still acceptable. Build with
    /// [`ReservableInboxSpec::new`] and customise via [`with_max_attempts`].
    ///
    /// [`with_max_attempts`]: ReservableInboxSpec::with_max_attempts
    pub struct ReservableInboxSpec {
        /// Maximum number of messages to reserve in a single call.
        pub limit: usize,
        /// Messages with `attempts >= max_attempts` are excluded from reservation.
        pub max_attempts: i32,
    }

    impl ReservableInboxSpec {
        /// Default number of processing attempts before a message is marked [`Dead`].
        ///
        /// [`Dead`]: crate::assembly::io::InboxStatus::Dead
        pub const DEFAULT_MAX_ATTEMPTS: i32 = 6;

        /// Create a spec with the given limit and [`DEFAULT_MAX_ATTEMPTS`].
        ///
        /// [`DEFAULT_MAX_ATTEMPTS`]: Self::DEFAULT_MAX_ATTEMPTS
        pub fn new(limit: usize) -> Self {
            Self {
                limit,
                max_attempts: Self::DEFAULT_MAX_ATTEMPTS,
            }
        }

        /// Override the maximum number of attempts.
        pub fn with_max_attempts(mut self, max_attempts: i32) -> Self {
            self.max_attempts = max_attempts;
            self
        }

        /// Build the ordered list of query criteria used by the storage adapter.
        ///
        /// Always returns, in order:
        /// 1. `StatusIn([Received, Failed])` — both statuses are eligible
        /// 2. `ScheduledBeforeNow` — only messages past their scheduled time
        /// 3. `MaxAttempts(n)` — exclude messages at the retry limit
        /// 4. `OrderByScheduledAtAsc` — oldest-first processing
        #[cfg(feature = "diesel")]
        pub(crate) fn criteria(&self) -> Vec<Criterion> {
            vec![
                Criterion::StatusIn(vec![InboxStatus::Received, InboxStatus::Failed]),
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
            let spec = ReservableInboxSpec::new(10);
            assert_eq!(spec.limit, 10);
            assert_eq!(spec.max_attempts, ReservableInboxSpec::DEFAULT_MAX_ATTEMPTS);
        }

        #[test]
        fn with_max_attempts_overrides_default() {
            let spec = ReservableInboxSpec::new(5).with_max_attempts(3);
            assert_eq!(spec.limit, 5);
            assert_eq!(spec.max_attempts, 3);
        }

        #[cfg(feature = "diesel")]
        #[test]
        fn criteria_returns_four_entries_in_order() {
            let spec = ReservableInboxSpec::new(10);
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
            let spec = ReservableInboxSpec::new(1).with_max_attempts(3);
            let criteria = spec.criteria();
            assert!(matches!(criteria[2], Criterion::MaxAttempts(3)));
        }

        #[cfg(feature = "diesel")]
        #[test]
        fn criteria_status_in_includes_received_and_failed() {
            let spec = ReservableInboxSpec::new(1);
            let criteria = spec.criteria();
            if let Criterion::StatusIn(statuses) = &criteria[0] {
                assert!(statuses.contains(&InboxStatus::Received));
                assert!(statuses.contains(&InboxStatus::Failed));
                assert_eq!(statuses.len(), 2);
            } else {
                panic!("first criterion should be StatusIn");
            }
        }
    }
}

mod ports {
    use super::reservable::ReservableInboxSpec;
    use crate::assembly::io::{InboxError, InboxMessageEnvelope};

    pub trait InboxReservePort: Send + Sync {
        fn reserve(
            &self,
            spec: &ReservableInboxSpec,
        ) -> Result<Vec<InboxMessageEnvelope>, InboxError>;
    }
}

mod repository {
    use super::ports::InboxReservePort;
    use super::reservable::ReservableInboxSpec;
    use crate::assembly::io::{InboxError, InboxMessageEnvelope, InboxProcessPort};
    use std::sync::Arc;
    use uuid::Uuid;

    /// Repository for the inbox consumer use case.
    ///
    /// Holds separate port references for reservation and processing so each
    /// storage concern can be implemented and replaced independently. Both ports
    /// are `Arc`-wrapped to allow the repository to be cloned across workers.
    #[derive(Clone)]
    pub struct InboxConsumerRepository {
        reserve: Arc<dyn InboxReservePort>,
        process: Arc<dyn InboxProcessPort>,
    }

    impl InboxConsumerRepository {
        pub fn new(reserve: Arc<dyn InboxReservePort>, process: Arc<dyn InboxProcessPort>) -> Self {
            Self { reserve, process }
        }

        pub fn reserve(
            &self,
            spec: &ReservableInboxSpec,
        ) -> Result<Vec<InboxMessageEnvelope>, InboxError> {
            self.reserve.reserve(spec)
        }

        /// Mark a reserved message as successfully processed.
        ///
        /// Returns [`InboxError::ReservationNotOwned`] if `reservation_id` does
        /// not match the current owner, preventing double-completion.
        pub fn completed(&self, id: Uuid, reservation_id: Uuid) -> Result<(), InboxError> {
            self.process.completed(id, reservation_id)
        }

        /// Mark a reserved message as failed and release the reservation.
        ///
        /// The storage adapter applies a backoff delay before the message becomes
        /// eligible for re-reservation and increments the attempt counter.
        /// Once the attempt limit is reached the message transitions to `Dead`.
        pub fn failed(
            &self,
            id: Uuid,
            reservation_id: Uuid,
            max_attempts: i32,
        ) -> Result<(), InboxError> {
            self.process.failed(id, reservation_id, max_attempts)
        }
    }
}

mod conversions {
    use crate::assembly::io::{InboxError, InboxMessageEnvelope};
    use commanding::io::{NewCommand, NewCommandEnvelope, NewCommandMetadata};
    use uuid::Uuid;

    impl TryFrom<InboxMessageEnvelope> for NewCommandEnvelope {
        type Error = InboxError;

        fn try_from(value: InboxMessageEnvelope) -> Result<Self, Self::Error> {
            let payload = value.payload().to_string();
            let command_type = value
                .meta
                .routing_key
                .ok_or_else(|| InboxError::Conversion("missing routing_key".into()))?;
            let metadata = NewCommandMetadata {
                command_id: Uuid::now_v7(),
                correlation_id: value.meta.correlation_id,
                causation_id: value.meta.message_id,
                source: value.meta.source,
            };
            Ok(NewCommandEnvelope {
                command: NewCommand {
                    command_type,
                    payload,
                },
                metadata: Some(metadata),
            })
        }
    }
}

mod consumer {
    use super::repository::InboxConsumerRepository;
    use super::reservable::ReservableInboxSpec;
    use crate::assembly::io::InboxError;
    use commanding::io::CommandGateway;
    use uuid::Uuid;

    pub struct InboxConsumer {
        repository: InboxConsumerRepository,
        next: CommandGateway,
    }

    impl InboxConsumer {
        pub fn new(repository: InboxConsumerRepository, next: CommandGateway) -> Self {
            Self { repository, next }
        }

        pub fn process(&self, spec: &ReservableInboxSpec) -> Result<(), Vec<InboxError>> {
            let messages = match self.repository.reserve(spec) {
                Ok(messages) => messages,
                Err(e) => return Err(vec![e]),
            };

            let mut errors: Vec<InboxError> = vec![];

            for message in messages {
                let id = message.id().to_owned();

                let Some(reservation_id) = message.reservation_id() else {
                    errors.push(InboxError::MissingReservation { id });
                    continue;
                };

                let envelope = match message.try_into() {
                    Ok(env) => env,
                    Err(e) => {
                        self.failed(id, reservation_id, spec.max_attempts)
                            .unwrap_or_else(|err| errors.push(err));
                        errors.push(e);
                        continue;
                    }
                };

                match self.next.dispatch(envelope) {
                    Ok(_) => self
                        .completed(id, reservation_id)
                        .unwrap_or_else(|e| errors.push(e)),
                    Err(e) => {
                        self.failed(id, reservation_id, spec.max_attempts)
                            .unwrap_or_else(|e| errors.push(e));

                        let err = InboxError::PublishFailed(e.to_string());
                        errors.push(err);
                    }
                }
            }

            if errors.is_empty() {
                return Ok(());
            }

            Err(errors)
        }

        pub fn completed(&self, id: Uuid, reservation_id: Uuid) -> Result<(), InboxError> {
            self.repository.completed(id, reservation_id)
        }

        pub fn failed(
            &self,
            id: Uuid,
            reservation_id: Uuid,
            max_attempts: i32,
        ) -> Result<(), InboxError> {
            self.repository.failed(id, reservation_id, max_attempts)
        }
    }
}

#[cfg(feature = "diesel")]
mod infra_diesel_pg {
    use super::io::{InboxReservePort, ReservableInboxSpec};
    use crate::assembly::io::InboxConsumerStorage;
    use crate::assembly::io::InboxEntry;
    use crate::assembly::io::inbox_entries;
    use crate::assembly::io::{Criterion, InboxStatus};
    use crate::assembly::io::{InboxError, InboxMessageEnvelope, InboxProcessPort};
    use chrono::{DateTime, Duration, Utc};
    use diesel::prelude::*;
    use diesel::sql_types::{Array, BigInt, Int4, Uuid as SqlUuid};
    use uuid::Uuid;

    impl InboxReservePort for InboxConsumerStorage {
        fn reserve(
            &self,
            spec: &ReservableInboxSpec,
        ) -> Result<Vec<InboxMessageEnvelope>, InboxError> {
            let mut conn = self
                .pool
                .get()
                .map_err(|e| InboxError::Storage(e.to_string()))?;

            let criteria = ReservationCriteria::from(spec);
            let reservation_id = Uuid::now_v7();

            let entries = diesel::sql_query(
                r#"
                WITH candidates AS (
                    SELECT id
                    FROM inbox_entries
                    WHERE status = ANY($1)
                      AND scheduled_at <= now()
                      AND attempts < $2
                    ORDER BY scheduled_at ASC
                    LIMIT $3
                    FOR UPDATE SKIP LOCKED
                )
                UPDATE inbox_entries AS entries
                SET status = $4,
                    reservation_id = $5,
                    reserved_at = now(),
                    attempts = entries.attempts + 1,
                    updated_at = now()
                FROM candidates
                WHERE entries.id = candidates.id
                RETURNING
                    entries.id,
                    entries.payload,
                    entries.meta,
                    entries.status,
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
            .bind::<Int4, _>(i32::from(InboxStatus::Reserved))
            .bind::<SqlUuid, _>(reservation_id)
            .load::<InboxEntry>(&mut conn)
            .map_err(|e| InboxError::Storage(e.to_string()))?;

            entries
                .into_iter()
                .map(InboxMessageEnvelope::try_from)
                .collect()
        }
    }

    impl InboxProcessPort for InboxConsumerStorage {
        fn completed(&self, id: Uuid, reservation_id: Uuid) -> Result<(), InboxError> {
            let mut conn = self
                .pool
                .get()
                .map_err(|e| InboxError::Storage(e.to_string()))?;

            let updated = diesel::update(
                inbox_entries::table
                    .filter(inbox_entries::id.eq(id))
                    .filter(inbox_entries::reservation_id.eq(reservation_id))
                    .filter(inbox_entries::status.eq(i32::from(InboxStatus::Reserved))),
            )
            .set((
                inbox_entries::status.eq(i32::from(InboxStatus::Completed)),
                inbox_entries::processed_at.eq(diesel::dsl::now),
                inbox_entries::updated_at.eq(diesel::dsl::now),
                inbox_entries::reservation_id.eq(None::<Uuid>),
                inbox_entries::reserved_at.eq(None::<DateTime<Utc>>),
            ))
            .execute(&mut conn)
            .map_err(|e| InboxError::Storage(e.to_string()))?;

            if updated == 0 {
                return Err(InboxError::ReservationNotOwned { id, reservation_id });
            }

            Ok(())
        }

        fn failed(
            &self,
            id: Uuid,
            reservation_id: Uuid,
            max_attempts: i32,
        ) -> Result<(), InboxError> {
            let mut conn = self
                .pool
                .get()
                .map_err(|e| InboxError::Storage(e.to_string()))?;

            conn.transaction::<(), diesel::result::Error, _>(|conn| {
                let attempts = diesel::update(
                    inbox_entries::table
                        .filter(inbox_entries::id.eq(id))
                        .filter(inbox_entries::reservation_id.eq(reservation_id))
                        .filter(inbox_entries::status.eq(i32::from(InboxStatus::Reserved))),
                )
                .set((
                    inbox_entries::updated_at.eq(diesel::dsl::now),
                    inbox_entries::reservation_id.eq(None::<Uuid>),
                    inbox_entries::reserved_at.eq(None::<DateTime<Utc>>),
                ))
                .returning(inbox_entries::attempts)
                .get_result::<i32>(conn)?;

                let status = if attempts >= max_attempts {
                    InboxStatus::Dead
                } else {
                    InboxStatus::Failed
                };

                let retry_delay_seconds =
                    i64::from(attempts.max(1)) * InboxConsumerStorage::RETRY_BACKOFF_SECONDS;
                let scheduled_at = Utc::now() + Duration::seconds(retry_delay_seconds);

                diesel::update(inbox_entries::table.find(id))
                    .set((
                        inbox_entries::status.eq(i32::from(status)),
                        inbox_entries::scheduled_at.eq(scheduled_at),
                        inbox_entries::updated_at.eq(diesel::dsl::now),
                    ))
                    .execute(conn)?;

                Ok(())
            })
            .map_err(|e| match e {
                diesel::result::Error::NotFound => {
                    InboxError::ReservationNotOwned { id, reservation_id }
                }
                e => InboxError::Storage(e.to_string()),
            })?;

            Ok(())
        }
    }

    struct ReservationCriteria {
        statuses: Vec<i32>,
        max_attempts: i32,
    }

    impl From<&ReservableInboxSpec> for ReservationCriteria {
        fn from(spec: &ReservableInboxSpec) -> Self {
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
                    Criterion::ScheduledBeforeNow | Criterion::OrderByScheduledAtAsc => {}
                }
            }

            criteria
        }
    }
}
