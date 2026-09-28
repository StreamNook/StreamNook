use crate::services::eventsub_service::EventSubService;
use log::debug;
use std::sync::Arc;
use crate::rt::AppHandle;
use tauri::State;
use tokio::sync::RwLock;

// Global EventSub service state
pub struct EventSubServiceState(pub Arc<RwLock<EventSubService>>);

/// Connect to EventSub for a specific broadcaster
/// This establishes a WebSocket connection and subscribes to all relevant events
#[tauri::command]
pub async fn connect_eventsub(
    broadcaster_id: String,
    app_handle: AppHandle,
    state: State<'_, EventSubServiceState>,
) -> Result<(), String> {
    let service = state.0.read().await;

    debug!(
        "[EventSub] Connecting to EventSub for broadcaster: {}",
        broadcaster_id
    );

    service
        .connect_and_listen(broadcaster_id, app_handle)
        .await
        .map_err(|e| format!("Failed to connect to EventSub: {}", e))?;

    Ok(())
}

/// Disconnect from EventSub
#[tauri::command]
pub async fn disconnect_eventsub(state: State<'_, EventSubServiceState>) -> Result<(), String> {
    let service = state.0.read().await;

    debug!("[EventSub] Disconnecting from EventSub");

    service.disconnect().await;

    Ok(())
}

/// Check if EventSub is currently connected
#[tauri::command]
pub async fn is_eventsub_connected(state: State<'_, EventSubServiceState>) -> Result<bool, String> {
    let service = state.0.read().await;
    Ok(service.is_connected().await)
}

/// Get the current EventSub session ID
#[tauri::command]
pub async fn get_eventsub_session_id(
    state: State<'_, EventSubServiceState>,
) -> Result<Option<String>, String> {
    let service = state.0.read().await;
    Ok(service.get_session_id().await)
}

/// Add a live channel.moderate subscription for a channel the user moderates
/// (e.g. a MultiNook tile), so its mod view populates alongside the main stream.
/// Safe to call for any channel: if you don't moderate it, the subscription is
/// silently skipped (403). The channel is remembered and re-subscribed across
/// EventSub reconnects until remove_eventsub_moderation is called.
#[tauri::command]
pub async fn add_eventsub_moderation(
    broadcaster_id: String,
    state: State<'_, EventSubServiceState>,
) -> Result<(), String> {
    let service = state.0.read().await;
    service.add_mod_channel(broadcaster_id).await;
    Ok(())
}

/// Stop tracking a moderated channel and tear down its live moderation subscription.
#[tauri::command]
pub async fn remove_eventsub_moderation(
    broadcaster_id: String,
    state: State<'_, EventSubServiceState>,
) -> Result<(), String> {
    let service = state.0.read().await;
    service.remove_mod_channel(broadcaster_id).await;
    Ok(())
}
