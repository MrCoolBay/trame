//! **Couture.** D'ou vient le travail.
//!
//! Une session part toujours de quelque chose : une issue GitLab, un thread de
//! review, une tache, ou un prompt tape a la main. En v0.1 il n'existe qu'une
//! implementation, [`ManualTask`], et elle ne fait rien d'interessant. Le trait
//! est la pour que brancher GitLab en v0.2 n'oblige pas a retoucher la session,
//! le journal et le TUI.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::clock::Timestamp;
use crate::error::Result;
use crate::ids::WorkItemId;
use crate::session::WorkItemRef;

/// Une source d'elements de travail.
#[async_trait]
pub trait TaskSource: Send + Sync {
    /// La nature de cette source, telle qu'elle sera journalisee.
    fn kind(&self) -> TaskSourceKind;

    /// Liste les elements de travail correspondant au filtre.
    async fn list(&self, filter: TaskFilter) -> Result<Vec<WorkItem>>;

    /// Recupere un element precis.
    async fn get(&self, id: &WorkItemId) -> Result<WorkItem>;
}

/// La nature d'une source. Persiste tel quel dans le journal.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TaskSourceKind {
    /// L'utilisateur a tape son prompt. Seule source de la v0.1.
    Manual,
    /// Une issue GitLab. **Cible primaire** : GitLab self-hosted.
    GitLabIssue,
    /// Un thread de review sur une merge request GitLab.
    GitLabReviewThread,
    /// Une issue GitHub.
    GitHubIssue,
    /// Autre chose, nommee.
    Other(String),
}

impl TaskSourceKind {
    /// Le libelle stable utilise en base.
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

/// Un element de travail : ce qui justifie l'existence d'une session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkItem {
    /// Son identifiant chez sa source.
    pub id: WorkItemId,
    /// La source qui l'a fourni.
    pub source: TaskSourceKind,
    /// Son titre.
    pub title: String,
    /// Son corps : description d'issue, texte du commentaire, prompt brut.
    pub body: String,
    /// Ses etiquettes, quand la source en a.
    pub labels: Vec<String>,
    /// Son URL, quand elle existe.
    pub url: Option<String>,
    /// Quand la source l'a vu pour la derniere fois modifie.
    pub updated_at: Option<Timestamp>,
}

impl WorkItem {
    /// La reference journalisable de cet element, a poser sur la session.
    #[must_use]
    pub fn as_ref_for_session(&self) -> WorkItemRef {
        WorkItemRef {
            source: self.source.clone(),
            id: self.id.clone(),
            url: self.url.clone(),
        }
    }
}

/// Filtre de listing. Volontairement pauvre : on l'etendra quand une source
/// reelle en aura besoin, pas avant.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct TaskFilter {
    /// N'accepter que les elements portant toutes ces etiquettes.
    pub labels: Vec<String>,
    /// Ne rien renvoyer de plus ancien que cet instant.
    pub updated_since: Option<Timestamp>,
    /// Nombre maximum d'elements.
    pub limit: Option<usize>,
}

/// La seule source de la v0.1 : l'utilisateur tape son prompt.
///
/// Elle ne liste rien — il n'y a pas de backlog a interroger — mais elle
/// fabrique un [`WorkItem`], ce qui suffit a ce que la chaine auditable
/// `prompt -> session -> ecritures` soit complete des le premier jour.
#[derive(Debug, Clone, Default)]
pub struct ManualTask;

impl ManualTask {
    /// Fabrique un element de travail depuis un prompt saisi a la main.
    ///
    /// Le titre est la premiere ligne, tronquee. Le corps est le prompt entier.
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
        // Un prompt manuel n'a pas de backlog interrogeable. Liste vide, pas
        // une erreur : l'appelant n'a rien fait de mal.
        Ok(Vec::new())
    }

    async fn get(&self, id: &WorkItemId) -> Result<WorkItem> {
        Err(crate::error::CoreError::NotFound {
            what: "work item manuel",
            id: id.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_titre_est_la_premiere_ligne() {
        let item = ManualTask::from_prompt("Refacto l'auth\n\nEt tant qu'on y est, les handlers.");
        assert_eq!(item.title, "Refacto l'auth");
        assert!(
            item.body.contains("handlers"),
            "le corps garde le prompt entier"
        );
    }

    #[test]
    fn un_titre_trop_long_est_tronque() {
        let item = ManualTask::from_prompt(&"a".repeat(200));
        assert!(item.title.ends_with('…'));
        assert_eq!(item.title.chars().count(), 73);
    }

    #[test]
    fn un_prompt_vide_ne_panique_pas() {
        assert_eq!(ManualTask::from_prompt("").title, "");
    }
}
