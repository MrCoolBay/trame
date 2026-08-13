//! **A seam.** Where work comes from.
//!
//! A session always starts from something: a GitLab issue, a review thread, a task,
//! or a hand-typed prompt. In v0.1 there is only one implementation,
//! [`ManualTask`], and it does nothing interesting. The trait is there so that
//! wiring GitLab in v0.2 does not force changes to the session, the journal and the
//! TUI.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::clock::Timestamp;
use crate::error::Result;
use crate::ids::WorkItemId;
use crate::session::WorkItemRef;

/// A source of work items.
#[async_trait]
pub trait TaskSource: Send + Sync {
    /// What kind of source this is, as it will be journalled.
    fn kind(&self) -> TaskSourceKind;

    /// List the work items matching the filter.
    async fn list(&self, filter: TaskFilter) -> Result<Vec<WorkItem>>;

    /// Fetch one specific item.
    async fn get(&self, id: &WorkItemId) -> Result<WorkItem>;
}

/// What kind of source. Persisted as-is in the journal.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TaskSourceKind {
    /// The user typed their prompt. The only source in v0.1.
    Manual,
    /// A GitLab issue. **Primary target**: self-hosted GitLab.
    GitLabIssue,
    /// A review thread on a GitLab merge request.
    GitLabReviewThread,
    /// A GitHub issue.
    GitHubIssue,
    /// Something else, named.
    Other(String),
}

impl TaskSourceKind {
    /// The stable label used in the database.
    #[must_use]
    pub fn label(&self) -> &str {
        match self {
            Self::Manual => "manual",
            Self::GitLabIssue => "gitlab_issue",
            Self::GitLabReviewThread => "gitlab_review_thread",
            Self::GitHubIssue => "github_issue",
            Self::Other(name) => name,
        }
    }
}

/// A work item: what justifies a session existing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkItem {
    /// Its identifier at its source.
    pub id: WorkItemId,
    /// The source that supplied it.
    pub source: TaskSourceKind,
    /// Its title.
    pub title: String,
    /// Its body: issue description, comment text, raw prompt.
    pub body: String,
    /// Its labels, when the source has any.
    pub labels: Vec<String>,
    /// Its URL, when there is one.
    pub url: Option<String>,
    /// When the source last saw it modified.
    pub updated_at: Option<Timestamp>,
}

impl WorkItem {
    /// This item's journallable reference, to attach to the session.
    #[must_use]
    pub fn as_ref_for_session(&self) -> WorkItemRef {
        WorkItemRef {
            source: self.source.clone(),
            id: self.id.clone(),
            url: self.url.clone(),
        }
    }
}

/// A listing filter. Deliberately thin: it grows when a real source needs it, not
/// before.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct TaskFilter {
    /// Only accept items carrying all of these labels.
    pub labels: Vec<String>,
    /// Return nothing older than this instant.
    pub updated_since: Option<Timestamp>,
    /// Maximum number of items.
    pub limit: Option<usize>,
}

/// The only source in v0.1: the user types their prompt.
///
/// It lists nothing — there is no backlog to query — but it does build a
/// [`WorkItem`], which is enough for the auditable chain
/// `prompt -> session -> writes` to be complete from day one.
#[derive(Debug, Clone, Default)]
pub struct ManualTask;

impl ManualTask {
    /// Build a work item from a hand-typed prompt.
    ///
    /// The title is the first line, truncated. The body is the whole prompt.
    #[must_use]
    pub fn from_prompt(prompt: &str) -> WorkItem {
        const TITLE_MAX: usize = 72;
        let first_line = prompt.lines().next().unwrap_or_default().trim();
        let title = if first_line.chars().count() > TITLE_MAX {
            let truncated: String = first_line.chars().take(TITLE_MAX).collect();
            format!("{truncated}…")
        } else {
            first_line.to_owned()
        };

        WorkItem {
            id: WorkItemId::new(uuid::Uuid::new_v4().to_string()),
            source: TaskSourceKind::Manual,
            title,
            body: prompt.to_owned(),
            labels: Vec::new(),
            url: None,
            updated_at: None,
        }
    }
}

#[async_trait]
impl TaskSource for ManualTask {
    fn kind(&self) -> TaskSourceKind {
        TaskSourceKind::Manual
    }

    async fn list(&self, _filter: TaskFilter) -> Result<Vec<WorkItem>> {
        // A manual prompt has no queryable backlog. An empty list, not an error:
        // the caller did nothing wrong.
        Ok(Vec::new())
    }

    async fn get(&self, id: &WorkItemId) -> Result<WorkItem> {
        Err(crate::error::CoreError::NotFound {
            what: "manual work item",
            id: id.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_title_is_the_first_line_of_the_prompt() {
        let item =
            ManualTask::from_prompt("Refactor auth\n\nAnd while we are at it, the handlers.");
        assert_eq!(item.title, "Refactor auth");
        assert!(
            item.body.contains("handlers"),
            "the body keeps the whole prompt"
        );
    }

    #[test]
    fn an_over_long_title_is_truncated() {
        let item = ManualTask::from_prompt(&"a".repeat(200));
        assert!(item.title.ends_with('…'));
        assert_eq!(item.title.chars().count(), 73);
    }

    #[test]
    fn an_empty_prompt_does_not_panic() {
        assert_eq!(ManualTask::from_prompt("").title, "");
    }
}
