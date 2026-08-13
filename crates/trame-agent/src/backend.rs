//! Le trait [`AgentBackend`] et ses capacites.

use async_trait::async_trait;
use futures_core::Stream;
use tokio::sync::mpsc;

use crate::error::AgentError;
use crate::event::AgentEvent;

/// Ce qu'un backend sait faire.
///
/// **A interroger, jamais a deduire du type de backend au point d'appel.** Un
/// utilisateur en mode degrade qui croit avoir la garantie d'admission est dans une
/// situation *pire* que sans outil : il fait confiance a un filet qui n'existe pas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    /// Les ecritures sont-elles interceptables **avant** le disque ?
    ///
    /// `true` en ACP, `false` en PTY. C'est la capacite qui porte tout l'edifice : sans
    /// elle, le registre ne peut qu'observer apres coup, et l'avis de lecture perimee
    /// arrive quand l'agent est deja passe a la suite.
    pub can_intercept_writes: bool,
    /// Peut-on injecter du contexte dans le prompt de l'agent ?
    pub can_inject_context: bool,
    /// Peut-on lui ask une permission, et wait_for la reponse ?
    pub can_request_permission: bool,
}

impl Capabilities {
    /// Ce qu'un transport ACP offre.
    #[must_use]
    pub const fn acp() -> Self {
        Self {
            can_intercept_writes: true,
            can_inject_context: true,
            can_request_permission: true,
        }
    }

    /// Ce qu'un transport PTY offre. Mode degrade, a afficher comme tel.
    #[must_use]
    pub const fn pty() -> Self {
        Self {
            can_intercept_writes: false,
            can_inject_context: false,
            can_request_permission: false,
        }
    }

    /// Vrai si ce backend permet la garantie d'admission. Sinon, l'interface **doit**
    /// afficher la banniere de degradation.
    #[must_use]
    pub const fn is_degraded(self) -> bool {
        !self.can_intercept_writes
    }
}

/// Un message de l'utilisateur vers l'agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserMessage {
    /// Le texte du prompt.
    pub text: String,
    /// Le contexte injecte, place avant le prompt.
    ///
    /// C'est par ce champ que l'avis de lecture perimee atteint l'agent, apres avoir ete
    /// compose par `trame_core::PromptPipeline`.
    pub injected_context: Option<String>,
}

impl UserMessage {
    /// Un prompt nu.
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            injected_context: None,
        }
    }

    /// Attache du contexte injecte.
    #[must_use]
    pub fn with_context(mut self, context: impl Into<String>) -> Self {
        self.injected_context = Some(context.into());
        self
    }

    /// Le texte final envoye a l'agent : le contexte d'abord, le prompt ensuite.
    #[must_use]
    pub fn rendered(&self) -> String {
        match &self.injected_context {
            Some(context) if !context.is_empty() => format!("{context}\n\n{}", self.text),
            _ => self.text.clone(),
        }
    }
}

/// Le feed d'evenements d'un backend.
///
/// Un `Stream` par-dessus un `mpsc::Receiver` : ca donne l'ergonomie du feed annonce par
/// le cadrage tout en gardant un type **concret**, donc un trait compatible `dyn`. Une
/// signature `fn events(&mut self) -> impl Stream<...>` sur le trait aurait interdit
/// `Box<dyn AgentBackend>`, or le daemon tient des backends de types differents.
#[derive(Debug)]
pub struct AgentEventStream {
    rx: mpsc::Receiver<AgentEvent>,
}

impl AgentEventStream {
    pub(crate) fn new(rx: mpsc::Receiver<AgentEvent>) -> Self {
        Self { rx }
    }

    /// L'evenement suivant, ou `None` quand le backend s'est arrete.
    pub async fn next(&mut self) -> Option<AgentEvent> {
        self.rx.recv().await
    }
}

impl Stream for AgentEventStream {
    type Item = AgentEvent;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.rx.poll_recv(cx)
    }
}

/// Abstraction sur les harness. Le reste du core ne sait jamais s'il parle a de l'ACP ou
/// a un PTY.
#[async_trait]
pub trait AgentBackend: Send {
    /// Ce que ce backend sait faire.
    fn capabilities(&self) -> Capabilities;

    /// Envoie un message et **rend la main immediatement**.
    ///
    /// Le turn de l'agent se suit par le feed d'evenements, pas par le retour de cette
    /// methode : un agent peut reflechir plusieurs minutes, et bloquer ici empecherait
    /// de handle ses requetes d'ecriture — donc de l'admettre.
    async fn send(&mut self, msg: UserMessage) -> Result<(), AgentError>;

    /// Le feed d'evenements. Disponible une seule fois : c'est un feed, pas une
    /// diffusion.
    fn events(&mut self) -> Option<AgentEventStream>;

    /// Arrete proprement le backend et le sous-process qu'il pilot.
    async fn shutdown(&mut self) -> Result<(), AgentError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pty_declares_itself_degraded_and_acp_does_not() {
        assert!(Capabilities::pty().is_degraded());
        assert!(!Capabilities::acp().is_degraded());
        assert!(Capabilities::acp().can_intercept_writes);
        assert!(!Capabilities::pty().can_intercept_writes);
    }

    #[test]
    fn injected_context_comes_before_the_prompt() {
        let msg = UserMessage::new("continue").with_context("[Trame] auth.rs a change");
        assert_eq!(msg.rendered(), "[Trame] auth.rs a change\n\ncontinue");
    }

    #[test]
    fn empty_context_leaves_no_blank_lines() {
        assert_eq!(UserMessage::new("va").with_context("").rendered(), "va");
        assert_eq!(UserMessage::new("va").rendered(), "va");
    }
}
