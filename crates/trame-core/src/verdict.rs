//! Admission verdicts.
//!
//! The type lives here, in the foundation crate, because three crates need it:
//! `trame-registry` produces it, `trame-journal` persists it, `trame-tui` displays
//! it. The *logic* that computes it exists in exactly one place — the registry
//! actor.
//!
//! # Nothing is blocked in v0.1
//!
//! The registry observes, journals and informs. Blocking will come once we have
//! measured the real false-positive rate on real usage. A tool that cries wolf
//! gets switched off within a week.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::clock::Timestamp;
use crate::ids::{Seq, SessionId};

/// The outcome of a write admission request.
///
/// Four levels, not a boolean: the right answer to a collision is not always
/// "no".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Verdict {
    /// Level 0. No overlap. ~95% of traffic. Silent.
    Clean,

    /// Level 1. A file in this session's read-set has changed since it was read,
    /// and another session changed it.
    ///
    /// **Admitted**, and a notice is injected into the agent's context. This is the
    /// one mechanism in the product that exists nowhere else: we do not know
    /// *whether* it breaks anything, but we do know the agent is reasoning about a
    /// world that no longer exists.
    StaleRead {
        /// The stale files, most recently modified first.
        stale: Vec<StaleFile>,
    },

    /// Level 2. Same file, disjoint regions. Admitted.
    ///
    /// **Not implemented in v0.1**: granularity is the whole file, so this case is
    /// never produced. The variant exists so that adding it in v0.4 is a `match`
    /// arm to fill in rather than a change to a public type.
    DisjointWrite {
        /// The session that wrote the other region.
        other: SessionId,
    },

    /// Level 3. Overlapping regions. Blocked; we ask the human through the existing
    /// ACP permission mechanism.
    ///
    /// **Not implemented in v0.1**, same reason as [`Verdict::DisjointWrite`].
    Overlap {
        /// The session we overlap with.
        other: SessionId,
    },
}

impl Verdict {
    /// The stable label stored in the `writes.verdict` column.
    /// Never change it without a migration: the journal is append-only.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::Clean => "clean",
            Self::StaleRead { .. } => "stale_read",
            Self::DisjointWrite { .. } => "disjoint_write",
            Self::Overlap { .. } => "overlap",
        }
    }

    /// The numeric level, 0 to 3.
    #[must_use]
    pub fn level(&self) -> u8 {
        match self {
            Self::Clean => 0,
            Self::StaleRead { .. } => 1,
            Self::DisjointWrite { .. } => 2,
            Self::Overlap { .. } => 3,
        }
    }

    /// True if the write is admitted.
    ///
    /// In v0.1: always true, including for [`Verdict::Overlap`], which is never
    /// produced anyway. Blocking will be decided after measurement.
    #[must_use]
    pub fn is_admitted(&self) -> bool {
        !matches!(self, Self::Overlap { .. })
    }

    /// True if the agent must be informed. The only verdict that triggers a
    /// context injection.
    #[must_use]
    pub fn needs_notice(&self) -> bool {
        matches!(self, Self::StaleRead { .. })
    }
}

/// A file read by one session and modified since by another.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StaleFile {
    /// Its path, relative to the project root.
    pub path: PathBuf,
    /// The session that modified it.
    pub last_writer: SessionId,
    /// That session's display name. A UUID tells an agent nothing;
    /// "refactor-api" tells it something.
    pub last_writer_name: String,
    /// When the current session read it.
    pub read_at: Timestamp,
    /// When the other session modified it.
    pub written_at: Timestamp,
    /// The sequence number of that modification, project-local.
    pub seq: Seq,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_levels_are_ordered() {
        let session = SessionId::new();
        assert_eq!(Verdict::Clean.level(), 0);
        assert_eq!(Verdict::StaleRead { stale: vec![] }.level(), 1);
        assert_eq!(Verdict::DisjointWrite { other: session }.level(), 2);
        assert_eq!(Verdict::Overlap { other: session }.level(), 3);
    }

    #[test]
    fn stale_read_is_admitted_and_notified() {
        let verdict = Verdict::StaleRead { stale: vec![] };
        assert!(verdict.is_admitted(), "level 1 informs, it does not block");
        assert!(verdict.needs_notice());
    }

    #[test]
    fn clean_says_nothing() {
        assert!(Verdict::Clean.is_admitted());
        assert!(
            !Verdict::Clean.needs_notice(),
            "95% of traffic must pass without a word"
        );
    }
}
