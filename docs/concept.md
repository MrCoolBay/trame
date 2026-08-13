# Trame — coding-agent orchestrator, macOS desktop, local-first

> **Revision 5.** This document describes **what exists**, not what was imagined at the start.
> It has already diverged twice — the roadmap put the read-set in v0.5 when it is the v0.1
> deliverable, and revision 4 quoted a measurement taken on a twin of the shipped notice — and
> this revision exists so that it does not happen a third time.
>
> It is the **source of truth for every future session**. Decisions and their reasons live in
> [`adr/`](adr/); this document says where we stand and why. When it diverges from the code, it
> is this document that gets corrected, immediately.
>
> Previous revisions: 2 (multi-project, macOS framing, licence) · 3 (open source switch) ·
> 4 (the real scope of the invariant, and its two holes).

**Codename**: `Trame` — the horizontal thread of weaving: several shuttles, one cloth.

---

## 1. The pitch in one sentence

A macOS desktop application written in Rust that runs several coding agents in parallel,
**per project, in one working directory per project**, attributing every change to a virtual
branch, and making coordination between agents **explicit and observable** instead of silent.

---

## 2. The problem

Multi-agent orchestration rests on two models, both wobbly for a solo developer or a small
team.

### The worktree model (Conductor, Xirp, Crystal, most tools)

One git worktree per session. Real physical isolation, but workspace duplication, N branches to
land separately, and a heaviness out of proportion at three sessions.

Above all: **isolation does not only remove collisions, it removes the possibility of
coordinating.** Two agents in two worktrees are blind to each other *by construction*, and no
layer added on top fixes that.

### The virtual branches model (GitButler)

One working directory, changes **tagged** rather than isolated. No divergence over time, so
**the conflict has nowhere to be born**. Conceptually superior.

**But** this model trades a **loud** failure mode (git stops, inserts markers) for a **silent**
one (last writer wins, nobody is told). An excellent deal for a lone human. A bad deal for N
autonomous agents.

### The thesis

> Keep virtual branches — the model is right — and add the layer that makes collisions loud.
> Then multiply parallelism by **projects** rather than by sessions.

The failure mode to catch produces **no write collision at all**:

```
1. Agent A reads auth.rs, remembers verify_token()'s signature
2. Agent B modifies auth.rs, renaming verify_token() → validate_token()
3. Agent A writes handlers.rs, calling verify_token()

→ Two different files. A per-file lock sees nothing.
→ The tree is broken.
```

---

## 3. Design principles (non-negotiable)

| # | Principle | Consequence |
|---|---|---|
| 1 | **macOS desktop only** | No cross-platform abstraction. FSEvents, Keychain, launchd, APFS. |
| 2 | **Local-first** | One binary. No server, no account, no cloud. |
| 3 | **One working directory per project** | No worktree, no copy-on-write. |
| 4 | **Multi-project from the architecture up** | Parallelism is obtained by adding projects. |
| 5 | **2–5 sessions per project** | Anything that only helps beyond that is out of scope. |
| 6 | **ACP first, PTY as fallback** | Writes are intercepted *before* the disk where possible. |
| 7 | **Total observability** | Every write journalled with its provenance **and its origin**. |
| 8 | **Silent when clean** | ~95% of traffic without friction, or the feature is switched off within a week. |
| 9 | **Not an IDE** | No embedded editor. |
| 10 | **A named hole beats an ignored one** | Added in revision 4. See §6.7. |

---

## 4. Multi-project: the central insight

**Two sessions in two different projects physically cannot collide.** Separate directories,
repositories and indexes. The isolation is free and perfect.

```
5 projects × 3 sessions = 15 active agents
… without ever leaving the safe operating point (3 per working dir)
```

### The hierarchy

```
Workspace (the application)
 └── Project (a folder + a git repository)
      ├── One working directory
      ├── A dedicated Write Registry
      ├── A dedicated FSEvents watcher
      ├── Virtual branches
      └── Session (one agent + one goal)
```

### What is per project vs global

| Per project | Global (workspace) |
|---|---|
| Write Registry (one actor) | One SQLite journal (`project_id` column) |
| **Sequence counter** | Resource claims (**ports, dev databases**) |
| Working directory + VCS backend | Concurrency budget (CPU, RAM) |
| FSEvents watcher | API quotas and rate limits |
| Virtual branches, agent config | Credentials in the Keychain |

The subtle point: **resource claims must be global.** Port 3000 is machine-wide. It is the
first genuine cross-project conflict.

This is **the only irreversible architectural choice**: the Supervisor and the per-project
registry have existed since the first commit
([ADR 0010](adr/0010-parallelisme-par-projets.md)).

---

## 5. Architecture

```
┌────────────────────────────────────────────────────────────────┐
│  UI          v0: TUI (ratatui)      v1: GUI (gpui)             │
└──────────────────────────────┬─────────────────────────────────┘
                               │ Receiver<Observation> — one-way
┌──────────────────────────────▼─────────────────────────────────┐
│  Core — Rust / tokio daemon                                    │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │  SUPERVISOR (root actor)                                 │  │
│  │  ├─ Resource Claims  ├─ Concurrency Budget  ├─ Journal   │  │
│  └───────────┬──────────────────────────┬───────────────────┘  │
│  ┌───────────▼────────────┐  ┌──────────▼─────────────┐        │
│  │ PROJECT "portailfcd"   │  │ PROJECT "lyra-rp"      │  ...   │
│  │  ├─ SessionPilot ×N    │  │  ├─ SessionPilot ×N    │        │
│  │  ├─ Agent Transport ×N │  │  ├─ Agent Transport ×N │        │
│  │  ├─ WRITE REGISTRY     │  │  ├─ WRITE REGISTRY     │        │
│  │  ├─ FSEvents Watcher   │  │  ├─ FSEvents Watcher   │        │
│  │  └─ VCS Layer          │  │  └─ VCS Layer          │        │
│  └────────────────────────┘  └────────────────────────┘        │
└────────────────────────────────────────────────────────────────┘
```

The **core** is the product. The UI is interchangeable and comes second — and that is not a
figure of speech: it is what makes betting on a pre-1.0 framework acceptable
([ADR 0022](adr/0022-decoupage-daemon-gui.md) and
[0023](adr/0023-gpui-amont-pour-la-gui.md)). An interface receives **only a
`Receiver<Observation>`**, never a `RegistryHandle`.

That claim used to rest on the shape of `App`'s fields, and describing it as "in the typing"
was **false**: `trame-view` depended on `trame-registry` and called `admit` six times. It is now
enforced by the crate graph — a crate that does not depend on `trame-registry` cannot name
`admit`, whatever it wants — and `scripts/interface_boundary.py` fails if that edge comes back.
Dev-dependencies are allowed on purpose: the measurement harness drives the registry, which is
what an experiment is for.

The local IPC (UDS, JSON-RPC) of the original sketch does not exist and is not needed: both live
in the same binary. The channel prefigures the boundary without paying for it — it can be
replaced by a socket without changing the shape of the interface code.

The reverse direction exists too, and it is typed the same way: `trame_daemon::command` carries
a `Commander` and a `Command` enum, bounded at 32. **Observations drop under saturation,
commands wait** — losing a display line is a cosmetic problem, losing a user's instruction is
not. No `Command` variant carries a file to write: the interface asks for a session, a prompt or
a stop, never for an admission.

### The crates, and what they actually contain

```
crates/
├── trame-core/      ids · hash · clock · paths(ProjectRoot) · verdict
│                    project · session · prompt · notice · task_source · forge
├── trame-journal/   schema · records · store · actor
├── trame-registry/  state (★ the admission logic) · actor · msg
├── trame-agent/     backend · event · jsonrpc · acp · pty
├── trame-vcs/       (still empty: constants only)
├── trame-daemon/    session (SessionPilot) · watcher (FSEvents) · observe (UI channel)
│                    command (UI → daemon) · project (opening + scenario)
└── trame-view/      state (pure display state)
apps/
├── trame-tui/       run (loop) · ui (ratatui rendering)
└── trame-gui/       view (gpui rendering) · theme (colours and markers)
```

One dependency direction, never violated:
`core ← journal ← registry ← {agent, vcs} ← daemon ← view ← {tui, gui}`.

---

## 6. The modules

### 6.1 Supervisor

**Not written yet.** The boundaries exist; the project table and the claims do not. The framing
is in [ADR 0010](adr/0010-parallelisme-par-projets.md).

### 6.2 Session Manager

`SessionPilot` (`trame-daemon`) drives one session: it consumes the agent's stream, talks to the
registry, and places the notice in front of the next message. Session persistence and recovery
after a restart are not done.

**Special sessions**: `SessionId::EXTERNAL` exists and serves out-of-band writes (§6.5). A
`human` session will follow the same model.

### 6.3 Agent Transport

```rust
#[async_trait]
pub trait AgentBackend: Send {
    fn capabilities(&self) -> Capabilities;
    async fn send(&mut self, msg: UserMessage) -> Result<(), AgentError>;
    fn events(&mut self) -> Option<AgentEventStream>;
    async fn shutdown(&mut self) -> Result<(), AgentError>;
}
```

`AcpBackend` works, against one target: **Claude Code**. `PtyBackend` is a `todo!()` skeleton
whose only real method is `capabilities()` — and that is the most important one, since it is
what announces its degradation.

#### The inversion that makes the product possible

**In ACP, Trame is the client and the agent is the server.** The agent does not write and then
tell us: the agent *asks* Trame to write, through `fs/write_text_file`. The interception point
is not a hook to install, it is the protocol's normal path.

Better: **announcing `fs.writeTextFile` makes the agent's native `Write` and `Edit` tools
disappear.** It *can no longer* write by itself.
[ADR 0016](adr/0016-interception-avant-disque-validee.md) — validated live: two real sessions
asked to write, we refused, nothing reached the disk.

#### Three things learned that the documentation does not say

1. **There is no end-of-turn `sessionUpdate`.** End of turn is the **response to
   `session/prompt`**, carrying its `stopReason`. Waiting for an "end_of_turn" notification is a
   wait that never completes — it cost an experimental round.
2. **`tool_call_update` sometimes arrives with no preceding `tool_call`.** Translating only the
   initial shape leaves tool calls invisible.
3. **Paths arrive absolute and resolved.** Root `/var/…` → the agent answers `/private/var/…`.
   Hence `trame_core::ProjectRoot`, through which every file key passes.

#### The adapter is pinned, and that is a known problem

`@zed-industries/claude-code-acp` **0.16.2**, deprecated. The successor
`@agentclientprotocol/claude-agent-acp` **no longer removes `Write` or `Edit`**: measured, not
assumed.

```
0.16.2: --disallowedTools AskUserQuestion,Read,Write,Edit  → interception possible
0.66.0: --disallowedTools AskUserQuestion --tools default  → interception lost
```

Migrating would remove the central mechanism **silently**. A canary watches this unspecified
third-party behaviour on every `just ci`.
[ADR 0017](adr/0017-adaptateur-acp-epingle.md) looks the cost in the face and lists four ways
out, including the `PreToolUse` hooks — the least explored one.

### 6.4 Write Registry — the technical core (one per project)

**This is not a lock system.** Pessimistic locking does not fit: agents hold their transaction
for minutes, do not declare their intent in advance, and blocking a tool call in flight triggers
timeouts on the harness side.

The model is the databases' one: **optimistic concurrency with read-set validation**
([ADR 0007](adr/0007-concurrence-optimiste-read-set.md)).

#### The registry writes, it does not merely return a verdict

`admit` **evaluates, writes, then records** — in that order, inside the same actor
([ADR 0014](adr/0014-le-registre-ecrit-sur-disque.md)). An invariant that rests on the caller's
discipline is not an invariant.

State is updated **only after the disk succeeds**: otherwise the registry would believe the file
changed and would wrongly stale the other sessions' reads.

#### Four verdicts, not a boolean

| Level | Situation | Response | Status |
|---|---|---|---|
| **0 — Clean** | No overlap | Admitted, silent. ~95% of traffic. | ✅ |
| **1 — StaleRead** | Intersection with the read-set | **Admitted, and the agent is told.** | ✅ |
| **2 — DisjointWrite** | Same file, disjoint regions | Admitted. | ⏳ v0.4 |
| **3 — Overlap** | Overlapping regions | Blocked → ask the human. | ⏳ v0.4 |

Levels 2 and 3 are **never produced**: at whole-file granularity
([ADR 0012](adr/0012-granularite-fichier-en-v0-1.md)) they are indistinguishable. The variants
exist so that adding them is a `match` to complete.

**Nothing is blocked in v0.1.** The registry observes, journals and informs.

#### The notice, and the measurement that settled its form — corrected

`StaleFile` carries the path, the author, the timestamps and the sequence — **and no summary of
the change**. The registry computes **no diff** at admission. That part holds.

**Revision 4 quoted the wrong numbers, and this is the correction.** The `5/5` and `15/15`
figures were taken on `ConfigurableNotice::Neutral`, a twin of the shipped `StaleReadNotice` bar
one line. The harness never consumed the production contributor. Measured directly, on the same
day and under the same conditions:

| variant | score |
|---|---|
| neutral | 3/3 |
| directive | 3/3 |
| **production, as shipped** | **3/6** |

The string Trame was actually sending was **the only one of the three that failed**. The third
line — `Re-read it before continuing if your work depends on it.` — was removed, and the replay
gives **6/6**. The notice is two lines:

```
[Trame] auth.rs was changed by session "refactor-api"
        after you read it (a few seconds ago).
```

The mechanism, which generalises beyond this case: **an agent that receives a fact acts on it;
an agent that receives a fact plus permission to ignore it ignores it half the time.**

Two things not to conflate:

- **The question of the summary stays closed.** None of the three forms carries a diff, and the
  neutral says *less* than production while succeeding more — so the failure is not explained by
  a lack of context. The hypothesis "the agent will only follow the notice if it knows *what*
  changed" is still **refuted** ([ADR 0018](adr/0018-pas-de-diff-dans-stalefile.md)).
- **The wording of the third line is open**, with two candidate causes still confounded: the
  pronoun `it`, and the conditional `if your work depends on it`. Discriminating them means
  varying one point at a time.

And the reserve, written plainly: **six runs give no statistical power.** What is solid is the
direction, not the magnitude.

**The measurement preceded the spending**, and that is the method point to keep. The validation
debt is also explicit: a three-turn scenario, little accumulated context, a renamed identifier —
the most legible change there is. `Grep`/`Glob`/`Bash` were closed; that limit has been lifted
and the read-set filled anyway. A `15/15` was never evidence that the message was optimal; it
was evidence that **the device could no longer distinguish anything**.

#### State maintained

```rust
struct FileState {
    last_writer: SessionId,      // or SessionId::EXTERNAL
    last_seq: Seq,
    content_hash: ContentHash,   // blake3
    written_at: Timestamp,
    // modified_regions: Vec<Range> → v0.4
}

struct SessionState {
    name: String,
    read_set: HashMap<PathBuf, (ContentHash, Timestamp)>,  // TTL 10 min
    write_set: Vec<PathBuf>,
}
```

Read-set filtering: **substantial** reads only (`ReadKind::FullFile`). Grep hits and listings do
not enter — otherwise the read-set explodes and everything becomes level 1. Those hits go into a
parallel **shadow** read-set instead, which participates in no verdict and exists to measure the
false-positive rate before the hole is closed
([ADR 0027](adr/0027-trou-lecture-ouvert-et-mesure-en-ombre.md)).

### 6.5 The FSEvents watcher — promoted from comfort to requirement

**It was not planned this way.** Revision 2 listed it as a comfort net to "accept and display"
out-of-band writes. That is wrong: it is a **correctness requirement**.

A session *can* write outside admission — `sed -i` inside a `Bash`, a git hook, a build, the user
in their editor. Without the watcher:

```
A reads auth.rs                → read-set: hash v1
B runs `sed -i` on auth.rs     → the disk has v2, the registry still believes v1
A writes handlers.rs           → Clean, when it should be StaleRead
```

The problem is not journal coverage: **the registry becomes wrong**, and the central mechanism
fails **silently**. The tool looks like it is working and does nothing.

`RegistryMsg::ObserveExternalWrite` repairs that. Three properties:

- **It prevents nothing.** By the time FSEvents notifies, the file is written. There is no
  verdict to return. The watcher catches the state up so that the *next* admissions are correct.
- **No double counting.** The registry writes itself, so FSEvents also sees its own writes. The
  rule: *an observation whose fingerprint is already the known one is an echo, not an event.* No
  timestamps, no tolerance window, no race. It handles the formatter that rewrites identical
  content for free.
- **The noise stays out.** Filtered on the project's `.gitignore` rules plus a hard-coded
  exclusion list. A `cargo build` does not drown the registry.

These writes are attributed to `SessionId::EXTERNAL`, named "out-of-band" in the display, and
journalled with `origin = observed` **with no verdict** — nobody admitted them.

A detail that only real rendering found: the watcher used to emit its observations **without
knowing** whether the registry had recorded them, so the interface showed the registry's own
writes as out-of-band — the exact opposite of the truth.
`RegistryHandle::observe_external_write` now returns an `ExternalWrite::{Recorded, Echo}`.

### 6.6 Journal (global)

One SQLite database (`rusqlite`), **append-only**, in `~/Library/Application Support/Trame/`. A
global database, never inside the repository: it does not pollute projects, it survives their
deletion, and it makes the cross-project timeline possible
([ADR 0008](adr/0008-journal-sqlite-append-only.md)).

**The real schema**, current:

```sql
projects(id, path, name, toolchain, added_at, last_opened_at)
sessions(id, project_id, name, harness, target_branch, work_item, initial_state, created_at)
prompts(id, session_id, content, ts)
reads(id, project_id, session_id, path, hash, ts)
writes(id, project_id, session_id, session_name, seq, path,
       hash_before, hash_after, verdict, origin, ts)
resource_claims(id, resource, project_id, session_id, claimed_at)

UNIQUE(project_id, seq)   -- the sequence is per project
```

Four choices that are not in the original version:

- **`initial_state`** rather than `state`. In an append-only table, a `state` column would be
  read as a current state and would lie from the first transition onwards. Transitions will need
  an events table.
- **`session_name` denormalised** into `writes`. An audit row must read on its own, without a
  join, and survive the session's disappearance.
- **`origin`** — `admitted` or `observed`. Conflating the two would make the journal wrong on
  the only point that matters: provenance.
- **`verdict` nullable** — `NULL` for an observed write. Nobody admitted it, so no verdict
  exists; putting a value there would be a lie.

**This module has value on its own.** Even with no conflict detection, answering "who wrote this
line, in which project, in which session, in response to which prompt" is immediately useful.

### 6.7 ★ The real scope of the invariant, and the two holes

This is the most important section of revision 4, because it was the one that was missing.

> **The registry is the single point of passage for agent writes made by the file tools —
> `Write`, `Edit`, `NotebookEdit`.**
>
> **The read-set contains only reads made through the ACP read tool.**

No more, no less. That is the sentence that must be shown to the user.

#### Hole 1 — writing through the shell

`Bash`, `BashOutput` and `KillShell` **remain available**: they are only removed if the client
announces the `terminal` capability, which Trame does not. An `echo > file` therefore escapes
admission. **Measured** on the real command line, and **confirmed** by probe.

Mitigated, not closed: the FSEvents watcher catches the state up (§6.5). The journal carries the
row with `origin = observed`, with no verdict.

#### Hole 2 — reading through another tool, and this is the worse one

Removing `Read` **does not force** the agent through us: `Grep`, `Glob` and `Bash` remain
available. An agent reading through any of them **does not enter the read-set**.

**A read that escapes is worse than a write that escapes.** A missing write leaves a gap in the
journal; a missing read removes the **precondition** for a `StaleRead` — the central mechanism
does not fire, and nothing indicates it.

No mitigation is **implemented** today. The experimental round had to **close** `Grep`, `Glob`
and `Bash` to measure anything at all: acceptable for an experiment, **not for a product**. An
agent deprived of search on a real codebase is a degraded agent.

**The way out is measured, it is not yet built** ([probe 3](sondes/2026-08-12-postooluse.md)).
`PostToolUse` carries `tool_response.filenames` — the list of files actually read — in
`files_with_matches` mode and for `Glob`, and it fires **neither** on a refused call **nor** on
a failed one: a read-set fed from there cannot contain a phantom read. **Closing `Grep` and
`Glob` is therefore unnecessary.**

Two decisions frame that path before it is built. The fingerprint comes **only** from
`fs/read_text_file`, never from a hook payload — the CLI injects a `<system-reminder>` into it
([ADR 0020](adr/0020-empreinte-uniquement-depuis-fs-read-text-file.md), invariant 10). And
`Grep`'s `content` mode, where paths exist only inside an output string, becomes a **third named
and counted blind spot** rather than a case to reconstruct
([ADR 0021](adr/0021-pas-d-analyse-de-la-sortie-de-grep.md)) — with a real mitigation: an agent
that wants the context around the lines it found **opens the file**, and therefore comes back
through `fs/read_text_file`.

Meanwhile the hole is **instrumented rather than closed**: shadow mode counts what we *would*
have said and says nothing, so the threshold can be chosen on a measured size distribution
instead of a guess ([ADR 0027](adr/0027-trou-lecture-ouvert-et-mesure-en-ombre.md)).

Together with refusing `Bash` writes in `PreToolUse`, the same path also covers hole 1 and the
dependency on the deprecated adapter — three problems at once.

#### What no registry can catch

Semantic interference **with no read overlap**: A and B contradict each other without having read
the same file. The only real net: the compiler and the tests. A possible path: a quiescence
detector.

### 6.8 VCS Layer

**Still empty.** Two constants. The framing holds:

- One working directory, never a worktree.
- **Attribution is deterministic**: every admitted write carries its `session_id`, and therefore
  its branch. It is no longer a heuristic, it is data.
- `ButBackend` as a shell-out, `but ... --format json` always
  ([ADR 0003](adr/0003-gitbutler-en-shell-out.md), [ADR 0004](adr/0004-parsing-json-du-vcs.md)).
  Careful: `--format json`, **not** `--json`, which does not exist.

---

## 7. macOS framing

Unchanged since revision 2. What we gain: FSEvents (**now genuinely used**, §6.5), Keychain,
launchd, native notifications, a menu bar item, APFS, a single CI target. What it costs: Apple
Developer Program (~€99/year), no Mac App Store, an updater to wire, TCC to handle carefully.
[ADR 0001](adr/0001-macos-uniquement.md).

---

## 8. Licence: open source, MIT OR Apache-2.0

**MIT OR Apache-2.0**, at the user's choice — the Rust ecosystem's convention. Trame is open
source in the OSI sense, with no vocabulary hedging. No CLA: a contribution is offered under the
same terms.

Revision 2 chose FSL-1.1-MIT and forbade the term "open source". That choice is abandoned: the
protection was theoretical (a local desktop app has no service to compete with), the cost real
(a non-OSI licence, complicated packaging, a CLA on every contribution), and protection comes
not from the licence but from execution and the brand.
[ADR 0013](adr/0013-licence-open-source-mit-apache.md), which supersedes ADR 0009.

**What this does not settle**: Trame's licence grants no rights over GitButler's code. `but`
remains an **external prerequisite installed by the user, never vendored** — it is that
non-inclusion that carries the analysis.

---

## 9. Stack

| Area | Choice | State |
|---|---|---|
| Runtime | `tokio`, one actor per domain | ✅ registry, journal |
| Hash | `blake3`, at admission and at read only | ✅ |
| Storage | `rusqlite`, append-only | ✅ six tables |
| Agent transport | JSON-RPC over stdio, ACP | ✅ `AcpBackend` |
| Watcher | `notify` (FSEvents) | ✅ with an `ignore` (gitignore) filter |
| PTY | `portable-pty` | ⏳ `todo!()` skeleton |
| Git | `but` CLI as a shell-out | ⏳ constants only |
| Keychain | `security-framework` | ⏳ not started |
| UI v0 | `ratatui` | ✅ panels, feed, verdicts, degradation |
| UI v1 | `gpui` **Zed upstream, pinned** 0.2.2 | ✅ `apps/trame-gui` ([ADR 0023](adr/0023-gpui-amont-pour-la-gui.md)) |
| Components | `gpui-component` **0.5.1**, crates.io | ✅ adopted for the multi-line field ([ADR 0028](adr/0028-adoption-de-gpui-component.md)) |
| UI escape hatch, tested | `gpui-ce` — drop-in fork, two lines of `Cargo.toml` | ⏳ if upstream breaks or stalls |
| UI last resort | Tauri v2 + **Vue** — not Nuxt: routing and SSR are useless on a single window | ⏳ if `gpui` disappoints |

No `unsafe`; `unsafe_code = "forbid"` at the workspace level.

---

## 10. Roadmap — corrected

> **The original roadmap put the read-set in v0.5.** That was wrong: it is the v0.1 deliverable,
> and it is the only thing that distinguishes Trame. This section is the correct version.

| Phase | Contents | State |
|---|---|---|
| **0** | Tooling, crate boundaries, seams, ADRs, skills | ✅ |
| **1** | `trame-journal` + `trame-registry`. Canonical scenario testable with no agent | ✅ |
| **2** | `trame-agent`, ACP, interception validated live | ✅ |
| **3.1** | The registry writes after admission | ✅ |
| **3.2** | Full chain: `FileRead` → read-set, `FileWrite` → admission → notice | ✅ |
| **3.3** | Experimental round on the notice's form | ✅ settled, and re-measured against production |
| **3.4** | FSEvents watcher — **moved ahead of the TUI** | ✅ |
| **3.5** | Minimal ratatui TUI | ✅ |
| **4.0** | `gpui` probe: window, tokio `Receiver`, scrolling list, escape hatch tested | ✅ [probe 4](sondes/2026-08-12-gpui-ce.md) |
| **4.1** | `apps/trame-gui` — the same display scope as the TUI | ✅ |
| **5.1** | Typed command channel, UI → daemon | ✅ |
| **5.2** | Per-session thread model, persisted to the journal | ⏳ |
| **5.3** | UI primitives inventory and their cost | ⏳ |
| **5.4** | Layout | ⏳ |
| **v0.2** | Attribution → assigning hunks to virtual branches | ⏳ |
| **v0.3** | Multi-project: Supervisor, toolchain, resource claims | ⏳ |
| **v0.4** | Hunks: `DisjointWrite` and `Overlap`, level 3 blocking | ⏳ |
| **v1** | Signing, notarisation, Homebrew cask, updater | ⏳ |

> **Do not jump to blocking.** Product risk number one is still the false-positive rate. Stay in
> detection-only mode on your own workflow, measure, *then* decide what deserves a block.

**187 tests** (191 with doctests), deterministic, with no `sleep` — except the FSEvents tests,
which wait on a condition by bounded polling because the system notifies when it notifies.

---

## 11. Non-goals

- ❌ Windows and Linux
- ❌ A code editor — this is not an IDE
- ❌ A proprietary model or agent
- ❌ SaaS, an account, a backend, multi-user
- ❌ 50 sessions in one project / copy-on-write / a distributed scheduler
- ❌ A replacement for git or for the forge
- ❌ **Any form of isolation**

---

## 12. Risks — updated by measurement

| Risk | Severity | State |
|---|---|---|
| **Read hole** (`Grep`/`Glob`/`Bash`) | 🟡 Medium | **Open, but the way out is measured** (probe 3) and the hole is **instrumented in shadow mode** (ADR 0027). `PostToolUse` reports the files read, with no phantom reads. Remaining: implement it, and settle `Grep`'s `content` mode. A reading `Bash` is still uncovered. |
| **Deprecated ACP adapter** | 🔴 High | **Pinned at 0.16.2**, canary in place. The successor breaks interception. A reprieve, not a solution. |
| **GitButler licence (FSL)** | 🔴 High | Open. `but` not vendored; the soundest path remains the FSL→MIT conversion at two years. |
| **Scope creep** | 🔴 High | Section 11 exists for this. |
| **Registry false positives** | 🟠 Medium | Not yet measured on real usage. Two dials before paying for hunks: the read-set filter, and the TTL. |
| **`Bash` write hole** | 🟠 Medium | **Mitigated** by the watcher: the registry no longer becomes wrong. Not admitted, not prevented. |
| **The round's validation debt** | 🟠 Medium | The `15/15` was a device that had stopped discriminating — and it was not even measuring the product. Now 6/6 on the shipped text, over six runs, which is **no statistical power**. Each remaining limit is a trigger. |
| **Holes in ACP** | 🟠 Medium | Three undocumented behaviours found. Canary + double-checking third parties. |
| **Multi-project retrofit** | 🟢 Settled | Per-project registry since the first commit. |
| **Semantic interference** | 🟡 Low | No registry can catch it. The net: compiler + tests. |
| **Cross-project resources** | 🟡 Low | Global claims in the Supervisor. Not written yet. |
| **macOS distribution** | 🟡 Low | To be budgeted, not discovered. |

---

## 13. Open questions

Settled questions are removed from here and live in their ADR. Remaining:

1. **The read hole.** **Probed three times** — the contract, a real session, then tool results:
   [`pretooluse`](sondes/2026-08-12-pretooluse.md),
   [`pretooluse-live`](sondes/2026-08-12-pretooluse-live.md),
   [`postooluse`](sondes/2026-08-12-postooluse.md).
   Established: the hook fires, a `deny` really blocks, the reason reaches the agent which falls
   back **onto the admitted path**, the settings file lives **outside the project** through
   `extraArgs.settings`, and `PostToolUse` reports the files read without ever firing on a
   refused or failed call. Total cost ~11.7 ms per tool call, two processes.
   The chosen direction, **not implemented**: **refuse** shell commands that write, which brings
   the hole back inside admission's scope instead of modelling it — and **record** what `Grep`
   and `Glob` read rather than refusing them.
   **Settled since**: the fingerprint comes only from `fs/read_text_file`
   ([ADR 0020](adr/0020-empreinte-uniquement-depuis-fs-read-text-file.md)); `Grep`'s `content`
   mode is an accepted blind spot, not reconstructed
   ([ADR 0021](adr/0021-pas-d-analyse-de-la-sortie-de-grep.md)); and the hole stays open and is
   measured in shadow mode rather than closed on a guessed threshold
   ([ADR 0027](adr/0027-trou-lecture-ouvert-et-mesure-en-ombre.md)).
   **What stays open**: `head_limit` and its possibly silent truncation; a **reading** `Bash`
   (`cat`, `head`), which nothing covers; and the cost of fingerprinting N files reported by a
   single `Grep`.
2. **The third line of the notice.** Removing it took the shipped text from 3/6 to 6/6, but two
   candidate causes are still confounded — the pronoun `it` and the conditional `if your work
   depends on it`. Discriminating them means varying one point at a time, and it needs a device
   that has already been seen to fail.
3. **A way out of the deprecated adapter**: contribute upstream, an adapter maintained by Trame,
   `PreToolUse` hooks, or accept the degradation?
   [ADR 0017](adr/0017-adaptateur-acp-epingle.md) lists all four without committing to any.
4. **`but` CLI or native `gix`** for v0.2?
5. **Can a project have several repositories** (monorepo vs linked multi-repo)?
6. **Positioning**: a personal tool, or a product with an auditability / sovereignty angle for
   the European market? The licence is settled, and so is the hosting
   ([ADR 0019](adr/0019-heberger-trame-sur-github.md): GitHub, to find contributors there); the
   positioning is not.

### Settled since revision 2

| Question | Answer | ADR |
|---|---|---|
| Which harness first? | Claude Code over ACP. Interception before the disk **works**. | [0016](adr/0016-interception-avant-disque-validee.md) |
| Does level 1 tell the agent automatically? | **Yes**, and the agent re-reads and adapts — 6/6 on the shipped text, after the third line was removed. | [0018](adr/0018-pas-de-diff-dans-stalefile.md) |
| Must the notice say what changed? | **No.** Measured, refuted, and the correction reinforced it. | [0018](adr/0018-pas-de-diff-dans-stalefile.md) |
| Fair Source or open source? | Open source, MIT OR Apache-2.0. | [0013](adr/0013-licence-open-source-mit-apache.md) |
| Does the registry return a verdict or write? | **It writes.** | [0014](adr/0014-le-registre-ecrit-sur-disque.md) |
| Does a GitHub macOS runner's graphical session let an AppKit app open a window? | **Yes.** It is an **Aqua** session: the window opens and the shader smoke test returns `SMOKE_OK`. The job is on the critical path. | — |
| Build the UI primitives, or take a dependency? | `gpui-component` 0.5.1, adopted with its costs written down. | [0028](adr/0028-adoption-de-gpui-component.md) |
