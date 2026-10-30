//! Discord Rich Presence commands. Presence itself runs in Rust
//! (`services::discord_presence`); the main window only reports its grid and
//! its player, and the settings editor asks for a preview.

use crate::models::settings::{AppState, DiscordPresenceSettings};
use crate::rt::Window;
use crate::services::discord_presence::{self, MediaFacts, PreviewModel, SampleChannel, SlotFacts, WriteResult};
use tauri::State;

/// The editor's card for one tab, built with the draft style.
#[tauri::command]
pub async fn discord_presence_preview(
    app: crate::rt::AppHandle,
    style: DiscordPresenceSettings,
    scene: String,
    state: State<'_, AppState>,
) -> Result<PreviewModel, String> {
    // Samples come from the viewer's own favorites (still favorited).
    let favorites: Vec<SampleChannel> = {
        let settings = state.settings.lock().map_err(|e| e.to_string())?;
        settings
            .favorite_channels
            .iter()
            .filter(|c| settings.favorite_streamers.contains(&c.id))
            .map(|c| SampleChannel {
                provider: c.provider.clone(),
                channel: c.channel.clone(),
                name: c.display_name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| c.channel.clone()),
                avatar: c.avatar.clone().unwrap_or_default(),
            })
            .collect()
    };
    Ok(discord_presence::preview(&app, &style, &scene, &favorites))
}

/// The main window's page generation, read when its feed starts and again
/// after a `stale` answer.
#[tauri::command]
pub async fn discord_presence_page() -> u64 {
    discord_presence::page()
}

/// MultiNook's tiles as the main window holds them (`null` when closed).
#[tauri::command]
pub async fn discord_presence_set_grid(window: Window, gen: u64, seq: u64, slots: Option<Vec<SlotFacts>>) -> WriteResult {
    discord_presence::set_grid(window.label(), gen, seq, slots)
}

/// The main window's player (`null` when it plays no VOD or clip).
#[tauri::command]
pub async fn discord_presence_set_media(window: Window, gen: u64, seq: u64, media: Option<MediaFacts>) -> WriteResult {
    discord_presence::set_media(window.label(), gen, seq, media)
}
