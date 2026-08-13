//! Le cote daemon des hooks de la CLI. **C'est ici que la politique vit.**
//!
//! `trame-hook` ne decide rien : il transporte le payload et rend la reponse (ADR 0025). Toute
//! la politique est ici, a un seul endroit, avec le registre a portee.
//!
//! # Deux hooks, deux roles opposes
//!
//! | Hook | Outil | Ce qu'on fait |
//! |---|---|---|
//! | `PreToolUse` | `Bash` | **Refuser** une redirection vers un fichier du projet (ADR 0026) |
//! | `PostToolUse` | `Grep`, `Glob` | **Enregistrer en ombre** les fichiers lus. On ne refuse jamais |
//!
//! Sur `Bash` on refuse parce qu'on ne peut pas savoir ce qu'une commande ecrit, et qu'on prefere
//! ramener le trou dans le perimetre de l'admission. Sur `Grep` c'est l'inverse : refuser
//! priverait l'agent de recherche, ce qui le degraderait sur un vrai codebase — on enregistre.
//!
//! # ★ L'empreinte ne vient jamais du payload
//!
//! Invariant 10, et ADR 0020. Le hook rapporte des **chemins** ; Trame **relit le fichier** pour
//! l'empreinter. Empreinter le payload d'un hook produirait une empreinte qui ne correspond a
//! aucun etat du disque — et l'echec serait totalement silencieux : read-set peuple, `StaleRead`
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
/// `{"decision":"refus","motif":"…"}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reponse {
    /// Rien a dire. Le hook n'ecrit rien, la CLI poursuit.
    Silence,
    /// Refus, avec le motif transmis a l'agent.
    Refus(String),
}

impl Reponse {
    /// La ligne JSON envoyee sur la socket.
    #[must_use]
    pub fn en_ligne(&self) -> String {
        match self {
            Self::Silence => "{\"decision\":\"silence\"}\n".to_owned(),
            Self::Refus(motif) => {
                let valeur = serde_json::json!({ "decision": "refus", "motif": motif });
                format!("{valeur}\n")
            }
        }
    }
}

/// Ce qu'un traitement de hook a produit, pour l'interface et les tests.
///
/// **Rend visible ce qui n'a PAS ete enregistre.** Un compteur qui ne compte que les succes
/// laisse croire a une couverture qu'on n'a pas.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Bilan {
    /// Les fichiers dont la lecture est entree dans le registre.
    pub enregistres: Vec<PathBuf>,
    /// Les chemins rapportes mais **non enregistres**, avec le motif.
    ///
    /// Trois causes : hors du projet, illisible, ou au-dela de la borne. Aucune n'est
    /// silencieuse (ADR 0021 : un angle mort compte et affiche).
    pub ignores: Vec<(String, &'static str)>,
    /// Vrai si l'appel etait un `Grep` en mode `content` ou `count`, dont les chemins ne sont
    /// **pas** dans `filenames`. Angle mort assume, jamais reconstruit par parsing (ADR 0021).
    pub mode_aveugle: bool,
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
/// Ne rend jamais d'erreur : un payload qu'on ne sait pas traiter donne [`Reponse::Silence`].
/// Le refus par defaut est la regle du **hook** quand il ne peut pas nous joindre (ADR 0025) ;
/// ici, joints, nous n'avons pas de raison de refuser ce que nous ne reconnaissons pas.
pub async fn traiter(
    payload: &Payload,
    root: &ProjectRoot,
    registry: &RegistryHandle,
    session: SessionId,
    borne: usize,
) -> (Reponse, Bilan) {
    match (payload.hook_event_name.as_str(), payload.tool_name.as_str()) {
        ("PreToolUse", "Bash") => (bash(payload), Bilan::default()),
        ("PostToolUse", "Grep" | "Glob") => {
            let bilan = enregistrer_lectures(payload, root, registry, session, borne).await;
            (Reponse::Silence, bilan)
        }
        _ => (Reponse::Silence, Bilan::default()),
    }
}

/// La politique `Bash` : un seul motif (ADR 0026).
fn bash(payload: &Payload) -> Reponse {
    let Some(commande) = payload
        .tool_input
        .get("command")
        .and_then(serde_json::Value::as_str)
    else {
        return Reponse::Silence;
    };
    match trame_hook::bash::evaluer(commande) {
        trame_hook::bash::Verdict::Laisser => Reponse::Silence,
        trame_hook::bash::Verdict::Refuser { cible } => {
            tracing::info!(%cible, "ecriture par le shell refusee");
            Reponse::Refus(trame_hook::bash::motif(&cible))
        }
    }
}

/// Enregistre les fichiers rapportes par `Grep` ou `Glob`.
///
/// **`filenames` uniquement.** En mode `content` et `count`, la cle est vide et les chemins
/// n'existent que dans la chaine de sortie : angle mort assume, compte et affiche, jamais
/// reconstruit par parsing (ADR 0021).
async fn enregistrer_lectures(
    payload: &Payload,
    root: &ProjectRoot,
    registry: &RegistryHandle,
    session: SessionId,
    borne: usize,
) -> Bilan {
    let mut bilan = Bilan::default();

    let mode = payload
        .tool_response
        .get("mode")
        .and_then(serde_json::Value::as_str);
    // `Glob` n'a pas de `mode` ; `Grep` en a un, et seul `files_with_matches` peuple `filenames`.
    bilan.mode_aveugle = matches!(mode, Some("content" | "count"));

    let Some(filenames) = payload
        .tool_response
        .get("filenames")
        .and_then(serde_json::Value::as_array)
    else {
        return bilan;
    };

    for (index, valeur) in filenames.iter().enumerate() {
        let Some(brut) = valeur.as_str() else {
            continue;
        };
        if index >= borne {
            // Jamais de troncature muette : ce qui est laisse de cote est nomme.
            bilan.ignores.push((brut.to_owned(), "au-dela de la borne"));
            continue;
        }
        // ★ Les deux formes de chemins. `Grep` rend du relatif au cwd, `Glob` de l'absolu.
        let absolu = if std::path::Path::new(brut).is_absolute() {
            PathBuf::from(brut)
        } else {
            root.resolve(std::path::Path::new(brut))
        };
        let Ok(cle) = root.relativize(&absolu) else {
            bilan.ignores.push((brut.to_owned(), "hors du projet"));
            continue;
        };
        // ★ On RELIT le fichier. L'empreinte ne vient jamais du payload (invariant 10).
        let Ok(contenu) = tokio::fs::read_to_string(&absolu).await else {
            // Disparu entre la recherche et maintenant : cas normal, et il faut le dire.
            bilan.ignores.push((brut.to_owned(), "illisible"));
            continue;
        };
        // ★ **Mode ombre.** La lecture est enregistree dans un read-set parallele qui ne
        // participe a aucun verdict : elle compte ce qu'on aurait dit, et ne dit rien
        // (ADR 0027). `filenames.len()` accompagne chaque entree — c'est cette taille qui
        // rendra le seuil decidable APRES la mesure, au lieu d'etre choisi a l'intuition.
        if registry
            .record_shadow_read(session, cle.clone(), contenu, filenames.len())
            .await
            .is_ok()
        {
            bilan.enregistres.push(cle);
        } else {
            bilan.ignores.push((brut.to_owned(), "registre arrete"));
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

    /// La politique Bash est appliquee, et son motif traverse.
    #[test]
    fn un_bash_qui_redirige_dans_le_projet_est_refuse() {
        let p = payload(
            r#"{"hook_event_name":"PreToolUse","tool_name":"Bash",
                "tool_input":{"command":"echo x > notes.txt"}}"#,
        );
        let Reponse::Refus(motif) = bash(&p) else {
            panic!("une redirection vers le projet doit etre refusee");
        };
        assert!(motif.contains("notes.txt"), "{motif}");
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
            assert_eq!(bash(&p), Reponse::Silence, "commande : {commande}");
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
            Reponse::Silence.en_ligne().trim(),
            r#"{"decision":"silence"}"#
        );
        let ligne = Reponse::Refus("motif".to_owned()).en_ligne();
        let valeur: serde_json::Value = serde_json::from_str(ligne.trim()).expect("JSON");
        assert_eq!(valeur["decision"], "refus");
        assert_eq!(valeur["motif"], "motif");
    }
}
