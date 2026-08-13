//! ★ **The full chain.** A session's pilot: agent ↔ registry ↔ notice.
//!
//! This is where everything before it meets, and the whole thing fits in one `match`:
//!
//! ```text
//! FileRead  -> RecordRead                        (fills the read-set)
//! FileWrite -> Admit  ──> Clean     : writes, journals, stays silent
//!                     └─> StaleRead : writes, journals, AND injects a notice
//! ```
//!
//! # The order is non-negotiable
//!
//! The registry **writes** (ADR 0014), then we acknowledge to the agent. Acknowledging
//! before the file is on the disk would make the agent believe in a write that never
//! happened. A dropped [`trame_agent::FileWriteRequest`] refuses by default, so a forgotten
//! error path cannot turn into an unadmitted write.
//!
//! # Where the notice is injected
//!
//! Not at verdict time: the agent is in the middle of a tool call, and there is no channel
//! to speak to it. The notice is **held** and placed in front of the next message sent to
//! the session, through `PromptPipeline`. That is what [`SessionPilot::take_notice`] does.

use std::path::PathBuf;
use std::sync::Arc;

use trame_agent::{AgentBackend, AgentEvent, AgentEventStream, UserMessage};
use trame_core::clock::Clock;
use trame_core::prompt::{PromptPipeline, SessionContext, StaleReadNotice};
use trame_core::{Project, ProjectRoot, Session, SessionId, SessionState, Verdict};
use trame_registry::{ReadKind, RegistryHandle};

use crate::observe::{Observation, Observer, Transport};

/// How a turn ended.
///
/// Explicit rather than a boolean: a timed-out turn and a failed turn are not counted the
/// same way in an experimental round.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TurnOutcome {
    /// The agent handed control back. The response to `session/prompt` is what says so.
    Done,
    /// The turn failed, with its reason.
    Failed(String),
    /// The stream closed before the end: the harness is gone.
    StreamClosed,
}

/// What a session produced, for the interface and the tests.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SessionActivity {
    /// The reads passed to the registry.
    pub reads: Vec<PathBuf>,
    /// The admitted writes, with the label of the verdict returned.
    pub writes: Vec<(PathBuf, String)>,
    /// The refused writes, with the reason passed back to the agent.
    pub refusals: Vec<(PathBuf, String)>,
    /// The notices injected, in order.
    pub notices: Vec<String>,
    /// The agent's messages, concatenated.
    pub message: String,
}

/// Drives a session: consumes the agent's stream and talks to the registry.
pub struct SessionPilot {
    session: Session,
    project: Project,
    root: ProjectRoot,
    registry: RegistryHandle,
    clock: Arc<dyn Clock>,
    pipeline: PromptPipeline,
    /// The pending notice, to be placed in front of the next message.
    pending_notice: Option<Verdict>,
    /// The last potential-notice total sent to the interface.
    ///
    /// Kept so that we only emit on change: re-emitting an unchanged counter on every
    /// write would fill the feed with lines that say nothing.
    potential_notices: u64,
    activity: SessionActivity,
    /// The observation channel, if there is an interface on the other end.
    observer: Option<Observer>,
    /// What is guaranteed for this session. `Absent` until someone declares it.
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
    /// Builds a session's pilot.
    ///
    /// The prompt pipeline holds [`StaleReadNotice`]: it is v0.1's only contributor, and
    /// it is the one that carries the product.
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
            potential_notices: 0,
        }
    }

    /// Lets an interface observe this session, **without handing it control**.
    ///
    /// The two arguments go together: showing a session without saying which transport
    /// drives it would let the user assume the admission guarantee. The transport is read
    /// from the backend's real capabilities:
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

    /// Replaces the prompt composition pipeline.
    ///
    /// Used by the experimental round, which substitutes a notice variant for
    /// [`StaleReadNotice`] in order to measure it. Once the variant is settled, this
    /// extension point will have no reason to exist.
    #[must_use]
    pub fn with_pipeline(mut self, pipeline: PromptPipeline) -> Self {
        self.pipeline = pipeline;
        self
    }

    /// Makes the session known to the registry, so its name appears in notices.
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

    /// What has been observed so far.
    #[must_use]
    pub fn activity(&self) -> &SessionActivity {
        &self.activity
    }

    /// The identifier of the session being driven.
    #[must_use]
    pub fn session_id(&self) -> SessionId {
        self.session.id
    }

    /// ★ Handles an agent event. **The heart of the wiring.**
    pub async fn handle(&mut self, event: AgentEvent) {
        match event {
            // A read: the client serves the content, and the registry records it. That is
            // what fills the read-set, and therefore what makes `StaleRead` possible.
            AgentEvent::FileRead(request) => {
                let absolute = self.absolutize(&request.path);
                match tokio::fs::read_to_string(&absolute).await {
                    Ok(content) => {
                        // A read we serve is a full read: it is the only substantial form
                        // (see `ReadKind`).
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
                        // A missing file is a normal case: the agent is exploring.
                        tracing::debug!(path = %absolute.display(), %error, "cannot read");
                        request.fail(error.to_string());
                    }
                }
            }

            // ★ A write. The registry admits AND writes, then we acknowledge.
            AgentEvent::FileWrite(request) => {
                let path = request.path.clone();
                // The state moves to Writing *before* admission: admission is when the
                // agent is waiting, so that is the moment worth showing.
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

                        // The notice cannot be injected now — the agent is in the middle
                        // of a tool call. We keep it for the next message.
                        if verdict.needs_notice() {
                            self.pending_notice = Some(verdict);
                        }
                        // The file is on the disk: we can acknowledge.
                        request.admitted();

                        // ★ The shadow-mode counter, if it moved. It does NOT come from the
                        // verdict: no notice was injected, this is a measurement (ADR 0027).
                        if let Ok(stats) = self.registry.shadow_stats().await
                            && stats.potential_notices != self.potential_notices
                        {
                            self.potential_notices = stats.potential_notices;
                            self.observe(Observation::PotentialNotices {
                                total: stats.potential_notices,
                            });
                        }
                        self.observe_state(SessionState::Thinking);
                    }
                    Err(error) => {
                        // An explicit denial, with its reason. The agent knows how to
                        // handle a failed tool; leaving it waiting would be worse.
                        let reason = error.to_string();
                        tracing::warn!(path = %path.display(), %reason, "write refused");
                        self.activity.refusals.push((path.clone(), reason.clone()));
                        let session = self.session.id;
                        self.observe(Observation::Refused {
                            session,
                            path,
                            reason: reason.clone(),
                        });
                        request.refuse(reason);
                        self.observe_state(SessionState::Thinking);
                    }
                }
            }

            AgentEvent::PermissionRequest(request) => {
                // `allow_once` and never `allow_always`: a persistent option makes the
                // agent write a settings file into the working directory, outside
                // admission (ADR 0016).
                match request.allow_once().map(|option| option.id.clone()) {
                    Some(id) => request.choose(id),
                    None => {
                        tracing::warn!(
                            tool = %request.tool_name,
                            "no non-persistent permission option: cancelling"
                        );
                        request.cancel();
                    }
                }
            }

            AgentEvent::Message(text) => self.activity.message.push_str(&text),
            AgentEvent::ToolCall { .. } | AgentEvent::Done => {}
            AgentEvent::Error(error) => tracing::error!(%error, "harness error"),
            _ => {}
        }
    }

    /// The notice to place in front of the next message, if there is one.
    ///
    /// Consumes it: a notice is injected once. Repeating it every turn would be noise,
    /// and noise is what gets a feature switched off.
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

    /// Sends a message to the agent, **with the pending notice if there is one**.
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

    /// Consumes the stream until the end of the turn.
    ///
    /// **The wait condition is explicit**: `AgentEvent::Done` or `AgentEvent::Error`.
    /// `Done` arrives when the response to `session/prompt` comes back — it is not a
    /// notification, and waiting for anything else is a wait that never completes.
    pub async fn run_turn(&mut self, events: &mut AgentEventStream) -> TurnOutcome {
        tracing::info!(
            session = %self.session.name,
            "turn starting — waiting for Done (the session/prompt response) or Error"
        );
        self.observe_state(SessionState::Thinking);
        while let Some(event) = events.next().await {
            let ending = match &event {
                AgentEvent::Done => Some(TurnOutcome::Done),
                AgentEvent::Error(message) => Some(TurnOutcome::Failed(message.clone())),
                _ => None,
            };
            self.handle(event).await;
            if let Some(outcome) = ending {
                tracing::info!(
                    session = %self.session.name,
                    ?outcome,
                    reads = self.activity.reads.len(),
                    writes = self.activity.writes.len(),
                    "turn finished — condition met"
                );
                self.observe_state(match &outcome {
                    TurnOutcome::Done => SessionState::Idle,
                    TurnOutcome::Failed(reason) => SessionState::Failed(reason.clone()),
                    _ => SessionState::Idle,
                });
                return outcome;
            }
        }
        tracing::warn!(session = %self.session.name, "stream closed before the turn ended");
        self.observe_state(SessionState::Failed("stream closed".to_owned()));
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
