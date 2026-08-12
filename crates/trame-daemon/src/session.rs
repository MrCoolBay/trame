//! ★ **La chaine complete.** Le pilote d'une session : agent ↔ registre ↔ avis.
//!
//! C'est ici que tout ce qui precede se rencontre, et le tout tient en un `match` :
//!
//! ```text
//! FileRead  -> RecordRead                        (alimente le read-set)
//! FileWrite -> Admit  ──> Clean     : ecrit, journalise, silence
//!                     └─> StaleRead : ecrit, journalise, ET injecte un avis
//! ```
//!
//! # L'ordre est non negociable
//!
//! Le registre **ecrit** (ADR 0014), puis on acquitte a l'agent. Acquitter avant que le
//! fichier soit sur le disque ferait croire a l'agent une ecriture qui n'a pas eu lieu.
//! Une [`trame_agent::FileWriteRequest`] abandonnee refuse par defaut, donc un chemin
//! d'erreur oublie ne peut pas se transformer en ecriture non admise.
//!
//! # Ou l'avis est injecte
//!
//! Pas au moment du verdict : l'agent est au milieu d'un tool call, il n'y a pas de canal
//! pour lui parler. L'avis est **retenu** et pose devant le prochain message envoye a la
//! session, via `PromptPipeline`. C'est ce que fait [`SessionPilot::take_notice`].

use std::path::PathBuf;
use std::sync::Arc;

use trame_agent::{AgentBackend, AgentEvent, AgentEventStream, UserMessage};
use trame_core::clock::Clock;
use trame_core::prompt::{PromptPipeline, SessionContext, StaleReadNotice};
use trame_core::{Project, ProjectRoot, Session, SessionId, Verdict};
use trame_registry::{ReadKind, RegistryHandle};

/// Ce qu'une session a produit, pour l'interface et les tests.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SessionActivity {
    /// Les lectures transmises au registre.
    pub reads: Vec<PathBuf>,
    /// Les ecritures admises, avec le libelle du verdict rendu.
    pub writes: Vec<(PathBuf, String)>,
    /// Les ecritures refusees, avec le motif transmis a l'agent.
    pub refusals: Vec<(PathBuf, String)>,
    /// Les avis injectes, dans l'ordre.
    pub notices: Vec<String>,
    /// Les messages de l'agent, concatenes.
    pub message: String,
}

/// Pilote une session : consomme le flux de l'agent et parle au registre.
pub struct SessionPilot {
    session: Session,
    project: Project,
    root: ProjectRoot,
    registry: RegistryHandle,
    clock: Arc<dyn Clock>,
    pipeline: PromptPipeline,
    /// L'avis en attente, a poser devant le prochain message.
    pending_notice: Option<Verdict>,
    activity: SessionActivity,
}

impl std::fmt::Debug for SessionPilot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionPilot")
            .field("session", &self.session.id)
            .field("pending_notice", &self.pending_notice.is_some())
            .finish_non_exhaustive()
    }
}

impl SessionPilot {
    /// Construit le pilote d'une session.
    ///
    /// Le pipeline de prompt contient [`StaleReadNotice`] : c'est le seul contributeur de
    /// la v0.1, et c'est celui qui porte le produit.
    pub fn new(
        session: Session,
        project: Project,
        root: ProjectRoot,
        registry: RegistryHandle,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            session,
            project,
            root,
            registry,
            clock,
            pipeline: PromptPipeline::new().with(StaleReadNotice),
            pending_notice: None,
            activity: SessionActivity::default(),
        }
    }

    /// Fait connaitre la session au registre, pour que son nom apparaisse dans les avis.
    pub async fn register(&self) -> Result<(), trame_registry::RegistryGone> {
        self.registry
            .register_session(self.session.id, self.session.name.clone())
            .await
    }

    /// Ce qui a ete observe jusqu'ici.
    #[must_use]
    pub fn activity(&self) -> &SessionActivity {
        &self.activity
    }

    /// L'identifiant de la session pilotee.
    #[must_use]
    pub fn session_id(&self) -> SessionId {
        self.session.id
    }

    /// ★ Traite un evenement de l'agent. **Le coeur du cablage.**
    pub async fn handle(&mut self, event: AgentEvent) {
        match event {
            // Une lecture : le client sert le contenu, et le registre l'enregistre. C'est
            // ce qui remplit le read-set, donc ce qui rend `StaleRead` possible.
            AgentEvent::FileRead(request) => {
                let absolu = self.absolutize(&request.path);
                match tokio::fs::read_to_string(&absolu).await {
                    Ok(content) => {
                        // Une lecture servie par nous est une lecture complete : c'est la
                        // seule forme substantielle (voir `ReadKind`).
                        let _ = self
                            .registry
                            .record_read(
                                self.session.id,
                                request.path.clone(),
                                content.clone(),
                                ReadKind::FullFile,
                            )
                            .await;
                        if let Ok(key) = self.root.relativize(&request.path) {
                            self.activity.reads.push(key);
                        }
                        request.provide(content);
                    }
                    Err(error) => {
                        // Un fichier absent est un cas normal : l'agent explore.
                        tracing::debug!(path = %absolu.display(), %error, "lecture impossible");
                        request.fail(error.to_string());
                    }
                }
            }

            // ★ Une ecriture. Le registre admet ET ecrit, puis on acquitte.
            AgentEvent::FileWrite(request) => {
                let path = request.path.clone();
                match self
                    .registry
                    .admit(self.session.id, path.clone(), request.content.clone())
                    .await
                {
                    Ok(verdict) => {
                        let key = self.root.relativize(&path).unwrap_or(path);
                        self.activity.writes.push((key, verdict.label().to_owned()));

                        // L'avis n'est pas injectable maintenant — l'agent est au milieu
                        // d'un tool call. On le retient pour le prochain message.
                        if verdict.needs_notice() {
                            self.pending_notice = Some(verdict);
                        }
                        // Le fichier est sur le disque : on peut acquitter.
                        request.admitted();
                    }
                    Err(error) => {
                        // Refus explicite, avec son motif. L'agent sait traiter un outil
                        // en echec ; le laisser attendre serait pire.
                        let motif = error.to_string();
                        tracing::warn!(path = %path.display(), %motif, "ecriture refusee");
                        self.activity.refusals.push((path, motif.clone()));
                        request.refuse(motif);
                    }
                }
            }

            AgentEvent::PermissionRequest(request) => {
                // `allow_once` et jamais `allow_always` : une option persistante fait
                // ecrire un fichier de reglages dans le repertoire de travail, hors
                // admission (ADR 0016).
                match request.allow_once().map(|option| option.id.clone()) {
                    Some(id) => request.choose(id),
                    None => {
                        tracing::warn!(
                            tool = %request.tool_name,
                            "aucune option d'autorisation non persistante : annulation"
                        );
                        request.cancel();
                    }
                }
            }

            AgentEvent::Message(text) => self.activity.message.push_str(&text),
            AgentEvent::ToolCall { .. } | AgentEvent::Done => {}
            AgentEvent::Error(error) => tracing::error!(%error, "erreur du harness"),
            _ => {}
        }
    }

    /// L'avis a poser devant le prochain message, s'il y en a un.
    ///
    /// Le consomme : un avis ne s'injecte qu'une fois. Le repeter a chaque tour serait du
    /// bruit, et le bruit fait desactiver la fonctionnalite.
    pub fn take_notice(&mut self) -> Option<String> {
        let verdict = self.pending_notice.take()?;
        let ctx = SessionContext::new(&self.session, &self.project, self.clock.now())
            .with_last_verdict(&verdict);
        let notice = self.pipeline.render(&ctx);
        if let Some(notice) = &notice {
            self.activity.notices.push(notice.clone());
        }
        notice
    }

    /// Envoie un message a l'agent, **avec l'avis en attente s'il y en a un**.
    pub async fn send(
        &mut self,
        backend: &mut dyn AgentBackend,
        text: impl Into<String>,
    ) -> Result<(), trame_agent::AgentError> {
        let mut message = UserMessage::new(text);
        if let Some(notice) = self.take_notice() {
            message = message.with_context(notice);
        }
        backend.send(message).await
    }

    /// Consomme le flux jusqu'a `Done` ou `Error`.
    pub async fn run_turn(&mut self, events: &mut AgentEventStream) {
        while let Some(event) = events.next().await {
            let fin = matches!(event, AgentEvent::Done | AgentEvent::Error(_));
            self.handle(event).await;
            if fin {
                break;
            }
        }
    }

    fn absolutize(&self, path: &std::path::Path) -> PathBuf {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.resolve(path)
        }
    }
}
