//! JSON-RPC 2.0 sur stdio, une enveloppe par line.
//!
//! # Pourquoi stdout appartient au protocole
//!
//! La trame est du JSON delimite par des retours a la line sur les tubes standard du
//! sous-process. Un seul `println!` egare dans ce path corrompt la trame et coupe la
//! communication — c'est pour ca que `print_stdout` est en `deny` dans tout le
//! workspace, et que tous les logs vont sur stderr.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Une enveloppe JSON-RPC entrante, dans sa forme la plus permissive.
///
/// On ne desserialise pas en trois types distincts : la seule facon de savoir si c'est
/// une requete, une reponse ou une notification est de regarder quels champs sont
/// presents.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Incoming {
    /// Present sur les requetes et les reponses, absent sur les notifications.
    #[serde(default)]
    pub id: Option<Value>,
    /// Present sur les requetes et les notifications.
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub params: Option<Value>,
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default)]
    pub error: Option<RpcError>,
}

impl Incoming {
    /// Une requete de l'agent vers nous : elle a un `id` **et** une `method`.
    /// C'est par la que passent `fs/write_text_file` et `session/request_permission`.
    pub(crate) fn as_request(&self) -> Option<(&Value, &str)> {
        match (&self.id, &self.method) {
            (Some(id), Some(method)) => Some((id, method.as_str())),
            _ => None,
        }
    }

    /// Une notification : une `method`, pas d'`id`. C'est par la que passent les
    /// `session/update`.
    pub(crate) fn as_notification(&self) -> Option<&str> {
        match (&self.id, &self.method) {
            (None, Some(method)) => Some(method.as_str()),
            _ => None,
        }
    }

    /// La reponse a une requete que nous avions emise.
    pub(crate) fn as_response(&self) -> Option<&Value> {
        match (&self.id, &self.method) {
            (Some(id), None) => Some(id),
            _ => None,
        }
    }
}

/// Une erreur JSON-RPC.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RpcError {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// Une requete sortante.
#[derive(Debug, Serialize)]
pub(crate) struct Request<'a> {
    pub jsonrpc: &'static str,
    pub id: u64,
    pub method: &'a str,
    pub params: Value,
}

impl<'a> Request<'a> {
    pub(crate) fn new(id: u64, method: &'a str, params: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            method,
            params,
        }
    }
}

/// Une reponse sortante, en succes.
#[derive(Debug, Serialize)]
pub(crate) struct Response {
    pub jsonrpc: &'static str,
    pub id: Value,
    pub result: Value,
}

/// Une reponse sortante, en erreur.
#[derive(Debug, Serialize)]
pub(crate) struct ErrorResponse {
    pub jsonrpc: &'static str,
    pub id: Value,
    pub error: RpcError,
}

impl Response {
    pub(crate) fn new(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result,
        }
    }
}

impl ErrorResponse {
    /// Code -32603 « Internal error » : le seul dont on ait besoin pour signaler a
    /// l'agent qu'un de ses appels d'outil a echoue.
    pub(crate) fn internal(id: Value, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            error: RpcError {
                code: -32603,
                message: message.into(),
                data: None,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(line: &str) -> Incoming {
        serde_json::from_str(line).unwrap()
    }

    #[test]
    fn une_requete_a_un_id_et_une_methode() {
        let msg = parse(r#"{"jsonrpc":"2.0","id":7,"method":"fs/write_text_file","params":{}}"#);
        let (id, method) = msg.as_request().unwrap();
        assert_eq!(id, &serde_json::json!(7));
        assert_eq!(method, "fs/write_text_file");
        assert!(msg.as_notification().is_none());
        assert!(msg.as_response().is_none());
    }

    #[test]
    fn une_notification_n_a_pas_d_id() {
        let msg = parse(r#"{"jsonrpc":"2.0","method":"session/update","params":{}}"#);
        assert_eq!(msg.as_notification().unwrap(), "session/update");
        assert!(msg.as_request().is_none());
    }

    #[test]
    fn une_reponse_a_un_id_sans_methode() {
        let msg = parse(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
        assert_eq!(msg.as_response().unwrap(), &serde_json::json!(1));
        assert!(msg.result.is_some());
    }

    #[test]
    fn une_reponse_en_erreur_se_lit_aussi() {
        let msg = parse(r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32603,"message":"boom"}}"#);
        assert!(msg.as_response().is_some());
        assert_eq!(msg.error.unwrap().message, "boom");
    }

    #[test]
    fn un_id_textuel_est_accepte() {
        // La specification autorise les identifiants chaine. Les supposer numeriques
        // marcherait avec l'implementation actuelle et casserait a la premiere autre.
        let msg = parse(r#"{"jsonrpc":"2.0","id":"abc","method":"fs/read_text_file"}"#);
        assert_eq!(msg.as_request().unwrap().0, &serde_json::json!("abc"));
    }
}
