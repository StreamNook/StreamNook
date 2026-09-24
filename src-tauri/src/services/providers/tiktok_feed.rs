//! TikTok's live directory, fetched directly with requests TikTok's own page
//! signed.
//!
//! TikTok signs its web API requests inside its page (`X-Gnarly` and
//! `X-Dynosaur`, plus an `msToken`) and refuses one it did not sign, or one
//! altered after signing: a 403, or an empty 200. What it does not do is tie a
//! signature to the page that made it. The exact signed request, replayed from
//! here under the same user agent, is answered in full, fresh each time, with
//! no cookies at all. So the page is used only to SIGN, and every fetch of the
//! directory is a plain request from Rust:
//!
//!   * a hidden tiktok.com window is opened only when a category has no signed
//!     request yet, or TikTok stops accepting the one it has. It makes its feed
//!     request for each category that needs one, reports the signed URLs it
//!     sent, and closes soon after;
//!   * the signed requests are kept on disk, so a restart does not need the page;
//!   * a fetch is then a few hundred milliseconds, all categories at once.
//!
//! The feed answers each room with its full stream data, the same block room
//! info returns, so a room seen here also starts playing without that lookup.
//!
//! Two properties of the page decide the window's shape:
//!   * It requests its feed only once it has LAID OUT, so the window has real
//!     dimensions although it is never shown. A zero-size one loads nothing.
//!   * The request carries the whole browser environment, and a hand-built
//!     subset is refused, so the page's own request serves as the template.

use crate::models::provider_stream::ProviderStream;
use crate::services::providers::key::make_key;
use anyhow::{anyhow, Result};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[cfg(desktop)]
const LABEL: &str = "tiktok-feed";
#[cfg(desktop)]
const FEED_PAGE: &str = "https://www.tiktok.com/live/explore/Popular";
#[cfg(desktop)]
const SCRIPT: &str = include_str!("tiktok_feed.js");
/// A signing includes loading the page, which is the slow part.
#[cfg(desktop)]
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// How long the window outlives the last signing. Short: once the requests are
/// signed, nothing here needs the page until TikTok stops accepting them.
#[cfg(desktop)]
const IDLE_CLOSE: Duration = Duration::from_secs(20);
/// Long enough that moving around Home does not re-ask, short enough that
/// viewer counts are not visibly stale.
const CACHE_TTL: Duration = Duration::from_secs(60);
/// Rows younger than this are answered at once even when past `CACHE_TTL`,
/// with a refresh behind them, so a return to Discover never waits on the
/// network. Older than this, enough rooms have ended that waiting is better.
const STALE_OK: Duration = Duration::from_secs(10 * 60);
/// A direct fetch is one small request; past this it is not going to answer.
const REPLAY_TIMEOUT: Duration = Duration::from_secs(8);

/// What "Top live" is made of. The feed answers about a dozen rooms per keyword
/// and repeated calls mostly repeat, so breadth comes from asking across a few
/// of TikTok's own LIVE categories rather than from paging.
const TOP_LIVE: [&str; 3] = ["Popular", "Gaming", "Music"];

/// One room from the feed, or from the page's trimmed copy of it (which uses
/// these names in camelCase). Every field is untrusted input.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct FeedRow {
    room_id: String,
    handle: String,
    nickname: Option<String>,
    user_id: Option<String>,
    title: Option<String>,
    viewers: Option<u64>,
    cover: Option<String>,
    snapshot: Option<String>,
    avatar: Option<String>,
    category: Option<String>,
    /// When the LIVE started, unix seconds. Search answers carry it; the feed
    /// does not.
    started: Option<i64>,
}

/// Feed requests the page signed, replayable from here. Kept on disk.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct Signed {
    /// The user agent the page signed under. The signatures cover it: the same
    /// URL sent under any other agent is refused.
    ua: String,
    /// Category to the signed URL the page sent for it.
    urls: HashMap<String, String>,
    /// When each was signed, in unix seconds. Only ever reported: how long
    /// TikTok keeps accepting a signature is learned from its refusals.
    #[serde(default)]
    at: HashMap<String, u64>,
}

/// What the page reports after signing.
#[derive(Debug, Deserialize)]
struct SignAnswer {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    ua: String,
    #[serde(default)]
    signed: HashMap<String, String>,
    /// The page's own answer for each category it signed, trimmed to what a
    /// card shows. Rows are parsed one at a time so a malformed one is dropped
    /// rather than failing the signing.
    #[serde(default)]
    rows: HashMap<String, Vec<Value>>,
}

/// What the page reports after a LIVE search: TikTok's answer, untouched.
#[derive(Debug, Deserialize)]
struct SearchAnswer {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    body: String,
}

/// Searches asked for again within this long get the same answer: the picker
/// asks as you type, and a query typed twice is not news.
const SEARCH_TTL: Duration = Duration::from_secs(60);
/// Results asked of TikTok per search: its own page size. The answer comes
/// back through the window's URL, whole, and each room carries its stream
/// data, so a bigger page would approach what a URL can hold.
const SEARCH_COUNT: u32 = 12;

static SEARCHES: Lazy<Mutex<HashMap<String, (Instant, Vec<ProviderStream>)>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

static CACHE: Lazy<Mutex<HashMap<String, (Instant, Vec<ProviderStream>)>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
/// Categories with a background refresh already under way, so a burst of
/// visits queues one refresh, not one each.
static REFRESHING: Lazy<Mutex<HashSet<String>>> = Lazy::new(|| Mutex::new(HashSet::new()));
static SIGNED: Lazy<Mutex<Signed>> = Lazy::new(|| Mutex::new(load_signed().unwrap_or_default()));
/// One signing at a time, so categories needing a signature together cost one
/// visit to the page rather than one each.
static SIGNING: Lazy<tokio::sync::Mutex<()>> = Lazy::new(|| tokio::sync::Mutex::new(()));

/// "Top live": several categories merged, one row per creator, busiest first.
pub async fn top_live() -> Result<Vec<ProviderStream>> {
    let answers = futures::future::join_all(TOP_LIVE.iter().map(|kw| keyword(kw))).await;
    let mut merged: Vec<ProviderStream> = Vec::new();
    let mut last_err = None;
    for answer in answers {
        match answer {
            Ok(rows) => {
                for row in rows {
                    match merged.iter_mut().find(|m| m.user_login == row.user_login) {
                        Some(existing) if row.viewer_count > existing.viewer_count => *existing = row,
                        Some(_) => {}
                        None => merged.push(row),
                    }
                }
            }
            Err(e) => last_err = Some(e),
        }
    }
    // One category failing is not the directory failing; all of them is.
    if merged.is_empty() {
        if let Some(e) = last_err {
            return Err(e);
        }
    }
    merged.sort_by(|a, b| b.viewer_count.cmp(&a.viewer_count));
    Ok(merged)
}

/// One category, as TikTok names it ("Gaming", "Music", ...).
pub async fn keyword(kw: &str) -> Result<Vec<ProviderStream>> {
    let key = kw.trim().to_string();
    if key.is_empty() || key.len() > 64 {
        return Err(anyhow!("not a TikTok category"));
    }
    let cached = CACHE.lock().ok().and_then(|c| c.get(&key).cloned());
    if let Some((at, rows)) = cached {
        let age = at.elapsed();
        if age < CACHE_TTL {
            return Ok(rows);
        }
        if age < STALE_OK {
            refresh_behind(key);
            return Ok(rows);
        }
    }
    fetch_keyword(&key).await
}

/// Refresh one category without anyone waiting on it.
fn refresh_behind(key: String) {
    let first = REFRESHING.lock().map(|mut r| r.insert(key.clone())).unwrap_or(false);
    if !first {
        return;
    }
    tauri::async_runtime::spawn(async move {
        if let Err(e) = fetch_keyword(&key).await {
            log::debug!("[TikTokFeed] background refresh of {} failed: {}", key, e);
        }
        if let Ok(mut r) = REFRESHING.lock() {
            r.remove(&key);
        }
    });
}

/// Fetch one category directly, signing first only when there is no signed
/// request for it or TikTok has stopped accepting the one there is.
async fn fetch_keyword(key: &str) -> Result<Vec<ProviderStream>> {
    if let Some((ua, url)) = signed_for(key) {
        match replay(&ua, &url).await {
            Ok(rows) => return Ok(remember(key, rows)),
            Err(Replay::Refused(why)) => {
                let age = forget_signed(key)
                    .map(|secs| format!("{} min", secs / 60))
                    .unwrap_or_else(|| "unknown".into());
                log::info!(
                    "[TikTokFeed] {}: its signed request is no longer accepted ({}, signed {} ago); signing afresh",
                    key,
                    why,
                    age
                );
            }
            // The network, not the signature: signing again would not help.
            Err(Replay::Failed(e)) => return Err(e),
        }
    }
    sign_missing(key).await?;
    let err = match signed_for(key) {
        Some((ua, url)) => match replay(&ua, &url).await {
            Ok(rows) => return Ok(remember(key, rows)),
            Err(Replay::Refused(why)) => anyhow!("TikTok refused its own page's request: {}", why),
            Err(Replay::Failed(e)) => e,
        },
        None => anyhow!("TikTok's page did not sign a request for {}", key),
    };
    // The page fetched this category to sign it, so there is a fresh answer
    // even when there is no signed request to use. If TikTok ever stops
    // accepting signed requests from outside its page, the directory degrades
    // to reading through the page on every refresh, rather than to an error.
    if let Some(rows) = fresh(key) {
        log::info!("[TikTokFeed] {}: the direct fetch failed right after signing ({}); showing the page's own answer", key, err);
        return Ok(rows);
    }
    Err(err)
}

/// Cached rows young enough to answer with as they are.
fn fresh(key: &str) -> Option<Vec<ProviderStream>> {
    let cache = CACHE.lock().ok()?;
    let (at, rows) = cache.get(key)?;
    (at.elapsed() < CACHE_TTL).then(|| rows.clone())
}

fn remember(key: &str, rows: Vec<ProviderStream>) -> Vec<ProviderStream> {
    if let Ok(mut cache) = CACHE.lock() {
        cache.insert(key.to_string(), (Instant::now(), rows.clone()));
    }
    rows
}

/// Why a direct fetch failed, which decides whether the signature is at fault.
enum Replay {
    /// TikTok answered and would not serve it: sign afresh.
    Refused(String),
    /// Nothing TikTok said about the request: the network, or a server error.
    Failed(anyhow::Error),
}

/// One direct fetch of a signed feed request.
async fn replay(ua: &str, url: &str) -> std::result::Result<Vec<ProviderStream>, Replay> {
    if !feed_url_ok(url) || !ua_ok(ua) {
        return Err(Replay::Refused("not a replayable feed request".into()));
    }
    let res = crate::services::http::client()
        .get(url)
        .header("User-Agent", ua)
        .header("Referer", "https://www.tiktok.com/")
        .timeout(REPLAY_TIMEOUT)
        .send()
        .await
        .map_err(|e| Replay::Failed(anyhow!("TikTok's live directory did not answer: {e}")))?;
    let status = res.status();
    if status.is_server_error() {
        return Err(Replay::Failed(anyhow!("TikTok's live directory answered HTTP {}", status.as_u16())));
    }
    if !status.is_success() {
        return Err(Replay::Refused(format!("HTTP {}", status.as_u16())));
    }
    let body = res
        .bytes()
        .await
        .map_err(|e| Replay::Failed(anyhow!("TikTok's live directory stopped answering: {e}")))?;
    read_feed(&body).map_err(Replay::Refused)
}

/// Accept a feed body or say why it is a refusal.
pub(crate) fn read_feed(body: &[u8]) -> std::result::Result<Vec<ProviderStream>, String> {
    // TikTok's quiet refusal is an empty 200.
    if body.is_empty() {
        return Err("an empty answer".into());
    }
    let json: Value = serde_json::from_slice(body).map_err(|_| "not a feed".to_string())?;
    match json.get("status_code").and_then(Value::as_i64) {
        Some(0) => Ok(rows_from_feed(&json)),
        other => Err(format!("feed status {:?}", other)),
    }
}

/// The feed's rooms as browse rows.
fn rows_from_feed(json: &Value) -> Vec<ProviderStream> {
    let Some(entries) = json.get("data").and_then(Value::as_array) else {
        return Vec::new();
    };
    entries.iter().filter_map(|entry| room_row(entry.get("data")?)).collect()
}

/// Accept a LIVE search answer or say why it is a refusal. Each result carries
/// its room as a JSON string, in the same shape the feed's rooms have.
fn read_search(body: &str) -> std::result::Result<Vec<ProviderStream>, String> {
    if body.is_empty() {
        return Err("an empty answer".into());
    }
    let json: Value = serde_json::from_str(body).map_err(|_| "not a search answer".to_string())?;
    match json.get("status_code").and_then(Value::as_i64) {
        Some(0) => {}
        other => return Err(format!("search status {:?}", other)),
    }
    let Some(entries) = json.get("data").and_then(Value::as_array) else {
        // Nothing matched.
        return Ok(Vec::new());
    };
    Ok(entries
        .iter()
        .filter_map(|entry| entry.pointer("/live_info/raw_data").and_then(Value::as_str))
        .filter_map(|raw| serde_json::from_str::<Value>(raw).ok())
        .filter_map(|room| room_row(&room))
        .collect())
}

/// One room as a browse row, or `None` for one that is not live or names no
/// creator. Its stream data is recorded on the way past, so a click on its
/// card plays without asking about the room.
fn room_row(room: &Value) -> Option<ProviderStream> {
    let text = |v: Option<&Value>| v.and_then(Value::as_str).map(str::to_string);
    let first_url = |v: Option<&Value>| {
        v.and_then(|i| i.get("url_list"))
            .and_then(|l| l.get(0))
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    // 2 is live. The feed occasionally carries a room that has just ended.
    if room.get("status").and_then(Value::as_i64) != Some(2) {
        return None;
    }
    let owner = room.get("owner")?;
    let row = FeedRow {
        room_id: text(room.get("id_str")).unwrap_or_default(),
        handle: text(owner.get("display_id"))?,
        nickname: text(owner.get("nickname")),
        user_id: text(owner.get("id_str")),
        title: text(room.get("title")),
        viewers: room.get("user_count").and_then(Value::as_u64),
        cover: first_url(room.get("cover")),
        // A frame of the stream itself, carried under `urls` rather than the
        // `url_list` every other image uses.
        snapshot: room
            .get("stream_snapshot")
            .and_then(|s| s.get("urls"))
            .and_then(|u| u.get(0))
            .and_then(Value::as_str)
            .map(str::to_string),
        avatar: first_url(owner.get("avatar_thumb")),
        category: room.get("hashtag").and_then(|h| h.get("title")).and_then(Value::as_str).map(str::to_string),
        started: room
            .get("start_time")
            .or_else(|| room.get("create_time"))
            .and_then(Value::as_i64),
    };
    let stream = row_to_stream(row)?;
    if let Some(tiers) = tiktok_live::http::api::stream_url_of_room(room) {
        crate::services::providers::tiktok_media::note_renditions(&stream.id, tiers.renditions);
    }
    Some(stream)
}

/// Live creators matching `query` by name or handle, from TikTok's own LIVE
/// search, in TikTok's order. It answers signed out; the directory page signs
/// the request, and each query is a new one, so it goes through the page.
pub async fn search(query: &str) -> Result<Vec<ProviderStream>> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let key = query.to_lowercase();
    if let Some(rows) = SEARCHES
        .lock()
        .ok()
        .and_then(|s| s.get(&key).filter(|(at, _)| at.elapsed() < SEARCH_TTL).map(|(_, r)| r.clone()))
    {
        return Ok(rows);
    }
    let answer = search_page(query).await?;
    if !answer.ok {
        return Err(anyhow!(
            "TikTok's search did not answer: {}",
            answer.error.unwrap_or_else(|| "no reason given".into())
        ));
    }
    let rows = read_search(&answer.body).map_err(|why| anyhow!("TikTok's search refused: {why}"))?;
    if let Ok(mut s) = SEARCHES.lock() {
        s.retain(|_, (at, _)| at.elapsed() < SEARCH_TTL);
        s.insert(key, (Instant::now(), rows.clone()));
    }
    Ok(rows)
}

/// Validate one room and shape it as a browse row.
///
/// The row came from a remote service, so nothing in it is taken on trust: the
/// handle must look like a handle, every url must be HTTPS, and every string is
/// bounded. A row that fails is dropped rather than repaired.
fn row_to_stream(r: FeedRow) -> Option<ProviderStream> {
    let handle = r.handle.trim().trim_start_matches('@').to_lowercase();
    let handle_ok = !handle.is_empty()
        && handle.len() <= 64
        && handle
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.');
    if !handle_ok {
        return None;
    }
    let https = |u: Option<String>| u.filter(|s| s.starts_with("https://") && s.len() < 4096);
    let clip = |s: Option<String>, n: usize| {
        s.map(|v| v.trim().chars().take(n).collect::<String>())
            .filter(|v| !v.is_empty())
    };
    let digits = |s: String| s.chars().filter(|c| c.is_ascii_digit()).collect::<String>();

    let room_id = digits(r.room_id);
    if !room_id.is_empty() {
        // The room is live right now and this is its id, so a click on the card
        // can skip the profile lookup that would otherwise find it.
        crate::services::providers::tiktok_media::note_live_room(&handle, &room_id);
    }

    Some(ProviderStream {
        provider: "tiktok".to_string(),
        key: make_key("tiktok", &handle),
        id: room_id,
        user_id: digits(r.user_id.unwrap_or_default()),
        user_login: handle.clone(),
        user_name: clip(r.nickname, 80).unwrap_or_else(|| handle.clone()),
        title: clip(r.title, 200).unwrap_or_default(),
        viewer_count: r.viewers.unwrap_or(0).min(u32::MAX as u64) as u32,
        game_id: String::new(),
        // TikTok's own LIVE category for the room, as it labels it.
        game_name: clip(r.category, 60).unwrap_or_default(),
        category_thumbnail: None,
        // A frame of the stream when TikTok has one, else the cover, which for
        // most rooms is the creator's avatar.
        thumbnail_url: https(r.snapshot).or_else(|| https(r.cover)).unwrap_or_default(),
        started_at: r
            .started
            .filter(|s| *s > 0)
            .and_then(|s| chrono::DateTime::from_timestamp(s, 0))
            .map(|d| d.to_rfc3339())
            .unwrap_or_default(),
        profile_image_url: https(r.avatar),
        is_live: true,
        watch_url: format!("https://www.tiktok.com/@{}/live", handle),
        tags: None,
    })
}

// --- Signed requests --------------------------------------------------------

/// Only TikTok's webcast feed, over HTTPS, is ever fetched with a URL taken
/// from the page. The page is TikTok's, not ours.
pub(crate) fn feed_url_ok(url: &str) -> bool {
    let Ok(u) = reqwest::Url::parse(url) else {
        return false;
    };
    let host_ok = u
        .host_str()
        .map(|h| h == "webcast.tiktok.com" || (h.starts_with("webcast.") && h.ends_with(".tiktok.com")))
        .unwrap_or(false);
    u.scheme() == "https" && host_ok && u.path() == "/webcast/feed/" && url.len() <= 8192
}

pub(crate) fn ua_ok(ua: &str) -> bool {
    !ua.is_empty() && ua.len() <= 512 && ua.chars().all(|c| c.is_ascii_graphic() || c == ' ')
}

fn signed_path() -> Option<std::path::PathBuf> {
    crate::services::twitch_service::get_app_data_dir()
        .ok()
        .map(|d| d.join("tiktok_feed_signed.json"))
}

fn load_signed() -> Option<Signed> {
    let raw = std::fs::read(signed_path()?).ok()?;
    let mut s: Signed = serde_json::from_slice(&raw).ok()?;
    if !ua_ok(&s.ua) {
        return None;
    }
    s.urls.retain(|_, u| feed_url_ok(u));
    Some(s)
}

fn save_signed(s: &Signed) {
    if let (Some(path), Ok(json)) = (signed_path(), serde_json::to_vec(s)) {
        let _ = std::fs::write(path, json);
    }
}

fn signed_for(key: &str) -> Option<(String, String)> {
    let s = SIGNED.lock().ok()?;
    s.urls.get(key).map(|u| (s.ua.clone(), u.clone()))
}

/// Drop a refused request, answering how long ago it was signed.
fn forget_signed(key: &str) -> Option<u64> {
    let mut s = SIGNED.lock().ok()?;
    let signed_at = s.at.remove(key);
    if s.urls.remove(key).is_some() {
        save_signed(&s);
    }
    signed_at.map(|t| unix_now().saturating_sub(t))
}

pub(crate) fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Fold a signing into what is kept. Signatures made under another user agent
/// cannot be sent under this one, so an agent change starts the set over.
fn merge_signed(current: &mut Signed, answer: SignAnswer, wanted: &[String], now: u64) -> Result<()> {
    if !answer.ok {
        return Err(anyhow!(
            "TikTok's page could not sign its feed request: {}",
            answer.error.unwrap_or_else(|| "no reason given".into())
        ));
    }
    if !ua_ok(&answer.ua) {
        return Err(anyhow!("TikTok's page reported an unusable user agent"));
    }
    if current.ua != answer.ua {
        current.urls.clear();
        current.at.clear();
        current.ua = answer.ua;
    }
    for (kw, url) in answer.signed {
        if wanted.contains(&kw) && feed_url_ok(&url) {
            current.at.insert(kw.clone(), now);
            current.urls.insert(kw, url);
        }
    }
    Ok(())
}

/// Have the page sign `key`, and every other Top live category that lacks a
/// request, in one visit.
async fn sign_missing(key: &str) -> Result<()> {
    let _one = SIGNING.lock().await;
    // Another caller may have signed it, or fetched it, while this one waited.
    if signed_for(key).is_some() || fresh(key).is_some() {
        return Ok(());
    }
    let mut wanted = vec![key.to_string()];
    for kw in TOP_LIVE {
        if kw != key && signed_for(kw).is_none() {
            wanted.push(kw.to_string());
        }
    }
    let mut answer = sign(&wanted).await?;
    let page_rows = std::mem::take(&mut answer.rows);
    {
        let mut s = SIGNED.lock().map_err(|_| anyhow!("poisoned"))?;
        merge_signed(&mut s, answer, &wanted, unix_now())?;
        save_signed(&s);
    }
    // The page's answer is as fresh as any, so it serves until the direct
    // fetch replaces it with the full one.
    for (kw, rows) in page_rows {
        if wanted.contains(&kw) {
            remember(&kw, rows_from_page(rows));
        }
    }
    Ok(())
}

/// The page's trimmed rows as browse rows, each validated like any other.
fn rows_from_page(rows: Vec<Value>) -> Vec<ProviderStream> {
    rows.into_iter()
        .filter_map(|v| serde_json::from_value::<FeedRow>(v).ok())
        .filter_map(row_to_stream)
        .collect()
}

/// Pull `SNFEED=<id>:<json>` for request `id` out of the window's fragment.
fn read_answer<T: serde::de::DeserializeOwned>(fragment: Option<&str>, id: u64) -> Option<Result<T>> {
    let rest = fragment?.strip_prefix("SNFEED=")?;
    let (got, payload) = rest.split_once(':')?;
    if got.parse::<u64>().ok()? != id {
        // A previous request's answer still sitting in the fragment.
        return None;
    }
    let json = match urlencoding::decode(payload) {
        Ok(s) => s.into_owned(),
        Err(e) => return Some(Err(anyhow!("undecodable page answer: {e}"))),
    };
    Some(serde_json::from_str::<T>(&json).map_err(|e| anyhow!("unreadable page answer: {e}")))
}

#[cfg(desktop)]
pub(crate) mod window {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use tauri::Manager;

    static SEQ: AtomicU64 = AtomicU64::new(1);

    /// A hidden tiktok.com page, opened on first use, asked one question at a
    /// time through its URL fragment, and destroyed once nothing has asked for a
    /// while. The injected script answers each question in the fragment.
    pub(crate) struct HiddenPage {
        label: &'static str,
        page: &'static str,
        profile: fn() -> std::path::PathBuf,
        idle_close: Duration,
        /// What the page is for, in errors and the log.
        what: &'static str,
        /// Its own store on macOS, set by `own_store`.
        store: Option<[u8; 16]>,
        /// One request at a time: one window, one fragment to answer in.
        flight: tokio::sync::Mutex<()>,
        last_used: Mutex<Option<Instant>>,
        closer_running: AtomicBool,
    }

    impl HiddenPage {
        pub(crate) fn new(
            label: &'static str,
            page: &'static str,
            profile: fn() -> std::path::PathBuf,
            idle_close: Duration,
            what: &'static str,
        ) -> Self {
            HiddenPage {
                label,
                page,
                profile,
                idle_close,
                what,
                store: None,
                flight: tokio::sync::Mutex::new(()),
                last_used: Mutex::new(None),
                closer_running: AtomicBool::new(false),
            }
        }

        /// A store of its own on macOS as well, for a page that holds no
        /// account (see `platform::webview_store`). Without it the page shares
        /// the default store there, which is right for one that runs as the
        /// signed-in account.
        pub(crate) fn own_store(mut self, name: &str) -> Self {
            self.store = Some(crate::platform::webview_store::own_store(name));
            self
        }

        fn touch(&self) {
            if let Ok(mut t) = self.last_used.lock() {
                *t = Some(Instant::now());
            }
        }

        /// Close the window once nothing has asked for a while.
        fn arm_closer(&'static self) {
            if self.closer_running.swap(true, Ordering::SeqCst) {
                return;
            }
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    let idle = self
                        .last_used
                        .lock()
                        .ok()
                        .and_then(|t| *t)
                        .map(|t| t.elapsed() >= self.idle_close)
                        .unwrap_or(true);
                    if !idle {
                        continue;
                    }
                    // Never under a request that is still waiting for its answer.
                    let Ok(_quiet) = self.flight.try_lock() else { continue };
                    if let Some(app) = crate::services::providers::app_handle() {
                        if let Some(w) = app.get_webview_window(self.label) {
                            // `destroy`, not `close`: a close is a request the page
                            // can defer, and this page is tiktok.com.
                            let _ = w.destroy();
                            log::info!("[TikTok] {} page released", self.what);
                        }
                    }
                    self.closer_running.store(false, Ordering::SeqCst);
                    return;
                }
            });
        }

        async fn open(&self, app: &tauri::AppHandle) -> Result<tauri::WebviewWindow> {
            use tauri::{WebviewUrl, WebviewWindowBuilder};
            let url = self.page.parse().map_err(|e| anyhow!("bad page url: {e}"))?;
            let mut builder = WebviewWindowBuilder::new(app, self.label, WebviewUrl::External(url))
                .data_directory((self.profile)())
                .initialization_script(SCRIPT)
                .visible(false)
                .skip_taskbar(true)
                .focused(false)
                // Real dimensions although it is never shown: the page requests its
                // feed only after it lays out, and a tiny viewport is also the
                // loudest possible automation signal to a bot-defense script.
                .inner_size(1280.0, 800.0);
            if let Some(id) = self.store {
                builder = builder.data_store_identifier(id);
            }
            let win = builder
                .build()
                .map_err(|e| anyhow!("could not open the page for {}: {e}", self.what))?;
            // `eval` against a window still on about:blank would run in a document
            // that is about to be replaced, so wait for the page itself.
            let started = Instant::now();
            while started.elapsed() < Duration::from_secs(15) {
                if let Ok(u) = win.url() {
                    if u.host_str().map(|h| h.ends_with("tiktok.com")).unwrap_or(false) {
                        return Ok(win);
                    }
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            let _ = win.destroy();
            Err(anyhow!("the page for {} never loaded", self.what))
        }

        /// Destroy the window now, if it is open.
        pub(crate) fn close(&self) {
            if let Some(app) = crate::services::providers::app_handle() {
                if let Some(w) = app.get_webview_window(self.label) {
                    let _ = w.destroy();
                }
            }
        }

        /// Ask the page one question: `call(id)` is the script to run, whose
        /// handler answers in the fragment as `SNFEED=<id>:<json>`.
        pub(crate) async fn ask<T: serde::de::DeserializeOwned>(
            &'static self,
            call: impl FnOnce(u64) -> String,
            timeout: Duration,
        ) -> Result<T> {
            let app = crate::services::providers::app_handle()
                .ok_or_else(|| anyhow!("app handle not available for {}", self.what))?;
            let _one = self.flight.lock().await;
            let win = match app.get_webview_window(self.label) {
                Some(w) => w,
                None => self.open(&app).await?,
            };
            self.touch();
            self.arm_closer();

            let id = SEQ.fetch_add(1, Ordering::Relaxed);
            win.eval(&call(id))
                .map_err(|e| anyhow!("could not ask the page for {}: {e}", self.what))?;
            let started = Instant::now();
            while started.elapsed() < timeout {
                tokio::time::sleep(Duration::from_millis(150)).await;
                let fragment = win.url().ok().and_then(|u| u.fragment().map(|f| f.to_string()));
                if let Some(answer) = read_answer(fragment.as_deref(), id) {
                    self.touch();
                    return answer;
                }
            }
            Err(anyhow!("{} did not answer in time", self.what))
        }
    }

    /// The script that calls page handler `handler` with request `id` and
    /// `args` (already JSON). The handler is defined by the injected script at
    /// document start; if a navigation is still settling, this retries briefly
    /// inside the page rather than answering "not ready" for a moment's delay.
    pub(crate) fn handler_call(id: u64, handler: &str, args: &str) -> String {
        format!(
            "location.hash = ''; (function go(n) {{ \
               if (window.{handler}) return window.{handler}({id}, {args}); \
               if (n > 60) {{ location.hash = 'SNFEED={id}:' + encodeURIComponent(JSON.stringify({{ ok: false, error: 'the page is not ready' }})); return; }} \
               setTimeout(function () {{ go(n + 1); }}, 250); \
             }})(0);"
        )
    }

    fn directory_profile() -> std::path::PathBuf {
        let base = crate::services::twitch_service::get_app_data_dir()
            .unwrap_or_else(|_| std::env::temp_dir());
        let dir = base.join("platform_web_profiles").join("tiktok-feed");
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    /// The signed-out directory page: signs feed requests and runs searches.
    static DIRECTORY: Lazy<HiddenPage> = Lazy::new(|| {
        HiddenPage::new(LABEL, FEED_PAGE, directory_profile, IDLE_CLOSE, "TikTok's live directory")
            .own_store("tiktok-feed")
    });

    pub(super) async fn sign(keywords: &[String]) -> Result<SignAnswer> {
        let kws = serde_json::to_string(keywords)?;
        DIRECTORY
            .ask(|id| handler_call(id, "__snTikTokSign", &kws), REQUEST_TIMEOUT)
            .await
    }

    /// Have the page run one LIVE search and hand back TikTok's answer.
    pub(super) async fn search(query: &str, count: u32) -> Result<SearchAnswer> {
        let args = format!("{}, {count}", serde_json::to_string(query)?);
        DIRECTORY
            .ask(|id| handler_call(id, "__snTikTokSearch", &args), REQUEST_TIMEOUT)
            .await
    }

    /// Open `page` in a hidden window on `profile`, take the page's own signed
    /// request for feed `channel_id`, and close the window again. Answers the
    /// user agent it was signed under and the URL.
    ///
    /// For a feed whose answer depends on the profile's session (Following):
    /// the page signs it with that session's own device parameters, so the
    /// replay matches the cookies it is sent with.
    pub(crate) async fn capture_feed(
        label: &str,
        page: &str,
        profile: std::path::PathBuf,
        channel_id: &str,
    ) -> Result<(String, String)> {
        use tauri::{WebviewUrl, WebviewWindowBuilder};
        let app = crate::services::providers::app_handle()
            .ok_or_else(|| anyhow!("app handle not available for TikTok"))?;
        if let Some(stale) = app.get_webview_window(label) {
            let _ = stale.destroy();
        }
        let url = page.parse().map_err(|e| anyhow!("bad page url: {e}"))?;
        let win = WebviewWindowBuilder::new(&app, label, WebviewUrl::External(url))
            .data_directory(profile)
            .initialization_script(SCRIPT)
            .visible(false)
            .skip_taskbar(true)
            .focused(false)
            // The page requests its feeds only once it has laid out.
            .inner_size(1280.0, 800.0)
            .build()
            .map_err(|e| anyhow!("could not open the TikTok page: {e}"))?;
        let result = async {
            let started = Instant::now();
            while !win
                .url()
                .ok()
                .and_then(|u| u.host_str().map(|h| h.ends_with("tiktok.com")))
                .unwrap_or(false)
            {
                if started.elapsed() > Duration::from_secs(15) {
                    return Err(anyhow!("the TikTok page never loaded"));
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            let id = SEQ.fetch_add(1, Ordering::Relaxed);
            let channel = serde_json::to_string(channel_id)?;
            let call = format!(
                "location.hash = ''; (function go(n) {{ \
                   if (window.__snTikTokCapture) return window.__snTikTokCapture({id}, {channel}); \
                   if (n > 60) {{ location.hash = 'SNFEED={id}:' + encodeURIComponent(JSON.stringify({{ ok: false, error: 'the page is not ready' }})); return; }} \
                   setTimeout(function () {{ go(n + 1); }}, 250); \
                 }})(0);"
            );
            win.eval(&call).map_err(|e| anyhow!("could not ask the TikTok page: {e}"))?;
            let started = Instant::now();
            while started.elapsed() < REQUEST_TIMEOUT {
                tokio::time::sleep(Duration::from_millis(200)).await;
                let fragment = win.url().ok().and_then(|u| u.fragment().map(|f| f.to_string()));
                if let Some(answer) = read_answer::<SignAnswer>(fragment.as_deref(), id) {
                    let answer = answer?;
                    if !answer.ok {
                        return Err(anyhow!(
                            "TikTok's page did not make that request: {}",
                            answer.error.unwrap_or_else(|| "no reason given".into())
                        ));
                    }
                    let signed = answer.signed.get("feed").cloned().unwrap_or_default();
                    if !ua_ok(&answer.ua) || !feed_url_ok(&signed) {
                        return Err(anyhow!("TikTok's page reported an unusable request"));
                    }
                    return Ok((answer.ua, signed));
                }
            }
            Err(anyhow!("TikTok's page did not answer in time"))
        }
        .await;
        let _ = win.destroy();
        result
    }
}

#[cfg(desktop)]
async fn sign(keywords: &[String]) -> Result<SignAnswer> {
    window::sign(keywords).await
}

#[cfg(desktop)]
async fn search_page(query: &str) -> Result<SearchAnswer> {
    window::search(query, SEARCH_COUNT).await
}

#[cfg(not(desktop))]
async fn search_page(_query: &str) -> Result<SearchAnswer> {
    Err(anyhow!("TikTok's search is not available on mobile"))
}

/// See `window::capture_feed`.
#[cfg(desktop)]
pub(crate) async fn capture_feed(
    label: &str,
    page: &str,
    profile: std::path::PathBuf,
    channel_id: &str,
) -> Result<(String, String)> {
    window::capture_feed(label, page, profile, channel_id).await
}

#[cfg(not(desktop))]
pub(crate) async fn capture_feed(
    _label: &str,
    _page: &str,
    _profile: std::path::PathBuf,
    _channel_id: &str,
) -> Result<(String, String)> {
    Err(anyhow!("TikTok's signed pages are not available on mobile"))
}

// Signing needs a hidden webview, which is a desktop technique.
#[cfg(not(desktop))]
async fn sign(_keywords: &[String]) -> Result<SignAnswer> {
    Err(anyhow!("TikTok's live directory is not available on mobile"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn row(handle: &str) -> FeedRow {
        FeedRow {
            room_id: "7688472311816637215".into(),
            handle: handle.into(),
            nickname: Some("Irby".into()),
            user_id: Some("7439806080451101727".into()),
            title: Some("WARZONE SOLO ADVENTURES".into()),
            viewers: Some(1131),
            cover: Some("https://p16-common-sign.tiktokcdn-us.com/avatar.webp".into()),
            snapshot: Some("https://p16-common-sign.tiktokcdn-us.com/frame.image".into()),
            avatar: Some("https://p16-common-sign.tiktokcdn-us.com/a.webp".into()),
            category: Some("Gaming".into()),
            started: None,
        }
    }

    fn search_entry(handle: &str, status: i64, viewers: u64) -> serde_json::Value {
        let room = json!({
            "id_str": "7688875167535221535",
            "status": status,
            "title": "Send daily heart",
            "user_count": viewers,
            "start_time": 1_790_200_000,
            "owner": {
                "display_id": handle,
                "nickname": "Yayo",
                "id_str": "7328554429847323694",
                "avatar_thumb": { "url_list": ["https://p16-common-sign.tiktokcdn-us.com/a.webp"] },
            },
            "cover": { "url_list": ["https://p16-common-sign.tiktokcdn-us.com/c.webp"] },
        });
        // TikTok carries each room as a JSON string inside the result.
        json!({ "live_info": { "raw_data": room.to_string(), "room_info": {} } })
    }

    #[test]
    fn a_live_search_answer_becomes_browse_rows() {
        let body = json!({
            "status_code": 0,
            "data": [search_entry("yayoworldwide1", 2, 51), search_entry("ended_now", 4, 3)],
            "has_more": 1,
            "cursor": 12,
        })
        .to_string();
        let rows = read_search(&body).expect("an answer");
        assert_eq!(rows.len(), 1, "a room that has ended is not a live result");
        let row = &rows[0];
        assert_eq!(row.user_login, "yayoworldwide1");
        assert_eq!(row.viewer_count, 51);
        assert!(row.is_live);
        assert_eq!(row.started_at, "2026-09-23T21:46:40+00:00");
        assert_eq!(row.watch_url, "https://www.tiktok.com/@yayoworldwide1/live");
    }

    #[test]
    fn a_search_that_matches_nothing_is_empty_and_a_refusal_is_an_error() {
        assert!(read_search(&json!({ "status_code": 0 }).to_string()).unwrap().is_empty());
        assert!(read_search(&json!({ "status_code": 10101, "data": [] }).to_string()).is_err());
        assert!(read_search("").is_err());
    }

    #[test]
    fn a_reported_room_becomes_a_browse_row() {
        let s = row_to_stream(row("Irby.007")).expect("valid row");
        assert_eq!(s.provider, "tiktok");
        assert_eq!(s.key, "tiktok:irby.007");
        assert_eq!(s.user_login, "irby.007");
        assert_eq!(s.user_name, "Irby");
        assert_eq!(s.viewer_count, 1131);
        assert_eq!(s.game_name, "Gaming");
        assert!(s.is_live);
        assert_eq!(s.watch_url, "https://www.tiktok.com/@irby.007/live");
        // A frame of the stream beats the avatar-as-cover.
        assert!(s.thumbnail_url.ends_with("frame.image"), "{}", s.thumbnail_url);
    }

    #[test]
    fn without_a_frame_the_cover_is_the_thumbnail() {
        let mut r = row("irby.007");
        r.snapshot = None;
        let s = row_to_stream(r).unwrap();
        assert!(s.thumbnail_url.ends_with("avatar.webp"), "{}", s.thumbnail_url);
    }

    #[test]
    fn nothing_from_the_feed_is_taken_on_trust() {
        // A handle that is not a handle is dropped, not repaired.
        assert!(row_to_stream(row("../../etc")).is_none());
        assert!(row_to_stream(row("a b")).is_none());
        assert!(row_to_stream(row("")).is_none());
        // Non-HTTPS urls never reach an <img>.
        let mut r = row("irby.007");
        r.snapshot = Some("javascript:alert(1)".into());
        r.cover = Some("http://insecure.example.com/x.jpg".into());
        r.avatar = Some("data:image/png;base64,AAAA".into());
        let s = row_to_stream(r).unwrap();
        assert_eq!(s.thumbnail_url, "");
        assert_eq!(s.profile_image_url, None);
        // Ids are digits and nothing else.
        let mut r = row("irby.007");
        r.room_id = "12<script>34".into();
        assert_eq!(row_to_stream(r).unwrap().id, "1234");
        // Titles are bounded.
        let mut r = row("irby.007");
        r.title = Some("x".repeat(5000));
        assert_eq!(row_to_stream(r).unwrap().title.chars().count(), 200);
    }

    /// The feed's shape, cut down to the fields read, with the stream data a
    /// room carries.
    fn feed() -> Value {
        let tiers = json!({ "data": { "hd": { "main": {
            "flv": "https://pull-flv-l77-tt02.tiktokcdn.com/game/stream-1_hd.flv",
            "cmaf": "https://pull-f5-tt01.tiktokcdn.com/game/stream-1_hd/index.mpd",
            "hls": "",
            "sdk_params": "{\"VCodec\":\"h264\",\"resolution\":\"720x1280\",\"vbitrate\":1800000}"
        }}}});
        let room = |id: &str, handle: &str, status: i64, viewers: u64| {
            json!({ "data": {
                "id_str": id,
                "status": status,
                "title": "WARZONE",
                "user_count": viewers,
                "cover": { "url_list": ["https://p16-webcast.tiktokcdn-us.com/cover.jpeg"] },
                "stream_snapshot": { "urls": ["https://p16-webcast.tiktokcdn-us.com/frame.jpeg"] },
                "hashtag": { "title": "Gaming" },
                "owner": {
                    "display_id": handle,
                    "nickname": "Someone",
                    "id_str": "123",
                    "avatar_thumb": { "url_list": ["https://p16-common-sign.tiktokcdn-us.com/a.webp"] }
                },
                "stream_url": { "live_core_sdk_data": { "pull_data": { "stream_data": tiers.to_string() } } }
            }})
        };
        json!({
            "status_code": 0,
            "data": [
                room("7688000000000000001", "someone.live", 2, 1500),
                // Ended between the feed being built and read.
                room("7688000000000000002", "ended.room", 4, 90),
                // Not a room at all.
                { "data": null },
            ]
        })
    }

    #[test]
    fn the_feed_becomes_rows_and_seeds_each_rooms_stream_data() {
        let rows = rows_from_feed(&feed());
        assert_eq!(rows.len(), 1, "only the live room: {rows:?}");
        let r = &rows[0];
        assert_eq!(r.user_login, "someone.live");
        assert_eq!(r.id, "7688000000000000001");
        assert_eq!(r.viewer_count, 1500);
        assert_eq!(r.game_name, "Gaming");
        assert!(r.thumbnail_url.ends_with("frame.jpeg"), "{}", r.thumbnail_url);
        assert!(
            crate::services::providers::tiktok_media::has_renditions("7688000000000000001"),
            "a click on this card should not need room info"
        );
        assert!(rows_from_feed(&json!({ "status_code": 0 })).is_empty());
    }

    #[test]
    fn the_pages_own_answer_is_read_row_by_row() {
        let body = r#"{"ok":true,"ua":"UA","signed":{},"rows":{"Gaming":[
            {"roomId":"7688000000000000009","handle":"Someone.Live","nickname":"Someone","viewers":42,
             "snapshot":"https://p16-webcast.tiktokcdn-us.com/frame.jpeg","category":"Gaming"},
            {"roomId":"1","handle":"a b"},
            {"roomId":"2","handle":7},
            "not a row"
        ]}}"#;
        let answer: SignAnswer = serde_json::from_str(body).expect("one bad row must not fail the answer");
        let rows = rows_from_page(answer.rows.into_iter().next().unwrap().1);
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].user_login, "someone.live");
        assert_eq!(rows[0].viewer_count, 42);
        assert!(rows[0].thumbnail_url.ends_with("frame.jpeg"));
    }

    #[test]
    fn a_refusal_is_told_apart_from_a_feed() {
        assert!(read_feed(b"").is_err(), "the quiet refusal");
        assert!(read_feed(b"<html>verify you are human</html>").is_err());
        assert!(read_feed(br#"{"status_code":10011,"data":[]}"#).is_err());
        assert_eq!(read_feed(br#"{"status_code":0,"data":[]}"#).unwrap().len(), 0);
    }

    #[test]
    fn only_tiktoks_feed_is_ever_replayed() {
        assert!(feed_url_ok("https://webcast.us.tiktok.com/webcast/feed/?aid=1988&X-Gnarly=x"));
        assert!(feed_url_ok("https://webcast.tiktok.com/webcast/feed/?aid=1988"));
        assert!(!feed_url_ok("http://webcast.us.tiktok.com/webcast/feed/?aid=1988"), "plain http");
        assert!(!feed_url_ok("https://webcast.us.tiktok.com.evil.example/webcast/feed/"), "lookalike host");
        assert!(!feed_url_ok("https://www.tiktok.com/webcast/feed/"), "not the webcast host");
        assert!(!feed_url_ok("https://webcast.us.tiktok.com/webcast/room/info/"), "another endpoint");
        assert!(!feed_url_ok("https://127.0.0.1/webcast/feed/"));
        assert!(ua_ok("Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/140.0.0.0 Safari/537.36"));
        assert!(!ua_ok(""));
        assert!(!ua_ok("bad\r\nInjected: header"));
    }

    fn answer(ua: &str, signed: &[(&str, &str)]) -> SignAnswer {
        SignAnswer {
            ok: true,
            error: None,
            ua: ua.into(),
            signed: signed.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            rows: HashMap::new(),
        }
    }

    #[test]
    fn a_signing_is_kept_only_for_what_was_asked_and_its_agent() {
        let url = "https://webcast.us.tiktok.com/webcast/feed/?search_keywords=Gaming&X-Gnarly=a";
        let mut s = Signed::default();
        let wanted = vec!["Gaming".to_string()];
        merge_signed(&mut s, answer("UA-1", &[("Gaming", url), ("Music", url), ("Sneaky", "https://evil.example/")]), &wanted, 1_000).unwrap();
        assert_eq!(s.ua, "UA-1");
        assert_eq!(s.urls.keys().collect::<Vec<_>>(), vec!["Gaming"], "only what was asked");
        assert_eq!(s.at.get("Gaming"), Some(&1_000));
        assert_eq!(s.at.len(), 1);

        // A new agent makes every earlier signature unusable.
        let mut s2 = s.clone();
        s2.urls.insert("Popular".into(), url.into());
        s2.at.insert("Popular".into(), 900);
        merge_signed(&mut s2, answer("UA-2", &[("Gaming", url)]), &wanted, 2_000).unwrap();
        assert_eq!(s2.ua, "UA-2");
        assert!(!s2.urls.contains_key("Popular"), "signed under the old agent");
        assert!(!s2.at.contains_key("Popular"));
        assert_eq!(s2.at.get("Gaming"), Some(&2_000));

        // A refusal says why and keeps what there was.
        let refused = SignAnswer {
            ok: false,
            error: Some("the feed answered HTTP 403".into()),
            ua: String::new(),
            signed: HashMap::new(),
            rows: HashMap::new(),
        };
        let e = merge_signed(&mut s, refused, &wanted, 3_000).unwrap_err().to_string();
        assert!(e.contains("403"), "{e}");
        assert!(s.urls.contains_key("Gaming"));
    }

    #[test]
    fn what_is_kept_on_disk_reads_back() {
        let s = Signed {
            ua: "UA".into(),
            urls: [("Gaming".to_string(), "https://webcast.us.tiktok.com/webcast/feed/?x=1".to_string())].into(),
            at: [("Gaming".to_string(), 1_790_000_000)].into(),
        };
        let back: Signed = serde_json::from_slice(&serde_json::to_vec(&s).unwrap()).unwrap();
        assert_eq!(back, s);
        // A set kept before signing times were recorded still reads.
        let older: Signed = serde_json::from_str(r#"{"ua":"UA","urls":{"Gaming":"https://webcast.us.tiktok.com/webcast/feed/?x=1"}}"#).unwrap();
        assert_eq!(older.urls.len(), 1);
        assert!(older.at.is_empty());
    }

    /// Replays requests TikTok's page signed, the way the app does, and times
    /// them. `SN_TIKTOK_SIGNED` names a JSON file shaped like the kept set:
    /// `{"ua": "...", "urls": {"Gaming": "https://webcast.us.tiktok.com/webcast/feed/?..."}}`.
    #[tokio::test]
    #[ignore = "needs requests signed by TikTok's page; set SN_TIKTOK_SIGNED"]
    async fn replays_page_signed_requests_directly() {
        let Ok(path) = std::env::var("SN_TIKTOK_SIGNED") else {
            return;
        };
        let set: Signed = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        for round in ["cold", "warm"] {
            let started = Instant::now();
            let answers = futures::future::join_all(set.urls.iter().map(|(kw, url)| {
                let ua = set.ua.clone();
                async move { (kw.clone(), replay(&ua, url).await) }
            }))
            .await;
            println!("{round}: {} categories together in {} ms", answers.len(), started.elapsed().as_millis());
            for (kw, answer) in answers {
                match answer {
                    Ok(rows) => {
                        let seeded = rows.iter().filter(|r| crate::services::providers::tiktok_media::has_renditions(&r.id)).count();
                        println!("  {kw}: {} rooms, {seeded} with stream data", rows.len());
                        assert!(!rows.is_empty(), "{kw} came back empty");
                    }
                    Err(Replay::Refused(why)) => panic!("{kw}: refused: {why}"),
                    Err(Replay::Failed(e)) => panic!("{kw}: {e}"),
                }
            }
        }
    }

    #[test]
    fn an_answer_is_read_only_for_its_own_request() {
        let body = urlencoding::encode(r#"{"ok":true,"ua":"UA","signed":{"Gaming":"https://webcast.us.tiktok.com/webcast/feed/?a=1"}}"#);
        let frag = format!("SNFEED=42:{body}");
        let got = read_answer::<SignAnswer>(Some(&frag), 42).expect("our answer").expect("parses");
        assert!(got.ok);
        assert_eq!(got.signed.len(), 1);
        // A stale answer left by an earlier request is ignored, not returned.
        assert!(read_answer::<SignAnswer>(Some(&frag), 43).is_none());
        // Unrelated fragments are ignored.
        assert!(read_answer::<SignAnswer>(Some("something-else"), 42).is_none());
        assert!(read_answer::<SignAnswer>(None, 42).is_none());
        // A failure report still parses, so its reason reaches the user.
        let err = urlencoding::encode(r#"{"ok":false,"error":"the page never requested its live feed"}"#);
        let got = read_answer::<SignAnswer>(Some(&format!("SNFEED=7:{err}")), 7).unwrap().unwrap();
        assert!(!got.ok);
        assert_eq!(got.error.as_deref(), Some("the page never requested its live feed"));
    }
}
