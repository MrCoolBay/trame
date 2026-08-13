//! The records, one per table.
//!
//! They are deliberately flat and logic-free: the journal stores what it is given. Enum
//! values arrive already in their stable form — the one `trame-core`'s `label()` methods
//! return — because **changing a persisted label requires a migration**: the journal is
//! append-only, and old rows are never rewritten.

use std::path::PathBuf;

use trame_core::clock::Timestamp;
use trame_core::{ContentHash, ProjectId, Seq, SessionId, Toolchain};

/// A `projects` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRecord {
    /// Its identifier.
    pub id: ProjectId,
    /// The working directory root, absolute — the only absolute path in the schema,
    /// and for good reason: it is what gives the relative paths their meaning.
    pub path: PathBuf,
    /// The display name.
    pub name: String,
    /// The detected toolchain.
    pub toolchain: Toolchain,
    /// When the project was added.
    pub added_at: Timestamp,
    /// The last known opening.
    pub last_opened_at: Option<Timestamp>,
}

/// A `sessions` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRecord {
    /// Its identifier.
    pub id: SessionId,
    /// The project it belongs to.
    pub project: ProjectId,
    /// The display name, as typed.
    pub name: String,
    /// The harness label — `Harness::label()`.
    pub harness: String,
    /// The target branch — `BranchTarget::as_str()`.
    pub target_branch: String,
    /// An opaque reference to the originating work item. The encoding belongs to the
    /// caller; the journal does not interpret it.
    pub work_item: Option<String>,
    /// The state **at creation** — `SessionState::label()`. Not the current state: the
    /// journal is append-only and never updates it.
    pub initial_state: String,
    /// When it was created.
    pub created_at: Timestamp,
}

/// A `prompts` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptRecord {
    /// The session that received it.
    pub session: SessionId,
    /// The text, in full. This is what makes the auditable chain complete.
    pub content: String,
    /// When.
    pub ts: Timestamp,
}

/// A `reads` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadRecord {
    /// The project.
    pub project: ProjectId,
    /// The session that read.
    pub session: SessionId,
    /// The path, **relative to the project root**.
    pub path: PathBuf,
    /// The fingerprint of the content read.
    pub hash: ContentHash,
    /// When.
    pub ts: Timestamp,
}

/// Where a recorded write came from.
///
/// The distinction is structural, not cosmetic: an admitted write was **decided** by the
/// registry before touching the disk, an observed write was **noticed after the fact**.
/// Conflating them would make the journal wrong about the one thing that matters,
/// provenance — and would imply a guarantee we do not have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum WriteOrigin {
    /// Went through admission: the registry returned a verdict, then wrote.
    Admitted,
    /// Noticed by the watcher, outside admission. `sed -i`, a hook, a build, the editor.
    /// **The registry could prevent nothing**; it records so as not to become wrong.
    Observed,
}

impl WriteOrigin {
    /// The stable label stored in the database. Never change it without a migration.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Admitted => "admitted",
            Self::Observed => "observed",
        }
    }

    /// The inverse. An unknown label reads back as `Observed`: the cautious assumption,
    /// the one that does not claim an admission guarantee.
    #[must_use]
    pub fn from_label(label: &str) -> Self {
        if label == "admitted" {
            Self::Admitted
        } else {
            Self::Observed
        }
    }
}

/// A `writes` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteRecord {
    /// The project.
    pub project: ProjectId,
    /// The session that wrote.
    pub session: SessionId,
    /// That session's display name, **denormalised**. An audit row must read without a
    /// join, and survive the session's disappearance.
    pub session_name: String,
    /// The sequence number, project-local.
    pub seq: Seq,
    /// The path, **relative to the project root**.
    pub path: PathBuf,
    /// The fingerprint before. `None` when the file is being created.
    pub hash_before: Option<ContentHash>,
    /// The fingerprint after.
    pub hash_after: ContentHash,
    /// The verdict returned — `Verdict::label()`. `None` for an observed write: nobody
    /// admitted it, so no verdict exists.
    pub verdict: Option<String>,
    /// Admitted or observed.
    pub origin: WriteOrigin,
    /// When.
    pub ts: Timestamp,
}

/// A `resource_claims` row.
///
/// Claims are **global**, not per project: port 3000 is machine-wide. Two projects each
/// starting their dev server is the first genuine cross-project conflict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceClaimRecord {
    /// The resource, as a qualified string: `port:3000`, `db:dev`.
    pub resource: String,
    /// The project holding it.
    pub project: ProjectId,
    /// The session holding it.
    pub session: SessionId,
    /// When.
    pub claimed_at: Timestamp,
}
