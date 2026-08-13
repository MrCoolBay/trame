//! ★ **Le coeur du produit.** Le controleur d'admission en ecriture.
//!
//! Un acteur tokio **par projet**. Il possede son state ; personne ne le partage.
//!
//! # Ce n'est pas un systeme de verrous
//!
//! Le locking pessimiste est inadapte, pour trois raisons dont chacune suffit : les
//! agents tiennent leur transaction pendant des minutes, ils ne declarent pas leur
//! intention a l'avance, et bloquer un tool call en vol declenche des timeouts cote
//! harness. Le modele est celui des bases de donnees — **controle de concurrence
//! optimiste avec validation du read-set**.
//!
//! # Pourquoi valider les lectures et pas seulement les ecritures
//!
//! Le mode d'echec le plus frequent a trois agents ne produit **aucune collision
//! d'ecriture** :
//!
//! ```text
//! 1. Session A lit auth.rs, memorise la signature de verify_token()
//! 2. Session B ecrit auth.rs, renomme verify_token() -> validate_token()
//! 3. Session A ecrit handlers.rs, appelle verify_token()
//!
//! -> Deux fichiers differents. Un verrou par file ne voit rien.
//! -> L'arbre est casse.
//! ```
//!
//! On ne sait pas *si* ca casse. On sait que **A raisonne sur un monde qui n'existe
//! plus**, et c'est le seul invariant qui compte.
//!
//! # Regles de la v0.1
//!
//! - **Granularite file entier.** Pas de tracked de hunks : file plus fenetre
//!   temporelle donne 90 % de la valeur pour 5 % du travail (ADR 0012).
//! - **Read-set filter** aux lectures substantielles — voir [`ReadKind`]. Sinon le
//!   read-set explose et tout devient `StaleRead`.
//! - **Decroissance a [`READ_SET_TTL`]**, dix minutes.
//! - Compteur de sequence **par projet**, jamais global.
//! - blake3 a l'admission et a la lecture. Jamais l'arbre entier.
//! - [`trame_core::Verdict::DisjointWrite`] et [`trame_core::Verdict::Overlap`] ne sont
//!   **jamais produits** : les variantes existent, la logique attend la v0.4.
//!
//! **Rien n'est bloque.** Le registre observe, journalise et informe. Le blocage viendra
//! apres mesure du taux reel de faux positifs — un outil qui crie au loup est desactive
//! en une semaine.
//!
//! # Architecture
//!
//! - `state` (prive) — la logique, **pure et synchrone**. Testable sans runtime, sans
//!   agent et sans base. Prive a dessein : le verdict se demande a l'acteur, jamais a
//!   l'state directement.
//! - [`spawn_registry`] / [`RegistryHandle`] — l'acteur qui la possede.
//!
//! # Exemple
//!
//! ```no_run
//! # async fn demo() -> Result<(), Box<dyn std::error::Error>> {
//! use std::sync::Arc;
//! use trame_core::{ProjectId, ProjectRoot, SessionId, clock::SystemClock};
//! use trame_journal::{Journal, spawn_journal};
//! use trame_registry::{ReadKind, spawn_registry};
//!
//! let (journal, _j) = spawn_journal(Journal::open_default()?);
//! let root = ProjectRoot::new("/path/vers/projet")?;
//! let (registry, _r) =
//!     spawn_registry(ProjectId::new(), root, Arc::new(SystemClock), journal);
//!
//! let session = SessionId::new();
//! registry.register_session(session, "refacto-api").await?;
//! registry.record_read(session, "auth.rs", "fn verify_token()", ReadKind::FullFile).await?;
//!
//! let verdict = registry.admit(session, "handlers.rs", "verify_token()").await?;
//! if verdict.needs_notice() {
//!     // L'avis est injecte via `trame_core::StaleReadNotice`.
//! }
//! # Ok(())
//! # }
//! ```

mod actor;
mod error;
mod msg;
mod state;

pub use actor::{RegistryHandle, spawn_registry};
pub use error::{RegistryError, RegistryGone};
pub use msg::{
    ExternalWrite, FileSnapshot, ReadKind, RegistrySnapshot, SessionSnapshot, ShadowStats,
};
pub use state::READ_SET_TTL;
