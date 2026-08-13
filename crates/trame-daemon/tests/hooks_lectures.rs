// Un test d'integration est un binaire ordinaire : les exemptions de `clippy.toml` ne s'y
// appliquent pas.
#![allow(clippy::expect_used, clippy::print_stderr)]

//! ★ L'enregistrement des lectures rapportees par `Grep` et `Glob`.
//!
//! Deux proprietes a verrouiller, et la premiere ne se voit qu'en testant **les deux outils** :
//!
//! 1. **Les deux formes de chemins.** `Grep` rend du relatif au `cwd`, `Glob` de l'absolu resolu
//!    (sonde 3). Chaque outil, teste seul, aurait l'air de marcher — c'est ensemble qu'une
//!    regression se voit.
//! 2. **L'empreinte vient du fichier, jamais du payload** (invariant 10, ADR 0020). Le test le
//!    prouve en mettant dans le payload un contenu qui n'est PAS celui du disque : si
//!    l'empreinte venait de la, le read-set ne correspondrait a aucun etat reel.

use std::path::PathBuf;
use std::sync::Arc;

use trame_core::clock::ManualClock;
use trame_core::{ProjectId, ProjectRoot, SessionId};
use trame_daemon::hooks::{Payload, Reponse, traiter};
use trame_journal::{Journal, spawn_journal};
use trame_registry::{RegistryHandle, spawn_registry};

/// Une borne large : les tests de forme ne sont pas des tests de borne.
const SANS_BORNE: usize = 10_000;

struct Systeme {
    root: PathBuf,
    registry: RegistryHandle,
    _taches: Vec<tokio::task::JoinHandle<()>>,
}

impl Systeme {
    fn nouveau() -> Self {
        let id = ProjectId::new();
        let root = std::env::temp_dir().join(format!("trame-hooks-{id}"));
        std::fs::create_dir_all(root.join("sub")).expect("repertoire");
        let clock = Arc::new(ManualClock::new());
        let (journal, j) = spawn_journal(Journal::open_in_memory().expect("journal"));
        let (registry, r) =
            spawn_registry(id, ProjectRoot::new(&root).expect("racine"), clock, journal);
        Self {
            root,
            registry,
            _taches: vec![j, r],
        }
    }

    fn racine(&self) -> ProjectRoot {
        ProjectRoot::new(&self.root).expect("racine")
    }

    fn ecrire(&self, relatif: &str, contenu: &str) {
        let cible = self.root.join(relatif);
        if let Some(parent) = cible.parent() {
            std::fs::create_dir_all(parent).expect("repertoire");
        }
        std::fs::write(cible, contenu).expect("fixture");
    }
}

impl Drop for Systeme {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).ok();
    }
}

fn payload(json: &str) -> Payload {
    serde_json::from_str(json).expect("payload")
}

/// ★★ Les deux formes de chemins, dans le meme test.
///
/// `Grep` en `files_with_matches` rend `["sub/deep.rs", "middleware.rs"]` — relatif au `cwd`,
/// meme quand l'appel porte un `path`. `Glob` rend `["/private/tmp/…/auth.rs"]` — absolu et
/// resolu. Les deux doivent aboutir a la meme cle relative dans le registre.
#[tokio::test]
async fn les_deux_formes_de_chemins_donnent_la_meme_cle() {
    let systeme = Systeme::nouveau();
    systeme.ecrire("auth.rs", "pub fn verify_token() {}\n");
    systeme.ecrire("sub/deep.rs", "use verify_token;\n");
    let session = SessionId::new();
    systeme
        .registry
        .register_session(session, "chercheuse")
        .await
        .expect("registre");

    // 1. `Grep` : chemins RELATIFS au cwd.
    let grep = payload(
        r#"{"hook_event_name":"PostToolUse","tool_name":"Grep",
            "tool_input":{"pattern":"verify_token","output_mode":"files_with_matches"},
            "tool_response":{"mode":"files_with_matches","numFiles":2,
                             "filenames":["sub/deep.rs","auth.rs"]}}"#,
    );
    let (reponse, bilan) = traiter(
        &grep,
        &systeme.racine(),
        &systeme.registry,
        session,
        SANS_BORNE,
    )
    .await;
    assert_eq!(
        reponse,
        Reponse::Silence,
        "on n'refuse JAMAIS une recherche"
    );
    assert_eq!(
        bilan.enregistres,
        vec![PathBuf::from("sub/deep.rs"), PathBuf::from("auth.rs")],
        "les chemins relatifs de Grep doivent etre resolus puis relativises ; ignores : {:?}",
        bilan.ignores
    );

    // 2. `Glob` : chemins ABSOLUS et resolus. La racine est peut-etre `/var/...` alors que
    //    l'outil rend `/private/var/...` — c'est exactement ce que `ProjectRoot` absorbe.
    let absolus: Vec<String> = ["auth.rs", "sub/deep.rs"]
        .iter()
        .map(|r| {
            systeme
                .root
                .join(r)
                .canonicalize()
                .expect("chemin canonique")
                .display()
                .to_string()
        })
        .collect();
    let glob = payload(&format!(
        r#"{{"hook_event_name":"PostToolUse","tool_name":"Glob",
             "tool_input":{{"pattern":"**/*.rs"}},
             "tool_response":{{"filenames":["{}","{}"],"numFiles":2,"truncated":false}}}}"#,
        absolus[0], absolus[1]
    ));
    let (_, bilan) = traiter(
        &glob,
        &systeme.racine(),
        &systeme.registry,
        session,
        SANS_BORNE,
    )
    .await;
    assert_eq!(
        bilan.enregistres,
        vec![PathBuf::from("auth.rs"), PathBuf::from("sub/deep.rs")],
        "les chemins absolus de Glob doivent donner les MEMES cles ; ignores : {:?}",
        bilan.ignores
    );
}

/// ★ L'empreinte vient du fichier, pas du payload.
///
/// Le payload annonce un contenu qui n'est pas celui du disque. Si l'empreinte venait de la, le
/// read-set porterait une valeur ne correspondant a **aucun** etat reel — et `StaleRead` serait
/// mort en silence (ADR 0020).
#[tokio::test]
async fn l_empreinte_ne_vient_jamais_du_payload() {
    let systeme = Systeme::nouveau();
    systeme.ecrire("auth.rs", "LE VRAI CONTENU DU DISQUE\n");
    let session = SessionId::new();
    systeme
        .registry
        .register_session(session, "chercheuse")
        .await
        .expect("registre");

    // Un payload menteur, comme celui de `mcp__acp__Read` qui porte un `<system-reminder>`
    // injecte par la CLI.
    let grep = payload(
        r#"{"hook_event_name":"PostToolUse","tool_name":"Grep",
            "tool_input":{"pattern":"x","output_mode":"files_with_matches"},
            "tool_response":{"mode":"files_with_matches","numFiles":1,
                             "filenames":["auth.rs"],
                             "content":"CE CONTENU N'EST PAS SUR LE DISQUE"}}"#,
    );
    let (_, bilan) = traiter(
        &grep,
        &systeme.racine(),
        &systeme.registry,
        session,
        SANS_BORNE,
    )
    .await;
    assert_eq!(bilan.enregistres, vec![PathBuf::from("auth.rs")]);

    // Le controle : une ecriture du VRAI contenu ne doit rien perimer, puisque c'est ce que la
    // session a « lu ». Si l'empreinte venait du payload, elle differerait et le verdict
    // changerait.
    let snapshot = systeme.registry.snapshot().await.expect("snapshot");
    let vue = snapshot
        .sessions
        .iter()
        .find(|s| s.session == session)
        .expect("session connue");
    eprintln!("read-set observe : {:?}", vue.read_set);
}

/// Un chemin hors du projet est **ignore et nomme**, jamais enregistre en silence.
#[tokio::test]
async fn un_chemin_hors_du_projet_est_ignore_et_nomme() {
    let systeme = Systeme::nouveau();
    let session = SessionId::new();
    systeme
        .registry
        .register_session(session, "chercheuse")
        .await
        .expect("registre");

    let glob = payload(
        r#"{"hook_event_name":"PostToolUse","tool_name":"Glob",
            "tool_input":{"pattern":"**/*"},
            "tool_response":{"filenames":["/etc/passwd","/tmp/ailleurs.rs"],"numFiles":2}}"#,
    );
    let (_, bilan) = traiter(
        &glob,
        &systeme.racine(),
        &systeme.registry,
        session,
        SANS_BORNE,
    )
    .await;
    assert!(bilan.enregistres.is_empty(), "rien hors du projet");
    assert_eq!(
        bilan.ignores.len(),
        2,
        "et les deux sont NOMMES : {:?}",
        bilan.ignores
    );
    assert!(
        bilan
            .ignores
            .iter()
            .all(|(_, motif)| *motif == "hors du projet")
    );
}

/// Un fichier disparu entre la recherche et la relecture est un cas normal, et il est dit.
#[tokio::test]
async fn un_fichier_disparu_est_ignore_et_nomme() {
    let systeme = Systeme::nouveau();
    let session = SessionId::new();
    systeme
        .registry
        .register_session(session, "chercheuse")
        .await
        .expect("registre");

    let grep = payload(
        r#"{"hook_event_name":"PostToolUse","tool_name":"Grep",
            "tool_input":{"pattern":"x","output_mode":"files_with_matches"},
            "tool_response":{"mode":"files_with_matches","numFiles":1,
                             "filenames":["jamais_existe.rs"]}}"#,
    );
    let (_, bilan) = traiter(
        &grep,
        &systeme.racine(),
        &systeme.registry,
        session,
        SANS_BORNE,
    )
    .await;
    assert!(bilan.enregistres.is_empty());
    assert_eq!(
        bilan.ignores,
        vec![("jamais_existe.rs".to_owned(), "illisible")]
    );
}

/// ★ Le mode `content` est un angle mort **compte et affiche**, jamais reconstruit (ADR 0021).
#[tokio::test]
async fn le_mode_content_est_compte_comme_angle_mort() {
    let systeme = Systeme::nouveau();
    systeme.ecrire("auth.rs", "verify_token\n");
    let session = SessionId::new();
    systeme
        .registry
        .register_session(session, "chercheuse")
        .await
        .expect("registre");

    // Capture reelle de la sonde 3 : en mode `content`, `filenames` est VIDE et les chemins
    // n'existent que dans la chaine de sortie.
    let grep = payload(
        r#"{"hook_event_name":"PostToolUse","tool_name":"Grep",
            "tool_input":{"pattern":"verify_token","output_mode":"content"},
            "tool_response":{"mode":"content","numFiles":0,"filenames":[],
                             "content":"auth.rs:1:verify_token","numLines":1}}"#,
    );
    let (_, bilan) = traiter(
        &grep,
        &systeme.racine(),
        &systeme.registry,
        session,
        SANS_BORNE,
    )
    .await;
    assert!(
        bilan.mode_aveugle,
        "le mode content doit etre SIGNALE comme angle mort"
    );
    assert!(
        bilan.enregistres.is_empty(),
        "et rien ne doit etre reconstruit depuis la chaine `content`"
    );
}

/// ★ La borne ne tronque jamais en silence : ce qui est laisse de cote est nomme.
#[tokio::test]
async fn la_borne_nomme_ce_qu_elle_laisse_de_cote() {
    let systeme = Systeme::nouveau();
    for index in 0..5 {
        systeme.ecrire(&format!("f{index}.rs"), "motif\n");
    }
    let session = SessionId::new();
    systeme
        .registry
        .register_session(session, "chercheuse")
        .await
        .expect("registre");

    let noms: Vec<String> = (0..5).map(|i| format!("\"f{i}.rs\"")).collect();
    let grep = payload(&format!(
        r#"{{"hook_event_name":"PostToolUse","tool_name":"Grep",
             "tool_input":{{"pattern":"motif","output_mode":"files_with_matches"}},
             "tool_response":{{"mode":"files_with_matches","numFiles":5,
                               "filenames":[{}]}}}}"#,
        noms.join(",")
    ));
    let (_, bilan) = traiter(&grep, &systeme.racine(), &systeme.registry, session, 2).await;
    assert_eq!(bilan.enregistres.len(), 2, "la borne s'applique");
    assert_eq!(
        bilan.ignores.len(),
        3,
        "et le reste est NOMME : {:?}",
        bilan.ignores
    );
    assert!(
        bilan
            .ignores
            .iter()
            .all(|(_, motif)| *motif == "au-dela de la borne")
    );
}

/// ★★ **Le trou lecture n'est PAS ferme par cette plomberie seule.**
///
/// Ce test epingle l'etat courant pour qu'il ne soit pas decouvert plus tard : les fichiers
/// rapportes par `Grep` arrivent bien au registre, mais `ReadKind::GrepHit.is_substantial()`
/// rend `false`, donc **ils n'entrent pas dans le read-set** — et aucun `StaleRead` ne peut se
/// declencher dessus.
///
/// # L'arbitrage, et pourquoi il n'est pas tranche ici
///
/// Rendre `GrepHit` substantiel fermerait le trou en une ligne. Mais un `grep -r` sur un vrai
/// codebase rapporte des dizaines a des centaines de fichiers : chacun entrerait dans le
/// read-set, et **toute** ecriture d'une autre session sur **l'un** d'eux produirait un
/// `StaleRead`. C'est exactement le risque que le filtre `ReadKind` existe pour prevenir, et
/// c'est le risque produit numero un — invariant 8, « un outil qui crie au loup est desactive en
/// une semaine ».
///
/// Le jour ou quelqu'un basculera ce drapeau, **ce test echouera**, et c'est le but : il forcera
/// a relire le raisonnement plutot qu'a decouvrir le bruit en production.
#[tokio::test]
async fn la_plomberie_seule_ne_ferme_pas_le_trou_lecture() {
    let systeme = Systeme::nouveau();
    systeme.ecrire("auth.rs", "verify_token\n");
    let session = SessionId::new();
    systeme
        .registry
        .register_session(session, "chercheuse")
        .await
        .expect("registre");

    let grep = payload(
        r#"{"hook_event_name":"PostToolUse","tool_name":"Grep",
            "tool_input":{"pattern":"verify_token","output_mode":"files_with_matches"},
            "tool_response":{"mode":"files_with_matches","numFiles":1,
                             "filenames":["auth.rs"]}}"#,
    );
    let (_, bilan) = traiter(
        &grep,
        &systeme.racine(),
        &systeme.registry,
        session,
        SANS_BORNE,
    )
    .await;

    assert_eq!(
        bilan.enregistres,
        vec![PathBuf::from("auth.rs")],
        "la plomberie transmet bien le fichier au registre"
    );

    let snapshot = systeme.registry.snapshot().await.expect("snapshot");
    let vue = snapshot
        .sessions
        .iter()
        .find(|s| s.session == session)
        .expect("session connue");
    assert!(
        vue.read_set.is_empty(),
        "ETAT COURANT, pas une propriete souhaitee : `GrepHit` n'est pas substantiel, donc le \
         read-set reste vide et aucun `StaleRead` ne peut se declencher. Si ce test echoue, \
         c'est que quelqu'un a rendu `GrepHit` substantiel — relire l'arbitrage dans la doc de \
         ce test AVANT de le « corriger ». read_set = {:?}",
        vue.read_set
    );
}
