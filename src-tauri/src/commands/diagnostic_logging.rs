//! Tauri commands for diagnostic logging control.

use crate::services::diagnostic_logger;
use tauri::command;

/// Raise or lower diagnostic logging for this session without saving it.
/// The saved `error_reporting_enabled` setting is applied by Rust itself, at
/// boot and when a settings write changes it; no window re-applies it.
#[command]
pub fn set_diagnostics_enabled(enabled: bool) {
    diagnostic_logger::set_diagnostics_enabled(enabled);
}

/// Check if diagnostics are currently enabled.
#[command]
pub fn is_diagnostics_enabled() -> bool {
    diagnostic_logger::is_diagnostics_enabled()
}
