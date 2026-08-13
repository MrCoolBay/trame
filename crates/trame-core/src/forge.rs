//! **A seam.** Where the result goes.
//!
//! # Naming
//!
//! `ChangeRequest`, **never** `PullRequest`. "Pull request" is GitHub's term; GitLab
//! says "merge request". Trame's primary target is **self-hosted** GitLab, so the
//! code's vocabulary is neutral and `base_url` is a first-class field from the
//! start — not an optional parameter bolted on later to appease private instances.
//!
//! No implementation in v0.1. The trait fixes the boundary, nothing more.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::clock::Timestamp;
use crate::error::Result;
use crate::ids::{BranchName, CrId, ThreadId};

/// A forge: self-hosted GitLab first, GitHub next.
#[async_trait]
pub trait Forge: Send + Sync {
    /// The instance's base URL. A private instance is the normal case, not the
    /// exception.
    fn base_url(&self) -> &str;

    /// Push a branch to the forge.
    async fn push(&self, branch: &BranchName) -> Result<()>;

    /// Open a change request.
    async fn open_change_request(&self, req: ChangeRequest) -> Result<CrId>;

    /// The discussion threads on a change request.
    ///
    /// This is the entry point of the review loop: every unresolved thread can become
    /// a [`crate::WorkItem`], and therefore a session.
    async fn review_threads(&self, id: &CrId) -> Result<Vec<ReviewThread>>;

    /// Reply in a thread.
    async fn reply(&self, thread: &ThreadId, body: &str) -> Result<()>;
}

/// A request to open a change request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeRequest {
    /// The source branch.
    pub source_branch: BranchName,
    /// The target branch.
    pub target_branch: BranchName,
    /// The title.
    pub title: String,
    /// The description.
    pub description: String,
    /// Open as a draft. True by default: what an agent produces gets re-read before
    /// reviewers are called in.
    pub draft: bool,
}

/// A discussion thread on a change request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewThread {
    /// Its identifier.
    pub id: ThreadId,
    /// The file in question, if the thread is anchored on code.
    pub path: Option<String>,
    /// The line in question, if the thread is anchored on code.
    pub line: Option<u32>,
    /// Who wrote the first message.
    pub author: String,
    /// The body of the first message.
    pub body: String,
    /// True if the thread is resolved.
    pub resolved: bool,
    /// When it was created.
    pub created_at: Option<Timestamp>,
}
