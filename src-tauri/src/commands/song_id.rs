//! Song recognition and Your songs. "/song" and the player's music button
//! decode the captured PCM and identify the track here; every match is filed
//! in the song history (services/song_id/history.rs), which the list's
//! commands below read and change.

use crate::rt::AppHandle;
use crate::services::song_id::{self, history, SongMatch};
use base64::{engine::general_purpose, Engine};

/// Run the history's file work off the async runtime.
async fn blocking<R: Send + 'static>(
    work: impl FnOnce() -> Result<R, String> + Send + 'static,
) -> Result<R, String> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| format!("song history task failed: {e}"))?
}

/// Identify the song from a base64-encoded little-endian i16 mono 16 kHz PCM
/// buffer (captured frontend-side from the active player). `source` is the
/// channel that player shows, kept with the song in Your songs. Returns `None`
/// when no match is found; a match comes back with its `key` there.
#[tauri::command]
pub async fn identify_song(
    app: AppHandle,
    audio_b64: String,
    source: Option<serde_json::Value>,
) -> Result<Option<SongMatch>, String> {
    let bytes = general_purpose::STANDARD
        .decode(audio_b64.as_bytes())
        .map_err(|e| format!("bad audio payload: {}", e))?;

    if bytes.len() % 2 != 0 {
        return Err("audio payload has an odd byte length".to_string());
    }

    // Where it was heard is a note on the song: a source that does not read
    // never costs the identification.
    let source = source.and_then(|v| serde_json::from_value::<history::SongSource>(v).ok());

    // Reinterpret the byte stream as little-endian i16 samples.
    let samples: Vec<i16> = bytes
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]))
        .collect();

    // A match with no title names nothing: it is no match, here and on the page.
    let mut found = song_id::identify(samples)
        .await?
        .filter(|s| !s.title.trim().is_empty());
    if let Some(song) = found.as_mut() {
        // Filed here, where the match is made, so every surface that
        // identifies a song keeps it without asking.
        let filed = song.clone();
        match blocking(move || history::record(&app, &filed, source)).await {
            Ok(key) => song.key = Some(key),
            Err(e) => log::warn!("[SongId] could not keep the song in Your songs: {e}"),
        }
    }
    Ok(found)
}

/// One page of Your songs, newest heard first: the songs matching every word
/// of `query` (title, artist, album or channel), from `offset`.
#[tauri::command]
pub async fn get_song_history(
    query: Option<String>,
    offset: Option<u32>,
    limit: Option<u32>,
) -> Result<history::SongPage, String> {
    blocking(move || {
        history::page(
            query.as_deref().unwrap_or(""),
            offset.unwrap_or(0) as usize,
            limit.unwrap_or(50) as usize,
        )
    })
    .await
}

/// One song of Your songs, with its links and where it was heard.
#[tauri::command]
pub async fn get_song(key: String) -> Result<Option<history::SongEntry>, String> {
    blocking(move || history::get(&key)).await
}

/// Take one song out of Your songs, kept for an Undo. False when it was not
/// there.
#[tauri::command]
pub async fn remove_song(app: AppHandle, key: String) -> Result<bool, String> {
    blocking(move || history::remove(&app, &key)).await
}

/// Undo the last Remove, when `key` names its song.
#[tauri::command]
pub async fn restore_song(app: AppHandle, key: String) -> Result<bool, String> {
    blocking(move || history::restore(&app, &key)).await
}

/// Empty Your songs.
#[tauri::command]
pub async fn clear_song_history(app: AppHandle) -> Result<(), String> {
    blocking(move || history::clear(&app)).await
}
