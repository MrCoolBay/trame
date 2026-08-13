//! The bridge between a CLI hook and the daemon. **It decides nothing.**
//!
//! The CLI runs this binary on every agent tool call, passes it the hook payload on `stdin`,
//! and reads its decision from `stdout`. All the policy lives in the registry, on the daemon
//! side: a hook that decided would be a second copy of the admission rules, and two copies
//! diverge (ADR 0025).
//!
//! # ★ The rule that governs this crate
//!
//! > **If the daemon cannot be reached, we REFUSE, and we say so.**
//!
//! The failure mode being guarded against is precise. The daemon is not listening — never
//! started, crashed, stale socket. If this binary exits 0 saying nothing, the CLI reads "no
//! objection" and the write goes through. The invariant is dead and the agent works normally:
//! **no symptom at all.**
//!
//! It is the same reasoning as `FileWriteRequest`'s `Drop`, which refuses by default
//! (ADR 0016): on the admission path, the absence of an answer is never a yes.
//!
//! The accepted consequence: with the daemon absent, the agent is blocked on shell writes.
//! That is loud, and therefore fixable. The opposite is silent, and therefore not.

pub mod bash;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The delay past which the daemon is considered unreachable.
///
/// Short on purpose: the agent is waiting. A daemon that is alive but stuck is treated as an
/// absent one — **refusal**. Better to block a shell write than to suspend a session.
pub const TIMEOUT: Duration = Duration::from_millis(2_000);

/// What the hook returns to the CLI.
///
/// The format is `hookSpecificOutput`, observed in probe 2 — not inferred from a type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Nothing to say: the CLI carries on. That is ~95% of traffic.
    Silence,
    /// Denied, with the reason passed to the agent. Probe 2 measured that it reads and quotes
    /// it.
    Deny(String),
}

impl Decision {
    /// The JSON the CLI expects, or nothing when there is nothing to say.
    ///
    /// A hook that writes nothing to `stdout` lets the call through: exactly what we want for
    /// [`Decision::Silence`], and exactly what we **never** want on an error.
    #[must_use]
    pub fn to_json(&self) -> Option<String> {
        match self {
            Self::Silence => None,
            Self::Deny(reason) => Some(
                serde_json::json!({
                    "hookSpecificOutput": {
                        "hookEventName": "PreToolUse",
                        "permissionDecision": "deny",
                        "permissionDecisionReason": reason,
                    }
                })
                .to_string(),
            ),
        }
    }
}

/// Why the hook could not consult the policy.
///
/// **Every variant leads to a refusal**, never to a free pass. The type exists so the reason
/// shown is precise: "daemon absent" and "unreadable response" are not fixed the same way.
#[derive(Debug, thiserror::Error)]
pub enum HookError {
    /// The socket does not exist: the daemon never started for this project.
    #[error("no Trame daemon is listening on {path} — is the project open in Trame?")]
    SocketMissing {
        /// The expected path.
        path: PathBuf,
    },
    /// The socket exists but nobody is at the other end: crashed daemon, stale socket.
    #[error("the Trame daemon did not answer on {path} ({source}) — stale socket?")]
    Unreachable {
        /// The path attempted.
        path: PathBuf,
        /// The system cause.
        source: std::io::Error,
    },
    /// The daemon answered something we do not understand.
    #[error("unreadable response from the daemon: {0}")]
    UnreadableResponse(String),
    /// The CLI's payload is not JSON.
    #[error("unreadable hook payload: {0}")]
    UnreadablePayload(String),
}

impl HookError {
    /// The refusal reason passed to the agent.
    ///
    /// It names the cause and the action, because the agent relays it to the user: a refusal
    /// that only says "refused" sends someone searching for ten minutes.
    #[must_use]
    pub fn reason(&self) -> String {
        format!(
            "Trame could not check this action, so it is refused. {self} \
             (Trame denies by default: an unverified action is not an authorised action.)"
        )
    }
}

/// Asks the daemon for a verdict.
///
/// # Errors
///
/// Every error must be translated into a **refusal** by the caller. See [`HookError`].
pub fn ask(socket: &Path, payload: &str) -> Result<Decision, HookError> {
    // Existence is checked before connecting, to tell "never started" from "crashed" — two
    // different repairs.
    if !socket.exists() {
        return Err(HookError::SocketMissing {
            path: socket.to_path_buf(),
        });
    }
    // An unreadable payload is a bug on our side or a break in the CLI. Either way we do not
    // guess: we refuse.
    serde_json::from_str::<serde_json::Value>(payload)
        .map_err(|error| HookError::UnreadablePayload(error.to_string()))?;

    let stream = UnixStream::connect(socket).map_err(|source| HookError::Unreachable {
        path: socket.to_path_buf(),
        source,
    })?;
    stream
        .set_read_timeout(Some(TIMEOUT))
        .and_then(|()| stream.set_write_timeout(Some(TIMEOUT)))
        .map_err(|source| HookError::Unreachable {
            path: socket.to_path_buf(),
            source,
        })?;

    let mut writer = &stream;
    // One JSON per line, like ACP. The `\n` is the delimiter, not a convenience.
    let line = payload.replace('\n', " ");
    writer
        .write_all(format!("{line}\n").as_bytes())
        .and_then(|()| writer.flush())
        .map_err(|source| HookError::Unreachable {
            path: socket.to_path_buf(),
            source,
        })?;

    let mut response = String::new();
    BufReader::new(&stream)
        .read_line(&mut response)
        .map_err(|source| HookError::Unreachable {
            path: socket.to_path_buf(),
            source,
        })?;
    read_verdict(&response)
}

/// Translates the daemon's response.
///
/// A deliberately poor format: `{"decision":"silence"}` or
/// `{"decision":"deny","reason":"…"}`. An empty response is an **unreadable** response, and
/// therefore a refusal — that is the case of a daemon that closes the connection without
/// answering.
fn read_verdict(response: &str) -> Result<Decision, HookError> {
    let raw = response.trim();
    if raw.is_empty() {
        return Err(HookError::UnreadableResponse(
            "the daemon closed without answering".to_owned(),
        ));
    }
    let parsed: serde_json::Value =
        serde_json::from_str(raw).map_err(|e| HookError::UnreadableResponse(e.to_string()))?;
    match parsed.get("decision").and_then(serde_json::Value::as_str) {
        Some("silence") => Ok(Decision::Silence),
        Some("deny") => Ok(Decision::Deny(
            parsed
                .get("reason")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("refused by Trame")
                .to_owned(),
        )),
        other => Err(HookError::UnreadableResponse(format!(
            "unknown decision: {other:?}"
        ))),
    }
}

/// A project's socket path.
///
/// In the data directory, **never inside the watched project** — which is precisely what we
/// are observing. One path per project: the registry is per project (invariant 3), and the
/// socket follows.
///
/// # Errors
///
/// Fails if `HOME` is absent.
pub fn socket_path(project: &str) -> Result<PathBuf, HookError> {
    let home = std::env::var_os("HOME").ok_or_else(|| {
        HookError::UnreadablePayload("HOME missing from the environment".to_owned())
    })?;
    Ok(PathBuf::from(home)
        .join("Library")
        .join("Application Support")
        .join("Trame")
        .join("sockets")
        .join(format!("{project}.sock")))
}
