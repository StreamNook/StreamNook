use crate::models::settings::AppState;
use crate::models::{
    stream::TwitchStream,
    user::{ChannelInfo, UserInfo},
};
use crate::services::token_vault;
use crate::services::twitch_auth_service::TwitchAuthService;
use anyhow::Result;
use chrono::{Duration as ChronoDuration, Utc};
use log::{debug, error, warn};
use reqwest::header::{ACCEPT, AUTHORIZATION};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::Emitter;

const CLIENT_ID: &str = env!("TWITCH_APP_CLIENT_ID");
const CLIENT_SECRET: &str = env!("TWITCH_APP_CLIENT_SECRET");
const TWITCH_GQL_CLIENT_ID: &str = "kimne78kx3ncx6brgo4mv6wki5h1ko";
const REDIRECT_URI: &str = "http://localhost:3000/callback";
/// How long a web sign-in's `state` stays good for its redirect.
const WEB_LOGIN_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// The `state` of the web sign-in in progress and when it began.
fn pending_web_login() -> &'static std::sync::Mutex<Option<(String, std::time::Instant)>> {
    static PENDING: std::sync::Mutex<Option<(String, std::time::Instant)>> =
        std::sync::Mutex::new(None);
    &PENDING
}

/// How recently someone must have chatted to count as present in a viewers list.
const CHATTED_RECENTLY_MS: i64 = 30 * 60 * 1000;
// Adding ANY scope here invalidates every stored token at once (see the
// missing-scopes branch in verify_token_health, which calls AccountStore::
// reset_all). That makes a scope change a one-time, whole-user-base re-auth, so
// new scopes are batched deliberately rather than added one at a time. The same
// branch records why in `twitch_signin`, so every window asks the user to sign
// in again until they do.
//
// The 2026-07 batch added, in one event:
//   channel:manage:polls / channel:manage:predictions — create and resolve them
//   moderator:manage:blocked_terms — /blockterm and /unblockterm (read-only
//     moderator:read:blocked_terms was already held)
//   user:bot — the chat-bot badge on /bot sends
// A later re-auth added:
//   moderator:manage:shield_mode: /shield and /shieldoff (Helix
//     PUT moderation/shield_mode refuses the token without it)
const SCOPES: &str = "user:read:follows user:read:email chat:read chat:edit channel:read:redemptions channel:manage:redemptions moderator:read:followers openid user:manage:whispers user:read:whispers user:read:emotes channel:read:hype_train moderator:read:blocked_terms moderator:manage:blocked_terms moderator:manage:chat_settings moderator:manage:unban_requests moderator:manage:banned_users moderator:manage:chat_messages moderator:read:warnings moderator:read:moderators moderator:read:vips moderator:read:chatters channel:manage:moderators channel:manage:vips channel:manage:polls channel:manage:predictions moderator:manage:suspicious_users user:manage:chat_color user:manage:blocked_users user:read:blocked_users moderator:manage:announcements moderator:manage:shoutouts channel:edit:commercial channel:manage:raids channel:manage:broadcast moderation:read user:write:chat user:bot clips:edit moderator:manage:automod moderator:read:suspicious_users moderator:manage:shield_mode";
/// Scopes every new sign-in asks for, that the token check never requires: a
/// stored sign-in without one is kept (no reset of every account), and the
/// command that needs it asks for a fresh sign-in when it is used.
//   moderator:manage:warnings: /warn (Helix POST moderation/warnings)
//   channel:read:ads, channel:manage:ads: the stream manager's ad schedule
//     (Helix GET channels/ads) and Snooze (POST channels/ads/schedule/snooze)
const OPTIONAL_SCOPES: &str = "moderator:manage:warnings channel:read:ads channel:manage:ads";

/// What a sign-in asks Twitch for: the required scopes, then the optional ones.
fn requested_scopes() -> String {
    format!("{SCOPES} {OPTIONAL_SCOPES}")
}

/// The app's Helix client id, for services that make their own Helix calls.
pub(crate) fn helix_client_id() -> &'static str {
    CLIENT_ID
}

/// Whether a token's granted scopes leave out `scope`.
pub(crate) fn lacks_scope(granted: &[String], scope: &str) -> bool {
    !granted.iter().any(|g| g == scope)
}

/// What /warn says to a sign-in from before warnings were asked for.
const WARN_SCOPE_MISSING: &str =
    "Warnings need one more Twitch permission. Sign in to Twitch again from your accounts to grant it; nothing else changes.";

const TOKEN_FILE_NAME: &str = ".twitch_token";

/// Normalize a GQL Language enum spelling ("FR", "ZH_HK") to the Helix form
/// ("fr", "zh-hk") so `TwitchStream.language` is uniform across API paths.
pub(crate) fn normalize_gql_language(value: &str) -> String {
    value.to_ascii_lowercase().replace('_', "-")
}

/// A Helix-shaped live row from a public GQL `user` node, or `None` when the
/// channel is not streaming. The thumbnail keeps Helix's `{width}x{height}`
/// template so every surface sizes it the same way.
pub(crate) fn live_row_from_gql(user: &serde_json::Value) -> Option<TwitchStream> {
    let stream = user.get("stream").filter(|s| !s.is_null())?;
    let s = |v: &serde_json::Value, key: &str| v.get(key).and_then(|x| x.as_str()).unwrap_or_default().to_string();
    let login = s(user, "login");
    let game = stream.get("game").filter(|g| !g.is_null());
    Some(TwitchStream {
        id: s(stream, "id"),
        user_id: s(user, "id"),
        user_name: s(user, "displayName"),
        thumbnail_url: format!("https://static-cdn.jtvnw.net/previews-ttv/live_user_{login}-{{width}}x{{height}}.jpg"),
        user_login: login,
        title: s(stream, "title"),
        viewer_count: stream.get("viewersCount").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
        game_id: game.map(|g| s(g, "id")).unwrap_or_default(),
        game_name: game.map(|g| s(g, "displayName")).unwrap_or_default(),
        started_at: s(stream, "createdAt"),
        broadcaster_type: None,
        profile_image_url: Some(s(user, "profileImageURL")).filter(|u| !u.is_empty()),
        is_live: Some(true),
        tags: None,
        language: None,
        drops_earnable: false,
    })
}

/// "partner" or "affiliate" from a GQL `roles` node (GQL `User` has no
/// `broadcasterType`).
fn broadcaster_type_from_roles(roles: Option<&serde_json::Value>) -> Option<String> {
    let has = |key: &str| roles.and_then(|r| r.get(key)).and_then(|v| v.as_bool()).unwrap_or(false);
    if has("isPartner") {
        Some("partner".to_string())
    } else if has("isAffiliate") {
        Some("affiliate".to_string())
    } else {
        None
    }
}

/// A Helix-shaped live row from a GQL `Stream` node (recommendations, a
/// category's streams, tag-filtered streams). The node's own `game` wins;
/// `fallback_game` (id, name) names the category when the query did not ask
/// for it. `None` when the node has no broadcaster.
pub(crate) fn stream_row_from_gql_node(
    node: &serde_json::Value,
    fallback_game: (&str, &str),
) -> Option<TwitchStream> {
    let broadcaster = node.get("broadcaster").filter(|b| !b.is_null())?;
    let s = |v: &serde_json::Value, key: &str| v.get(key).and_then(|x| x.as_str()).unwrap_or_default().to_string();
    let user_login = s(broadcaster, "login");
    let user_name = Some(s(broadcaster, "displayName")).filter(|n| !n.is_empty()).unwrap_or_else(|| user_login.clone());
    let tags: Vec<String> = node
        .get("freeformTags")
        .and_then(|t| t.as_array())
        .map(|arr| arr.iter().filter_map(|t| t.get("name").and_then(|n| n.as_str()).map(String::from)).collect())
        .unwrap_or_default();
    let (game_id, game_name) = match node.get("game").filter(|g| !g.is_null()) {
        Some(g) => (
            s(g, "id"),
            g.get("displayName")
                .and_then(|v| v.as_str())
                .filter(|n| !n.is_empty())
                .or_else(|| g.get("name").and_then(|v| v.as_str()))
                .unwrap_or_default()
                .to_string(),
        ),
        None => (fallback_game.0.to_string(), fallback_game.1.to_string()),
    };
    Some(TwitchStream {
        id: s(node, "id"),
        user_id: s(broadcaster, "id"),
        user_name,
        user_login,
        title: s(node, "title"),
        viewer_count: node.get("viewersCount").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
        game_id,
        game_name,
        thumbnail_url: s(node, "previewImageURL"),
        started_at: s(node, "createdAt"),
        broadcaster_type: broadcaster_type_from_roles(broadcaster.get("roles")),
        profile_image_url: broadcaster.get("profileImageURL").and_then(|v| v.as_str()).map(String::from),
        is_live: Some(true),
        tags: if tags.is_empty() { None } else { Some(tags) },
        language: node.get("language").and_then(|v| v.as_str()).map(normalize_gql_language),
        drops_earnable: false,
    })
}

/// Signed-out cursors carry this prefix, so a list paged from the public GQL
/// keeps paging there even when a token appears mid-scroll: Helix cannot read
/// a GQL cursor, nor GQL a Helix one.
const PUBLIC_CURSOR: &str = "gql:";

/// The GQL cursor inside a signed-out cursor, or `None` for a Helix one.
fn public_cursor(cursor: Option<&str>) -> Option<&str> {
    cursor.and_then(|c| c.strip_prefix(PUBLIC_CURSOR))
}

/// The next signed-out cursor for a GQL connection: its last edge's cursor
/// while `pageInfo.hasNextPage` holds.
fn public_next_cursor(connection: Option<&serde_json::Value>) -> Option<String> {
    let connection = connection?;
    let has_next = connection.pointer("/pageInfo/hasNextPage").and_then(|v| v.as_bool()).unwrap_or(false);
    if !has_next {
        return None;
    }
    connection
        .get("edges")
        .and_then(|e| e.as_array())
        .and_then(|e| e.last())
        .and_then(|e| e.get("cursor"))
        .and_then(|c| c.as_str())
        .filter(|c| !c.is_empty())
        .map(|c| format!("{PUBLIC_CURSOR}{c}"))
}

/// Whether `s` could be a Twitch login (lowercase letters, digits and
/// underscores, 1 to 25 long), so it is safe to ask GQL for that user.
fn is_twitch_login(s: &str) -> bool {
    (1..=25).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// The nodes of a GQL connection's edges, nulls skipped.
fn gql_nodes(connection: Option<&serde_json::Value>) -> Vec<&serde_json::Value> {
    connection
        .and_then(|c| c.get("edges"))
        .and_then(|e| e.as_array())
        .map(|edges| edges.iter().filter_map(|e| e.get("node").filter(|n| !n.is_null())).collect())
        .unwrap_or_default()
}

/// A category as the category directory and category search answer it, under
/// Helix's `games` field names so every caller reads it as before.
/// `viewer_count` is Twitch's own live total for the category
/// (`Game.viewersCount`, 0 when nobody is live), so the directory and search
/// always show the same number; it is absent only when GQL was unreachable and
/// the answer came from Helix, which carries no viewers. The box art keeps the
/// `{width}x{height}` template.
///
/// Serialized with `viewers_line`, the tile's words for `viewer_count`
/// (`count_text::viewers_line`), so no shell formats the number itself.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct CategoryRow {
    pub id: String,
    pub name: String,
    pub box_art_url: String,
    #[serde(default)]
    pub viewer_count: Option<u64>,
}

impl Serialize for CategoryRow {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut row = serializer.serialize_struct("CategoryRow", 5)?;
        row.serialize_field("id", &self.id)?;
        row.serialize_field("name", &self.name)?;
        row.serialize_field("box_art_url", &self.box_art_url)?;
        match self.viewer_count {
            Some(viewers) => row.serialize_field("viewer_count", &viewers)?,
            None => row.skip_field("viewer_count")?,
        }
        match crate::services::count_text::viewers_line(self.viewer_count) {
            Some(line) => row.serialize_field("viewers_line", &line)?,
            None => row.skip_field("viewers_line")?,
        }
        row.end()
    }
}

impl CategoryRow {
    /// A row from a GQL `Game` node; a null `viewersCount` means nobody is live.
    fn from_gql(node: &serde_json::Value) -> Option<Self> {
        let id = node.get("id").and_then(|v| v.as_str()).filter(|v| !v.is_empty())?;
        let name = node
            .get("name")
            .and_then(|v| v.as_str())
            .filter(|n| !n.is_empty())
            .or_else(|| node.get("displayName").and_then(|v| v.as_str()))?;
        Some(Self {
            id: id.to_string(),
            name: name.to_string(),
            box_art_url: node.get("boxArtURL").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            viewer_count: Some(node.get("viewersCount").and_then(|v| v.as_u64()).unwrap_or(0)),
        })
    }

    /// A row from a Helix `search/categories` or `games` row, which carries no viewers.
    fn from_helix(row: &serde_json::Value) -> Option<Self> {
        let id = row.get("id").and_then(|v| v.as_str()).filter(|v| !v.is_empty())?;
        let name = row.get("name").and_then(|v| v.as_str()).filter(|n| !n.is_empty())?;
        Some(Self {
            id: id.to_string(),
            name: name.to_string(),
            box_art_url: row.get("box_art_url").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            viewer_count: None,
        })
    }
}

/// A `games` page from the public GQL: its rows and the next signed-out cursor.
fn top_games_page(resp: &serde_json::Value) -> (Vec<CategoryRow>, Option<String>) {
    let games = resp.pointer("/data/games");
    (gql_nodes(games).into_iter().filter_map(CategoryRow::from_gql).collect(), public_next_cursor(games))
}

/// A category search from the public GQL: the category named exactly what was
/// typed (`exact`), then the fuzzy results. Unranked; repeats are left in.
fn search_page(resp: &serde_json::Value) -> Vec<CategoryRow> {
    resp.pointer("/data/exact")
        .filter(|v| !v.is_null())
        .into_iter()
        .chain(gql_nodes(resp.pointer("/data/searchCategories")))
        .filter_map(CategoryRow::from_gql)
        .collect()
}

/// A category name as search compares it: trimmed, lowercase, with Latin
/// accents folded, so "pokemon" typed on any keyboard matches "Pokémon".
fn fold_category_name(s: &str) -> String {
    s.trim()
        .chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => 'a',
            'ç' | 'ć' | 'č' => 'c',
            'ď' | 'đ' => 'd',
            'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ė' | 'ę' | 'ě' => 'e',
            'ğ' => 'g',
            'ì' | 'í' | 'î' | 'ï' | 'ī' | 'į' | 'ı' => 'i',
            'ł' => 'l',
            'ñ' | 'ń' | 'ň' => 'n',
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ő' => 'o',
            'ř' => 'r',
            'ś' | 'š' | 'ş' => 's',
            'ť' => 't',
            'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ů' | 'ű' => 'u',
            'ý' | 'ÿ' => 'y',
            'ź' | 'ż' | 'ž' => 'z',
            c => c,
        })
        .collect()
}

/// Category search order: the category named exactly what was typed first
/// (ignoring case and accents), then names that start with it, then the rest; within
/// each, the most live viewers first, and Twitch's own order on a tie. Repeats
/// of an id keep their first place (the exact lookup and the search can both
/// return the same category). Cut to `limit`.
pub fn rank_category_results(query: &str, rows: Vec<CategoryRow>, limit: usize) -> Vec<CategoryRow> {
    let q = fold_category_name(query);
    let mut seen = std::collections::HashSet::new();
    let mut rows: Vec<CategoryRow> = rows.into_iter().filter(|r| seen.insert(r.id.clone())).collect();
    let tier = |r: &CategoryRow| -> u8 {
        let name = fold_category_name(&r.name);
        if name == q {
            0
        } else if name.starts_with(&q) {
            1
        } else {
            2
        }
    };
    rows.sort_by(|a, b| {
        tier(a)
            .cmp(&tier(b))
            .then_with(|| b.viewer_count.unwrap_or(0).cmp(&a.viewer_count.unwrap_or(0)))
    });
    rows.truncate(limit);
    rows
}

/// A Helix `search/channels`-shaped row from a GQL `User` node: a live
/// channel carries its stream id, preview and start time, an offline one its
/// user id and avatar, and both the channel title and category.
pub(crate) fn channel_row_from_gql(user: &serde_json::Value) -> Option<TwitchStream> {
    let s = |v: Option<&serde_json::Value>, key: &str| {
        v.and_then(|v| v.get(key)).and_then(|x| x.as_str()).unwrap_or_default().to_string()
    };
    let user_id = s(Some(user), "id");
    let user_login = s(Some(user), "login");
    if user_id.is_empty() || user_login.is_empty() {
        return None;
    }
    let stream = user.get("stream").filter(|v| !v.is_null());
    let settings = user.get("broadcastSettings").filter(|v| !v.is_null());
    let game = stream
        .and_then(|st| st.get("game"))
        .filter(|g| !g.is_null())
        .or_else(|| settings.and_then(|b| b.get("game")).filter(|g| !g.is_null()));
    let avatar = s(Some(user), "profileImageURL");
    Some(TwitchStream {
        id: stream.map(|st| s(Some(st), "id")).filter(|id| !id.is_empty()).unwrap_or_else(|| user_id.clone()),
        user_name: Some(s(Some(user), "displayName")).filter(|n| !n.is_empty()).unwrap_or_else(|| user_login.clone()),
        user_id,
        user_login,
        title: s(settings, "title"),
        viewer_count: stream.and_then(|st| st.get("viewersCount")).and_then(|v| v.as_u64()).unwrap_or(0) as u32,
        game_id: s(game, "id"),
        game_name: s(game, "displayName"),
        thumbnail_url: match stream {
            Some(st) => s(Some(st), "previewImageURL"),
            None => avatar.clone(),
        },
        started_at: s(stream, "createdAt"),
        broadcaster_type: broadcaster_type_from_roles(user.get("roles")),
        profile_image_url: Some(avatar).filter(|a| !a.is_empty()),
        is_live: Some(stream.is_some()),
        tags: None,
        language: None,
        drops_earnable: false,
    })
}

static GQL_DEVICE_ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Stable per-install device id for first-party GQL calls (recommendations,
/// similar channels, channel panels). Twitch keys its anonymous recommendation
/// profile and rate-limit bucket off X-Device-Id, so it must survive restarts.
pub(crate) fn gql_device_id() -> String {
    GQL_DEVICE_ID
        .get_or_init(|| {
            let path = get_app_data_dir().map(|d| d.join(".gql_device_id")).ok();
            if let Some(p) = &path {
                if let Ok(s) = fs::read_to_string(p) {
                    let s = s.trim().to_string();
                    if (16..=40).contains(&s.len()) && s.chars().all(|c| c.is_ascii_alphanumeric())
                    {
                        return s;
                    }
                }
            }
            let id = uuid::Uuid::new_v4().to_string().replace('-', "");
            if let Some(p) = &path {
                if let Some(dir) = p.parent() {
                    let _ = fs::create_dir_all(dir);
                }
                let _ = fs::write(p, &id);
            }
            id
        })
        .clone()
}

/// Get the app data directory (works consistently in dev and release)
pub(crate) fn get_app_data_dir() -> Result<PathBuf> {
    // Mobile: app-private sandbox dir (desktop keeps the dirs-based path below).
    if let Some(base) = crate::services::app_paths::mobile_base() {
        return Ok(base.join("StreamNook"));
    }
    // Try to use the standard config directory first
    if let Some(config_dir) = dirs::config_dir() {
        let app_dir = config_dir.join("StreamNook");
        return Ok(app_dir);
    }

    // Fallback to data directory
    if let Some(data_dir) = dirs::data_dir() {
        let app_dir = data_dir.join("StreamNook");
        return Ok(app_dir);
    }

    // Last resort: use current exe directory
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            let app_dir = exe_dir.join("data");
            return Ok(app_dir);
        }
    }

    Err(anyhow::anyhow!("Could not determine app data directory"))
}

/// Per-account WebView2 user-data folder for Twitch *web* sessions
/// (`app_data_dir/twitch_web_profiles/<account_id>`). Child webviews that load
/// twitch.tv (the subscribe page, the device-code activation page) are built
/// with this as their `data_directory`, so each linked account keeps its own
/// isolated, persistent browser session. Switching the main account therefore
/// switches the web session too, and a re-login can never silently inherit a
/// different account's session. Mirrors the 7TV per-account profile pattern.
pub(crate) fn twitch_web_profile_dir(account_id: &str) -> Result<PathBuf> {
    let path = crate::platform::webview_store::profile_dir(
        get_app_data_dir()?,
        Path::new("twitch_web_profiles").join(account_id),
    );
    fs::create_dir_all(&path)?;
    Ok(path)
}

/// The whole `twitch_web_profiles` tree: every account's profile and the
/// pending stage.
pub(crate) fn twitch_web_profiles_root() -> Result<PathBuf> {
    Ok(crate::platform::webview_store::profile_dir(
        get_app_data_dir()?,
        "twitch_web_profiles",
    ))
}

/// Staging WebView2 profile for a login whose account id isn't known yet. The
/// device-code web session has to be captured somewhere before Twitch tells us
/// who signed in; a single fixed folder is safe because only one login is ever
/// in flight at once. `adopt_pending_web_profile` moves this into the account's
/// own profile once the id is known, so the subscribe window (and every other
/// per-account twitch webview) sees the same session the user logged into.
pub(crate) fn twitch_pending_web_profile_dir() -> Result<PathBuf> {
    let path = twitch_web_profiles_root()?.join("_pending");
    fs::create_dir_all(&path)?;
    Ok(path)
}

/// The WebView2 profile directory holding the ACTIVE account's twitch.tv web
/// session. This is the single source of truth for the signed-in web session:
/// the login window, the subscribe window, and the stream resolver's token read
/// all bind to it, so they can never disagree about which account is signed in.
/// When an account is linked it is that account's own profile (adopting a
/// freshly-staged first login if one is waiting); before any account is recorded
/// it is the staging profile the login window writes to.
pub(crate) fn active_twitch_web_profile_dir() -> Result<PathBuf> {
    if let Some(primary) = crate::services::account_store::AccountStore::primary() {
        adopt_pending_web_profile(&primary.user_id);
        twitch_web_profile_dir(&primary.user_id)
    } else {
        twitch_pending_web_profile_dir()
    }
}

/// True when a web profile folder holds a twitch.tv session (its cookie
/// store has been written at least once): WebView2 keeps the store under
/// `EBWebView/Default/Network/`, a Chromium profile (Linux) at its root.
fn web_profile_has_session(dir: &PathBuf) -> bool {
    if cfg!(target_os = "linux") {
        dir.join("Cookies").exists()
    } else {
        dir.join("EBWebView")
            .join("Default")
            .join("Network")
            .join("Cookies")
            .exists()
    }
}

/// Adopt a freshly-staged login session into `account_id`'s own web profile.
///
/// First sign-in captures the web session in the `_pending` staging profile
/// (the account id isn't known until the device-code grant completes). Once the
/// account is recorded, the next per-account twitch window calls this to move
/// that staged session into the account's own profile, so the login window and
/// the subscribe window stop disagreeing about which profile holds the session.
///
/// No-ops unless there's a staged session to adopt and the target doesn't
/// already have its own. Best-effort: a failure just leaves the user to
/// re-login, it never breaks the window.
pub(crate) fn adopt_pending_web_profile(account_id: &str) {
    let base = match twitch_web_profiles_root() {
        Ok(b) => b,
        Err(_) => return,
    };
    let pending = base.join("_pending");
    let target = base.join(account_id);

    // Nothing staged -> nothing to adopt.
    if !web_profile_has_session(&pending) {
        return;
    }
    // Target already owns a session -> keep it, discard the stale stage.
    if web_profile_has_session(&target) {
        let _ = fs::remove_dir_all(&pending);
        return;
    }
    // Move the staged session into place. Clear the empty placeholder first so
    // the rename has a clean destination (Windows rename fails if it exists).
    if target.exists() {
        let _ = fs::remove_dir_all(&target);
    }
    if let Err(e) = fs::rename(&pending, &target) {
        // Rename can fail if the staged profile is still locked or lives on a
        // different volume; fall back to a recursive copy so the session isn't
        // stranded, then drop the stage.
        debug!("[twitch-web] adopt rename failed ({e}); copying instead");
        if let Err(e) = copy_dir_recursive(&pending, &target) {
            warn!("[twitch-web] could not adopt staged login session: {e}");
            return;
        }
        let _ = fs::remove_dir_all(&pending);
    }
}

/// Minimal recursive directory copy (std has no equivalent). Only used as the
/// rename fallback in `adopt_pending_web_profile`.
fn copy_dir_recursive(from: &PathBuf, to: &PathBuf) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let src = entry.path();
        let dst = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&src, &dst)?;
        } else {
            fs::copy(&src, &dst)?;
        }
    }
    Ok(())
}

/// Best-effort removal of an account's Twitch web profile (called when the
/// account is unlinked or fully signed out, so a removed account leaves no
/// lingering browser session behind).
pub(crate) fn delete_twitch_web_profile(account_id: &str) {
    if let Ok(base) = twitch_web_profiles_root() {
        let profile = base.join(account_id);
        if profile.exists() {
            let _ = fs::remove_dir_all(&profile);
        }
    }
}

#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    interval: u64,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub(crate) struct StorableToken {
    pub(crate) access_token: String,
    pub(crate) refresh_token: String,
    pub(crate) expires_at: i64, // Unix timestamp
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeviceCodeInfo {
    pub user_code: String,
    pub verification_uri: String,
    pub device_code: String,
    pub interval: u64,
    pub expires_in: u64,
}

/// Twitch answered a token refresh with an error status.
#[derive(Debug)]
struct RefreshRefused {
    status: u16,
    body: String,
}

impl std::fmt::Display for RefreshRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Twitch refused the token refresh ({}): {}", self.status, self.body)
    }
}

impl std::error::Error for RefreshRefused {}

/// Why a token refresh failed, which decides whether the sign-in survives it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RefreshFailure {
    /// Twitch was not reached, or answered that it is busy: the sign-in holds
    /// and the next read tries again.
    Unreachable,
    /// Twitch turned the refresh token down: the sign-in is over.
    Refused,
}

/// A refusal Twitch would give differently a moment later.
fn refusal_is_transient(status: u16) -> bool {
    status >= 500 || status == 429
}

fn refresh_failure_kind(e: &anyhow::Error) -> RefreshFailure {
    for cause in e.chain() {
        if let Some(refused) = cause.downcast_ref::<RefreshRefused>() {
            return if refusal_is_transient(refused.status) {
                RefreshFailure::Unreachable
            } else {
                RefreshFailure::Refused
            };
        }
        if let Some(http) = cause.downcast_ref::<reqwest::Error>() {
            if http.is_timeout() || http.is_connect() || http.is_request() || http.is_body() {
                return RefreshFailure::Unreachable;
            }
        }
    }
    RefreshFailure::Refused
}

/// The error a failed refresh gives its caller, with its cause kept. The pages
/// read these words: a network failure says "network", which both the shared
/// sign-in check (`authErrors.ts`) and the iPhone's (`SignIn.isNetwork`) keep a
/// session through; a refusal says to sign in again.
fn refresh_failure_message(kind: RefreshFailure, cause: &str) -> String {
    match kind {
        RefreshFailure::Unreachable => format!(
            "Token refresh failed with a network error; the sign-in is kept and tried again ({cause})"
        ),
        RefreshFailure::Refused => {
            format!("Token expired and refresh failed ({cause}). Please log in again.")
        }
    }
}

/// Token health status returned by verify_token_health
#[derive(Debug, Clone, Serialize)]
pub struct TokenHealthStatus {
    pub is_valid: bool,
    pub seconds_remaining: i64,
    pub hours_remaining: i64,
    pub minutes_remaining: i64,
    pub scopes: Vec<String>,
    pub user_id: Option<String>,
    pub login: Option<String>,
    pub needs_refresh: bool,
    pub error: Option<String>,
}

/// The scopes in `required` (space separated, as `SCOPES` is) that the token
/// was not granted. Twitch reports a token's scopes exactly, so a plain string
/// match is the whole test.
fn missing_scopes<'a>(required: &'a str, granted: &[String]) -> Vec<&'a str> {
    required
        .split_whitespace()
        .filter(|&scope| !granted.iter().any(|g| g == scope))
        .collect()
}

/// Helix Send Chat Message answered with an error status. Carried inside the
/// `anyhow::Error` so a caller that needs the status (429 means "too fast")
/// can downcast; the text reads as it always has.
#[derive(Debug)]
pub struct HelixSendRefused {
    pub status: u16,
    pub body: serde_json::Value,
}

impl std::fmt::Display for HelixSendRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Helix send returned HTTP {}: {}", self.status, self.body)
    }
}

impl std::error::Error for HelixSendRefused {}

/// Twitch's own reason from a refused Helix moderation call, in plain words
/// where Twitch's are cryptic, so a command can say why it failed.
pub fn helix_refusal_text(status: u16, body: &str) -> String {
    let message = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v["message"].as_str().map(str::to_string))
        .unwrap_or_default();
    let lower = message.to_ascii_lowercase();
    if lower.contains("may not be banned") {
        "that user can't be banned or timed out here".into()
    } else if lower.contains("already banned") {
        "they are already banned".into()
    } else if lower.contains("is not banned") {
        "they are not banned or timed out".into()
    } else if lower.contains("not one of the broadcaster's moderators") {
        "you are not a moderator in this channel".into()
    } else if lower.contains("missing scope") {
        "StreamNook needs moderation access again: sign out and back in to Twitch".into()
    } else if !message.is_empty() {
        message
    } else {
        format!("Twitch answered HTTP {status}")
    }
}

pub struct TwitchService;

impl TwitchService {
    fn get_token_file_path() -> Result<PathBuf> {
        let mut path = get_app_data_dir()?;

        // Create directory if it doesn't exist
        if !path.exists() {
            debug!("[TWITCH_SERVICE] Creating directory: {:?}", path);
            fs::create_dir_all(&path)?;
        }

        path.push(TOKEN_FILE_NAME);
        debug!("[TWITCH_SERVICE] Token file path: {:?}", path);
        Ok(path)
    }

    /// Check if stored credentials exist (for showing appropriate toast)
    pub async fn has_stored_credentials() -> bool {
        Self::get_token_file_path()
            .map(|path| path.exists())
            .unwrap_or(false)
    }

    fn store_token_to_file(token: &StorableToken) -> Result<()> {
        let path = Self::get_token_file_path()?;
        token_vault::store_json(&path, token)?;
        debug!("[STORAGE] Token sealed to {:?}", path);
        Ok(())
    }

    fn load_token_from_file() -> Result<Option<StorableToken>> {
        token_vault::load_json(&Self::get_token_file_path()?)
    }

    fn delete_token_file() -> Result<()> {
        let result = token_vault::remove(&Self::get_token_file_path()?);
        crate::services::account_roster::set_twitch_identity(None);
        result
    }

    /// Write a token to the primary slot. This is the single definition of
    /// "occupying the primary slot", so login, refresh, and account-switch all
    /// persist it identically. `refresh_token` deliberately does NOT call this:
    /// keeping refresh a pure transform is what stops a SECONDARY account's
    /// refresh (via `AccountStore::get_token_for`) from silently clobbering the
    /// primary's token.
    pub(crate) async fn persist_primary_token(token: &StorableToken) -> Result<()> {
        Self::store_token_to_file(token)
    }

    /// Read whatever token currently occupies the primary slot. The account
    /// registry uses this to capture the outgoing main's token before a switch so
    /// the old main can be preserved as a linked secondary instead of being lost.
    pub(crate) async fn load_primary_token() -> Result<StorableToken> {
        Self::load_token_from_file()?.ok_or_else(|| anyhow::anyhow!("No stored Twitch token"))
    }

    // Device Code Flow - the main login method.
    //
    // Returns `(verification_uri, user_code)` from a SINGLE device flow and
    // spawns the poller against that same flow's device_code. The caller shows
    // this user_code and opens this verification_uri, so the code the user
    // authorizes is always the one the poller is waiting on. (A previous version
    // started a second, separate device flow just to fetch a code to display,
    // which could leave the poller waiting on a code the user never authorized,
    // hanging the login and leaving the app unauthenticated.)
    pub async fn login(
        _state: &AppState,
        app_handle: crate::rt::AppHandle,
    ) -> Result<(String, String)> {
        let client = crate::services::http::client().clone();

        // Start device flow
        let device_response = Self::start_device_flow(&client).await?;
        let user_code = device_response.user_code.clone();

        debug!(
            "Device code flow started. User code: {}",
            device_response.user_code
        );

        // Clone values for the spawned task
        let device_code = device_response.device_code.clone();
        let interval = device_response.interval;
        let expires_in = device_response.expires_in;
        let verification_uri = device_response.verification_uri.clone();

        // Spawn a task to poll for token
        tokio::task::spawn(async move {
            debug!("[LOGIN] Starting token polling task...");
            let result = Self::poll_for_token(&client, &device_code, interval, expires_in).await;

            match result {
                Ok(token_response) => {
                    debug!("[LOGIN] Token received from Twitch!");
                    // Dismiss the in-app login overlay the instant we have the
                    // token, so the loading screen behind it shows without waiting
                    // on a frontend round-trip. The frontend dismisses it too as a
                    // backup. (Desktop-only overlay; no-op on mobile device-code login.)
                    #[cfg(desktop)]
                    crate::commands::twitch::dismiss_login_overlay(&app_handle, "twitch-login");
                    debug!(
                        "[LOGIN] Access token (first 10 chars): {}...",
                        &token_response.access_token[..10.min(token_response.access_token.len())]
                    );
                    debug!(
                        "[LOGIN] Refresh token present: {}",
                        token_response.refresh_token.is_some()
                    );
                    debug!(
                        "[LOGIN] Expires in: {} seconds",
                        token_response.expires_in.unwrap_or(0)
                    );

                    // Store the token
                    let expires_at = chrono::Utc::now()
                        + chrono::Duration::seconds(
                            token_response.expires_in.unwrap_or(3600) as i64
                        );

                    let storable_token = StorableToken {
                        access_token: token_response.access_token.clone(),
                        refresh_token: token_response.refresh_token.clone().unwrap_or_default(),
                        expires_at: expires_at.timestamp(),
                    };

                    Self::complete_primary_login(&app_handle, storable_token).await;
                }
                Err(e) => {
                    error!("[LOGIN] Token polling failed: {}", e);
                    let _ = app_handle.emit("twitch-login-error", e.to_string());
                }
            }
        });

        // Return the verification URI and the matching user code for the frontend
        // to open/display. Both come from the single device flow polled above.
        Ok((verification_uri, user_code))
    }

    /// Finish a primary sign-in once Twitch has handed over its token, however
    /// it was obtained (the device-code poll, or the web sign-in's code
    /// exchange): seal it into the primary slot, reset the web-session reads,
    /// and announce the result with `twitch-login-complete` /
    /// `twitch-login-error`.
    pub(crate) async fn complete_primary_login(
        app_handle: &crate::rt::AppHandle,
        storable_token: StorableToken,
    ) {
        let store_result = Self::persist_primary_token(&storable_token).await;

        // The web-session reads (stream resolver, follow/unfollow, entitlement
        // checks) gate on a logged_out flag that twitch_logout sets. A fresh
        // login has to clear it and drop the stale cache, otherwise the next
        // get_token() short-circuits to "not logged in" and every web-session
        // feature stays dead until restart. Every primary login completes here,
        // so this owns the reset.
        {
            use tauri::Manager;
            app_handle
                .state::<crate::models::settings::AppState>()
                .twitch_auth
                .on_account_changed()
                .await;
        }

        match store_result {
            Ok(()) => {
                debug!("[LOGIN] Token stored");
                // This token was granted every scope in SCOPES, so a sign-in
                // StreamNook asked for is no longer owed.
                crate::services::twitch_signin::clear();
                // A phone's go-live pushes register under the account
                // that just signed in.
                #[cfg(mobile)]
                crate::services::push_alerts::server::schedule();

                // Emit success event
                debug!("[LOGIN] Emitting twitch-login-complete event...");
                if let Err(e) = app_handle.emit("twitch-login-complete", ()) {
                    error!("[LOGIN] Failed to emit login-complete event: {}", e);
                } else {
                    debug!("[LOGIN] Event emitted successfully");
                }
            }
            Err(e) => {
                error!("[LOGIN] Failed to store token: {:?}", e);
                let _ = app_handle.emit(
                    "twitch-login-error",
                    format!("Failed to store token: {}", e),
                );
            }
        }
    }

    /// Where a web sign-in's authorize page redirects with its result.
    pub(crate) fn web_login_redirect() -> &'static str {
        REDIRECT_URI
    }

    /// Start a primary sign-in on Twitch's own web pages: the authorize URL for
    /// an in-app sheet to load, with a fresh CSRF `state` kept here for
    /// `finish_web_login` to check. `force_verify` is off, so a twitch.tv
    /// session the sheet already holds goes straight to the Authorize page. A
    /// second start replaces the first, whose redirect then fails the check.
    pub(crate) fn begin_web_login() -> Result<String> {
        let state = format!("{:032x}", rand::random::<u128>());
        let url = Self::authorize_url(&state, false)?;
        *pending_web_login().lock().unwrap_or_else(|p| p.into_inner()) =
            Some((state, std::time::Instant::now()));
        Ok(url)
    }

    /// Finish a web sign-in from its redirect (`code` and `state`, or `error`):
    /// check the state, exchange the code and complete the primary login
    /// exactly as the device-code poll does. A failure is announced with
    /// `twitch-login-error` as well as returned.
    pub(crate) async fn finish_web_login(
        app_handle: &crate::rt::AppHandle,
        code: Option<String>,
        state: Option<String>,
        error: Option<String>,
    ) -> Result<()> {
        let result = Self::web_login_token(code, state, error).await;
        match result {
            Ok(token) => {
                Self::complete_primary_login(app_handle, token).await;
                Ok(())
            }
            Err(e) => {
                error!("[LOGIN] Web sign-in failed: {}", e);
                let _ = app_handle.emit("twitch-login-error", e.to_string());
                Err(e)
            }
        }
    }

    async fn web_login_token(
        code: Option<String>,
        state: Option<String>,
        error: Option<String>,
    ) -> Result<StorableToken> {
        // Taken either way: a state answers one redirect only.
        let pending = pending_web_login()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take();
        if let Some(error) = error {
            return Err(anyhow::anyhow!("Sign-in was cancelled or failed: {}", error));
        }
        let Some((expected, started)) = pending else {
            return Err(anyhow::anyhow!("No sign-in is waiting. Please try again."));
        };
        if started.elapsed() > WEB_LOGIN_TIMEOUT {
            return Err(anyhow::anyhow!("The sign-in took too long. Please try again."));
        }
        if state.as_deref() != Some(expected.as_str()) {
            return Err(anyhow::anyhow!(
                "Sign-in could not be verified (state mismatch). Please try again."
            ));
        }
        let code = code.filter(|c| !c.is_empty()).ok_or_else(|| {
            anyhow::anyhow!("Twitch did not return an authorization code.")
        })?;
        Self::exchange_code_for_token(&code).await
    }

    // Device code flow methods (kept for backward compatibility if needed)
    pub async fn start_device_login(_state: &AppState) -> Result<DeviceCodeInfo> {
        let client = crate::services::http::client().clone();
        let device_response = Self::start_device_flow(&client).await?;

        Ok(DeviceCodeInfo {
            user_code: device_response.user_code,
            verification_uri: device_response.verification_uri,
            device_code: device_response.device_code,
            interval: device_response.interval,
            expires_in: device_response.expires_in,
        })
    }

    pub async fn complete_device_login(device_code: &str, _state: &AppState) -> Result<String> {
        let client = crate::services::http::client().clone();

        let token_response = Self::poll_for_token(&client, device_code, 5, 1800).await?;

        let expires_at =
            Utc::now() + ChronoDuration::seconds(token_response.expires_in.unwrap_or(3600) as i64);

        let storable_token = StorableToken {
            access_token: token_response.access_token.clone(),
            refresh_token: token_response.refresh_token.clone().unwrap_or_default(),
            expires_at: expires_at.timestamp(),
        };

        Self::persist_primary_token(&storable_token).await?;
        crate::services::twitch_signin::clear();

        debug!(
            "Access token: {}...",
            &token_response.access_token[..10.min(token_response.access_token.len())]
        );

        Ok("Login successful".to_string())
    }

    /// Build the Twitch OAuth authorize URL for linking a NEW account in the
    /// system browser. `force_verify=true` makes Twitch always show the account
    /// chooser / login, so the user signs in deliberately as a different account
    /// rather than silently reusing an existing browser session. `state` is an
    /// opaque CSRF token the caller verifies against the redirect. This reads
    /// nothing from the primary's cached token.
    pub(crate) fn build_authorize_url(state: &str) -> Result<String> {
        Self::authorize_url(state, true)
    }

    fn authorize_url(state: &str, force_verify: bool) -> Result<String> {
        let scopes = requested_scopes();
        let url = reqwest::Url::parse_with_params(
            "https://id.twitch.tv/oauth2/authorize",
            &[
                ("client_id", CLIENT_ID),
                ("redirect_uri", REDIRECT_URI),
                ("response_type", "code"),
                ("scope", scopes.as_str()),
                ("force_verify", if force_verify { "true" } else { "false" }),
                ("state", state),
            ],
        )?;
        Ok(url.to_string())
    }

    /// Exchange an authorization code (from the redirect) for a fresh token.
    /// Deliberately does NOT persist anything: the caller decides where the token
    /// goes. For a linked secondary account that is the AccountStore's secondary
    /// slot, never the primary's cache.
    pub(crate) async fn exchange_code_for_token(code: &str) -> Result<StorableToken> {
        let client = crate::services::http::client().clone();
        let params = [
            ("client_id", CLIENT_ID),
            ("client_secret", CLIENT_SECRET),
            ("code", code),
            ("grant_type", "authorization_code"),
            ("redirect_uri", REDIRECT_URI),
        ];

        let response = client
            .post("https://id.twitch.tv/oauth2/token")
            .form(&params)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(anyhow::anyhow!("Token exchange failed: {}", error_text));
        }

        let token_response: TokenResponse = response.json().await?;
        let expires_at =
            Utc::now() + ChronoDuration::seconds(token_response.expires_in.unwrap_or(3600) as i64);

        Ok(StorableToken {
            access_token: token_response.access_token,
            refresh_token: token_response.refresh_token.unwrap_or_default(),
            expires_at: expires_at.timestamp(),
        })
    }

    async fn start_device_flow(client: &Client) -> Result<DeviceCodeResponse> {
        let scopes = requested_scopes();
        let params = [("client_id", CLIENT_ID), ("scopes", scopes.as_str())];

        let response = client
            .post("https://id.twitch.tv/oauth2/device")
            .form(&params)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(anyhow::anyhow!(
                "Failed to start device flow: {}",
                error_text
            ));
        }

        let device_response: DeviceCodeResponse = response.json().await?;
        Ok(device_response)
    }

    async fn poll_for_token(
        client: &Client,
        device_code: &str,
        interval: u64,
        expires_in: u64,
    ) -> Result<TokenResponse> {
        let start_time = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        let expiry_time = start_time + expires_in;
        let mut poll_interval = interval;

        loop {
            let current_time = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
            if current_time >= expiry_time {
                return Err(anyhow::anyhow!(
                    "Device code expired. Please try logging in again."
                ));
            }

            tokio::time::sleep(Duration::from_secs(poll_interval)).await;

            let scopes = requested_scopes();
            let params = [
                ("client_id", CLIENT_ID),
                ("scopes", scopes.as_str()),
                ("device_code", device_code),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ];

            let response = client
                .post("https://id.twitch.tv/oauth2/token")
                .form(&params)
                .send()
                .await?;

            if response.status().is_success() {
                let token_response: TokenResponse = response.json().await?;
                return Ok(token_response);
            }

            let error_text = response.text().await?;

            if error_text.contains("authorization_pending") {
                // User hasn't authorized yet, continue polling
                continue;
            } else if error_text.contains("slow_down") {
                // Twitch wants us to slow down
                poll_interval += 2;
                continue;
            } else if error_text.contains("expired_token") {
                return Err(anyhow::anyhow!(
                    "Device code expired. Please try logging in again."
                ));
            } else {
                return Err(anyhow::anyhow!("Token polling failed: {}", error_text));
            }
        }
    }

    pub async fn logout() -> Result<()> {
        // Delete token from file storage
        let _ = Self::delete_token_file();

        debug!("[LOGOUT] Complete - all tokens cleared from all storage locations");
        Ok(())
    }

    pub(crate) async fn refresh_token(refresh_token: &str) -> Result<StorableToken> {
        let client = crate::services::http::client().clone();
        let params = [
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", CLIENT_ID),
            ("client_secret", CLIENT_SECRET),
        ];

        debug!("[REFRESH] Attempting to refresh token...");

        let response = client
            .post("https://id.twitch.tv/oauth2/token")
            .form(&params)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await?;
            return Err(anyhow::anyhow!(RefreshRefused { status: status.as_u16(), body }));
        }

        let token_response: TokenResponse = response.json().await?;
        let expires_at =
            Utc::now() + ChronoDuration::seconds(token_response.expires_in.unwrap_or(3600) as i64);

        let new_storable_token = StorableToken {
            access_token: token_response.access_token,
            refresh_token: token_response
                .refresh_token
                .unwrap_or_else(|| refresh_token.to_string()),
            expires_at: expires_at.timestamp(),
        };

        // Pure transform: the caller decides where this token is stored. The
        // primary callers below persist it via `persist_primary_token`; the
        // secondary-account path (`AccountStore::get_token_for`) persists it only
        // to that account's own store. Persisting here would write EVERY refresh
        // into the primary slot, including a secondary's, which silently swaps
        // the main account out from under the user.
        Ok(new_storable_token)
    }

    pub async fn get_token() -> Result<String> {
        // Serialized end to end. Two callers landing inside the 5-minute
        // expiry buffer would otherwise BOTH refresh, and whichever persists
        // last can write back a stale refresh token, killing the session until
        // the user logs in again. The no-refresh path this also serializes is
        // a file read, so contention costs nothing. (On Android the background
        // notification worker shares this path with the running app, which
        // widened the race enough to be worth closing.)
        static REFRESH_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
        let _guard = REFRESH_LOCK.lock().await;

        let mut token = match Self::load_primary_token().await {
            Ok(token) => token,
            Err(e) => {
                debug!("[GET_TOKEN] No usable stored token: {:#}", e);
                return Err(anyhow::anyhow!(
                    "Not authenticated. Please log in to Twitch first."
                ));
            }
        };

        // Refresh when expired or within 5 minutes of expiring.
        let buffer_time = 300;
        if Utc::now().timestamp() >= (token.expires_at - buffer_time) {
            if token.refresh_token.is_empty() {
                return Err(anyhow::anyhow!(
                    "Token expired and no refresh token available. Please log in again."
                ));
            }
            debug!("[GET_TOKEN] Token expired or expiring soon, refreshing...");
            match Self::refresh_token(&token.refresh_token).await {
                Ok(new_token) => {
                    token = new_token;
                    let _ = Self::persist_primary_token(&token).await;
                }
                Err(e) => {
                    let kind = refresh_failure_kind(&e);
                    error!("[GET_TOKEN] Failed to refresh token ({kind:?}): {:?}", e);
                    return Err(anyhow::anyhow!(refresh_failure_message(kind, &format!("{e:#}"))));
                }
            }
        }

        Ok(token.access_token)
    }

    /// Verify the current token's health and return detailed status
    /// This should be called on app startup to proactively check/refresh the token
    pub async fn verify_token_health() -> Result<TokenHealthStatus> {
        let client = crate::services::http::client().clone();

        // Try to get the current token (this will auto-refresh if needed)
        let access_token = match Self::get_token().await {
            Ok(t) => t,
            Err(e) => {
                debug!("[Auth Debug] No valid token available: {:?}", e);
                return Ok(TokenHealthStatus {
                    is_valid: false,
                    seconds_remaining: 0,
                    hours_remaining: 0,
                    minutes_remaining: 0,
                    scopes: vec![],
                    user_id: None,
                    login: None,
                    needs_refresh: false,
                    error: Some(e.to_string()),
                });
            }
        };

        // Validate the token with Twitch
        let response = client
            .get("https://id.twitch.tv/oauth2/validate")
            .header("Authorization", format!("OAuth {}", access_token))
            .send()
            .await?;

        if !response.status().is_success() {
            debug!("[Auth Debug] Token is INVALID or EXPIRED. User needs to login or refresh.");
            return Ok(TokenHealthStatus {
                is_valid: false,
                seconds_remaining: 0,
                hours_remaining: 0,
                minutes_remaining: 0,
                scopes: vec![],
                user_id: None,
                login: None,
                needs_refresh: true,
                error: Some("Token validation failed".to_string()),
            });
        }

        let data: serde_json::Value = response.json().await?;

        let seconds_remaining = data["expires_in"].as_i64().unwrap_or(0);
        let hours = seconds_remaining / 3600;
        let minutes = (seconds_remaining % 3600) / 60;

        let scopes: Vec<String> = data["scopes"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        let missing_scopes = missing_scopes(SCOPES, &scopes);

        if !missing_scopes.is_empty() {
            // INFO on purpose: this line is the only evidence of a forced
            // re-login in the shipped log, and a scope Twitch will not grant
            // would otherwise loop the user through login forever.
            log::warn!(
                "[Auth] Token is missing required scopes {:?} (token has {} scopes); clearing accounts for re-login",
                missing_scopes,
                scopes.len()
            );

            // Clear the token AND the account registry. A scopes upgrade
            // invalidates every stored token at once, so this is a full re-auth,
            // not a single-account sign-out (no promotion). Emptying the registry
            // is load-bearing: it forces the subsequent login to open in the
            // DEFAULT WebView2 profile, the only profile `TwitchAuthService::
            // get_token` reads. If the registry survived, the re-login would
            // reopen in the old account's per-account profile and the new
            // web-session cookie would be invisible to the stream resolver — the
            // "non-proxy mode requires a twitch login" dead-end even though the
            // user just logged in.
            crate::services::account_store::AccountStore::reset_all().await;

            // Say why, to every window and across a relaunch, until the user
            // signs in again. A sign-out with no reason reads as a bug.
            crate::services::twitch_signin::require(
                crate::services::twitch_signin::SignInReason::NewPermissions,
            );

            return Ok(TokenHealthStatus {
                is_valid: false,
                seconds_remaining: 0,
                hours_remaining: 0,
                minutes_remaining: 0,
                scopes,
                user_id: None,
                login: None,
                needs_refresh: false,
                error: Some(format!(
                    "Missing scopes: {}. Please log in again.",
                    missing_scopes.join(", ")
                )),
            });
        }

        let user_id = data["user_id"].as_str().map(|s| s.to_string());
        let login = data["login"].as_str().map(|s| s.to_string());

        // A token with every scope settles any sign-in StreamNook asked for,
        // whichever path stored it.
        crate::services::twitch_signin::clear();

        debug!("[Auth Debug] Token is VALID and has all required scopes.");
        debug!("[Auth Debug] Scopes: {}", scopes.join(", "));
        debug!(
            "[Auth Debug] Time remaining: {}h {}m ({}s)",
            hours, minutes, seconds_remaining
        );

        let needs_refresh = seconds_remaining < 3600;
        if needs_refresh {
            debug!("[Auth Debug] Token expires in less than 1 hour! Consider refreshing soon.");
        }

        Ok(TokenHealthStatus {
            is_valid: true,
            seconds_remaining,
            hours_remaining: hours,
            minutes_remaining: minutes,
            scopes,
            user_id,
            login,
            needs_refresh,
            error: None,
        })
    }

    /// Force refresh the token even if it hasn't expired yet
    pub async fn force_refresh_token() -> Result<String> {
        let token = Self::load_primary_token().await?;

        if token.refresh_token.is_empty() {
            return Err(anyhow::anyhow!(
                "No refresh token available. Please log in again."
            ));
        }

        debug!("[Auth Debug] Force refreshing token...");
        let new_token = Self::refresh_token(&token.refresh_token).await?;

        // This is an explicit PRIMARY refresh, so persist it to the primary slot.
        let _ = Self::persist_primary_token(&new_token).await;

        debug!("[Auth Debug] Token refreshed successfully!");
        Ok(new_token.access_token)
    }

    pub async fn get_followed_streams(_state: &AppState) -> Result<Vec<TwitchStream>> {
        let token = match Self::get_token().await {
            Ok(t) => t,
            Err(_) => {
                return Err(anyhow::anyhow!(
                    "Not authenticated. Please log in to Twitch first."
                ));
            }
        };
        let client = crate::services::http::client().clone();

        // First, get the user ID
        let user_response = client
            .get("https://api.twitch.tv/helix/users")
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .header("Client-Id", CLIENT_ID)
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        let user_id = user_response["data"][0]["id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Failed to get user ID"))?;

        // Now get followed streams with user_id
        let response = client
            .get(format!(
                "https://api.twitch.tv/helix/streams/followed?user_id={}",
                user_id
            ))
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .header("Client-Id", CLIENT_ID)
            .header(ACCEPT, "application/json")
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        // Handle null or missing data field
        let data = response.get("data").and_then(|d| d.as_array());

        match data {
            Some(arr) => {
                let mut streams: Vec<TwitchStream> =
                    serde_json::from_value(serde_json::Value::Array(arr.clone()))?;

                // Fetch broadcaster types for all streams in a batch
                if !streams.is_empty() {
                    let user_ids: Vec<String> = streams.iter().map(|s| s.user_id.clone()).collect();
                    let user_ids_param = user_ids
                        .iter()
                        .map(|id| format!("id={}", id))
                        .collect::<Vec<_>>()
                        .join("&");

                    let users_url = format!("https://api.twitch.tv/helix/users?{}", user_ids_param);
                    let users_response = client
                        .get(&users_url)
                        .header("Client-Id", CLIENT_ID)
                        .header(AUTHORIZATION, format!("Bearer {}", token))
                        .send()
                        .await?
                        .json::<serde_json::Value>()
                        .await?;

                    if let Some(users_data) = users_response.get("data").and_then(|d| d.as_array())
                    {
                        // Create maps of user_id -> broadcaster_type and user_id -> profile image.
                        // The followed-streams endpoint only returns a stream-preview thumbnail
                        // (with {width}x{height} placeholders), so the real avatar has to come from
                        // this users batch — which we already fetch for broadcaster_type anyway.
                        let mut broadcaster_types = std::collections::HashMap::new();
                        let mut profile_images = std::collections::HashMap::new();
                        for user in users_data {
                            if let Some(id) = user.get("id").and_then(|v| v.as_str()) {
                                if let Some(broadcaster_type) =
                                    user.get("broadcaster_type").and_then(|v| v.as_str())
                                {
                                    if !broadcaster_type.is_empty() {
                                        broadcaster_types
                                            .insert(id.to_string(), broadcaster_type.to_string());
                                    }
                                }
                                if let Some(profile_image_url) =
                                    user.get("profile_image_url").and_then(|v| v.as_str())
                                {
                                    if !profile_image_url.is_empty() {
                                        profile_images
                                            .insert(id.to_string(), profile_image_url.to_string());
                                    }
                                }
                            }
                        }

                        // Update streams with broadcaster types and profile images
                        for stream in &mut streams {
                            if let Some(broadcaster_type) = broadcaster_types.get(&stream.user_id) {
                                stream.broadcaster_type = Some(broadcaster_type.clone());
                            }
                            if let Some(profile_image_url) = profile_images.get(&stream.user_id) {
                                stream.profile_image_url = Some(profile_image_url.clone());
                            }
                        }
                    }
                }

                Ok(streams)
            }
            None => Ok(Vec::new()), // Return empty vec if no data
        }
    }

    pub async fn get_channel_info(channel_name: &str, _state: &AppState) -> Result<ChannelInfo> {
        let token = match Self::get_token().await {
            Ok(t) => t,
            Err(_) => {
                return Err(anyhow::anyhow!(
                    "Not authenticated. Please log in to Twitch first."
                ));
            }
        };
        // Helix /channels accepts broadcaster_id ONLY. There is no
        // broadcaster_login parameter, so querying by login always came back
        // with an empty `data` array and surfaced as "Channel not found" for
        // channels that plainly exist. That failure is why a channel the user
        // does not follow rendered chat with no Twitch badges: without an id
        // the badge metadata cache is never populated for the room, so every
        // badge resolves to no image. Resolve the login to an id first.
        let broadcaster_id = Self::get_user_by_login(channel_name).await?.id;
        Self::channel_row_by_id(&broadcaster_id, &token).await
    }

    /// The Helix `channels` row for a broadcaster id: title and category, live
    /// or not. Split out so a caller that already holds the id does not pay a
    /// second users lookup for it.
    async fn channel_row_by_id(broadcaster_id: &str, token: &str) -> Result<ChannelInfo> {
        let client = crate::services::http::client().clone();
        let response = client
            .get(format!(
                "https://api.twitch.tv/helix/channels?broadcaster_id={}",
                urlencoding::encode(broadcaster_id)
            ))
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .header("Client-Id", CLIENT_ID)
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        // Check if data exists and has at least one element
        let data = response
            .get("data")
            .and_then(|d| d.as_array())
            .and_then(|arr| arr.first())
            .ok_or_else(|| anyhow::anyhow!("Channel '{}' not found", broadcaster_id))?;

        let info: ChannelInfo = serde_json::from_value(data.clone())?;
        Ok(info)
    }

    /// Create a clip of a live broadcast via Helix `POST /clips` (needs the
    /// `clips:edit` scope). Returns `(clip_id, edit_url)`. The clip captures the
    /// last ~30s and is processed async, so it may take a few seconds before its
    /// thumbnail/playback is ready (the public URL `clips.twitch.tv/<id>` works
    /// once processed). Documented statuses are mapped to short codes the UI
    /// turns into friendly messages, rather than leaking raw HTTP errors.
    pub async fn create_clip(broadcaster_id: &str) -> Result<(String, String)> {
        let token = Self::get_token()
            .await
            .map_err(|_| anyhow::anyhow!("REAUTH"))?;
        let client = crate::services::http::client().clone();
        let resp = client
            .post("https://api.twitch.tv/helix/clips")
            .query(&[("broadcaster_id", broadcaster_id)])
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .header("Client-Id", CLIENT_ID)
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("NETWORK"))?;

        let status = resp.status();
        if status.is_success() {
            let json: serde_json::Value =
                resp.json().await.map_err(|_| anyhow::anyhow!("ERROR"))?;
            let data = json
                .get("data")
                .and_then(|d| d.as_array())
                .and_then(|a| a.first())
                .ok_or_else(|| anyhow::anyhow!("ERROR"))?;
            let id = data
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let edit_url = data
                .get("edit_url")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if id.is_empty() {
                return Err(anyhow::anyhow!("ERROR"));
            }
            return Ok((id, edit_url));
        }

        // Short codes the frontend maps to messages (see createClip in AppStore).
        let code = match status.as_u16() {
            401 => "REAUTH",    // token lacks clips:edit → re-login needed
            403 => "FORBIDDEN", // channel restricts who can clip
            400 => "DISABLED",  // clips disabled for this channel
            404 => "NOTFOUND",
            422 => "OFFLINE", // can only clip a live broadcast
            _ => "ERROR",
        };
        Err(anyhow::anyhow!("{}", code))
    }

    /// Fetch the current live broadcast's stream id + start time via Helix Get
    /// Streams. The stream `id` equals the GQL `broadcastID` that `CreateRawMedia`
    /// wants for a live clip (confirmed by capture: `clip.broadcaster.stream.id`
    /// == the `broadcastID`), and `started_at` lets the caller derive the live
    /// offset (uptime). Errors "OFFLINE" when the channel isn't live.
    pub async fn get_live_broadcast(broadcaster_id: &str) -> Result<(String, String)> {
        let token = Self::get_token()
            .await
            .map_err(|_| anyhow::anyhow!("REAUTH"))?;
        let client = crate::services::http::client().clone();
        let resp = client
            .get("https://api.twitch.tv/helix/streams")
            .query(&[("user_id", broadcaster_id)])
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .header("Client-Id", CLIENT_ID)
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("NETWORK"))?;
        if !resp.status().is_success() {
            let code = if resp.status().as_u16() == 401 {
                "REAUTH"
            } else {
                "ERROR"
            };
            return Err(anyhow::anyhow!("{}", code));
        }
        let json: serde_json::Value = resp.json().await.map_err(|_| anyhow::anyhow!("ERROR"))?;
        let data = json
            .get("data")
            .and_then(|d| d.as_array())
            .and_then(|a| a.first())
            .ok_or_else(|| anyhow::anyhow!("OFFLINE"))?;
        let id = data
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let started_at = data
            .get("started_at")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if id.is_empty() {
            return Err(anyhow::anyhow!("OFFLINE"));
        }
        Ok((id, started_at))
    }

    pub async fn get_user_info() -> Result<UserInfo> {
        let token = Self::get_token().await?;
        let info = Self::get_user_info_with_token(&token).await?;
        // The token check runs alongside this read and can clear every account
        // while it is in flight. A profile read from the token it just removed
        // must not report the user as signed in.
        if crate::services::twitch_signin::current().is_some() {
            return Err(anyhow::anyhow!(
                "Sign-in required: Twitch needs you to sign in again"
            ));
        }
        crate::services::account_roster::set_twitch_identity(Some(info.clone()));
        // Every client confirms its member here, at start and after signing in.
        crate::services::member_signin::signed_in(&info);
        Ok(info)
    }

    /// Fetch the Helix user profile for an arbitrary access token, not just the
    /// primary login's. Used by the multi-account store to identify a freshly
    /// linked account from its own token.
    pub(crate) async fn get_user_info_with_token(token: &str) -> Result<UserInfo> {
        let client = crate::services::http::client().clone();

        let response = client
            .get("https://api.twitch.tv/helix/users")
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .header("Client-Id", CLIENT_ID)
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        let data = response
            .get("data")
            .and_then(|d| d.as_array())
            .and_then(|arr| arr.first())
            .ok_or_else(|| anyhow::anyhow!("Failed to get user info"))?;

        let user_info: UserInfo = serde_json::from_value(data.clone())?;
        Ok(user_info)
    }

    /// Send a chat message via Helix Send Chat Message. Returns
    /// (message_id, is_sent, drop_reason). The message_id is the AUTHORITATIVE id
    /// Twitch assigns, so an own message can be deleted immediately without
    /// waiting to catch its IRC echo. `is_sent` is false when Twitch accepted the
    /// request but dropped the message (e.g. AutoMod), with `drop_reason` set.
    /// Requires the `user:write:chat` scope; callers fall back to IRC on error.
    pub async fn send_chat_message_helix(
        broadcaster_id: &str,
        sender_id: &str,
        message: &str,
        reply_parent_msg_id: Option<&str>,
    ) -> Result<(Option<String>, bool, Option<String>)> {
        let token = Self::get_token().await?;
        Self::send_chat_message_helix_with_token(
            &token,
            broadcaster_id,
            sender_id,
            message,
            reply_parent_msg_id,
        )
        .await
    }

    /// Same as `send_chat_message_helix`, but with an explicit token so a chosen
    /// secondary account can send as itself. The caller passes that account's
    /// token plus its own user id as `sender_id`.
    pub async fn send_chat_message_helix_with_token(
        token: &str,
        broadcaster_id: &str,
        sender_id: &str,
        message: &str,
        reply_parent_msg_id: Option<&str>,
    ) -> Result<(Option<String>, bool, Option<String>)> {
        let client = crate::services::http::client().clone();

        let mut body = serde_json::json!({
            "broadcaster_id": broadcaster_id,
            "sender_id": sender_id,
            "message": message,
        });
        if let Some(reply) = reply_parent_msg_id {
            body["reply_parent_message_id"] = serde_json::Value::String(reply.to_string());
        }

        let response = client
            .post("https://api.twitch.tv/helix/chat/messages")
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .header("Client-Id", CLIENT_ID)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = response.status();
        let json = response
            .json::<serde_json::Value>()
            .await
            .unwrap_or(serde_json::Value::Null);
        if !status.is_success() {
            return Err(HelixSendRefused { status: status.as_u16(), body: json }.into());
        }

        let data = json
            .get("data")
            .and_then(|d| d.as_array())
            .and_then(|a| a.first());
        let message_id = data
            .and_then(|d| d.get("message_id"))
            .and_then(|v| v.as_str())
            .map(String::from);
        let is_sent = data
            .and_then(|d| d.get("is_sent"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let drop_reason = data
            .and_then(|d| d.get("drop_reason"))
            .and_then(|dr| {
                dr.get("message")
                    .or_else(|| dr.get("code"))
                    .and_then(|v| v.as_str())
            })
            .map(String::from);
        Ok((message_id, is_sent, drop_reason))
    }

    pub async fn get_user_by_login(login: &str) -> Result<UserInfo> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();

        let response = client
            .get(format!(
                "https://api.twitch.tv/helix/users?login={}",
                urlencoding::encode(login)
            ))
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .header("Client-Id", CLIENT_ID)
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        let data = response
            .get("data")
            .and_then(|d| d.as_array())
            .and_then(|arr| arr.first())
            .ok_or_else(|| anyhow::anyhow!("User '{}' not found", login))?;

        let user_info: UserInfo = serde_json::from_value(data.clone())?;
        Ok(user_info)
    }

    pub async fn get_user_by_id(user_id: &str) -> Result<UserInfo> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();

        let response = client
            .get(format!("https://api.twitch.tv/helix/users?id={}", user_id))
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .header("Client-Id", CLIENT_ID)
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        let data = response
            .get("data")
            .and_then(|d| d.as_array())
            .and_then(|arr| arr.first())
            .ok_or_else(|| anyhow::anyhow!("User with ID '{}' not found", user_id))?;

        let user_info: UserInfo = serde_json::from_value(data.clone())?;
        Ok(user_info)
    }

    /// Discover feed dispatcher. First page (no cursor) is Twitch's real
    /// recommendation feed via GQL; infinite scroll continues into the Helix
    /// top-streams list behind a "helix:"-prefixed cursor. The cursor stays an
    /// opaque string to the frontend, so its pagination contract is unchanged.
    pub async fn get_recommended_streams_paginated(
        _state: &AppState,
        cursor: Option<String>,
        limit: u32,
        languages: Vec<String>,
        personalized: bool,
    ) -> Result<(Vec<TwitchStream>, Option<String>)> {
        match cursor {
            None => match Self::fetch_gql_recommended(40, &languages, personalized).await {
                Ok(streams) if !streams.is_empty() => {
                    debug!("[Discovery] recommendations page: {} streams", streams.len());
                    Ok((streams, Some("helix:".to_string())))
                }
                // Empty (including filtered-to-empty) or failed recommendations
                // fall through to Helix in the same call: an empty first page
                // could never trigger the scroll-based load-more.
                Ok(_) => {
                    debug!("[Discovery] recommendations empty, serving Helix top streams");
                    Self::fetch_helix_top(_state, None, limit, &languages).await
                }
                Err(e) => {
                    warn!("[Discovery] recommendations failed ({}), serving Helix top streams", e);
                    Self::fetch_helix_top(_state, None, limit, &languages).await
                }
            },
            Some(c) => {
                let after = c.strip_prefix("helix:").unwrap_or(&c);
                let after = if after.is_empty() {
                    None
                } else {
                    Some(after.to_string())
                };
                Self::fetch_helix_top(_state, after, limit, &languages).await
            }
        }
    }

    /// Twitch's recommendation feed (GQL `recommendedStreams`). With
    /// `personalized` (an opt-in setting, off by default) it tries the
    /// account-personalized form first (drops token + Android client, the
    /// proven authorized-GQL pairing); any rejection is remembered for the
    /// session so the wasted round-trip is paid at most once per app run.
    /// Otherwise, and as the fallback, it makes the anonymous web-client call,
    /// which is device/geo-aware via the stable X-Device-Id and never sends
    /// account credentials.
    async fn fetch_gql_recommended(
        limit: u32,
        languages: &[String],
        personalized: bool,
    ) -> Result<Vec<TwitchStream>> {
        use std::sync::atomic::{AtomicBool, Ordering};
        static AUTH_RECS_REJECTED: AtomicBool = AtomicBool::new(false);

        let client = crate::services::http::client().clone();
        // recRequestID must be typed ID!, not String! (String! fails GQL
        // validation even though the value is a plain string).
        let query = "query($first: Int!, $recRequestID: ID!) { \
            recommendedStreams(first: $first, recRequestID: $recRequestID, location: \"LEFT_NAV\", \
                context: { platform: \"web\", clientApp: \"twilight\" }) { \
                edges { node { \
                    id title viewersCount language previewImageURL createdAt \
                    freeformTags { name } \
                    game { id name displayName } \
                    broadcaster { id login displayName profileImageURL(width: 70) roles { isPartner isAffiliate } } \
                } } \
            } }";
        let body = serde_json::json!({
            "query": query,
            "variables": {
                "first": limit,
                "recRequestID": uuid::Uuid::new_v4().to_string().replace('-', ""),
            }
        });
        let device_id = gql_device_id();

        let mut json: Option<serde_json::Value> = None;
        if personalized && !AUTH_RECS_REJECTED.load(Ordering::Relaxed) {
            if let Ok(token) =
                crate::services::drops_auth_service::DropsAuthService::get_token().await
            {
                let attempt = tokio::time::timeout(
                    Duration::from_secs(4),
                    client
                        .post("https://gql.twitch.tv/gql")
                        .header("Client-ID", env!("TWITCH_ANDROID_CLIENT_ID"))
                        .header(AUTHORIZATION, format!("OAuth {}", token))
                        .header("Origin", "https://www.twitch.tv")
                        .header("Referer", "https://www.twitch.tv")
                        .header("X-Device-Id", &device_id)
                        .header(
                            "Client-Session-Id",
                            uuid::Uuid::new_v4().to_string().replace('-', ""),
                        )
                        .json(&body)
                        .send(),
                )
                .await;
                match attempt {
                    Ok(Ok(resp)) => {
                        let status = resp.status();
                        match resp.json::<serde_json::Value>().await {
                            Ok(parsed) => {
                                let has_edges = parsed
                                    .pointer("/data/recommendedStreams/edges")
                                    .and_then(|e| e.as_array())
                                    .map(|a| !a.is_empty())
                                    .unwrap_or(false);
                                if status.is_success()
                                    && parsed.get("errors").is_none()
                                    && has_edges
                                {
                                    debug!("[Discovery] authorized recommendations resolved");
                                    json = Some(parsed);
                                } else {
                                    // Only a real rejection disables the attempt for
                                    // the session; transient emptiness does not.
                                    if status.is_client_error() || parsed.get("errors").is_some() {
                                        warn!(
                                            "[Discovery] authorized recommendations rejected (HTTP {}): {:?}",
                                            status,
                                            parsed.get("errors")
                                        );
                                        AUTH_RECS_REJECTED.store(true, Ordering::Relaxed);
                                    }
                                }
                            }
                            Err(e) => {
                                debug!("[Discovery] authorized recommendations parse failed: {}", e)
                            }
                        }
                    }
                    Ok(Err(e)) => {
                        debug!("[Discovery] authorized recommendations request failed: {}", e)
                    }
                    Err(_) => debug!("[Discovery] authorized recommendations timed out"),
                }
            }
        }

        let json = match json {
            Some(j) => j,
            None => {
                let resp = client
                    .post("https://gql.twitch.tv/gql")
                    .header("Client-Id", TWITCH_GQL_CLIENT_ID)
                    .header("X-Device-ID", &device_id)
                    .json(&body)
                    .send()
                    .await?
                    .json::<serde_json::Value>()
                    .await?;
                if let Some(errors) = resp.get("errors") {
                    return Err(anyhow::anyhow!("recommendedStreams GQL errors: {}", errors));
                }
                debug!("[Discovery] anonymous recommendations resolved");
                resp
            }
        };

        let mut streams = json
            .pointer("/data/recommendedStreams/edges")
            .and_then(|e| e.as_array())
            .map(|e| Self::map_recommended_edges(e))
            .unwrap_or_default();

        if !languages.is_empty() {
            let wanted: Vec<String> = languages.iter().map(|l| l.to_ascii_lowercase()).collect();
            streams.retain(|s| {
                s.language
                    .as_deref()
                    .map(|l| wanted.iter().any(|w| w == l))
                    .unwrap_or(false)
            });
        }

        Ok(streams)
    }

    /// Map GQL `recommendedStreams` edges into the app's stream shape.
    fn map_recommended_edges(edges: &[serde_json::Value]) -> Vec<TwitchStream> {
        edges
            .iter()
            .filter_map(|edge| edge.get("node").filter(|n| !n.is_null()))
            .filter_map(|node| stream_row_from_gql_node(node, ("", "")))
            .collect()
    }

    /// Helix top-live-streams list (the pre-recommendations Discover source),
    /// now with an optional broadcast-language filter. Cursors returned from
    /// here carry the "helix:" phase prefix.
    async fn fetch_helix_top(
        _state: &AppState,
        cursor: Option<String>,
        limit: u32,
        languages: &[String],
    ) -> Result<(Vec<TwitchStream>, Option<String>)> {
        let token = Self::get_token().await.ok();
        let public = |after: Option<String>| async move {
            let (streams, next) = Self::top_streams_public(after.as_deref(), limit, languages).await?;
            Ok::<_, anyhow::Error>((streams, next.map(|c| format!("helix:{c}"))))
        };
        if token.is_none() || public_cursor(cursor.as_deref()).is_some() {
            return public(public_cursor(cursor.as_deref()).map(String::from)).await;
        }
        let client = crate::services::http::client().clone();

        // Build URL with pagination
        let mut url = format!("https://api.twitch.tv/helix/streams?first={}", limit);
        if let Some(cursor) = cursor {
            url.push_str(&format!("&after={}", cursor));
        }
        for lang in languages {
            if lang
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-')
            {
                url.push_str(&format!("&language={}", lang.to_ascii_lowercase()));
            }
        }

        let mut request = client.get(&url).header("Client-Id", CLIENT_ID);

        // Add authorization if we have a token
        if let Some(token) = &token {
            request = request.header(AUTHORIZATION, format!("Bearer {}", token));
        }

        let response = request.send().await?;
        if !response.status().is_success() {
            debug!("[Discovery] top streams: HTTP {}, asking the public GQL", response.status());
            return public(None).await;
        }
        let response = response.json::<serde_json::Value>().await?;

        // Get pagination cursor
        let next_cursor = response
            .get("pagination")
            .and_then(|p| p.get("cursor"))
            .and_then(|c| c.as_str())
            .map(|s| s.to_string());

        // Handle null or missing data field
        let data = response.get("data").and_then(|d| d.as_array());

        match data {
            Some(arr) => {
                let mut streams: Vec<TwitchStream> =
                    serde_json::from_value(serde_json::Value::Array(arr.clone()))?;

                // Fetch broadcaster types for all streams in a batch
                if !streams.is_empty() && token.is_some() {
                    let user_ids: Vec<String> = streams.iter().map(|s| s.user_id.clone()).collect();
                    let user_ids_param = user_ids
                        .iter()
                        .map(|id| format!("id={}", id))
                        .collect::<Vec<_>>()
                        .join("&");

                    let users_url = format!("https://api.twitch.tv/helix/users?{}", user_ids_param);
                    let users_response = client
                        .get(&users_url)
                        .header("Client-Id", CLIENT_ID)
                        .header(AUTHORIZATION, format!("Bearer {}", token.unwrap()))
                        .send()
                        .await?
                        .json::<serde_json::Value>()
                        .await?;

                    if let Some(users_data) = users_response.get("data").and_then(|d| d.as_array())
                    {
                        // Create a map of user_id -> broadcaster_type
                        let mut broadcaster_types = std::collections::HashMap::new();
                        for user in users_data {
                            if let (Some(id), Some(broadcaster_type)) = (
                                user.get("id").and_then(|v| v.as_str()),
                                user.get("broadcaster_type").and_then(|v| v.as_str()),
                            ) {
                                if !broadcaster_type.is_empty() {
                                    broadcaster_types
                                        .insert(id.to_string(), broadcaster_type.to_string());
                                }
                            }
                        }

                        // Update streams with broadcaster types
                        for stream in &mut streams {
                            if let Some(broadcaster_type) = broadcaster_types.get(&stream.user_id) {
                                stream.broadcaster_type = Some(broadcaster_type.clone());
                            }
                        }
                    }
                }

                Ok((streams, next_cursor.map(|c| format!("helix:{}", c))))
            }
            None => Ok((Vec::new(), None)),
        }
    }

    pub async fn get_recommended_streams(_state: &AppState) -> Result<Vec<TwitchStream>> {
        let (streams, _) =
            Self::get_recommended_streams_paginated(_state, None, 20, Vec::new(), false).await?;

        Ok(streams)
    }

    pub async fn get_top_games(_state: &AppState, limit: u32) -> Result<Vec<CategoryRow>> {
        let (games, _) = Self::get_top_games_paginated(_state, None, limit).await?;
        Ok(games)
    }

    /// The category directory, most-watched first, each with Twitch's own live
    /// total (the number category search shows too): the public GQL, signed in
    /// or out. Helix's `games/top` answers when GQL fails on the first page, or
    /// for a Helix cursor, without viewers.
    pub async fn get_top_games_paginated(
        _state: &AppState,
        cursor: Option<String>,
        limit: u32,
    ) -> Result<(Vec<CategoryRow>, Option<String>)> {
        let limit = limit.clamp(1, 100);
        match (cursor.as_deref(), public_cursor(cursor.as_deref())) {
            (Some(helix), None) => Self::top_games_helix(Some(helix), limit).await,
            (_, after) => match Self::top_games_public(after, limit).await {
                Ok(page) => Ok(page),
                Err(e) if after.is_none() => {
                    debug!("[TwitchService] top games over GQL failed ({e}), asking Helix");
                    Self::top_games_helix(None, limit).await
                }
                Err(e) => Err(e),
            },
        }
    }

    /// A Helix `games/top` page (signed in), in Twitch's order, without viewers.
    async fn top_games_helix(after: Option<&str>, limit: u32) -> Result<(Vec<CategoryRow>, Option<String>)> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();
        let mut url = format!("https://api.twitch.tv/helix/games/top?first={}", limit);
        if let Some(after) = after {
            url.push_str(&format!("&after={}", urlencoding::encode(after)));
        }
        let response = client
            .get(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(anyhow::anyhow!("Helix top games: HTTP {}", response.status()));
        }
        let response = response.json::<serde_json::Value>().await?;
        let next = response
            .pointer("/pagination/cursor")
            .and_then(|c| c.as_str())
            .filter(|c| !c.is_empty())
            .map(String::from);
        let rows = response
            .get("data")
            .and_then(|d| d.as_array())
            .map(|rows| rows.iter().filter_map(CategoryRow::from_helix).collect())
            .unwrap_or_default();
        Ok((rows, next))
    }

    /// Live streams in any of several categories, most-watched first, in one
    /// Helix request (it takes up to 100 `game_id` values). A drop that counts
    /// in many categories (the Pokémon balls count in 21) is watched from here.
    pub async fn get_streams_in_categories(
        state: &AppState,
        category_ids: &[String],
        limit: u32,
    ) -> Result<Vec<TwitchStream>> {
        let ids: Vec<&str> = category_ids
            .iter()
            .map(String::as_str)
            .filter(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()))
            .take(100)
            .collect();
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let (streams, _) =
            Self::get_streams_by_game(state, &ids.join("&game_id="), None, limit).await?;
        Ok(streams)
    }

    pub async fn get_streams_by_game(
        _state: &AppState,
        game_id: &str,
        cursor: Option<String>,
        limit: u32,
    ) -> Result<(Vec<TwitchStream>, Option<String>)> {
        let token = Self::get_token().await.ok();
        if token.is_none() || public_cursor(cursor.as_deref()).is_some() {
            return Self::game_streams_public(game_id, public_cursor(cursor.as_deref()), limit).await;
        }
        let client = crate::services::http::client().clone();

        let mut url = format!(
            "https://api.twitch.tv/helix/streams?game_id={}&first={}",
            game_id, limit
        );
        if let Some(cursor) = cursor {
            url.push_str(&format!("&after={}", cursor));
        }

        let mut request = client.get(&url).header("Client-Id", CLIENT_ID);

        if let Some(token) = &token {
            request = request.header(AUTHORIZATION, format!("Bearer {}", token));
        }

        let response = request.send().await?;
        if !response.status().is_success() {
            debug!("[TwitchService] category streams: HTTP {}, asking the public GQL", response.status());
            return Self::game_streams_public(game_id, None, limit).await;
        }
        let response = response.json::<serde_json::Value>().await?;

        let next_cursor = response
            .get("pagination")
            .and_then(|p| p.get("cursor"))
            .and_then(|c| c.as_str())
            .map(|s| s.to_string());

        let data = response.get("data").and_then(|d| d.as_array());

        match data {
            Some(arr) => {
                let mut streams: Vec<TwitchStream> =
                    serde_json::from_value(serde_json::Value::Array(arr.clone()))?;

                if !streams.is_empty() && token.is_some() {
                    let user_ids: Vec<String> = streams.iter().map(|s| s.user_id.clone()).collect();
                    let user_ids_param = user_ids
                        .iter()
                        .map(|id| format!("id={}", id))
                        .collect::<Vec<_>>()
                        .join("&");

                    let users_url = format!("https://api.twitch.tv/helix/users?{}", user_ids_param);
                    let users_response = client
                        .get(&users_url)
                        .header("Client-Id", CLIENT_ID)
                        .header(AUTHORIZATION, format!("Bearer {}", token.unwrap()))
                        .send()
                        .await?
                        .json::<serde_json::Value>()
                        .await?;

                    if let Some(users_data) = users_response.get("data").and_then(|d| d.as_array())
                    {
                        let mut broadcaster_types = std::collections::HashMap::new();
                        for user in users_data {
                            if let (Some(id), Some(broadcaster_type)) = (
                                user.get("id").and_then(|v| v.as_str()),
                                user.get("broadcaster_type").and_then(|v| v.as_str()),
                            ) {
                                if !broadcaster_type.is_empty() {
                                    broadcaster_types
                                        .insert(id.to_string(), broadcaster_type.to_string());
                                }
                            }
                        }

                        for stream in &mut streams {
                            if let Some(broadcaster_type) = broadcaster_types.get(&stream.user_id) {
                                stream.broadcaster_type = Some(broadcaster_type.clone());
                            }
                        }
                    }
                }

                Ok((streams, next_cursor))
            }
            None => Ok((Vec::new(), None)),
        }
    }

    pub async fn search_channels(_state: &AppState, query: &str) -> Result<Vec<TwitchStream>> {
        let token = Self::get_token().await.ok();
        if token.is_none() {
            return Self::search_channels_public(query).await;
        }
        let client = crate::services::http::client().clone();

        let url = format!(
            "https://api.twitch.tv/helix/search/channels?query={}&first=60&live_only=false",
            urlencoding::encode(query)
        );

        let mut request = client.get(&url).header("Client-Id", CLIENT_ID);

        if let Some(token) = &token {
            request = request.header(AUTHORIZATION, format!("Bearer {}", token));
        }

        let response = request.send().await?;
        if !response.status().is_success() {
            debug!("[TwitchService] channel search: HTTP {}, asking the public GQL", response.status());
            return Self::search_channels_public(query).await;
        }
        let response = response.json::<serde_json::Value>().await?;

        let data = response.get("data").and_then(|d| d.as_array());

        match data {
            Some(arr) => {
                // Convert search results to TwitchStream format
                let mut streams = Vec::new();
                let mut user_ids = Vec::new();

                for channel in arr {
                    if let (
                        Some(id),
                        Some(user_id),
                        Some(user_name),
                        Some(user_login),
                        Some(title),
                    ) = (
                        channel.get("id").and_then(|v| v.as_str()),
                        channel.get("id").and_then(|v| v.as_str()), // Note: Search result 'id' is the user_id
                        channel.get("display_name").and_then(|v| v.as_str()),
                        channel.get("broadcaster_login").and_then(|v| v.as_str()),
                        channel.get("title").and_then(|v| v.as_str()),
                    ) {
                        let game_name = channel
                            .get("game_name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");

                        let thumbnail_url = channel
                            .get("thumbnail_url")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();

                        let broadcaster_type = channel
                            .get("broadcaster_type")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                            .map(|s| s.to_string());

                        user_ids.push(user_id.to_string());

                        streams.push(TwitchStream {
                            id: id.to_string(), // This is user_id, but search result doesn't provide stream_id
                            user_id: user_id.to_string(),
                            user_name: user_name.to_string(),
                            user_login: user_login.to_string(),
                            title: title.to_string(),
                            viewer_count: 0, // Will be populated from streams API
                            game_id: String::new(), // Will be populated from streams API
                            game_name: game_name.to_string(),
                            thumbnail_url: thumbnail_url.clone(),
                            started_at: channel
                                .get("started_at")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                            broadcaster_type,
                            profile_image_url: Some(thumbnail_url.clone()), // Preserve the actual profile picture from search
                            is_live: channel.get("is_live").and_then(|v| v.as_bool()),
                            tags: None,
                            language: None,
                            drops_earnable: false,
                        });
                    }
                }

                // Append exact match if query looks like a username, to patch Twitch's search algo filtering out inactive direct matches
                let trimmed_query = query.trim();
                if !trimmed_query.contains(' ')
                    && !streams
                        .iter()
                        .any(|s| s.user_login.eq_ignore_ascii_case(trimmed_query))
                {
                    if let Ok(exact_user) = Self::get_user_by_login(trimmed_query).await {
                        let synthesize = TwitchStream {
                            id: exact_user.id.clone(),
                            user_id: exact_user.id,
                            user_name: exact_user.display_name,
                            user_login: exact_user.login,
                            title: String::new(),
                            viewer_count: 0,
                            game_id: String::new(),
                            game_name: String::new(),
                            thumbnail_url: exact_user.profile_image_url.clone().unwrap_or_default(),
                            started_at: String::new(),
                            broadcaster_type: exact_user.broadcaster_type,
                            profile_image_url: exact_user.profile_image_url,
                            is_live: Some(false),
                            tags: None,
                            language: None,
                            drops_earnable: false,
                        };
                        user_ids.push(synthesize.user_id.clone());
                        streams.insert(0, synthesize);
                    }
                }

                // Fetch actual stream data to get viewer counts and accurate info
                if !user_ids.is_empty() {
                    // Map of user_id -> live stream data, filled across chunks.
                    let mut stream_data_map: std::collections::HashMap<String, serde_json::Value> =
                        std::collections::HashMap::new();
                    // The ids whose chunk Helix answered. For these the streams
                    // answer is the truth about liveness; search's own `is_live`
                    // lags and kept channels that had ended "live".
                    let mut checked: std::collections::HashSet<String> =
                        std::collections::HashSet::new();

                    // Helix takes at most 100 `user_id` params per call, and defaults
                    // `first` to 20. Without it a fully-live page of search results
                    // would only enrich its first 20 rows.
                    for chunk in user_ids.chunks(100) {
                        let user_ids_param = chunk
                            .iter()
                            .map(|id| format!("user_id={}", id))
                            .collect::<Vec<_>>()
                            .join("&");

                        let streams_url = format!(
                            "https://api.twitch.tv/helix/streams?{}&first=100",
                            user_ids_param
                        );

                        let mut streams_request =
                            client.get(&streams_url).header("Client-Id", CLIENT_ID);

                        if let Some(token) = &token {
                            streams_request =
                                streams_request.header(AUTHORIZATION, format!("Bearer {}", token));
                        }

                        if let Ok(streams_response) = streams_request.send().await {
                            if !streams_response.status().is_success() {
                                debug!(
                                    "[TwitchService] channel search: streams HTTP {}, liveness left to search",
                                    streams_response.status()
                                );
                                continue;
                            }
                            if let Ok(streams_json) =
                                streams_response.json::<serde_json::Value>().await
                            {
                                if let Some(streams_data) =
                                    streams_json.get("data").and_then(|d| d.as_array())
                                {
                                    checked.extend(chunk.iter().cloned());
                                    for stream_data in streams_data {
                                        if let Some(uid) =
                                            stream_data.get("user_id").and_then(|v| v.as_str())
                                        {
                                            stream_data_map
                                                .insert(uid.to_string(), stream_data.clone());
                                        }
                                    }
                                }
                            }
                        }
                    }

                    mark_search_liveness(&mut streams, &stream_data_map, &checked);

                    // Update our streams with actual stream data
                    for stream in &mut streams {
                        if let Some(stream_data) = stream_data_map.get(&stream.user_id) {
                            // Update viewer count
                            if let Some(viewer_count) =
                                stream_data.get("viewer_count").and_then(|v| v.as_u64())
                            {
                                stream.viewer_count = viewer_count as u32;
                            }

                            // Update stream ID (actual stream_id, not user_id)
                            if let Some(stream_id) = stream_data.get("id").and_then(|v| v.as_str())
                            {
                                stream.id = stream_id.to_string();
                            }

                            // Update thumbnail URL with actual stream thumbnail
                            if let Some(thumbnail) =
                                stream_data.get("thumbnail_url").and_then(|v| v.as_str())
                            {
                                stream.thumbnail_url = thumbnail.to_string();
                            }

                            // Update started_at if available
                            if let Some(started_at) =
                                stream_data.get("started_at").and_then(|v| v.as_str())
                            {
                                stream.started_at = started_at.to_string();
                            }
                        }
                    }
                }


                rank_search_results(query, &mut streams);
                Ok(streams)
            }
            None => Ok(Vec::new()),
        }
    }

    /// Follow a channel via Twitch GQL persisted mutation (FollowButton_FollowUser).
    /// Hash captured live from Twitch web client on 2025-03-25.
    ///
    /// Uses the Android-app client id paired with the device-login OAuth token from
    /// DropsAuthService. Twitch does not gate Android-client mutations behind
    /// Client-Integrity, so no browser-minted integrity token (and no hidden
    /// webview) is involved. The OAuth token and the Client-Id must be issued for
    /// the same app, which is why both come from the Android pair here.
    pub async fn follow_channel(target_user_id: &str) -> Result<()> {
        use crate::services::drops_auth_service::DropsAuthService;

        let token = DropsAuthService::get_token().await?;
        let client = crate::services::http::client().clone();

        const GQL_CLIENT_ID: &str = env!("TWITCH_ANDROID_CLIENT_ID");

        let payload = serde_json::json!({
            "operationName": "FollowButton_FollowUser",
            "variables": {
                "input": {
                    "disableNotifications": false,
                    "targetID": target_user_id
                }
            },
            "extensions": {
                "persistedQuery": {
                    "version": 1,
                    "sha256Hash": "800e7346bdf7e5278a3c1d3f21b2b56e2639928f86815677a7126b093b2fdd08"
                }
            }
        });

        debug!(
            "[follow_channel] Sending GQL FollowButton_FollowUser for target: {}",
            target_user_id
        );

        let response = client
            .post("https://gql.twitch.tv/gql")
            .header("Client-Id", GQL_CLIENT_ID)
            .header(AUTHORIZATION, format!("OAuth {}", token))
            .header(ACCEPT, "*/*")
            .json(&payload)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await?;
            return Err(anyhow::anyhow!(
                "GQL follow failed (HTTP {}): {}",
                status,
                error_text
            ));
        }

        let body: serde_json::Value = response.json().await?;

        // Check for GQL-level errors
        if let Some(errors) = body.get("errors") {
            return Err(anyhow::anyhow!("GQL follow errors: {:?}", errors));
        }

        debug!(
            "[follow_channel] Successfully followed user {}",
            target_user_id
        );
        Ok(())
    }

    /// Unfollow a channel via Twitch GQL persisted mutation (FollowButton_UnfollowUser).
    /// Hash captured live from Twitch web client on 2025-03-25.
    ///
    /// Same Android-client credential as `follow_channel`: the DropsAuthService
    /// device-login token plus the Android Client-Id, integrity-free.
    pub async fn unfollow_channel(target_user_id: &str) -> Result<()> {
        use crate::services::drops_auth_service::DropsAuthService;

        let token = DropsAuthService::get_token().await?;
        let client = crate::services::http::client().clone();

        const GQL_CLIENT_ID: &str = env!("TWITCH_ANDROID_CLIENT_ID");

        let payload = serde_json::json!({
            "operationName": "FollowButton_UnfollowUser",
            "variables": {
                "input": {
                    "targetID": target_user_id
                }
            },
            "extensions": {
                "persistedQuery": {
                    "version": 1,
                    "sha256Hash": "f7dae976ebf41c755ae2d758546bfd176b4eeb856656098bb40e0a672ca0d880"
                }
            }
        });

        debug!(
            "[unfollow_channel] Sending GQL FollowButton_UnfollowUser for target: {}",
            target_user_id
        );

        let response = client
            .post("https://gql.twitch.tv/gql")
            .header("Client-Id", GQL_CLIENT_ID)
            .header(AUTHORIZATION, format!("OAuth {}", token))
            .header(ACCEPT, "*/*")
            .json(&payload)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await?;
            return Err(anyhow::anyhow!(
                "GQL unfollow failed (HTTP {}): {}",
                status,
                error_text
            ));
        }

        let body: serde_json::Value = response.json().await?;

        if let Some(errors) = body.get("errors") {
            return Err(anyhow::anyhow!("GQL unfollow errors: {:?}", errors));
        }

        debug!(
            "[unfollow_channel] Successfully unfollowed user {}",
            target_user_id
        );
        Ok(())
    }

    /// Fetch pinned chat messages for a channel via Twitch GQL (GetPinnedChat).
    /// Pins are public: the read goes out anonymously, so they show for a
    /// viewer without the Drops sign-in, live or offline.
    pub async fn get_pinned_chat_messages(channel_id: &str) -> Result<Vec<serde_json::Value>> {
        let client = crate::services::http::client().clone();

        let payload = serde_json::json!({
            "operationName": "GetPinnedChat",
            "variables": {
                "channelID": channel_id,
                "count": 10
            },
            "extensions": {
                "persistedQuery": {
                    "version": 1,
                    "sha256Hash": "2d099d4c9b6af80a07d8440140c4f3dbb04d516b35c401aab7ce8f60765308d5"
                }
            }
        });

        debug!(
            "[get_pinned_chat] Fetching pinned messages for channel: {}",
            channel_id
        );

        let body = Self::gql_public_read(payload).await?;

        // Parse pinned messages and collect unique user IDs for avatar lookup
        let mut raw_pins = Vec::new();
        let mut user_ids = std::collections::HashSet::new();

        if let Some(edges) = body
            .pointer("/data/channel/pinnedChatMessages/edges")
            .and_then(|v| v.as_array())
        {
            for edge in edges {
                if let Some(node) = edge.get("node") {
                    let sender_id = node
                        .pointer("/pinnedMessage/sender/id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let pinned_by_id = node
                        .pointer("/pinnedBy/id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");

                    if !sender_id.is_empty() {
                        user_ids.insert(sender_id.to_string());
                    }
                    if !pinned_by_id.is_empty() {
                        user_ids.insert(pinned_by_id.to_string());
                    }

                    // Extract badges array, in the order chat draws them
                    let mut badges: Vec<serde_json::Value> = node
                        .pointer("/pinnedMessage/sender/displayBadges")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter().map(|b| {
                            serde_json::json!({
                                "set_id": b.get("setID").and_then(|v| v.as_str()).unwrap_or(""),
                                "version": b.get("version").and_then(|v| v.as_str()).unwrap_or("1"),
                            })
                        }).collect()
                        })
                        .unwrap_or_default();
                    badges.sort_by_key(|b| {
                        crate::models::chat_layout::twitch_badge_rank(
                            b.get("set_id").and_then(|v| v.as_str()).unwrap_or(""),
                        )
                    });

                    raw_pins.push((
                        node.clone(),
                        sender_id.to_string(),
                        pinned_by_id.to_string(),
                        badges,
                    ));
                }
            }
        }

        // Batch fetch profile images via Helix API for all unique user IDs
        let mut avatar_map: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        // Logins from the same lookup: the pin names its sender by display
        // name only, and the shown name needs the login beside it.
        let mut login_map: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        if !user_ids.is_empty() {
            if let Ok(helix_token) = Self::get_token().await {
                let ids: Vec<&str> = user_ids.iter().map(|s| s.as_str()).collect();
                let mut url = String::from("https://api.twitch.tv/helix/users?");
                for (i, id) in ids.iter().enumerate() {
                    if i > 0 {
                        url.push('&');
                    }
                    url.push_str(&format!("id={}", id));
                }

                if let Ok(resp) = client
                    .get(&url)
                    .header("Client-ID", CLIENT_ID)
                    .header(AUTHORIZATION, format!("Bearer {}", helix_token))
                    .send()
                    .await
                {
                    if let Ok(data) = resp.json::<serde_json::Value>().await {
                        if let Some(users) = data.get("data").and_then(|v| v.as_array()) {
                            for user in users {
                                if let (Some(id), Some(login)) = (
                                    user.get("id").and_then(|v| v.as_str()),
                                    user.get("login").and_then(|v| v.as_str()),
                                ) {
                                    login_map.insert(id.to_string(), login.to_string());
                                }
                                if let (Some(id), Some(img)) = (
                                    user.get("id").and_then(|v| v.as_str()),
                                    user.get("profile_image_url").and_then(|v| v.as_str()),
                                ) {
                                    avatar_map.insert(id.to_string(), img.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }

        // Build final response with avatars
        let mut pinned_messages: Vec<serde_json::Value> = raw_pins.iter().map(|(node, sender_id, pinned_by_id, badges)| {
            let sender_name = node.pointer("/pinnedMessage/sender/displayName").and_then(|v| v.as_str()).unwrap_or("Unknown");
            let sender_login = login_map.get(sender_id).map(String::as_str).unwrap_or("");
            serde_json::json!({
                "id": node.get("id").and_then(|v| v.as_str()).unwrap_or(""),
                // The underlying chat message id (NOT the pin-record id above), so the
                // client can tell which live chat message is currently pinned.
                "message_id": node.pointer("/pinnedMessage/id").and_then(|v| v.as_str()).unwrap_or(""),
                "type": node.get("type").and_then(|v| v.as_str()).unwrap_or("MOD"),
                "message_text": node.pointer("/pinnedMessage/content/text").and_then(|v| v.as_str()).unwrap_or(""),
                // The same text split into text and links as a chat row is, so the
                // page runs no link pattern of its own.
                "segments": crate::services::text_spans::pin_segments(
                    node.pointer("/pinnedMessage/content/text").and_then(|v| v.as_str()).unwrap_or(""),
                ),
                "sender_id": sender_id,
                "sender_name": sender_name,
                "sender_login": sender_login,
                // The same name the sender's chat rows show.
                "sender_shown_name": crate::services::chatter_name::shown_name("twitch", sender_login, sender_name),
                "sender_color": node.pointer("/pinnedMessage/sender/chatColor").and_then(|v| v.as_str()).unwrap_or("#FFFFFF"),
                "sender_avatar": avatar_map.get(sender_id).cloned().unwrap_or_default(),
                "sender_badges": badges,
                "pinned_by": node.pointer("/pinnedBy/displayName").and_then(|v| v.as_str()).unwrap_or(""),
                "pinned_by_id": pinned_by_id,
                "pinned_by_avatar": avatar_map.get(pinned_by_id).cloned().unwrap_or_default(),
                "started_at": node.get("startsAt").and_then(|v| v.as_str()).unwrap_or(""),
            })
        }).collect();
        newest_pin_first(&mut pinned_messages);
        // A native shell draws the sender as its chat rows do, with art Rust resolved.
        if crate::services::chat_row_view::enabled() {
            crate::services::chat_row_view::stamp_pins(&mut pinned_messages, channel_id);
        }

        debug!(
            "[get_pinned_chat] Found {} pinned messages for channel {}",
            pinned_messages.len(),
            channel_id
        );
        Ok(pinned_messages)
    }

    pub async fn get_all_followed_channels(
        limit: u32,
        cursor: Option<String>,
    ) -> Result<(Vec<TwitchStream>, Option<String>)> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();

        let user_info = Self::get_user_info().await?;

        let mut url = format!(
            "https://api.twitch.tv/helix/channels/followed?user_id={}&first={}",
            user_info.id, limit
        );
        if let Some(c) = cursor {
            url.push_str(&format!("&after={}", c));
        }

        let response = client
            .get(&url)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .header("Client-Id", CLIENT_ID)
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        let next_cursor = response
            .get("pagination")
            .and_then(|p| p.get("cursor"))
            .and_then(|c| c.as_str())
            .map(|s| s.to_string());

        let data = response.get("data").and_then(|d| d.as_array());

        match data {
            Some(arr) => {
                let mut streams = Vec::new();
                let mut user_ids = Vec::new();

                for channel in arr {
                    if let (
                        Some(broadcaster_id),
                        Some(broadcaster_login),
                        Some(broadcaster_name),
                        Some(followed_at),
                    ) = (
                        channel.get("broadcaster_id").and_then(|v| v.as_str()),
                        channel.get("broadcaster_login").and_then(|v| v.as_str()),
                        channel.get("broadcaster_name").and_then(|v| v.as_str()),
                        channel.get("followed_at").and_then(|v| v.as_str()),
                    ) {
                        user_ids.push(broadcaster_id.to_string());

                        streams.push(TwitchStream {
                            id: broadcaster_id.to_string(),
                            user_id: broadcaster_id.to_string(),
                            user_name: broadcaster_name.to_string(),
                            user_login: broadcaster_login.to_string(),
                            title: String::new(),
                            viewer_count: 0,
                            game_id: String::new(),
                            game_name: String::new(),
                            thumbnail_url: String::new(),
                            started_at: followed_at.to_string(), // use started_at to store followed_at temporarily
                            broadcaster_type: None,
                            profile_image_url: None,
                            is_live: Some(false),
                            tags: None,
                            language: None,
                            drops_earnable: false,
                        });
                    }
                }

                // Fetch actual user avatars via helix/users
                if !user_ids.is_empty() {
                    // Split into chunks of 100 (Twitch API max) just in case
                    for chunk in user_ids.chunks(100) {
                        let user_ids_param = chunk
                            .iter()
                            .map(|id| format!("id={}", id))
                            .collect::<Vec<_>>()
                            .join("&");

                        let users_url =
                            format!("https://api.twitch.tv/helix/users?{}", user_ids_param);
                        if let Ok(users_response) = client
                            .get(&users_url)
                            .header("Client-Id", CLIENT_ID)
                            .header(AUTHORIZATION, format!("Bearer {}", token))
                            .send()
                            .await
                        {
                            if let Ok(users_json) = users_response.json::<serde_json::Value>().await
                            {
                                if let Some(users_data) =
                                    users_json.get("data").and_then(|d| d.as_array())
                                {
                                    let mut profiles = std::collections::HashMap::new();
                                    for user in users_data {
                                        if let (Some(id), Some(profile_image_url)) = (
                                            user.get("id").and_then(|v| v.as_str()),
                                            user.get("profile_image_url").and_then(|v| v.as_str()),
                                        ) {
                                            profiles.insert(
                                                id.to_string(),
                                                profile_image_url.to_string(),
                                            );
                                        }
                                    }

                                    for stream in &mut streams {
                                        if let Some(avatar) = profiles.get(&stream.user_id) {
                                            stream.profile_image_url = Some(avatar.clone());
                                            stream.thumbnail_url = avatar.clone();
                                            // use avatar as thumbnail fallback
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                Ok((streams, next_cursor))
            }
            None => Ok((Vec::new(), None)),
        }
    }

    pub async fn get_offline_last_broadcasts(
        user_ids: Vec<String>,
    ) -> Result<std::collections::HashMap<String, Option<String>>> {
        let client = crate::services::http::client().clone();
        // create the query
        let query = format!(
            r#"query {{ users(ids: {}) {{ id lastBroadcast {{ startedAt }} videos(first: 1, sort: TIME) {{ edges {{ node {{ createdAt lengthSeconds }} }} }} }} }}"#,
            serde_json::to_string(&user_ids).unwrap_or_else(|_| "[]".to_string())
        );

        let payload = serde_json::json!([{
            "operationName": null,
            "variables": {},
            "query": query
        }]);

        // IMPORTANT: Unauthenticated GQL requires the Twitch Web Client-ID, NOT the Helix Developer application Client-ID
        let response = client
            .post("https://gql.twitch.tv/gql")
            .header("Client-ID", env!("TWITCH_WEB_CLIENT_ID"))
            .json(&payload)
            .send()
            .await?;

        let text = response.text().await?;
        let json: serde_json::Value = serde_json::from_str(&text)?;

        let mut result_map = std::collections::HashMap::new();

        if let Some(arr) = json.as_array() {
            if let Some(users_data) = arr
                .first()
                .and_then(|obj| obj.get("data"))
                .and_then(|d| d.get("users"))
                .and_then(|u| u.as_array())
            {
                for user in users_data {
                    if let Some(id) = user.get("id").and_then(|i| i.as_str()) {
                        let mut final_time = None;

                        // Try calculating exact true end time using their most recent VOD/Stream duration
                        if let Some(edges) = user
                            .get("videos")
                            .and_then(|v| v.get("edges"))
                            .and_then(|e| e.as_array())
                        {
                            if let Some(node) = edges.first().and_then(|e| e.get("node")) {
                                if let (Some(created_at), Some(length)) = (
                                    node.get("createdAt").and_then(|c| c.as_str()),
                                    node.get("lengthSeconds").and_then(|l| l.as_i64()),
                                ) {
                                    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(created_at)
                                    {
                                        let end_time = dt + ChronoDuration::seconds(length);
                                        // Format back to RFC3339 string so frontend can trivially parse it using new Date()
                                        final_time = Some(end_time.to_rfc3339());
                                    }
                                }
                            }
                        }

                        // Fallback to start time for streamers who completely disable all VOD recordings
                        if final_time.is_none() {
                            final_time = user
                                .get("lastBroadcast")
                                .and_then(|lb| lb.get("startedAt"))
                                .and_then(|sa| sa.as_str())
                                .map(|s| s.to_string());
                        }

                        result_map.insert(id.to_string(), final_time);
                    }
                }
            }
        }

        debug!(
            "[TWITCH] Parsed {} last broadcast timestamps",
            result_map.len()
        );
        Ok(result_map)
    }

    pub async fn check_following_status(target_user_id: &str) -> Result<bool> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();

        // Get the current user's ID
        let user_info = Self::get_user_info().await?;

        // Use the new channels/followed endpoint (the old users/follows was deprecated)
        // This endpoint checks if user_id follows broadcaster_id
        let url = format!(
            "https://api.twitch.tv/helix/channels/followed?user_id={}&broadcaster_id={}",
            user_info.id, target_user_id
        );

        let response = client
            .get(&url)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .header("Client-Id", CLIENT_ID)
            .send()
            .await?;

        // Check if request was successful
        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            debug!(
                "[check_following_status] API error: {} - {}",
                status, error_text
            );
            // Return false if we can't check (might not be following)
            return Ok(false);
        }

        let json: serde_json::Value = response.json().await?;

        // If data array has items, user is following
        // The new endpoint returns total in the response and data array with the follow info
        let is_following = json
            .get("data")
            .and_then(|d| d.as_array())
            .map(|arr| !arr.is_empty())
            .unwrap_or(false);

        debug!(
            "[check_following_status] User {} following {}: {}",
            user_info.id, target_user_id, is_following
        );

        Ok(is_following)
    }

    /// Check if a specific stream is currently online by user login
    /// Returns the stream data if online, None if offline
    pub async fn check_stream_online(user_login: &str) -> Result<Option<TwitchStream>> {
        let token = Self::get_token().await.ok();
        let client = crate::services::http::client().clone();

        let url = format!(
            "https://api.twitch.tv/helix/streams?user_login={}",
            urlencoding::encode(user_login)
        );

        let mut request = client.get(&url).header("Client-Id", CLIENT_ID);

        if let Some(token) = &token {
            request = request.header(AUTHORIZATION, format!("Bearer {}", token));
        }

        let response = request.send().await?;
        // A refusal (signed out, an expired token, a rate limit) carries no
        // `data`; reading it as "offline" would call a live channel offline.
        // The public GQL answers the same question without an account.
        let status = response.status();
        if !status.is_success() {
            debug!("[TwitchService] stream check for {user_login}: HTTP {status}, asking the public GQL");
            let rows = Self::get_live_public(&[], &[user_login.to_string()]).await?;
            return Ok(rows.into_iter().next());
        }
        let response = response.json::<serde_json::Value>().await?;

        let data = response.get("data").and_then(|d| d.as_array());

        match data {
            Some(arr) if !arr.is_empty() => {
                let stream: TwitchStream = serde_json::from_value(arr[0].clone())?;
                Ok(Some(stream))
            }
            _ => Ok(None), // Stream is offline
        }
    }

    /// A Twitch login: 1..25 of lowercase ASCII letters, digits and underscore.
    /// Checked before a login from a deep link or a search box reaches a query
    /// string.
    fn valid_login(login: &str) -> bool {
        (1..=25).contains(&login.len())
            && login
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    }

    /// One login → one enriched stream row (avatar, partner mark, liveness;
    /// the channel's title and category when offline). The single lookup
    /// every non-list open path uses, so a stream reached by a raid, a
    /// notification tap, a deep link or a search has the same shape as one
    /// opened from Following.
    pub async fn resolve_stream_for_login(login: &str) -> Result<TwitchStream> {
        let login = login.trim().to_lowercase();
        if !Self::valid_login(&login) {
            return Err(anyhow::anyhow!("'{}' is not a Twitch login", login));
        }
        let user = Self::get_user_by_login(&login).await?;
        let live = Self::check_stream_online(&login).await;
        let channel = match &live {
            Ok(Some(_)) => None,
            _ => match Self::get_token().await {
                Ok(token) => Self::channel_row_by_id(&user.id, &token).await.ok(),
                Err(_) => None,
            },
        };
        Ok(Self::fuse_stream_row(user, &login, live, channel))
    }

    /// Pure merge of the three Helix answers into the row shape Following
    /// hands over. A FAILED liveness probe is `is_live: None`, never `false`:
    /// the chat header treats `false` as offline, and a transient 5xx on a
    /// raid must not paint a live channel dark.
    fn fuse_stream_row(
        user: UserInfo,
        login: &str,
        live: Result<Option<TwitchStream>>,
        channel: Option<ChannelInfo>,
    ) -> TwitchStream {
        let avatar = user.profile_image_url.clone().filter(|u| !u.is_empty());
        let kind = user.broadcaster_type.clone().filter(|t| !t.is_empty());
        let probe_failed = live.is_err();
        match live {
            Ok(Some(mut row)) => {
                row.is_live = Some(true);
                row.profile_image_url = avatar;
                row.broadcaster_type = kind;
                row
            }
            _ => TwitchStream {
                id: String::new(),
                user_id: user.id,
                user_name: user.display_name,
                user_login: login.to_string(),
                title: channel.as_ref().map(|c| c.title.clone()).unwrap_or_default(),
                viewer_count: 0,
                game_id: String::new(),
                game_name: channel
                    .as_ref()
                    .map(|c| c.game_name.clone())
                    .unwrap_or_default(),
                thumbnail_url: String::new(),
                started_at: String::new(),
                broadcaster_type: kind,
                profile_image_url: avatar,
                is_live: if probe_failed { None } else { Some(false) },
                tags: None,
                language: None,
                drops_earnable: false,
            },
        }
    }

    /// Batched liveness check. Helix `streams` accepts up to 100 `user_login`
    /// params per call, so an N-login allow-list resolves in ceil(N/100) requests
    /// instead of N. Offline logins are simply absent from the response, so the
    /// returned vec is exactly the live subset. This replaces firing one
    /// `check_stream_online` per channel (an ACL like EWC has ~1180 channels; the
    /// per-channel fan-out rate-limited and silently dropped live channels).
    pub async fn check_streams_online(user_logins: &[String]) -> Result<Vec<TwitchStream>> {
        let token = Self::get_token().await.ok();
        let client = crate::services::http::client().clone();

        let mut live: Vec<TwitchStream> = Vec::new();

        for chunk in user_logins.chunks(100) {
            // ?first=100&user_login=a&user_login=b... — first=100 so a fully-live
            // chunk isn't truncated by Helix's default page size of 20.
            let params: String = chunk
                .iter()
                .map(|l| format!("user_login={}", urlencoding::encode(l)))
                .collect::<Vec<_>>()
                .join("&");
            let url = format!("https://api.twitch.tv/helix/streams?first=100&{}", params);

            let mut request = client.get(&url).header("Client-Id", CLIENT_ID);
            if let Some(token) = &token {
                request = request.header(AUTHORIZATION, format!("Bearer {}", token));
            }

            let response = request.send().await?;
            // A refusal (signed out, an expired token) carries no `data`, which
            // would read as every channel offline: ask the public GQL instead.
            if !response.status().is_success() {
                debug!("[TwitchService] streams batch: HTTP {}, asking the public GQL", response.status());
                live.extend(Self::get_live_public(&[], chunk).await?);
                continue;
            }
            let response = response.json::<serde_json::Value>().await?;
            if let Some(arr) = response.get("data").and_then(|d| d.as_array()) {
                for item in arr {
                    if let Ok(stream) = serde_json::from_value::<TwitchStream>(item.clone()) {
                        live.push(stream);
                    }
                }
            }
        }

        Ok(live)
    }

    /// Get the game ID by game name
    pub async fn get_game_id_by_name(game_name: &str) -> Result<Option<String>> {
        let token = Self::get_token().await.ok();
        if token.is_none() {
            return Self::game_id_by_name_public(game_name).await;
        }
        let client = crate::services::http::client().clone();

        let url = format!(
            "https://api.twitch.tv/helix/games?name={}",
            urlencoding::encode(game_name)
        );

        let mut request = client.get(&url).header("Client-Id", CLIENT_ID);

        if let Some(token) = &token {
            request = request.header(AUTHORIZATION, format!("Bearer {}", token));
        }

        let response = request.send().await?;
        if !response.status().is_success() {
            debug!("[TwitchService] category lookup: HTTP {}, asking the public GQL", response.status());
            return Self::game_id_by_name_public(game_name).await;
        }
        let response = response.json::<serde_json::Value>().await?;

        let game_id = response
            .get("data")
            .and_then(|d| d.as_array())
            .and_then(|arr| arr.first())
            .and_then(|game| game.get("id"))
            .and_then(|id| id.as_str())
            .map(|s| s.to_string());

        Ok(game_id)
    }

    /// Resolve category ids → their objects (`{id, name, box_art_url}`) via Helix
    /// Get Games (up to 100 ids per call). Used to label a streamer's clips with
    /// the category they were taken in (clips carry `game_id`, not the name).
    pub async fn get_games_by_ids(ids: &[String]) -> Result<Vec<serde_json::Value>> {
        let query: Vec<String> = ids
            .iter()
            .filter(|id| !id.is_empty())
            .take(100)
            .map(|id| format!("id={}", urlencoding::encode(id)))
            .collect();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let token = Self::get_token().await.ok();
        let client = crate::services::http::client().clone();
        let url = format!("https://api.twitch.tv/helix/games?{}", query.join("&"));

        let mut request = client.get(&url).header("Client-Id", CLIENT_ID);
        if let Some(token) = &token {
            request = request.header(AUTHORIZATION, format!("Bearer {}", token));
        }
        let response = request.send().await?.json::<serde_json::Value>().await?;

        Ok(response
            .get("data")
            .and_then(|d| d.as_array())
            .cloned()
            .unwrap_or_default())
    }

    /// Categories matching what was typed, ranked by `rank_category_results`,
    /// each with its live viewers. One public GQL request answers both Twitch's
    /// fuzzy search and the category named exactly what was typed, which the
    /// search alone can miss or bury ("art" lists "Artifact" before "Art"). It
    /// needs no account. When GQL fails, Helix's search answers (signed in),
    /// with the exact name looked up beside it, and without viewers.
    pub async fn search_categories(query: &str, limit: u32) -> Result<Vec<CategoryRow>> {
        let query = query.trim();
        let limit = limit.clamp(1, 100);
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let rows = match Self::search_categories_public(query, limit).await {
            Ok(rows) => rows,
            Err(e) => {
                debug!("[TwitchService] category search over GQL failed ({e}), asking Helix");
                let exact = Self::categories_by_name(&[query.to_string()]).await.unwrap_or_default();
                let fuzzy = Self::search_categories_helix(query, limit).await?;
                exact.iter().chain(fuzzy.iter()).filter_map(CategoryRow::from_helix).collect()
            }
        };
        Ok(rank_category_results(query, rows, limit as usize))
    }

    /// Helix's fuzzy category search, as raw rows (no viewers).
    async fn search_categories_helix(query: &str, limit: u32) -> Result<Vec<serde_json::Value>> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();

        let url = format!(
            "https://api.twitch.tv/helix/search/categories?query={}&first={}",
            urlencoding::encode(query),
            limit
        );

        let response = client
            .get(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(anyhow::anyhow!("Helix category search: HTTP {}", response.status()));
        }
        let response = response.json::<serde_json::Value>().await?;

        let data = response
            .get("data")
            .and_then(|d| d.as_array())
            .cloned()
            .unwrap_or_default();

        Ok(data)
    }

    /// Look up categories by EXACT name, many at a time.
    ///
    /// `search/categories` is a fuzzy search, so it answers "something like
    /// this" and cannot tell a caller whether a given string is genuinely a
    /// category. This one can: Helix `games?name=` matches exactly and takes up
    /// to 100 names per request, which is what makes it affordable to test a
    /// whole list of candidate names at once. Names Twitch doesn't know are
    /// simply absent from the response.
    pub async fn categories_by_name(names: &[String]) -> Result<Vec<serde_json::Value>> {
        if names.is_empty() {
            return Ok(Vec::new());
        }
        let token = Self::get_token().await.ok();
        let client = crate::services::http::client().clone();

        let mut found = Vec::new();
        for chunk in names.chunks(100) {
            let query = chunk
                .iter()
                .map(|n| format!("name={}", urlencoding::encode(n)))
                .collect::<Vec<_>>()
                .join("&");
            let url = format!("https://api.twitch.tv/helix/games?{}", query);

            let mut request = client.get(&url).header("Client-Id", CLIENT_ID);
            if let Some(token) = &token {
                request = request.header(AUTHORIZATION, format!("Bearer {}", token));
            }

            let response = request.send().await?.json::<serde_json::Value>().await?;
            if let Some(data) = response.get("data").and_then(|d| d.as_array()) {
                found.extend(data.iter().cloned());
            }
        }

        Ok(found)
    }

    /// Send a whisper message to another user
    /// Requires user:manage:whispers scope
    pub async fn send_whisper(to_user_id: &str, message: &str) -> Result<()> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();

        // Get the current user's ID
        let user_info = Self::get_user_info().await?;

        let response = client
            .post(format!(
                "https://api.twitch.tv/helix/whispers?from_user_id={}&to_user_id={}",
                user_info.id, to_user_id
            ))
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .header("Client-Id", CLIENT_ID)
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "message": message
            }))
            .send()
            .await
            .map_err(|e| {
                log::warn!("[Whisper] send request failed: {e}");
                anyhow::anyhow!(crate::services::whisper_inbox::send_failure_reason(None, ""))
            })?;

        let status = response.status();
        if status.is_success() {
            Ok(())
        } else {
            // Twitch's reason, in the words every platform shows (the raw body
            // stays in the log).
            let error_text = response.text().await.unwrap_or_default();
            log::warn!("[Whisper] Twitch refused a whisper ({status}): {error_text}");
            Err(anyhow::anyhow!(crate::services::whisper_inbox::send_failure_reason(
                Some(status.as_u16()),
                &error_text
            )))
        }
    }

    /// Get streams by game name (convenience method that resolves game name to ID)
    /// Returns streams sorted by viewer count (highest first)
    pub async fn get_streams_by_game_name(
        state: &AppState,
        game_name: &str,
        exclude_user_login: Option<&str>,
        cursor: Option<&str>,
        limit: u32,
    ) -> Result<(Vec<TwitchStream>, Option<String>)> {
        // Resolve the display name to a category id, then reuse the id-based path.
        let game_id = match Self::get_game_id_by_name(game_name).await? {
            Some(id) => id,
            None => return Ok((Vec::new(), None)), // Game not found
        };

        Self::get_streams_by_game_id(state, &game_id, exclude_user_login, cursor, limit).await
    }

    /// Fetch live streams in a category by its id, skipping the name→id lookup.
    /// Prefer this when the caller already holds the category id (e.g. the live
    /// `game_id` carried on the stream being watched).
    pub async fn get_streams_by_game_id(
        _state: &AppState,
        game_id: &str,
        exclude_user_login: Option<&str>,
        cursor: Option<&str>,
        limit: u32,
    ) -> Result<(Vec<TwitchStream>, Option<String>)> {
        let token = Self::get_token().await.ok();
        let public = |after: Option<String>| async move {
            let (mut streams, next) = Self::game_streams_public(game_id, after.as_deref(), limit).await?;
            if let Some(exclude_login) = exclude_user_login {
                streams.retain(|s| !s.user_login.eq_ignore_ascii_case(exclude_login));
            }
            Ok::<_, anyhow::Error>((streams, next))
        };
        if token.is_none() || public_cursor(cursor).is_some() {
            return public(public_cursor(cursor).map(String::from)).await;
        }
        let client = crate::services::http::client().clone();

        let mut url = format!(
            "https://api.twitch.tv/helix/streams?game_id={}&first={}",
            game_id, limit
        );

        if let Some(c) = cursor {
            url.push_str(&format!("&after={}", c));
        }

        let mut request = client.get(&url).header("Client-Id", CLIENT_ID);

        if let Some(token) = &token {
            request = request.header(AUTHORIZATION, format!("Bearer {}", token));
        }

        let response = request.send().await?;
        if !response.status().is_success() {
            debug!("[TwitchService] category streams: HTTP {}, asking the public GQL", response.status());
            return public(None).await;
        }
        let response = response.json::<serde_json::Value>().await?;

        let data = response.get("data").and_then(|d| d.as_array());

        match data {
            Some(arr) => {
                let mut streams: Vec<TwitchStream> =
                    serde_json::from_value(serde_json::Value::Array(arr.clone()))?;

                // Filter out the excluded user if provided
                if let Some(exclude_login) = exclude_user_login {
                    streams.retain(|s| s.user_login.to_lowercase() != exclude_login.to_lowercase());
                }

                // Fetch broadcaster types for all streams in a batch
                if !streams.is_empty() && token.is_some() {
                    let user_ids: Vec<String> = streams.iter().map(|s| s.user_id.clone()).collect();
                    let user_ids_param = user_ids
                        .iter()
                        .map(|id| format!("id={}", id))
                        .collect::<Vec<_>>()
                        .join("&");

                    let users_url = format!("https://api.twitch.tv/helix/users?{}", user_ids_param);
                    let users_response = client
                        .get(&users_url)
                        .header("Client-Id", CLIENT_ID)
                        .header(AUTHORIZATION, format!("Bearer {}", token.unwrap()))
                        .send()
                        .await?
                        .json::<serde_json::Value>()
                        .await?;

                    if let Some(users_data) = users_response.get("data").and_then(|d| d.as_array())
                    {
                        let mut broadcaster_types = std::collections::HashMap::new();
                        let mut profile_images = std::collections::HashMap::new();
                        for user in users_data {
                            if let Some(id) = user.get("id").and_then(|v| v.as_str()) {
                                if let Some(broadcaster_type) =
                                    user.get("broadcaster_type").and_then(|v| v.as_str())
                                {
                                    if !broadcaster_type.is_empty() {
                                        broadcaster_types
                                            .insert(id.to_string(), broadcaster_type.to_string());
                                    }
                                }
                                if let Some(profile_image) =
                                    user.get("profile_image_url").and_then(|v| v.as_str())
                                {
                                    profile_images
                                        .insert(id.to_string(), profile_image.to_string());
                                }
                            }
                        }

                        for stream in &mut streams {
                            if let Some(broadcaster_type) = broadcaster_types.get(&stream.user_id) {
                                stream.broadcaster_type = Some(broadcaster_type.clone());
                            }
                            if let Some(profile_image) = profile_images.get(&stream.user_id) {
                                stream.profile_image_url = Some(profile_image.clone());
                            }
                        }
                    }
                }

                // Extract pagination cursor
                let pagination_cursor = response
                    .get("pagination")
                    .and_then(|p| p.get("cursor"))
                    .and_then(|c| c.as_str())
                    .map(|s| s.to_string());

                // Streams are already sorted by viewer count (highest first) from the API
                Ok((streams, pagination_cursor))
            }
            None => Ok((Vec::new(), None)),
        }
    }

    pub async fn get_category_info(
        game_name: &str,
    ) -> Result<Option<crate::models::stream::CategoryInfo>> {
        let client = crate::services::http::client().clone();
        let query = "query($name: String!) { game(name: $name) { id name displayName description followersCount boxArtURL tags(tagType: CONTENT) { id localizedName } } }";

        let body = serde_json::json!({
            "query": query,
            "variables": { "name": game_name }
        });

        let response = client
            .post("https://gql.twitch.tv/gql")
            .header("Client-Id", TWITCH_GQL_CLIENT_ID)
            .json(&body)
            .send()
            .await?;

        if response.status().is_success() {
            let data: crate::models::stream::GqlDataResponse = response.json().await?;
            if let Some(game_data) = data.data {
                return Ok(game_data.game);
            }
        }

        Ok(None)
    }

    /// Live streams in a category filtered by freeform tags, server-side, via
    /// GQL. Helix can't filter streams by tag, so this is the only way to match
    /// Twitch's directory tag filter (returns every matching stream by viewer
    /// count, not just whatever happens to be on the loaded Helix pages). `tags`
    /// are tag display names (e.g. "Speedrun"); a stream must carry all of them.
    pub async fn get_streams_by_game_with_tags(
        game_name: &str,
        tags: Vec<String>,
        cursor: Option<String>,
        limit: u32,
    ) -> Result<(Vec<TwitchStream>, Option<String>)> {
        let client = crate::services::http::client().clone();

        // `options.freeformTags` is the field that actually filters by freeform
        // tag NAME (case-insensitive), server-side, ranked by viewer count — this
        // is Twitch's directory tag filter. The plain `tags` argument is silently
        // ignored (legacy), so it must NOT be used. Partner status comes from
        // `roles` (GQL `User` has no `broadcasterType`).
        let query = "query($name: String!, $first: Int!, $after: Cursor, $tags: [String!]) { \
            game(name: $name) { id name \
                streams(first: $first, after: $after, options: { sort: VIEWER_COUNT, freeformTags: $tags }) { \
                    edges { cursor node { \
                        id title viewersCount createdAt type language \
                        previewImageURL \
                        freeformTags { name } \
                        broadcaster { id login displayName profileImageURL(width: 70) roles { isPartner isAffiliate } } \
                    } } \
                    pageInfo { hasNextPage } \
                } \
            } }";

        let body = serde_json::json!({
            "query": query,
            "variables": {
                "name": game_name,
                "first": limit,
                "after": cursor,
                "tags": tags,
            }
        });

        let response = client
            .post("https://gql.twitch.tv/gql")
            .header("Client-Id", TWITCH_GQL_CLIENT_ID)
            .json(&body)
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        let game_id = response
            .pointer("/data/game/id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let resolved_game_name = response
            .pointer("/data/game/name")
            .and_then(|v| v.as_str())
            .unwrap_or(game_name)
            .to_string();

        let streams_node = response.pointer("/data/game/streams");
        let edges = streams_node
            .and_then(|s| s.get("edges"))
            .and_then(|e| e.as_array());

        let mut streams: Vec<TwitchStream> = Vec::new();
        let mut last_cursor: Option<String> = None;

        if let Some(edges) = edges {
            for edge in edges {
                if let Some(c) = edge.get("cursor").and_then(|c| c.as_str()) {
                    last_cursor = Some(c.to_string());
                }
                let node = match edge.get("node") {
                    Some(n) if !n.is_null() => n,
                    _ => continue,
                };
                let broadcaster = match node.get("broadcaster") {
                    Some(b) if !b.is_null() => b,
                    _ => continue,
                };

                let user_login = broadcaster
                    .get("login")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let user_name = broadcaster
                    .get("displayName")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .unwrap_or(&user_login)
                    .to_string();
                let roles = broadcaster.get("roles");
                let broadcaster_type = if roles
                    .and_then(|r| r.get("isPartner"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                {
                    Some("partner".to_string())
                } else if roles
                    .and_then(|r| r.get("isAffiliate"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                {
                    Some("affiliate".to_string())
                } else {
                    None
                };
                let profile_image_url = broadcaster
                    .get("profileImageURL")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                let stream_tags: Vec<String> = node
                    .get("freeformTags")
                    .and_then(|t| t.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|t| {
                                t.get("name").and_then(|n| n.as_str()).map(|s| s.to_string())
                            })
                            .collect()
                    })
                    .unwrap_or_default();

                streams.push(TwitchStream {
                    id: node.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    user_id: broadcaster.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    user_name,
                    user_login,
                    title: node.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    viewer_count: node
                        .get("viewersCount")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as u32,
                    game_id: game_id.clone(),
                    game_name: resolved_game_name.clone(),
                    thumbnail_url: node
                        .get("previewImageURL")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    started_at: node.get("createdAt").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    broadcaster_type,
                    profile_image_url,
                    is_live: Some(true),
                    tags: if stream_tags.is_empty() { None } else { Some(stream_tags) },
                    language: node
                        .get("language")
                        .and_then(|v| v.as_str())
                        .map(normalize_gql_language),
                    drops_earnable: false,
                });
            }
        }

        let has_next = streams_node
            .and_then(|s| s.get("pageInfo"))
            .and_then(|p| p.get("hasNextPage"))
            .and_then(|h| h.as_bool())
            .unwrap_or(false);

        Ok((streams, if has_next { last_cursor } else { None }))
    }

    pub async fn get_clips_by_game(
        game_id: &str,
        limit: u32,
        cursor: Option<&str>,
        period: Option<&str>,
    ) -> Result<(Vec<crate::models::stream::TwitchClip>, Option<String>)> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();

        let mut url = format!(
            "https://api.twitch.tv/helix/clips?game_id={}&first={}",
            game_id, limit
        );

        if let Some(p) = period {
            if p != "all" && !p.is_empty() {
                let now = chrono::Utc::now();
                let started_at = match p {
                    "day" | "24h" => Some(now - chrono::Duration::days(1)),
                    "week" | "7d" => Some(now - chrono::Duration::days(7)),
                    "month" | "30d" => Some(now - chrono::Duration::days(30)),
                    _ => None,
                };

                if let Some(date) = started_at {
                    // Helix Get Clips needs BOTH bounds for the time filter to
                    // apply — with only started_at it ignores the window (every
                    // period returns the same set). RFC3339 required.
                    url.push_str(&format!(
                        "&started_at={}&ended_at={}",
                        date.to_rfc3339(),
                        now.to_rfc3339()
                    ));
                }
            }
        }

        if let Some(c) = cursor {
            url.push_str(&format!("&after={}", c));
        }

        let response = client
            .get(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        let data = response.get("data").and_then(|d| d.as_array());

        match data {
            Some(arr) => {
                let clips: Vec<crate::models::stream::TwitchClip> =
                    serde_json::from_value(serde_json::Value::Array(arr.clone()))?;

                let pagination_cursor = response
                    .get("pagination")
                    .and_then(|p| p.get("cursor"))
                    .and_then(|c| c.as_str())
                    .map(|s| s.to_string());

                Ok((clips, pagination_cursor))
            }
            None => Ok((Vec::new(), None)),
        }
    }

    /// POST an inline GQL read query for public channel lists (clips, videos)
    /// with per-call device + session ids and no OAuth. Sends the Android client
    /// id, not the web client id: Twitch now fails the integrity check on
    /// paginated anonymous web-client reads, so the first page loads but "load
    /// more" comes back with "failed integrity check". The Android client id is
    /// exempt from that challenge (the same reason follows and reactions route
    /// through it), so pagination works without minting a Client-Integrity token.
    /// The device/session ids keep us out of the harsh anonymous rate-limit
    /// bucket.
    pub(crate) async fn gql_public_read(body: serde_json::Value) -> Result<serde_json::Value> {
        let client = crate::services::http::client().clone();
        let device_id = uuid::Uuid::new_v4().to_string().replace('-', "");
        let session_id = uuid::Uuid::new_v4().to_string().replace('-', "");

        let resp = client
            .post("https://gql.twitch.tv/gql")
            .header("Client-ID", env!("TWITCH_ANDROID_CLIENT_ID"))
            .header(ACCEPT, "*/*")
            .header("X-Device-Id", device_id)
            .header("Client-Session-Id", session_id)
            .json(&body)
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        // Surface top-level GraphQL errors (bad field, throttling, etc.) instead
        // of silently returning an empty list.
        if let Some(errors) = resp.get("errors").and_then(|e| e.as_array()) {
            if !errors.is_empty() {
                let msg = errors
                    .iter()
                    .filter_map(|e| e.get("message").and_then(|m| m.as_str()))
                    .collect::<Vec<_>>()
                    .join("; ");
                return Err(anyhow::anyhow!("Twitch GQL error: {}", msg));
            }
        }

        Ok(resp)
    }

    /// The category directory with no account, most-watched first, each with
    /// its live total: what `get_top_games_paginated` answers, signed in or out.
    /// `after` is the GQL cursor (without the signed-out prefix).
    async fn top_games_public(after: Option<&str>, limit: u32) -> Result<(Vec<CategoryRow>, Option<String>)> {
        let body = serde_json::json!({
            "operationName": "StreamNookTopGames",
            "query": "query StreamNookTopGames($first: Int!, $after: Cursor) { \
                games(first: $first, after: $after, options: { sort: VIEWER_COUNT }) { \
                    edges { cursor node { id name displayName boxArtURL viewersCount } } \
                    pageInfo { hasNextPage } } }",
            "variables": { "first": limit.clamp(1, 100), "after": after },
        });
        let resp = Self::gql_public_read(body).await?;
        Ok(top_games_page(&resp))
    }

    /// A category's live streams with no account, most-watched first: the
    /// signed-out answer for `get_streams_by_game` and the by-id and by-name
    /// variants. `game_id` may join several ids with `&game_id=` (the Helix
    /// form `get_streams_in_categories` builds); those are read one by one,
    /// merged by viewers and answered as a single page.
    async fn game_streams_public(
        game_id: &str,
        after: Option<&str>,
        limit: u32,
    ) -> Result<(Vec<TwitchStream>, Option<String>)> {
        let ids: Vec<&str> = game_id.split("&game_id=").filter(|id| !id.is_empty()).collect();
        if ids.len() > 1 {
            let mut streams = Vec::new();
            for id in ids.iter().take(25) {
                match Self::game_streams_public_one(id, None, limit).await {
                    Ok((rows, _)) => streams.extend(rows),
                    Err(e) => debug!("[TwitchService] public streams for category {id}: {e}"),
                }
            }
            streams.sort_by(|a, b| b.viewer_count.cmp(&a.viewer_count));
            streams.truncate(limit as usize);
            return Ok((streams, None));
        }
        match ids.first() {
            Some(id) => Self::game_streams_public_one(id, after, limit).await,
            None => Ok((Vec::new(), None)),
        }
    }

    async fn game_streams_public_one(
        game_id: &str,
        after: Option<&str>,
        limit: u32,
    ) -> Result<(Vec<TwitchStream>, Option<String>)> {
        let body = serde_json::json!({
            "operationName": "StreamNookGameStreams",
            "query": "query StreamNookGameStreams($id: ID!, $first: Int!, $after: Cursor) { \
                game(id: $id) { id name \
                    streams(first: $first, after: $after, options: { sort: VIEWER_COUNT }) { \
                        edges { cursor node { id title viewersCount createdAt type language previewImageURL \
                            freeformTags { name } \
                            broadcaster { id login displayName profileImageURL(width: 70) roles { isPartner isAffiliate } } } } \
                        pageInfo { hasNextPage } } } }",
            "variables": { "id": game_id, "first": limit.clamp(1, 100), "after": after },
        });
        let resp = Self::gql_public_read(body).await?;
        let game = resp.pointer("/data/game").filter(|g| !g.is_null());
        let id = game.and_then(|g| g.get("id")).and_then(|v| v.as_str()).unwrap_or(game_id);
        let name = game.and_then(|g| g.get("name")).and_then(|v| v.as_str()).unwrap_or_default();
        let connection = game.and_then(|g| g.get("streams"));
        let rows = gql_nodes(connection)
            .into_iter()
            .filter_map(|node| stream_row_from_gql_node(node, (id, name)))
            .collect();
        Ok((rows, public_next_cursor(connection)))
    }

    /// Live streams across Twitch with no account, most-watched first: the
    /// signed-out continuation of the Discover feed after its first page.
    /// Languages filter here, as in the recommendations read.
    async fn top_streams_public(
        after: Option<&str>,
        limit: u32,
        languages: &[String],
    ) -> Result<(Vec<TwitchStream>, Option<String>)> {
        let body = serde_json::json!({
            "operationName": "StreamNookTopStreams",
            "query": "query StreamNookTopStreams($first: Int!, $after: Cursor) { \
                streams(first: $first, after: $after, options: { sort: VIEWER_COUNT }) { \
                    edges { cursor node { id title viewersCount createdAt type language previewImageURL \
                        freeformTags { name } game { id name displayName } \
                        broadcaster { id login displayName profileImageURL(width: 70) roles { isPartner isAffiliate } } } } \
                    pageInfo { hasNextPage } } }",
            // Twitch refuses more than 30 here.
            "variables": { "first": limit.clamp(1, 30), "after": after },
        });
        let resp = Self::gql_public_read(body).await?;
        let connection = resp.pointer("/data/streams");
        let mut rows: Vec<TwitchStream> = gql_nodes(connection)
            .into_iter()
            .filter_map(|node| stream_row_from_gql_node(node, ("", "")))
            .collect();
        if !languages.is_empty() {
            let wanted: Vec<String> = languages.iter().map(|l| l.to_ascii_lowercase()).collect();
            rows.retain(|s| s.language.as_deref().is_some_and(|l| wanted.iter().any(|w| w == l)));
        }
        Ok((rows, public_next_cursor(connection)))
    }

    /// Channel search with no account: the signed-out answer for
    /// `search_channels`, ranked the same way. The exact login rides the same
    /// request, so a channel Twitch's search leaves out still leads.
    async fn search_channels_public(query: &str) -> Result<Vec<TwitchStream>> {
        let trimmed = query.trim();
        let login = trimmed.trim_start_matches('@').to_lowercase();
        let fields = "id login displayName profileImageURL(width: 300) roles { isPartner isAffiliate } \
            broadcastSettings { title game { id displayName } } \
            stream { id viewersCount createdAt previewImageURL game { id displayName } }";
        let body = if is_twitch_login(&login) {
            serde_json::json!({
                "operationName": "StreamNookSearchChannels",
                "query": format!(
                    "query StreamNookSearchChannels($query: String!, $login: String!) {{ \
                     searchUsers(userQuery: $query, first: 60) {{ edges {{ node {{ {fields} }} }} }} \
                     exact: user(login: $login) {{ {fields} }} }}"
                ),
                "variables": { "query": trimmed, "login": login },
            })
        } else {
            serde_json::json!({
                "operationName": "StreamNookSearchChannels",
                "query": format!(
                    "query StreamNookSearchChannels($query: String!) {{ \
                     searchUsers(userQuery: $query, first: 60) {{ edges {{ node {{ {fields} }} }} }} }}"
                ),
                "variables": { "query": trimmed },
            })
        };
        let resp = Self::gql_public_read(body).await?;
        let mut rows: Vec<TwitchStream> = gql_nodes(resp.pointer("/data/searchUsers"))
            .into_iter()
            .filter_map(channel_row_from_gql)
            .collect();
        if let Some(exact) = resp.pointer("/data/exact").and_then(channel_row_from_gql) {
            if !rows.iter().any(|r| r.user_id == exact.user_id) {
                rows.insert(0, exact);
            }
        }
        rank_search_results(query, &mut rows);
        Ok(rows)
    }

    /// Category search with no account, in one request: the category named
    /// exactly `query` (`game(name:)` ignores case) first, then Twitch's fuzzy
    /// search, every row with its live viewers. Unranked; repeats are left in.
    async fn search_categories_public(query: &str, limit: u32) -> Result<Vec<CategoryRow>> {
        let body = serde_json::json!({
            "operationName": "StreamNookSearchCategories",
            "query": "query StreamNookSearchCategories($query: String!, $first: Int!) { \
                exact: game(name: $query) { id name displayName boxArtURL viewersCount } \
                searchCategories(query: $query, first: $first) { edges { node { id name displayName boxArtURL viewersCount } } } }",
            "variables": { "query": query, "first": limit },
        });
        let resp = Self::gql_public_read(body).await?;
        Ok(search_page(&resp))
    }

    /// A category's id from its exact name, with no account.
    async fn game_id_by_name_public(name: &str) -> Result<Option<String>> {
        let body = serde_json::json!({
            "operationName": "StreamNookGameId",
            "query": "query StreamNookGameId($name: String!) { game(name: $name) { id } }",
            "variables": { "name": name },
        });
        let resp = Self::gql_public_read(body).await?;
        Ok(resp.pointer("/data/game/id").and_then(|v| v.as_str()).filter(|id| !id.is_empty()).map(String::from))
    }

    /// Shared Viewership (Stream Together) for many Twitch channel ids in one
    /// anonymous request on the Android client id: the list field twitch.tv
    /// itself reads for the channels on its screen. One row per channel, its
    /// `session` null when it is not in a group; the host leads, and every
    /// guest carries its own viewers and the group's combined count. One list
    /// argument rather than an alias per channel, so Twitch's cap on aliases
    /// per request never applies. The rows come back for
    /// `services::collaboration` to read.
    pub async fn get_collaborations(ids: &[String]) -> Result<Vec<serde_json::Value>> {
        let resp = Self::gql_public_read(Self::collaborations_body(ids)).await?;
        resp.pointer("/data/guestStarChannelCollaboration")
            .and_then(|rows| rows.as_array())
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("no Stream Together rows in the answer"))
    }

    /// The request `get_collaborations` sends: every id in one list argument,
    /// no root aliases.
    pub(crate) fn collaborations_body(ids: &[String]) -> serde_json::Value {
        serde_json::json!({
            "operationName": "StreamNookCollaborations",
            "query": "query StreamNookCollaborations($ids: [ID!]!) { \
                guestStarChannelCollaboration(options: { channelIDs: $ids }) { id \
                session { host { id } guests { user { id login displayName profileImageURL(width: 70) \
                stream { viewersCount collaborationViewersCount } } } } } }",
            "variables": { "ids": ids },
        })
    }

    /// Live rows for Twitch channels from the public GQL, with no account: the
    /// signed-out stand-in for Helix `streams`. Pass ids or logins. Offline and
    /// unknown channels are simply absent, like in Helix.
    pub async fn get_live_public(ids: &[String], logins: &[String]) -> Result<Vec<TwitchStream>> {
        let (arg, ty, keys) = if ids.is_empty() { ("logins", "String", logins) } else { ("ids", "ID", ids) };
        let mut out = Vec::new();
        for chunk in keys.chunks(100) {
            let body = serde_json::json!({
                "operationName": "StreamNookLivePublic",
                "query": format!(
                    "query StreamNookLivePublic($keys: [{ty}!]) {{ users({arg}: $keys) {{ id login displayName \
                     profileImageURL(width: 70) stream {{ id type createdAt viewersCount title \
                     game {{ id displayName }} }} }} }}"
                ),
                "variables": { "keys": chunk },
            });
            let resp = Self::gql_public_read(body).await?;
            let users = resp.pointer("/data/users").and_then(|u| u.as_array()).cloned().unwrap_or_default();
            out.extend(users.iter().filter_map(live_row_from_gql));
        }
        Ok(out)
    }

    /// How many people are in a channel's chat right now, live or offline, from
    /// the public chatter list (the Android client id, which Twitch exempts from
    /// the integrity check the web one fails here). No account needed.
    pub async fn get_chatter_count(login: &str) -> Result<u64> {
        let body = serde_json::json!({
            "query": "query StreamNookChatterCount($login: String!) { channel(name: $login) { chatters { count } } }",
            "variables": { "login": login.to_lowercase() },
        });
        let resp = Self::gql_public_read(body).await?;
        resp.pointer("/data/channel/chatters/count")
            .and_then(|c| c.as_u64())
            .ok_or_else(|| anyhow::anyhow!("no chatter count for {login}"))
    }

    /// Everything a channel's offline room shows, in one public GQL read (no
    /// account): who it is, its offline image, whether it is live after all,
    /// the last broadcast, the latest archived VOD and the next scheduled
    /// stream. The raw `user` node; `services::offline_room` reads it.
    pub async fn get_offline_room(login: &str) -> Result<serde_json::Value> {
        let body = serde_json::json!({
            "operationName": "StreamNookOfflineRoom",
            "query": "query StreamNookOfflineRoom($login: String!) { user(login: $login) { \
                id login displayName profileImageURL(width: 150) offlineImageURL \
                stream { id type createdAt viewersCount title game { id displayName } } \
                lastBroadcast { startedAt title game { displayName } } \
                videos(first: 1, type: ARCHIVE, sort: TIME) { edges { node { \
                    id title createdAt lengthSeconds previewThumbnailURL(width: 440, height: 248) \
                    game { displayName } } } } \
                channel { schedule { nextSegment { startAt endAt title isCancelled categories { name } } } } \
            } }",
            "variables": { "login": login },
        });
        let resp = Self::gql_public_read(body).await?;
        Ok(resp.pointer("/data/user").cloned().unwrap_or(serde_json::Value::Null))
    }

    /// The Shared Chat session a channel is in: its host and every participating
    /// broadcaster id. `None` when it is in none. Helix only; Twitch answers
    /// this for signed-in callers alone, and one channel per request.
    pub async fn get_shared_chat_session(broadcaster_id: &str) -> Result<Option<(String, Vec<String>)>> {
        let token = Self::get_token().await?;
        let response = crate::services::http::client()
            .get("https://api.twitch.tv/helix/chat/shared")
            .query(&[("broadcaster_id", broadcaster_id)])
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;
        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !status.is_success() {
            return Err(anyhow::anyhow!("shared chat lookup: HTTP {status}"));
        }
        let body: serde_json::Value = response.json().await?;
        let Some(session) = body.pointer("/data/0") else { return Ok(None) };
        let host = session
            .get("host_broadcaster_id")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let participants = session
            .get("participants")
            .and_then(|v| v.as_array())
            .map(|list| {
                list.iter()
                    .filter_map(|p| p.get("broadcaster_id").and_then(|v| v.as_str()).map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        Ok(Some((host, participants)))
    }

    /// Name, avatar and live viewer count for a batch of Twitch channel ids,
    /// one aliased anonymous GQL request (`u0`, `u1`, ... in the order given).
    /// `stream` is null for a channel that is not live.
    /// Callers send at most `twitch_limits::GQL_MAX_ALIASES` ids.
    pub async fn get_channel_faces(ids: &[String]) -> Result<serde_json::Value> {
        let resp = Self::gql_public_read(Self::channel_faces_body(ids)).await?;
        Ok(resp.get("data").cloned().unwrap_or(serde_json::Value::Null))
    }

    /// The request `get_channel_faces` sends: one `uN` alias per id.
    pub(crate) fn channel_faces_body(ids: &[String]) -> serde_json::Value {
        let mut vars = serde_json::Map::new();
        let mut params = Vec::with_capacity(ids.len());
        let mut fields = String::new();
        for (i, id) in ids.iter().enumerate() {
            params.push(format!("$i{i}: ID!"));
            vars.insert(format!("i{i}"), serde_json::Value::String(id.clone()));
            fields.push_str(&format!(
                "u{i}: user(id: $i{i}) {{ id login displayName profileImageURL(width: 70) stream {{ viewersCount }} }}\n"
            ));
        }
        serde_json::json!({
            "operationName": "StreamNookChannelFaces",
            "query": format!("query StreamNookChannelFaces({}) {{\n{fields}}}", params.join(", ")),
            "variables": vars,
        })
    }

    /// One streamer's clips, fetched the way the Twitch website's Clips tab does:
    /// `user.clips(criteria: { filter: <period> })`. Unlike Helix `/clips`, this
    /// returns the correct time window (Helix caps an open-ended `started_at` at
    /// one week, breaking the 30-day filter) AND orders by view count (Helix has
    /// no sort), so "Top clips (last 7 days)" etc. match twitch.tv exactly. Game
    /// names come back inline, so no follow-up `get_games_by_ids` round-trip.
    pub async fn get_clips_by_broadcaster(
        broadcaster_id: &str,
        limit: u32,
        cursor: Option<&str>,
        period: Option<&str>,
    ) -> Result<(Vec<crate::models::stream::TwitchClip>, Option<String>)> {
        // Map our period strings → Twitch's ClipsPeriod enum (always has a value;
        // "all" → ALL_TIME). The value is a fixed keyword, never user input, so
        // interpolating it into the query is safe.
        let filter = match period.unwrap_or("all") {
            "24h" | "day" => "LAST_DAY",
            "7d" | "week" => "LAST_WEEK",
            "30d" | "month" => "LAST_MONTH",
            _ => "ALL_TIME",
        };

        let query = format!(
            r#"query StreamNookUserClips($id: ID!, $first: Int!, $after: Cursor) {{
                user(id: $id) {{
                    clips(first: $first, after: $after, criteria: {{ filter: {filter} }}) {{
                        edges {{
                            cursor
                            node {{
                                slug
                                url
                                embedURL
                                title
                                viewCount
                                createdAt
                                durationSeconds
                                thumbnailURL
                                videoOffsetSeconds
                                video {{ id }}
                                game {{ id name }}
                                curator {{ id displayName }}
                                broadcaster {{ id displayName login }}
                            }}
                        }}
                        pageInfo {{ hasNextPage }}
                    }}
                }}
            }}"#,
            filter = filter
        );

        let body = serde_json::json!({
            "operationName": "StreamNookUserClips",
            "query": query,
            "variables": { "id": broadcaster_id, "first": limit, "after": cursor },
        });

        let resp = Self::gql_public_read(body).await?;
        let clips_node = resp
            .get("data")
            .and_then(|d| d.get("user"))
            .and_then(|u| u.get("clips"));

        let edges = clips_node
            .and_then(|c| c.get("edges"))
            .and_then(|e| e.as_array())
            .cloned()
            .unwrap_or_default();

        let str_at = |v: &serde_json::Value, path: &[&str]| -> String {
            let mut cur = v;
            for k in path {
                match cur.get(k) {
                    Some(next) => cur = next,
                    None => return String::new(),
                }
            }
            cur.as_str().unwrap_or("").to_string()
        };

        let mut clips = Vec::with_capacity(edges.len());
        let mut last_cursor: Option<String> = None;
        for edge in &edges {
            last_cursor = edge
                .get("cursor")
                .and_then(|c| c.as_str())
                .map(|s| s.to_string());
            let node = match edge.get("node") {
                Some(n) if !n.is_null() => n,
                _ => continue,
            };
            let slug = str_at(node, &["slug"]);
            let game_name = node
                .get("game")
                .and_then(|g| g.get("name"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            clips.push(crate::models::stream::TwitchClip {
                id: slug.clone(),
                url: str_at(node, &["url"]),
                embed_url: str_at(node, &["embedURL"]),
                broadcaster_id: str_at(node, &["broadcaster", "id"]),
                broadcaster_name: str_at(node, &["broadcaster", "displayName"]),
                broadcaster_login: node
                    .get("broadcaster")
                    .and_then(|b| b.get("login"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_lowercase()),
                creator_id: str_at(node, &["curator", "id"]),
                creator_name: str_at(node, &["curator", "displayName"]),
                video_id: str_at(node, &["video", "id"]),
                game_id: str_at(node, &["game", "id"]),
                game_name,
                language: String::new(),
                title: str_at(node, &["title"]),
                view_count: node.get("viewCount").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                created_at: str_at(node, &["createdAt"]),
                thumbnail_url: str_at(node, &["thumbnailURL"]),
                duration: node
                    .get("durationSeconds")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0) as f32,
                vod_offset: node
                    .get("videoOffsetSeconds")
                    .and_then(|v| v.as_u64())
                    .map(|n| n as u32),
            });
        }

        let has_next = clips_node
            .and_then(|c| c.get("pageInfo"))
            .and_then(|p| p.get("hasNextPage"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let next = if has_next { last_cursor } else { None };

        Ok((clips, next))
    }

    /// Reaction ("likes") counts for a batch of clips, via the
    /// `aggregateReactionsBreakdownByContentKeys` query (NOT on the clip list
    /// type — Twitch loads it per-clip). Mirrors the app's proven authed
    /// persisted-hash path (Android client id + drops OAuth token, integrity-free;
    /// the app token + web client id 401s). Twitch fails the WHOLE request if any
    /// single content key is invalid (e.g. an old clip that predates reactions),
    /// so we chunk and, on that error, retry the chunk one clip at a time — every
    /// valid clip still gets its count. Hash captured 2026-06-03.
    pub async fn get_clip_reactions(
        slugs: &[String],
    ) -> Result<Vec<crate::models::stream::ClipReactions>> {
        if slugs.is_empty() {
            return Ok(Vec::new());
        }
        // Auth once (Android client id + drops token; see fetch_reactions_batch).
        let token = crate::services::drops_auth_service::DropsAuthService::get_token().await?;

        let mut out: Vec<crate::models::stream::ClipReactions> = Vec::new();
        for chunk in slugs.chunks(20) {
            match Self::fetch_reactions_batch(chunk, &token).await {
                Ok(rows) => out.extend(rows),
                // One clip in the chunk is un-reactable and Twitch rejected the
                // entire batch; retry each clip alone, skipping the bad one(s).
                Err(e) if e.to_string().to_lowercase().contains("content key") => {
                    for s in chunk {
                        if let Ok(rows) =
                            Self::fetch_reactions_batch(std::slice::from_ref(s), &token).await
                        {
                            out.extend(rows);
                        }
                    }
                }
                // Auth/transport failure — fatal for every chunk, so surface it.
                Err(e) => return Err(e),
            }
        }
        Ok(out)
    }

    /// One reactions request for a batch of clip slugs (see get_clip_reactions).
    /// Errs with the GraphQL/auth message; the caller keys off "content key" to
    /// decide whether to split-and-retry.
    async fn fetch_reactions_batch(
        slugs: &[String],
        token: &str,
    ) -> Result<Vec<crate::models::stream::ClipReactions>> {
        let client = crate::services::http::client().clone();
        let device_id = uuid::Uuid::new_v4().to_string().replace('-', "");
        let session_id = uuid::Uuid::new_v4().to_string().replace('-', "");

        let content_keys: Vec<serde_json::Value> = slugs
            .iter()
            .map(|s| serde_json::json!({ "id": s, "type": "CLIP" }))
            .collect();

        let body = serde_json::json!({
            "operationName": "aggregateReactionsBreakdownByContentKeys",
            "variables": { "contentKeys": content_keys },
            "extensions": {
                "persistedQuery": {
                    "version": 1,
                    "sha256Hash": "f74807b6d62e390c87a2dc140ee24cc6e6c989c5e5170f9ebe32a39014da86fd"
                }
            }
        });

        let resp = client
            .post("https://gql.twitch.tv/gql")
            .header("Client-ID", env!("TWITCH_ANDROID_CLIENT_ID"))
            .header(ACCEPT, "*/*")
            .header(AUTHORIZATION, format!("OAuth {}", token))
            .header("Origin", "https://www.twitch.tv")
            .header("Referer", "https://www.twitch.tv")
            .header("X-Device-Id", device_id)
            .header("Client-Session-Id", session_id)
            .json(&body)
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        // Top-level HTTP-style error (e.g. 401 `{"error":"Unauthorized", ...}`) —
        // this is NOT a GraphQL `errors` array, so check it explicitly or it would
        // slip through as "empty totals".
        if let Some(err) = resp.get("error").and_then(|e| e.as_str()) {
            let msg = resp.get("message").and_then(|m| m.as_str()).unwrap_or(err);
            return Err(anyhow::anyhow!("Twitch reactions auth error: {}", msg));
        }

        if let Some(errors) = resp.get("errors").and_then(|e| e.as_array()) {
            if !errors.is_empty() {
                let msg = errors
                    .iter()
                    .filter_map(|e| e.get("message").and_then(|m| m.as_str()))
                    .collect::<Vec<_>>()
                    .join("; ");
                return Err(anyhow::anyhow!("Twitch reactions error: {}", msg));
            }
        }

        let totals = resp
            .get("data")
            .and_then(|d| d.get("aggregateReactionsBreakdownByContentKeys"))
            .and_then(|a| a.get("totals"))
            .and_then(|t| t.as_array())
            .cloned()
            .unwrap_or_default();

        let mut out = Vec::with_capacity(totals.len());
        for t in &totals {
            let id = t
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if id.is_empty() {
                continue;
            }
            let total = t.get("totalCount").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            let like = t
                .get("totalBreakdown")
                .and_then(|b| b.as_array())
                .map(|arr| {
                    arr.iter()
                        .find(|e| {
                            e.get("reaction")
                                .and_then(|r| r.get("type"))
                                .and_then(|ty| ty.as_str())
                                == Some("LIKE")
                        })
                        .and_then(|e| e.get("count"))
                        .and_then(|c| c.as_u64())
                        .unwrap_or(0) as u32
                })
                .unwrap_or(0);
            out.push(crate::models::stream::ClipReactions { id, total, like });
        }

        // Empty `out` is valid here — a clip with no reactions just won't appear
        // in totals (e.g. older clips that predate the reactions feature).
        Ok(out)
    }

    pub async fn get_videos_by_game(
        game_id: &str,
        sort: &str,
        period: Option<&str>,
        limit: u32,
        cursor: Option<&str>,
    ) -> Result<(Vec<crate::models::stream::TwitchVideo>, Option<String>)> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();

        let mut url = format!(
            "https://api.twitch.tv/helix/videos?game_id={}&sort={}&first={}",
            game_id, sort, limit
        );

        if let Some(p) = period {
            if !p.is_empty() && p != "all" {
                url.push_str(&format!("&period={}", p));
            }
        }

        if let Some(c) = cursor {
            url.push_str(&format!("&after={}", c));
        }

        let response = client
            .get(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        let data = response.get("data").and_then(|d| d.as_array());

        match data {
            Some(arr) => {
                let mut videos: Vec<crate::models::stream::TwitchVideo> =
                    serde_json::from_value(serde_json::Value::Array(arr.clone()))?;
                // Helix only formats the length ("3h21m4s"); recover the seconds
                // so the card's progress bar has a denominator.
                for v in videos.iter_mut() {
                    if v.length_seconds.is_none() {
                        v.length_seconds = Self::parse_duration_str(&v.duration);
                    }
                }

                let pagination_cursor = response
                    .get("pagination")
                    .and_then(|p| p.get("cursor"))
                    .and_then(|c| c.as_str())
                    .map(|s| s.to_string());

                Ok((videos, pagination_cursor))
            }
            None => Ok((Vec::new(), None)),
        }
    }

    /// One streamer's videos, fetched the way the Twitch website's Videos tab
    /// does: `user.videos(sort:, type:)`. Twitch has no time-window filter for a
    /// user's videos (Helix only accepts `period` with a game_id, never a
    /// user_id), so the website offers Sort (Recent/Popular) + Type (broadcast
    /// kind) instead — this mirrors that. `sort` is "time" or "views"; `kind`
    /// is "archive" | "highlight" | "upload" (None = all types).
    pub async fn get_user_videos(
        user_id: &str,
        sort: &str,
        kind: Option<&str>,
        limit: u32,
        cursor: Option<&str>,
    ) -> Result<(Vec<crate::models::stream::TwitchVideo>, Option<String>)> {
        let sort_enum = match sort.to_lowercase().as_str() {
            "views" | "view" | "popular" => "VIEWS",
            _ => "TIME",
        };
        let type_enum: Option<&str> = match kind {
            Some("archive") => Some("ARCHIVE"),
            Some("highlight") => Some("HIGHLIGHT"),
            Some("upload") => Some("UPLOAD"),
            _ => None, // all types
        };

        let query = r#"query StreamNookUserVideos($id: ID!, $first: Int!, $after: Cursor, $sort: VideoSort, $type: BroadcastType) {
            user(id: $id) {
                videos(first: $first, after: $after, sort: $sort, type: $type) {
                    edges {
                        cursor
                        node {
                            id
                            title
                            publishedAt
                            createdAt
                            lengthSeconds
                            status
                            viewCount
                            previewThumbnailURL(width: 440, height: 248)
                            broadcastType
                            game { displayName }
                            owner { id login displayName }
                        }
                    }
                    pageInfo { hasNextPage }
                }
            }
        }"#;

        let body = serde_json::json!({
            "operationName": "StreamNookUserVideos",
            "query": query,
            "variables": {
                "id": user_id,
                "first": limit,
                "after": cursor,
                "sort": sort_enum,
                "type": type_enum,
            },
        });

        let resp = Self::gql_public_read(body).await?;
        let videos_node = resp
            .get("data")
            .and_then(|d| d.get("user"))
            .and_then(|u| u.get("videos"));

        let edges = videos_node
            .and_then(|v| v.get("edges"))
            .and_then(|e| e.as_array())
            .cloned()
            .unwrap_or_default();

        let str_at = |v: &serde_json::Value, path: &[&str]| -> String {
            let mut cur = v;
            for k in path {
                match cur.get(k) {
                    Some(next) => cur = next,
                    None => return String::new(),
                }
            }
            cur.as_str().unwrap_or("").to_string()
        };

        let mut videos = Vec::with_capacity(edges.len());
        let mut last_cursor: Option<String> = None;
        for edge in &edges {
            last_cursor = edge
                .get("cursor")
                .and_then(|c| c.as_str())
                .map(|s| s.to_string());
            let node = match edge.get("node") {
                Some(n) if !n.is_null() => n,
                _ => continue,
            };
            let id = str_at(node, &["id"]);
            let published_at = str_at(node, &["publishedAt"]);
            // GQL gives length in seconds; the UI badge expects a Helix-style
            // string ("3h21m4s"). Format it to match.
            let length_secs = node
                .get("lengthSeconds")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            // GQL broadcastType is uppercase (ARCHIVE/HIGHLIGHT/UPLOAD); the UI's
            // label map expects Helix-style lowercase.
            let video_type = node
                .get("broadcastType")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_lowercase();
            videos.push(crate::models::stream::TwitchVideo {
                url: format!("https://www.twitch.tv/videos/{}", id),
                user_id: str_at(node, &["owner", "id"]),
                user_login: str_at(node, &["owner", "login"]),
                user_name: str_at(node, &["owner", "displayName"]),
                title: str_at(node, &["title"]),
                description: String::new(),
                created_at: if published_at.is_empty() {
                    str_at(node, &["createdAt"])
                } else {
                    published_at.clone()
                },
                published_at,
                thumbnail_url: str_at(node, &["previewThumbnailURL"]),
                viewable: "public".to_string(),
                view_count: node.get("viewCount").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                language: String::new(),
                video_type,
                duration: Self::fmt_duration_secs(length_secs),
                // GQL Video.status is RECORDING while the broadcast is live.
                status: node
                    .get("status")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_lowercase()),
                length_seconds: Some(length_secs.min(u32::MAX as u64) as u32),
                game_name: Some(str_at(node, &["game", "displayName"])).filter(|g| !g.is_empty()),
                progress: None,
                stream_id: None,
                id,
            });
        }

        let has_next = videos_node
            .and_then(|v| v.get("pageInfo"))
            .and_then(|p| p.get("hasNextPage"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let next = if has_next { last_cursor } else { None };

        Ok((videos, next))
    }

    /// Read many VODs by id in ONE request, using GraphQL aliases.
    ///
    /// Returns `(fresh metadata, ids Twitch no longer has)`. Probed live
    /// 2026-09-12: a deleted id resolves to `null` for just its own alias and
    /// the rest of the response is intact, so one call both refreshes the
    /// Continue Watching cards and tells us which stored entries to forget.
    ///
    /// Ids travel as `$iN: ID!` variables. They come from our own store and
    /// are numeric in practice, but a query built by string interpolation is
    /// not a thing worth shipping.
    pub async fn get_videos_by_ids(
        ids: &[String],
    ) -> Result<(
        Vec<crate::services::vod_progress_service::VodRefresh>,
        Vec<String>,
    )> {
        let mut found = Vec::new();
        let mut gone = Vec::new();

        for chunk in ids.chunks(Self::VODS_BATCH) {
            let resp = Self::gql_public_read(Self::videos_by_ids_body(chunk)).await?;

            for (i, id) in chunk.iter().enumerate() {
                // Map back by ALIAS INDEX, never by the returned id: a deleted
                // video comes back as a bare `null` with no id to match on.
                let node = resp.pointer(&format!("/data/v{i}"));
                match node.filter(|v| !v.is_null()) {
                    None => gone.push(id.clone()),
                    Some(v) => found.push(vod_refresh_from_node(id, v)),
                }
            }
        }

        Ok((found, gone))
    }

    /// VODs per `get_videos_by_ids` request: one alias each.
    pub(crate) const VODS_BATCH: usize = crate::services::twitch_limits::GQL_MAX_ALIASES;

    /// The request for one `get_videos_by_ids` chunk: one `vN` alias per id.
    pub(crate) fn videos_by_ids_body(chunk: &[String]) -> serde_json::Value {
        let params = (0..chunk.len())
            .map(|i| format!("$i{i}: ID!"))
            .collect::<Vec<_>>()
            .join(", ");
        let selections = (0..chunk.len())
            .map(|i| format!("v{i}: video(id: $i{i}) {{ {VOD_CARD_FIELDS} }}"))
            .collect::<Vec<_>>()
            .join(" ");
        let variables: serde_json::Map<String, serde_json::Value> = chunk
            .iter()
            .enumerate()
            .map(|(i, id)| (format!("i{i}"), serde_json::Value::String(id.clone())))
            .collect();
        serde_json::json!({
            "operationName": "StreamNookVodsByIds",
            "query": format!("query StreamNookVodsByIds({params}) {{ {selections} }}"),
            "variables": variables,
        })
    }

    /// Format a second count as a Helix-style duration string ("3h21m4s").
    /// Inverse of `fmt_duration_secs` for Helix's "1h2m3s" strings. None for
    /// anything that is not that shape.
    pub fn parse_duration_str(s: &str) -> Option<u32> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        let mut total: u64 = 0;
        let mut num: u64 = 0;
        let mut saw_digit = false;
        for c in s.chars() {
            match c {
                '0'..='9' => {
                    num = num.checked_mul(10)?.checked_add(c as u64 - '0' as u64)?;
                    saw_digit = true;
                }
                'h' => {
                    total = total.checked_add(num.checked_mul(3600)?)?;
                    num = 0;
                }
                'm' => {
                    total = total.checked_add(num.checked_mul(60)?)?;
                    num = 0;
                }
                's' => {
                    total = total.checked_add(num)?;
                    num = 0;
                }
                _ => return None,
            }
        }
        if !saw_digit || num != 0 {
            return None;
        }
        Some(total.min(u32::MAX as u64) as u32)
    }

    fn fmt_duration_secs(total: u64) -> String {
        let h = total / 3600;
        let m = (total % 3600) / 60;
        let s = total % 60;
        if h > 0 {
            format!("{}h{}m{}s", h, m, s)
        } else if m > 0 {
            format!("{}m{}s", m, s)
        } else {
            format!("{}s", s)
        }
    }

    /// Update Chat Settings (Emote mode, Follower mode, Slow mode, etc.)
    pub async fn update_chat_settings(
        broadcaster_id: &str,
        settings: serde_json::Value,
    ) -> Result<()> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();
        let user_info = Self::get_user_info().await?;
        let moderator_id = &user_info.id;

        let url = format!(
            "https://api.twitch.tv/helix/chat/settings?broadcaster_id={}&moderator_id={}",
            broadcaster_id, moderator_id
        );

        let response = client
            .patch(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .header("Content-Type", "application/json")
            .json(&settings)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let error_text = response.text().await.unwrap_or_default();
            error!(
                "[TwitchService] Failed to update chat settings: {}",
                error_text
            );
            return Err(anyhow::anyhow!(helix_refusal_text(status, &error_text)));
        }

        Ok(())
    }

    /// Clear all messages from chat
    pub async fn clear_chat(broadcaster_id: &str) -> Result<()> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();
        let user_info = Self::get_user_info().await?;
        let moderator_id = &user_info.id;

        let url = format!(
            "https://api.twitch.tv/helix/moderation/chat?broadcaster_id={}&moderator_id={}",
            broadcaster_id, moderator_id
        );

        let response = client
            .delete(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;

        if !response.status().is_success() && response.status() != 204 {
            let status = response.status().as_u16();
            let error_text = response.text().await.unwrap_or_default();
            error!("[TwitchService] Failed to clear chat: {}", error_text);
            return Err(anyhow::anyhow!(helix_refusal_text(status, &error_text)));
        }

        Ok(())
    }

    /// Delete a specific chat message
    pub async fn delete_chat_message(broadcaster_id: &str, message_id: &str) -> Result<()> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();
        let user_info = Self::get_user_info().await?;
        let moderator_id = &user_info.id;

        let url = format!(
            "https://api.twitch.tv/helix/moderation/chat?broadcaster_id={}&moderator_id={}&message_id={}",
            broadcaster_id, moderator_id, message_id
        );

        let response = client
            .delete(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;

        if !response.status().is_success() && response.status() != 204 {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            error!(
                "[TwitchService] Failed to delete message (HTTP {}): {}",
                status, error_text
            );
            // Surface Twitch's status + reason so the caller can tell apart a
            // restricted delete (broadcaster/other-mod message, >6h old) from a
            // scope/auth failure instead of an opaque "Failed to delete message".
            return Err(anyhow::anyhow!(
                "Twitch rejected delete (HTTP {}): {}",
                status,
                error_text
            ));
        }

        Ok(())
    }

    /// Pin a chat message to the top of the broadcaster's chat room (mod action).
    ///
    /// Helix `PUT /helix/chat/pins` — every parameter rides in the query string
    /// and the body is empty. Uses the `moderator:manage:chat_messages` scope,
    /// the SAME scope `delete_chat_message` already holds, so no re-auth is
    /// needed. Only one mod-pinned message is active at a time; pinning a new one
    /// replaces the prior pin. `duration_seconds` is optional (omitted, the pin
    /// lasts until the stream ends); when given, Twitch only accepts
    /// `PIN_DURATION_RANGE` and answers anything else with a bare 400.
    pub async fn pin_chat_message(
        broadcaster_id: &str,
        message_id: &str,
        duration_seconds: Option<u32>,
    ) -> Result<()> {
        check_pin_duration(duration_seconds)?;
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();
        let user_info = Self::get_user_info().await?;
        let moderator_id = &user_info.id;

        let mut url = format!(
            "https://api.twitch.tv/helix/chat/pins?broadcaster_id={}&moderator_id={}&message_id={}",
            broadcaster_id, moderator_id, message_id
        );
        if let Some(d) = duration_seconds {
            url.push_str(&format!("&duration_seconds={}", d));
        }

        let response = client
            .put(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;

        if !response.status().is_success() && response.status() != 204 {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            error!(
                "[TwitchService] Failed to pin message (HTTP {}): {}",
                status, error_text
            );
            return Err(anyhow::anyhow!(
                "Twitch rejected pin (HTTP {}): {}",
                status,
                error_text
            ));
        }

        Ok(())
    }

    /// Unpin a mod-pinned chat message (mod action).
    ///
    /// Helix `DELETE /helix/chat/pins?broadcaster_id=..&moderator_id=..&message_id=..`
    /// — `message_id` IS required (Twitch returns 400 "Missing required parameter
    /// message_id" without it). Same `moderator:manage:chat_messages` scope as
    /// pin/delete. Callers that only know the channel (`/unpin`, the moderator
    /// menu) pass `None` and the current pin is looked up here. Returns the
    /// chat message id that was unpinned.
    pub async fn unpin_chat_message(broadcaster_id: &str, message_id: Option<&str>) -> Result<String> {
        let message_id = match message_id.filter(|id| !id.is_empty()) {
            Some(id) => id.to_string(),
            None => Self::current_pinned_message_id(broadcaster_id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("Nothing is pinned right now"))?,
        };
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();
        let user_info = Self::get_user_info().await?;
        let moderator_id = &user_info.id;

        let url = format!(
            "https://api.twitch.tv/helix/chat/pins?broadcaster_id={}&moderator_id={}&message_id={}",
            broadcaster_id, moderator_id, message_id
        );

        let response = client
            .delete(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;

        if !response.status().is_success() && response.status() != 204 {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            error!(
                "[TwitchService] Failed to unpin message (HTTP {}): {}",
                status, error_text
            );
            return Err(anyhow::anyhow!(
                "Twitch rejected unpin (HTTP {}): {}",
                status,
                error_text
            ));
        }

        Ok(message_id)
    }

    /// The chat message id of the channel's active pin, if there is one.
    async fn current_pinned_message_id(broadcaster_id: &str) -> Result<Option<String>> {
        let pins = Self::get_pinned_chat_messages(broadcaster_id).await?;
        Ok(pins.iter().find_map(|p| {
            p.get("message_id")
                .and_then(|v| v.as_str())
                .filter(|id| !id.is_empty())
                .map(str::to_string)
        }))
    }

    /// Ban or timeout a user
    pub async fn ban_user(
        broadcaster_id: &str,
        target_user_id: &str,
        duration: Option<u32>,
        reason: Option<&str>,
    ) -> Result<()> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();
        let user_info = Self::get_user_info().await?;
        let moderator_id = &user_info.id;
        let action = if duration.is_some() { "time out" } else { "ban" };
        if target_user_id == broadcaster_id {
            return Err(anyhow::anyhow!("You can't {} the broadcaster", action));
        }
        if target_user_id == moderator_id {
            return Err(anyhow::anyhow!("You can't {} yourself", action));
        }

        let url = format!(
            "https://api.twitch.tv/helix/moderation/bans?broadcaster_id={}&moderator_id={}",
            broadcaster_id, moderator_id
        );

        let mut payload = serde_json::json!({
            "data": {
                "user_id": target_user_id,
            }
        });

        if let Some(d) = duration {
            payload["data"]["duration"] = serde_json::json!(d);
        }

        if let Some(r) = reason {
            if !r.is_empty() {
                payload["data"]["reason"] = serde_json::json!(r);
            }
        }

        let response = client
            .post(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let error_text = response.text().await.unwrap_or_default();
            error!("[TwitchService] Failed to ban/timeout user: {}", error_text);
            return Err(anyhow::anyhow!(
                "Twitch refused the {}: {}",
                if duration.is_some() { "timeout" } else { "ban" },
                helix_refusal_text(status, &error_text)
            ));
        }

        Ok(())
    }

    /// Unban a user
    pub async fn unban_user(broadcaster_id: &str, target_user_id: &str) -> Result<()> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();
        let user_info = Self::get_user_info().await?;
        let moderator_id = &user_info.id;

        let url = format!(
            "https://api.twitch.tv/helix/moderation/bans?broadcaster_id={}&moderator_id={}&user_id={}",
            broadcaster_id, moderator_id, target_user_id
        );

        let response = client
            .delete(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;

        if !response.status().is_success() && response.status() != 204 {
            let status = response.status().as_u16();
            let error_text = response.text().await.unwrap_or_default();
            error!("[TwitchService] Failed to unban user: {}", error_text);
            return Err(anyhow::anyhow!(
                "Twitch refused the unban: {}",
                helix_refusal_text(status, &error_text)
            ));
        }

        Ok(())
    }

    /// Add a channel moderator
    pub async fn add_channel_moderator(broadcaster_id: &str, user_id: &str) -> Result<()> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();

        let url = format!(
            "https://api.twitch.tv/helix/moderation/moderators?broadcaster_id={}&user_id={}",
            broadcaster_id, user_id
        );

        let response = client
            .post(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;

        if !response.status().is_success() && response.status() != 204 {
            let status = response.status().as_u16();
            let error_text = response.text().await.unwrap_or_default();
            error!("[TwitchService] Failed to add moderator: {}", error_text);
            return Err(anyhow::anyhow!(helix_refusal_text(status, &error_text)));
        }

        Ok(())
    }

    /// Remove a channel moderator
    pub async fn remove_channel_moderator(broadcaster_id: &str, user_id: &str) -> Result<()> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();

        let url = format!(
            "https://api.twitch.tv/helix/moderation/moderators?broadcaster_id={}&user_id={}",
            broadcaster_id, user_id
        );

        let response = client
            .delete(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;

        if !response.status().is_success() && response.status() != 204 {
            let status = response.status().as_u16();
            let error_text = response.text().await.unwrap_or_default();
            error!("[TwitchService] Failed to remove moderator: {}", error_text);
            return Err(anyhow::anyhow!(helix_refusal_text(status, &error_text)));
        }

        Ok(())
    }

    /// Add a channel VIP
    pub async fn add_channel_vip(broadcaster_id: &str, user_id: &str) -> Result<()> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();

        let url = format!(
            "https://api.twitch.tv/helix/channels/vips?broadcaster_id={}&user_id={}",
            broadcaster_id, user_id
        );

        let response = client
            .post(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;

        if !response.status().is_success() && response.status() != 204 {
            let status = response.status().as_u16();
            let error_text = response.text().await.unwrap_or_default();
            error!("[TwitchService] Failed to add VIP: {}", error_text);
            return Err(anyhow::anyhow!(helix_refusal_text(status, &error_text)));
        }

        Ok(())
    }

    /// Remove a channel VIP
    pub async fn remove_channel_vip(broadcaster_id: &str, user_id: &str) -> Result<()> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();

        let url = format!(
            "https://api.twitch.tv/helix/channels/vips?broadcaster_id={}&user_id={}",
            broadcaster_id, user_id
        );

        let response = client
            .delete(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;

        if !response.status().is_success() && response.status() != 204 {
            let status = response.status().as_u16();
            let error_text = response.text().await.unwrap_or_default();
            error!("[TwitchService] Failed to remove VIP: {}", error_text);
            return Err(anyhow::anyhow!(helix_refusal_text(status, &error_text)));
        }

        Ok(())
    }

    /// Update Suspicious User Status (Restrict/Monitor)
    /// Set the stream title and/or category (`channel:manage:broadcast`).
    /// A category is given by name and resolved through Helix `games?name=`;
    /// an unknown name is an error, never a silent no-op.
    pub async fn update_channel_info(
        broadcaster_id: &str,
        title: Option<&str>,
        game_name: Option<&str>,
    ) -> Result<()> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();
        let mut payload = serde_json::Map::new();
        if let Some(t) = title {
            payload.insert("title".into(), serde_json::json!(t));
        }
        if let Some(name) = game_name {
            let resp = client
                .get("https://api.twitch.tv/helix/games")
                .query(&[("name", name)])
                .header("Client-Id", CLIENT_ID)
                .header(AUTHORIZATION, format!("Bearer {}", token))
                .send()
                .await?;
            let json: serde_json::Value = resp.json().await?;
            let id = json
                .pointer("/data/0/id")
                .and_then(|v| v.as_str())
                .map(String::from)
                .ok_or_else(|| anyhow::anyhow!("No Twitch category named \"{}\"", name))?;
            payload.insert("game_id".into(), serde_json::json!(id));
        }
        if payload.is_empty() {
            return Ok(());
        }
        let response = client
            .patch(format!(
                "https://api.twitch.tv/helix/channels?broadcaster_id={}",
                broadcaster_id
            ))
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .header("Content-Type", "application/json")
            .json(&serde_json::Value::Object(payload))
            .send()
            .await?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let text = response.text().await.unwrap_or_default();
            error!("[TwitchService] update_channel_info failed ({}): {}", status, text);
            return Err(anyhow::anyhow!(match status {
                401 | 403 => "Only the broadcaster can change the title or category".to_string(),
                _ => format!("Channel update failed ({})", status),
            }));
        }
        Ok(())
    }

    /// Allow or deny an AutoMod-held message (`moderator:manage:automod`).
    pub async fn resolve_automod_message(msg_id: &str, allow: bool) -> Result<()> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();
        let user_info = Self::get_user_info().await?;
        let payload = serde_json::json!({
            "user_id": user_info.id,
            "msg_id": msg_id,
            "action": if allow { "ALLOW" } else { "DENY" },
        });
        let response = client
            .post("https://api.twitch.tv/helix/moderation/automod/message")
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let error_text = response.text().await.unwrap_or_default();
            error!("[TwitchService] AutoMod resolve failed ({}): {}", status, error_text);
            return Err(anyhow::anyhow!(match status {
                400 => "That message is no longer held".to_string(),
                403 => "Not allowed to review AutoMod for this channel".to_string(),
                _ => format!("AutoMod request failed ({})", status),
            }));
        }
        Ok(())
    }

    pub async fn update_suspicious_user_status(
        broadcaster_id: &str,
        target_user_id: &str,
        status: &str,
    ) -> Result<()> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();
        let user_info = Self::get_user_info().await?;
        let moderator_id = &user_info.id;

        let url = format!(
            "https://api.twitch.tv/helix/moderation/suspicious_users?broadcaster_id={}&moderator_id={}",
            broadcaster_id, moderator_id
        );

        let payload = serde_json::json!({
            "user_id": target_user_id,
            "status": status // "RESTRICTED", "NO_TREATMENT", "ACTIVE_MONITORING"
        });

        let response = client
            .put(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let error_text = response.text().await.unwrap_or_default();
            error!(
                "[TwitchService] Failed to update suspicious user status: {}",
                error_text
            );
            return Err(anyhow::anyhow!(helix_refusal_text(status, &error_text)));
        }

        Ok(())
    }

    /// A user's Twitch bio (Helix `description`); empty when they have none.
    pub async fn get_user_bio(user_id: &str) -> Result<String> {
        let token = Self::get_token().await?;
        let response = crate::services::http::client()
            .get("https://api.twitch.tv/helix/users")
            .query(&[("id", user_id)])
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(anyhow::anyhow!("Twitch did not answer for that user ({})", response.status()));
        }
        let body: serde_json::Value = response.json().await?;
        Ok(body.pointer("/data/0/description").and_then(|v| v.as_str()).unwrap_or_default().to_string())
    }

    /// Update User Chat Color. `color` is what the viewer typed; it goes out in
    /// the form Helix takes (`helix_chat_color`), as an encoded query value so
    /// a hex value's "#" is not read as the URL's fragment.
    pub async fn update_user_chat_color(user_id: &str, color: &str) -> Result<()> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();
        let color = helix_chat_color(color);

        let response = client
            .put("https://api.twitch.tv/helix/chat/color")
            .query(&[("user_id", user_id), ("color", color.as_str())])
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let error_text = response.text().await.unwrap_or_default();
            error!("[TwitchService] Failed to update user chat color ({status}): {error_text}");
            return Err(anyhow::anyhow!(helix_refusal_text(status, &error_text)));
        }

        Ok(())
    }

    /// Live status for an arbitrary set of channels, by numeric user id.
    ///
    /// This is what makes a FAVOURITE work when you don't follow the channel:
    /// `get_followed_streams` only ever answers for your follow list, so a
    /// favourite outside it is invisible without this.
    ///
    /// Helix facts this depends on (dev.twitch.tv, verified 2026-08-27):
    ///   - up to 100 `user_id` values per request, hence the chunking;
    ///   - **`first` defaults to 20**, so a 100-id request WITHOUT `first=100`
    ///     answers for only 20 of them and the rest read as offline;
    ///   - offline channels are simply absent from the response, so every row
    ///     that comes back is live;
    ///   - an app OR user token is required. There is no client-credentials
    ///     path here (see `get_token`), so signed out this returns an error
    ///     rather than pretending nobody is live.
    ///
    /// A failing chunk warns and is skipped rather than failing the whole call:
    /// partial liveness beats none, and the next sweep retries anyway.
    pub async fn get_streams_by_user_ids(user_ids: &[String]) -> Result<Vec<TwitchStream>> {
        if user_ids.is_empty() {
            return Ok(vec![]);
        }

        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();
        let mut out: Vec<TwitchStream> = Vec::new();

        for chunk in user_ids.chunks(100) {
            let query = chunk
                .iter()
                .map(|id| format!("user_id={}", id))
                .collect::<Vec<_>>()
                .join("&");
            // `first=100` is load-bearing, not decoration. See the note above.
            let url = format!("https://api.twitch.tv/helix/streams?{}&first=100", query);

            let response = match client
                .get(&url)
                .header("Client-Id", CLIENT_ID)
                .header(AUTHORIZATION, format!("Bearer {}", token))
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    warn!("[TwitchService] get_streams_by_user_ids request failed: {}", e);
                    continue;
                }
            };

            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                warn!(
                    "[TwitchService] get_streams_by_user_ids non-success {}: {}",
                    status, body
                );
                continue;
            }

            let json = match response.json::<serde_json::Value>().await {
                Ok(j) => j,
                Err(e) => {
                    warn!("[TwitchService] get_streams_by_user_ids bad JSON: {}", e);
                    continue;
                }
            };

            let Some(arr) = json.get("data").and_then(|d| d.as_array()) else {
                continue;
            };

            match serde_json::from_value::<Vec<TwitchStream>>(serde_json::Value::Array(
                arr.clone(),
            )) {
                Ok(mut streams) => {
                    // Helix omits offline channels entirely, so everything here
                    // is live. Stamp it: the frontend lists filter on `is_live`,
                    // and Helix doesn't ship the field.
                    for s in &mut streams {
                        s.is_live = Some(true);
                    }
                    out.append(&mut streams);
                }
                Err(e) => warn!(
                    "[TwitchService] get_streams_by_user_ids failed to parse rows: {}",
                    e
                ),
            }
        }

        Ok(out)
    }

    /// `user_id -> (login, display name, avatar)` for a batch of Twitch users.
    ///
    /// Used to back-fill identity for favourites saved before the identity
    /// sidecar existed: those are bare ids, and without a name and a face they
    /// can't be drawn in the offline roster at all.
    pub async fn users_by_ids(
        ids: &[String],
    ) -> std::collections::HashMap<String, (String, String, Option<String>)> {
        let mut out = std::collections::HashMap::new();
        if ids.is_empty() {
            return out;
        }

        let Ok(token) = Self::get_token().await else {
            return out;
        };
        let client = crate::services::http::client().clone();

        for chunk in ids.chunks(100) {
            let query = chunk
                .iter()
                .map(|id| format!("id={}", id))
                .collect::<Vec<_>>()
                .join("&");
            let url = format!("https://api.twitch.tv/helix/users?{}", query);

            let Ok(response) = client
                .get(&url)
                .header("Client-Id", CLIENT_ID)
                .header(AUTHORIZATION, format!("Bearer {}", token))
                .send()
                .await
            else {
                continue;
            };
            if !response.status().is_success() {
                continue;
            }
            let Ok(json) = response.json::<serde_json::Value>().await else {
                continue;
            };
            let Some(arr) = json.get("data").and_then(|d| d.as_array()) else {
                continue;
            };
            for user in arr {
                let Some(id) = user.get("id").and_then(|v| v.as_str()) else {
                    continue;
                };
                let login = user
                    .get("login")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let display = user
                    .get("display_name")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .unwrap_or(&login)
                    .to_string();
                let avatar = user
                    .get("profile_image_url")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string());
                out.insert(id.to_string(), (login, display, avatar));
            }
        }

        out
    }

    /// Get User Chat Color for a batch of users, returning `user_id -> hex color`.
    /// Helix allows up to 100 ids per request and ignores duplicate/unknown ids.
    /// Degrades to an empty map on a missing token (anonymous viewing) or any
    /// request failure, so the chat render path always has a value to fall back
    /// on rather than surfacing an error.
    pub async fn get_user_chat_colors(
        user_ids: Vec<String>,
    ) -> std::collections::HashMap<String, String> {
        let mut out: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        if user_ids.is_empty() {
            return out;
        }

        let token = match Self::get_token().await {
            Ok(t) => t,
            Err(_) => return out, // anonymous / logged-out: no color lookup available
        };
        let client = crate::services::http::client().clone();

        for chunk in user_ids.chunks(100) {
            let query = chunk
                .iter()
                .map(|id| format!("user_id={}", id))
                .collect::<Vec<_>>()
                .join("&");
            let url = format!("https://api.twitch.tv/helix/chat/color?{}", query);

            let response = match client
                .get(&url)
                .header("Client-Id", CLIENT_ID)
                .header(AUTHORIZATION, format!("Bearer {}", token))
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    warn!("[TwitchService] get_user_chat_colors request failed: {}", e);
                    continue;
                }
            };

            if !response.status().is_success() {
                let error_text = response.text().await.unwrap_or_default();
                warn!(
                    "[TwitchService] get_user_chat_colors non-success: {}",
                    error_text
                );
                continue;
            }

            let body: serde_json::Value = match response.json().await {
                Ok(v) => v,
                Err(e) => {
                    warn!("[TwitchService] get_user_chat_colors parse failed: {}", e);
                    continue;
                }
            };

            if let Some(data) = body.get("data").and_then(|d| d.as_array()) {
                for item in data {
                    let uid = item.get("user_id").and_then(|v| v.as_str()).unwrap_or("");
                    let color = item.get("color").and_then(|v| v.as_str()).unwrap_or("");
                    if !uid.is_empty() && !color.is_empty() {
                        out.insert(uid.to_string(), color.to_string());
                    }
                }
            }
        }

        out
    }

    /// Block user
    pub async fn block_user(target_user_id: &str) -> Result<()> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();
        let user_info = Self::get_user_info().await?;

        let url = format!(
            "https://api.twitch.tv/helix/users/blocks?target_user_id={}",
            target_user_id
        );

        let response = client
            .put(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;

        if !response.status().is_success() && response.status() != 204 {
            let status = response.status().as_u16();
            let error_text = response.text().await.unwrap_or_default();
            error!("[TwitchService] Failed to block user: {}", error_text);
            return Err(anyhow::anyhow!(helix_refusal_text(status, &error_text)));
        }

        Ok(())
    }

    /// Unblock user
    pub async fn unblock_user(target_user_id: &str) -> Result<()> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();
        let user_info = Self::get_user_info().await?;

        let url = format!(
            "https://api.twitch.tv/helix/users/blocks?target_user_id={}",
            target_user_id
        );

        let response = client
            .delete(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;

        if !response.status().is_success() && response.status() != 204 {
            let status = response.status().as_u16();
            let error_text = response.text().await.unwrap_or_default();
            error!("[TwitchService] Failed to unblock user: {}", error_text);
            return Err(anyhow::anyhow!(helix_refusal_text(status, &error_text)));
        }

        Ok(())
    }

    /// Get Channel Moderators
    pub async fn get_channel_moderators(broadcaster_id: &str) -> Result<Vec<serde_json::Value>> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();

        let url = format!(
            "https://api.twitch.tv/helix/moderation/moderators?broadcaster_id={}",
            broadcaster_id
        );

        let response = client
            .get(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            error!(
                "[TwitchService] Failed to get moderators ({}): {}",
                status, error_text
            );
            return Err(anyhow::anyhow!("Failed to get moderators: HTTP {}", status));
        }

        let json: serde_json::Value = response.json().await?;

        if let Some(data) = json.get("data").and_then(|d| d.as_array()) {
            Ok(data.clone())
        } else {
            Ok(Vec::new())
        }
    }

    /// Get Channel VIPs
    pub async fn get_channel_vips(broadcaster_id: &str) -> Result<Vec<serde_json::Value>> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();

        let url = format!(
            "https://api.twitch.tv/helix/channels/vips?broadcaster_id={}",
            broadcaster_id
        );

        let response = client
            .get(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            error!(
                "[TwitchService] Failed to get VIPs ({}): {}",
                status, error_text
            );
            return Err(anyhow::anyhow!("Failed to get VIPs: HTTP {}", status));
        }

        let json: serde_json::Value = response.json().await?;

        if let Some(data) = json.get("data").and_then(|d| d.as_array()) {
            Ok(data.clone())
        } else {
            Ok(Vec::new())
        }
    }

    /// A channel's moderators and VIPs from Twitch's GQL, for any viewer (the
    /// Helix lists answer only the broadcaster): `{ moderators, vips }`, each
    /// member `{ id, login, displayName }`, plus `moderators_listed` and
    /// `vips_listed`, false when Twitch would not list that group to anyone
    /// the app could ask as, and `moderators_error` / `vips_error` set to what
    /// the viewer reads instead, so a refusal is never shown as "no moderators".
    pub async fn get_chatters_by_role(
        channel_login: &str,
        auth: Option<&TwitchAuthService>,
    ) -> Result<serde_json::Value> {
        let callers = role_list_callers(auth).await;
        let signed_in = callers.iter().any(GqlCaller::is_signed_in);
        let (moderators, vips) = tokio::join!(
            Self::channel_role_members(channel_login, ChannelRole::Moderators, &callers),
            Self::channel_role_members(channel_login, ChannelRole::Vips, &callers),
        );
        let moderators = role_members_or_error(moderators, "moderators", "moderator list", signed_in);
        let vips = role_members_or_error(vips, "VIPs", "VIP list", signed_in);
        Ok(serde_json::json!({
            "moderators_listed": moderators.is_ok(),
            "vips_listed": vips.is_ok(),
            "moderators": moderators.as_ref().cloned().unwrap_or_default(),
            "vips": vips.as_ref().cloned().unwrap_or_default(),
            "moderators_error": moderators.err(),
            "vips_error": vips.err(),
        }))
    }

    /// Everyone holding `role` in a channel, read as the first of `callers`
    /// Twitch lists the group to. None when it lists it to none of them.
    pub(crate) async fn channel_role_members(
        channel_login: &str,
        role: ChannelRole,
        callers: &[GqlCaller],
    ) -> Result<Option<Vec<serde_json::Value>>> {
        let login = channel_login.to_lowercase();
        let mut refused = false;
        let mut failure = None;
        for caller in callers {
            match Self::role_members_as(&login, role, caller).await {
                Ok(Some(members)) => return Ok(Some(members)),
                Ok(None) => refused = true,
                Err(e) => failure = Some(e),
            }
        }
        match failure {
            Some(e) if !refused => Err(e),
            _ => Ok(None),
        }
    }

    /// Every page of one role list, read as `caller`. None when Twitch would
    /// not list the group to that caller.
    async fn role_members_as(
        login: &str,
        role: ChannelRole,
        caller: &GqlCaller,
    ) -> Result<Option<Vec<serde_json::Value>>> {
        const MAX_PAGES: usize = 10;
        let mut members = Vec::new();
        let mut after: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let body = gql_as(caller, &role_page_request(login, role, after.as_deref())).await?;
            let Some((page, next)) = role_page(&body, role) else {
                debug!(
                    "[TwitchService] {} list for {login} refused as {}: {}",
                    role.field(),
                    caller.name(),
                    body.get("errors").cloned().unwrap_or_default()
                );
                return Ok(None);
            };
            members.extend(page);
            match next {
                Some(cursor) => after = Some(cursor),
                None => break,
            }
        }
        Ok(Some(members))
    }

    /// The chatters list any viewer can see, grouped by role: Twitch's public
    /// `channel.chatters` (the same `ChatViewers` query twitch.tv's own viewer
    /// list sends), read with the Android client id (the web client id
    /// fails Twitch's integrity check on it) and no token. It lists every
    /// broadcaster, moderator, VIP and staff member present, but at most 100
    /// viewers (it takes no paging arguments), with the full `count`. Chatters
    /// come back as logins only, so their ids and display names are looked up in
    /// one batch per 100. Anyone seen chatting in the channel in the last
    /// half hour is added to the viewers.
    pub async fn get_public_chatters(channel_login: &str) -> Result<serde_json::Value> {
        use std::collections::HashMap;

        let client = crate::services::http::client().clone();
        let android_id = env!("TWITCH_ANDROID_CLIENT_ID");
        let login = serde_json::to_string(&channel_login.to_lowercase())?;
        let query = format!(
            "query {{ channel(name: {login}) {{ chatters {{ count broadcasters {{ login }} moderators {{ login }} vips {{ login }} staff {{ login }} chatbots {{ login }} viewers {{ login }} }} }} }}"
        );
        let json: serde_json::Value = client
            .post("https://gql.twitch.tv/gql")
            .header("Client-Id", android_id)
            .json(&serde_json::json!({ "query": query }))
            .send()
            .await?
            .json()
            .await?;
        let chatters = json
            .pointer("/data/channel/chatters")
            .filter(|c| !c.is_null())
            .ok_or_else(|| anyhow::anyhow!("Twitch returned no chatters for {channel_login}"))?;
        let logins_of = |group: &str| -> Vec<String> {
            chatters
                .get(group)
                .and_then(|g| g.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|c| c.get("login").and_then(|l| l.as_str()).map(str::to_lowercase))
                        .collect()
                })
                .unwrap_or_default()
        };
        let broadcaster = logins_of("broadcasters");
        let mut moderators = logins_of("moderators");
        moderators.extend(logins_of("staff"));
        let vips = logins_of("vips");
        // Chat bots (Twitch lists them on their own) are present like viewers.
        let mut viewers = logins_of("viewers");
        viewers.extend(logins_of("chatbots"));

        // Ids and display names for everyone listed, 100 logins per lookup.
        let mut names: HashMap<String, (String, String)> = HashMap::new();
        let all: Vec<&String> = broadcaster.iter().chain(&moderators).chain(&vips).chain(&viewers).collect();
        for chunk in all.chunks(100) {
            let logins = serde_json::to_string(&chunk)?;
            let q = format!("query {{ users(logins: {logins}) {{ id login displayName }} }}");
            let resp: serde_json::Value = match client
                .post("https://gql.twitch.tv/gql")
                .header("Client-Id", android_id)
                .json(&serde_json::json!({ "query": q }))
                .send()
                .await
            {
                Ok(r) => r.json().await.unwrap_or_default(),
                Err(_) => continue,
            };
            for u in resp.pointer("/data/users").and_then(|u| u.as_array()).into_iter().flatten() {
                if let (Some(id), Some(login)) = (u.get("id").and_then(|v| v.as_str()), u.get("login").and_then(|v| v.as_str())) {
                    let display = u.get("displayName").and_then(|v| v.as_str()).unwrap_or(login);
                    names.insert(login.to_lowercase(), (id.to_string(), display.to_string()));
                }
            }
        }
        // Twitch lists at most 100 viewers, so everyone seen chatting here in
        // the last half hour joins them, with the id and name their messages
        // carried.
        let since_ms = chrono::Utc::now().timestamp_millis() - CHATTED_RECENTLY_MS;
        let listed_logins: std::collections::HashSet<String> =
            broadcaster.iter().chain(&moderators).chain(&vips).chain(&viewers).cloned().collect();
        for (login, id, display) in
            crate::services::chat_history::ChatHistory::recent_chatters(channel_login, since_ms)
        {
            if listed_logins.contains(&login) {
                continue;
            }
            names.entry(login.clone()).or_insert((id, if display.is_empty() { login.clone() } else { display }));
            viewers.push(login);
        }

        let entry = |login: &String| {
            let (id, display) = names.get(login).cloned().unwrap_or_else(|| (String::new(), login.clone()));
            serde_json::json!({ "user_id": id, "user_login": login, "user_name": display })
        };
        let sorted = |mut v: Vec<String>| {
            v.sort();
            v.dedup();
            v
        };
        let listed = broadcaster.len() + moderators.len() + vips.len() + viewers.len();
        let total = chatters.get("count").and_then(|c| c.as_u64()).unwrap_or(listed as u64) as usize;
        Ok(serde_json::json!({
            "broadcaster": broadcaster.iter().map(entry).collect::<Vec<_>>(),
            "moderators": sorted(moderators).iter().map(entry).collect::<Vec<_>>(),
            "vips": sorted(vips).iter().map(entry).collect::<Vec<_>>(),
            "viewers": sorted(viewers).iter().map(entry).collect::<Vec<_>>(),
            "total": total.max(listed),
            "truncated": listed < total,
        }))
    }

    /// The chatters list for a channel, grouped by role: the full roster when
    /// the signed-in user moderates or owns the channel, otherwise the public
    /// list every viewer can see (`get_public_chatters`).
    pub async fn get_channel_chatters(
        broadcaster_id: &str,
        channel_login: &str,
        auth: Option<&TwitchAuthService>,
    ) -> Result<serde_json::Value> {
        match Self::get_moderator_chatters(broadcaster_id, channel_login, auth).await {
            Ok(full) => Ok(full),
            // Not a moderator here, signed out, or a token without the roster
            // scope: everyone can still have the public list.
            Err(e) => {
                debug!("[TwitchService] Full chatters roster unavailable ({e}); using the public list");
                Self::get_public_chatters(channel_login).await
            }
        }
    }

    /// Get the full official chatters roster for a channel, grouped by role.
    ///
    /// Uses Helix Get Chatters, which requires the caller to be a moderator or the
    /// broadcaster of the channel and to hold the `moderator:read:chatters` scope.
    /// That endpoint returns logins only, so we compose it with the GQL Mods/VIPs
    /// lookup (`get_chatters_by_role`, read signed in) to bucket each chatter
    /// into broadcaster / moderators / vips / viewers.
    async fn get_moderator_chatters(
        broadcaster_id: &str,
        channel_login: &str,
        auth: Option<&TwitchAuthService>,
    ) -> Result<serde_json::Value> {
        use std::collections::HashSet;

        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();

        // The roster endpoint requires moderator_id = the authenticated user's id.
        let moderator_id = Self::get_user_info().await?.id;

        // 1. Page through the official chatters roster (first=1000 is the max page).
        const MAX_PAGES: usize = 10; // ~10k chatters; surfaced via `truncated` if hit.
        let mut roster: Vec<serde_json::Value> = Vec::new();
        let mut cursor: Option<String> = None;
        let mut truncated = false;
        for page in 0..MAX_PAGES {
            let mut url = format!(
                "https://api.twitch.tv/helix/chat/chatters?broadcaster_id={}&moderator_id={}&first=1000",
                broadcaster_id, moderator_id
            );
            if let Some(ref c) = cursor {
                url.push_str(&format!("&after={}", c));
            }

            let response = client
                .get(&url)
                .header("Client-Id", CLIENT_ID)
                .header(AUTHORIZATION, format!("Bearer {}", token))
                .send()
                .await?;

            if !response.status().is_success() {
                let status = response.status();
                let error_text = response.text().await.unwrap_or_default();
                error!(
                    "[TwitchService] Failed to get chatters ({}): {}",
                    status, error_text
                );
                // 401 here means the token predates the moderator:read:chatters scope;
                // surface a distinct signal so the UI can prompt a one-time re-login.
                if status.as_u16() == 401 {
                    return Err(anyhow::anyhow!("REAUTH"));
                }
                return Err(anyhow::anyhow!("Failed to get chatters: HTTP {}", status));
            }

            let json: serde_json::Value = response.json().await?;
            if let Some(data) = json.get("data").and_then(|d| d.as_array()) {
                roster.extend(data.iter().cloned());
            }
            cursor = json
                .get("pagination")
                .and_then(|p| p.get("cursor"))
                .and_then(|c| c.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string());
            if cursor.is_none() {
                break;
            }
            if page == MAX_PAGES - 1 {
                truncated = true;
            }
        }

        // 2. Role sets from the GQL Mods/VIPs lookup (best-effort: a list that
        //    could not be read leaves those chatters among the viewers, and is
        //    logged rather than taken for "no moderators").
        let mut mod_logins: HashSet<String> = HashSet::new();
        let mut vip_logins: HashSet<String> = HashSet::new();
        if let Ok(roles) = Self::get_chatters_by_role(channel_login, auth).await {
            for key in ["moderators_error", "vips_error"] {
                if let Some(e) = roles.get(key).and_then(|e| e.as_str()) {
                    warn!("[TwitchService] chatters roles for {channel_login}: {e}");
                }
            }
            if let Some(mods) = roles.get("moderators").and_then(|m| m.as_array()) {
                for m in mods {
                    if let Some(login) = m.get("login").and_then(|l| l.as_str()) {
                        mod_logins.insert(login.to_lowercase());
                    }
                }
            }
            if let Some(vips) = roles.get("vips").and_then(|v| v.as_array()) {
                for v in vips {
                    if let Some(login) = v.get("login").and_then(|l| l.as_str()) {
                        vip_logins.insert(login.to_lowercase());
                    }
                }
            }
        }

        // 3. Bucket each chatter by role.
        fn login_key(v: &serde_json::Value) -> String {
            v.get("user_login")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_lowercase()
        }

        let broadcaster_login = channel_login.to_lowercase();
        let mut broadcaster: Vec<serde_json::Value> = Vec::new();
        let mut moderators: Vec<serde_json::Value> = Vec::new();
        let mut vips: Vec<serde_json::Value> = Vec::new();
        let mut viewers: Vec<serde_json::Value> = Vec::new();

        for chatter in &roster {
            let login = login_key(chatter);
            if login == broadcaster_login {
                broadcaster.push(chatter.clone());
            } else if mod_logins.contains(&login) {
                moderators.push(chatter.clone());
            } else if vip_logins.contains(&login) {
                vips.push(chatter.clone());
            } else {
                viewers.push(chatter.clone());
            }
        }

        // Alphabetical within each bucket, matching the native viewer list.
        moderators.sort_by_cached_key(login_key);
        vips.sort_by_cached_key(login_key);
        viewers.sort_by_cached_key(login_key);

        let total = roster.len();
        Ok(serde_json::json!({
            "broadcaster": broadcaster,
            "moderators": moderators,
            "vips": vips,
            "viewers": viewers,
            "total": total,
            "truncated": truncated,
        }))
    }

    /// Send Chat Announcement
    pub async fn send_chat_announcement(
        broadcaster_id: &str,
        message: &str,
        color: Option<&str>,
    ) -> Result<()> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();
        let user_info = Self::get_user_info().await?;
        let moderator_id = &user_info.id;

        let url = format!(
            "https://api.twitch.tv/helix/chat/announcements?broadcaster_id={}&moderator_id={}",
            broadcaster_id, moderator_id
        );

        let mut payload = serde_json::json!({
            "message": message
        });

        if let Some(c) = color {
            payload["color"] = serde_json::json!(c);
        }

        let response = client
            .post(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await?;

        if !response.status().is_success() && response.status() != 204 {
            let status = response.status().as_u16();
            let error_text = response.text().await.unwrap_or_default();
            error!(
                "[TwitchService] Failed to send announcement: {}",
                error_text
            );
            return Err(anyhow::anyhow!(helix_refusal_text(status, &error_text)));
        }

        Ok(())
    }

    /// Send Shoutout
    pub async fn send_shoutout(broadcaster_id: &str, target_user_id: &str) -> Result<()> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();
        let user_info = Self::get_user_info().await?;
        let moderator_id = &user_info.id;

        let url = format!(
            "https://api.twitch.tv/helix/chat/shoutouts?from_broadcaster_id={}&to_broadcaster_id={}&moderator_id={}",
            broadcaster_id, target_user_id, moderator_id
        );

        let response = client
            .post(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;

        if !response.status().is_success() && response.status() != 204 {
            let status = response.status().as_u16();
            let error_text = response.text().await.unwrap_or_default();
            error!("[TwitchService] Failed to send shoutout: {}", error_text);
            return Err(anyhow::anyhow!(helix_refusal_text(status, &error_text)));
        }

        Ok(())
    }

    /// Start Commercial
    pub async fn start_commercial(broadcaster_id: &str, length: u32) -> Result<()> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();

        let url = "https://api.twitch.tv/helix/channels/commercial";

        let payload = serde_json::json!({
            "broadcaster_id": broadcaster_id,
            "length": length
        });

        let response = client
            .post(url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let error_text = response.text().await.unwrap_or_default();
            error!("[TwitchService] Failed to start commercial: {}", error_text);
            return Err(anyhow::anyhow!(helix_refusal_text(status, &error_text)));
        }

        Ok(())
    }

    /// Create a poll. Broadcaster-only (`channel:manage:polls`).
    ///
    /// `duration` is seconds (Twitch allows 15..1800). Channel-point voting is
    /// only sent when a cost is set, because Twitch rejects the pair when the
    /// enable flag is true with a zero cost.
    pub async fn create_poll(
        broadcaster_id: &str,
        title: &str,
        choices: &[String],
        duration: u32,
        points_per_vote: Option<u32>,
    ) -> Result<serde_json::Value> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();

        let mut payload = serde_json::json!({
            "broadcaster_id": broadcaster_id,
            "title": title,
            "duration": duration,
            "choices": choices
                .iter()
                .map(|c| serde_json::json!({ "title": c }))
                .collect::<Vec<_>>(),
        });
        if let Some(cost) = points_per_vote.filter(|c| *c > 0) {
            payload["channel_points_voting_enabled"] = serde_json::json!(true);
            payload["channel_points_per_vote"] = serde_json::json!(cost);
        }

        Self::helix_json_post("https://api.twitch.tv/helix/polls", &token, payload, "create poll")
            .await
    }

    /// Add an AutoMod blocked term. Needs `moderator:manage:blocked_terms`.
    ///
    /// Twitch takes the ids as query params and only the phrase in the body.
    pub async fn add_blocked_term(broadcaster_id: &str, text: &str) -> Result<serde_json::Value> {
        let token = Self::get_token().await?;
        let moderator_id = Self::get_user_info().await?.id;
        let url = format!(
            "https://api.twitch.tv/helix/moderation/blocked_terms?broadcaster_id={}&moderator_id={}",
            broadcaster_id, moderator_id
        );
        Self::helix_json_post(&url, &token, serde_json::json!({ "text": text }), "add blocked term")
            .await
    }

    /// Remove a blocked term by its phrase.
    ///
    /// The delete endpoint only takes the term's id, so this looks the phrase up
    /// first, across every page of the list. Matching is case-insensitive
    /// because Twitch stores terms lowercased but echoes back whatever case
    /// they were added in.
    pub async fn remove_blocked_term(broadcaster_id: &str, text: &str) -> Result<()> {
        let terms = Self::fetch_blocked_terms(broadcaster_id).await?;
        let id = blocked_term_id_for(&terms, text)
            .ok_or_else(|| anyhow::anyhow!("\"{}\" is not in the blocked terms list", text.trim()))?;
        Self::remove_blocked_term_by_id(broadcaster_id, &id).await
    }

    /// Remove one blocked term by its id. Twitch answers 204 even for an id it
    /// no longer has, so removing a term twice is not an error.
    pub async fn remove_blocked_term_by_id(broadcaster_id: &str, id: &str) -> Result<()> {
        let token = Self::get_token().await?;
        let moderator_id = Self::get_user_info().await?.id;
        let client = crate::services::http::client().clone();
        let response = client
            .delete("https://api.twitch.tv/helix/moderation/blocked_terms")
            .query(&[
                ("broadcaster_id", broadcaster_id),
                ("moderator_id", moderator_id.as_str()),
                ("id", id),
            ])
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            error!("[TwitchService] Failed to remove blocked term: {} {}", status, body);
            return Err(anyhow::anyhow!(blocked_terms_refusal(status, &body)));
        }
        Ok(())
    }

    /// Every blocked term in the channel, newest first, in the words the
    /// moderation sheets show. Needs `moderator:read:blocked_terms`.
    pub async fn list_blocked_terms(broadcaster_id: &str) -> Result<Vec<BlockedTermRow>> {
        let terms = Self::fetch_blocked_terms(broadcaster_id).await?;
        Ok(blocked_term_rows(terms, Utc::now().timestamp_millis()))
    }

    /// The channel's blocked terms as Twitch lists them. Helix gives at most
    /// 100 a page, so this follows the cursor, up to `BLOCKED_TERM_PAGES`.
    async fn fetch_blocked_terms(broadcaster_id: &str) -> Result<Vec<RawBlockedTerm>> {
        let token = Self::get_token().await?;
        let moderator_id = Self::get_user_info().await?.id;
        let client = crate::services::http::client().clone();
        let mut terms = Vec::new();
        let mut after: Option<String> = None;
        for _ in 0..BLOCKED_TERM_PAGES {
            let mut query = vec![
                ("broadcaster_id", broadcaster_id.to_string()),
                ("moderator_id", moderator_id.clone()),
                ("first", "100".to_string()),
            ];
            if let Some(cursor) = after.take() {
                query.push(("after", cursor));
            }
            let response = client
                .get("https://api.twitch.tv/helix/moderation/blocked_terms")
                .query(&query)
                .header("Client-Id", CLIENT_ID)
                .header(AUTHORIZATION, format!("Bearer {}", token))
                .send()
                .await?;
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            if !status.is_success() {
                error!("[TwitchService] Failed to list blocked terms: {} {}", status, body);
                return Err(anyhow::anyhow!(blocked_terms_refusal(status.as_u16(), &body)));
            }
            let page = serde_json::from_str::<serde_json::Value>(&body).unwrap_or(serde_json::Value::Null);
            let (mut rows, cursor) = blocked_terms_page(&page);
            let last = rows.is_empty() || cursor.is_none();
            terms.append(&mut rows);
            if last {
                break;
            }
            after = cursor;
        }
        Ok(terms)
    }

    /// The channel's currently-running poll, if any.
    ///
    /// Twitch exposes no GQL read for polls (the web client learns them over
    /// PubSub), which is why the overlay is event-fed. Helix does have one, and
    /// it became reachable when `channel:manage:polls` was added — so commands
    /// that need the live poll id no longer have to guess at it.
    pub async fn get_active_poll(broadcaster_id: &str) -> Result<Option<serde_json::Value>> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();
        let url = format!(
            "https://api.twitch.tv/helix/polls?broadcaster_id={}&first=1",
            broadcaster_id
        );
        let response = client
            .get(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;
        let value = Self::helix_json_result(response, "get polls").await?;
        // Helix returns the most recent poll regardless of state, so an ended
        // one must not be reported as live.
        let poll = value["data"].get(0).cloned().filter(|p| {
            matches!(p["status"].as_str(), Some("ACTIVE"))
        });
        Ok(poll)
    }

    /// One of the broadcaster's own polls by id, whatever its state: its
    /// status (ACTIVE, COMPLETED, TERMINATED, ARCHIVED, MODERATED, INVALID).
    /// Helix only answers for the token's own channel.
    pub async fn get_poll_status(broadcaster_id: &str, poll_id: &str) -> Result<Option<String>> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();
        let url = format!(
            "https://api.twitch.tv/helix/polls?broadcaster_id={}&id={}",
            urlencoding::encode(broadcaster_id),
            urlencoding::encode(poll_id)
        );
        let response = client
            .get(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;
        let value = Self::helix_json_result(response, "get poll").await?;
        Ok(value["data"]
            .get(0)
            .filter(|p| p["id"].as_str() == Some(poll_id))
            .and_then(|p| p["status"].as_str())
            .map(str::to_string))
    }

    /// End a poll. `status` is TERMINATED (show the result) or ARCHIVED (hide it).
    pub async fn end_poll(broadcaster_id: &str, poll_id: &str, status: &str) -> Result<serde_json::Value> {
        let token = Self::get_token().await?;
        let payload = serde_json::json!({
            "broadcaster_id": broadcaster_id,
            "id": poll_id,
            "status": status,
        });
        Self::helix_json_patch("https://api.twitch.tv/helix/polls", &token, payload, "end poll").await
    }

    /// Create a prediction. Broadcaster-only (`channel:manage:predictions`).
    ///
    /// Note the field is `prediction_window`, not `prediction_window_seconds`.
    pub async fn create_prediction(
        broadcaster_id: &str,
        title: &str,
        outcomes: &[String],
        window: u32,
    ) -> Result<serde_json::Value> {
        let token = Self::get_token().await?;
        let payload = serde_json::json!({
            "broadcaster_id": broadcaster_id,
            "title": title,
            "prediction_window": window,
            "outcomes": outcomes
                .iter()
                .map(|o| serde_json::json!({ "title": o }))
                .collect::<Vec<_>>(),
        });
        Self::helix_json_post(
            "https://api.twitch.tv/helix/predictions",
            &token,
            payload,
            "create prediction",
        )
        .await
    }

    /// Lock, resolve or cancel a prediction. `status` is LOCKED, RESOLVED or
    /// CANCELED; RESOLVED additionally requires the winning outcome id.
    pub async fn end_prediction(
        broadcaster_id: &str,
        prediction_id: &str,
        status: &str,
        winning_outcome_id: Option<&str>,
    ) -> Result<serde_json::Value> {
        let token = Self::get_token().await?;
        let mut payload = serde_json::json!({
            "broadcaster_id": broadcaster_id,
            "id": prediction_id,
            "status": status,
        });
        if let Some(outcome) = winning_outcome_id {
            payload["winning_outcome_id"] = serde_json::json!(outcome);
        }
        Self::helix_json_patch(
            "https://api.twitch.tv/helix/predictions",
            &token,
            payload,
            "end prediction",
        )
        .await
    }

    /// The channel's prediction while it can still be ended: the latest one,
    /// when it is ACTIVE or LOCKED. Asked fresh each time, so a prediction
    /// started on the dashboard or before StreamNook opened is found too.
    /// Broadcaster-only, like the rest of Helix predictions.
    pub async fn get_open_prediction(broadcaster_id: &str) -> Result<Option<serde_json::Value>> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();
        let url = format!(
            "https://api.twitch.tv/helix/predictions?broadcaster_id={}&first=1",
            broadcaster_id
        );
        let response = client
            .get(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;
        let value = Self::helix_json_result(response, "get predictions").await?;
        Ok(value["data"]
            .get(0)
            .cloned()
            .filter(|p| matches!(p["status"].as_str(), Some("ACTIVE" | "LOCKED"))))
    }

    /// Lock, cancel (refunding every bet) or pay out the channel's open
    /// prediction. `outcome` names the winner for a payout, by its position
    /// (1-based) or its title. Answers the line to show the user.
    pub async fn finish_open_prediction(
        broadcaster_id: &str,
        action: PredictionEnd,
        outcome: Option<&str>,
    ) -> std::result::Result<String, String> {
        let open = Self::get_open_prediction(broadcaster_id)
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "No prediction is running right now".to_string())?;
        let (status, winner) = prediction_end_plan(&open, action, outcome)?;
        let id = open["id"].as_str().unwrap_or_default();
        Self::end_prediction(broadcaster_id, id, status, winner.as_deref())
            .await
            .map_err(|e| e.to_string())?;
        Ok(match action {
            PredictionEnd::Lock => "Prediction locked",
            PredictionEnd::Cancel => "Prediction cancelled, points refunded",
            PredictionEnd::Complete => "Prediction paid out",
        }
        .to_string())
    }

    /// POST a JSON body to Helix and return the parsed response. Surfaces
    /// Twitch's own error text, which names the offending field on a 400.
    async fn helix_json_post(
        url: &str,
        token: &str,
        payload: serde_json::Value,
        what: &str,
    ) -> Result<serde_json::Value> {
        let client = crate::services::http::client().clone();
        let response = client
            .post(url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .json(&payload)
            .send()
            .await?;
        Self::helix_json_result(response, what).await
    }

    async fn helix_json_patch(
        url: &str,
        token: &str,
        payload: serde_json::Value,
        what: &str,
    ) -> Result<serde_json::Value> {
        let client = crate::services::http::client().clone();
        let response = client
            .patch(url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .json(&payload)
            .send()
            .await?;
        Self::helix_json_result(response, what).await
    }

    async fn helix_json_result(
        response: reqwest::Response,
        what: &str,
    ) -> Result<serde_json::Value> {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            error!("[TwitchService] Failed to {}: {} {}", what, status, body);
            // Twitch puts a human-readable reason in `message`; pass it through
            // so the composer can show why rather than a generic failure.
            let reason = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| v.get("message").and_then(|m| m.as_str()).map(String::from))
                .unwrap_or_else(|| format!("HTTP {}", status));
            return Err(anyhow::anyhow!("{}", reason));
        }
        Ok(serde_json::from_str(&body).unwrap_or(serde_json::Value::Null))
    }

    /// Start Raid
    pub async fn start_raid(broadcaster_id: &str, target_user_id: &str) -> Result<()> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();

        let url = format!(
            "https://api.twitch.tv/helix/raids?from_broadcaster_id={}&to_broadcaster_id={}",
            broadcaster_id, target_user_id
        );

        let response = client
            .post(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let error_text = response.text().await.unwrap_or_default();
            error!("[TwitchService] Failed to start raid: {}", error_text);
            return Err(anyhow::anyhow!(helix_refusal_text(status, &error_text)));
        }

        Ok(())
    }

    /// Cancel Raid
    pub async fn cancel_raid(broadcaster_id: &str) -> Result<()> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();

        let url = format!(
            "https://api.twitch.tv/helix/raids?broadcaster_id={}",
            broadcaster_id
        );

        let response = client
            .delete(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;

        if !response.status().is_success() && response.status() != 204 {
            let status = response.status().as_u16();
            let error_text = response.text().await.unwrap_or_default();
            error!("[TwitchService] Failed to cancel raid: {}", error_text);
            return Err(anyhow::anyhow!(helix_refusal_text(status, &error_text)));
        }

        Ok(())
    }

    /// Create Stream Marker
    pub async fn create_stream_marker(user_id: &str, description: Option<&str>) -> Result<()> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();

        let url = "https://api.twitch.tv/helix/streams/markers";

        let mut payload = serde_json::json!({
            "user_id": user_id
        });

        if let Some(desc) = description {
            if !desc.is_empty() {
                payload["description"] = serde_json::json!(desc);
            }
        }

        let response = client
            .post(url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let error_text = response.text().await.unwrap_or_default();
            error!(
                "[TwitchService] Failed to create stream marker: {}",
                error_text
            );
            return Err(anyhow::anyhow!(helix_refusal_text(status, &error_text)));
        }

        Ok(())
    }

    /// The scopes Twitch says `token` carries (`oauth2/validate`).
    pub(crate) async fn token_scopes(token: &str) -> Result<Vec<String>> {
        let response = crate::services::http::client()
            .get("https://id.twitch.tv/oauth2/validate")
            .header(AUTHORIZATION, format!("OAuth {token}"))
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(anyhow::anyhow!("Twitch did not accept the sign-in (HTTP {})", response.status().as_u16()));
        }
        let data: serde_json::Value = response.json().await?;
        Ok(data["scopes"]
            .as_array()
            .map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect())
            .unwrap_or_default())
    }

    /// Warn a chat user (Helix: POST /moderation/warnings)
    pub async fn warn_chat_user(
        broadcaster_id: &str,
        target_user_id: &str,
        reason: &str,
    ) -> Result<()> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();
        // The warnings scope is optional: a sign-in from before it was asked for
        // lacks it, and is asked to sign in again rather than refused blind.
        let granted = Self::token_scopes(&token).await?;
        if lacks_scope(&granted, "moderator:manage:warnings") {
            return Err(anyhow::anyhow!(WARN_SCOPE_MISSING));
        }
        let user_info = Self::get_user_info().await?;
        let moderator_id = &user_info.id;

        let url = format!(
            "https://api.twitch.tv/helix/moderation/warnings?broadcaster_id={}&moderator_id={}",
            broadcaster_id, moderator_id
        );

        let payload = serde_json::json!({
            "data": {
                "user_id": target_user_id,
                "reason": reason
            }
        });

        let response = client
            .post(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let error_text = response.text().await.unwrap_or_default();
            error!("[TwitchService] Failed to warn user: {}", error_text);
            return Err(anyhow::anyhow!(helix_refusal_text(status, &error_text)));
        }

        Ok(())
    }

    /// Update Shield Mode (Helix: PUT /moderation/shield_mode)
    pub async fn update_shield_mode(broadcaster_id: &str, is_active: bool) -> Result<()> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();
        let user_info = Self::get_user_info().await?;
        let moderator_id = &user_info.id;

        let url = format!(
            "https://api.twitch.tv/helix/moderation/shield_mode?broadcaster_id={}&moderator_id={}",
            broadcaster_id, moderator_id
        );

        let payload = serde_json::json!({
            "is_active": is_active
        });

        let response = client
            .put(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let error_text = response.text().await.unwrap_or_default();
            error!(
                "[TwitchService] Failed to update shield mode: {} {}",
                status, error_text
            );
            return Err(anyhow::anyhow!("{}", helix_refusal_text(status, &error_text)));
        }

        Ok(())
    }
}

/// The `Video` fields a Continue Watching card is built from. Shared by the
/// by-id hydrate and the viewing-history sync so both read cards one way.
pub(crate) const VOD_CARD_FIELDS: &str = "id title lengthSeconds status \
     previewThumbnailURL(width: 440, height: 248) \
     game { displayName } \
     owner { id login displayName profileImageURL(width: 70) roles { isPartner } }";

/// One `Video` node selected with `VOD_CARD_FIELDS`, as a card refresh.
pub(crate) fn vod_refresh_from_node(
    id: &str,
    v: &serde_json::Value,
) -> crate::services::vod_progress_service::VodRefresh {
    let text = |ptr: &str| {
        v.pointer(ptr)
            .and_then(|t| t.as_str())
            .unwrap_or_default()
            .to_string()
    };
    crate::services::vod_progress_service::VodRefresh {
        video_id: id.to_string(),
        title: text("/title"),
        channel_name: text("/owner/displayName"),
        thumbnail_url: text("/previewThumbnailURL"),
        length_secs: v
            .get("lengthSeconds")
            .and_then(|n| n.as_f64())
            .unwrap_or(0.0),
        finished: text("/status").eq_ignore_ascii_case("RECORDED"),
        profile_image_url: text("/owner/profileImageURL"),
        partner: v.pointer("/owner/roles/isPartner").and_then(|b| b.as_bool()),
        game_name: text("/game/displayName"),
        channel_id: text("/owner/id"),
    }
}

/// Twitch's named chat colors, spelled the way Helix takes them.
const CHAT_COLOR_NAMES: [&str; 15] = [
    "blue", "blue_violet", "cadet_blue", "chocolate", "coral", "dodger_blue", "firebrick", "golden_rod", "green",
    "hot_pink", "orange_red", "red", "sea_green", "spring_green", "yellow_green",
];

/// A typed chat color in the form Helix takes: a named color in its
/// snake_case spelling ("SpringGreen", "spring green" and "springgreen" all
/// read as `spring_green`), or a six-digit hex value with its "#". Anything
/// else goes as typed, for Twitch to refuse with its own reason.
pub(crate) fn helix_chat_color(typed: &str) -> String {
    let typed = typed.trim();
    let hex = typed.trim_start_matches('#');
    if hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return format!("#{hex}");
    }
    let squashed: String = typed.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase();
    CHAT_COLOR_NAMES
        .iter()
        .find(|name| name.replace('_', "") == squashed)
        .map(|name| name.to_string())
        .unwrap_or_else(|| typed.to_string())
}

/// What a viewer reads when a role list could not be loaded.
fn role_error_text(what: &str, error: &str, signed_in: bool) -> String {
    if !signed_in && error.eq_ignore_ascii_case("unauthenticated") {
        format!("Sign in to Twitch to see this channel's {what}.")
    } else {
        format!("Couldn't load this channel's {what} (Twitch: {error}).")
    }
}

/// One `channel_role_members` answer as the members, or what the viewer reads
/// when Twitch would not list the group (`what` names it, `list` its list).
fn role_members_or_error(
    read: Result<Option<Vec<serde_json::Value>>>,
    what: &str,
    list: &str,
    signed_in: bool,
) -> std::result::Result<Vec<serde_json::Value>, String> {
    match read {
        Ok(Some(members)) => Ok(members),
        Ok(None) if signed_in => Err(format!("Twitch would not share this channel's {list}.")),
        Ok(None) => Err(role_error_text(what, "unauthenticated", false)),
        Err(e) => Err(role_error_text(what, &e.to_string(), signed_in)),
    }
}

/// What `/lockprediction`, `/cancelprediction` and `/completeprediction` do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PredictionEnd {
    Lock,
    Cancel,
    Complete,
}

/// The Helix status (and winning outcome id) that ends `open` as asked, or
/// why it can't be done.
fn prediction_end_plan(
    open: &serde_json::Value,
    action: PredictionEnd,
    outcome: Option<&str>,
) -> std::result::Result<(&'static str, Option<String>), String> {
    match action {
        PredictionEnd::Lock if open["status"].as_str() == Some("LOCKED") => {
            Err("The prediction is already locked".into())
        }
        PredictionEnd::Lock => Ok(("LOCKED", None)),
        PredictionEnd::Cancel => Ok(("CANCELED", None)),
        PredictionEnd::Complete => {
            let wanted = outcome
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or("Say which outcome won, by number or title")?;
            let outcomes = open["outcomes"].as_array().map(Vec::as_slice).unwrap_or_default();
            let picked = wanted
                .parse::<usize>()
                .ok()
                .and_then(|n| n.checked_sub(1))
                .and_then(|i| outcomes.get(i))
                .or_else(|| {
                    outcomes
                        .iter()
                        .find(|o| o["title"].as_str().is_some_and(|t| t.eq_ignore_ascii_case(wanted)))
                });
            let id = picked
                .and_then(|o| o["id"].as_str())
                .ok_or_else(|| format!("No outcome matches \"{wanted}\""))?;
            Ok(("RESOLVED", Some(id.to_string())))
        }
    }
}

/// How many pages of blocked terms one listing reads before it stops: 5,000
/// terms, far past any real channel's list.
const BLOCKED_TERM_PAGES: usize = 50;

/// One blocked term as the moderation sheets list it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BlockedTermRow {
    pub id: String,
    pub text: String,
    /// "Added 3 days ago"; empty when Twitch gave no time.
    pub added: String,
    /// "Expires in 3d" for AutoMod's temporary terms; `None` for terms that stay.
    pub expires: Option<String>,
    pub created_at_ms: i64,
    pub expires_at_ms: Option<i64>,
}

/// A blocked term as Helix lists it, before it is put into words.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RawBlockedTerm {
    id: String,
    text: String,
    created_at_ms: i64,
    expires_at_ms: Option<i64>,
}

/// One Helix blocked-terms page: its terms and the cursor to the next page.
fn blocked_terms_page(page: &serde_json::Value) -> (Vec<RawBlockedTerm>, Option<String>) {
    let ms = |v: &serde_json::Value| {
        v.as_str()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|d| d.timestamp_millis())
    };
    let terms: Vec<RawBlockedTerm> = page["data"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter_map(|r| {
                    let id = r["id"].as_str().filter(|s| !s.is_empty())?;
                    let text = r["text"].as_str()?;
                    Some(RawBlockedTerm {
                        id: id.to_string(),
                        text: text.to_string(),
                        created_at_ms: ms(&r["created_at"]).unwrap_or(0),
                        expires_at_ms: ms(&r["expires_at"]),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let cursor = page["pagination"]["cursor"]
        .as_str()
        .filter(|c| !c.is_empty())
        .map(String::from);
    (terms, cursor)
}

/// The listed terms as rows, newest first, each id once.
fn blocked_term_rows(terms: Vec<RawBlockedTerm>, now_ms: i64) -> Vec<BlockedTermRow> {
    let mut seen = std::collections::HashSet::new();
    let mut rows: Vec<BlockedTermRow> = terms
        .into_iter()
        .filter(|t| seen.insert(t.id.clone()))
        .map(|t| BlockedTermRow {
            added: blocked_term_added_words(t.created_at_ms, now_ms),
            expires: t.expires_at_ms.map(|at| blocked_term_expiry_words(at, now_ms)),
            created_at_ms: t.created_at_ms,
            expires_at_ms: t.expires_at_ms,
            id: t.id,
            text: t.text,
        })
        .collect();
    rows.sort_by(|a, b| {
        b.created_at_ms
            .cmp(&a.created_at_ms)
            .then_with(|| a.text.to_lowercase().cmp(&b.text.to_lowercase()))
    });
    rows
}

/// "Added 3 days ago", or nothing when Twitch gave no time.
fn blocked_term_added_words(created_at_ms: i64, now_ms: i64) -> String {
    let ago = crate::services::mod_history::ago(created_at_ms, now_ms);
    if ago.is_empty() {
        ago
    } else {
        format!("Added {ago}")
    }
}

/// How long until AutoMod lets a temporary term go: "Expires in 3d", "in 5h",
/// "in 12m". Days and hours round down; minutes round up, so the last
/// seconds still read as a minute and the last minute of an hour as "1h".
fn blocked_term_expiry_words(expires_at_ms: i64, now_ms: i64) -> String {
    let secs = (expires_at_ms - now_ms) / 1000;
    match secs {
        s if s <= 0 => "Expiring now".to_string(),
        s if s < 3_600 => match (s + 59) / 60 {
            60 => "Expires in 1h".to_string(),
            m => format!("Expires in {m}m"),
        },
        s if s < 86_400 => format!("Expires in {}h", s / 3_600),
        s => format!("Expires in {}d", s / 86_400),
    }
}

/// The id of the listed term matching `text`, ignoring case and outer spaces.
fn blocked_term_id_for(terms: &[RawBlockedTerm], text: &str) -> Option<String> {
    let wanted = text.trim().to_lowercase();
    terms
        .iter()
        .find(|t| t.text.trim().to_lowercase() == wanted)
        .map(|t| t.id.clone())
}

/// A refused blocked-terms call as a sentence the sheets can show as is.
fn blocked_terms_refusal(status: u16, body: &str) -> String {
    let reason = helix_refusal_text(status, body);
    let mut chars = reason.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod prediction_end_tests {
    use super::{prediction_end_plan, PredictionEnd};

    fn open(status: &str) -> serde_json::Value {
        serde_json::json!({
            "id": "p1",
            "status": status,
            "outcomes": [
                { "id": "o-yes", "title": "Yes" },
                { "id": "o-no", "title": "No" },
            ],
        })
    }

    #[test]
    fn lock_and_cancel_need_no_outcome() {
        assert_eq!(prediction_end_plan(&open("ACTIVE"), PredictionEnd::Lock, None), Ok(("LOCKED", None)));
        assert_eq!(prediction_end_plan(&open("LOCKED"), PredictionEnd::Cancel, None), Ok(("CANCELED", None)));
        assert!(prediction_end_plan(&open("LOCKED"), PredictionEnd::Lock, None).is_err());
    }

    #[test]
    fn a_payout_names_its_winner_by_number_or_title() {
        let by_number = prediction_end_plan(&open("LOCKED"), PredictionEnd::Complete, Some("2"));
        assert_eq!(by_number, Ok(("RESOLVED", Some("o-no".into()))));
        let by_title = prediction_end_plan(&open("ACTIVE"), PredictionEnd::Complete, Some(" yes "));
        assert_eq!(by_title, Ok(("RESOLVED", Some("o-yes".into()))));
        assert!(prediction_end_plan(&open("LOCKED"), PredictionEnd::Complete, None).is_err());
        assert!(prediction_end_plan(&open("LOCKED"), PredictionEnd::Complete, Some("3")).is_err());
        assert!(prediction_end_plan(&open("LOCKED"), PredictionEnd::Complete, Some("0")).is_err());
        assert!(prediction_end_plan(&open("LOCKED"), PredictionEnd::Complete, Some("Maybe")).is_err());
    }
}

/// Orders pins newest first. Only one mod pin is active at a time, but the
/// read can list more than one entry (a pin it has not dropped yet beside the
/// one replacing it), and the banner shows the first.
fn newest_pin_first(pins: &mut [serde_json::Value]) {
    pins.sort_by(|a, b| {
        let at = |p: &serde_json::Value| {
            p.get("started_at")
                .and_then(|v| v.as_str())
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        };
        at(b).cmp(&at(a))
    });
}

/// Pin lengths Helix accepts, in seconds (30 s to 30 min).
const PIN_DURATION_RANGE: std::ops::RangeInclusive<u32> = 30..=1800;

fn check_pin_duration(duration_seconds: Option<u32>) -> Result<()> {
    match duration_seconds {
        Some(d) if !PIN_DURATION_RANGE.contains(&d) => Err(anyhow::anyhow!(
            "A pin can last 30 seconds to 30 minutes; leave the length off to keep it until the stream ends"
        )),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod pin_tests {
    use super::{check_pin_duration, newest_pin_first};

    #[test]
    fn the_newest_pin_comes_first() {
        let mut pins = vec![
            serde_json::json!({"message_id": "old", "started_at": "2026-01-01T10:00:00Z"}),
            serde_json::json!({"message_id": "none", "started_at": ""}),
            serde_json::json!({"message_id": "new", "started_at": "2026-01-01T10:05:00.5Z"}),
        ];
        newest_pin_first(&mut pins);
        let order: Vec<&str> = pins.iter().map(|p| p["message_id"].as_str().unwrap()).collect();
        assert_eq!(order, ["new", "old", "none"]);
    }

    #[test]
    fn pin_length_follows_helix_bounds() {
        assert!(check_pin_duration(None).is_ok());
        assert!(check_pin_duration(Some(30)).is_ok());
        assert!(check_pin_duration(Some(1800)).is_ok());
        assert!(check_pin_duration(Some(29)).is_err());
        assert!(check_pin_duration(Some(3600)).is_err());
    }
}

#[cfg(test)]
mod scope_check_tests {
    use super::{missing_scopes, requested_scopes, SCOPES};

    fn granted(scopes: &str) -> Vec<String> {
        scopes.split_whitespace().map(str::to_string).collect()
    }

    #[test]
    fn a_token_with_every_scope_owes_nothing() {
        assert!(missing_scopes(SCOPES, &granted(SCOPES)).is_empty());
        // Extra scopes Twitch granted beyond ours are not a problem.
        let mut more = granted(SCOPES);
        more.push("analytics:read:games".into());
        assert!(missing_scopes(SCOPES, &more).is_empty());
    }

    #[test]
    fn a_token_from_before_a_new_scope_names_exactly_that_scope() {
        let before: Vec<String> = granted(SCOPES)
            .into_iter()
            .filter(|s| s != "moderator:manage:shield_mode")
            .collect();
        assert_eq!(missing_scopes(SCOPES, &before), ["moderator:manage:shield_mode"]);
    }

    #[test]
    fn warn_asks_for_the_scope_only_when_the_sign_in_lacks_it() {
        use super::lacks_scope;
        assert!(lacks_scope(&granted(SCOPES), "moderator:manage:warnings"));
        assert!(!lacks_scope(&granted(&requested_scopes()), "moderator:manage:warnings"));
    }

    #[test]
    fn an_optional_scope_is_asked_for_but_never_resets_a_sign_in() {
        // Every new sign-in asks for it...
        assert!(requested_scopes().split_whitespace().any(|s| s == "moderator:manage:warnings"));
        assert!(!SCOPES.split_whitespace().any(|s| s == "moderator:manage:warnings"));
        // ...but a stored token without it misses nothing the token check requires,
        // and that check clears accounts (reset_all) only when something is missing.
        assert!(missing_scopes(SCOPES, &granted(SCOPES)).is_empty());
        let with_it: Vec<String> = granted(&requested_scopes());
        assert!(missing_scopes(SCOPES, &with_it).is_empty());
    }

    #[test]
    fn scope_names_match_exactly() {
        // A read scope does not stand in for the manage scope of the same name.
        let required = "moderator:read:blocked_terms moderator:manage:blocked_terms";
        assert_eq!(
            missing_scopes(required, &granted("moderator:read:blocked_terms")),
            ["moderator:manage:blocked_terms"]
        );
        assert_eq!(missing_scopes(required, &[]).len(), 2);
    }
}

#[cfg(test)]
mod moderation_refusal_tests {
    use super::helix_refusal_text;

    #[test]
    fn twitch_reasons_come_through_in_plain_words() {
        let body = |m: &str| serde_json::json!({ "error": "Bad Request", "status": 400, "message": m }).to_string();
        assert_eq!(
            helix_refusal_text(400, &body("The user specified in the user_id field may not be banned.")),
            "that user can't be banned or timed out here"
        );
        assert_eq!(
            helix_refusal_text(400, &body("The user specified in the user_id field is already banned.")),
            "they are already banned"
        );
        assert_eq!(
            helix_refusal_text(403, &body("The user in moderator_id is not one of the broadcaster's moderators.")),
            "you are not a moderator in this channel"
        );
        // Anything else is Twitch's own sentence, or the status when there is none.
        assert_eq!(helix_refusal_text(400, &body("Something new.")), "Something new.");
        assert_eq!(helix_refusal_text(502, "<html>"), "Twitch answered HTTP 502");
    }
}

#[cfg(test)]
mod discovery_tests {
    use super::*;

    #[test]
    fn public_gql_rows_read_like_helix() {
        let user = serde_json::json!({
            "id": "233741947", "login": "strogo", "displayName": "StRoGo", "profileImageURL": "https://x/a.png",
            "stream": { "id": "317", "type": "live", "createdAt": "2026-10-09T07:59:52Z", "viewersCount": 38699,
                        "title": "FURIA vs MOUZ", "game": { "id": "32399", "displayName": "Counter-Strike" } }
        });
        let row = live_row_from_gql(&user).expect("a live user is a row");
        assert_eq!((row.user_id.as_str(), row.user_login.as_str(), row.viewer_count), ("233741947", "strogo", 38699));
        assert_eq!((row.game_id.as_str(), row.game_name.as_str()), ("32399", "Counter-Strike"));
        assert_eq!(row.started_at, "2026-10-09T07:59:52Z");
        assert!(row.thumbnail_url.contains("live_user_strogo-{width}x{height}"));
        assert!(live_row_from_gql(&serde_json::json!({ "id": "1", "login": "x", "stream": null })).is_none());
    }

    #[test]
    fn gql_language_normalizes_to_helix_form() {
        assert_eq!(normalize_gql_language("FR"), "fr");
        assert_eq!(normalize_gql_language("EN"), "en");
        assert_eq!(normalize_gql_language("ZH_HK"), "zh-hk");
        assert_eq!(normalize_gql_language("OTHER"), "other");
    }

    #[test]
    fn recommended_edges_map_to_streams() {
        let edges: Vec<serde_json::Value> = serde_json::from_str(
            r#"[
            {"node": {
                "id": "317579120484", "title": "ranked", "viewersCount": 5701,
                "language": "FR", "createdAt": "2026-08-25T18:20:15Z",
                "previewImageURL": "https://static-cdn.jtvnw.net/previews-ttv/live_user_kamet0-{width}x{height}.jpg",
                "freeformTags": [{"name": "Francais"}],
                "game": {"id": "21779", "name": "League of Legends", "displayName": "League of Legends"},
                "broadcaster": {"id": "27115917", "login": "kamet0", "displayName": "Kamet0",
                    "profileImageURL": "https://example/p.png",
                    "roles": {"isPartner": true, "isAffiliate": false}}
            }},
            {"node": {"id": "1", "title": "no broadcaster", "viewersCount": 1, "broadcaster": null}},
            {"node": {
                "id": "2", "title": "cantonese", "viewersCount": 9, "language": "ZH_HK",
                "previewImageURL": "", "game": null,
                "broadcaster": {"id": "5", "login": "hk_chan", "displayName": "",
                    "roles": {"isPartner": false, "isAffiliate": true}}
            }}
        ]"#,
        )
        .unwrap();

        let streams = TwitchService::map_recommended_edges(&edges);
        assert_eq!(streams.len(), 2, "null-broadcaster edge must be skipped");

        let first = &streams[0];
        assert_eq!(first.user_login, "kamet0");
        assert_eq!(first.user_name, "Kamet0");
        assert_eq!(first.viewer_count, 5701);
        assert_eq!(first.game_id, "21779");
        assert_eq!(first.game_name, "League of Legends");
        assert_eq!(first.language.as_deref(), Some("fr"));
        assert_eq!(first.broadcaster_type.as_deref(), Some("partner"));
        assert_eq!(first.started_at, "2026-08-25T18:20:15Z");
        assert_eq!(first.is_live, Some(true));
        assert_eq!(first.tags.as_deref(), Some(&["Francais".to_string()][..]));

        let second = &streams[1];
        assert_eq!(second.user_name, "hk_chan", "empty displayName falls back to login");
        assert_eq!(second.language.as_deref(), Some("zh-hk"));
        assert_eq!(second.broadcaster_type.as_deref(), Some("affiliate"));
        assert_eq!(second.game_name, "");
    }

    #[test]
    fn signed_out_categories_read_like_helix_games() {
        let games = serde_json::json!({
            "edges": [
                { "cursor": "c1", "node": { "id": "509658", "name": "Just Chatting", "displayName": "Just Chatting",
                  "boxArtURL": "https://static-cdn.jtvnw.net/ttv-boxart/509658-{width}x{height}.jpg", "viewersCount": 247184 } },
                { "cursor": "c2", "node": null },
                { "cursor": "c3", "node": { "id": "", "name": "no id" } }
            ],
            "pageInfo": { "hasNextPage": true }
        });
        let (rows, next) = top_games_page(&serde_json::json!({ "data": { "games": games } }));
        assert_eq!(rows.len(), 1, "null and id-less nodes are skipped");
        assert_eq!(rows[0].id, "509658");
        assert_eq!(rows[0].name, "Just Chatting");
        assert_eq!(rows[0].viewer_count, Some(247184));
        assert!(rows[0].box_art_url.ends_with("509658-{width}x{height}.jpg"));
        assert_eq!(next.as_deref(), Some("gql:c3"));
    }

    #[test]
    fn signed_out_cursors_page_only_on_the_public_side() {
        assert_eq!(public_cursor(Some("gql:abc")), Some("abc"));
        assert_eq!(public_cursor(Some("eyJiIjp7")), None, "a Helix cursor stays on Helix");
        assert_eq!(public_cursor(None), None);
        let last_page = serde_json::json!({ "edges": [{ "cursor": "z", "node": {} }], "pageInfo": { "hasNextPage": false } });
        assert_eq!(public_next_cursor(Some(&last_page)), None);
        assert_eq!(public_next_cursor(None), None);
    }

    #[test]
    fn signed_out_category_streams_carry_the_category() {
        let node = serde_json::json!({
            "id": "320736562012", "title": "news", "viewersCount": 25168, "createdAt": "2026-10-13T18:08:56Z",
            "type": "live", "language": "EN",
            "previewImageURL": "https://static-cdn.jtvnw.net/previews-ttv/live_user_hasanabi-{width}x{height}.jpg",
            "broadcaster": { "id": "207813352", "login": "hasanabi", "displayName": "HasanAbi",
                "profileImageURL": "https://x/p.jpeg", "roles": { "isPartner": true, "isAffiliate": false } }
        });
        let row = stream_row_from_gql_node(&node, ("509658", "Just Chatting")).expect("a row");
        assert_eq!((row.game_id.as_str(), row.game_name.as_str()), ("509658", "Just Chatting"));
        assert_eq!((row.user_login.as_str(), row.viewer_count), ("hasanabi", 25168));
        assert_eq!(row.broadcaster_type.as_deref(), Some("partner"));
        assert_eq!(row.language.as_deref(), Some("en"));
        assert!(row.thumbnail_url.contains("{width}x{height}"));
        assert!(stream_row_from_gql_node(&serde_json::json!({ "id": "1", "broadcaster": null }), ("", "")).is_none());
    }

    #[test]
    fn signed_out_channel_search_rows_read_like_helix_search() {
        let live = serde_json::json!({
            "id": "71092938", "login": "xqc", "displayName": "xQc", "profileImageURL": "https://x/xqc-300x300.jpeg",
            "roles": { "isPartner": true, "isAffiliate": false },
            "broadcastSettings": { "title": "LOCK IN", "game": { "id": "32982", "displayName": "Grand Theft Auto V" } },
            "stream": { "id": "320737343964", "viewersCount": 26948, "createdAt": "2026-10-13T19:31:06Z",
                "previewImageURL": "https://static-cdn.jtvnw.net/previews-ttv/live_user_xqc-{width}x{height}.jpg",
                "game": { "id": "32982", "displayName": "Grand Theft Auto V" } }
        });
        let row = channel_row_from_gql(&live).expect("a row");
        assert_eq!((row.id.as_str(), row.user_id.as_str()), ("320737343964", "71092938"));
        assert_eq!((row.is_live, row.viewer_count), (Some(true), 26948));
        assert_eq!(row.title, "LOCK IN");
        assert!(row.thumbnail_url.contains("live_user_xqc"));
        assert_eq!(row.profile_image_url.as_deref(), Some("https://x/xqc-300x300.jpeg"));

        let offline = serde_json::json!({
            "id": "251183247", "login": "xqcisoffline", "displayName": "", "profileImageURL": "https://x/d.png",
            "roles": { "isPartner": false, "isAffiliate": false },
            "broadcastSettings": { "title": "f", "game": { "id": "509658", "displayName": "Just Chatting" } },
            "stream": null
        });
        let row = channel_row_from_gql(&offline).expect("a row");
        assert_eq!((row.id.as_str(), row.user_name.as_str()), ("251183247", "xqcisoffline"));
        assert_eq!((row.is_live, row.started_at.as_str()), (Some(false), ""));
        assert_eq!((row.game_name.as_str(), row.thumbnail_url.as_str()), ("Just Chatting", "https://x/d.png"));
        assert_eq!(row.broadcaster_type, None);
        assert!(channel_row_from_gql(&serde_json::json!({ "id": "", "login": "x" })).is_none());
    }

    #[test]
    fn only_login_shaped_queries_ask_for_the_exact_user() {
        assert!(is_twitch_login("xqc"));
        assert!(is_twitch_login("some_user_42"));
        assert!(!is_twitch_login("two words"));
        assert!(!is_twitch_login(""));
        assert!(!is_twitch_login("UPPER"));
        assert!(!is_twitch_login(&"a".repeat(26)));
    }
}

#[cfg(test)]
mod resolve_stream_tests {
    use super::*;
    use crate::models::user::{ChannelInfo, UserInfo};

    fn user() -> UserInfo {
        UserInfo {
            id: "1".into(),
            login: "a".into(),
            display_name: "A".into(),
            email: None,
            profile_image_url: Some("https://x/a.png".into()),
            broadcaster_type: Some("partner".into()),
        }
    }

    fn row() -> TwitchStream {
        serde_json::from_value(serde_json::json!({
            "id": "9", "user_id": "1", "user_name": "A", "user_login": "a", "title": "t",
            "viewer_count": 5, "game_id": "2", "game_name": "g", "thumbnail_url": "",
            "started_at": "2026-01-01T00:00:00Z"
        }))
        .unwrap()
    }

    #[test]
    fn live_row_gets_avatar_mark_and_liveness() {
        let s = TwitchService::fuse_stream_row(user(), "a", Ok(Some(row())), None);
        assert_eq!(s.is_live, Some(true));
        assert_eq!(s.profile_image_url.as_deref(), Some("https://x/a.png"));
        assert_eq!(s.broadcaster_type.as_deref(), Some("partner"));
        assert_eq!(s.viewer_count, 5);
    }

    #[test]
    fn offline_row_carries_channel_title_and_is_not_live() {
        let ch = ChannelInfo {
            broadcaster_id: "1".into(),
            broadcaster_name: "A".into(),
            game_name: "g".into(),
            title: "t".into(),
        };
        let s = TwitchService::fuse_stream_row(user(), "a", Ok(None), Some(ch));
        assert_eq!(s.is_live, Some(false));
        assert_eq!(s.title, "t");
        assert_eq!(s.game_name, "g");
        assert_eq!(s.started_at, "");
        assert_eq!(s.user_id, "1");
    }

    #[test]
    fn failed_probe_is_unknown_not_offline() {
        let s = TwitchService::fuse_stream_row(user(), "a", Err(anyhow::anyhow!("5xx")), None);
        assert_eq!(s.is_live, None);
        assert_eq!(s.profile_image_url.as_deref(), Some("https://x/a.png"));
    }

    #[test]
    fn login_validation() {
        assert!(TwitchService::valid_login("xqc_2"));
        assert!(TwitchService::valid_login("a"));
        assert!(!TwitchService::valid_login("foo&id=1"));
        assert!(!TwitchService::valid_login("Mixed"));
        assert!(!TwitchService::valid_login(""));
        assert!(!TwitchService::valid_login("abcdefghijklmnopqrstuvwxyz"));
    }
}

/// Settles each channel search row's liveness from Helix's streams answer.
/// A row the answer lists is live. A row whose lookup was answered but which
/// it leaves out has ended: Helix search keeps `is_live` true for a while after
/// a stream stops, and such a row drew as a live card with 0 viewers that
/// opened onto nothing. A row whose lookup failed keeps search's word.
pub fn mark_search_liveness(
    rows: &mut [TwitchStream],
    live: &std::collections::HashMap<String, serde_json::Value>,
    checked: &std::collections::HashSet<String>,
) {
    for row in rows {
        if live.contains_key(&row.user_id) {
            row.is_live = Some(true);
        } else if checked.contains(&row.user_id) {
            row.is_live = Some(false);
            row.viewer_count = 0;
        }
    }
}

/// Order Twitch channel-search rows by how well they answer what was typed.
///
/// Helix returns its own order, which is not "best match first": a live
/// channel whose login IS the query can come back fifteenth, and every picker
/// shows only the first few rows, so typing a streamer's exact name could fail
/// to show them. Tiers: exact login, login starting with the query, display
/// name starting with it, contains it, anything else; live before offline
/// inside a tier, then more viewers. Stable, so Twitch's order breaks ties.
pub fn rank_search_results(query: &str, rows: &mut [TwitchStream]) {
    let q = query.trim().trim_start_matches('@').to_lowercase();
    if q.is_empty() {
        return;
    }
    let tier = |s: &TwitchStream| -> u8 {
        let login = s.user_login.to_lowercase();
        let name = s.user_name.to_lowercase();
        if login == q {
            0
        } else if login.starts_with(&q) {
            1
        } else if name.starts_with(&q) {
            2
        } else if login.contains(&q) || name.contains(&q) {
            3
        } else {
            4
        }
    };
    rows.sort_by(|a, b| {
        tier(a)
            .cmp(&tier(b))
            .then_with(|| b.is_live.unwrap_or(false).cmp(&a.is_live.unwrap_or(false)))
            .then_with(|| b.viewer_count.cmp(&a.viewer_count))
    });
}

#[cfg(test)]
mod category_rank_tests {
    use super::{rank_category_results, search_page, top_games_page, CategoryRow};

    #[test]
    fn the_directory_and_search_show_one_viewer_total() {
        let node = serde_json::json!({ "id": "32982", "name": "Grand Theft Auto V", "displayName": "Grand Theft Auto V",
            "boxArtURL": "https://static-cdn.jtvnw.net/ttv-boxart/32982_IGDB-{width}x{height}.jpg", "viewersCount": 41803 });
        let (directory, _) = top_games_page(&serde_json::json!({ "data": { "games": {
            "edges": [{ "cursor": "c1", "node": node.clone() }], "pageInfo": { "hasNextPage": false } } } }));
        let searched = search_page(&serde_json::json!({ "data": { "exact": node.clone(),
            "searchCategories": { "edges": [{ "node": node }] } } }));
        assert_eq!(directory[0], searched[0], "the same category reads the same in both answers");
        assert_eq!(directory[0].viewer_count, Some(41803), "Twitch's total, not a sum of some streams");
    }

    #[test]
    fn a_category_nobody_streams_reads_zero_and_a_missing_exact_match_is_skipped() {
        let searched = search_page(&serde_json::json!({ "data": { "exact": null, "searchCategories": { "edges": [
            { "node": { "id": "1", "name": "Just Cause 3", "boxArtURL": "", "viewersCount": null } }] } } }));
        assert_eq!(searched.len(), 1);
        assert_eq!(searched[0].viewer_count, Some(0));
    }

    fn row(id: &str, name: &str, viewers: Option<u64>) -> CategoryRow {
        CategoryRow { id: id.into(), name: name.into(), box_art_url: String::new(), viewer_count: viewers }
    }

    fn names(rows: &[CategoryRow]) -> Vec<&str> {
        rows.iter().map(|r| r.name.as_str()).collect()
    }

    #[test]
    fn the_exact_name_leads_ignoring_case() {
        let rows = vec![row("1", "Artifact", Some(28)), row("2", "Art Alive", Some(0)), row("3", "Art", Some(7419))];
        assert_eq!(names(&rank_category_results("art", rows, 40))[0], "Art");
        let rows = vec![row("1", "VALORANT Mobile", Some(2)), row("2", "VALORANT", Some(27043))];
        assert_eq!(names(&rank_category_results(" valorant ", rows, 40))[0], "VALORANT");
    }

    #[test]
    fn prefix_beats_the_rest_and_viewers_order_each_tier() {
        let rows = vec![
            row("1", "Tom Clancy's Rainbow Six", Some(2)),
            row("2", "Rainbow Six Mobile", Some(0)),
            row("3", "Tom Clancy's Rainbow Six Extraction", Some(40)),
            row("4", "Rainbow Six Siege", Some(6792)),
        ];
        assert_eq!(
            names(&rank_category_results("rainbow six", rows, 40)),
            ["Rainbow Six Siege", "Rainbow Six Mobile", "Tom Clancy's Rainbow Six Extraction", "Tom Clancy's Rainbow Six"]
        );
    }

    #[test]
    fn accents_do_not_hide_the_names_that_match() {
        let rows = vec![
            row("1", "Pokemon Auto Chess", Some(25)),
            row("2", "Pokémon Community Game", Some(2700)),
            row("3", "Pocket Monster", Some(0)),
            row("4", "Pokémon", Some(527)),
            row("5", "Pokémon FireRed/LeafGreen", Some(2000)),
        ];
        assert_eq!(
            names(&rank_category_results("pokemon", rows, 40)),
            ["Pokémon", "Pokémon Community Game", "Pokémon FireRed/LeafGreen", "Pokemon Auto Chess", "Pocket Monster"]
        );
    }

    #[test]
    fn ties_keep_twitchs_order_and_missing_viewers_count_as_none_live() {
        let rows = vec![row("1", "Just Cause 3", None), row("2", "Just Cause 4", Some(0)), row("3", "Just Chess", None)];
        assert_eq!(names(&rank_category_results("just", rows, 40)), ["Just Cause 3", "Just Cause 4", "Just Chess"]);
    }

    #[test]
    fn a_repeated_id_keeps_its_first_place_and_the_limit_cuts() {
        let rows = vec![row("9", "Art", Some(7419)), row("1", "Artifact", Some(28)), row("9", "Art", Some(7419)), row("2", "Artisan TD", Some(0))];
        let ranked = rank_category_results("art", rows, 2);
        assert_eq!(names(&ranked), ["Art", "Artifact"]);
    }

    #[test]
    fn rows_serialize_under_helix_field_names() {
        let json = serde_json::to_value(row("509660", "Art", Some(7419))).unwrap();
        assert_eq!(json["box_art_url"], "");
        assert_eq!(json["viewer_count"], 7419);
        assert!(serde_json::to_value(row("1", "A", None)).unwrap().get("viewer_count").is_none());
    }
}

#[cfg(test)]
mod search_rank_tests {
    use super::{rank_search_results, TwitchStream};

    fn row(login: &str, live: bool, viewers: u32) -> TwitchStream {
        TwitchStream {
            id: login.into(),
            user_id: login.into(),
            user_name: login.into(),
            user_login: login.into(),
            title: String::new(),
            viewer_count: viewers,
            game_id: String::new(),
            game_name: String::new(),
            thumbnail_url: String::new(),
            started_at: String::new(),
            broadcaster_type: None,
            profile_image_url: None,
            is_live: Some(live),
            tags: None,
            language: None,
            drops_earnable: false,
        }
    }

    fn logins(rows: &[TwitchStream]) -> Vec<&str> {
        rows.iter().map(|r| r.user_login.as_str()).collect()
    }

    #[test]
    fn the_exact_login_leads_wherever_twitch_put_it() {
        let mut rows: Vec<_> = (0..20).map(|i| row(&format!("other{i}"), true, 5000)).collect();
        rows.push(row("loostzao", true, 300));
        rank_search_results("Loostzao", &mut rows);
        assert_eq!(rows[0].user_login, "loostzao");
    }

    #[test]
    fn prefix_beats_contains_and_live_leads_its_tier() {
        let mut rows = vec![
            row("themoonfan", true, 9000),
            row("moonoffline", false, 0),
            row("moonlive", true, 40),
            row("moonbig", true, 900),
        ];
        rank_search_results("moon", &mut rows);
        assert_eq!(logins(&rows), ["moonbig", "moonlive", "moonoffline", "themoonfan"]);
    }

    #[test]
    fn the_streams_answer_settles_liveness_search_got_wrong() {
        use super::mark_search_liveness;
        use std::collections::{HashMap, HashSet};
        // Search still calls `dnd` live after its stream ended; streams does not list it.
        let mut rows = vec![row("dnd", true, 0), row("onair", false, 0), row("unasked", true, 0)];
        let live: HashMap<_, _> = [("onair".to_string(), serde_json::json!({}))].into();
        let checked: HashSet<_> = ["dnd".to_string(), "onair".to_string()].into();
        mark_search_liveness(&mut rows, &live, &checked);
        assert_eq!(rows[0].is_live, Some(false));
        assert_eq!(rows[1].is_live, Some(true));
        // Its lookup failed, so search's word stands.
        assert_eq!(rows[2].is_live, Some(true));
    }
}

#[cfg(test)]
mod web_login_tests {
    use super::TwitchService;

    fn state_of(url: &str) -> String {
        reqwest::Url::parse(url)
            .unwrap()
            .query_pairs()
            .find(|(k, _)| k == "state")
            .map(|(_, v)| v.into_owned())
            .unwrap()
    }

    // One test, in order: the pending state is process-wide.
    #[tokio::test]
    async fn a_redirect_is_only_taken_with_the_state_it_was_sent_with() {
        let url = TwitchService::begin_web_login().unwrap();
        assert!(url.contains("force_verify=false"), "an existing session goes straight to Authorize");
        assert!(url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A3000%2Fcallback"));
        let state = state_of(&url);

        let wrong = TwitchService::web_login_token(Some("c".into()), Some("other".into()), None).await;
        assert!(wrong.unwrap_err().to_string().contains("state mismatch"));
        // A state answers one redirect: the right one now finds nothing waiting.
        let replay = TwitchService::web_login_token(Some("c".into()), Some(state), None).await;
        assert!(replay.unwrap_err().to_string().contains("No sign-in is waiting"));

        let url = TwitchService::begin_web_login().unwrap();
        let declined =
            TwitchService::web_login_token(None, Some(state_of(&url)), Some("access_denied".into())).await;
        assert!(declined.unwrap_err().to_string().contains("access_denied"));

        let url = TwitchService::begin_web_login().unwrap();
        let no_code = TwitchService::web_login_token(None, Some(state_of(&url)), None).await;
        assert!(no_code.unwrap_err().to_string().contains("authorization code"));
    }
}

#[cfg(test)]
mod chat_color_tests {
    use super::helix_chat_color;

    #[test]
    fn named_colors_take_twitchs_snake_case_spelling() {
        assert_eq!(helix_chat_color("SpringGreen"), "spring_green");
        assert_eq!(helix_chat_color("spring green"), "spring_green");
        assert_eq!(helix_chat_color("springgreen"), "spring_green");
        assert_eq!(helix_chat_color("Red"), "red");
        assert_eq!(helix_chat_color("GoldenRod"), "golden_rod");
    }

    #[test]
    fn hex_values_keep_their_hash_with_or_without_one() {
        assert_eq!(helix_chat_color("#ff69b4"), "#ff69b4");
        assert_eq!(helix_chat_color("ff69b4"), "#ff69b4");
    }

    #[test]
    fn anything_else_goes_as_typed_for_twitch_to_refuse() {
        assert_eq!(helix_chat_color("rainbow"), "rainbow");
        assert_eq!(helix_chat_color("#ff69"), "#ff69");
    }
}

#[cfg(test)]
mod role_list_answer_tests {
    use super::role_error_text;

    #[test]
    fn the_viewer_reads_why_the_list_is_missing() {
        assert_eq!(role_error_text("moderators", "unauthenticated", false), "Sign in to Twitch to see this channel's moderators.");
        assert_eq!(
            role_error_text("moderators", "unauthenticated", true),
            "Couldn't load this channel's moderators (Twitch: unauthenticated)."
        );
        assert!(!role_error_text("VIPs", "service error", true).contains("no VIPs"));
    }
}

/// A role a channel hands out, as GQL's `User` lists it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelRole {
    Moderators,
    Vips,
}

impl ChannelRole {
    /// The connection on GQL's `User`.
    pub fn field(self) -> &'static str {
        match self {
            ChannelRole::Moderators => "mods",
            ChannelRole::Vips => "vips",
        }
    }
}

/// Who a role-list read goes out as. Twitch lists a channel's moderators only
/// to a signed-in viewer (an anonymous read answers `unauthenticated`), so the
/// read tries the twitch.tv web session the app's sign-in created, then the
/// rewards (Drops) sign-in, then no one.
pub enum GqlCaller {
    WebSession(String),
    Rewards(String),
    Anonymous,
}

impl GqlCaller {
    fn name(&self) -> &'static str {
        match self {
            GqlCaller::WebSession(_) => "the web session",
            GqlCaller::Rewards(_) => "the rewards sign-in",
            GqlCaller::Anonymous => "an anonymous viewer",
        }
    }

    pub fn is_signed_in(&self) -> bool {
        !matches!(self, GqlCaller::Anonymous)
    }
}

/// Every caller a role-list read can go out as, best first.
pub async fn role_list_callers(auth: Option<&TwitchAuthService>) -> Vec<GqlCaller> {
    let mut callers = Vec::new();
    if let Some(auth) = auth {
        if let Ok(token) = crate::services::twitch_web_gql::web_session_token(auth).await {
            callers.push(GqlCaller::WebSession(token));
        }
    }
    if let Ok(token) = crate::services::drops_auth_service::DropsAuthService::get_token().await {
        callers.push(GqlCaller::Rewards(token));
    }
    callers.push(GqlCaller::Anonymous);
    callers
}

/// POST one GQL operation as `caller`.
async fn gql_as(caller: &GqlCaller, body: &serde_json::Value) -> Result<serde_json::Value> {
    use crate::services::twitch_web_gql::post;
    let json = match caller {
        GqlCaller::WebSession(token) => post(Some(token), body.clone(), "RoleList").await,
        GqlCaller::Anonymous => post(None, body.clone(), "RoleList").await,
        GqlCaller::Rewards(token) => rewards_gql(token, body).await,
    };
    json.map_err(|e| anyhow::anyhow!(e))
}

/// A GQL read as the rewards sign-in, which belongs to the Android client id.
async fn rewards_gql(token: &str, body: &serde_json::Value) -> std::result::Result<serde_json::Value, String> {
    let resp = crate::services::http::client()
        .post("https://gql.twitch.tv/gql")
        .header("Client-Id", env!("TWITCH_ANDROID_CLIENT_ID"))
        .header(AUTHORIZATION, format!("OAuth {token}"))
        .header("X-Device-Id", uuid::Uuid::new_v4().simple().to_string())
        .json(body)
        .send()
        .await
        .map_err(|e| format!("GQL request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("GQL answered {}", resp.status()));
    }
    resp.json().await.map_err(|e| format!("GQL response was not JSON: {e}"))
}

/// One page of a channel's moderators or VIPs, 100 at a time.
pub fn role_page_request(login: &str, role: ChannelRole, after: Option<&str>) -> serde_json::Value {
    let field = role.field();
    serde_json::json!({
        "operationName": "ChannelRoleList",
        "query": format!(
            "query ChannelRoleList($login: String!, $after: Cursor) {{ user(login: $login) {{ \
             {field}(first: 100, after: $after) {{ edges {{ cursor node {{ id login displayName }} }} \
             pageInfo {{ hasNextPage }} }} }} }}"
        ),
        "variables": { "login": login, "after": after },
    })
}

/// The members on one `role_page_request` answer and the cursor of the next
/// page. None when Twitch did not list the group (its answer to an anonymous
/// read of `mods`: an `unauthenticated` error and a null field).
pub fn role_page(body: &serde_json::Value, role: ChannelRole) -> Option<(Vec<serde_json::Value>, Option<String>)> {
    let user = body.pointer("/data/user")?;
    if user.is_null() {
        // No channel by that name holds no roles; an error beside it is a refusal.
        return body.get("errors").is_none().then(|| (Vec::new(), None));
    }
    let list = user.get(role.field()).filter(|l| !l.is_null())?;
    let edges = list.get("edges").and_then(|e| e.as_array()).map(Vec::as_slice).unwrap_or_default();
    let members = edges
        .iter()
        .filter_map(|e| e.get("node"))
        .filter(|n| n.get("login").and_then(|l| l.as_str()).is_some_and(|l| !l.is_empty()))
        .cloned()
        .collect();
    let more = list.pointer("/pageInfo/hasNextPage").and_then(|m| m.as_bool()).unwrap_or(false);
    let next = edges
        .last()
        .and_then(|e| e.get("cursor"))
        .and_then(|c| c.as_str())
        .filter(|c| more && !c.is_empty())
        .map(str::to_string);
    Some((members, next))
}

#[cfg(test)]
mod role_list_tests {
    use super::{role_page, role_page_request, ChannelRole};
    use serde_json::json;

    #[test]
    fn the_request_pages_the_field_it_names() {
        let req = role_page_request("chan", ChannelRole::Moderators, Some("c1"));
        assert_eq!(req["variables"], json!({ "login": "chan", "after": "c1" }));
        let query = req["query"].as_str().unwrap();
        assert!(query.contains("mods(first: 100, after: $after)"), "{query}");
        assert!(role_page_request("chan", ChannelRole::Vips, None)["query"].as_str().unwrap().contains("vips(first: 100"));
    }

    #[test]
    fn an_anonymous_mods_read_is_a_refusal_not_an_empty_list() {
        // What Twitch answers an anonymous read of `mods`.
        let refused = json!({
            "errors": [{ "message": "unauthenticated", "path": ["user", "mods"], "extensions": { "code": "Unauthenticated" } }],
            "data": { "user": { "mods": null } },
        });
        assert_eq!(role_page(&refused, ChannelRole::Moderators), None);
        // Errors with no data at all are a refusal too.
        assert_eq!(role_page(&json!({ "errors": [{ "message": "failed integrity check" }] }), ChannelRole::Vips), None);
    }

    #[test]
    fn a_page_keeps_real_members_and_its_next_cursor() {
        let body = json!({ "data": { "user": { "vips": {
            "edges": [
                { "cursor": "a", "node": { "id": "1", "login": "first", "displayName": "First" } },
                { "cursor": "b", "node": null },
                { "cursor": "c", "node": { "id": "3", "login": "third", "displayName": "Third" } },
            ],
            "pageInfo": { "hasNextPage": true },
        } } } });
        let (members, next) = role_page(&body, ChannelRole::Vips).unwrap();
        let logins: Vec<_> = members.iter().map(|m| m["login"].as_str().unwrap()).collect();
        assert_eq!(logins, ["first", "third"]);
        assert_eq!(next.as_deref(), Some("c"));
    }

    #[test]
    fn the_last_page_has_no_cursor() {
        // Twitch sends empty cursors when everything fit on one page.
        let body = json!({ "data": { "user": { "mods": {
            "edges": [{ "cursor": "", "node": { "id": "1", "login": "only", "displayName": "Only" } }],
            "pageInfo": { "hasNextPage": false },
        } } } });
        assert_eq!(role_page(&body, ChannelRole::Moderators).unwrap().1, None);
        let no_channel = json!({ "data": { "user": null } });
        assert_eq!(role_page(&no_channel, ChannelRole::Moderators), Some((Vec::new(), None)));
    }
}

#[cfg(test)]
mod refresh_failure_tests {
    use super::{refresh_failure_kind, refresh_failure_message, refusal_is_transient, RefreshFailure, RefreshRefused};
    use std::time::Duration;

    /// The words the pages read a failure as the network by: `isNetworkAuthError`
    /// (src/utils/authErrors.ts) and the iPhone's `SignIn.isNetwork`.
    const NETWORK_WORDS: [&str; 10] = [
        "error sending request",
        "timeout",
        "network",
        "dns error",
        "proxy",
        "failed to fetch",
        "500",
        "502",
        "503",
        "504",
    ];

    fn reads_as_network(text: &str) -> bool {
        let text = text.to_lowercase();
        NETWORK_WORDS.iter().any(|word| text.contains(word))
    }

    fn refused(status: u16, body: &str) -> anyhow::Error {
        anyhow::anyhow!(RefreshRefused { status, body: body.to_string() })
    }

    #[test]
    fn twitch_turning_the_refresh_token_down_ends_the_sign_in() {
        let e = refused(400, r#"{"status":400,"message":"Invalid refresh token"}"#);
        assert_eq!(refresh_failure_kind(&e), RefreshFailure::Refused);
        let message = refresh_failure_message(RefreshFailure::Refused, &format!("{e:#}"));
        assert!(!reads_as_network(&message), "a refusal must read as a sign-out: {message}");
        assert!(message.contains("Please log in again"), "{message}");
        assert_eq!(refresh_failure_kind(&refused(401, "")), RefreshFailure::Refused);
    }

    #[test]
    fn a_busy_twitch_keeps_the_sign_in() {
        assert!(refusal_is_transient(503));
        assert!(refusal_is_transient(429));
        assert!(!refusal_is_transient(400));
        assert_eq!(refresh_failure_kind(&refused(503, "")), RefreshFailure::Unreachable);
        assert_eq!(refresh_failure_kind(&refused(429, "")), RefreshFailure::Unreachable);
    }

    #[tokio::test]
    async fn a_refresh_that_cannot_reach_twitch_keeps_the_sign_in() {
        // Nothing listens on the local discard port, so the connection is refused at once.
        let err = reqwest::Client::new()
            .post("http://127.0.0.1:9/oauth2/token")
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .expect_err("nothing listens on port 9");
        let e = anyhow::Error::new(err).context("refreshing the Twitch token");
        assert_eq!(refresh_failure_kind(&e), RefreshFailure::Unreachable);
        let message = refresh_failure_message(RefreshFailure::Unreachable, &format!("{e:#}"));
        assert!(reads_as_network(&message), "a network failure must keep the session: {message}");
        assert!(!message.contains("log in again"), "{message}");
    }

    #[test]
    fn a_failure_of_unknown_kind_still_signs_out() {
        assert_eq!(refresh_failure_kind(&anyhow::anyhow!("token response was not JSON")), RefreshFailure::Refused);
    }
}

#[cfg(test)]
mod blocked_term_tests {
    use super::{
        blocked_term_expiry_words, blocked_term_id_for, blocked_term_rows, blocked_terms_page,
        blocked_terms_refusal, RawBlockedTerm,
    };

    const DAY_MS: i64 = 86_400_000;

    fn raw(id: &str, text: &str, created_at_ms: i64, expires_at_ms: Option<i64>) -> RawBlockedTerm {
        RawBlockedTerm { id: id.into(), text: text.into(), created_at_ms, expires_at_ms }
    }

    #[test]
    fn a_page_reads_its_terms_and_cursor() {
        let page = serde_json::json!({
            "data": [
                { "id": "a1", "text": "bad", "moderator_id": "9", "created_at": "2026-10-01T12:00:00Z",
                  "updated_at": "2026-10-01T12:00:00Z", "expires_at": null },
                { "id": "a2", "text": "worse*", "created_at": "2026-10-02T12:00:00Z",
                  "expires_at": "2026-10-05T12:00:00Z" },
                { "id": "", "text": "no id" },
                { "id": "a3" }
            ],
            "pagination": { "cursor": "eyJiIjpudWxsfQ==" }
        });
        let (terms, cursor) = blocked_terms_page(&page);
        assert_eq!(cursor.as_deref(), Some("eyJiIjpudWxsfQ=="));
        assert_eq!(terms.len(), 2, "rows without an id or text are skipped");
        assert_eq!(terms[0].id, "a1");
        assert_eq!(terms[0].expires_at_ms, None);
        assert_eq!(terms[0].created_at_ms, 1_790_856_000_000);
        assert_eq!(terms[1].expires_at_ms, Some(1_790_856_000_000 + 4 * DAY_MS));
    }

    #[test]
    fn the_last_page_has_no_cursor() {
        let (terms, cursor) = blocked_terms_page(&serde_json::json!({ "data": [], "pagination": {} }));
        assert!(terms.is_empty());
        assert_eq!(cursor, None);
        let (_, cursor) = blocked_terms_page(&serde_json::json!({ "data": [], "pagination": { "cursor": "" } }));
        assert_eq!(cursor, None);
        let (terms, cursor) = blocked_terms_page(&serde_json::Value::Null);
        assert!(terms.is_empty() && cursor.is_none());
    }

    #[test]
    fn rows_are_newest_first_with_each_id_once() {
        let now = 10 * DAY_MS;
        let rows = blocked_term_rows(
            vec![
                raw("old", "old", 2 * DAY_MS, None),
                raw("new", "new", 9 * DAY_MS, Some(now + 3 * DAY_MS)),
                raw("new", "new", 9 * DAY_MS, Some(now + 3 * DAY_MS)),
                raw("b", "Beta", 5 * DAY_MS, None),
                raw("a", "alpha", 5 * DAY_MS, None),
            ],
            now,
        );
        let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["new", "a", "b", "old"]);
        assert_eq!(rows[0].added, "Added 1 day ago");
        assert_eq!(rows[0].expires.as_deref(), Some("Expires in 3d"));
        assert_eq!(rows[3].added, "Added 1 week ago");
        assert_eq!(rows[3].expires, None);
    }

    #[test]
    fn a_term_without_a_time_says_nothing_about_when() {
        let rows = blocked_term_rows(vec![raw("x", "x", 0, None)], DAY_MS);
        assert_eq!(rows[0].added, "");
    }

    #[test]
    fn expiry_reads_in_whole_units() {
        let now = 1_000_000_000;
        let at = |secs: i64| blocked_term_expiry_words(now + secs * 1000, now);
        assert_eq!(at(-5), "Expiring now");
        assert_eq!(at(0), "Expiring now");
        assert_eq!(at(20), "Expires in 1m");
        assert_eq!(at(12 * 60), "Expires in 12m");
        assert_eq!(at(59 * 60 + 30), "Expires in 1h");
        assert_eq!(at(3_600), "Expires in 1h");
        assert_eq!(at(5 * 3_600 + 1_800), "Expires in 5h");
        assert_eq!(at(86_400), "Expires in 1d");
        assert_eq!(at(3 * 86_400 + 7_200), "Expires in 3d");
    }

    #[test]
    fn a_phrase_is_found_on_any_page_ignoring_case() {
        // 150 terms, as two pages would list them: the match sits past the first 100.
        let terms: Vec<RawBlockedTerm> = (0..150)
            .map(|i| raw(&format!("id{i}"), &format!("term {i}"), i, None))
            .collect();
        assert_eq!(blocked_term_id_for(&terms, "  TERM 142 ").as_deref(), Some("id142"));
        assert_eq!(blocked_term_id_for(&terms, "term 999"), None);
    }

    #[test]
    fn refusals_start_with_a_capital() {
        let body = serde_json::json!({
            "message": "The user in moderator_id is not one of the broadcaster's moderators."
        })
        .to_string();
        assert_eq!(blocked_terms_refusal(403, &body), "You are not a moderator in this channel");
        assert_eq!(blocked_terms_refusal(502, ""), "Twitch answered HTTP 502");
    }
}
