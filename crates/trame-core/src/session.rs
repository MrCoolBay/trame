//! La session : un agent, un objectif, un projet.

use serde::{Deserialize, Serialize};

use crate::clock::Timestamp;
use crate::ids::{BranchId, BranchName, ProjectId, SessionId, WorkItemId};
use crate::task_source::TaskSourceKind;

/// Une session de travail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    /// Son identifiant.
    pub id: SessionId,
    /// Le projet auquel elle appartient. Une session n'existe jamais hors projet.
    pub project_id: ProjectId,
    /// Le nom affiche, tel que saisi par l'utilisateur (« refacto-api »).
    pub name: String,
    /// Le harness qui l'execute.
    pub harness: Harness,
    /// La branche visee : neuve ou existante.
    pub target_branch: BranchTarget,
    /// D'ou vient le travail. `None` pour un prompt tape a la main sans
    /// reference.
    ///
    /// Ce champ est ce qui ferme la chaine auditable complete :
    /// `issue -> session -> agent -> ecritures -> hunks -> branche -> MR`.
    pub work_item: Option<WorkItemRef>,
    /// Son state courant.
    pub state: SessionState,
    /// Sa date de creation.
    pub created_at: Timestamp,
}

/// Le harness d'agent qui execute la session.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Harness {
    /// Claude Code. **Seule target de la v0.1**, via ACP.
    ClaudeCode,
    /// Codex CLI.
    Codex,
    /// Gemini CLI.
    Gemini,
    /// L'utilisateur dans son editeur. Traite exactement comme un agent : ses
    /// ecritures sont detectees par FSEvents et journalisees.
    Human,
    /// Un process qui n'est pas un agent : build, formatter, script. Ses
    /// ecritures sont hors-bande — rattrapees, mais jamais admises.
    External,
    /// Un harness quelconque pilot en PTY.
    Custom(String),
}

impl Harness {
    /// Le libelle stable utilise en base. Ne jamais le changer sans migration.
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

    /// Vrai si le harness est pilot par Trame, faux pour l'humain et les
    /// process externes. Ces deux-la ne recoivent jamais de prompt injecte.
    #[must_use]
    pub fn is_agent(&self) -> bool {
        !matches!(self, Self::Human | Self::External)
    }
}

/// L'state d'une session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SessionState {
    /// En attente d'un prompt.
    Idle,
    /// L'agent reflechit.
    Thinking,
    /// L'agent ecrit. Le registre est sollicite.
    Writing,
    /// Bloque sur une demande de permission adressee a l'humain.
    AwaitingPermission,
    /// Termine.
    Done,
    /// Echoue, avec le reason.
    Failed(String),
}

impl SessionState {
    /// Le libelle stable utilise en base.
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

    /// Vrai si la session ne produira plus rien.
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Done | Self::Failed(_))
    }
}

/// La branche que vise une session.
///
/// Cette distinction existe des la v0.1 alors qu'elle ne sert qu'a la v0.2 :
/// sans elle, handle les commentaires de review d'une MR existante imposerait
/// un refactor de la session, du journal et du VCS d'un coup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BranchTarget {
    /// Une branche virtuelle a creer.
    New(BranchName),
    /// Une branche existante, identifiee cote GitButler. Cas de la reponse aux
    /// commentaires d'une change request deja opened.
    Existing(BranchId),
}

impl BranchTarget {
    /// La representation textuelle stable, pour la colonne `target_branch`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::New(name) => name.as_str(),
            Self::Existing(id) => id.as_str(),
        }
    }
}

/// Une reference vers l'element de travail a l'origine d'une session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkItemRef {
    /// La source qui l'a fourni.
    pub source: TaskSourceKind,
    /// Son identifiant chez cette source.
    pub id: WorkItemId,
    /// Son URL, quand elle existe. Rend le journal cliquable.
    pub url: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn l_humain_et_l_externe_ne_sont_pas_des_agents() {
        assert!(!Harness::Human.is_agent());
        assert!(!Harness::External.is_agent());
        assert!(Harness::ClaudeCode.is_agent());
    }

    #[test]
    fn seuls_done_et_failed_sont_terminaux() {
        assert!(SessionState::Done.is_terminal());
        assert!(SessionState::Failed("boom".into()).is_terminal());
        assert!(!SessionState::Writing.is_terminal());
        assert!(!SessionState::AwaitingPermission.is_terminal());
    }
}
