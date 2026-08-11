//! ★ **Le test canonique.** La raison d'etre du produit.
//!
//! Si ce fichier casse, ce n'est pas le test qui a un probleme.
//!
//! ```text
//! 1. Session A lit auth.rs
//! 2. Session B ecrit auth.rs        -> Clean
//! 3. Session A ecrit handlers.rs    -> StaleRead { auth.rs, par B }
//! ```
//!
//! **Deux fichiers differents. Il n'y a aucune collision d'ecriture.** Un systeme de
//! verrous par fichier ne verrait rien, et c'est tout l'interet : on ne sait pas *si*
//! le travail de A casse, on sait que A raisonne sur un monde qui n'existe plus.

mod common;

use std::path::PathBuf;

use common::Harness;
use trame_core::Verdict;
use trame_registry::ReadKind;

/// Ce que valide ce test, dans l'ordre exact du scenario.
#[tokio::test]
async fn stale_read_sans_aucune_collision_d_ecriture() {
    let h = Harness::new();
    let a = h.session("ajout-handlers").await;
    let b = h.session("refacto-api").await;

    // 1. Session A lit auth.rs. Lecture substantielle : elle entre dans le read-set.
    h.registry
        .record_read(
            a,
            "auth.rs",
            "fn verify_token() -> bool { true }",
            ReadKind::FullFile,
        )
        .await
        .unwrap();

    // 2. Session B ecrit auth.rs et renomme la fonction.
    //    Verdict Clean : B n'a rien lu qui ait bouge depuis. La validation du read-set
    //    se fait au moment ou l'on ecrit, pas au moment ou quelqu'un d'autre ecrit.
    let verdict_b = h
        .registry
        .admit(b, "auth.rs", "fn validate_token() -> bool { true }")
        .await
        .unwrap();
    assert_eq!(
        verdict_b,
        Verdict::Clean,
        "B n'a lu personne : son ecriture est propre"
    );

    // 3. Session A ecrit handlers.rs, qui appelle verify_token().
    //    Le fichier ecrit n'a rien a voir avec le fichier perime.
    let verdict_a = h
        .registry
        .admit(a, "handlers.rs", "verify_token()")
        .await
        .unwrap();

    let Verdict::StaleRead { stale } = &verdict_a else {
        panic!("attendu StaleRead, obtenu {verdict_a:?}");
    };
    assert_eq!(stale.len(), 1, "un seul fichier du read-set de A a bouge");

    let file = &stale[0];
    assert_eq!(
        file.path,
        PathBuf::from("auth.rs"),
        "c'est auth.rs qui est perime"
    );
    assert_eq!(file.last_writer, b, "et c'est B qui l'a modifie");
    assert_eq!(
        file.last_writer_name, "refacto-api",
        "l'avis doit nommer la session"
    );
    assert!(
        file.read_at <= file.written_at,
        "la lecture precede l'ecriture"
    );

    assert_eq!(verdict_a.level(), 1);
    assert!(
        verdict_a.is_admitted(),
        "le niveau 1 informe, il ne bloque pas"
    );
    assert!(
        verdict_a.needs_notice(),
        "et il declenche une injection de contexte"
    );
}

/// Le pendant du test canonique : l'avis remonte reellement a l'agent.
///
/// Le verdict serait une ligne de journal que personne ne lit si le contributeur de
/// prompt ne savait pas le rendre. C'est le seul chemin qui compte pour le produit.
#[tokio::test]
async fn le_verdict_devient_un_avis_lisible_par_l_agent() {
    use trame_core::prompt::{PromptPipeline, SessionContext, StaleReadNotice};
    use trame_core::{BranchName, BranchTarget, Harness as AgentHarness, Project, Session};
    use trame_core::{SessionState, Toolchain};

    let h = Harness::new();
    let a = h.session("ajout-handlers").await;
    let b = h.session("refacto-api").await;

    h.registry
        .record_read(a, "auth.rs", "fn verify_token()", ReadKind::FullFile)
        .await
        .unwrap();
    h.registry
        .admit(b, "auth.rs", "fn validate_token()")
        .await
        .unwrap();
    let verdict = h
        .registry
        .admit(a, "handlers.rs", "verify_token()")
        .await
        .unwrap();

    let now = h.now();
    let project = Project {
        id: h.project,
        path: PathBuf::from("/tmp/projet"),
        name: "projet".into(),
        toolchain: Toolchain::Cargo,
        added_at: now,
        last_opened_at: Some(now),
    };
    let session = Session {
        id: a,
        project_id: h.project,
        name: "ajout-handlers".into(),
        harness: AgentHarness::ClaudeCode,
        target_branch: BranchTarget::New(BranchName::new("feat/handlers")),
        work_item: None,
        state: SessionState::Writing,
        created_at: now,
    };
    let ctx = SessionContext::new(&session, &project, now)
        .with_last_verdict(&verdict)
        .with_pending_write(std::path::Path::new("handlers.rs"));

    let avis = PromptPipeline::new()
        .with(StaleReadNotice)
        .render(&ctx)
        .expect("un StaleRead doit produire un avis");

    // Structure, pas prose : le message sera itere souvent.
    assert!(avis.contains("auth.rs"), "l'avis nomme le fichier : {avis}");
    assert!(
        avis.contains("refacto-api"),
        "l'avis nomme la session : {avis}"
    );
}

/// Le cas le plus frequent, et celui qui decide si l'outil survit : **le silence**.
///
/// ~95 % du trafic est propre. Un outil qui crie au loup est desactive en une semaine.
#[tokio::test]
async fn deux_sessions_sans_recouvrement_restent_silencieuses() {
    let h = Harness::new();
    let a = h.session("front").await;
    let b = h.session("back").await;

    h.registry
        .record_read(a, "ui/page.rs", "// page", ReadKind::FullFile)
        .await
        .unwrap();
    h.registry
        .record_read(b, "api/route.rs", "// route", ReadKind::FullFile)
        .await
        .unwrap();

    assert_eq!(
        h.registry
            .admit(a, "ui/page.rs", "// page v2")
            .await
            .unwrap(),
        Verdict::Clean
    );
    assert_eq!(
        h.registry
            .admit(b, "api/route.rs", "// route v2")
            .await
            .unwrap(),
        Verdict::Clean
    );
}

/// Une session qui ecrit un fichier qu'elle a elle-meme lu n'est pas perimee.
/// Sinon toute session serait au niveau 1 des sa deuxieme ecriture.
#[tokio::test]
async fn une_session_ne_se_declare_pas_perimee_elle_meme() {
    let h = Harness::new();
    let a = h.session("solo").await;

    h.registry
        .record_read(a, "auth.rs", "v1", ReadKind::FullFile)
        .await
        .unwrap();
    assert_eq!(
        h.registry.admit(a, "auth.rs", "v2").await.unwrap(),
        Verdict::Clean
    );
    // Deuxieme ecriture : son propre changement ne doit pas lui revenir en avis.
    assert_eq!(
        h.registry.admit(a, "autre.rs", "x").await.unwrap(),
        Verdict::Clean
    );
}

/// Une reecriture a l'identique ne perime rien : le monde n'a pas change.
#[tokio::test]
async fn un_contenu_identique_ne_perime_pas_la_lecture() {
    let h = Harness::new();
    let a = h.session("lecteur").await;
    let b = h.session("formatteur").await;

    h.registry
        .record_read(a, "auth.rs", "fn f() {}", ReadKind::FullFile)
        .await
        .unwrap();
    // B reecrit exactement le meme contenu — cas typique d'un formatteur sans effet.
    h.registry.admit(b, "auth.rs", "fn f() {}").await.unwrap();

    assert_eq!(
        h.registry.admit(a, "handlers.rs", "f()").await.unwrap(),
        Verdict::Clean,
        "meme empreinte : rien n'a change, donc rien a signaler"
    );
}
