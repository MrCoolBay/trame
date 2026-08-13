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
use trame_core::{Project, ProjectRoot, Session, SessionId, SessionState, Verdict};
use trame_registry::{ReadKind, RegistryHandle};

use crate::observe::{Observation, Observer, Transport};

/// Comment un tour s'est termine.
///
/// Explicite plutot que booleen : un tour expire et un tour en echec ne se comptent pas
/// de la meme facon dans une manche experimentale.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TurnOutcome {
    /// L'agent a rendu la main. C'est la reponse a `session/prompt` qui le dit.
    Done,
    /// Le tour a echoue, avec son motif.
    Failed(String),
    /// Le flux s'est ferme avant la fin : le harness est parti.
    StreamClosed,
}

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
    /// Le dernier cumul d'avis potentiels transmis a l'interface.
    ///
    /// Garde pour n'emettre que sur variation : reemettre un compteur inchange a chaque
    /// ecriture remplirait le flux de lignes qui ne disent rien.
    avis_potentiels: u64,
    activity: SessionActivity,
    /// Le canal d'observation, s'il y a une interface en face.
    observer: Option<Observer>,
    /// Ce qui est garanti pour cette session. `Absent` tant que personne ne l'a declare.
    transport: Transport,
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
            observer: None,
            transport: Transport::Absent,
            avis_potentiels: 0,
        }
    }

    /// Fait observer cette session par une interface, **sans lui donner la main**.
    ///
    /// Les deux arguments vont ensemble : afficher une session sans dire par quel transport
    /// elle est pilotee laisserait l'utilisateur supposer la garantie d'admission. Le
    /// transport se lit sur les capacites reelles du backend :
    ///
    /// ```no_run
    /// # use trame_daemon::{Transport, observe_channel};
    /// # fn demo(pilot: trame_daemon::SessionPilot, backend: &dyn trame_agent::AgentBackend) {
    /// let (observer, _rx) = observe_channel();
    /// let pilot = pilot.observed_by(observer, Transport::from(backend.capabilities()));
    /// # }
    /// ```
    #[must_use]
    pub fn observed_by(mut self, observer: Observer, transport: Transport) -> Self {
        self.observer = Some(observer);
        self.transport = transport;
        self
    }

    fn observe(&mut self, observation: Observation) {
        if let Some(observer) = self.observer.as_mut() {
            observer.emit(observation);
        }
    }

    fn observe_state(&mut self, state: SessionState) {
        let session = self.session.id;
        self.observe(Observation::StateChanged { session, state });
    }

    /// Remplace le pipeline de composition du prompt.
    ///
    /// Sert a la manche experimentale, qui substitue une variante d'avis a
    /// [`StaleReadNotice`] pour la mesurer. Une fois la variante tranchee, ce point
    /// d'extension n'aura plus de raison d'etre.
    #[must_use]
    pub fn with_pipeline(mut self, pipeline: PromptPipeline) -> Self {
        self.pipeline = pipeline;
        self
    }

    /// Fait connaitre la session au registre, pour que son nom apparaisse dans les avis.
    pub async fn register(&mut self) -> Result<(), trame_registry::RegistryGone> {
        self.registry
            .register_session(self.session.id, self.session.name.clone())
            .await?;
        let (session, name, transport) =
            (self.session.id, self.session.name.clone(), self.transport);
        self.observe(Observation::SessionOpened {
            session,
            name,
            transport,
        });
        self.observe_state(SessionState::Idle);
        Ok(())
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
                            self.activity.reads.push(key.clone());
                            let session = self.session.id;
                            self.observe(Observation::Read { session, path: key });
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
                // L'etat passe a Writing *avant* l'admission : c'est pendant l'admission
                // que l'agent attend, donc c'est ce moment-la qu'il faut donner a voir.
                self.observe_state(SessionState::Writing);
                match self
                    .registry
                    .admit(self.session.id, path.clone(), request.content.clone())
                    .await
                {
                    Ok(verdict) => {
                        let key = self.root.relativize(&path).unwrap_or(path);
                        self.activity
                            .writes
                            .push((key.clone(), verdict.label().to_owned()));
                        let session = self.session.id;
                        self.observe(Observation::Write {
                            session,
                            path: key,
                            verdict: verdict.clone(),
                        });

                        // L'avis n'est pas injectable maintenant — l'agent est au milieu
                        // d'un tool call. On le retient pour le prochain message.
                        if verdict.needs_notice() {
                            self.pending_notice = Some(verdict);
                        }
                        // Le fichier est sur le disque : on peut acquitter.
                        request.admitted();

                        // ★ Le compteur du mode ombre, s'il a bouge. Il ne vient PAS du verdict :
                        // aucun avis n'a ete injecte, c'est une mesure (ADR 0027).
                        if let Ok(stats) = self.registry.stats_ombre().await
                            && stats.avis_potentiels != self.avis_potentiels
                        {
                            self.avis_potentiels = stats.avis_potentiels;
                            self.observe(Observation::AvisPotentiels {
                                total: stats.avis_potentiels,
                            });
                        }
                        self.observe_state(SessionState::Thinking);
                    }
                    Err(error) => {
                        // Refus explicite, avec son motif. L'agent sait traiter un outil
                        // en echec ; le laisser attendre serait pire.
                        let motif = error.to_string();
                        tracing::warn!(path = %path.display(), %motif, "ecriture refusee");
                        self.activity.refusals.push((path.clone(), motif.clone()));
                        let session = self.session.id;
                        self.observe(Observation::Refused {
                            session,
                            path,
                            reason: motif.clone(),
                        });
                        request.refuse(motif);
                        self.observe_state(SessionState::Thinking);
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
            let (session, text) = (self.session.id, notice.clone());
            self.observe(Observation::Notice { session, text });
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

    /// Consomme le flux jusqu'a la fin du tour.
    ///
    /// **La condition d'attente est explicite** : `AgentEvent::Done` ou
    /// `AgentEvent::Error`. `Done` arrive quand la reponse a `session/prompt` revient —
    /// ce n'est pas une notification, et attendre autre chose est une attente qui
    /// n'aboutit jamais.
    pub async fn run_turn(&mut self, events: &mut AgentEventStream) -> TurnOutcome {
        tracing::info!(
            session = %self.session.name,
            "debut de tour — attente de Done (reponse a session/prompt) ou Error"
        );
        self.observe_state(SessionState::Thinking);
        while let Some(event) = events.next().await {
            let fin = match &event {
                AgentEvent::Done => Some(TurnOutcome::Done),
                AgentEvent::Error(message) => Some(TurnOutcome::Failed(message.clone())),
                _ => None,
            };
            self.handle(event).await;
            if let Some(outcome) = fin {
                tracing::info!(
                    session = %self.session.name,
                    ?outcome,
                    lectures = self.activity.reads.len(),
                    ecritures = self.activity.writes.len(),
                    "fin de tour — condition remplie"
                );
                self.observe_state(match &outcome {
                    TurnOutcome::Done => SessionState::Idle,
                    TurnOutcome::Failed(motif) => SessionState::Failed(motif.clone()),
                    _ => SessionState::Idle,
                });
                return outcome;
            }
        }
        tracing::warn!(session = %self.session.name, "flux ferme avant la fin du tour");
        self.observe_state(SessionState::Failed("flux ferme".to_owned()));
        TurnOutcome::StreamClosed
    }

    fn absolutize(&self, path: &std::path::Path) -> PathBuf {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.resolve(path)
        }
    }
}
