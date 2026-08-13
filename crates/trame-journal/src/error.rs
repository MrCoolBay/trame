//! Journal errors.

use std::path::PathBuf;

use thiserror::Error;

/// Local alias.
pub type Result<T, E = JournalError> = std::result::Result<T, E>;

/// What can fail on the journal side.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum JournalError {
    /// The database location cannot be determined.
    #[error("cannot locate the database: {0} is not set")]
    NoHome(&'static str),

    /// The application support directory could not be created.
    #[error("cannot create directory {path}")]
    CreateDir {
        /// The target directory.
        path: PathBuf,
        /// The cause.
        #[source]
        source: std::io::Error,
    },

    /// A SQLite error. Covers opening, migrations and queries.
    #[error("SQLite error")]
    Sqlite(#[from] rusqlite::Error),

    /// A row read back does not decode into the expected types. A sign of a missing
    /// migration, or of a write made outside this crate.
    #[error("unreadable row in {table}.{column}: {value}")]
    Decode {
        /// The table in question.
        table: &'static str,
        /// The column in question.
        column: &'static str,
        /// The offending value, as read.
        value: String,
    },

    /// A sequence number does not fit in a SQLite integer. Unreachable in practice; we
    /// prefer the error to a silent cast.
    #[error("sequence number out of range: {0}")]
    SeqOutOfRange(u64),
}

/// The journal actor is no longer reachable.
///
/// The only error [`crate::JournalHandle`]'s methods return: the channel is closed, so
/// the task is dead. Nothing else can fail on the caller's side — a SQLite write error is
/// logged by the actor and counted in [`crate::FlushReport::errors`], it does not surface
/// on every call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("the journal is no longer reachable")]
pub struct JournalGone;
