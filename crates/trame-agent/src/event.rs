//! Le feed d'evenements normalise.
//!
//! # Pourquoi certains « evenements » sont des requetes
//!
//! Un feed est unidirectionnel : on ne peut pas intercepter avec un feed. Or c'est
//! precisement ce qu'on doit faire pour les ecritures — decider **avant** que le disque
//! soit touche.
//!
//! Les variantes concernees portent donc un canal de reponse. Le type garantit alors ce
//! que la documentation ne pourrait que ask : ignorer une [`FileWriteRequest`] est
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

    /// L'agent demande a **lire** un file. C'est le client qui sert la lecture, donc
    /// c'est lui qui connait le content — et c'est ce qui alimente le read-set.
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
    /// Le path demande, tel que l'agent l'a formule.
    pub path: PathBuf,
    /// Ligne de depart, si l'agent n'a demande qu'une portion.
    pub line: Option<u32>,
    /// Nombre de lines, si l'agent n'a demande qu'une portion.
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

    /// Fournit le content lu.
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
    /// Le path vise, tel que l'agent l'a formule.
    pub path: PathBuf,
    /// Le content propose, en entier.
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
    /// A n'appeler qu'apres que le file est reellement sur le disque : c'est ce que
    /// l'agent va croire.
    pub fn admitted(mut self) {
        if let Some(reply) = self.reply.take() {
            let _ = reply.send(Ok(()));
        }
    }

    /// L'ecriture est refusee. Le reason remonte a l'agent, qui sait deja quoi faire
    /// d'un outil en echec.
    pub fn refuse(mut self, reason: impl Into<String>) {
        if let Some(reply) = self.reply.take() {
            let _ = reply.send(Err(reason.into()));
        }
    }
}

impl Drop for FileWriteRequest {
    /// **Deny par defaut.**
    ///
    /// Allow tomber une demande d'ecriture sans repondre laisserait l'agent wait_for
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
/// Le mecanisme existe deja cote agent : il sait wait_for une permission, il n'y a rien
/// a lui apprendre. C'est par la que le niveau 3 du registre passera, en v0.4.
#[derive(Debug)]
pub struct PermissionRequest {
    /// Ce que l'agent veut faire, en une line affichable.
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
    /// La nature de l'option, telle qu'annoncee par l'agent : `allow_once`,
    /// `allow_always`, `reject_once`, `reject_always`.
    pub kind: String,
}

impl PermissionOption {
    /// Vrai si cette option autorise l'action.
    #[must_use]
    pub fn is_allow(&self) -> bool {
        self.kind.starts_with("allow")
    }

    /// Vrai si choisir cette option **fait ecrire une decision persistante**.
    ///
    /// # Pourquoi c'est important, et pas un detail
    ///
    /// Constate a la validation live : choisir `allow_always` a fait ecrire
    /// `.claude/settings.local.json` **dans le repertoire de travail du projet**, avec
    /// `{"permissions":{"allow":["mcp__acp__Write"]}}`. Ce file n'est jamais passe par
    /// `fs/write_text_file` : c'est une **ecriture hors-bande, a l'interieur du projet**,
    /// provoquee par notre propre choix.
    ///
    /// Autrement dit : en repondant a une demande de permission, on peut se salir
    /// soi-meme l'arbre qu'on est cense surveiller.
    #[must_use]
    pub fn is_persistent(&self) -> bool {
        self.kind.ends_with("always")
    }
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

    /// Annule : l'agent abandonne le turn.
    pub fn cancel(mut self) {
        if let Some(reply) = self.reply.take() {
            let _ = reply.send(None);
        }
    }

    /// Une option qui autorise **sans rien persister**.
    ///
    /// Prefere systematiquement `allow_once` a `allow_always`. Ce n'est pas de la
    /// prudence gratuite : choisir une option persistante fait ecrire un file de
    /// reglages **dans le repertoire de travail du projet**, hors admission — voir
    /// [`PermissionOption::is_persistent`]. On ne salit pas l'arbre qu'on surveille.
    ///
    /// Rend `None` si l'agent ne propose que des options persistantes : dans ce cas c'est
    /// a l'humain de trancher, pas a un defaut silencieux.
    #[must_use]
    pub fn allow_once(&self) -> Option<&PermissionOption> {
        self.options
            .iter()
            .find(|option| option.is_allow() && !option.is_persistent())
    }

    /// Une option qui refuse sans rien persister.
    #[must_use]
    pub fn reject_once(&self) -> Option<&PermissionOption> {
        self.options
            .iter()
            .find(|option| !option.is_allow() && !option.is_persistent())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn options() -> Vec<PermissionOption> {
        // L'ordre est celui observe en live : `allow_always` arrivait AVANT `allow_once`,
        // ce qui est exactement pourquoi « la premiere qui autorise » etait un mauvais
        // critere.
        vec![
            PermissionOption {
                id: "aa".into(),
                label: "Toujours".into(),
                kind: "allow_always".into(),
            },
            PermissionOption {
                id: "ao".into(),
                label: "Une fois".into(),
                kind: "allow_once".into(),
            },
            PermissionOption {
                id: "ro".into(),
                label: "Deny".into(),
                kind: "reject_once".into(),
            },
        ]
    }

    #[test]
    fn a_non_persistent_permission_is_always_preferred() {
        let (request, _rx) = PermissionRequest::new("Write".into(), "Write".into(), options());
        assert_eq!(
            request.allow_once().map(|option| option.id.as_str()),
            Some("ao"),
            "allow_always est propose en premier : le prendre ferait ecrire un file \
             de reglages dans le repertoire de travail du projet"
        );
        assert_eq!(
            request.reject_once().map(|option| option.id.as_str()),
            Some("ro")
        );
    }

    #[test]
    fn with_no_non_persistent_option_we_do_not_choose_for_the_human() {
        let persistantes = vec![PermissionOption {
            id: "aa".into(),
            label: "Toujours".into(),
            kind: "allow_always".into(),
        }];
        let (request, _rx) = PermissionRequest::new("Write".into(), "Write".into(), persistantes);
        assert!(request.allow_once().is_none());
    }

    #[test]
    fn each_permission_option_kind_is_read_correctly() {
        for option in options() {
            assert_eq!(option.is_allow(), option.kind.starts_with("allow"));
            assert_eq!(option.is_persistent(), option.kind.ends_with("always"));
        }
    }
}
