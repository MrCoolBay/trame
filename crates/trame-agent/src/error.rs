//! Erreurs du transport agent.

use thiserror::Error;

/// Ce qui peut echouer cote transport.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum AgentError {
    /// Le sous-process n'a pas pu etre lance. Cas courant : l'adaptateur ACP n'est pas
    /// installe.
    #[error("impossible de lancer le harness `{command}`")]
    Spawn {
        /// La commande tentee.
        command: String,
        /// La cause.
        #[source]
        source: std::io::Error,
    },

    /// I/O sur les tubes du sous-process.
    #[error("erreur d'entree-sortie avec le harness")]
    Io(#[from] std::io::Error),

    /// La trame JSON-RPC est illisible.
    #[error("trame JSON-RPC invalide")]
    Protocol(#[from] serde_json::Error),

    /// L'agent a repondu une erreur JSON-RPC.
    #[error("le harness a repondu une erreur ({code}) : {message}")]
    Rpc {
        /// Le code JSON-RPC.
        code: i64,
        /// Le message.
        message: String,
    },

    /// L'agent a repondu quelque chose d'inattendu la ou un champ etait requis.
    #[error("reponse inattendue du harness sur {method} : {detail}")]
    Unexpected {
        /// La methode appelee.
        method: &'static str,
        /// Ce qui manquait ou ne collait pas.
        detail: String,
    },

    /// Le sous-process est mort, ou la tache de transport s'est arretee.
    #[error("le harness n'est plus joignable")]
    Gone,

    /// L'agent demande une authentification que Trame ne sait pas fournir.
    ///
    /// A remonter tel quel a l'utilisateur : c'est **son** compte, et le message de
    /// l'agent contient la marche a suivre.
    #[error("authentification requise par le harness : {0}")]
    AuthRequired(String),

    /// Le backend ne sait pas faire. Cas typique : `PtyBackend` a qui on demande
    /// d'intercepter une ecriture. **A afficher**, pas a avaler.
    #[error("non disponible sur ce backend : {0}")]
    Unsupported(&'static str),
}
