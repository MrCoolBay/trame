//! Le read-set : ce qui y entre, et combien de temps il y reste.
//!
//! Ces deux reglages sont les seuls cadrans disponibles avant de payer le suivi de
//! hunks. Ils decident du taux de faux positifs, qui est le risque produit numero un.

mod common;

use chrono::TimeDelta;
use common::Harness;
use trame_core::Verdict;
use trame_registry::{READ_SET_TTL, ReadKind};

/// Une lecture expiree ne declenche plus d'avis : au-dela de dix minutes, le contexte
/// de l'agent a tourne de toute facon, et l'avertissement serait du bruit.
///
/// Deterministe : le temps n'avance que sur `advance`, jamais par un `sleep`.
#[tokio::test]
async fn une_lecture_expiree_ne_declenche_plus_d_avis() {
    let h = Harness::new();
    let a = h.session("lent").await;
    let b = h.session("rapide").await;

    h.registry
        .record_read(a, "auth.rs", "v1", ReadKind::FullFile)
        .await
        .unwrap();
    h.registry.admit(b, "auth.rs", "v2").await.unwrap();

    // Onze minutes plus tard, la lecture de A ne compte plus.
    h.clock.advance(TimeDelta::minutes(11));

    assert_eq!(
        h.registry.admit(a, "handlers.rs", "x").await.unwrap(),
        Verdict::Clean,
        "au-dela du TTL, le registre se tait"
    );
}

/// Juste avant l'echeance, en revanche, l'avis est encore pertinent.
/// Les deux cotes de la frontiere sont testes, pas un seul.
#[tokio::test]
async fn une_lecture_juste_avant_l_echeance_compte_encore() {
    let h = Harness::new();
    let a = h.session("lent").await;
    let b = h.session("rapide").await;

    h.registry
        .record_read(a, "auth.rs", "v1", ReadKind::FullFile)
        .await
        .unwrap();
    h.registry.admit(b, "auth.rs", "v2").await.unwrap();

    h.clock.advance(TimeDelta::minutes(9));

    let verdict = h.registry.admit(a, "handlers.rs", "x").await.unwrap();
    assert_eq!(
        verdict.level(),
        1,
        "a 9 min, la lecture est encore fraiche : {verdict:?}"
    );
}

/// Le TTL expose par le crate et celui applique par l'acteur sont le meme.
/// Sans ce test, changer la constante sans changer la logique passerait inapercu.
#[tokio::test]
async fn le_ttl_applique_est_celui_de_la_constante_publique() {
    let ttl = TimeDelta::from_std(READ_SET_TTL).expect("TTL representable");

    // Une seconde avant l'echeance : encore valide.
    let h = Harness::new();
    let a = h.session("a").await;
    let b = h.session("b").await;
    h.registry
        .record_read(a, "auth.rs", "v1", ReadKind::FullFile)
        .await
        .unwrap();
    h.registry.admit(b, "auth.rs", "v2").await.unwrap();
    h.clock.advance(ttl - TimeDelta::seconds(1));
    assert_eq!(
        h.registry.admit(a, "x.rs", "x").await.unwrap().level(),
        1,
        "a TTL - 1 s, la lecture compte"
    );

    // Une seconde apres : expiree.
    let h = Harness::new();
    let a = h.session("a").await;
    let b = h.session("b").await;
    h.registry
        .record_read(a, "auth.rs", "v1", ReadKind::FullFile)
        .await
        .unwrap();
    h.registry.admit(b, "auth.rs", "v2").await.unwrap();
    h.clock.advance(ttl + TimeDelta::seconds(1));
    assert_eq!(
        h.registry.admit(a, "x.rs", "x").await.unwrap(),
        Verdict::Clean,
        "a TTL + 1 s, la lecture est oubliee"
    );
}

/// **Le filtrage.** Les agents lisent enormement : grep, glob, listings. Si tout
/// entrait dans le read-set, il exploserait et tout deviendrait niveau 1 — ce qui
/// reviendrait a desactiver la fonctionnalite en la rendant inutilisable.
///
/// Seule la lecture substantielle — un fichier lu en entier — compte.
#[tokio::test]
async fn seules_les_lectures_substantielles_entrent_dans_le_read_set() {
    for kind in [ReadKind::GrepHit, ReadKind::DirListing, ReadKind::Metadata] {
        let h = Harness::new();
        let a = h.session("chercheur").await;
        let b = h.session("ecrivain").await;

        h.registry
            .record_read(a, "auth.rs", "fn f() {}", kind)
            .await
            .unwrap();
        h.registry.admit(b, "auth.rs", "fn g() {}").await.unwrap();

        assert_eq!(
            h.registry.admit(a, "handlers.rs", "f()").await.unwrap(),
            Verdict::Clean,
            "{kind:?} ne doit pas entrer dans le read-set"
        );
    }
}

/// Corollaire du precedent : le snapshot montre que rien n'a ete retenu.
#[tokio::test]
async fn une_lecture_non_substantielle_laisse_le_read_set_vide() {
    let h = Harness::new();
    let a = h.session("chercheur").await;

    h.registry
        .record_read(a, "auth.rs", "fn f() {}", ReadKind::GrepHit)
        .await
        .unwrap();
    h.registry
        .record_read(a, "db.rs", "fn g() {}", ReadKind::FullFile)
        .await
        .unwrap();

    let snapshot = h.registry.snapshot().await.unwrap();
    let session = snapshot
        .sessions
        .iter()
        .find(|s| s.session == a)
        .expect("la session doit apparaitre dans le snapshot");

    assert_eq!(
        session.read_set,
        vec![std::path::PathBuf::from("db.rs")],
        "seule la lecture complete est retenue"
    );
}

/// Plusieurs fichiers perimes remontent tous, du plus recemment modifie au plus ancien.
/// L'agent doit pouvoir tout relire, pas seulement le premier.
#[tokio::test]
async fn tous_les_fichiers_perimes_remontent_dans_l_avis() {
    let h = Harness::new();
    let a = h.session("lecteur").await;
    let b = h.session("refacto-api").await;
    let c = h.session("migration-sqlx").await;

    h.registry
        .record_read(a, "auth.rs", "v1", ReadKind::FullFile)
        .await
        .unwrap();
    h.registry
        .record_read(a, "db/pool.rs", "v1", ReadKind::FullFile)
        .await
        .unwrap();
    h.registry
        .record_read(a, "intact.rs", "v1", ReadKind::FullFile)
        .await
        .unwrap();

    h.registry.admit(b, "auth.rs", "v2").await.unwrap();
    h.clock.advance(TimeDelta::seconds(30));
    h.registry.admit(c, "db/pool.rs", "v2").await.unwrap();

    let verdict = h.registry.admit(a, "handlers.rs", "x").await.unwrap();
    let Verdict::StaleRead { stale } = &verdict else {
        panic!("attendu StaleRead, obtenu {verdict:?}");
    };

    assert_eq!(
        stale.len(),
        2,
        "intact.rs n'a pas bouge et ne doit pas remonter"
    );
    assert_eq!(
        stale[0].path,
        std::path::PathBuf::from("db/pool.rs"),
        "le plus recent d'abord"
    );
    assert_eq!(stale[1].path, std::path::PathBuf::from("auth.rs"));
}
