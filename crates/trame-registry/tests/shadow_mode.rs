// Un test d'integration est un binaire ordinaire : les exemptions de `clippy.toml` ne s'y
// appliquent pas.
#![allow(clippy::expect_used, clippy::unwrap_used)]

//! ★★ Le mode shadow : **il compte, il ne dit rien.**
//!
//! Le trou lecture reste ouvert parce que le fermer serait un pari : la manche experimentale
//! mesure des taux de succes, jamais le taux de **faux positifs** — et c'est la variable du
//! risque produit numero un (ADR 0027).
//!
//! Le mode shadow produit la donnee manquante sans rien risquer. Deux proprietes a verrouiller,
//! et la premiere est la condition de validite de la seconde :
//!
//! 1. **L'shadow ne change aucun verdict.** Une mesure qui modifie ce qu'elle mesure ne mesure
//!    rien. C'est le test le plus important du file.
//! 2. **La distribution des tailles est enregistree**, pour que le seuil se decide apres coup et
//!    pour n'importe quel N — au lieu d'etre choisi a l'intuition.

mod common;

use std::path::PathBuf;

use trame_core::Verdict;
use trame_registry::ReadKind;

/// ★★ **La condition de validite de toute la mesure.**
///
/// Le meme scenario, joue deux fois : une fois avec des lectures d'shadow, une fois sans. Les
/// verdicts doivent etre **identiques**. Si l'ombre pouvait changer un verdict, tous les
/// chiffres qu'elle produit seraient sans valeur.
#[tokio::test]
async fn shadow_mode_changes_no_verdict() {
    let mut verdicts = Vec::new();
    for avec_ombre in [false, true] {
        let systeme = common::Harness::new();
        let (a, b) = (systeme.session("a").await, systeme.session("b").await);

        // A a lu `auth.rs` — mais uniquement par une recherche, donc en shadow.
        if avec_ombre {
            systeme
                .registry
                .record_shadow_read(a, "auth.rs", "verify_token", 1)
                .await
                .expect("registre");
        }

        // B ecrit `auth.rs`.
        let verdict_b = systeme
            .registry
            .admit(b, "auth.rs", "validate_token")
            .await
            .expect("admission");
        // A ecrit ailleurs. Avec un vrai read-set ce serait un `StaleRead` ; en shadow, non.
        let verdict_a = systeme
            .registry
            .admit(a, "handlers.rs", "verify_token()")
            .await
            .expect("admission");
        verdicts.push((verdict_b, verdict_a));
    }

    assert_eq!(
        verdicts[0], verdicts[1],
        "l'ombre a change un verdict : toute mesure qu'elle produit est alors sans valeur"
    );
    assert_eq!(
        verdicts[1].1,
        Verdict::Clean,
        "et le verdict reste Clean : c'est bien le trou lecture, toujours ouvert"
    );
}

/// ★ L'avis potentiel est compte, avec la taille du resultat d'ou il vient.
#[tokio::test]
async fn a_potential_notice_is_counted_with_its_search_size() {
    let systeme = common::Harness::new();
    let (a, b) = (systeme.session("a").await, systeme.session("b").await);

    // Une recherche ciblee : trois fichiers dans le resultat.
    systeme
        .registry
        .record_shadow_read(a, "auth.rs", "verify_token", 3)
        .await
        .expect("registre");

    systeme
        .registry
        .admit(b, "auth.rs", "validate_token")
        .await
        .expect("admission");
    systeme
        .registry
        .admit(a, "handlers.rs", "verify_token()")
        .await
        .expect("admission");

    let stats = systeme.registry.shadow_stats().await.expect("stats");
    assert_eq!(stats.potential_notices, 1, "un avis aurait ete emis");
    assert_eq!(
        stats.by_size.get(&3),
        Some(&1),
        "et il vient d'une recherche a 3 fichiers : {:?}",
        stats.by_size
    );
    assert_eq!(stats.shadow_reads, 1, "le denominateur est compte aussi");
}

/// ★★ La distribution repond pour **n'importe quel** seuil, sans rejouer la mesure.
///
/// C'est ce qui evite de choisir N a l'intuition : on enregistre les tailles, et on demande
/// ensuite « combien d'avis si N = 5 ? ». **N n'a aucune valeur par defaut.**
#[tokio::test]
async fn the_recorded_distribution_answers_for_any_threshold() {
    let systeme = common::Harness::new();
    let a = systeme.session("a").await;
    let b = systeme.session("b").await;

    // Trois lectures d'shadow de tailles differentes : ciblee, moyenne, exploration.
    for (file, taille) in [("target.rs", 2_usize), ("moyen.rs", 12), ("masse.rs", 300)] {
        systeme
            .registry
            .record_shadow_read(a, file, "avant", taille)
            .await
            .expect("registre");
        // B modifie chacun : les trois deviendraient perimes.
        systeme
            .registry
            .admit(b, file, "apres")
            .await
            .expect("admission");
    }
    systeme
        .registry
        .admit(a, "handlers.rs", "x")
        .await
        .expect("admission");

    let stats = systeme.registry.shadow_stats().await.expect("stats");
    assert_eq!(stats.potential_notices, 3);
    // La meme mesure repond pour chaque hypothese de seuil.
    assert_eq!(
        stats.potential_notices_if_threshold(1),
        0,
        "aucune a 1 file"
    );
    assert_eq!(
        stats.potential_notices_if_threshold(2),
        1,
        "la recherche ciblee"
    );
    assert_eq!(
        stats.potential_notices_if_threshold(50),
        2,
        "ciblee + moyenne"
    );
    assert_eq!(
        stats.potential_notices_if_threshold(300),
        3,
        "tout, exploration incluse"
    );
}

/// Un avis que le **vrai** verdict a deja dit n'est pas compte comme potentiel.
///
/// Sinon le compteur doublerait ce qui existe deja, et surestimerait le bruit que la bascule
/// ajouterait — la mesure serait pessimiste sans qu'on le sache.
#[tokio::test]
async fn a_notice_the_real_verdict_already_gave_is_not_counted_twice() {
    let systeme = common::Harness::new();
    let (a, b) = (systeme.session("a").await, systeme.session("b").await);

    // La MEME lecture, dans le read-set reel ET en shadow.
    systeme
        .registry
        .record_read(a, "auth.rs", "verify_token", ReadKind::FullFile)
        .await
        .expect("registre");
    systeme
        .registry
        .record_shadow_read(a, "auth.rs", "verify_token", 1)
        .await
        .expect("registre");

    systeme
        .registry
        .admit(b, "auth.rs", "validate_token")
        .await
        .expect("admission");
    let verdict = systeme
        .registry
        .admit(a, "handlers.rs", "verify_token()")
        .await
        .expect("admission");

    let Verdict::StaleRead { stale } = &verdict else {
        panic!("le read-set reel doit produire un StaleRead : {verdict:?}");
    };
    assert_eq!(stale.len(), 1);
    let stats = systeme.registry.shadow_stats().await.expect("stats");
    assert_eq!(
        stats.potential_notices, 0,
        "cet avis existe DEJA : le compter en potentiel surestimerait le bruit ajoute"
    );
}

/// Une lecture d'shadow expiree ne compte pas, comme une vraie.
#[tokio::test]
async fn an_expired_shadow_read_does_not_count() {
    let systeme = common::Harness::new();
    let (a, b) = (systeme.session("a").await, systeme.session("b").await);

    systeme
        .registry
        .record_shadow_read(a, "auth.rs", "verify_token", 1)
        .await
        .expect("registre");
    systeme
        .registry
        .admit(b, "auth.rs", "validate_token")
        .await
        .expect("admission");

    // Au-dela du TTL, le contexte de l'agent a tourne — meme regle que le read-set reel.
    systeme.clock.advance(chrono::TimeDelta::minutes(11));
    systeme
        .registry
        .admit(a, "handlers.rs", "x")
        .await
        .expect("admission");

    let stats = systeme.registry.shadow_stats().await.expect("stats");
    assert_eq!(
        stats.potential_notices, 0,
        "expiree, donc pas d'avis potentiel"
    );
}

/// Un path hors du projet n'entre pas en shadow non plus.
#[tokio::test]
async fn a_path_outside_the_project_never_enters_the_shadow_read_set() {
    let systeme = common::Harness::new();
    let a = systeme.session("a").await;
    let dehors = PathBuf::from("/etc/passwd");
    systeme
        .registry
        .record_shadow_read(a, dehors, "x", 1)
        .await
        .expect("registre");
    let stats = systeme.registry.shadow_stats().await.expect("stats");
    assert_eq!(
        stats.shadow_reads, 0,
        "hors du projet, rien n'est enregistre"
    );
}
