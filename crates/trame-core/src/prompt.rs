//! **A seam that is not speculative.** The prompt composition pipeline.
//!
//! This is the mechanism that injects the stale-read notice into the agent's
//! context. Without it, [`crate::Verdict::StaleRead`] would be a journal line
//! nobody reads — and the product would have no reason to exist. v0.1 therefore
//! needs it, unlike the other two seams.
//!
//! The model is an ordered list of contributors. Each looks at the session context
//! and decides whether it has anything to say.

use std::path::Path;

use chrono::TimeDelta;

use crate::clock::Timestamp;
use crate::project::Project;
use crate::session::Session;
use crate::verdict::Verdict;

/// What a contributor sees when composing.
///
/// `#[non_exhaustive]`: adding a field must not break callers. The trade-off is that
/// it cannot be built with a struct expression from another crate — hence
/// [`SessionContext::new`] and its combinators, which are **the** way to make one.
/// The daemon builds one on every admission.
///
/// ```
/// # use trame_core::prompt::SessionContext;
/// # fn demo(session: &trame_core::Session, project: &trame_core::Project,
/// #         now: trame_core::clock::Timestamp, verdict: &trame_core::Verdict) {
/// let ctx = SessionContext::new(session, project, now)
///     .with_last_verdict(verdict)
///     .with_pending_write(std::path::Path::new("handlers.rs"));
/// # }
/// ```
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct SessionContext<'a> {
    /// The session in question.
    pub session: &'a Session,
    /// Its project.
    pub project: &'a Project,
    /// The current instant, supplied by the injected clock. A contributor never reads
    /// the clock itself, or it becomes untestable.
    pub now: Timestamp,
    /// The verdict from the last admission, if there was one.
    pub last_verdict: Option<&'a Verdict>,
    /// The file the session is about to write.
    pub pending_write: Option<&'a Path>,
}

impl<'a> SessionContext<'a> {
    /// The minimal context: a session, its project, an instant.
    ///
    /// Those three are always known when composing a prompt; everything else is
    /// optional and gets added through the combinators.
    #[must_use]
    pub fn new(session: &'a Session, project: &'a Project, now: Timestamp) -> Self {
        Self {
            session,
            project,
            now,
            last_verdict: None,
            pending_write: None,
        }
    }

    /// Attach the verdict from the last admission.
    #[must_use]
    pub fn with_last_verdict(mut self, verdict: &'a Verdict) -> Self {
        self.last_verdict = Some(verdict);
        self
    }

    /// Attach the file the session is about to write.
    #[must_use]
    pub fn with_pending_write(mut self, path: &'a Path) -> Self {
        self.pending_write = Some(path);
        self
    }
}

/// A piece of prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptFragment {
    /// Who produced it. Journalled, so that "why did the agent see this?" has an
    /// answer.
    pub source: &'static str,
    /// Assembly order. Lower goes first.
    pub priority: u8,
    /// The injected text.
    pub body: String,
}

/// A contributor to the prompt.
///
/// No `async`: composing a prompt is a computation over data already in memory. A
/// contributor that needed I/O would be a misplaced contributor — it is the caller's
/// job to have already gathered what is needed into the [`SessionContext`].
pub trait PromptContributor: Send + Sync {
    /// The contributor's name, for the journal.
    fn name(&self) -> &'static str;

    /// Its contribution, or `None` if it has nothing to say.
    ///
    /// **`None` is the normal case.** 95% of traffic is clean and must pass without a
    /// word.
    fn contribute(&self, ctx: &SessionContext<'_>) -> Option<PromptFragment>;
}

/// Assembles the contributions in order.
#[derive(Default)]
pub struct PromptPipeline {
    contributors: Vec<Box<dyn PromptContributor>>,
}

impl PromptPipeline {
    /// An empty pipeline.
    #[must_use]
    pub fn new() -> Self {
        Self {
            contributors: Vec::new(),
        }
    }

    /// Append a contributor to the chain.
    #[must_use]
    pub fn with(mut self, contributor: impl PromptContributor + 'static) -> Self {
        self.contributors.push(Box::new(contributor));
        self
    }

    /// The fragments to inject, sorted by priority. Empty if nobody has anything to
    /// say.
    #[must_use]
    pub fn compose(&self, ctx: &SessionContext<'_>) -> Vec<PromptFragment> {
        let mut fragments: Vec<_> = self
            .contributors
            .iter()
            .filter_map(|c| c.contribute(ctx))
            .collect();
        fragments.sort_by_key(|fragment| fragment.priority);
        fragments
    }

    /// The final text, fragments joined by a blank line. `None` if there is nothing to say.
    #[must_use]
    pub fn render(&self, ctx: &SessionContext<'_>) -> Option<String> {
        let bodies: Vec<_> = self
            .compose(ctx)
            .into_iter()
            .map(|fragment| fragment.body)
            .collect();
        if bodies.is_empty() {
            None
        } else {
            Some(bodies.join("\n\n"))
        }
    }
}

impl std::fmt::Debug for PromptPipeline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PromptPipeline")
            .field(
                "contributors",
                &self
                    .contributors
                    .iter()
                    .map(|c| c.name())
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

/// The contributor that tells the agent one of its reads is stale.
///
/// # The message format is what matters
///
/// The surrounding code is plumbing; **this text is the variable to iterate on**.
/// It stays neutral, factual, actionable: no orders, no alarm. An agent that gets
/// shouted at does not behave better, and a user that gets shouted at switches the
/// feature off.
///
/// # ★ Three lines became two, and that is measured (ADR 0018)
///
/// This notice used to end with `Re-read it before continuing if your work depends
/// on it.` Measured directly, it scored **3/6** where the instruction-free
/// [`crate::NoticeVariant::Neutral`] scored 3/3. Three of the six runs saw the agent
/// not re-read at all and ship the old identifier — the exact failure Trame exists to
/// catch.
///
/// The line was removed. The reasoning, in one sentence: **an agent that receives a
/// fact acts on it; an agent that receives a fact plus permission to ignore it
/// ignores it half the time.** `if your work depends on it` was an explicit licence
/// to do nothing.
///
/// This was not a new decision — ADR 0018 already named the neutral text as the
/// canonical form. The code had simply never matched it.
///
/// **Do not read more into the numbers than they carry.** Six runs give no
/// statistical power. What is solid is the direction: one variable, a clean split, an
/// identified failure mechanism. The magnitude is weak.
#[derive(Debug, Clone, Copy, Default)]
pub struct StaleReadNotice;

impl PromptContributor for StaleReadNotice {
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
            body.push_str(&format!(
                "[Trame] {} was changed by session \"{}\"\n        \
                 after you read it ({} ago).\n",
                file.path.display(),
                file.last_writer_name,
                ago,
            ));
        }

        Some(PromptFragment {
            source: self.name(),
            priority: 10,
            body: body.trim_end().to_owned(),
        })
    }
}

/// A duration, rounded to the unit that helps.
///
/// "2 min ago" is actionable. "127.4 s ago" is not.
pub(crate) fn humanize(delta: TimeDelta) -> String {
    let seconds = delta.num_seconds().max(0);
    match seconds {
        0..=44 => "a few seconds".to_owned(),
        45..=5399 => {
            let minutes = (seconds + 30) / 60;
            format!("{} min", minutes.max(1))
        }
        _ => {
            let hours = (seconds + 1800) / 3600;
            format!("{hours} h")
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use chrono::{TimeDelta, Utc};

    use super::*;
    use crate::ids::{ProjectId, Seq, SessionId};
    use crate::project::Toolchain;
    use crate::session::{BranchTarget, Harness, SessionState};
    use crate::{BranchName, StaleFile};

    fn fixture() -> (Project, Session) {
        let now = Utc::now();
        let project = Project {
            id: ProjectId::new(),
            path: PathBuf::from("/Users/x/dev/portailfcd"),
            name: "portailfcd".into(),
            toolchain: Toolchain::Cargo,
            added_at: now,
            last_opened_at: Some(now),
        };
        let session = Session {
            id: SessionId::new(),
            project_id: project.id,
            name: "ajout-handlers".into(),
            harness: Harness::ClaudeCode,
            target_branch: BranchTarget::New(BranchName::new("feat/handlers")),
            work_item: None,
            state: SessionState::Writing,
            created_at: now,
        };
        (project, session)
    }

    #[test]
    fn nothing_is_said_when_the_write_is_clean() {
        let (project, session) = fixture();
        let ctx =
            SessionContext::new(&session, &project, Utc::now()).with_last_verdict(&Verdict::Clean);
        let pipeline = PromptPipeline::new().with(StaleReadNotice);
        assert_eq!(
            pipeline.render(&ctx),
            None,
            "a clean verdict must be silent"
        );
    }

    /// Build a stale file, read `read_ago` ago.
    fn stale_file(path: &str, writer_name: &str, read_ago: TimeDelta, now: Timestamp) -> StaleFile {
        StaleFile {
            path: PathBuf::from(path),
            last_writer: SessionId::new(),
            last_writer_name: writer_name.to_owned(),
            read_at: now - read_ago,
            written_at: now - TimeDelta::seconds(30),
            seq: Seq::FIRST,
        }
    }

    // The next two tests check the **structure** of the notice — that the three
    // actionable facts are there — and never its prose.
    //
    // The message text is the variable we will iterate on most: it decides whether the
    // agent re-reads and adapts. A test pinning the words would fail the suite on every
    // wording tweak, and so discourage exactly the tweaks we want to encourage. The
    // delay is compared against [`humanize`]'s output rather than a literal, so that
    // changing the duration format does not break these two either — that is
    // `delays_are_rounded_to_a_unit_that_helps`'s job.

    #[test]
    fn the_notice_carries_the_file_the_session_and_the_delay() {
        let (project, session) = fixture();
        let now = Utc::now();
        let read_ago = TimeDelta::minutes(2);
        let verdict = Verdict::StaleRead {
            stale: vec![stale_file("auth.rs", "refacto-api", read_ago, now)],
        };
        let ctx = SessionContext::new(&session, &project, now)
            .with_last_verdict(&verdict)
            .with_pending_write(Path::new("handlers.rs"));

        let fragments = PromptPipeline::new().with(StaleReadNotice).compose(&ctx);
        assert_eq!(
            fragments.len(),
            1,
            "exactly one contributor has something to say"
        );

        let fragment = &fragments[0];
        assert_eq!(
            fragment.source, "stale_read_notice",
            "the fragment must be attributable"
        );

        let body = &fragment.body;
        assert!(
            body.contains("auth.rs"),
            "the stale file path must appear: {body}"
        );
        assert!(
            body.contains("refacto-api"),
            "the writing session name must appear: {body}"
        );
        assert!(
            body.contains(&humanize(read_ago)),
            "the elapsed delay must appear: {body}"
        );
    }

    #[test]
    fn every_stale_file_is_named_in_the_notice() {
        let (project, session) = fixture();
        let now = Utc::now();
        let verdict = Verdict::StaleRead {
            stale: vec![
                stale_file("auth.rs", "refacto-api", TimeDelta::minutes(2), now),
                stale_file("db/pool.rs", "migration-sqlx", TimeDelta::hours(1), now),
            ],
        };
        let ctx = SessionContext::new(&session, &project, now)
            .with_last_verdict(&verdict)
            .with_pending_write(Path::new("handlers.rs"));

        let body = PromptPipeline::new()
            .with(StaleReadNotice)
            .render(&ctx)
            .unwrap();
        for expected in ["auth.rs", "refacto-api", "db/pool.rs", "migration-sqlx"] {
            assert!(
                body.contains(expected),
                "{expected} must appear in the notice: {body}"
            );
        }
    }

    // Here, pinning the output is legitimate: `humanize` is a pure function whose
    // contract *is* the rendered form. This test carries the duration format, which is
    // what lets the two above ignore it.
    #[test]
    fn delays_are_rounded_to_a_unit_that_helps() {
        assert_eq!(humanize(TimeDelta::seconds(3)), "a few seconds");
        assert_eq!(humanize(TimeDelta::seconds(120)), "2 min");
        assert_eq!(humanize(TimeDelta::minutes(89)), "89 min");
        assert_eq!(humanize(TimeDelta::hours(3)), "3 h");
        assert_eq!(humanize(TimeDelta::seconds(-5)), "a few seconds");
    }
}
