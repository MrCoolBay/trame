// Les exemptions `allow-expect-in-tests` de `clippy.toml` reposent sur `cfg(test)` ou
// `#[test]`, or un test d'integration est un binaire ordinaire et les methodes du faux
// agent ne portent ni l'un ni l'autre. Dans un harnais de test, echouer bruyamment est
// le comportement voulu.
#![allow(clippy::expect_used)]

//! ★ **L'interception avant ecriture.** La piece porteuse de tout l'edifice.
//!
//! Ces tests scenarisent l'agent **en memoire**, par un `tokio::io::duplex`. Pas de
//! sous-process, pas de reseau, pas d'authentification, pas de token consomme — et un
//! resultat deterministe, sans un seul `sleep`.
//!
//! Ce qu'ils prouvent :
//!
//! 1. Le client annonce bien `fs.writeTextFile` a l'initialisation. C'est cette annonce
//!    qui fait desactiver les outils d'ecriture natifs de l'agent.
//! 2. Une requete `fs/write_text_file` remonte comme [`AgentEvent::FileWrite`], et
//!    **aucune reponse ne part** tant que le consommateur n'a pas decide.
//! 3. Un refus remonte a l'agent comme une erreur d'outil.
//! 4. Une requete **abandonnee** refuse par defaut, elle n'admet jamais en silence.

use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream};
use trame_agent::{AcpBackend, AgentBackend, AgentEvent, UserMessage};

/// Le cote « agent » du tube : ce que le faux harness lit et ecrit.
struct FakeAgent {
    lines: tokio::io::Lines<BufReader<tokio::io::ReadHalf<DuplexStream>>>,
    writer: tokio::io::WriteHalf<DuplexStream>,
}

impl FakeAgent {
    /// La prochaine trame recue du client. `None` si le client a ferme.
    async fn recv(&mut self) -> Option<Value> {
        let line = self.lines.next_line().await.ok()??;
        serde_json::from_str(&line).ok()
    }

    async fn send(&mut self, value: Value) {
        let payload = format!("{value}\n");
        let _ = self.writer.write_all(payload.as_bytes()).await;
        let _ = self.writer.flush().await;
    }

    /// Repond a `initialize` comme le fait l'adaptateur reel.
    async fn accept_initialize(&mut self) -> Value {
        let request = self.recv().await.expect("initialize attendu");
        assert_eq!(request["method"], "initialize");
        self.send(json!({
            "jsonrpc": "2.0",
            "id": request["id"],
            "result": {
                "protocolVersion": 1,
                "agentCapabilities": {},
                "agentInfo": { "name": "faux-agent", "version": "0.0.0" }
            }
        }))
        .await;
        request
    }

    async fn accept_new_session(&mut self, session_id: &str) -> Value {
        let request = self.recv().await.expect("session/new attendu");
        assert_eq!(request["method"], "session/new");
        self.send(json!({
            "jsonrpc": "2.0",
            "id": request["id"],
            "result": { "sessionId": session_id }
        }))
        .await;
        request
    }

    /// Emet une demande d'ecriture, comme le ferait l'agent via `fs/write_text_file`.
    async fn ask_write(&mut self, id: u64, path: &str, content: &str) {
        self.send(json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "fs/write_text_file",
            "params": { "sessionId": "s1", "path": path, "content": content }
        }))
        .await;
    }
}

/// Monte un backend connecte a un faux agent, initialise et pret.
async fn harness() -> (AcpBackend, FakeAgent) {
    let (client_side, agent_side) = tokio::io::duplex(64 * 1024);
    let (agent_read, agent_write) = tokio::io::split(agent_side);
    let mut agent = FakeAgent {
        lines: BufReader::new(agent_read).lines(),
        writer: agent_write,
    };

    let (client_read, client_write) = tokio::io::split(client_side);
    // `connect` fait l'initialize : il faut donc repondre en parallele.
    let connect = tokio::spawn(AcpBackend::connect(
        client_read,
        client_write,
        "/tmp/projet",
    ));
    let init = agent.accept_initialize().await;

    // ★ La negociation est ce qui declenche l'interception cote agent.
    assert_eq!(
        init["params"]["clientCapabilities"]["fs"]["writeTextFile"], true,
        "le client DOIT annoncer fs.writeTextFile : c'est cette annonce qui fait \
         desactiver les outils d'ecriture natifs de l'agent"
    );
    assert_eq!(
        init["params"]["clientCapabilities"]["fs"]["readTextFile"],
        true
    );

    let backend = connect
        .await
        .expect("tache de connexion")
        .expect("connexion");
    (backend, agent)
}

/// Ouvre une session. `join!` est indispensable : `new_session` envoie la requete et
/// attend la reponse, `accept_new_session` attend la requete et repond. Les enchainer
/// sequentiellement interbloque — chacun attend que l'autre ait commence.
async fn open_session(backend: &mut AcpBackend, agent: &mut FakeAgent) -> Value {
    let (request, session) = tokio::join!(agent.accept_new_session("s1"), backend.new_session());
    assert_eq!(session.expect("session ouverte"), "s1");
    request
}

/// ★ Le test qui porte la phase 2.
///
/// L'agent demande a ecrire. On recoit la demande, avec son contenu. **Aucune reponse ne
/// part tant qu'on n'a pas decide.** Puis on admet, et l'agent recoit son acquittement.
#[tokio::test]
async fn une_ecriture_est_interceptee_et_rien_ne_part_avant_la_decision() {
    let (mut backend, mut agent) = harness().await;
    let mut events = backend.events().expect("flux d'evenements");
    open_session(&mut backend, &mut agent).await;

    agent
        .ask_write(42, "auth.rs", "fn validate_token() {}")
        .await;

    let event = events.next().await.expect("un evenement");
    let AgentEvent::FileWrite(request) = event else {
        panic!("attendu FileWrite, obtenu {event:?}");
    };
    assert_eq!(request.path, std::path::PathBuf::from("auth.rs"));
    assert_eq!(
        request.content, "fn validate_token() {}",
        "le contenu propose est visible"
    );

    // ★ Le point qui compte : l'agent attend, aucune reponse n'a ete emise.
    let rien = tokio::time::timeout(Duration::from_millis(50), agent.recv()).await;
    assert!(
        rien.is_err(),
        "aucune reponse ne doit partir avant la decision : l'agent doit attendre"
    );

    // On admet — dans le vrai systeme, apres que le registre a ecrit le fichier.
    request.admitted();

    let response = agent
        .recv()
        .await
        .expect("l'acquittement doit partir apres la decision");
    assert_eq!(response["id"], 42);
    assert!(
        response.get("error").is_none(),
        "une admission n'est pas une erreur"
    );
}

/// Un refus remonte a l'agent comme une erreur d'outil, avec son motif. L'agent sait
/// deja quoi faire d'un outil en echec.
#[tokio::test]
async fn un_refus_remonte_a_l_agent_avec_son_motif() {
    let (mut backend, mut agent) = harness().await;
    let mut events = backend.events().expect("flux");
    open_session(&mut backend, &mut agent).await;

    agent.ask_write(7, "hors-projet.rs", "x").await;
    let AgentEvent::FileWrite(request) = events.next().await.unwrap() else {
        panic!("attendu FileWrite");
    };
    request.refuse("chemin hors du repertoire de travail du projet");

    let response = agent.recv().await.expect("une reponse");
    assert_eq!(response["id"], 7);
    let message = response["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("hors du repertoire"),
        "motif transmis : {message}"
    );
}

/// **Refus par defaut.** Une demande abandonnee sans decision ne devient jamais une
/// ecriture admise.
///
/// C'est l'inverse du defaut naif : si on repondait « admis » en tombant, une requete
/// oubliee produirait une ecriture non admise — exactement ce que le produit existe pour
/// empecher.
#[tokio::test]
async fn une_demande_abandonnee_refuse_au_lieu_d_admettre() {
    let (mut backend, mut agent) = harness().await;
    let mut events = backend.events().expect("flux");
    open_session(&mut backend, &mut agent).await;

    agent.ask_write(9, "auth.rs", "x").await;
    let event = events.next().await.unwrap();
    assert!(matches!(event, AgentEvent::FileWrite(_)));
    drop(event); // le consommateur laisse tomber la requete sans decider

    let response = agent
        .recv()
        .await
        .expect("une reponse doit quand meme partir");
    assert_eq!(response["id"], 9);
    assert!(
        response.get("error").is_some(),
        "un abandon doit REFUSER, jamais admettre en silence : {response}"
    );
}

/// Une lecture est servie par le client : c'est lui qui connait le contenu, et c'est ce
/// qui alimentera le read-set du registre.
#[tokio::test]
async fn une_lecture_est_servie_par_le_client() {
    let (mut backend, mut agent) = harness().await;
    let mut events = backend.events().expect("flux");
    open_session(&mut backend, &mut agent).await;

    agent
        .send(json!({
            "jsonrpc": "2.0", "id": 3, "method": "fs/read_text_file",
            "params": { "sessionId": "s1", "path": "auth.rs" }
        }))
        .await;

    let AgentEvent::FileRead(request) = events.next().await.unwrap() else {
        panic!("attendu FileRead");
    };
    assert_eq!(request.path, std::path::PathBuf::from("auth.rs"));
    request.provide("fn verify_token() {}");

    let response = agent.recv().await.unwrap();
    assert_eq!(response["result"]["content"], "fn verify_token() {}");
}

/// Le flux normalise traduit les messages, puis la fin de tour.
///
/// **Ce test encodait une erreur.** Il envoyait un `sessionUpdate` « end_of_turn » et
/// attendait `Done`. Or cette notification **n'existe pas** dans l'adaptateur : les seuls
/// `sessionUpdate` emis sont `agent_message_chunk`, `agent_thought_chunk`,
/// `available_commands_update`, `current_mode_update`, `plan`, `tool_call` et
/// `tool_call_update`. La fin de tour est la **reponse a `session/prompt`**, avec son
/// `stopReason`.
///
/// Le test passait quand meme, parce qu'il fabriquait lui-meme la notification qu'il
/// attendait. C'est le piege des tests qui simulent un tiers : ils verifient la simulation
/// et pas le tiers. Le canari et cette correction existent pour ca.
#[tokio::test]
async fn les_messages_puis_la_fin_de_tour_arrivent_dans_le_flux_normalise() {
    let (mut backend, mut agent) = harness().await;
    let mut events = backend.events().expect("flux");
    open_session(&mut backend, &mut agent).await;

    backend.send(UserMessage::new("bonjour")).await.unwrap();
    let prompt = agent.recv().await.expect("session/prompt");
    assert_eq!(prompt["method"], "session/prompt");
    assert_eq!(prompt["params"]["prompt"][0]["text"], "bonjour");

    agent
        .send(json!({
            "jsonrpc": "2.0", "method": "session/update",
            "params": { "sessionId": "s1", "update": {
                "sessionUpdate": "agent_message_chunk",
                "content": { "type": "text", "text": "je regarde auth.rs" }
            }}
        }))
        .await;
    // La fin de tour, telle que l'adaptateur la signale reellement.
    agent
        .send(json!({ "jsonrpc": "2.0", "id": prompt["id"],
                      "result": { "stopReason": "end_turn" } }))
        .await;

    match events.next().await.unwrap() {
        AgentEvent::Message(text) => assert_eq!(text, "je regarde auth.rs"),
        other => panic!("attendu Message, obtenu {other:?}"),
    }
    assert!(matches!(events.next().await.unwrap(), AgentEvent::Done));
}

/// Le contexte injecte precede le prompt dans ce qui part reellement sur le fil.
///
/// C'est le chemin par lequel l'avis de lecture perimee atteindra l'agent.
#[tokio::test]
async fn le_contexte_injecte_part_bien_sur_le_fil() {
    let (mut backend, mut agent) = harness().await;
    let _events = backend.events();
    open_session(&mut backend, &mut agent).await;

    backend
        .send(UserMessage::new("continue").with_context("[Trame] auth.rs a ete modifie"))
        .await
        .unwrap();

    let prompt = agent.recv().await.unwrap();
    let text = prompt["params"]["prompt"][0]["text"].as_str().unwrap();
    assert!(
        text.starts_with("[Trame] auth.rs a ete modifie"),
        "obtenu : {text}"
    );
    assert!(text.ends_with("continue"));
}

/// `session/new` ferme explicitement les outils qui ecriraient hors du chemin
/// d'admission — `NotebookEdit`, que l'adaptateur ne desactive pas de lui-meme.
#[tokio::test]
async fn la_session_ferme_les_outils_d_ecriture_restants() {
    let (mut backend, mut agent) = harness().await;
    let _events = backend.events();
    let new_session = open_session(&mut backend, &mut agent).await;

    let disallowed = &new_session["params"]["_meta"]["claudeCode"]["options"]["disallowedTools"];
    assert!(
        disallowed
            .as_array()
            .is_some_and(|tools| tools.iter().any(|t| t == "NotebookEdit")),
        "NotebookEdit doit etre ferme explicitement : {disallowed}"
    );
    assert_eq!(new_session["params"]["cwd"], "/tmp/projet");
}

/// Un harness qui meurt est un cas normal, pas une panique : l'erreur remonte dans le
/// flux et le backend cesse proprement.
#[tokio::test]
async fn la_mort_du_harness_remonte_comme_une_erreur() {
    let (mut backend, agent) = harness().await;
    let mut events = backend.events().expect("flux");
    drop(agent); // le sous-process disparait

    let event = events.next().await.expect("une erreur doit remonter");
    assert!(matches!(event, AgentEvent::Error(_)), "obtenu {event:?}");
}

/// ★ **La fin de tour est la reponse a `session/prompt`, pas une notification.**
///
/// Ce test existe parce que l'inverse a ete suppose et que ca a coute une manche
/// experimentale entiere : `AgentEvent::Done` etait attendu sur un `sessionUpdate`
/// « end_of_turn » **qui n'existe pas** dans l'adaptateur. Le consommateur attendait donc
/// un signal qui ne venait jamais.
#[tokio::test]
async fn la_fin_de_tour_arrive_par_la_reponse_au_prompt() {
    let (mut backend, mut agent) = harness().await;
    let mut events = backend.events().expect("flux");
    open_session(&mut backend, &mut agent).await;

    backend.send(UserMessage::new("va")).await.unwrap();
    let prompt = agent.recv().await.expect("session/prompt");
    assert_eq!(prompt["method"], "session/prompt");

    // L'agent repond a `session/prompt`. Aucune notification de fin n'est emise.
    agent
        .send(json!({ "jsonrpc": "2.0", "id": prompt["id"],
                      "result": { "stopReason": "end_turn" } }))
        .await;

    let event = tokio::time::timeout(Duration::from_secs(2), events.next())
        .await
        .expect("Done doit arriver, et sans attendre un end_of_turn qui n'existe pas")
        .expect("un evenement");
    assert!(matches!(event, AgentEvent::Done), "obtenu {event:?}");
}

/// Un tour en echec remonte comme `Error`, pas comme un silence.
#[tokio::test]
async fn un_tour_en_echec_remonte_comme_erreur() {
    let (mut backend, mut agent) = harness().await;
    let mut events = backend.events().expect("flux");
    open_session(&mut backend, &mut agent).await;

    backend.send(UserMessage::new("va")).await.unwrap();
    let prompt = agent.recv().await.expect("session/prompt");
    agent
        .send(json!({ "jsonrpc": "2.0", "id": prompt["id"],
                      "error": { "code": -32603, "message": "boum" } }))
        .await;

    let event = tokio::time::timeout(Duration::from_secs(2), events.next())
        .await
        .expect("une erreur doit remonter")
        .expect("un evenement");
    let AgentEvent::Error(message) = event else {
        panic!("attendu Error, obtenu {event:?}");
    };
    assert!(message.contains("boum"), "{message}");
}

/// `tool_call_update` doit etre traduit, pas seulement `tool_call`.
///
/// L'adaptateur n'emet parfois QUE des `update` : quand une demande de permission a deja
/// fait emettre le `tool_call`, le flux qui suit ne fait que le raffiner. Ne traduire que
/// la forme initiale laissait des appels d'outil totalement invisibles.
#[tokio::test]
async fn tool_call_et_tool_call_update_sont_tous_deux_traduits() {
    let (mut backend, mut agent) = harness().await;
    let mut events = backend.events().expect("flux");
    open_session(&mut backend, &mut agent).await;

    for kind in ["tool_call", "tool_call_update"] {
        agent
            .send(json!({
                "jsonrpc": "2.0", "method": "session/update",
                "params": { "sessionId": "s1", "update": {
                    "sessionUpdate": kind,
                    "title": format!("Read auth.rs ({kind})"),
                    "rawInput": { "file_path": "auth.rs" }
                }}
            }))
            .await;

        let event = tokio::time::timeout(Duration::from_secs(2), events.next())
            .await
            .unwrap_or_else(|_| panic!("{kind} doit etre traduit"))
            .expect("un evenement");
        let AgentEvent::ToolCall { name, input } = event else {
            panic!("attendu ToolCall pour {kind}, obtenu {event:?}");
        };
        assert!(name.contains(kind), "le titre doit remonter : {name}");
        assert_eq!(input["file_path"], "auth.rs");
    }
}

/// Les outils fermes en plus arrivent bien dans `session/new`, fusionnes et non ecrases.
#[tokio::test]
async fn les_outils_fermes_en_plus_arrivent_dans_la_session() {
    let (mut backend, mut agent) = harness().await;
    let _events = backend.events();
    backend.disallow_tools(["Grep", "Glob", "Bash"]);
    let requete = open_session(&mut backend, &mut agent).await;

    let fermes = &requete["params"]["_meta"]["claudeCode"]["options"]["disallowedTools"];
    let fermes: Vec<&str> = fermes
        .as_array()
        .expect("une liste")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    // Ceux de la constante ET ceux ajoutes : la liste s'accumule.
    for attendu in ["NotebookEdit", "Grep", "Glob", "Bash"] {
        assert!(
            fermes.contains(&attendu),
            "{attendu} doit etre ferme : {fermes:?}"
        );
    }
}
