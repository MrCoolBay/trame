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
use trame_daemon::SessionPilot;
use trame_journal::{Journal, JournalHandle, spawn_journal};
use trame_registry::{RegistryHandle, spawn_registry};

/// Le cote « agent » du tube.
struct FauxAgent {
    lignes: tokio::io::Lines<BufReader<tokio::io::ReadHalf<DuplexStream>>>,
    ecriture: tokio::io::WriteHalf<DuplexStream>,
}

impl FauxAgent {
    async fn recv(&mut self) -> Option<Value> {
        let ligne = self.lignes.next_line().await.ok()??;
        serde_json::from_str(&ligne).ok()
    }

    async fn send(&mut self, valeur: Value) {
        let payload = format!("{valeur}\n");
        let _ = self.ecriture.write_all(payload.as_bytes()).await;
        let _ = self.ecriture.flush().await;
    }

    /// Emet une requete et attend son acquittement. C'est ce que fait un vrai agent : il
    /// n'enchaine pas ses tool calls avant d'avoir la reponse.
    async fn demander(&mut self, id: u64, methode: &str, params: Value) -> Value {
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
struct Systeme {
    root: PathBuf,
    project: ProjectId,
    registry: RegistryHandle,
    journal: JournalHandle,
    clock: Arc<ManualClock>,
    _joins: Vec<tokio::task::JoinHandle<()>>,
}

impl Systeme {
    async fn nouveau() -> Self {
        let project = ProjectId::new();
        let root = std::env::temp_dir().join(format!("trame-chaine-{project}"));
        std::fs::create_dir_all(&root).expect("repertoire de travail");
        std::fs::write(
            root.join("auth.rs"),
            "pub fn verify_token() -> bool { true }\n",
        )
        .expect("fichier initial");

        let clock = Arc::new(ManualClock::at(chrono::Utc::now()));
        let (journal, j) = spawn_journal(Journal::open_in_memory().expect("journal"));
        let (registry, r) = spawn_registry(
            project,
            ProjectRoot::new(&root).expect("racine"),
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

    /// Branche une session : un backend ACP sur un faux agent, plus son pilote.
    async fn session(&self, nom: &str) -> (AcpBackend, FauxAgent, SessionPilot) {
        let (cote_client, cote_agent) = tokio::io::duplex(64 * 1024);
        let (ar, aw) = tokio::io::split(cote_agent);
        let mut agent = FauxAgent {
            lignes: BufReader::new(ar).lines(),
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
        let pilote = SessionPilot::new(
            session,
            project,
            ProjectRoot::new(&self.root).expect("racine"),
            self.registry.clone(),
            self.clock.clone(),
        );
        pilote.register().await.expect("registre joignable");
        (backend, agent, pilote)
    }

    fn on_disk(&self, relative: &str) -> Option<String> {
        std::fs::read_to_string(self.root.join(relative)).ok()
    }
}

impl Drop for Systeme {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).ok();
    }
}

/// ★★ Le scenario canonique, de bout en bout, transport reel compris.
#[tokio::test]
async fn le_scenario_canonique_de_bout_en_bout() {
    let systeme = Systeme::nouveau().await;
    let (mut backend_a, mut agent_a, mut pilote_a) = systeme.session("ajout-handlers").await;
    let (mut backend_b, mut agent_b, mut pilote_b) = systeme.session("refacto-api").await;
    let mut flux_a = backend_a.events().expect("flux A");

    // ---- 1. A lit auth.rs -------------------------------------------------------
    let attente = tokio::spawn(async move {
        let evenement = flux_a.next().await.expect("FileRead");
        pilote_a.handle(evenement).await;
        (flux_a, pilote_a)
    });
    let reponse = agent_a
        .demander(
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
    let mut flux_b = backend_b.events().expect("flux B");
    let attente = tokio::spawn(async move {
        let evenement = flux_b.next().await.expect("FileWrite B");
        pilote_b.handle(evenement).await;
        pilote_b
    });
    let reponse = agent_b
        .demander(
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
        .demander(
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
        "l'avis nomme le fichier perime : {texte}"
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

/// Un avis ne s'injecte qu'une fois : le repeter a chaque tour serait du bruit, et le
/// bruit fait desactiver la fonctionnalite.
#[tokio::test]
async fn un_avis_n_est_injecte_qu_une_fois() {
    let systeme = Systeme::nouveau().await;
    let (mut backend, mut agent, mut pilote) = systeme.session("solo").await;
    let (mut backend_b2, mut agent_b, mut pilote_b) = systeme.session("autre").await;
    let mut flux = backend.events().expect("flux");
    let mut flux_b = backend_b2.events().expect("flux B");

    // Le pilote lit, l'autre ecrase, le pilote ecrit ailleurs.
    let t = tokio::spawn(async move {
        let e = flux.next().await.expect("read");
        pilote.handle(e).await;
        let e = flux.next().await.expect("write");
        pilote.handle(e).await;
        (flux, pilote)
    });
    agent
        .demander(1, "fs/read_text_file", json!({ "path": "auth.rs" }))
        .await;

    let tb = tokio::spawn(async move {
        let e = flux_b.next().await.expect("write B");
        pilote_b.handle(e).await;
    });
    agent_b
        .demander(
            2,
            "fs/write_text_file",
            json!({ "path": "auth.rs", "content": "v2" }),
        )
        .await;
    tb.await.expect("tache B");

    agent
        .demander(
            3,
            "fs/write_text_file",
            json!({ "path": "x.rs", "content": "y" }),
        )
        .await;
    let (_flux, mut pilote) = t.await.expect("tache");

    assert!(
        pilote.take_notice().is_some(),
        "le premier appel rend l'avis"
    );
    assert!(pilote.take_notice().is_none(), "le second ne le rend plus");
}

/// Un chemin hors du projet est refuse a l'agent, avec un motif, et rien n'est ecrit.
#[tokio::test]
async fn une_ecriture_hors_projet_est_refusee_a_l_agent() {
    let systeme = Systeme::nouveau().await;
    let (mut backend, mut agent, mut pilote) = systeme.session("solo").await;
    let mut flux = backend.events().expect("flux");

    let cible = std::env::temp_dir().join("trame-hors-projet-daemon.txt");
    let _ = std::fs::remove_file(&cible);

    let t = tokio::spawn(async move {
        let e = flux.next().await.expect("write");
        pilote.handle(e).await;
        pilote
    });
    let reponse = agent
        .demander(
            1,
            "fs/write_text_file",
            json!({ "path": cible.to_string_lossy(), "content": "malveillant" }),
        )
        .await;
    let pilote = t.await.expect("tache");

    assert!(
        reponse.get("error").is_some(),
        "l'agent doit recevoir une erreur : {reponse}"
    );
    assert!(
        !cible.exists(),
        "rien ne doit avoir ete ecrit hors du projet"
    );
    assert_eq!(pilote.activity().refusals.len(), 1);
    assert!(pilote.activity().writes.is_empty());
}

/// ★ **La condition d'attente de chaque tour, rendue explicite et verifiee.**
///
/// La premiere manche experimentale a bloque parce que la condition reelle du tour 1 —
/// « la lecture de A est entree dans le read-set » — n'etait ni verifiee ni visible. Ce
/// test la verifie a chaque etape, et il verifie aussi la fin de tour, qui etait attendue
/// sur une notification inexistante.
#[tokio::test]
async fn les_conditions_d_attente_de_chaque_tour_sont_verifiables() {
    let systeme = Systeme::nouveau().await;
    let (mut backend, mut agent, mut pilote) = systeme.session("lecteur").await;
    let mut flux = backend.events().expect("flux");

    // ── tour 1 : condition = auth.rs entre dans le read-set ──────────────────────
    let t = tokio::spawn(async move {
        let outcome = pilote.run_turn(&mut flux).await;
        (flux, pilote, outcome)
    });

    let reponse = agent
        .demander(1, "fs/read_text_file", json!({ "path": "auth.rs" }))
        .await;
    assert!(
        reponse["result"]["content"]
            .as_str()
            .unwrap_or_default()
            .contains("verify_token"),
        "le client sert la lecture : {reponse}"
    );

    // La fin de tour est la reponse a session/prompt. Ici on n'a pas envoye de prompt,
    // donc on simule le signal comme le ferait l'adaptateur pour un tour deja lance.
    // C'est exactement ce qui manquait : sans ce signal, run_turn n'aurait jamais rendu.
    drop(agent);
    let (_flux, pilote, outcome) = t.await.expect("tache");

    assert_eq!(
        pilote.activity().reads,
        vec![std::path::PathBuf::from("auth.rs")],
        "condition du tour 1 : la lecture doit etre ENREGISTREE, pas seulement servie"
    );
    assert!(
        matches!(
            outcome,
            trame_daemon::TurnOutcome::Failed(_) | trame_daemon::TurnOutcome::StreamClosed
        ),
        "un harness qui disparait termine le tour au lieu de le laisser pendre : {outcome:?}"
    );
}

/// Une lecture servie mais **hors du projet** n'entre pas dans le read-set.
///
/// Sinon un `StaleRead` pourrait porter sur un fichier que le registre ne surveille pas.
#[tokio::test]
async fn une_lecture_hors_projet_n_entre_pas_dans_le_read_set() {
    let systeme = Systeme::nouveau().await;
    let (mut backend, mut agent, mut pilote) = systeme.session("lecteur").await;
    let mut flux = backend.events().expect("flux");

    let dehors = std::env::temp_dir().join("trame-lecture-hors-projet.txt");
    std::fs::write(&dehors, "secret").expect("fichier temoin");

    let t = tokio::spawn(async move {
        let e = flux.next().await.expect("FileRead");
        pilote.handle(e).await;
        pilote
    });
    let reponse = agent
        .demander(
            1,
            "fs/read_text_file",
            json!({ "path": dehors.to_string_lossy() }),
        )
        .await;
    let pilote = t.await.expect("tache");

    // Le contenu est bien servi — refuser la lecture casserait l'agent pour rien — mais
    // il n'entre pas dans le read-set du projet.
    assert!(
        reponse.get("result").is_some(),
        "la lecture est servie : {reponse}"
    );
    assert!(
        pilote.activity().reads.is_empty(),
        "aucune entree de read-set hors du projet : {:?}",
        pilote.activity().reads
    );
    std::fs::remove_file(&dehors).ok();
}
