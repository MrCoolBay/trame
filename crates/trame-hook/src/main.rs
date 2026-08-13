//! Le binaire lance par la CLI a chaque appel d'outil.
//!
//! Lit le payload sur `stdin`, demande un verdict au daemon, ecrit la decision sur `stdout`.
//!
//! # Les codes de sortie, et pourquoi ils comptent
//!
//! - **0 avec rien sur `stdout`** — silence, la CLI poursuit. ~95 % du trafic.
//! - **0 avec le JSON de refus** — refus decide par le registre. La CLI bloque et l'agent lit
//!   le motif.
//! - **1 avec un message sur `stderr`** — Trame n'a pas pu verifier. **Refus aussi**, mais
//!   bruyant : c'est une panne, pas une politique.
//!
//! Le troisieme cas est la raison d'etre de ce fichier. Un hook qui sortirait 0 sans avoir
//! consulte la politique laisserait passer l'ecriture, et l'invariant mourrait sans un
//! symptome (ADR 0025).

use std::io::{Read, Write};
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut payload = String::new();
    if let Err(error) = std::io::stdin().read_to_string(&mut payload) {
        return refuser(&format!("payload illisible sur stdin : {error}"));
    }

    // Le projet est passe par la ligne de commande : c'est Trame qui ecrit le fichier de
    // reglages, donc Trame qui sait de quel projet il s'agit.
    let Some(projet) = std::env::args().nth(1) else {
        return refuser("usage : trame-hook <projet> — appele sans projet");
    };

    let socket = match trame_hook::chemin_socket(&projet) {
        Ok(chemin) => chemin,
        Err(error) => return refuser(&error.motif()),
    };

    match trame_hook::demander(&socket, &payload) {
        Ok(decision) => {
            if let Some(json) = decision.en_json() {
                // Un echec d'ecriture sur stdout laisserait la CLI conclure « pas d'objection ».
                if let Err(error) = std::io::stdout()
                    .write_all(json.as_bytes())
                    .and_then(|()| std::io::stdout().flush())
                {
                    return refuser(&format!("refus non transmis a la CLI : {error}"));
                }
            }
            ExitCode::SUCCESS
        }
        // ★ Toute erreur mene ici, et ici on refuse.
        Err(error) => refuser(&error.motif()),
    }
}

/// Refuse bruyamment : message sur `stderr`, code non nul.
///
/// On n'ecrit **pas** de JSON de refus sur `stdout` dans ce cas : la CLI traite un code non nul
/// comme une erreur de hook, ce qui est exactement ce que c'est. Un refus de politique et une
/// panne de Trame ne doivent pas se ressembler dans les logs.
fn refuser(motif: &str) -> ExitCode {
    let _ = writeln!(std::io::stderr(), "trame-hook : {motif}");
    ExitCode::FAILURE
}
