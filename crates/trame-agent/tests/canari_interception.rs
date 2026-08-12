// Voir la note sur `expect_used` dans `interception.rs` : un test d'integration est un
// binaire ordinaire, les exemptions de `clippy.toml` ne s'y appliquent pas.
#![allow(clippy::expect_used, clippy::print_stderr)]

//! ★ **Le canari.** Il surveille le detail d'implementation tiers dont depend l'invariant.
//!
//! # Ce qu'il garde
//!
//! L'interception avant disque ne repose pas sur une garantie du protocole ACP. Elle
//! repose sur un **choix d'implementation, non specifie, de l'adaptateur** : quand le
//! client annonce `fs.writeTextFile`, l'adaptateur retire `Write` et `Edit` des outils de
//! l'agent, qui n'a alors plus que le chemin passant par nous.
//!
//! Ce choix a deja disparu une fois. Le paquet successeur,
//! `@agentclientprotocol/claude-agent-acp` 0.66.0, ne retire plus rien : il passe
//! `--disallowedTools AskUserQuestion` et `--tools default`, donc l'agent garde ses outils
//! natifs et ecrit directement sur le disque. Migrer aurait supprime le produit en
//! silence, sans qu'aucun test existant ne bronche.
//!
//! D'ou ce canari. Il ne verifie pas notre code : **il verifie le leur.**
//!
//! # Comment il fonctionne, sans authentification ni jeton
//!
//! L'adaptateur honore `CLAUDE_CODE_EXECUTABLE`. On l'oriente vers un faux `claude` qui
//! ne fait qu'ecrire ses arguments dans un fichier avant de sortir. La negociation a donc
//! lieu pour de vrai, et on lit la ligne de commande que l'adaptateur *aurait* donnee au
//! vrai binaire.
//!
//! Effet de bord utile : comme le vrai `claude` n'est jamais lance, le garde-fou
//! anti-imbrication de Claude Code ne s'applique pas. Ce canari tourne donc partout, y
//! compris depuis une session Claude Code, et en CI.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

/// Les outils qui doivent etre retires quand on annonce `fs.writeTextFile`.
///
/// S'ils restent disponibles, l'agent peut ecrire sans passer par le registre.
const OUTILS_A_RETIRER: &[&str] = &["Write", "Edit"];

/// La commande de l'adaptateur. Surchargeable, pour tester une version candidate avant
/// de s'y engager.
fn commande_adaptateur() -> String {
    std::env::var("TRAME_ACP_COMMAND").unwrap_or_else(|_| "claude-code-acp".to_owned())
}

fn adaptateur_disponible(commande: &str) -> bool {
    // Un chemin explicite se verifie directement : `which` ne cherche que dans le PATH,
    // et on veut pouvoir viser une version candidate installee ailleurs.
    if commande.contains('/') {
        return Path::new(commande).is_file();
    }
    std::process::Command::new("which")
        .arg(commande)
        .stdout(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// Ecrit un faux `claude` qui capture son `argv` puis sort.
fn faux_claude(dir: &Path) -> (PathBuf, PathBuf) {
    let script = dir.join("faux-claude.sh");
    let capture = dir.join("argv.txt");
    let contenu = format!(
        "#!/bin/sh\nfor a in \"$@\"; do echo \"$a\" >> {}; done\nexit 0\n",
        capture.display()
    );
    std::fs::write(&script, contenu).expect("ecriture du faux claude");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
            .expect("faux claude executable");
    }
    (script, capture)
}

/// Fait la negociation contre le vrai adaptateur et rend l'`argv` qu'il a produit.
async fn argv_negocie(commande: &str, dir: &Path) -> Vec<String> {
    let (script, capture) = faux_claude(dir);

    let mut child = Command::new(commande)
        .current_dir(dir)
        .env("CLAUDE_CODE_EXECUTABLE", &script)
        // Le vrai `claude` n'etant jamais lance, ce garde-fou n'a pas d'objet ici.
        .env_remove("CLAUDECODE")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("l'adaptateur doit demarrer");

    let mut stdin = child.stdin.take().expect("stdin");
    let mut lignes = BufReader::new(child.stdout.take().expect("stdout")).lines();

    async fn envoyer(stdin: &mut tokio::process::ChildStdin, valeur: Value) {
        let payload = format!("{valeur}\n");
        let _ = stdin.write_all(payload.as_bytes()).await;
        let _ = stdin.flush().await;
    }

    // 1. La negociation : c'est elle qu'on met a l'epreuve.
    envoyer(
        &mut stdin,
        json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "protocolVersion": 1,
                "clientCapabilities": {
                    "fs": { "readTextFile": true, "writeTextFile": true },
                    "terminal": false
                }
            }
        }),
    )
    .await;

    // On attend la reponse a `initialize` avant d'ouvrir la session.
    let mut initialise = false;
    while let Ok(Ok(Some(ligne))) =
        tokio::time::timeout(Duration::from_secs(30), lignes.next_line()).await
    {
        if let Ok(msg) = serde_json::from_str::<Value>(&ligne)
            && msg.get("id") == Some(&json!(1))
        {
            assert!(
                msg.get("result").is_some(),
                "l'adaptateur a refuse l'initialisation : {ligne}"
            );
            initialise = true;
            break;
        }
    }
    assert!(initialise, "aucune reponse a `initialize`");

    // 2. `session/new` : c'est la que l'adaptateur construit la ligne de commande. Elle
    //    echouera — le faux binaire sort tout de suite — et c'est sans importance : on
    //    veut l'`argv`, pas la session.
    envoyer(
        &mut stdin,
        json!({
            "jsonrpc": "2.0", "id": 2, "method": "session/new",
            "params": { "cwd": dir, "mcpServers": [] }
        }),
    )
    .await;

    // On lit jusqu'a la reponse (succes ou erreur), ou jusqu'a expiration.
    let _ = tokio::time::timeout(Duration::from_secs(60), async {
        while let Ok(Some(ligne)) = lignes.next_line().await {
            if let Ok(msg) = serde_json::from_str::<Value>(&ligne)
                && msg.get("id") == Some(&json!(2))
            {
                return;
            }
        }
    })
    .await;

    let _ = child.kill().await;
    std::fs::read_to_string(&capture)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// ★ Le canari.
///
/// Echoue **bruyamment** si l'adaptateur laisse `Write` ou `Edit` disponibles alors que
/// nous avons annonce `fs.writeTextFile`.
#[tokio::test]
async fn l_adaptateur_retire_toujours_les_outils_d_ecriture_natifs() {
    let commande = commande_adaptateur();
    if !adaptateur_disponible(&commande) {
        // On ne fait pas passer un test qui n'a rien verifie : on le dit fort.
        eprintln!(
            "\n⚠️  CANARI NON EXECUTE — `{commande}` introuvable sur le PATH.\n\
             L'invariant d'interception n'est donc PAS surveille dans cette execution.\n\
             Pour l'activer :  npm install -g @zed-industries/claude-code-acp\n"
        );
        return;
    }

    let dir = std::env::temp_dir().join(format!("trame-canari-{}", uuid_court()));
    std::fs::create_dir_all(&dir).expect("repertoire temporaire");
    let argv = argv_negocie(&commande, &dir).await;
    std::fs::remove_dir_all(&dir).ok();

    assert!(
        !argv.is_empty(),
        "aucun argument capture : l'adaptateur n'a pas lance CLAUDE_CODE_EXECUTABLE.\n\
         Le mecanisme de la sonde a change — le canari doit etre revu avant d'etre cru."
    );

    // `--disallowedTools` est suivi soit d'un argument unique separe par des virgules,
    // soit de plusieurs arguments selon la version. On regarde tout l'argv.
    let complet = argv.join(" ");
    let position = argv.iter().position(|arg| arg == "--disallowedTools");

    let manquants: Vec<&str> = OUTILS_A_RETIRER
        .iter()
        .copied()
        .filter(|outil| {
            !argv
                .iter()
                .skip(position.map_or(usize::MAX, |index| index + 1))
                .take(4)
                .any(|arg| arg.split(',').any(|nom| nom == *outil))
        })
        .collect();

    assert!(
        manquants.is_empty(),
        "\n\n★ CANARI DECLENCHE — L'INVARIANT D'INTERCEPTION EST ROMPU.\n\n\
         Nous avons annonce `fs.writeTextFile`, et l'adaptateur `{commande}` laisse \
         pourtant {manquants:?} disponible(s).\n\
         L'agent peut donc ecrire directement sur le disque, sans passer par le registre : \
         l'avis de lecture perimee n'a plus aucun point d'accroche.\n\n\
         Ligne de commande observee :\n  {complet}\n\n\
         Ce n'est PAS un test a ajuster. C'est un changement de comportement d'un paquet \
         tiers qui supprime la raison d'exister du produit.\n\
         Ne pas contourner par un watcher. Lire l'ADR 0016 et l'ADR 0017, puis en parler.\n"
    );

    eprintln!("canari : {commande} retire bien {OUTILS_A_RETIRER:?} — argv = {complet}");
}

/// Le canari doit aussi verifier qu'il **sait detecter** une rupture, sinon il pourrait
/// passer au vert sans rien garder. On rejoue son analyse sur l'`argv` reellement observe
/// avec le paquet successeur.
#[test]
fn le_canari_detecte_la_rupture_connue_du_paquet_successeur() {
    // Capture reelle de `@agentclientprotocol/claude-agent-acp` 0.66.0.
    let successeur: Vec<String> = [
        "--output-format",
        "stream-json",
        "--verbose",
        "--input-format",
        "stream-json",
        "--permission-prompt-tool",
        "stdio",
        "--disallowedTools",
        "AskUserQuestion",
        "--tools",
        "default",
        "--permission-mode",
        "default",
    ]
    .iter()
    .map(|arg| (*arg).to_owned())
    .collect();

    let position = successeur.iter().position(|arg| arg == "--disallowedTools");
    let retires: Vec<&str> = OUTILS_A_RETIRER
        .iter()
        .copied()
        .filter(|outil| {
            successeur
                .iter()
                .skip(position.map_or(usize::MAX, |index| index + 1))
                .take(4)
                .any(|arg| arg.split(',').any(|nom| nom == *outil))
        })
        .collect();

    assert!(
        retires.is_empty(),
        "le canari doit voir que 0.66.0 ne retire NI Write NI Edit ; il a cru voir {retires:?}"
    );

    // Et le meme raisonnement sur la capture reelle de 0.16.2 doit, lui, tout trouver.
    let connu: Vec<String> = [
        "--disallowedTools",
        "AskUserQuestion,Read,Write,Edit",
        "--tools",
        "default",
    ]
    .iter()
    .map(|arg| (*arg).to_owned())
    .collect();
    let position = connu.iter().position(|arg| arg == "--disallowedTools");
    for outil in OUTILS_A_RETIRER {
        assert!(
            connu
                .iter()
                .skip(position.map_or(usize::MAX, |index| index + 1))
                .take(4)
                .any(|arg| arg.split(',').any(|nom| nom == *outil)),
            "le canari doit reconnaitre {outil} comme retire dans la capture de 0.16.2"
        );
    }
}

/// Un suffixe unique sans dependre d'un crate supplementaire.
fn uuid_court() -> String {
    trame_core::ProjectId::new()
        .to_string()
        .chars()
        .take(8)
        .collect()
}
