//! Platform primitives: the small set of operations whose IMPLEMENTATION is
//! OS-specific but whose MEANING is not.
//!
//! # Why this module exists
//!
//! Before it, every such operation was a pair of `#[cfg(windows)]` /
//! `#[cfg(not(windows))]` twins living inside whatever service happened to need
//! it, and the non-Windows twin was almost always a stub returning `Err`,
//! `false`, or an empty collection. That shape has three specific problems, all
//! of which cost real time during the macOS port:
//!
//! 1. **The degradation is invisible from the call site.** `open_in_browser(url)`
//!    reads as "opens a browser". It returned
//!    `Err("only wired for Windows right now")` on every other platform, and the
//!    caller could not tell.
//! 2. **Windows CI structurally cannot catch a missing gate.** Only a non-Windows
//!    compile finds one, and until this port there was no non-Windows compile.
//! 3. **Nobody could answer "what still degrades?"** without grepping the whole
//!    tree, because there was no single place the answer lived.
//!
//! # The rule for this module
//!
//! **Every function here must have a real implementation on every desktop
//! platform we ship.** A stub does not belong here. If something genuinely
//! cannot be implemented somewhere, it returns a typed "unsupported" that the
//! caller must handle explicitly, rather than a silent empty value that looks
//! like a legitimate answer.
//!
//! Policy stays with the caller. `process::running_names()` reports what is
//! running; deciding which of those names means "a broadcast is live" is
//! `streamer_mode`'s business, not this module's.

pub mod app_update;
pub mod browser;
pub mod capture;
pub mod cookies;
pub mod fs;
pub mod process;
pub mod responsiveness;
pub mod webview_store;
