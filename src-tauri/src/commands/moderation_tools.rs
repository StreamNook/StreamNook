//! Tauri boundary for the AutoMod queue and streamer mode. Thin adapters;
//! state and decisions live in services::automod_queue / streamer_mode.
//! Each command has its `generate_handler!` entry and its ACL line.

use crate::services::automod_queue::{AutomodQueue, HeldMessage};
use crate::services::streamer_mode::{StreamerMode, StreamerModeState};
use crate::services::twitch_service::TwitchService;

/// Messages AutoMod is currently holding for a channel (oldest first).
#[tauri::command]
pub async fn get_automod_queue(channel: String) -> Result<Vec<HeldMessage>, String> {
    Ok(AutomodQueue::list(&channel))
}

/// Allow or deny a held message. The queue row leaves on the EventSub
/// `automod.message.update` that follows.
#[tauri::command]
pub async fn resolve_automod_message(message_id: String, allow: bool) -> Result<(), String> {
    TwitchService::resolve_automod_message(&message_id, allow)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_streamer_mode_state() -> Result<StreamerModeState, String> {
    Ok(StreamerMode::state())
}

/// Ban, time out, lift, or delete on any platform. The service picks the
/// platform API, applies its timeout rules and words the result.
#[tauri::command]
pub async fn moderate_chatter(
    request: crate::services::moderation::ModRequest,
) -> Result<crate::services::moderation::ModOutcome, String> {
    crate::services::moderation::moderate(request).await.map_err(|e| e.to_string())
}

/// The timeout steps the dock and the row slider offer on `provider`, as that
/// platform will apply them, ending in Ban. `presets` is the list the page is
/// showing (it can be a moment ahead of the saved copy while a settings edit
/// is in flight); without it the saved list is used.
#[tauri::command]
pub fn get_mod_timeout_steps(
    provider: Option<String>,
    presets: Option<Vec<u32>>,
) -> Vec<crate::services::moderation::ModStep> {
    crate::services::moderation::steps_for(provider.as_deref(), presets)
}

/// The row slider's steps on `provider`: Delete, the timeouts that platform
/// applies, Ban, each as the viewer turned them on. `config` is the page's
/// copy of those settings; without it the saved one is used.
#[tauri::command]
pub fn get_mod_slider_steps(
    provider: Option<String>,
    config: Option<crate::services::moderation::SliderConfig>,
) -> Vec<crate::services::moderation::ModStep> {
    crate::services::moderation::slider_steps_for(provider.as_deref(), config)
}

/// A timeout length typed into Settings ("45m", "1h30m", "2d", a bare number
/// of minutes), as the step it adds: its seconds and label. Err says how to
/// write one.
#[tauri::command]
pub fn parse_mod_timeout(text: String) -> Result<crate::services::moderation::ModStep, String> {
    crate::services::moderation::typed_timeout_step(&text)
}

/// `/settitle` and `/setgame`: broadcaster-only Helix channel update.
#[tauri::command]
pub async fn update_channel_info(
    broadcaster_id: String,
    title: Option<String>,
    game_name: Option<String>,
) -> Result<(), String> {
    TwitchService::update_channel_info(&broadcaster_id, title.as_deref(), game_name.as_deref())
        .await
        .map_err(|e| e.to_string())
}

/// Pronouns from pronouns.alejo.io (opt-in via user_card.show_pronouns).
#[tauri::command]
pub async fn get_user_pronouns(login: String) -> Result<Option<String>, String> {
    Ok(crate::services::pronouns::Pronouns::for_login(&login).await)
}

/// Private note on a user (keyed by Twitch user id, persisted in app data).
#[tauri::command]
pub async fn get_user_note(user_id: String) -> Result<Option<crate::services::user_notes::UserNote>, String> {
    Ok(crate::services::user_notes::UserNotes::get(&user_id))
}

#[tauri::command]
pub async fn set_user_note(
    user_id: String,
    note: String,
) -> Result<Option<crate::services::user_notes::UserNote>, String> {
    crate::services::user_notes::UserNotes::set(&user_id, &note)
}
