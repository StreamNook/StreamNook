use crate::services::eventsub_service::EventSubService;
use std::sync::Arc;
use tauri::State;
use tokio::sync::RwLock;

/// The watch EventSub socket. Only services/watch_session.rs aims it (the
/// watched channel, or MultiNook's focused tile); these commands only read it.
pub struct EventSubServiceState(pub Arc<RwLock<EventSubService>>);

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
