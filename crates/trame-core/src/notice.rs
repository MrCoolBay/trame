//! Les variantes de l'avis de lecture perimee.
//!
//! # Pourquoi ce module est parametrable
//!
//! Le code autour de l'avis est de la plomberie. **Le texte est la variable du produit** :
//! c'est lui qui decide si l'agent relit et s'adapte, ou s'il ignore, ou s'il sur-reagit.
//! Aucune quantite de raffinement architectural ne compense un message que les agents
//! rationalisent au lieu de suivre.
//!
//! Ces trois variantes existent pour etre **mesurees**, pas pour offrir un reglage a
//! l'utilisateur. Quand une aura gagne sur du vrai usage, les autres disparaitront.
//!
//! # Ce que chacune fait varier
//!
//! - [`NoticeVariant::Neutral`] — les faits, rien d'autre : le fichier, l'auteur, le
//!   delai. Pas d'ordre. C'est la position de depart du cadrage.
//! - [`NoticeVariant::Directive`] — les memes faits, plus une instruction explicite de
//!   relecture. Teste si l'agent a besoin qu'on lui dise quoi faire.
//! - [`NoticeVariant::Contextual`] — les faits, plus **ce qui a change**. Teste
//!   l'hypothese que l'agent ne suit un avis que s'il comprend de quoi il parle.
//!
//! La variante contextuelle a un cout que les deux autres n'ont pas : `StaleFile` doit
//! porter un resume du changement, donc le registre doit le calculer a l'admission. C'est
//! pourquoi elle est ici en **simulation** — le resume est fourni de l'exterieur, pas
//! encore produit par le registre.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::prompt::{PromptContributor, PromptFragment, SessionContext, humanize};
use crate::verdict::Verdict;

/// Laquelle des trois formulations utiliser.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum NoticeVariant {
    /// Les faits seuls : fichier, auteur, delai. Aucun ordre. C'est le defaut : en cas de
    /// doute, on informe sans ordonner.
    #[default]
    Neutral,
    /// Les faits, plus une instruction explicite de relecture.
    Directive,
    /// Les faits, plus un resume de ce qui a change.
    Contextual,
}

impl NoticeVariant {
    /// Le libelle stable, pour les journaux d'experience.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Neutral => "neutre",
            Self::Directive => "directive",
            Self::Contextual => "contextuelle",
        }
    }

    /// Les trois, dans l'ordre de la manche experimentale.
    #[must_use]
    pub fn all() -> &'static [Self] {
        &[Self::Neutral, Self::Directive, Self::Contextual]
    }
}

/// Un contributeur d'avis dont la formulation est parametrable.
///
/// Remplace [`crate::StaleReadNotice`] pendant la manche experimentale. Une fois la
/// variante tranchee, l'un des deux disparaitra.
#[derive(Debug, Clone, Default)]
pub struct ConfigurableNotice {
    variant: NoticeVariant,
    /// Resume du changement, par chemin. Alimente de l'exterieur tant que le registre ne
    /// le calcule pas : c'est precisement ce que la manche doit decider de financer.
    summaries: HashMap<PathBuf, String>,
}

impl ConfigurableNotice {
    /// Un contributeur pour la variante donnee.
    #[must_use]
    pub fn new(variant: NoticeVariant) -> Self {
        Self {
            variant,
            summaries: HashMap::new(),
        }
    }

    /// Declare ce qui a change dans un fichier. N'a d'effet que sur
    /// [`NoticeVariant::Contextual`].
    #[must_use]
    pub fn with_summary(mut self, path: impl Into<PathBuf>, summary: impl Into<String>) -> Self {
        self.summaries.insert(path.into(), summary.into());
        self
    }

    /// La variante active.
    #[must_use]
    pub fn variant(&self) -> NoticeVariant {
        self.variant
    }
}

impl PromptContributor for ConfigurableNotice {
    fn name(&self) -> &'static str {
        "stale_read_notice"
    }

    fn contribute(&self, ctx: &SessionContext<'_>) -> Option<PromptFragment> {
        let Some(Verdict::StaleRead { stale }) = ctx.last_verdict else {
            return None;
        };
        if stale.is_empty() {
            return None;
        }

        let mut body = String::new();
        for file in stale {
            let ago = humanize(ctx.now - file.read_at);
            let fichier = file.path.display();
            let auteur = &file.last_writer_name;

            match self.variant {
                NoticeVariant::Neutral => {
                    body.push_str(&format!(
                        "[Trame] {fichier} a ete modifie par la session « {auteur} »\n        \
                         apres que tu l'aies lu (il y a {ago}).\n"
                    ));
                }
                NoticeVariant::Directive => {
                    body.push_str(&format!(
                        "[Trame] {fichier} a ete modifie par la session « {auteur} »\n        \
                         apres que tu l'aies lu (il y a {ago}).\n        \
                         Relis {fichier} avant de continuer, et corrige ce qui en depend.\n"
                    ));
                }
                NoticeVariant::Contextual => {
                    let resume = self
                        .summaries
                        .get(&file.path)
                        .map_or("le contenu a change", String::as_str);
                    body.push_str(&format!(
                        "[Trame] {fichier} a ete modifie par la session « {auteur} »\n        \
                         apres que tu l'aies lu (il y a {ago}) : {resume}.\n        \
                         Relis {fichier} avant de continuer si ton travail en depend.\n"
                    ));
                }
            }
        }

        Some(PromptFragment {
            source: self.name(),
            priority: 10,
            body: body.trim_end().to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use chrono::{TimeDelta, Utc};

    use super::*;
    use crate::prompt::PromptPipeline;
    use crate::{
        BranchName, BranchTarget, Harness, Project, ProjectId, Session, SessionId, SessionState,
        StaleFile, Toolchain,
    };

    fn contexte(verdict: &Verdict) -> (Project, Session) {
        let now = Utc::now();
        let project = Project {
            id: ProjectId::new(),
            path: PathBuf::from("/projet"),
            name: "projet".into(),
            toolchain: Toolchain::Cargo,
            added_at: now,
            last_opened_at: Some(now),
        };
        let session = Session {
            id: SessionId::new(),
            project_id: project.id,
            name: "ajout-handlers".into(),
            harness: Harness::ClaudeCode,
            target_branch: BranchTarget::New(BranchName::new("feat/x")),
            work_item: None,
            state: SessionState::Writing,
            created_at: now,
        };
        let _ = verdict;
        (project, session)
    }

    fn verdict() -> Verdict {
        let now = Utc::now();
        Verdict::StaleRead {
            stale: vec![StaleFile {
                path: PathBuf::from("auth.rs"),
                last_writer: SessionId::new(),
                last_writer_name: "refacto-api".into(),
                read_at: now - TimeDelta::minutes(2),
                written_at: now - TimeDelta::seconds(20),
                seq: crate::Seq::FIRST,
            }],
        }
    }

    fn rendu(notice: ConfigurableNotice) -> String {
        let verdict = verdict();
        let (project, session) = contexte(&verdict);
        let ctx = SessionContext::new(&session, &project, Utc::now())
            .with_last_verdict(&verdict)
            .with_pending_write(Path::new("handlers.rs"));
        PromptPipeline::new()
            .with(notice)
            .render(&ctx)
            .expect("un avis")
    }

    /// Les trois variantes portent les trois faits. C'est leur socle commun, et il est
    /// verifie en structure : le texte, lui, est fait pour changer.
    #[test]
    fn les_trois_variantes_portent_le_fichier_l_auteur_et_le_delai() {
        for variante in NoticeVariant::all() {
            let texte = rendu(ConfigurableNotice::new(*variante));
            for attendu in ["auth.rs", "refacto-api", "2 min"] {
                assert!(
                    texte.contains(attendu),
                    "la variante {} doit porter {attendu} : {texte}",
                    variante.label()
                );
            }
        }
    }

    /// Ce qui distingue les variantes, exprime comme une propriete et non comme une
    /// citation : la neutre n'ordonne rien, les deux autres si.
    #[test]
    fn seule_la_neutre_n_ordonne_rien() {
        assert!(!rendu(ConfigurableNotice::new(NoticeVariant::Neutral)).contains("Relis"));
        assert!(rendu(ConfigurableNotice::new(NoticeVariant::Directive)).contains("Relis"));
        assert!(rendu(ConfigurableNotice::new(NoticeVariant::Contextual)).contains("Relis"));
    }

    /// La contextuelle inclut le resume du changement quand il est connu.
    #[test]
    fn la_contextuelle_dit_ce_qui_a_change() {
        let notice = ConfigurableNotice::new(NoticeVariant::Contextual).with_summary(
            "auth.rs",
            "la fonction verify_token a ete renommee validate_token",
        );
        let texte = rendu(notice);
        assert!(texte.contains("verify_token"), "{texte}");
        assert!(texte.contains("validate_token"), "{texte}");
    }

    /// Sans resume connu, la contextuelle degrade proprement plutot que de mentir ou de
    /// laisser un trou dans la phrase.
    #[test]
    fn la_contextuelle_degrade_proprement_sans_resume() {
        let texte = rendu(ConfigurableNotice::new(NoticeVariant::Contextual));
        assert!(texte.contains("le contenu a change"), "{texte}");
    }
}
