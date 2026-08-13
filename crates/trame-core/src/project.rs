//! The project: one directory, one git repository, **one shared working directory**.
//!
//! There is no worktree, no copy, no copy-on-write. That is the precondition for
//! coordination: two agents that do not share a directory have nothing to coordinate.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::clock::Timestamp;
use crate::ids::ProjectId;

/// A project the workspace has open or knows about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    /// Its identifier.
    pub id: ProjectId,
    /// The working directory root. Single, shared by all its sessions.
    pub path: PathBuf,
    /// The display name. By default, the last segment of the path.
    pub name: String,
    /// The detected toolchain. It determines what counts as the project's shared
    /// state, and therefore which resources must be reserved globally.
    pub toolchain: Toolchain,
    /// When the project was added to the workspace.
    pub added_at: Timestamp,
    /// The last time it was opened. `None` if never opened since being added.
    pub last_opened_at: Option<Timestamp>,
}

/// A project's toolchain, inferred from the files present at its root.
///
/// The point is not to know how to build the project — Trame builds nothing — but to
/// know **which shared state** its sessions contend over: `node_modules` and ports for
/// Node, `target/` for Cargo, `.venv` for Python.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Toolchain {
    /// `Cargo.toml`.
    Cargo,
    /// `package.json`.
    Node,
    /// `pyproject.toml`.
    Python,
    /// `go.mod`.
    Go,
    /// Nothing recognised. Trame still works, it just reserves nothing.
    Unknown,
}

impl Toolchain {
    /// The marker file that gives this toolchain away.
    #[must_use]
    pub fn marker(self) -> Option<&'static str> {
        match self {
            Self::Cargo => Some("Cargo.toml"),
            Self::Node => Some("package.json"),
            Self::Python => Some("pyproject.toml"),
            Self::Go => Some("go.mod"),
            Self::Unknown => None,
        }
    }

    /// The directories to exclude from the watcher and the read-set.
    ///
    /// Without this, a single `cargo build` would drown the journal under thousands of
    /// writes no agent asked for.
    #[must_use]
    pub fn shared_state_dirs(self) -> &'static [&'static str] {
        match self {
            Self::Cargo => &["target"],
            Self::Node => &["node_modules", ".next", "dist"],
            Self::Python => &[".venv", "__pycache__", ".pytest_cache"],
            Self::Go => &["vendor"],
            Self::Unknown => &[],
        }
    }

    /// Detection order. The first marker found wins.
    #[must_use]
    pub fn all() -> &'static [Self] {
        &[Self::Cargo, Self::Node, Self::Python, Self::Go]
    }

    /// The stable label stored in the database. Never change it without a migration.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Cargo => "cargo",
            Self::Node => "node",
            Self::Python => "python",
            Self::Go => "go",
            Self::Unknown => "unknown",
        }
    }

    /// The inverse of [`Toolchain::label`]. An unknown label — written by a newer
    /// version of Trame — reads back as [`Toolchain::Unknown`] rather than failing: the
    /// journal is append-only, and the past cannot be rewritten.
    #[must_use]
    pub fn from_label(label: &str) -> Self {
        match label {
            "cargo" => Self::Cargo,
            "node" => Self::Node,
            "python" => Self::Python,
            "go" => Self::Go,
            _ => Self::Unknown,
        }
    }
}

impl Project {
    /// A project's default name: the last segment of its path.
    #[must_use]
    pub fn default_name(path: &Path) -> String {
        path.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_project_name_is_the_last_path_segment() {
        assert_eq!(
            Project::default_name(Path::new("/Users/x/dev/portailfcd")),
            "portailfcd"
        );
    }

    #[test]
    fn every_known_toolchain_has_a_marker_file() {
        for toolchain in Toolchain::all() {
            assert!(
                toolchain.marker().is_some(),
                "{toolchain:?} must have a marker file"
            );
        }
        assert!(Toolchain::Unknown.marker().is_none());
    }
}
