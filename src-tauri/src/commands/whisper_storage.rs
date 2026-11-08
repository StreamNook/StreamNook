//! The old whisper commands, answering from the new store until each one's
//! last caller is gone (`whisper_inbox::legacy`).

use crate::services::whisper_inbox;

/// Every conversation with its messages (the desktop Whispers window until it
/// moves to `whisper_list` and `whisper_page`). `owner_id` is ignored: the
/// owner is the followed account.
#[tauri::command]
pub async fn load_whisper_storage(owner_id: String) -> Result<serde_json::Value, String> {
    let _ = owner_id;
    whisper_inbox::legacy::load_storage()
}

/// "Delete" a conversation: archived here and on Twitch, so the next sync
/// does not bring it back.
#[tauri::command]
pub async fn delete_whisper_conversation(owner_id: String, user_id: String) -> Result<(), String> {
    let _ = owner_id;
    whisper_inbox::archive(&user_id).await
}

/// Where the whisper store lives (for debugging).
#[tauri::command]
pub async fn get_whisper_storage_path(owner_id: String) -> Result<String, String> {
    let _ = owner_id;
    whisper_inbox::legacy::storage_path()
}

/// The old browser-storage migration: nothing is left to migrate.
#[tauri::command]
pub async fn migrate_whispers_from_localstorage(
    owner_id: String,
    conversations: serde_json::Value,
) -> Result<(), String> {
    let _ = (owner_id, conversations);
    Ok(())
}

/// Files are not imported any more.
#[tauri::command]
pub async fn whisper_import(owner_id: String, export: serde_json::Value) -> Result<serde_json::Value, String> {
    let _ = (owner_id, export);
    whisper_inbox::legacy::import()
}

/// One conversation with all its messages, oldest first.
#[tauri::command]
pub async fn whisper_thread(owner_id: String, key: String) -> Result<Option<serde_json::Value>, String> {
    let _ = owner_id;
    whisper_inbox::legacy::thread(&key)
}

/// A sync; answers how many messages it added.
#[tauri::command]
pub async fn whisper_refresh(owner_id: String) -> Result<usize, String> {
    let _ = owner_id;
    whisper_inbox::legacy::refresh().await
}
