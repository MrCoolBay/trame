//! The schema, and the migrations that apply it.
//!
//! # Append-only
//!
//! `prompts`, `reads` and `writes` are never updated: one more event is one more row.
//! That is what makes the tool auditable — answering "who wrote this line, in which
//! session, in response to which prompt" is a query, not a reconstruction.
//!
//! # Migrations
//!
//! One [`MIGRATIONS`] entry per version, **never modified after the fact**. Each
//! migration runs in a transaction: a half-applied migration is worse than a failed one.
//! Migrations are **additive** — add a table, add a nullable column. Never rename, never
//! drop, never change a type.

use rusqlite::Connection;

use crate::error::Result;

/// A migration: its version and its SQL.
struct Migration {
    version: u32,
    sql: &'static str,
}

/// Every migration, in order.
static MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    sql: r#"
CREATE TABLE projects (
    id             TEXT PRIMARY KEY,
    path           TEXT NOT NULL,
    name           TEXT NOT NULL,
    toolchain      TEXT NOT NULL,
    added_at       TEXT NOT NULL,
    last_opened_at TEXT
);

CREATE TABLE sessions (
    id            TEXT PRIMARY KEY,
    project_id    TEXT NOT NULL REFERENCES projects(id),
    name          TEXT NOT NULL,
    harness       TEXT NOT NULL,
    target_branch TEXT NOT NULL,
    -- An opaque reference to the originating work item (issue, review thread).
    -- The encoding belongs to the caller: the journal does not interpret it.
    work_item     TEXT,
    -- State AT CREATION, and the name says so. A `state` column in an append-only
    -- table would be read as a current state in phase 3, and it would lie from the
    -- first transition. Transitions will want an events table rather than an UPDATE:
    -- to be settled when sessions really run.
    initial_state TEXT NOT NULL,
    created_at    TEXT NOT NULL
);

CREATE TABLE prompts (
    id         INTEGER PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id),
    content    TEXT NOT NULL,
    ts         TEXT NOT NULL
);

CREATE TABLE reads (
    id         INTEGER PRIMARY KEY,
    project_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    -- Relative to the project root. An absolute path would break the first time the
    -- repository moved, and would leak the owner's directory layout.
    path       TEXT NOT NULL,
    hash       TEXT NOT NULL,
    ts         TEXT NOT NULL
);

CREATE TABLE writes (
    id          INTEGER PRIMARY KEY,
    project_id  TEXT    NOT NULL,
    session_id  TEXT    NOT NULL,
    -- Denormalised on purpose. An audit row must read on its own, with no join, and
    -- stay readable even if the session has gone from the rest of the schema.
    -- The cost is one duplicated string per write; the benefit is that "who wrote
    -- this line" is answered by a SELECT on a single table.
    session_name TEXT   NOT NULL,
    seq         INTEGER NOT NULL,
    path        TEXT    NOT NULL,
    hash_before TEXT,               -- NULL = the file was being created
    hash_after  TEXT    NOT NULL,
    -- Verdict::label(), a stable value. NULL for an OBSERVED write: nobody admitted it,
    -- so no verdict was ever returned. Writing a fake verdict would be a lie.
    verdict     TEXT,
    -- "admitted" or "observed". An observed write is noticed AFTER the fact by the
    -- watcher: the registry could prevent nothing. Conflating them would make the journal
    -- wrong about the one thing that matters — provenance.
    origin      TEXT    NOT NULL,
    ts          TEXT    NOT NULL,
    -- The sequence is PROJECT-LOCAL. The constraint is carried by the database and not
    -- only by the code: a counter bug fails at insert time instead of silently producing
    -- a false journal.
    UNIQUE (project_id, seq)
);

CREATE TABLE resource_claims (
    id         INTEGER PRIMARY KEY,
    resource   TEXT NOT NULL,
    project_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    claimed_at TEXT NOT NULL
);

CREATE INDEX writes_project_ts ON writes (project_id, ts DESC);
CREATE INDEX writes_path       ON writes (project_id, path);
CREATE INDEX writes_session    ON writes (session_id);
CREATE INDEX writes_origin     ON writes (project_id, origin);
CREATE INDEX reads_session_ts  ON reads (session_id, ts DESC);
CREATE INDEX reads_project_path ON reads (project_id, path);
CREATE INDEX sessions_project  ON sessions (project_id);
CREATE INDEX claims_resource   ON resource_claims (resource);
"#,
}];

/// The schema version this binary targets.
pub const TARGET_VERSION: u32 = 1;

/// Apply the missing migrations. Idempotent: reopening an up-to-date database does
/// nothing.
pub(crate) fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_version (
             version    INTEGER NOT NULL,
             applied_at TEXT    NOT NULL
         );",
    )?;

    let current: u32 = conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_version",
        [],
        |row| row.get(0),
    )?;

    for migration in MIGRATIONS.iter().filter(|m| m.version > current) {
        // One transaction per migration: all or nothing.
        conn.execute_batch("BEGIN")?;
        match apply(conn, migration) {
            Ok(()) => conn.execute_batch("COMMIT")?,
            Err(error) => {
                conn.execute_batch("ROLLBACK")?;
                return Err(error);
            }
        }
        tracing::info!(version = migration.version, "migration appliquee");
    }

    Ok(())
}

fn apply(conn: &Connection, migration: &Migration) -> Result<()> {
    conn.execute_batch(migration.sql)?;
    conn.execute(
        "INSERT INTO schema_version (version, applied_at) VALUES (?1, datetime('now'))",
        [migration.version],
    )?;
    Ok(())
}
