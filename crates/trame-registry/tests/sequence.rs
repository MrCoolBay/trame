//! Le compteur de sequence et le comportement sous concurrence reelle.
//!
//! Ici on lance vraiment en parallele, parce que le parallelisme *est* le sujet. Les
//! assertions portent donc sur des **invariants** — unicite, contiguite, ordre total —
//! et jamais sur un ordre d'arrivee precis, que rien ne garantit.

mod common;

use common::Harness;
use tokio::task::JoinSet;
use trame_core::Seq;
use trame_registry::ReadKind;

/// La sequence commence a 1 et avance d'une unite par ecriture admise.
#[tokio::test]
async fn the_sequence_starts_at_one_and_advances_by_one() {
    let h = Harness::new();
    let a = h.session("a").await;

    h.registry.admit(a, "f1.rs", "x").await.unwrap();
    let snapshot = h.registry.snapshot().await.unwrap();
    assert_eq!(
        snapshot.seq,
        Seq::FIRST,
        "la premiere ecriture porte le numero 1"
    );

    h.registry.admit(a, "f2.rs", "x").await.unwrap();
    h.registry.admit(a, "f3.rs", "x").await.unwrap();
    let snapshot = h.registry.snapshot().await.unwrap();
    assert_eq!(snapshot.seq.get(), 3);
}

/// **Sous charge concurrente : aucun numero reutilise, aucun trou.**
///
/// C'est ce que l'acteur donne par construction — un message a la fois, donc un ordre
/// total — et ce test existe pour que la propriete reste vraie si l'implementation
/// change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sequence_numbers_stay_unique_and_gapless_under_load() {
    const ECRITURES: u64 = 60;

    let h = Harness::new();
    let mut sessions = Vec::new();
    for i in 0..3 {
        sessions.push(h.session(&format!("session-{i}")).await);
    }

    let mut set = JoinSet::new();
    for i in 0..ECRITURES {
        let registry = h.registry.clone();
        let session = sessions[(i % 3) as usize];
        set.spawn(async move {
            registry
                .admit(session, format!("f{i}.rs"), "content")
                .await
                .unwrap();
        });
    }
    while set.join_next().await.is_some() {}

    let snapshot = h.registry.snapshot().await.unwrap();
    assert_eq!(
        snapshot.seq.get(),
        ECRITURES,
        "le compteur a compte exactement les admissions"
    );

    let mut seqs: Vec<u64> = snapshot.files.iter().map(|f| f.last_seq.get()).collect();
    seqs.sort_unstable();
    seqs.dedup();
    assert_eq!(
        seqs.len() as u64,
        ECRITURES,
        "aucun numero de sequence n'a ete reutilise"
    );
    assert_eq!(seqs.first().copied(), Some(1));
    assert_eq!(
        seqs.last().copied(),
        Some(ECRITURES),
        "et il n'y a aucun trou"
    );
}

/// Deux registres sont deux projets : leurs compteurs sont independants.
///
/// C'est l'invariant « la sequence est par projet, jamais globale ». Un compteur global
/// serait un point de contention entre projets qui, par construction, ne peuvent pas
/// entrer en collision.
#[tokio::test]
async fn two_projects_have_two_independent_counters() {
    let p1 = Harness::new();
    let p2 = Harness::new();
    assert_ne!(p1.project, p2.project);

    let a = p1.session("a").await;
    let b = p2.session("b").await;

    p1.registry.admit(a, "f.rs", "x").await.unwrap();
    p1.registry.admit(a, "g.rs", "x").await.unwrap();
    p1.registry.admit(a, "h.rs", "x").await.unwrap();
    p2.registry.admit(b, "f.rs", "x").await.unwrap();

    assert_eq!(p1.registry.snapshot().await.unwrap().seq.get(), 3);
    assert_eq!(
        p2.registry.snapshot().await.unwrap().seq.get(),
        1,
        "le second projet part de 1, il n'herite rien du premier"
    );
}

/// Sous concurrence, chaque session voit un verdict coherent avec ce qu'elle a lu.
/// Une seule session lit, N sessions ecrivent : la lectrice doit finir en niveau 1.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_reading_session_is_informed_even_when_writes_race() {
    let h = Harness::new();
    let lectrice = h.session("lectrice").await;
    h.registry
        .record_read(lectrice, "auth.rs", "v0", ReadKind::FullFile)
        .await
        .unwrap();

    let mut set = JoinSet::new();
    for i in 0..8 {
        let registry = h.registry.clone();
        let session = h.session(&format!("ecrivain-{i}")).await;
        set.spawn(async move {
            registry
                .admit(session, "auth.rs", format!("v{i}"))
                .await
                .unwrap();
        });
    }
    while set.join_next().await.is_some() {}

    let verdict = h
        .registry
        .admit(lectrice, "handlers.rs", "x")
        .await
        .unwrap();
    assert_eq!(
        verdict.level(),
        1,
        "auth.rs a bouge huit fois : {verdict:?}"
    );

    let trame_core::Verdict::StaleRead { stale } = &verdict else {
        panic!("attendu StaleRead");
    };
    assert_eq!(
        stale.len(),
        1,
        "un file perime, pas huit : c'est un file, pas un evenement"
    );
    assert_ne!(stale[0].last_writer, lectrice);
}
