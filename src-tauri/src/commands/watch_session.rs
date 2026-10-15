//! The main window's watch session (see services/watch_session.rs).

use crate::services::offline_room::{self, OfflineRoom};
use crate::services::watch_session::{self, OfflineDecision, OfflineTrigger, WatchTarget};
use crate::models::settings::AppState;
use crate::rt::AppHandle;
use tauri::Manager;

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

/// The watched stream looks offline: confirm it, and say what to do. `trigger`
/// is "player" when the player stalled, absent for every other signal.
#[tauri::command]
pub async fn watch_session_resolve_offline(app: AppHandle, trigger: Option<OfflineTrigger>) -> OfflineDecision {
    watch_session::resolve_offline(app, trigger.unwrap_or_default()).await
}

/// Everything a Twitch channel's offline room shows, in one answer.
#[tauri::command]
pub async fn open_offline_room(app: AppHandle, login: String) -> Result<OfflineRoom, String> {
    let mut room = offline_room::open(&login).await?;
    let resume = app
        .state::<AppState>()
        .settings
        .lock()
        .map(|s| s.video_player.resume_vod_playback)
        .unwrap_or(false);
    if resume {
        offline_room::attach_resume(&mut room).await;
    }
    Ok(room)
}

/// Republish Discord presence for the current session (Discord was switched on).
#[tauri::command]
pub async fn watch_session_refresh_presence(app: AppHandle) -> Result<(), String> {
    watch_session::refresh_presence(app).await;
    Ok(())
}
