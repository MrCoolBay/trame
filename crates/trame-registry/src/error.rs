//! Erreurs du registre.

use std::path::PathBuf;

use thiserror::Error;

/// L'acteur du registre n'est plus joignable.
///
/// Le canal est ferme, donc la tache est morte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("le registre n'est plus joignable")]
pub struct RegistryGone;

/// Ce qui peut echouer a l'admission.
///
/// L'admission **inclut l'ecriture** (ADR 0014), donc elle peut echouer. Un verdict rendu
/// sans que l'ecriture ait eu lieu serait un mensonge : l'appelant repondrait « admis » a
/// l'agent, qui croirait son file ecrit.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum RegistryError {
    /// Le registre n'est plus joignable.
    #[error(transparent)]
    Gone(#[from] RegistryGone),

    /// Le path sort du repertoire de travail du projet. **Toujours refuse** : le
    /// registre ne peut rien garantir sur ce qu'il ne voit pas, et une ecriture hors du
    /// projet n'a aucune raison de passer par lui.
    #[error("path hors du repertoire de travail du projet : {0}")]
    PathOutsideProject(PathBuf),

    /// L'ecriture sur disque a echoue.
    ///
    /// L'state du registre n'a **pas** ete mis a jour : sinon il croirait le file
    /// modifie et perimerait a tort les lectures des autres sessions.
    #[error("ecriture de {path} impossible")]
    Write {
        /// Le path vise, relatif a la root du projet.
        path: PathBuf,
        /// La cause.
        #[source]
        source: std::io::Error,
    },
}
