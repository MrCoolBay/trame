//! Le journal ecrit-il **reellement** en base ?
//!
//! Ces tests n'utilisent pas la base par defaut : ils ecrivent dans un file
//! temporaire, le ferment, le rouvrent, et relisent. Un journal qui guard tout en
//! memoire passerait des tests naifs et perdrait tout au premier redemarrage.

use std::path::PathBuf;

use chrono::{TimeDelta, Utc};
use trame_core::{ContentHash, ProjectId, Seq, SessionId, Verdict};
use trame_journal::{
    Journal, ProjectRecord, ReadRecord, SessionRecord, Toolchain, WriteOrigin, WriteRecord,
    spawn_journal,
};

/// Un path de base temporaire, unique par test.
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

/// **Le test qui compte** : ce qui est ecrit survit a la fermeture du processus.
#[tokio::test]
async fn writes_survive_closing_and_reopening_the_database() {
    let path = temp_db();
    let project = ProjectId::new();
    let session = SessionId::new();

    // Premiere ouverture : on ecrit.
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

        // Barriere deterministe : quand flush rend la main, tout est en base.
        // Aucun sleep, la file est FIFO.
        let report = handle.flush().await.unwrap();
        assert_eq!(report.errors, 0, "aucune ecriture ne doit avoir echoue");
    }

    // Le file existe vraiment sur le disque.
    assert!(
        path.exists(),
        "la base doit etre un file reel : {}",
        path.display()
    );
    let taille = std::fs::metadata(&path).unwrap().len();
    assert!(taille > 0, "la base ne doit pas etre vide");

    // Seconde ouverture, processus logiquement neuf : on relit.
    {
        let journal = Journal::open(&path).expect("reouverture");
        let (handle, _join) = spawn_journal(journal);

        let writes = handle.writes_for_project(project).await.unwrap();
        assert_eq!(writes.len(), 2, "les deux ecritures doivent avoir survecu");
        assert_eq!(writes[0].seq, Seq::from_u64(1));
        assert_eq!(writes[0].path, PathBuf::from("auth.rs"));
        assert_eq!(writes[1].path, PathBuf::from("handlers.rs"));
        assert_eq!(writes[0].verdict.as_deref(), Some("clean"));
        assert_eq!(writes[0].origin, WriteOrigin::Admitted);
    }

    let _ = std::fs::remove_file(&path);
}

/// `UNIQUE(project_id, seq)` est applique **par la base**, pas seulement par le code.
/// Un bug de compteur doit echouer a l'insertion plutot que produire un journal faux.
#[tokio::test]
async fn the_unique_project_seq_constraint_is_enforced_by_the_database() {
    let journal = Journal::open_in_memory().unwrap();
    let project = ProjectId::new();
    let session = SessionId::new();

    journal
        .insert_write(&write_record(project, session, 1, "a.rs"))
        .unwrap();

    let doublon = journal.insert_write(&write_record(project, session, 1, "b.rs"));
    assert!(
        doublon.is_err(),
        "reutiliser un numero de sequence doit echouer"
    );

    // Le meme numero dans un AUTRE projet est parfaitement legitime : la sequence est
    // locale au projet.
    let autre_projet = ProjectId::new();
    journal
        .insert_write(&write_record(autre_projet, session, 1, "a.rs"))
        .expect("la sequence 1 d'un autre projet ne collisionne pas");
}

/// Le journal est append-only : deux lectures du meme file produisent deux lines.
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
        "trois lectures, trois lines : rien n'est ecrase"
    );
    assert_eq!(reads[0].hash, ContentHash::of("v1"));
    assert_eq!(reads[2].hash, ContentHash::of("v3"));
}

/// Les six tables du schema existent, et la version de schema est enregistree.
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
            "la table {table} doit exister"
        );
    }
    assert!(
        journal.schema_version().unwrap() >= 1,
        "la version de schema doit etre posee"
    );
}

/// Ouvrir deux fois la meme base ne rejoue pas les migrations.
#[tokio::test]
async fn migrations_are_idempotent() {
    let path = temp_db();

    let v1 = Journal::open(&path).unwrap().schema_version().unwrap();
    let v2 = Journal::open(&path).unwrap().schema_version().unwrap();
    assert_eq!(
        v1, v2,
        "une reouverture ne doit pas rejouer ni incrementer les migrations"
    );

    let _ = std::fs::remove_file(&path);
}

/// L'emplacement par defaut est bien sous `~/Library/Application Support/Trame/`,
/// jamais dans le depot : ca ne pollue pas les projets et ca survit a leur suppression.
#[tokio::test]
async fn the_default_journal_location_is_under_application_support() {
    let path = trame_journal::default_database_path().expect("path par defaut");
    let texte = path.to_string_lossy();

    assert!(
        texte.contains("Library/Application Support/Trame"),
        "obtenu : {texte}"
    );
    assert!(texte.ends_with("trame.sqlite"), "obtenu : {texte}");
    // Le test ne cree rien : il verifie le path, pas la base.
}
