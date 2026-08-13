//! Horloge injectable.
//!
//! Le registre prend des decisions qui dependent du temps : une entree du
//! read-set expire au bout de dix minutes. Tester ca avec l'horloge systeme
//! imposerait des `sleep` dans les tests — donc des tests lents et instables.
//! Toute lecture de l'heure passe donc par [`Clock`].

use chrono::{DateTime, Utc};

/// Un instant, toujours en UTC. Le journal ne stocke jamais d'heure locale.
pub type Timestamp = DateTime<Utc>;

/// Source de temps. Injectee partout ou une decision depend de l'heure.
///
/// `Send + Sync + 'static` parce qu'une horloge traverse les frontieres de
/// tasks tokio. C'est une valeur sans state metier : la partager ne viole pas
/// l'invariant « un acteur possede son state ».
pub trait Clock: Send + Sync + 'static {
    /// L'instant courant.
    fn now(&self) -> Timestamp;
}

/// L'horloge du systeme. La seule implementation utilisee en production.
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

    /// Horloge pilotee a la main, pour les tests.
    ///
    /// Le temps n'avance que sur appel explicite a [`ManualClock::advance`]. Un
    /// test de decroissance du read-set se lit alors comme une suite d'evenements
    /// ordonnes, sans le moindre `sleep`.
    ///
    /// L'state tient dans un `AtomicI64` : pas de `Mutex`, donc pas de `unwrap`
    /// sur un lock empoisonne.
    #[derive(Debug)]
    pub struct ManualClock {
        millis: AtomicI64,
    }

    impl ManualClock {
        /// Une horloge figee a l'epoch Unix.
        #[must_use]
        pub fn new() -> Self {
            Self {
                millis: AtomicI64::new(0),
            }
        }

        /// Une horloge figee a un instant donne.
        #[must_use]
        pub fn at(instant: Timestamp) -> Self {
            Self {
                millis: AtomicI64::new(instant.timestamp_millis()),
            }
        }

        /// Avance l'horloge. Le seul moyen de faire passer le temps.
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
    fn manual_clock_n_avance_que_sur_demande() {
        let clock = ManualClock::new();
        let t0 = clock.now();
        assert_eq!(
            clock.now(),
            t0,
            "l'horloge manuelle ne doit pas deriver toute seule"
        );

        clock.advance(TimeDelta::minutes(11));
        assert_eq!(clock.now() - t0, TimeDelta::minutes(11));
    }
}
