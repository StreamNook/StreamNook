//! The old whisper commands, answering from the new store until each one's
//! last caller is gone (`whisper_inbox::legacy`).

use crate::services::whisper_inbox;

/// One conversation with all its messages, oldest first.
#[tauri::command]
pub async fn whisper_thread(owner_id: String, key: String) -> Result<Option<serde_json::Value>, String> {
    let _ = owner_id;
    whisper_inbox::legacy::thread(&key)
}
