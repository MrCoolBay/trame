//! Agent transport errors.

use thiserror::Error;

/// What can fail on the transport side.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum AgentError {
    /// The subprocess could not be started. Common case: the ACP adapter is not installed.
    #[error("cannot start the harness `{command}`")]
    Spawn {
        /// The command attempted.
        command: String,
        /// The cause.
        #[source]
        source: std::io::Error,
    },

    /// I/O on the subprocess's pipes.
    #[error("input/output error with the harness")]
    Io(#[from] std::io::Error),

    /// The JSON-RPC frame is unreadable.
    #[error("invalid JSON-RPC frame")]
    Protocol(#[from] serde_json::Error),

    /// The agent answered with a JSON-RPC error.
    #[error("the harness answered with an error ({code}): {message}")]
    Rpc {
        /// The JSON-RPC code.
        code: i64,
        /// The message.
        message: String,
    },

    /// The agent answered something unexpected where a field was required.
    #[error("unexpected response from the harness on {method}: {detail}")]
    Unexpected {
        /// The method called.
        method: &'static str,
        /// What was missing or did not fit.
        detail: String,
    },

    /// The subprocess died, or the transport task stopped.
    #[error("the harness is no longer reachable")]
    Gone,

    /// The agent asks for authentication Trame cannot supply.
    ///
    /// To be surfaced verbatim to the user: it is **their** account, and the agent's message
    /// contains the steps to follow.
    #[error("authentication required by the harness: {0}")]
    AuthRequired(String),

    /// The backend cannot do this. Typical case: a `PtyBackend` asked to intercept a write.
    /// **To be displayed**, not swallowed.
    #[error("not available on this backend: {0}")]
    Unsupported(&'static str),
}
