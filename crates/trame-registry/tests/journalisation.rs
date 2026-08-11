//! Le cablage registre → journal.
//!
//! Les tests de `trame-journal` prouvent que la base ecrit. Ceux-ci prouvent que le
//! **registre** l'alimente : sans ce chemin, le verdict serait rendu a l'agent et perdu,
//! et la chaine auditable n'existerait pas.
//!
//! La barriere est `journal.flush()` : la file est FIFO, donc quand sa reponse arrive,
//! tous les messages precedents sont traites. Aucun `sleep`.

mod common;

use common::Harness;
use trame_registry::ReadKind;

/// Chaque admission produit une ligne dans `writes`, avec son verdict et sa sequence.
#[tokio::test]
async fn chaque_admission_est_journalisee_avec_son_verdict() {
    let h = Harness::new();
    let a = h.session("ajout-handlers").await;
    let b = h.session("refacto-api").await;

    h.registry
        .record_read(a, "auth.rs", "v1", ReadKind::FullFile)
        .await
        .unwrap();
    h.registry.admit(b, "auth.rs", "v2").await.unwrap();
    h.registry.admit(a, "handlers.rs", "x").await.unwrap();

    let report = h.journal.flush().await.unwrap();
    assert_eq!(
        report.errors, 0,
        "aucune ecriture au journal ne doit avoir echoue"
    );

    let writes = h.journal.writes_for_project(h.project).await.unwrap();
    assert_eq!(writes.len(), 2, "deux admissions, deux lignes");

    assert_eq!(writes[0].seq.get(), 1);
    assert_eq!(writes[0].session, b);
    assert_eq!(writes[0].path, std::path::PathBuf::from("auth.rs"));
    assert_eq!(writes[0].verdict, "clean");
    assert!(
        writes[0].hash_before.is_none(),
        "premiere ecriture du fichier"
    );

    assert_eq!(writes[1].seq.get(), 2);
    assert_eq!(writes[1].session, a);
    assert_eq!(
        writes[1].verdict, "stale_read",
        "le verdict journalise est celui rendu a l'agent"
    );
}

/// Le nom de la session est **denormalise** dans `writes`.
///
/// Une ligne d'audit doit se lire seule : « qui a ecrit cette ligne » se repond par un
/// SELECT sur une table, sans jointure, et la reponse survit a la disparition de la
/// session du reste du schema.
#[tokio::test]
async fn le_nom_de_session_est_denormalise_dans_writes() {
    let h = Harness::new();
    let a = h.session("refacto-api").await;
    let b = h.session("ajout-handlers").await;

    h.registry.admit(a, "auth.rs", "v1").await.unwrap();
    h.registry.admit(b, "handlers.rs", "v1").await.unwrap();
    h.journal.flush().await.unwrap();

    let writes = h.journal.writes_for_project(h.project).await.unwrap();
    assert_eq!(writes[0].session_name, "refacto-api");
    assert_eq!(writes[1].session_name, "ajout-handlers");
}

/// Une session jamais enregistree laisse quand meme une ligne exploitable : la forme
/// courte de son identifiant, plutot qu'une chaine vide ou une panique.
#[tokio::test]
async fn une_session_anonyme_laisse_un_nom_exploitable() {
    let h = Harness::new();
    let inconnue = trame_core::SessionId::new();

    h.registry.admit(inconnue, "x.rs", "v1").await.unwrap();
    h.journal.flush().await.unwrap();

    let writes = h.journal.writes_for_project(h.project).await.unwrap();
    assert_eq!(writes[0].session_name.len(), 8, "forme courte de l'UUID");
    assert!(inconnue.to_string().starts_with(&writes[0].session_name));
}

/// Les lectures substantielles sont journalisees ; les autres ne le sont pas.
///
/// Le journal doit refleter ce que le registre a **retenu**, sinon mesurer le taux de
/// faux positifs a partir du journal donnerait un chiffre faux.
#[tokio::test]
async fn seules_les_lectures_retenues_sont_journalisees() {
    let h = Harness::new();
    let a = h.session("chercheur").await;

    h.registry
        .record_read(a, "auth.rs", "v1", ReadKind::FullFile)
        .await
        .unwrap();
    h.registry
        .record_read(a, "grep.rs", "v1", ReadKind::GrepHit)
        .await
        .unwrap();
    h.registry
        .record_read(a, "dir", "", ReadKind::DirListing)
        .await
        .unwrap();

    h.journal.flush().await.unwrap();

    let reads = h.journal.reads_for_session(a).await.unwrap();
    assert_eq!(reads.len(), 1, "une seule lecture substantielle");
    assert_eq!(reads[0].path, std::path::PathBuf::from("auth.rs"));
}

/// Le hash d'avant et celui d'apres sont enregistres : c'est ce qui permet de rejouer
/// l'histoire d'un fichier sans avoir garde ses contenus.
#[tokio::test]
async fn les_empreintes_avant_et_apres_sont_enregistrees() {
    let h = Harness::new();
    let a = h.session("a").await;

    h.registry.admit(a, "auth.rs", "v1").await.unwrap();
    h.registry.admit(a, "auth.rs", "v2").await.unwrap();
    h.journal.flush().await.unwrap();

    let writes = h.journal.writes_for_project(h.project).await.unwrap();
    assert_eq!(writes.len(), 2);
    assert!(writes[0].hash_before.is_none(), "creation du fichier");
    assert_eq!(
        writes[1].hash_before,
        Some(writes[0].hash_after),
        "l'empreinte d'avant est l'empreinte d'apres de l'ecriture precedente"
    );
    assert_ne!(writes[1].hash_after, writes[0].hash_after);
}

/// Le compteur de sequence du registre et celui de la base ne peuvent pas diverger :
/// `UNIQUE(project_id, seq)` ferait echouer l'insertion, ce que `flush` rapporterait.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn le_journal_accepte_toutes_les_sequences_sous_charge() {
    let h = Harness::new();
    let a = h.session("a").await;

    let mut set = tokio::task::JoinSet::new();
    for i in 0..40 {
        let registry = h.registry.clone();
        set.spawn(async move {
            registry.admit(a, format!("f{i}.rs"), "x").await.unwrap();
        });
    }
    while set.join_next().await.is_some() {}

    let report = h.journal.flush().await.unwrap();
    assert_eq!(
        report.errors, 0,
        "un numero de sequence duplique serait rejete par la contrainte UNIQUE"
    );
    assert_eq!(
        h.journal.writes_for_project(h.project).await.unwrap().len(),
        40
    );
}
