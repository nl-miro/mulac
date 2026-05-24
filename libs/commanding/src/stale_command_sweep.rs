pub mod io {
    pub use super::ports::CommandSweepPort;
    pub use super::spec::StaleCommandSpec;
    pub use super::sweeper::CommandSweeper;
}

mod spec {
    #[cfg(feature = "diesel")]
    use crate::assembly::io::Criterion;
    use chrono::Duration;
    #[cfg(feature = "diesel")]
    use chrono::Utc;

    pub struct StaleCommandSpec {
        pub timeout: Duration,
        pub max_attempts: i32,
    }

    impl StaleCommandSpec {
        pub const DEFAULT_MAX_ATTEMPTS: i32 = 6;

        pub fn new(timeout: Duration) -> Self {
            Self {
                timeout,
                max_attempts: Self::DEFAULT_MAX_ATTEMPTS,
            }
        }

        pub fn with_max_attempts(mut self, max_attempts: i32) -> Self {
            self.max_attempts = max_attempts;
            self
        }

        #[cfg(feature = "diesel")]
        pub(crate) fn criteria(&self) -> Vec<Criterion> {
            let cutoff = Utc::now() - self.timeout;
            vec![Criterion::ReservedBefore(cutoff)]
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn new_uses_default_max_attempts() {
            let spec = StaleCommandSpec::new(Duration::minutes(5));
            assert_eq!(spec.max_attempts, StaleCommandSpec::DEFAULT_MAX_ATTEMPTS);
        }

        #[test]
        fn with_max_attempts_overrides_default() {
            let spec = StaleCommandSpec::new(Duration::minutes(5)).with_max_attempts(3);
            assert_eq!(spec.max_attempts, 3);
        }

        #[cfg(feature = "diesel")]
        #[test]
        fn criteria_reserved_before_is_in_the_past() {
            use chrono::Utc;
            let spec = StaleCommandSpec::new(Duration::minutes(5));
            let criteria = spec.criteria();
            assert_eq!(criteria.len(), 1);
            if let Criterion::ReservedBefore(cutoff) = &criteria[0] {
                assert!(*cutoff < Utc::now());
            } else {
                panic!("first criterion should be ReservedBefore");
            }
        }
    }
}

mod ports {
    use super::spec::StaleCommandSpec;
    use crate::assembly::io::CommandError;

    pub trait CommandSweepPort: Send + Sync {
        fn sweep(&self, spec: &StaleCommandSpec) -> Result<u64, CommandError>;
    }
}

mod sweeper {
    use super::ports::CommandSweepPort;
    use super::spec::StaleCommandSpec;
    use crate::assembly::io::CommandError;
    use std::sync::Arc;

    #[derive(Clone)]
    pub struct CommandSweeper {
        port: Arc<dyn CommandSweepPort>,
    }

    impl CommandSweeper {
        pub fn new(port: Arc<dyn CommandSweepPort>) -> Self {
            Self { port }
        }

        pub fn sweep(&self, spec: &StaleCommandSpec) -> Result<u64, CommandError> {
            self.port.sweep(spec)
        }
    }
}

#[cfg(feature = "diesel")]
mod infra_diesel_pg {
    use super::io::{CommandSweepPort, StaleCommandSpec};
    use crate::assembly::io::{
        CommandConsumerStorage,
        CommandError,
        CommandStatus,
        Criterion,
        //
    };
    use chrono::{DateTime, Utc};
    use diesel::prelude::*;
    use diesel::sql_types::{BigInt, Int4, Timestamptz};

    impl CommandSweepPort for CommandConsumerStorage {
        fn sweep(&self, spec: &StaleCommandSpec) -> Result<u64, CommandError> {
            let mut conn = self
                .pool
                .get()
                .map_err(|e| CommandError::Storage(e.to_string()))?;

            let criteria = SweepCriteria::from(spec);
            let now: DateTime<Utc> = Utc::now();

            let affected = diesel::sql_query(
                r#"
                UPDATE command_entries
                SET
                    status         = $1,
                    reservation_id = NULL,
                    reserved_at    = NULL,
                    scheduled_at   = $2 + (attempts * $3 * interval '1 second'),
                    updated_at     = $2
                WHERE status = $4
                  AND reserved_at IS NOT NULL
                  AND reserved_at < $5
                "#,
            )
            .bind::<Int4, _>(i32::from(CommandStatus::Failed)) // $1
            .bind::<Timestamptz, _>(now) // $2
            .bind::<BigInt, _>(CommandConsumerStorage::RETRY_BACKOFF_SECONDS) // $3
            .bind::<Int4, _>(i32::from(CommandStatus::Reserved)) // $4
            .bind::<Timestamptz, _>(criteria.cutoff) // $5
            .execute(&mut conn)
            .map_err(|e| CommandError::Storage(e.to_string()))?;

            Ok(affected as u64)
        }
    }

    struct SweepCriteria {
        cutoff: DateTime<Utc>,
    }

    impl From<&StaleCommandSpec> for SweepCriteria {
        fn from(spec: &StaleCommandSpec) -> Self {
            let mut criteria = Self {
                cutoff: DateTime::<Utc>::MIN_UTC,
            };

            for criterion in spec.criteria() {
                match criterion {
                    Criterion::ReservedBefore(cutoff) => {
                        criteria.cutoff = cutoff;
                    }
                    _ => {}
                }
            }

            criteria
        }
    }
}
