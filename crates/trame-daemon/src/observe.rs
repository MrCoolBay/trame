//! The observation channel. **One-way: the daemon speaks, the interface listens.**
//!
//! # Why a channel and not an accessor
//!
//! The interface could poll [`trame_registry::RegistryHandle::snapshot`] in its run loop.
//! Two reasons not to, and the second is the real one:
//!
//! 1. A snapshot gives the **state**, not the **events**. It does not say that a `StaleRead`
//!    verdict was returned, only that a file has a last writer.
//! 2. An accessor invites writing. If the interface holds a `RegistryHandle`, nothing stops
//!    it calling `admit`. **The interface observes, it does not drive** — and the way to
//!    guarantee that is structural: it receives nothing but a `Receiver`.
//!
//! # Losing an observation is acceptable. Losing an admission is not.
//!
//! ADR 0015 says a bounded channel saturates and we wait, because saturating the admission
//! path is a bug that must be seen. **This channel is the exception, and the exception is
//! justified by the direction of the flow**: if the interface falls behind, making the
//! registry wait would let the display slow down an agent's writes. The cure would be worse
//! than the disease.
//!
//! So [`Observer::emit`] never blocks — it counts what it loses, and says so at the first
//! opportunity through [`Observation::Lost`]. A silent loss would show an incomplete feed
//! while presenting it as complete, which is exactly the failure mode this project refuses
//! everywhere else.

use std::path::PathBuf;

use tokio::sync::mpsc;
use trame_agent::Capabilities;
use trame_core::{SessionId, SessionState, Verdict};

/// Capacity of the observation channel.
///
/// Generous on purpose: the bound is not there to apply backpressure — we do not want any
/// here — but to stop a stalled interface from growing an endless queue.
pub const OBSERVE_CAPACITY: usize = 256;

/// Which transport drives a session, **and therefore what is guaranteed**.
///
/// Lives here rather than in `trame-agent` so the interface can name it without depending on
/// the agent crate: the dependency direction stays
/// `core <- journal <- registry <- {agent, vcs} <- daemon <- view <- {tui, gui}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Transport {
    /// ACP. Writes go through admission before the disk.
    Acp,
    /// PTY. **Degraded mode**: nothing can be intercepted before the disk.
    Pty,
    /// No agent attached. The state of a session nobody drives.
    Absent,
}

impl Transport {
    /// True if this session's writes can be intercepted before the disk.
    #[must_use]
    pub const fn can_intercept_writes(self) -> bool {
        matches!(self, Self::Acp)
    }

    /// True if the interface **must** show a degradation banner.
    ///
    /// A user who believes they have the admission guarantee without having it is worse off
    /// than with no tool at all.
    #[must_use]
    pub const fn is_degraded(self) -> bool {
        !self.can_intercept_writes()
    }

    /// The displayable label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Acp => "ACP",
            Self::Pty => "PTY",
            Self::Absent => "none",
        }
    }
}

impl From<Capabilities> for Transport {
    /// Derives the transport from the backend's **real** capabilities.
    ///
    /// We do not guess from the backend type at the call site: `can_intercept_writes`
    /// decides, and nothing else.
    fn from(capabilities: Capabilities) -> Self {
        if capabilities.can_intercept_writes {
            Self::Acp
        } else {
            Self::Pty
        }
    }
}

/// What the daemon gives the interface to show. **Nothing more than what it displays.**
///
/// Deliberately poor: every variant added here is something the interface will be able to
/// show, and therefore a promise made to the user.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Observation {
    /// A session appears, with the transport driving it.
    SessionOpened {
        /// Its identifier.
        session: SessionId,
        /// Its displayable name.
        name: String,
        /// What is guaranteed for it.
        transport: Transport,
    },
    /// Its state changed.
    StateChanged {
        /// The session concerned.
        session: SessionId,
        /// Its new state.
        state: SessionState,
    },
    /// A read entered the read-set.
    Read {
        /// The session that read.
        session: SessionId,
        /// The path read, relative to the project root.
        path: PathBuf,
    },
    /// An **admitted** write, with the verdict returned.
    Write {
        /// The session that wrote.
        session: SessionId,
        /// The path written, relative to the project root.
        path: PathBuf,
        /// The verdict. `StaleRead` is the only one worth seeing.
        verdict: Verdict,
    },
    /// A **refused** write, with the reason passed back to the agent.
    Refused {
        /// The session that asked.
        session: SessionId,
        /// The path refused.
        path: PathBuf,
        /// The reason, exactly as the agent received it.
        reason: String,
    },
    /// A notice placed in front of the session's next message.
    Notice {
        /// The session that was told.
        session: SessionId,
        /// The exact text injected.
        text: String,
    },
    /// An **out-of-band** write, seen by the watcher after the fact.
    ///
    /// **No verdict, and the interface must not invent one**: nobody admitted it. The
    /// watcher observes, it prevents nothing.
    ExternalWrite {
        /// The path observed, relative to the project root.
        path: PathBuf,
    },
    /// ★ Notices that `Grep` reads **would** have produced, if they counted.
    ///
    /// **These are not notices.** Nothing was injected, no agent was told. This is the
    /// missing data for deciding whether the read hole can close without crying wolf
    /// (ADR 0027), and the interface must show it **distinctly** from real notices —
    /// otherwise it announces a coverage that does not exist.
    PotentialNotices {
        /// The running total since the project was opened.
        total: u64,
    },
    /// Observations were lost for want of room.
    ///
    /// The interface displays it: a feed with holes, presented as complete, would be a lie.
    Lost {
        /// How many.
        count: u64,
    },
}

/// The sending end of the observation channel.
///
/// Cloneable: each session pilot and the watcher hold one.
#[derive(Debug)]
pub struct Observer {
    tx: mpsc::Sender<Observation>,
    /// What could not be sent, waiting to be reported.
    dropped: u64,
}

impl Clone for Observer {
    /// The loss counter **is not cloned**: it belongs to the sender that lost. Duplicating it
    /// would count the same loss twice.
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            dropped: 0,
        }
    }
}

/// Creates the observation channel.
///
/// Returns the sender to the daemon and the receiver to the interface. **The receiver allows
/// nothing but listening**, and that is the guarantee that the interface does not drive.
#[must_use]
pub fn observe_channel() -> (Observer, mpsc::Receiver<Observation>) {
    let (tx, rx) = mpsc::channel(OBSERVE_CAPACITY);
    (Observer { tx, dropped: 0 }, rx)
}

impl Observer {
    /// Sends an observation. **Never blocks, never returns an error.**
    ///
    /// If the channel is full or closed, the observation is lost and counted. The count is
    /// sent as soon as room frees up, through [`Observation::Lost`].
    pub fn emit(&mut self, observation: Observation) {
        // Losses go first: otherwise a counter would climb without ever being displayed, and
        // the gap would stay invisible — precisely what we are trying to avoid.
        if self.dropped > 0
            && self
                .tx
                .try_send(Observation::Lost {
                    count: self.dropped,
                })
                .is_ok()
        {
            self.dropped = 0;
        }
        if self.tx.try_send(observation).is_err() {
            self.dropped = self.dropped.saturating_add(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pty_transport_is_degraded_and_acp_is_not() {
        assert!(Transport::Pty.is_degraded(), "PTY intercepts nothing");
        assert!(
            Transport::Absent.is_degraded(),
            "with no agent, nothing is guaranteed"
        );
        assert!(!Transport::Acp.is_degraded());
        assert!(Transport::Acp.can_intercept_writes());
    }

    #[test]
    fn transport_is_derived_from_the_real_capabilities() {
        assert_eq!(Transport::from(Capabilities::acp()), Transport::Acp);
        assert_eq!(Transport::from(Capabilities::pty()), Transport::Pty);
    }

    #[tokio::test]
    async fn a_full_channel_never_blocks_and_declares_its_losses() {
        let (mut observer, mut rx) = observe_channel();
        let path = PathBuf::from("auth.rs");

        // Fill the channel exactly, then overshoot by three.
        for _ in 0..OBSERVE_CAPACITY + 3 {
            observer.emit(Observation::ExternalWrite { path: path.clone() });
        }
        assert_eq!(observer.dropped, 3, "three observations lost, and counted");

        // Free exactly one slot. It is used to **declare the loss**, not to send the next
        // observation — which is therefore lost in its turn. That is the right order:
        // better to know that you do not know.
        rx.recv().await.unwrap();
        observer.emit(Observation::ExternalWrite { path: path.clone() });
        let mut seen = Vec::new();
        while let Ok(observation) = rx.try_recv() {
            seen.push(observation);
        }
        assert!(
            seen.contains(&Observation::Lost { count: 3 }),
            "a silent loss would present a feed with holes as complete"
        );
        assert_eq!(observer.dropped, 1, "the new loss is counted in its turn");

        // Empty channel: the next emission goes through, and the counter settles.
        observer.emit(Observation::ExternalWrite { path });
        assert_eq!(rx.try_recv().unwrap(), Observation::Lost { count: 1 });
        assert!(matches!(
            rx.try_recv().unwrap(),
            Observation::ExternalWrite { .. }
        ));
        assert_eq!(
            observer.dropped, 0,
            "the counter settles once room comes back"
        );
    }

    /// A cloned `Observer` must not inherit its parent's losses: the same loss would be
    /// reported twice, and the interface would show a gap that does not exist.
    #[test]
    fn a_cloned_observer_does_not_inherit_past_losses() {
        let (mut observer, _rx) = observe_channel();
        for _ in 0..OBSERVE_CAPACITY + 1 {
            observer.emit(Observation::Lost { count: 1 });
        }
        assert_eq!(observer.dropped, 1);
        assert_eq!(observer.clone().dropped, 0);
    }

    /// A closed channel must not panic the sender: the interface can close while a session
    /// is running, and that is not a daemon error.
    #[tokio::test]
    async fn a_closed_receiver_breaks_nothing() {
        let (mut observer, rx) = observe_channel();
        drop(rx);
        observer.emit(Observation::Lost { count: 1 });
        assert_eq!(observer.dropped, 1);
    }
}
