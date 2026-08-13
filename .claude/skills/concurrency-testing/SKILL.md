---
name: concurrency-testing
description: Testing concurrent code deterministically in Trame — clock injection, controlled ordering, no sleep in tests, a mandatory negative control on every measuring device. Read before writing a test that touches an actor, time, or several sessions, and before writing a probe or a canary.
---

# Testing concurrency — Trame

## The rule

> **No `sleep` in a test. Ever.**

A `sleep` in a test means one of two things: either the test is waiting on real time, and it
is slow; or it is waiting on a scheduling order, and it is flaky. Both are unacceptable, and
the second is worse: a test that fails one time in thirty is a test people end up ignoring,
and an ignored suite protects nothing.

The registry is precisely the component where that risk peaks — its verdicts depend on the
order of events and on the passage of time.

## Technique 1 — Inject the clock

`trame_core::Clock` exists for this. The registry makes a time-dependent decision: a read-set
entry expires after ten minutes. Testing that for real would mean a ten-minute test.

✅ **Correct** — time only moves on command, the test is instantaneous:

```rust
use std::sync::Arc;
use chrono::TimeDelta;
use trame_core::clock::{Clock, ManualClock};

#[tokio::test]
async fn an_expired_read_no_longer_triggers_a_notice() {
    let clock = Arc::new(ManualClock::new());
    let (registry, _join) = spawn_registry(project, clock.clone());

    registry.record_read(session_a, "auth.rs").await.unwrap();
    registry.admit(session_b, "auth.rs", "// changed").await.unwrap();

    // Just before expiry: the notice is still relevant.
    clock.advance(TimeDelta::minutes(9));
    let verdict = registry.admit(session_a, "handlers.rs", "// ...").await.unwrap();
    assert_eq!(verdict.level(), 1, "at 9 min the read still counts");

    // After: the agent's context has turned over, so we stay quiet.
    clock.advance(TimeDelta::minutes(2));
    registry.record_read(session_a, "auth.rs").await.unwrap();
    registry.admit(session_b, "auth.rs", "// again").await.unwrap();
    clock.advance(TimeDelta::minutes(11));
    let verdict = registry.admit(session_a, "other.rs", "// ...").await.unwrap();
    assert_eq!(verdict, Verdict::Clean, "beyond 10 min, no more notices");
}
```

❌ **Counter-example** — not only slow, but wrong: it tests the system clock.

```rust
#[tokio::test]
async fn an_expired_read_no_longer_triggers_a_notice() {
    registry.record_read(session_a, "auth.rs").await.unwrap();
    tokio::time::sleep(Duration::from_secs(601)).await;   // ten minutes of CI
    // ...
}
```

`ManualClock` lives behind `trame-core`'s `test-support` feature. It is enabled as a
dev-dependency, never as a production dependency.

## Technique 2 — Order by messages, not by time

An actor handles its messages one at a time. An `await` on the reply `oneshot` is therefore an
**exact synchronisation barrier**: when `admit(...).await` returns, the message has been
handled and the actor's state includes its effect. There is nothing further to wait for.

That is what makes the product's canonical scenario testable in five deterministic lines,
**with no agent at all**:

```rust
#[tokio::test]
async fn stale_read_with_no_write_collision_at_all() {
    let clock = Arc::new(ManualClock::new());
    let (registry, _join) = spawn_registry(project, clock.clone());

    // 1. A reads auth.rs
    registry.record_read(session_a, "auth.rs").await.unwrap();

    // 2. B writes auth.rs -> Clean: nobody else read what B overwrites
    let verdict_b = registry.admit(session_b, "auth.rs", "fn validate_token()").await.unwrap();
    assert_eq!(verdict_b, Verdict::Clean);

    // 3. A writes handlers.rs -> StaleRead, even though there is NO write collision
    //    at all: two different files. A per-file lock would see nothing.
    let verdict_a = registry.admit(session_a, "handlers.rs", "verify_token()").await.unwrap();
    let Verdict::StaleRead { stale } = verdict_a else {
        panic!("expected StaleRead, got {verdict_a:?}");
    };
    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0].path, PathBuf::from("auth.rs"));
    assert_eq!(stale[0].last_writer, session_b);
}
```

This test is **the product's reason to exist**. If it breaks, the test is not what has a
problem.

## Technique 3 — `start_paused` for tokio's time

When it is tokio's own time that is at stake — an `interval`, a `timeout` — rather than the
business clock:

```rust
#[tokio::test(start_paused = true)]
async fn an_admission_timeout_is_surfaced() {
    // tokio's virtual time jumps instantly to the next wake-up.
    tokio::time::advance(Duration::from_secs(30)).await;
    // ...
}
```

Two clocks, two uses. `ManualClock` for business decisions, `start_paused` for tokio's time
primitives. Do not mix them in one test: you no longer know which one you are testing.

## Technique 4 — Real concurrency, when it is the subject

To check that N concurrent sessions really produce a coherent total order, run genuinely in
parallel, but assert on **invariants**, never on a precise order.

```rust
#[tokio::test]
async fn sequence_numbers_stay_unique_and_gapless_under_load() {
    let (registry, _join) = spawn_registry(project, Arc::new(SystemClock));

    let mut set = tokio::task::JoinSet::new();
    for i in 0..50 {
        let registry = registry.clone();
        set.spawn(async move {
            registry.admit(sessions[i % 3], &format!("f{i}.rs"), "x").await
        });
    }
    while set.join_next().await.is_some() {}

    let snapshot = registry.snapshot().await.unwrap();
    let mut seqs: Vec<u64> = snapshot.writes.iter().map(|w| w.seq.get()).collect();
    seqs.sort_unstable();
    seqs.dedup();
    assert_eq!(seqs.len(), 50, "no sequence number may be reused");
    assert_eq!(seqs.first().copied(), Some(1));
    assert_eq!(seqs.last().copied(), Some(50), "and no gaps");
}
```

❌ **Counter-example** — asserting an order nothing guarantees:

```rust
assert_eq!(snapshot.writes[0].session, session_a);  // depends on the scheduler
```

## ★ Choose the narrowest observable that expresses the property

A property **per file** cannot be tested with a **global** counter. That mistake shipped
here: an assertion checked that an echo consumed no sequence number by comparing the global
counter, while fixture writes were also advancing it. The test passed **by luck** for weeks,
on a timing coincidence local to one machine, and the macOS CI job found it in one run.

> **A shared proxy moves for reasons unrelated to the property.** When it does not move on
> your machine, the test looks green. Pick the observable that cannot move for any other
> reason.

The same blind spot took another shape in the scroll test: it counted writes against a
threshold, when what mattered was displayed **rows** — reads and session lines occupy rows
too.

## ★ A test that simulates a third party does not verify the third party

**The rule**, born from three consecutive bugs:

> **Every test that simulates a third party must be paired with a test that questions the
> real third party.**

The in-memory fake agent is indispensable — it makes the transport deterministic, with no
subprocess, no authentication, no token spent. But it has a structural flaw: **we are the ones
writing what the third party replies.** A test that manufactures the answer it expects
verifies its own fiction, and it stays green while the product is broken.

### The three bugs, and what they have in common

| Bug | What the test simulated | What the third party actually does |
|---|---|---|
| End of turn never detected | the test emitted an "end_of_turn" `sessionUpdate` | **that notification does not exist** — end of turn is the response to `session/prompt` |
| Invisible tool calls | the test only emitted `tool_call` | the adapter sometimes emits **only** `tool_call_update` |
| Reads never recorded | the test always read through `fs/read_text_file` | the agent reads through `Grep` or `Bash`, which **escape** interception |

The first is the most instructive: the test **manufactured the notification it was waiting
for**. It stayed green for the whole of phase 2 and validated a path that did not exist. The
block only appeared at the first experimental round, when a real agent had to chain two turns.

All three were found **outside the tests**, by questioning the third party.

### The technique that found them

Point `CLAUDE_CODE_EXECUTABLE` at a fake `claude` that only writes its `argv`, then run the
**real negotiation** against the **real adapter**. You then read the command line the adapter
*would* have handed to the real binary.

```sh
#!/bin/sh
for a in "$@"; do echo "$a" >> "$CAPTURE"; done
exit 0
```

Three properties make this a technique rather than a hack:

- **No authentication, no token spent.** The model is never called.
- **The anti-nesting guard does not apply**: the real `claude` is never launched, so this runs
  even from inside a Claude Code session, and in CI.
- **It observes the third party, not our simulation of it.** That is the entire point.

It is what allowed the ACP migration to be refused on a measurement, rather than on an
optimistic reading of code:

```
0.16.2: --disallowedTools AskUserQuestion,Read,Write,Edit   -> interception possible
0.66.0: --disallowedTools AskUserQuestion                   -> interception lost
```

### The discipline that follows

- Every third-party behaviour an invariant depends on has **a canary**:
  `crates/trame-agent/tests/interception_canary.rs`. It fails loudly, and a second test
  checks that it **knows how** to fail — a canary that cannot fail guards nothing.
- When a test simulates a protocol, its comment states **which observation of the real third
  party** justifies the simulated shape. Without that trace, the simulation drifts unseen.
- A test that verified nothing **says so loudly** rather than going green: the canary prints a
  warning when the adapter is absent.

## ★ Every measuring device carries a negative control

**The rule**:

> **A test, a canary or a probe must prove it knows how to fail, before its success is
> believed.** Without that control you are not measuring the subject: you are measuring the
> device's ability to produce a plausible trace.

It is the same rule as the previous section, seen from the other end. Simulating a third party
makes you believe you observed it; a device with no negative control makes you believe you
measured.

### Two occurrences, one failure mode

| Device | What it seemed to show | What was actually happening |
|---|---|---|
| End-of-turn test (phase 2) | the stream does emit `Done` at the end of a turn | the test **emitted the notification itself** — and that notification does not exist |
| Probe hook (probe 3) | `PostToolUse` fires after a refused call | the hook **observed nothing and refused nothing** — the turn produced a credible trace anyway |

Probe 3's hook was written like this:

```sh
/usr/bin/python3 - <<'PY'      # the heredoc IS python's stdin
raw = sys.stdin.read()         # so this reads EOF, never the hook payload
```

With the program arriving on stdin, `sys.stdin.read()` returned an empty string. The hook wrote
no capture and returned no decision — but the turn ran normally, with a "no results" `Grep`
that was tempting to read as "refused". The conclusion would have been **the opposite of the
truth**, and nothing in the trace flagged it.

Neither device measured anything. Both looked like they worked. **That is the signature of the
failure mode: the output is plausible, so it triggers no verification.**

### The negative control in practice

Before drawing a conclusion from a device, make it fail on purpose:

```sh
# Probe: does the hook really decide? Question it empty, outside a session.
echo '{"tool_name":"Grep","tool_input":{"pattern":"base_url"}}' | ./hook.sh   # -> must decide
echo '{"tool_name":"Grep","tool_input":{"pattern":"other"}}'    | ./hook.sh   # -> must stay quiet
```

```rust
// Canary: a test verifies the canary fails when the condition disappears.
// A canary that cannot fail guards nothing.
#[test]
fn the_canary_knows_how_to_fail() { /* ... */ }
```

And **a counting invariant**, whenever the device produces traces: the expected number of
captures is computed *before* reading them. "`pre.jsonl` should hold one line per tool call"
was enough to find the heredoc bug; without that prior arithmetic, an empty file reads as
"nothing happened".

### ★★ A negative control must be carried by a sample that exercises it alone

The rule above has a floor under it, and it took a real hole to find. The `check-language`
guard was fresh; to check it knew how to fail, one word was removed from its list. **The
self-test stayed green.** That looked like solidity.

What it actually meant: the sample meant to exercise that word contained another listed word
too. **The detector caught it by a different signal**, so the control was not testing what it
claimed to test.

> **Otherwise the hole hides behind the other signals, and the control passes while giving
> the impression of having verified.**

A control redone properly — four ways to break the file, each of which must make it go red —
immediately found two real holes: the search was case-sensitive, and **no sample exercised
accent detection at all**, so that branch could have been dead code with the self-test green.
Hence the shape in [`scripts/no_french.py`](../../../scripts/no_french.py): two samples marked
`ONLY`, one accent-only and one word-only, each carrying **exactly one** signal.

### ★★ A harness measures the production component, never a twin

The experimental round of ADR 0018 spent two campaigns measuring `ConfigurableNotice::Neutral`
while believing it measured `StaleReadNotice`. The two texts differed by one line; the round's
`15/15` ceiling made the substitution undetectable. Measured directly, the shipped text scored
**3/6** where the twin scored 3/3.

> **When a type exists for an experiment, it is built as a comparison against production, and
> a test observes that they differ.**

That is what `production_is_exactly_the_neutral_text` does: it pins byte-for-byte equality with
the variant that is meant to match, and difference from the others. And the harness's default
is the production contributor, so forgetting a flag measures the product rather than the twin.

Corollary on the ceiling: **a ceiling is not a result, it is an admission.** `15/15` does not
say "the message is optimal", it says "this device can no longer distinguish anything". Until a
round has put something in the wrong, it has not yet shown it is able to.

### Corollary on writing it up

A probe report states **explicitly** which negative control was run. A report that mentions
none is not a measurement, it is an impression — and it will be reread in six months as though
it were a measurement.

## Rules of detail

- **An actor's `JoinHandle` is kept** (`let (h, _join) = ...`). Dropping it can stop the actor
  mid-test.
- **No `#[tokio::test(flavor = "multi_thread")]` by default.** The single-threaded runtime is
  deterministic; multi-thread only earns its place when the test *is* about real parallelism.
- **One test per behaviour**, named **in English** as a specification sentence:
  `stale_read_with_no_write_collision_at_all` states what is guaranteed.
- **The assertion message explains the invariant**, not the value:
  `assert!(x, "95% of traffic must pass without a word")`.
- **Test through the handle, not through internal state.** If a test needs to read the actor's
  private state, a `Snapshot` message is missing.
- `unwrap()` is **allowed in tests** (`allow-unwrap-in-tests` in `clippy.toml`): it is the most
  readable way to fail.
- If a test turns flaky, do not rerun it in a loop and do not mark it `#[ignore]`: find the
  `sleep` or the ordering assertion hiding in it.
