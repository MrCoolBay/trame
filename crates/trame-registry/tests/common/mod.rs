// Ce module est compile une fois par binaire de test, et chacun n'en utilise qu'une
// partie : `dead_code` et `unreachable_pub` sont inevitables et sans interet ici. C'est
// l'idiome usuel pour un `tests/common/`, et la seule exception aux `-D warnings` du
// workspace.
//
// `expect_used` est aussi neutralise, pour une raison moins evidente : les exemptions
// `allow-expect-in-tests` de `clippy.toml` reposent sur `cfg(test)` ou sur `#[test]`, or
// un test d'integration est un binaire ordinaire et ces fonctions de harnais ne portent
// aucun des deux. Dans un harnais de test, echouer bruyamment est le comportement voulu.
#![allow(dead_code, unreachable_pub, clippy::expect_used)]

//! Outillage commun aux tests d'integration du registre.
//!
//! Deux principes, tenus par ces quelques lignes :
//!
//! - **Aucun agent.** On parle au registre par son handle, avec des chemins et des
//!   contenus. Le registre ne touche pas au disque.
//! - **Aucun `sleep`.** Le temps est un [`ManualClock`] qui n'avance que sur ordre,
//!   et la barriere de synchronisation est le `oneshot` de reponse : quand
//!   `admit(...).await` rend la main, le message est traite.

use std::sync::Arc;

use tokio::task::JoinHandle;
use trame_core::clock::{Clock, ManualClock, Timestamp};
use trame_core::{ProjectId, SessionId};
use trame_journal::{Journal, JournalHandle};
use trame_registry::{RegistryHandle, spawn_registry};

/// Un registre pret a l'emploi, avec son horloge manuelle et un journal en memoire.
pub struct Harness {
    pub registry: RegistryHandle,
    pub clock: Arc<ManualClock>,
    pub journal: JournalHandle,
    pub project: ProjectId,
    _joins: Vec<JoinHandle<()>>,
}

impl Harness {
    /// Demarre un registre pour un projet neuf.
    ///
    /// Le journal est en memoire : les tests n'ecrivent jamais dans
    /// `~/Library/Application Support/Trame/`.
    pub fn new() -> Self {
        let project = ProjectId::new();
        let clock = Arc::new(ManualClock::new());
        let journal = Journal::open_in_memory().expect("journal en memoire");
        let (journal, journal_join) = trame_journal::spawn_journal(journal);
        let (registry, registry_join) = spawn_registry(project, clock.clone(), journal.clone());
        Self {
            registry,
            clock,
            journal,
            project,
            _joins: vec![journal_join, registry_join],
        }
    }

    /// Enregistre une session nommee. Le nom sert a l'avis injecte : un UUID ne dit
    /// rien a un agent, « refacto-api » lui dit quelque chose.
    pub async fn session(&self, name: &str) -> SessionId {
        let id = SessionId::new();
        self.registry
            .register_session(id, name)
            .await
            .expect("registre joignable");
        id
    }

    /// L'instant courant selon l'horloge manuelle du harnais.
    pub fn now(&self) -> Timestamp {
        self.clock.now()
    }
}
