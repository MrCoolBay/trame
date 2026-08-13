//! L'interface Trame en terminal, en bibliotheque.
//!
//! Le binaire n'est qu'un point d'entree : le rendu vit ici pour etre **testable sans
//! terminal**. Les proprietes a garantir — un `StaleRead` distinct d'un `Clean`, une ecriture
//! observee jamais presentee comme admise — sont verifiees par des tests d'integration qui
//! rendent dans un `TestBackend` et relisent le buffer.
//!
//! Un rendu qu'on ne peut pas relire est un rendu dont on affirme les proprietes sans les
//! mesurer.
//!
//! L'state d'affichage et l'ouverture d'un projet vivent dans [`trame_view`], partages avec la
//! GUI : ces proprietes ont un seul domicile, et cette crate-ci ne contient que la mise en
//! forme terminal.

pub mod run;
pub mod ui;

/// L'state d'affichage, partage avec la GUI.
pub use trame_view as app;
/// L'ouverture d'un projet — journal, registre, watcher — partagee avec la GUI.
pub use trame_view::source;
