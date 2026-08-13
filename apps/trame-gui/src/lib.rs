//! The Trame desktop application. **It observes, it does not drive.**
//!
//! # v0.1 scope, identical to the TUI
//!
//! - one panel per session, with its state — `Idle` / `Thinking` / `Writing`
//! - the live event feed, with verdicts highlighted
//! - `StaleRead` distinguished from `Clean` **on two axes**: colour and marker
//! - the **admitted / observed** distinction visible
//! - a degradation banner when `can_intercept_writes` is false
//!
//! No multi-project, no branches, no diffs, no configuration.
//!
//! # Why it structurally cannot drive
//!
//! It receives only a `Receiver<Observation>`. It holds no `RegistryHandle`, so `admit` is not
//! reachable from here — and that is enforced by the crate graph rather than by review
//! ([ADR 0022](https://github.com/mrcoolbay/trame/blob/main/docs/adr/0022-decoupage-daemon-gui.md)).
//!
//! The display state comes from [`trame_view`], shared with the TUI: these properties have one
//! home.

pub mod theme;
pub mod view;
