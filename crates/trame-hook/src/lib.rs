//! Le pont entre un hook de la CLI et le daemon. **Il ne decide rien.**
//!
//! La CLI lance ce binaire a chaque appel d'outil de l'agent, lui passe le payload du hook sur
//! `stdin`, et lit sa decision sur `stdout`. Toute la politique vit dans le registre, cote
//! daemon : un hook qui deciderait serait une seconde copie des regles d'admission, et deux
//! copies divergent (ADR 0025).
//!
//! # ★ La regle qui gouverne ce crate
//!
//! > **En cas d'impossibilite de joindre le daemon, on REFUSE, et on le dit.**
//!
//! Le mode d'echec a deny est precis. Le daemon n'ecoute pas — pas demarre, plante, socket
//! perimee. Si ce binaire sort 0 sans rien dire, la CLI comprend « pas d'objection » et
//! l'ecriture passe. L'invariant est mort et l'agent travaille normalement : **aucun symptome.**
//!
//! C'est le meme raisonnement que le `Drop` de `FileWriteRequest`, qui refuse par defaut
//! (ADR 0016) : sur le path d'admission, l'absence de reponse n'est jamais un oui.
//!
//! Consequence assumee : si le daemon est absent, l'agent est bloque en ecriture shell. C'est
//! bruyant, donc reparable. L'inverse est silencieux, donc pas.

pub mod bash;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Delai au-dela duquel on considere le daemon injoignable.
///
/// Court a dessein : l'agent attend. Un daemon vivant mais bloque est traite comme un daemon
/// absent — **refus**. Mieux vaut bloquer une ecriture shell que suspendre une session.
pub const TIMEOUT: Duration = Duration::from_millis(2_000);

/// Ce que le hook rend a la CLI.
///
/// Le format est celui de `hookSpecificOutput`, observe en sonde 2 — pas deduit d'un typage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Rien a dire : la CLI poursuit. C'est le cas de ~95 % du trafic.
    Silence,
    /// Deny, avec le reason transmis a l'agent. La sonde 2 a mesure qu'il le lit et le cite.
    Deny(String),
}

impl Decision {
    /// Le JSON attendu par la CLI, ou rien quand il n'y a rien a dire.
    ///
    /// Un hook qui n'ecrit rien sur `stdout` laisse passer : c'est exactement ce qu'on veut
    /// pour [`Decision::Silence`], et exactement ce qu'on ne veut **jamais** en cas d'erreur.
    #[must_use]
    pub fn to_json(&self) -> Option<String> {
        match self {
            Self::Silence => None,
            Self::Deny(reason) => Some(
                serde_json::json!({
                    "hookSpecificOutput": {
                        "hookEventName": "PreToolUse",
                        "permissionDecision": "deny",
                        "permissionDecisionReason": reason,
                    }
                })
                .to_string(),
            ),
        }
    }
}

/// Pourquoi le hook n'a pas pu consulter la politique.
///
/// **Chaque variante mene a un refus**, jamais a un laissez-passer. Le type existe pour que le
/// reason affiche soit precis : « daemon absent » et « reponse illisible » ne se reparent pas de
/// la meme facon.
#[derive(Debug, thiserror::Error)]
pub enum HookError {
    /// La socket n'existe pas : le daemon n'a jamais demarre pour ce projet.
    #[error("aucun daemon Trame n'ecoute sur {path} — projet ouvert dans Trame ?")]
    SocketMissing {
        /// Le path attendu.
        path: PathBuf,
    },
    /// La socket existe mais personne au bout : daemon plante, socket perimee.
    #[error("le daemon Trame n'a pas repondu sur {path} ({source}) — socket perimee ?")]
    Unreachable {
        /// Le path tente.
        path: PathBuf,
        /// La cause systeme.
        source: std::io::Error,
    },
    /// Le daemon a repondu quelque chose qu'on ne comprend pas.
    #[error("reponse illisible du daemon : {0}")]
    UnreadableResponse(String),
    /// Le payload de la CLI n'est pas du JSON.
    #[error("payload de hook illisible : {0}")]
    UnreadablePayload(String),
}

impl HookError {
    /// Le reason de refus transmis a l'agent.
    ///
    /// Il nomme la cause et l'action, parce que l'agent le relaie a l'utilisateur : un refus
    /// qui dit seulement « refuse » envoie chercher pendant dix minutes.
    #[must_use]
    pub fn reason(&self) -> String {
        format!(
            "Trame n'a pas pu check cette action, elle est donc refusee. {self} \
             (Trame refuse par defaut : une action non verifiee n'est pas une action autorisee.)"
        )
    }
}

/// Demande un verdict au daemon.
///
/// # Erreurs
///
/// Toute erreur doit etre traduite en **refus** par l'appelant. Voir [`HookError`].
pub fn ask(socket: &Path, payload: &str) -> Result<Decision, HookError> {
    // On verifie l'existence avant de connecter, pour distinguer « jamais demarre » de
    // « plante » — deux reparations differentes.
    if !socket.exists() {
        return Err(HookError::SocketMissing {
            path: socket.to_path_buf(),
        });
    }
    // Un payload illisible est un bug chez nous ou une rupture de la CLI. Dans les deux cas on
    // ne devine pas : on refuse.
    serde_json::from_str::<serde_json::Value>(payload)
        .map_err(|error| HookError::UnreadablePayload(error.to_string()))?;

    let feed = UnixStream::connect(socket).map_err(|source| HookError::Unreachable {
        path: socket.to_path_buf(),
        source,
    })?;
    feed.set_read_timeout(Some(TIMEOUT))
        .and_then(|()| feed.set_write_timeout(Some(TIMEOUT)))
        .map_err(|source| HookError::Unreachable {
            path: socket.to_path_buf(),
            source,
        })?;

    let mut ecriture = &feed;
    // Un JSON par line, comme ACP. Le `\n` est le delimiteur, pas une commodite.
    let line = payload.replace('\n', " ");
    ecriture
        .write_all(format!("{line}\n").as_bytes())
        .and_then(|()| ecriture.flush())
        .map_err(|source| HookError::Unreachable {
            path: socket.to_path_buf(),
            source,
        })?;

    let mut reponse = String::new();
    BufReader::new(&feed)
        .read_line(&mut reponse)
        .map_err(|source| HookError::Unreachable {
            path: socket.to_path_buf(),
            source,
        })?;
    read_verdict(&reponse)
}

/// Traduit la reponse du daemon.
///
/// Format volontairement pauvre : `{"decision":"silence"}` ou
/// `{"decision":"refus","reason":"…"}`. Une reponse vide est une reponse **illisible**, donc un
/// refus — c'est le cas d'un daemon qui ferme la connexion sans repondre.
fn read_verdict(reponse: &str) -> Result<Decision, HookError> {
    let brut = reponse.trim();
    if brut.is_empty() {
        return Err(HookError::UnreadableResponse(
            "le daemon a ferme sans repondre".to_owned(),
        ));
    }
    let parsed: serde_json::Value =
        serde_json::from_str(brut).map_err(|e| HookError::UnreadableResponse(e.to_string()))?;
    match parsed.get("decision").and_then(serde_json::Value::as_str) {
        Some("silence") => Ok(Decision::Silence),
        Some("refus") => Ok(Decision::Deny(
            parsed
                .get("reason")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("refuse par Trame")
                .to_owned(),
        )),
        autre => Err(HookError::UnreadableResponse(format!(
            "decision inconnue : {autre:?}"
        ))),
    }
}

/// Le path de la socket d'un projet.
///
/// Dans le repertoire de donnees, **jamais dans le projet surveille** — qui est precisement ce
/// qu'on observe. Un path par projet : le registre est par projet (invariant 3), la socket
/// suit.
///
/// # Erreurs
///
/// Echoue si `HOME` est absent.
pub fn socket_path(projet: &str) -> Result<PathBuf, HookError> {
    let home = std::env::var_os("HOME")
        .ok_or_else(|| HookError::UnreadablePayload("HOME absent de l'environnement".to_owned()))?;
    Ok(PathBuf::from(home)
        .join("Library")
        .join("Application Support")
        .join("Trame")
        .join("sockets")
        .join(format!("{projet}.sock")))
}
