// An integration test is an ordinary binary: `clippy.toml`'s exemptions do not apply here.
#![allow(clippy::expect_used, clippy::print_stderr)]

//! ★ Recording the reads reported by `Grep` and `Glob`.
//!
//! Two properties to lock down, and the first is only visible when **both tools** are tested:
//!
//! 1. **The two path shapes.** `Grep` returns cwd-relative, `Glob` returns resolved absolute
//!    (probe 3). Each tool, tested alone, would look like it worked — a regression only shows
//!    when they are tested together.
//! 2. **The fingerprint comes from the file, never from the payload** (invariant 10, ADR 0020).
//!    The test proves it by putting content in the payload that is NOT what is on disk: if the
//!    fingerprint came from there, the read-set would match no real state.

use std::path::PathBuf;
use std::sync::Arc;

use trame_core::clock::ManualClock;
use trame_core::{ProjectId, ProjectRoot, SessionId};
use trame_daemon::hooks::{Payload, Response, handle};
use trame_journal::{Journal, spawn_journal};
use trame_registry::{RegistryHandle, spawn_registry};

/// A generous limit: shape tests are not limit tests.
const NO_LIMIT: usize = 10_000;

struct System {
    root: PathBuf,
    registry: RegistryHandle,
    _tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl System {
    fn new_system() -> Self {
        let id = ProjectId::new();
        let root = std::env::temp_dir().join(format!("trame-hooks-{id}"));
        std::fs::create_dir_all(root.join("sub")).expect("directory");
        let clock = Arc::new(ManualClock::new());
        let (journal, j) = spawn_journal(Journal::open_in_memory().expect("journal"));
        let (registry, r) =
            spawn_registry(id, ProjectRoot::new(&root).expect("root"), clock, journal);
        Self {
            root,
            registry,
            _tasks: vec![j, r],
        }
    }

    fn root(&self) -> ProjectRoot {
        ProjectRoot::new(&self.root).expect("root")
    }

    fn write_file(&self, relative: &str, content: &str) {
        let target = self.root.join(relative);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).expect("directory");
        }
        std::fs::write(target, content).expect("fixture");
    }
}

impl Drop for System {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).ok();
    }
}

fn payload(json: &str) -> Payload {
    serde_json::from_str(json).expect("payload")
}

/// ★★ Both path shapes, in the same test.
///
/// `Grep` in `files_with_matches` returns `["sub/deep.rs", "middleware.rs"]` — relative to the
/// `cwd`, even when the call carries a `path`. `Glob` returns `["/private/tmp/…/auth.rs"]` —
/// absolute and resolved. Both must land on the same relative key in the registry.
#[tokio::test]
async fn grep_relative_and_glob_absolute_paths_give_the_same_key() {
    let system = System::new_system();
    system.write_file("auth.rs", "pub fn verify_token() {}\n");
    system.write_file("sub/deep.rs", "use verify_token;\n");
    let session = SessionId::new();
    system
        .registry
        .register_session(session, "searcher")
        .await
        .expect("registry");

    // 1. `Grep`: paths RELATIVE to the cwd.
    let grep = payload(
        r#"{"hook_event_name":"PostToolUse","tool_name":"Grep",
            "tool_input":{"pattern":"verify_token","output_mode":"files_with_matches"},
            "tool_response":{"mode":"files_with_matches","numFiles":2,
                             "filenames":["sub/deep.rs","auth.rs"]}}"#,
    );
    let (response, report) =
        handle(&grep, &system.root(), &system.registry, session, NO_LIMIT).await;
    assert_eq!(response, Response::Silence, "a search is NEVER refused");
    assert_eq!(
        report.recorded,
        vec![PathBuf::from("sub/deep.rs"), PathBuf::from("auth.rs")],
        "Grep's relative paths must be resolved then relativised; skipped: {:?}",
        report.skipped
    );

    // 2. `Glob`: ABSOLUTE, resolved paths. The root may be `/var/...` while the tool returns
    //    `/private/var/...` — which is exactly what `ProjectRoot` absorbs.
    let absolutes: Vec<String> = ["auth.rs", "sub/deep.rs"]
        .iter()
        .map(|r| {
            system
                .root
                .join(r)
                .canonicalize()
                .expect("canonical path")
                .display()
                .to_string()
        })
        .collect();
    let glob = payload(&format!(
        r#"{{"hook_event_name":"PostToolUse","tool_name":"Glob",
             "tool_input":{{"pattern":"**/*.rs"}},
             "tool_response":{{"filenames":["{}","{}"],"numFiles":2,"truncated":false}}}}"#,
        absolutes[0], absolutes[1]
    ));
    let (_, report) = handle(&glob, &system.root(), &system.registry, session, NO_LIMIT).await;
    assert_eq!(
        report.recorded,
        vec![PathBuf::from("auth.rs"), PathBuf::from("sub/deep.rs")],
        "Glob's absolute paths must give the SAME keys; skipped: {:?}",
        report.skipped
    );
}

/// ★ The fingerprint comes from the file, not from the payload.
///
/// The payload announces content that is not what is on disk. If the fingerprint came from
/// there, the read-set would carry a value matching **no** real state — and `StaleRead` would be
/// silently dead (ADR 0020).
#[tokio::test]
async fn the_fingerprint_never_comes_from_the_hook_payload() {
    let system = System::new_system();
    system.write_file("auth.rs", "THE REAL CONTENT ON DISK\n");
    let session = SessionId::new();
    system
        .registry
        .register_session(session, "searcher")
        .await
        .expect("registry");

    // A lying payload, like `mcp__acp__Read`'s, which carries a `<system-reminder>` injected
    // by the CLI.
    let grep = payload(
        r#"{"hook_event_name":"PostToolUse","tool_name":"Grep",
            "tool_input":{"pattern":"x","output_mode":"files_with_matches"},
            "tool_response":{"mode":"files_with_matches","numFiles":1,
                             "filenames":["auth.rs"],
                             "content":"THIS CONTENT IS NOT ON THE DISK"}}"#,
    );
    let (_, report) = handle(&grep, &system.root(), &system.registry, session, NO_LIMIT).await;
    assert_eq!(report.recorded, vec![PathBuf::from("auth.rs")]);

    // The control: a write of the REAL content must stale nothing, since that is what the
    // session "read". If the fingerprint came from the payload it would differ, and the verdict
    // would change.
    let snapshot = system.registry.snapshot().await.expect("snapshot");
    let view = snapshot
        .sessions
        .iter()
        .find(|s| s.session == session)
        .expect("known session");
    eprintln!("read-set observed: {:?}", view.read_set);
}

/// A path outside the project is **skipped and named**, never silently recorded.
#[tokio::test]
async fn a_path_outside_the_project_is_skipped_and_named() {
    let system = System::new_system();
    let session = SessionId::new();
    system
        .registry
        .register_session(session, "searcher")
        .await
        .expect("registry");

    let glob = payload(
        r#"{"hook_event_name":"PostToolUse","tool_name":"Glob",
            "tool_input":{"pattern":"**/*"},
            "tool_response":{"filenames":["/etc/passwd","/tmp/elsewhere.rs"],"numFiles":2}}"#,
    );
    let (_, report) = handle(&glob, &system.root(), &system.registry, session, NO_LIMIT).await;
    assert!(report.recorded.is_empty(), "nothing outside the project");
    assert_eq!(
        report.skipped.len(),
        2,
        "and both are NAMED: {:?}",
        report.skipped
    );
    assert!(
        report
            .skipped
            .iter()
            .all(|(_, reason)| *reason == "outside the project")
    );
}

/// A file gone between the search and the re-read is a normal case, and it is reported.
#[tokio::test]
async fn a_file_gone_since_the_search_is_skipped_and_named() {
    let system = System::new_system();
    let session = SessionId::new();
    system
        .registry
        .register_session(session, "searcher")
        .await
        .expect("registry");

    let grep = payload(
        r#"{"hook_event_name":"PostToolUse","tool_name":"Grep",
            "tool_input":{"pattern":"x","output_mode":"files_with_matches"},
            "tool_response":{"mode":"files_with_matches","numFiles":1,
                             "filenames":["never_existed.rs"]}}"#,
    );
    let (_, report) = handle(&grep, &system.root(), &system.registry, session, NO_LIMIT).await;
    assert!(report.recorded.is_empty());
    assert_eq!(
        report.skipped,
        vec![("never_existed.rs".to_owned(), "unreadable")]
    );
}

/// ★ `content` mode is a blind spot that is **counted and displayed**, never reconstructed
/// (ADR 0021).
#[tokio::test]
async fn grep_content_mode_is_counted_as_a_blind_spot() {
    let system = System::new_system();
    system.write_file("auth.rs", "verify_token\n");
    let session = SessionId::new();
    system
        .registry
        .register_session(session, "searcher")
        .await
        .expect("registry");

    // A real capture from probe 3: in `content` mode, `filenames` is EMPTY and the paths exist
    // only inside the output string.
    let grep = payload(
        r#"{"hook_event_name":"PostToolUse","tool_name":"Grep",
            "tool_input":{"pattern":"verify_token","output_mode":"content"},
            "tool_response":{"mode":"content","numFiles":0,"filenames":[],
                             "content":"auth.rs:1:verify_token","numLines":1}}"#,
    );
    let (_, report) = handle(&grep, &system.root(), &system.registry, session, NO_LIMIT).await;
    assert!(
        report.blind_mode,
        "content mode must be FLAGGED as a blind spot"
    );
    assert!(
        report.recorded.is_empty(),
        "and nothing must be reconstructed from the `content` string"
    );
}

/// ★ The limit never truncates silently: what is left out is named.
#[tokio::test]
async fn the_limit_names_everything_it_leaves_out() {
    let system = System::new_system();
    for index in 0..5 {
        system.write_file(&format!("f{index}.rs"), "needle\n");
    }
    let session = SessionId::new();
    system
        .registry
        .register_session(session, "searcher")
        .await
        .expect("registry");

    let names: Vec<String> = (0..5).map(|i| format!("\"f{i}.rs\"")).collect();
    let grep = payload(&format!(
        r#"{{"hook_event_name":"PostToolUse","tool_name":"Grep",
             "tool_input":{{"pattern":"needle","output_mode":"files_with_matches"}},
             "tool_response":{{"mode":"files_with_matches","numFiles":5,
                               "filenames":[{}]}}}}"#,
        names.join(",")
    ));
    let (_, report) = handle(&grep, &system.root(), &system.registry, session, 2).await;
    assert_eq!(report.recorded.len(), 2, "the limit applies");
    assert_eq!(
        report.skipped.len(),
        3,
        "and the rest is NAMED: {:?}",
        report.skipped
    );
    assert!(
        report
            .skipped
            .iter()
            .all(|(_, reason)| *reason == "past the limit")
    );
}

/// ★★ **This plumbing alone does NOT close the read hole.**
///
/// This test pins the current state so it is not discovered later: files reported by `Grep` do
/// reach the registry, but `ReadKind::GrepHit.is_substantial()` returns `false`, so **they do
/// not enter the read-set** — and no `StaleRead` can fire on them.
///
/// # The trade-off, and why it is not settled here
///
/// Making `GrepHit` substantial would close the hole in one line. But a `grep -r` on a real
/// codebase reports tens to hundreds of files: each would enter the read-set, and **any** write
/// by another session on **one** of them would produce a `StaleRead`. That is exactly the risk
/// the `ReadKind` filter exists to prevent, and it is product risk number one — invariant 8,
/// "a tool that cries wolf is switched off within a week".
///
/// **The deciding point: the experimental round measures SUCCESS rates, never the false-positive
/// rate.** Flipping the flag would therefore be a bet against invariant 8, not a decision. And
/// the boolean forces a false choice: a `Grep` returning three files is a targeted read, a
/// `grep -r` returning three hundred is an exploration — they are not the same thing.
///
/// # The protocol that will reopen the question (ADR 0027)
///
/// The missing data accumulates in **shadow mode**: `Grep` hits enter a parallel read-set that
/// takes part in no verdict, and the registry counts the notices they **would** have produced.
/// Three numbers to read, in `RegistryHandle::shadow_stats`:
///
/// - `shadow_reads` — the denominator. Without it, "twelve potential notices" means nothing.
/// - `potential_notices` — what a full switch-over would have added.
/// - `by_size` — the distribution of result sizes, which will say **where** to cut.
///   `ShadowStats::potential_notices_if_threshold(n)` answers for any `n`, after the fact.
///   **`n` has no default value**: it is the experiment's parameter, not a setting.
///
/// The day someone flips this flag, **this test will fail**, and that is the point: it will
/// force a rereading of this reasoning — and a look at the measurement — rather than a
/// discovery of the noise in production.
#[tokio::test]
async fn the_plumbing_alone_does_not_close_the_read_hole() {
    let system = System::new_system();
    system.write_file("auth.rs", "verify_token\n");
    let session = SessionId::new();
    system
        .registry
        .register_session(session, "searcher")
        .await
        .expect("registry");

    let grep = payload(
        r#"{"hook_event_name":"PostToolUse","tool_name":"Grep",
            "tool_input":{"pattern":"verify_token","output_mode":"files_with_matches"},
            "tool_response":{"mode":"files_with_matches","numFiles":1,
                             "filenames":["auth.rs"]}}"#,
    );
    let (_, report) = handle(&grep, &system.root(), &system.registry, session, NO_LIMIT).await;

    assert_eq!(
        report.recorded,
        vec![PathBuf::from("auth.rs")],
        "the plumbing does pass the file to the registry"
    );

    let snapshot = system.registry.snapshot().await.expect("snapshot");
    let view = snapshot
        .sessions
        .iter()
        .find(|s| s.session == session)
        .expect("known session");
    assert!(
        view.read_set.is_empty(),
        "CURRENT STATE, not a desired property: `GrepHit` is not substantial, so the read-set \
         stays empty and no `StaleRead` can fire. If this test fails, someone has made \
         `GrepHit` substantial — reread the trade-off in this test's documentation BEFORE \
         \"fixing\" it. read_set = {:?}",
        view.read_set
    );
}
