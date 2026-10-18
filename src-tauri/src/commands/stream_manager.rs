//! Tauri boundary for the stream manager. Thin adapters; the rules, the view
//! and every Twitch call live in services::stream_manager.

use crate::models::settings::AppState;
use crate::rt::AppHandle;
use crate::services::stream_manager::{self, ActOutcome, Action, InfoCheck, QueueView, RaidTarget, StreamInfoDraft, StreamManagerView};
use tauri::{Emitter, State};

/// Everything the manager shows for a Twitch channel, for the signed-in
/// account's role there.
#[tauri::command]
pub async fn get_stream_manager(
    state: State<'_, AppState>,
    channel_id: String,
    channel_login: String,
) -> Result<StreamManagerView, String> {
    stream_manager::view(&state.twitch_auth, &channel_id, &channel_login).await
}

/// The stream info form's problems, counts and preview tile for a draft.
#[tauri::command]
pub async fn check_stream_info(channel_id: String, draft: StreamInfoDraft) -> Result<InfoCheck, String> {
    Ok(stream_manager::check(&channel_id, &draft).await)
}

/// Run one stream manager action. The answer carries the fresh view; every
/// other window showing this channel's manager gets it as
/// `stream-manager-state`.
#[tauri::command]
pub async fn stream_manager_act(
    app: AppHandle,
    state: State<'_, AppState>,
    channel_id: String,
    channel_login: String,
    action: Action,
) -> Result<ActOutcome, String> {
    let outcome = stream_manager::act(&state.twitch_auth, &channel_id, &channel_login, action).await;
    if let Some(view) = &outcome.view {
        let _ = app.emit(
            "stream-manager-state",
            serde_json::json!({ "channel_id": channel_id, "view": view, "queue": outcome.queue }),
        );
    }
    Ok(outcome)
}

/// The channel points redemptions waiting in a channel, grouped by reward.
#[tauri::command]
pub async fn get_reward_queue(state: State<'_, AppState>, channel_id: String) -> Result<QueueView, String> {
    stream_manager::read_queue(&state.twitch_auth, &channel_id).await
}

/// Channels to raid for a search, live ones first.
#[tauri::command]
pub async fn search_raid_targets(
    state: State<'_, AppState>,
    query: String,
    channel_id: String,
) -> Result<Vec<RaidTarget>, String> {
    stream_manager::search_raid_targets(&state, &query, &channel_id).await
}
