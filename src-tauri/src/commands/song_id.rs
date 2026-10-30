//! Song recognition and Your songs. "/song" and the player's music button
//! decode the captured PCM and identify the track here; every match is filed
//! in the song history (services/song_id/history.rs), which the list's
//! commands below read and change. Rust decides the background work they
//! start: a cover's colors when a song is found, opened or listed without
//! them, and a link refresh when a song's links were cut short or are stale.

use crate::rt::AppHandle;
use crate::services::song_id::cover_tint::{self, Priority};
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
        let filer = app.clone();
        match blocking(move || history::record(&filer, &filed, source)).await {
            Ok(recorded) => {
                if let Some(cover) = recorded.cover {
                    cover_tint::enqueue(&app, vec![cover], Priority::High);
                }
                // The lookups the identification cut short finish now.
                if !song.links_complete {
                    history::queue_refresh(&app, recorded.key.clone());
                }
                song.key = Some(recorded.key);
            }
            Err(e) => log::warn!("[SongId] could not keep the song in Your songs: {e}"),
        }
    }
    Ok(found)
}

/// One page of Your songs, newest heard first: the songs matching every word
/// of `query` (title, artist, album or channel), from `offset`, grouped by
/// the page's time `bounds` (ms, newest first; groups are empty without
/// them). Covers on the page with no colors yet are fetched.
#[tauri::command]
pub async fn get_song_history(
    app: AppHandle,
    query: Option<String>,
    offset: Option<u32>,
    limit: Option<u32>,
    bounds: Option<Vec<i64>>,
) -> Result<history::SongPage, String> {
    let (page, covers) = blocking(move || {
        history::page(
            query.as_deref().unwrap_or(""),
            offset.unwrap_or(0) as usize,
            limit.unwrap_or(50) as usize,
            bounds,
        )
    })
    .await?;
    cover_tint::enqueue(&app, covers, Priority::Normal);
    Ok(page)
}

/// One song of Your songs, with its links, where it was heard and its
/// cover's colors. Opening it fetches those colors first when it has none,
/// and looks its links up again when they may have changed.
#[tauri::command]
pub async fn get_song(app: AppHandle, key: String) -> Result<Option<history::SongDetail>, String> {
    let asked = key.clone();
    let Some(opened) = blocking(move || history::get(&asked)).await? else {
        return Ok(None);
    };
    if let Some(cover) = opened.cover {
        cover_tint::enqueue(&app, vec![cover], Priority::High);
    }
    if opened.refresh {
        history::queue_refresh(&app, key);
    }
    Ok(Some(opened.detail))
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
