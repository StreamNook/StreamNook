//! Cross-platform app-data directory base.
//!
//! Desktop resolves its data dir from Windows env vars (`%LOCALAPPDATA%`) or the
//! `dirs` crate (`config_dir`/`data_dir`). Neither works on Android: `dirs`
//! returns `None` there, and `%LOCALAPPDATA%` is unset, so every file-based
//! token/cookie/cache/settings store would land on an unwritable path.
//!
//! On mobile we resolve the app-private sandbox dir once at startup (from
//! Tauri's `path().app_data_dir()`, which returns the app's `filesDir` on
//! Android) and cache it here. Each resolver checks [`mobile_base`] first and,
//! only when it is set, returns a path under it. On desktop the cell is never
//! set, so every resolver keeps its existing platform path unchanged — an
//! installed desktop user's data location does not move.

use std::path::PathBuf;
use std::sync::OnceLock;

static MOBILE_BASE: OnceLock<PathBuf> = OnceLock::new();

/// Record the app-private data dir. Called once at startup on mobile with
/// Tauri's resolved `app_data_dir`. A no-op on desktop (never called there) and
/// idempotent (the first value wins).
pub fn set_base(dir: PathBuf) {
    let _ = MOBILE_BASE.set(dir);
}

/// The mobile app-private data dir, or `None` on desktop.
///
/// Resolvers use this as: `if let Some(base) = mobile_base() { return
/// Ok(base.join("StreamNook")...); }` before their existing desktop logic. All
/// stores share this one sandboxed dir on mobile; their distinct filenames keep
/// them from colliding.
pub fn mobile_base() -> Option<PathBuf> {
    MOBILE_BASE.get().cloned()
}
