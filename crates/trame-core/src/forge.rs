//! **Couture.** Ou va le resultat.
//!
//! # Nommage
//!
//! `ChangeRequest`, **jamais** `PullRequest`. « Pull request » est un terme
//! GitHub ; GitLab dit « merge request ». La target primaire de Trame est GitLab
//! **self-hosted**, donc le vocabulaire du code est neutre et `base_url` est un
//! champ de premiere classe des le depart — pas un parametre optionnel ajoute
//! plus tard pour faire plaisir aux instances privees.
//!
//! Aucune implementation en v0.1. Le trait fixe la frontiere, c'est tout.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::clock::Timestamp;
use crate::error::Result;
use crate::ids::{BranchName, CrId, ThreadId};

/// Une forge : GitLab self-hosted d'abord, GitHub ensuite.
#[async_trait]
pub trait Forge: Send + Sync {
    /// L'URL de base de l'instance. Une instance privee est le cas normal, pas
    /// l'exception.
    fn base_url(&self) -> &str;

    /// Pousse une branche vers la forge.
    async fn push(&self, branch: &BranchName) -> Result<()>;

    /// Ouvre une change request.
    async fn open_change_request(&self, req: ChangeRequest) -> Result<CrId>;

    /// Les threads de discussion d'une change request.
    ///
    /// C'est le point d'entree de la run_loop de review : chaque thread non resolu
    /// peut devenir un [`crate::WorkItem`], donc une session.
    async fn review_threads(&self, id: &CrId) -> Result<Vec<ReviewThread>>;

    /// Repond dans un thread.
    async fn reply(&self, thread: &ThreadId, body: &str) -> Result<()>;
}

/// La demande d'ouverture d'une change request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeRequest {
    /// La branche source.
    pub source_branch: BranchName,
    /// La branche target.
    pub target_branch: BranchName,
    /// Le titre.
    pub title: String,
    /// La description.
    pub description: String,
    /// Ouvrir en brouillon. Par defaut oui : ce qu'un agent produit se relit
    /// avant de solliciter des reviewers.
    pub draft: bool,
}

/// Un thread de discussion sur une change request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewThread {
    /// Son identifiant.
    pub id: ThreadId,
    /// Le file concerne, si le thread est ancre sur du code.
    pub path: Option<String>,
    /// La line concernee, si le thread est ancre sur du code.
    pub line: Option<u32>,
    /// L'auteur du premier message.
    pub author: String,
    /// Le corps du premier message.
    pub body: String,
    /// Vrai si le thread est resolu.
    pub resolved: bool,
    /// Quand il a ete cree.
    pub created_at: Option<Timestamp>,
}
