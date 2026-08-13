//! Where the observations come from. **Nothing is simulated.**
//!
//! The real SQLite journal, the real registry, the real FSEvents watcher. Opening a project
//! in the interface opens what the daemon would open — and an out-of-band write made by
//! hand in the directory really does appear in the feed.
//!
//! # What v0.1 does not do here
//!
//! **It launches no agent.** There is no session service in the daemon yet, so this binary
//! opens no ACP session. Session panels appear when a [`trame_daemon::SessionPilot`] is
//! observed — which is what `--scenario` does, by running the canonical scenario through
//! the **real registry**, with no agent.
//!
//! That is a limit, not a stage set: the transport shown is then `none`, because that is
//! the truth. Showing `ACP` to look good would promise a guarantee nothing provides.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use trame_core::clock::Clock;
use trame_core::{ProjectId, ProjectRoot, SessionId};
use trame_daemon::{Observation, Observer, Transport, WatcherGuard, observe_channel};
use trame_journal::{Journal, spawn_journal};
use trame_registry::{ReadKind, RegistryHandle, spawn_registry};

/// An open project, and the feed of what happens in it.
///
/// As long as this value lives, the watcher watches. Dropping it closes the project.
pub struct Source {
    /// The observation feed. **Read-only: the interface does not drive.**
    pub observations: mpsc::Receiver<Observation>,
    /// The project's display name.
    pub project: String,
    _watcher: WatcherGuard,
    _taches: Vec<JoinHandle<()>>,
}

/// Refuse a root the scenario has no business writing into.
///
/// **The scenario writes into the target project** — that is the whole point, the verdicts
/// shown are those of real writes. The corollary is that a defaulted root is dangerous:
/// running scenario mode from a repository drops `auth.rs` and `handlers.rs` into it.
///
/// This happened. Two scenario files ended up at this repository's root, and I could not
/// reproduce the exact invocation — all the more reason to close the class of accident
/// rather than hunt the culprit. A directory containing `.git` is not a sandbox.
///
/// # Errors
///
/// Fails if the root contains a `.git`.
pub fn refuse_dangerous_root(root: &Path) -> Result<()> {
    if root.join(".git").exists() {
        anyhow::bail!(
            "{} contains a .git: scenario mode would write auth.rs and handlers.rs into \
             it.\n\
             Pass a throwaway directory, /tmp/trame-demo for instance.",
            root.display()
        );
    }
    Ok(())
}

/// Open a project: journal, registry, watcher.
///
/// # Errors
///
/// Fails if the root does not exist, if the SQLite journal cannot be opened, or if
/// FSEvents refuses to watch the directory.
pub async fn open(root: &Path, clock: Arc<dyn Clock>, scenario: bool) -> Result<Source> {
    // Scenario mode **creates** its sandbox. It writes into it anyway, and requiring it to
    // exist already buys no safety: the useful guard is elsewhere, in
    // `refuse_dangerous_root`, which refuses a repository. Without this, `--scenario
    // /tmp/demo` fails with "invalid project root" — which is true and useless.
    //
    // Observation, on the other hand, still requires an **existing** project: watching a
    // directory you just created is watching nothing, and it is more likely a typo in the
    // path.
    if scenario && !root.exists() {
        std::fs::create_dir_all(root)
            .with_context(|| format!("cannot create sandbox: {}", root.display()))?;
        tracing::info!(root = %root.display(), "sandbox created for the scenario");
    }
    let root = ProjectRoot::new(root)
        .with_context(|| format!("invalid project root: {}", root.display()))?;
    let nom = root.as_path().file_name().map_or_else(
        || root.as_path().display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );

    let journal = Journal::open_default().context("cannot open the SQLite journal")?;
    let (journal, tache_journal) = spawn_journal(journal);
    let project = ProjectId::new();
    let (registry, tache_registre) =
        spawn_registry(project, root.clone(), clock.clone(), journal.clone());

    let (observer, observations) = observe_channel();
    let (tache_watcher, guard) = trame_daemon::spawn_watcher_observed(
        root.clone(),
        registry.clone(),
        Some(observer.clone()),
    )
    .context("FSEvents refuses to watch this root")?;

    let mut tasks = vec![tache_journal, tache_registre, tache_watcher];
    if scenario {
        tasks.push(tokio::spawn(play_scenario(registry, observer)));
    }

    Ok(Source {
        observations,
        project: nom,
        _watcher: guard,
        _taches: tasks,
    })
}

/// Run the canonical scenario through the **real** registry, with no agent.
///
/// A reads `auth.rs`, B writes `auth.rs` (→ `Clean`), A writes `handlers.rs`
/// (→ `StaleRead`). Two different files, no write collision: the failure mode Trame exists
/// to catch, and what the interface has to make obvious.
///
/// The verdicts shown are the registry's. **Nothing is fabricated here** — otherwise the
/// interface would be validating its own fiction, which this project has already paid for
/// twice.
///
/// After those four steps it keeps going, for a reason that is about the interface rather
/// than the product: see [`play_filler`].
async fn play_scenario(registry: RegistryHandle, mut observer: Observer) {
    let (a, b) = (SessionId::new(), SessionId::new());
    for (session, name) in [(a, "scenario-a"), (b, "scenario-b")] {
        if registry.register_session(session, name).await.is_err() {
            return;
        }
        observer.emit(Observation::SessionOpened {
            session,
            name: name.to_owned(),
            // The truth: no agent is attached, so nothing is intercepted.
            transport: Transport::Absent,
        });
    }

    let auth = PathBuf::from("auth.rs");
    let handlers = PathBuf::from("handlers.rs");
    let v1 = "pub fn verify_token(token: &str) -> bool { !token.is_empty() }\n";

    // 0. The fixture goes through admission, not through a direct `fs::write`.
    //
    // Writing the file behind the registry's back would make it a genuine out-of-band
    // write, which the watcher would rightly report — and the interface's "out-of-band"
    // counter would show 1 without the user having done anything. Depending on the race
    // between FSEvents and the next write it would show 0 or 1: non-deterministic output
    // on precisely the counter that has to stay believable.
    if let Ok(verdict) = registry.admit(a, auth.clone(), v1).await {
        observer.emit(Observation::Write {
            session: a,
            path: auth.clone(),
            verdict,
        });
    }

    // 1. A reads auth.rs. This is what fills the read-set.
    if registry
        .record_read(a, auth.clone(), v1, ReadKind::FullFile)
        .await
        .is_ok()
    {
        observer.emit(Observation::Read {
            session: a,
            path: auth.clone(),
        });
    }

    // 2. B writes auth.rs. Nobody read what B overwrites: Clean.
    if let Ok(verdict) = registry
        .admit(
            b,
            auth.clone(),
            "pub fn validate_token(t: &str) -> bool { !t.is_empty() }\n",
        )
        .await
    {
        observer.emit(Observation::Write {
            session: b,
            path: auth,
            verdict,
        });
    }

    // 3. A writes handlers.rs. A different file, and yet StaleRead.
    if let Ok(verdict) = registry
        .admit(a, handlers.clone(), "verify_token(t)\n")
        .await
    {
        observer.emit(Observation::Write {
            session: a,
            path: handlers,
            verdict,
        });
    }

    play_filler(&registry, &mut observer, a, b).await;
}

/// How many extra rounds the filler plays after the canonical scenario.
///
/// Four observations per round, so this has to clear any plausible window height with room
/// to spare — otherwise the thing it exists to make testable stays untestable.
const FILLER_ROUNDS: usize = 12;

/// How long to wait between rounds, so the feed fills **visibly**.
///
/// Not a test, so a delay is legitimate here — the point is to be watched. Short enough that
/// the list is full in a couple of seconds, slow enough that arrival is perceptible.
const FILLER_INTERVAL: std::time::Duration = std::time::Duration::from_millis(120);

/// Keeps the feed going past the canonical four steps, **through the real registry**.
///
/// # Why this exists, and why it is not part of the scenario
///
/// The canonical scenario is four observations. That is exactly right for what it
/// demonstrates, and exactly wrong for exercising the interface: a list that never exceeds
/// the window height means the scroll path is **never executed**. It compiles, it looks
/// finished, and nobody finds out until a real session produces forty lines.
///
/// That is the project's usual failure shape — a plausible output triggers no verification —
/// so the fix is to make the untested path routine rather than exceptional.
///
/// # It fabricates nothing
///
/// Every line comes from a real admission by the real registry, exactly like the four steps
/// above. Each round replays the canonical shape on its own pair of files, so the verdicts
/// are a genuine mix rather than a decorative one:
///
/// ```text
/// B writes  module_NN.rs    -> Clean      (nobody had read it)
/// A reads   module_NN.rs                  (fills A's read-set)
/// B writes  module_NN.rs    -> Clean      (B does not stale itself)
/// A writes  caller_NN.rs    -> StaleRead  (A read module_NN.rs, B changed it since)
/// ```
///
/// Emitting `Clean` lines it invented would have been shorter and would have defeated the
/// purpose: the point of `--scenario` is that what you see on screen happened.
async fn play_filler(
    registry: &RegistryHandle,
    observer: &mut Observer,
    a: SessionId,
    b: SessionId,
) {
    for round in 0..FILLER_ROUNDS {
        tokio::time::sleep(FILLER_INTERVAL).await;

        let module = PathBuf::from(format!("module_{round:02}.rs"));
        let caller = PathBuf::from(format!("caller_{round:02}.rs"));
        let v1 = format!("pub fn step_{round:02}() -> usize {{ {round} }}\n");
        let v2 = format!("pub fn stage_{round:02}() -> usize {{ {round} }}\n");

        // B creates the module. Nobody has read it, so this is Clean.
        if let Ok(verdict) = registry.admit(b, module.clone(), &v1).await {
            observer.emit(Observation::Write {
                session: b,
                path: module.clone(),
                verdict,
            });
        }

        // A reads it. This is the entry that can go stale.
        if registry
            .record_read(a, module.clone(), &v1, ReadKind::FullFile)
            .await
            .is_ok()
        {
            observer.emit(Observation::Read {
                session: a,
                path: module.clone(),
            });
        }

        // B renames the function under A's feet. Still Clean for B: a session never
        // stales its own read.
        if let Ok(verdict) = registry.admit(b, module.clone(), &v2).await {
            observer.emit(Observation::Write {
                session: b,
                path: module,
                verdict,
            });
        }

        // A writes elsewhere, calling the name it read. StaleRead, on a different file.
        if let Ok(verdict) = registry
            .admit(a, caller.clone(), &format!("step_{round:02}()\n"))
            .await
        {
            observer.emit(Observation::Write {
                session: a,
                path: caller,
                verdict,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A repository is not a sandbox. The scenario must refuse to write into one.
    #[test]
    fn the_scenario_mode_refuses_a_git_repository() {
        let root = std::env::temp_dir().join(format!("trame-guard-{}", ProjectId::new()));
        std::fs::create_dir_all(root.join(".git")).unwrap();
        let error = refuse_dangerous_root(&root).unwrap_err().to_string();
        assert!(error.contains(".git"), "the reason must say why: {error}");
        assert!(
            error.contains("auth.rs"),
            "and what would be written: {error}"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    /// ★ Scenario mode produces enough observations to exceed any window height.
    ///
    /// # Why this is a test and not an eyeball check
    ///
    /// The scroll path in both interfaces is only executed once the list is taller than the
    /// window. For as long as `--scenario` emitted four lines, that path was dead code that
    /// compiled — and the way this project usually finds that out is a real session
    /// producing forty lines in front of a user.
    ///
    /// So the count is pinned. `FILLER_ROUNDS` can be tuned; dropping it low enough that
    /// scrolling stops being exercised fails here instead of silently in the GUI.
    ///
    /// It also checks the mix: a feed of nothing but `Clean` would scroll without showing
    /// that a `StaleRead` looks different, which is the one thing the display has to get
    /// right.
    #[tokio::test]
    async fn the_scenario_produces_enough_observations_to_scroll() {
        let root = std::env::temp_dir().join(format!("trame-scroll-{}", ProjectId::new()));
        let source = open(&root, Arc::new(trame_core::clock::SystemClock), true)
            .await
            .expect("scenario mode creates its sandbox");
        let mut observations = source.observations;

        // Wait on a condition with a cap, never on a fixed delay: the filler paces itself
        // and the admissions take as long as the disk takes.
        // Every observation is a feed row, so THIS is the observable that decides whether
        // the list outgrows the window. Counting writes alone was the first version, and it
        // measured a proxy: reads and session lines take up rows too.
        let mut rows = 0_usize;
        let mut stale = 0_usize;
        let mut clean = 0_usize;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
        while tokio::time::Instant::now() < deadline {
            match tokio::time::timeout(std::time::Duration::from_secs(2), observations.recv()).await
            {
                Ok(Some(Observation::Write { verdict, .. })) => {
                    rows += 1;
                    match verdict {
                        trame_core::Verdict::StaleRead { .. } => stale += 1,
                        trame_core::Verdict::Clean => clean += 1,
                        _ => {}
                    }
                }
                Ok(Some(_)) => rows += 1,
                // Channel closed, or nothing more coming: the scenario is done.
                Ok(None) | Err(_) => break,
            }
        }

        let on_disk = std::fs::read_dir(&root).map(|d| d.count()).unwrap_or(0);
        std::fs::remove_dir_all(&root).ok();

        // The registry writes to disk itself (ADR 0014), so the rows above correspond to
        // real files. Checking it here is what makes "nothing is fabricated" verified
        // rather than asserted.
        assert!(
            on_disk >= 25,
            "the scenario must have really written to disk: {on_disk} files"
        );

        // A generous window is ~40 rows. The canonical scenario alone gives 6.
        assert!(
            rows >= 48,
            "the scenario must fill more than a window: got {rows} rows, which leaves the \
             scroll path in the TUI and the GUI unexercised"
        );
        assert!(
            stale >= 5 && clean >= 5,
            "the feed must mix verdicts, or scrolling shows nothing about how a StaleRead \
             differs from a Clean: {stale} stale, {clean} clean"
        );
        assert!(
            rows < crate::state::FEED_CAPACITY,
            "the scenario must not overflow the feed, or the canonical four steps scroll \
             out of reach: {rows} rows for a capacity of {}",
            crate::state::FEED_CAPACITY
        );
    }

    /// Scenario mode creates its own sandbox.
    ///
    /// This test exists because the case failed for real: `just gui-scenario /tmp/trame-demo`
    /// on a path that did not exist yet answered "invalid project root" — when the ADR was
    /// recommending exactly that path.
    #[tokio::test]
    async fn the_scenario_mode_creates_its_own_sandbox() {
        let root = std::env::temp_dir().join(format!("trame-bac-{}", ProjectId::new()));
        assert!(!root.exists());
        // We do not open the whole project here — the real journal and the watcher have
        // nothing to do with the question. We check the one thing that broke: the directory.
        let failure = open(&root, Arc::new(trame_core::clock::SystemClock), true)
            .await
            .err()
            .map(|e| format!("{e:#}"));
        assert!(
            root.is_dir(),
            "the sandbox must exist after opening (error: {failure:?})"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    /// Negative control: without `--scenario`, a missing directory stays an error.
    ///
    /// Watching a directory you just created is watching nothing — and it is more likely a
    /// typo in the path.
    #[tokio::test]
    async fn observing_a_directory_that_does_not_exist_stays_an_error() {
        let root = std::env::temp_dir().join(format!("trame-absent-{}", ProjectId::new()));
        let outcome = open(&root, Arc::new(trame_core::clock::SystemClock), false).await;
        assert!(outcome.is_err(), "watching nothing must fail");
        assert!(!root.exists(), "and must create nothing");
    }

    /// Negative control: a throwaway directory passes, or the guard would block normal use
    /// and get worked around within the week.
    #[test]
    fn a_throwaway_directory_is_accepted() {
        let root = std::env::temp_dir().join(format!("trame-guard-{}", ProjectId::new()));
        std::fs::create_dir_all(&root).unwrap();
        assert!(refuse_dangerous_root(&root).is_ok());
        std::fs::remove_dir_all(&root).ok();
    }
}
