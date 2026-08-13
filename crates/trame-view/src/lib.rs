//! What **every Trame interface** shares, whatever its rendering engine.
//!
//! Two things, and nothing else:
//!
//! - [`state`] — the display state. Pure, synchronous, engine-free.
//! - [`source`] — opening a project: journal, registry, watcher, and the observation feed
//!   that comes out of it.
//!
//! # Why a crate and not a module in each interface
//!
//! [`state`] does not hold formatting, it holds the **properties a Trame interface has to
//! keep**: a `StaleRead` is notable and a `Clean` is not, an observed write does not count
//! as an admitted one, a single degraded session is enough to say so, the feed is bounded.
//!
//! Copying those rules into the TUI and into the GUI would give them two places to diverge —
//! and they are precisely the rules ADR 0022 makes the product's display contract. One home.
//!
//! What stays specific to each interface: colours, characters, layout. What lives here:
//! **what gets shown, and why it deserves attention**.
//!
//! Position in the dependency chain:
//! `core <- journal <- registry <- {agent, vcs} <- daemon <- view <- {tui, gui}`.

/// ★ How a feed row's time is written. **Milliseconds, and that is not cosmetic.**
///
/// The first version was `%H:%M:%S`. In a real scenario run, twelve rounds of three
/// admissions each landed inside a handful of seconds, so three or four consecutive rows
/// carried the **same** timestamp — and the feed became unreadable on the one thing it is
/// ordered by.
///
/// That matters more here than in a typical log. This product's whole claim is about
/// *ordering*: "this file changed **since** you read it". A display that cannot separate
/// two events by time undercuts the argument it exists to make.
///
/// It lives in `trame-view` rather than in each interface because both have to agree — the
/// TUI and the GUI showing different resolutions would be two places for the same rule to
/// drift.
pub const TIME_FORMAT: &str = "%H:%M:%S%.3f";

pub mod state;

pub use state::{App, FEED_CAPACITY, Kind, Line, Panel};
