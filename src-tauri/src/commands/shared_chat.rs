//! Twitch Shared Chat for the chat surfaces: who a partner's row came from,
//! and which channels share the chat being watched.

use crate::services::shared_chat::{self, SharedChat};
use crate::services::shared_chat_source::{self, SharedSource};

/// The partner channel a Shared Chat row came from (`room_id`: the row's
/// `source-room-id`), its color floored for a dark or light theme. None while
/// Twitch cannot say.
#[tauri::command]
pub async fn get_shared_chat_source(room_id: String, dark: Option<bool>) -> Result<Option<SharedSource>, String> {
    Ok(shared_chat_source::source(&room_id, dark.unwrap_or(true)).await)
}

/// The Shared Chat session of the channel being watched: the channel first,
/// then its live partners. None when it shares with no one (or signed out).
#[tauri::command]
pub async fn get_shared_chat_partners(channel_id: String) -> Result<Option<SharedChat>, String> {
    if channel_id.is_empty() {
        return Ok(None);
    }
    Ok(shared_chat::fetch(std::slice::from_ref(&channel_id)).await.remove(&channel_id).flatten())
}
