//! `PtyBackend` — le mode degrade. **Squelette.**
//!
//! # Pourquoi il existe deja
//!
//! ACP est incomplet et inegal selon les harness. Sans repli, chaque trou du protocole
//! devient un harness non supporte, et le repli n'est donc pas optionnel (ADR 0005).
//!
//! # Pourquoi il ne fait rien encore
//!
//! Ce qui compte en v0.1 est le path qui **permet l'admission**, et c'est l'ACP. Un
//! `PtyBackend` a moitie fait donnerait l'illusion d'un support qui n'existe pas. Il est
//! donc explicitement `todo!()`, avec des capacites honnetes des maintenant : c'est
//! `capabilities()` qui compte, et elle dit deja la verite.
//!
//! # Ce que le mode degrade signifie
//!
//! En PTY, on voit du texte. Les ecritures se decouvrent *apres coup* par FSEvents,
//! quand le tool call est termine et que l'agent est passe a la suite. On peut encore
//! journaliser et attribuer — la valeur du journal seul est reelle — mais l'avis de
//! lecture perimee, lui, disparait.
//!
//! **L'interface doit afficher la banniere de degradation.** Un utilisateur qui croit avoir la
//! garantie d'admission est dans une situation *pire* que sans outil : il fait confiance
//! a un filet qui n'existe pas.

use async_trait::async_trait;

use crate::backend::{AgentBackend, AgentEventStream, Capabilities, UserMessage};
use crate::error::AgentError;

/// Pilotage d'une CLI par un pseudo-terminal. Mode degrade, non implemente.
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct PtyBackend {
    /// La commande a piloter, quand ce backend sera implemente.
    pub command: String,
}

impl PtyBackend {
    /// Prepare un backend PTY pour une commande donnee.
    ///
    /// Ne lance rien : l'implementation viendra avec `portable-pty`.
    #[must_use]
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
        }
    }
}

#[async_trait]
impl AgentBackend for PtyBackend {
    /// La seule methode reellement implementee, et c'est la plus importante : elle
    /// annonce que **les ecritures ne sont pas interceptables**.
    fn capabilities(&self) -> Capabilities {
        Capabilities::pty()
    }

    async fn send(&mut self, _msg: UserMessage) -> Result<(), AgentError> {
        todo!("PtyBackend : pilotage via portable-pty, apres la v0.1")
    }

    fn events(&mut self) -> Option<AgentEventStream> {
        todo!("PtyBackend : normalisation du feed depuis la sortie terminal")
    }

    async fn shutdown(&mut self) -> Result<(), AgentError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Le squelette doit deja dire la verite sur ce qu'il ne sait pas faire. C'est la
    /// seule chose qu'on lui demande en v0.1, et c'est testable.
    #[test]
    fn le_backend_pty_annonce_sa_degradation() {
        let backend = PtyBackend::new("claude");
        assert!(backend.capabilities().is_degraded());
        assert!(!backend.capabilities().can_intercept_writes);
        assert!(!backend.capabilities().can_inject_context);
        assert!(!backend.capabilities().can_request_permission);
    }
}
