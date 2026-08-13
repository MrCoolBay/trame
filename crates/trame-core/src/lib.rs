//! Trame's shared types and seams.
//!
//! This crate is the foundation: it depends on no other crate in the workspace
//! and performs no I/O. It holds the vocabulary — identifiers, verdicts, session
//! states — and the *seams*: the traits that mark the boundaries where the
//! application will grow.
//!
//! # The thesis these types serve
//!
//! When agent A is about to write, if a file it **read** has since been modified
//! by another session, A is reasoning about a world that no longer exists. Trame
//! detects that and tells it. [`Verdict::StaleRead`] is the finding, and
//! [`PromptContributor`] is the channel that carries it back to the agent.
//!
//! # The seams
//!
//! They do almost nothing in v0.1 and everything in six months:
//!
//! - [`TaskSource`] — where work comes from (an issue, a review thread, a manual prompt).
//! - [`Forge`] — where the result goes. Neutral naming: `ChangeRequest`, never
//!   `PullRequest`, because self-hosted GitLab is the primary target.
//! - [`PromptContributor`] — the prompt composition pipeline. **This one is not
//!   speculative**: it is how the stale-read notice reaches the agent.
//! - [`BranchTarget`] — a session targets either a new branch *or* an existing one.
//!   Without it, answering review comments on an MR would force a refactor.

pub mod clock;
pub mod error;
pub mod forge;
pub mod hash;
pub mod ids;
pub mod notice;
pub mod paths;
pub mod project;
pub mod prompt;
pub mod session;
pub mod task_source;
pub mod verdict;

pub use clock::{Clock, SystemClock, Timestamp};
pub use error::{CoreError, Result};
pub use forge::{ChangeRequest, Forge, ReviewThread};
pub use hash::ContentHash;
pub use ids::{BranchId, BranchName, CrId, ProjectId, Seq, SessionId, ThreadId, WorkItemId};
pub use notice::{ConfigurableNotice, NoticeVariant};
pub use paths::ProjectRoot;
pub use project::{Project, Toolchain};
pub use prompt::{
    PromptContributor, PromptFragment, PromptPipeline, SessionContext, StaleReadNotice,
};
pub use session::{BranchTarget, Harness, Session, SessionState, WorkItemRef};
pub use task_source::{ManualTask, TaskFilter, TaskSource, TaskSourceKind, WorkItem};
pub use verdict::{StaleFile, Verdict};
