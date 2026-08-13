//! Les variantes de l'avis, **mesurees puis ecartees**.
//!
//! # Statut : experimental, non retenu
//!
//! Ce module a servi a une manche experimentale, et cette manche est finie. Son resultat est
//! dans l'[ADR 0018](../../../docs/adr/0018-pas-de-diff-dans-stalefile.md) :
//!
//! | variante | relit le file | bon nom | sur-ecriture |
//! |---|---|---|---|
//! | **neutre** | 5/5 | 5/5 | 0/5 |
//! | directive | 5/5 | 5/5 | 0/5 |
//! | contextuelle | 5/5 | 5/5 | 0/5 |
//!
//! La variante **neutre** fait aussi bien que les deux autres tout en etant la moins chere
//! et la moins intrusive. L'hypothese qui justifiait la depense — « l'agent ne suivra l'avis
//! que s'il sait *ce qui* a change » — est **refutee** : dire qu'il faut relire suffit.
//!
//! # Ce que ce module n'est plus
//!
//! Ce n'est **pas un point d'extension du produit**. Le contributeur du produit est
//! [`crate::StaleReadNotice`], avec le texte neutre. Ce module est conserve pour deux
//! raisons, et pas une troisieme :
//!
//! 1. Documenter ce qui a ete mesure, pour qu'un rejeu ulterieur reparte de la meme base.
//!    La dette de validation est reelle — scenario court, outils fermes, peu de contexte
//!    accumule — et l'ADR 0018 la detaille.
//! 2. Rendre un rejeu possible sans reecrire le harnais.
//!
//! **Le summary du changement reste une simulation.** Il est fourni de l'exterieur par
//! [`ConfigurableNotice::with_summary`] et **le registre n'en calcule aucun** : pas de diff a
//! l'admission, c'est precisement la depense que la mesure a evitee.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::prompt::{PromptContributor, PromptFragment, SessionContext, humanize};
use crate::verdict::Verdict;

/// Laquelle des trois formulations utiliser.
///
/// **Experimental, non retenu** (ADR 0018). La forme canonique du produit est le texte
/// neutre, porte par [`crate::StaleReadNotice`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum NoticeVariant {
    /// Les faits seuls : file, auteur, delai. Aucun ordre. C'est le defaut : en cas de
    /// doute, on informe sans ordonner.
    #[default]
    Neutral,
    /// Les faits, plus une instruction explicite de relecture.
    Directive,
    /// Les faits, plus un summary de ce qui a change.
    Contextual,
}

impl NoticeVariant {
    /// Le libelle stable, pour les journaux d'experience.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Neutral => "neutral",
            Self::Directive => "directive",
            Self::Contextual => "contextual",
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
/// **Experimental, non retenu** (ADR 0018). Ne pas cabler dans le produit : le
/// contributeur du produit est [`crate::StaleReadNotice`]. Celui-ci ne sert qu'a rejouer la
/// manche experimentale.
#[derive(Debug, Clone, Default)]
pub struct ConfigurableNotice {
    variant: NoticeVariant,
    /// Resume du changement, par path. Alimente de l'exterieur tant que le registre ne
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

    /// Declare ce qui a change dans un file. N'a d'effet que sur
    /// [`NoticeVariant::Contextual`].
    ///
    /// **C'est une simulation.** Le registre ne calcule aucun diff a l'admission et
    /// `StaleFile` ne porte aucun summary : la mesure a montre que ca n'apportait rien
    /// (ADR 0018). Ce summary est donc fourni a la main par le harnais experimental, et par
    /// personne d'autre.
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
        for stale_file in stale {
            let ago = humanize(ctx.now - stale_file.read_at);
            let author = &stale_file.last_writer_name;
            let file = stale_file.path.display();

            match self.variant {
                NoticeVariant::Neutral => {
                    body.push_str(&format!(
                        "[Trame] {file} was changed by session \"{author}\"\n        \
                         after you read it ({ago} ago).\n"
                    ));
                }
                NoticeVariant::Directive => {
                    body.push_str(&format!(
                        "[Trame] {file} was changed by session \"{author}\"\n        \
                         after you read it ({ago} ago).\n        \
                         Re-read {file} before continuing, and fix whatever depends on it.\n"
                    ));
                }
                NoticeVariant::Contextual => {
                    let summary = self
                        .summaries
                        .get(&stale_file.path)
                        .map_or("the contents changed", String::as_str);
                    body.push_str(&format!(
                        "[Trame] {file} was changed by session \"{author}\"\n        \
                         after you read it ({ago} ago): {summary}.\n        \
                         Re-read {file} before continuing if your work depends on it.\n"
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
        assert!(!rendu(ConfigurableNotice::new(NoticeVariant::Neutral)).contains("Re-read"));
        assert!(rendu(ConfigurableNotice::new(NoticeVariant::Directive)).contains("Re-read"));
        assert!(rendu(ConfigurableNotice::new(NoticeVariant::Contextual)).contains("Re-read"));
    }

    /// La contextuelle inclut le summary du changement quand il est connu.
    #[test]
    fn la_contextuelle_dit_ce_qui_a_change() {
        let notice = ConfigurableNotice::new(NoticeVariant::Contextual).with_summary(
            "auth.rs",
            "the verify_token function was renamed to validate_token",
        );
        let texte = rendu(notice);
        assert!(texte.contains("verify_token"), "{texte}");
        assert!(texte.contains("validate_token"), "{texte}");
    }

    /// Sans summary connu, la contextuelle degrade proprement plutot que de mentir ou de
    /// laisser un trou dans la phrase.
    #[test]
    fn la_contextuelle_degrade_proprement_sans_resume() {
        let texte = rendu(ConfigurableNotice::new(NoticeVariant::Contextual));
        assert!(texte.contains("the contents changed"), "{texte}");
    }

    /// Rend le texte du contributeur **de production**, dans le meme contexte que `rendu`.
    fn rendu_production() -> String {
        let verdict = verdict();
        let (project, session) = contexte(&verdict);
        let ctx = SessionContext::new(&session, &project, Utc::now())
            .with_last_verdict(&verdict)
            .with_pending_write(Path::new("handlers.rs"));
        PromptPipeline::new()
            .with(crate::prompt::StaleReadNotice)
            .render(&ctx)
            .expect("un avis")
    }

    /// ★ **Aucune variante n'est le texte de production.** Le test qui manquait.
    ///
    /// Pendant deux campagnes, l'ADR 0018 a mesure `NoticeVariant::Neutral` en le presentant
    /// comme la forme canonique du produit. Il ne l'etait pas : `StaleReadNotice` ajoute une
    /// line de relecture que la neutre n'a pas. Les deux textes se ressemblaient assez pour
    /// qu'une lecture rapide les confonde, et **rien dans la suite de tests ne les comparait**
    /// — donc rien ne pouvait le dire.
    ///
    /// Ce test rend l'ecart impossible a reintroduire en silence. Il n'exige pas l'egalite :
    /// la production **doit** pouvoir differer d'un dispositif experimental. Il exige que la
    /// difference reste **constatee**, pour que personne ne rapporte un chiffre mesure sur une
    /// variante comme un chiffre sur le produit.
    ///
    /// Son controle negatif : rendre les deux textes identiques le fait echouer, ce qui est le
    /// comportement voulu — l'egalite devrait etre une decision, donc un rejeu de mesure.
    #[test]
    fn no_variant_is_the_production_notice() {
        let production = rendu_production();
        for variante in NoticeVariant::all() {
            let variant_text = rendu(ConfigurableNotice::new(*variante));
            assert_ne!(
                variant_text,
                production,
                "la variante {} est devenue identique au texte de production. Si c'est \
                 volontaire, la mesure de l'ADR 0018 doit etre rejouee et l'ADR mis a jour \
                 avant de lever cette assertion.",
                variante.label()
            );
        }
    }

    /// Les faits sont les memes de part et d'autre : c'est ce qui rend les variantes
    /// comparables a la production, et donc utiles comme dispositif de comparaison.
    #[test]
    fn production_and_variants_carry_the_same_facts() {
        let production = rendu_production();
        for attendu in ["auth.rs", "refacto-api", "2 min"] {
            assert!(production.contains(attendu), "{attendu} : {production}");
        }
    }

    /// L'ecart exact, nomme : la production ordonne la relecture, la neutre s'arrete au
    /// constat. C'est cette line-la qui n'a jamais ete mesuree.
    #[test]
    fn production_orders_a_re_read_where_the_neutral_variant_stops_at_the_facts() {
        assert!(rendu_production().contains("Re-read"));
        assert!(!rendu(ConfigurableNotice::new(NoticeVariant::Neutral)).contains("Re-read"));
    }
}
