use crate::services::mod_log_storage_service::ModLogStorageService;
use tauri::AppHandle;

/// Load a channel's persisted mod-log entries (oldest first).
#[tauri::command]
pub async fn load_mod_logs(
    app_handle: AppHandle,
    channel: String,
) -> Result<Vec<serde_json::Value>, String> {
    Ok(ModLogStorageService::load_channel(&app_handle, &channel))
}

/// Record one moderation action and return the entry to show: the same action
/// already reported (by the other feed, or by another window) resolves to one
/// entry, so every window shows and stores it once.
#[tauri::command]
pub async fn record_mod_log(
    app_handle: AppHandle,
    channel: String,
    entry: serde_json::Value,
) -> Result<serde_json::Value, String> {
    ModLogStorageService::record(&app_handle, &channel, entry)
}

/// Clear a channel's persisted mod-log entries.
#[tauri::command]
pub async fn clear_mod_logs(app_handle: AppHandle, channel: String) -> Result<(), String> {
    ModLogStorageService::clear_channel(&app_handle, &channel)
}
