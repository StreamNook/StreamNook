//! Diagnostic-aware logging configuration service.
//!
//! Controls the log level at runtime based on user's diagnostic settings.
//! When diagnostics are disabled, only WARN and ERROR logs are output.
//! When enabled, DEBUG and INFO logs are also shown.

use lazy_static::lazy_static;
use log::LevelFilter;
use std::sync::atomic::{AtomicBool, Ordering};

lazy_static! {
    /// Global diagnostics enabled state
    static ref DIAGNOSTICS_ENABLED: AtomicBool = AtomicBool::new(true);
}

/// Initialize the logging system.
/// Call this once at application startup.
pub fn init_logging() {
    // Terminal default is INFO, not DEBUG: debug-level logs are the per-frame /
    // per-request firehose that makes the dev terminal unreadable (and the
    // sheer write volume is itself overhead). INFO keeps lifecycle + meaningful
    // lines. RUST_LOG overrides if a developer wants the firehose back, and the
    // runtime toggle below can still raise to Debug. The structured file
    // capture (ll_diagnostics) is independent of this and stays full-fidelity.
    let mut builder = env_logger::Builder::new();
    if std::env::var("RUST_LOG").is_ok() {
        builder.parse_default_env();
    } else {
        builder.filter_level(LevelFilter::Info);
        // Third-party crates that warn about things we cannot act on. fontdb
        // reports every malformed face in C:\Windows\Fonts on each launch,
        // which is pure noise in a log a user sends us. RUST_LOG still wins.
        builder.filter_module("fontdb", LevelFilter::Error);
    }
    builder
        .format_timestamp_millis()
        .format_module_path(true);

    // Fan out to stderr (dev) AND the persistent streamnook.log file sink.
    // Release builds have no stderr (windows_subsystem = "windows"), so the
    // file is the only place a shipped build's logs survive.
    let stderr_logger = builder.build();
    crate::services::file_log::install(stderr_logger, std::env::var("RUST_LOG").is_ok());

    println!("[DiagnosticLogger] Logging system initialized (terminal: info; file: streamnook.log)");
}

/// Set whether diagnostic logging is enabled.
/// When disabled, debug/info logs are silenced.
pub fn set_diagnostics_enabled(enabled: bool) {
    DIAGNOSTICS_ENABLED.store(enabled, Ordering::SeqCst);

    // Update the log level filter. "Enabled" raises to Info (not Debug — debug
    // is the firehose that floods the terminal); disabled drops to Warn.
    // RUST_LOG wins. Without this the runtime toggle forced Info on every
    // startup, so a developer setting RUST_LOG=...=debug got NOTHING and had no
    // way to tell the filter was being overridden — debug diagnostics simply
    // never appeared.
    if std::env::var("RUST_LOG").is_ok() {
        return;
    }
    let level = if enabled {
        LevelFilter::Info
    } else {
        LevelFilter::Warn
    };

    if enabled {
        crate::services::file_log::set_file_level(level);
        log::set_max_level(crate::services::file_log::global_level());
        // Logged AFTER raising the level so the transition lands in the file.
        log::info!("[DiagnosticLogger] Diagnostics ENABLED - showing all logs");
    } else {
        // Logged BEFORE dropping the level for the same reason.
        log::info!("[DiagnosticLogger] Diagnostics DISABLED - only warnings/errors");
        crate::services::file_log::set_file_level(level);
        log::set_max_level(crate::services::file_log::global_level());
    }
}

/// The saved "Detailed logging" setting last applied: 0 none yet, 1 off, 2 on.
static APPLIED_SETTING: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// Whether applying the saved setting `enabled` changes anything: true the
/// first time and whenever the saved value flips, false for every other save.
fn note_applied_setting(enabled: bool) -> bool {
    let next = if enabled { 2 } else { 1 };
    APPLIED_SETTING.swap(next, Ordering::SeqCst) != next
}

/// Apply the saved "Detailed logging" setting. Rust owns it: applied once at
/// boot and again only when a settings write changes it, so saving any other
/// setting, or a second window loading its settings, never resets a level
/// raised for the session.
pub fn apply_setting(enabled: bool) {
    if note_applied_setting(enabled) {
        set_diagnostics_enabled(enabled);
    }
}

/// Check if diagnostics are currently enabled.
pub fn is_diagnostics_enabled() -> bool {
    DIAGNOSTICS_ENABLED.load(Ordering::SeqCst)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_saved_setting_applies_only_when_it_changes() {
        // First apply (boot) always lands.
        assert!(note_applied_setting(false));
        // Saving any other setting, or another window loading, changes nothing.
        assert!(!note_applied_setting(false));
        assert!(!note_applied_setting(false));
        // Turning it on in Settings applies, once.
        assert!(note_applied_setting(true));
        assert!(!note_applied_setting(true));
        assert!(note_applied_setting(false));
    }
}
