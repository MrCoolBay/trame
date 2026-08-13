//! ★ **The heart of the product.** The write admission controller.
//!
//! One tokio actor **per project**. It owns its state; nobody shares it.
//!
//! # This is not a lock manager
//!
//! Pessimistic locking does not fit, for three reasons any one of which would be
//! enough: agents hold their transaction for minutes, they do not declare their
//! intent up front, and blocking a tool call in flight triggers timeouts on the
//! harness side. The model is the database one — **optimistic concurrency control
//! with read-set validation**.
//!
//! # Why validate reads and not only writes
//!
//! The most frequent failure mode with three agents produces **no write collision at
//! all**:
//!
//! ```text
//! 1. Session A reads auth.rs, remembers verify_token()'s signature
//! 2. Session B writes auth.rs, renaming verify_token() -> validate_token()
//! 3. Session A writes handlers.rs, calling verify_token()
//!
//! -> Two different files. A per-file lock sees nothing.
//! -> The tree is broken.
//! ```
//!
//! We do not know *whether* it breaks. We know that **A is reasoning about a world
//! that no longer exists**, and that is the only invariant that matters.
//!
//! # v0.1 rules
//!
//! - **Whole-file granularity.** No hunk tracking: file plus time window gives 90% of
//!   the value for 5% of the work (ADR 0012).
//! - **The read-set is filtered** down to substantial reads — see [`ReadKind`].
//!   Otherwise it explodes and everything becomes a `StaleRead`.
//! - **Decay at [`READ_SET_TTL`]**, ten minutes.
//! - The sequence counter is **per project**, never global.
//! - blake3 at admission and at read time. Never the whole tree.
//! - [`trame_core::Verdict::DisjointWrite`] and [`trame_core::Verdict::Overlap`] are
//!   **never produced**: the variants exist, the logic waits for v0.4.
//!
//! **Nothing is blocked.** The registry observes, journals and informs. Blocking will
//! come after the real false-positive rate has been measured — a tool that cries wolf
//! gets switched off within a week.
//!
//! # Architecture
//!
//! - `state` (private) — the logic, **pure and synchronous**. Testable with no runtime,
//!   no agent and no database. Private on purpose: a verdict is asked of the actor,
//!   never of the state directly.
//! - [`spawn_registry`] / [`RegistryHandle`] — the actor that owns it.
//!
//! # Example
//!
//! ```no_run
//! # async fn demo() -> Result<(), Box<dyn std::error::Error>> {
//! use std::sync::Arc;
//! use trame_core::{ProjectId, ProjectRoot, SessionId, clock::SystemClock};
//! use trame_journal::{Journal, spawn_journal};
//! use trame_registry::{ReadKind, spawn_registry};
//!
//! let (journal, _j) = spawn_journal(Journal::open_default()?);
//! let root = ProjectRoot::new("/path/to/project")?;
//! let (registry, _r) =
//!     spawn_registry(ProjectId::new(), root, Arc::new(SystemClock), journal);
//!
//! let session = SessionId::new();
//! registry.register_session(session, "refactor-api").await?;
//! registry.record_read(session, "auth.rs", "fn verify_token()", ReadKind::FullFile).await?;
//!
//! let verdict = registry.admit(session, "handlers.rs", "verify_token()").await?;
//! if verdict.needs_notice() {
//!     // The notice is injected through `trame_core::StaleReadNotice`.
//! }
//! # Ok(())
//! # }
//! ```

mod actor;
mod error;
mod msg;
mod state;

pub use actor::{RegistryHandle, spawn_registry};
pub use error::{RegistryError, RegistryGone};
pub use msg::{
    ExternalWrite, FileSnapshot, ReadKind, RegistrySnapshot, SessionSnapshot, ShadowStats,
};
pub use state::READ_SET_TTL;
