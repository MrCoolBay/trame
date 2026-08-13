//! L'application desktop Trame. **Elle observe, elle ne pilot pas.**
//!
//! # Perimetre v0.1, identique a la TUI
//!
//! - un panel par session, avec son state — `Idle` / `Thinking` / `Writing`
//! - le feed d'evenements en direct, verdicts mis en evidence
//! - `StaleRead` distinct de `Clean` **sur deux axes** : color et marker
//! - la distinction **admis / observe** visible
//! - un indicateur de degraded_banner quand `can_intercept_writes` est faux
//!
//! Pas de multi-projet, pas de branches, pas de diffs, pas de configuration.
//!
//! # Pourquoi elle ne peut structurellement pas piloter
//!
//! Elle ne recoit qu'un `Receiver<Observation>`. Elle ne tient aucun `RegistryHandle`, donc
//! `admit` ne lui est pas accessible — c'est du typage, pas une convention de revue
//! ([ADR 0022](https://github.com/mrcoolbay/trame/blob/main/docs/adr/0022-decoupage-daemon-gui.md)).
//!
//! L'state d'affichage et l'ouverture d'un projet viennent de [`trame_view`], partages avec la
//! TUI : ces proprietes ont un seul domicile.

pub mod theme;
pub mod view;
