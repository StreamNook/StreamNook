//! Home snapshot commands: the data behind the Home grid and the Sidebar,
//! owned and kept warm by `services::home_snapshot`.

use crate::services::home_snapshot;

/// Everything Home renders, as Rust holds it right now. A mounting Home
/// paints from this without any network on its critical path.
#[tauri::command]
pub async fn get_home_snapshot() -> Result<home_snapshot::HomeSnapshot, String> {
    Ok(home_snapshot::snapshot().await)
}

/// A Home component mounted or unmounted. Drives the recommended poll and
/// the on-mount stale refresh.
#[tauri::command]
pub async fn set_home_mounted(mounted: bool) -> Result<(), String> {
    home_snapshot::set_home_mounted(mounted).await;
    Ok(())
}

/// Manual refresh of one section (`followed_live`, `offline`, `recommended`,
/// `hype_trains`, `watch_streaks`, `drops`), floored at 15 s per section. The result arrives as a
/// `home-snapshot` event like any other update.
#[tauri::command]
pub async fn refresh_home_section(
    section: String,
    languages: Option<Vec<String>>,
    personalized: Option<bool>,
) -> Result<(), String> {
    home_snapshot::refresh(&section, languages, personalized).await
}

/// Channel ids a Home has on screen beyond followed and recommended (category
/// grid, search results), so the hype-train poll covers them too.
#[tauri::command]
pub async fn set_home_extra_channels(channel_ids: Vec<String>) -> Result<(), String> {
    home_snapshot::set_extra_channels(channel_ids).await;
    Ok(())
}

/// Append the next recommended page. The result arrives as a `home-snapshot`
/// `recommended` update carrying the whole list and the new cursor.
#[tauri::command]
pub async fn load_more_home_recommended() -> Result<(), String> {
    home_snapshot::load_more_recommended().await
}
