//! An injectable clock.
//!
//! The registry makes decisions that depend on time: a read-set entry expires
//! after ten minutes. Testing that against the system clock would force `sleep`
//! calls into the tests — so slow, flaky tests. Every read of the current time
//! therefore goes through [`Clock`].

use chrono::{DateTime, Utc};

/// A point in time, always UTC. The journal never stores local time.
pub type Timestamp = DateTime<Utc>;

/// A source of time, injected everywhere a decision depends on the clock.
///
/// `Send + Sync + 'static` because a clock crosses tokio task boundaries. It is
/// a value with no business state: sharing it does not violate the "an actor
/// owns its state" invariant.
pub trait Clock: Send + Sync + 'static {
    /// The current instant.
    fn now(&self) -> Timestamp;
}

/// The system clock. The only implementation used in production.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        Utc::now()
    }
}

#[cfg(any(test, feature = "test-support"))]
mod manual {
    use std::sync::atomic::{AtomicI64, Ordering};

    use chrono::{DateTime, TimeDelta, Utc};

    use super::{Clock, Timestamp};

    /// A hand-driven clock, for tests.
    ///
    /// Time only moves on an explicit call to [`ManualClock::advance`]. A test of
    /// read-set decay then reads as an ordered sequence of events, with no `sleep`
    /// anywhere.
    ///
    /// The state fits in an `AtomicI64`: no `Mutex`, so no `unwrap` on a poisoned
    /// lock.
    #[derive(Debug)]
    pub struct ManualClock {
        millis: AtomicI64,
    }

    impl ManualClock {
        /// A clock frozen at the Unix epoch.
        #[must_use]
        pub fn new() -> Self {
            Self {
                millis: AtomicI64::new(0),
            }
        }

        /// A clock frozen at a given instant.
        #[must_use]
        pub fn at(instant: Timestamp) -> Self {
            Self {
                millis: AtomicI64::new(instant.timestamp_millis()),
            }
        }

        /// Advance the clock. The only way to make time pass.
        pub fn advance(&self, delta: TimeDelta) {
            self.millis
                .fetch_add(delta.num_milliseconds(), Ordering::SeqCst);
        }
    }

    impl Default for ManualClock {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Clock for ManualClock {
        fn now(&self) -> Timestamp {
            DateTime::from_timestamp_millis(self.millis.load(Ordering::SeqCst))
                .unwrap_or(DateTime::<Utc>::UNIX_EPOCH)
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
pub use manual::ManualClock;

#[cfg(test)]
mod tests {
    use chrono::TimeDelta;

    use super::*;

    #[test]
    fn manual_clock_only_advances_when_asked() {
        let clock = ManualClock::new();
        let t0 = clock.now();
        assert_eq!(
            clock.now(),
            t0,
            "the manual clock must not drift on its own"
        );

        clock.advance(TimeDelta::minutes(11));
        assert_eq!(clock.now() - t0, TimeDelta::minutes(11));
    }
}
