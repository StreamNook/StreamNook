use crate::services::emote_prefetch_service::{EmotePrefetchService, PrefetchProgress};
use std::sync::Arc;
use crate::rt::AppHandle;
use tauri::State;

/// Managed state wrapper for the AFK emote prefetch service.
pub struct EmotePrefetchServiceState(pub Arc<EmotePrefetchService>);

/// Phase 1: scan all followed channels and compute how much there is to
/// download. Kicks off a background scan and returns the initial snapshot;
/// live counts arrive via the `emote-prefetch-progress` event. `tier` is the
/// frontend's per-DPI emote tier ("1x" | "2x" | "4x") so prefetched 7TV files
/// match the keys the picker actually looks up.
#[tauri::command]
pub async fn emote_prefetch_plan(
    tier: String,
    state: State<'_, EmotePrefetchServiceState>,
    app_handle: AppHandle,
) -> Result<PrefetchProgress, String> {
    state.0.plan(app_handle, tier).await;
    Ok(state.0.get_progress().await)
}

/// Phase 2: download the planned to-download list in the background. Returns
/// immediately; progress arrives via `emote-prefetch-progress` and a final
/// `emote-prefetch-complete` event.
#[tauri::command]
pub async fn emote_prefetch_start(
    state: State<'_, EmotePrefetchServiceState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    state.0.start(app_handle).await;
    Ok(())
}

/// Cancel the current scan or download.
#[tauri::command]
pub async fn emote_prefetch_stop(
    state: State<'_, EmotePrefetchServiceState>,
) -> Result<(), String> {
    state.0.stop().await;
    Ok(())
}

/// Current progress snapshot (so a reopened panel re-syncs to a running job).
#[tauri::command]
pub async fn emote_prefetch_status(
    state: State<'_, EmotePrefetchServiceState>,
) -> Result<PrefetchProgress, String> {
    Ok(state.0.get_progress().await)
}
