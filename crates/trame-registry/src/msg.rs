//! The registry's messages, and what they return.

use std::path::PathBuf;

use tokio::sync::oneshot;
use trame_core::{ContentHash, ProjectId, Seq, SessionId, Verdict};

use crate::error::RegistryError;

/// ★ What `Grep` reads **would** have produced, if they counted.
///
/// # Why measure instead of flipping the switch
///
/// `ReadKind::GrepHit` is not substantial, so files reported by a search never enter the
/// read-set and can produce no notice. The read hole therefore stays open (ADR 0027).
///
/// Closing it by making `GrepHit` substantial would be a **bet**, not a decision: the
/// experimental round measures success rates, never the **false-positive** rate — and that
/// is precisely the variable behind product risk number one (invariant 8). Shadow mode
/// produces that data at no risk: it counts what would have been said, and says nothing.
///
/// # Why a distribution rather than a threshold
///
/// A `Grep` returning three files is a targeted read; a `grep -r` returning three hundred
/// is an exploration. They are not the same thing, and a boolean forces the false choice
/// between no coverage and all the noise.
///
/// So for every potential notice we record **the size of the `Grep` result it came from**.
/// [`ShadowStats::potential_notices_if_threshold`] then answers for **any** N, after the
/// fact. That is what avoids picking N by intuition: the measurement will give it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ShadowStats {
    /// How many notices the `Grep` hits would have produced in total.
    pub potential_notices: u64,
    /// How many potential notices, by **result size** of the originating `Grep`.
    ///
    /// This distribution is what will give the threshold, not an intuition.
    pub by_size: std::collections::BTreeMap<usize, u64>,
    /// How many `Grep` reads were recorded in shadow, across all files.
    ///
    /// The denominator: without it, "twelve potential notices" means nothing.
    pub shadow_reads: u64,
}

impl ShadowStats {
    /// How many notices a threshold of `n` files **would** have produced.
    ///
    /// **`n` has no default, and will not get one before the measurement.** It is the
    /// experiment's parameter: "had we counted `Grep`s returning at most n files, how many
    /// notices would we have emitted?". The answer for every n reads out of the same
    /// distribution, which avoids replaying the measurement for each hypothesis.
    #[must_use]
    pub fn potential_notices_if_threshold(&self, n: usize) -> u64 {
        self.by_size
            .iter()
            .filter(|(size, _)| **size <= n)
            .map(|(_, count)| *count)
            .sum()
    }
}

/// What the registry can be asked to do.
///
/// One variant per operation: no `Msg::Do { op }`, which would move the dispatch into a
/// second `match` and throw away the typing of the replies.
pub(crate) enum RegistryMsg {
    /// Make a session and its display name known.
    ///
    /// The name is what the injected notice uses: "auth.rs was changed by session
    /// *refactor-api*" is actionable, a UUID is not.
    RegisterSession {
        session: SessionId,
        name: String,
        reply: oneshot::Sender<()>,
    },

    /// An agent read a file. Enters the read-set **if** the read is substantial.
    RecordRead {
        session: SessionId,
        path: PathBuf,
        content: String,
        kind: ReadKind,
        reply: oneshot::Sender<()>,
    },

    /// ★ An agent wants to write. **This is the admission controller**, and it is the one
    /// that performs the write (ADR 0014) — hence the `Result`.
    Admit {
        session: SessionId,
        path: PathBuf,
        content: String,
        reply: oneshot::Sender<Result<Verdict, RegistryError>>,
    },

    /// The watcher noticed an **out-of-band** write.
    ///
    /// It was not admitted and nothing could be prevented: the watcher notices after the
    /// fact. The message exists so the registry does not become **wrong** — without it, a
    /// `sed -i` leaves a stale `FileState` and the matching `StaleRead` never fires.
    ObserveExternalWrite {
        path: PathBuf,
        hash: ContentHash,
        reply: oneshot::Sender<ExternalWrite>,
    },

    /// ★ A read reported by a search, recorded **in shadow**.
    ///
    /// It does not enter the real read-set and can therefore produce no notice: it only
    /// serves to count what would have been said (see [`ShadowStats`]).
    ///
    /// `result_size` is the number of files the originating `Grep` returned. That is what
    /// makes the threshold decidable after the fact.
    RecordShadowRead {
        session: SessionId,
        path: PathBuf,
        content: String,
        result_size: usize,
        reply: oneshot::Sender<()>,
    },

    /// Shadow mode's counters.
    ShadowStats(oneshot::Sender<ShadowStats>),

    /// The current state, for the interface and the tests.
    Snapshot(oneshot::Sender<RegistrySnapshot>),
}

/// What the registry did with an out-of-band observation.
///
/// **A `()` reply was not enough**, and the defect was not theoretical: the registry
/// writes to disk itself (ADR 0014), so the watcher sees **its own writes too**. Without
/// this distinction the caller reports them as out-of-band, and an interface shows a write
/// that went through admission with a verdict as "noticed after the fact, no verdict".
/// That is the exact opposite of the truth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ExternalWrite {
    /// A new fingerprint: the registry's state has been caught up and the row journalled,
    /// attributed to [`trame_core::SessionId::EXTERNAL`] and **with no verdict**.
    Recorded {
        /// The sequence number assigned, project-local.
        seq: Seq,
    },
    /// A fingerprint identical to the one already known: this is the echo of an admitted
    /// write. Nothing was journalled, and **nothing must be displayed**.
    Echo,
}

impl ExternalWrite {
    /// True if this observation was a genuine out-of-band write.
    #[must_use]
    pub const fn is_recorded(self) -> bool {
        matches!(self, Self::Recorded { .. })
    }
}

/// What kind of read this was.
///
/// **This is the read-set filter**, and it lives here rather than in the caller so the
/// policy has a single home. Agents read enormously: if everything entered the read-set it
/// would explode and everything would become a `StaleRead` — which amounts to disabling
/// the feature by making it unusable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ReadKind {
    /// A file read in full. **The only substantial read.**
    FullFile,
    /// A search result: a few lines of context. The agent did not see the file, it saw a
    /// match.
    GrepHit,
    /// A directory listing. No content read.
    DirListing,
    /// Metadata only: existence, size, date.
    Metadata,
}

impl ReadKind {
    /// True if this read enters the read-set.
    #[must_use]
    pub fn is_substantial(self) -> bool {
        matches!(self, Self::FullFile)
    }
}

/// The registry's state at a point in time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistrySnapshot {
    /// The project. One registry per project, never shared.
    pub project: ProjectId,
    /// The last sequence number assigned. `Seq::from_u64(0)` if there has been no write.
    pub seq: Seq,
    /// The known files, sorted by path.
    pub files: Vec<FileSnapshot>,
    /// The known sessions, sorted by identifier.
    pub sessions: Vec<SessionSnapshot>,
    /// What the `Grep` reads would have produced. **No effect on verdicts.**
    pub shadow: ShadowStats,
}

/// A tracked file's state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSnapshot {
    /// Its path, relative to the project root.
    pub path: PathBuf,
    /// The last session to have written it.
    pub last_writer: SessionId,
    /// That write's sequence number.
    pub last_seq: Seq,
    /// The fingerprint of the current content.
    pub hash: ContentHash,
}

/// A session's state as the registry sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSnapshot {
    /// Its identifier.
    pub session: SessionId,
    /// Its display name.
    pub name: String,
    /// The files in its read-set that have **not expired**, sorted by path.
    pub read_set: Vec<PathBuf>,
    /// The files it has written, sorted by path.
    pub write_set: Vec<PathBuf>,
}
