//! L'interface Trame, en bibliotheque.
//!
//! Le binaire n'est qu'un point d'entree : l'etat et le rendu vivent ici pour etre
//! **testables sans terminal**. Les proprietes a garantir — un `StaleRead` distinct d'un
//! `Clean`, une ecriture observee jamais presentee comme admise — sont verifiees par des
//! tests d'integration qui rendent dans un `TestBackend` et relisent le buffer.
//!
//! Un rendu qu'on ne peut pas relire est un rendu dont on affirme les proprietes sans les
//! mesurer.

pub mod app;
pub mod run;
pub mod source;
pub mod ui;
