//! L'application desktop Trame. **Elle observe, elle ne pilote pas.**
//!
//! # Perimetre v0.1, identique a la TUI
//!
//! - un panneau par session, avec son etat — `Idle` / `Thinking` / `Writing`
//! - le flux d'evenements en direct, verdicts mis en evidence
//! - `StaleRead` distinct de `Clean` **sur deux axes** : couleur et marqueur
//! - la distinction **admis / observe** visible
//! - un indicateur de degradation quand `can_intercept_writes` est faux
//!
//! Pas de multi-projet, pas de branches, pas de diffs, pas de configuration.
//!
//! # Pourquoi elle ne peut structurellement pas piloter
//!
//! Elle ne recoit qu'un `Receiver<Observation>`. Elle ne tient aucun `RegistryHandle`, donc
//! `admit` ne lui est pas accessible — c'est du typage, pas une convention de revue
//! ([ADR 0022](https://github.com/mrcoolbay/trame/blob/main/docs/adr/0022-decoupage-daemon-gui.md)).
//!
//! L'etat d'affichage et l'ouverture d'un projet viennent de [`trame_view`], partages avec la
//! TUI : ces proprietes ont un seul domicile.

pub mod theme;
pub mod vue;
