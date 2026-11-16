//! The port for the current time.

use time::OffsetDateTime;

#[cfg(any(test, feature = "testing"))]
pub use fixed::FixedClock;

/// The port every use case reads the time through, so that a test decides what time it is.
pub trait Clock: Send + Sync {
    /// The current instant, in UTC.
    fn now(&self) -> OffsetDateTime;
}

/// The clock of the machine.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> OffsetDateTime {
        OffsetDateTime::now_utc()
    }
}

#[cfg(any(test, feature = "testing"))]
mod fixed {
    use std::sync::{Mutex, MutexGuard, PoisonError};

    use time::{Duration, OffsetDateTime};

    use super::Clock;

    /// A clock for tests: it returns the instant it was given and moves only when told.
    /// It moves through a shared reference, so a test keeps one handle while the code
    /// under test holds another.
    #[derive(Debug)]
    pub struct FixedClock {
        now: Mutex<OffsetDateTime>,
    }

    impl FixedClock {
        pub fn new(at: OffsetDateTime) -> Self {
            Self {
                now: Mutex::new(at),
            }
        }

        pub fn set(&self, at: OffsetDateTime) {
            *self.lock() = at;
        }

        /// Moves the clock by `duration`, backwards when it is negative. At either end of
        /// what the type can hold it stops there instead of panicking.
        pub fn advance(&self, duration: Duration) {
            let mut now = self.lock();
            *now = now.saturating_add(duration);
        }

        fn lock(&self) -> MutexGuard<'_, OffsetDateTime> {
            // Nothing panics while the lock is held, so it is never poisoned; taking the
            // value either way only spares an `unwrap`.
            self.now.lock().unwrap_or_else(PoisonError::into_inner)
        }
    }

    impl Clock for FixedClock {
        fn now(&self) -> OffsetDateTime {
            *self.lock()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use time::{Duration, UtcOffset};

    use super::*;

    fn instant() -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(1_800_000_000).unwrap()
    }

    #[test]
    fn the_system_clock_tells_the_current_time_in_utc() {
        let before = OffsetDateTime::now_utc();
        let now = SystemClock.now();
        let after = OffsetDateTime::now_utc();
        assert!(before <= now && now <= after);
        assert_eq!(now.offset(), UtcOffset::UTC);
    }

    #[test]
    fn a_fixed_clock_returns_what_it_was_given() {
        let clock = FixedClock::new(instant());
        assert_eq!(clock.now(), instant());
        assert_eq!(clock.now(), instant());
    }

    #[test]
    fn a_fixed_clock_moves_only_when_told() {
        let clock = FixedClock::new(instant());

        clock.advance(Duration::minutes(15));
        assert_eq!(clock.now(), instant() + Duration::minutes(15));

        clock.advance(Duration::minutes(-20));
        assert_eq!(clock.now(), instant() - Duration::minutes(5));

        clock.set(instant() + Duration::days(30));
        assert_eq!(clock.now(), instant() + Duration::days(30));
    }

    #[test]
    fn a_fixed_clock_is_moved_while_a_use_case_holds_it_as_the_port() {
        let clock = Arc::new(FixedClock::new(instant()));
        let port: Arc<dyn Clock> = clock.clone();

        clock.advance(Duration::days(1));

        assert_eq!(port.now(), instant() + Duration::days(1));
    }

    #[test]
    fn advancing_past_the_last_instant_stops_there() {
        let clock = FixedClock::new(instant());
        clock.advance(Duration::MAX);
        let last = clock.now();
        assert!(last > instant());
        clock.advance(Duration::MAX);
        assert_eq!(clock.now(), last);
    }
}
