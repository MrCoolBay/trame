// Cet exemple est un rapport destine a l'oeil humain : `eprintln!` EST son interface.
// C'est la seule exception au `print_stderr` deny du workspace, et elle est locale a ce
// fichier — le code de bibliotheque, lui, passe par `tracing`.
#![allow(clippy::print_stderr)]

//! Verification live du point de controle phase 2.
//!
//! Lance **deux sessions Claude Code reelles** dans un repertoire de travail partage,
//! affiche leurs evenements dans le flux normalise, et tranche la question qui porte tout
//! l'edifice : **l'ecriture est-elle interceptable avant que le disque soit touche ?**
//!
//! Le protocole du test est volontairement brutal : on demande a chaque session d'ecrire
//! un fichier, on intercepte la demande, et **on refuse d'ecrire**. Si le fichier
//! n'existe pas a la fin, l'agent n'a pas pu ecrire lui-meme. C'est la preuve.
//!
//! # A lancer dans un terminal PROPRE
//!
//! Claude Code refuse de demarrer a l'interieur d'une autre session Claude Code, et
//! previent que contourner ce garde-fou peut faire tomber les sessions actives. Ce n'est
//! donc pas a un agent de le neutraliser.
//!
//! ```sh
//! npm install -g @zed-industries/claude-code-acp   # une fois
//! cd /chemin/vers/trame
//! cargo run -p trame-agent --example deux_sessions
//! ```
//!
//! Consomme quelques jetons d'API : deux tours courts.

use std::path::{Path, PathBuf};
use std::time::Duration;

use trame_agent::{AcpBackend, AgentBackend, AgentEvent, AgentEventStream, UserMessage};

/// Ce qu'une session a observe.
#[derive(Debug, Default)]
struct Observations {
    ecritures_interceptees: Vec<(PathBuf, usize)>,
    lectures: Vec<PathBuf>,
    messages: usize,
    erreurs: Vec<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "trame=info".into()),
        )
        .with_writer(std::io::stderr)
        .init();

    let cwd = std::env::temp_dir().join("trame-verif-phase2");
    std::fs::create_dir_all(&cwd)?;
    // On part d'un repertoire propre : les fichiers cibles ne doivent pas exister.
    for name in ["session_a.txt", "session_b.txt"] {
        let _ = std::fs::remove_file(cwd.join(name));
    }
    // Un fichier a lire, pour observer aussi le chemin des lectures.
    std::fs::write(
        cwd.join("auth.rs"),
        "pub fn verify_token() -> bool { true }\n",
    )?;

    eprintln!("repertoire de travail partage : {}", cwd.display());
    eprintln!("les deux sessions y travaillent en meme temps, sans isolation\n");

    let a = lancer("session-A", &cwd, "session_a.txt").await?;
    let b = lancer("session-B", &cwd, "session_b.txt").await?;
    let (a, b) = tokio::join!(a, b);
    let (a, b) = (a?, b?);

    rapport(&cwd, &a, &b);
    Ok(())
}

/// Demarre une session et rend la tache qui la pilote jusqu'au bout de son tour.
async fn lancer(
    nom: &'static str,
    cwd: &Path,
    cible: &'static str,
) -> Result<tokio::task::JoinHandle<Observations>, Box<dyn std::error::Error>> {
    eprintln!("[{nom}] demarrage de l'adaptateur ACP…");
    let mut backend = AcpBackend::spawn_claude_code(cwd.to_path_buf())
        .await
        .map_err(|error| {
            format!(
                "{error}\n\nL'adaptateur est-il installe ?\n  \
             npm install -g @zed-industries/claude-code-acp"
            )
        })?;

    let capacites = backend.capabilities();
    eprintln!(
        "[{nom}] capacites : interception={} injection={} permission={}",
        capacites.can_intercept_writes,
        capacites.can_inject_context,
        capacites.can_request_permission
    );
    assert!(
        !capacites.is_degraded(),
        "un backend ACP ne doit jamais s'annoncer degrade"
    );

    let events = backend.events().ok_or("flux d'evenements deja consomme")?;
    let session = backend.new_session().await?;
    eprintln!("[{nom}] session ACP ouverte : {session}");

    let cwd = cwd.to_path_buf();
    backend
        .send(UserMessage::new(format!(
            "Lis auth.rs, puis cree le fichier {cible} contenant exactement : bonjour"
        )))
        .await?;

    Ok(tokio::spawn(async move {
        let observations = boucle(nom, events, cwd).await;
        let _ = backend.shutdown().await;
        observations
    }))
}

/// La boucle qui consomme le flux normalise. C'est ici que se joue l'interception.
async fn boucle(nom: &'static str, mut events: AgentEventStream, cwd: PathBuf) -> Observations {
    let mut obs = Observations::default();

    loop {
        // Un agent peut reflechir longtemps, mais pas indefiniment : au-dela, on rend la
        // main plutot que de rester pendu.
        let Ok(Some(event)) = tokio::time::timeout(Duration::from_secs(180), events.next()).await
        else {
            eprintln!("[{nom}] fin du flux");
            break;
        };

        match event {
            AgentEvent::Message(text) => {
                obs.messages += 1;
                let extrait: String = text.chars().take(70).collect();
                eprintln!("[{nom}] message : {extrait}");
            }

            AgentEvent::ToolCall { name, .. } => eprintln!("[{nom}] outil : {name}"),

            AgentEvent::FileRead(request) => {
                eprintln!("[{nom}] LECTURE  {}", request.path.display());
                obs.lectures.push(request.path.clone());
                // Dans le vrai systeme, c'est ici que le read-set se remplit.
                match std::fs::read_to_string(cwd.join(&request.path)) {
                    Ok(content) => request.provide(content),
                    Err(error) => request.fail(error.to_string()),
                }
            }

            // ★ LE POINT QUI COMPTE.
            AgentEvent::FileWrite(request) => {
                eprintln!(
                    "[{nom}] ★ ECRITURE INTERCEPTEE  {}  ({} octets)",
                    request.path.display(),
                    request.content.len()
                );
                obs.ecritures_interceptees
                    .push((request.path.clone(), request.content.len()));
                // On REFUSE volontairement : si le fichier n'apparait pas malgre tout,
                // c'est que l'agent ne peut pas ecrire sans passer par nous.
                eprintln!("[{nom}]   refus volontaire, rien n'est ecrit sur le disque");
                request.refuse("verification Trame : ecriture volontairement refusee");
            }

            AgentEvent::PermissionRequest(request) => {
                let choix = request.first_allow().map(|option| option.id.clone());
                eprintln!("[{nom}] permission « {} » -> {choix:?}", request.title);
                match choix {
                    Some(id) => request.choose(id),
                    None => request.cancel(),
                }
            }

            AgentEvent::Done => {
                eprintln!("[{nom}] tour termine");
                break;
            }

            AgentEvent::Error(error) => {
                eprintln!("[{nom}] erreur : {error}");
                obs.erreurs.push(error);
                break;
            }

            // `AgentEvent` est `#[non_exhaustive]` : une variante ajoutee plus tard ne
            // doit pas casser les consommateurs. Le prix est ce bras, et il est correct.
            autre => eprintln!("[{nom}] evenement non traite : {autre:?}"),
        }
    }
    obs
}

fn rapport(cwd: &Path, a: &Observations, b: &Observations) {
    eprintln!("\n================ POINT DE CONTROLE PHASE 2 ================");
    for (nom, obs) in [("session-A", a), ("session-B", b)] {
        eprintln!(
            "{nom} : {} ecriture(s) interceptee(s), {} lecture(s), {} message(s), {} erreur(s)",
            obs.ecritures_interceptees.len(),
            obs.lectures.len(),
            obs.messages,
            obs.erreurs.len()
        );
    }

    let interceptees = a.ecritures_interceptees.len() + b.ecritures_interceptees.len();
    let sur_disque: Vec<_> = ["session_a.txt", "session_b.txt"]
        .into_iter()
        .filter(|name| cwd.join(name).exists())
        .collect();

    eprintln!("\necritures interceptees      : {interceptees}");
    eprintln!("fichiers presents sur disque : {sur_disque:?}");
    eprintln!();

    if interceptees > 0 && sur_disque.is_empty() {
        eprintln!("=> INTERCEPTION AVANT DISQUE : CONFIRMEE.");
        eprintln!("   Les agents ont demande a ecrire, nous avons refuse, rien n'a ete ecrit.");
        eprintln!("   L'ADR 0016 est valide sur son dernier point.");
    } else if interceptees == 0 && !sur_disque.is_empty() {
        eprintln!("=> ECHEC. Les agents ont ecrit sans passer par nous.");
        eprintln!("   C'est un probleme de these produit, pas de transport :");
        eprintln!("   ne pas contourner par un watcher, en parler d'abord.");
    } else if interceptees > 0 && !sur_disque.is_empty() {
        eprintln!("=> PARTIEL. Des ecritures passent par nous, d'autres non.");
        eprintln!("   Chercher par quel outil : voir les trous nommes de l'ADR 0016.");
    } else {
        eprintln!("=> INCONCLUSIF : aucune ecriture tentee. Les agents ont-ils repondu ?");
        eprintln!("   Verifier l'authentification (`claude /login`) et relancer.");
    }
}
