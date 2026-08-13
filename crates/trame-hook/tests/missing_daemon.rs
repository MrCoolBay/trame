// Un test d'integration est un binaire ordinaire : les exemptions de `clippy.toml` ne s'y
// appliquent pas.
#![allow(clippy::expect_used, clippy::unwrap_used)]

//! ★★ **Le controle negatif, ecrit avant le path nominal.**
//!
//! C'est le premier IPC du projet, donc l'endroit privilegie pour une sortie plausible qui
//! mente. Le mode d'echec a deny est precis :
//!
//! > Le daemon n'ecoute pas. `trame-hook` ne peut pas ask. S'il sort 0 sans rien dire, la
//! > CLI comprend « pas d'objection » et l'ecriture passe. L'invariant est mort, et l'agent
//! > travaille normalement : **aucun symptome.**
//!
//! Ces tests existent **avant** le path nominal, et pas apres, parce qu'un path nominal qui
//! fonctionne rend le cas degrade abstrait — et un cas degrade abstrait ne se teste jamais
//! vraiment.
//!
//! Ils couvrent les quatre facons de ne pas obtenir de verdict : socket absente, socket perimee,
//! daemon muet, reponse incomprehensible. **Les quatre doivent deny.**

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;

use trame_hook::{Decision, HookError, ask};

const PAYLOAD: &str = r#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"echo x > f.txt"}}"#;

/// Un faux daemon fidele : il **lit la demande avant de repondre**.
///
/// Ce detail n'est pas de la coquetterie. Un faux daemon qui ecrit puis ferme sans lire gagne
/// parfois la course contre notre propre ecriture, qui prend alors un `BrokenPipe` — et le test
/// echoue une fois sur dix pour une raison qui n'a rien a voir avec ce qu'il verifie. Un vrai
/// daemon lit puis repond ; le faux doit faire pareil.
fn reply_after_read(ecoute: &UnixListener, reponse: &str) {
    if let Ok((feed, _)) = ecoute.accept() {
        let mut demande = String::new();
        let _ = BufReader::new(&feed).read_line(&mut demande);
        let mut ecriture = &feed;
        let _ = ecriture.write_all(reponse.as_bytes());
        let _ = ecriture.flush();
    }
}

fn throwaway_path(nom: &str) -> PathBuf {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("horloge systeme")
        .as_nanos();
    std::env::temp_dir().join(format!("trame-hook-{nom}-{unique}.sock"))
}

/// ★ Socket absente : le daemon n'a jamais demarre pour ce projet.
///
/// Le cas le plus banal — l'utilisateur n'a pas ouvert le projet dans Trame — et le plus
/// dangereux si on le laisse passer.
#[test]
fn a_missing_socket_denies_and_says_why() {
    let socket = throwaway_path("absente");
    assert!(!socket.exists());

    let erreur = ask(&socket, PAYLOAD).expect_err("un daemon absent ne doit PAS laisser passer");

    assert!(
        matches!(erreur, HookError::SocketMissing { .. }),
        "la cause doit etre nommee, pas generique : {erreur:?}"
    );
    let reason = erreur.reason();
    assert!(
        reason.contains("refuse"),
        "le reason doit dire que c'est refuse : {reason}"
    );
    assert!(
        reason.contains(&socket.display().to_string()),
        "et ou chercher : {reason}"
    );
}

/// ★ Socket perimee : le file existe, personne au bout.
///
/// C'est le cas d'un daemon qui a plante en laissant son file. Il se distingue du precedent
/// parce qu'il ne se repare pas de la meme facon, et le reason doit le dire.
#[test]
fn a_stale_socket_denies_and_says_why() {
    let socket = throwaway_path("perimee");
    // Un file ordinaire au nom d'une socket : exactement ce que laisse un processus mort.
    std::fs::write(&socket, b"").expect("file temoin");

    let erreur = ask(&socket, PAYLOAD).expect_err("une socket perimee ne doit PAS laisser passer");

    assert!(
        matches!(erreur, HookError::Unreachable { .. }),
        "une socket perimee n'est pas une socket absente : {erreur:?}"
    );
    assert!(erreur.reason().contains("refuse"));
    std::fs::remove_file(&socket).ok();
}

/// ★ Daemon muet : il accepte la connexion puis ferme sans repondre.
///
/// Le plus retors des quatre, parce que tout a l'air de marcher — la connexion reussit et il n'y
/// a simplement pas de verdict. Une reponse vide **n'est pas** un silence.
///
/// # Ce test n'epingle pas la variante d'erreur, et c'est deliberé
///
/// Selon qui gagne la course, notre ecriture part avant la fermeture (`UnreadableResponse`, le
/// daemon a ferme sans repondre) ou apres (`Unreachable` sur un `BrokenPipe`). Une premiere
/// version epinglait `UnreadableResponse` et passait **trois fois sur cinq** — un test instable,
/// donc un test qu'on finit par ignorer.
///
/// La propriete qui compte n'est pas laquelle des deux : c'est qu'aucune ne laisse passer.
#[test]
fn a_daemon_that_answers_nothing_denies() {
    let socket = throwaway_path("muet");
    let ecoute = UnixListener::bind(&socket).expect("socket d'ecoute");
    let fil = std::thread::spawn(move || {
        if let Ok((feed, _)) = ecoute.accept() {
            drop(feed); // accepte, puis ferme sans un mot
        }
    });

    let erreur = ask(&socket, PAYLOAD).expect_err("un daemon muet ne doit PAS laisser passer");
    assert!(
        matches!(
            erreur,
            HookError::UnreadableResponse(_) | HookError::Unreachable { .. }
        ),
        "sans verdict, on refuse — quelle que soit la facon dont la connexion meurt : {erreur:?}"
    );
    assert!(
        erreur.reason().contains("refuse"),
        "et le reason le dit : {}",
        erreur.reason()
    );

    fil.join().ok();
    std::fs::remove_file(&socket).ok();
}

/// ★ Response incomprehensible : le daemon repond quelque chose qu'on ne sait pas lire.
///
/// Cas d'une rupture de protocole entre deux versions. On ne devine pas, on refuse.
#[test]
fn an_unreadable_response_denies() {
    for reponse in [
        "pas du json\n",
        "{\"decision\":\"peut-etre\"}\n",
        "{\"autre\":\"chose\"}\n",
    ] {
        let socket = throwaway_path("illisible");
        let ecoute = UnixListener::bind(&socket).expect("socket d'ecoute");
        let attendu = reponse.to_owned();
        let fil = std::thread::spawn(move || reply_after_read(&ecoute, &attendu));

        let erreur =
            ask(&socket, PAYLOAD).expect_err("une reponse illisible ne doit PAS laisser passer");
        assert!(
            matches!(erreur, HookError::UnreadableResponse(_)),
            "reponse {reponse:?} : {erreur:?}"
        );

        fil.join().ok();
        std::fs::remove_file(&socket).ok();
    }
}

/// Un payload que la CLI n'aurait pas du envoyer est refuse aussi.
///
/// Si le contrat de la CLI change, on ne devine pas ce qu'elle voulait dire.
#[test]
fn an_unreadable_payload_denies() {
    let socket = throwaway_path("payload");
    let ecoute = UnixListener::bind(&socket).expect("socket d'ecoute");
    let fil = std::thread::spawn(move || {
        // Un daemon complaisant : il dirait « silence » a tout. Le hook ne doit meme pas lui
        // poser la question sur un payload illisible.
        reply_after_read(&ecoute, "{\"decision\":\"silence\"}\n");
    });

    let erreur = ask(&socket, "ceci n'est pas du json")
        .expect_err("un payload illisible ne doit PAS laisser passer");
    assert!(
        matches!(erreur, HookError::UnreadablePayload(_)),
        "{erreur:?}"
    );

    drop(fil);
    std::fs::remove_file(&socket).ok();
}

/// ★★ **Le controle du controle** : le dispositif sait-il dire oui ?
///
/// Sans ce test, tous les precedents passeraient avec un `ask` qui refuserait *tout*, y
/// compris un daemon en bonne sante. Un controle negatif sans son pendant positif ne prouve
/// rien — c'est la lecon inscrite dans la skill `concurrency-testing`.
#[test]
fn the_apparatus_can_say_both_yes_and_no() {
    for (reponse, attendu) in [
        ("{\"decision\":\"silence\"}\n", Decision::Silence),
        (
            "{\"decision\":\"refus\",\"reason\":\"ecris par ton outil de file\"}\n",
            Decision::Deny("ecris par ton outil de file".to_owned()),
        ),
    ] {
        let socket = throwaway_path("nominal");
        let ecoute = UnixListener::bind(&socket).expect("socket d'ecoute");
        let a_ecrire = reponse.to_owned();
        let fil = std::thread::spawn(move || reply_after_read(&ecoute, &a_ecrire));

        let decision = ask(&socket, PAYLOAD).expect("un daemon sain doit repondre");
        assert_eq!(decision, attendu);

        fil.join().ok();
        std::fs::remove_file(&socket).ok();
    }
}

/// Le refus produit le JSON que la CLI attend, et le silence ne produit **rien**.
///
/// Les cles viennent de la sonde 2, ou elles ont ete observees — pas d'un typage.
#[test]
fn the_emitted_json_is_the_shape_the_cli_expects() {
    assert!(
        Decision::Silence.to_json().is_none(),
        "ne rien dire est ce qui laisse passer, et c'est voulu pour le silence"
    );
    let json = Decision::Deny("reason".to_owned())
        .to_json()
        .expect("un refus produit du JSON");
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("JSON valide");
    let sortie = &parsed["hookSpecificOutput"];
    assert_eq!(sortie["hookEventName"], "PreToolUse");
    assert_eq!(sortie["permissionDecision"], "deny");
    assert_eq!(sortie["permissionDecisionReason"], "reason");
}
