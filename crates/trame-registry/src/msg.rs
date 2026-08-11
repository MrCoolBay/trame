//! Les messages du registre, et ce qu'ils rendent.

use std::path::PathBuf;

use tokio::sync::oneshot;
use trame_core::{ContentHash, ProjectId, Seq, SessionId, Verdict};

/// Ce qu'on peut demander au registre.
///
/// Une variante par operation : pas de `Msg::Do { op }`, qui deplacerait le dispatch
/// dans un second `match` et supprimerait le typage des reponses.
pub(crate) enum RegistryMsg {
    /// Fait connaitre une session et son nom affichable.
    ///
    /// Le nom sert a l'avis injecte : « auth.rs a ete modifie par la session
    /// *refacto-api* » est actionnable, un UUID ne l'est pas.
    RegisterSession {
        session: SessionId,
        name: String,
        reply: oneshot::Sender<()>,
    },

    /// Un agent a lu un fichier. Entre dans le read-set **si** la lecture est
    /// substantielle.
    RecordRead {
        session: SessionId,
        path: PathBuf,
        content: String,
        kind: ReadKind,
        reply: oneshot::Sender<()>,
    },

    /// ★ Un agent veut ecrire. **C'est le controleur d'admission.**
    Admit {
        session: SessionId,
        path: PathBuf,
        content: String,
        reply: oneshot::Sender<Verdict>,
    },

    /// L'etat courant, pour l'interface et les tests.
    Snapshot(oneshot::Sender<RegistrySnapshot>),
}

/// La nature d'une lecture.
///
/// **C'est le filtre du read-set**, et il est ici plutot que chez l'appelant pour que
/// la politique vive a un seul endroit. Les agents lisent enormement : si tout entrait
/// dans le read-set, il exploserait et tout deviendrait `StaleRead` — ce qui
/// reviendrait a desactiver la fonctionnalite en la rendant inutilisable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ReadKind {
    /// Un fichier lu en entier. **La seule lecture substantielle.**
    FullFile,
    /// Un resultat de recherche : quelques lignes de contexte. L'agent n'a pas vu le
    /// fichier, il a vu une correspondance.
    GrepHit,
    /// Un listing de repertoire. Aucun contenu lu.
    DirListing,
    /// Des metadonnees seules : existence, taille, date.
    Metadata,
}

impl ReadKind {
    /// Vrai si cette lecture entre dans le read-set.
    #[must_use]
    pub fn is_substantial(self) -> bool {
        matches!(self, Self::FullFile)
    }
}

/// L'etat du registre a un instant donne.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistrySnapshot {
    /// Le projet. Un registre par projet, jamais partage.
    pub project: ProjectId,
    /// Le dernier numero de sequence attribue. `Seq::from_u64(0)` si aucune ecriture.
    pub seq: Seq,
    /// Les fichiers connus, tries par chemin.
    pub files: Vec<FileSnapshot>,
    /// Les sessions connues, triees par identifiant.
    pub sessions: Vec<SessionSnapshot>,
}

/// L'etat d'un fichier suivi.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSnapshot {
    /// Son chemin, relatif a la racine du projet.
    pub path: PathBuf,
    /// La derniere session a l'avoir ecrit.
    pub last_writer: SessionId,
    /// Le numero de sequence de cette ecriture.
    pub last_seq: Seq,
    /// L'empreinte du contenu courant.
    pub hash: ContentHash,
}

/// L'etat d'une session vue par le registre.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSnapshot {
    /// Son identifiant.
    pub session: SessionId,
    /// Son nom affichable.
    pub name: String,
    /// Les fichiers de son read-set **non expires**, tries par chemin.
    pub read_set: Vec<PathBuf>,
    /// Les fichiers qu'elle a ecrits, tries par chemin.
    pub write_set: Vec<PathBuf>,
}
