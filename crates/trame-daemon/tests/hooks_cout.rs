// Un test d'integration est un binaire ordinaire.
#![allow(clippy::expect_used, clippy::print_stderr)]

//! ★ Le cout de relecture, mesure avant de decider de borner.
//!
//! L'invariant 10 impose de **relire** chaque fichier rapporte pour l'empreinter — le payload du
//! hook ne peut pas servir de source. Un `Grep` a 300 correspondances coute donc 300 lectures
//! disque. La question etait : faut-il borner ?
//!
//! Ce fichier repond par la mesure, sur la vraie taille observee en sonde 3 (300 fichiers).
//! Il n'assertionne pas un seuil — un test qui epingle une duree est un test instable — il
//! **imprime** le cout et verifie la seule propriete qui compte : que tout soit enregistre ou
//! nomme, jamais perdu en silence.

use std::sync::Arc;
use std::time::Instant;

use trame_core::clock::ManualClock;
use trame_core::{ProjectId, ProjectRoot, SessionId};
use trame_daemon::hooks::{Payload, traiter};
use trame_journal::{Journal, spawn_journal};
use trame_registry::spawn_registry;

const CORRESPONDANCES: usize = 300;

#[tokio::test]
async fn le_cout_de_relecture_sur_300_correspondances() {
    let id = ProjectId::new();
    let root = std::env::temp_dir().join(format!("trame-cout-{id}"));
    std::fs::create_dir_all(root.join("src")).expect("repertoire");
    // Des fichiers de taille realiste : ~2 Ko, l'ordre de grandeur d'un module Rust.
    let contenu = "// ".to_owned() + &"motif_de_masse ".repeat(120) + "\n";
    for index in 0..CORRESPONDANCES {
        std::fs::write(root.join(format!("src/f{index:03}.rs")), &contenu).expect("fixture");
    }

    let (journal, _j) = spawn_journal(Journal::open_in_memory().expect("journal"));
    let (registry, _r) = spawn_registry(
        id,
        ProjectRoot::new(&root).expect("racine"),
        Arc::new(ManualClock::new()),
        journal,
    );
    let session = SessionId::new();
    registry
        .register_session(session, "chercheuse")
        .await
        .expect("registre");

    let noms: Vec<String> = (0..CORRESPONDANCES)
        .map(|i| format!("\"src/f{i:03}.rs\""))
        .collect();
    let payload: Payload = serde_json::from_str(&format!(
        r#"{{"hook_event_name":"PostToolUse","tool_name":"Grep",
             "tool_input":{{"pattern":"motif_de_masse","output_mode":"files_with_matches"}},
             "tool_response":{{"mode":"files_with_matches","numFiles":{CORRESPONDANCES},
                               "filenames":[{}]}}}}"#,
        noms.join(",")
    ))
    .expect("payload");

    let racine = ProjectRoot::new(&root).expect("racine");
    let depart = Instant::now();
    let (_, bilan) = traiter(&payload, &racine, &registry, session, usize::MAX).await;
    let ecoule = depart.elapsed();

    eprintln!(
        "COUT_RELECTURE {CORRESPONDANCES} fichiers de ~{} o : {:.1} ms total, {:.3} ms par fichier",
        contenu.len(),
        ecoule.as_secs_f64() * 1000.0,
        ecoule.as_secs_f64() * 1000.0 / CORRESPONDANCES as f64,
    );

    // La propriete, elle, est verifiee : rien de perdu en silence.
    assert_eq!(bilan.enregistres.len(), CORRESPONDANCES);
    assert!(bilan.ignores.is_empty(), "{:?}", bilan.ignores);
    std::fs::remove_dir_all(&root).ok();
}
