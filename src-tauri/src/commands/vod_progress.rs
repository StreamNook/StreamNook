//! VOD watch-position commands. Thin: validate intent, hand to
//! `vod_progress_service`, return a compact result.

use crate::services::vod_progress_service::{self, VodMeta, VodProgress, VodProgressSummary};

/// The player's playhead checkpoint for the VOD it is playing. Sent at most
/// every few seconds plus on pause / seek / end / unmount; the service owns
/// the resume policy and persistence.
#[tauri::command]
pub async fn report_vod_position(
    video_id: String,
    position_secs: f64,
    duration_secs: Option<f64>,
    channel_login: Option<String>,
    title: Option<String>,
    thumbnail_url: Option<String>,
) -> Result<VodProgressSummary, String> {
    let meta = VodMeta {
        channel_login,
        title,
        thumbnail_url,
    };
    tokio::task::spawn_blocking(move || {
        vod_progress_service::record(
            &video_id,
            position_secs,
            duration_secs.unwrap_or(0.0),
            meta,
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Stored progress for a set of VODs (cards that were not fetched through a
/// list command, e.g. a link preview).
#[tauri::command]
pub async fn get_vod_progress(video_ids: Vec<String>) -> Result<Vec<VodProgress>, String> {
    Ok(tokio::task::spawn_blocking(move || vod_progress_service::lookup_many(&video_ids))
        .await
        .map_err(|e| e.to_string())?)
}

/// Forget one VOD's position ("start over").
#[tauri::command]
pub async fn clear_vod_progress(video_id: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || vod_progress_service::clear(&video_id))
        .await
        .map_err(|e| e.to_string())?
}
