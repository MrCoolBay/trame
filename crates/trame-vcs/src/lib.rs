//! The VCS layer, one per project. One working directory, never a worktree.
//!
//! # Attribution is data, not a heuristic
//!
//! Every admitted write carries its `session_id`, and therefore its virtual branch. The
//! hunk -> branch assignment is not guessed after the fact: it is known at write time. Three
//! agents finish, three branches are already correctly filled, zero manual sorting.
//!
//! # Shelling out to `but`, deliberately
//!
//! `ButBackend` calls the GitButler CLI as a subprocess, always with `--format json`: a
//! structured API, not scraping human-readable output. The surface we need is about ten
//! commands. Reimplementing virtual branches would be six to eighteen months of work on what
//! is, for Trame, a commodity.
//!
//! `but` is treated as an **external dependency installed by the user**, never vendored — the
//! same way an agent orchestrator does not ship Claude Code with it.
//!
//! `GixBackend`, a native reimplementation on `gitoxide`, is a possible long-term exit. Not a
//! v0.1 goal.
//!
//! This crate is empty as of phase 0.

/// The binary expected on the `PATH`.
///
/// If it is absent, Trame stops and says so. It **never** falls back to plain git: the virtual
/// branch model has no git equivalent, and simulating one with the other would produce false
/// attributions.
pub const BUT_BINARY: &str = "but";

/// The minimum CLI version validated against this code.
pub const BUT_MIN_VERSION: &str = "0.21";
