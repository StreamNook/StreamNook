//! Twitch Shared Chat for the chat surfaces: who a partner's row came from.
//! (Which channels share the watched chat rides the channel state's collab
//! section, `services::channel_state`.)

use crate::services::shared_chat_source::{self, SharedSource};

/// The partner channel a Shared Chat row came from (`room_id`: the row's
/// `source-room-id`), its color floored for a dark or light theme. None while
/// Twitch cannot say.
#[tauri::command]
pub async fn get_shared_chat_source(room_id: String, dark: Option<bool>) -> Result<Option<SharedSource>, String> {
    Ok(shared_chat_source::source(&room_id, dark.unwrap_or(true)).await)
}
