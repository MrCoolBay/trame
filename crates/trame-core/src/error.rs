//! Errors for the foundation crate.
//!
//! Project rule: `thiserror` in libraries, `anyhow` only in binaries. A library
//! that returns `anyhow::Error` forces its caller to pattern-match on strings.

use std::path::PathBuf;

use thiserror::Error;

/// Local alias. `Result<T>` is enough throughout this crate.
pub type Result<T, E = CoreError> = std::result::Result<T, E>;

/// Errors common to the `trame-core` seams.
///
/// Concrete implementations of the traits ([`crate::Forge`], [`crate::TaskSource`])
/// live in other crates and have their own errors: they surface them through
/// [`CoreError::Backend`] rather than imposing their variants here.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CoreError {
    /// A path escapes the project's working directory. Always refused: the
    /// registry can guarantee nothing about what it cannot see.
    #[error("path outside the project working directory: {0}")]
    PathOutsideProject(PathBuf),

    /// A requested entity does not exist.
    #[error("{what} not found: {id}")]
    NotFound {
        /// What kind of entity: "session", "project", "work item".
        what: &'static str,
        /// Its identifier, as supplied.
        id: String,
    },

    /// The backend cannot do this. Typical case: a `PtyBackend` asked to intercept
    /// a write. **Surface it to the user** rather than swallowing it: they need to
    /// know they are running degraded.
    #[error("not available on this backend: {0}")]
    Unsupported(&'static str),

    /// An error raised by a concrete implementation.
    #[error("backend error: {0}")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync + 'static>),
}

impl CoreError {
    /// Wrap a backend's error.
    pub fn backend(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Backend(Box::new(source))
    }
}
