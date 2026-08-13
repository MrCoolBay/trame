---
name: actor-pattern
description: How to write a tokio actor in Trame — mpsc plus oneshot, state ownership, a cloneable handle, clean shutdown, backpressure. Read before creating an actor, before adding a message to an existing one, or the moment you are tempted to write Arc<Mutex<...>>.
---

# The actor pattern — Trame

## The invariant

> **An actor owns its state. Never `Arc<Mutex<_>>` for business state.**

This is not a style preference. The registry has to answer "has this file changed **since**
this session read it", which presupposes a **total order** over a project's reads and
writes. A `Mutex` gives mutual exclusion, not a total order. An actor that handles its
messages one at a time gives the total order for free, and there is no interleaving left to
reason about.

An `Arc` over an **immutable** value — a clock, a configuration — is not covered by this.
There is no mutation, so there is no order to guarantee.

## The five pieces

1. A `...Msg` enum, one variant per operation, each carrying its reply
   `oneshot::Sender`.
2. A **private** state struct, never exposed, that owns everything.
3. A `while let Some(msg) = rx.recv().await` loop.
4. A public, cloneable `...Handle` struct that wraps the `mpsc::Sender` and exposes typed
   `async` methods. **The caller never builds a message by hand.**
5. A `spawn` that returns the handle and the `JoinHandle`.

## A complete example

Resource claims: port 3000 is machine-wide, so this actor is **global** to the workspace,
unlike the registry which is per project (ADR 0010).

```rust
use std::collections::HashMap;

use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

/// A refused claim says **who** holds it, otherwise the user has no way to act.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimOutcome {
    Granted,
    Held { by: String },
}

/// A message addressed to the actor. Each variant carries its return channel.
enum ClaimMsg {
    Claim { resource: String, owner: String, reply: oneshot::Sender<ClaimOutcome> },
    Release { resource: String, owner: String, reply: oneshot::Sender<bool> },
    Snapshot { reply: oneshot::Sender<Vec<(String, String)>> },
}

/// The state. Private, owned by the actor, never shared.
struct ClaimActor {
    held: HashMap<String, String>,
    rx: mpsc::Receiver<ClaimMsg>,
}

impl ClaimActor {
    /// The loop. One message at a time: the total order is structural.
    async fn run(mut self) {
        while let Some(msg) = self.rx.recv().await {
            match msg {
                ClaimMsg::Claim { resource, owner, reply } => {
                    let outcome = match self.held.get(&resource) {
                        Some(existing) if *existing != owner => {
                            ClaimOutcome::Held { by: existing.clone() }
                        }
                        _ => {
                            self.held.insert(resource.clone(), owner.clone());
                            ClaimOutcome::Granted
                        }
                    };
                    tracing::debug!(%resource, %owner, ?outcome, "claim");
                    // The caller may have given up: its `oneshot` is closed. That is not
                    // an error, so it is ignored. `let _ =`, not `.unwrap()`.
                    let _ = reply.send(outcome);
                }
                ClaimMsg::Release { resource, owner, reply } => {
                    let released = self.held.get(&resource) == Some(&owner);
                    if released {
                        self.held.remove(&resource);
                    }
                    let _ = reply.send(released);
                }
                ClaimMsg::Snapshot { reply } => {
                    let snapshot =
                        self.held.iter().map(|(r, o)| (r.clone(), o.clone())).collect();
                    let _ = reply.send(snapshot);
                }
            }
        }
        // Leaving the loop = every handle has dropped. Clean shutdown, with no dedicated
        // signal, no `select!`, no `CancellationToken`.
        tracing::info!("claim actor stopped");
    }
}

/// The handle. Cloneable, and the only way to reach the actor.
#[derive(Debug, Clone)]
pub struct ClaimHandle {
    tx: mpsc::Sender<ClaimMsg>,
}

impl ClaimHandle {
    /// One possible error: the actor is dead. Nothing else can fail.
    pub async fn claim(&self, resource: &str, owner: &str) -> Result<ClaimOutcome, ActorGone> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(ClaimMsg::Claim {
                resource: resource.to_owned(),
                owner: owner.to_owned(),
                reply,
            })
            .await
            .map_err(|_| ActorGone)?;
        rx.await.map_err(|_| ActorGone)
    }

    pub async fn release(&self, resource: &str, owner: &str) -> Result<bool, ActorGone> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(ClaimMsg::Release {
                resource: resource.to_owned(),
                owner: owner.to_owned(),
                reply,
            })
            .await
            .map_err(|_| ActorGone)?;
        rx.await.map_err(|_| ActorGone)
    }

    pub async fn snapshot(&self) -> Result<Vec<(String, String)>, ActorGone> {
        let (reply, rx) = oneshot::channel();
        self.tx.send(ClaimMsg::Snapshot { reply }).await.map_err(|_| ActorGone)?;
        rx.await.map_err(|_| ActorGone)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the claim actor is no longer reachable")]
pub struct ActorGone;

/// Starts the actor. The channel bound **is** the backpressure policy.
pub fn spawn_claims() -> (ClaimHandle, JoinHandle<()>) {
    // Bounded, never `unbounded_channel()`: an unbounded queue turns an overload into a
    // silent memory leak. 64 pending messages on an actor that handles them in
    // microseconds is already a lot.
    let (tx, rx) = mpsc::channel(64);
    let join = tokio::spawn(ClaimActor { held: HashMap::new(), rx }.run());
    (ClaimHandle { tx }, join)
}
```

The matching test — **no agent, no `sleep`, deterministic**:

```rust
#[tokio::test]
async fn an_already_held_resource_names_its_holder() {
    let (claims, _join) = spawn_claims();

    assert_eq!(claims.claim("port:3000", "portailfcd").await.unwrap(), ClaimOutcome::Granted);
    assert_eq!(
        claims.claim("port:3000", "lyra-rp").await.unwrap(),
        ClaimOutcome::Held { by: "portailfcd".into() },
        "a refusal must say who holds the resource"
    );

    assert!(claims.release("port:3000", "portailfcd").await.unwrap());
    assert_eq!(claims.claim("port:3000", "lyra-rp").await.unwrap(), ClaimOutcome::Granted);
}
```

## Counter-example

```rust
// ❌ EVERYTHING here is wrong.
#[derive(Clone)]
pub struct Claims {
    held: Arc<Mutex<HashMap<String, String>>>,   // shared business state
}

impl Claims {
    pub fn claim(&self, resource: &str, owner: &str) -> bool {
        let mut held = self.held.lock().unwrap();   // unwrap: denied in CI
        if held.contains_key(resource) {
            return false;                          // a bool: "by whom" is lost
        }
        held.insert(resource.to_owned(), owner.to_owned());
        true
    }
}
```

Four problems: business state under a `Mutex` (invariant 1 in `AGENTS.md`), `unwrap()` on a
poisoned lock, a bool where an actionable result is needed, and a `std::sync::Mutex` held
across an await point the moment the function becomes `async`.

## Rules of detail

- **`let _ = reply.send(...)`, never `.unwrap()`.** A closed `oneshot` means the caller gave
  up. That is normal, not an error.
- **A bounded `mpsc::channel`.** Never `unbounded_channel()`.
- **No dedicated shutdown signal.** The actor stops when the last handle drops. An
  additional `CancellationToken` is a second way to die, and therefore one more bug.
- **The handle exposes methods, not the enum.** The `...Msg` enum stays private to the
  module.
- **An actor never calls another actor and awaits its reply inside its own loop** — that is
  a deadlock waiting to happen. When it is needed, spawn the subtask and hand it the
  requester's `oneshot::Sender`.
- **Six fields maximum per message variant.** Beyond that the actor is badly carved up;
  `clippy.toml` enforces the threshold.
- **One operation per variant.** No `Msg::Do { op: Operation }` — that moves the dispatch
  into a second `match` and destroys the typing of the replies.
