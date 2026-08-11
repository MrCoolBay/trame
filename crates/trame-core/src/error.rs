//! Erreurs du crate fondation.
//!
//! Regle du projet : `thiserror` dans les bibliotheques, `anyhow` uniquement
//! dans les binaires. Une bibliotheque qui renvoie `anyhow::Error` force son
//! appelant a faire du pattern matching sur des chaines de caracteres.

use std::path::PathBuf;

use thiserror::Error;

/// Alias local. `Result<T>` suffit dans tout le crate.
pub type Result<T, E = CoreError> = std::result::Result<T, E>;

/// Erreurs communes aux coutures de `trame-core`.
///
/// Les implementations concretes des traits ([`crate::Forge`], [`crate::TaskSource`])
/// vivent dans d'autres crates et ont leurs propres erreurs : elles les
/// remontent via [`CoreError::Backend`] plutot que d'imposer leurs variantes ici.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CoreError {
    /// Un chemin sort du working directory du projet. Toujours refuse : le
    /// registre ne peut rien garantir sur ce qu'il ne voit pas.
    #[error("chemin hors du repertoire de travail du projet : {0}")]
    PathOutsideProject(PathBuf),

    /// Une entite reclamee n'existe pas.
    #[error("{what} introuvable : {id}")]
    NotFound {
        /// La nature de l'entite : « session », « projet », « work item ».
        what: &'static str,
        /// Son identifiant, tel que fourni.
        id: String,
    },

    /// Le backend ne sait pas faire. Cas typique : `PtyBackend` a qui on demande
    /// d'intercepter une ecriture. **A afficher a l'utilisateur** plutot qu'a
    /// avaler : il doit savoir qu'il tourne en mode degrade.
    #[error("non disponible sur ce backend : {0}")]
    Unsupported(&'static str),

    /// Erreur remontee par une implementation concrete.
    #[error("erreur de backend : {0}")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync + 'static>),
}

impl CoreError {
    /// Emballe l'erreur d'un backend.
    pub fn backend(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Backend(Box::new(source))
    }
}
