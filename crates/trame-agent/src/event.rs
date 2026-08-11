//! Le flux d'evenements normalise.
//!
//! # Pourquoi certains « evenements » sont des requetes
//!
//! Un flux est unidirectionnel : on ne peut pas intercepter avec un flux. Or c'est
//! precisement ce qu'on doit faire pour les ecritures — decider **avant** que le disque
//! soit touche.
//!
//! Les variantes concernees portent donc un canal de reponse. Le type garantit alors ce
//! que la documentation ne pourrait que demander : ignorer une [`FileWriteRequest`] est
//! impossible sans que quelqu'un s'en apercoive, parce qu'elle refuse par defaut en
//! tombant.

use std::path::PathBuf;

use serde_json::Value;
use tokio::sync::oneshot;

/// Ce que le backend remonte au reste du core.
///
/// Le reste du core ne sait jamais s'il parle a de l'ACP ou a un PTY.
#[derive(Debug)]
#[non_exhaustive]
pub enum AgentEvent {
    /// Un fragment de message de l'agent.
    Message(String),

    /// Un appel d'outil, tel que l'agent l'annonce. Informatif : les outils qui
    /// touchent aux fichiers remontent en plus comme requetes.
    ToolCall {
        /// Le nom de l'outil.
        name: String,
        /// Ses parametres, non interpretes.
        input: Value,
    },

    /// L'agent demande a **lire** un fichier. C'est le client qui sert la lecture, donc
    /// c'est lui qui connait le contenu — et c'est ce qui alimente le read-set.
    FileRead(FileReadRequest),

    /// ★ L'agent demande a **ecrire**. **Rien n'est ecrit avant la reponse.**
    FileWrite(FileWriteRequest),

    /// L'agent demande une permission a l'humain.
    PermissionRequest(PermissionRequest),

    /// L'agent a rendu la main.
    Done,

    /// Quelque chose a echoue. Un sous-process qui meurt est un cas normal, pas une
    /// panique.
    Error(String),
}

/// Une demande de lecture adressee au client.
#[derive(Debug)]
pub struct FileReadRequest {
    /// Le chemin demande, tel que l'agent l'a formule.
    pub path: PathBuf,
    /// Ligne de depart, si l'agent n'a demande qu'une portion.
    pub line: Option<u32>,
    /// Nombre de lignes, si l'agent n'a demande qu'une portion.
    pub limit: Option<u32>,
    reply: Option<oneshot::Sender<Result<String, String>>>,
}

impl FileReadRequest {
    /// Construit la requete et son canal de reponse.
    pub(crate) fn new(
        path: PathBuf,
        line: Option<u32>,
        limit: Option<u32>,
    ) -> (Self, oneshot::Receiver<Result<String, String>>) {
        let (tx, rx) = oneshot::channel();
        (
            Self {
                path,
                line,
                limit,
                reply: Some(tx),
            },
            rx,
        )
    }

    /// Fournit le contenu lu.
    ///
    /// **C'est aussi le moment d'alimenter le read-set** : le client est le seul a
    /// savoir ce que l'agent a reellement vu.
    pub fn provide(mut self, content: impl Into<String>) {
        if let Some(reply) = self.reply.take() {
            let _ = reply.send(Ok(content.into()));
        }
    }

    /// Signale que la lecture est impossible.
    pub fn fail(mut self, reason: impl Into<String>) {
        if let Some(reply) = self.reply.take() {
            let _ = reply.send(Err(reason.into()));
        }
    }
}

impl Drop for FileReadRequest {
    fn drop(&mut self) {
        if let Some(reply) = self.reply.take() {
            tracing::warn!(path = %self.path.display(), "requete de lecture abandonnee");
            let _ = reply.send(Err("lecture abandonnee par le client".to_owned()));
        }
    }
}

/// ★ Une demande d'ecriture adressee au client. **Le point d'admission.**
///
/// Tant que [`FileWriteRequest::admitted`] ou [`FileWriteRequest::refuse`] n'a pas ete
/// appele, l'agent attend et **le disque n'a pas ete touche**.
#[derive(Debug)]
pub struct FileWriteRequest {
    /// Le chemin vise, tel que l'agent l'a formule.
    pub path: PathBuf,
    /// Le contenu propose, en entier.
    pub content: String,
    reply: Option<oneshot::Sender<Result<(), String>>>,
}

impl FileWriteRequest {
    /// Construit la requete et son canal de reponse.
    pub(crate) fn new(
        path: PathBuf,
        content: String,
    ) -> (Self, oneshot::Receiver<Result<(), String>>) {
        let (tx, rx) = oneshot::channel();
        (
            Self {
                path,
                content,
                reply: Some(tx),
            },
            rx,
        )
    }

    /// L'ecriture a ete **admise et effectuee** par le registre (ADR 0014).
    ///
    /// A n'appeler qu'apres que le fichier est reellement sur le disque : c'est ce que
    /// l'agent va croire.
    pub fn admitted(mut self) {
        if let Some(reply) = self.reply.take() {
            let _ = reply.send(Ok(()));
        }
    }

    /// L'ecriture est refusee. Le motif remonte a l'agent, qui sait deja quoi faire
    /// d'un outil en echec.
    pub fn refuse(mut self, reason: impl Into<String>) {
        if let Some(reply) = self.reply.take() {
            let _ = reply.send(Err(reason.into()));
        }
    }
}

impl Drop for FileWriteRequest {
    /// **Refus par defaut.**
    ///
    /// Laisser tomber une demande d'ecriture sans repondre laisserait l'agent attendre
    /// indefiniment. Pire : si on repondait « admis » par defaut, une requete oubliee
    /// deviendrait une ecriture non admise — exactement ce que le produit existe pour
    /// empecher. Le defaut est donc le refus, et il est bruyant.
    fn drop(&mut self) {
        if let Some(reply) = self.reply.take() {
            tracing::warn!(
                path = %self.path.display(),
                "demande d'ecriture abandonnee sans decision : refus par defaut"
            );
            let _ = reply.send(Err("ecriture non admise : requete abandonnee".to_owned()));
        }
    }
}

/// Une demande de permission adressee a l'humain.
///
/// Le mecanisme existe deja cote agent : il sait attendre une permission, il n'y a rien
/// a lui apprendre. C'est par la que le niveau 3 du registre passera, en v0.4.
#[derive(Debug)]
pub struct PermissionRequest {
    /// Ce que l'agent veut faire, en une ligne affichable.
    pub title: String,
    /// L'outil concerne.
    pub tool_name: String,
    /// Les options proposees par l'agent.
    pub options: Vec<PermissionOption>,
    reply: Option<oneshot::Sender<Option<String>>>,
}

/// Une option de permission, telle que l'agent la propose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionOption {
    /// L'identifiant a renvoyer pour la choisir.
    pub id: String,
    /// Le libelle affichable.
    pub label: String,
    /// La nature de l'option, telle qu'annoncee par l'agent (`allow_once`, `reject`…).
    pub kind: String,
}

impl PermissionRequest {
    pub(crate) fn new(
        title: String,
        tool_name: String,
        options: Vec<PermissionOption>,
    ) -> (Self, oneshot::Receiver<Option<String>>) {
        let (tx, rx) = oneshot::channel();
        (
            Self {
                title,
                tool_name,
                options,
                reply: Some(tx),
            },
            rx,
        )
    }

    /// Choisit une option, par son identifiant.
    pub fn choose(mut self, option_id: impl Into<String>) {
        if let Some(reply) = self.reply.take() {
            let _ = reply.send(Some(option_id.into()));
        }
    }

    /// Annule : l'agent abandonne le tour.
    pub fn cancel(mut self) {
        if let Some(reply) = self.reply.take() {
            let _ = reply.send(None);
        }
    }

    /// La premiere option dont la nature contient `allow`, s'il y en a une.
    #[must_use]
    pub fn first_allow(&self) -> Option<&PermissionOption> {
        self.options
            .iter()
            .find(|option| option.kind.contains("allow"))
    }
}

impl Drop for PermissionRequest {
    fn drop(&mut self) {
        if let Some(reply) = self.reply.take() {
            tracing::warn!(tool = %self.tool_name, "demande de permission abandonnee");
            let _ = reply.send(None);
        }
    }
}
