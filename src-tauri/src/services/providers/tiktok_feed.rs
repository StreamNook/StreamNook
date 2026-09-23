//! TikTok's live directory, read through one hidden tiktok.com window.
//!
//! TikTok serves no live directory to a plain HTTP client: every discovery
//! endpoint refuses a request it did not sign inside its own page, answering
//! "Url does not match" or an empty body. Its web app signs from a fetch wrapper
//! in the page, so the directory is read FROM a page: one hidden window on the
//! live explore page, asked through `eval`, answering through its URL fragment.
//! Nothing is exposed to that origin; the window never calls into the app.
//!
//! Two properties of that page decide the window's shape:
//!   * It requests its feed only once it has LAID OUT, so the window has real
//!     dimensions although it is never shown. A zero-size one loads nothing.
//!   * The request carries the whole browser environment, and a hand-built
//!     subset is refused, so the page's own request serves as the template.
//!
//! One window, reused, destroyed once browsing stops. A webview costs on the
//! order of a hundred megabytes, and building one per request on the same GPU as
//! a playing video shows up as stutter, so it is neither kept forever nor
//! rebuilt per call.

use crate::models::provider_stream::ProviderStream;
use crate::services::providers::key::make_key;
use anyhow::{anyhow, Result};
use once_cell::sync::Lazy;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[cfg(desktop)]
const LABEL: &str = "tiktok-feed";
#[cfg(desktop)]
const FEED_PAGE: &str = "https://www.tiktok.com/live/explore/Popular";
#[cfg(desktop)]
const SCRIPT: &str = include_str!("tiktok_feed.js");
/// A first request includes loading the page, which is the slow part.
#[cfg(desktop)]
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// How long the window outlives the last request.
#[cfg(desktop)]
const IDLE_CLOSE: Duration = Duration::from_secs(90);
/// Long enough that moving around Home does not re-ask, short enough that
/// viewer counts are not visibly stale.
const CACHE_TTL: Duration = Duration::from_secs(60);

/// What "Top live" is made of. The feed answers about a dozen rooms per keyword
/// and repeated calls mostly repeat, so breadth comes from asking across a few
/// of TikTok's own LIVE categories rather than from paging.
const TOP_LIVE: [&str; 3] = ["Popular", "Gaming", "Music"];

/// One room as the page script reports it. Every field is untrusted input.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FeedRow {
    #[serde(default)]
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
    #[serde(default)]
    #[allow(dead_code)]
    age_restricted: bool,
}

#[derive(Debug, Deserialize)]
struct FeedAnswer {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    rows: Vec<FeedRow>,
}

static CACHE: Lazy<Mutex<HashMap<String, (Instant, Vec<ProviderStream>)>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// "Top live": several categories merged, one row per creator, busiest first.
pub async fn top_live() -> Result<Vec<ProviderStream>> {
    let mut merged: Vec<ProviderStream> = Vec::new();
    let mut last_err = None;
    for kw in TOP_LIVE {
        match keyword(kw).await {
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
    if let Ok(cache) = CACHE.lock() {
        if let Some((at, rows)) = cache.get(&key) {
            if at.elapsed() < CACHE_TTL {
                return Ok(rows.clone());
            }
        }
    }
    let answer = ask(&key).await?;
    if !answer.ok {
        return Err(anyhow!(
            "TikTok's live directory did not answer: {}",
            answer.error.unwrap_or_else(|| "no reason given".into())
        ));
    }
    let rows: Vec<ProviderStream> = answer.rows.into_iter().filter_map(row_to_stream).collect();
    if let Ok(mut cache) = CACHE.lock() {
        cache.insert(key, (Instant::now(), rows.clone()));
    }
    Ok(rows)
}

/// Validate one reported room and shape it as a browse row.
///
/// The row came out of a remote page, so nothing in it is taken on trust: the
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
        started_at: String::new(),
        profile_image_url: https(r.avatar),
        is_live: true,
        watch_url: format!("https://www.tiktok.com/@{}/live", handle),
        tags: None,
    })
}

/// Pull `SNFEED=<id>:<json>` for request `id` out of the window's fragment.
fn read_answer(fragment: Option<&str>, id: u64) -> Option<Result<FeedAnswer>> {
    let rest = fragment?.strip_prefix("SNFEED=")?;
    let (got, payload) = rest.split_once(':')?;
    if got.parse::<u64>().ok()? != id {
        // A previous request's answer still sitting in the fragment.
        return None;
    }
    let json = match urlencoding::decode(payload) {
        Ok(s) => s.into_owned(),
        Err(e) => return Some(Err(anyhow!("undecodable feed answer: {e}"))),
    };
    Some(serde_json::from_str::<FeedAnswer>(&json).map_err(|e| anyhow!("unreadable feed answer: {e}")))
}

#[cfg(desktop)]
mod window {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use tauri::Manager;

    static SEQ: AtomicU64 = AtomicU64::new(1);
    /// One request at a time: one window, one fragment to answer in.
    pub(super) static FLIGHT: Lazy<tokio::sync::Mutex<()>> =
        Lazy::new(|| tokio::sync::Mutex::new(()));
    static LAST_USED: Lazy<Mutex<Option<Instant>>> = Lazy::new(|| Mutex::new(None));
    static CLOSER_RUNNING: AtomicBool = AtomicBool::new(false);

    fn profile_dir() -> std::path::PathBuf {
        let base = crate::services::twitch_service::get_app_data_dir()
            .unwrap_or_else(|_| std::env::temp_dir());
        let dir = base.join("platform_web_profiles").join("tiktok-feed");
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    fn touch() {
        if let Ok(mut t) = LAST_USED.lock() {
            *t = Some(Instant::now());
        }
    }

    /// Close the window once nothing has asked for a while.
    fn arm_closer() {
        if CLOSER_RUNNING.swap(true, Ordering::SeqCst) {
            return;
        }
        tokio::spawn(async {
            loop {
                tokio::time::sleep(Duration::from_secs(15)).await;
                let idle = LAST_USED
                    .lock()
                    .ok()
                    .and_then(|t| *t)
                    .map(|t| t.elapsed() >= IDLE_CLOSE)
                    .unwrap_or(true);
                if !idle {
                    continue;
                }
                // Never under a request that is still waiting for its answer.
                let Ok(_quiet) = FLIGHT.try_lock() else { continue };
                if let Some(app) = crate::services::providers::app_handle() {
                    if let Some(w) = app.get_webview_window(LABEL) {
                        // `destroy`, not `close`: a close is a request the page
                        // can defer, and this page is tiktok.com.
                        let _ = w.destroy();
                        log::info!("[TikTokFeed] browsing idle; window released");
                    }
                }
                CLOSER_RUNNING.store(false, Ordering::SeqCst);
                return;
            }
        });
    }

    async fn open(app: &tauri::AppHandle) -> Result<tauri::WebviewWindow> {
        use tauri::{WebviewUrl, WebviewWindowBuilder};
        let url = FEED_PAGE.parse().map_err(|e| anyhow!("bad feed url: {e}"))?;
        let win = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::External(url))
            .data_directory(profile_dir())
            .initialization_script(SCRIPT)
            .visible(false)
            .skip_taskbar(true)
            .focused(false)
            // Real dimensions although it is never shown: the page requests its
            // feed only after it lays out, and a tiny viewport is also the
            // loudest possible automation signal to a bot-defense script.
            .inner_size(1280.0, 800.0)
            .build()
            .map_err(|e| anyhow!("could not open the TikTok directory window: {e}"))?;
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
        Err(anyhow!("the TikTok directory page never loaded"))
    }

    pub(super) async fn ask(keywords: &str) -> Result<FeedAnswer> {
        let app = crate::services::providers::app_handle()
            .ok_or_else(|| anyhow!("app handle not available for the TikTok directory"))?;
        let _one = FLIGHT.lock().await;
        let win = match app.get_webview_window(LABEL) {
            Some(w) => w,
            None => open(&app).await?,
        };
        touch();
        arm_closer();

        let id = SEQ.fetch_add(1, Ordering::Relaxed);
        let kw = serde_json::to_string(keywords)?;
        // The handler is defined by the injected script at document start. If
        // a navigation is still settling, retry briefly inside the page rather
        // than answering "not ready" for what is only a moment's delay.
        let call = format!(
            "location.hash = ''; (function go(n) {{ \
               if (window.__snTikTokFeed) return window.__snTikTokFeed({id}, {kw}); \
               if (n > 60) {{ location.hash = 'SNFEED={id}:' + encodeURIComponent(JSON.stringify({{ ok: false, error: 'the directory page is not ready' }})); return; }} \
               setTimeout(function () {{ go(n + 1); }}, 250); \
             }})(0);"
        );
        win.eval(&call)
            .map_err(|e| anyhow!("could not ask the TikTok directory window: {e}"))?;

        let started = Instant::now();
        while started.elapsed() < REQUEST_TIMEOUT {
            tokio::time::sleep(Duration::from_millis(200)).await;
            let fragment = win.url().ok().and_then(|u| u.fragment().map(|f| f.to_string()));
            if let Some(answer) = read_answer(fragment.as_deref(), id) {
                touch();
                return answer;
            }
        }
        Err(anyhow!("TikTok's live directory did not answer in time"))
    }
}

#[cfg(desktop)]
async fn ask(keywords: &str) -> Result<FeedAnswer> {
    window::ask(keywords).await
}

// The directory is read through a hidden webview, which is a desktop technique.
#[cfg(not(desktop))]
async fn ask(_keywords: &str) -> Result<FeedAnswer> {
    Err(anyhow!("TikTok's live directory is not available on mobile"))
}

#[cfg(test)]
mod tests {
    use super::*;

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
            age_restricted: false,
        }
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
    fn nothing_from_the_page_is_taken_on_trust() {
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

    #[test]
    fn an_answer_is_read_only_for_its_own_request() {
        let body = urlencoding::encode(r#"{"ok":true,"rows":[{"roomId":"1","handle":"irby.007"}]}"#);
        let frag = format!("SNFEED=42:{body}");
        let got = read_answer(Some(&frag), 42).expect("our answer").expect("parses");
        assert!(got.ok);
        assert_eq!(got.rows.len(), 1);
        // A stale answer left by an earlier request is ignored, not returned.
        assert!(read_answer(Some(&frag), 43).is_none());
        // Unrelated fragments are ignored.
        assert!(read_answer(Some("something-else"), 42).is_none());
        assert!(read_answer(None, 42).is_none());
        // A failure report still parses, so its reason reaches the user.
        let err = urlencoding::encode(r#"{"ok":false,"error":"the feed answered HTTP 403"}"#);
        let got = read_answer(Some(&format!("SNFEED=7:{err}")), 7).unwrap().unwrap();
        assert!(!got.ok);
        assert_eq!(got.error.as_deref(), Some("the feed answered HTTP 403"));
    }
}
