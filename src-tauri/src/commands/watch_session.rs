//! The main window's watch session (see services/watch_session.rs).

use crate::services::watch_session::{self, OfflineDecision, WatchTarget};
use tauri::AppHandle;

/// The main window started playing this stream.
#[tauri::command]
pub async fn watch_session_start(app: AppHandle, target: WatchTarget) -> Result<(), String> {
    watch_session::start(app, target).await;
    Ok(())
}

/// The main window stopped playing. `preserve_backend` hands the channel to
/// MultiNook with its subscriptions intact.
#[tauri::command]
pub async fn watch_session_stop(app: AppHandle, preserve_backend: bool) -> Result<(), String> {
    watch_session::stop(app, preserve_backend).await;
    Ok(())
}

/// The watched Twitch stream looks offline: confirm it, and say what to do.
#[tauri::command]
pub async fn watch_session_resolve_offline(app: AppHandle) -> OfflineDecision {
    watch_session::resolve_offline(app).await
}

/// Republish Discord presence for the current session (Discord was switched on).
#[tauri::command]
pub async fn watch_session_refresh_presence(app: AppHandle) -> Result<(), String> {
    watch_session::refresh_presence(app).await;
    Ok(())
}
