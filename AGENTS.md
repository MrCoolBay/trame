# Trame

A macOS desktop application, in Rust, that orchestrates several coding agents (Claude Code,
Codex, Gemini CLI) working **in parallel in one shared working directory**.

> **This file is the project's canonical framing.** It is deliberately neutral: Trame
> orchestrates three different harnesses, and the day a Trame session launches Codex or
> Gemini on the Trame repository itself, this is the file they will read. `CLAUDE.md`
> imports it and adds only what is specific to Claude Code.
> A piece of information has **one home**: copy nothing from here to anywhere else.

## The thesis — read these five lines before anything else

Every competing tool (Conductor, Xirp, Crystal, pdb-env) isolates each agent: a git worktree
or a directory copy per session. Isolation removes collisions, but it makes coordination
**impossible** — each agent is blind to the others by construction. Trame takes the opposite
bet: **shared directory + enforced coordination**.

> When agent A is about to write, if a file it **read** has since been modified by another
> session, A is reasoning about a world that no longer exists. Trame detects that and
> **tells it**.

That is the only mechanism in the product that exists nowhere else, and the only one
structurally out of reach for competitors: too late at the forge (the code is already
written), impossible at the filesystem level (the agents are isolated).

**Everything else in this project exists to serve that mechanism.** Faced with a design
decision, the question is: does this serve that notice? If not, it is probably out of scope.

The failure mode to catch, the one that produces **no write collision at all**:

```
1. Session A reads auth.rs, remembers verify_token()'s signature
2. Session B writes auth.rs, renaming verify_token() -> validate_token()
3. Session A writes handlers.rs, calling verify_token()

Two different files. A per-file lock sees nothing. The tree is broken.
```

## Settled decisions — do not reopen them

They are decided. If you think one is wrong, **say so and argue it**, but do not deviate
without approval. One ADR per row, in [`docs/adr/`](docs/adr/).

| Decision | Choice | Reason | ADR |
|---|---|---|---|
| Platform | macOS only | FSEvents, Keychain, launchd. No cross-platform abstraction. | [0001](docs/adr/0001-macos-uniquement.md) |
| Isolation | **None.** One working directory per project | It is the precondition for coordination. | [0002](docs/adr/0002-aucune-isolation.md) |
| VCS | GitButler through the `but` CLI, as a shell-out | The needed surface is ~7 commands. Reimplementing would be 6-18 months on a commodity. | [0003](docs/adr/0003-gitbutler-en-shell-out.md) |
| VCS parsing | `but ... --format json`, always | A structured API, not scraping. | [0004](docs/adr/0004-parsing-json-du-vcs.md) |
| Agent transport | ACP first, PTY as fallback | ACP allows intercepting writes **before** the disk. Indispensable. | [0005](docs/adr/0005-acp-en-premier-pty-en-secours.md) |
| Interception | **Validated**: announcing `fs.writeTextFile` disables the agent's native write tools | Holes named and measured: `Bash`, out-of-band, PTY. A net whose holes you do not know is worse than a net whose holes you do. | [0016](docs/adr/0016-interception-avant-disque-validee.md) |
| ACP adapter | **Pinned** to `@zed-industries/claude-code-acp` 0.16.2, despite its deprecation | The successor no longer removes `Write` or `Edit`: migrating would silently delete the central mechanism. A canary watches it. | [0017](docs/adr/0017-adaptateur-acp-epingle.md) |
| Concurrency | tokio actors, one per domain | mpsc + oneshot. **No shared state.** | [0006](docs/adr/0006-acteurs-tokio.md) |
| Concurrency control | Optimistic, read-set validation | Pessimistic locking starves on transactions lasting minutes. | [0007](docs/adr/0007-concurrence-optimiste-read-set.md) |
| Storage | SQLite through `rusqlite`, append-only | We will want to query across projects. | [0008](docs/adr/0008-journal-sqlite-append-only.md) |
| Licence | **Open source, MIT OR Apache-2.0** | The Rust convention. Protection does not come from a clause. Supersedes ADR 0009's FSL choice. | [0013](docs/adr/0013-licence-open-source-mit-apache.md) |
| Parallelism | By **project**, not by session | 2-5 sessions per project. 5 projects × 3 sessions = 15 agents, all safe. | [0010](docs/adr/0010-parallelisme-par-projets.md) |
| Forge **driven** | GitLab **self-hosted** as the first target | `base_url` is a first-class field from the start. `ChangeRequest`, never `PullRequest`. | [0011](docs/adr/0011-gitlab-self-hosted-en-premier.md) |
| Hosting **Trame** | GitHub | Under MIT/Apache, hosting belongs where the contributors are. **This does not affect the row above**: Trame is hosted on GitHub and speaks GitLab. | [0019](docs/adr/0019-heberger-trame-sur-github.md) |
| v0.1 granularity | Whole file, no hunks | 90% of the value for 5% of the work. We refine after measuring. | [0012](docs/adr/0012-granularite-fichier-en-v0-1.md) |
| Disk writes | **The registry writes**, it does not merely return a verdict | An invariant that rests on the caller's discipline is not an invariant. | [0014](docs/adr/0014-le-registre-ecrit-sur-disque.md) |
| Backpressure | Channel bounded at 64, we wait when saturated | An unbounded queue turns an overload into a memory leak. Saturation is a bug, not a shortage of capacity. | [0015](docs/adr/0015-canal-admit-borne.md) |
| Interface | **It observes, it does not drive**: a `Receiver<Observation>`, no `RegistryHandle` | The daemon is the product, the GUI is interchangeable — and that is what makes betting on a pre-1.0 framework acceptable. | [0022](docs/adr/0022-decoupage-daemon-gui.md) |
| Read hole | **Open**, and measured in **shadow mode** | Closing it without measuring the false-positive rate would be a bet against invariant 8. The shadow counts what we would have said and says nothing; the size distribution will give the threshold. | [0027](docs/adr/0027-trou-lecture-ouvert-et-mesure-en-ombre.md) |
| CLI hooks | `trame-hook` asks the daemon over a **unix socket per project**; an absent daemon makes the hook **fail** | On the admission path, the absence of an answer is never a yes. A hook that exits 0 without consulting the policy kills the invariant silently. | [0025](docs/adr/0025-ipc-hook-daemon.md) |
| Our own write tool | **No.** A documented path, not built | It would double the write path's surface, and nothing says the agent would pick our tool over the `Write` it already knows. Three re-examination triggers, all observable. | [0024](docs/adr/0024-pas-de-serveur-mcp-maison.md) |
| Component library | `gpui-component` **0.5.1**, from crates.io | The multi-line field was worth the dependency; the rest was not. `Styled` being exposed, plus `.refine_style()` refining on top of the preset, give the escape hatch — we dress the library rather than being dressed by it. **Multi-line field: `auto_grow(min, max)` by default, one call and it grows. The `multi_line(true).rows(n)` + height-on-the-element path, three calls, is for a fixed height.** | [0028](docs/adr/0028-adoption-de-gpui-component.md) |
| GUI framework | `gpui` from **Zed upstream**, pinned at 0.2.2 | Crate ownership established by the crates-io team, API parity observed (probe rebuilt without touching `main.rs`), a version rather than a git branch. `gpui-ce` remains the escape hatch, already tested. | [0023](docs/adr/0023-gpui-amont-pour-la-gui.md) |

## Non-goals — to be refused explicitly

- ❌ Windows, Linux
- ❌ An embedded code editor (**this is not an IDE**)
- ❌ A proprietary model or agent
- ❌ SaaS, accounts, a backend, multi-user
- ❌ Worktrees, containers, copy-on-write, microVMs
- ❌ Webhooks, polling, automatic session triggering
- ❌ **Any form of isolation**

## Architectural invariants

These are invariants, not preferences. A violation is a bug.

1. **An actor owns its state.** Never `Arc<Mutex<_>>` for business state. Communication goes
   through `mpsc` in and `oneshot` back. That gives serialisation and a total order **by
   construction**, with no lock. An `Arc` over an immutable value (a clock, a config) is not
   covered.
2. **The registry is the single point of passage for agent writes made by the file tools** —
   `Write`, `Edit`, `NotebookEdit` — and **it is the registry that writes** (ADR 0014).
   Writes through the agent's shell (`Bash`) escape it: that is measured, accepted, and must
   be displayed as such (ADR 0016). It does not return a verdict and leave the caller to
   write: an invariant that rests on the discipline of every call site is not an invariant. A
   write that bypasses the registry is a write with no provenance, and therefore a false row
   in the journal — worse than no journal.
   Out-of-band writes (`sed -i`, hooks, builds, formatters) are caught after the fact by
   FSEvents and never admitted.
   **Symmetrically, the read-set contains only reads made through the ACP read tool.** A read
   through `Grep`, `Glob` or `Bash` escapes the registry — and that is worse than an escaping
   write: with no read-set entry, `StaleRead` never fires and nothing says so. The way out is
   **measured but not built**: the `PostToolUse` hook reports the files read by `Grep` and
   `Glob` ([probe 3](docs/sondes/2026-08-12-postooluse.md)). Until that is implemented, the
   statement above stands exactly as written.
   On the write side, the **FSEvents watcher** catches the out-of-band: it prevents nothing,
   but it prevents the registry from becoming **wrong**. Without it, a `sed -i` leaves a stale
   `FileState` and the corresponding `StaleRead` never fires. Those writes are attributed to
   `SessionId::EXTERNAL` and journalled with `origin = observed`, **with no verdict** — nobody
   admitted them.
3. **The sequence number is per project, never global.** A global counter would be a point of
   contention between projects that, by construction, cannot collide. Constraint
   `UNIQUE(project_id, seq)`.
4. **No `unwrap()` / `expect()` / `panic!()` outside tests.** Denied by clippy at the
   workspace level; `clippy.toml` carries the test exemptions.
5. **Errors: `thiserror` in libraries, `anyhow` only in binaries.** A library that returns
   `anyhow::Error` forces its caller to pattern-match on strings.
6. **All I/O is instrumented with `tracing`.** Never `println!` / `eprintln!` — both are
   denied by clippy. Logs go to stderr: stdout belongs to ratatui's alternate screen and to
   JSON-RPC.
7. **`trame-core` depends on no internal crate.** The dependency direction is one-way:
   `core <- journal <- registry <- {agent, vcs} <- daemon <- view <- {tui, gui}`. An interface
   receives only a `Receiver<Observation>`, **never a `RegistryHandle`**: "it observes, it does
   not drive" is in the typing (ADR 0022), and enforced by the crate graph rather than by a
   struct's shape — `scripts/interface_boundary.py` fails if an interface crate takes
   `trame-registry` as a normal dependency.
8. **Silent when clean.** ~95% of traffic must pass without a word. A tool that cries wolf is
   switched off within a week — that is product risk number one, ahead of any technical risk.
9. **Nothing is blocked in v0.1.** The registry observes, journals and informs. Blocking will
   be decided after the real false-positive rate has been measured.
10. **A read's fingerprint is computed only from the content served in response to
    `fs/read_text_file`** — never from a hook payload (ADR 0020). The CLI injects a
    `<system-reminder>` into that payload: the fingerprint would match **no** state of the
    disk, and the failure would be entirely silent — read-set populated, `StaleRead` dead, no
    test broken. When a hook reports a path (`Grep`, `Glob`), Trame **re-reads the file** to
    fingerprint it; the hook supplies paths, never content.

## Structure

```
crates/
├── trame-core/       # shared types, seams. No internal dependency.
├── trame-journal/    # append-only SQLite, global to the workspace
├── trame-registry/   # ★ the admission actor — the core of the product
├── trame-agent/      # trait AgentBackend, AcpBackend, PtyBackend
├── trame-vcs/        # trait VcsBackend, ButBackend
├── trame-daemon/     # Supervisor, orchestration, observation channel, project opening
└── trame-view/       # display state, shared by the interfaces
apps/
├── trame-tui/        # ratatui — terminal rendering, and nothing else
└── trame-gui/        # gpui (Zed upstream) — the desktop application
```

`trame-vcs` is still nearly empty. **That is deliberate**: crate boundaries *are* the
architecture. Drawing them now costs a day, retrofitting them costs a rewrite.

### `trame-core`'s seams

Defined from phase 0, nearly useless today, structural in six months:

- `TaskSource` — where work comes from. One implementation in v0.1: `ManualTask` (the user
  types their prompt).
- `Forge` — where the result goes. **Neutral naming: `ChangeRequest`, never `PullRequest`.**
  GitLab is the primary target, not a second-class citizen.
- `PromptContributor` — the prompt composition pipeline. **Not speculative**: it is the
  mechanism through which the stale-read notice is injected. v0.1 needs it.
- `BranchTarget` — `New(BranchName)` or `Existing(BranchId)`. Without it, handling review
  comments on a change request would force a refactor.
- `Session.work_item: Option<WorkItemRef>` — closes the full auditable chain
  `issue -> session -> agent -> writes -> hunks -> branch -> change request`.
- `Clock` — every read of the time goes through it. The registry makes decisions that depend
  on time; testing them against the system clock would force `sleep`s, and therefore slow,
  flaky tests.

## Commands

```sh
just check       # compile the whole workspace, tests included
just test        # the full suite
just test-one X  # a single test, by name, with --nocapture
just lint        # fmt --check + clippy -D warnings + the guards. What CI checks.
just run         # the daemon, logs on stderr
just tui         # the TUI
just ci          # lint + test + release build, locally before pushing
just status      # but status --format json
```

Zero warnings tolerated. CI ([`.github/workflows/ci.yml`](.github/workflows/ci.yml)) fails on
the smallest clippy warning, which includes a missing doc comment on a public item.

**Two exclusions in CI, and they are validity conditions, not oversights.** `trame-gui` does
not run in the Linux jobs — gpui has no platform layer without `x11`/`wayland`. And
`real_watcher` only compiles on macOS: `notify` picks inotify on Linux, so a green Linux run
on that file would measure a different backend from the one its title names. Both are covered
by the `macos` job, which has been on the critical path since we measured that a GitHub macOS
runner really is inside an **Aqua** session — the window opens and the shader smoke test
returns `SMOKE_OK`.

## Licence

Trame is **open source** under **MIT OR Apache-2.0**
([ADR 0013](docs/adr/0013-licence-open-source-mit-apache.md)). No CLA: a contribution is
offered under the same terms, as everywhere in the Rust ecosystem.

`but` (GitButler, under FSL-1.1-MIT) is an **external prerequisite installed by the user,
never vendored** — that is a licence constraint, not a packaging preference
([ADR 0003](docs/adr/0003-gitbutler-en-shell-out.md)).

## Where the project stands

**Phases 0 to 4.1 delivered, TUI and GUI included.** 187 tests (191 with doctests). The only
`sleep`s in the repository are in `real_watcher.rs`, where FSEvents is a system service no
injected clock controls — and even there, by waiting on a condition with a ceiling, not by a
fixed delay.

- **Phase 0** — tooling, crate boundaries, seams, ADRs, skills.
- **Phase 1** — `trame-journal` (six append-only tables, real writes) and `trame-registry`
  (the admission actor). The canonical scenario passes: A reads `auth.rs`, B writes `auth.rs`
  (→ `Clean`), A writes `handlers.rs` (→ `StaleRead { auth.rs, by B }`). Two different files,
  no write collision.
- **Phase 2** — `trame-agent`: `AgentBackend`, a normalised stream, `AcpBackend` for Claude
  Code, `PtyBackend` as an honest skeleton. Interception before the disk is **validated, live
  run included** (ADR 0016): two real Claude Code sessions asked to write, we refused, nothing
  reached the disk.

**A rule born from the live run**: every file key goes through `trame_core::ProjectRoot`. The
agent returns absolute, resolved paths (`/private/var/…` when the root is `/var/…`); without
normalisation, `StaleRead` stops firing **without anything breaking**, and the tests still
pass.

- **Phase 3** — 3.1 and 3.2 delivered: the registry **writes** after admission (ADR 0014), and
  `SessionPilot` wires the full chain. The end-to-end test drives the canonical scenario
  through the real transport, up to the notice placed in front of the next prompt.
  3.3 delivered on the tooling side: `just experiment`
  (`-p trame-tui --example notice_experiment`) measures the notice variants on real sessions.

  3.3 **decided, and the shipped text changed**: `StaleFile` carries **no** summary of the
  change and the registry computes no diff at admission (ADR 0018) — that part holds, and the
  2026-08-13 measurement reinforces it rather than the opposite.
  **But the shipped text had never been measured.** The `5/5` then `3/3` were on
  `ConfigurableNotice::Neutral`, a twin of `StaleReadNotice` bar one line. Measured directly,
  production scored **`3/6`**, against `3/3` for the neutral and `3/3` for the directive, same
  day and same conditions. The string Trame was sending was **the only one of the three that
  failed**. The third line — `Re-read it before continuing if your work depends on it.` — was
  **removed**, and the replay gives **`6/6`**. The notice is two lines:

  ```
  [Trame] auth.rs was changed by session "refactor-api"
          after you read it (a few seconds ago).
  ```

  The mechanism, which generalises beyond this case: **an agent that receives a fact acts on
  it; an agent that receives a fact plus permission to ignore it ignores it half the time.**
  And the reserve, not to be lost: **six runs give no statistical power** — what is solid is
  the direction, not the magnitude.

  3.4 — the **FSEvents watcher**, then the **TUI**. `trame_daemon::observe` carries the
  observation channel, one-way: the interface receives a `Receiver<Observation>` and **no
  `RegistryHandle`**, so it structurally cannot drive. It shows one panel per session with its
  state, the verdict feed, `StaleRead` distinguished from `Clean`, the **admitted / observed**
  distinction, and a degradation banner when `can_intercept_writes` is false.
  `trame-tui <project> [--scenario]` opens the real journal, the real registry and the real
  watcher.

  What rendering in a real terminal found, and the tests could not see: the watcher was
  emitting observations **without knowing** whether the registry had recorded them. Since the
  registry writes itself (ADR 0014), FSEvents reports its own writes, and the interface showed
  them as out-of-band — the exact opposite of the truth.
  `RegistryHandle::observe_external_write` now returns an `ExternalWrite::{Recorded, Echo}`.

- **Phase 4.1** — `apps/trame-gui`, the same display scope as the TUI, on gpui pinned at 0.2.2
  from Zed upstream, with `gpui-component` 0.5.1 adopted for the multi-line field (ADR 0028).

### Validation debt, not to be forgotten

The round's `15/15` signalled **a test that no longer discriminates**, not an optimal message —
and that was truer than we knew: **it was not even measuring the right text.** The device
started discriminating the day it was given `StaleReadNotice` to measure, and it immediately
produced a failure, `3/6`.

The scenario is still short (three turns), the accumulated context small, and the change being
measured — a renamed identifier — the most legible one there is. `Grep`/`Glob`/`Bash` were
closed; that particular limit has been lifted (ADR 0018) and the read-set filled anyway.

**Two things not to conflate in a progress report:**

- **The wording of the notice's third line** is open, with a measurement behind it and two
  candidate causes still confounded — the pronoun `it` and the conditional `if your work
  depends on it`. Discriminating them means varying one point at a time.
- **The question of the summary** stays closed. None of the three forms carries a diff, and the
  neutral says **less** than production while succeeding more: the failure is not explained by
  a lack of context. The reopening trigger remains the realistic case — a long session, a
  subtle change, a plan already committed to. Detail in ADR 0018.

### Language debt, dated and counted

2026-08-13. The repository was written in French and converted to English. Two directories are
**deliberately still French**, and this is a visible debt rather than an oversight:

| Left in French | Detected lines | Files | Why it can wait |
|---|---|---|---|
| `docs/adr/` | 2,194 | 29 | They describe past decisions and prescribe nothing. Their filenames also need renaming, with every cross-reference updated. |
| `docs/sondes/` | 674 | 6 | Probe reports: a dated record of what was measured, read after the fact and rarely. |

Both are on an explicit exclusion list in `just check-language`, so CI stays green rather than
permanently red — **a guard that is red all the time is a guard that gets switched off**, which
is invariant 8 applied to our own tooling.

The exclusion is not permission. A **new** ADR is written in English, and the `adr-format`
skill says so. The pass is planned; when it runs, the criterion is
**translate in full what will be quoted, summarise what will be reread**: measurement tables,
verdicts and reproducible method get translated; the narrative of how a harness broke three
times gets a synthesis paragraph and a pointer to the commit.

One detail that is not cosmetic: because the filenames stay French for now, every file that
*links* to an ADR carries French inside a path it cannot rename. `scripts/no_french.py` strips
those two directories' paths before scanning, with a two-way negative control so the exception
cannot become a way to smuggle French in. The regex erases itself the day the ADRs are renamed.

The phases and their stopping points are in [`docs/concept.md`](docs/concept.md) (Roadmap
section). **One phase at a time, stopping at each checkpoint.**

## How to work here

- One ADR per non-trivial decision, with a "what would invalidate this decision" section
  containing an **observable** condition.
- **Tests before wiring** on anything that touches concurrency.
- No `sleep` in tests. The clock is injected.
- If something about the architecture is ambiguous: **ask**, do not guess.
- **Anything that crosses a boundary is seen running for real.** See below.

### ★ The rule born from the same bug ten times

> **Every mechanism that crosses a boundary — a third-party protocol, the filesystem, a
> terminal — must have been seen running for real before it counts as settled.** Tests
> establish that it is consistent with what we believe about the boundary. They never
> establish what the boundary does.

Ten times on this project, the same failure mode. Every time it was **real execution** that
settled it, never the test suite — which was green.

The table below holds nine of the ten; the eighth has its own section further down, because it
is the pattern applied to a negative control rather than to the product.

| What was asserted | What was actually happening | What found it |
|---|---|---|
| the stream emits `Done` at end of turn (phase 2) | the test **emitted the expected notification itself**, and it does not exist | the first round with a real agent, stuck between two turns |
| `PostToolUse` fires after a refusal (probe 3) | the heredoc was python's stdin, so the hook observed **nothing** | a count: "`pre.jsonl` should hold one line per call" |
| the interface distinguishes admitted from observed (TUI) | the watcher showed the **registry's own** writes as out-of-band | rendering in a real terminal, before a test existed |
| the watcher sees out-of-band writes for the whole session (`--tui`) | a `?` on session opening released the foundation, and the watcher **stopped** | a write made by hand during a run, which did not appear |
| `real_watcher` tests FSEvents (CI) | `notify` picks **inotify** on Linux: a Linux job would have validated a different backend | reading the code while preparing the CI migration — **the first one caught before damage** |
| the echo of an admitted write consumes no sequence number | the assertion compared the **global** counter for a **per-file** property; fixture writes were advancing it | the CI's macOS job. The test had been passing **by luck** for weeks, on a timing coincidence specific to one machine |
| the round measures the product's notice (ADR 0018) | it measured `ConfigurableNotice`, a **twin** of `StaleReadNotice` bar one line. The shipped text scores `3/6` where the twin scores `3/3` | a **side-by-side reread**, while translating the repository. Neither a test nor a run: the two strings read within the same hour |
| a `_ => panic!()` warns if a `Command` variant appears | `#[non_exhaustive]` only constrains **other** crates: inside the crate that defines the type, the arm was **dead** | **clippy**, `unreachable_pattern`. The first of the series found by a lint |
| `multi_line(true)` has **no** public workaround (probe 6) | `InputState::rows(n)` is a **public** builder, line 495. I had grepped a list of **guessed** names and concluded on the absence of what I had not looked for | **the official documentation**, quoted by the human. Not a test, not a run, not a lint: a different source from the one I had chosen |

The mechanism is always the same, and that is why it repeats: **a plausible output triggers no
verification.** A green test, a credible stream, a screen that fills up — none of these ask to
be looked at more closely. A crash does.

The boundaries were different — an unspecified protocol, a hook contract, a terminal, a
filesystem — and their nature changed nothing: every time we had **modelled** their behaviour
and tested our model.

The fourth is the most instructive about lifetime, because **no test could have seen it**: the
mechanism worked, it simply did not live long enough. A lifetime is not tested by questioning a
function — it is observed by watching the screen while doing something.

**The seventh is the worst of the series**, because the boundary was not outside: it was our own
measuring device. It worked perfectly, it just measured **something other than the product** —
and its `15/15` ceiling made the substitution undetectable for two campaigns.

> **A measuring harness must consume the production component, not a twin.** If the measurement
> goes through a type dedicated to the experiment, that type is built as a **comparison against
> production**, and a test observes that they differ.

The three properties that made the trap invisible are worth recognising elsewhere: the two texts
**resembled each other**, the **ceiling masked** any possible gap, and **no test compared them**
— each was pinned against itself. It is the same blind spot as the global counter in the fifth
case: the chosen observable could not express the property.

### ★★ The nastiest case: the pattern applied to the verification loop

The cases above are about the product — except the seventh, which was about the measuring
device. This one is about **the control itself**, and that is why it deserves its own section.

```sh
just lint >/dev/null 2>&1 && echo "lint OK"     # ← NEVER WRITE THIS
```

When the command fails, this form **prints nothing**. No error, no mention, nothing — and an
absent line reads as a success when you skim output. It happened twice in a row in the same
session, with a commit on top each time.

> **Rule: every control command displays success AND failure explicitly.** Never one by the
> absence of the other.

```sh
if just lint; then echo "LINT: GREEN"; else echo "LINT: RED"; fi
```

That form immediately revealed a **second** error CI had not yet seen.

The corollary holds for any verification script: a `python3` that raises before writing its file
leaves the code unchanged, and the test that follows passes — testing the old version.
**Verifying that a modification actually happened is part of the verification.**

What this imposes, concretely:

- **The order.** See it run first, lock it with a test second. The reverse produces tests that
  pin the belief rather than the behaviour. The third bug happened with 139 green tests and cost
  a ten-second run in a pty.
- **A negative control** on every measuring device: make it fail on purpose before believing its
  success. Detail and examples in the `concurrency-testing` skill.
- **★ A negative control must be carried by a sample that exercises it alone.** Otherwise a hole
  hides behind the other signals, and the control passes while giving the impression of having
  verified. Eighth case of the pattern, detailed below.
- **★ To conclude that a capability is ABSENT, enumerate the surface — never query a list of
  guessed names.** `grep -E '^    pub fn'` over an `impl`, not
  `grep 'pub fn the_name_I_imagine'`. Tenth case of the pattern.
- **★ Reading the source is a source, not the source.** Before asserting anything about a
  third-party API — and above all before writing upstream — check the conclusion against the
  project's public documentation: its README, its site, its `docs.rs`. One minute, and it
  catches what no test will ever see.
- **★ A wildcard is the opposite of a checklist.** An exhaustive `match` **with no** `_` arm is a
  compile-time checkpoint: adding a variant breaks the build until someone classifies it. Adding
  `_ => panic!("an unknown variant!")` destroys exactly that property — and `#[non_exhaustive]`
  rescues nothing, it only constrains **other** crates, so the arm is dead inside the crate that
  defines the type.

  > Ninth case of the pattern, and the first found by **clippy** rather than by a run. A lint is
  > a measuring device too: `-D warnings` is not a style affectation.
- **A display that does not separate two events in time undermines the thesis.** This is not
  cosmetic and it is a design criterion: **Trame's thesis is an order** — "this file changed
  *since* you read it". A feed where three lines carry the same timestamp cannot show the order
  it is meant to demonstrate. Hence `trame_view::TIME_FORMAT` in milliseconds, pinned by a test.

  The useful generalisation: **before choosing a display precision, ask which property of the
  product that display is the evidence for.**
- **A canary** on every third-party behaviour an invariant depends on — and a test that verifies
  the canary knows how to fail.
- **Say it when you have not seen it.** A component that has only been tested is reported as
  such. The sentence to avoid is "it should work".
- **A per-file property is not tested with a global counter.** The fifth case in the table went
  unnoticed for weeks because the assertion used a shared proxy: any unrelated event moved it,
  and it did not move on my machine. Choose the narrowest observable that expresses the property.
- **Another machine is a measuring device.** The CI's macOS job found in one run what no local
  pass had seen, because it changed the scheduling. A green test on one machine is a green test
  on one machine.
- **A harness measures the production component, never a twin.** Seventh case in the table:
  ADR 0018's round measured `ConfigurableNotice` while believing it measured `StaleReadNotice`.
  When a type exists for the experiment, it is built as a **comparison against production**, and
  a test observes that they differ.
- **A ceiling is not a result, it is an admission.** `15/15` does not say "the message is
  optimal", it says "this device can no longer distinguish anything". Until a round has put
  something in the wrong, it has not yet shown that it is able to.
- **A dead command in a document is an executable lie.** A renamed flag, a vanished `justfile`
  recipe, a moved test path: the prose around it may stay true, but the command fails at the
  reader's end. A rename is not finished until the ADRs, the README, the skills and CI quote
  commands that run.
- **A convention is kept by a tool, not by vigilance.** A convention nothing checks lasts one
  session. `just check-language` fails if French comes back into the code, the docs or the
  markdown — and its **negative control runs on every invocation**, before it reports anything
  about the repository. That control found a real hole on the first attempt: the search was
  case-sensitive, so a sentence-initial capital walked straight past the guard.

  The word list is **deliberately short** and measured at zero false positives: `on`, `plus`,
  `son`, `sans`, `par`, `sur` are real English words, and `ce` would match `gpui-ce`. That is
  invariant 8 applied to our own tooling — **a guard that cries wolf is switched off within a
  week**, and the price of a missed word is a follow-up commit while the price of a false
  positive is the guard itself.

This is not an argument against tests, of which there are 187 here and which are
non-negotiable. It is an argument about **what a test is evidence of**: internal consistency,
never the behaviour on the other side of the boundary.

### ★★★ Eighth case: the negative control that could not fail

The first seven cases are about the product, or about the verification loop. This one is about
**the negative control itself**, and it is therefore the pattern one layer deeper than any of
them.

The `check-language` guard had just been written. To check that it knew how to fail, I removed
from its word list the one word that the sample `Le domaine s'ecrit en francais.` was there to
exercise, and reran its self-test. **It stayed green.** I nearly read that as "the guard is
solid".

What it actually meant: that same sample, `Le domaine s'ecrit en francais.`, also contained a
second listed word. **The detector caught it by another signal**, so my control was not testing
what I claimed it tested.

> **A negative control must be carried by a sample that exercises it alone.** Otherwise the hole
> hides behind the other signals, and the control passes while giving the impression of having
> verified.

A control redone properly — four ways of breaking the file, each of which had to turn it red —
immediately found **two real holes** the first one had missed:

| what was broken | why nothing saw it |
|---|---|
| the search was case-sensitive | `Le domaine s'ecrit en francais.` walked past a guard whose literal job that was |
| **no sample exercised accent detection** | every French line in the list also contained a listed word, so the accent branch could have been dead code with a green self-test |

Hence the shape settled on: two samples marked `ONLY` in
[`scripts/no_french.py`](scripts/no_french.py), one accent-only and one word-only. Each carries
**exactly one** signal.

**What this case adds to the other seven.** They all said "do not believe a green test without
having seen the device fail". This one adds the next notch: **having seen a device fail is not
enough if you do not know what its failure is evidence of.** A negative control is itself a
measuring device, so it falls under its own rule — and the recursion stops there, because a
single-signal sample leaves no room for a shortcut.

### ★★★ Tenth case: concluding on the absence of what was never looked for

The first nine cases share one mechanism: **a plausible output triggers no verification.** This
one belongs to a different family, and that is why it counts.

While probing `gpui-component`, I concluded that `InputState::multi_line(true)` was
**unfixable from outside the crate** — and I drafted an upstream issue whose central point was
"there is no workaround".

That was false. `InputState::rows(n)` is a **public** builder, at line 495 of the very file I
had just read. The project's official documentation shows exactly `multi_line(true).rows(10)`.

**What produced the error** was not a complacent test nor a badly modelled boundary. It was the
shape of my search:

```sh
grep -nE 'pub fn new|pub fn multi_line|pub fn placeholder|pub fn value|pub fn text' state.rs
```

A list of **guessed** names. `rows` was not in it, so `rows` did not exist. The same failing had
already struck two lines earlier: I had quoted `soft_wrap` after reading it on a `pub(super)`
field, without seeing that `soft_wrap(bool)` was a public builder.

> **A search by guessed names cannot find what was not guessed.** To conclude the **absence** of
> a capability, you must have **enumerated the surface**, not queried a list.
> `grep -E '^    pub fn'` would have given the lot in one go.

**And the second failing, which is the real one.** The project had online documentation, a
`docs.rs`, a README distinguishing two library tiers, and a separate assets crate. **I consulted
none of those sources.** I read the vendored code and stopped there, presenting the result as
established.

> **Reading the source is a source, not the source.** Checking a conclusion about a third-party
> API against its public documentation costs a minute and catches what no test will see —
> because a test verifies what you wrote, never what you failed to look for.

**What this case adds to the other nine.** They all said "do not believe a plausible output".
This one says: **a negative conclusion demands a different method from a positive one.**
Asserting "it works" is verified by running it. Asserting "it does not exist" is not verified by
looking again in the same place — you have to have exhausted the places.

The cost avoided is concrete: the issue would have been published under a real identity, on a
repository with 12,700 stars, and would have been answered with "use `.rows(10)`" within two
minutes.

### ★★ The order of a conversion: prescriptions, code, prose

> **When a convention changes, fix the documents that prescribe it first, then the code, then
> the descriptive prose last.**

This is not an organisational preference, it is what separates fixing something once from fixing
it forever:

> **A stale rule reproduces itself every session; stale prose sleeps.**

A forgotten French comment in a file stays a French comment. A skill that says "write your
comments in French" **regenerates French** in every session that reads it, including in the
files just translated.

**Why this rule is not obvious, and why everyone falls into it**: it is the exact inverse of the
order by volume. Prescriptions run to a few dozen lines, prose to thousands — so you
spontaneously start with the big piece, the one that looks like the work. The useful order puts
first what looks smallest.

**The example to cite, because it happened here**: in the middle of translating the error
messages, the `rust-conventions` skill — the one that **governs** those messages — still
prescribed "messages in lowercase, no trailing full stop, in French". I was translating strings
into English while the document with authority over them said the opposite. Two others did the
same: `adr-format` and `doc-keeper`. The initial find was the same one notch earlier:
`test-writer.md` prescribed French test names, with French examples, right after the 179 names
had been moved to English.

The operational corollary: **before starting a conversion, look for who prescribes the rule you
are changing.** A `grep` over the skills, the subagents and `AGENTS.md` costs a minute.

## Version control rule — GitButler workspace mode

This repository is in **GitButler workspace mode**.

> **Never use `git commit`, `git add`, `git push`.**

The correct flow:

```sh
but status --format json    # ALWAYS first: fetches the current cliIds
but commit <branch-id> -m "message" --changes <file-ids>
```

Mind the flag's form: it is **`--format json`**, not `--json` (which does not exist and fails).
A file's `cliId` is the value to pass to `--changes`, as a comma-separated list.

**`BUT_PAGER=cat` on long output.** `but` opens `less` by default as soon as the output exceeds
a screen, which blocks indefinitely in a non-interactive shell — a `but pull` once span for five
minutes doing nothing before being killed. The safe form:

```sh
BUT_PAGER=cat but pull
BUT_PAGER=cat but status
```

Branch and file IDs are **volatile**: they change on every mutation of the tree. An ID read
three commands ago is a stale ID. `but status --format json` before every mutation, without
exception.

The block below is generated by `but agent setup` and is authoritative on command **syntax**.
The rule above is authoritative on what is **forbidden**. This is that block's only home in the
repository.

<!-- gitbutler-agent-setup:start -->
## Version control

- Use GitButler (`but`) for version-control inspection and write operations, including status, diffs, branching, committing, pushing, and history edits.
- Assume multiple agents may be working in this repository. Do not move, amend, squash, discard, commit, push, or otherwise modify another agent's work unless the user asks.
- For commit just/only/specific changes on a new branch (selected-change requests), use the two-command fast path from the GitButler skill: `but diff`, then `but commit <branch> -c -m "message" --changes <id>,<id>`.
- For that fast path, after the commit succeeds, stop and summarize; do not run separate branch, staging, status, or diff commands unless the commit output is missing information you need.
- Use the installed GitButler skill for command recipes and syntax before guessing flags, using `--help`, or translating Git habits directly.
- After a successful GitButler write command, use the workspace state it returns. Rerun status or diff only when that output lacks information you need or files changed since.
- Use a dedicated GitButler branch for each agent session, unless the user asks for a different branch structure. Commit only changes that belong to that session.
- Do not push or open pull requests unless the user asks.
- Keep commit messages and pull request descriptions succinct: explain what changed, why it changed, and any important decision.

### Amend local fixes into the right commits

- For small cleanup or follow-up fixes, amend an unpublished local commit when the change clearly belongs with that commit's intent.
- Do not create tiny fixup commits unless the user asks.
- Use GitButler to move the relevant changes into the commit where they belong.
- Ask before rewriting pushed, reviewed, shared, or ambiguous history.

### Split unrelated changes into separate commits

- If one file contains unrelated changes, split them by hunk instead of committing the whole file.
- Keep tests with the behavior they verify.
- Split generated output, docs-only edits, or mechanical cleanup into separate commits when each commit remains coherent on its own.
- If the split is ambiguous, summarize the options before committing.
<!-- gitbutler-agent-setup:end -->
