//! Does the journal **really** write to the database?
//!
//! These tests do not use the default database: they write to a temporary file, close it,
//! reopen it, and read back. A journal that kept everything in memory would pass naive
//! tests and lose everything on the first restart.

use std::path::PathBuf;

use chrono::{TimeDelta, Utc};
use trame_core::{ContentHash, ProjectId, Seq, SessionId, Verdict};
use trame_journal::{
    Journal, ProjectRecord, ReadRecord, SessionRecord, Toolchain, WriteOrigin, WriteRecord,
    spawn_journal,
};

/// A temporary database path, unique per test.
fn temp_db() -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("trame-test-{}.sqlite", ProjectId::new()));
    path
}

fn write_record(project: ProjectId, session: SessionId, seq: u64, path: &str) -> WriteRecord {
    WriteRecord {
        project,
        session,
        session_name: "refacto-api".into(),
        seq: Seq::from_u64(seq),
        path: PathBuf::from(path),
        hash_before: None,
        hash_after: ContentHash::of("content"),
        verdict: Some(Verdict::Clean.label().to_owned()),
        origin: WriteOrigin::Admitted,
        ts: Utc::now(),
    }
}

/// **The test that matters**: what is written survives the process closing.
#[tokio::test]
async fn writes_survive_closing_and_reopening_the_database() {
    let path = temp_db();
    let project = ProjectId::new();
    let session = SessionId::new();

    // First opening: we write.
    {
        let journal = Journal::open(&path).expect("ouverture");
        let (handle, _join) = spawn_journal(journal);

        handle
            .record_project(ProjectRecord {
                id: project,
                path: PathBuf::from("/tmp/projet"),
                name: "projet".into(),
                toolchain: Toolchain::Cargo,
                added_at: Utc::now(),
                last_opened_at: None,
            })
            .await
            .unwrap();
        handle
            .record_session(SessionRecord {
                id: session,
                project,
                name: "refacto-api".into(),
                harness: "claude_code".into(),
                target_branch: "feat/api".into(),
                work_item: None,
                initial_state: "writing".into(),
                created_at: Utc::now(),
            })
            .await
            .unwrap();
        handle
            .record_write(write_record(project, session, 1, "auth.rs"))
            .await
            .unwrap();
        handle
            .record_write(write_record(project, session, 2, "handlers.rs"))
            .await
            .unwrap();

        // A deterministic barrier: when flush returns, everything is in the database.
        // No sleep — the queue is FIFO.
        let report = handle.flush().await.unwrap();
        assert_eq!(report.errors, 0, "no write must have failed");
    }

    // The file really exists on disk.
    assert!(
        path.exists(),
        "the database must be a real file: {}",
        path.display()
    );
    let size = std::fs::metadata(&path).unwrap().len();
    assert!(size > 0, "the database must not be empty");

    // Second opening, a logically fresh process: we read back.
    {
        let journal = Journal::open(&path).expect("reouverture");
        let (handle, _join) = spawn_journal(journal);

        let writes = handle.writes_for_project(project).await.unwrap();
        assert_eq!(writes.len(), 2, "both writes must have survived");
        assert_eq!(writes[0].seq, Seq::from_u64(1));
        assert_eq!(writes[0].path, PathBuf::from("auth.rs"));
        assert_eq!(writes[1].path, PathBuf::from("handlers.rs"));
        assert_eq!(writes[0].verdict.as_deref(), Some("clean"));
        assert_eq!(writes[0].origin, WriteOrigin::Admitted);
    }

    let _ = std::fs::remove_file(&path);
}

/// `UNIQUE(project_id, seq)` is enforced **by the database**, not only by the code. A
/// counter bug must fail at insert time rather than produce a false journal.
#[tokio::test]
async fn the_unique_project_seq_constraint_is_enforced_by_the_database() {
    let journal = Journal::open_in_memory().unwrap();
    let project = ProjectId::new();
    let session = SessionId::new();

    journal
        .insert_write(&write_record(project, session, 1, "a.rs"))
        .unwrap();

    let doublon = journal.insert_write(&write_record(project, session, 1, "b.rs"));
    assert!(doublon.is_err(), "reusing a sequence number must fail");

    // The same number in ANOTHER project is perfectly legitimate: the sequence is
    // project-local.
    let autre_projet = ProjectId::new();
    journal
        .insert_write(&write_record(autre_projet, session, 1, "a.rs"))
        .expect("sequence 1 in another project does not collide");
}

/// The journal is append-only: two reads of the same file produce two rows.
#[tokio::test]
async fn reads_accumulate_without_overwriting_each_other() {
    let journal = Journal::open_in_memory().unwrap();
    let project = ProjectId::new();
    let session = SessionId::new();
    let now = Utc::now();

    for (index, hash) in ["v1", "v2", "v3"].iter().enumerate() {
        journal
            .insert_read(&ReadRecord {
                project,
                session,
                path: PathBuf::from("auth.rs"),
                hash: ContentHash::of(hash),
                ts: now + TimeDelta::seconds(index as i64),
            })
            .unwrap();
    }

    let reads = journal.reads_for_session(session).unwrap();
    assert_eq!(
        reads.len(),
        3,
        "three reads, three rows: nothing is overwritten"
    );
    assert_eq!(reads[0].hash, ContentHash::of("v1"));
    assert_eq!(reads[2].hash, ContentHash::of("v3"));
}

/// All six schema tables exist, and the schema version is recorded.
#[tokio::test]
async fn the_schema_creates_all_six_tables() {
    let journal = Journal::open_in_memory().unwrap();

    for table in [
        "projects",
        "sessions",
        "prompts",
        "reads",
        "writes",
        "resource_claims",
    ] {
        assert!(
            journal.table_exists(table).unwrap(),
            "table {table} must exist"
        );
    }
    assert!(
        journal.schema_version().unwrap() >= 1,
        "the schema version must be recorded"
    );
}

/// Opening the same database twice does not replay the migrations.
#[tokio::test]
async fn migrations_are_idempotent() {
    let path = temp_db();

    let v1 = Journal::open(&path).unwrap().schema_version().unwrap();
    let v2 = Journal::open(&path).unwrap().schema_version().unwrap();
    assert_eq!(v1, v2, "reopening must not replay or bump the migrations");

    let _ = std::fs::remove_file(&path);
}

/// The default location really is under `~/Library/Application Support/Trame/`, never
/// inside the repository: it does not pollute projects and it survives their deletion.
#[tokio::test]
async fn the_default_journal_location_is_under_application_support() {
    let path = trame_journal::default_database_path().expect("default path");
    let texte = path.to_string_lossy();

    assert!(
        texte.contains("Library/Application Support/Trame"),
        "obtenu : {texte}"
    );
    assert!(texte.ends_with("trame.sqlite"), "obtenu : {texte}");
    // The test creates nothing: it checks the path, not the database.
}
