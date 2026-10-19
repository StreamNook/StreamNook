//! What the settings pages read that is not a setting: the community Discord
//! card, the follow list the live-alert sheet mutes channels from, and custom
//! sound files kept in step with their settings entries.

use crate::commands::settings::commit_settings_patch;
use crate::commands::settings_fields::{get_path, patch_for_changes};
use crate::models::settings::AppState;
use crate::models::stream::TwitchStream;
use crate::rt::AppHandle;
use crate::services::twitch_service::TwitchService;
use once_cell::sync::Lazy;
use serde_json::{json, Map, Value};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::State;

const INVITE_CODE: &str = "2xvuF9TES7";
/// The hosted header art, kept off the app bundle so it can change without a
/// release. The server's own banner is the fallback when it fails to load.
const HOSTED_BANNER: &str = "https://cdn.streamnook.app/community-banner.gif";
const INVITE_TTL: Duration = Duration::from_secs(60);

/// The community card: the server as Discord describes it, with its picture
/// URLs resolved (an `a_` hash is animated).
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct CommunityInvite {
    pub name: Option<String>,
    pub description: Option<String>,
    pub icon_url: Option<String>,
    /// The server icon to try in order: the animated one first when the hash
    /// is animated, then the same icon as a still. A page falls back to its own
    /// art when none loads. Empty when the server has no icon.
    pub icon_urls: Vec<String>,
    /// The hosted banner first, then the server's own.
    pub banner_urls: Vec<String>,
    pub online: Option<u64>,
    pub members: Option<u64>,
    pub invite_url: String,
    /// `discord.gg/<code>`, the line under the name.
    pub invite_label: String,
}

static INVITE: Lazy<Mutex<Option<(Instant, CommunityInvite)>>> = Lazy::new(|| Mutex::new(None));

fn discord_image(kind: &str, id: &str, hash: &str, size: u32) -> String {
    let ext = if hash.starts_with("a_") { "gif" } else { "png" };
    format!("https://cdn.discordapp.com/{kind}/{id}/{hash}.{ext}?size={size}")
}

/// Every URL a picture can load from, best first: an animated (`a_`) hash is
/// also served as a still, which loads where the animation does not.
fn discord_image_urls(kind: &str, id: &str, hash: &str, size: u32) -> Vec<String> {
    let mut urls = vec![discord_image(kind, id, hash, size)];
    if hash.starts_with("a_") {
        urls.push(format!("https://cdn.discordapp.com/{kind}/{id}/{hash}.png?size={size}"));
    }
    urls
}

fn invite_from(body: &Value) -> CommunityInvite {
    let guild = body.get("guild").cloned().unwrap_or(Value::Null);
    let text = |key: &str| guild.get(key).and_then(Value::as_str).map(str::to_string);
    let id = text("id").unwrap_or_default();
    let mut banner_urls = vec![HOSTED_BANNER.to_string()];
    if let Some(hash) = text("banner") {
        banner_urls.push(discord_image("banners", &id, &hash, 512));
    }
    CommunityInvite {
        name: text("name"),
        description: text("description").filter(|d| !d.trim().is_empty()),
        icon_url: text("icon").map(|hash| discord_image("icons", &id, &hash, 128)),
        icon_urls: text("icon").map(|hash| discord_image_urls("icons", &id, &hash, 128)).unwrap_or_default(),
        banner_urls,
        online: body.get("approximate_presence_count").and_then(Value::as_u64),
        members: body.get("approximate_member_count").and_then(Value::as_u64),
        invite_url: format!("https://discord.gg/{INVITE_CODE}"),
        invite_label: format!("discord.gg/{INVITE_CODE}"),
    }
}

/// The community Discord card, read at most once a minute. A failed read
/// answers the card without the server's details, so the page still shows it.
#[tauri::command]
pub async fn get_community_invite() -> Result<CommunityInvite, String> {
    if let Some((at, invite)) = INVITE.lock().map_err(|e| e.to_string())?.clone() {
        if at.elapsed() < INVITE_TTL {
            return Ok(invite);
        }
    }
    let url = format!("https://discord.com/api/v10/invites/{INVITE_CODE}?with_counts=true");
    let fetched = async {
        let response = reqwest::Client::new().get(&url).timeout(Duration::from_secs(10)).send().await.ok()?;
        if !response.status().is_success() {
            return None;
        }
        response.json::<Value>().await.ok()
    }
    .await;
    match fetched {
        Some(body) => {
            let invite = invite_from(&body);
            *INVITE.lock().map_err(|e| e.to_string())? = Some((Instant::now(), invite.clone()));
            Ok(invite)
        }
        None => Ok(invite_from(&Value::Null)),
    }
}

/// Every followed Twitch channel, for the live-alert sheet: sorted by name.
#[derive(Clone, Debug, serde::Serialize)]
pub struct AlertChannels {
    pub channels: Vec<TwitchStream>,
    /// The list stopped at `ALERT_PAGES` pages of follows.
    pub truncated: bool,
}

const ALERT_PAGE: u32 = 100;
const ALERT_PAGES: usize = 20;

#[tauri::command]
pub async fn get_followed_channels_for_alerts() -> Result<AlertChannels, String> {
    let mut channels: Vec<TwitchStream> = Vec::new();
    let mut cursor: Option<String> = None;
    let mut truncated = false;
    for page in 1..=ALERT_PAGES {
        let (batch, next) = TwitchService::get_all_followed_channels(ALERT_PAGE, cursor.take())
            .await
            .map_err(|e| e.to_string())?;
        let empty = batch.is_empty();
        channels.extend(batch);
        match next {
            Some(next) if !empty => {
                if page == ALERT_PAGES {
                    truncated = true;
                    break;
                }
                cursor = Some(next);
            }
            _ => break,
        }
    }
    channels.sort_by_key(|c| c.user_name.to_lowercase());
    Ok(AlertChannels { channels, truncated })
}

/// Copy a sound file into the app's sounds folder and add it to the custom
/// sounds list as one step, answering the stored entry.
#[tauri::command]
pub async fn add_custom_sound(app: AppHandle, path: String, state: State<'_, AppState>) -> Result<Value, String> {
    let imported = crate::commands::sounds::import_custom_sound(path)?;
    let entry = json!({ "id": format!("file:{}", imported.id), "name": imported.name, "path": imported.path });
    let added = entry.clone();
    commit_settings_patch(&app, &state, None, move |current| {
        let root = serde_json::to_value(current).map_err(|e| e.to_string())?;
        let mut list = get_path(&root, "chat_highlights.custom_sounds").and_then(Value::as_array).cloned().unwrap_or_default();
        list.push(added);
        let mut changes = Map::new();
        changes.insert("chat_highlights.custom_sounds".into(), Value::Array(list));
        patch_for_changes(current, &changes)
    })?;
    Ok(entry)
}

/// Take a sound out of the custom sounds list and delete its file (only a
/// file inside the sounds folder is ever deleted).
#[tauri::command]
pub async fn delete_custom_sound(app: AppHandle, id: String, state: State<'_, AppState>) -> Result<(), String> {
    let mut removed_path: Option<String> = None;
    commit_settings_patch(&app, &state, None, |current| {
        let root = serde_json::to_value(current).map_err(|e| e.to_string())?;
        let mut list = get_path(&root, "chat_highlights.custom_sounds").and_then(Value::as_array).cloned().unwrap_or_default();
        let Some(at) = list.iter().position(|s| s.get("id").and_then(Value::as_str) == Some(id.as_str())) else {
            return Ok(Map::new());
        };
        removed_path = list[at].get("path").and_then(Value::as_str).map(str::to_string);
        list.remove(at);
        let mut changes = Map::new();
        changes.insert("chat_highlights.custom_sounds".into(), Value::Array(list));
        patch_for_changes(current, &changes)
    })?;
    if let Some(path) = removed_path {
        crate::commands::sounds::remove_custom_sound(path)?;
    }
    Ok(())
}

/// Settings rows that answer a search, best first: the page, section and title
/// to show and the paths each writes.
#[tauri::command]
pub async fn search_settings(query: String) -> Result<Vec<crate::services::settings_catalog::CatalogRow>, String> {
    Ok(crate::services::settings_catalog::search(&query, 40))
}

/// The alert types this build sends, in the order the phones list them: each
/// with its setting, its OS category, and whether the setup wizard offers it.
#[tauri::command]
pub fn get_alert_types() -> Vec<crate::services::alert_types::AlertType> {
    crate::services::alert_types::alert_types()
}

/// The settings rows whose values differ from a fresh install, in page order,
/// for the "changed from default" review.
#[tauri::command]
pub async fn get_changed_settings(
    state: State<'_, AppState>,
) -> Result<Vec<crate::services::settings_catalog::CatalogRow>, String> {
    let current = {
        let settings = state.settings.lock().map_err(|e| e.to_string())?;
        serde_json::to_value(&*settings).map_err(|e| e.to_string())?
    };
    let changed = crate::commands::settings_fields::get_settings_view_from(current).changed;
    Ok(crate::services::settings_catalog::changed_rows(&changed))
}

/// The daily settings copies kept on this device, newest first.
#[tauri::command]
pub async fn list_settings_backups() -> Result<Vec<crate::services::settings_backups::BackupInfo>, String> {
    Ok(crate::services::settings_backups::list())
}

/// Put one day's copy back (every portable setting; accounts stay signed in).
#[tauri::command]
pub async fn restore_settings_backup(app: AppHandle, id: String, state: State<'_, AppState>) -> Result<(), String> {
    let backup = crate::services::settings_backups::read(&id)?;
    crate::commands::settings::apply_imported_settings(&app, &state, &backup)
}

fn theme_inputs(state: &AppState) -> Result<(String, Vec<Value>, Option<String>), String> {
    let settings = state.settings.lock().map_err(|e| e.to_string())?;
    let custom = settings.extra.get("custom_themes").and_then(Value::as_array).cloned().unwrap_or_default();
    let accent = settings.extra.get("oled_accent").and_then(Value::as_str).map(str::to_string);
    Ok((settings.theme.clone(), custom, accent))
}

/// The theme gallery: categories, built-in themes (OLED in the saved accent),
/// the viewer's custom themes and the OLED accent presets.
#[tauri::command]
pub async fn get_theme_catalog(state: State<'_, AppState>) -> Result<Value, String> {
    let (_, custom, accent) = theme_inputs(&state)?;
    Ok(crate::services::theme_palette::catalog(&custom, accent.as_deref()))
}

/// A palette as a shell is about to paint it (a built-in, a custom theme, one being edited, OLED
/// in a picked accent), with Rust's floors applied: muted and secondary text, and the accent on
/// cards (`accentOnSurface`). The desktop and Android paint the palette they hold and take these
/// floored keys from here, so every theme clears the same contrast the iPhone gets.
#[tauri::command]
pub fn lift_theme_palette(palette: Value) -> Value {
    let mut palette = palette;
    crate::services::theme_palette::lift_text(&mut palette);
    palette
}

/// The palette the saved theme paints with.
#[tauri::command]
pub async fn get_active_theme(
    state: State<'_, AppState>,
) -> Result<crate::services::theme_palette::ResolvedTheme, String> {
    let (id, custom, accent) = theme_inputs(&state)?;
    Ok(crate::services::theme_palette::resolve(&id, &custom, accent.as_deref()))
}

/// A custom theme as saving would store it, muted text lifted to the contrast
/// floor: the theme creator previews this, so its preview is the saved theme.
#[tauri::command]
pub async fn lift_custom_theme(theme: Value) -> Result<Value, String> {
    let mut list = Value::Array(vec![theme]);
    crate::services::theme_palette::lift_custom_themes(&mut list);
    list.as_array_mut().and_then(Vec::pop).ok_or_else(|| "no theme".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invite_card_resolves_pictures() {
        let card = invite_from(&json!({
            "guild": { "id": "1", "name": "StreamNook", "icon": "a_abc", "banner": "def", "description": " " },
            "approximate_presence_count": 12, "approximate_member_count": 340,
        }));
        assert_eq!(card.icon_url.as_deref(), Some("https://cdn.discordapp.com/icons/1/a_abc.gif?size=128"));
        assert_eq!(
            card.icon_urls,
            vec![
                "https://cdn.discordapp.com/icons/1/a_abc.gif?size=128".to_string(),
                "https://cdn.discordapp.com/icons/1/a_abc.png?size=128".to_string(),
            ]
        );
        assert_eq!(card.banner_urls[0], HOSTED_BANNER);
        assert_eq!(card.banner_urls[1], "https://cdn.discordapp.com/banners/1/def.png?size=512");
        assert_eq!(card.description, None);
        assert_eq!((card.online, card.members), (Some(12), Some(340)));
        let empty = invite_from(&Value::Null);
        assert_eq!(empty.name, None);
        assert_eq!(empty.banner_urls, vec![HOSTED_BANNER.to_string()]);
        assert!(empty.icon_urls.is_empty());
    }

    #[test]
    fn a_still_icon_has_one_url() {
        let card = invite_from(&json!({ "guild": { "id": "1", "icon": "abc" } }));
        assert_eq!(card.icon_urls, vec!["https://cdn.discordapp.com/icons/1/abc.png?size=128".to_string()]);
    }
}
