//! The registry actor. **One per project.**
//!
//! It owns its state; nobody shares it. Communication is `mpsc` in and `oneshot` back,
//! which gives **serialisation and a total order by construction** — with no lock, and no
//! interleaving to reason about.
//!
//! That is necessary, not cosmetic: a verdict answers "did this file change **since** this
//! session read it", which presupposes a total order over the project's reads and writes.
//! A `Mutex` would give mutual exclusion, not order.
//!
//! Two registries never talk to each other: two projects are independent by construction,
//! so no deadlock between them is possible.

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use trame_core::clock::Clock;
use trame_core::{ContentHash, ProjectId, ProjectRoot, SessionId, Verdict};
use trame_journal::{JournalHandle, ReadRecord, WriteOrigin, WriteRecord};

use crate::error::{RegistryError, RegistryGone};
use crate::msg::{ExternalWrite, ReadKind, RegistryMsg, RegistrySnapshot, ShadowStats};
use crate::state::RegistryState;

/// Queue capacity. At two to five sessions per project and an admission handled in
/// microseconds, 64 messages waiting is already generous. Bounded, **never
/// `unbounded_channel`**: an unbounded queue turns an overload into a silent memory leak.
const CHANNEL_CAPACITY: usize = 64;

struct RegistryActor {
    state: RegistryState,
    clock: Arc<dyn Clock>,
    journal: JournalHandle,
    rx: mpsc::Receiver<RegistryMsg>,
}

impl RegistryActor {
    async fn run(mut self) {
        while let Some(msg) = self.rx.recv().await {
            match msg {
                RegistryMsg::RegisterSession {
                    session,
                    name,
                    reply,
                } => {
                    self.state.register_session(session, name);
                    // The caller may have given up: its `oneshot` is closed. That is not
                    // an error, so we ignore it. `let _ =`, never `.unwrap()`.
                    let _ = reply.send(());
                }

                RegistryMsg::RecordRead {
                    session,
                    path,
                    content,
                    kind,
                    reply,
                } => {
                    let now = self.clock.now();
                    let retenue = self.state.record_read(session, &path, &content, kind, now);
                    let _ = reply.send(());

                    // Journal after replying: the journal is a sink. The path journalled
                    // is the normalised key, not the one the agent phrased.
                    if let Some((key, hash)) = retenue {
                        let record = ReadRecord {
                            project: self.state.project(),
                            session,
                            path: key,
                            hash,
                            ts: now,
                        };
                        if self.journal.record_read(record).await.is_err() {
                            tracing::error!("journal injoignable : lecture non enregistree");
                        }
                    }
                }

                RegistryMsg::Admit {
                    session,
                    path,
                    content,
                    reply,
                } => {
                    let outcome = self.admit(session, &path, &content).await;
                    let _ = reply.send(outcome);
                }

                RegistryMsg::ObserveExternalWrite { path, hash, reply } => {
                    let now = self.clock.now();
                    let observation = self.state.observe_external_write(&path, hash, now);
                    // The reply carries what was done: the caller cannot guess an echo,
                    // and if it guesses it will lie.
                    let _ = reply.send(match &observation {
                        Some(observation) => ExternalWrite::Recorded {
                            seq: observation.seq,
                        },
                        None => ExternalWrite::Echo,
                    });

                    // An ignored observation — the echo of a write we made ourselves —
                    // leaves no row. Otherwise the journal would count every admission
                    // twice.
                    if let Some(observation) = observation {
                        let record = WriteRecord {
                            project: self.state.project(),
                            session: SessionId::EXTERNAL,
                            session_name: "hors-bande".to_owned(),
                            seq: observation.seq,
                            path: observation.key,
                            hash_before: observation.hash_before,
                            hash_after: observation.hash,
                            // No verdict: nobody admitted this write.
                            verdict: None,
                            origin: WriteOrigin::Observed,
                            ts: now,
                        };
                        if self.journal.record_write(record).await.is_err() {
                            tracing::error!(
                                "journal injoignable : ecriture hors-bande non enregistree"
                            );
                        }
                    }
                }

                // ★ Shadow mode: we record, we say nothing. No effect on verdicts — that
                // is the condition for the measurement to be a measurement (ADR 0027).
                RegistryMsg::RecordShadowRead {
                    session,
                    path,
                    content,
                    result_size,
                    reply,
                } => {
                    let now = self.clock.now();
                    self.state
                        .record_shadow_read(session, &path, &content, result_size, now);
                    let _ = reply.send(());
                }

                RegistryMsg::ShadowStats(reply) => {
                    let _ = reply.send(self.state.shadow_stats());
                }

                RegistryMsg::Snapshot(reply) => {
                    let _ = reply.send(self.state.snapshot(self.clock.now()));
                }
            }
        }
        tracing::info!(project = %self.state.project(), "registre arrete");
    }

    /// ★ Admission **and** write, in that order, in the same actor (ADR 0014).
    ///
    /// Order matters: evaluate, write, then record. Recording before writing would make the
    /// registry believe the file changed when the write may have failed, and it would
    /// wrongly stale the other sessions' reads.
    async fn admit(
        &mut self,
        session: SessionId,
        path: &std::path::Path,
        content: &str,
    ) -> Result<Verdict, RegistryError> {
        let now = self.clock.now();
        let admission = self.state.evaluate_write(session, path, content, now)?;

        // The write itself. `tokio::fs` so the runtime is not blocked; serialisation by
        // the actor is intended — two writes to the same file in an undetermined order
        // would be a bug.
        let target = self.state.resolve(&admission.key);
        if let Some(parent) = target.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|source| RegistryError::Write {
                    path: admission.key.clone(),
                    source,
                })?;
        }
        tokio::fs::write(&target, content)
            .await
            .map_err(|source| RegistryError::Write {
                path: admission.key.clone(),
                source,
            })?;

        // The write succeeded: the state may now reflect the disk.
        self.state.commit_write(session, &admission, now);

        let verdict = admission.verdict.clone();
        let record = WriteRecord {
            project: self.state.project(),
            session,
            session_name: admission.session_name,
            seq: admission.seq,
            path: admission.key,
            hash_before: admission.hash_before,
            hash_after: admission.hash_after,
            verdict: Some(verdict.label().to_owned()),
            origin: WriteOrigin::Admitted,
            ts: now,
        };
        if self.journal.record_write(record).await.is_err() {
            tracing::error!("journal injoignable : ecriture non enregistree");
        }
        Ok(verdict)
    }
}

/// The registry's handle. Cloneable, and the only way in.
#[derive(Debug, Clone)]
pub struct RegistryHandle {
    tx: mpsc::Sender<RegistryMsg>,
}

/// Start a project's registry.
///
/// `root` is the working directory root: the registry writes there, and **every file key
/// goes through it**. Without that normalisation, the same read and the same write can
/// produce two different keys and `StaleRead` silently stops firing.
///
/// The clock is injected: the read-set TTL would be untestable otherwise, and this
/// project's tests are not allowed to sleep. An `Arc` around a clock is not business
/// state — there is no mutation, so there is no order to guarantee.
pub fn spawn_registry(
    project: ProjectId,
    root: ProjectRoot,
    clock: Arc<dyn Clock>,
    journal: JournalHandle,
) -> (RegistryHandle, JoinHandle<()>) {
    let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
    let actor = RegistryActor {
        state: RegistryState::new(project, root),
        clock,
        journal,
        rx,
    };
    (RegistryHandle { tx }, tokio::spawn(actor.run()))
}

impl RegistryHandle {
    async fn ask<T>(
        &self,
        make: impl FnOnce(oneshot::Sender<T>) -> RegistryMsg,
    ) -> Result<T, RegistryGone> {
        let (reply, rx) = oneshot::channel();
        self.tx.send(make(reply)).await.map_err(|_| RegistryGone)?;
        rx.await.map_err(|_| RegistryGone)
    }

    /// Make a session and its display name known.
    pub async fn register_session(
        &self,
        session: SessionId,
        name: impl Into<String>,
    ) -> Result<(), RegistryGone> {
        let name = name.into();
        self.ask(|reply| RegistryMsg::RegisterSession {
            session,
            name,
            reply,
        })
        .await
    }

    /// Record a read. Only [`ReadKind::FullFile`] enters the read-set.
    pub async fn record_read(
        &self,
        session: SessionId,
        path: impl Into<PathBuf>,
        content: impl Into<String>,
        kind: ReadKind,
    ) -> Result<(), RegistryGone> {
        let (path, content) = (path.into(), content.into());
        self.ask(|reply| RegistryMsg::RecordRead {
            session,
            path,
            content,
            kind,
            reply,
        })
        .await
    }

    /// ★ Submit a write for admission. The registry **evaluates, writes, journals**, and
    /// returns the verdict (ADR 0014).
    ///
    /// Fallible: admission includes the write, so it can fail. Returning a verdict without
    /// having written would be a lie — the caller would answer "admitted" to an agent that
    /// would believe its file written.
    ///
    /// **Nothing is blocked in v0.1**: the registry observes, journals and informs.
    /// Blocking will be decided after the real false-positive rate has been measured.
    pub async fn admit(
        &self,
        session: SessionId,
        path: impl Into<PathBuf>,
        content: impl Into<String>,
    ) -> Result<Verdict, RegistryError> {
        let (path, content) = (path.into(), content.into());
        self.ask(|reply| RegistryMsg::Admit {
            session,
            path,
            content,
            reply,
        })
        .await?
    }

    /// Report an **out-of-band** write noticed by the watcher.
    ///
    /// The watcher notices after the fact: this message prevents nothing, it only prevents
    /// the registry from becoming wrong. An observation whose fingerprint is already known
    /// is the echo of an admitted write and will be ignored.
    pub async fn observe_external_write(
        &self,
        path: impl Into<PathBuf>,
        hash: ContentHash,
    ) -> Result<ExternalWrite, RegistryGone> {
        let path = path.into();
        self.ask(|reply| RegistryMsg::ObserveExternalWrite { path, hash, reply })
            .await
    }

    /// ★ Record a read reported by a search, **in shadow**.
    ///
    /// It takes part in no verdict: it exists to count what would have been said if `Grep`
    /// hits counted (ADR 0027). `result_size` is the number of files the originating search
    /// returned — that is what makes the threshold decidable **after** the measurement.
    ///
    /// # Errors
    ///
    /// Fails if the actor has stopped.
    pub async fn record_shadow_read(
        &self,
        session: SessionId,
        path: impl Into<PathBuf>,
        content: impl Into<String>,
        result_size: usize,
    ) -> Result<(), RegistryGone> {
        let (path, content) = (path.into(), content.into());
        self.ask(|reply| RegistryMsg::RecordShadowRead {
            session,
            path,
            content,
            result_size,
            reply,
        })
        .await
    }

    /// Shadow mode's counters.
    ///
    /// # Errors
    ///
    /// Fails if the actor has stopped.
    pub async fn shadow_stats(&self) -> Result<ShadowStats, RegistryGone> {
        self.ask(RegistryMsg::ShadowStats).await
    }

    /// The current state.
    pub async fn snapshot(&self) -> Result<RegistrySnapshot, RegistryGone> {
        self.ask(RegistryMsg::Snapshot).await
    }
}
