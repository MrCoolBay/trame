//! Transport agent. Abstraction sur les harness.
//!
//! Le reste du core ne sait jamais s'il parle a de l'ACP ou a un PTY.
//!
//! # L'inversion qui rend le produit possible
//!
//! En ACP, **Trame est le client et l'agent est le serveur**. Ce n'est pas l'agent qui
//! ecrit puis nous previent : c'est l'agent qui *demande* a Trame d'ecrire. Le point
//! d'interception n'est pas un hook a installer, c'est le chemin normal du protocole.
//!
//! Validation empirique et trous nommes :
//! [ADR 0016](../../../docs/adr/0016-interception-avant-disque-validee.md).
//!
//! # L'ordre est non negociable
//!
//! ```text
//! requete d'ecriture entrante
//!   -> AgentEvent::FileWrite(request)
//!   -> registre : admission + ecriture       ★ AVANT que l'agent croie avoir ecrit
//!   -> request.admitted()
//! ```
//!
//! Inverser les deux etapes du milieu produit du code qui compile, passe les tests, et
//! supprime la raison d'exister du produit. C'est le bug le plus important a ne pas
//! ecrire dans ce depot — et le type le rend difficile : une
//! [`FileWriteRequest`] abandonnee **refuse** l'ecriture au lieu
//! de l'autoriser en silence.
//!
//! # Deux backends
//!
//! - [`AcpBackend`] — JSON-RPC sur stdio. Le chemin qui compte. Une seule cible en
//!   v0.1 : Claude Code.
//! - [`PtyBackend`] — squelette `todo!()`, avec des capacites honnetes. Le repli n'est
//!   pas optionnel, mais il n'est pas la priorite de la v0.1.
//!
//! # Testable sans agent
//!
//! [`AcpBackend::connect`] accepte n'importe quel couple lecteur/ecrivain asynchrone.
//! Les tests scenarisent l'agent en memoire : pas de sous-process, pas de reseau, pas
//! d'authentification, et un resultat deterministe.

mod acp;
mod backend;
mod error;
mod event;
mod jsonrpc;
mod pty;

pub use acp::{AcpBackend, CLAUDE_CODE_ACP_COMMAND, PROTOCOL_VERSION};
pub use backend::{AgentBackend, AgentEventStream, Capabilities, UserMessage};
pub use error::AgentError;
pub use event::{
    AgentEvent, FileReadRequest, FileWriteRequest, PermissionOption, PermissionRequest,
};
pub use pty::PtyBackend;

/// Le module des evenements, pour la documentation croisee.
pub mod events {
    pub use crate::event::*;
}
