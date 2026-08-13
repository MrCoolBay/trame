//! Registry errors.

use std::path::PathBuf;

use thiserror::Error;

/// The registry actor is no longer reachable.
///
/// The channel is closed, so the task is dead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("the registry is no longer reachable")]
pub struct RegistryGone;

/// What can fail at admission.
///
/// Admission **includes the write** (ADR 0014), so it can fail. A verdict returned without
/// the write having happened would be a lie: the caller would answer "admitted" to the
/// agent, which would believe its file written.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum RegistryError {
    /// The registry is no longer reachable.
    #[error(transparent)]
    Gone(#[from] RegistryGone),

    /// The path escapes the project's working directory. **Always refused**: the registry
    /// can guarantee nothing about what it cannot see, and a write outside the project has
    /// no reason to go through it.
    #[error("path outside the project working directory: {0}")]
    PathOutsideProject(PathBuf),

    /// The write to disk failed.
    ///
    /// The registry's state was **not** updated: otherwise it would believe the file
    /// changed and would wrongly stale the other sessions' reads.
    #[error("cannot write {path}")]
    Write {
        /// The target path, relative to the project root.
        path: PathBuf,
        /// The cause.
        #[source]
        source: std::io::Error,
    },
}
