//! YouTube webview-session auth.
//!
//! There is no OAuth app / Data-API path here (its per-project quota caps the whole
//! userbase at ~200 actions/day). Instead we drive the user's own logged-in YouTube
//! web session — exactly how StreamNook already drives authenticated Twitch/Kick
//! sessions, and how masterchat / YouTube.js work: the user signs into YouTube in a
//! webview, we harvest the session cookies from a persistent per-platform WebView2
//! profile, and authenticate private `youtubei/v1` requests (send / moderate) with
//! the `SAPISIDHASH` scheme the web client uses.
//!
//! The harvested cookies are cached + persisted (keyring, obfuscated-file fallback)
//! so a send doesn't re-open a webview every launch; the WebView2 profile also keeps
//! the login itself across restarts.

use crate::services::twitch_service::get_app_data_dir;
use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const ORIGIN: &str = "https://www.youtube.com";
const LOGIN_WINDOW_LABEL: &str = "youtube-login";
// Land on Google's sign-in with youtube.com as the continuation, so the overlay
// opens on the thing the user came to do (Kick's overlay opens kick.com/login for
// the same reason) and the redirect back leaves the youtube.com cookies we harvest.
//
// If Google ever refuses the embedded webview ("this browser or app may not be
// secure"), the fallback is to open plain `ORIGIN` and let the user press Sign in
// there. It is the SAME gate either way, just reached one click later, so it is not
// worth pre-emptively degrading the flow.
const LOGIN_URL: &str = "https://accounts.google.com/ServiceLogin?service=youtube&continue=https%3A%2F%2Fwww.youtube.com%2F";
const HARVEST_WINDOW_LABEL: &str = "youtube-harvest";
// We harvest + send the ENTIRE youtube.com cookie set (not a cherry-picked list),
// exactly what the browser sends. Modern YouTube validates more than the classic
// SAPISID/APISID/HSID/SID/SSID set (e.g. the __Secure-*PSIDTS session-timestamp
// cookies), so sending all of them is what stops the 401 "must be signed in".
const KEYRING_SERVICE: &str = "streamnook_youtube_session";
const KEYRING_USER: &str = "default";
const OBF_KEY: &[u8] = b"StreamNookYouTubeKey2026";

#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
struct YouTubeSession {
    cookies: HashMap<String, String>,
    #[serde(default)]
    account_name: Option<String>,
    /// The signed-in account's picture, harvested from the same account-menu
    /// response as the name so Accounts can show who is connected.
    #[serde(default)]
    account_avatar: Option<String>,
    // True once harvested with the full-cookie-set logic. Sessions persisted before
    // that (serde default false) report disconnected so a frictionless reconnect
    // re-harvests the complete set from the still-signed-in profile.
    #[serde(default)]
    complete: bool,
}

static SESSION: OnceLock<Mutex<Option<YouTubeSession>>> = OnceLock::new();

fn session_cell() -> &'static Mutex<Option<YouTubeSession>> {
    SESSION.get_or_init(|| Mutex::new(load_persisted()))
}

fn session_path() -> Option<PathBuf> {
    get_app_data_dir().ok().map(|d| d.join(".youtube_session"))
}

fn obfuscate(data: &[u8]) -> Vec<u8> {
    data.iter()
        .enumerate()
        .map(|(i, b)| b ^ OBF_KEY[i % OBF_KEY.len()])
        .collect()
}

fn persist(sess: &YouTubeSession) {
    let Ok(json) = serde_json::to_string(sess) else {
        return;
    };
    if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER) {
        let _ = entry.set_password(&json);
    }
    if let Some(p) = session_path() {
        let _ = std::fs::write(p, obfuscate(json.as_bytes()));
    }
}

fn load_persisted() -> Option<YouTubeSession> {
    if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER) {
        if let Ok(json) = entry.get_password() {
            if let Ok(s) = serde_json::from_str::<YouTubeSession>(&json) {
                return Some(s);
            }
        }
    }
    let p = session_path()?;
    let raw = std::fs::read(p).ok()?;
    let json = String::from_utf8(obfuscate(&raw)).ok()?;
    serde_json::from_str(&json).ok()
}

fn clear_persisted() {
    if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER) {
        let _ = entry.delete_credential();
    }
    if let Some(p) = session_path() {
        let _ = std::fs::remove_file(p);
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn sha1_hex(input: &str) -> String {
    use sha1::{Digest, Sha1};
    let mut h = Sha1::new();
    h.update(input.as_bytes());
    h.finalize().iter().map(|b| format!("{:02x}", b)).collect()
}

/// The per-platform WebView2 profile that persists the YouTube login (mirrors the
/// Kick resolver profile layout). Public so the shared login overlay can mount
/// into this cookie jar when it is asked for the `youtube-account` profile.
pub fn youtube_profile_dir() -> PathBuf {
    let base = get_app_data_dir().unwrap_or_else(|_| std::env::temp_dir());
    let dir = base.join("platform_web_profiles").join("youtube");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

// --- Public surface ---------------------------------------------------------

/// Whether we hold a usable YouTube session. Requires SAPISID (the hashed cookie)
/// AND APISID — a session missing APISID (e.g. one harvested before APISID was
/// captured) reports disconnected so a reconnect re-harvests the full set from the
/// still-logged-in profile rather than failing every authenticated request.
pub fn is_connected() -> bool {
    session_cell()
        .lock()
        .ok()
        .and_then(|s| s.clone())
        .map(|s| s.complete && sapisid(&s.cookies).is_some() && s.cookies.contains_key("APISID"))
        .unwrap_or(false)
}

/// The cached connected-account name (None if not yet fetched).
pub fn account_name() -> Option<String> {
    session_cell().lock().ok().and_then(|s| s.clone()).and_then(|s| s.account_name)
}

/// The connected account's name for the Connections UI: cached, else fetched once
/// (and cached/persisted) so an already-connected session gets its name without a
/// reconnect. None when signed out or the fetch fails.
pub async fn account_name_lazy() -> Option<String> {
    if let Some(n) = account_name() {
        return Some(n);
    }
    if !is_connected() {
        return None;
    }
    let name = fetch_account_name().await?;
    if let Ok(mut s) = session_cell().lock() {
        if let Some(sess) = s.as_mut() {
            sess.account_name = Some(name.clone());
        }
    }
    if let Some(sess) = session_cell().lock().ok().and_then(|s| s.clone()) {
        persist(&sess);
    }
    Some(name)
}

/// The headers that authenticate a private `youtubei/v1` request as this user:
/// the Cookie header + the per-request `SAPISIDHASH` Authorization. None when not
/// connected. Recomputed each call (the hash is timestamped).
pub fn auth_headers() -> Option<Vec<(String, String)>> {
    let sess = session_cell().lock().ok()?.clone()?;
    let sapisid = sapisid(&sess.cookies)?;
    let ts = now();
    let digest = sha1_hex(&format!("{} {} {}", ts, sapisid, ORIGIN));
    let cookie = sess
        .cookies
        .iter()
        .map(|(k, v)| format!("{}={};", k, v))
        .collect::<Vec<_>>()
        .join(" ");
    Some(vec![
        ("Cookie".to_string(), cookie),
        ("Authorization".to_string(), format!("SAPISIDHASH {}_{}", ts, digest)),
        ("Origin".to_string(), ORIGIN.to_string()),
        ("X-Origin".to_string(), ORIGIN.to_string()),
        ("X-Goog-AuthUser".to_string(), "0".to_string()),
    ])
}

fn sapisid(cookies: &HashMap<String, String>) -> Option<&String> {
    cookies
        .get("SAPISID")
        .or_else(|| cookies.get("__Secure-3PAPISID"))
        .or_else(|| cookies.get("__Secure-1PAPISID"))
}

/// Sign out: drop the cached/persisted session and wipe the YouTube webview profile
/// so the next connect is a fresh login.
pub fn disconnect() {
    if let Ok(mut s) = session_cell().lock() {
        *s = None;
    }
    clear_persisted();
    let _ = std::fs::remove_dir_all(youtube_profile_dir());
    // Moderation answers were computed for the account that just signed out.
    crate::services::providers::youtube::clear_moderation_cache();
    crate::services::providers::emit_platform_account_changed(&["youtube"]);
}

/// Recover a session that YouTube has stopped accepting, at most once every few
/// minutes.
///
/// YouTube rotates the `__Secure-*PSIDTS` session cookies, so a harvested set goes
/// stale on its own while the WebView2 profile stays signed in. When that happens
/// every authed call keeps returning HTTP 200 with signed-OUT content, so nothing
/// errors and nothing recovers: the app looks connected and quietly does nothing.
///
/// `reharvest` was written for exactly this and was never called from anywhere.
/// This is the guarded entry point for it: callers invoke it when they SEE a
/// signed-out response, and the cooldown stops a failing poll from spawning a
/// webview every sweep.
pub async fn recover_stale_session() -> bool {
    use std::sync::atomic::{AtomicU64, Ordering};
    static LAST_ATTEMPT: AtomicU64 = AtomicU64::new(0);
    const COOLDOWN_SECS: u64 = 300;

    let now_s = now();
    let last = LAST_ATTEMPT.load(Ordering::Relaxed);
    if now_s.saturating_sub(last) < COOLDOWN_SECS {
        return false;
    }
    LAST_ATTEMPT.store(now_s, Ordering::Relaxed);
    log::info!("[YouTube] session looks stale; re-harvesting from the signed-in profile");
    let ok = reharvest().await;
    log::info!("[YouTube] re-harvest {}", if ok { "succeeded" } else { "failed" });
    ok
}

// --- Connect (login webview + cookie harvest) -------------------------------

/// Sign in to YouTube in the app's shared login OVERLAY, the same in-app surface
/// Twitch and Kick use, rather than a popup window. Signing into YouTube should
/// look like signing into either of those.
///
/// The session is a cookie harvest, not OAuth: YouTube's private `youtubei/v1`
/// endpoints authenticate with the site cookies plus a SAPISIDHASH, and Google
/// issues no public API key that reads a user's own subscriptions. So the flow is
/// "let the user sign in normally, then read the jar", and the jar is read from
/// RUST because the cookies that matter are HttpOnly and page script cannot see
/// them (the same reason Kick reads its site session this way).
pub async fn connect() -> Result<()> {
    use tauri::Manager;

    let app = crate::services::providers::app_handle()
        .ok_or_else(|| anyhow!("app handle not available for YouTube login"))?;

    // Hand the overlay the sign-in page. React measures the app body and mounts
    // the webview at that rect; `youtube-account` selects YouTube's own cookie jar.
    crate::commands::twitch::emit_overlay_open_with(
        &app,
        LOGIN_WINDOW_LABEL,
        LOGIN_URL,
        "fullbody",
        Some("youtube-account"),
    )
    .map_err(|e| anyhow!("couldn't open the YouTube sign-in overlay: {}", e))?;

    // The overlay mounts asynchronously (Rust asks, React measures, Rust builds),
    // so wait for the window to exist before addressing it.
    let mut mounted = false;
    for _ in 0..60 {
        if app.get_webview_window(LOGIN_WINDOW_LABEL).is_some() {
            mounted = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    if !mounted {
        crate::commands::twitch::dismiss_login_overlay(&app, LOGIN_WINDOW_LABEL);
        return Err(anyhow!("YouTube sign-in overlay never mounted"));
    }

    // Poll the overlay's cookie jar until the user finishes signing in (SAPISID
    // lands on youtube.com after the redirect back). Cap at ~5 minutes.
    let mut harvested: Option<HashMap<String, String>> = None;
    let mut dismissed = false;
    for _ in 0..200 {
        if app.get_webview_window(LOGIN_WINDOW_LABEL).is_none() {
            // The window going away mid-poll means the user closed the overlay.
            // Reported as a cancellation, not a timeout, so the UI can stay quiet.
            dismissed = true;
            break;
        }
        if let Ok(map) = fetch_cookies_from_window(&app, LOGIN_WINDOW_LABEL, &[]).await {
            // Wait for the full auth set (SAPISID + APISID), not just SAPISID, so we
            // never persist a half-harvested session that 401s every request.
            if sapisid(&map).is_some() && map.contains_key("APISID") {
                harvested = Some(map);
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(1500)).await;
    }

    // Closing goes through the overlay's own dismissal so React tears down its
    // chrome too; destroying the window alone would leave the frame on screen.
    crate::commands::twitch::dismiss_login_overlay(&app, LOGIN_WINDOW_LABEL);

    if dismissed && harvested.is_none() {
        log::info!("[YouTube] sign-in overlay dismissed by the user");
        return Err(anyhow!("Sign-in was cancelled"));
    }
    let cookies = harvested.ok_or_else(|| anyhow!("YouTube sign-in wasn't completed"))?;
    let mut sess = YouTubeSession {
        cookies,
        account_name: None,
        account_avatar: None,
        complete: true,
    };
    // Store first so auth_headers() (used by the account-name fetch) sees the session.
    if let Ok(mut s) = session_cell().lock() {
        *s = Some(sess.clone());
    }
    sess.account_name = fetch_account_name().await;
    persist(&sess);
    if let Ok(mut s) = session_cell().lock() {
        *s = Some(sess);
    }
    crate::services::providers::emit_platform_account_changed(&["youtube"]);
    Ok(())
}

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

/// The connected account's display name, via the authenticated account-menu endpoint
/// (best-effort; None on any failure). The public web key works for authed calls too.
/// Ask YouTube whether the harvested cookie session is still accepted.
///
/// - `Some(true)`  — verified good.
/// - `Some(false)` — YouTube rejected it; the session has been cleared.
/// - `None`        — could not tell. Nothing changed.
///
/// Only an explicit 401 counts as rejection. A 200 that simply doesn't parse into
/// an account name is ambiguous — YouTube reshapes these payloads regularly, and
/// the last thing a renderer change should do is silently sign the user out. Same
/// for network errors. When in doubt, report nothing and leave the session alone.
pub async fn validate_session() -> Option<bool> {
    let headers = auth_headers()?;
    let body = serde_json::json!({
        "context": { "client": { "clientName": "WEB", "clientVersion": "2.20240101.00.00", "hl": "en", "gl": "US" } }
    });
    let url = "https://www.youtube.com/youtubei/v1/account/account_menu?key=AIzaSyAO_FJ2SlqU8Q4STEHLGCilw_Y9_11qcW8&prettyPrint=false";
    let mut req = reqwest::Client::new()
        .post(url)
        .timeout(Duration::from_secs(10))
        .header("User-Agent", UA);
    for (k, v) in headers {
        req = req.header(k, v);
    }
    let resp = req.json(&body).send().await.ok()?;
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        log::info!("[youtube] stored session was rejected (401); signing out");
        disconnect();
        crate::services::providers::emit_platform_session_expired("youtube");
        return Some(false);
    }
    if !resp.status().is_success() {
        log::debug!("[youtube] session check inconclusive: HTTP {}", resp.status());
        return None;
    }
    let v: serde_json::Value = resp.json().await.ok()?;
    // A parsed account name is positive proof.
    if find_account_name(&v).is_some() {
        return Some(true);
    }
    // 200 with signed-OUT content is how a rotted cookie set actually presents
    // (Google rotates `__Secure-*PSIDTS`; nothing ever 401s). Left alone, this is
    // the zombie state: the app shows a connected account whose every authed call
    // quietly does nothing. Try the profile re-harvest; if the profile itself
    // can't produce a signed-in set, the session is genuinely gone — say so.
    log::info!("[youtube] session check: 200 with no account header; attempting re-harvest");
    // The raw `reharvest`, not `recover_stale_session`: that wrapper's cooldown
    // returns false when a recovery ran recently, which here would misread
    // "just recovered" as "dead". This check is already low-frequency.
    if reharvest().await && account_name_lazy().await.is_some() {
        return Some(true);
    }
    log::info!("[youtube] re-harvest could not restore the session; signing out");
    disconnect();
    crate::services::providers::emit_platform_session_expired("youtube");
    Some(false)
}

/// Proactively re-harvest the cookie set on a slow clock, so rotation is absorbed
/// BEFORE anything fails rather than after. The reactive `recover_stale_session`
/// stays as the fast path when a sweep actually sees signed-out content; this
/// daemon just keeps the harvested set young. No-op while disconnected.
pub fn start_reharvest_daemon() {
    tauri::async_runtime::spawn(async {
        let mut tick = tokio::time::interval(Duration::from_secs(24 * 60 * 60));
        // The first interval tick fires immediately; skip it so app start doesn't
        // spawn a harvest webview alongside everything else that is launching.
        tick.tick().await;
        loop {
            tick.tick().await;
            if !is_connected() {
                continue;
            }
            log::info!("[youtube] daily proactive cookie re-harvest");
            let ok = reharvest().await;
            log::info!("[youtube] proactive re-harvest {}", if ok { "succeeded" } else { "failed" });
        }
    });
}

async fn fetch_account_name() -> Option<String> {
    let headers = auth_headers()?;
    let body = serde_json::json!({
        "context": { "client": { "clientName": "WEB", "clientVersion": "2.20240101.00.00", "hl": "en", "gl": "US" } }
    });
    let url = "https://www.youtube.com/youtubei/v1/account/account_menu?key=AIzaSyAO_FJ2SlqU8Q4STEHLGCilw_Y9_11qcW8&prettyPrint=false";
    let mut req = reqwest::Client::new().post(url).header("User-Agent", UA);
    for (k, v) in headers {
        req = req.header(k, v);
    }
    let resp = req.json(&body).send().await.ok()?;
    let v: serde_json::Value = resp.json().await.ok()?;
    // Stash the picture on the way past: it is in this same response, so fetching
    // it separately would be a second authenticated round trip for nothing.
    if let Some(photo) = find_account_photo(&v) {
        if let Ok(mut s) = session_cell().lock() {
            if let Some(sess) = s.as_mut() {
                sess.account_avatar = Some(photo);
            }
        }
    }
    find_account_name(&v)
}

/// The signed-in account's picture, if harvested.
pub fn account_avatar() -> Option<String> {
    session_cell()
        .lock()
        .ok()
        .and_then(|s| s.clone())
        .and_then(|s| s.account_avatar)
}

/// Name + picture, fetching once if either is missing.
///
/// `account_name_lazy` returns a cached NAME without asking, so a session
/// connected before the picture was captured would never backfill it. Both come
/// from the same account-menu response, so a miss on either is worth the one
/// request that fills both.
pub async fn account_identity() -> (Option<String>, Option<String>) {
    if !is_connected() {
        return (None, None);
    }
    if let (Some(name), Some(avatar)) = (account_name(), account_avatar()) {
        return (Some(name), Some(avatar));
    }
    // Populates the picture as a side effect and caches the name.
    let name = account_name_lazy_uncached().await;
    let avatar = account_avatar();
    // The picture and the name come from ONE response, so "name but no picture"
    // is a parsing miss, not a missing request, and is worth saying out loud.
    if avatar.is_none() {
        log::warn!(
            "[YouTube] account identity resolved without a picture (name={:?})",
            name
        );
    }
    (name, avatar)
}

/// `account_name_lazy` without its cache short-circuit, so the account-menu
/// request actually runs and repopulates whatever was missing.
async fn account_name_lazy_uncached() -> Option<String> {
    let name = fetch_account_name().await?;
    if let Ok(mut s) = session_cell().lock() {
        if let Some(sess) = s.as_mut() {
            sess.account_name = Some(name.clone());
        }
    }
    if let Some(sess) = session_cell().lock().ok().and_then(|s| s.clone()) {
        persist(&sess);
    }
    Some(name)
}

/// Recursively pull the account PICTURE out of the same account-menu response.
///
/// Separate walk rather than one that returns a pair, because the two live under
/// the same renderer but YouTube has moved either of them independently before;
/// a miss on one should not cost the other.
fn find_account_photo(v: &serde_json::Value) -> Option<String> {
    // Preferred shape first, then progressively looser fallbacks. YouTube moves
    // surfaces between renderer shapes (the subscriptions feed already caught us
    // out that way, see `youtube_account::channels_in`), and pinning ONE path is
    // exactly what breaks when it does. The name and the picture come from the
    // same response, so a rename here showed up as "the account row has a name
    // but no picture" rather than as an obvious failure.
    if let Some(url) = photo_under_key(v, "accountPhoto") {
        return Some(url);
    }
    // Some builds carry it as `accountPhotoThumbnail` / `avatar` instead.
    for key in ["accountPhotoThumbnail", "avatar", "profilePhoto"] {
        if let Some(url) = photo_under_key(v, key) {
            return Some(url);
        }
    }
    // Last resort: the biggest thumbnail inside the account header itself.
    find_header(v).and_then(|h| largest_thumbnail_url(h))
}

/// The largest thumbnail url under any object stored at `key`, anywhere in the tree.
fn photo_under_key(v: &serde_json::Value, key: &str) -> Option<String> {
    match v {
        serde_json::Value::Object(map) => {
            if let Some(node) = map.get(key) {
                if let Some(url) = largest_thumbnail_url(node) {
                    return Some(url);
                }
            }
            map.values().find_map(|c| photo_under_key(c, key))
        }
        serde_json::Value::Array(arr) => arr.iter().find_map(|c| photo_under_key(c, key)),
        _ => None,
    }
}

/// The account header renderer, wherever it sits.
fn find_header(v: &serde_json::Value) -> Option<&serde_json::Value> {
    match v {
        serde_json::Value::Object(map) => {
            if let Some(h) = map.get("activeAccountHeaderRenderer") {
                return Some(h);
            }
            map.values().find_map(find_header)
        }
        serde_json::Value::Array(arr) => arr.iter().find_map(find_header),
        _ => None,
    }
}

/// Widest entry in any `thumbnails` list beneath this node.
///
/// Width rather than position: "last is largest" holds for most InnerTube
/// thumbnail lists but is a convention, not a guarantee, and picking a 32px
/// avatar for a profile row is a silent quality regression.
fn largest_thumbnail_url(v: &serde_json::Value) -> Option<String> {
    fn walk(v: &serde_json::Value, best: &mut Option<(u64, String)>, depth: usize) {
        if depth > 6 {
            return;
        }
        match v {
            serde_json::Value::Object(map) => {
                if let Some(list) = map.get("thumbnails").and_then(|t| t.as_array()) {
                    for (i, t) in list.iter().enumerate() {
                        let Some(url) = t.get("url").and_then(|u| u.as_str()) else {
                            continue;
                        };
                        // Fall back to index so an unsized list still resolves in
                        // list order rather than being skipped entirely.
                        let w = t
                            .get("width")
                            .and_then(|w| w.as_u64())
                            .unwrap_or(i as u64 + 1);
                        if best.as_ref().is_none_or(|(bw, _)| w > *bw) {
                            *best = Some((w, url.to_string()));
                        }
                    }
                }
                for child in map.values() {
                    walk(child, best, depth + 1);
                }
            }
            serde_json::Value::Array(arr) => {
                for child in arr {
                    walk(child, best, depth + 1);
                }
            }
            _ => {}
        }
    }
    let mut best = None;
    walk(v, &mut best, 0);
    best.map(|(_, url)| url)
}

/// Recursively pull `activeAccountHeaderRenderer.accountName` out of the account-menu
/// response (its exact action index varies).
fn find_account_name(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::Object(map) => {
            if let Some(h) = map.get("activeAccountHeaderRenderer") {
                if let Some(name) = h.pointer("/accountName/simpleText").and_then(|x| x.as_str()) {
                    return Some(name.to_string());
                }
                if let Some(runs) = h.pointer("/accountName/runs").and_then(|r| r.as_array()) {
                    let s: String = runs
                        .iter()
                        .filter_map(|r| r.get("text").and_then(|t| t.as_str()))
                        .collect();
                    if !s.is_empty() {
                        return Some(s);
                    }
                }
            }
            map.values().find_map(find_account_name)
        }
        serde_json::Value::Array(arr) => arr.iter().find_map(find_account_name),
        _ => None,
    }
}

/// Re-read the auth cookies from the (still-logged-in) YouTube profile via a hidden
/// webview — used to recover when a cached session goes stale without making the
/// user sign in again. Returns true if a SAPISID was found.
///
/// WHY THIS LOADS A REAL PAGE, and why the sign-in "expires" without it:
///
/// Google's auth set is not static. Alongside the long-lived SAPISID/APISID/SID
/// cookies it issues `__Secure-1PSIDTS` / `__Secure-3PSIDTS` — session-TIMESTAMP
/// cookies it rotates on the order of an hour. Authenticated InnerTube calls are
/// validated against the current ones, so a stored snapshot goes stale on its own
/// no matter how carefully it was persisted. When it does, YouTube does not answer
/// 401; it answers 200 with signed-out content, which is why a dead session looks
/// exactly like "you follow nobody who is live".
///
/// Only Google can mint fresh timestamps, and only in response to a request to a
/// Google origin. This used to open `about:blank`, which contacts nothing — so it
/// re-read the same expired cookies off disk and reported success. Loading a real
/// youtube.com page lets the server issue new `Set-Cookie` values into the profile
/// first, which is what actually renews the session and is what a browser sitting
/// open does for free.
pub async fn reharvest() -> bool {
    use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

    let Some(app) = crate::services::providers::app_handle() else {
        return false;
    };
    if let Some(existing) = app.get_webview_window(HARVEST_WINDOW_LABEL) {
        let _ = existing.destroy();
    }
    let Ok(url) = tauri::Url::parse(ORIGIN) else {
        return false;
    };
    if WebviewWindowBuilder::new(&app, HARVEST_WINDOW_LABEL, WebviewUrl::External(url))
        .title("")
        .inner_size(1.0, 1.0)
        .visible(false)
        .focused(false)
        .skip_taskbar(true)
        .data_directory(youtube_profile_dir())
        .build()
        .is_err()
    {
        return false;
    }
    // Give the page a moment to actually load and be answered before reading the
    // jar. Harvesting instantly would capture the pre-request cookies — the very
    // staleness this exists to cure — and the old early break at attempt 3 made
    // that near-certain now that a real navigation is involved.
    tokio::time::sleep(Duration::from_millis(1200)).await;

    let mut found = false;
    // ~9s of budget. A cold youtube.com load on a slow link takes seconds, and
    // giving up early costs a re-login the user did not need.
    for _ in 0..30 {
        if let Ok(map) = fetch_cookies_from_window(&app, HARVEST_WINDOW_LABEL, &[]).await {
            if sapisid(&map).is_some() && map.contains_key("APISID") {
                let sess = YouTubeSession {
                    cookies: map,
                    // A re-harvest replaces the cookies, not the identity.
                    account_name: account_name(),
                    account_avatar: account_avatar(),
                    complete: true,
                };
                persist(&sess);
                if let Ok(mut s) = session_cell().lock() {
                    *s = Some(sess);
                }
                found = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    if let Some(window) = app.get_webview_window(HARVEST_WINDOW_LABEL) {
        let _ = window.destroy();
    }
    found
}

/// Cross-platform cookie read, used everywhere except Windows.
///
/// `cookies_for_url` returns HTTP-only and secure cookies, which is exactly the
/// capability the WebView2 COM path was hand-rolled for. Note it only answers
/// for `http`/`https` origins — cookies set by script under `tauri://` are not
/// visible to it, which is fine because every caller passes a real site origin.
#[cfg(not(windows))]
pub(crate) async fn fetch_cookies_for_origin(
    app: &tauri::AppHandle,
    window_label: &str,
    names: &[&str],
    origin: &str,
) -> Result<HashMap<String, String>> {
    use tauri::Manager;

    let webview = app
        .get_webview_window(window_label)
        .ok_or_else(|| anyhow!("webview window '{}' unavailable", window_label))?;

    let url = tauri::Url::parse(origin)
        .map_err(|e| anyhow!("cookie origin '{}' is not a valid URL: {}", origin, e))?;

    let jar = webview
        .cookies_for_url(url)
        .map_err(|e| anyhow!("cookies_for_url({}) failed: {}", origin, e))?;

    let mut found: HashMap<String, String> = HashMap::new();
    for cookie in jar {
        let name = cookie.name().to_string();
        // Empty `names` means "take everything", matching the harvest callers.
        if names.is_empty() || names.iter().any(|wanted| *wanted == name) {
            found.insert(name, cookie.value().to_string());
        }
    }
    Ok(found)
}

// --- WebView2 cookie read (Windows) — mirrors twitch_auth_service ------------

async fn fetch_cookies_from_window(
    app: &tauri::AppHandle,
    window_label: &str,
    names: &[&str],
) -> Result<HashMap<String, String>> {
    fetch_cookies_for_origin(app, window_label, names, ORIGIN).await
}

/// Read cookies straight from a webview's cookie manager.
///
/// This sees **HttpOnly** cookies, which page script never can — which is the
/// whole reason it exists. `origin` selects whose jar to read, so other
/// platforms' sign-in windows can reuse the same path.
///
/// An empty `names` means "every cookie for that origin", which is what the
/// harvest paths pass.
///
/// # Two implementations, on purpose
///
/// Windows keeps the hand-rolled WebView2 `GetCookies` COM path below. It is
/// proven in production and Tauri's own `cookies_for_url` carries a documented
/// **deadlock** on Windows when called from a synchronous command or event
/// handler (wry#583), so there is no upside to swapping it there.
///
/// Everywhere else uses `WebviewWindow::cookies_for_url`, which has been
/// cross-platform since Tauri 2.4.0 and abstracts `WKHTTPCookieStore` on macOS
/// and `WebKitCookieManager` on Linux. Read-only is all this needs, so the
/// cookie-SETTER gap (tauri#11691) does not matter here.
#[cfg(windows)]
pub(crate) async fn fetch_cookies_for_origin(
    app: &tauri::AppHandle,
    window_label: &str,
    names: &[&str],
    origin: &str,
) -> Result<HashMap<String, String>> {
    use std::sync::Arc;
    use tauri::Manager;
    use tokio::sync::oneshot;

    let webview = app
        .get_webview_window(window_label)
        .ok_or_else(|| anyhow!("webview window '{}' unavailable", window_label))?;

    let (tx, rx) = oneshot::channel::<Result<HashMap<String, String>>>();
    let tx_slot: Arc<std::sync::Mutex<Option<oneshot::Sender<_>>>> =
        Arc::new(std::sync::Mutex::new(Some(tx)));
    let tx_for_closure = tx_slot.clone();
    let targets: Vec<String> = names.iter().map(|s| s.to_string()).collect();
    let origin_owned = origin.to_string();

    let dispatched = webview.with_webview(move |platform_webview| {
        let setup = unsafe {
            request_cookies(
                platform_webview,
                tx_for_closure.clone(),
                targets.clone(),
                origin_owned.clone(),
            )
        };
        if let Err(e) = setup {
            if let Some(sender) = tx_for_closure.lock().unwrap().take() {
                let _ = sender.send(Err(anyhow!("WebView2 GetCookies setup failed: {}", e)));
            }
        }
    });
    if let Err(e) = dispatched {
        return Err(anyhow!("with_webview: {}", e));
    }
    rx.await
        .map_err(|_| anyhow!("WebView2 cookie callback dropped"))?
}

#[cfg(windows)]
unsafe fn request_cookies(
    platform_webview: tauri::webview::PlatformWebview,
    tx_slot: std::sync::Arc<std::sync::Mutex<Option<tokio::sync::oneshot::Sender<Result<HashMap<String, String>>>>>>,
    targets: Vec<String>,
    origin: String,
) -> windows::core::Result<()> {
    use webview2_com::GetCookiesCompletedHandler;
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2_2;
    use windows::core::{Interface, HSTRING};

    let controller = platform_webview.controller();
    let core = controller.CoreWebView2()?;
    let core2: ICoreWebView2_2 = core.cast()?;
    let manager = core2.CookieManager()?;
    let uri = HSTRING::from(origin.as_str());

    let handler = GetCookiesCompletedHandler::create(Box::new(move |error_code, cookie_list| {
        let result = extract_cookies(error_code, cookie_list, &targets);
        if let Some(sender) = tx_slot.lock().unwrap().take() {
            let _ = sender.send(result);
        }
        Ok(())
    }));
    manager.GetCookies(&uri, &handler)?;
    Ok(())
}

#[cfg(windows)]
fn extract_cookies(
    completion: windows::core::Result<()>,
    cookie_list: Option<webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2CookieList>,
    targets: &[String],
) -> Result<HashMap<String, String>> {
    use webview2_com::take_pwstr;
    use windows::core::PWSTR;

    completion.map_err(|e| anyhow!("GetCookies: {}", e))?;
    let list = cookie_list.ok_or_else(|| anyhow!("WebView2 returned null cookie list"))?;

    let mut count: u32 = 0;
    unsafe { list.Count(&mut count as *mut u32) }.map_err(|e| anyhow!("CookieList::Count: {}", e))?;

    let mut found: HashMap<String, String> = HashMap::new();
    for i in 0..count {
        let cookie = unsafe { list.GetValueAtIndex(i) }.map_err(|e| anyhow!("CookieList[{}]: {}", i, e))?;
        let mut name_ptr = PWSTR::null();
        unsafe { cookie.Name(&mut name_ptr as *mut PWSTR) }.map_err(|e| anyhow!("cookie.Name: {}", e))?;
        let name = take_pwstr(name_ptr);
        // Empty targets = capture every cookie (the whole browser Cookie set).
        if targets.is_empty() || targets.iter().any(|t| t == &name) {
            let mut value_ptr = PWSTR::null();
            unsafe { cookie.Value(&mut value_ptr as *mut PWSTR) }.map_err(|e| anyhow!("cookie.Value: {}", e))?;
            let value = take_pwstr(value_ptr);
            if !value.is_empty() {
                found.insert(name, value);
            }
        }
    }
    Ok(found)
}

#[cfg(test)]
mod account_photo_tests {
    use super::*;
    use serde_json::json;

    /// The shape we already handled, nested where InnerTube actually puts it.
    #[test]
    fn reads_the_canonical_account_photo() {
        let v = json!({ "actions": [ { "openPopupAction": { "popup": { "multiPageMenuRenderer": {
            "header": { "activeAccountHeaderRenderer": {
                "accountName": { "simpleText": "Brandon" },
                "accountPhoto": { "thumbnails": [
                    { "url": "https://yt3/small.jpg", "width": 48 },
                    { "url": "https://yt3/big.jpg", "width": 176 }
                ] }
            } }
        } } } } ] });
        assert_eq!(
            find_account_photo(&v).as_deref(),
            Some("https://yt3/big.jpg"),
            "should take the WIDEST thumbnail, not the first"
        );
    }

    /// Width wins over position, so an out-of-order list cannot yield a 32px avatar.
    #[test]
    fn prefers_width_over_list_order() {
        let v = json!({ "accountPhoto": { "thumbnails": [
            { "url": "https://yt3/huge.jpg", "width": 800 },
            { "url": "https://yt3/tiny.jpg", "width": 32 }
        ] } });
        assert_eq!(find_account_photo(&v).as_deref(), Some("https://yt3/huge.jpg"));
    }

    /// The point of the rewrite: a renamed key still resolves instead of leaving
    /// the account row with a name and no picture.
    #[test]
    fn falls_back_to_a_renamed_key() {
        let v = json!({ "header": { "activeAccountHeaderRenderer": {
            "accountName": { "simpleText": "Brandon" },
            "avatar": { "thumbnails": [ { "url": "https://yt3/avatar.jpg", "width": 176 } ] }
        } } });
        assert_eq!(find_account_photo(&v).as_deref(), Some("https://yt3/avatar.jpg"));
    }

    /// Unknown key entirely: the header subtree scan still finds a thumbnail.
    #[test]
    fn falls_back_to_any_thumbnail_in_the_header() {
        let v = json!({ "header": { "activeAccountHeaderRenderer": {
            "accountName": { "simpleText": "Brandon" },
            "somethingNew": { "image": { "thumbnails": [ { "url": "https://yt3/new.jpg", "width": 176 } ] } }
        } } });
        assert_eq!(find_account_photo(&v).as_deref(), Some("https://yt3/new.jpg"));
    }

    /// A list with no width fields still resolves, in list order.
    #[test]
    fn unsized_thumbnails_still_resolve() {
        let v = json!({ "accountPhoto": { "thumbnails": [
            { "url": "https://yt3/a.jpg" },
            { "url": "https://yt3/b.jpg" }
        ] } });
        assert_eq!(find_account_photo(&v).as_deref(), Some("https://yt3/b.jpg"));
    }

    #[test]
    fn no_photo_anywhere_is_none() {
        assert_eq!(find_account_photo(&json!({ "unrelated": { "x": 1 } })), None);
    }
}
