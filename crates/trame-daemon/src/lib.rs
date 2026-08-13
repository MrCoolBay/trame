//! The Supervisor and the wiring.
//!
//! # Hierarchy
//!
//! ```text
//! Workspace (the application)
//!  └── Project (a folder + a git repository)
//!       ├── One working directory
//!       ├── A dedicated Write Registry
//!       ├── Virtual branches
//!       └── Session (one agent + one goal)
//! ```
//!
//! # Per project versus global
//!
//! | Per project | Global |
//! |---|---|
//! | Write Registry (one actor) | One SQLite journal (`project_id` column) |
//! | Sequence counter | Resource claims (ports, dev databases) |
//! | Working directory, VCS backend | Concurrency budget (CPU, RAM) |
//! | FSEvents watcher | API quotas and rate limits |
//! | Virtual branches, agent config | Credentials in the Keychain |
//!
//! The subtle point: **resource claims are global**. Port 3000 is machine-wide. Two projects
//! each starting their dev server is the first genuine cross-project conflict.
//!
//! # Parallelism comes from projects, not from sessions
//!
//! Two sessions in two different projects physically cannot collide: separate directories,
//! repositories and indexes. `5 projects × 3 sessions = 15 agents` without ever leaving the
//! safe operating point.
//!
//! # The full chain (phase 3)
//!
//! [`SessionPilot`] is where everything meets.

pub mod command;
pub mod hooks;
pub mod observe;
pub mod project;
pub mod session;
pub mod watcher;

pub use command::{COMMAND_CAPACITY, Command, Commander, DaemonGone, command_channel};
pub use hooks::{Payload, Report, Response, handle};
pub use observe::{OBSERVE_CAPACITY, Observation, Observer, Transport, observe_channel};
pub use project::{Source, open, refuse_dangerous_root};
pub use session::{SessionActivity, SessionPilot, TurnOutcome};
pub use watcher::{PathFilter, WatcherGuard, spawn_watcher, spawn_watcher_observed};

/// The number of sessions per project beyond which Trame was not designed to work.
///
/// This is not a technical limit enforced by the code, it is product framing: anything that
/// only helps beyond that point is out of scope. Parallelism is obtained by adding projects.
pub const SESSIONS_PER_PROJECT_TARGET: u8 = 5;
