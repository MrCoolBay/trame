//! The normalised event stream.
//!
//! # Why some "events" are requests
//!
//! A stream is one-way: you cannot intercept with a stream. And intercepting is precisely
//! what has to happen for writes — deciding **before** the disk is touched.
//!
//! The variants concerned therefore carry a reply channel. The type then guarantees what
//! documentation could only ask for: ignoring a [`FileWriteRequest`] is impossible without
//! someone noticing, because it refuses by default when dropped.

use std::path::PathBuf;

use serde_json::Value;
use tokio::sync::oneshot;

/// What the backend reports to the rest of the core.
///
/// The rest of the core never knows whether it is talking to ACP or to a PTY.
#[derive(Debug)]
#[non_exhaustive]
pub enum AgentEvent {
    /// A fragment of a message from the agent.
    Message(String),

    /// A tool call, as the agent announces it. Informative: tools that touch files are
    /// additionally reported as requests.
    ToolCall {
        /// The tool's name.
        name: String,
        /// Its parameters, uninterpreted.
        input: Value,
    },

    /// The agent asks to **read** a file. The client serves the read, so the client is the
    /// one that knows the content — and that is what fills the read-set.
    FileRead(FileReadRequest),

    /// ★ The agent asks to **write**. **Nothing is written before the reply.**
    FileWrite(FileWriteRequest),

    /// The agent asks the human for permission.
    PermissionRequest(PermissionRequest),

    /// The agent handed control back.
    Done,

    /// Something failed. A subprocess that dies is a normal case, not a panic.
    Error(String),
}

/// A read request addressed to the client.
#[derive(Debug)]
pub struct FileReadRequest {
    /// The path requested, exactly as the agent phrased it.
    pub path: PathBuf,
    /// Starting line, if the agent asked for only a portion.
    pub line: Option<u32>,
    /// Number of lines, if the agent asked for only a portion.
    pub limit: Option<u32>,
    reply: Option<oneshot::Sender<Result<String, String>>>,
}

impl FileReadRequest {
    /// Builds the request and its reply channel.
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

    /// Supplies the content read.
    ///
    /// **This is also the moment to fill the read-set**: the client is the only one that
    /// knows what the agent actually saw.
    pub fn provide(mut self, content: impl Into<String>) {
        if let Some(reply) = self.reply.take() {
            let _ = reply.send(Ok(content.into()));
        }
    }

    /// Reports that the read is impossible.
    pub fn fail(mut self, reason: impl Into<String>) {
        if let Some(reply) = self.reply.take() {
            let _ = reply.send(Err(reason.into()));
        }
    }
}

impl Drop for FileReadRequest {
    fn drop(&mut self) {
        if let Some(reply) = self.reply.take() {
            tracing::warn!(path = %self.path.display(), "read request dropped");
            let _ = reply.send(Err("read dropped by the client".to_owned()));
        }
    }
}

/// ★ A write request addressed to the client. **The admission point.**
///
/// Until [`FileWriteRequest::admitted`] or [`FileWriteRequest::refuse`] has been called, the
/// agent is waiting and **the disk has not been touched**.
#[derive(Debug)]
pub struct FileWriteRequest {
    /// The target path, exactly as the agent phrased it.
    pub path: PathBuf,
    /// The proposed content, in full.
    pub content: String,
    reply: Option<oneshot::Sender<Result<(), String>>>,
}

impl FileWriteRequest {
    /// Builds the request and its reply channel.
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

    /// The write was **admitted and performed** by the registry (ADR 0014).
    ///
    /// Only to be called once the file is genuinely on the disk: that is what the agent will
    /// believe.
    pub fn admitted(mut self) {
        if let Some(reply) = self.reply.take() {
            let _ = reply.send(Ok(()));
        }
    }

    /// The write is refused. The reason goes back to the agent, which already knows what to
    /// do with a failed tool.
    pub fn refuse(mut self, reason: impl Into<String>) {
        if let Some(reply) = self.reply.take() {
            let _ = reply.send(Err(reason.into()));
        }
    }
}

impl Drop for FileWriteRequest {
    /// **Denies by default.**
    ///
    /// Dropping a write request without answering would leave the agent waiting forever.
    /// Worse: if we answered "admitted" by default, a forgotten request would become an
    /// unadmitted write — exactly what the product exists to prevent. The default is therefore
    /// refusal, and it is loud.
    fn drop(&mut self) {
        if let Some(reply) = self.reply.take() {
            tracing::warn!(
                path = %self.path.display(),
                "write request dropped with no decision: refused by default"
            );
            let _ = reply.send(Err("write not admitted: request dropped".to_owned()));
        }
    }
}

/// A permission request addressed to the human.
///
/// The mechanism already exists on the agent side: it knows how to wait for a permission,
/// there is nothing to teach it. This is the path the registry's level 3 will take, in v0.4.
#[derive(Debug)]
pub struct PermissionRequest {
    /// What the agent wants to do, as one displayable line.
    pub title: String,
    /// The tool concerned.
    pub tool_name: String,
    /// The options the agent proposes.
    pub options: Vec<PermissionOption>,
    reply: Option<oneshot::Sender<Option<String>>>,
}

/// A permission option, as the agent proposes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionOption {
    /// The identifier to send back in order to choose it.
    pub id: String,
    /// The displayable label.
    pub label: String,
    /// The option's nature, as announced by the agent: `allow_once`, `allow_always`,
    /// `reject_once`, `reject_always`.
    pub kind: String,
}

impl PermissionOption {
    /// True if this option allows the action.
    #[must_use]
    pub fn is_allow(&self) -> bool {
        self.kind.starts_with("allow")
    }

    /// True if choosing this option **causes a persistent decision to be written**.
    ///
    /// # Why this matters, and is not a detail
    ///
    /// Observed during live validation: choosing `allow_always` made the agent write
    /// `.claude/settings.local.json` **into the project's working directory**, containing
    /// `{"permissions":{"allow":["mcp__acp__Write"]}}`. That file never went through
    /// `fs/write_text_file`: it is an **out-of-band write, inside the project**, caused by our
    /// own choice.
    ///
    /// In other words: by answering a permission request, we can dirty the very tree we are
    /// supposed to be watching.
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

    /// Chooses an option, by its identifier.
    pub fn choose(mut self, option_id: impl Into<String>) {
        if let Some(reply) = self.reply.take() {
            let _ = reply.send(Some(option_id.into()));
        }
    }

    /// Cancels: the agent abandons the turn.
    pub fn cancel(mut self) {
        if let Some(reply) = self.reply.take() {
            let _ = reply.send(None);
        }
    }

    /// An option that allows **without persisting anything**.
    ///
    /// Always prefers `allow_once` over `allow_always`. That is not free-floating caution:
    /// choosing a persistent option makes the agent write a settings file **into the project's
    /// working directory**, outside admission — see [`PermissionOption::is_persistent`]. We do
    /// not dirty the tree we are watching.
    ///
    /// Returns `None` if the agent only proposes persistent options: in that case it is for
    /// the human to decide, not for a silent default.
    #[must_use]
    pub fn allow_once(&self) -> Option<&PermissionOption> {
        self.options
            .iter()
            .find(|option| option.is_allow() && !option.is_persistent())
    }

    /// An option that refuses without persisting anything.
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
            tracing::warn!(tool = %self.tool_name, "permission request dropped");
            let _ = reply.send(None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options() -> Vec<PermissionOption> {
        // The order is the one observed live: `allow_always` came BEFORE `allow_once`, which
        // is exactly why "the first one that allows" was a bad criterion.
        vec![
            PermissionOption {
                id: "aa".into(),
                label: "Always".into(),
                kind: "allow_always".into(),
            },
            PermissionOption {
                id: "ao".into(),
                label: "Once".into(),
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
            "allow_always is offered first: taking it would write a settings file into the \
             project's working directory"
        );
        assert_eq!(
            request.reject_once().map(|option| option.id.as_str()),
            Some("ro")
        );
    }

    #[test]
    fn with_no_non_persistent_option_we_do_not_choose_for_the_human() {
        let persistent_only = vec![PermissionOption {
            id: "aa".into(),
            label: "Always".into(),
            kind: "allow_always".into(),
        }];
        let (request, _rx) =
            PermissionRequest::new("Write".into(), "Write".into(), persistent_only);
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
