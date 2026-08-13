---
name: acp-integration
description: Agent Client Protocol in Trame — JSON-RPC over stdio, capability negotiation, session lifecycle, intercepting writes before the disk, known protocol holes and the PTY fallback strategy. Read before touching trame-agent or wiring up a harness.
---

# ACP integration — Trame

## Why ACP and not a PTY

One reason, and it carries the whole product:

> **In ACP, filesystem access goes through the protocol. So Trame sees it before the disk,
> and can submit it to the registry.**

The inversion is what matters: in ACP, **Trame is the client and the agent is the server**.
The agent does not write and then tell us — the agent *asks* Trame to write. The interception
point is therefore not a hook to install, it is the protocol's normal path.

Over a PTY, you see text. Writes are discovered after the fact through FSEvents, once the
tool call has finished and the agent has moved on. Journalling and attribution remain
possible; informing at the right moment becomes impossible.

## Lifecycle

```
1. spawn the harness as a subprocess, stdin/stdout as pipes
2. initialize            -> version and capability negotiation
3. session/new           -> an ACP session id, paired with our SessionId
4. session/prompt        -> we send the work
5. session/update (n)    -> incoming notifications: text, tool calls, results
   including fs/* requests -> ★ THE ADMISSION POINT
6. end of turn           -> the agent hands control back
```

Transport details — JSON-RPC 2.0, one envelope per line over stdio — are standard. **The
exact method names and payload shapes are to be checked against the specification**
(`agentclientprotocol.com`) at the time of writing code, not copied from this skill: the
protocol moves.

### What has been verified, and holds

State as of 2026-08-11, protocol 1, adapter `@zed-industries/claude-code-acp` 0.16.2. Detail
and method in [ADR 0016](../../../docs/adr/0016-interception-avant-disque-validee.md).

- The **client's** methods are `fs/read_text_file`, `fs/write_text_file` and
  `session/request_permission`. The agent's: `initialize`, `authenticate`, `session/new`,
  `session/load`, `session/prompt`, `session/cancel`, `session/set_mode`,
  `session/set_model`, plus the `terminal/*` family.
- **Announcing `fs.writeTextFile` makes the agent's native `Write` and `Edit` tools
  disappear.** It is not merely a notification: the agent *can no longer* write by itself.
- `session/new` accepts `_meta.claudeCode.options.disallowedTools`, **merged** rather than
  overwritten. That is how we close `NotebookEdit`, which the adapter leaves open.
- **Paths arrive absolute and resolved.** Passing `/var/folders/…/project` as `cwd` yields
  paths under `/private/var/folders/…`. Every file key must therefore go through
  `trame_core::ProjectRoot`: without it, a read and a write of the same file become two
  different keys and `StaleRead` stops firing **without anything breaking**.
- **Removing `Read` is not enough.** `Grep`, `Glob` and `Bash` remain available, and a read
  through any of them does not enter the read-set — so no `StaleRead` is possible. Closing
  those tools goes through `AcpBackend::disallow_tools`, before `new_session`.
- **There is no end-of-turn `sessionUpdate`.** End of turn is the **response to
  `session/prompt`**, carrying its `stopReason`. Waiting for an "end_of_turn" notification is
  a wait that never completes — it cost a whole experimental round.
- **`tool_call_update` sometimes arrives with no preceding `tool_call`.** When a permission
  request already caused the call to be emitted, the stream that follows only refines it.
  Translating only the initial shape leaves tool calls invisible.
- **Never pick a persistent permission option.** `allow_always` makes the agent write
  `.claude/settings.local.json` into the project's working directory, outside admission. Use
  `PermissionRequest::allow_once`, which returns `None` rather than falling back to a
  persistent choice.

## The admission point

An agent write request arrives as an incoming call to handle, not as an event to observe. The
response is deferred until the registry's verdict.

✅ **Correct** — the registry decides, the normalised event comes out afterwards:

```rust
async fn on_write_request(&mut self, path: PathBuf, content: String) -> Result<()> {
    // 1. Admission BEFORE the disk. That is the whole point of ACP.
    let verdict = self.registry.admit(self.session, &path, &content).await?;

    // 2. Any notice is prepared for the agent's next turn.
    if verdict.needs_notice() {
        self.pending_notice = Some(verdict.clone());
    }

    // 3. Only then, the write.
    if verdict.is_admitted() {
        tokio::fs::write(&path, &content).await?;
    }

    self.emit(AgentEvent::FileWrite { path, content });
    Ok(())
}
```

❌ **Counter-example** — the order is reversed, the registry is a bystander:

```rust
async fn on_write_request(&mut self, path: PathBuf, content: String) -> Result<()> {
    tokio::fs::write(&path, &content).await?;              // already on the disk
    let _ = self.registry.admit(self.session, &path, &content).await;  // too late
    Ok(())
}
```

The second one compiles, passes the tests, and removes the product's reason to exist. It is
the most important bug not to write in this repository.

## Reads matter as much as writes

The read-set fills from the agent's read requests. Without them there is no `StaleRead`
possible, and therefore no product.

**Filter**: substantial reads only — a file read in full. Not grep hits, not directory
listings. Agents read enormously; with no filter the read-set explodes and everything becomes
level 1 (ADR 0007).

## Capabilities: declare degradation, never mask it

```rust
pub struct Capabilities {
    pub can_intercept_writes: bool,   // ACP: true, PTY: false
    pub can_inject_context: bool,
    pub can_request_permission: bool,
}
```

A user in PTY mode who believes they have the admission guarantee is **worse off** than with
no tool at all: they are trusting a net that does not exist. The interface must show the
degradation.

Code corollary: never infer a capability from the backend type at the call site. Ask
`capabilities()`.

## Known protocol holes

- **`AskUserQuestion` is unavailable in plan mode.** Verified: the adapter puts it in
  `disallowedTools` unconditionally.
- **`Bash` stays native** as long as the client does not announce the `terminal` capability.
  An `echo > file` therefore escapes admission. Accepted in v0.1, and named in ADR 0016
  alongside the other holes: a net whose holes you do not know is worse than a net whose
  holes you do.
- ACP support is **uneven across harnesses**. An advertised capability is not always a
  functional one.
- The PTY fallback is **not optional** (ADR 0005). Without it, every protocol hole becomes
  an unsupported harness.

**We do not fork ACP.** Gaps are contributed upstream: a private dialect would remove
compatibility with harnesses, which is the only reason to use a standard protocol.

## Rules of detail

- **The subprocess's stdout belongs to the protocol.** Its own logs go to stderr, and so do
  ours. A `println!` on the ACP path corrupts the JSON-RPC framing — that is also why
  `print_stdout` is `deny`.
- A subprocess that dies is a normal case, not a panic: `AgentEvent::Error` then
  `SessionState::Failed`.
- Trame's `SessionId` and the ACP session id are **two different things**. Keep the mapping;
  do not reuse one for the other.
- The ACP permission mechanism already exists and the agent already knows how to wait. The
  registry's level 3 will hook into it rather than inventing a channel (v0.4, not v0.1).
- Timeouts: an agent may think for minutes. Never put a short timeout on a turn; do put one
  on admission, which must answer in milliseconds.

## How to test with no agent, no token, no authentication

`AcpBackend::connect` accepts any async reader/writer pair; `spawn` is only the special case
where they come from a subprocess. A test supplies a `tokio::io::duplex` and scripts the agent
in memory. See `crates/trame-agent/tests/interception.rs`.

⚠️ **And that is not enough.** A fake agent verifies our code, not the adapter's: we are the
ones writing its replies. Three bugs came through here, one of them a test that stayed green
for a whole phase by manufacturing the notification it was waiting for. The rule, and the
technique that found them, are in the `concurrency-testing` skill, section "a test that
simulates a third party does not verify the third party".

⚠️ **A trap already paid for**: `session/new` sends a request *and* awaits its response.
Sequencing "the fake agent waits for the request" then "the client sends it" deadlocks — each
waits for the other to start. It needs a `tokio::join!`.

## The phase 2 blocking check — lifted

It is lifted (ADR 0016): interception works, and more solidly than hoped.

The rule still stands for what comes next: if a harness does not allow intercepting before
the disk, **stop and say so**. Do not work around it with a watcher, do not fall back on
after-the-fact detection. That is a product-thesis problem, not an implementation detail, and
therefore a human decision.
