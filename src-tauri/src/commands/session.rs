use crate::services::cache_service;
use serde::{Deserialize, Serialize};
use std::fs;

/// A one-shot snapshot of what the user was doing, written just before an
/// update restart and consumed once on the next launch so the app can put them
/// back where they left off. Machine-local; never part of a settings backup.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ResumeSnapshot {
    /// Login of the live channel that was playing (the thing to reopen).
    pub stream_login: Option<String>,
    /// 'live' | 'clip' | 'video' | etc. Only live streams are restored today.
    pub media_type: Option<String>,
    pub original_media_url: Option<String>,
    /// Whether the opt-in drops/points automation was running.
    pub was_running: bool,
    /// Campaign being collected, if a specific one (vs auto-collect).
    pub automation_campaign_id: Option<String>,
    /// The viewer had paused the live stream: it reopens paused.
    #[serde(default)]
    pub paused: bool,
    /// MultiNook was open: it reopens with its saved tiles.
    #[serde(default)]
    pub multinook_active: bool,
}

fn resume_snapshot_path() -> Result<std::path::PathBuf, String> {
    let app_dir = cache_service::get_app_data_dir()
        .map_err(|e| format!("Failed to get app data directory: {}", e))?;
    Ok(app_dir.join("resume_snapshot.json"))
}

/// Persist a resume snapshot (called right before the app restarts to update).
#[tauri::command]
pub fn save_resume_snapshot(snapshot: ResumeSnapshot) -> Result<(), String> {
    let path = resume_snapshot_path()?;
    let json = serde_json::to_string_pretty(&snapshot)
        .map_err(|e| format!("Failed to serialize resume snapshot: {}", e))?;
    fs::write(&path, json).map_err(|e| format!("Failed to write resume snapshot: {}", e))?;
    Ok(())
}

/// Read and delete the resume snapshot. Consume-once: deleting on read means a
/// failed restore can't loop on every subsequent launch. Returns None if absent.
#[tauri::command]
pub fn take_resume_snapshot() -> Result<Option<ResumeSnapshot>, String> {
    let path = resume_snapshot_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let data =
        fs::read_to_string(&path).map_err(|e| format!("Failed to read resume snapshot: {}", e))?;
    let _ = fs::remove_file(&path);
    let snapshot = serde_json::from_str::<ResumeSnapshot>(&data)
        .map_err(|e| format!("Failed to parse resume snapshot: {}", e))?;
    Ok(Some(snapshot))
}

/// Relaunch the app. The page saves a resume snapshot first, so the stream it
/// was on reopens. Used when WebView2's GPU process came back without the GPU.
#[tauri::command]
pub fn restart_app(app: crate::rt::AppHandle, reason: String) {
    crate::services::app_exit::request_restart(&app, &format!("restart asked by the page ({reason})"));
}
