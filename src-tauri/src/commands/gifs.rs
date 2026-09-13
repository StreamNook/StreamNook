//! Twitch chat GIFs: eligibility, search, and send.
//!
//! Twitch's GIF keyboard is not in any public API. Helix's send-message endpoint
//! takes only `broadcaster_id`, `sender_id` and `message`, and IRC cannot help
//! either because Twitch strips client-supplied tags, so the `gifs` tag we PARSE
//! on the way in (see irc_service) can never be forged on the way out. The web
//! client uses two private GQL operations, read out of its own bundle rather
//! than guessed, because Twitch strips GQL introspection silently.
//!
//! Two things make this Rust's job rather than the page's:
//!
//! 1. `gifPickerConfig` hands the client the **GIPHY API key**. It stays here.
//!    React never sees it, and the search runs here so it cannot leak through a
//!    devtools network pane in a webview that also loads remote origins.
//! 2. Eligibility (`isAllowlisted`, the server-side Tier 2/3 gate) is a fact
//!    about the channel + account, so it is cached per channel and answered
//!    once rather than rediscovered by every window that opens a picker.

use serde::Serialize;
use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

use crate::services::auth_proxy::TWITCH_WEB_CLIENT_ID;
use crate::services::twitch_service::{gql_device_id, TwitchService};

const GQL_URL: &str = "https://gql.twitch.tv/gql";
const GIPHY_BASE: &str = "https://api.giphy.com/v1/gifs";
/// Eligibility changes when a sub starts or lapses, not minute to minute.
const CONFIG_TTL: Duration = Duration::from_secs(600);
const SEARCH_LIMIT: u32 = 30;

/// The full config, GIPHY key included. Never leaves Rust.
#[derive(Clone, Debug, Default)]
struct GifConfig {
    is_enabled: bool,
    is_allowlisted: bool,
    api_key: Option<String>,
    content_rating: Option<String>,
}

impl GifConfig {
    /// Searchable AND sendable: the broadcaster has GIFs on, this account is
    /// allowlisted for the channel, and Twitch gave us a key to search with.
    fn usable(&self) -> bool {
        self.is_enabled && self.is_allowlisted && self.api_key.is_some()
    }
}

/// What the page is allowed to know. Deliberately omits the API key.
#[derive(Serialize, Clone, Debug)]
pub struct GifPickerStatus {
    /// The broadcaster allows GIFs in this channel.
    pub is_enabled: bool,
    /// This account may send them here (Twitch's server-side Tier 2/3 gate).
    pub is_allowlisted: bool,
    /// Both of the above, plus a usable search key: show the tab.
    pub can_use: bool,
    /// GIPHY content rating Twitch wants applied to searches (g/pg/pg-13/r).
    pub content_rating: Option<String>,
}

/// One search result, already reduced to what a grid needs.
#[derive(Serialize, Clone, Debug)]
pub struct GifItem {
    pub id: String,
    pub title: String,
    /// Small looping preview for the grid (~100px wide), not the full asset.
    pub preview_url: String,
    /// The full asset URL. Sent to Twitch verbatim; it must not be modified.
    pub url: String,
    pub width: u32,
    pub height: u32,
}

/// Result of a send attempt. `error` carries Twitch's own enum value.
#[derive(Serialize, Clone, Debug, Default)]
pub struct SendGifOutcome {
    pub sent: bool,
    pub error: Option<String>,
    /// Twitch's cooldown. Honour it; do not retry a bare error.
    pub seconds_until_can_send: u32,
}

type ConfigCache = RwLock<HashMap<String, (Instant, GifConfig)>>;
static CONFIG_CACHE: OnceLock<ConfigCache> = OnceLock::new();

fn config_cache() -> &'static ConfigCache {
    CONFIG_CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Drop cached eligibility. Called on account switch, since `isAllowlisted` is
/// a fact about the SIGNED-IN account and would otherwise carry over.
pub async fn clear_config_cache() {
    config_cache().write().await.clear();
}

/// POST an inline GQL operation with the viewer's token.
async fn gql(token: &str, body: serde_json::Value) -> Result<serde_json::Value, String> {
    let resp = crate::services::http::client()
        .post(GQL_URL)
        .header("Client-ID", TWITCH_WEB_CLIENT_ID)
        .header("Authorization", format!("OAuth {}", token))
        .header("X-Device-ID", gql_device_id())
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("GQL request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("GQL answered {}", resp.status()));
    }
    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("GQL response was not JSON: {e}"))?;
    if let Some(errors) = json.get("errors") {
        // Surfaced rather than swallowed: a schema drift here is exactly the
        // failure mode that made the 7TV trending query silently dead for weeks.
        log::warn!("[GIFs] GQL errors: {}", errors);
    }
    Ok(json)
}

const CONFIG_QUERY: &str = r#"query getGifPickerConfig($channelID: ID!) {
  gifPickerConfig(channelID: $channelID) {
    isEnabled
    isAllowlisted
    apiKey
    contentRating
  }
}"#;

/// Fetch (or serve cached) the GIF picker config for a channel.
async fn load_config(channel_id: &str) -> Result<GifConfig, String> {
    if let Some((at, cfg)) = config_cache().read().await.get(channel_id) {
        if at.elapsed() < CONFIG_TTL {
            return Ok(cfg.clone());
        }
    }

    let token = TwitchService::get_token()
        .await
        .map_err(|_| "not signed in to Twitch".to_string())?;
    let json = gql(
        &token,
        serde_json::json!({
            "operationName": "getGifPickerConfig",
            "query": CONFIG_QUERY,
            "variables": { "channelID": channel_id },
        }),
    )
    .await?;

    let node = json.pointer("/data/gifPickerConfig");
    let cfg = GifConfig {
        is_enabled: node
            .and_then(|n| n.get("isEnabled"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        is_allowlisted: node
            .and_then(|n| n.get("isAllowlisted"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        api_key: node
            .and_then(|n| n.get("apiKey"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        content_rating: node
            .and_then(|n| n.get("contentRating"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string),
    };

    config_cache()
        .write()
        .await
        .insert(channel_id.to_string(), (Instant::now(), cfg.clone()));
    Ok(cfg)
}

/// Can this account use the GIF picker in this channel, and under what rating?
#[tauri::command]
pub async fn get_gif_picker_status(channel_id: String) -> Result<GifPickerStatus, String> {
    let cfg = load_config(&channel_id).await?;
    Ok(GifPickerStatus {
        is_enabled: cfg.is_enabled,
        is_allowlisted: cfg.is_allowlisted,
        can_use: cfg.usable(),
        content_rating: cfg.content_rating.clone(),
    })
}

/// Parse one GIPHY object into the shape the grid renders.
fn parse_gif(item: &serde_json::Value) -> Option<GifItem> {
    let id = item.get("id")?.as_str()?.to_string();
    let images = item.get("images")?;
    // The full asset is what Twitch expects in `gifURL`; the small fixed-width
    // rendition is for the grid so opening the tab does not pull megabytes.
    let url = images
        .pointer("/original/url")
        .and_then(|v| v.as_str())?
        .to_string();
    let preview_url = images
        .pointer("/fixed_width_small/url")
        .or_else(|| images.pointer("/fixed_width/url"))
        .and_then(|v| v.as_str())
        .unwrap_or(&url)
        .to_string();
    let dim = |p: &str| {
        images
            .pointer(p)
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0)
    };
    Some(GifItem {
        id,
        title: item
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        preview_url,
        url,
        width: dim("/fixed_width_small/width"),
        height: dim("/fixed_width_small/height"),
    })
}

/// Search GIPHY with the key Twitch handed us. An empty query returns trending,
/// matching what the web picker shows when it opens.
#[tauri::command]
pub async fn search_gifs(
    channel_id: String,
    query: Option<String>,
    offset: Option<u32>,
) -> Result<Vec<GifItem>, String> {
    let cfg = load_config(&channel_id).await?;
    let Some(api_key) = cfg.api_key.as_deref() else {
        return Err("GIFs are not available in this channel".to_string());
    };

    let q = query.unwrap_or_default();
    let q = q.trim();
    let offset = offset.unwrap_or(0).to_string();
    let limit = SEARCH_LIMIT.to_string();
    let rating = cfg.content_rating.clone().unwrap_or_else(|| "pg-13".into());

    let mut params: Vec<(&str, &str)> = vec![
        ("api_key", api_key),
        ("limit", &limit),
        ("offset", &offset),
        ("rating", &rating),
        ("bundle", "messaging_non_clips"),
    ];
    let url = if q.is_empty() {
        format!("{GIPHY_BASE}/trending")
    } else {
        params.push(("q", q));
        format!("{GIPHY_BASE}/search")
    };

    let resp = crate::services::http::client()
        .get(&url)
        .query(&params)
        .send()
        .await
        .map_err(|e| format!("GIPHY request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("GIPHY answered {}", resp.status()));
    }
    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("GIPHY response was not JSON: {e}"))?;

    Ok(json
        .get("data")
        .and_then(|d| d.as_array())
        .map(|arr| arr.iter().filter_map(parse_gif).collect())
        .unwrap_or_default())
}

const SEND_MUTATION: &str = r#"mutation sendGifMessage($input: SendGifMessageInput!) {
  sendGifMessage(input: $input) {
    error
    secondsUntilCanSend
    message { id }
  }
}"#;

/// Post a GIF to a channel's chat.
///
/// `gif_url` goes out exactly as GIPHY gave it: Twitch requires the full URL
/// unmodified, and the receiving clients render from it. A rejection comes back
/// as Twitch's own enum in `error` plus a cooldown in `seconds_until_can_send`;
/// the caller shows both and does NOT retry on its own.
#[tauri::command]
pub async fn send_gif_message(
    channel_id: String,
    gif_id: String,
    gif_url: String,
    search_term: Option<String>,
) -> Result<SendGifOutcome, String> {
    if channel_id.is_empty() || gif_id.is_empty() || gif_url.is_empty() {
        return Err("missing channel or GIF".to_string());
    }
    let token = TwitchService::get_token()
        .await
        .map_err(|_| "not signed in to Twitch".to_string())?;

    let mut input = serde_json::json!({
        "channelID": channel_id,
        "gifID": gif_id,
        "gifURL": gif_url,
    });
    // Optional in the web client too: omitted rather than sent empty.
    if let Some(term) = search_term.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        input["searchTerm"] = serde_json::Value::String(term.to_string());
    }

    let json = gql(
        &token,
        serde_json::json!({
            "operationName": "sendGifMessage",
            "query": SEND_MUTATION,
            "variables": { "input": input },
        }),
    )
    .await?;

    let node = json.pointer("/data/sendGifMessage");
    let error = node
        .and_then(|n| n.get("error"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let seconds_until_can_send = node
        .and_then(|n| n.get("secondsUntilCanSend"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    let sent = error.is_none()
        && node
            .and_then(|n| n.pointer("/message/id"))
            .and_then(|v| v.as_str())
            .is_some();

    if let Some(code) = &error {
        log::warn!(
            "[GIFs] send rejected for channel {}: {} (retry in {}s)",
            channel_id,
            code,
            seconds_until_can_send
        );
    }

    Ok(SendGifOutcome {
        sent,
        error,
        seconds_until_can_send,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_giphy_object_into_grid_shape() {
        let item = serde_json::json!({
            "id": "joSNxeswxuc74Juo8X",
            "title": "Y A Y Yes GIF by Djemilah Birnie",
            "images": {
                "original": { "url": "https://media4.giphy.com/media/joSNxeswxuc74Juo8X/giphy.gif?cid=abc&ct=g" },
                "fixed_width_small": { "url": "https://media4.giphy.com/media/joSNxeswxuc74Juo8X/100w.gif", "width": "100", "height": "56" }
            }
        });
        let g = parse_gif(&item).expect("parsed");
        assert_eq!(g.id, "joSNxeswxuc74Juo8X");
        // The full asset keeps every query parameter: Twitch requires it verbatim.
        assert_eq!(
            g.url,
            "https://media4.giphy.com/media/joSNxeswxuc74Juo8X/giphy.gif?cid=abc&ct=g"
        );
        assert!(g.preview_url.ends_with("100w.gif"));
        assert_eq!((g.width, g.height), (100, 56));
    }

    #[test]
    fn a_gif_without_an_original_rendition_is_skipped_not_faked() {
        let item = serde_json::json!({ "id": "x", "images": { "fixed_width_small": { "url": "u" } } });
        assert!(parse_gif(&item).is_none());
    }

    #[test]
    fn usable_requires_enabled_allowlisted_and_a_key() {
        let base = GifConfig {
            is_enabled: true,
            is_allowlisted: true,
            api_key: Some("k".into()),
            content_rating: None,
        };
        assert!(base.usable());
        assert!(!GifConfig { is_enabled: false, ..base.clone() }.usable());
        assert!(!GifConfig { is_allowlisted: false, ..base.clone() }.usable());
        assert!(!GifConfig { api_key: None, ..base }.usable());
    }
}
