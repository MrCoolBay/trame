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
//! 2. **L'empreinte vient du file, jamais du payload** (invariant 10, ADR 0020). Le test le
//!    prouve en mettant dans le payload un content qui n'est PAS celui du disque : si
//!    l'empreinte venait de la, le read-set ne correspondrait a aucun state reel.

use std::path::PathBuf;
use std::sync::Arc;

use trame_core::clock::ManualClock;
use trame_core::{ProjectId, ProjectRoot, SessionId};
use trame_daemon::hooks::{Payload, Response, handle};
use trame_journal::{Journal, spawn_journal};
use trame_registry::{RegistryHandle, spawn_registry};

/// Une limit large : les tests de forme ne sont pas des tests de limit.
const NO_LIMIT: usize = 10_000;

struct System {
    root: PathBuf,
    registry: RegistryHandle,
    _taches: Vec<tokio::task::JoinHandle<()>>,
}

impl System {
    fn new_system() -> Self {
        let id = ProjectId::new();
        let root = std::env::temp_dir().join(format!("trame-hooks-{id}"));
        std::fs::create_dir_all(root.join("sub")).expect("repertoire");
        let clock = Arc::new(ManualClock::new());
        let (journal, j) = spawn_journal(Journal::open_in_memory().expect("journal"));
        let (registry, r) =
            spawn_registry(id, ProjectRoot::new(&root).expect("root"), clock, journal);
        Self {
            root,
            registry,
            _taches: vec![j, r],
        }
    }

    fn root(&self) -> ProjectRoot {
        ProjectRoot::new(&self.root).expect("root")
    }

    fn write_file(&self, relatif: &str, content: &str) {
        let target = self.root.join(relatif);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).expect("repertoire");
        }
        std::fs::write(target, content).expect("fixture");
    }
}

impl Drop for System {
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
/// resolu. Les deux doivent aboutir a la meme key relative dans le registre.
#[tokio::test]
async fn les_deux_formes_de_chemins_donnent_la_meme_cle() {
    let systeme = System::new_system();
    systeme.write_file("auth.rs", "pub fn verify_token() {}\n");
    systeme.write_file("sub/deep.rs", "use verify_token;\n");
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
    let (reponse, bilan) =
        handle(&grep, &systeme.root(), &systeme.registry, session, NO_LIMIT).await;
    assert_eq!(
        reponse,
        Response::Silence,
        "on n'refuse JAMAIS une recherche"
    );
    assert_eq!(
        bilan.recorded,
        vec![PathBuf::from("sub/deep.rs"), PathBuf::from("auth.rs")],
        "les chemins relatifs de Grep doivent etre resolus puis relativises ; skipped : {:?}",
        bilan.skipped
    );

    // 2. `Glob` : chemins ABSOLUS et resolus. La root est peut-etre `/var/...` alors que
    //    l'outil rend `/private/var/...` — c'est exactement ce que `ProjectRoot` absorbe.
    let absolus: Vec<String> = ["auth.rs", "sub/deep.rs"]
        .iter()
        .map(|r| {
            systeme
                .root
                .join(r)
                .canonicalize()
                .expect("path canonique")
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
    let (_, bilan) = handle(&glob, &systeme.root(), &systeme.registry, session, NO_LIMIT).await;
    assert_eq!(
        bilan.recorded,
        vec![PathBuf::from("auth.rs"), PathBuf::from("sub/deep.rs")],
        "les chemins absolus de Glob doivent donner les MEMES cles ; skipped : {:?}",
        bilan.skipped
    );
}

/// ★ L'empreinte vient du file, pas du payload.
///
/// Le payload annonce un content qui n'est pas celui du disque. Si l'empreinte venait de la, le
/// read-set porterait une valeur ne correspondant a **aucun** state reel — et `StaleRead` serait
/// mort en silence (ADR 0020).
#[tokio::test]
async fn l_empreinte_ne_vient_jamais_du_payload() {
    let systeme = System::new_system();
    systeme.write_file("auth.rs", "LE VRAI CONTENU DU DISQUE\n");
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
    let (_, bilan) = handle(&grep, &systeme.root(), &systeme.registry, session, NO_LIMIT).await;
    assert_eq!(bilan.recorded, vec![PathBuf::from("auth.rs")]);

    // Le controle : une ecriture du VRAI content ne doit rien perimer, puisque c'est ce que la
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

/// Un path hors du projet est **ignore et nomme**, jamais enregistre en silence.
#[tokio::test]
async fn un_chemin_hors_du_projet_est_ignore_et_nomme() {
    let systeme = System::new_system();
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
    let (_, bilan) = handle(&glob, &systeme.root(), &systeme.registry, session, NO_LIMIT).await;
    assert!(bilan.recorded.is_empty(), "rien hors du projet");
    assert_eq!(
        bilan.skipped.len(),
        2,
        "et les deux sont NOMMES : {:?}",
        bilan.skipped
    );
    assert!(
        bilan
            .skipped
            .iter()
            .all(|(_, reason)| *reason == "hors du projet")
    );
}

/// Un file disparu entre la recherche et la relecture est un cas normal, et il est dit.
#[tokio::test]
async fn un_fichier_disparu_est_ignore_et_nomme() {
    let systeme = System::new_system();
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
    let (_, bilan) = handle(&grep, &systeme.root(), &systeme.registry, session, NO_LIMIT).await;
    assert!(bilan.recorded.is_empty());
    assert_eq!(
        bilan.skipped,
        vec![("jamais_existe.rs".to_owned(), "illisible")]
    );
}

/// ★ Le mode `content` est un angle mort **compte et affiche**, jamais reconstruit (ADR 0021).
#[tokio::test]
async fn le_mode_content_est_compte_comme_angle_mort() {
    let systeme = System::new_system();
    systeme.write_file("auth.rs", "verify_token\n");
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
    let (_, bilan) = handle(&grep, &systeme.root(), &systeme.registry, session, NO_LIMIT).await;
    assert!(
        bilan.blind_mode,
        "le mode content doit etre SIGNALE comme angle mort"
    );
    assert!(
        bilan.recorded.is_empty(),
        "et rien ne doit etre reconstruit depuis la chaine `content`"
    );
}

/// ★ La limit ne tronque jamais en silence : ce qui est laisse de cote est nomme.
#[tokio::test]
async fn la_borne_nomme_ce_qu_elle_laisse_de_cote() {
    let systeme = System::new_system();
    for index in 0..5 {
        systeme.write_file(&format!("f{index}.rs"), "reason\n");
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
             "tool_input":{{"pattern":"reason","output_mode":"files_with_matches"}},
             "tool_response":{{"mode":"files_with_matches","numFiles":5,
                               "filenames":[{}]}}}}"#,
        noms.join(",")
    ));
    let (_, bilan) = handle(&grep, &systeme.root(), &systeme.registry, session, 2).await;
    assert_eq!(bilan.recorded.len(), 2, "la limit s'applique");
    assert_eq!(
        bilan.skipped.len(),
        3,
        "et le reste est NOMME : {:?}",
        bilan.skipped
    );
    assert!(
        bilan
            .skipped
            .iter()
            .all(|(_, reason)| *reason == "au-dela de la limite")
    );
}

/// ★★ **Le trou lecture n'est PAS ferme par cette plomberie seule.**
///
/// Ce test epingle l'state courant pour qu'il ne soit pas decouvert plus tard : les fichiers
/// rapportes par `Grep` arrivent bien au registre, mais `ReadKind::GrepHit.is_substantial()`
/// rend `false`, donc **ils n'entrent pas dans le read-set** — et aucun `StaleRead` ne peut se
/// declencher dessus.
///
/// # L'arbitrage, et pourquoi il n'est pas tranche ici
///
/// Rendre `GrepHit` substantiel fermerait le trou en une line. Mais un `grep -r` sur un vrai
/// codebase rapporte des dizaines a des centaines de fichiers : chacun entrerait dans le
/// read-set, et **toute** ecriture d'une autre session sur **l'un** d'eux produirait un
/// `StaleRead`. C'est exactement le risque que le filter `ReadKind` existe pour prevenir, et
/// c'est le risque produit numero un — invariant 8, « un outil qui crie au loup est desactive en
/// une semaine ».
///
/// **Le point qui tranche : la manche experimentale mesure des taux de SUCCES, jamais le taux de
/// faux positifs.** Basculer serait donc un pari sur l'invariant 8, pas une decision. Et le
/// booleen force un faux choix : un `Grep` qui rend trois fichiers est une lecture ciblee, un
/// `grep -r` qui en rend trois cents est une exploration — ce ne sont pas la meme chose.
///
/// # Le protocole qui rouvrira la question (ADR 0027)
///
/// La donnee manquante s'accumule en **mode shadow** : les hits `Grep` entrent dans un read-set
/// parallele qui ne participe a aucun verdict, et le registre compte les avis qu'ils **auraient**
/// produits. Trois chiffres a lire, dans `RegistryHandle::shadow_stats` :
///
/// - `shadow_reads` — le denominateur. Sans lui, « douze avis potential » ne veut rien dire.
/// - `potential_notices` — ce que la bascule complete aurait ajoute.
/// - `by_size` — la distribution des tailles de resultat, qui dira **ou** couper.
///   `ShadowStats::potential_notices_if_threshold(n)` repond pour n'importe quel `n`, apres coup. **`n`
///   n'a aucune valeur par defaut** : c'est le parametre de l'experience, pas un reglage.
///
/// Le jour ou quelqu'un basculera ce drapeau, **ce test echouera**, et c'est le but : il forcera
/// a relire ce raisonnement — et a regarder la mesure — plutot qu'a decouvrir le bruit en
/// production.
#[tokio::test]
async fn la_plomberie_seule_ne_ferme_pas_le_trou_lecture() {
    let systeme = System::new_system();
    systeme.write_file("auth.rs", "verify_token\n");
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
    let (_, bilan) = handle(&grep, &systeme.root(), &systeme.registry, session, NO_LIMIT).await;

    assert_eq!(
        bilan.recorded,
        vec![PathBuf::from("auth.rs")],
        "la plomberie transmet bien le file au registre"
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
