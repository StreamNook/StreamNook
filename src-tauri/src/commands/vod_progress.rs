//! VOD watch-position commands. Thin: validate intent, hand to
//! `vod_progress_service`, return a compact result.

use crate::services::live_bookmark::{self, Origin};
use crate::services::vod_progress_service::{self, VodMeta, VodProgress, VodProgressSummary};

/// The answer to a report that wrote nothing: the playhead, echoed.
fn unrecorded(position_secs: f64, duration_secs: Option<f64>) -> VodProgressSummary {
    VodProgressSummary {
        position_secs,
        duration_secs: duration_secs.unwrap_or(0.0),
        completed: false,
    }
}

/// The player's playhead checkpoint for the VOD it is playing. Sent at most
/// every few seconds plus on pause / seek / end / unmount; the service owns
/// the resume policy and persistence.
///
/// `play_id` is the stamp Rust put on that VOD's start. Only a VOD the viewer
/// opened writes the store (and so the Continue Watching row and the Twitch
/// sync); a live rewind only moves the live session's bookmark, in memory; an
/// id this run never issued writes nothing at all.
#[tauri::command]
pub async fn report_vod_position(
    video_id: String,
    play_id: Option<u64>,
    position_secs: f64,
    duration_secs: Option<f64>,
    channel_login: Option<String>,
    channel_name: Option<String>,
    title: Option<String>,
    thumbnail_url: Option<String>,
) -> Result<VodProgressSummary, String> {
    match live_bookmark::origin_of(play_id) {
        Some(Origin::Deliberate) => {}
        Some(Origin::Rewind) => {
            live_bookmark::note_rewind_position(&video_id, position_secs);
            return Ok(unrecorded(position_secs, duration_secs));
        }
        None => {
            log::debug!("[VodProgress] report for {video_id} with unknown play id {play_id:?}: not recorded");
            return Ok(unrecorded(position_secs, duration_secs));
        }
    }
    let meta = VodMeta {
        channel_login,
        channel_name,
        title,
        thumbnail_url,
    };
    let id = video_id.clone();
    let summary = tokio::task::spawn_blocking(move || {
        vod_progress_service::record(&id, position_secs, duration_secs.unwrap_or(0.0), meta)
    })
    .await
    .map_err(|e| e.to_string())??;
    // Queue the save to the viewer's Twitch history. Only this command feeds
    // it, and only for a deliberate VOD (checked above).
    crate::services::vod_sync::note_recorded(
        &video_id,
        position_secs,
        chrono::Utc::now().timestamp(),
    );
    // Home's Continue Watching row is derived from this store. The notify is a
    // no-op unless a Home is actually on screen, and coalesces on a trailing
    // edge, so the 5 s checkpoint cadence costs nothing here.
    crate::services::home_snapshot::note_progress_changed();
    Ok(summary)
}

/// Stored progress for a set of VODs (cards that were not fetched through a
/// list command, e.g. a link preview).
#[tauri::command]
pub async fn get_vod_progress(video_ids: Vec<String>) -> Result<Vec<VodProgress>, String> {
    Ok(tokio::task::spawn_blocking(move || vod_progress_service::lookup_many(&video_ids))
        .await
        .map_err(|e| e.to_string())?)
}

/// Forget one VOD's position ("start over", or dismissing a Continue Watching
/// card).
#[tauri::command]
pub async fn clear_vod_progress(video_id: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || vod_progress_service::clear(&video_id))
        .await
        .map_err(|e| e.to_string())??;
    crate::services::home_snapshot::note_progress_changed();
    // The card leaves the viewer's other devices too.
    crate::services::vod_sync::note_dismissed();
    Ok(())
}
