//! Le schema, et les migrations qui l'appliquent.
//!
//! # Append-only
//!
//! `prompts`, `reads` et `writes` ne sont jamais mis a jour : un evenement de plus est
//! une ligne de plus. C'est ce qui rend l'outil auditable — repondre a « qui a ecrit
//! cette ligne, dans quelle session, en reponse a quel prompt » est une requete, pas
//! une reconstruction.
//!
//! # Migrations
//!
//! Un element de [`MIGRATIONS`] par version, **jamais modifie apres coup**. Chaque
//! migration tourne dans une transaction : une migration a moitie appliquee est pire
//! qu'une migration echouee. Les migrations sont **additives** — ajouter une table,
//! ajouter une colonne nullable. Jamais renommer, jamais supprimer, jamais changer un
//! type.

use rusqlite::Connection;

use crate::error::Result;

/// Une migration : sa version et son SQL.
struct Migration {
    version: u32,
    sql: &'static str,
}

/// Toutes les migrations, dans l'ordre.
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
    -- Reference opaque vers l'element de travail d'origine (issue, thread de review).
    -- L'encodage appartient a l'appelant : le journal ne l'interprete pas.
    work_item     TEXT,
    -- Etat A LA CREATION, et le nom le dit. Une colonne `state` dans une table
    -- append-only serait lue comme un etat courant en phase 3, et elle mentirait des
    -- la premiere transition. Les transitions demanderont une table d'evenements
    -- plutot qu'un UPDATE : a trancher quand les sessions tourneront vraiment.
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
    -- Relatif a la racine du projet. Un chemin absolu casserait au premier
    -- deplacement du depot et ferait fuiter l'arborescence personnelle.
    path       TEXT NOT NULL,
    hash       TEXT NOT NULL,
    ts         TEXT NOT NULL
);

CREATE TABLE writes (
    id          INTEGER PRIMARY KEY,
    project_id  TEXT    NOT NULL,
    session_id  TEXT    NOT NULL,
    -- Denormalise a dessein. Une ligne de journal d'audit doit se lire seule, sans
    -- jointure, et rester lisible meme si la session a disparu du reste du schema.
    -- Le cout est une chaine dupliquee par ecriture ; le benefice est que la question
    -- « qui a ecrit cette ligne » se repond par un SELECT sur une seule table.
    session_name TEXT   NOT NULL,
    seq         INTEGER NOT NULL,
    path        TEXT    NOT NULL,
    hash_before TEXT,               -- NULL = creation du fichier
    hash_after  TEXT    NOT NULL,
    verdict     TEXT    NOT NULL,   -- Verdict::label(), valeur stable
    ts          TEXT    NOT NULL,
    -- La sequence est LOCALE AU PROJET. Contrainte portee par la base et pas
    -- seulement par le code : un bug de compteur echoue a l'insertion au lieu de
    -- produire silencieusement un journal faux.
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
CREATE INDEX reads_session_ts  ON reads (session_id, ts DESC);
CREATE INDEX reads_project_path ON reads (project_id, path);
CREATE INDEX sessions_project  ON sessions (project_id);
CREATE INDEX claims_resource   ON resource_claims (resource);
"#,
}];

/// La version de schema visee par ce binaire.
pub const TARGET_VERSION: u32 = 1;

/// Applique les migrations manquantes. Idempotent : rouvrir une base a jour ne fait
/// rien.
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
        // Une transaction par migration : tout ou rien.
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
