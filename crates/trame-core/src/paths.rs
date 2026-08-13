//! Path normalisation around a project root.
//!
//! # Why this module exists
//!
//! It was written after a finding from the live validation of the ACP transport, and it
//! fixes a **silent** failure mode that would have made the product inert without any
//! test noticing.
//!
//! The agent returns paths that are **absolute** and **resolved**: the root we passed was
//! `/var/folders/…/project`, and the agent answered
//! `/private/var/folders/…/project/auth.rs`. On macOS, `/var` is a symlink to
//! `/private/var`.
//!
//! What happens if you just strip the prefix as given: the prefix does not match,
//! relativising fails, and two forms of the **same file** become two different keys in
//! the read-set. The canonical scenario would stop working without breaking anything
//! visible:
//!
//! ```text
//! A reads  /var/folders/…/auth.rs          -> key "/var/folders/…/auth.rs"
//! B writes /private/var/folders/…/auth.rs  -> key "/private/var/folders/…/auth.rs"
//! A writes handlers.rs                     -> Clean, when it should be StaleRead
//! ```
//!
//! The registry would go quiet exactly when it should speak. That is the worst possible
//! failure mode for this tool, and it is invisible: the tests, which use relative paths,
//! all pass.
//!
//! **Every file key in the registry and the journal therefore goes through
//! [`ProjectRoot`].**

use std::path::{Component, Path, PathBuf};

use crate::error::CoreError;

/// A project's canonical root, and the only way to derive paths from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRoot {
    canonical: PathBuf,
}

impl ProjectRoot {
    /// Canonicalise the project root.
    ///
    /// Done once, when the project is opened: this is the only symlink resolution that
    /// touches the disk.
    pub fn new(path: impl AsRef<Path>) -> Result<Self, CoreError> {
        let path = path.as_ref();
        let canonical = path
            .canonicalize()
            .map_err(|source| CoreError::Backend(Box::new(source)))?;
        Ok(Self { canonical })
    }

    /// Build without touching the disk. For tests, and for a project whose root is
    /// already known to be canonical.
    #[must_use]
    pub fn from_canonical(path: impl Into<PathBuf>) -> Self {
        Self {
            canonical: path.into(),
        }
    }

    /// The root, in canonical form.
    #[must_use]
    pub fn as_path(&self) -> &Path {
        &self.canonical
    }

    /// Bring any path to its **root-relative** form, which is the key used everywhere
    /// else.
    ///
    /// Accepts a relative or absolute path, resolved or not. Refuses anything that escapes
    /// the root: the registry can guarantee nothing about what it cannot see.
    ///
    /// Does not require the file to exist — a write often creates a new file. Only the
    /// existing part of the path is resolved.
    pub fn relativize(&self, path: impl AsRef<Path>) -> Result<PathBuf, CoreError> {
        let path = path.as_ref();
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.canonical.join(path)
        };

        let resolved = resolve_existing_prefix(&absolute);
        let normalized = lexical_normalize(&resolved);

        normalized
            .strip_prefix(&self.canonical)
            .map(Path::to_path_buf)
            .map_err(|_| CoreError::PathOutsideProject(path.to_path_buf()))
    }

    /// The inverse: from a relative key to the absolute path to open.
    #[must_use]
    pub fn resolve(&self, relative: impl AsRef<Path>) -> PathBuf {
        self.canonical.join(relative)
    }

    /// True if this path belongs to the project.
    #[must_use]
    pub fn contains(&self, path: impl AsRef<Path>) -> bool {
        self.relativize(path).is_ok()
    }
}

/// Canonicalise the longest **existing** prefix of the path, then replay the tail.
///
/// `canonicalize` requires the target to exist, which is not the case when creating a
/// file. So we resolve what exists — where the symlinks live — and leave the rest as it
/// is.
fn resolve_existing_prefix(path: &Path) -> PathBuf {
    let mut ancestor = path;
    let mut tail: Vec<&std::ffi::OsStr> = Vec::new();

    loop {
        if let Ok(canonical) = ancestor.canonicalize() {
            let mut out = canonical;
            for segment in tail.iter().rev() {
                out.push(segment);
            }
            return out;
        }
        match (ancestor.parent(), ancestor.file_name()) {
            (Some(parent), Some(name)) => {
                tail.push(name);
                ancestor = parent;
            }
            // No existing ancestor left: nothing to resolve, return the path as given.
            _ => return path.to_path_buf(),
        }
    }
}

/// Drop `.` components and resolve `..` **lexically**.
///
/// Essential for the non-existent part of the path: without it, `project/new/../../..`
/// would escape the root without `strip_prefix` noticing.
fn lexical_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                // `pop` on a root does nothing, which is the behaviour we want:
                // we never climb above `/`.
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Create a temporary directory whose path goes through a symlink, like `/var` on
    /// macOS. This is the exact situation from the live validation.
    fn temp_root() -> (PathBuf, ProjectRoot) {
        let brut = std::env::temp_dir().join(format!("trame-paths-{}", crate::ProjectId::new()));
        std::fs::create_dir_all(&brut).unwrap();
        let root = ProjectRoot::new(&brut).unwrap();
        (brut, root)
    }

    /// ★ The test this module exists for.
    ///
    /// On macOS, `std::env::temp_dir()` returns `/var/folders/…` and `canonicalize`
    /// returns `/private/var/folders/…`. Both forms must give **the same key**.
    #[test]
    fn both_forms_of_the_same_path_give_the_same_key() {
        let (brut, root) = temp_root();

        let via_brut = root.relativize(brut.join("auth.rs")).unwrap();
        let via_canonique = root.relativize(root.as_path().join("auth.rs")).unwrap();
        let via_relatif = root.relativize("auth.rs").unwrap();

        assert_eq!(via_brut, PathBuf::from("auth.rs"));
        assert_eq!(via_canonique, PathBuf::from("auth.rs"));
        assert_eq!(via_relatif, PathBuf::from("auth.rs"));

        std::fs::remove_dir_all(&brut).ok();
    }

    #[test]
    fn a_path_that_does_not_exist_still_relativizes() {
        let (brut, root) = temp_root();
        // A creation: the file does not exist yet, and neither does its directory.
        let key = root.relativize(brut.join("src/neuf/file.rs")).unwrap();
        assert_eq!(key, PathBuf::from("src/neuf/file.rs"));
        std::fs::remove_dir_all(&brut).ok();
    }

    #[test]
    fn a_path_outside_the_project_is_refused() {
        let root = ProjectRoot::from_canonical("/projet");
        assert!(matches!(
            root.relativize("/etc/passwd"),
            Err(CoreError::PathOutsideProject(_))
        ));
        assert!(!root.contains("/autre/projet/auth.rs"));
    }

    #[test]
    fn dot_dot_cannot_climb_out_of_the_project_root() {
        let root = ProjectRoot::from_canonical("/projet");
        // Lexically this escapes the project: it must be refused, not silently
        // normalised into something admissible.
        assert!(root.relativize("../../etc/passwd").is_err());
        assert!(root.relativize("/projet/../autre/x.rs").is_err());
        // An internal round trip, on the other hand, stays inside the project.
        assert_eq!(
            root.relativize("/projet/src/../auth.rs").unwrap(),
            PathBuf::from("auth.rs")
        );
    }

    #[test]
    fn redundant_path_components_are_dropped() {
        let root = ProjectRoot::from_canonical("/projet");
        assert_eq!(
            root.relativize("/projet/./src/./auth.rs").unwrap(),
            PathBuf::from("src/auth.rs")
        );
    }

    #[test]
    fn resolve_is_the_inverse_of_relativize() {
        let root = ProjectRoot::from_canonical("/projet");
        let key = root.relativize("/projet/src/auth.rs").unwrap();
        assert_eq!(root.resolve(&key), PathBuf::from("/projet/src/auth.rs"));
    }

    #[test]
    fn the_root_itself_relativizes_to_the_empty_path() {
        let root = ProjectRoot::from_canonical("/projet");
        assert_eq!(root.relativize("/projet").unwrap(), PathBuf::from(""));
    }
}
