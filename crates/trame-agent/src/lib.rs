//! Transport agent. Abstraction sur les harness.
//!
//! Le reste du core ne sait jamais s'il parle a de l'ACP ou a un PTY.
//!
//! # ACP en premier, PTY en secours
//!
//! - `AcpBackend` — JSON-RPC sur stdio. **Le chemin qui compte** : on voit les
//!   ecritures avant qu'elles touchent le disque, donc on peut les soumettre au
//!   registre. C'est la piece porteuse de tout l'edifice.
//! - `PtyBackend` — pilotage de CLI via `portable-pty`. Mode degrade : detection
//!   *a posteriori* par FSEvents, pas d'admission. **L'UI doit afficher la
//!   degradation** plutot que de laisser croire a une garantie qu'on n'a pas.
//!
//! Une seule cible en v0.1 : Claude Code, en ACP.
//!
//! # Interface visee (phase 2)
//!
//! ```rust,ignore
//! pub trait AgentBackend {
//!     fn capabilities(&self) -> Capabilities;
//!     async fn send(&mut self, msg: UserMessage) -> Result<()>;
//!     fn events(&mut self) -> impl Stream<Item = AgentEvent>;
//! }
//!
//! pub struct Capabilities {
//!     pub can_intercept_writes: bool,   // ACP: true, PTY: false
//!     pub can_inject_context: bool,
//!     pub can_request_permission: bool,
//! }
//!
//! pub enum AgentEvent {
//!     Message(String),
//!     ToolCall { name: String, input: Value },
//!     FileRead { path: PathBuf },
//!     FileWrite { path: PathBuf, content: String },  // <- passe par le registre
//!     PermissionRequest(PermissionRequest),
//!     Done,
//!     Error(String),
//! }
//! ```
//!
//! # Risque connu
//!
//! ACP est incomplet et inegal selon les harness — `AskUserQuestion` est
//! indisponible en plan mode, par exemple. Le repli PTY n'est pas optionnel.
//!
//! Ce crate est vide en phase 0.
