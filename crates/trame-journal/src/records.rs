//! Les enregistrements, un par table.
//!
//! Ils sont volontairement plats et sans logique : le journal stocke ce qu'on lui
//! donne. Les valeurs d'enums arrivent deja sous leur forme stable — celle rendue par
//! les methodes `label()` de `trame-core` — parce que **changer un libelle persiste
//! exige une migration** : le journal est append-only, les anciennes lignes ne se
//! reecrivent pas.

use std::path::PathBuf;

use trame_core::clock::Timestamp;
use trame_core::{ContentHash, ProjectId, Seq, SessionId, Toolchain};

/// Une ligne de `projects`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRecord {
    /// Son identifiant.
    pub id: ProjectId,
    /// La racine du working directory, en absolu — c'est le seul chemin absolu du
    /// schema, et pour cause : c'est lui qui donne son sens aux chemins relatifs.
    pub path: PathBuf,
    /// Le nom affiche.
    pub name: String,
    /// La toolchain detectee.
    pub toolchain: Toolchain,
    /// Quand le projet a ete ajoute.
    pub added_at: Timestamp,
    /// La derniere ouverture connue.
    pub last_opened_at: Option<Timestamp>,
}

/// Une ligne de `sessions`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRecord {
    /// Son identifiant.
    pub id: SessionId,
    /// Le projet auquel elle appartient.
    pub project: ProjectId,
    /// Le nom affiche, tel que saisi.
    pub name: String,
    /// Le libelle du harness — `Harness::label()`.
    pub harness: String,
    /// La branche visee — `BranchTarget::as_str()`.
    pub target_branch: String,
    /// Reference opaque vers l'element de travail d'origine. L'encodage appartient a
    /// l'appelant ; le journal ne l'interprete pas.
    pub work_item: Option<String>,
    /// L'etat a la creation — `SessionState::label()`.
    pub state: String,
    /// Sa date de creation.
    pub created_at: Timestamp,
}

/// Une ligne de `prompts`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptRecord {
    /// La session qui l'a recu.
    pub session: SessionId,
    /// Le texte, en entier. C'est lui qui rend la chaine auditable complete.
    pub content: String,
    /// Quand.
    pub ts: Timestamp,
}

/// Une ligne de `reads`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadRecord {
    /// Le projet.
    pub project: ProjectId,
    /// La session qui a lu.
    pub session: SessionId,
    /// Le chemin, **relatif a la racine du projet**.
    pub path: PathBuf,
    /// L'empreinte du contenu lu.
    pub hash: ContentHash,
    /// Quand.
    pub ts: Timestamp,
}

/// Une ligne de `writes`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteRecord {
    /// Le projet.
    pub project: ProjectId,
    /// La session qui a ecrit.
    pub session: SessionId,
    /// Le numero de sequence, local au projet.
    pub seq: Seq,
    /// Le chemin, **relatif a la racine du projet**.
    pub path: PathBuf,
    /// L'empreinte d'avant. `None` a la creation du fichier.
    pub hash_before: Option<ContentHash>,
    /// L'empreinte d'apres.
    pub hash_after: ContentHash,
    /// Le verdict rendu — `Verdict::label()`.
    pub verdict: String,
    /// Quand.
    pub ts: Timestamp,
}

/// Une ligne de `resource_claims`.
///
/// Les reservations sont **globales**, pas par projet : le port 3000 est machine-wide.
/// Deux projets qui lancent chacun leur dev server, c'est le premier vrai conflit
/// inter-projets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceClaimRecord {
    /// La ressource, sous forme de chaine qualifiee : `port:3000`, `db:dev`.
    pub resource: String,
    /// Le projet qui la tient.
    pub project: ProjectId,
    /// La session qui la tient.
    pub session: SessionId,
    /// Quand.
    pub claimed_at: Timestamp,
}
