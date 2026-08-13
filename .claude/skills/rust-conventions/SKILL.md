---
name: rust-conventions
description: Trame's Rust idioms. Read before writing or changing Rust here — the thiserror/anyhow split, tracing instrumentation, the unwrap ban, module layout, naming conventions.
---

# Rust conventions — Trame

Six rules. Each one is enforced by clippy at the workspace level, so a violation fails CI.

## 1. Errors: `thiserror` in libraries, `anyhow` in binaries

A library that returns `anyhow::Error` forces its caller to pattern-match on strings. The
only `anyhow` in the repository is in `crates/trame-daemon/src/main.rs` and
`apps/trame-tui/src/main.rs`.

✅ **Correct** — a library, a typed error, context carried by the variants:

```rust
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("session unknown to the registry: {0}")]
    UnknownSession(SessionId),
    #[error("cannot hash {path}")]
    Hash {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

pub fn admit(&mut self, session: SessionId) -> Result<Verdict, RegistryError> {
    let state = self
        .sessions
        .get_mut(&session)
        .ok_or(RegistryError::UnknownSession(session))?;
    // ...
}
```

❌ **Counter-example** — the caller cannot decide anything:

```rust
pub fn admit(&mut self, session: SessionId) -> anyhow::Result<Verdict> {
    let state = self
        .sessions
        .get_mut(&session)
        .context("unknown session")?;   // <- a String. Good luck reacting to it.
}
```

Secondary rules:
- `#[source]` on the originating error, always. A broken error chain loses the real cause.
- Messages in lowercase, no trailing full stop, **in English**. They are concatenated by
  the chain's display, and they reach both the agent and the user.
- A public error enum is `#[non_exhaustive]`: adding a variant must not be a breaking
  change.

## 2. No `unwrap()`, `expect()`, `panic!()` outside tests

`unwrap_used`, `expect_used` and `panic` are `deny` in `[workspace.lints.clippy]`.
`clippy.toml` carries `allow-unwrap-in-tests = true` and its siblings: inside a test,
`unwrap()` is the most readable way to fail, and it is allowed.

✅ **Correct**:

```rust
let Some(read) = self.read_set.get(path) else {
    return Ok(Verdict::Clean);   // no recorded read: nothing to report
};
```

❌ **Counter-example**:

```rust
let read = self.read_set.get(path).unwrap();   // panic = dead daemon = sessions lost
```

A `todo!()` is allowed and intended for an unimplemented seam: it is explicit and
localised. `todo = "allow"` at the workspace level.

## 3. All I/O is instrumented with `tracing`

Never `println!` or `eprintln!` — `print_stdout` and `print_stderr` are `deny`. Logs go to
**stderr**: stdout belongs to ratatui's alternate screen and to ACP's JSON-RPC.

✅ **Correct** — structured fields, no interpolation:

```rust
#[tracing::instrument(skip(content), fields(bytes = content.len()))]
async fn admit(&mut self, session: SessionId, path: &Path, content: &str) -> Verdict {
    let verdict = self.evaluate(session, path, content);
    tracing::info!(verdict = verdict.label(), seq = %self.seq, "write admitted");
    verdict
}
```

❌ **Counter-example**:

```rust
println!("admitted {} for {}", path.display(), session);   // wrecks the TUI's display
tracing::info!("verdict {:?} for {}", verdict, session);   // neither filterable nor queryable
```

`skip` on file contents, always: a 40 kB `content` in a log is unusable, and it may carry
secrets.

## 4. Documentation is mandatory on every public item

`missing_docs = "warn"` plus `-D warnings` in CI: an undocumented public item fails the
build. That includes the **fields** of public structs and the variants of enums.

Documentation says *why*, not *what*. `/// The session identifier.` on a field named
`session_id` serves nobody.

✅ **Correct**:

```rust
/// The sequence number of an admitted write.
///
/// **Per project, never global.** A global counter would be a point of contention
/// between projects which, by construction, cannot collide with each other.
pub struct Seq(u64);
```

## 5. Module layout

- One module per concept, declared in `lib.rs`, re-exported flat at the bottom of
  `lib.rs`. The caller writes `trame_core::Verdict`, not `trame_core::verdict::Verdict`.
- **No `mod.rs`.** A module with children is written `foo.rs` plus `foo/bar.rs`.
- `use` statements are grouped in three blocks separated by a blank line, in this order:
  `std`, external crates, `crate`/`super`. `rustfmt` does not do it for you; it is done by
  hand.
- Unit tests live in a `mod tests` at the bottom of the file they test. Integration tests
  go in `tests/`.

## 6. Naming

- **Everything in English**: identifiers, comments, module documentation, error messages,
  test names. The repository is public and addresses an international audience.
- A test name reads as a **specification sentence**, not as a label:
  `fn stale_read_with_no_write_collision_at_all()` states what is guaranteed.
- Domain terms keep their established form — read-set, hunk, worktree, backpressure, stale
  read — rather than an invented translation.
- Mandated vocabulary: `ChangeRequest`, **never** `PullRequest` (ADR 0011). `Verdict`, not
  `ConflictResult`. `Admit`, not `CheckWrite`.
- Newtypes on identifiers, systematically. A `SessionId` must never be interchangeable
  with a `ProjectId`, even when both carry a UUID.
- `Handle` as a suffix for an actor's cloneable handle: `RegistryHandle`.
- `label()` for the stable representation persisted in the database. Never change a
  `label()` value without a migration: the journal is append-only.

## Before calling it done

```sh
just lint    # fmt --check + clippy -D warnings. Exactly what CI checks.
just test
```
