//! Notice variants: a **comparison device against production**, never a substitute.
//!
//! # Status: experimental, and deliberately different from what ships
//!
//! These variants exist to be measured against [`crate::StaleReadNotice`], which is what
//! the product actually sends. Results are in
//! [ADR 0018](../../../docs/adr/0018-pas-de-diff-dans-stalefile.md):
//!
//! | text injected | runs | re-reads the file | right name | overwrite |
//! |---|---|---|---|---|
//! | **`StaleReadNotice` — what ships** | **6** | **3/6** | **3/6** | 0/6 |
//! | `Neutral` | 3 | 3/3 | 3/3 | 0/3 |
//! | `Directive` | 3 | 3/3 | 3/3 | 0/3 |
//!
//! # ★ Read this before quoting any of those numbers
//!
//! For two measurement campaigns the harness measured `Neutral` while believing it measured
//! the product. The two texts differ by one line — `Neutral` stops at the facts, production
//! adds a re-read line — so the earlier `5/5` and `3/3` never bore on the string Trame
//! sends. **The shipped text is the only one of the three that fails.**
//!
//! Nobody caught it because nothing could: the texts look alike, the columns were at
//! ceiling, and no test compared them. `tests::no_variant_is_the_production_notice` now
//! fails if a variant becomes identical to production — it does not demand equality, it
//! demands the difference stay observed.
//!
//! # What this module is not
//!
//! It is **not a product extension point**. The product contributor is
//! [`crate::StaleReadNotice`]. This module is kept for two reasons, and not a third:
//!
//! 1. To document what was measured, so a later replay starts from the same base. The
//!    validation debt is real — short scenario, little accumulated context — and ADR 0018
//!    details it.
//! 2. To make a replay possible without rewriting the harness.
//!
//! **The change summary stays a simulation.** It is supplied from outside by
//! [`ConfigurableNotice::with_summary`] and **the registry computes none**: no diff at
//! admission. That decision survives the new measurement — `Neutral` says *less* than
//! production and does better, so the failure is not a context shortage.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::prompt::{PromptContributor, PromptFragment, SessionContext, humanize};
use crate::verdict::Verdict;

/// Which of the three wordings to use.
///
/// **Experimental** (ADR 0018). None of them is the shipped text: that is
/// [`crate::StaleReadNotice`], and it differs from every variant here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum NoticeVariant {
    /// The facts alone: file, author, delay. No order. This is the default: when in
    /// doubt, inform without instructing.
    #[default]
    Neutral,
    /// The facts, plus an explicit instruction to re-read.
    Directive,
    /// The facts, plus a summary of what changed.
    Contextual,
}

impl NoticeVariant {
    /// The stable label, for experiment logs.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Neutral => "neutral",
            Self::Directive => "directive",
            Self::Contextual => "contextual",
        }
    }

    /// All three, in the order the experimental round runs them.
    #[must_use]
    pub fn all() -> &'static [Self] {
        &[Self::Neutral, Self::Directive, Self::Contextual]
    }
}

/// A notice contributor whose wording is a parameter.
///
/// **Experimental** (ADR 0018). Do not wire it into the product: the product contributor
/// is [`crate::StaleReadNotice`]. This one exists to be measured against it.
#[derive(Debug, Clone, Default)]
pub struct ConfigurableNotice {
    variant: NoticeVariant,
    /// Change summary, by path. Supplied from outside because the registry does not
    /// compute one — that is exactly the spend the measurement declined.
    summaries: HashMap<PathBuf, String>,
}

impl ConfigurableNotice {
    /// A contributor for the given variant.
    #[must_use]
    pub fn new(variant: NoticeVariant) -> Self {
        Self {
            variant,
            summaries: HashMap::new(),
        }
    }

    /// Declare what changed in a file. Only affects [`NoticeVariant::Contextual`].
    ///
    /// **This is a simulation.** The registry computes no diff at admission and `StaleFile`
    /// carries no summary: measurement showed it added nothing (ADR 0018). This summary is
    /// therefore supplied by hand by the experimental harness, and by nobody else.
    #[must_use]
    pub fn with_summary(mut self, path: impl Into<PathBuf>, summary: impl Into<String>) -> Self {
        self.summaries.insert(path.into(), summary.into());
        self
    }

    /// The active variant.
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

    fn context(verdict: &Verdict) -> (Project, Session) {
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

    fn render_variant(notice: ConfigurableNotice) -> String {
        let verdict = verdict();
        let (project, session) = context(&verdict);
        let ctx = SessionContext::new(&session, &project, Utc::now())
            .with_last_verdict(&verdict)
            .with_pending_write(Path::new("handlers.rs"));
        PromptPipeline::new()
            .with(notice)
            .render(&ctx)
            .expect("a notice")
    }

    /// All three variants carry the three facts. That is their common base, and it is
    /// checked structurally: the text itself is meant to change.
    #[test]
    fn all_three_variants_carry_the_file_the_author_and_the_delay() {
        for variant in NoticeVariant::all() {
            let text = render_variant(ConfigurableNotice::new(*variant));
            for expected in ["auth.rs", "refacto-api", "2 min"] {
                assert!(
                    text.contains(expected),
                    "variant {} must carry {expected}: {text}",
                    variant.label()
                );
            }
        }
    }

    /// What separates the variants, stated as a property rather than a quotation: the
    /// neutral one orders nothing, the other two do.
    #[test]
    fn only_the_neutral_variant_orders_nothing() {
        assert!(
            !render_variant(ConfigurableNotice::new(NoticeVariant::Neutral)).contains("Re-read")
        );
        assert!(
            render_variant(ConfigurableNotice::new(NoticeVariant::Directive)).contains("Re-read")
        );
        assert!(
            render_variant(ConfigurableNotice::new(NoticeVariant::Contextual)).contains("Re-read")
        );
    }

    /// The contextual variant includes the change summary when it is known.
    #[test]
    fn the_contextual_variant_says_what_changed() {
        let notice = ConfigurableNotice::new(NoticeVariant::Contextual).with_summary(
            "auth.rs",
            "the verify_token function was renamed to validate_token",
        );
        let text = render_variant(notice);
        assert!(text.contains("verify_token"), "{text}");
        assert!(text.contains("validate_token"), "{text}");
    }

    /// With no summary known, the contextual variant degrades cleanly rather than lying
    /// or leaving a hole in the sentence.
    #[test]
    fn the_contextual_variant_degrades_cleanly_without_a_summary() {
        let text = render_variant(ConfigurableNotice::new(NoticeVariant::Contextual));
        assert!(text.contains("the contents changed"), "{text}");
    }

    /// Render the **production** contributor's text, in the same context as `rendu`.
    fn render_production() -> String {
        let verdict = verdict();
        let (project, session) = context(&verdict);
        let ctx = SessionContext::new(&session, &project, Utc::now())
            .with_last_verdict(&verdict)
            .with_pending_write(Path::new("handlers.rs"));
        PromptPipeline::new()
            .with(crate::prompt::StaleReadNotice)
            .render(&ctx)
            .expect("a notice")
    }

    /// ★ **No variant is the production text.** The test that was missing.
    ///
    /// For two campaigns ADR 0018 measured `NoticeVariant::Neutral` while presenting it as
    /// the product's canonical form. It was not: `StaleReadNotice` adds a re-read line the
    /// neutral variant does not have. The two texts looked alike enough for a quick read to
    /// conflate them, and **nothing in the test suite compared them** — so nothing could
    /// say otherwise.
    ///
    /// This test makes the gap impossible to reintroduce silently. It does not demand
    /// equality: production **must** be allowed to differ from an experimental device. It
    /// demands the difference stay **observed**, so that nobody reports a number measured on
    /// a variant as a number about the product.
    ///
    /// Its negative control: making the two texts identical makes it fail, which is the
    /// intended behaviour — equality should be a decision, and therefore a measurement
    /// replay.
    #[test]
    fn no_variant_is_the_production_notice() {
        let production = render_production();
        for variant in NoticeVariant::all() {
            let variant_text = render_variant(ConfigurableNotice::new(*variant));
            assert_ne!(
                variant_text,
                production,
                "variant {} has become identical to the production text. If that is \
                 intended, the ADR 0018 measurement must be replayed and the ADR updated \
                 before lifting this assertion.",
                variant.label()
            );
        }
    }

    /// The facts are the same on both sides: that is what makes the variants comparable to
    /// production, and therefore useful as a comparison device.
    #[test]
    fn production_and_variants_carry_the_same_facts() {
        let production = render_production();
        for expected in ["auth.rs", "refacto-api", "2 min"] {
            assert!(production.contains(expected), "{expected} : {production}");
        }
    }

    /// The exact gap, named: production orders a re-read, the neutral variant stops at the
    /// facts. That is the line that was never measured.
    #[test]
    fn production_orders_a_re_read_where_the_neutral_variant_stops_at_the_facts() {
        assert!(render_production().contains("Re-read"));
        assert!(
            !render_variant(ConfigurableNotice::new(NoticeVariant::Neutral)).contains("Re-read")
        );
    }
}
