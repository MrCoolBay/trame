# Trame

Several coding agents in parallel, in **one shared working directory** per project, with
coordination that is explicit and observable instead of silent.

macOS. Rust. Local-first: one binary, no account, no server, no cloud.

> **Status: under construction. Usable for observing, not yet for working.**
> What it does today and what it does not is spelled out below, unrounded.

## The thesis

Every competing tool isolates each agent — a git worktree or a directory copy per session.
Isolation removes collisions, but it also makes coordination **impossible**: each agent is
blind to the others by construction, and no layer added on top fixes that.

Trame takes the opposite bet: **shared directory, enforced coordination.**

> When agent A is about to write, if a file it **read** has since been modified by another
> session, A is reasoning about a world that no longer exists. Trame detects that and
> **tells it**.

Enforced, not suggested: this is not an instruction in a prompt that an agent can forget,
it is a **point of passage**. The agent's writes go through the admission registry, which
returns a verdict and performs the write itself.

The failure mode this catches produces **no write collision at all**:

```
1. Session A reads auth.rs, remembers verify_token()'s signature
2. Session B edits auth.rs, renaming verify_token() -> validate_token()
3. Session A writes handlers.rs, calling verify_token()

Two different files. A per-file lock sees nothing. The tree is broken.
```

That is the central mechanism, and the only one in the product that exists nowhere else —
too late at the forge, where the code is already written; impossible at the filesystem
level, where the agents are isolated.

**Trame does not block, it informs.** A notified agent re-reads and adapts. That behaviour
was measured on real Claude Code sessions, and the measurement is explicitly marked as
**not discriminating**: short scenario, little accumulated context, a very legible change.
To be replayed on a realistic case before concluding anything
([ADR 0018](docs/adr/0018-pas-de-diff-dans-stalefile.md)).

## The invariant's real scope, and its two holes

A net whose holes you do not know is worse than a net whose holes you do. Here they are, as
measured:

**What is covered.** Writes made by the agent's file tools — `Write`, `Edit`, `NotebookEdit`
— go through the registry **before the disk**. Validated on two real Claude Code sessions:
they asked to write, we refused, nothing reached the disk
([ADR 0016](docs/adr/0016-interception-avant-disque-validee.md)).

**Hole 1 — writing through the shell.** An `echo > file` or a `sed -i` inside a `Bash` call
escapes admission. The FSEvents watcher **notices it after the fact**: it prevents nothing,
but it stops the registry from becoming *wrong*. Those writes are journalled as `observed`,
**with no verdict**, and the interface shows them as such — never as admitted writes.

**Hole 2 — reading through another tool, and this one is worse.** Removing `Read` does not
force the agent through us: `Grep`, `Glob` and `Bash` remain available. A read through any
of them **does not enter the read-set** — and a missing read does not leave a gap in the
journal, it removes the **precondition** for a notice. The central mechanism simply does not
fire, and nothing says so.

The way out is measured but **not yet built**: the CLI's `PreToolUse` and `PostToolUse`
hooks expose what is needed ([probe 3](docs/sondes/2026-08-12-postooluse.md)). Meanwhile the
measurement rounds **close** those tools, which is acceptable for an experiment and **not
for a product**: an agent with no search on a real codebase is a degraded agent.

**What no registry can catch**: two agents contradicting each other without having read the
same file. The only real net there is the compiler and the tests.

## Status

Phases 0 to 4.1 delivered. **187 tests** (191 with doctests). What exists:

| Component | State |
|---|---|
| `trame-journal` — append-only SQLite journal, six tables | ✅ real writes, provenance and origin |
| `trame-registry` — the admission actor, **the core** | ✅ `Clean` / `StaleRead` verdicts, writes to disk |
| `trame-agent` — ACP over stdio, interception before disk | ✅ validated live; `PtyBackend` is an honest skeleton |
| `trame-daemon` — session pilot, FSEvents watcher, project opening | ✅ the whole chain, up to the notice placed before the prompt |
| `trame-hook` — the CLI hook bridge, unix socket per project | ✅ denies by default when the daemon is unreachable |
| `trame-view` — display state shared by both interfaces | ✅ observes only; it cannot name the registry |
| `trame-tui` — terminal interface | ✅ panels, feed, verdicts, degradation |
| `trame-gui` — `gpui` desktop app | ✅ same display scope |
| `trame-vcs` — GitButler shell-out | ⏳ boundary drawn, content to come |
| Multi-project supervisor | ⏳ framed, not written |

What is **not** done: attributing changes to virtual branches, real multi-project, closing
the two holes above, and everything to do with distribution — signing, notarisation,
updates.

The full framing is in [`docs/concept.md`](docs/concept.md), the decisions and their reasons
in [`docs/adr/`](docs/adr/), and what was measured rather than assumed in
[`docs/sondes/`](docs/sondes/).

## Non-goals

Explicitly refused, so nobody opens a change request that will be closed: Windows and Linux,
an embedded code editor, a proprietary model or agent, a SaaS mode, and **any form of
isolation** — worktrees, containers, copy-on-write, microVMs. The last one is the product's
precondition, not a preference.

## Try it

```sh
just tui-scenario /tmp/trame-demo    # the canonical scenario, in a terminal
just gui-scenario /tmp/trame-demo    # the same, as a desktop app
```

Both play the canonical scenario through the **real** registry, with no agent: the verdicts
shown are the ones it returns. While it runs, from another terminal:

```sh
echo '// added by hand' >> /tmp/trame-demo/notes.txt
```

The line appears as **out-of-band, with no verdict** — the watcher noticed it after the
fact, nobody admitted it.

## Developing

```sh
just check     # compile everything, tests included
just test      # the whole suite
just lint      # fmt + clippy + the guards, zero warnings tolerated
just ci        # what CI checks, locally
just canary    # ★ checks the ACP adapter still removes the write tools
just smoke     # ★ opens the GUI and requires an image to really be produced
```

Prerequisites: macOS on Apple Silicon, a stable Rust toolchain, the
[GitButler](https://gitbutler.com) CLI (`but`) on `PATH`, and Node for the ACP adapter
(`npm install -g @zed-industries/claude-code-acp@0.16.2` — a **pinned** version, see
[ADR 0017](docs/adr/0017-adaptateur-acp-epingle.md)).

A full Xcode install is **not** required: the GUI compiles its shaders at launch rather than
at build time ([ADR 0023](docs/adr/0023-gpui-amont-pour-la-gui.md)).

Trame calls `but` as an external dependency; it does not vendor it. This repository is in
**GitButler workspace mode**: no `git commit`, no `git add`, no `git push` — see
[`AGENTS.md`](AGENTS.md).

## Contributing

The project's rules, architectural invariants and settled decisions are in
[`AGENTS.md`](AGENTS.md). Reading them before opening a change request saves a round trip —
particularly the non-goals above and the ten invariants.

Two things are non-negotiable: `just lint` with zero warnings, and `just test` green.
Everything else is open to discussion.

One rule of method, born from the same bug ten times over: **any mechanism that crosses a
boundary — a third-party protocol, the filesystem, a terminal — must have been seen running
for real before it counts as settled.** A test establishes consistency with what you believe
about the boundary, never what the boundary does. Detail in `AGENTS.md`, where all ten cases
are listed with what caught each one.

## Licence

Trame is **open source**, dual-licensed at your option:

- [MIT](LICENSE-MIT)
- [Apache-2.0](LICENSE-APACHE)

Use, modification, forking, redistribution including commercial: all permitted. This is the
Rust ecosystem's convention — MIT for its brevity, Apache-2.0 for the explicit patent clause
MIT lacks.

Unless you state otherwise, any contribution you intentionally submit for inclusion in Trame
is offered under those same two licences, with no additional terms. **There is no CLA.**

The reasoning behind that choice — and why the project left FSL-1.1-MIT — is in
[ADR 0013](docs/adr/0013-licence-open-source-mit-apache.md).

> Note: the GitButler CLI (`but`) is separate software, under FSL-1.1-MIT. Trame calls it as
> an external prerequisite and **never** vendors it. If you redistribute Trame, do not
> package `but` with it.
