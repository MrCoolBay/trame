//! An **append-only** SQLite journal, global to the workspace.
//!
//! # Location
//!
//! `~/Library/Application Support/Trame/trame.sqlite`. One database for every project,
//! with a `project_id` column — **never** a database inside the repository: it does not
//! pollute projects, it survives their deletion, and it makes the cross-project question
//! possible ("what did I do this week, across everything"). That last one is the main
//! reason.
//!
//! # Append-only
//!
//! We do not `UPDATE` and we do not delete. That is what makes the tool auditable: the
//! answer to "who wrote this line, in which session, in response to which prompt" is a
//! query, not a reconstruction.
//!
//! # This module has value on its own
//!
//! Even with no conflict detection at all, a tool that answers the question above is
//! immediately useful. It is also the product's auditability angle.
//!
//! # Architecture
//!
//! - [`Journal`] — the connection and the operations, **synchronous**. Testable without
//!   tokio.
//! - [`spawn_journal`] / [`JournalHandle`] — the actor that owns the `Journal`. A
//!   `Connection` is `Send` but not `Sync`: sharing it behind an `Arc<Mutex<_>>` would be
//!   the obvious solution and the wrong one — this is business state.
//!
//! # Example
//!
//! ```no_run
//! # async fn demo() -> Result<(), Box<dyn std::error::Error>> {
//! use trame_journal::{Journal, spawn_journal};
//!
//! let (journal, _join) = spawn_journal(Journal::open_default()?);
//! // ... record_read / record_write ...
//! let report = journal.flush().await?;
//! assert_eq!(report.errors, 0);
//! # Ok(())
//! # }
//! ```

mod actor;
mod error;
mod records;
mod schema;
mod store;

pub use actor::{FlushReport, JournalHandle, spawn_journal};
pub use error::{JournalError, JournalGone, Result};
pub use records::{
    ProjectRecord, PromptRecord, ReadRecord, ResourceClaimRecord, SessionRecord, WriteOrigin,
    WriteRecord,
};
pub use schema::TARGET_VERSION;
pub use store::{
    APPLICATION_SUPPORT_DIR, DATABASE_FILE_NAME, Journal, data_dir, default_database_path,
};

/// Re-export: building a [`ProjectRecord`] should not force an explicit dependency on
/// `trame-core`.
pub use trame_core::Toolchain;
