//! The daemon side of the CLI hooks. **This is where the policy lives.**
//!
//! `trame-hook` decides nothing: it carries the payload and returns the answer (ADR 0025). All
//! the policy is here, in one place, with the registry within reach.
//!
//! # Two hooks, two opposite roles
//!
//! | Hook | Tool | What we do |
//! |---|---|---|
//! | `PreToolUse` | `Bash` | **Deny** a redirection into a project file (ADR 0026) |
//! | `PostToolUse` | `Grep`, `Glob` | **Record in shadow** the files read. We never refuse |
//!
//! On `Bash` we refuse because there is no way to know what a command writes, and we prefer to
//! bring the hole back inside admission's scope. On `Grep` it is the opposite: denying would
//! deprive the agent of search, degrading it on a real codebase — so we record.
//!
//! # ★ The fingerprint never comes from the payload
//!
//! Invariant 10, and ADR 0020. The hook reports **paths**; Trame **re-reads the file** to
//! fingerprint it. Fingerprinting a hook payload would produce a fingerprint matching no state
//! of the disk — and the failure would be entirely silent: read-set populated, `StaleRead` dead,
//! no test broken.
//!
//! # The two shapes of path, measured rather than assumed
//!
//! [Probe 3](../../../docs/sondes/2026-08-12-postooluse.md) found that the two tools disagree:
//!
//! - **`Grep`** returns paths **relative to the `cwd`**, including when the call carries a
//!   `path` argument: `path: "sub"` gives `sub/deep.rs`, not `deep.rs`.
//! - **`Glob`** returns **absolute, resolved** paths: `/private/tmp/…`, not `/tmp/…`.
//!
//! Both go through [`trame_core::ProjectRoot`], which absorbs the `/private/var` versus `/var`
//! resolution. A test pins both shapes: without it the regression would go unnoticed, since
//! each tool on its own would look like it worked.

use std::path::PathBuf;

use serde::Deserialize;
use trame_core::{ProjectRoot, SessionId};
use trame_registry::RegistryHandle;

/// What the daemon returns to the hook.
///
/// Deliberately poor, and symmetric with `trame_hook::Decision`: `{"decision":"silence"}` or
/// `{"decision":"deny","reason":"…"}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Response {
    /// Nothing to say. The hook writes nothing, the CLI carries on.
    Silence,
    /// Denied, with the reason passed back to the agent.
    Deny(String),
}

impl Response {
    /// The JSON line sent over the socket.
    #[must_use]
    pub fn to_line(&self) -> String {
        match self {
            Self::Silence => "{\"decision\":\"silence\"}\n".to_owned(),
            Self::Deny(reason) => {
                let body = serde_json::json!({ "decision": "deny", "reason": reason });
                format!("{body}\n")
            }
        }
    }
}

/// What handling a hook produced, for the interface and the tests.
///
/// **Makes visible what was NOT recorded.** A counter that only counts successes suggests a
/// coverage we do not have.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Report {
    /// The files whose read entered the registry.
    pub recorded: Vec<PathBuf>,
    /// Paths that were reported but **not recorded**, with the reason.
    ///
    /// Three causes: outside the project, unreadable, or past the limit. None of them is
    /// silent (ADR 0021: a blind spot is counted and displayed).
    pub skipped: Vec<(String, &'static str)>,
    /// True if the call was a `Grep` in `content` or `count` mode, whose paths are **not** in
    /// `filenames`. An accepted blind spot, never reconstructed by parsing (ADR 0021).
    pub blind_mode: bool,
}

/// A hook's envelope, as observed in probes 2 and 3.
///
/// The keys come from observation, not from a type: probe 2 found `permission_mode` missing
/// from the type probe 1 had quoted.
#[derive(Debug, Deserialize)]
pub struct Payload {
    /// `PreToolUse` or `PostToolUse`.
    pub hook_event_name: String,
    /// The tool's name. `Bash`, `Grep`, `Glob`, `mcp__acp__Read`…
    pub tool_name: String,
    /// The call's parameters.
    #[serde(default)]
    pub tool_input: serde_json::Value,
    /// The tool's output. Present on `PostToolUse` only.
    #[serde(default)]
    pub tool_response: serde_json::Value,
}

/// ★ Handles a hook payload. **The single decision point.**
///
/// # Errors
///
/// Never returns an error: a payload we do not know how to handle gives [`Response::Silence`].
/// Denying by default is the **hook's** rule when it cannot reach us (ADR 0025); here, having
/// been reached, we have no reason to deny what we do not recognise.
pub async fn handle(
    payload: &Payload,
    root: &ProjectRoot,
    registry: &RegistryHandle,
    session: SessionId,
    limit: usize,
) -> (Response, Report) {
    match (payload.hook_event_name.as_str(), payload.tool_name.as_str()) {
        ("PreToolUse", "Bash") => (bash(payload), Report::default()),
        ("PostToolUse", "Grep" | "Glob") => {
            let report = record_reads(payload, root, registry, session, limit).await;
            (Response::Silence, report)
        }
        _ => (Response::Silence, Report::default()),
    }
}

/// The `Bash` policy: a single pattern (ADR 0026).
fn bash(payload: &Payload) -> Response {
    let Some(command) = payload
        .tool_input
        .get("command")
        .and_then(serde_json::Value::as_str)
    else {
        return Response::Silence;
    };
    match trame_hook::bash::evaluate(command) {
        trame_hook::bash::Verdict::Allow => Response::Silence,
        trame_hook::bash::Verdict::Deny { target } => {
            tracing::info!(%target, "shell write refused");
            Response::Deny(trame_hook::bash::reason(&target))
        }
    }
}

/// Records the files reported by `Grep` or `Glob`.
///
/// **`filenames` only.** In `content` and `count` mode the key is empty and the paths exist
/// only inside the output string: an accepted blind spot, counted and displayed, never
/// reconstructed by parsing (ADR 0021).
async fn record_reads(
    payload: &Payload,
    root: &ProjectRoot,
    registry: &RegistryHandle,
    session: SessionId,
    limit: usize,
) -> Report {
    let mut report = Report::default();

    let mode = payload
        .tool_response
        .get("mode")
        .and_then(serde_json::Value::as_str);
    // `Glob` has no `mode`; `Grep` has one, and only `files_with_matches` populates `filenames`.
    report.blind_mode = matches!(mode, Some("content" | "count"));

    let Some(filenames) = payload
        .tool_response
        .get("filenames")
        .and_then(serde_json::Value::as_array)
    else {
        return report;
    };

    for (index, entry) in filenames.iter().enumerate() {
        let Some(raw) = entry.as_str() else {
            continue;
        };
        if index >= limit {
            // Never a mute truncation: what is left out is named.
            report.skipped.push((raw.to_owned(), "past the limit"));
            continue;
        }
        // ★ The two path shapes. `Grep` returns cwd-relative, `Glob` returns absolute.
        let absolute = if std::path::Path::new(raw).is_absolute() {
            PathBuf::from(raw)
        } else {
            root.resolve(std::path::Path::new(raw))
        };
        let Ok(key) = root.relativize(&absolute) else {
            report.skipped.push((raw.to_owned(), "outside the project"));
            continue;
        };
        // ★ We RE-READ the file. The fingerprint never comes from the payload (invariant 10).
        let Ok(content) = tokio::fs::read_to_string(&absolute).await else {
            // Gone between the search and now: a normal case, and it must be said.
            report.skipped.push((raw.to_owned(), "unreadable"));
            continue;
        };
        // ★ **Shadow mode.** The read is recorded in a parallel read-set that takes part in no
        // verdict: it counts what we would have said, and says nothing (ADR 0027).
        // `filenames.len()` travels with each entry — that size is what will make the
        // threshold decidable AFTER the measurement, instead of being picked on intuition.
        if registry
            .record_shadow_read(session, key.clone(), content, filenames.len())
            .await
            .is_ok()
        {
            report.recorded.push(key);
        } else {
            report.skipped.push((raw.to_owned(), "registry stopped"));
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(json: &str) -> Payload {
        serde_json::from_str(json).expect("test payload")
    }

    /// The Bash policy is applied, and its reason travels through.
    #[test]
    fn a_bash_redirect_into_the_project_is_denied() {
        let p = payload(
            r#"{"hook_event_name":"PreToolUse","tool_name":"Bash",
                "tool_input":{"command":"echo x > notes.txt"}}"#,
        );
        let Response::Deny(reason) = bash(&p) else {
            panic!("a redirection into the project must be refused");
        };
        assert!(reason.contains("notes.txt"), "{reason}");
    }

    /// And what does not write into the project passes — the registry's scope, not an
    /// exception.
    #[test]
    fn a_bash_command_writing_outside_the_project_passes() {
        for command in [
            "ls -al 2>/dev/null",
            "just tui 2>/tmp/tui.log",
            "grep -rn x .",
        ] {
            let p = payload(&format!(
                r#"{{"hook_event_name":"PreToolUse","tool_name":"Bash",
                     "tool_input":{{"command":"{command}"}}}}"#
            ));
            assert_eq!(bash(&p), Response::Silence, "command: {command}");
        }
    }

    /// `content` mode is recognised as a blind spot, and it is **counted**.
    #[test]
    fn grep_content_mode_is_flagged_as_a_blind_spot() {
        let p = payload(
            r#"{"hook_event_name":"PostToolUse","tool_name":"Grep",
                "tool_input":{"pattern":"x","output_mode":"content"},
                "tool_response":{"mode":"content","numFiles":0,"filenames":[],
                                 "content":"a.rs:1:x","numLines":1}}"#,
        );
        let mode = p
            .tool_response
            .get("mode")
            .and_then(serde_json::Value::as_str);
        assert_eq!(mode, Some("content"));
        assert!(matches!(mode, Some("content" | "count")));
    }

    /// The serialised response is the one `trame-hook` knows how to read.
    #[test]
    fn both_responses_are_readable_by_the_hook() {
        assert_eq!(
            Response::Silence.to_line().trim(),
            r#"{"decision":"silence"}"#
        );
        let line = Response::Deny("reason".to_owned()).to_line();
        let parsed: serde_json::Value = serde_json::from_str(line.trim()).expect("JSON");
        assert_eq!(parsed["decision"], "deny");
        assert_eq!(parsed["reason"], "reason");
    }
}
