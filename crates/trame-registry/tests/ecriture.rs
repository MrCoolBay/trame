//! ★ **Le registre ecrit** (ADR 0014).
//!
//! Rendre un verdict en laissant l'appelant ecrire fait reposer l'invariant sur la
//! discipline de chaque site d'appel : ce n'est plus un invariant, c'est une convention.
//! Ces tests verifient qu'il n'existe plus de chemin par lequel une ecriture admise ne
//! serait pas effectuee, ni d'ecriture effectuee hors du projet.

mod common;

use common::Harness;
use trame_core::Verdict;
use trame_registry::{ReadKind, RegistryError};

/// Une admission propre pose reellement le contenu sur le disque.
#[tokio::test]
async fn une_admission_ecrit_le_fichier() {
    let h = Harness::new();
    let a = h.session("solo").await;

    let verdict = h
        .registry
        .admit(a, "src/auth.rs", "fn validate_token() {}")
        .await
        .unwrap();

    assert_eq!(verdict, Verdict::Clean);
    assert_eq!(
        h.on_disk("src/auth.rs").as_deref(),
        Some("fn validate_token() {}"),
        "le registre doit avoir ecrit, pas seulement rendu un verdict"
    );
}

/// Un `StaleRead` **ecrit quand meme**. Rien n'est bloque en v0.1 : le registre observe,
/// journalise et informe.
#[tokio::test]
async fn un_stale_read_ecrit_quand_meme() {
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

    assert_eq!(verdict.level(), 1, "{verdict:?}");
    assert_eq!(
        h.on_disk("handlers.rs").as_deref(),
        Some("verify_token()"),
        "le niveau 1 informe, il ne bloque pas : le fichier doit etre ecrit"
    );
}

/// Les repertoires intermediaires sont crees : un agent ecrit dans des chemins qui
/// n'existent pas encore.
#[tokio::test]
async fn les_repertoires_manquants_sont_crees() {
    let h = Harness::new();
    let a = h.session("solo").await;

    h.registry
        .admit(a, "src/api/v2/routes.rs", "// neuf")
        .await
        .unwrap();

    assert_eq!(
        h.on_disk("src/api/v2/routes.rs").as_deref(),
        Some("// neuf")
    );
}

/// Un chemin hors du projet est **refuse**, et rien n'est ecrit.
///
/// Le registre ne peut rien garantir sur ce qu'il ne voit pas ; une ecriture hors du
/// projet n'a donc aucune raison de passer par lui.
#[tokio::test]
async fn un_chemin_hors_du_projet_est_refuse_et_rien_n_est_ecrit() {
    let h = Harness::new();
    let a = h.session("solo").await;
    let cible = std::env::temp_dir().join("trame-ne-doit-pas-exister.txt");
    let _ = std::fs::remove_file(&cible);

    let erreur = h.registry.admit(a, &cible, "contenu").await.unwrap_err();

    assert!(
        matches!(erreur, RegistryError::PathOutsideProject(_)),
        "obtenu {erreur:?}"
    );
    assert!(
        !cible.exists(),
        "aucun fichier ne doit avoir ete cree hors du projet"
    );
}

/// Une remontee par `..` ne permet pas de sortir du projet.
#[tokio::test]
async fn une_remontee_relative_ne_sort_pas_du_projet() {
    let h = Harness::new();
    let a = h.session("solo").await;

    let erreur = h
        .registry
        .admit(a, "../evade.txt", "contenu")
        .await
        .unwrap_err();
    assert!(
        matches!(erreur, RegistryError::PathOutsideProject(_)),
        "obtenu {erreur:?}"
    );
}

/// **Un refus ne consomme pas d'etat.** Le fichier refuse n'entre ni dans le read-set ni
/// dans le write-set, et ne perime rien pour personne.
#[tokio::test]
async fn un_refus_ne_laisse_aucune_trace_dans_l_etat() {
    let h = Harness::new();
    let a = h.session("solo").await;

    let _ = h.registry.admit(a, "/etc/passwd", "malveillant").await;

    let snapshot = h.registry.snapshot().await.unwrap();
    assert!(
        snapshot.files.is_empty(),
        "aucun fichier suivi : {:?}",
        snapshot.files
    );
    let session = snapshot
        .sessions
        .iter()
        .find(|s| s.session == a)
        .expect("session connue");
    assert!(session.write_set.is_empty());
    assert!(session.read_set.is_empty());
}

/// Les deux formes du meme chemin — telle que l'agent la formule, et telle que le systeme
/// la resout — donnent **la meme cle**.
///
/// C'est le mode d'echec silencieux que `ProjectRoot` existe pour empecher : sans lui,
/// une lecture en `/var/…` et une ecriture en `/private/var/…` ne se rencontreraient
/// jamais, et `StaleRead` cesserait de se declencher sans que rien ne casse.
#[tokio::test]
async fn un_chemin_absolu_non_resolu_designe_le_meme_fichier_qu_un_chemin_relatif() {
    let h = Harness::new();
    let a = h.session("lecteur").await;
    let b = h.session("ecrivain").await;

    // A lit par le chemin absolu **non resolu** (celui que donne env::temp_dir()).
    let absolu = h.root.join("auth.rs");
    h.registry
        .record_read(a, &absolu, "fn verify_token()", ReadKind::FullFile)
        .await
        .unwrap();

    // B ecrit par un chemin relatif. Ce doit etre le meme fichier.
    h.registry
        .admit(b, "auth.rs", "fn validate_token()")
        .await
        .unwrap();

    let verdict = h
        .registry
        .admit(a, "handlers.rs", "verify_token()")
        .await
        .unwrap();
    let Verdict::StaleRead { stale } = &verdict else {
        panic!(
            "attendu StaleRead : les deux formes du chemin doivent designer le meme \
             fichier, obtenu {verdict:?}"
        );
    };
    assert_eq!(stale.len(), 1);
    assert_eq!(
        stale[0].path,
        std::path::PathBuf::from("auth.rs"),
        "la cle est relative"
    );
}

/// La cle journalisee est **relative**, jamais le chemin absolu que l'agent a formule.
///
/// Un chemin absolu casserait au premier deplacement du depot et ferait fuiter
/// l'arborescence personnelle dans un journal cense etre partageable.
#[tokio::test]
async fn le_journal_ne_recoit_que_des_chemins_relatifs() {
    let h = Harness::new();
    let a = h.session("solo").await;
    let absolu = h.root.join("src/auth.rs");

    h.registry
        .record_read(a, &absolu, "v1", ReadKind::FullFile)
        .await
        .unwrap();
    h.registry.admit(a, &absolu, "v2").await.unwrap();
    h.journal.flush().await.unwrap();

    let reads = h.journal.reads_for_session(a).await.unwrap();
    let writes = h.journal.writes_for_project(h.project).await.unwrap();
    assert_eq!(reads[0].path, std::path::PathBuf::from("src/auth.rs"));
    assert_eq!(writes[0].path, std::path::PathBuf::from("src/auth.rs"));
    assert!(!writes[0].path.is_absolute());
}
