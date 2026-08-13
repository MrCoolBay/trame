//! ★ The command channel. **The other direction, and only the other direction.**
//!
//! [`crate::observe`] carries what happened, from the daemon to the interface. This module
//! carries what the user asked for, from the interface to the daemon. Together they are the
//! whole contract between the two, and neither of them is a handle.
//!
//! # Why this is not "just give it the handle"
//!
//! [ADR 0022](../../../docs/adr/0022-decoupage-daemon-gui.md) wrote the answer before the
//! need existed:
//!
//! > The day the interface has to start a session or cancel a turn, the answer is not
//! > "let's give it the handle" but "let's add a command channel", typed, with its own
//! > enumeration of permitted actions.
//!
//! A `RegistryHandle` would carry [`trame_registry::RegistryHandle::admit`], and admission
//! is the one thing an interface must never reach. Not because it would misuse it, but
//! because a write that did not come from an agent has **no provenance** — and a journal row
//! without provenance is worse than no row (invariant 2).
//!
//! [`Command`] therefore has no variant that writes a file, and that is the point: an
//! interface cannot request what the enum cannot express.
//!
//! # ★ Typed is not the same as enforced
//!
//! An enum with no `Admit` variant stops nobody who also holds a `RegistryHandle`. The
//! enforcement is a **crate boundary**: an interface that does not depend on
//! `trame-registry` cannot name `admit`, whatever it wants.
//!
//! That boundary did not hold when this module was written. `trame-view` depended on
//! `trame-registry` and called `admit` six times — in scenario code, not in the `App`, so
//! the display state really was handle-free. But the door was open, and ADR 0022 claimed it
//! was shut. This channel exists so it can actually be shut, and
//! `just check-interface-boundary` is what keeps it shut.
//!
//! # Capacity, and why saturation is different here
//!
//! [`crate::observe`] drops observations rather than block: an interface that stalls must
//! never slow an agent's write down. Commands are the opposite. A user action is **rare and
//! consequential** — a prompt typed by hand, a session started — and silently dropping one
//! would be a bug the user experiences as the application ignoring them.
//!
//! So this channel **waits** when full, and its capacity is small on purpose: 32 pending
//! user actions already means something is wrong upstream.

use tokio::sync::mpsc;
use trame_core::{Harness, SessionId};

/// How many user actions can be in flight.
///
/// Small on purpose. Unlike observations, commands come from a human at human speed: 32
/// queued actions is not backpressure, it is a symptom.
pub const COMMAND_CAPACITY: usize = 32;

/// ★ What an interface is allowed to ask for.
///
/// **Every variant is an intent, never an effect.** `SendPrompt` asks the daemon to prompt a
/// session; it does not write a file, and no variant here can. Adding a display means adding
/// an [`crate::Observation`] variant; adding an action means adding a variant here — in both
/// directions the cost is a deliberate decision, which is the reason for the shape.
///
/// `#[non_exhaustive]` so that adding an action is not a breaking change for other consumers,
/// and so an interface must handle the possibility of a command it does not know.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Command {
    /// Start a session on a harness, in the currently open project.
    ///
    /// The daemon chooses the [`SessionId`]: an interface that minted its own would be
    /// deciding identity, which belongs to the side that owns the journal.
    StartSession {
        /// Which agent runs it.
        harness: Harness,
        /// The display name the user typed. Reaches the injected notice, so it matters:
        /// "changed by session refactor-api" is actionable, a UUID is not.
        name: String,
    },

    /// Send a prompt to a running session.
    ///
    /// This is the only variant that carries user text, and it is the reason the channel
    /// waits rather than drops: a prompt silently discarded is the application ignoring a
    /// human.
    SendPrompt {
        /// Which session.
        session: SessionId,
        /// What the user typed, verbatim. The daemon prepends whatever the prompt pipeline
        /// contributes — a stale-read notice, typically — and the interface never composes
        /// that itself.
        text: String,
    },

    /// Stop a session, releasing its agent process.
    ///
    /// Not a cancel: a turn in flight is a tool call the agent is already running, and
    /// interrupting one is a separate problem with its own protocol question.
    StopSession {
        /// Which session.
        session: SessionId,
    },
}

/// The interface's end of the command channel. Cloneable.
///
/// Deliberately **not** `Observer`-shaped: `Observer::emit` never blocks and declares its
/// losses, because losing an observation is acceptable. Losing a command is not, so this side
/// is `async` and waits.
#[derive(Debug, Clone)]
pub struct Commander {
    tx: mpsc::Sender<Command>,
}

impl Commander {
    /// Ask the daemon for something.
    ///
    /// Waits if the channel is full, which is the intended behaviour: see the module docs.
    ///
    /// # Errors
    ///
    /// Fails only if the daemon is gone. An interface that gets this error has lost its
    /// daemon and should say so rather than carry on looking functional.
    pub async fn send(&self, command: Command) -> Result<(), DaemonGone> {
        self.tx.send(command).await.map_err(|_| DaemonGone)
    }
}

/// The daemon is no longer listening for commands.
///
/// One variant, because there is one cause: the receiving task is dead. An interface cannot
/// do anything about it except tell the user, so a richer error would carry no more action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the Trame daemon is no longer accepting commands")]
pub struct DaemonGone;

/// Create the channel. The daemon keeps the receiver, the interface gets the [`Commander`].
#[must_use]
pub fn command_channel() -> (Commander, mpsc::Receiver<Command>) {
    let (tx, rx) = mpsc::channel(COMMAND_CAPACITY);
    (Commander { tx }, rx)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_prompt() -> Command {
        Command::SendPrompt {
            session: SessionId::new(),
            text: "refactor auth".to_owned(),
        }
    }

    /// A command reaches the daemon, and the daemon sees exactly what was sent.
    #[tokio::test]
    async fn a_command_arrives_unchanged() {
        let (commander, mut rx) = command_channel();
        let sent = a_prompt();
        commander.send(sent.clone()).await.expect("daemon alive");
        assert_eq!(rx.recv().await, Some(sent));
    }

    /// ★ A command is never silently dropped — the channel waits instead.
    ///
    /// This is the property that separates commands from observations, and it is worth a test
    /// because the two channels sit next to each other and the wrong one is easy to copy.
    /// `Observer::emit` drops on saturation on purpose; doing that here would make the
    /// application ignore a human.
    #[tokio::test]
    async fn a_full_channel_makes_the_sender_wait_rather_than_drop() {
        let (commander, mut rx) = command_channel();
        for _ in 0..COMMAND_CAPACITY {
            commander.send(a_prompt()).await.expect("daemon alive");
        }

        // One more would block. If it completes, the channel dropped something.
        let extra = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            commander.send(a_prompt()),
        )
        .await;
        assert!(
            extra.is_err(),
            "the command channel must wait when full, not drop: a dropped command is the \
             application ignoring the user"
        );

        // And once the daemon drains one, the waiting send proceeds.
        rx.recv().await.expect("a queued command");
        tokio::time::timeout(
            std::time::Duration::from_millis(500),
            commander.send(a_prompt()),
        )
        .await
        .expect("draining one slot must unblock the sender")
        .expect("daemon alive");
    }

    /// A dead daemon is reported, not swallowed. An interface that keeps looking functional
    /// after losing its daemon is lying to the user.
    #[tokio::test]
    async fn a_dead_daemon_is_reported() {
        let (commander, rx) = command_channel();
        drop(rx);
        assert_eq!(commander.send(a_prompt()).await, Err(DaemonGone));
    }

    /// ★ No command can write a file, and adding one has to say so out loud.
    ///
    /// # How this actually enforces anything
    ///
    /// The match below is **exhaustive with no wildcard**, so adding a `Command` variant
    /// stops this crate from compiling until someone comes here and classifies it. That is
    /// the whole mechanism: it is a compile-time checklist, not a runtime assertion.
    ///
    /// My first version had a `_ => panic!(...)` arm carrying that message, and clippy
    /// rejected it as unreachable — correctly. `#[non_exhaustive]` constrains *other* crates;
    /// inside the defining crate the three variants are all there is, so the wildcard was
    /// dead code. The guard I thought I was adding did not exist, and the lint is what said
    /// so.
    ///
    /// > **A wildcard arm is the opposite of a checklist.** It is what lets a new variant slip
    /// > through, which is exactly what this test exists to prevent.
    ///
    /// If a future variant *does* carry file content, the interface has just gained a path to
    /// admission without provenance, and invariant 2 is gone — so that variant needs an ADR,
    /// not a line in this match.
    #[test]
    fn no_command_carries_a_file_to_write() {
        let session = SessionId::new();
        for command in &[
            Command::StartSession {
                harness: Harness::ClaudeCode,
                name: "refactor-api".to_owned(),
            },
            Command::SendPrompt {
                session,
                text: "hello".to_owned(),
            },
            Command::StopSession { session },
        ] {
            let carries_file_content = match command {
                Command::StartSession { .. } => false,
                // Carries user text, which the daemon passes to an agent. The agent may then
                // ask to write, and that request goes through admission like any other.
                Command::SendPrompt { .. } => false,
                Command::StopSession { .. } => false,
            };
            assert!(
                !carries_file_content,
                "a Command variant carries file content: the interface can now reach the disk \
                 without an agent, so a journal row would exist with no provenance"
            );
        }
    }
}
