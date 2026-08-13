//! [`Journal`]: the SQLite connection and the operations that touch it.
//!
//! Everything here is **synchronous**. Asynchrony is the actor's problem
//! ([`crate::actor`]), which owns this `Journal` and serialises access. That separation
//! makes the storage testable without a tokio runtime.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use rusqlite::{Connection, OptionalExtension, params};
use trame_core::clock::Timestamp;
use trame_core::{ContentHash, ProjectId, Seq, SessionId};

use crate::error::{JournalError, Result};
use crate::records::{
    ProjectRecord, PromptRecord, ReadRecord, ResourceClaimRecord, SessionRecord, WriteOrigin,
    WriteRecord,
};
use crate::schema;

/// The database file name, under the application support directory.
pub const DATABASE_FILE_NAME: &str = "trame.sqlite";

/// The subdirectory of `~/Library/Application Support/`.
pub const APPLICATION_SUPPORT_DIR: &str = "Trame";

/// The data directory: `~/Library/Application Support/Trame/`.
///
/// **Global, never inside the repository.** Three reasons: it does not pollute projects,
/// it survives their deletion, and it makes the cross-project question possible — "what
/// did I do this week, across everything". The last one is the main reason.
pub fn data_dir() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").ok_or(JournalError::NoHome("HOME"))?;
    Ok(PathBuf::from(home)
        .join("Library")
        .join("Application Support")
        .join(APPLICATION_SUPPORT_DIR))
}

/// The full path of the default database.
pub fn default_database_path() -> Result<PathBuf> {
    Ok(data_dir()?.join(DATABASE_FILE_NAME))
}

/// The append-only journal.
pub struct Journal {
    conn: Connection,
}

impl Journal {
    /// Open — or create — the database at the given path, and apply the migrations.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| JournalError::CreateDir {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let conn = Connection::open(path)?;
        // WAL: a writer does not block readers, which matters for a database shared
        // between projects. `execute_batch` tolerates the PRAGMA's returned row.
        conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;")?;
        Self::prepare(conn)
    }

    /// The default database, under `~/Library/Application Support/Trame/`.
    pub fn open_default() -> Result<Self> {
        Self::open(&default_database_path()?)
    }

    /// An in-memory database. For tests: they never write into the application support
    /// directory.
    pub fn open_in_memory() -> Result<Self> {
        Self::prepare(Connection::open_in_memory()?)
    }

    fn prepare(conn: Connection) -> Result<Self> {
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        schema::migrate(&conn)?;
        Ok(Self { conn })
    }

    /// The applied schema version.
    pub fn schema_version(&self) -> Result<u32> {
        Ok(self.conn.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_version",
            [],
            |row| row.get(0),
        )?)
    }

    /// True if the table exists. A test and diagnostic helper.
    pub fn table_exists(&self, name: &str) -> Result<bool> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
                params![name],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    // ------------------------------------------------------------------- writes

    /// Append a project.
    pub fn insert_project(&self, record: &ProjectRecord) -> Result<()> {
        self.conn.execute(
            "INSERT INTO projects (id, path, name, toolchain, added_at, last_opened_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                record.id.to_string(),
                path_to_text(&record.path),
                record.name,
                record.toolchain.label(),
                record.added_at,
                record.last_opened_at,
            ],
        )?;
        Ok(())
    }

    /// Append a session.
    pub fn insert_session(&self, record: &SessionRecord) -> Result<()> {
        self.conn.execute(
            "INSERT INTO sessions
               (id, project_id, name, harness, target_branch, work_item, initial_state, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                record.id.to_string(),
                record.project.to_string(),
                record.name,
                record.harness,
                record.target_branch,
                record.work_item,
                record.initial_state,
                record.created_at,
            ],
        )?;
        Ok(())
    }

    /// Append a prompt.
    pub fn insert_prompt(&self, record: &PromptRecord) -> Result<()> {
        self.conn.execute(
            "INSERT INTO prompts (session_id, content, ts) VALUES (?1, ?2, ?3)",
            params![record.session.to_string(), record.content, record.ts],
        )?;
        Ok(())
    }

    /// Append a read.
    pub fn insert_read(&self, record: &ReadRecord) -> Result<()> {
        self.conn.execute(
            "INSERT INTO reads (project_id, session_id, path, hash, ts) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                record.project.to_string(),
                record.session.to_string(),
                path_to_text(&record.path),
                record.hash.to_hex(),
                record.ts,
            ],
        )?;
        Ok(())
    }

    /// Append a write.
    ///
    /// Fails if `(project_id, seq)` already exists — the `UNIQUE` constraint is what
    /// makes the database enforce the per-project counter invariant.
    pub fn insert_write(&self, record: &WriteRecord) -> Result<()> {
        let seq = i64::try_from(record.seq.get())
            .map_err(|_| JournalError::SeqOutOfRange(record.seq.get()))?;
        self.conn.execute(
            "INSERT INTO writes
               (project_id, session_id, session_name, seq, path,
                hash_before, hash_after, verdict, origin, ts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                record.project.to_string(),
                record.session.to_string(),
                record.session_name,
                seq,
                path_to_text(&record.path),
                record.hash_before.map(|hash| hash.to_hex()),
                record.hash_after.to_hex(),
                record.verdict,
                record.origin.label(),
                record.ts,
            ],
        )?;
        Ok(())
    }

    /// Append a resource claim.
    pub fn insert_resource_claim(&self, record: &ResourceClaimRecord) -> Result<()> {
        self.conn.execute(
            "INSERT INTO resource_claims (resource, project_id, session_id, claimed_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                record.resource,
                record.project.to_string(),
                record.session.to_string(),
                record.claimed_at,
            ],
        )?;
        Ok(())
    }

    // ----------------------------------------------------------------- lectures

    /// A project's writes, in sequence order.
    pub fn writes_for_project(&self, project: ProjectId) -> Result<Vec<WriteRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT project_id, session_id, session_name, seq, path,
                    hash_before, hash_after, verdict, origin, ts
             FROM writes WHERE project_id = ?1 ORDER BY seq ASC",
        )?;
        let rows = stmt.query_map(params![project.to_string()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, Timestamp>(9)?,
            ))
        })?;

        let mut out = Vec::new();
        for row in rows {
            let (project, session, session_name, seq, path, before, after, verdict, origin, ts) =
                row?;
            out.push(WriteRecord {
                project: parse_project(&project)?,
                session: parse_session(&session)?,
                session_name,
                seq: Seq::from_u64(u64::try_from(seq).map_err(|_| JournalError::Decode {
                    table: "writes",
                    column: "seq",
                    value: seq.to_string(),
                })?),
                path: PathBuf::from(path),
                hash_before: before.as_deref().map(parse_hash).transpose()?,
                hash_after: parse_hash(&after)?,
                verdict,
                origin: WriteOrigin::from_label(&origin),
                ts,
            });
        }
        Ok(out)
    }

    /// A session's reads, oldest first.
    ///
    /// `ORDER BY ts, id` and not just `ts`: two events can share a millisecond, and the
    /// autoincrementing `id` breaks the tie in real insertion order.
    pub fn reads_for_session(&self, session: SessionId) -> Result<Vec<ReadRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT project_id, session_id, path, hash, ts
             FROM reads WHERE session_id = ?1 ORDER BY ts ASC, id ASC",
        )?;
        let rows = stmt.query_map(params![session.to_string()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Timestamp>(4)?,
            ))
        })?;

        let mut out = Vec::new();
        for row in rows {
            let (project, session, path, hash, ts) = row?;
            out.push(ReadRecord {
                project: parse_project(&project)?,
                session: parse_session(&session)?,
                path: PathBuf::from(path),
                hash: parse_hash(&hash)?,
                ts,
            });
        }
        Ok(out)
    }

    /// The row count of a table in the schema. Diagnostics and tests.
    ///
    /// `table` is not a bound parameter — SQLite does not accept one for a table name —
    /// so it is validated against the schema's allow-list before interpolation. No
    /// caller-supplied string ever reaches the query.
    pub fn count(&self, table: &str) -> Result<u64> {
        const TABLES: &[&str] = &[
            "projects",
            "sessions",
            "prompts",
            "reads",
            "writes",
            "resource_claims",
        ];
        if !TABLES.contains(&table) {
            return Err(JournalError::Decode {
                table: "sqlite_master",
                column: "name",
                value: table.to_owned(),
            });
        }
        let count: i64 =
            self.conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })?;
        u64::try_from(count).map_err(|_| JournalError::Decode {
            table: "sqlite_master",
            column: "count",
            value: count.to_string(),
        })
    }
}

/// Paths are stored as UTF-8. A non-UTF-8 path is replaced by its lossy form rather than
/// failing a journal write: losing an exotic accent matters less than losing provenance.
fn path_to_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn parse_project(raw: &str) -> Result<ProjectId> {
    ProjectId::from_str(raw).map_err(|_| JournalError::Decode {
        table: "writes",
        column: "project_id",
        value: raw.to_owned(),
    })
}

fn parse_session(raw: &str) -> Result<SessionId> {
    SessionId::from_str(raw).map_err(|_| JournalError::Decode {
        table: "writes",
        column: "session_id",
        value: raw.to_owned(),
    })
}

fn parse_hash(raw: &str) -> Result<ContentHash> {
    ContentHash::from_hex(raw).map_err(|_| JournalError::Decode {
        table: "writes",
        column: "hash",
        value: raw.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use trame_core::Verdict;

    use super::*;

    fn journal() -> Journal {
        Journal::open_in_memory().unwrap()
    }

    #[test]
    fn a_fresh_database_is_at_the_target_schema_version() {
        assert_eq!(journal().schema_version().unwrap(), schema::TARGET_VERSION);
    }

    #[test]
    fn count_refuses_an_arbitrary_table_name() {
        let journal = journal();
        // No caller-supplied string can be interpolated into the query.
        assert!(journal.count("writes; DROP TABLE writes").is_err());
        assert!(journal.count("sqlite_master").is_err());
        assert_eq!(journal.count("writes").unwrap(), 0);
    }

    #[test]
    fn a_write_read_back_is_identical_to_the_one_stored() {
        let journal = journal();
        let record = WriteRecord {
            project: ProjectId::new(),
            session: SessionId::new(),
            session_name: "refacto-api".into(),
            seq: Seq::FIRST,
            path: PathBuf::from("src/auth.rs"),
            hash_before: Some(ContentHash::of("before")),
            hash_after: ContentHash::of("after"),
            verdict: Some(Verdict::StaleRead { stale: vec![] }.label().to_owned()),
            origin: WriteOrigin::Admitted,
            ts: chrono::Utc::now(),
        };
        journal.insert_write(&record).unwrap();

        let relu = journal.writes_for_project(record.project).unwrap();
        assert_eq!(relu.len(), 1);
        assert_eq!(relu[0].path, record.path);
        assert_eq!(relu[0].hash_before, record.hash_before);
        assert_eq!(relu[0].hash_after, record.hash_after);
        assert_eq!(relu[0].verdict.as_deref(), Some("stale_read"));
        assert_eq!(relu[0].origin, WriteOrigin::Admitted);
    }
}
