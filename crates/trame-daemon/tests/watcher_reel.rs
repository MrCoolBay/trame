// Un test d'integration est un binaire ordinaire : les exemptions de `clippy.toml` ne s'y
// appliquent pas.
#![allow(clippy::expect_used, clippy::print_stderr)]
// ★ macOS UNIQUEMENT, et ce n'est pas une restriction de portabilite : c'est une condition
// de validite.
//
// `notify::recommended_watcher` choisit le backend de la plateforme — FSEvents sur macOS,
// **inotify** sur Linux. Sans ce `cfg`, un job de CI Linux ferait passer au vert un fichier
// dont le titre dit « FSEvents, en vrai » en mesurant tout autre chose. Un test qui passe
// doit mesurer ce qu'il pretend mesurer ; sinon il produit une assurance fausse, ce qui est
// pire que pas de test.
//
// Corollaire assume : ces tests ne tournent que dans le job `macos` de la CI, qui est manuel.
#![cfg(target_os = "macos")]

//! ★★ **Le watcher FSEvents, en vrai, sur un vrai `sed -i`.**
//!
//! Les tests de `trame-registry` verifient la logique d'observation avec un message envoye a
//! la main. Ceux-ci verifient que **FSEvents remonte reellement l'evenement** et que la
//! chaine complete tient : shell → FSEvents → filtre → registre → `StaleRead`.
//!
//! # Ces tests attendent un evenement du systeme
//!
//! On ne peut pas les rendre deterministes avec une horloge injectee : FSEvents est un
//! service du systeme, il notifie quand il notifie. On attend donc **une condition**, par
//! interrogation, avec un plafond — jamais un `sleep` fixe qui serait a la fois trop long et
//! parfois trop court.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use trame_core::clock::SystemClock;
use trame_core::{ProjectId, ProjectRoot, SessionId, Verdict};
use trame_daemon::Observation;
use trame_journal::{Journal, spawn_journal};
use trame_registry::{ReadKind, RegistryHandle, spawn_registry};

/// Attend qu'une condition devienne vraie, ou abandonne.
///
/// Interrogation courte plutot que `sleep` unique : le test se termine des que la condition
/// est remplie, et il echoue avec un message utile si elle ne l'est jamais.
async fn attendre<F>(quoi: &str, mut condition: F) -> bool
where
    F: AsyncCondition,
{
    let limite = Duration::from_secs(10);
    let debut = std::time::Instant::now();
    while debut.elapsed() < limite {
        if condition.verifier().await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    eprintln!("condition jamais remplie en {limite:?} : {quoi}");
    false
}

/// Petit trait pour pouvoir attendre sur une condition asynchrone.
trait AsyncCondition {
    async fn verifier(&mut self) -> bool;
}

struct SeqAtteinte {
    registry: RegistryHandle,
    minimum: u64,
}

impl AsyncCondition for SeqAtteinte {
    async fn verifier(&mut self) -> bool {
        self.registry
            .snapshot()
            .await
            .is_ok_and(|s| s.seq.get() >= self.minimum)
    }
}

struct Systeme {
    root: std::path::PathBuf,
    registry: RegistryHandle,
    _joins: Vec<tokio::task::JoinHandle<()>>,
    _garde: trame_daemon::WatcherGuard,
}

impl Systeme {
    async fn nouveau(gitignore: &str) -> Self {
        Self::construire(gitignore, None)
    }

    /// Le meme systeme, avec le canal d'observation que l'interface consommerait.
    ///
    /// **Un seul watcher par racine.** Deux watchers sur le meme repertoire se volent le
    /// premier arrive : celui qui perd ne voit plus qu'un echo, puisque l'autre a deja
    /// rattrape l'empreinte. C'est ce qui a fait echouer la premiere version de ce test.
    async fn observe(gitignore: &str) -> (Self, tokio::sync::mpsc::Receiver<Observation>) {
        let (observer, rx) = trame_daemon::observe_channel();
        (Self::construire(gitignore, Some(observer)), rx)
    }

    fn construire(gitignore: &str, observer: Option<trame_daemon::Observer>) -> Self {
        let project = ProjectId::new();
        let root = std::env::temp_dir().join(format!("trame-watcher-{project}"));
        std::fs::create_dir_all(root.join("src")).expect("repertoire");
        std::fs::create_dir_all(root.join("target/debug")).expect("repertoire");
        std::fs::write(root.join(".gitignore"), gitignore).expect("gitignore");
        std::fs::write(root.join("auth.rs"), "pub fn verify_token() {}\n").expect("auth.rs");

        let project_root = ProjectRoot::new(&root).expect("racine");
        let (journal, j) = spawn_journal(Journal::open_in_memory().expect("journal"));
        let (registry, r) = spawn_registry(
            project,
            project_root.clone(),
            Arc::new(SystemClock),
            journal,
        );
        let (watch_join, garde) =
            trame_daemon::spawn_watcher_observed(project_root, registry.clone(), observer)
                .expect("watcher demarre");

        Self {
            root,
            registry,
            _joins: vec![j, r, watch_join],
            _garde: garde,
        }
    }

    /// Modifie un fichier **sans passer par le registre**, comme le ferait `sed -i`.
    fn sed(&self, relatif: &str, contenu: &str) {
        let cible = self.root.join(relatif);
        if let Some(parent) = cible.parent() {
            std::fs::create_dir_all(parent).expect("repertoire");
        }
        std::fs::write(&cible, contenu).expect("ecriture hors-bande");
    }
}

impl Drop for Systeme {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).ok();
    }
}

/// ★★ Le scenario canonique quand B contourne l'admission.
///
/// A lit `auth.rs`. Une commande shell le modifie. **FSEvents le constate tout seul**, et A
/// obtient quand meme son `StaleRead`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_obtient_son_stale_read_quand_le_watcher_constate_un_sed() {
    let systeme = Systeme::nouveau("").await;
    let a = SessionId::new();
    systeme
        .registry
        .register_session(a, "ajout-handlers")
        .await
        .expect("registre");

    // 1. A lit auth.rs par le chemin normal.
    systeme
        .registry
        .record_read(
            a,
            "auth.rs",
            "pub fn verify_token() {}\n",
            ReadKind::FullFile,
        )
        .await
        .expect("lecture");

    // 2. Le shell contourne. Personne ne previent le registre.
    systeme.sed("auth.rs", "pub fn validate_token() {}\n");

    // 3. On attend que FSEvents ait fait son travail : la sequence doit avancer.
    let vu = attendre(
        "le watcher doit avoir observe l'ecriture hors-bande",
        SeqAtteinte {
            registry: systeme.registry.clone(),
            minimum: 1,
        },
    )
    .await;
    assert!(
        vu,
        "FSEvents n'a rien remonte : le watcher ne sert a rien dans cet etat"
    );

    // 4. A ecrit ailleurs, et doit etre informe.
    let verdict = systeme
        .registry
        .admit(a, "handlers.rs", "verify_token()")
        .await
        .expect("admission");

    let Verdict::StaleRead { stale } = &verdict else {
        panic!("attendu StaleRead, obtenu {verdict:?}");
    };
    assert_eq!(stale[0].path, Path::new("auth.rs"));
    assert_eq!(stale[0].last_writer, SessionId::EXTERNAL);
    assert_eq!(stale[0].last_writer_name, "hors-bande");
}

/// Le bruit de build **n'atteint pas** le registre.
///
/// Sans ce filtre, un `cargo build` noierait le registre et rendrait le journal illisible.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn le_bruit_de_build_n_atteint_pas_le_registre() {
    let systeme = Systeme::nouveau("*.log\n").await;

    // Trois ecritures qui doivent toutes etre filtrees.
    systeme.sed("target/debug/binaire", "bruit de compilation");
    systeme.sed("build.log", "bruit de journalisation");
    systeme.sed("target/debug/deps/objet.o", "bruit");

    // Puis une qui doit passer, pour prouver que le watcher fonctionne bien.
    systeme.sed("src/auth.rs", "pub fn nouveau() {}");

    let vu = attendre(
        "l'ecriture legitime doit etre observee",
        SeqAtteinte {
            registry: systeme.registry.clone(),
            minimum: 1,
        },
    )
    .await;
    assert!(
        vu,
        "le watcher n'a rien vu du tout : le test ne prouverait rien"
    );

    // Laisse le temps a d'eventuels evenements filtres d'arriver — s'ils devaient arriver.
    tokio::time::sleep(Duration::from_millis(300)).await;

    let snapshot = systeme.registry.snapshot().await.expect("snapshot");
    let suivis: Vec<String> = snapshot
        .files
        .iter()
        .map(|f| f.path.display().to_string())
        .collect();

    assert!(
        suivis.iter().any(|p| p.contains("auth.rs")),
        "l'ecriture legitime doit etre suivie : {suivis:?}"
    );
    for bruit in ["target", "build.log"] {
        assert!(
            !suivis.iter().any(|p| p.contains(bruit)),
            "{bruit} ne doit jamais atteindre le registre : {suivis:?}"
        );
    }
}

/// **L'echo des ecritures du registre n'est pas compte deux fois.**
///
/// Le registre ecrit lui-meme, donc FSEvents voit aussi ses ecritures. Le test verifie que
/// le compteur de sequence n'avance pas une seconde fois pour la meme ecriture.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn l_ecriture_du_registre_n_est_pas_comptee_deux_fois() {
    let systeme = Systeme::nouveau("").await;
    let a = SessionId::new();
    systeme
        .registry
        .register_session(a, "solo")
        .await
        .expect("registre");

    systeme
        .registry
        .admit(a, "src/nouveau.rs", "pub fn f() {}")
        .await
        .expect("admission");
    let seq_apres_admission = systeme
        .registry
        .snapshot()
        .await
        .expect("snapshot")
        .seq
        .get();
    assert_eq!(seq_apres_admission, 1);

    // FSEvents va remonter cette ecriture. On laisse largement le temps.
    tokio::time::sleep(Duration::from_millis(500)).await;

    let snapshot = systeme.registry.snapshot().await.expect("snapshot");
    assert_eq!(
        snapshot.seq.get(),
        seq_apres_admission,
        "l'echo d'une ecriture admise ne doit pas consommer de numero de sequence"
    );
    let fichier = snapshot
        .files
        .iter()
        .find(|f| f.path.ends_with("nouveau.rs"))
        .expect("suivi");
    assert_eq!(
        fichier.last_writer, a,
        "l'echo ne doit pas voler la provenance a la session qui a ecrit"
    );
}

/// ★ **Le controle qui manquait a l'interface.** Le registre ecrit lui-meme (ADR 0014), donc
/// FSEvents remonte ses propres ecritures. Elles ne doivent **jamais** atteindre le canal
/// d'observation : les afficher presenterait comme « constate apres coup, sans verdict » une
/// ecriture passee par l'admission avec un verdict.
///
/// Ce test a ete ecrit **apres** avoir vu le defaut a l'ecran, dans un vrai terminal. La
/// version precedente du watcher emettait l'observation sans demander au registre s'il
/// l'avait retenue — et le registre ne le disait pas.
#[tokio::test]
async fn l_echo_d_une_ecriture_admise_n_atteint_pas_l_interface() {
    let (systeme, mut vues) = Systeme::observe("").await;
    let a = SessionId::new();
    systeme
        .registry
        .register_session(a, "solo")
        .await
        .expect("registre");

    // 1. Une ecriture ADMISE. Le registre l'ecrit, FSEvents la voit.
    systeme
        .registry
        .admit(a, "src/admise.rs", "pub fn f() {}")
        .await
        .expect("admission");
    tokio::time::sleep(Duration::from_millis(500)).await;
    let apres_admission: Vec<_> = std::iter::from_fn(|| vues.try_recv().ok()).collect();
    assert!(
        apres_admission.is_empty(),
        "l'echo d'une admission ne doit rien afficher, or : {apres_admission:?}"
    );

    // 2. Une ecriture HORS-BANDE, faite dans le dos du registre. Celle-la doit apparaitre.
    std::fs::write(systeme.root.join("src/hors_bande.rs"), "// sed -i").expect("ecriture");
    tokio::time::sleep(Duration::from_millis(500)).await;
    let apres_hors_bande: Vec<_> = std::iter::from_fn(|| vues.try_recv().ok()).collect();
    assert!(
        apres_hors_bande.iter().any(|o| matches!(
            o,
            Observation::ExternalWrite { path } if path.ends_with("hors_bande.rs")
        )),
        "une vraie ecriture hors-bande doit etre affichee : {apres_hors_bande:?}"
    );
}
