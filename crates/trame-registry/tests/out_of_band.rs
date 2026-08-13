//! ★★ **Le scenario canonique, quand B ecrit par le shell.**
//!
//! La sonde a confirme qu'une session peut ecrire hors admission. Le probleme n'est pas la
//! couverture du journal : c'est que **le registre devient faux**. Si B modifie `auth.rs`
//! par `sed -i`, le `FileState` guard l'ancien hash, et A n'obtient **jamais** son
//! `StaleRead`. Le mecanisme central echoue silencieusement — l'outil a l'air de
//! fonctionner et ne fait rien.
//!
//! Ces tests couvrent les deux faces :
//!
//! - **sans observation**, le registre est faux et se tait a tort. C'est le bug, et il est
//!   teste pour qu'on voie ce qu'on repare.
//! - **avec observation**, A obtient son `StaleRead` malgre le contournement.

// Meme raison que dans les autres tests d'integration : un binaire ordinaire n'a ni
// `cfg(test)` ni `#[test]` sur ses fonctions utilitaires, donc les exemptions de
// `clippy.toml` ne s'y appliquent pas.
#![allow(clippy::expect_used)]

mod common;

use common::Harness;
use trame_core::{ContentHash, SessionId, Verdict};
use trame_journal::WriteOrigin;
use trame_registry::ReadKind;

/// Ecrit un file **sans passer par le registre**, comme le ferait `sed -i`.
fn write_out_of_band(h: &Harness, relatif: &str, content: &str) -> ContentHash {
    let target = h.root.join(relatif);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).expect("repertoire");
    }
    std::fs::write(&target, content).expect("ecriture hors-bande");
    ContentHash::of(content)
}

/// ★★ Le test qui justifie le watcher.
///
/// A lit `auth.rs`. B le modifie **par le shell**, hors admission. A ecrit `handlers.rs`.
/// Grace a l'observation, A obtient son `StaleRead` — exactement comme si B etait passe par
/// l'admission.
#[tokio::test]
async fn a_gets_its_stale_read_even_when_b_writes_through_the_shell() {
    let h = Harness::new();
    let a = h.session("ajout-handlers").await;

    // 1. A lit auth.rs par le path normal.
    h.registry
        .record_read(a, "auth.rs", "pub fn verify_token() {}", ReadKind::FullFile)
        .await
        .unwrap();

    // 2. B contourne : `sed -i` sur auth.rs. Aucune admission, aucun verdict.
    let hash = write_out_of_band(&h, "auth.rs", "pub fn validate_token() {}");

    // 3. Le watcher constate. Apres coup : il n'a rien empeche.
    h.registry
        .observe_external_write("auth.rs", hash)
        .await
        .unwrap();

    // 4. A ecrit ailleurs — et doit etre informe.
    let verdict = h
        .registry
        .admit(a, "handlers.rs", "verify_token()")
        .await
        .unwrap();

    let Verdict::StaleRead { stale } = &verdict else {
        panic!(
            "attendu StaleRead : sans l'observation, le registre resterait sur l'ancien hash \
             et se tairait a tort. Obtenu {verdict:?}"
        );
    };
    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0].path, std::path::PathBuf::from("auth.rs"));
    assert_eq!(
        stale[0].last_writer,
        SessionId::EXTERNAL,
        "l'auteur est la session conventionnelle des ecritures hors-bande"
    );
    assert_eq!(
        stale[0].last_writer_name, "hors-bande",
        "un agent doit comprendre qu'aucune session nommee n'a fait ca"
    );
}

/// **Le bug, sans le correctif.** Sans observation, le registre se tait a tort.
///
/// Ce test documente ce qu'on repare. S'il se mettait a echouer, ce serait que quelque chose
/// d'autre rattrape les ecritures hors-bande — et il faudrait comprendre quoi.
#[tokio::test]
async fn without_the_watcher_the_registry_stays_wrongly_silent() {
    let h = Harness::new();
    let a = h.session("ajout-handlers").await;

    h.registry
        .record_read(a, "auth.rs", "pub fn verify_token() {}", ReadKind::FullFile)
        .await
        .unwrap();
    write_out_of_band(&h, "auth.rs", "pub fn validate_token() {}");
    // Pas d'observation : le watcher est absent.

    let verdict = h
        .registry
        .admit(a, "handlers.rs", "verify_token()")
        .await
        .unwrap();
    assert_eq!(
        verdict,
        Verdict::Clean,
        "c'est precisement le mode d'echec silencieux que le watcher existe pour supprimer"
    );
}

/// **Pas de double comptage.** L'echo d'une ecriture admise est ignore.
///
/// Le registre ecrit lui-meme, donc le watcher voit aussi ses propres ecritures. Sans
/// deduplication, chaque admission produirait deux lines de journal et deux numeros de
/// sequence — et le file changerait d'auteur pour devenir « hors-bande » juste apres
/// avoir ete correctement attribue.
#[tokio::test]
async fn the_echo_of_an_admitted_write_is_ignored() {
    let h = Harness::new();
    let a = h.session("solo").await;

    h.registry.admit(a, "auth.rs", "v1").await.unwrap();
    let seq_apres_admission = h.registry.snapshot().await.unwrap().seq;

    // Le watcher voit l'ecriture que le registre vient de faire, avec la meme empreinte.
    h.registry
        .observe_external_write("auth.rs", ContentHash::of("v1"))
        .await
        .unwrap();

    let snapshot = h.registry.snapshot().await.unwrap();
    assert_eq!(
        snapshot.seq, seq_apres_admission,
        "l'echo ne consomme pas de numero de sequence"
    );
    let file = snapshot
        .files
        .iter()
        .find(|f| f.path.ends_with("auth.rs"))
        .expect("tracked");
    assert_eq!(
        file.last_writer, a,
        "l'echo ne doit PAS voler la provenance a la session qui a ecrit"
    );

    h.journal.flush().await.unwrap();
    let writes = h.journal.writes_for_project(h.project).await.unwrap();
    assert_eq!(writes.len(), 1, "une seule line : {writes:?}");
    assert_eq!(writes[0].origin, WriteOrigin::Admitted);
}

/// Une ecriture hors-bande **a l'identique** ne perime rien : le monde n'a pas change.
///
/// Cas reel : un formatter qui reecrit un file deja conforme.
#[tokio::test]
async fn an_out_of_band_write_of_identical_content_stales_nothing() {
    let h = Harness::new();
    let a = h.session("lecteur").await;

    h.registry
        .record_read(a, "auth.rs", "inchange", ReadKind::FullFile)
        .await
        .unwrap();
    h.registry
        .observe_external_write("auth.rs", ContentHash::of("inchange"))
        .await
        .unwrap();

    assert_eq!(
        h.registry.admit(a, "handlers.rs", "x").await.unwrap(),
        Verdict::Clean,
        "meme empreinte : rien n'a change"
    );
}

/// Le journal distingue les deux origines, et une ecriture observee **n'a pas de verdict**.
#[tokio::test]
async fn the_journal_tells_an_observed_write_from_an_admitted_one() {
    let h = Harness::new();
    let a = h.session("solo").await;

    h.registry.admit(a, "admise.rs", "content").await.unwrap();
    let hash = write_out_of_band(&h, "observee.rs", "content externe");
    h.registry
        .observe_external_write("observee.rs", hash)
        .await
        .unwrap();
    h.journal.flush().await.unwrap();

    let writes = h.journal.writes_for_project(h.project).await.unwrap();
    assert_eq!(writes.len(), 2);

    let admise = writes
        .iter()
        .find(|w| w.path.ends_with("admise.rs"))
        .expect("admise");
    assert_eq!(admise.origin, WriteOrigin::Admitted);
    assert_eq!(admise.verdict.as_deref(), Some("clean"));
    assert_eq!(admise.session, a);

    let observee = writes
        .iter()
        .find(|w| w.path.ends_with("observee.rs"))
        .expect("observee");
    assert_eq!(observee.origin, WriteOrigin::Observed);
    assert_eq!(
        observee.verdict, None,
        "personne n'a admis cette ecriture : mettre un verdict serait un mensonge"
    );
    assert_eq!(observee.session, SessionId::EXTERNAL);
    assert_eq!(observee.session_name, "hors-bande");
}

/// L'observation consomme un numero de sequence : l'ordre total du projet reste total.
#[tokio::test]
async fn an_observation_takes_its_own_place_in_the_sequence() {
    let h = Harness::new();
    let a = h.session("solo").await;

    h.registry.admit(a, "un.rs", "1").await.unwrap();
    let hash = write_out_of_band(&h, "deux.rs", "2");
    h.registry
        .observe_external_write("deux.rs", hash)
        .await
        .unwrap();
    h.registry.admit(a, "trois.rs", "3").await.unwrap();

    assert_eq!(h.registry.snapshot().await.unwrap().seq.get(), 3);
    h.journal.flush().await.unwrap();
    let writes = h.journal.writes_for_project(h.project).await.unwrap();
    let seqs: Vec<u64> = writes.iter().map(|w| w.seq.get()).collect();
    assert_eq!(
        seqs,
        vec![1, 2, 3],
        "la sequence reste contigue toutes origines confondues"
    );
}

/// Une observation hors du projet est ignoree : le registre ne suit que son arbre.
#[tokio::test]
async fn an_observation_outside_the_project_is_ignored() {
    let h = Harness::new();
    let dehors = std::env::temp_dir().join("trame-observation-hors-projet.txt");

    h.registry
        .observe_external_write(&dehors, ContentHash::of("x"))
        .await
        .unwrap();

    let snapshot = h.registry.snapshot().await.unwrap();
    assert!(snapshot.files.is_empty(), "{:?}", snapshot.files);
    assert_eq!(snapshot.seq.get(), 0, "aucun numero de sequence consomme");
}
