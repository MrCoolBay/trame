//! Erreurs du registre.

use thiserror::Error;

/// L'acteur du registre n'est plus joignable.
///
/// **Erreur unique** des methodes de [`crate::RegistryHandle`] : le canal est ferme,
/// donc la tache est morte. Rien d'autre ne peut echouer cote appelant — une admission
/// rend toujours un verdict, et un echec de journalisation est trace par l'acteur sans
/// remonter, parce qu'il ne change pas le verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("le registre n'est plus joignable")]
pub struct RegistryGone;
