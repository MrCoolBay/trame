//! Le Supervisor et le cablage.
//!
//! # Hierarchie
//!
//! ```text
//! Workspace (l'application)
//!  └── Project (un dossier + un depot git)
//!       ├── Working directory unique
//!       ├── Write Registry dedie
//!       ├── Branches virtuelles
//!       └── Session (un agent + un objectif)
//! ```
//!
//! # Par projet contre global
//!
//! | Par projet | Global |
//! |---|---|
//! | Write Registry (un acteur) | Journal SQLite unique (colonne `project_id`) |
//! | Compteur de sequence | Reservations de ressources (ports, bases de dev) |
//! | Working directory, backend VCS | Budget de concurrence (CPU, RAM) |
//! | Watcher FSEvents | Quotas et rate limits API |
//! | Branches virtuelles, config agent | Identifiants dans le Keychain |
//!
//! Le point subtil : **les reservations de ressources sont globales**. Le port
//! 3000 est machine-wide. Deux projets qui lancent chacun leur dev server, c'est
//! le premier vrai conflit inter-projets.
//!
//! # Le parallelisme se fait par projets, pas par sessions
//!
//! Deux sessions dans deux projets differents ne peuvent physiquement pas entrer
//! en collision : repertoires, depots et index distincts. `5 projets × 3 sessions
//! = 15 agents` sans jamais sortir du point de fonctionnement sur.
//!
//! # La chaine complete (phase 3)
//!
//! [`SessionPilot`] est le point ou tout se rencontre.

pub mod session;

pub use session::{SessionActivity, SessionPilot};

/// Le nombre de sessions par projet au-dela duquel Trame n'a pas ete concu.
///
/// Ce n'est pas une limite technique appliquee par le code, c'est un cadrage
/// produit : tout ce qui ne sert qu'au-dela est hors scope. Le parallelisme
/// s'obtient en ajoutant des projets.
pub const SESSIONS_PER_PROJECT_TARGET: u8 = 5;
