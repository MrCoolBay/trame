//! Erreurs du journal.

use std::path::PathBuf;

use thiserror::Error;

/// Alias local.
pub type Result<T, E = JournalError> = std::result::Result<T, E>;

/// Ce qui peut echouer cote journal.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum JournalError {
    /// Impossible de determiner l'emplacement de la base.
    #[error("emplacement de la base introuvable : variable {0} absente")]
    NoHome(&'static str),

    /// Le repertoire de support de l'application n'a pas pu etre cree.
    #[error("creation du repertoire {path} impossible")]
    CreateDir {
        /// Le repertoire vise.
        path: PathBuf,
        /// La cause.
        #[source]
        source: std::io::Error,
    },

    /// Erreur SQLite. Couvre l'ouverture, les migrations et les requetes.
    #[error("erreur SQLite")]
    Sqlite(#[from] rusqlite::Error),

    /// Une line relue ne se decode pas dans les types attendus. Signe d'une
    /// migration manquante ou d'une ecriture faite hors de ce crate.
    #[error("line illisible dans {table}.{column} : {value}")]
    Decode {
        /// La table concernee.
        table: &'static str,
        /// La colonne concernee.
        column: &'static str,
        /// La valeur fautive, telle que lue.
        value: String,
    },

    /// Un numero de sequence ne tient pas dans un entier SQLite. Inatteignable en
    /// pratique ; on prefere l'erreur au cast silencieux.
    #[error("numero de sequence hors bornes : {0}")]
    SeqOutOfRange(u64),
}

/// L'acteur du journal n'est plus joignable.
///
/// Erreur unique des methodes de [`crate::JournalHandle`] : le canal est ferme, donc
/// la tache est morte. Rien d'autre ne peut echouer cote appelant — une erreur
/// d'ecriture SQLite est journalisee par l'acteur et comptee dans
/// [`crate::FlushReport::errors`], elle ne remonte pas a chaque appel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("le journal n'est plus joignable")]
pub struct JournalGone;
