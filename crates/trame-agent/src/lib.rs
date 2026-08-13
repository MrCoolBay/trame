//! Agent transport. An abstraction over the harnesses.
//!
//! The rest of the core never knows whether it is talking to ACP or to a PTY.
//!
//! # The inversion that makes the product possible
//!
//! In ACP, **Trame is the client and the agent is the server**. The agent does not write and
//! then tell us: the agent *asks* Trame to write. The interception point is not a hook to
//! install, it is the protocol's normal path.
//!
//! Empirical validation and named holes:
//! [ADR 0016](../../../docs/adr/0016-interception-avant-disque-validee.md).
//!
//! # The order is non-negotiable
//!
//! ```text
//! incoming write request
//!   -> AgentEvent::FileWrite(request)
//!   -> registry: admission + write           ★ BEFORE the agent believes it has written
//!   -> request.admitted()
//! ```
//!
//! Swapping the two middle steps produces code that compiles, passes the tests, and removes
//! the product's reason to exist. It is the most important bug not to write in this
//! repository — and the type makes it hard: a dropped [`FileWriteRequest`] **refuses** the
//! write instead of silently allowing it.
//!
//! # Two backends
//!
//! - [`AcpBackend`] — JSON-RPC over stdio. The path that matters. One target in v0.1: Claude
//!   Code.
//! - [`PtyBackend`] — a `todo!()` skeleton, with honest capabilities. The fallback is not
//!   optional, but it is not v0.1's priority.
//!
//! # Testable with no agent
//!
//! [`AcpBackend::connect`] accepts any async reader/writer pair. The tests script the agent in
//! memory: no subprocess, no network, no authentication, and a deterministic result.

mod acp;
mod backend;
mod error;
mod event;
mod jsonrpc;
mod pty;

pub use acp::{AcpBackend, CLAUDE_CODE_ACP_COMMAND, PROTOCOL_VERSION};
pub use backend::{AgentBackend, AgentEventStream, Capabilities, UserMessage};
pub use error::AgentError;
pub use event::{
    AgentEvent, FileReadRequest, FileWriteRequest, PermissionOption, PermissionRequest,
};
pub use pty::PtyBackend;

/// The events module, for cross-referencing in the documentation.
pub mod events {
    pub use crate::event::*;
}
