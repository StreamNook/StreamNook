//! The main window's watch session (see services/watch_session.rs).

use crate::services::offline_room::{self, OfflineRoom};
use crate::services::watch_session::{self, OfflineDecision, OfflineTrigger, WatchTarget};
use crate::commands::streaming::StartError;
use crate::models::settings::AppState;
// Named apart: the command macros make a same-named item for each command.
use crate::services::watch_start::{self as starts, RoomOpen, WatchSeed, WatchStart};
use crate::rt::AppHandle;
use tauri::{Manager, State};

/// Start watching a live channel in the main view: relay, row, drops aim and
/// watch session in one call (see services/watch_start.rs). `provider` is
/// Twitch when absent; `login` is then a Twitch login, else the platform's
/// channel (a Kick slug).
#[tauri::command]
pub async fn watch_start(
    app: AppHandle,
    state: State<'_, AppState>,
    login: String,
    provider: Option<String>,
    seed: Option<WatchSeed>,
    quality: Option<String>,
    relay_ready: Option<bool>,
) -> Result<WatchStart, StartError> {
    starts::start(app, state, provider, login, seed, quality, relay_ready.unwrap_or(false)).await
}

/// Stop watching in the main view: relay, watch session, and unless MultiNook
/// takes the channel over, drops and channel points.
#[tauri::command]
pub async fn watch_stop(app: AppHandle, state: State<'_, AppState>, preserve_backend: Option<bool>) -> Result<(), String> {
    starts::stop(app, state, preserve_backend.unwrap_or(false)).await;
    Ok(())
}

/// Open a Twitch channel's offline room in the main view (see
/// services/watch_start.rs `open_room`).
#[tauri::command]
pub async fn watch_open_room(
    app: AppHandle,
    login: String,
    seed: Option<WatchSeed>,
    chat_only: Option<bool>,
) -> Result<RoomOpen, String> {
    Ok(starts::open_room(app, login, seed, chat_only.unwrap_or(false)).await)
}

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
