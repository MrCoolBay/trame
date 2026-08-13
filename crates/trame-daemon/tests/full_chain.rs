// Un test d'integration est un binaire ordinaire : les exemptions `allow-*-in-tests` de
// `clippy.toml` ne s'y appliquent pas.
#![allow(clippy::expect_used)]

//! ★★ **Le test qui valide le produit.**
//!
//! La chaine complete, du protocole JSON-RPC jusqu'a l'avis pose sur le fil :
//!
//! ```text
//! agent A : fs/read_text_file auth.rs   -> RecordRead
//! agent B : fs/write_text_file auth.rs  -> Admit -> Clean, ecrit, journalise
//! agent A : fs/write_text_file handlers -> Admit -> StaleRead, ecrit, journalise
//! agent A : message suivant             -> l'avis est DEVANT le prompt, sur le fil
//! ```
//!
//! Les agents sont scenarises en memoire par un `tokio::io::duplex` : pas de sous-process,
//! pas de reseau, pas d'authentification, pas un jeton consomme, et un resultat
//! deterministe. Mais le transport, lui, est le vrai.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream};
use trame_agent::{AcpBackend, AgentBackend};
use trame_core::clock::{Clock, ManualClock};
use trame_core::{
    BranchName, BranchTarget, Harness, Project, ProjectId, ProjectRoot, Session, SessionId,
    SessionState, Toolchain,
};
use trame_daemon::{Observation, SessionPilot, Transport, observe_channel};
use trame_journal::{Journal, JournalHandle, spawn_journal};
use trame_registry::{RegistryHandle, spawn_registry};

/// Le cote « agent » du tube.
struct FauxAgent {
    lines: tokio::io::Lines<BufReader<tokio::io::ReadHalf<DuplexStream>>>,
    ecriture: tokio::io::WriteHalf<DuplexStream>,
}

impl FauxAgent {
    async fn recv(&mut self) -> Option<Value> {
        let line = self.lines.next_line().await.ok()??;
        serde_json::from_str(&line).ok()
    }

    async fn send(&mut self, message: Value) {
        let payload = format!("{message}\n");
        let _ = self.ecriture.write_all(payload.as_bytes()).await;
        let _ = self.ecriture.flush().await;
    }

    /// Emet une requete et attend son acquittement. C'est ce que fait un vrai agent : il
    /// n'enchaine pas ses tool calls avant d'avoir la reponse.
    async fn ask(&mut self, id: u64, methode: &str, params: Value) -> Value {
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": methode, "params": params }))
            .await;
        loop {
            let msg = self.recv().await.expect("une reponse");
            if msg.get("id") == Some(&json!(id)) && msg.get("method").is_none() {
                return msg;
            }
        }
    }
}

/// Tout le systeme : journal, registre, et deux sessions branchees.
struct System {
    root: PathBuf,
    project: ProjectId,
    registry: RegistryHandle,
    journal: JournalHandle,
    clock: Arc<ManualClock>,
    _joins: Vec<tokio::task::JoinHandle<()>>,
}

impl System {
    async fn new_system() -> Self {
        let project = ProjectId::new();
        let root = std::env::temp_dir().join(format!("trame-chaine-{project}"));
        std::fs::create_dir_all(&root).expect("repertoire de travail");
        std::fs::write(
            root.join("auth.rs"),
            "pub fn verify_token() -> bool { true }\n",
        )
        .expect("file initial");

        let clock = Arc::new(ManualClock::at(chrono::Utc::now()));
        let (journal, j) = spawn_journal(Journal::open_in_memory().expect("journal"));
        let (registry, r) = spawn_registry(
            project,
            ProjectRoot::new(&root).expect("root"),
            clock.clone(),
            journal.clone(),
        );
        Self {
            root,
            project,
            registry,
            journal,
            clock,
            _joins: vec![j, r],
        }
    }

    /// Branche une session : un backend ACP sur un faux agent, plus son pilot.
    async fn session(&self, nom: &str) -> (AcpBackend, FauxAgent, SessionPilot) {
        let (backend, agent, pilot, _rx) = self.session_observee(nom).await;
        (backend, agent, pilot)
    }

    /// Idem, en gardant le canal d'observation que l'interface consommerait.
    ///
    /// Le transport declare est `Acp` parce qu'il l'est reellement : le backend est un
    /// `AcpBackend`, et ses capacites disent `can_intercept_writes`. Annoncer autre chose
    /// serait afficher une garantie qui n'existe pas.
    async fn session_observee(
        &self,
        nom: &str,
    ) -> (
        AcpBackend,
        FauxAgent,
        SessionPilot,
        tokio::sync::mpsc::Receiver<Observation>,
    ) {
        let (cote_client, cote_agent) = tokio::io::duplex(64 * 1024);
        let (ar, aw) = tokio::io::split(cote_agent);
        let mut agent = FauxAgent {
            lines: BufReader::new(ar).lines(),
            ecriture: aw,
        };

        let (cr, cw) = tokio::io::split(cote_client);
        let root = self.root.clone();
        let connexion = tokio::spawn(AcpBackend::connect(cr, cw, root));

        // `initialize` : on repond comme le vrai adaptateur.
        let init = agent.recv().await.expect("initialize");
        assert_eq!(
            init["params"]["clientCapabilities"]["fs"]["writeTextFile"],
            true
        );
        agent
            .send(json!({ "jsonrpc": "2.0", "id": init["id"],
                          "result": { "protocolVersion": 1, "agentInfo": { "name": "faux" } } }))
            .await;
        let mut backend = connexion.await.expect("tache").expect("connexion");

        // `session/new` : la requete part et attend sa reponse, donc le faux agent doit
        // repondre **dans la meme fenetre**. Les enchainer sequentiellement interbloque —
        // chacun attendrait que l'autre commence.
        let repondre = async {
            let requete = agent.recv().await.expect("session/new");
            agent
                .send(json!({ "jsonrpc": "2.0", "id": requete["id"],
                              "result": { "sessionId": nom } }))
                .await;
        };
        let (_, ouverture) = tokio::join!(repondre, backend.new_session());
        assert_eq!(ouverture.expect("session ouverte"), nom);

        let now = self.clock.now();
        let project = Project {
            id: self.project,
            path: self.root.clone(),
            name: "projet".into(),
            toolchain: Toolchain::Cargo,
            added_at: now,
            last_opened_at: Some(now),
        };
        let session = Session {
            id: SessionId::new(),
            project_id: self.project,
            name: nom.to_owned(),
            harness: Harness::ClaudeCode,
            target_branch: BranchTarget::New(BranchName::new("feat/x")),
            work_item: None,
            state: SessionState::Writing,
            created_at: now,
        };
        let (observer, rx) = observe_channel();
        let transport = Transport::from(backend.capabilities());
        assert_eq!(
            transport,
            Transport::Acp,
            "le transport observe se lit sur les capacites du backend"
        );
        let mut pilot = SessionPilot::new(
            session,
            project,
            ProjectRoot::new(&self.root).expect("root"),
            self.registry.clone(),
            self.clock.clone(),
        )
        .observed_by(observer, transport);
        pilot.register().await.expect("registre joignable");
        (backend, agent, pilot, rx)
    }

    fn on_disk(&self, relative: &str) -> Option<String> {
        std::fs::read_to_string(self.root.join(relative)).ok()
    }
}

impl Drop for System {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).ok();
    }
}

/// Vide un canal d'observation sans wait_for.
fn drain(rx: &mut tokio::sync::mpsc::Receiver<Observation>) -> Vec<Observation> {
    let mut vues = Vec::new();
    while let Ok(observation) = rx.try_recv() {
        vues.push(observation);
    }
    vues
}

/// ★★ Le scenario canonique, de bout en bout, transport reel compris —
/// **jusqu'a ce que l'interface en voit**.
///
/// Les verdicts qui arrivent dans le canal d'observation sont ceux que le registre a
/// rendus. Un test qui poserait lui-meme ces observations verifierait sa propre fiction.
///
/// Le canal de B est **volontairement ferme** avant son ecriture : c'est le controle
/// negatif. Une interface fermee ne doit pas pouvoir faire echouer une admission, et ce
/// couplage-la ne se verrait qu'en production.
#[tokio::test]
async fn the_canonical_scenario_end_to_end_through_the_real_transport() {
    let systeme = System::new_system().await;
    let (mut backend_a, mut agent_a, mut pilote_a, mut vues_a) =
        systeme.session_observee("ajout-handlers").await;
    let (mut backend_b, mut agent_b, mut pilote_b, vues_b) =
        systeme.session_observee("refacto-api").await;

    // L'ouverture de session porte le nom et le transport : sans eux, l'interface ne peut
    // ni nommer la session ni dire ce qui est garanti.
    assert!(
        matches!(
            drain(&mut vues_a).first(),
            Some(Observation::SessionOpened { name, transport: Transport::Acp, .. })
                if name == "ajout-handlers"
        ),
        "l'interface doit apprendre l'ouverture de A"
    );
    drop(vues_b); // controle negatif : plus personne n'ecoute B
    let mut flux_a = backend_a.events().expect("feed A");

    // ---- 1. A lit auth.rs -------------------------------------------------------
    let attente = tokio::spawn(async move {
        let evenement = flux_a.next().await.expect("FileRead");
        pilote_a.handle(evenement).await;
        (flux_a, pilote_a)
    });
    let reponse = agent_a
        .ask(
            10,
            "fs/read_text_file",
            json!({ "sessionId": "a", "path": "auth.rs" }),
        )
        .await;
    assert!(
        reponse["result"]["content"]
            .as_str()
            .unwrap_or_default()
            .contains("verify_token"),
        "le client sert la lecture : {reponse}"
    );
    let (mut flux_a, mut pilote_a) = attente.await.expect("tache A");

    // ---- 2. B ecrit auth.rs et renomme la fonction -> Clean ---------------------
    let mut flux_b = backend_b.events().expect("feed B");
    let attente = tokio::spawn(async move {
        let evenement = flux_b.next().await.expect("FileWrite B");
        pilote_b.handle(evenement).await;
        pilote_b
    });
    let reponse = agent_b
        .ask(
            20,
            "fs/write_text_file",
            json!({ "sessionId": "b", "path": "auth.rs",
                    "content": "pub fn validate_token() -> bool { true }\n" }),
        )
        .await;
    assert!(
        reponse.get("error").is_none(),
        "l'ecriture de B doit etre admise : {reponse}"
    );
    let pilote_b = attente.await.expect("tache B");
    assert_eq!(pilote_b.activity().writes.len(), 1);
    assert_eq!(pilote_b.activity().writes[0].1, "clean");
    assert!(
        systeme
            .on_disk("auth.rs")
            .unwrap_or_default()
            .contains("validate_token"),
        "le registre a reellement ecrit le renommage"
    );

    // ---- 3. A ecrit handlers.rs, qui appelle l'ancien nom -> StaleRead ----------
    let attente = tokio::spawn(async move {
        let evenement = flux_a.next().await.expect("FileWrite A");
        pilote_a.handle(evenement).await;
        pilote_a
    });
    let reponse = agent_a
        .ask(
            30,
            "fs/write_text_file",
            json!({ "sessionId": "a", "path": "handlers.rs",
                    "content": "verify_token();\n" }),
        )
        .await;
    assert!(
        reponse.get("error").is_none(),
        "le niveau 1 informe, il ne bloque pas : {reponse}"
    );
    let mut pilote_a = attente.await.expect("tache A");

    assert_eq!(pilote_a.activity().writes.len(), 1);
    assert_eq!(
        pilote_a.activity().writes[0].1,
        "stale_read",
        "A ecrit ailleurs, mais sa lecture d'auth.rs est perimee"
    );
    assert_eq!(
        systeme.on_disk("handlers.rs").as_deref(),
        Some("verify_token();\n")
    );

    // ---- 3 bis. Ce que l'interface en voit --------------------------------------
    let vues = drain(&mut vues_a);
    assert!(
        vues.iter().any(|o| matches!(
            o,
            Observation::Read { path, .. } if path == &PathBuf::from("auth.rs")
        )),
        "la lecture de A doit etre visible : {vues:?}"
    );
    let stale = vues
        .iter()
        .find_map(|o| match o {
            Observation::Write {
                path,
                verdict: trame_core::Verdict::StaleRead { stale },
                ..
            } => Some((path.clone(), stale.clone())),
            _ => None,
        })
        .expect("le canal doit porter le StaleRead rendu par le registre");
    assert_eq!(stale.0, PathBuf::from("handlers.rs"));
    assert_eq!(stale.1.len(), 1);
    assert_eq!(stale.1[0].path, PathBuf::from("auth.rs"));
    assert_eq!(
        stale.1[0].last_writer_name, "refacto-api",
        "l'interface doit pouvoir nommer qui a modifie le file"
    );
    assert!(
        vues.iter().any(|o| matches!(
            o,
            Observation::StateChanged {
                state: SessionState::Writing,
                ..
            }
        )),
        "l'state Writing doit etre visible pendant l'admission : {vues:?}"
    );

    // ---- 4. L'avis part DEVANT le prochain message, sur le fil ------------------
    let envoi = tokio::spawn(async move {
        let mut backend = backend_a;
        pilote_a
            .send(&mut backend, "continue")
            .await
            .expect("envoi");
        (backend, pilote_a)
    });
    let prompt = agent_a.recv().await.expect("session/prompt");
    let (_backend, pilote_a) = envoi.await.expect("tache d'envoi");

    assert_eq!(prompt["method"], "session/prompt");
    let texte = prompt["params"]["prompt"][0]["text"]
        .as_str()
        .expect("texte du prompt");
    assert!(
        texte.contains("auth.rs"),
        "l'avis nomme le file perime : {texte}"
    );
    assert!(
        texte.contains("refacto-api"),
        "l'avis nomme la session : {texte}"
    );
    assert!(
        texte.ends_with("continue"),
        "l'avis PRECEDE le prompt : {texte}"
    );
    assert_eq!(pilote_a.activity().notices.len(), 1);
    assert!(
        drain(&mut vues_a)
            .iter()
            .any(|o| matches!(o, Observation::Notice { text, .. } if text.contains("auth.rs"))),
        "l'avis injecte doit apparaitre dans le feed de l'interface"
    );

    // ---- 5. Le journal porte la chaine auditable -------------------------------
    systeme.journal.flush().await.expect("flush");
    let writes = systeme
        .journal
        .writes_for_project(systeme.project)
        .await
        .expect("writes");
    assert_eq!(writes.len(), 2, "deux ecritures admises");
    assert_eq!(writes[0].path, PathBuf::from("auth.rs"));
    assert_eq!(writes[0].session_name, "refacto-api");
    assert_eq!(writes[0].verdict.as_deref(), Some("clean"));
    assert_eq!(writes[1].path, PathBuf::from("handlers.rs"));
    assert_eq!(writes[1].session_name, "ajout-handlers");
    assert_eq!(writes[1].verdict.as_deref(), Some("stale_read"));
    // Les deux sont passees par l'admission : le journal doit le dire explicitement.
    for write in &writes {
        assert_eq!(write.origin, trame_journal::WriteOrigin::Admitted);
    }
}

/// Un avis ne s'injecte qu'une fois : le repeter a chaque turn serait du bruit, et le
/// bruit fait desactiver la fonctionnalite.
#[tokio::test]
async fn a_notice_is_injected_once_and_not_again() {
    let systeme = System::new_system().await;
    let (mut backend, mut agent, mut pilot) = systeme.session("solo").await;
    let (mut backend_b2, mut agent_b, mut pilote_b) = systeme.session("autre").await;
    let mut feed = backend.events().expect("feed");
    let mut flux_b = backend_b2.events().expect("feed B");

    // Le pilot lit, l'autre ecrase, le pilot ecrit ailleurs.
    let t = tokio::spawn(async move {
        let e = feed.next().await.expect("read");
        pilot.handle(e).await;
        let e = feed.next().await.expect("write");
        pilot.handle(e).await;
        (feed, pilot)
    });
    agent
        .ask(1, "fs/read_text_file", json!({ "path": "auth.rs" }))
        .await;

    let tb = tokio::spawn(async move {
        let e = flux_b.next().await.expect("write B");
        pilote_b.handle(e).await;
    });
    agent_b
        .ask(
            2,
            "fs/write_text_file",
            json!({ "path": "auth.rs", "content": "v2" }),
        )
        .await;
    tb.await.expect("tache B");

    agent
        .ask(
            3,
            "fs/write_text_file",
            json!({ "path": "x.rs", "content": "y" }),
        )
        .await;
    let (_flux, mut pilot) = t.await.expect("tache");

    assert!(
        pilot.take_notice().is_some(),
        "le premier appel rend l'avis"
    );
    assert!(pilot.take_notice().is_none(), "le second ne le rend plus");
}

/// Un path hors du projet est refuse a l'agent, avec un reason, et rien n'est ecrit.
#[tokio::test]
async fn a_write_outside_the_project_is_denied_to_the_agent() {
    let systeme = System::new_system().await;
    let (mut backend, mut agent, mut pilot) = systeme.session("solo").await;
    let mut feed = backend.events().expect("feed");

    let target = std::env::temp_dir().join("trame-hors-projet-daemon.txt");
    let _ = std::fs::remove_file(&target);

    let t = tokio::spawn(async move {
        let e = feed.next().await.expect("write");
        pilot.handle(e).await;
        pilot
    });
    let reponse = agent
        .ask(
            1,
            "fs/write_text_file",
            json!({ "path": target.to_string_lossy(), "content": "malveillant" }),
        )
        .await;
    let pilot = t.await.expect("tache");

    assert!(
        reponse.get("error").is_some(),
        "l'agent doit recevoir une erreur : {reponse}"
    );
    assert!(
        !target.exists(),
        "rien ne doit avoir ete ecrit hors du projet"
    );
    assert_eq!(pilot.activity().refusals.len(), 1);
    assert!(pilot.activity().writes.is_empty());
}

/// ★ **La condition d'attente de chaque turn, rendue explicite et verifiee.**
///
/// La premiere manche experimentale a bloque parce que la condition reelle du turn 1 —
/// « la lecture de A est entree dans le read-set » — n'etait ni verifiee ni visible. Ce
/// test la verifie a chaque etape, et il verifie aussi la fin de turn, qui etait attendue
/// sur une notification inexistante.
#[tokio::test]
async fn each_turn_precondition_is_observable_rather_than_assumed() {
    let systeme = System::new_system().await;
    let (mut backend, mut agent, mut pilot) = systeme.session("lecteur").await;
    let mut feed = backend.events().expect("feed");

    // ── turn 1 : condition = auth.rs entre dans le read-set ──────────────────────
    let t = tokio::spawn(async move {
        let outcome = pilot.run_turn(&mut feed).await;
        (feed, pilot, outcome)
    });

    let reponse = agent
        .ask(1, "fs/read_text_file", json!({ "path": "auth.rs" }))
        .await;
    assert!(
        reponse["result"]["content"]
            .as_str()
            .unwrap_or_default()
            .contains("verify_token"),
        "le client sert la lecture : {reponse}"
    );

    // La fin de turn est la reponse a session/prompt. Ici on n'a pas envoye de prompt,
    // donc on simule le signal comme le ferait l'adaptateur pour un turn deja lance.
    // C'est exactement ce qui manquait : sans ce signal, run_turn n'aurait jamais rendu.
    drop(agent);
    let (_flux, pilot, outcome) = t.await.expect("tache");

    assert_eq!(
        pilot.activity().reads,
        vec![std::path::PathBuf::from("auth.rs")],
        "condition du turn 1 : la lecture doit etre ENREGISTREE, pas seulement servie"
    );
    assert!(
        matches!(
            outcome,
            trame_daemon::TurnOutcome::Failed(_) | trame_daemon::TurnOutcome::StreamClosed
        ),
        "un harness qui disparait termine le turn au lieu de le laisser pendre : {outcome:?}"
    );
}

/// Une lecture servie mais **hors du projet** n'entre pas dans le read-set.
///
/// Sinon un `StaleRead` pourrait porter sur un file que le registre ne surveille pas.
#[tokio::test]
async fn a_read_outside_the_project_never_enters_the_read_set() {
    let systeme = System::new_system().await;
    let (mut backend, mut agent, mut pilot) = systeme.session("lecteur").await;
    let mut feed = backend.events().expect("feed");

    let dehors = std::env::temp_dir().join("trame-lecture-hors-projet.txt");
    std::fs::write(&dehors, "secret").expect("file temoin");

    let t = tokio::spawn(async move {
        let e = feed.next().await.expect("FileRead");
        pilot.handle(e).await;
        pilot
    });
    let reponse = agent
        .ask(
            1,
            "fs/read_text_file",
            json!({ "path": dehors.to_string_lossy() }),
        )
        .await;
    let pilot = t.await.expect("tache");

    // Le content est bien servi — deny la lecture casserait l'agent pour rien — mais
    // il n'entre pas dans le read-set du projet.
    assert!(
        reponse.get("result").is_some(),
        "la lecture est servie : {reponse}"
    );
    assert!(
        pilot.activity().reads.is_empty(),
        "aucune entree de read-set hors du projet : {:?}",
        pilot.activity().reads
    );
    std::fs::remove_file(&dehors).ok();
}
