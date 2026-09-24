//! Home snapshot commands: the data behind the Home grid and the Sidebar,
//! owned and kept warm by `services::home_snapshot`.

use crate::services::home_snapshot;

/// Everything Home renders, as Rust holds it right now. A mounting Home
/// paints from this without any network on its critical path.
#[tauri::command]
pub async fn get_home_snapshot() -> Result<home_snapshot::HomeSnapshot, String> {
    Ok(home_snapshot::snapshot().await)
}

/// A Home component mounted or unmounted in the calling window, and whether it
/// shows every platform at once (`unified`). Drives the recommended poll and
/// the on-mount stale refresh, and on the unified view the Discover list, which
/// Rust builds and fetches only while such a Home is on screen. A Home that
/// switches view re-announces itself: unmounted as the old view, mounted as the
/// new one. `context` names the page (one id per JS context), which is how the
/// claims of a page that reloaded without unmounting are told from live ones.
#[tauri::command]
pub async fn set_home_mounted(
    window: tauri::Window,
    mounted: bool,
    unified: Option<bool>,
    context: Option<String>,
) -> Result<(), String> {
    home_snapshot::set_home_mounted(
        window.label(),
        context.as_deref().unwrap_or_default(),
        mounted,
        unified.unwrap_or(false),
    )
    .await;
    Ok(())
}

/// The Sidebar in the calling window shows its second section for `scope`
/// (`"all"`, or one provider id), or shows none (`None`: the sidebar is
/// disabled, or the section is switched off). Rust builds that list
/// (`services::unified_discover`) and sends it as a `home-snapshot`
/// `sidebar_discover` update. `context` names the page, as for
/// `set_home_mounted`.
#[tauri::command]
pub async fn set_home_sidebar(
    window: tauri::Window,
    scope: Option<String>,
    context: Option<String>,
) -> Result<(), String> {
    home_snapshot::set_sidebar(
        window.label(),
        context.as_deref().unwrap_or_default(),
        scope.as_deref(),
    )
    .await;
    Ok(())
}

/// Manual refresh of one section (`followed_live`, `offline`, `recommended`,
/// `hype_trains`, `watch_streaks`, `drops`, `continue_watching`, and
/// `discover` for the directories the Discover lists on screen read), floored
/// at 15 s per section. The result arrives as a `home-snapshot` event like any
/// other update.
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
