//! What the settings pages read that is not a setting: the community Discord
//! card, the follow list the live-alert sheet mutes channels from, and custom
//! sound files kept in step with their settings entries.

use crate::commands::settings::commit_settings_patch;
use crate::commands::settings_fields::{get_path, patch_for_changes};
use crate::models::settings::{AppState, FavoriteChannel, ProviderFollow};
use crate::models::stream::TwitchStream;
use crate::rt::AppHandle;
use crate::services::account_store::AccountStore;
use crate::services::live_mute::{self, CopyAnswer};
use crate::services::providers::key::is_youtube_channel_id;
use crate::services::twitch_service::TwitchService;
use once_cell::sync::Lazy;
use serde_json::{json, Map, Value};
use std::collections::HashSet;
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

/// One channel in the live-alert sheet: the Twitch row the shells already
/// decode, plus its platform and whether its go-lives alert here.
#[derive(Clone, Debug, serde::Serialize)]
pub struct AlertChannelRow {
    #[serde(flatten)]
    pub stream: TwitchStream,
    pub provider: String,
    pub on: bool,
}

/// The channels the live-alert sheet lists: newest follow first on a phone,
/// by name on desktop.
#[derive(Clone, Debug, serde::Serialize)]
pub struct AlertChannels {
    pub channels: Vec<AlertChannelRow>,
    /// The Twitch list stopped at `ALERT_PAGES` pages of follows.
    pub truncated: bool,
    /// Listed channels whose alerts are off (the "except N" number); `None`
    /// when the Twitch follows could not be read.
    pub muted_count: Option<usize>,
    /// The last Copy from Twitch on this account within 10 minutes, so a
    /// reopened sheet shows its result line and Undo again.
    pub last_copy: Option<CopyAnswer>,
}

const ALERT_PAGE: u32 = 100;
const ALERT_PAGES: usize = 20;
/// How long the sheet's Twitch follow list is reused. Settings screens ask on
/// open and after every mute; only a cold or cleared cache pages Helix.
const ALERT_FOLLOWS_TTL: Duration = Duration::from_secs(10 * 60);

/// The Twitch follows last read for the sheet: (account, when, rows, truncated).
type FollowCache = Option<(String, Instant, Vec<TwitchStream>, bool)>;

static ALERT_FOLLOWS: Lazy<Mutex<FollowCache>> = Lazy::new(|| Mutex::new(None));
/// Held for a cold read, so two sheets opening at once share one fetch.
static ALERT_FOLLOWS_FETCH: Lazy<tokio::sync::Mutex<()>> = Lazy::new(|| tokio::sync::Mutex::new(()));

/// Forget the cached follow list: a follow or unfollow made here changes it.
pub fn clear_alert_follows_cache() {
    if let Ok(mut cache) = ALERT_FOLLOWS.lock() {
        *cache = None;
    }
}

fn cached_follows(owner: &str) -> Option<(Vec<TwitchStream>, bool)> {
    let cache = ALERT_FOLLOWS.lock().ok()?;
    let (who, at, rows, truncated) = cache.as_ref()?;
    (who == owner && at.elapsed() < ALERT_FOLLOWS_TTL).then(|| (rows.clone(), *truncated))
}

/// Every followed Twitch channel (up to `ALERT_PAGES` pages), from the cache
/// when it is warm for this account.
async fn twitch_follows_for_alerts() -> Result<(Vec<TwitchStream>, bool), String> {
    // The generation moves on every sign-in, switch and sign-out, so a cache
    // filled for one account is never served to the next.
    let owner = format!(
        "{}:{}",
        TwitchService::account_generation(),
        AccountStore::primary().map(|a| a.user_id).unwrap_or_default()
    );
    if let Some(hit) = cached_follows(&owner) {
        return Ok(hit);
    }
    let _fetch = ALERT_FOLLOWS_FETCH.lock().await;
    // Another sheet may have filled it while this one waited.
    if let Some(hit) = cached_follows(&owner) {
        return Ok(hit);
    }
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
    if let Ok(mut cache) = ALERT_FOLLOWS.lock() {
        *cache = Some((owner, Instant::now(), channels.clone(), truncated));
    }
    Ok((channels, truncated))
}

/// A channel known only from settings (another platform's follow, a
/// favourite) in the row shape the sheet draws. The platform rides on
/// `AlertChannelRow.provider`.
fn settings_row(user_id: &str, user_login: &str, name: &str, avatar: Option<String>) -> TwitchStream {
    TwitchStream {
        id: String::new(),
        user_id: user_id.to_string(),
        user_name: name.to_string(),
        user_login: user_login.to_string(),
        title: String::new(),
        viewer_count: 0,
        game_id: String::new(),
        game_name: String::new(),
        thumbnail_url: String::new(),
        started_at: String::new(),
        broadcaster_type: None,
        profile_image_url: avatar,
        is_live: None,
        tags: None,
        language: None,
        drops_earnable: false,
    }
}

/// A YouTube channel stored by its UC id names that id as its user id too, so
/// `live_mute::canonical` finds it.
fn ids_for(provider: &str, user_id: &str, channel: &str) -> (String, String) {
    if provider == "youtube" && user_id.is_empty() && is_youtube_channel_id(channel) {
        (channel.to_string(), channel.to_string())
    } else {
        (user_id.to_string(), channel.to_string())
    }
}

/// Add one channel unless it has no stable identity or is already listed.
#[allow(clippy::too_many_arguments)]
fn push_settings_row(
    rows: &mut Vec<AlertChannelRow>,
    seen: &mut HashSet<String>,
    muted: &[String],
    provider: &str,
    user_id: &str,
    channel: &str,
    name: Option<&str>,
    avatar: Option<String>,
) {
    let (user_id, user_login) = ids_for(provider, user_id, channel);
    let Some(key) = live_mute::canonical(provider, &user_id, &user_login) else { return };
    if !seen.insert(key.to_lowercase()) {
        return;
    }
    let keys = live_mute::keys_for_channel(provider, &user_id, &user_login);
    let name = name.filter(|n| !n.trim().is_empty()).unwrap_or(&user_login).to_string();
    rows.push(AlertChannelRow {
        stream: settings_row(&user_id, &user_login, &name, avatar),
        provider: provider.to_string(),
        on: !live_mute::is_muted(muted, &keys),
    });
}

/// The sheet's rows from what was read, pure so its rules are tested: Twitch
/// follows first, then (with `all`) the other platforms' follows and every
/// favourite, each channel listed once by its canonical key; sorted by name
/// with `all`, else in Twitch's newest-follow-first order.
/// The count is the listed channels whose alerts are off, `None` when the
/// Twitch follows could not be read.
fn alert_rows(
    twitch: Vec<TwitchStream>,
    twitch_read: bool,
    muted: &[String],
    follows: &[ProviderFollow],
    favs: &[FavoriteChannel],
    all: bool,
) -> (Vec<AlertChannelRow>, Option<usize>) {
    let mut seen: HashSet<String> = HashSet::new();
    let mut channels: Vec<AlertChannelRow> = Vec::new();
    for stream in twitch {
        let keys = live_mute::keys_for_twitch(&stream.user_login);
        let Some(key) = keys.first() else { continue };
        if !seen.insert(key.clone()) {
            continue;
        }
        let on = !live_mute::is_muted(muted, &keys);
        channels.push(AlertChannelRow { stream, provider: "twitch".to_string(), on });
    }
    if all {
        for f in follows {
            push_settings_row(
                &mut channels,
                &mut seen,
                muted,
                &f.provider,
                f.user_id.as_deref().unwrap_or_default(),
                &f.channel,
                f.display_name.as_deref(),
                f.avatar.clone(),
            );
        }
        for f in favs {
            // A Twitch favourite is stored by its numeric id; its channel is the login.
            let user_id = if f.provider == "twitch" { f.id.as_str() } else { "" };
            push_settings_row(
                &mut channels,
                &mut seen,
                muted,
                &f.provider,
                user_id,
                &f.channel,
                f.display_name.as_deref(),
                f.avatar.clone(),
            );
        }
    }
    // Phones list Twitch follows only, in Twitch's own order: newest follow
    // first (SN-264), so the channel just followed is at the top. Desktop mixes
    // in other platforms and favourites, which carry no follow date, so it
    // keeps one A to Z list.
    if all {
        channels.sort_by_key(|c| c.stream.user_name.to_lowercase());
    }
    let muted_count = twitch_read.then(|| channels.iter().filter(|c| !c.on).count());
    (channels, muted_count)
}

#[cfg(test)]
mod alert_rows_tests {
    use super::*;

    fn twitch(id: &str, login: &str) -> TwitchStream {
        settings_row(id, login, login, None)
    }

    fn favorite(id: &str, provider: &str, channel: &str) -> FavoriteChannel {
        serde_json::from_value(serde_json::json!({ "id": id, "provider": provider, "channel": channel })).unwrap()
    }

    #[test]
    fn a_twitch_favourite_that_is_also_followed_is_listed_once() {
        let favs = [favorite("1001", "twitch", "kamet0")];
        let (rows, _) = alert_rows(vec![twitch("1001", "kamet0")], true, &[], &[], &favs, true);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].provider, "twitch");
        assert_eq!(rows[0].stream.user_login, "kamet0", "listed by login, never the numeric id");
    }

    #[test]
    fn a_youtube_favourite_by_uc_id_is_muted_by_its_lowercased_entry() {
        let uc = "UCabcdefghijklmnopqrstuv";
        let favs = [favorite(&format!("youtube:{uc}"), "youtube", uc)];
        let muted = [format!("youtube:{}", uc.to_lowercase())];
        let (rows, count) = alert_rows(Vec::new(), true, &muted, &[], &favs, true);
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].on, "a legacy lowercased entry still mutes it");
        assert_eq!(count, Some(1));
    }

    #[test]
    fn phones_keep_twitchs_newest_follow_first_order_and_desktop_sorts_by_name() {
        // Twitch answers followed channels newest follow first.
        let follows = || vec![twitch("3", "zed"), twitch("2", "amy"), twitch("1", "mo")];
        let (phone, _) = alert_rows(follows(), true, &[], &[], &[], false);
        let order = |rows: &[AlertChannelRow]| rows.iter().map(|r| r.stream.user_login.clone()).collect::<Vec<_>>();
        assert_eq!(order(&phone), ["zed", "amy", "mo"]);
        let (desktop, _) = alert_rows(follows(), true, &[], &[], &[], true);
        assert_eq!(order(&desktop), ["amy", "mo", "zed"]);
    }

    #[test]
    fn the_count_is_listed_channels_only() {
        let muted = ["kamet0".to_string(), "unfollowed".to_string()];
        let (rows, count) = alert_rows(vec![twitch("1", "kamet0"), twitch("2", "xqc")], true, &muted, &[], &[], false);
        assert_eq!(rows.len(), 2);
        assert_eq!(count, Some(1), "an entry for a channel not listed is not counted");
    }

    #[test]
    fn no_count_when_the_twitch_follows_could_not_be_read() {
        let favs = [favorite("kick:xqc", "kick", "xqc")];
        let (rows, count) = alert_rows(Vec::new(), false, &["kick:xqc".to_string()], &[], &favs, true);
        assert_eq!(rows.len(), 1, "the other platforms still list");
        assert_eq!(count, None);
    }
}

/// The channels the live-alert sheet lists, each with whether it alerts.
/// Phones list their Twitch follows; desktop (`all_platforms`) adds the
/// other platforms' follows and every favourite, and still lists those when
/// the Twitch follows cannot be read.
#[tauri::command]
pub async fn get_followed_channels_for_alerts(
    all_platforms: Option<bool>,
    state: State<'_, AppState>,
) -> Result<AlertChannels, String> {
    let all = all_platforms.unwrap_or(false);
    let (twitch_rows, truncated, twitch_read) = match twitch_follows_for_alerts().await {
        Ok((rows, truncated)) => (rows, truncated, true),
        Err(e) if all => {
            log::debug!("[LiveAlerts] Twitch follows unreadable, listing the rest: {e}");
            (Vec::new(), false, false)
        }
        Err(e) => return Err(e),
    };
    let (muted, provider_follows, favorites): (Vec<String>, Vec<ProviderFollow>, Vec<FavoriteChannel>) = {
        let s = state.settings.lock().map_err(|e| e.to_string())?;
        let favorites = s.favorite_channels.iter().filter(|f| s.favorite_streamers.contains(&f.id)).cloned().collect();
        (s.live_notifications.muted_live_channels.clone(), s.provider_follows.clone(), favorites)
    };
    let (channels, muted_count) = alert_rows(twitch_rows, twitch_read, &muted, &provider_follows, &favorites, all);
    let last_copy = live_mute::last_copy_answer(TwitchService::account_generation());
    Ok(AlertChannels { channels, truncated, muted_count, last_copy })
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
    let mut path_in_use = false;
    commit_settings_patch(&app, &state, None, |current| {
        let root = serde_json::to_value(current).map_err(|e| e.to_string())?;
        let mut list = get_path(&root, "chat_highlights.custom_sounds").and_then(Value::as_array).cloned().unwrap_or_default();
        let Some(at) = list.iter().position(|s| s.get("id").and_then(Value::as_str) == Some(id.as_str())) else {
            return Ok(Map::new());
        };
        removed_path = list[at].get("path").and_then(Value::as_str).map(str::to_string);
        list.remove(at);
        // Another entry may name the same file.
        path_in_use = removed_path.as_deref().is_some_and(|p| list.iter().any(|s| s.get("path").and_then(Value::as_str) == Some(p)));
        let mut changes = Map::new();
        changes.insert("chat_highlights.custom_sounds".into(), Value::Array(list));
        patch_for_changes(current, &changes)
    })?;
    if let Some(path) = removed_path {
        crate::commands::sounds::remove_custom_sound(path.clone())?;
        // A sound that still pointed at the user's own file leaves the asset
        // scope with its entry, unless another entry names the same file.
        if !path_in_use {
            crate::commands::sounds::forget_legacy_sound(&app, &path);
        }
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
