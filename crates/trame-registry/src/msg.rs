//! Les messages du registre, et ce qu'ils rendent.

use std::path::PathBuf;

use tokio::sync::oneshot;
use trame_core::{ContentHash, ProjectId, Seq, SessionId, Verdict};

use crate::error::RegistryError;

/// ★ Ce que les lectures `Grep` **auraient** produit, si elles comptaient.
///
/// # Pourquoi une mesure et pas une bascule
///
/// `ReadKind::GrepHit` n'est pas substantiel, donc les fichiers rapportes par une recherche
/// n'entrent pas dans le read-set et ne peuvent produire aucun avis. Le trou lecture reste
/// donc ouvert (ADR 0027).
///
/// Le fermer en rendant `GrepHit` substantiel serait un **pari**, pas une decision : la manche
/// experimentale mesure des taux de succes, jamais le taux de **faux positifs** — et c'est
/// precisement la variable du risque produit numero un (invariant 8). Le mode ombre produit
/// cette donnee sans rien risquer : il compte ce qui aurait ete dit, et ne dit rien.
///
/// # Pourquoi la distribution, et pas un seuil
///
/// Un `Grep` qui rend trois fichiers est une lecture ciblee ; un `grep -r` qui en rend trois
/// cents est une exploration. Ce ne sont pas la meme chose, et un booleen force le faux choix
/// entre aucune couverture et tout le bruit.
///
/// On enregistre donc, pour chaque avis potentiel, **la taille du resultat du `Grep` d'ou il
/// vient**. [`StatsOmbre::avis_potentiels_si_seuil`] repond alors pour **n'importe quel** N,
/// apres coup. C'est ce qui evite de choisir N a l'intuition : la mesure le donnera.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StatsOmbre {
    /// Le nombre total d'avis que les hits `Grep` auraient produits.
    pub avis_potentiels: u64,
    /// Combien d'avis potentiels par **taille du resultat** du `Grep` d'origine.
    ///
    /// C'est cette distribution qui donnera le seuil, pas une intuition.
    pub par_taille: std::collections::BTreeMap<usize, u64>,
    /// Combien de lectures `Grep` ont ete enregistrees en ombre, tous fichiers confondus.
    ///
    /// Le denominateur : sans lui, « douze avis potentiels » ne veut rien dire.
    pub lectures_ombre: u64,
}

impl StatsOmbre {
    /// Combien d'avis un seuil de `n` fichiers **aurait** produits.
    ///
    /// **`n` n'a pas de valeur par defaut, et n'en aura pas avant la mesure.** C'est le
    /// parametre de l'experience : « si on avait compte les `Grep` rendant au plus n fichiers,
    /// combien d'avis aurait-on emis ? ». La reponse pour tout n se lit dans la meme
    /// distribution, ce qui evite de rejouer la mesure pour chaque hypothese.
    #[must_use]
    pub fn avis_potentiels_si_seuil(&self, n: usize) -> u64 {
        self.par_taille
            .iter()
            .filter(|(taille, _)| **taille <= n)
            .map(|(_, compte)| *compte)
            .sum()
    }
}

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

    /// ★ Un agent veut ecrire. **C'est le controleur d'admission**, et c'est lui qui
    /// effectue l'ecriture (ADR 0014) — d'ou le `Result`.
    Admit {
        session: SessionId,
        path: PathBuf,
        content: String,
        reply: oneshot::Sender<Result<Verdict, RegistryError>>,
    },

    /// Une ecriture **hors-bande** a ete constatee par le watcher.
    ///
    /// Elle n'a pas ete admise et n'a rien pu etre empeche : le watcher constate apres
    /// coup. Le message existe pour que le registre ne devienne pas **faux** — sans lui,
    /// un `sed -i` laisse un `FileState` perime et le `StaleRead` correspondant ne se
    /// declenche jamais.
    ObserveExternalWrite {
        path: PathBuf,
        hash: ContentHash,
        reply: oneshot::Sender<ExternalWrite>,
    },

    /// ★ Une lecture rapportee par une recherche, enregistree **en ombre**.
    ///
    /// Elle n'entre pas dans le read-set reel et ne peut donc produire aucun avis : elle sert
    /// uniquement a compter ce qui aurait ete dit (voir [`StatsOmbre`]).
    ///
    /// `taille_resultat` est le nombre de fichiers rendus par le `Grep` d'origine. C'est lui
    /// qui rend le seuil decidable apres coup.
    RecordShadowRead {
        session: SessionId,
        path: PathBuf,
        content: String,
        taille_resultat: usize,
        reply: oneshot::Sender<()>,
    },

    /// Les compteurs du mode ombre.
    StatsOmbre(oneshot::Sender<StatsOmbre>),

    /// L'etat courant, pour l'interface et les tests.
    Snapshot(oneshot::Sender<RegistrySnapshot>),
}

/// Ce que le registre a fait d'une observation hors-bande.
///
/// **Un `()` en reponse ne suffisait pas**, et le defaut n'etait pas theorique : le
/// registre ecrit lui-meme sur le disque (ADR 0014), donc le watcher voit **aussi ses
/// propres ecritures**. Sans cette distinction, l'appelant les signale comme hors-bande, et
/// une interface affiche comme « constate apres coup, sans verdict » une ecriture qui est
/// passee par l'admission avec un verdict. C'est exactement l'inverse de la verite.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ExternalWrite {
    /// Nouvelle empreinte : l'etat du registre a ete rattrape et la ligne journalisee,
    /// attribuee a [`trame_core::SessionId::EXTERNAL`] et **sans verdict**.
    Recorded {
        /// Le numero de sequence attribue, local au projet.
        seq: Seq,
    },
    /// Empreinte identique a celle deja connue : c'est l'echo d'une ecriture admise.
    /// Rien n'a ete journalise, et **rien ne doit etre affiche**.
    Echo,
}

impl ExternalWrite {
    /// Vrai si cette observation etait une vraie ecriture hors-bande.
    #[must_use]
    pub const fn is_recorded(self) -> bool {
        matches!(self, Self::Recorded { .. })
    }
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
    /// Ce que les lectures `Grep` auraient produit. **Aucun effet sur les verdicts.**
    pub ombre: StatsOmbre,
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
