//! Le cote daemon des hooks de la CLI. **C'est ici que la politique vit.**
//!
//! `trame-hook` ne decide rien : il transporte le payload et rend la reponse (ADR 0025). Toute
//! la politique est ici, a un seul endroit, avec le registre a portee.
//!
//! # Deux hooks, deux roles opposes
//!
//! | Hook | Outil | Ce qu'on fait |
//! |---|---|---|
//! | `PreToolUse` | `Bash` | **Deny** une redirection vers un file du projet (ADR 0026) |
//! | `PostToolUse` | `Grep`, `Glob` | **Enregistrer en shadow** les fichiers lus. On ne refuse jamais |
//!
//! Sur `Bash` on refuse parce qu'on ne peut pas savoir ce qu'une commande ecrit, et qu'on prefere
//! ramener le trou dans le perimetre de l'admission. Sur `Grep` c'est l'inverse : deny
//! priverait l'agent de recherche, ce qui le degraderait sur un vrai codebase — on enregistre.
//!
//! # ★ L'empreinte ne vient jamais du payload
//!
//! Invariant 10, et ADR 0020. Le hook rapporte des **chemins** ; Trame **relit le file** pour
//! l'empreinter. Empreinter le payload d'un hook produirait une empreinte qui ne correspond a
//! aucun state du disque — et l'echec serait totalement silencieux : read-set peuple, `StaleRead`
//! mort, aucun test casse.
//!
//! # Les deux formes de chemins, mesurees et non supposees
//!
//! La [sonde 3](../../../docs/sondes/2026-08-12-postooluse.md) a releve que les deux outils ne
//! s'accordent pas :
//!
//! - **`Grep`** rend des chemins **relatifs au `cwd`**, y compris quand l'appel porte un
//!   argument `path` : `path: "sub"` donne `sub/deep.rs`, pas `deep.rs`.
//! - **`Glob`** rend des chemins **absolus et resolus** : `/private/tmp/…`, pas `/tmp/…`.
//!
//! Les deux passent par [`trame_core::ProjectRoot`], qui absorbe la resolution `/private/var`
//! contre `/var`. Un test epingle les deux formes : sans lui, la regression passerait inapercue
//! puisque chaque outil, seul, aurait l'air de marcher.

use std::path::PathBuf;

use serde::Deserialize;
use trame_core::{ProjectRoot, SessionId};
use trame_registry::RegistryHandle;

/// Ce que le daemon rend au hook.
///
/// Volontairement pauvre, et symetrique de `trame_hook::Decision` : `{"decision":"silence"}` ou
/// `{"decision":"refus","reason":"…"}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Response {
    /// Rien a dire. Le hook n'ecrit rien, la CLI poursuit.
    Silence,
    /// Deny, avec le reason transmis a l'agent.
    Deny(String),
}

impl Response {
    /// La line JSON envoyee sur la socket.
    #[must_use]
    pub fn to_line(&self) -> String {
        match self {
            Self::Silence => "{\"decision\":\"silence\"}\n".to_owned(),
            Self::Deny(reason) => {
                let body = serde_json::json!({ "decision": "refus", "reason": reason });
                format!("{body}\n")
            }
        }
    }
}

/// Ce qu'un traitement de hook a produit, pour l'interface et les tests.
///
/// **Rend visible ce qui n'a PAS ete enregistre.** Un compteur qui ne compte que les succes
/// laisse croire a une couverture qu'on n'a pas.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Report {
    /// Les fichiers dont la lecture est entree dans le registre.
    pub recorded: Vec<PathBuf>,
    /// Les chemins rapportes mais **non recorded**, avec le reason.
    ///
    /// Trois causes : hors du projet, illisible, ou au-dela de la limite. Aucune n'est
    /// silencieuse (ADR 0021 : un angle mort compte et affiche).
    pub skipped: Vec<(String, &'static str)>,
    /// Vrai si l'appel etait un `Grep` en mode `content` ou `count`, dont les chemins ne sont
    /// **pas** dans `filenames`. Angle mort assume, jamais reconstruit par parsing (ADR 0021).
    pub blind_mode: bool,
}

/// L'enveloppe d'un hook, telle qu'observee en sondes 2 et 3.
///
/// Les cles viennent de l'observation, pas d'un typage : la sonde 2 a trouve `permission_mode`
/// absent du type que la sonde 1 citait.
#[derive(Debug, Deserialize)]
pub struct Payload {
    /// `PreToolUse` ou `PostToolUse`.
    pub hook_event_name: String,
    /// Le nom de l'outil. `Bash`, `Grep`, `Glob`, `mcp__acp__Read`…
    pub tool_name: String,
    /// Les parametres de l'appel.
    #[serde(default)]
    pub tool_input: serde_json::Value,
    /// La sortie de l'outil. Presente en `PostToolUse` uniquement.
    #[serde(default)]
    pub tool_response: serde_json::Value,
}

/// ★ Traite un payload de hook. **Le point unique de decision.**
///
/// # Erreurs
///
/// Ne rend jamais d'erreur : un payload qu'on ne sait pas handle donne [`Response::Silence`].
/// Le refus par defaut est la regle du **hook** quand il ne peut pas nous joindre (ADR 0025) ;
/// ici, joints, nous n'avons pas de raison de deny ce que nous ne reconnaissons pas.
pub async fn handle(
    payload: &Payload,
    root: &ProjectRoot,
    registry: &RegistryHandle,
    session: SessionId,
    limit: usize,
) -> (Response, Report) {
    match (payload.hook_event_name.as_str(), payload.tool_name.as_str()) {
        ("PreToolUse", "Bash") => (bash(payload), Report::default()),
        ("PostToolUse", "Grep" | "Glob") => {
            let bilan = record_reads(payload, root, registry, session, limit).await;
            (Response::Silence, bilan)
        }
        _ => (Response::Silence, Report::default()),
    }
}

/// La politique `Bash` : un seul reason (ADR 0026).
fn bash(payload: &Payload) -> Response {
    let Some(commande) = payload
        .tool_input
        .get("command")
        .and_then(serde_json::Value::as_str)
    else {
        return Response::Silence;
    };
    match trame_hook::bash::evaluate(commande) {
        trame_hook::bash::Verdict::Allow => Response::Silence,
        trame_hook::bash::Verdict::Deny { target } => {
            tracing::info!(%target, "ecriture par le shell refusee");
            Response::Deny(trame_hook::bash::reason(&target))
        }
    }
}

/// Enregistre les fichiers rapportes par `Grep` ou `Glob`.
///
/// **`filenames` uniquement.** En mode `content` et `count`, la key est vide et les chemins
/// n'existent que dans la chaine de sortie : angle mort assume, compte et affiche, jamais
/// reconstruit par parsing (ADR 0021).
async fn record_reads(
    payload: &Payload,
    root: &ProjectRoot,
    registry: &RegistryHandle,
    session: SessionId,
    limit: usize,
) -> Report {
    let mut bilan = Report::default();

    let mode = payload
        .tool_response
        .get("mode")
        .and_then(serde_json::Value::as_str);
    // `Glob` n'a pas de `mode` ; `Grep` en a un, et seul `files_with_matches` peuple `filenames`.
    bilan.blind_mode = matches!(mode, Some("content" | "count"));

    let Some(filenames) = payload
        .tool_response
        .get("filenames")
        .and_then(serde_json::Value::as_array)
    else {
        return bilan;
    };

    for (index, entry) in filenames.iter().enumerate() {
        let Some(brut) = entry.as_str() else {
            continue;
        };
        if index >= limit {
            // Jamais de troncature muette : ce qui est laisse de cote est nomme.
            bilan
                .skipped
                .push((brut.to_owned(), "au-dela de la limite"));
            continue;
        }
        // ★ Les deux formes de chemins. `Grep` rend du relatif au cwd, `Glob` de l'absolu.
        let absolu = if std::path::Path::new(brut).is_absolute() {
            PathBuf::from(brut)
        } else {
            root.resolve(std::path::Path::new(brut))
        };
        let Ok(key) = root.relativize(&absolu) else {
            bilan.skipped.push((brut.to_owned(), "hors du projet"));
            continue;
        };
        // ★ On RELIT le file. L'empreinte ne vient jamais du payload (invariant 10).
        let Ok(content) = tokio::fs::read_to_string(&absolu).await else {
            // Disparu entre la recherche et maintenant : cas normal, et il faut le dire.
            bilan.skipped.push((brut.to_owned(), "illisible"));
            continue;
        };
        // ★ **Mode shadow.** La lecture est enregistree dans un read-set parallele qui ne
        // participe a aucun verdict : elle compte ce qu'on aurait dit, et ne dit rien
        // (ADR 0027). `filenames.len()` accompagne chaque entree — c'est cette taille qui
        // rendra le seuil decidable APRES la mesure, au lieu d'etre choisi a l'intuition.
        if registry
            .record_shadow_read(session, key.clone(), content, filenames.len())
            .await
            .is_ok()
        {
            bilan.recorded.push(key);
        } else {
            bilan.skipped.push((brut.to_owned(), "registre arrete"));
        }
    }
    bilan
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(json: &str) -> Payload {
        serde_json::from_str(json).expect("payload de test")
    }

    /// La politique Bash est appliquee, et son reason traverse.
    #[test]
    fn un_bash_qui_redirige_dans_le_projet_est_refuse() {
        let p = payload(
            r#"{"hook_event_name":"PreToolUse","tool_name":"Bash",
                "tool_input":{"command":"echo x > notes.txt"}}"#,
        );
        let Response::Deny(reason) = bash(&p) else {
            panic!("une redirection vers le projet doit etre refusee");
        };
        assert!(reason.contains("notes.txt"), "{reason}");
    }

    /// Et ce qui n'ecrit pas dans le projet passe — la portee du registre, pas une exception.
    #[test]
    fn un_bash_hors_du_projet_passe() {
        for commande in [
            "ls -la 2>/dev/null",
            "just tui 2>/tmp/tui.log",
            "grep -rn x .",
        ] {
            let p = payload(&format!(
                r#"{{"hook_event_name":"PreToolUse","tool_name":"Bash",
                     "tool_input":{{"command":"{commande}"}}}}"#
            ));
            assert_eq!(bash(&p), Response::Silence, "commande : {commande}");
        }
    }

    /// Le mode `content` est reconnu comme angle mort, et il est **compte**.
    #[test]
    fn le_mode_content_est_signale_comme_aveugle() {
        let p = payload(
            r#"{"hook_event_name":"PostToolUse","tool_name":"Grep",
                "tool_input":{"pattern":"x","output_mode":"content"},
                "tool_response":{"mode":"content","numFiles":0,"filenames":[],
                                 "content":"a.rs:1:x","numLines":1}}"#,
        );
        let mode = p
            .tool_response
            .get("mode")
            .and_then(serde_json::Value::as_str);
        assert_eq!(mode, Some("content"));
        assert!(matches!(mode, Some("content" | "count")));
    }

    /// La reponse serialisee est celle que `trame-hook` sait lire.
    #[test]
    fn les_deux_reponses_sont_lisibles_par_le_hook() {
        assert_eq!(
            Response::Silence.to_line().trim(),
            r#"{"decision":"silence"}"#
        );
        let line = Response::Deny("reason".to_owned()).to_line();
        let parsed: serde_json::Value = serde_json::from_str(line.trim()).expect("JSON");
        assert_eq!(parsed["decision"], "refus");
        assert_eq!(parsed["reason"], "reason");
    }
}
