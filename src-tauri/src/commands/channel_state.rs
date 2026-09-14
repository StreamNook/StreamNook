//! Per-channel chat state (viewers, channel points, pinned messages) and
//! per-user history watches, owned by Rust. See `services::channel_state`.

use crate::services::channel_state;
use crate::services::user_message_history_service::UserMessageHistoryService;

/// A window is showing chat for `login`. Returns the current state so the
/// caller can paint before the first `channel-state` event.
#[tauri::command]
pub async fn watch_channel_state(
    login: String,
    channel_id: String,
) -> Result<channel_state::ChannelState, String> {
    Ok(channel_state::watch(&login, &channel_id).await)
}

/// A window stopped showing chat for `login`.
#[tauri::command]
pub async fn unwatch_channel_state(login: String) -> Result<(), String> {
    channel_state::unwatch(&login).await;
    Ok(())
}

/// Current state for a watched channel, `None` if nobody watches it.
#[tauri::command]
pub async fn get_channel_state(login: String) -> Result<Option<channel_state::ChannelState>, String> {
    Ok(channel_state::get(&login).await)
}

/// Manual refresh of one section (`viewers`, `points`, `pinned`) after a user
/// action that changes it (pin, claim, spend). The result arrives as an event.
#[tauri::command]
pub async fn refresh_channel_state(login: String, section: String) -> Result<(), String> {
    channel_state::refresh(&login, &section).await
}

/// A user card is open on `user_key`; every new message from that user is
/// emitted as `user-history-message` until unwatched.
#[tauri::command]
pub async fn watch_user_history(user_key: String) -> Result<(), String> {
    UserMessageHistoryService::watch_user(&user_key);
    Ok(())
}

#[tauri::command]
pub async fn unwatch_user_history(user_key: String) -> Result<(), String> {
    UserMessageHistoryService::unwatch_user(&user_key);
    Ok(())
}
