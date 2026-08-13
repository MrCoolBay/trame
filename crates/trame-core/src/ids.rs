//! Identifiers. Newtypes throughout: a `SessionId` is never interchangeable with
//! a `ProjectId`, even though both carry a UUID.

use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Declare an opaque identifier backed by a v4 UUID.
macro_rules! uuid_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            /// Draw a fresh identifier.
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            /// The underlying UUID. Useful when persisting.
            #[must_use]
            pub fn as_uuid(&self) -> &Uuid {
                &self.0
            }

            /// Rebuild an identifier from a known UUID (reading the journal back).
            #[must_use]
            pub fn from_uuid(uuid: Uuid) -> Self {
                Self(uuid)
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        /// Read back from a `TEXT` column of the journal.
        impl std::str::FromStr for $name {
            type Err = uuid::Error;

            fn from_str(raw: &str) -> Result<Self, Self::Err> {
                Ok(Self(Uuid::parse_str(raw)?))
            }
        }
    };
}

uuid_id! {
    /// A project: one directory, one git repository, one shared working directory.
    ProjectId
}

uuid_id! {
    /// A session: one agent and one goal, inside a project.
    ///
    /// The `human` session (the user in their editor) and the `external` one
    /// (build, formatter, script) are sessions like any other. That uniformity
    /// removes a whole category of special cases.
    SessionId
}

impl SessionId {
    /// The conventional session for **out-of-band** writes.
    ///
    /// `sed -i`, a git hook, a formatter, a build, or the user in their editor: anything
    /// that touches the tree without going through admission. The FSEvents watcher
    /// attributes those writes to this identifier.
    ///
    /// # Why a session rather than the absence of one
    ///
    /// Because the registry has to be able to say "this file changed, and not by you". A
    /// write with no author would stale nothing: the `last_writer == session` comparison
    /// would be meaningless. Handling it as a session like any other removes a whole
    /// category of special cases — the same choice as `Harness::External`.
    ///
    /// The UUID is fixed and documented: it must be recognisable in the journal, and stable
    /// across runs.
    pub const EXTERNAL: Self = Self(Uuid::from_u128(0x7242_414d_4500_0000_0000_0000_0000_0001));

    /// True if this identifier denotes out-of-band writes.
    #[must_use]
    pub fn is_external(&self) -> bool {
        *self == Self::EXTERNAL
    }
}

/// The sequence number of an admitted write.
///
/// **Project-local, never global.** A global counter would be a contention point
/// between projects that, by construction, cannot collide — and it would make the
/// journal unreadable across projects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Seq(u64);

impl Seq {
    /// A project's first sequence number.
    pub const FIRST: Self = Self(1);

    /// The next number.
    #[must_use]
    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }

    /// The raw value, for persistence.
    #[must_use]
    pub fn get(self) -> u64 {
        self.0
    }

    /// Rebuild from the journal.
    #[must_use]
    pub fn from_u64(value: u64) -> Self {
        Self(value)
    }
}

impl fmt::Display for Seq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// A virtual branch's human-readable name. What the user sees.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BranchName(String);

impl BranchName {
    /// Build a branch name.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// The raw name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for BranchName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Declare an opaque identifier backed by a string supplied by a third party.
macro_rules! opaque_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Build the identifier from the value the third party supplied.
            #[must_use]
            pub fn new(raw: impl Into<String>) -> Self {
                Self(raw.into())
            }

            /// The raw value.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

opaque_id! {
    /// A branch's stable identifier on the GitButler side, as returned by
    /// `but status --format json`. Opaque: we never build one ourselves.
    BranchId
}

opaque_id! {
    /// A *change request*: a GitLab merge request, a GitHub pull request.
    ///
    /// The name is deliberately neutral. GitLab is the primary target, not a
    /// second-class citizen.
    CrId
}

opaque_id! {
    /// A discussion thread on a change request.
    ThreadId
}

opaque_id! {
    /// A work item at its source: issue number, thread id, ticket key. The shape
    /// depends on the source, hence the opacity.
    WorkItemId
}
