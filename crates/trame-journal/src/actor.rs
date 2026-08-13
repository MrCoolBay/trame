//! The journal actor: it owns the SQLite connection.
//!
//! A `rusqlite::Connection` is `Send` but not `Sync`. Putting it behind an
//! `Arc<Mutex<_>>` would be the obvious solution and the wrong one: this is business
//! state, so it belongs to an actor. The journal is also the component that must
//! **serialise insertion order**, and an actor gives that by construction.
//!
//! # The journal is a sink, not a source
//!
//! The append methods carry **no** reply `oneshot`: their `await` only waits for a slot
//! in the queue, never for the SQLite write. That is what keeps registry admission in
//! microseconds. A write error is logged by the actor and counted; it does not surface on
//! every call.
//!
//! For tests, [`JournalHandle::flush`] is a **deterministic barrier**: the queue is FIFO,
//! so when the reply to `Flush` arrives, every earlier message has been processed. No
//! `sleep` anywhere.

use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use trame_core::{ProjectId, SessionId};

use crate::error::{JournalGone, Result};
use crate::records::{
    ProjectRecord, PromptRecord, ReadRecord, ResourceClaimRecord, SessionRecord, WriteRecord,
};
use crate::store::Journal;

/// Queue capacity. The journal processes in tens of microseconds; 256 messages waiting
/// is already a lot. Bounded, **never `unbounded_channel`**: an unbounded queue turns an
/// overload into a silent memory leak.
const CHANNEL_CAPACITY: usize = 256;

/// What the journal can be asked to do.
enum JournalMsg {
    Project(Box<ProjectRecord>),
    Session(Box<SessionRecord>),
    Prompt(Box<PromptRecord>),
    Read(Box<ReadRecord>),
    Write(Box<WriteRecord>),
    ResourceClaim(Box<ResourceClaimRecord>),
    WritesForProject {
        project: ProjectId,
        reply: oneshot::Sender<Result<Vec<WriteRecord>>>,
    },
    ReadsForSession {
        session: SessionId,
        reply: oneshot::Sender<Result<Vec<ReadRecord>>>,
    },
    Count {
        table: &'static str,
        reply: oneshot::Sender<Result<u64>>,
    },
    Flush(oneshot::Sender<FlushReport>),
}

/// What the [`JournalHandle::flush`] barrier reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlushReport {
    /// How many appends have been processed since startup.
    pub appended: u64,
    /// How many appends failed. **Must stay at zero**: a test that asserts on it catches
    /// the SQLite errors that reply-less appends would otherwise swallow.
    pub errors: u64,
}

struct JournalActor {
    journal: Journal,
    appended: u64,
    errors: u64,
    rx: mpsc::Receiver<JournalMsg>,
}

impl JournalActor {
    async fn run(mut self) {
        while let Some(msg) = self.rx.recv().await {
            match msg {
                JournalMsg::Project(record) => self.append(self.journal.insert_project(&record)),
                JournalMsg::Session(record) => self.append(self.journal.insert_session(&record)),
                JournalMsg::Prompt(record) => self.append(self.journal.insert_prompt(&record)),
                JournalMsg::Read(record) => self.append(self.journal.insert_read(&record)),
                JournalMsg::Write(record) => self.append(self.journal.insert_write(&record)),
                JournalMsg::ResourceClaim(record) => {
                    self.append(self.journal.insert_resource_claim(&record));
                }
                JournalMsg::WritesForProject { project, reply } => {
                    let _ = reply.send(self.journal.writes_for_project(project));
                }
                JournalMsg::ReadsForSession { session, reply } => {
                    let _ = reply.send(self.journal.reads_for_session(session));
                }
                JournalMsg::Count { table, reply } => {
                    let _ = reply.send(self.journal.count(table));
                }
                JournalMsg::Flush(reply) => {
                    let _ = reply.send(FlushReport {
                        appended: self.appended,
                        errors: self.errors,
                    });
                }
            }
        }
        // Loop exit: every handle has been dropped. A clean shutdown, with no dedicated
        // signal and no cancellation token — a second way to die would be one more bug.
        tracing::info!(
            appended = self.appended,
            errors = self.errors,
            "journal arrete"
        );
    }

    /// Count the append and log the failure. A write error must never kill the daemon:
    /// that would lose every session in the process.
    fn append(&mut self, outcome: Result<()>) {
        match outcome {
            Ok(()) => self.appended += 1,
            Err(error) => {
                self.errors += 1;
                tracing::error!(%error, "ecriture au journal echouee");
            }
        }
    }
}

/// The journal's handle. Cloneable, and the only way in.
#[derive(Debug, Clone)]
pub struct JournalHandle {
    tx: mpsc::Sender<JournalMsg>,
}

/// Start the journal actor.
pub fn spawn_journal(journal: Journal) -> (JournalHandle, JoinHandle<()>) {
    let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
    let join = tokio::spawn(
        JournalActor {
            journal,
            appended: 0,
            errors: 0,
            rx,
        }
        .run(),
    );
    (JournalHandle { tx }, join)
}

impl JournalHandle {
    async fn send(&self, msg: JournalMsg) -> Result<(), JournalGone> {
        self.tx.send(msg).await.map_err(|_| JournalGone)
    }

    async fn ask<T>(
        &self,
        make: impl FnOnce(oneshot::Sender<T>) -> JournalMsg,
    ) -> Result<T, JournalGone> {
        let (reply, rx) = oneshot::channel();
        self.send(make(reply)).await?;
        rx.await.map_err(|_| JournalGone)
    }

    /// Append a project.
    pub async fn record_project(&self, record: ProjectRecord) -> Result<(), JournalGone> {
        self.send(JournalMsg::Project(Box::new(record))).await
    }

    /// Append a session.
    pub async fn record_session(&self, record: SessionRecord) -> Result<(), JournalGone> {
        self.send(JournalMsg::Session(Box::new(record))).await
    }

    /// Append a prompt.
    pub async fn record_prompt(&self, record: PromptRecord) -> Result<(), JournalGone> {
        self.send(JournalMsg::Prompt(Box::new(record))).await
    }

    /// Append a read.
    pub async fn record_read(&self, record: ReadRecord) -> Result<(), JournalGone> {
        self.send(JournalMsg::Read(Box::new(record))).await
    }

    /// Append a write.
    pub async fn record_write(&self, record: WriteRecord) -> Result<(), JournalGone> {
        self.send(JournalMsg::Write(Box::new(record))).await
    }

    /// Append a resource claim.
    pub async fn record_resource_claim(
        &self,
        record: ResourceClaimRecord,
    ) -> Result<(), JournalGone> {
        self.send(JournalMsg::ResourceClaim(Box::new(record))).await
    }

    /// Wait until every message already sent has been processed, and report the append
    /// and error counters.
    ///
    /// The queue being FIFO, this is an exact barrier: no `sleep` needed.
    pub async fn flush(&self) -> Result<FlushReport, JournalGone> {
        self.ask(JournalMsg::Flush).await
    }

    /// A project's writes, in sequence order.
    pub async fn writes_for_project(
        &self,
        project: ProjectId,
    ) -> Result<Vec<WriteRecord>, JournalGone> {
        self.ask(|reply| JournalMsg::WritesForProject { project, reply })
            .await?
            .map_err(|error| {
                tracing::error!(%error, "reading writes failed");
                JournalGone
            })
    }

    /// A session's reads.
    pub async fn reads_for_session(
        &self,
        session: SessionId,
    ) -> Result<Vec<ReadRecord>, JournalGone> {
        self.ask(|reply| JournalMsg::ReadsForSession { session, reply })
            .await?
            .map_err(|error| {
                tracing::error!(%error, "reading reads failed");
                JournalGone
            })
    }

    /// The row count of a table in the schema.
    pub async fn count(&self, table: &'static str) -> Result<u64, JournalGone> {
        self.ask(|reply| JournalMsg::Count { table, reply })
            .await?
            .map_err(|error| {
                tracing::error!(%error, "comptage echoue");
                JournalGone
            })
    }
}
