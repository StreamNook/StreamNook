//! Tauri boundary for the stream manager. Thin adapters; the rules, the view
//! and every Twitch call live in services::stream_manager.

use crate::models::settings::AppState;
use crate::rt::AppHandle;
use crate::services::stream_manager::{self, ActOutcome, Action, InfoCheck, QueueView, RaidTarget, StreamInfoDraft, StreamManagerView};
use tauri::{Emitter, State};

/// Carry the poll recents settings kept over to Rust's own file, once per run.
fn seed_recents(state: &AppState) {
    static SEEDED: std::sync::Once = std::sync::Once::new();
    SEEDED.call_once(|| {
        // Cloned first: the recents lock is never taken while settings are held.
        let extra = state.settings.lock().map(|s| s.extra.clone()).unwrap_or_default();
        stream_manager::seed_poll_recents(&extra);
    });
}

/// Everything the manager shows for a Twitch channel, for the signed-in
/// account's role there.
#[tauri::command]
pub async fn get_stream_manager(
    state: State<'_, AppState>,
    channel_id: String,
    channel_login: String,
) -> Result<StreamManagerView, String> {
    seed_recents(&state);
    stream_manager::view(&state.twitch_auth, &channel_id, &channel_login).await
}

/// The stream info form's problems, counts and preview tile for a draft.
#[tauri::command]
pub async fn check_stream_info(channel_id: String, draft: StreamInfoDraft) -> Result<InfoCheck, String> {
    Ok(stream_manager::check(&channel_id, &draft).await)
}

/// Run one stream manager action. The answer carries the fresh view; every
/// other window showing this channel's manager gets it as
/// `stream-manager-state`. With `expected_user_id`, an action refuses when
/// another Twitch account is signed in by now.
#[tauri::command]
pub async fn stream_manager_act(
    app: AppHandle,
    state: State<'_, AppState>,
    channel_id: String,
    channel_login: String,
    action: Action,
    expected_user_id: Option<String>,
) -> Result<ActOutcome, String> {
    seed_recents(&state);
    let outcome =
        stream_manager::act(&state.twitch_auth, &channel_id, &channel_login, action, expected_user_id.as_deref()).await;
    if let Some(view) = &outcome.view {
        let _ = app.emit(
            "stream-manager-state",
            serde_json::json!({ "channel_id": channel_id, "view": view, "queue": outcome.queue }),
        );
    }
    Ok(outcome)
}

/// A window's manager surface (panel, builder, window, Tools menu, phone
/// sheet) opened on a channel (`on`) or closed. While any surface shows a
/// channel, Rust reads it again by itself and sends `stream-manager-state`.
/// `seq` only goes up within a page; an older call for the same surface is
/// ignored.
#[tauri::command]
pub async fn stream_manager_watch(
    window: crate::rt::Window,
    surface: String,
    channel_id: String,
    channel_login: String,
    on: bool,
    seq: u64,
) -> Result<(), String> {
    let channel = (on && !channel_id.is_empty()).then(|| (channel_id, channel_login));
    stream_manager::watch(window.label(), &surface, channel, seq);
    Ok(())
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
