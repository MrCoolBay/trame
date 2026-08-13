//! `AcpBackend` — Agent Client Protocol, JSON-RPC sur stdio.
//!
//! # L'inversion qui rend le produit possible
//!
//! En ACP, **Trame est le client et l'agent est le serveur**. Ce n'est pas l'agent qui
//! ecrit puis nous previent : c'est l'agent qui *demande* a Trame d'ecrire, par
//! `fs/write_text_file`. Le point d'interception n'est donc pas un hook a installer,
//! c'est le path normal du protocole.
//!
//! Mieux : quand le client annonce `fs.writeTextFile`, l'adaptateur Claude Code
//! **desactive les outils `Write` et `Edit` natifs** de l'agent. Ce dernier ne peut plus
//! ecrire lui-meme, il n'a plus que le path qui passe par nous. Voir
//! `docs/adr/0016-interception-avant-disque-validee.md` pour la validation detaillee et
//! les trous nommes.
//!
//! # Transport injectable
//!
//! [`AcpBackend::connect`] prend n'importe quel couple lecteur/ecrivain asynchrone.
//! `spawn` n'est qu'un cas particulier ou ils viennent d'un sous-process. C'est ce qui
//! rend tout ce module testable sans agent, sans reseau et sans authentification : les
//! tests scenarisent l'agent en memoire, de facon deterministe.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::backend::{AgentBackend, AgentEventStream, Capabilities, UserMessage};
use crate::error::AgentError;
use crate::event::{
    AgentEvent, FileReadRequest, FileWriteRequest, PermissionOption, PermissionRequest,
};
use crate::jsonrpc::{ErrorResponse, Incoming, Request, Response};

/// Version du protocole que ce client parle.
pub const PROTOCOL_VERSION: u32 = 1;

/// La commande par defaut de l'adaptateur ACP de Claude Code.
///
/// Traite comme une **dependance externe installee par l'utilisateur**, exactement comme
/// `but` : Trame ne l'embarque pas.
pub const CLAUDE_CODE_ACP_COMMAND: &str = "claude-code-acp";

/// Capacite de la file d'evenements. Bornee, jamais `unbounded_channel` (ADR 0015).
const EVENT_CAPACITY: usize = 64;

/// Ce que le client demande a l'agent, en interne.
struct Outgoing {
    method: &'static str,
    params: Value,
    reply: oneshot::Sender<Result<Value, AgentError>>,
}

/// Le backend ACP.
pub struct AcpBackend {
    tx: mpsc::Sender<Outgoing>,
    /// Emetteur du feed, conserve pour pouvoir signaler la fin de turn.
    ///
    /// La fin de turn n'est **pas** une notification : c'est la reponse a
    /// `session/prompt`. Elle arrive donc par le canal des reponses, pas par celui des
    /// notifications, et c'est ici qu'on la retraduit en [`AgentEvent::Done`].
    event_tx: mpsc::Sender<AgentEvent>,
    events: Option<AgentEventStream>,
    session_id: Option<String>,
    /// Outils a fermer **en plus** de ceux que l'adaptateur retire deja.
    extra_disallowed: Vec<String>,
    cwd: PathBuf,
    child: Option<Child>,
    _pump: JoinHandle<()>,
}

impl std::fmt::Debug for AcpBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AcpBackend")
            .field("session_id", &self.session_id)
            .field("cwd", &self.cwd)
            .finish_non_exhaustive()
    }
}

impl AcpBackend {
    /// Lance l'adaptateur ACP en sous-process et se connecte a ses tubes.
    ///
    /// `cwd` est la root du projet — le repertoire de travail **unique et partage**.
    pub async fn spawn(command: &str, cwd: impl Into<PathBuf>) -> Result<Self, AgentError> {
        let cwd = cwd.into();
        let mut child = Command::new(command)
            .current_dir(&cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // stderr du sous-process reste separe : stdout appartient au protocole.
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|source| AgentError::Spawn { command: command.to_owned(), source })?;

        let stdin = child.stdin.take().ok_or(AgentError::Gone)?;
        let stdout = child.stdout.take().ok_or(AgentError::Gone)?;
        if let Some(stderr) = child.stderr.take() {
            // Les diagnostics du harness sont utiles, mais ils ne doivent jamais etre
            // confondus avec la trame.
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if !line.trim().is_empty() {
                        tracing::debug!(target: "trame::acp::stderr", "{line}");
                    }
                }
            });
        }

        let mut backend = Self::connect(stdout, stdin, cwd).await?;
        backend.child = Some(child);
        Ok(backend)
    }

    /// Lance l'adaptateur Claude Code avec la commande par defaut.
    pub async fn spawn_claude_code(cwd: impl Into<PathBuf>) -> Result<Self, AgentError> {
        Self::spawn(CLAUDE_CODE_ACP_COMMAND, cwd).await
    }

    /// Se connecte a un agent deja accessible par un couple lecteur/ecrivain.
    ///
    /// C'est le constructeur qui rend ce module testable : un test fournit un
    /// `tokio::io::duplex` et scenarise l'agent en memoire.
    pub async fn connect<R, W>(
        reader: R,
        writer: W,
        cwd: impl Into<PathBuf>,
    ) -> Result<Self, AgentError>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let (out_tx, out_rx) = mpsc::channel(EVENT_CAPACITY);
        let (event_tx, event_rx) = mpsc::channel(EVENT_CAPACITY);
        let event_tx_pour_backend = event_tx.clone();
        let pump = tokio::spawn(pump(reader, writer, out_rx, event_tx));

        let mut backend = Self {
            tx: out_tx,
            event_tx: event_tx_pour_backend,
            events: Some(AgentEventStream::new(event_rx)),
            session_id: None,
            extra_disallowed: Vec::new(),
            cwd: cwd.into(),
            child: None,
            _pump: pump,
        };
        backend.initialize().await?;
        Ok(backend)
    }

    async fn call(&self, method: &'static str, params: Value) -> Result<Value, AgentError> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(Outgoing {
                method,
                params,
                reply,
            })
            .await
            .map_err(|_| AgentError::Gone)?;
        rx.await.map_err(|_| AgentError::Gone)?
    }

    /// La negociation. **C'est ici que l'interception se decide.**
    ///
    /// En annoncant `fs.writeTextFile`, on obtient que l'agent ne puisse plus ecrire
    /// directement : ses outils d'ecriture natifs sont desactives au profit d'un path
    /// qui passe par nous.
    async fn initialize(&mut self) -> Result<(), AgentError> {
        let result = self
            .call(
                "initialize",
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "clientCapabilities": {
                        "fs": { "readTextFile": true, "writeTextFile": true },
                        // On n'annonce PAS `terminal`. Consequence a connaitre : les
                        // outils shell de l'agent restent natifs, donc ses ecritures via
                        // shell echappent a l'admission. C'est le trou documente dans
                        // l'ADR 0016, assume en v0.1.
                        "terminal": false
                    }
                }),
            )
            .await?;

        let version = result.get("protocolVersion").and_then(Value::as_u64);
        if version != Some(u64::from(PROTOCOL_VERSION)) {
            return Err(AgentError::Unexpected {
                method: "initialize",
                detail: format!("version de protocole {version:?}, attendue {PROTOCOL_VERSION}"),
            });
        }
        // Calcule hors du macro : dans une position de champ `tracing`, l'identifiant
        // `Value` resout vers `tracing::field::Value` et non vers `serde_json::Value`.
        let agent_name = result
            .get("agentInfo")
            .and_then(|info| info.get("name"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("inconnu");
        tracing::info!(
            agent = agent_name,
            "harness initialise, interception annoncee"
        );
        Ok(())
    }

    /// Ouvre une session sur le repertoire de travail du projet.
    ///
    /// `disallowed` ferme les outils qui ecriraient hors du path d'admission. Voir
    /// [`AcpBackend::DISALLOWED_WRITE_TOOLS`].
    pub async fn new_session(&mut self) -> Result<String, AgentError> {
        let result = self
            .call(
                "session/new",
                json!({
                    "cwd": self.cwd,
                    "mcpServers": [],
                    // Fusionne avec la liste que l'adaptateur construit lui-meme, il ne
                    // l'ecrase pas : c'est ce qui permet de fermer les trous restants.
                    "_meta": {
                        "claudeCode": {
                            "options": { "disallowedTools": self.disallowed_tools() }
                        }
                    }
                }),
            )
            .await
            .map_err(|error| match error {
                // L'agent refuse faute d'authentification : c'est le compte de
                // l'utilisateur, et le message de l'agent porte la marche a suivre.
                AgentError::Rpc { code, message }
                    if code == -32000 || message.contains("uthenticat") =>
                {
                    AgentError::AuthRequired(message)
                }
                other => other,
            })?;

        let id = result
            .get("sessionId")
            .and_then(Value::as_str)
            .ok_or_else(|| AgentError::Unexpected {
                method: "session/new",
                detail: "sessionId absent".to_owned(),
            })?
            .to_owned();
        self.session_id = Some(id.clone());
        tracing::info!(session = %id, "session ACP opened");
        Ok(id)
    }

    /// Les outils a desactiver **en plus** de ceux que l'adaptateur desactive deja.
    ///
    /// L'adaptateur retire `Write` et `Edit` des qu'on annonce `fs.writeTextFile`, mais
    /// pas `NotebookEdit`, qui ecrirait un `.ipynb` directement sur le disque. On le
    /// ferme explicitement plutot que de laisser un trou silencieux.
    pub const DISALLOWED_WRITE_TOOLS: &'static [&'static str] = &["NotebookEdit"];

    /// Ferme des outils **en plus** de ceux que l'adaptateur retire deja.
    ///
    /// A appeler avant [`AcpBackend::new_session`] : la liste est fusionnee par
    /// l'adaptateur, pas ecrasee.
    ///
    /// # Pourquoi c'est necessaire
    ///
    /// Retirer `Read` ne force pas l'agent a passer par nous : `Grep`, `Glob` et `Bash`
    /// restent disponibles, et un agent qui lit par l'un d'eux **n'entre pas dans le
    /// read-set**. Sans entree de read-set, aucun `StaleRead` n'est possible — le
    /// mecanisme du produit ne se declenche jamais, et rien ne le signale.
    ///
    /// Voir l'ADR 0016 : c'est le pendant, cote lecture, du trou `Bash` cote ecriture.
    pub fn disallow_tools<I, S>(&mut self, tools: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.extra_disallowed
            .extend(tools.into_iter().map(Into::into));
    }

    /// La liste complete des outils que nous demandons de fermer.
    fn disallowed_tools(&self) -> Vec<String> {
        Self::DISALLOWED_WRITE_TOOLS
            .iter()
            .map(|tool| (*tool).to_owned())
            .chain(self.extra_disallowed.iter().cloned())
            .collect()
    }

    /// L'identifiant de session ACP, s'il y en a une d'opened.
    ///
    /// Distinct du `SessionId` de Trame : on guard la correspondance, on ne reutilise
    /// jamais l'un pour l'autre.
    #[must_use]
    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }
}

#[async_trait]
impl AgentBackend for AcpBackend {
    fn capabilities(&self) -> Capabilities {
        Capabilities::acp()
    }

    async fn send(&mut self, msg: UserMessage) -> Result<(), AgentError> {
        let session = self.session_id.clone().ok_or(AgentError::Unexpected {
            method: "session/prompt",
            detail: "aucune session ouverte".to_owned(),
        })?;
        let params = json!({
            "sessionId": session,
            "prompt": [{ "type": "text", "text": msg.rendered() }]
        });

        // On n'attend PAS la fin du turn : un agent peut reflechir plusieurs minutes, et
        // bloquer ici empecherait de handle ses requetes d'ecriture — donc de l'admettre.
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(Outgoing {
                method: "session/prompt",
                params,
                reply,
            })
            .await
            .map_err(|_| AgentError::Gone)?;

        // La reponse a `session/prompt` **est** la fin de turn. L'adaptateur n'emet
        // aucune notification pour ca : wait_for un `sessionUpdate` de fin est une
        // attente qui n'aboutit jamais. On retraduit donc la reponse en `Done`, pour que
        // le consommateur du feed ait un signal de fin.
        let event_tx = self.event_tx.clone();
        tokio::spawn(async move {
            let evenement = match rx.await {
                Ok(Ok(resultat)) => {
                    let raison = resultat
                        .get("stopReason")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("inconnue")
                        .to_owned();
                    tracing::info!(raison = %raison, "fin de turn");
                    AgentEvent::Done
                }
                Ok(Err(error)) => {
                    tracing::error!(%error, "turn en echec");
                    AgentEvent::Error(error.to_string())
                }
                Err(_) => AgentEvent::Error("turn abandonne".to_owned()),
            };
            let _ = event_tx.send(evenement).await;
        });
        Ok(())
    }

    fn events(&mut self) -> Option<AgentEventStream> {
        self.events.take()
    }

    async fn shutdown(&mut self) -> Result<(), AgentError> {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill().await;
        }
        Ok(())
    }
}

/// La run_loop de transport : lit les lines entrantes, ecrit les sortantes, et traduit.
async fn pump<R, W>(
    reader: R,
    mut writer: W,
    mut outgoing: mpsc::Receiver<Outgoing>,
    events: mpsc::Sender<AgentEvent>,
) where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let mut lines = BufReader::new(reader).lines();
    let mut pending: HashMap<u64, oneshot::Sender<Result<Value, AgentError>>> = HashMap::new();
    let mut next_id: u64 = 1;
    // Les reponses aux requetes de l'agent partent par ce canal : le traitement d'une
    // requete est asynchrone (il attend une decision d'admission), donc il ne peut pas
    // ecrire directement sur le writer que cette run_loop possede.
    let (reply_tx, mut reply_rx) = mpsc::channel::<String>(EVENT_CAPACITY);

    loop {
        tokio::select! {
            biased;

            // Reponses differees aux requetes de l'agent : priorite, l'agent attend.
            Some(line) = reply_rx.recv() => {
                if write_line(&mut writer, &line).await.is_err() { break; }
            }

            // Requetes sortantes.
            Some(Outgoing { method, params, reply }) = outgoing.recv() => {
                let id = next_id;
                next_id += 1;
                let payload = match serde_json::to_string(&Request::new(id, method, params)) {
                    Ok(payload) => payload,
                    Err(error) => {
                        let _ = reply.send(Err(error.into()));
                        continue;
                    }
                };
                pending.insert(id, reply);
                if write_line(&mut writer, &payload).await.is_err() {
                    break;
                }
            },

            // Trame entrante.
            line = lines.next_line() => {
                let line = match line {
                    Ok(Some(line)) => line,
                    Ok(None) => break,                 // stdout ferme : le harness est parti
                    Err(error) => {
                        let _ = events.send(AgentEvent::Error(error.to_string())).await;
                        break;
                    }
                };
                if line.trim().is_empty() { continue; }

                let msg: Incoming = match serde_json::from_str(&line) {
                    Ok(msg) => msg,
                    Err(error) => {
                        tracing::warn!(%error, "trame illisible, ignoree");
                        continue;
                    }
                };
                handle_incoming(msg, &mut pending, &events, &reply_tx).await;
            }

            else => break,
        }
    }

    // Le harness est parti : on l'annonce plutot que de laisser les appelants wait_for.
    for (_, reply) in pending.drain() {
        let _ = reply.send(Err(AgentError::Gone));
    }
    let _ = events
        .send(AgentEvent::Error("le harness s'est arrete".to_owned()))
        .await;
    tracing::info!("transport ACP arrete");
}

async fn write_line<W: AsyncWrite + Unpin>(writer: &mut W, line: &str) -> std::io::Result<()> {
    writer.write_all(line.as_bytes()).await?;
    writer.write_all(b"\n").await?;
    writer.flush().await
}

async fn handle_incoming(
    msg: Incoming,
    pending: &mut HashMap<u64, oneshot::Sender<Result<Value, AgentError>>>,
    events: &mpsc::Sender<AgentEvent>,
    reply_tx: &mpsc::Sender<String>,
) {
    // 1. Une reponse a l'une de nos requetes.
    if msg.as_response().is_some() {
        let Some(id) = msg.id.as_ref().and_then(Value::as_u64) else {
            return;
        };
        if let Some(reply) = pending.remove(&id) {
            let outcome = match (msg.result, msg.error) {
                (_, Some(error)) => Err(AgentError::Rpc {
                    code: error.code,
                    message: error.message,
                }),
                (Some(result), None) => Ok(result),
                (None, None) => Ok(Value::Null),
            };
            let _ = reply.send(outcome);
        }
        return;
    }

    // 2. Une requete de l'agent vers nous. ★ Le point d'interception.
    if let Some((id, method)) = msg.as_request().map(|(id, m)| (id.clone(), m.to_owned())) {
        let params = msg.params.unwrap_or(Value::Null);
        dispatch_request(id, &method, params, events, reply_tx).await;
        return;
    }

    // 3. Une notification.
    if let Some(method) = msg.as_notification()
        && method == "session/update"
        && let Some(update) = msg.params.as_ref().and_then(|params| params.get("update"))
    {
        emit_update(update, events).await;
    }
}

/// Traduit une requete entrante en evenement porteur de son canal de reponse.
///
/// La reponse a l'agent est **differee** : elle part quand le consommateur a decide. Le
/// sous-process attend pendant ce temps, ce qui est exactement le comportement voulu.
async fn dispatch_request(
    id: Value,
    method: &str,
    params: Value,
    events: &mpsc::Sender<AgentEvent>,
    reply_tx: &mpsc::Sender<String>,
) {
    match method {
        "fs/write_text_file" => {
            let path = params
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let content = params
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let (request, rx) = FileWriteRequest::new(PathBuf::from(path), content.to_owned());
            forward(
                events,
                AgentEvent::FileWrite(request),
                reply_tx,
                id,
                rx,
                |()| Value::Null,
            )
            .await;
        }
        "fs/read_text_file" => {
            let path = params
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let line = params.get("line").and_then(Value::as_u64).map(|v| v as u32);
            let limit = params
                .get("limit")
                .and_then(Value::as_u64)
                .map(|v| v as u32);
            let (request, rx) = FileReadRequest::new(PathBuf::from(path), line, limit);
            forward(
                events,
                AgentEvent::FileRead(request),
                reply_tx,
                id,
                rx,
                |content: String| json!({ "content": content }),
            )
            .await;
        }
        "session/request_permission" => {
            let title = params
                .get("toolCall")
                .and_then(|c| c.get("title"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let tool_name = params
                .get("toolCall")
                .and_then(|c| c.get("toolName"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let options = params
                .get("options")
                .and_then(Value::as_array)
                .map(|options| {
                    options
                        .iter()
                        .map(|option| PermissionOption {
                            id: option
                                .get("optionId")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_owned(),
                            label: option
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_owned(),
                            kind: option
                                .get("kind")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_owned(),
                        })
                        .collect()
                })
                .unwrap_or_default();

            let (request, rx) = PermissionRequest::new(title, tool_name, options);
            let reply_tx = reply_tx.clone();
            if events
                .send(AgentEvent::PermissionRequest(request))
                .await
                .is_err()
            {
                return;
            }
            tokio::spawn(async move {
                let line = match rx.await {
                    Ok(Some(option_id)) => serde_json::to_string(&Response::new(
                        id,
                        json!({ "outcome": { "outcome": "selected", "optionId": option_id } }),
                    )),
                    _ => serde_json::to_string(&Response::new(
                        id,
                        json!({ "outcome": { "outcome": "cancelled" } }),
                    )),
                };
                if let Ok(line) = line {
                    let _ = reply_tx.send(line).await;
                }
            });
        }
        other => {
            tracing::debug!(
                method = other,
                "requete entrante non geree, refusee proprement"
            );
            if let Ok(line) =
                serde_json::to_string(&ErrorResponse::internal(id, format!("{other} non gere")))
            {
                let _ = reply_tx.send(line).await;
            }
        }
    }
}

/// Emet l'evenement, puis attend la decision du consommateur et repond a l'agent.
async fn forward<T, F>(
    events: &mpsc::Sender<AgentEvent>,
    event: AgentEvent,
    reply_tx: &mpsc::Sender<String>,
    id: Value,
    rx: oneshot::Receiver<Result<T, String>>,
    to_result: F,
) where
    T: Send + 'static,
    F: FnOnce(T) -> Value + Send + 'static,
{
    if events.send(event).await.is_err() {
        return;
    }
    let reply_tx = reply_tx.clone();
    tokio::spawn(async move {
        let line = match rx.await {
            Ok(Ok(value)) => serde_json::to_string(&Response::new(id, to_result(value))),
            Ok(Err(reason)) => serde_json::to_string(&ErrorResponse::internal(id, reason)),
            // Le consommateur a disparu sans decider : refus, jamais silence.
            Err(_) => {
                serde_json::to_string(&ErrorResponse::internal(id, "aucune decision d'admission"))
            }
        };
        if let Ok(line) = line {
            let _ = reply_tx.send(line).await;
        }
    });
}

/// Traduit un `session/update` en evenement normalise.
async fn emit_update(update: &Value, events: &mpsc::Sender<AgentEvent>) {
    let kind = update
        .get("sessionUpdate")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match kind {
        "agent_message_chunk" => {
            if let Some(text) = update
                .get("content")
                .and_then(|c| c.get("text"))
                .and_then(Value::as_str)
            {
                let _ = events.send(AgentEvent::Message(text.to_owned())).await;
            }
        }
        // `tool_call` annonce un appel, `tool_call_update` le raffine. L'adaptateur
        // n'emet parfois QUE des `tool_call_update` : quand une demande de permission a
        // deja fait emettre le `tool_call`, le feed qui suit ne fait que le mettre a
        // jour. Ne traduire que la forme initiale laisserait donc des appels invisibles.
        "tool_call" | "tool_call_update" => {
            let name = update
                .get("title")
                .and_then(Value::as_str)
                .or_else(|| update.get("toolCallId").and_then(Value::as_str))
                .unwrap_or("outil")
                .to_owned();
            let input = update
                .get("rawInput")
                .or_else(|| update.get("content"))
                .cloned()
                .unwrap_or(Value::Null);
            let _ = events.send(AgentEvent::ToolCall { name, input }).await;
        }
        // Il n'existe pas de `sessionUpdate` de fin de turn : elle arrive comme reponse a
        // `session/prompt`, et c'est `AcpBackend::send` qui la retraduit en `Done`.
        "agent_thought_chunk" | "available_commands_update" | "current_mode_update" | "plan" => {
            tracing::debug!(
                update = kind,
                "notification non pertinente pour le registre"
            );
        }
        other => tracing::debug!(update = other, "session/update non traduit"),
    }
}
