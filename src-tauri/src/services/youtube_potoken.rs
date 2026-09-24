//! Mints YouTube Proof-of-Origin tokens in an isolated webview.
//!
//! SABR playback is the only transport that carries 1440p/2160p on many live
//! broadcasts, and it refuses any request whose `StreamerContext` lacks a PO
//! token. The token can only be produced by running BotGuard, an obfuscated
//! integrity check that needs a real browser, so this borrows one.
//!
//! **Isolation is the point.** The bundle runs in a hidden window pointed at an
//! EXTERNAL url, which is what keeps Tauri from injecting its API there: the
//! BotGuard interpreter is third-party code fetched at runtime, and it must
//! never execute anywhere it could reach the app's IPC. The only channel back is
//! the URL fragment, which carries a token and nothing else. Same shape as the
//! 7TV token capture in `commands::seventv_cosmetics`.
//!
//! The bundle is built from `potoken/mint.mjs` by `npm run build:potoken`.

use anyhow::{anyhow, Result};
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Built from `potoken/mint.mjs`. Checked in so a fresh clone compiles before
/// anyone has run npm; see the placeholder's own comment.
const MINT_BUNDLE: &str = include_str!("../../potoken/mint.bundle.js");

/// A real origin is needed so the BotGuard endpoints answer a cross-origin
/// fetch, and the embed shell is far lighter than the full watch page while
/// still being youtube.com.
const MINT_PAGE: &str = "https://www.youtube.com/embed/";

/// The store every page here opens in. Elsewhere they run in the main window's
/// store, which holds no YouTube account. On macOS a webview without its own
/// store shares the default one, where the YouTube sign-in lives too, so
/// without this the pages would run signed in there (see
/// `platform::webview_store`).
fn own_store() -> [u8; 16] {
    crate::platform::webview_store::own_store("youtube-potoken")
}

/// Tokens outlive a single request comfortably. Re-minting costs a webview and
/// several network round trips, so cache well inside the real lifetime rather
/// than paying that per segment fetch.
const TOKEN_TTL: Duration = Duration::from_secs(3 * 3600);

/// BotGuard is not fast. Measured runs land in a few seconds; this is the point
/// at which something is wrong rather than slow.
const MINT_TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Clone)]
struct Cached {
    token: Vec<u8>,
    at: Instant,
}

static CACHE: Lazy<Mutex<HashMap<String, Cached>>> = Lazy::new(|| Mutex::new(HashMap::new()));

fn cached(binding: &str) -> Option<Vec<u8>> {
    let map = CACHE.lock().ok()?;
    let hit = map.get(binding)?;
    (hit.at.elapsed() < TOKEN_TTL).then(|| hit.token.clone())
}

fn store(binding: &str, token: &[u8]) {
    if let Ok(mut map) = CACHE.lock() {
        map.retain(|_, v| v.at.elapsed() < TOKEN_TTL);
        map.insert(
            binding.to_string(),
            Cached { token: token.to_vec(), at: Instant::now() },
        );
    }
}

/// Forget a token, so the next call re-mints. Call this when SABR starts
/// refusing a session that previously worked.
pub fn invalidate(binding: &str) {
    if let Ok(mut map) = CACHE.lock() {
        map.remove(binding);
    }
}

/// Pull the token (or the error) out of the window's URL fragment.
fn read_fragment(url: &str) -> Option<Result<String>> {
    if let Some(rest) = url.split("#POTOKEN=").nth(1) {
        let raw = rest.split('&').next().unwrap_or("");
        return Some(
            urlencoding::decode(raw)
                .map(|s| s.into_owned())
                .map_err(|e| anyhow!("undecodable token: {}", e)),
        );
    }
    if let Some(rest) = url.split("#POTOKEN_ERR=").nth(1) {
        let raw = rest.split('&').next().unwrap_or("");
        let msg = urlencoding::decode(raw)
            .map(|s| s.into_owned())
            .unwrap_or_else(|_| raw.to_string());
        return Some(Err(anyhow!("{}", msg)));
    }
    None
}

/// A PO token bound to `content_binding` (a visitor id, video id or data sync
/// id, depending on what the caller is authenticating).
///
/// Cached: repeated calls for the same binding do not spawn a webview.
pub async fn mint(content_binding: &str) -> Result<Vec<u8>> {
    if let Some(t) = cached(content_binding) {
        return Ok(t);
    }
    let token = mint_uncached(content_binding).await?;
    store(content_binding, &token);
    Ok(token)
}

async fn mint_uncached(content_binding: &str) -> Result<Vec<u8>> {
    use tauri::{WebviewUrl, WebviewWindowBuilder};

    let app = crate::services::providers::app_handle()
        .ok_or_else(|| anyhow!("no app handle; cannot open the minting window"))?;

    let label = format!("yt-potoken-{}", chrono::Utc::now().timestamp_millis());
    // The binding travels in the fragment so the page needs no other input, and
    // the bundle reports back by rewriting that same fragment.
    let url = format!("{}#BIND={}", MINT_PAGE, urlencoding::encode(content_binding));

    let window = WebviewWindowBuilder::new(&app, &label, WebviewUrl::External(url.parse()?))
        .title("YouTube playback")
        .inner_size(480.0, 320.0)
        .visible(false)
        .data_store_identifier(own_store())
        .initialization_script(MINT_BUNDLE)
        .build()
        .map_err(|e| anyhow!("could not open the minting window: {}", e))?;

    let started = Instant::now();
    let mut outcome: Option<Result<String>> = None;
    while started.elapsed() < MINT_TIMEOUT {
        tokio::time::sleep(Duration::from_millis(250)).await;
        if let Ok(u) = window.url() {
            if let Some(r) = read_fragment(u.as_str()) {
                outcome = Some(r);
                break;
            }
        }
    }
    let _ = window.close();

    match outcome {
        Some(Ok(b64)) => {
            let bytes = decode_websafe(&b64)?;
            log::info!(
                "[YouTubePoToken] minted {} bytes for binding '{}' in {:.1}s",
                bytes.len(),
                short(content_binding),
                started.elapsed().as_secs_f64()
            );
            Ok(bytes)
        }
        Some(Err(e)) => Err(anyhow!("BotGuard failed: {}", e)),
        None => Err(anyhow!(
            "BotGuard did not answer within {}s",
            MINT_TIMEOUT.as_secs()
        )),
    }
}

/// The minter hands back a web-safe base64 string; SABR wants the raw bytes.
fn decode_websafe(s: &str) -> Result<Vec<u8>> {
    use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
    use base64::Engine;
    let trimmed = s.trim_end_matches('=');
    URL_SAFE_NO_PAD
        .decode(trimmed)
        .or_else(|_| STANDARD.decode(s))
        .map_err(|e| anyhow!("token was not base64: {}", e))
}

/// Bindings are long opaque strings; logging one whole is noise.
fn short(s: &str) -> String {
    if s.len() <= 16 {
        s.to_string()
    } else {
        format!("{}…{}", &s[..8], &s[s.len() - 4..])
    }
}

// ---------------------------------------------------------------------------
// Stream resolution
// ---------------------------------------------------------------------------

/// Playable per-itag URLs for one video, resolved inside the webview.
///
/// "Playable" is doing real work here. The raw URLs on the watch page are
/// refused: they carry an untransformed `n` nonce, no PO token, and `alr=yes`.
/// These have been through youtubei.js's `decipher`, carry a token, and have
/// `alr=no`, which is the same treatment Invidious applies.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedVideo {
    pub url: String,
    pub itag: u32,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub mime_type: String,
    /// Menu label, e.g. "1440p60".
    pub name: String,
    /// Short-edge height, which is what the label is built from and what a
    /// vertical stream must be ranked by.
    pub quality: u32,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedStreams {
    /// Every rendition above the requested floor, tallest first.
    pub videos: Vec<ResolvedVideo>,
    /// One AAC-in-MP4 track, shared by all of them.
    pub audio_url: String,
    pub audio_itag: u32,
}

/// Signed URLs outlive a viewing session but not by much, and re-resolving costs
/// a webview plus a BotGuard run. Well inside the real expiry.
const STREAMS_TTL: Duration = Duration::from_secs(90 * 60);

static STREAMS: Lazy<Mutex<HashMap<String, (ResolvedStreams, Instant)>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn cached_streams(video_id: &str) -> Option<ResolvedStreams> {
    cached_streams_with_age(video_id).map(|(s, _)| s)
}

/// The cached resolution AND how old it is.
///
/// A caller that needs FRESH urls can then decide for itself, instead of
/// invalidating the entry and forcing a resolve. That distinction is the whole
/// fix for the rotation thrash: see `resolve_streams_fresh`.
fn cached_streams_with_age(video_id: &str) -> Option<(ResolvedStreams, Duration)> {
    let map = STREAMS.lock().ok()?;
    let (s, at) = map.get(video_id)?;
    let age = at.elapsed();
    // GATED urls die of old age in about thirty seconds, so STREAMS_TTL (90
    // minutes) is meaningless for them: handing one back does not save a resolve,
    // it starts playback on a url the origin already refuses. Ungated visionos
    // urls last as long as their own expiry says and keep the long TTL.
    //
    // Mirrors cached_highs in youtube_media, which bounds a gated set by
    // ROTATE_AFTER for the same reason. Caching gated urls for the long TTL means
    // a start well after the last resolve takes an instant upstream 403, and every
    // 1440p selection falls back to 1080p.
    let ttl = if s
        .videos
        .iter()
        .any(|v| crate::services::youtube_dash::is_gated(&v.url))
    {
        crate::services::youtube_dash::ROTATE_AFTER
    } else {
        STREAMS_TTL
    };
    (age < ttl).then(|| (s.clone(), age))
}

fn store_streams(video_id: &str, s: &ResolvedStreams) {
    if let Ok(mut map) = STREAMS.lock() {
        map.retain(|_, (_, at)| at.elapsed() < STREAMS_TTL);
        map.insert(video_id.to_string(), (s.clone(), Instant::now()));
    }
}

fn read_streams_fragment(url: &str) -> Option<Result<ResolvedStreams>> {
    if let Some(rest) = url.split("#YT_STREAMS=").nth(1) {
        let raw = rest.split('&').next().unwrap_or("");
        return Some(decode_streams(raw));
    }
    if let Some(rest) = url.split("#YT_ERR=").nth(1) {
        let raw = rest.split('&').next().unwrap_or("");
        let msg = urlencoding::decode(raw)
            .map(|s| s.into_owned())
            .unwrap_or_else(|_| raw.to_string());
        return Some(Err(anyhow!("{}", msg)));
    }
    None
}

fn decode_streams(b64: &str) -> Result<ResolvedStreams> {
    use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
    use base64::Engine;
    let bytes = STANDARD
        .decode(b64)
        .or_else(|_| URL_SAFE_NO_PAD.decode(b64.trim_end_matches('=')))
        .map_err(|e| anyhow!("stream blob was not base64: {}", e))?;
    serde_json::from_slice(&bytes).map_err(|e| anyhow!("stream blob was not valid json: {}", e))
}

/// Set when the relay sees an upstream 403, so the NEXT resolve rebuilds the
/// resolver's warm session instead of handing back more urls signed with the
/// credential that was just refused.
///
/// The warm session (visitor data + PO token + parsed player) is what makes
/// rotation cheap, and it does not expire on the rotation timescale. But when it
/// finally does go stale, every url it produces 403s on first fetch, and with no
/// way to discard it playback fell back to HLS (1080p ceiling) permanently. That
/// is the "1440p is listed but never applies" symptom.
static NEEDS_FRESH_SESSION: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Tell the next resolve to rebuild the warm session from scratch.
pub fn invalidate_session() {
    NEEDS_FRESH_SESSION.store(true, std::sync::atomic::Ordering::SeqCst);
}

/// One "yt-resolve" webview serves every stream, so only one resolve may drive
/// it at a time. Before this, two streams rotating together both evaluated into
/// the same window and both polled the same result: `ResolvedStreams` carries no
/// video id, so a mixed-up read is undetectable and one stream simply plays the
/// other's broadcast at the right resolution.
///
/// A tokio mutex, not a std one, because it is deliberately held across the
/// resolve await. It is the ONLY async lock in this path.
static RESOLVER: Lazy<tokio::sync::Mutex<()>> = Lazy::new(|| tokio::sync::Mutex::new(()));

/// Resolve, reusing the cached entry ONLY while it is younger than `max_age`.
///
/// `resolve_streams` reuses an entry for the whole `STREAMS_TTL`, which is right
/// for starting playback and wrong for a rotation: the point of rotating is to hold
/// urls newer than the ones in hand.
///
/// The caller used to get that by invalidating the entry first. With TWO surfaces
/// on one broadcast that was pathological: each wiped the other's result, so the
/// re-check inside the lock could never hit, and every rotation cost two full
/// hidden-webview resolves, forever. Bounding by age gives both halves. A rotation
/// never adopts stale urls, and a second surface rotating moments later reuses the
/// fresh ones the first just fetched instead of discarding them.
///
/// **The bound is not free to choose.** A stream that adopts an entry of age
/// `max_age` records its own `issued` as now, so it will not rotate again for
/// another `ROTATE_AFTER`. `max_age + ROTATE_AFTER` must therefore stay under the
/// ~30s gated-url wall. The caller passes it rather than reading a constant here so
/// that arithmetic is visible where both numbers are.
pub async fn resolve_streams_fresh(
    video_id: &str,
    min_height: u32,
    max_age: Duration,
) -> Result<ResolvedStreams> {
    if let Some((s, age)) = cached_streams_with_age(video_id) {
        if age <= max_age {
            return Ok(s);
        }
    }
    let _drive = RESOLVER.lock().await;
    // Re-check INSIDE the lock. While queueing, the resolve being waited on may
    // have been for this very video, in which case its result is seconds old and
    // is exactly what this call wanted. This is the line the old invalidate made
    // unreachable.
    if let Some((s, age)) = cached_streams_with_age(video_id) {
        if age <= max_age {
            return Ok(s);
        }
    }
    let s = resolve_uncached(video_id, min_height).await?;
    store_streams(video_id, &s);
    Ok(s)
}

/// Resolve playable video and audio URLs for a video, using the webview.
///
/// Cached, because this costs a hidden window and a BotGuard run. Callers may
/// call it per stream start; they should not call it per fragment.
pub async fn resolve_streams(video_id: &str, min_height: u32) -> Result<ResolvedStreams> {
    if let Some(s) = cached_streams(video_id) {
        return Ok(s);
    }
    let _drive = RESOLVER.lock().await;
    // Re-check inside the lock: while queueing, the resolve we were waiting on
    // may have been for this very video, in which case there is nothing to do.
    // This collapses N concurrent starts on one broadcast into a single resolve.
    if let Some(s) = cached_streams(video_id) {
        return Ok(s);
    }
    let s = resolve_uncached(video_id, min_height).await?;
    store_streams(video_id, &s);
    Ok(s)
}

/// Close the reused resolver window.
///
/// It is deliberately kept open across url rotations (rebuilding it per rotation
/// was a visible stutter), so playback teardown is what ends it, otherwise a
/// hidden YouTube webview would outlive the stream it was serving.
///
/// Takes RESOLVER first. Closing the window out from under an in-flight resolve
/// does not fail that resolve, it makes it poll a window that no longer exists
/// until MINT_TIMEOUT (45s), by which point its urls are already past the ~30s
/// gated-url wall. The caller must also be sure no stream still needs it: see
/// youtube_dash::stop, which only reaches here once the registry is empty.
pub async fn close_resolver() {
    let _drive = RESOLVER.lock().await;
    if let Some(app) = crate::services::providers::app_handle() {
        use tauri::Manager;
        if let Some(w) = app.get_webview_window("yt-resolve") {
            let _ = w.close();
        }
    }
}

async fn resolve_uncached(video_id: &str, min_height: u32) -> Result<ResolvedStreams> {
    use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

    let app = crate::services::providers::app_handle()
        .ok_or_else(|| anyhow!("no app handle; cannot open the resolver window"))?;

    // ONE window for the whole session, re-navigated per resolve.
    //
    // This used to BUILD a fresh WebView2 window (and tear it down) on every
    // call. Playback re-issues its urls every ~15s, so that was a webview being
    // created and destroyed on the same GPU and compositor as the video, on a
    // fixed cadence. Measured as a visible stutter whose period tracked
    // ROTATE_AFTER exactly: halving the interval halved the stutter period.
    // Reusing the window keeps the expensive part alive; a navigate is cheap.
    const LABEL: &str = "yt-resolve";
    // Cache-busted in the QUERY, so a re-navigate actually reloads rather than
    // being treated as an in-page fragment change.
    let url = format!(
        "{}?r={}#VIDEO={}&MIN={}",
        MINT_PAGE,
        chrono::Utc::now().timestamp_millis(),
        urlencoding::encode(video_id),
        min_height
    );

    // The only line that distinguishes a real resolve from a cache reuse. Without
    // it the rotation logs look identical either way, which is what made the
    // two-surface thrash invisible for as long as it was.
    //
    // Same [YouTubeStreams] prefix as the completion line below, deliberately, so
    // one grep returns both halves of every resolve and they pair up start-to-
    // finish. A rotation that logs "re-issued urls" with NO resolve line between
    // these two is a reuse, which is what the fix is supposed to produce.
    log::info!(
        "[YouTubeStreams] resolving {} (min {}p)",
        video_id,
        min_height
    );

    let window = match app.get_webview_window(LABEL) {
        Some(existing) => {
            // Ask the ALREADY-LOADED page to resolve again. Navigating would
            // reload it, which throws away the warm Innertube session and the PO
            // token and forces another BotGuard run — the expensive work this
            // whole arrangement exists to avoid.
            //
            // The fragment is cleared first so the poller cannot read the
            // PREVIOUS resolve's answer and return stale urls immediately.
            // Take-and-clear: one 403 forces exactly one rebuild, and the
            // rotations after it go back to reusing the (now fresh) session.
            let fresh = NEEDS_FRESH_SESSION.swap(false, std::sync::atomic::Ordering::SeqCst);
            if fresh {
                log::info!("[YouTube] rebuilding the resolver session after a refusal");
            }
            let script = format!(
                "location.hash = ''; if (globalThis.__snResolve) {{ globalThis.__snResolve({}, {}, {}); }} else {{ location.hash = '#YT_ERR=' + encodeURIComponent('no resolver'); }}",
                serde_json::to_string(video_id).unwrap_or_else(|_| "\"\"".into()),
                min_height,
                fresh
            );
            existing
                .eval(&script)
                .map_err(|e| anyhow!("could not ask the resolver window: {}", e))?;
            existing
        }
        None => WebviewWindowBuilder::new(&app, LABEL, WebviewUrl::External(url.parse()?))
            .title("YouTube playback")
            .inner_size(480.0, 320.0)
            .visible(false)
            .data_store_identifier(own_store())
            .initialization_script(MINT_BUNDLE)
            .build()
            .map_err(|e| anyhow!("could not open the resolver window: {}", e))?,
    };

    let started = Instant::now();
    let mut outcome: Option<Result<ResolvedStreams>> = None;
    while started.elapsed() < MINT_TIMEOUT {
        tokio::time::sleep(Duration::from_millis(250)).await;
        if let Ok(u) = window.url() {
            if let Some(r) = read_streams_fragment(u.as_str()) {
                outcome = Some(r);
                break;
            }
        }
    }
    // Deliberately NOT closed: the next resolve reuses it. Building and
    // tearing down this webview per call is what caused the periodic stutter.

    match outcome {
        Some(Ok(s)) => {
            // Whether `pot` actually landed on the URL is the difference between
            // playing and getting ~30 seconds before Proof-of-Origin enforcement
            // starts refusing, so say it plainly rather than assuming decipher
            // attached it.
            let pot = |u: &str| if u.contains("pot=") { "pot" } else { "NO-POT" };
            log::info!(
                "[YouTubeStreams] url params: video={} audio={}",
                s.videos.first().map(|v| pot(&v.url)).unwrap_or("none"),
                pot(&s.audio_url)
            );
            log::info!(
                "[YouTubeStreams] '{}' resolved in {:.1}s: {:?} paired with audio itag {}",
                video_id,
                started.elapsed().as_secs_f64(),
                s.videos
                    .iter()
                    .map(|v| format!("{} (itag {}, {})", v.name, v.itag, v.mime_type))
                    .collect::<Vec<_>>(),
                s.audio_itag
            );
            Ok(s)
        }
        Some(Err(e)) => Err(anyhow!("resolution failed: {}", e)),
        None => Err(anyhow!(
            "the resolver window did not answer within {}s",
            MINT_TIMEOUT.as_secs()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_token_out_of_the_fragment() {
        let r = read_fragment("https://www.youtube.com/embed/#POTOKEN=abc%2Ddef").unwrap();
        assert_eq!(r.unwrap(), "abc-def");
    }

    #[test]
    fn reads_an_error_out_of_the_fragment() {
        let r = read_fragment("https://x/#POTOKEN_ERR=no%20interpreter").unwrap();
        assert_eq!(r.unwrap_err().to_string(), "no interpreter");
    }

    #[test]
    fn ignores_a_url_carrying_neither() {
        assert!(read_fragment("https://www.youtube.com/embed/#BIND=xyz").is_none());
    }

    #[test]
    fn stops_at_an_ampersand_so_later_fragment_keys_do_not_leak_in() {
        let r = read_fragment("https://x/#POTOKEN=tok&other=1").unwrap();
        assert_eq!(r.unwrap(), "tok");
    }

    #[test]
    fn decodes_both_websafe_and_standard_base64() {
        // Web-safe uses - and _ where standard uses + and /.
        let bytes = decode_websafe("q-_v").unwrap();
        assert_eq!(bytes.len(), 3);
        assert!(decode_websafe("YWJjZA==").is_ok());
        assert!(decode_websafe("!!!not base64!!!").is_err());
    }

    #[test]
    fn cache_hits_within_ttl_and_invalidate_clears_it() {
        let b = "binding-under-test";
        store(b, &[1, 2, 3]);
        assert_eq!(cached(b).unwrap(), vec![1, 2, 3]);
        invalidate(b);
        assert!(cached(b).is_none());
    }

    #[test]
    fn short_keeps_small_bindings_whole() {
        assert_eq!(short("abc"), "abc");
        assert!(short(&"x".repeat(40)).contains('…'));
    }

    fn a_resolution() -> ResolvedStreams {
        ResolvedStreams {
            videos: vec![],
            audio_url: "https://example/a".into(),
            audio_itag: 140,
        }
    }

    /// The rotation thrash, and the reason it existed.
    ///
    /// Two surfaces on ONE broadcast is a supported case: youtube_dash's registry
    /// is keyed by stream id, not video id, so the same channel can be a tile and
    /// the solo player at once. Rotation used to `invalidate_streams` BEFORE taking
    /// the resolver lock, so each surface wiped the other's just-stored result, the
    /// coalescing re-check inside the lock could never hit, and both paid a full
    /// hidden-webview resolve every ROTATE_AFTER, indefinitely.
    ///
    /// Reuse is bounded by AGE now. These two cases are the whole contract, and
    /// they pull in opposite directions on purpose: reuse what is fresh, refuse
    /// what is stale.
    ///
    /// Both assert through `resolve_uncached`, which cannot run in a test (no app
    /// handle, so no webview). That is what makes them meaningful: reuse returns
    /// Ok WITHOUT resolving, and a refusal to reuse can only surface as the Err
    /// from attempting one.
    #[tokio::test]
    async fn a_fresh_entry_is_reused_rather_than_re_resolved() {
        let id = "thrash-fresh";
        store_streams(id, &a_resolution());

        let out = resolve_streams_fresh(id, 1080, Duration::from_secs(5)).await;

        assert!(
            out.is_ok(),
            "a seconds-old entry must be reused; resolving instead is the thrash"
        );
        assert_eq!(out.unwrap().audio_url, "https://example/a");
    }

    #[tokio::test]
    async fn a_stale_entry_is_not_reused_so_rotation_stays_honest() {
        let id = "thrash-stale";
        store_streams(id, &a_resolution());

        // Zero tolerance: anything already stored is older than this.
        let out = resolve_streams_fresh(id, 1080, Duration::ZERO).await;

        // It could only get here by declining the cache and attempting a real
        // resolve, which has no webview to use. Reusing would have returned Ok and
        // silently handed a rotation the very urls it was rotating away from.
        assert!(
            out.is_err(),
            "a stale entry must NOT be reused, or rotation is a no-op"
        );
    }

    #[test]
    fn the_placeholder_bundle_reports_a_build_step_rather_than_hanging() {
        // A missing build step must read as a missing build step, not as a
        // YouTube change or a hung webview.
        // Assert on the marker Rust actually reads, not on a size threshold: an
        // earlier version guessed "> 10 KB means real", and the real bundle came
        // in at 9.5 KB. Whether placeholder or real build, the invariant that
        // matters is that a failure can be REPORTED rather than hanging the
        // resolver until it times out.
        assert!(
            MINT_BUNDLE.contains("YT_ERR"),
            "bundle must be able to report failure through the fragment Rust polls"
        );
    }
}
