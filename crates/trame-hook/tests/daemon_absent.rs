// Un test d'integration est un binaire ordinaire : les exemptions de `clippy.toml` ne s'y
// appliquent pas.
#![allow(clippy::expect_used, clippy::unwrap_used)]

//! ★★ **Le controle negatif, ecrit avant le chemin nominal.**
//!
//! C'est le premier IPC du projet, donc l'endroit privilegie pour une sortie plausible qui
//! mente. Le mode d'echec a refuser est precis :
//!
//! > Le daemon n'ecoute pas. `trame-hook` ne peut pas demander. S'il sort 0 sans rien dire, la
//! > CLI comprend « pas d'objection » et l'ecriture passe. L'invariant est mort, et l'agent
//! > travaille normalement : **aucun symptome.**
//!
//! Ces tests existent **avant** le chemin nominal, et pas apres, parce qu'un chemin nominal qui
//! fonctionne rend le cas degrade abstrait — et un cas degrade abstrait ne se teste jamais
//! vraiment.
//!
//! Ils couvrent les quatre facons de ne pas obtenir de verdict : socket absente, socket perimee,
//! daemon muet, reponse incomprehensible. **Les quatre doivent refuser.**

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;

use trame_hook::{Decision, HookError, demander};

const PAYLOAD: &str = r#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"echo x > f.txt"}}"#;

/// Un faux daemon fidele : il **lit la demande avant de repondre**.
///
/// Ce detail n'est pas de la coquetterie. Un faux daemon qui ecrit puis ferme sans lire gagne
/// parfois la course contre notre propre ecriture, qui prend alors un `BrokenPipe` — et le test
/// echoue une fois sur dix pour une raison qui n'a rien a voir avec ce qu'il verifie. Un vrai
/// daemon lit puis repond ; le faux doit faire pareil.
fn repondre_apres_lecture(ecoute: &UnixListener, reponse: &str) {
    if let Ok((flux, _)) = ecoute.accept() {
        let mut demande = String::new();
        let _ = BufReader::new(&flux).read_line(&mut demande);
        let mut ecriture = &flux;
        let _ = ecriture.write_all(reponse.as_bytes());
        let _ = ecriture.flush();
    }
}

fn chemin_jetable(nom: &str) -> PathBuf {
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
fn une_socket_absente_refuse_et_le_dit() {
    let socket = chemin_jetable("absente");
    assert!(!socket.exists());

    let erreur =
        demander(&socket, PAYLOAD).expect_err("un daemon absent ne doit PAS laisser passer");

    assert!(
        matches!(erreur, HookError::SocketAbsente { .. }),
        "la cause doit etre nommee, pas generique : {erreur:?}"
    );
    let motif = erreur.motif();
    assert!(
        motif.contains("refuse"),
        "le motif doit dire que c'est refuse : {motif}"
    );
    assert!(
        motif.contains(&socket.display().to_string()),
        "et ou chercher : {motif}"
    );
}

/// ★ Socket perimee : le fichier existe, personne au bout.
///
/// C'est le cas d'un daemon qui a plante en laissant son fichier. Il se distingue du precedent
/// parce qu'il ne se repare pas de la meme facon, et le motif doit le dire.
#[test]
fn une_socket_perimee_refuse_et_le_dit() {
    let socket = chemin_jetable("perimee");
    // Un fichier ordinaire au nom d'une socket : exactement ce que laisse un processus mort.
    std::fs::write(&socket, b"").expect("fichier temoin");

    let erreur =
        demander(&socket, PAYLOAD).expect_err("une socket perimee ne doit PAS laisser passer");

    assert!(
        matches!(erreur, HookError::Injoignable { .. }),
        "une socket perimee n'est pas une socket absente : {erreur:?}"
    );
    assert!(erreur.motif().contains("refuse"));
    std::fs::remove_file(&socket).ok();
}

/// ★ Daemon muet : il accepte la connexion puis ferme sans repondre.
///
/// Le plus retors des quatre, parce que tout a l'air de marcher — la connexion reussit et il n'y
/// a simplement pas de verdict. Une reponse vide **n'est pas** un silence.
///
/// # Ce test n'epingle pas la variante d'erreur, et c'est deliberé
///
/// Selon qui gagne la course, notre ecriture part avant la fermeture (`ReponseIllisible`, le
/// daemon a ferme sans repondre) ou apres (`Injoignable` sur un `BrokenPipe`). Une premiere
/// version epinglait `ReponseIllisible` et passait **trois fois sur cinq** — un test instable,
/// donc un test qu'on finit par ignorer.
///
/// La propriete qui compte n'est pas laquelle des deux : c'est qu'aucune ne laisse passer.
#[test]
fn un_daemon_muet_refuse() {
    let socket = chemin_jetable("muet");
    let ecoute = UnixListener::bind(&socket).expect("socket d'ecoute");
    let fil = std::thread::spawn(move || {
        if let Ok((flux, _)) = ecoute.accept() {
            drop(flux); // accepte, puis ferme sans un mot
        }
    });

    let erreur = demander(&socket, PAYLOAD).expect_err("un daemon muet ne doit PAS laisser passer");
    assert!(
        matches!(
            erreur,
            HookError::ReponseIllisible(_) | HookError::Injoignable { .. }
        ),
        "sans verdict, on refuse — quelle que soit la facon dont la connexion meurt : {erreur:?}"
    );
    assert!(
        erreur.motif().contains("refuse"),
        "et le motif le dit : {}",
        erreur.motif()
    );

    fil.join().ok();
    std::fs::remove_file(&socket).ok();
}

/// ★ Reponse incomprehensible : le daemon repond quelque chose qu'on ne sait pas lire.
///
/// Cas d'une rupture de protocole entre deux versions. On ne devine pas, on refuse.
#[test]
fn une_reponse_incomprehensible_refuse() {
    for reponse in [
        "pas du json\n",
        "{\"decision\":\"peut-etre\"}\n",
        "{\"autre\":\"chose\"}\n",
    ] {
        let socket = chemin_jetable("illisible");
        let ecoute = UnixListener::bind(&socket).expect("socket d'ecoute");
        let attendu = reponse.to_owned();
        let fil = std::thread::spawn(move || repondre_apres_lecture(&ecoute, &attendu));

        let erreur = demander(&socket, PAYLOAD)
            .expect_err("une reponse illisible ne doit PAS laisser passer");
        assert!(
            matches!(erreur, HookError::ReponseIllisible(_)),
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
fn un_payload_illisible_refuse() {
    let socket = chemin_jetable("payload");
    let ecoute = UnixListener::bind(&socket).expect("socket d'ecoute");
    let fil = std::thread::spawn(move || {
        // Un daemon complaisant : il dirait « silence » a tout. Le hook ne doit meme pas lui
        // poser la question sur un payload illisible.
        repondre_apres_lecture(&ecoute, "{\"decision\":\"silence\"}\n");
    });

    let erreur = demander(&socket, "ceci n'est pas du json")
        .expect_err("un payload illisible ne doit PAS laisser passer");
    assert!(
        matches!(erreur, HookError::PayloadIllisible(_)),
        "{erreur:?}"
    );

    drop(fil);
    std::fs::remove_file(&socket).ok();
}

/// ★★ **Le controle du controle** : le dispositif sait-il dire oui ?
///
/// Sans ce test, tous les precedents passeraient avec un `demander` qui refuserait *tout*, y
/// compris un daemon en bonne sante. Un controle negatif sans son pendant positif ne prouve
/// rien — c'est la lecon inscrite dans la skill `concurrency-testing`.
#[test]
fn le_dispositif_sait_dire_oui_et_non() {
    for (reponse, attendu) in [
        ("{\"decision\":\"silence\"}\n", Decision::Silence),
        (
            "{\"decision\":\"refus\",\"motif\":\"ecris par ton outil de fichier\"}\n",
            Decision::Refus("ecris par ton outil de fichier".to_owned()),
        ),
    ] {
        let socket = chemin_jetable("nominal");
        let ecoute = UnixListener::bind(&socket).expect("socket d'ecoute");
        let a_ecrire = reponse.to_owned();
        let fil = std::thread::spawn(move || repondre_apres_lecture(&ecoute, &a_ecrire));

        let decision = demander(&socket, PAYLOAD).expect("un daemon sain doit repondre");
        assert_eq!(decision, attendu);

        fil.join().ok();
        std::fs::remove_file(&socket).ok();
    }
}

/// Le refus produit le JSON que la CLI attend, et le silence ne produit **rien**.
///
/// Les cles viennent de la sonde 2, ou elles ont ete observees — pas d'un typage.
#[test]
fn le_json_rendu_est_celui_que_la_cli_attend() {
    assert!(
        Decision::Silence.en_json().is_none(),
        "ne rien dire est ce qui laisse passer, et c'est voulu pour le silence"
    );
    let json = Decision::Refus("motif".to_owned())
        .en_json()
        .expect("un refus produit du JSON");
    let valeur: serde_json::Value = serde_json::from_str(&json).expect("JSON valide");
    let sortie = &valeur["hookSpecificOutput"];
    assert_eq!(sortie["hookEventName"], "PreToolUse");
    assert_eq!(sortie["permissionDecision"], "deny");
    assert_eq!(sortie["permissionDecisionReason"], "motif");
}
