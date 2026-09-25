//! YouTube live chat adapter.
//!
//! Reads a live stream's chat over YouTube's internal "InnerTube" API, the same
//! private API the website uses, and normalizes each message into the shared
//! `ChatMessage` published onto the local-WS bus. Anonymous: no YouTube login is
//! needed to read public live chat. SENDING and MODERATING need the viewer's
//! identity, which comes from the harvested website session (see
//! `youtube_auth_service`), so `send_capability` reports Sendable once an account
//! is connected and NeedsLogin otherwise.
//!
//! Flow (no browser, plain HTTP — YouTube isn't Cloudflare-gated like Kick):
//!   1. resolve the currently-live video from a handle / channel id / video id by
//!      fetching its watch page and scraping the InnerTube key + client version +
//!      the initial live-chat continuation token,
//!   2. long-poll `youtubei/v1/live_chat/get_live_chat` on the server-provided
//!      `timeoutMs`, decoding `addChatItemAction` renderers into chat messages.
//!
//! A consent cookie (`SOCS=CAI`) is sent so a server-side fetch skips the EU
//! interstitial. If a future host IP ever gets bot-challenged, the documented
//! fallback is the same hidden-webview page-context fetch the Kick adapter uses.

use super::{
    dec_bridge_users, inc_bridge_users, key, publish_chat_message, publish_frame, youtube_emotes,
    ChatProvider, SendCapability, SendOutcome,
};
use crate::models::chat_layout::{
    Badge, ChatMessage, LayoutResult, MessageMetadata, MessageSegment,
};
use crate::models::settings::YouTubeChatView;
use crate::services::youtube_auth_service;
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use base64::Engine;
use futures::FutureExt;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet, VecDeque};
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";
// Fallback poll cadence when the response omits a timeoutMs (it usually carries one).
const DEFAULT_POLL_MS: u64 = 5000;
// After a stream ends / errors, wait this long before re-resolving (catches a
// channel that drops and comes back, or a brief network blip).
const RE_RESOLVE_DELAY_SECS: u64 = 15;
// A poll ERROR used to retry after a flat 3s. That is right for a blip on a busy
// chat, and badly wrong for a stream whose chat is simply unavailable: that case
// errors instantly every time, so each retry is a full watch-page fetch plus up
// to three live_chat fetches every ~4.5 seconds, forever — the fastest recurring
// network loop in the app, aimed at the endpoint most likely to challenge us for
// it. So the error path backs off geometrically instead: 3, 6, 12, 24, 30, 30…
const ERROR_BACKOFF_BASE_SECS: u64 = 3;
// Capped LOW on purpose. The point of backing off is to stop a permanently-dead
// stream hammering YouTube every ~4.5s; it is NOT to make a live chat slow to
// recover. A transient blip must reconnect in seconds, so the ceiling stays close
// to the original flat retry rather than climbing to half a minute.
const ERROR_BACKOFF_MAX_SECS: u64 = 12;
// A round that streamed for at least this long was healthy, whatever ended it, so
// the next failure starts from the bottom of the backoff rather than the top.
// A round that streamed even briefly was working, so the NEXT failure starts from
// the bottom again. This was 60s, which was wrong: a chat that hiccups every
// half-minute would have ratcheted its retry delay upward forever and felt
// broken. Recovery should be cheap and forgetting should be fast.
const HEALTHY_ROUND_SECS: u64 = 10;

/// Next error delay, advancing the streak. Saturates at `ERROR_BACKOFF_MAX_SECS`.
fn error_backoff(streak: &mut u32) -> u64 {
    let delay = ERROR_BACKOFF_BASE_SECS
        .saturating_mul(1u64 << (*streak).min(4))
        .min(ERROR_BACKOFF_MAX_SECS);
    *streak = streak.saturating_add(1);
    delay
}
// The FIRST response after a (re)resolve is a BACKLOG, not live traffic. Measured
// on a busy stream: 76 messages spanning the last ~21 seconds, where a steady-state
// poll carries 5-16 spanning ~2. Pacing that clump like live chat replays 20 seconds
// of a stream at ~10x speed the moment you join, which is what "YouTube chat floods"
// actually was. So the first batch is delivered at once (the way Twitch's history
// backfill arrives) and trimmed to its newest rows.
const BACKLOG_KEEP: usize = 30;
// How many published message ids to remember per connection, so a re-resolve doesn't
// replay its backlog into the pane a second time. Only the recent tail can ever
// overlap a fresh backlog, so this is bounded rather than a full history.
const SEEN_IDS_MAX: usize = 512;
// Ceiling on the per-message gap when pacing a live batch, so a quiet chat still
// shows messages promptly instead of trickling them across the whole poll window.
const MAX_PACE_GAP_MS: u64 = 150;

static FALLBACK_SEQ: AtomicU64 = AtomicU64::new(0);

struct Connection {
    consumers: HashSet<String>,
    // None while the live video is still resolving; Some once the stream task is
    // spawned. Reserving the slot before the (network) resolve stops a concurrent
    // connect for the same channel from spinning a second resolve+stream.
    task: Option<JoinHandle<()>>,
}

pub struct YouTubeProvider {
    // Active stream tasks keyed by the lowercased identifier.
    conns: Mutex<HashMap<String, Connection>>,
    // One client reused for both the watch-page scrape and the InnerTube long-poll:
    // desktop-Chrome UA + a consent cookie so a server-side fetch isn't bounced to
    // the EU "before you continue" interstitial.
    http: reqwest::Client,
}

impl YouTubeProvider {
    pub fn new() -> Self {
        Self {
            conns: Mutex::new(HashMap::new()),
            http: build_client(),
        }
    }
}

fn build_client() -> reqwest::Client {
    let mut headers = reqwest::header::HeaderMap::new();
    if let Ok(v) = reqwest::header::HeaderValue::from_str("SOCS=CAI") {
        headers.insert(reqwest::header::COOKIE, v);
    }
    if let Ok(v) = reqwest::header::HeaderValue::from_str("en-US,en;q=0.9") {
        headers.insert(reqwest::header::ACCEPT_LANGUAGE, v);
    }
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .default_headers(headers)
        // get_live_chat returns immediately (the timeoutMs is a client-side wait, not
        // a server hold), and page fetches are quick — so without these a stuck
        // connection blocks the poll loop forever and the pane sits on "Waiting for
        // messages" until a hard refresh. A bounded timeout turns a hang into an error
        // the loop recovers from by re-resolving.
        .timeout(Duration::from_secs(12))
        .connect_timeout(Duration::from_secs(8))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

#[async_trait]
impl ChatProvider for YouTubeProvider {
    fn id(&self) -> &'static str {
        "youtube"
    }

    async fn connect(&self, channel: &str, window: &str) -> Result<()> {
        // YouTube video + channel ids are case-sensitive, but the composite key the
        // store routes by is lowercased — so a reconnect/restore can hand us the
        // identifier lowercased. Resolve with the original case remembered from the
        // first (add-time) connect; the conn map + bus key stay lowercased so
        // disconnect + the frontend slice key still line up.
        let identifier = canonical_identifier(channel);
        let channel = identifier.as_str();
        let id_lc = channel.to_lowercase();
        {
            let mut conns = self.conns.lock().await;
            if let Some(conn) = conns.get_mut(&id_lc) {
                conn.consumers.insert(window.to_string());
                return Ok(());
            }
            let mut consumers = HashSet::new();
            consumers.insert(window.to_string());
            conns.insert(id_lc.clone(), Connection { consumers, task: None });
        }

        // Count this as a bridge user BEFORE the (network) resolve, for the same
        // reason as Kick: otherwise a concurrent Twitch start_chat could see "no
        // providers" mid-resolve and tear the shared local-WS bridge down.
        inc_bridge_users();

        // Resolve the live video now so a clear error (not live / members-only /
        // chat disabled) surfaces on the pane immediately, the way Kick's resolve
        // does. On success the meta is cached for the chrome and the streaming task
        // takes over (it re-resolves on its own if the stream later ends).
        let resolved = match resolve_live_video(&self.http, channel).await {
            Ok(r) => r,
            Err(e) => {
                dec_bridge_users();
                self.conns.lock().await.remove(&id_lc);
                return Err(e);
            }
        };
        store_meta(&id_lc, resolved.meta.clone());
        // 7TV emotes for this channel, if its community uses them. Fetched at
        // connect (not on first message) so the set is ready before chat starts
        // flowing, and spawned so a slow 7TV can't delay the connection.
        if let Some(uc) = resolved.meta.user_id.clone() {
            let ident = id_lc.clone();
            tokio::spawn(async move {
                youtube_emotes::refresh(&ident, &uc).await;
                // Live 7TV events for this channel (emote-set edits + present
                // users' cosmetics). Skips itself if the channel isn't on 7TV,
                // leaving the TTL refetch above as the only source, as before.
                crate::services::seventv_eventapi::subscribe_channel_on(&ident, &uc, "youtube")
                    .await;
            });
        }

        let task = {
            let http = self.http.clone();
            let identifier = channel.to_string();
            let channel_key = key::make_key("youtube", channel);
            tokio::spawn(async move {
                run_connection(http, identifier, channel_key, resolved.ctx).await
            })
        };
        let mut conns = self.conns.lock().await;
        match conns.get_mut(&id_lc) {
            Some(conn) if conn.task.is_none() => conn.task = Some(task),
            _ => {
                task.abort();
                dec_bridge_users();
            }
        }
        Ok(())
    }

    async fn disconnect(&self, channel: &str, window: &str) -> Result<()> {
        let id_lc = channel.to_lowercase();
        let mut conns = self.conns.lock().await;
        let drop_it = if let Some(conn) = conns.get_mut(&id_lc) {
            conn.consumers.remove(window);
            conn.consumers.is_empty()
        } else {
            false
        };
        if drop_it {
            if let Some(conn) = conns.remove(&id_lc) {
                if let Some(task) = conn.task {
                    task.abort();
                    dec_bridge_users();
                }
            }
        }
        Ok(())
    }

    async fn release_window(&self, window: &str) {
        let mut conns = self.conns.lock().await;
        conns.retain(|id_lc, conn| {
            if !conn.consumers.remove(window) || !conn.consumers.is_empty() {
                return true;
            }
            // Live connection (task Some) released here; a still-resolving one
            // (task None) is released by connect()'s attach/failure path.
            if let Some(task) = conn.task.take() {
                task.abort();
                dec_bridge_users();
                log::debug!("[youtube] released '{}' with window '{}'", id_lc, window);
            }
            false
        });
    }

    async fn send(&self, channel: &str, text: &str, _reply_to: Option<&str>) -> Result<SendOutcome> {
        let drop = |reason: &str| SendOutcome {
            message_id: None,
            is_sent: false,
            drop_reason: Some(reason.to_string()),
        };
        if !youtube_auth_service::is_connected() {
            return Ok(drop("Connect your YouTube account to send"));
        }
        let Some(meta) = channel_meta(channel) else {
            return Ok(drop("YouTube channel isn't resolved yet — try again in a moment"));
        };
        let (Some(channel_id), Some(video_id), Some(api_key)) = (
            meta.user_id.as_deref(),
            meta.video_id.as_deref(),
            meta.api_key.as_deref(),
        ) else {
            return Ok(drop("YouTube channel isn't fully resolved yet"));
        };
        let params = send_message_params(channel_id, video_id);
        let body = json!({ "richMessage": { "textSegments": [{ "text": text }] }, "params": params });
        match post_innertube_authed("send_message", api_key, &meta, body).await {
            Ok(v) => {
                // The send response can say WE were timed out instead of accepting.
                if let Some(us) = v.get("timeoutDurationUsec").and_then(|x| x.as_str()) {
                    let secs = us.parse::<i64>().unwrap_or(0) / 1_000_000;
                    return Ok(drop(&format!("You're in timeout for {}s", secs)));
                }
                // The response carries the FULL rendered message (author, id, runs).
                // Publish it immediately so our own line shows at once instead of
                // waiting for the next read poll (~5s); the read loop's later echo of
                // the same id is deduped by the store's seenMessageIds.
                let item = v.pointer("/actions/0/addChatItemAction/item");
                let message_id = item
                    .and_then(|i| i.pointer("/liveChatTextMessageRenderer/id"))
                    .and_then(|x| x.as_str())
                    .map(String::from);
                if let Some(item) = item {
                    let channel_key = key::make_key("youtube", channel);
                    if let Some(msg) = parse_item(item, &channel_key) {
                        publish_chat_message(&msg).await;
                    }
                }
                Ok(SendOutcome {
                    message_id,
                    is_sent: true,
                    drop_reason: None,
                })
            }
            Err(e) => {
                log::warn!("[YouTube] send failed: {}", e);
                let detail: String = e.to_string().chars().take(160).collect();
                Ok(drop(&format!("YouTube send failed: {}", detail)))
            }
        }
    }

    async fn send_capability(&self, _channel: &str) -> SendCapability {
        if youtube_auth_service::is_connected() {
            SendCapability::Sendable
        } else {
            SendCapability::NeedsLogin
        }
    }
}

// --- Outgoing send / moderation (authenticated InnerTube, webview-session) ---
// All actions POST a protobuf `params` to youtubei/v1 with the user's SAPISIDHASH
// auth (youtube_auth_service). The param builders are ported from masterchat's
// assembler and validated against its fixtures in the tests below.

/// POST an authenticated InnerTube live-chat request (send_message / moderate). Adds
/// the user's session auth headers; merges `extra` (params / richMessage) into the
/// standard `{context:{client}}` body. Errors on non-2xx or an `error` payload.
async fn post_innertube_authed(
    endpoint: &str,
    api_key: &str,
    meta: &YouTubeChannelMeta,
    extra: Value,
) -> Result<Value> {
    let headers = youtube_auth_service::auth_headers()
        .ok_or_else(|| anyhow!("Sign into YouTube to do that"))?;
    let mut client = json!({
        "clientName": "WEB",
        "clientVersion": meta.client_version.as_deref().unwrap_or("2.20240101.00.00"),
        "hl": "en",
        "gl": "US",
    });
    if let Some(vd) = &meta.visitor_data {
        client["visitorData"] = json!(vd);
    }
    let mut body = json!({ "context": { "client": client } });
    if let Some(obj) = extra.as_object() {
        for (k, v) in obj {
            body[k] = v.clone();
        }
    }
    let url = format!(
        "https://www.youtube.com/youtubei/v1/live_chat/{}?key={}&prettyPrint=false",
        endpoint, api_key
    );
    let mut req = reqwest::Client::new()
        .post(url)
        .header("User-Agent", USER_AGENT);
    for (k, v) in headers {
        req = req.header(k, v);
    }
    let resp = req.json(&body).send().await?;
    let status = resp.status();
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        return Err(anyhow!("{} HTTP {}: {}", endpoint, status, v));
    }
    if let Some(err) = v.get("error") {
        return Err(anyhow!("{}: {}", endpoint, err));
    }
    Ok(v)
}

/// Pull the (channel id, video id, api key) a moderation call needs from the cached
/// resolve, or a clear error if the channel isn't resolved.
fn mod_ctx(meta: &YouTubeChannelMeta) -> Result<(String, String, String)> {
    match (
        meta.user_id.clone(),
        meta.video_id.clone(),
        meta.api_key.clone(),
    ) {
        (Some(c), Some(v), Some(k)) => Ok((c, v, k)),
        _ => Err(anyhow!("YouTube channel isn't fully resolved yet")),
    }
}

/// Delete a single chat message (the `message_id` is the live-chat item id).
pub async fn delete_message(channel: &str, message_id: &str) -> Result<()> {
    let meta = channel_meta(channel).ok_or_else(|| anyhow!("YouTube channel not resolved"))?;
    let (channel_id, video_id, api_key) = mod_ctx(&meta)?;
    let params = remove_message_params(message_id, &channel_id, &video_id);
    post_innertube_authed("moderate", &api_key, &meta, json!({ "params": params })).await?;
    Ok(())
}

/// Time out (`duration_seconds` Some — YouTube's fixed ~300s timeout) or permanently
/// hide/ban (`duration_seconds` None) a user, addressed by their channel id.
pub async fn ban_user(
    channel: &str,
    target_channel_id: &str,
    duration_seconds: Option<u32>,
) -> Result<()> {
    let meta = channel_meta(channel).ok_or_else(|| anyhow!("YouTube channel not resolved"))?;
    let (channel_id, video_id, api_key) = mod_ctx(&meta)?;
    let params = if duration_seconds.is_some() {
        timeout_params(target_channel_id, &channel_id, &video_id)
    } else {
        hide_params(target_channel_id, &channel_id, &video_id, false)
    };
    post_innertube_authed("moderate", &api_key, &meta, json!({ "params": params })).await?;
    Ok(())
}

/// Lift a hide/ban on a user (unhide), addressed by their channel id.
pub async fn unban_user(channel: &str, target_channel_id: &str) -> Result<()> {
    let meta = channel_meta(channel).ok_or_else(|| anyhow!("YouTube channel not resolved"))?;
    let (channel_id, video_id, api_key) = mod_ctx(&meta)?;
    let params = hide_params(target_channel_id, &channel_id, &video_id, true);
    post_innertube_authed("moderate", &api_key, &meta, json!({ "params": params })).await?;
    Ok(())
}

// --- Mod-capability probe ----------------------------------------------------
// YouTube exposes no "are you a mod" flag, but a message's context menu offers the
// moderation actions ONLY to someone who can moderate. So we POST a recent message's
// context-menu params and check whether the response carries moderate endpoints.

// Latest seen context-menu params per channel (any recent message works to probe).
static CONTEXT_PARAMS: OnceLock<std::sync::Mutex<HashMap<String, String>>> = OnceLock::new();
// Cached probe result per channel, so the gate doesn't re-probe every render.
static CAN_MOD: OnceLock<std::sync::Mutex<HashMap<String, bool>>> = OnceLock::new();

/// When a moderation probe last FAILED, per channel.
///
/// A failed probe must not be cached as `false` — nothing re-probes a cached
/// answer, so one blip would hide the mod controls for good. But not recording it
/// at all is the opposite trap: the caller polls every 8s, so a session that
/// reliably fails would re-probe forever. This is the middle: retry, but no more
/// than once a minute.
static CAN_MOD_FAILED_AT: OnceLock<std::sync::Mutex<HashMap<String, Instant>>> = OnceLock::new();
const CAN_MOD_RETRY_AFTER: Duration = Duration::from_secs(60);

fn store_context_params(channel_key: &str, params: &str) {
    let slug = key::parse_key(channel_key).channel;
    if let Ok(mut m) = CONTEXT_PARAMS
        .get_or_init(|| std::sync::Mutex::new(HashMap::new()))
        .lock()
    {
        m.insert(slug, params.to_string());
    }
}

/// Whether the connected account can moderate this channel's chat. Cached after the
/// first successful probe; false when signed out or no message has been seen yet.
pub async fn can_moderate(channel: &str) -> bool {
    let slug = channel.to_lowercase();
    if let Some(b) = CAN_MOD
        .get_or_init(|| std::sync::Mutex::new(HashMap::new()))
        .lock()
        .ok()
        .and_then(|m| m.get(&slug).copied())
    {
        return b;
    }
    if !youtube_auth_service::is_connected() {
        return false;
    }
    // Recently failed: don't re-probe on every 8s tick from the composer.
    if let Some(true) = CAN_MOD_FAILED_AT
        .get_or_init(|| std::sync::Mutex::new(HashMap::new()))
        .lock()
        .ok()
        .map(|m| {
            m.get(&slug)
                .is_some_and(|at| at.elapsed() < CAN_MOD_RETRY_AFTER)
        })
    {
        return false;
    }
    let Some(meta) = channel_meta(&slug) else {
        return false;
    };
    let Some(api_key) = meta.api_key.clone() else {
        return false;
    };
    let Some(params) = CONTEXT_PARAMS
        .get_or_init(|| std::sync::Mutex::new(HashMap::new()))
        .lock()
        .ok()
        .and_then(|m| m.get(&slug).cloned())
    else {
        return false; // no message seen yet — can't probe
    };
    let is_mod = match post_innertube_authed(
        "get_item_context_menu",
        &api_key,
        &meta,
        json!({ "params": params }),
    )
    .await
    {
        Ok(v) => menu_grants_moderation(&v),
        // A failed probe says nothing about whether the user is a moderator. It
        // used to be cached as `false`, which meant one network blip hid the mod
        // controls for that channel permanently — nothing re-probes a cached
        // answer. Report false for THIS call only and leave the cache empty so
        // the next poll can settle it.
        Err(e) => {
            log::debug!("[youtube] moderation probe for '{}' failed: {}", slug, e);
            if let Ok(mut m) = CAN_MOD_FAILED_AT
                .get_or_init(|| std::sync::Mutex::new(HashMap::new()))
                .lock()
            {
                m.insert(slug, Instant::now());
            }
            return false;
        }
    };
    if let Ok(mut m) = CAN_MOD
        .get_or_init(|| std::sync::Mutex::new(HashMap::new()))
        .lock()
    {
        m.insert(slug, is_mod);
    }
    is_mod
}

/// Forget cached moderation answers. Called on sign-out: they were computed for
/// the account that just went away, and a different account has different rights.
pub fn clear_moderation_cache() {
    if let Some(m) = CAN_MOD.get() {
        if let Ok(mut m) = m.lock() {
            m.clear();
        }
    }
    // Also drop the failure backoff: a new sign-in deserves an immediate probe.
    if let Some(m) = CAN_MOD_FAILED_AT.get() {
        if let Ok(mut m) = m.lock() {
            m.clear();
        }
    }
}

/// True only if the context-menu response offers a REAL moderation action (Remove /
/// Timeout / Hide) — a menu item with a DIRECT `moderateLiveChatEndpoint`. "Block",
/// which everyone has, wraps its moderate endpoint inside a confirm-dialog, so it
/// doesn't count (matching on it bare was the false-positive that made non-mods look
/// like mods).
fn menu_grants_moderation(v: &Value) -> bool {
    match v {
        Value::Object(map) => {
            if let Some(item) = map.get("menuServiceItemRenderer") {
                if item
                    .pointer("/serviceEndpoint/moderateLiveChatEndpoint")
                    .is_some()
                {
                    return true;
                }
            }
            map.values().any(menu_grants_moderation)
        }
        Value::Array(arr) => arr.iter().any(menu_grants_moderation),
        _ => false,
    }
}

// --- InnerTube param protobufs (ported verbatim from masterchat's assembler.ts) --
// The encoder primitives are validated byte-for-byte against masterchat's fixtures
// in the tests. Every value we emit is < 128, so varints are single-byte.

mod pb {
    use base64::Engine;

    pub fn leb128(mut n: u64) -> Vec<u8> {
        let mut out = Vec::new();
        while n >> 7 != 0 {
            out.push(0x80 | (n & 0x7f) as u8);
            n >>= 7;
        }
        out.push(n as u8);
        out
    }

    fn pbh(fid: u64, wt: u64) -> Vec<u8> {
        leb128((fid << 3) | wt)
    }

    /// Length-delimited field: [header][len][payload].
    pub fn ld(fid: u64, payload: &[u8]) -> Vec<u8> {
        let mut v = pbh(fid, 2);
        v.extend(leb128(payload.len() as u64));
        v.extend_from_slice(payload);
        v
    }

    /// Varint field. masterchat writes the value as minimal big-endian bytes
    /// (`bitou8`), which equals a standard varint for the < 128 values we use.
    pub fn vt(fid: u64, val: u64) -> Vec<u8> {
        let mut v = pbh(fid, 0);
        if val == 0 {
            v.push(0);
        } else {
            let be = val.to_be_bytes();
            let start = be.iter().position(|&b| b != 0).unwrap_or(7);
            v.extend_from_slice(&be[start..]);
        }
        v
    }

    pub fn cat(parts: &[Vec<u8>]) -> Vec<u8> {
        parts.concat()
    }

    /// b64e B1 = encodeURIComponent(standard_base64): only +, /, = need escaping.
    pub fn b1(bytes: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD
            .encode(bytes)
            .replace('+', "%2B")
            .replace('/', "%2F")
            .replace('=', "%3D")
    }

    /// b64e B2 = standard_base64(utf8(b1)).
    pub fn b2(bytes: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(b1(bytes).as_bytes())
    }
}

fn cv_token(channel_id: &str, video_id: &str) -> Vec<u8> {
    pb::ld(
        5,
        &pb::cat(&[
            pb::ld(1, channel_id.as_bytes()),
            pb::ld(2, video_id.as_bytes()),
        ]),
    )
}

fn truc(channel_id: &str) -> &str {
    channel_id.strip_prefix("UC").unwrap_or(channel_id)
}

/// `chatToken` = base64-decode the live-chat item id (url-safe tolerant).
fn chat_token(chat_id: &str) -> Vec<u8> {
    let norm = chat_id.replace('-', "+").replace('_', "/");
    let pad = (4 - norm.len() % 4) % 4;
    let padded = format!("{}{}", norm, "=".repeat(pad));
    base64::engine::general_purpose::STANDARD
        .decode(padded.as_bytes())
        .unwrap_or_default()
}

fn send_message_params(channel_id: &str, video_id: &str) -> String {
    pb::b2(&pb::cat(&[
        pb::ld(1, &cv_token(channel_id, video_id)),
        pb::vt(2, 2),
        pb::vt(3, 4),
    ]))
}

fn remove_message_params(chat_id: &str, channel_id: &str, video_id: &str) -> String {
    pb::b2(&pb::cat(&[
        pb::ld(1, &cv_token(channel_id, video_id)),
        pb::ld(2, &pb::ld(1, &chat_token(chat_id))),
        pb::vt(10, 1),
        pb::vt(11, 1),
    ]))
}

fn timeout_params(target: &str, channel_id: &str, video_id: &str) -> String {
    pb::b2(&pb::cat(&[
        pb::ld(1, &cv_token(channel_id, video_id)),
        pb::ld(6, &pb::ld(1, truc(target).as_bytes())),
        pb::vt(10, 2),
        pb::vt(11, 1),
    ]))
}

fn hide_params(target: &str, channel_id: &str, video_id: &str, undo: bool) -> String {
    let op = if undo { 5 } else { 4 };
    pb::b2(&pb::cat(&[
        pb::ld(1, &cv_token(channel_id, video_id)),
        pb::ld(op, &pb::ld(1, truc(target).as_bytes())),
        pb::vt(10, 2),
        pb::vt(11, 1),
    ]))
}

// --- Channel metadata (chrome: title / channel name / viewers / uptime / avatar) -

/// Live metadata for the MultiChat chrome, scraped from the watch page during
/// resolve. The JSON field names mirror `KickChannelMeta` so the frontend reads
/// both through one `ProviderChannelMeta` shape; `user_id` here is the channel id
/// string (UC…) rather than a number.
#[derive(Clone, Default, serde::Serialize)]
pub struct YouTubeChannelMeta {
    pub user_id: Option<String>,
    pub username: Option<String>,
    pub viewer_count: Option<u64>,
    pub start_time: Option<String>,
    pub title: Option<String>,
    pub profile_pic: Option<String>,
    /// The game/category YouTube attached to the broadcast, when it attached one.
    /// Unlike Twitch (where the streamer picks it and it is always right) this is
    /// inferred by YouTube and is frequently ABSENT even on an obvious gaming
    /// stream, so every consumer has to treat empty as "unknown", not "none".
    pub game_name: Option<String>,
    pub is_live: bool,
    // Internal send/moderate context captured at resolve (the cvPair video id + the
    // scraped InnerTube creds). `serde(skip)` keeps them out of the frontend payload.
    #[serde(skip)]
    pub video_id: Option<String>,
    #[serde(skip)]
    pub api_key: Option<String>,
    #[serde(skip)]
    pub client_version: Option<String>,
    #[serde(skip)]
    pub visitor_data: Option<String>,
}

static YT_META: OnceLock<std::sync::Mutex<HashMap<String, (YouTubeChannelMeta, Instant)>>> =
    OnceLock::new();

fn yt_meta_cache() -> &'static std::sync::Mutex<HashMap<String, (YouTubeChannelMeta, Instant)>> {
    YT_META.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

fn store_meta(id_lc: &str, meta: YouTubeChannelMeta) {
    if let Ok(mut m) = yt_meta_cache().lock() {
        m.insert(id_lc.to_string(), (meta, Instant::now()));
    }
}

/// The cached metadata for an identifier, at any age.
///
/// Correct for the IDENTITY fields (api key, client version, visitor data), which
/// don't go stale in a way that matters. NOT correct for liveness — see
/// `channel_meta_fresh`.
pub fn channel_meta(identifier: &str) -> Option<YouTubeChannelMeta> {
    yt_meta_cache()
        .lock()
        .ok()
        .and_then(|m| m.get(&identifier.to_lowercase()).map(|(meta, _)| meta.clone()))
}

/// The cached metadata only if it is younger than `max_age`.
///
/// This entry carries `is_live` and a viewer count. Served unconditionally from
/// cache, as it used to be, a channel whose chat you once opened is reported LIVE
/// by the who's-live poller for the rest of the session. Callers that care about
/// liveness use this and re-resolve on `None`.
pub fn channel_meta_fresh(identifier: &str, max_age: Duration) -> Option<YouTubeChannelMeta> {
    yt_meta_cache().lock().ok().and_then(|m| {
        m.get(&identifier.to_lowercase())
            .filter(|(_, stored)| stored.elapsed() < max_age)
            .map(|(meta, _)| meta.clone())
    })
}

// Original-case identifiers, keyed by their lowercase. Seeded on the first connect
// (which carries the add-time case) so a later lowercased identifier — from a
// reconnect that re-derives the channel from the lowercased composite key — still
// resolves a case-sensitive video/channel id correctly. (@handles are
// case-insensitive at YouTube, so they're unaffected either way.)
static ORIG_CASE: OnceLock<std::sync::Mutex<HashMap<String, String>>> = OnceLock::new();

fn canonical_identifier(channel: &str) -> String {
    let lc = channel.to_lowercase();
    let Ok(mut map) = ORIG_CASE
        .get_or_init(|| std::sync::Mutex::new(HashMap::new()))
        .lock()
    else {
        return channel.to_string();
    };
    if channel != lc {
        map.entry(lc.clone()).or_insert_with(|| channel.to_string());
    }
    map.get(&lc).cloned().unwrap_or_else(|| channel.to_string())
}

// --- Live-video resolution (scrape the watch page) --------------------------

/// Everything the long-poll needs, scraped once from the watch/live page.
struct LiveContext {
    api_key: String,
    client_version: String,
    visitor_data: Option<String>,
    continuation: String,
}

struct Resolved {
    ctx: LiveContext,
    meta: YouTubeChannelMeta,
}

/// Build the watch/live URL for an identifier. The frontend `@`-prefixes handles
/// so they're never confused with an 11-char video id.
pub(crate) fn live_page_url(identifier: &str) -> String {
    let id = identifier.trim();
    if let Some(handle) = id.strip_prefix('@') {
        return format!("https://www.youtube.com/@{}/live", handle);
    }
    if id.starts_with("UC") && id.len() == 24 {
        return format!("https://www.youtube.com/channel/{}/live", id);
    }
    if is_video_id(id) {
        return format!("https://www.youtube.com/watch?v={}", id);
    }
    // Bare handle (frontend normally adds the @, but be tolerant).
    format!("https://www.youtube.com/@{}/live", id)
}

fn is_video_id(s: &str) -> bool {
    s.len() == 11 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Outcome of parsing a page we hoped was a live watch page.
enum WatchOutcome {
    /// A watch page with live chat: everything the long-poll needs.
    Resolved(Resolved),
    /// Not a watch page. YouTube handed us the channel browse shell instead, so the
    /// caller recovers candidate video ids from it and re-fetches their watch pages.
    BrowseShell,
}

/// GET a YouTube page's HTML, mapping the EU consent interstitial to a clear error.
pub(crate) async fn fetch_youtube_html(
    http: &reqwest::Client,
    url: &str,
    identifier: &str,
) -> Result<String> {
    let resp = http.get(url).send().await?;
    let final_url = resp.url().clone();
    let html = resp.text().await?;
    if final_url.as_str().contains("consent.youtube.com")
        || html.contains("Before you continue to YouTube")
    {
        return Err(anyhow!(
            "YouTube returned a consent page for '{}' (try again)",
            identifier
        ));
    }
    Ok(html)
}

/// Fetch the live page for `identifier`, PARSE it, and cache the metadata.
///
/// Parsing here is the point: the metadata cache is written only by the CHAT
/// connect path, so fetching without parsing answers only for channels whose
/// chat was opened this session. `live_check` is built on `channel_meta`, so
/// that leaves the who's-live poller and the favourites sweep blind to every
/// other YouTube channel, and a MultiNook tile titling itself with a raw
/// video id.
///
/// Accepts any identifier `live_page_url` does: a video id, an `@handle`, or
/// a `UC` id.
pub(crate) async fn refresh_channel_meta(
    http: &reqwest::Client,
    identifier: &str,
) -> Result<YouTubeChannelMeta> {
    let html = fetch_youtube_html(http, &live_page_url(identifier), identifier).await?;
    let player = extract_json(&html, "ytInitialPlayerResponse");
    let initial = extract_json(&html, "ytInitialData");

    // Only a real WATCH page carries `videoDetails`; a channel with no live
    // broadcast serves its browse shell instead. That is a definitive OFFLINE
    // answer, not a failed lookup, and the difference matters: the caller
    // answers an `Err` out of the last known meta, so returning one here kept
    // reporting a channel that had just gone offline as live for the rest of
    // the session. Report failure only when the page is not a YouTube page at
    // all - if `ytInitialData` parsed, we did reach YouTube and it showed us no
    // broadcast. (`fetch_youtube_html` has already turned the consent
    // interstitial and every transport error into an `Err` above.)
    let has_details = player
        .as_ref()
        .and_then(|p| p.get("videoDetails"))
        .is_some();
    if !has_details && initial.is_none() {
        return Err(anyhow!(
            "couldn't read a YouTube page for '{}' (try again)",
            identifier
        ));
    }

    let mut meta = extract_meta(player.as_ref(), initial.as_ref(), &html);

    // The innertube identity fields, read the same way `parse_watch_page` reads
    // them. Without these the refreshed entry would be POORER than a chat-stored
    // one, and overwriting the cache with it would break moderation context for
    // a channel whose chat is open.
    meta.api_key = json_str_after(&html, "\"INNERTUBE_API_KEY\":\"");
    meta.client_version = json_str_after(&html, "\"INNERTUBE_CONTEXT_CLIENT_VERSION\":\"")
        .or_else(|| json_str_after(&html, "\"clientVersion\":\""));
    meta.visitor_data = json_str_after(&html, "\"visitorData\":\"").map(|v| decode_json_escapes(&v));

    // Belt and braces: anything this page did not carry keeps whatever the cache
    // already knew, so a refresh can only ever add detail, never remove it.
    if let Some(prev) = channel_meta(identifier) {
        meta.api_key = meta.api_key.or(prev.api_key);
        meta.client_version = meta.client_version.or(prev.client_version);
        meta.visitor_data = meta.visitor_data.or(prev.visitor_data);
        meta.profile_pic = meta.profile_pic.or(prev.profile_pic);
        meta.user_id = meta.user_id.or(prev.user_id);
        // Identity, not liveness: a browse shell carries no `author`, and
        // dropping it left the UI labelling the row with a raw `UC...` id the
        // moment a channel went offline.
        meta.username = meta.username.or(prev.username);
    }

    store_meta(&identifier.to_lowercase(), meta.clone());
    Ok(meta)
}

/// Parse a fetched page as a live watch page. Returns `Resolved` on success,
/// `BrowseShell` when YouTube served the channel browse page instead of a watch
/// page (so the caller can re-resolve via /watch?v=), or an error for a real watch
/// page whose chat is gated (members-only / age / chat disabled).
async fn parse_watch_page(
    http: &reqwest::Client,
    html: &str,
    identifier: &str,
) -> Result<WatchOutcome> {
    let api_key = json_str_after(html, "\"INNERTUBE_API_KEY\":\"")
        .ok_or_else(|| anyhow!("couldn't read YouTube API key (is the channel valid?)"))?;
    let client_version = json_str_after(html, "\"INNERTUBE_CONTEXT_CLIENT_VERSION\":\"")
        .or_else(|| json_str_after(html, "\"clientVersion\":\""))
        .unwrap_or_else(|| "2.20240101.00.00".to_string());
    let visitor_data = json_str_after(html, "\"visitorData\":\"").map(|v| decode_json_escapes(&v));

    let player = extract_json(html, "ytInitialPlayerResponse");
    let initial = extract_json(html, "ytInitialData");

    // Playability gate: members-only / login-required / age-restricted streams have
    // no anonymous chat. Give a readable reason instead of a blank pane.
    if let Some(status) = player
        .as_ref()
        .and_then(|p| p.pointer("/playabilityStatus/status"))
        .and_then(|s| s.as_str())
    {
        if status != "OK" {
            let reason = player
                .as_ref()
                .and_then(|p| p.pointer("/playabilityStatus/reason"))
                .and_then(|r| r.as_str())
                .unwrap_or(status);
            return Err(anyhow!("YouTube: {}", reason));
        }
    }

    // An ENDED broadcast still carries a `liveChatRenderer`: it is the chat REPLAY.
    // Its continuation is a replay token, which `get_live_chat` rejects with HTTP
    // 400, and the stream loop treats that as a transient error and re-resolves
    // every few seconds forever. So liveness is checked BEFORE the renderer, not
    // after: the presence of a chat renderer says nothing about whether the stream
    // is on air.
    //
    // Symptom this fixes, verified on a broadcast that had just ended:
    //   "couldn't get the Live chat continuation ...; using Top chat (filtered)"
    //   "stream error: get_live_chat HTTP 400 Bad Request"   (repeating)
    // The first line is the same signal read a different way: the `live_chat` page
    // has no live renderer once a stream ends, which is why the fallback engaged.
    let is_watch = initial
        .as_ref()
        .and_then(|d| d.pointer("/contents/twoColumnWatchNextResults"))
        .is_some();
    if is_watch && !is_currently_live(html) {
        return Err(anyhow!(
            "'{}' isn't live right now (or its chat is unavailable)",
            identifier
        ));
    }

    let live_chat_renderer = initial.as_ref().and_then(|d| {
        d.pointer("/contents/twoColumnWatchNextResults/conversationBar/liveChatRenderer")
    });
    let Some(lcr) = live_chat_renderer else {
        // No live-chat renderer. If this is a real watch page, its chat is genuinely
        // unavailable (disabled / gated / it's a VOD), a hard error. If it's the
        // channel browse shell (YouTube's /live shortcut sometimes fails to surface a
        // live stream), hand back the recovered video id so the caller re-fetches its
        // watch page, which always carries the chat token.
        let is_watch_page = initial
            .as_ref()
            .and_then(|d| d.pointer("/contents/twoColumnWatchNextResults"))
            .is_some();
        if is_watch_page {
            // YouTube replaces the renderer with a stated reason ("Chat is disabled for
            // this live stream.", members-only, and so on). Verified live: a 24/7 stream
            // that PLAYS fine can still have chat switched off, so the generic
            // "isn't live right now" was blaming the wrong thing and reading as a bug.
            if let Some(reason) = conversation_bar_message(initial.as_ref()) {
                return Err(anyhow!("YouTube: {}", reason));
            }
            return Err(anyhow!(
                "'{}' isn't live right now (or its chat is unavailable)",
                identifier
            ));
        }
        return Ok(WatchOutcome::BrowseShell);
    };
    // The watch page's BOOTSTRAP continuation (continuations[0]) is the one
    // get_live_chat accepts — the watch page's view-selector tokens 400 against the
    // API. That bootstrap is the default "Top chat" view, which filters messages;
    // for the full "Live chat" firehose, fetch the dedicated live_chat page, whose
    // view-selector "Live chat" token IS get_live_chat-compatible. Fall back to the
    // bootstrap (Top chat) if that lookup fails.
    let bootstrap = bootstrap_continuation(lcr)
        .ok_or_else(|| anyhow!("couldn't read the YouTube live-chat continuation"))?;
    // Video id: prefer the player response, fall back to a direct scrape so a failed
    // player parse doesn't silently strand us on the filtered Top-chat view.
    let video_id = player
        .as_ref()
        .and_then(|p| p.pointer("/videoDetails/videoId"))
        .and_then(|v| v.as_str())
        .map(String::from)
        .or_else(|| json_str_after(html, "\"videoId\":\""));
    let continuation = if chat_view() == YouTubeChatView::Top {
        // Top chat IS the bootstrap token, so asking for it costs nothing: no
        // live_chat page fetch, no retries.
        log::info!("[YouTube] '{}' on Top chat (filtered, by setting)", identifier);
        // ...but the emoji set lives on that page, so skipping it also skipped the
        // emoji. The picker must not silently depend on a chat-view preference, so
        // fetch it here purely for the emoji when the watch page didn't carry them.
        if let Some(vid) = video_id.as_deref() {
            if channel_emojis(vid).is_empty() {
                let _ = fetch_live_continuation(http, vid, identifier).await;
            }
        }
        bootstrap
    } else {
        match video_id.as_deref() {
            Some(vid) => match fetch_live_continuation(http, vid, identifier).await {
                Some(live) => {
                    log::info!("[YouTube] '{}' on the FULL Live chat view", identifier);
                    live
                }
                None => {
                    log::warn!(
                        "[YouTube] '{}' couldn't get the Live chat continuation (video {}); using Top chat (filtered)",
                        identifier, vid
                    );
                    bootstrap
                }
            },
            None => {
                log::warn!("[YouTube] '{}' has no video id; using Top chat bootstrap", identifier);
                bootstrap
            }
        }
    };

    let mut meta = extract_meta(player.as_ref(), initial.as_ref(), html);
    // Stash the scraped InnerTube creds on the meta so send/moderate can reuse them
    // (kept internal via serde(skip)).
    meta.api_key = Some(api_key.clone());
    meta.client_version = Some(client_version.clone());
    meta.visitor_data = visitor_data.clone();

    Ok(WatchOutcome::Resolved(Resolved {
        ctx: LiveContext {
            api_key,
            client_version,
            visitor_data,
            continuation,
        },
        meta,
    }))
}

/// The user's chosen live-chat view. Read per resolve (not cached) so changing it
/// takes effect on the next channel you open rather than at the next app start;
/// falls back to the unfiltered firehose before the settings handle is wired.
fn chat_view() -> YouTubeChatView {
    super::setting(|s| s.youtube_chat_view).unwrap_or_default()
}

/// The conversation bar's availability message, present when YouTube swapped the
/// live-chat renderer for an explanation of why there's no chat.
fn conversation_bar_message(initial: Option<&Value>) -> Option<String> {
    let text = initial?.pointer(
        "/contents/twoColumnWatchNextResults/conversationBar/conversationBarRenderer/availabilityMessage/messageRenderer/text",
    )?;
    runs_text(Some(text)).filter(|s| !s.trim().is_empty())
}

/// Distinct 11-char video ids in document order, capped at `max`. Used to pick the
/// live video off a channel browse shell, which carries no reliable "this one is
/// live" marker: the live stream is normally surfaced at the top, but a channel
/// trailer or pinned upload can precede it, so the caller probes the first few.
fn candidate_video_ids(html: &str, max: usize) -> Vec<String> {
    let marker = "\"videoId\":\"";
    let mut out: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut rest = html;
    while out.len() < max {
        let Some(pos) = rest.find(marker) else { break };
        let start = &rest[pos + marker.len()..];
        let Some(end) = start.find('"') else { break };
        let id = &start[..end];
        if is_video_id(id) && seen.insert(id.to_string()) {
            out.push(id.to_string());
        }
        rest = &start[end..];
    }
    out
}

/// Whether a scraped watch page is a stream that is live right now (not an ended
/// broadcast's VOD, whose replay chat we must not attach to). Used to gate the
/// /watch?v= fallback so a browse shell's non-live grid item is rejected.
fn is_currently_live(html: &str) -> bool {
    extract_json(html, "ytInitialPlayerResponse")
        .as_ref()
        .map(|p| {
            p.pointer("/videoDetails/isLive")
                .and_then(|b| b.as_bool())
                .unwrap_or(false)
                || p.pointer(
                    "/microformat/playerMicroformatRenderer/liveBroadcastDetails/isLiveNow",
                )
                .and_then(|b| b.as_bool())
                .unwrap_or(false)
        })
        .unwrap_or(false)
}

/// Fetch the live page and extract the InnerTube config + the initial live-chat
/// continuation + the channel chrome. Errors (surfaced on the pane) distinguish
/// not-live / members-only / chat-disabled.
///
/// YouTube's `/@handle/live` (and `/channel/UC…/live`) shortcut normally serves the
/// live watch page, but it intermittently fails to surface a genuinely-live stream
/// and returns the channel's browse page instead (which carries no chat token). When
/// that happens we recover candidate video ids from the browse page and re-fetch their
/// watch pages directly, accepting the first one that is live right now.
async fn resolve_live_video(http: &reqwest::Client, identifier: &str) -> Result<Resolved> {
    let html = fetch_youtube_html(http, &live_page_url(identifier), identifier).await?;
    match parse_watch_page(http, &html, identifier).await? {
        WatchOutcome::Resolved(resolved) => return Ok(resolved),
        WatchOutcome::BrowseShell => {}
    }

    // /live handed us the channel browse shell instead of the live watch page. The
    // browse page has no reliable "this video is live" marker, so probe the first few
    // distinct video ids (the live stream is normally at the top, but a trailer/pinned
    // upload can precede it). Accept the first that is live now — its watch page always
    // carries the chat token; the is-live gate stops us attaching to a past stream's
    // replay chat when a non-live grid item comes first.
    let not_live = || {
        anyhow!(
            "'{}' isn't live right now (or its chat is unavailable)",
            identifier
        )
    };
    let candidates = candidate_video_ids(&html, 3);
    if candidates.is_empty() {
        return Err(not_live());
    }
    for vid in candidates {
        let watch_url = format!("https://www.youtube.com/watch?v={}", vid);
        let Ok(watch_html) = fetch_youtube_html(http, &watch_url, identifier).await else {
            continue;
        };
        if !is_currently_live(&watch_html) {
            continue;
        }
        match parse_watch_page(http, &watch_html, identifier).await {
            Ok(WatchOutcome::Resolved(resolved)) => {
                log::info!(
                    "[YouTube] '{}' resolved via /watch?v={} after /live returned a browse shell",
                    identifier,
                    vid
                );
                return Ok(resolved);
            }
            // A live candidate whose watch page still won't parse (e.g. members-only
            // playability): surface that reason rather than a misleading "not live".
            Ok(WatchOutcome::BrowseShell) => continue,
            Err(e) => return Err(e),
        }
    }
    Err(not_live())
}

/// The watch page's bootstrap live-chat continuation — the token get_live_chat
/// accepts (the watch page's view-selector tokens are for the chat iframe and 400
/// against the API). This is the default "Top chat" view; `fetch_live_continuation`
/// upgrades it to the full Live chat.
fn bootstrap_continuation(lcr: &Value) -> Option<String> {
    lcr.pointer("/continuations/0/reloadContinuationData/continuation")
        .or_else(|| lcr.pointer("/continuations/0/timedContinuationData/continuation"))
        .and_then(|c| c.as_str())
        .map(String::from)
}

/// Fetch the dedicated live_chat page and pull the "Live chat" view continuation,
/// which (unlike the watch page's) IS get_live_chat-compatible. None if the page or
/// the Live view isn't available (the caller falls back to the bootstrap/Top view).
/// One emoji offered in a channel's live chat.
#[derive(Debug, Clone, serde::Serialize)]
pub struct YouTubeEmoji {
    pub id: String,
    /// The `:shortcut:` a user types, else the emoji id.
    pub name: String,
    pub url: String,
    /// False for the channel's own custom emoji — the ones worth showing first.
    pub is_global: bool,
    /// Visible but not composable by this account (members-only emoji). See the
    /// note in `store_channel_emojis`: this is read defensively and stays false
    /// when YouTube ships no such flag.
    pub locked: bool,
}

/// Emoji sets, reachable by EITHER the video id or the channel identifier the
/// caller happens to hold, in any casing.
///
/// This matters more than it looks. The set is discovered while resolving a
/// VIDEO, but the frontend only ever knows the identifier it connected chat with
/// (`@handle`, a `UC…` id, or a video id) — and `MainProviderChat` lowercases it
/// on the way in. Keying by video id alone meant the lookup missed on two counts
/// at once: wrong value AND wrong case. Every key is stored lowercased and every
/// lookup lowercases, so any of those spellings finds the same set.
static EMOJIS: OnceLock<std::sync::Mutex<HashMap<String, std::sync::Arc<Vec<YouTubeEmoji>>>>> =
    OnceLock::new();
/// Distinct SETS to keep. Aliases are cheap (they share one Arc), so this bounds
/// the emoji data rather than the key count.
const MAX_EMOJI_CHANNELS: usize = 12;

/// Pull `liveChatRenderer.emojis` out of a live_chat page's `ytInitialData`.
///
/// YouTube publishes no emote-set endpoint, which is why this looked
/// un-enumerable — but the live_chat page ships the whole list inline, because
/// its own emoji picker has to render it. Each entry carries the image, the
/// `:shortcut:` and whether it is a global (unicode) emoji or one of the
/// channel's custom ones.
fn store_channel_emojis(video_id: &str, identifier: &str, init: &Value) {
    let Some(list) = find_emoji_array(init) else {
        // Reaching here means the page WAS parsed but carried no emoji array under
        // any name we recognise. Report the shape so this is one log line to
        // diagnose instead of a silent empty picker.
        log::warn!(
            "[YouTube] no emoji array on the live_chat page for '{}' (identifier '{}'); \
             emoji-ish keys anywhere in the payload: {:?}",
            video_id,
            identifier,
            emoji_shaped_keys(init),
        );
        return;
    };
    let mut out: Vec<YouTubeEmoji> = Vec::new();
    // Borrowed, not consumed: the diagnostic below re-reads the raw entries.
    for e in &list {
        let id = e.get("emojiId").and_then(|v| v.as_str()).unwrap_or("");
        if id.is_empty() {
            continue;
        }
        let url = e
            .pointer("/image/thumbnails")
            .and_then(|t| t.as_array())
            .and_then(|a| a.last())
            .and_then(|t| t.get("url"))
            .and_then(|u| u.as_str())
            .unwrap_or("");
        if url.is_empty() {
            continue;
        }
        let name = e
            .pointer("/shortcuts/0")
            .and_then(|s| s.as_str())
            .unwrap_or(id)
            .to_string();
        // Custom channel emoji carry `isCustomEmoji: true`; unicode ones don't.
        let is_global = !e
            .get("isCustomEmoji")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        // Members-only emoji: visible in the picker, not composable unless you
        // hold the membership. Read defensively across the spellings YouTube has
        // used rather than betting on one, because the alternative to guessing
        // wrong here is a picker that lies about what you can send.
        //
        // UNVERIFIED against a members-only channel as a non-member. If none of
        // these keys is ever present, the set is simply entitlement-filtered
        // upstream (a signed-out viewer already receives no array at all) and
        // this stays false, which is honest either way. The debug line below
        // reports the real key names so this can be settled without a code edit.
        let locked = ["isLocked", "locked", "isMembersOnly"]
            .iter()
            .any(|k| e.get(*k).and_then(|v| v.as_bool()).unwrap_or(false));
        out.push(YouTubeEmoji {
            id: id.to_string(),
            name,
            url: url.to_string(),
            is_global,
            locked,
        });
    }
    if out.is_empty() {
        return;
    }
    // Every field name YouTube actually used on a custom entry, so the
    // members-only question above can be settled from a log instead of a guess.
    // Debug level: invisible at the default filter, one line under RUST_LOG=debug.
    if log::log_enabled!(log::Level::Debug) {
        let keys: std::collections::BTreeSet<&str> = list
            .iter()
            .filter(|e| {
                e.get("isCustomEmoji")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
            })
            .filter_map(|e| e.as_object())
            .flat_map(|o| o.keys().map(|k| k.as_str()))
            .collect();
        log::debug!("[YouTube] custom-emoji entry keys for '{}': {:?}", video_id, keys);
    }
    let custom = out.iter().filter(|e| !e.is_global).count();
    log::info!(
        "[YouTube] {} emoji for '{}' ({} custom to this channel)",
        out.len(),
        video_id,
        custom
    );
    let shared = std::sync::Arc::new(out);
    // Every spelling the frontend might ask with, all lowercased. `identifier` is
    // what chat connected with; `video_id` is what this resolve produced. They are
    // usually different strings, and the frontend only has the former.
    let keys: Vec<String> = [video_id, identifier]
        .iter()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_lowercase())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();

    if let Ok(mut m) = EMOJIS
        .get_or_init(|| std::sync::Mutex::new(HashMap::new()))
        .lock()
    {
        // Count DISTINCT sets, not keys: aliases share one Arc, so a two-key entry
        // must not count double against the cap.
        let distinct = m
            .values()
            .map(|v| std::sync::Arc::as_ptr(v) as usize)
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        if distinct >= MAX_EMOJI_CHANNELS && !keys.iter().any(|k| m.contains_key(k)) {
            if let Some(victim) = m.values().next().cloned() {
                let ptr = std::sync::Arc::as_ptr(&victim) as usize;
                m.retain(|_, v| std::sync::Arc::as_ptr(v) as usize != ptr);
            }
        }
        for k in keys {
            m.insert(k, std::sync::Arc::clone(&shared));
        }
    }
}

/// Locate the emoji array wherever YouTube currently nests it. Searched rather
/// than pointer-addressed because the path has moved between layouts and a fixed
/// pointer would silently return nothing after the next reshuffle.
fn find_emoji_array(root: &Value) -> Option<Vec<Value>> {
    // Matched on SHAPE, not on the key name.
    //
    // The first version looked for a key literally called `emojis`, which is the
    // same brittleness as a hardcoded pointer wearing a different hat: measured
    // against a live payload it found nothing, because YouTube nests this under
    // more than one name across surfaces. An array whose entries carry `emojiId`
    // plus an image IS the emoji list regardless of what the field is called, so
    // that is what this looks for. Largest array wins — a category shelf can hold
    // a handful while the real set holds hundreds.
    fn is_emoji_entry(v: &Value) -> bool {
        v.get("emojiId").is_some() && (v.get("image").is_some() || v.get("shortcuts").is_some())
    }

    let mut best: Option<Vec<Value>> = None;
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        match node {
            Value::Object(map) => {
                for v in map.values() {
                    if let Some(arr) = v.as_array() {
                        if arr.len() > best.as_ref().map_or(0, |b| b.len())
                            && arr.iter().any(is_emoji_entry)
                        {
                            best = Some(arr.clone());
                        }
                    }
                    stack.push(v);
                }
            }
            Value::Array(items) => stack.extend(items.iter()),
            _ => {}
        }
    }
    best
}

/// Field names anywhere in a payload that mention emoji, with what they hold.
///
/// If the shape match above ever misses, the next question is always "what is the
/// field actually called this week" — so answer it in the same log line instead of
/// costing another round trip.
fn emoji_shaped_keys(root: &Value) -> Vec<String> {
    let mut found: std::collections::BTreeSet<String> = Default::default();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        match node {
            Value::Object(map) => {
                for (k, v) in map {
                    if k.to_lowercase().contains("emoji") {
                        let what = match v {
                            Value::Array(a) => format!("array[{}]", a.len()),
                            Value::Object(o) => format!("object{{{}}}", o.len()),
                            Value::String(_) => "string".to_string(),
                            _ => "scalar".to_string(),
                        };
                        found.insert(format!("{}={}", k, what));
                    }
                    stack.push(v);
                }
            }
            Value::Array(items) => stack.extend(items.iter()),
            _ => {}
        }
    }
    found.into_iter().take(20).collect()
}

/// The emoji set for a channel's live chat, custom ones first.
pub fn channel_emojis(channel: &str) -> Vec<YouTubeEmoji> {
    // Lowercased: the caller may hold a handle, a UC id or a video id, and
    // `MainProviderChat` lowercases whatever it has before it gets here.
    let key = channel.trim().to_lowercase();
    let mut list = EMOJIS
        .get()
        .and_then(|m| m.lock().ok())
        .and_then(|m| m.get(&key).map(|v| v.as_ref().clone()))
        .unwrap_or_default();
    // The channel's own emoji are the reason to open this tab; unicode ones are
    // available from the emoji tab anyway.
    list.sort_by_key(|e| e.is_global);
    list
}

/// Forget a channel's emoji when its chat is gone.
pub fn clear_channel_emojis(channel: &str) {
    let key = channel.trim().to_lowercase();
    if let Some(m) = EMOJIS.get() {
        if let Ok(mut m) = m.lock() {
            // Drop every alias of this set, not just the key we were handed.
            if let Some(victim) = m.get(&key).cloned() {
                let ptr = std::sync::Arc::as_ptr(&victim) as usize;
                m.retain(|_, v| std::sync::Arc::as_ptr(v) as usize != ptr);
            }
        }
    }
}

async fn fetch_live_continuation(
    http: &reqwest::Client,
    video_id: &str,
    identifier: &str,
) -> Option<String> {
    let url = format!("https://www.youtube.com/live_chat?v={}", video_id);
    // Retry: the live_chat page is the only reliable source of the full-Live-view
    // continuation (the watch page + the get_live_chat response only carry the
    // filtered Top view), so it's worth a few tries. Logs WHERE it fails so a Top-chat
    // fallback is diagnosable.
    for attempt in 0..3 {
        // Signed in where possible. The continuation comes back either way, but the
        // channel's EMOJI SET does not: the picker is a chat-authoring feature, and
        // a signed-out viewer cannot type, so YouTube has no reason to ship it. A
        // live payload confirmed that — singular per-message `emoji` objects and the
        // fountain entity, and no array at all. We already hold the session, so use
        // it and let the page decide what we are entitled to see.
        let mut req = http.get(&url);
        if let Some(headers) = crate::services::youtube_auth_service::auth_headers() {
            for (k, v) in headers {
                req = req.header(k, v);
            }
        }
        match req.send().await {
            Ok(resp) => {
                let final_url = resp.url().as_str().to_string();
                match resp.text().await {
                    Ok(html) => {
                        if final_url.contains("consent.youtube.com") {
                            log::warn!("[YouTube] live_chat hit consent (attempt {})", attempt);
                        } else if let Some(init) = extract_json(&html, "ytInitialData") {
                            // This page carries the channel's FULL emoji set, which
                            // is what YouTube's own picker reads. Harvesting emoji
                            // from messages only ever learns the ones somebody
                            // happened to post; this is the actual list.
                            store_channel_emojis(video_id, identifier, &init);
                            if let Some(c) = live_view_continuation(&init) {
                                return Some(c);
                            }
                            log::warn!(
                                "[YouTube] live_chat had no Live view selector (attempt {}, {} bytes)",
                                attempt, html.len()
                            );
                        } else {
                            log::warn!(
                                "[YouTube] live_chat had no ytInitialData (attempt {}, {} bytes)",
                                attempt, html.len()
                            );
                        }
                    }
                    Err(e) => log::warn!("[YouTube] live_chat read failed (attempt {}): {}", attempt, e),
                }
            }
            Err(e) => log::warn!("[YouTube] live_chat fetch failed (attempt {}): {}", attempt, e),
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    None
}

/// From the live_chat page's `ytInitialData`, the view-selector item whose title
/// isn't "Top chat" — the full live feed.
fn live_view_continuation(iframe_initial: &Value) -> Option<String> {
    let items = iframe_initial
        .pointer("/contents/liveChatRenderer/header/liveChatHeaderRenderer/viewSelector/sortFilterSubMenuRenderer/subMenuItems")?
        .as_array()?;
    let live = items.iter().find(|it| {
        it.get("title")
            .and_then(|t| t.as_str())
            .map(|t| !t.to_lowercase().contains("top"))
            .unwrap_or(false)
    })?;
    live.pointer("/continuation/reloadContinuationData/continuation")
        .and_then(|c| c.as_str())
        .map(String::from)
}

fn extract_meta(player: Option<&Value>, initial: Option<&Value>, html: &str) -> YouTubeChannelMeta {
    let vd = player.and_then(|p| p.get("videoDetails"));
    let title = vd
        .and_then(|d| d.get("title"))
        .and_then(|t| t.as_str())
        .map(String::from);
    let username = vd
        .and_then(|d| d.get("author"))
        .and_then(|t| t.as_str())
        .map(String::from);
    let user_id = vd
        .and_then(|d| d.get("channelId"))
        .and_then(|t| t.as_str())
        .map(String::from);
    // `isLive` is the ONLY field that means "live right now". `isLiveContent`
    // means "this video is, or once was, a broadcast", which is equally true of
    // a finished archive and of a waiting room that never started - so reading
    // it as a fallback put permanently-offline channels in the who's-live list.
    // YouTube's /live redirect serves a channel's scheduled broadcast even when
    // it was scheduled years ago and never began; measured 2026-09-19 on a
    // channel still fronting one from 2017, whose payload carries
    // `isLiveContent: true`, `isUpcoming: true` and NO `isLive`. Clicking it
    // failed with "This live event will begin in a few moments".
    //
    // Absent means NOT live, so `false` is the honest default: measured against
    // two running broadcasts the same day, a live payload always carries
    // `isLive: true` outright.
    let is_upcoming = vd
        .and_then(|d| d.get("isUpcoming"))
        .and_then(|b| b.as_bool())
        .unwrap_or(false);
    let is_live = !is_upcoming
        && vd
            .and_then(|d| d.get("isLive"))
            .and_then(|b| b.as_bool())
            .unwrap_or(false);
    let start_time = player
        .and_then(|p| {
            p.pointer("/microformat/playerMicroformatRenderer/liveBroadcastDetails/startTimestamp")
        })
        .and_then(|s| s.as_str())
        .map(String::from);
    // Concurrent viewers: best-effort from ytInitialData's view-count renderer.
    let viewer_count = json_str_after(html, "\"originalViewCount\":\"").and_then(|s| s.parse().ok());
    let profile_pic = initial.and_then(find_owner_avatar);
    let video_id = vd
        .and_then(|d| d.get("videoId"))
        .and_then(|v| v.as_str())
        .map(String::from);
    let game_name = initial.and_then(find_game_name);

    YouTubeChannelMeta {
        user_id,
        username,
        viewer_count,
        start_time,
        title,
        profile_pic,
        game_name,
        is_live,
        video_id,
        api_key: None,
        client_version: None,
        visitor_data: None,
    }
}

/// Best-effort: the game YouTube attached to this broadcast.
///
/// YouTube shows it as a "box art" rich-metadata card under the video. The shape
/// is not contractual, so this scans for the renderer anywhere in the tree rather
/// than walking a fixed path, and returns None when nothing matches — which is
/// simply the previous behavior (empty category), so a shape change costs nothing
/// beyond the log line.
fn find_game_name(initial: &Value) -> Option<String> {
    fn walk(v: &Value, depth: usize) -> Option<String> {
        if depth > 12 {
            return None;
        }
        match v {
            Value::Object(map) => {
                if let Some(rich) = map.get("richMetadataRenderer") {
                    // Only the box-art variant is a game; the others are playlists,
                    // articles and "watch on" promos.
                    let is_box_art = rich
                        .get("style")
                        .and_then(|s| s.as_str())
                        .is_some_and(|s| s.contains("BOX_ART"));
                    if is_box_art {
                        if let Some(t) = runs_text(rich.get("title")) {
                            if !t.is_empty() {
                                return Some(t);
                            }
                        }
                    }
                }
                map.values().find_map(|child| walk(child, depth + 1))
            }
            Value::Array(items) => items.iter().find_map(|child| walk(child, depth + 1)),
            _ => None,
        }
    }
    let found = walk(initial, 0);
    if found.is_none() {
        // Expected often: YouTube attaches a game to only some broadcasts.
        log::debug!("[YouTube] no box-art category on this broadcast");
    }
    found
}

/// Best-effort: the channel owner's avatar from the secondary-info owner renderer.
fn find_owner_avatar(initial: &Value) -> Option<String> {
    let contents = initial
        .pointer("/contents/twoColumnWatchNextResults/results/results/contents")
        .and_then(|c| c.as_array())?;
    for item in contents {
        if let Some(thumbs) = item.pointer(
            "/videoSecondaryInfoRenderer/owner/videoOwnerRenderer/thumbnail/thumbnails",
        ) {
            if let Some(arr) = thumbs.as_array() {
                if let Some(url) = arr.last().and_then(|t| t.get("url")).and_then(|u| u.as_str()) {
                    return Some(url.to_string());
                }
            }
        }
    }
    None
}

// --- The long-poll stream loop ----------------------------------------------

/// Bounded FIFO set of message ids already published on a connection. `insert`
/// reports whether the id is new, so a replayed backlog can be dropped without
/// growing memory for the whole stream.
struct SeenIds {
    set: HashSet<String>,
    order: VecDeque<String>,
}

impl SeenIds {
    fn new() -> Self {
        Self {
            set: HashSet::new(),
            order: VecDeque::new(),
        }
    }

    /// True if this id hadn't been published yet (and records it).
    fn insert(&mut self, id: &str) -> bool {
        if !self.set.insert(id.to_string()) {
            return false;
        }
        self.order.push_back(id.to_string());
        while self.order.len() > SEEN_IDS_MAX {
            if let Some(old) = self.order.pop_front() {
                self.set.remove(&old);
            }
        }
        true
    }
}

/// The renderer id of an `addChatItemAction`, used to suppress a replayed backlog.
/// Moderation actions carry no item id and are never suppressed.
fn action_item_id(action: &Value) -> Option<&str> {
    action
        .pointer("/addChatItemAction/item")?
        .as_object()?
        .values()
        .next()?
        .get("id")?
        .as_str()
}

/// Stream forever: run the live chat poll, and if it ends (stream over / token
/// expired / error) wait then re-resolve, until the task is aborted on disconnect.
async fn run_connection(
    http: reqwest::Client,
    identifier: String,
    channel_key: String,
    initial: LiveContext,
) {
    let id_lc = identifier.to_lowercase();
    let mut next_ctx = Some(initial);
    // Lives across re-resolves on purpose: each resolve restarts from a bootstrap
    // continuation, which re-serves the backlog we already showed. Without this, a
    // single blip repeats ~20 seconds of chat into the pane.
    let mut seen = SeenIds::new();
    // Consecutive failing rounds, driving the error backoff.
    let mut fail_streak: u32 = 0;
    loop {
        let ctx = match next_ctx.take() {
            Some(c) => c,
            None => match resolve_live_video(&http, &identifier).await {
                Ok(r) => {
                    store_meta(&id_lc, r.meta);
                    r.ctx
                }
                Err(e) => {
                    // A bare VIDEO id addresses ONE broadcast. Once that broadcast is
                    // over it never comes back, so re-resolving it is a loop that can
                    // only fail, forever, logging every time. A @handle or UC id is
                    // the opposite case: the channel can go live again, so those keep
                    // retrying.
                    if is_video_id(&identifier) {
                        log::info!(
                            "[YouTube] '{}' is a finished broadcast ({}); ending the chat connection",
                            identifier,
                            e
                        );
                        return;
                    }
                    log::info!("[YouTube] re-resolve '{}' failed: {}", identifier, e);
                    tokio::time::sleep(Duration::from_secs(RE_RESOLVE_DELAY_SECS)).await;
                    continue;
                }
            },
        };
        // A clean return means the stream/chat ended — back off before re-resolving so
        // we don't hammer an offline channel. An error (timeout, network blip, a bad
        // poll response) is transient on a live chat, so re-resolve quickly instead of
        // leaving the pane stuck on "Waiting for messages".
        //
        // The whole poll is wrapped in catch_unwind so one malformed YouTube payload
        // can't panic the task to death. A dead task is unrecoverable: its slot stays in
        // the conn map, so a later re-acquire finds it and never respawns the stream —
        // YouTube would go silently dead until an app restart. A caught panic recovers
        // like an error: log and re-resolve.
        let round_started = std::time::Instant::now();
        let outcome = AssertUnwindSafe(stream_live_chat(&http, ctx, &channel_key, &mut seen))
            .catch_unwind()
            .await;
        // A round that ran for a while was genuinely streaming, so whatever ended
        // it is a fresh problem and the backoff starts over.
        if round_started.elapsed().as_secs() >= HEALTHY_ROUND_SECS {
            fail_streak = 0;
        }
        let delay = match outcome {
            Ok(Ok(())) => {
                fail_streak = 0;
                RE_RESOLVE_DELAY_SECS
            }
            Ok(Err(e)) => {
                let delay = error_backoff(&mut fail_streak);
                log::warn!(
                    "[YouTube] '{}' stream error: {} (retry in {}s)",
                    identifier,
                    e,
                    delay
                );
                delay
            }
            Err(_) => {
                let delay = error_backoff(&mut fail_streak);
                log::error!(
                    "[YouTube] '{}' poll panicked; recovering in {}s",
                    identifier,
                    delay
                );
                delay
            }
        };
        tokio::time::sleep(Duration::from_secs(delay)).await;
    }
}

/// Poll `get_live_chat` until the continuation runs out (stream ended) or a
/// request fails. Each response carries the next continuation + how long to wait.
async fn stream_live_chat(
    http: &reqwest::Client,
    mut ctx: LiveContext,
    channel_key: &str,
    seen: &mut SeenIds,
) -> Result<()> {
    let url = format!(
        "https://www.youtube.com/youtubei/v1/live_chat/get_live_chat?key={}&prettyPrint=false",
        ctx.api_key
    );
    // Whether we've seen a live-chat block at all this session. Used to tell a real
    // stream-end (we were receiving, then it stopped) from a bad/stale first response
    // (resolve handed us a dud continuation) so the latter re-resolves fast.
    let mut got_chat = false;
    // The first response of THIS call is the join backlog (see BACKLOG_KEEP).
    let mut first_batch = true;
    loop {
        let mut client = json!({
            "clientName": "WEB",
            "clientVersion": ctx.client_version,
            "hl": "en",
            "gl": "US",
        });
        if let Some(vd) = &ctx.visitor_data {
            client["visitorData"] = json!(vd);
        }
        let body = json!({ "context": { "client": client }, "continuation": ctx.continuation });

        // The poll window is measured from the request going OUT, so the round trip
        // and the dispatch work come out of it rather than being added to it. The old
        // pacing added its sleeps on top of the request, which stretched the cadence
        // under load and pulled an even bigger batch on the next poll.
        let started = tokio::time::Instant::now();
        let resp = http.post(&url).json(&body).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            // A 400 means the token itself was refused, not that the network
            // blipped: retrying the same continuation in a few seconds just
            // repeats it. The usual cause is a broadcast that ended mid-poll, so
            // this exits CLEANLY and lets the caller take the long re-resolve
            // backoff (where the liveness check reports it properly) instead of
            // hammering. Everything else stays a fast-retry error.
            if status == reqwest::StatusCode::BAD_REQUEST {
                log::info!("[YouTube] continuation refused (400); treating as stream end");
                return Ok(());
            }
            return Err(anyhow!("get_live_chat HTTP {}", status));
        }
        let v: Value = resp.json().await?;
        let Some(lcc) = v.pointer("/continuationContents/liveChatContinuation") else {
            // No live-chat block. If we'd been receiving, the stream/chat ended (clean
            // exit, back off). If we never got one, the resolve handed us a dud — error
            // so the loop re-resolves quickly instead of stalling on "Waiting".
            return if got_chat {
                Ok(())
            } else {
                Err(anyhow!("no live chat block in first response"))
            };
        };
        got_chat = true;

        // Next token + poll delay first, so we can pace this batch across the delay.
        // An absent continuations array means it's over.
        let Some(cont) = lcc
            .get("continuations")
            .and_then(|c| c.as_array())
            .and_then(|a| a.first())
        else {
            return Ok(());
        };
        let (next, timeout_ms) = pick_next_continuation(cont);
        let Some(next) = next else { return Ok(()) };

        let deadline = started + Duration::from_millis(timeout_ms);

        // Drop anything already published on this connection. A re-resolve restarts
        // from a bootstrap continuation, which re-serves the backlog we just showed;
        // moderation actions carry no item id and always pass through.
        let batch: Vec<&Value> = lcc
            .get("actions")
            .and_then(|a| a.as_array())
            .map(|arr| {
                arr.iter()
                    .filter(|a| action_item_id(a).map(|id| seen.insert(id)).unwrap_or(true))
                    .collect()
            })
            .unwrap_or_default();

        if first_batch {
            // Join backlog: deliver at once (the frontend coalesces it into a single
            // render) and keep only the newest rows, so joining looks like arriving
            // with context instead of a high-speed replay of the last ~20 seconds.
            // Its rows are history (the join's context, or what a re-resolve missed),
            // so they carry `from_backfill` like Twitch's join history does: the list
            // then draws them without the per-row entrance animation instead of
            // playing it on thirty rows at once.
            first_batch = false;
            if batch.len() > BACKLOG_KEEP {
                log::info!(
                    "[YouTube] trimmed a {}-row join backlog to {}",
                    batch.len(),
                    BACKLOG_KEEP
                );
            }
            for action in batch.iter().skip(batch.len().saturating_sub(BACKLOG_KEEP)) {
                process_action(action, channel_key, true).await;
            }
        } else if !batch.is_empty() {
            // Live: spread the batch across what's LEFT of the poll window, so a fast
            // chat flows instead of arriving in one clump per poll then freezing. The
            // gap is capped so a quiet chat still shows messages promptly.
            let remaining = deadline
                .saturating_duration_since(tokio::time::Instant::now())
                .as_millis() as u64;
            let gap = (remaining / batch.len() as u64).min(MAX_PACE_GAP_MS);
            for action in &batch {
                process_action(action, channel_key, false).await;
                // Never pace past the window: if dispatch ran long, the rest of the
                // batch goes out at once so the next poll still leaves on time.
                if tokio::time::Instant::now() >= deadline {
                    continue;
                }
                tokio::time::sleep(Duration::from_millis(gap)).await;
            }
        }
        // Whatever the branch did, the next poll goes out on the original cadence: a
        // batch that overran its window costs nothing extra rather than compounding.
        // (A past deadline means the request itself took longer than the window, so
        // polls are already self-spaced by their own duration, so there's no busy loop.)
        tokio::time::sleep_until(deadline).await;
        ctx.continuation = next;
    }
}

/// Read the next continuation token + its poll delay from `continuations[0]`,
/// whichever sub-shape is present (live vs reload vs replay).
fn pick_next_continuation(c: &Value) -> (Option<String>, u64) {
    for kkey in [
        "invalidationContinuationData",
        "timedContinuationData",
        "reloadContinuationData",
        "liveChatReplayContinuationData",
    ] {
        if let Some(d) = c.get(kkey) {
            let cont = d
                .get("continuation")
                .and_then(|x| x.as_str())
                .map(String::from);
            let ms = d
                .get("timeoutMs")
                .and_then(|x| x.as_u64())
                .unwrap_or(DEFAULT_POLL_MS);
            // Cap well below YouTube's hint (it suggests ~10s and bundles the whole
            // window into one clump): a faster cadence pulls smaller batches so a busy
            // chat keeps up instead of arriving in 10s lurches. Verified to return no
            // duplicates/empties when polled at this rate.
            return (cont, ms.clamp(1000, 2000));
        }
    }
    (None, DEFAULT_POLL_MS)
}

/// Dispatch one live-chat action: a new chat item, or a moderation removal.
/// The top-level key names of a payload, for shape diagnostics that must not log
/// user content.
fn action_keys(action: &Value) -> Vec<String> {
    action
        .as_object()
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default()
}

async fn process_action(action: &Value, channel_key: &str, backlog: bool) {
    if let Some(item) = action.pointer("/addChatItemAction/item") {
        if let Some(mut msg) = parse_item(item, channel_key) {
            msg.metadata.from_backfill = backlog;
            publish_chat_message(&msg).await;
        }
        return;
    }
    // A single message removed by a mod -> CLEARMSG (the frontend recovers the
    // author + text from chat history by the target id). YouTube emits two shapes:
    // `markChatItemAsDeletedAction` (the authed mod view, leaves a "[deleted]" tomb)
    // and `removeChatItemAction` (what an anonymous reader actually receives) — so
    // accept either, else viewer-side mod logs never populate.
    if let Some(id) = action
        .pointer("/markChatItemAsDeletedAction/targetItemId")
        .or_else(|| action.pointer("/removeChatItemAction/targetItemId"))
        .and_then(|x| x.as_str())
    {
        // Prints the payload's SHAPE, not its content: we still want to know what
        // else YouTube carries here (deletor name, deleted-state message) to enrich
        // the mod log, but dumping the whole action logged deleted message text on
        // every deletion.
        log::info!("[YouTube][moddump] delete keys: {:?}", action_keys(action));
        let frame = json!({
            "type": "CLEARMSG",
            "provider": "youtube",
            "channel": channel_key,
            "target_msg_id": id,
        });
        publish_frame(frame.to_string()).await;
        return;
    }
    // All of an author's messages removed (ban/timeout) -> CLEARCHAT by channel id.
    // Same two-shape story: `markChatItemsByAuthorAsDeletedAction` vs the anonymous
    // `removeChatItemByAuthorAction`.
    if let Some(uid) = action
        .pointer("/markChatItemsByAuthorAsDeletedAction/externalChannelId")
        .or_else(|| action.pointer("/removeChatItemByAuthorAction/externalChannelId"))
        .and_then(|x| x.as_str())
    {
        // Shape only (see the delete arm). The open question here is whether
        // YouTube distinguishes a timeout from a permanent ban at all; the key list
        // answers that without logging anyone's messages.
        log::info!(
            "[YouTube][moddump] author-removal keys: {:?}",
            action_keys(action)
        );
        let frame = json!({
            "type": "CLEARCHAT",
            "provider": "youtube",
            "channel": channel_key,
            "target_user_id": uid,
        });
        publish_frame(frame.to_string()).await;
        return;
    }
    // A pinned message. Emits the same PINNED/UNPINNED frames Kick does, so the
    // pinned banner the frontend already renders for every non-Twitch provider
    // starts working with no frontend change.
    if let Some(banner) =
        action.pointer("/addBannerToLiveChatCommand/bannerRenderer/liveChatBannerRenderer")
    {
        if let Some(frame) = build_pinned_banner(banner, channel_key) {
            publish_frame(frame).await;
        } else {
            // MEASURE (chat-parity): a banner we could not read. Banners also
            // carry poll and product shapes, so this says which one arrived.
            log::info!("[YouTube][discover] unparsed banner: {}", banner);
        }
        return;
    }
    if action.get("removeBannerForLiveChatCommand").is_some() {
        let frame = json!({
            "type": "UNPINNED",
            "provider": "youtube",
            "channel": channel_key,
        });
        publish_frame(frame.to_string()).await;
        return;
    }
    // Anything else. Renderers already have a discovery log inside `parse_item`,
    // but ACTIONS had none, which is exactly where the banner (pinned message)
    // and poll payloads live — so an unhandled one looked identical to one that
    // never arrived. Ticker items are skipped because they restate a chat item
    // we already rendered and would flood the log on a busy channel.
    if let Some(name) = action
        .as_object()
        .and_then(|o| o.keys().find(|k| k.as_str() != "clickTrackingParams"))
    {
        if name != "addLiveChatTickerItemAction" {
            log::info!("[YouTube][discover] action {}: {}", name, action);
        }
    }
}

/// Build the shared PINNED frame from a `liveChatBannerRenderer`.
///
/// Every key here mirrors `kick.rs build_pinned` exactly, because the frontend
/// consumes one pin shape for all providers; an invented variant would render
/// blank fields rather than fail loudly. Returns None for banner kinds that are
/// not a pinned chat message (polls, product shelves), so the caller can log them.
fn build_pinned_banner(banner: &Value, channel_key: &str) -> Option<String> {
    let msg = banner.pointer("/contents/liveChatTextMessageRenderer")?;
    let message_id = msg.get("id").and_then(|x| x.as_str())?;
    // parse_runs flattens emoji runs to their alt text, which is the YouTube
    // equivalent of Kick's strip_emote_tokens for this plain-text surface.
    let (_segments, plain) = parse_runs(msg.get("message"));
    let sender_id = msg
        .get("authorExternalChannelId")
        .and_then(|x| x.as_str())
        .unwrap_or("");
    let sender_avatar = msg
        .pointer("/authorPhoto/thumbnails")
        .and_then(|t| t.as_array())
        .and_then(|a| a.last())
        .and_then(|t| t.get("url"))
        .and_then(|u| u.as_str())
        .map(absolutize_url)
        .unwrap_or_default();
    // The banner header reads like "Pinned by <name>". It is localized, so the
    // prefix is stripped only when it is actually there and the whole string is
    // kept otherwise rather than mangled.
    let pinned_by = runs_text(banner.pointer("/header/liveChatBannerHeaderRenderer/text"))
        .map(|t| t.trim_start_matches("Pinned by ").trim().to_string())
        .unwrap_or_default();
    let pin = json!({
        "id": message_id,
        "message_id": message_id,
        "type": "youtube",
        "message_text": plain,
        "sender_id": sender_id,
        // Same color derivation the chat row uses, so the banner and the message
        // agree on who this is.
        "sender_color": color_for(sender_id),
        "sender_name": msg
            .pointer("/authorName/simpleText")
            .and_then(|x| x.as_str())
            .unwrap_or(""),
        "sender_avatar": sender_avatar,
        "sender_badges": [],
        "pinned_by": pinned_by,
        "pinned_by_id": "",
        "pinned_by_avatar": "",
        "started_at": msg
            .get("timestampUsec")
            .and_then(|x| x.as_str())
            .map(usec_to_iso)
            .unwrap_or_default(),
    });
    let out = json!({ "type": "PINNED", "provider": "youtube", "channel": channel_key, "pin": pin });
    Some(out.to_string())
}

/// Decode one `addChatItemAction.item` renderer into a unified ChatMessage.
/// Normal text messages render natively; Super Chats / stickers / memberships are
/// stamped as event messages (msg_type + system message) so they show inline and
/// flow to the activity panel.
fn parse_item(item: &Value, channel_key: &str) -> Option<ChatMessage> {
    if let Some(r) = item.get("liveChatTextMessageRenderer") {
        return Some(parse_text_message(r, channel_key));
    }
    if let Some(r) = item.get("liveChatPaidMessageRenderer") {
        return Some(parse_paid(r, channel_key, "superchat"));
    }
    if let Some(r) = item.get("liveChatPaidStickerRenderer") {
        return Some(parse_paid(r, channel_key, "supersticker"));
    }
    if let Some(r) = item.get("liveChatMembershipItemRenderer") {
        return Some(parse_membership(r, channel_key));
    }
    if let Some(r) = item.get("liveChatSponsorshipsGiftPurchaseAnnouncementRenderer") {
        return Some(parse_gift_membership(r, channel_key));
    }
    // The RECEIVING half of a gift membership ("X was gifted a membership by Y").
    // Twitch shows both the gifter and the recipient; without this YouTube only
    // ever showed the gifter.
    if let Some(r) = item.get("liveChatSponsorshipsGiftRedemptionAnnouncementRenderer") {
        return Some(parse_gift_redemption(r, channel_key));
    }
    // MEASURE (chat-parity): the modern ViewModel shape for gift announcements.
    // On channels that send it, the legacy renderer arm above never fires and the
    // gift is dropped entirely. Logged in full until a real payload confirms where
    // the author and count live, then routed like parse_gift_membership.
    if let Some(vm) = item.get("giftMessageViewModel") {
        log::info!("[YouTube][discover] giftMessageViewModel: {}", vm);
        return None;
    }
    // Chat mode changes (slow mode on, members-only on, ...). Rendered as a
    // system row rather than parsed into machine state: the strings are localized
    // presentation text, so keyword-matching them would break on any non-English
    // client locale, and a misread would wrongly lock the composer.
    if let Some(r) = item.get("liveChatModeChangeMessageRenderer") {
        return parse_mode_change(r, channel_key);
    }
    // Engagement / system notices and anything else: ignore (logged for discovery).
    if let Some((name, _)) = item.as_object().and_then(|o| o.iter().next()) {
        // Placeholders are deliberately dropped (a real item replaces them) and
        // arrive constantly, so they are not news.
        let known_noise = matches!(
            name.as_str(),
            "liveChatViewerEngagementMessageRenderer" | "liveChatPlaceholderItemRenderer"
        );
        if !known_noise {
            log::info!("[YouTube][discover] renderer {}", name);
        }
    }
    None
}

/// "X was gifted a membership by Y". Stamped `subgift`, the msg-id Twitch uses
/// for a single gifted sub, so the existing subscription card renders it with no
/// frontend change (the same trick the Kick sub path uses).
fn parse_gift_redemption(r: &Value, channel_key: &str) -> ChatMessage {
    let (segments, plain) = parse_runs(r.get("message"));
    let display = r
        .pointer("/authorName/simpleText")
        .and_then(|x| x.as_str())
        .unwrap_or("");
    // The runs read "was gifted a membership by <gifter>", so the recipient's
    // name has to be prepended for the line to make sense on its own.
    let system = if plain.is_empty() {
        format!("{} was gifted a membership", display)
    } else {
        format!("{} {}", display, plain)
    };
    let (segments, plain) = if segments.is_empty() {
        (
            vec![MessageSegment::Text {
                content: system.clone(),
            }],
            system.clone(),
        )
    } else {
        (segments, plain)
    };
    base_message(
        r,
        channel_key,
        segments,
        plain,
        None,
        Some(("subgift", system)),
    )
}

/// A chat mode change, as the yellow system row (`username === 'System'` is what
/// the frontend keys on). Deliberately NOT a new msg-id: an id ChatMessage.tsx
/// does not recognize falls through to a plain row.
fn parse_mode_change(r: &Value, channel_key: &str) -> Option<ChatMessage> {
    let text = runs_text(r.get("text")).unwrap_or_default();
    let subtext = runs_text(r.get("subtext")).unwrap_or_default();
    let line = match (text.is_empty(), subtext.is_empty()) {
        (true, true) => return None,
        (false, false) => format!("{} · {}", text, subtext),
        (true, false) => subtext,
        (false, true) => text,
    };
    let mut msg = base_message(
        r,
        channel_key,
        vec![MessageSegment::Text {
            content: line.clone(),
        }],
        line,
        None,
        None,
    );
    // These renderers carry no author, so the identity is stamped here.
    msg.username = "System".to_string();
    msg.display_name = "YouTube".to_string();
    msg.badges = Vec::new();
    msg.tags
        .insert("display-name".to_string(), "YouTube".to_string());
    Some(msg)
}

fn parse_text_message(r: &Value, channel_key: &str) -> ChatMessage {
    let (segments, plain) = parse_runs(r.get("message"));
    let segments = bake_seventv(segments, channel_key);
    base_message(r, channel_key, segments, plain, None, None)
}

/// Super Chat + Super Sticker. The amount string drives a leading text segment so
/// the donation is visible inline; the renderer's header color tints the name.
fn parse_paid(r: &Value, channel_key: &str, kind: &str) -> ChatMessage {
    let amount_str = r
        .pointer("/purchaseAmountText/simpleText")
        .and_then(|x| x.as_str())
        .unwrap_or("");
    let (currency, amount_num) = parse_money(amount_str);
    let label = if kind == "supersticker" {
        format!("Super Sticker · {}", amount_str)
    } else {
        format!("Super Chat · {}", amount_str)
    };
    // The user's comment (empty for many super chats / all stickers).
    let (segments, comment) = parse_runs(r.get("message"));
    // A Super Chat comment is user-authored text like any other message.
    let mut segments = bake_seventv(segments, channel_key);
    // Prepend the donation label so the line reads e.g. "Super Chat · $5.00  <msg>".
    let lead = format!("{}  ", label);
    segments.insert(0, MessageSegment::Text { content: lead.clone() });
    let plain = format!("{}{}", lead, comment);
    // The header background color is YouTube's tier indicator; use it for the name.
    let color = argb_to_hex(r.get("headerBackgroundColor").or_else(|| r.get("backgroundColor")));
    let display = r
        .pointer("/authorName/simpleText")
        .and_then(|x| x.as_str())
        .unwrap_or("");
    let system = format!("{} from {}", label, display);
    let mut msg = base_message(r, channel_key, segments, plain, color, Some((kind, system)));
    // Activity-feed fields: the amount (currency + number) + the raw comment, so the
    // feed shows the value pill + the message (the producer reads these `sc-*` tags).
    // YouTube's OWN formatted amount ("$2.00", "₹100", "€2,00"). The numeric
    // tags below are for conversion/sorting; this one is what a card should show,
    // because reconstructing it from a bare float loses the currency's own
    // conventions (trailing zeros, comma decimals, symbol placement).
    if !amount_str.is_empty() {
        msg.tags.insert("sc-display".to_string(), amount_str.to_string());
    }
    if !currency.is_empty() {
        msg.tags.insert("sc-currency".to_string(), currency);
    }
    if let Some(a) = amount_num {
        msg.tags.insert("sc-amount".to_string(), format!("{}", a));
    }
    if !comment.is_empty() {
        msg.tags.insert("sc-message".to_string(), comment);
    }
    msg
}

/// Split a YouTube money string ("$5.00", "¥500", "€10,50") into (currency symbol,
/// numeric amount). Best-effort across the common formats.
fn parse_money(s: &str) -> (String, Option<f64>) {
    match s.find(|c: char| c.is_ascii_digit()) {
        Some(i) => {
            let currency = s[..i].trim().to_string();
            let num: String = s[i..]
                .chars()
                .filter(|c| c.is_ascii_digit() || *c == '.' || *c == ',')
                .collect();
            // If it has a '.', commas are thousands separators; else a comma is the
            // decimal mark.
            let normalized = if num.contains('.') {
                num.replace(',', "")
            } else {
                num.replace(',', ".")
            };
            (currency, normalized.parse::<f64>().ok())
        }
        None => (s.trim().to_string(), None),
    }
}

fn parse_membership(r: &Value, channel_key: &str) -> ChatMessage {
    // `headerSubtext` is a NEW member ("Welcome to <tier>!"); `headerPrimaryText`
    // is a milestone ("Member for 6 months"). Which key is present is the only
    // signal distinguishing them, and the milestone tag below is what lets the
    // activity feed tell them apart without a msg-id ChatMessage.tsx cannot render.
    let milestone = runs_text(r.get("headerPrimaryText"));
    let header = runs_text(r.get("headerSubtext"))
        .or_else(|| milestone.clone())
        .unwrap_or_else(|| "New member".to_string());
    let (segments, plain) = parse_runs(r.get("message"));
    let display = r
        .pointer("/authorName/simpleText")
        .and_then(|x| x.as_str())
        .unwrap_or("");
    let system = format!("{} · {}", display, header);
    let (segments, plain) = if segments.is_empty() {
        (vec![MessageSegment::Text { content: header.clone() }], header)
    } else {
        (segments, plain)
    };
    let mut msg = base_message(r, channel_key, segments, plain, None, Some(("membership", system)));
    // Keeps the recognized `membership` msg-id (so the member card still renders)
    // while telling the activity feed this is a tenure milestone, not a new member.
    if let Some(m) = milestone {
        msg.tags.insert("msg-param-milestone".to_string(), m);
    }
    msg
}

fn parse_gift_membership(r: &Value, channel_key: &str) -> ChatMessage {
    // The announcement header lives under a nested membership renderer.
    let inner = r.pointer("/header/liveChatSponsorshipsHeaderRenderer");
    let primary = runs_text(inner.and_then(|h| h.get("primaryText")))
        .unwrap_or_else(|| "gifted memberships".to_string());
    // YouTube embeds the count in the header text ("Gifted 5 memberships" /
    // "Sent 1 <channel> gift memberships"); the count is the first integer (it
    // precedes the channel name), so pull that out for the activity row.
    let count = primary
        .split(|c: char| !c.is_ascii_digit())
        .find_map(|w| w.parse::<u64>().ok())
        .unwrap_or(1);
    let display = inner
        .and_then(|h| h.pointer("/authorName/simpleText"))
        .and_then(|x| x.as_str())
        .unwrap_or("");
    let system = format!("{} {}", display, primary);
    let segs = vec![MessageSegment::Text { content: primary.clone() }];
    // Build off the inner header (it carries the gifter's name/badges/id).
    let mut msg = base_message(
        inner.unwrap_or(r),
        channel_key,
        segs,
        primary,
        None,
        Some(("membergift", system)),
    );
    // Reuse the Twitch mass-gift tag so the activity feed's gift-bomb row shows the
    // real count instead of "gifted 0".
    msg.tags
        .insert("msg-param-mass-gift-count".to_string(), count.to_string());
    msg
}

/// Assemble a ChatMessage from a renderer's common author fields + the prepared
/// segments. `event` (msg_type, system message) marks Super Chats / memberships.
fn base_message(
    r: &Value,
    channel_key: &str,
    segments: Vec<MessageSegment>,
    plain: String,
    color_override: Option<String>,
    event: Option<(&str, String)>,
) -> ChatMessage {
    let display_name = r
        .pointer("/authorName/simpleText")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let user_id = r
        .get("authorExternalChannelId")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let id = r
        .get("id")
        .and_then(|x| x.as_str())
        .map(String::from)
        .unwrap_or_else(|| format!("yt-{}", FALLBACK_SEQ.fetch_add(1, Ordering::Relaxed)));
    let timestamp = r
        .get("timestampUsec")
        .and_then(|x| x.as_str())
        .map(usec_to_iso)
        .unwrap_or_default();
    // YouTube exposes no per-user name color; derive a stable one from the channel
    // id so chatters are still distinguishable (donations keep the tier color).
    let color = color_override.or_else(|| Some(color_for(&user_id)));
    let badges = parse_badges(r.get("authorBadges"));

    // Stash this message's context-menu params so the mod-capability probe has a
    // real target to ask about (mod options appear only if WE can moderate).
    if let Some(p) = r
        .pointer("/contextMenuEndpoint/liveChatItemContextMenuEndpoint/params")
        .and_then(|x| x.as_str())
    {
        store_context_params(channel_key, p);
    }

    let mut tags = HashMap::new();
    tags.insert("display-name".to_string(), display_name.clone());
    tags.insert("id".to_string(), id.clone());
    // The chatter's profile picture rides every message; stamp it so the frontend
    // can render YouTube's native inline avatar (largest thumbnail).
    if let Some(url) = r
        .pointer("/authorPhoto/thumbnails")
        .and_then(|t| t.as_array())
        .and_then(|a| a.last())
        .and_then(|t| t.get("url"))
        .and_then(|u| u.as_str())
    {
        tags.insert("avatar".to_string(), absolutize_url(url));
    }

    let metadata = match &event {
        Some((kind, system)) => {
            tags.insert("msg-id".to_string(), (*kind).to_string());
            tags.insert("system-msg".to_string(), system.clone());
            MessageMetadata {
                msg_type: Some((*kind).to_string()),
                system_message: Some(system.clone()),
                ..Default::default()
            }
        }
        None => MessageMetadata::default(),
    };

    ChatMessage {
        id,
        user_id,
        username: display_name.to_lowercase(),
        display_name,
        color,
        badges,
        timestamp,
        content: plain,
        provider: "youtube".to_string(),
        channel: channel_key.to_string(),
        emotes: Vec::new(),
        tags,
        layout: LayoutResult {
            height: 0.0,
            width: 0.0,
            has_reply: false,
            is_first_message: false,
        },
        segments,
        metadata,
    }
}

/// Parse a renderer's `message.runs[]` into text + emoji image segments. Custom
/// and standard emoji both carry a thumbnail, so they're baked as image segments
/// (the renderer needs no YouTube emote context, mirroring Kick).
/// Swap words that name one of the channel's 7TV emotes for emote segments.
///
/// Runs over the segments `parse_runs` produced rather than inside it, because
/// the run parser has no channel context and is shared by every renderer. Only
/// Text segments are scanned; YouTube's own emoji are already Emote segments and
/// must not be re-examined.
///
/// The frontend renders provider segments verbatim, so this substitution has to
/// happen here - there is no name-to-emote pass on that side to fall back on.
fn bake_seventv(segments: Vec<MessageSegment>, channel_key: &str) -> Vec<MessageSegment> {
    let identifier = key::parse_key(channel_key).channel;
    // Overwhelmingly the common case: no 7TV set for this channel, so skip the
    // per-word scan entirely rather than taking the store lock once per word.
    if !youtube_emotes::has_emotes(&identifier) {
        return segments;
    }
    let mut out: Vec<MessageSegment> = Vec::with_capacity(segments.len());
    for seg in segments {
        let MessageSegment::Text { content } = &seg else {
            out.push(seg);
            continue;
        };
        // Split on spaces, keeping them, so the message reads exactly as sent.
        let mut buf = String::new();
        for (i, word) in content.split(' ').enumerate() {
            if i > 0 {
                buf.push(' ');
            }
            match youtube_emotes::lookup(&identifier, word) {
                Some(e) => {
                    if !buf.is_empty() {
                        out.push(MessageSegment::Text {
                            content: std::mem::take(&mut buf),
                        });
                    }
                    out.push(MessageSegment::Emote {
                        content: word.to_string(),
                        emote_id: Some(e.id),
                        emote_url: e.url,
                        is_zero_width: Some(e.zero_width),
                        modifier_flags: None,
                        is_personal: None,
                    });
                }
                None => buf.push_str(word),
            }
        }
        if !buf.is_empty() {
            out.push(MessageSegment::Text { content: buf });
        }
    }
    out
}

fn parse_runs(message: Option<&Value>) -> (Vec<MessageSegment>, String) {
    let mut segments = Vec::new();
    let mut plain = String::new();
    let Some(runs) = message.and_then(|m| m.get("runs")).and_then(|r| r.as_array()) else {
        return (segments, plain);
    };
    for run in runs {
        if let Some(text) = run.get("text").and_then(|t| t.as_str()) {
            plain.push_str(text);
            segments.push(MessageSegment::Text {
                content: text.to_string(),
            });
        } else if let Some(emoji) = run.get("emoji") {
            let url = emoji
                .pointer("/image/thumbnails")
                .and_then(|t| t.as_array())
                .and_then(|a| a.last())
                .and_then(|t| t.get("url"))
                .and_then(|u| u.as_str())
                .unwrap_or("");
            // Label: the :shortcut: when present, else the emoji id (often the char).
            let label = emoji
                .pointer("/shortcuts/0")
                .and_then(|s| s.as_str())
                .or_else(|| emoji.get("emojiId").and_then(|e| e.as_str()))
                .unwrap_or("");
            if url.is_empty() {
                plain.push_str(label);
                segments.push(MessageSegment::Text {
                    content: label.to_string(),
                });
            } else {
                plain.push_str(label);
                segments.push(MessageSegment::Emote {
                    content: label.to_string(),
                    emote_id: emoji.get("emojiId").and_then(|e| e.as_str()).map(String::from),
                    emote_url: url.to_string(),
                    is_zero_width: None,
                    modifier_flags: None,
                    is_personal: None,
                });
            }
        }
    }
    (segments, plain)
}

/// Map YouTube author badges. Member badges carry real custom art (rendered via
/// `image_url_1x`); the icon-only role badges (moderator/verified) carry no image,
/// so the frontend resolves them to bundled YouTube art by `name`.
fn parse_badges(author_badges: Option<&Value>) -> Vec<Badge> {
    let mut out = Vec::new();
    let Some(arr) = author_badges.and_then(|b| b.as_array()) else {
        return out;
    };
    for b in arr {
        let Some(r) = b.get("liveChatAuthorBadgeRenderer") else {
            continue;
        };
        let tooltip = r.get("tooltip").and_then(|t| t.as_str()).unwrap_or("");
        if let Some(thumbs) = r.pointer("/customThumbnail/thumbnails").and_then(|t| t.as_array()) {
            // Member badge: real per-tier art.
            let img = thumbs
                .last()
                .and_then(|t| t.get("url"))
                .and_then(|u| u.as_str())
                .map(String::from);
            out.push(Badge {
                name: "subscriber".to_string(),
                version: member_months(tooltip),
                image_url_1x: img,
                image_url_2x: None,
                image_url_4x: None,
                title: Some(if tooltip.is_empty() {
                    "Member".to_string()
                } else {
                    tooltip.to_string()
                }),
                description: None,
            });
        } else if let Some(icon) = r.pointer("/icon/iconType").and_then(|i| i.as_str()) {
            let name = match icon {
                "MODERATOR" => "moderator",
                "VERIFIED" => "verified",
                "OWNER" => "broadcaster",
                _ => continue,
            };
            out.push(Badge {
                name: name.to_string(),
                version: "1".to_string(),
                image_url_1x: None,
                image_url_2x: None,
                image_url_4x: None,
                title: Some(if tooltip.is_empty() {
                    name.to_string()
                } else {
                    tooltip.to_string()
                }),
                description: None,
            });
        }
    }
    out
}

/// Pull a month count out of a member tooltip like "Member (6 months)".
fn member_months(tooltip: &str) -> String {
    tooltip
        .split(|c: char| !c.is_ascii_digit())
        .find(|s| !s.is_empty())
        .map(String::from)
        .unwrap_or_else(|| "1".to_string())
}

// --- Small helpers ----------------------------------------------------------

/// Read a JSON string value that immediately follows a `"key":"` marker, up to the
/// next quote. Used for flat ytcfg values (API key, client version, visitorData)
/// whose values contain no embedded quotes.
#[cfg(test)]
mod absolutize_tests {
    use super::absolutize_url;

    /// The exact shape the subscriptions feed returns, and the exact shape that
    /// rendered as a broken avatar for 253 of one account's 258 follows.
    #[test]
    fn a_protocol_relative_feed_avatar_gets_a_scheme() {
        assert_eq!(
            absolutize_url("//yt3.googleusercontent.com/abc=s176-c-k-c0x00ffffff-no-rj-mo"),
            "https://yt3.googleusercontent.com/abc=s176-c-k-c0x00ffffff-no-rj-mo"
        );
    }

    /// The watch and live-chat endpoints already return absolute urls, and those
    /// must pass through byte for byte.
    #[test]
    fn an_absolute_url_is_untouched() {
        let u = "https://yt3.ggpht.com/ytc/AIdro_mN49Wf2Zqi=s176-c-k-c";
        assert_eq!(absolutize_url(u), u);
        let h = "http://example.test/a.png";
        assert_eq!(absolutize_url(h), h);
    }

    /// A single leading slash is a PATH, not a protocol-relative url, and
    /// rewriting it would invent a host out of the first path segment.
    #[test]
    fn a_rooted_path_is_not_protocol_relative() {
        assert_eq!(absolutize_url("/channel/UC123"), "/channel/UC123");
    }

    /// Idempotent, because the repair pass re-runs on every settings load.
    #[test]
    fn running_it_twice_changes_nothing() {
        let once = absolutize_url("//yt3.googleusercontent.com/x");
        assert_eq!(absolutize_url(&once), once);
    }
}

/// Make an InnerTube image URL absolute.
///
/// InnerTube is INCONSISTENT about this and the difference is invisible until it is
/// rendered. The watch and live-chat endpoints return `https://yt3.ggpht.com/...`,
/// while the BROWSE feeds return the same image protocol-relative, as
/// `//yt3.googleusercontent.com/...`.
///
/// A protocol-relative URL resolves against the page it is rendered in, and the app's
/// frontend is not served over https, so every one of those became a dead request and
/// drew a broken avatar. Measured on a real 258-channel subscription import: 253 came
/// back protocol-relative and 5 absolute, which is exactly how it presented — a
/// Following list of broken pictures with a handful that worked.
///
/// Worse than cosmetic: the broken value is still a value, so the stored avatar looked
/// present, the per-card resolver fell back for all 253 channels on every single load,
/// and the disk cache that exists to prevent precisely that did nothing.
pub(crate) fn absolutize_url(url: &str) -> String {
    match url.strip_prefix("//") {
        Some(rest) => format!("https://{}", rest),
        None => url.to_string(),
    }
}

pub(crate) fn json_str_after(html: &str, marker: &str) -> Option<String> {
    let idx = html.find(marker)?;
    let after = &html[idx + marker.len()..];
    let end = after.find('"')?;
    Some(after[..end].to_string())
}

/// Decode the handful of JSON escapes (`=`, `\/`) that can appear in a raw
/// scraped value like visitorData.
pub(crate) fn decode_json_escapes(s: &str) -> String {
    serde_json::from_str::<String>(&format!("\"{}\"", s)).unwrap_or_else(|_| s.to_string())
}

/// Extract a brace-balanced JSON object that follows a marker (`ytInitialData`,
/// `ytInitialPlayerResponse`), respecting strings/escapes so nested braces don't
/// trip it up.
pub(crate) fn extract_json(html: &str, marker: &str) -> Option<Value> {
    let idx = html.find(marker)?;
    let after = &html[idx + marker.len()..];
    let start = after.find('{')?;
    let bytes = after.as_bytes();
    let mut depth = 0i32;
    let mut in_str = false;
    let mut esc = false;
    let mut end = None;
    for (i, &byte) in bytes.iter().enumerate().skip(start) {
        let c = byte as char;
        if in_str {
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
        } else {
            match c {
                '"' => in_str = true,
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(i);
                        break;
                    }
                }
                _ => {}
            }
        }
    }
    serde_json::from_str(&after[start..=end?]).ok()
}

/// Flatten a `{ runs: [...] }` or `{ simpleText }` text object to a plain string.
fn runs_text(v: Option<&Value>) -> Option<String> {
    let v = v?;
    if let Some(s) = v.get("simpleText").and_then(|s| s.as_str()) {
        return Some(s.to_string());
    }
    let runs = v.get("runs").and_then(|r| r.as_array())?;
    let s: String = runs
        .iter()
        .filter_map(|r| r.get("text").and_then(|t| t.as_str()))
        .collect();
    (!s.is_empty()).then_some(s)
}

/// Convert a microsecond epoch string to an ISO-8601 UTC timestamp (what the
/// frontend's timestamp formatter expects, matching the Kick path's ISO strings).
fn usec_to_iso(usec: &str) -> String {
    usec.parse::<i64>()
        .ok()
        .and_then(|us| chrono::DateTime::from_timestamp(us / 1_000_000, ((us % 1_000_000) * 1000) as u32))
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_default()
}

/// YouTube's color ints are ARGB; render as `#rrggbb` (drop alpha).
fn argb_to_hex(v: Option<&Value>) -> Option<String> {
    let n = v?.as_u64()?;
    Some(format!("#{:06x}", n & 0x00ff_ffff))
}

/// A stable, readable name color derived from the channel id (YouTube gives none).
fn color_for(channel_id: &str) -> String {
    const PALETTE: [&str; 14] = [
        "#ff4f4f", "#ff8c42", "#ffd23f", "#9ee493", "#4fd1c5", "#4f9dff", "#7c6cff", "#c77dff",
        "#ff6fae", "#f25c54", "#43aa8b", "#577590", "#e07a5f", "#81b29a",
    ];
    let mut hash: u32 = 2166136261;
    for b in channel_id.bytes() {
        hash ^= b as u32;
        hash = hash.wrapping_mul(16777619);
    }
    PALETTE[(hash as usize) % PALETTE.len()].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Captured 2026-09-19 from `/channel/UCAhaFPP6v3WCfK5Tjao0B7A/live`, which
    /// still fronts a broadcast scheduled for March 2017 that never began.
    /// `isLiveContent` is true (it IS a broadcast), `isUpcoming` is true, and
    /// `isLive` is absent — reading the first as liveness reported this channel
    /// live in the who's-live list permanently, and clicking it failed with
    /// "This live event will begin in a few moments".
    #[test]
    fn an_abandoned_waiting_room_is_not_live() {
        let player = serde_json::json!({ "videoDetails": {
            "videoId": "ABAZa19-3s8",
            "title": "VIVE LA QUEUE ET LES OURS BLANCS",
            "author": "Mastu",
            "isLiveContent": true,
            "isUpcoming": true,
        }});
        let meta = extract_meta(Some(&player), None, "");
        assert!(!meta.is_live, "an upcoming broadcast has not started");
        assert_eq!(meta.username.as_deref(), Some("Mastu"));
    }

    /// The other half of the same measurement, from two broadcasts that really
    /// were running: a live payload states `isLive` outright. That is what makes
    /// `false` the safe default for an absent field.
    #[test]
    fn a_running_broadcast_is_live() {
        let player = serde_json::json!({ "videoDetails": {
            "videoId": "3PFJ9SETS4M",
            "author": "Lofi Girl",
            "isLive": true,
            "isLiveContent": true,
        }});
        assert!(extract_meta(Some(&player), None, "").is_live);
    }

    /// A finished stream keeps `isLiveContent: true` forever, and the browse
    /// shell a channel serves when nothing is on carries no `videoDetails` at
    /// all. Neither may be read as live.
    #[test]
    fn an_archive_and_a_browse_shell_are_not_live() {
        let archive = serde_json::json!({ "videoDetails": {
            "videoId": "1YRBlxS43xY",
            "author": "Ludwig",
            "isLiveContent": true,
        }});
        assert!(!extract_meta(Some(&archive), None, "").is_live, "an ended stream");
        assert!(!extract_meta(None, None, "").is_live, "a browse shell");
    }

    #[test]
    fn seen_ids_suppress_a_replayed_backlog() {
        let mut seen = SeenIds::new();
        assert!(seen.insert("a"));
        assert!(seen.insert("b"));
        // A re-resolve re-serves the same backlog: nothing new to publish.
        assert!(!seen.insert("a"));
        assert!(!seen.insert("b"));
        // Chat that arrived during the gap still gets through.
        assert!(seen.insert("c"));
    }

    #[test]
    fn seen_ids_stay_bounded() {
        let mut seen = SeenIds::new();
        for i in 0..(SEEN_IDS_MAX + 50) {
            assert!(seen.insert(&i.to_string()));
        }
        assert_eq!(seen.set.len(), SEEN_IDS_MAX);
        assert_eq!(seen.order.len(), SEEN_IDS_MAX);
        // The oldest ids aged out; the recent tail (what a fresh backlog overlaps) is kept.
        assert!(seen.insert("0"));
        assert!(!seen.insert(&(SEEN_IDS_MAX + 49).to_string()));
    }

    #[test]
    fn reads_the_item_id_off_any_renderer() {
        let add = json!({ "addChatItemAction": { "item": {
            "liveChatPaidMessageRenderer": { "id": "sc-1" }
        } } });
        assert_eq!(action_item_id(&add), Some("sc-1"));
        // Moderation actions carry no item id, so they're never suppressed.
        let del = json!({ "markChatItemAsDeletedAction": { "targetItemId": "x" } });
        assert_eq!(action_item_id(&del), None);
    }

    #[test]
    fn surfaces_youtubes_own_chat_unavailable_reason() {
        // Captured live from a 24/7 stream that PLAYS fine but has chat switched off.
        let initial = json!({ "contents": { "twoColumnWatchNextResults": {
            "conversationBar": { "conversationBarRenderer": {
                "availabilityMessage": { "messageRenderer": { "text": { "runs": [
                    { "text": "Chat is disabled for this live stream." }
                ] } } }
            } }
        } } });
        assert_eq!(
            conversation_bar_message(Some(&initial)).as_deref(),
            Some("Chat is disabled for this live stream."),
        );
        // A live watch page carries a renderer instead, and no message to report.
        let live = json!({ "contents": { "twoColumnWatchNextResults": {
            "conversationBar": { "liveChatRenderer": { "continuations": [] } }
        } } });
        assert_eq!(conversation_bar_message(Some(&live)), None);
    }

    #[test]
    fn classifies_identifiers() {
        assert!(live_page_url("@Repullze").contains("/@Repullze/live"));
        assert!(live_page_url("UC1234567890123456789012").contains("/channel/UC"));
        assert!(live_page_url("dQw4w9WgXcQ").contains("watch?v=dQw4w9WgXcQ"));
    }

    #[test]
    fn parses_a_text_message() {
        let item = json!({
            "liveChatTextMessageRenderer": {
                "id": "abc",
                "authorName": { "simpleText": "Alice" },
                "authorExternalChannelId": "UCxyz",
                "authorPhoto": { "thumbnails": [{ "url": "https://yt/a32.png" }, { "url": "https://yt/a64.png" }] },
                "timestampUsec": "1700000000000000",
                "message": { "runs": [
                    { "text": "hello " },
                    { "emoji": { "emojiId": "x", "shortcuts": [":smile:"],
                      "image": { "thumbnails": [{ "url": "https://e/x.png" }] } } }
                ] },
                "authorBadges": [
                    { "liveChatAuthorBadgeRenderer": { "icon": { "iconType": "MODERATOR" }, "tooltip": "Moderator" } }
                ]
            }
        });
        let msg = parse_item(&item, "youtube:test").expect("parsed");
        assert_eq!(msg.display_name, "Alice");
        assert_eq!(msg.user_id, "UCxyz");
        assert_eq!(msg.provider, "youtube");
        assert_eq!(msg.segments.len(), 2);
        assert_eq!(msg.content, "hello :smile:");
        assert_eq!(msg.badges.len(), 1);
        assert_eq!(msg.badges[0].name, "moderator");
        assert!(!msg.timestamp.is_empty());
        assert_eq!(msg.tags.get("avatar").map(String::as_str), Some("https://yt/a64.png"));
    }

    #[test]
    fn bootstrap_reads_watch_page_continuation() {
        let lcr = json!({ "continuations": [ { "reloadContinuationData": { "continuation": "BOOT" } } ] });
        assert_eq!(bootstrap_continuation(&lcr).as_deref(), Some("BOOT"));
    }

    #[test]
    fn live_view_prefers_non_top() {
        let iframe = json!({ "contents": { "liveChatRenderer": { "header": {
            "liveChatHeaderRenderer": { "viewSelector": { "sortFilterSubMenuRenderer": {
                "subMenuItems": [
                    { "title": "Top chat", "continuation": { "reloadContinuationData": { "continuation": "TOP" } } },
                    { "title": "Live chat", "continuation": { "reloadContinuationData": { "continuation": "LIVE" } } }
                ]
            } } } } } } });
        assert_eq!(live_view_continuation(&iframe).as_deref(), Some("LIVE"));
    }

    #[test]
    fn detects_currently_live() {
        // Live now: videoDetails.isLive true.
        let live = r#"var ytInitialPlayerResponse = {"videoDetails":{"isLive":true}};"#;
        assert!(is_currently_live(live));
        // Live now via the microformat broadcast details.
        let live2 = r#"ytInitialPlayerResponse = {"microformat":{"playerMicroformatRenderer":{"liveBroadcastDetails":{"isLiveNow":true}}}};"#;
        assert!(is_currently_live(live2));
        // Ended stream VOD (isLiveContent stays true, but isLive is false): reject,
        // so the browse-shell fallback never attaches to replay chat.
        let ended = r#"var ytInitialPlayerResponse = {"videoDetails":{"isLive":false,"isLiveContent":true}};"#;
        assert!(!is_currently_live(ended));
        // Plain upload / no player — reject.
        assert!(!is_currently_live(r#"var ytInitialPlayerResponse = {"videoDetails":{}};"#));
        assert!(!is_currently_live("no player here"));
    }

    #[test]
    fn collects_distinct_candidate_video_ids() {
        // Dedups repeats, skips non-11-char junk, preserves order, honors the cap.
        let html = concat!(
            r#"{"videoId":"AVfiMm5fK_U"}, {"videoId":"AVfiMm5fK_U"},"#,
            r#" {"videoId":"shortid"}, {"videoId":"qnuD9ZYOh9U"},"#,
            r#" {"videoId":"knjbSynIY5k"}, {"videoId":"jdkKmsj4ETY"}"#,
        );
        assert_eq!(
            candidate_video_ids(html, 3),
            vec![
                "AVfiMm5fK_U".to_string(),
                "qnuD9ZYOh9U".to_string(),
                "knjbSynIY5k".to_string(),
            ]
        );
        assert!(candidate_video_ids("no ids here", 3).is_empty());
    }

    #[test]
    fn next_continuation_reads_timeout() {
        // A long server-suggested interval is clamped down so fast chats don't lurch.
        let c = json!({ "timedContinuationData": { "continuation": "NEXT", "timeoutMs": 10000 } });
        let (next, ms) = pick_next_continuation(&c);
        assert_eq!(next.as_deref(), Some("NEXT"));
        assert_eq!(ms, 2000);
        // A value already inside the range passes through unchanged.
        let c2 = json!({ "timedContinuationData": { "continuation": "N2", "timeoutMs": 1500 } });
        assert_eq!(pick_next_continuation(&c2).1, 1500);
    }

    // Validate the protobuf encoder byte-for-byte against masterchat's own fixtures
    // (src/protobuf/assembler.test.ts). These exercise ld/vt/large-field-numbers/B1,
    // so matching them proves the primitives the send/moderate params are built from.
    #[test]
    fn protobuf_matches_masterchat_fixtures() {
        let t = pb::b1(&pb::cat(&[pb::ld(1, b"asr"), pb::ld(2, b"en"), pb::ld(3, b"")]));
        assert_eq!(t, "CgNhc3ISAmVuGgA%3D");

        let hdt = base64::engine::general_purpose::STANDARD.encode(pb::cat(&[
            pb::ld(1, &cv_token("bar", "foo")),
            pb::ld(3, &pb::ld(48687757, &pb::ld(1, b"foo"))),
            pb::vt(4, 1),
        ]));
        let lrc = pb::b1(&pb::ld(
            119693434,
            &pb::cat(&[pb::ld(3, hdt.as_bytes()), pb::vt(6, 1), pb::ld(16, &pb::vt(1, 4))]),
        ));
        assert_eq!(
            lrc,
            "0ofMyAMxGihDZ3dxQ2dvRFltRnlFZ05tYjI4YUMrcW8zYmtCQlFvRFptOXZJQUU9MAGCAQIIBA%3D%3D"
        );
    }
}

#[cfg(test)]
mod emoji_key_tests {
    use super::*;
    use serde_json::json;

    fn payload() -> Value {
        json!({
            "contents": { "liveChatRenderer": { "emojis": [
                {
                    "emojiId": "UCabc/xyz",
                    "shortcuts": [":hype:"],
                    "isCustomEmoji": true,
                    "image": { "thumbnails": [{ "url": "https://x/small.png" }, { "url": "https://x/big.png" }] }
                },
                {
                    "emojiId": "\u{1F600}",
                    "shortcuts": [":grinning:"],
                    "image": { "thumbnails": [{ "url": "https://x/grin.png" }] }
                }
            ]}}
        })
    }

    /// The whole point of the alias map: chat connects with an identifier, the
    /// emoji are found while resolving a VIDEO, and the frontend lowercases
    /// whatever it holds. Every one of those spellings must reach the same set.
    #[test]
    fn reachable_by_video_id_identifier_and_any_casing() {
        store_channel_emojis("AGr94tpNVkw", "UCsomeChannelId", &payload());

        for key in [
            "AGr94tpNVkw",     // the video id, as resolved
            "agr94tpnvkw",     // ...lowercased, which is what the frontend sends
            "UCsomeChannelId", // the identifier chat connected with
            "ucsomechannelid", // ...lowercased
        ] {
            assert_eq!(channel_emojis(key).len(), 2, "lookup failed for '{}'", key);
        }
        assert!(channel_emojis("UCsomethingElse").is_empty());
    }

    #[test]
    fn custom_emoji_come_first_and_take_the_largest_image() {
        store_channel_emojis("vid2", "chan2", &payload());
        let list = channel_emojis("vid2");
        assert!(!list[0].is_global, "the channel's own emoji should lead");
        assert_eq!(list[0].name, ":hype:");
        assert_eq!(list[0].url, "https://x/big.png");
        assert!(list[1].is_global);
    }

    /// Clearing one alias must drop the whole set, not orphan its other names.
    #[test]
    fn clearing_any_alias_drops_every_alias() {
        store_channel_emojis("vid3", "UCchan3", &payload());
        clear_channel_emojis("vid3");
        assert!(channel_emojis("vid3").is_empty());
        assert!(channel_emojis("UCchan3").is_empty(), "alias was orphaned");
    }
}

#[cfg(test)]
mod emoji_shape_tests {
    use super::*;
    use serde_json::json;

    /// The live payload had no key called `emojis` — matching on the NAME is what
    /// made the picker empty. Matching on the entry shape finds it wherever it is.
    #[test]
    fn finds_emoji_entries_under_an_unexpected_key() {
        let v = json!({
            "contents": { "liveChatRenderer": { "emojiPicker": { "someRenamedField": [
                { "emojiId": "a/b", "shortcuts": [":x:"], "isCustomEmoji": true,
                  "image": { "thumbnails": [{ "url": "https://x/1.png" }] } }
            ]}}}
        });
        let found = find_emoji_array(&v).expect("shape match should find a renamed field");
        assert_eq!(found.len(), 1);
    }

    /// A small category shelf must not win over the real set.
    #[test]
    fn prefers_the_largest_emoji_array() {
        let entry = |id: &str| json!({
            "emojiId": id, "shortcuts": [format!(":{}:", id)],
            "image": { "thumbnails": [{ "url": "https://x/i.png" }] }
        });
        let v = json!({
            "shelf": { "picks": [entry("a")] },
            "real":  { "list":  [entry("a"), entry("b"), entry("c")] }
        });
        assert_eq!(find_emoji_array(&v).unwrap().len(), 3);
    }

    #[test]
    fn ignores_arrays_that_merely_sit_near_the_word_emoji() {
        let v = json!({ "emojiPickerRenderer": { "categories": [{ "title": "Faces" }] } });
        assert!(find_emoji_array(&v).is_none());
    }
}

#[cfg(test)]
mod parity_tests {
    use super::*;

    // A pinned banner as YouTube nests it: the pinned chat message sits under
    // contents, with the "Pinned by X" header beside it.
    fn banner() -> Value {
        serde_json::json!({
            "header": { "liveChatBannerHeaderRenderer": {
                "text": { "runs": [{ "text": "Pinned by Streamer" }] }
            }},
            "contents": { "liveChatTextMessageRenderer": {
                "id": "MSG123",
                "authorExternalChannelId": "UCabc",
                "authorName": { "simpleText": "Alice" },
                "authorPhoto": { "thumbnails": [
                    { "url": "https://small.example/a.jpg" },
                    { "url": "https://large.example/a.jpg" }
                ]},
                "timestampUsec": "1700000000000000",
                "message": { "runs": [{ "text": "read the rules" }] }
            }}
        })
    }

    #[test]
    fn pinned_banner_matches_the_shared_pin_shape() {
        let raw = build_pinned_banner(&banner(), "youtube:chan").expect("banner should parse");
        let out: Value = serde_json::from_str(&raw).unwrap();

        assert_eq!(out["type"], "PINNED");
        assert_eq!(out["provider"], "youtube");
        assert_eq!(out["channel"], "youtube:chan");

        // Every key the frontend's pin consumer reads must be present; a missing
        // one renders as a blank field rather than failing, so assert the set.
        let pin = &out["pin"];
        for key in [
            "id", "message_id", "type", "message_text", "sender_id", "sender_name",
            "sender_color", "sender_avatar", "sender_badges", "pinned_by",
            "pinned_by_id", "pinned_by_avatar", "started_at",
        ] {
            assert!(pin.get(key).is_some(), "pin is missing `{key}`");
        }
        assert_eq!(pin["message_id"], "MSG123");
        assert_eq!(pin["message_text"], "read the rules");
        assert_eq!(pin["sender_name"], "Alice");
        assert_eq!(pin["pinned_by"], "Streamer");
        // Largest thumbnail wins, matching the inline-avatar rule.
        assert_eq!(pin["sender_avatar"], "https://large.example/a.jpg");
        // The banner and the chat row must agree on the author's color.
        assert_eq!(pin["sender_color"], color_for("UCabc"));
    }

    #[test]
    fn a_non_message_banner_is_rejected_rather_than_half_parsed() {
        // Poll and product banners share the command but carry different contents.
        let poll = serde_json::json!({
            "contents": { "liveChatPollRenderer": { "choices": [] } }
        });
        assert!(build_pinned_banner(&poll, "youtube:chan").is_none());
    }

    #[test]
    fn gift_redemption_rides_the_subgift_card() {
        let r = serde_json::json!({
            "id": "GIFT1",
            "authorExternalChannelId": "UCrecipient",
            "authorName": { "simpleText": "Bob" },
            "timestampUsec": "1700000000000000",
            "message": { "runs": [{ "text": "was gifted a membership by Carol" }] }
        });
        let msg = parse_gift_redemption(&r, "youtube:chan");
        // `subgift` is a msg-id ChatMessage.tsx already decorates; an unrecognized
        // one would silently fall through to a plain row.
        assert_eq!(msg.tags.get("msg-id").map(String::as_str), Some("subgift"));
        let system = msg.tags.get("system-msg").expect("system message");
        assert!(system.starts_with("Bob "), "recipient should lead: {system}");
        assert!(system.contains("Carol"), "gifter should survive: {system}");
    }

    #[test]
    fn mode_change_renders_as_the_system_row() {
        let r = serde_json::json!({
            "id": "MODE1",
            "timestampUsec": "1700000000000000",
            "text": { "runs": [{ "text": "Slow mode is on" }] },
            "subtext": { "runs": [{ "text": "Send a message every 10 seconds" }] }
        });
        let msg = parse_mode_change(&r, "youtube:chan").expect("should parse");
        // The frontend's yellow system card keys on exactly this username.
        assert_eq!(msg.username, "System");
        assert!(msg.content.contains("Slow mode is on"));
        assert!(msg.content.contains("every 10 seconds"));
        // No msg-id: an unrecognized event id would render plain instead.
        assert!(msg.tags.get("msg-id").is_none());
    }

    #[test]
    fn mode_change_without_text_is_dropped() {
        let r = serde_json::json!({ "id": "MODE2" });
        assert!(parse_mode_change(&r, "youtube:chan").is_none());
    }

    #[test]
    fn membership_milestone_is_tagged_but_keeps_the_membership_id() {
        let milestone = serde_json::json!({
            "id": "MEM1",
            "authorExternalChannelId": "UCm",
            "authorName": { "simpleText": "Dana" },
            "timestampUsec": "1700000000000000",
            "headerPrimaryText": { "runs": [{ "text": "Member for 6 months" }] }
        });
        let msg = parse_membership(&milestone, "youtube:chan");
        assert_eq!(msg.tags.get("msg-id").map(String::as_str), Some("membership"));
        assert_eq!(
            msg.tags.get("msg-param-milestone").map(String::as_str),
            Some("Member for 6 months")
        );

        // A brand-new member carries headerSubtext and must NOT be tagged.
        let new_member = serde_json::json!({
            "id": "MEM2",
            "authorExternalChannelId": "UCn",
            "authorName": { "simpleText": "Eve" },
            "timestampUsec": "1700000000000000",
            "headerSubtext": { "runs": [{ "text": "Welcome to Tier 1!" }] }
        });
        let msg = parse_membership(&new_member, "youtube:chan");
        assert_eq!(msg.tags.get("msg-id").map(String::as_str), Some("membership"));
        assert!(msg.tags.get("msg-param-milestone").is_none());
    }
}
