//! The mention rule: does a Twitch chat message name you, or reply to you?
//!
//! One copy, used by the app (the chat rule engine and the IRC parser) and by
//! the mention watcher, which reads followed channels' chat while the app is
//! closed. `tests/fixtures.json` holds the cases both must agree on.
//!
//! - `name`: which names count and the whole-word match.
//! - `normalize`: the text the rule reads (a `/me` unwrapped, a reply's
//!   leading `@parent` taken off).
//! - `judge`: the verdict for one message and one person.

pub mod judge;
pub mod name;
pub mod normalize;
