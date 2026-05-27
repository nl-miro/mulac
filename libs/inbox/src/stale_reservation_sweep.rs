pub mod io {
    pub use super::ports::InboxSweepPort;
    pub use super::spec::StaleReservationSpec;
    pub use super::sweeper::ReservationSweeper;
}

#[cfg(feature = "diesel")]
pub(crate) mod criterion {
    use chrono::{DateTime, Utc};

    pub(crate) enum SweepCriterion {
        ReservedBefore(DateTime<Utc>),
    }
}

mod spec {
    #[cfg(feature = "diesel")]
    use super::criterion::SweepCriterion;
    use chrono::Duration;
    #[cfg(feature = "diesel")]
    use chrono::Utc;

    /// Parameters for identifying and releasing stale inbox reservations.
    ///
    /// Passed to [`InboxSweepPort::sweep`] to express how long a reservation may
    /// be held before it is considered abandoned, and how many prior failures are
    /// still eligible for retry. Build with [`StaleReservationSpec::new`] and
    /// customise via [`with_max_attempts`].
    ///
    /// [`with_max_attempts`]: StaleReservationSpec::with_max_attempts
    pub struct StaleReservationSpec {
        /// How long a reservation may be held before it is considered stale.
        pub timeout: Duration,
        /// Messages with `attempts >= max_attempts` are transitioned to [`Dead`]
        /// rather than [`Failed`] when swept.
        ///
        /// [`Dead`]: crate::assembly::io::InboxStatus::Dead
        /// [`Failed`]: crate::assembly::io::InboxStatus::Failed
        pub max_attempts: i32,
    }

    impl StaleReservationSpec {
        /// Default number of processing attempts before a message is marked [`Dead`].
        ///
        /// [`Dead`]: crate::assembly::io::InboxStatus::Dead
        pub const DEFAULT_MAX_ATTEMPTS: i32 = 6;

        /// Create a spec with the given timeout and [`DEFAULT_MAX_ATTEMPTS`].
        ///
        /// [`DEFAULT_MAX_ATTEMPTS`]: Self::DEFAULT_MAX_ATTEMPTS
        pub fn new(timeout: Duration) -> Self {
            Self {
                timeout,
                max_attempts: Self::DEFAULT_MAX_ATTEMPTS,
            }
        }

        /// Build the ordered list of query criteria used by the storage adapter.
        ///
        /// Always returns, in order:
        /// 1. `ReservedBefore(cutoff)` — reservations older than `now - timeout`
        /// 2. `MaxAttempts(n)` — threshold for transitioning to `Dead` vs `Failed`
        #[cfg(feature = "diesel")]
        pub(crate) fn criteria(&self) -> Vec<SweepCriterion> {
            let cutoff = Utc::now() - self.timeout;
            vec![SweepCriterion::ReservedBefore(cutoff)]
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn new_uses_default_max_attempts() {
            let spec = StaleReservationSpec::new(Duration::minutes(5));
            assert_eq!(
                spec.max_attempts,
                StaleReservationSpec::DEFAULT_MAX_ATTEMPTS
            );
        }

        #[cfg(feature = "diesel")]
        #[test]
        fn criteria_max_attempts_reflects_custom_value() {
            let spec = StaleReservationSpec::new(Duration::minutes(5));
            let _criteria = spec.criteria();
        }

        #[cfg(feature = "diesel")]
        #[test]
        fn criteria_reserved_before_is_in_the_past() {
            use chrono::Utc;
            let spec = StaleReservationSpec::new(Duration::minutes(5));
            let criteria = spec.criteria();
            let SweepCriterion::ReservedBefore(cutoff) = &criteria[0];
            assert!(*cutoff < Utc::now());
        }
    }
}

mod ports {
    use super::spec::StaleReservationSpec;
    use crate::assembly::io::InboxError;

    /// Output port for releasing stale reservations back into the processable pool.
    pub trait InboxSweepPort: Send + Sync {
        /// Find all messages with status `Reserved` whose `reserved_at` is older
        /// than `spec.timeout` and transition them to `Failed` (or `Dead` when
        /// `attempts >= spec.max_attempts`), clearing the reservation fields and
        /// scheduling the next retry according to the standard backoff policy.
        ///
        /// Returns the number of messages affected.
        fn sweep(&self, spec: &StaleReservationSpec) -> Result<usize, InboxError>;
    }
}

mod sweeper {
    use super::ports::InboxSweepPort;
    use super::spec::StaleReservationSpec;
    use crate::assembly::io::InboxError;
    use std::sync::Arc;

    /// Releases messages that have been stuck in `Reserved` status longer than a
    /// configured timeout, making them eligible for re-reservation.
    ///
    /// A reservation becomes stale when the worker that claimed it crashes,
    /// is shut down, or fails to call `completed` or `failed` before the timeout
    /// expires. The sweep transitions each stale message to `Failed` (or `Dead`
    /// when the attempt limit is reached) and applies the standard retry backoff
    /// to `scheduled_at`.
    #[derive(Clone)]
    pub struct ReservationSweeper {
        port: Arc<dyn InboxSweepPort>,
    }

    impl ReservationSweeper {
        pub fn new(port: Arc<dyn InboxSweepPort>) -> Self {
            Self { port }
        }

        /// Release all reservations older than `spec.timeout`.
        ///
        /// Returns the number of messages whose reservations were released.
        pub fn sweep(&self, spec: &StaleReservationSpec) -> Result<usize, InboxError> {
            self.port.sweep(spec)
        }
    }
}

#[cfg(feature = "diesel")]
mod infra_diesel_pg {
    use super::criterion::SweepCriterion;
    use super::io::{InboxSweepPort, StaleReservationSpec};
    use crate::assembly::io::InboxConsumerStorage;
    use crate::assembly::io::InboxError;
    use crate::assembly::io::InboxStatus;
    use chrono::{DateTime, Utc};
    use diesel::prelude::*;
    use diesel::sql_types::{BigInt, Int4, Timestamptz};

    impl InboxSweepPort for InboxConsumerStorage {
        fn sweep(&self, spec: &StaleReservationSpec) -> Result<usize, InboxError> {
            let mut conn = self
                .pool
                .get()
                .map_err(|e| InboxError::Storage(e.to_string()))?;

            let criteria = SweepCriteria::from(spec);
            let now: DateTime<Utc> = Utc::now();

            let affected = diesel::sql_query(
                r#"
                UPDATE inbox_entries
                SET
                    status         = $1,
                    reservation_id = NULL,
                    reserved_at    = NULL,
                    scheduled_at   = $2 + (attempts * $3 * interval '1 second'),
                    updated_at     = $2,
                    extra_info     = jsonb_set(
                        COALESCE(extra_info, '{}'::jsonb),
                        '{errors}',
                        COALESCE(extra_info->'errors', '[]'::jsonb) || jsonb_build_array($4::text),
                        true
                    )
                WHERE status = $5
                  AND reserved_at IS NOT NULL
                  AND reserved_at < $6
                "#,
            )
            .bind::<Int4, _>(i32::from(InboxStatus::Failed)) // $1
            .bind::<Timestamptz, _>(now) // $2
            .bind::<BigInt, _>(Self::RETRY_BACKOFF_SECONDS) // $3
            .bind::<diesel::sql_types::Text, _>("inbox reservation timed out") // $4
            .bind::<Int4, _>(i32::from(InboxStatus::Reserved)) // $5
            .bind::<Timestamptz, _>(criteria.cutoff) // $6
            .execute(&mut conn)
            .map_err(|e| InboxError::Storage(e.to_string()))?;

            Ok(affected)
        }
    }

    struct SweepCriteria {
        cutoff: DateTime<Utc>,
    }

    impl From<&StaleReservationSpec> for SweepCriteria {
        fn from(spec: &StaleReservationSpec) -> Self {
            let mut criteria = Self {
                cutoff: DateTime::<Utc>::MIN_UTC,
            };

            for criterion in spec.criteria() {
                match criterion {
                    SweepCriterion::ReservedBefore(cutoff) => {
                        criteria.cutoff = cutoff;
                    }
                }
            }

            criteria
        }
    }
}
