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

    /// Render the **production** contributor's text, in the same context as
    /// `render_variant`.
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

    /// ★ **Production is byte-for-byte the neutral text, and differs from the other two.**
    ///
    /// # How this assertion got here
    ///
    /// Its previous form asserted the opposite — that *no* variant equalled production —
    /// and it existed because for two campaigns ADR 0018 measured
    /// `NoticeVariant::Neutral` while presenting it as the product's canonical form. It
    /// was not: production carried an extra re-read line. The texts looked alike enough
    /// to conflate, and nothing compared them.
    ///
    /// That assertion then did its job. Measured directly, production scored 3/6 against
    /// the neutral variant's 3/3; the decision was to drop the third line; and the
    /// assertion **failed on the next run**, carrying its own instructions — replay the
    /// measurement, update the ADR, then lift it. All three happened, so it is lifted
    /// here and replaced by the stronger claim.
    ///
    /// # What it guards now
    ///
    /// Equality with `Neutral` is no longer an accident to detect, it is **the decision**
    /// (ADR 0018). Pinning it means any future edit to either text fails this test and
    /// sends the reader to the ADR — which is exactly what was missing when the two
    /// drifted apart in silence.
    ///
    /// Its negative control: adding a line back to either text makes it fail.
    #[test]
    fn production_is_exactly_the_neutral_text() {
        let production = render_production();
        assert_eq!(
            render_variant(ConfigurableNotice::new(NoticeVariant::Neutral)),
            production,
            "the shipped notice must be byte-for-byte the neutral variant (ADR 0018). If \
             this text is changing on purpose, the six-run measurement has to be replayed \
             and ADR 0018 updated — the last time these two drifted, nothing noticed for \
             two campaigns."
        );
        for variant in [NoticeVariant::Directive, NoticeVariant::Contextual] {
            assert_ne!(
                render_variant(ConfigurableNotice::new(variant)),
                production,
                "variant {} must stay distinguishable from production, or it stops being a \
                 comparison device",
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

    /// ★ **The shipped notice orders nothing.** It states facts and stops.
    ///
    /// This is the property the measurement bought, so it is pinned as a property rather
    /// than as a quotation. It used to assert the reverse: production *did* order a
    /// re-read, and that line cost half the runs (ADR 0018).
    ///
    /// The mechanism, worth keeping in mind before adding any instruction here: **an agent
    /// that receives a fact acts on it; an agent that receives a fact plus permission to
    /// ignore it ignores it half the time.** The removed line ended with `if your work
    /// depends on it`.
    #[test]
    fn the_shipped_notice_states_facts_and_orders_nothing() {
        let production = render_production();
        assert!(
            !production.contains("Re-read"),
            "the shipped notice must not instruct — measured at 3/6 when it did: {production}"
        );
        assert!(
            !render_variant(ConfigurableNotice::new(NoticeVariant::Neutral)).contains("Re-read")
        );
    }
}
