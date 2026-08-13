//! The session: one agent, one goal, one project.

use serde::{Deserialize, Serialize};

use crate::clock::Timestamp;
use crate::ids::{BranchId, BranchName, ProjectId, SessionId, WorkItemId};
use crate::task_source::TaskSourceKind;

/// A working session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    /// Its identifier.
    pub id: SessionId,
    /// The project it belongs to. A session never exists outside a project.
    pub project_id: ProjectId,
    /// The display name, as the user typed it ("refactor-api").
    pub name: String,
    /// The harness running it.
    pub harness: Harness,
    /// The branch it targets: new or existing.
    pub target_branch: BranchTarget,
    /// Where the work came from. `None` for a hand-typed prompt with no reference.
    ///
    /// This field is what closes the full auditable chain:
    /// `issue -> session -> agent -> writes -> hunks -> branch -> MR`.
    pub work_item: Option<WorkItemRef>,
    /// Its current state.
    pub state: SessionState,
    /// When it was created.
    pub created_at: Timestamp,
}

/// The agent harness that runs the session.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Harness {
    /// Claude Code. **The only v0.1 target**, over ACP.
    ClaudeCode,
    /// Codex CLI.
    Codex,
    /// Gemini CLI.
    Gemini,
    /// The user in their editor. Handled exactly like an agent: their writes are
    /// caught by FSEvents and journalled.
    Human,
    /// A process that is not an agent: build, formatter, script. Its writes are
    /// out-of-band — caught after the fact, but never admitted.
    External,
    /// Any harness driven over a PTY.
    Custom(String),
}

impl Harness {
    /// The stable label used in the database. Never change it without a migration.
    #[must_use]
    pub fn label(&self) -> &str {
        match self {
            Self::ClaudeCode => "claude_code",
            Self::Codex => "codex",
            Self::Gemini => "gemini",
            Self::Human => "human",
            Self::External => "external",
            Self::Custom(name) => name,
        }
    }

    /// True if Trame drives the harness, false for the human and for external
    /// processes. Those two never receive an injected prompt.
    #[must_use]
    pub fn is_agent(&self) -> bool {
        !matches!(self, Self::Human | Self::External)
    }
}

/// A session's state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SessionState {
    /// Waiting for a prompt.
    Idle,
    /// The agent is thinking.
    Thinking,
    /// The agent is writing. The registry is being asked.
    Writing,
    /// Blocked on a permission request addressed to the human.
    AwaitingPermission,
    /// Finished.
    Done,
    /// Failed, with the reason.
    Failed(String),
}

impl SessionState {
    /// The stable label used in the database.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Thinking => "thinking",
            Self::Writing => "writing",
            Self::AwaitingPermission => "awaiting_permission",
            Self::Done => "done",
            Self::Failed(_) => "failed",
        }
    }

    /// True if the session will produce nothing more.
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Done | Self::Failed(_))
    }
}

/// The branch a session targets.
///
/// This distinction exists from v0.1 although it only serves v0.2: without it,
/// handling review comments on an existing MR would force a refactor of the
/// session, the journal and the VCS all at once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BranchTarget {
    /// A virtual branch to create.
    New(BranchName),
    /// An existing branch, identified on the GitButler side. The case of answering
    /// comments on a change request that is already open.
    Existing(BranchId),
}

impl BranchTarget {
    /// The stable textual form, for the `target_branch` column.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::New(name) => name.as_str(),
            Self::Existing(id) => id.as_str(),
        }
    }
}

/// A reference to the work item a session came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkItemRef {
    /// The source that supplied it.
    pub source: TaskSourceKind,
    /// Its identifier at that source.
    pub id: WorkItemId,
    /// Its URL, when there is one. Makes the journal clickable.
    pub url: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_human_and_the_external_writer_are_not_agents() {
        assert!(!Harness::Human.is_agent());
        assert!(!Harness::External.is_agent());
        assert!(Harness::ClaudeCode.is_agent());
    }

    #[test]
    fn only_done_and_failed_are_terminal_states() {
        assert!(SessionState::Done.is_terminal());
        assert!(SessionState::Failed("boom".into()).is_terminal());
        assert!(!SessionState::Writing.is_terminal());
        assert!(!SessionState::AwaitingPermission.is_terminal());
    }
}
