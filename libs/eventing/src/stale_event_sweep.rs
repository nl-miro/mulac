pub mod io {
    pub use super::ports::EventSweepPort;
    pub use super::spec::StaleEventSpec;
    pub use super::sweeper::EventSweeper;
}

mod spec {
    #[cfg(feature = "diesel")]
    use crate::assembly::io::Criterion;
    use chrono::Duration;
    #[cfg(feature = "diesel")]
    use chrono::Utc;

    pub struct StaleEventSpec {
        pub timeout: Duration,
        pub max_attempts: i32,
    }

    impl StaleEventSpec {
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
            let spec = StaleEventSpec::new(Duration::minutes(5));
            assert_eq!(spec.max_attempts, StaleEventSpec::DEFAULT_MAX_ATTEMPTS);
        }

        #[test]
        fn with_max_attempts_overrides_default() {
            let spec = StaleEventSpec::new(Duration::minutes(5)).with_max_attempts(3);
            assert_eq!(spec.max_attempts, 3);
        }

        #[cfg(feature = "diesel")]
        #[test]
        fn criteria_reserved_before_is_in_the_past() {
            use chrono::Utc;
            let spec = StaleEventSpec::new(Duration::minutes(5));
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
    use super::spec::StaleEventSpec;
    use crate::assembly::io::EventError;

    pub trait EventSweepPort: Send + Sync {
        fn sweep(&self, spec: &StaleEventSpec) -> Result<u64, EventError>;
    }
}

mod sweeper {
    use super::ports::EventSweepPort;
    use super::spec::StaleEventSpec;
    use crate::assembly::io::EventError;
    use std::sync::Arc;

    #[derive(Clone)]
    pub struct EventSweeper {
        port: Arc<dyn EventSweepPort>,
    }

    impl EventSweeper {
        pub fn new(port: Arc<dyn EventSweepPort>) -> Self {
            Self { port }
        }

        pub fn sweep(&self, spec: &StaleEventSpec) -> Result<u64, EventError> {
            self.port.sweep(spec)
        }
    }
}
