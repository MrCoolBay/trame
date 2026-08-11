//! [`Journal`] : la connexion SQLite et les operations qui la touchent.
//!
//! Tout est **synchrone**. L'asynchronisme est le probleme de l'acteur
//! ([`crate::actor`]), qui possede ce `Journal` et serialise les acces. Cette
//! separation rend le stockage testable sans runtime tokio.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use rusqlite::{Connection, OptionalExtension, params};
use trame_core::clock::Timestamp;
use trame_core::{ContentHash, ProjectId, Seq, SessionId};

use crate::error::{JournalError, Result};
use crate::records::{
    ProjectRecord, PromptRecord, ReadRecord, ResourceClaimRecord, SessionRecord, WriteRecord,
};
use crate::schema;

/// Le nom du fichier de base, sous le repertoire de support de l'application.
pub const DATABASE_FILE_NAME: &str = "trame.sqlite";

/// Le sous-repertoire de `~/Library/Application Support/`.
pub const APPLICATION_SUPPORT_DIR: &str = "Trame";

/// Le repertoire de donnees : `~/Library/Application Support/Trame/`.
///
/// **Global, jamais dans le depot.** Trois raisons : ca ne pollue pas les projets, ca
/// survit a leur suppression, et ca permet la question transverse — « qu'est-ce que
/// j'ai fait cette semaine, tous projets confondus ». La derniere est la principale.
pub fn data_dir() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").ok_or(JournalError::NoHome("HOME"))?;
    Ok(PathBuf::from(home)
        .join("Library")
        .join("Application Support")
        .join(APPLICATION_SUPPORT_DIR))
}

/// Le chemin complet de la base par defaut.
pub fn default_database_path() -> Result<PathBuf> {
    Ok(data_dir()?.join(DATABASE_FILE_NAME))
}

/// Le journal append-only.
pub struct Journal {
    conn: Connection,
}

impl Journal {
    /// Ouvre — ou cree — la base au chemin donne, et applique les migrations.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| JournalError::CreateDir {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let conn = Connection::open(path)?;
        // WAL : un ecrivain n'empeche pas les lecteurs, ce qui compte avec une base
        // partagee entre projets. `execute_batch` tolere le retour de ligne du PRAGMA.
        conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;")?;
        Self::prepare(conn)
    }

    /// La base par defaut, sous `~/Library/Application Support/Trame/`.
    pub fn open_default() -> Result<Self> {
        Self::open(&default_database_path()?)
    }

    /// Une base en memoire. Pour les tests : ils n'ecrivent jamais dans le repertoire
    /// de support de l'application.
    pub fn open_in_memory() -> Result<Self> {
        Self::prepare(Connection::open_in_memory()?)
    }

    fn prepare(conn: Connection) -> Result<Self> {
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        schema::migrate(&conn)?;
        Ok(Self { conn })
    }

    /// La version de schema appliquee.
    pub fn schema_version(&self) -> Result<u32> {
        Ok(self.conn.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_version",
            [],
            |row| row.get(0),
        )?)
    }

    /// Vrai si la table existe. Utilitaire de test et de diagnostic.
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

    // ---------------------------------------------------------------- ecritures

    /// Ajoute un projet.
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

    /// Ajoute une session.
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

    /// Ajoute un prompt.
    pub fn insert_prompt(&self, record: &PromptRecord) -> Result<()> {
        self.conn.execute(
            "INSERT INTO prompts (session_id, content, ts) VALUES (?1, ?2, ?3)",
            params![record.session.to_string(), record.content, record.ts],
        )?;
        Ok(())
    }

    /// Ajoute une lecture.
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

    /// Ajoute une ecriture admise.
    ///
    /// Echoue si `(project_id, seq)` existe deja — c'est la contrainte `UNIQUE` qui
    /// fait appliquer par la base l'invariant du compteur par projet.
    pub fn insert_write(&self, record: &WriteRecord) -> Result<()> {
        let seq = i64::try_from(record.seq.get())
            .map_err(|_| JournalError::SeqOutOfRange(record.seq.get()))?;
        self.conn.execute(
            "INSERT INTO writes
               (project_id, session_id, session_name, seq, path,
                hash_before, hash_after, verdict, ts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                record.project.to_string(),
                record.session.to_string(),
                record.session_name,
                seq,
                path_to_text(&record.path),
                record.hash_before.map(|hash| hash.to_hex()),
                record.hash_after.to_hex(),
                record.verdict,
                record.ts,
            ],
        )?;
        Ok(())
    }

    /// Ajoute une reservation de ressource.
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

    /// Les ecritures d'un projet, dans l'ordre de sequence.
    pub fn writes_for_project(&self, project: ProjectId) -> Result<Vec<WriteRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT project_id, session_id, session_name, seq, path,
                    hash_before, hash_after, verdict, ts
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
                row.get::<_, String>(7)?,
                row.get::<_, Timestamp>(8)?,
            ))
        })?;

        let mut out = Vec::new();
        for row in rows {
            let (project, session, session_name, seq, path, before, after, verdict, ts) = row?;
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
                ts,
            });
        }
        Ok(out)
    }

    /// Les lectures d'une session, du plus ancien au plus recent.
    ///
    /// `ORDER BY ts, id` et pas seulement `ts` : deux evenements peuvent partager une
    /// milliseconde, et `id` autoincremente departage dans l'ordre d'insertion reel.
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

    /// Le nombre de lignes d'une table du schema. Diagnostic et tests.
    ///
    /// `table` n'est pas un parametre lie — SQLite n'en accepte pas pour un nom de
    /// table — donc il est valide contre la liste blanche du schema avant
    /// interpolation. Aucune chaine d'appelant n'atteint la requete.
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

/// Les chemins sont stockes en UTF-8. Un chemin non UTF-8 est remplace par sa forme
/// approchee plutot que de faire echouer une ecriture de journal : perdre un accent
/// exotique est moins grave que perdre la provenance.
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
    fn une_base_neuve_est_a_la_version_cible() {
        assert_eq!(journal().schema_version().unwrap(), schema::TARGET_VERSION);
    }

    #[test]
    fn count_refuse_un_nom_de_table_arbitraire() {
        let journal = journal();
        // Pas d'interpolation possible d'une chaine d'appelant dans la requete.
        assert!(journal.count("writes; DROP TABLE writes").is_err());
        assert!(journal.count("sqlite_master").is_err());
        assert_eq!(journal.count("writes").unwrap(), 0);
    }

    #[test]
    fn une_ecriture_relue_est_identique_a_l_ecrite() {
        let journal = journal();
        let record = WriteRecord {
            project: ProjectId::new(),
            session: SessionId::new(),
            session_name: "refacto-api".into(),
            seq: Seq::FIRST,
            path: PathBuf::from("src/auth.rs"),
            hash_before: Some(ContentHash::of("avant")),
            hash_after: ContentHash::of("apres"),
            verdict: Verdict::StaleRead { stale: vec![] }.label().to_owned(),
            ts: chrono::Utc::now(),
        };
        journal.insert_write(&record).unwrap();

        let relu = journal.writes_for_project(record.project).unwrap();
        assert_eq!(relu.len(), 1);
        assert_eq!(relu[0].path, record.path);
        assert_eq!(relu[0].hash_before, record.hash_before);
        assert_eq!(relu[0].hash_after, record.hash_after);
        assert_eq!(relu[0].verdict, "stale_read");
    }
}
