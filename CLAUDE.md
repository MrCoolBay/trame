# Trame — Claude Code instructions

The project's framing — thesis, decisions, invariants, structure, commands, version-control
rule — lives in **[`AGENTS.md`](AGENTS.md)**, imported here:

@AGENTS.md

**Read `AGENTS.md` in full before writing a line of code.** If the import above was not
resolved into your context, open the file by hand.

Why the split: Trame orchestrates Claude Code, Codex and Gemini CLI. The day a Trame session
launches Codex on the Trame repository itself, it will only see `AGENTS.md`. The framing
there is therefore neutral, and this file holds only what is specific to Claude Code.

> **Copy nothing from `AGENTS.md` into here.** A piece of information has one home; two
> copies diverge at two different speeds. If `but agent setup` re-injects its
> `<!-- gitbutler-agent-setup -->` block into this file, delete it: its home is `AGENTS.md`.

## Project skills

Prescriptive and short, each with one correct example and one counter-example. **Read before
acting in their area**, not after.

| Skill | When |
|---|---|
| `rust-conventions` | Before writing or changing Rust here |
| `actor-pattern` | Before creating an actor, or the moment you are tempted to write `Arc<Mutex<_>>` |
| `acp-integration` | Before touching `trame-agent` or wiring a harness |
| `journal-schema` | Before touching the SQLite schema or writing a query |
| `concurrency-testing` | Before writing a test that touches an actor or time |
| `adr-format` | Before creating an ADR, or when unsure whether a choice deserves documenting |
| `gitbutler` | Provided by `but agent setup`. Authoritative on `but` syntax. |

## Subagents

| Subagent | Scope |
|---|---|
| `architect` | Tests every decision against the concept and the invariants. Writes the ADRs. **Has authority to say "this violates an invariant"**, and that stops the work. |
| `rust-reviewer` | Idiomatic review: errors, lifetimes, allocations, panic paths |
| `test-writer` | The tests, especially deterministic concurrency tests |
| `acp-specialist` | The ACP protocol and harness integration |
| `doc-keeper` | Stops `AGENTS.md`, the ADRs and the skills from lying |

### Execution discipline

On this greenfield project, subagents run **in sequence, not in parallel**: the crates do not
have stable boundaries yet, and parallel agents would step on each other. The irony will not
be lost on anyone — that is exactly the problem Trame solves, and Trame does not exist yet.

Parallelism arrives when the crates are genuinely independent.

## Reminders that are expensive to forget

- **`but`, never `git`** for any mutation. Detail in `AGENTS.md`.
- **One phase at a time**, stopping at each checkpoint. Do not continue into the next phase
  without explicit human validation.
- If a decision in `AGENTS.md`'s table looks wrong to you: **say so and argue it**, but do not
  deviate on your own.
- **Build commit messages through a quoted heredoc into a variable**, never inline. A commit
  message is shell input before it is prose: backticks get command-substituted and
  apostrophes terminate a single-quoted string. Both happened here, and one of them executed
  a recipe mid-commit.
