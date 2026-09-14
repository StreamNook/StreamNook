use crate::services::emote_service::{Emote, EmoteService, EmoteSet};
use std::sync::Arc;
use tauri::State;
use tokio::sync::RwLock;

pub struct EmoteServiceState(pub Arc<RwLock<EmoteService>>);

#[tauri::command]
pub async fn fetch_channel_emotes(
    channel_name: Option<String>,
    channel_id: Option<String>,
    access_token: Option<String>,
    // Which platform `channel_id` belongs to. Absent = twitch, so callers that
    // predate multi-platform are unchanged.
    provider: Option<String>,
    state: State<'_, EmoteServiceState>,
) -> Result<EmoteSet, String> {
    let service = state.0.read().await;
    service
        .fetch_channel_emotes(channel_name, channel_id, access_token, provider)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_emote_by_name(
    channel_id: Option<String>,
    emote_name: String,
    state: State<'_, EmoteServiceState>,
) -> Result<Option<Emote>, String> {
    let service = state.0.read().await;
    Ok(service.get_emote_by_name(channel_id, &emote_name).await)
}

#[tauri::command]
pub async fn clear_emote_cache(state: State<'_, EmoteServiceState>) -> Result<(), String> {
    let service = state.0.read().await;
    service.clear_cache().await;
    Ok(())
}
