//! YouTube's browse + watch adapter (`StreamSource`).
//!
//! Playback resolves through InnerTube's `player` endpoint rather than by
//! scraping the watch page for a manifest: the page's own player response is
//! increasingly session-gated, while a `player` call made as a client that
//! carries no Proof-of-Origin requirement still returns `hlsManifestUrl` for a
//! LIVE stream. Which client that is rotates over time, so the client table is
//! DATA (below) and tried in order — when YouTube retires one, the fix is an
//! edit to that table, not to this logic.
//!
//! Everything else reuses what the chat adapter already built: the watch-page
//! fetch (with its consent handling), the `ytInitialData` extraction, and the
//! per-channel metadata cache.

use crate::models::provider_stream::{CategoryPage, ProviderCategory, ProviderStream, StreamPage};
use crate::services::providers::hls_master;
use crate::services::providers::key::make_key;
use crate::services::providers::source::{
    PlaybackKind, PlaybackQuality, ResolvedPlayback, SourceCaps, StreamSource,
};
use crate::services::providers::youtube;
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use once_cell::sync::Lazy;
use serde_json::{json, Value};
use std::time::Duration;

const INNERTUBE: &str = "https://www.youtube.com/youtubei/v1";
/// How long a cached channel row may answer a liveness question. Shorter than the
/// who's-live poller's own YouTube cadence (120s) so each tick gets a real answer
/// rather than replaying the one from when chat first opened.
const META_TTL: Duration = Duration::from_secs(60);
/// The public web key. InnerTube no longer requires it on newer clients, but
/// sending it costs nothing and keeps older paths working.
const INNERTUBE_KEY: &str = "AIzaSyAO_FJ2SlqU8Q4STEHLGCilw_Y9_11qcW8";

/// Clients tried in order, each a context block for the `player` call.
///
/// These are chosen for one property: no Proof-of-Origin token requirement on
/// live HLS. That is a moving target — YouTube has already retired the older
/// mobile clients as defaults elsewhere — so treat this as a list to edit, not
/// logic to rewrite.
struct PlayerClient {
    name: &'static str,
    client_name: &'static str,
    client_version: &'static str,
    user_agent: &'static str,
}

const CLIENTS: &[PlayerClient] = &[
    PlayerClient {
        name: "android",
        client_name: "ANDROID",
        client_version: "21.08.266",
        user_agent: "com.google.android.youtube/21.08.266 (Linux; U; Android 14) gzip",
    },
    PlayerClient {
        name: "visionos",
        client_name: "VISIONOS",
        client_version: "1.02",
        user_agent: "Mozilla/5.0 (Apple Vision Pro; CPU OS 1_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15",
    },
    PlayerClient {
        name: "web_embedded",
        client_name: "WEB_EMBEDDED_PLAYER",
        client_version: "1.20240101.00.00",
        user_agent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/150.0.0.0 Safari/537.36",
    },
];

/// Clients tried WITH the user's YouTube session when the anonymous table is
/// refused for an age gate. Order and versions follow yt-dlp's signed-in
/// defaults (`tv_downgraded`, then `web_safari`). ANDROID and IOS are absent on
/// purpose: they ignore account cookies, so retrying them signed in cannot help.
struct AuthedClient {
    name: &'static str,
    client_name: &'static str,
    client_version: &'static str,
    /// The numeric id YouTube expects in `X-Youtube-Client-Name`.
    client_id: &'static str,
    user_agent: &'static str,
}

const AUTHED_CLIENTS: &[AuthedClient] = &[
    AuthedClient {
        name: "tv (authed)",
        client_name: "TVHTML5",
        client_version: "5.20260114",
        client_id: "7",
        user_agent: "Mozilla/5.0 (ChromiumStylePlatform) Cobalt/Version",
    },
    AuthedClient {
        name: "web safari (authed)",
        client_name: "WEB",
        client_version: "2.20260114.01.00",
        client_id: "1",
        user_agent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15",
    },
];

static HTTP: Lazy<reqwest::Client> = Lazy::new(|| {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        // Consent interstitial in the EU is skipped by this cookie, the same way
        // the chat adapter's page fetch does it.
        .default_headers({
            let mut h = reqwest::header::HeaderMap::new();
            h.insert(
                reqwest::header::COOKIE,
                reqwest::header::HeaderValue::from_static("SOCS=CAI"),
            );
            h
        })
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
});

pub struct YouTubeSource;

impl YouTubeSource {
    pub fn new() -> Self {
        Self
    }
}

/// Resolve an identifier (`@handle`, `UC…`, or a video id) to the live video id.
/// A bare 11-character id is already one; anything else needs the channel's live
/// page, which redirects to the current broadcast.
async fn live_video_id(identifier: &str) -> Result<String> {
    let looks_like_video = identifier.len() == 11
        && identifier
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if looks_like_video {
        return Ok(identifier.to_string());
    }
    let url = youtube::live_page_url(identifier);
    let html = youtube::fetch_youtube_html(&HTTP, &url, identifier).await?;
    // The live page embeds the current broadcast's id in its player response.
    if let Some(player) = youtube::extract_json(&html, "ytInitialPlayerResponse") {
        if let Some(id) = player.pointer("/videoDetails/videoId").and_then(|v| v.as_str()) {
            return Ok(id.to_string());
        }
    }
    // Channel pages without a live broadcast fall back to their first video,
    // which is NOT what "watch this channel" means — so treat it as offline.
    Err(anyhow!("{} isn't live right now", identifier))
}

/// POST `youtubei/v1/player` as `client`, returning the parsed player response.
async fn player_response(video_id: &str, client: &PlayerClient) -> Result<Value> {
    let body = json!({
        "videoId": video_id,
        "contentCheckOk": true,
        "racyCheckOk": true,
        "context": {
            "client": {
                "clientName": client.client_name,
                "clientVersion": client.client_version,
                "hl": "en",
                "gl": "US",
            }
        }
    });
    let resp = HTTP
        .post(format!("{}/player?key={}", INNERTUBE, INNERTUBE_KEY))
        .header(reqwest::header::USER_AGENT, client.user_agent)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .json(&body)
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(anyhow!("player returned {}", resp.status()));
    }
    Ok(resp.json::<Value>().await?)
}

/// POST `youtubei/v1/player` carrying the user's YouTube session, for a video
/// the anonymous table was refused for. Mirrors `youtube::post_innertube_authed`
/// (the chat send path): the harvested cookies plus the SAPISIDHASH header, and
/// the channel's cached `visitorData` when there is one. No `key=`, like the
/// VISIONOS call.
async fn player_response_authed(
    video_id: &str,
    client: &AuthedClient,
    visitor_data: Option<&str>,
) -> Result<Value> {
    let headers = crate::services::youtube_auth_service::auth_headers()
        .ok_or_else(|| anyhow!("no YouTube session"))?;
    let mut ctx = json!({
        "clientName": client.client_name,
        "clientVersion": client.client_version,
        "userAgent": client.user_agent,
        "hl": "en",
        "gl": "US",
    });
    if let Some(vd) = visitor_data {
        ctx["visitorData"] = json!(vd);
    }
    let body = json!({
        "videoId": video_id,
        "contentCheckOk": true,
        "racyCheckOk": true,
        "context": { "client": ctx }
    });
    let mut req = reqwest::Client::new()
        .post(format!("{}/player?prettyPrint=false", INNERTUBE))
        .header(reqwest::header::USER_AGENT, client.user_agent)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header("X-Youtube-Client-Name", client.client_id)
        .header("X-Youtube-Client-Version", client.client_version);
    if let Some(vd) = visitor_data {
        req = req.header("X-Goog-Visitor-Id", vd);
    }
    for (k, v) in headers {
        req = req.header(k, v);
    }
    let resp = req.json(&body).send().await?;
    if !resp.status().is_success() {
        return Err(anyhow!("{} player returned {}", client.name, resp.status()));
    }
    Ok(resp.json::<Value>().await?)
}

/// Whether a refusal is YouTube's age gate. Two wordings exist: the mobile
/// clients say "inappropriate for some users", everything else says "confirm
/// your age". Both mean the same thing and only a signed-in, of-age account
/// clears either.
fn is_age_gate(reason: &str) -> bool {
    let r = reason.to_lowercase();
    r.contains("confirm your age")
        || r.contains("inappropriate")
        || r.contains("age-restricted")
        || r.contains("age_verification_required")
        || r.contains("age_check_required")
}

// ---------------------------------------------------------------------------
// High renditions (1440p / 2160p), via the VISIONOS client
// ---------------------------------------------------------------------------

/// The `visionos` entry in CLIENTS above answers `LOGIN_REQUIRED` because a bare
/// context is not enough. What unlocks it is a VISITOR IDENTITY: a `visitorData`
/// token plus the cookie jar that YouTube hands out with a watch page. Measured
/// against a live payload 2026-08-21; also note the API key must be OMITTED.
const VISIONOS_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 15_7_3) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15";

/// Visitor identity is per-session, not per-video, so it is fetched once and
/// reused. An hour is well inside its usable life and keeps a long viewing
/// session from re-scraping a watch page.
const IDENTITY_TTL: Duration = Duration::from_secs(3600);

#[derive(Clone)]
struct VisitorIdentity {
    visitor_data: String,
    cookies: String,
    at: std::time::Instant,
}

static IDENTITY: Lazy<std::sync::Mutex<Option<VisitorIdentity>>> =
    Lazy::new(|| std::sync::Mutex::new(None));

/// Scrape a watch page for the visitor token and the cookies it sets.
async fn visitor_identity(video_id: &str) -> Result<VisitorIdentity> {
    if let Ok(guard) = IDENTITY.lock() {
        if let Some(id) = guard.as_ref() {
            if id.at.elapsed() < IDENTITY_TTL {
                return Ok(id.clone());
            }
        }
    }
    // `bpctr` and `has_verified` skip the "content warning" interstitial, which
    // would otherwise serve a page carrying no player response at all.
    let url = format!(
        "https://www.youtube.com/watch?v={}&bpctr=9999999999&has_verified=1",
        video_id
    );
    let resp = HTTP
        .get(&url)
        .header(reqwest::header::COOKIE, "PREF=hl=en&tz=UTC; SOCS=CAI")
        .send()
        .await?;
    let mut jar: Vec<String> = vec!["PREF=hl=en&tz=UTC".into(), "SOCS=CAI".into()];
    for value in resp.headers().get_all(reqwest::header::SET_COOKIE).iter() {
        if let Ok(s) = value.to_str() {
            if let Some(pair) = s.split(';').next() {
                if pair.contains('=') {
                    jar.push(pair.trim().to_string());
                }
            }
        }
    }
    let html = resp.text().await?;
    let visitor_data = youtube::json_str_after(&html, "\"visitorData\":\"")
        .map(|v| youtube::decode_json_escapes(&v))
        .ok_or_else(|| anyhow!("watch page carried no visitorData"))?;

    let id = VisitorIdentity {
        visitor_data,
        cookies: jar.join("; "),
        at: std::time::Instant::now(),
    };
    if let Ok(mut guard) = IDENTITY.lock() {
        *guard = Some(id.clone());
    }
    Ok(id)
}

/// POST the player as VISIONOS, carrying the visitor identity. This is the call
/// that returns 1440p/2160p `adaptiveFormats` with usable URLs.
async fn player_visionos(video_id: &str, id: &VisitorIdentity) -> Result<Value> {
    let body = json!({
        "videoId": video_id,
        "contentCheckOk": true,
        "racyCheckOk": true,
        "context": {
            "client": {
                "clientName": "VISIONOS",
                "clientVersion": "1.02",
                "deviceMake": "Apple",
                "deviceModel": "RealityDevice17,1",
                "userAgent": VISIONOS_UA,
                "osName": "visionOS",
                "osVersion": "26.5.23O471",
                "hl": "en",
                "timeZone": "UTC",
                "utcOffsetMinutes": 0,
            }
        }
    });
    // No `key=` here on purpose: sending the legacy API key is not what
    // authorises this call, and the endpoint is happier without it.
    let resp = HTTP
        .post(format!("{}/player?prettyPrint=false", INNERTUBE))
        .header(reqwest::header::USER_AGENT, VISIONOS_UA)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(reqwest::header::COOKIE, &id.cookies)
        .header(reqwest::header::ORIGIN, "https://www.youtube.com")
        .header("X-Goog-Visitor-Id", &id.visitor_data)
        .header("X-Youtube-Client-Name", "101")
        .header("X-Youtube-Client-Version", "1.02")
        .json(&body)
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(anyhow!("visionos player returned {}", resp.status()));
    }
    Ok(resp.json::<Value>().await?)
}

/// A video-only rendition plus the audio track it has to be paired with.
#[derive(Debug, Clone)]
pub struct HighRendition {
    pub name: String,
    pub itag: u64,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub bandwidth: u64,
    pub video_url: String,
    pub audio_url: String,
}

/// Every rendition taller than the HLS ladder can carry, newest resolve wins.
///
/// These are the whole reason this module talks to VISIONOS at all: YouTube's
/// live HLS master is the legacy muxed itag family and stops at 1080p60 by
/// construction, so anything higher only ever appears here.
pub async fn high_renditions(video_id: &str, above: u32) -> Vec<HighRendition> {
    // Preferred path: VISIONOS hands back `adaptiveFormats` whose URLs are
    // already playable. No `n` to descramble, no Proof-of-Origin token, no
    // `alr` to turn off, and — the part that matters most — they do not go
    // stale after half a minute. Measured 2026-08-21: 14 consecutive 1440p60
    // segment pairs over 78s with nothing but a visitor identity attached.
    match visitor_identity(video_id).await {
        Ok(id) => match player_visionos(video_id, &id).await {
            Ok(player) => {
                let out = renditions_from_player(&player, above);
                if !out.is_empty() {
                    report_renditions(video_id, above, "visionos", &out);
                    return out;
                }
                log::info!(
                    "[YouTube] '{}' offers nothing above {}p on visionos; trying the gated path",
                    video_id,
                    above
                );
            }
            Err(e) => log::info!("[YouTube] '{}' visionos player failed: {}", video_id, e),
        },
        Err(e) => log::info!("[YouTube] '{}' visitor identity failed: {}", video_id, e),
    }

    // Fallback, for broadcasts where the high rungs only exist on the WEB
    // client. Those URLs need a descrambled `n` and a BotGuard token, so they
    // are resolved in a webview — and they expire about 30 seconds after issue,
    // which is why the relay re-resolves them rather than holding one.
    let resolved = match crate::services::youtube_potoken::resolve_streams(video_id, above).await {
        Ok(r) => r,
        Err(e) => {
            log::info!(
                "[YouTube] '{}' has nothing playable above {}p: {}",
                video_id,
                above,
                e
            );
            return Vec::new();
        }
    };

    let mut out: Vec<HighRendition> = resolved
        .videos
        .iter()
        .map(|v| HighRendition {
            name: v.name.clone(),
            itag: v.itag as u64,
            width: v.width,
            height: v.height,
            fps: v.fps,
            bandwidth: 0,
            video_url: v.url.clone(),
            audio_url: resolved.audio_url.clone(),
        })
        .collect();

    // Tallest first by SHORT edge, so a vertical stream ranks by the edge its
    // label is built from.
    out.sort_by_key(|r| std::cmp::Reverse(r.height.min(r.width.max(1))));
    report_renditions(video_id, above, "web+potoken", &out);
    out
}

fn report_renditions(video_id: &str, above: u32, via: &str, out: &[HighRendition]) {
    log::info!(
        "[YouTube] '{}' offers {} rendition(s) above {}p via {}: {:?}",
        video_id,
        out.len(),
        above,
        via,
        out.iter().map(|r| r.name.as_str()).collect::<Vec<_>>()
    );
}

/// The short edge, so a vertical 1080x1920 broadcast is not read as 1920p.
fn short_edge(f: &Value) -> u32 {
    let h = f.get("height").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let w = f.get("width").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    if w == 0 { h } else { h.min(w) }
}

/// Pull every video-only rendition above `above` out of a player response,
/// paired with the best AAC track.
///
/// Only entries carrying a real `url` are usable. ANDROID and IOS advertise the
/// same high itags with the field missing entirely, because those clients are
/// expected to fetch them over SABR instead.
fn renditions_from_player(player: &Value, above: u32) -> Vec<HighRendition> {
    let formats = match player.pointer("/streamingData/adaptiveFormats").and_then(|v| v.as_array()) {
        Some(f) => f,
        None => return Vec::new(),
    };
    let has_url = |f: &&Value| f.get("url").and_then(|v| v.as_str()).is_some_and(|u| !u.is_empty());
    let mime = |f: &Value| {
        f.get("mimeType")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };

    // AAC in MP4, never Opus-in-WebM: only the MP4 one can be handed to the
    // player without a second transmux.
    let audio = formats
        .iter()
        .filter(has_url)
        .filter(|f| f.get("height").is_none() && mime(f).contains("mp4a"))
        .max_by_key(|f| f.get("bitrate").and_then(|v| v.as_u64()).unwrap_or(0));
    let audio_url = match audio.and_then(|a| a.get("url")).and_then(|v| v.as_str()) {
        Some(u) => u.to_string(),
        None => return Vec::new(),
    };

    let mut out: Vec<HighRendition> = formats
        .iter()
        .filter(has_url)
        .filter(|f| short_edge(f) > above)
        .map(|f| {
            let edge = short_edge(f);
            let fps = f.get("fps").and_then(|v| v.as_f64()).unwrap_or(30.0);
            HighRendition {
                name: if fps >= 50.0 {
                    format!("{}p{}", edge, fps.round() as u32)
                } else {
                    format!("{}p", edge)
                },
                itag: f.get("itag").and_then(|v| v.as_u64()).unwrap_or(0),
                width: f.get("width").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                height: f.get("height").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                fps,
                bandwidth: f.get("bitrate").and_then(|v| v.as_u64()).unwrap_or(0),
                video_url: f
                    .get("url")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                audio_url: audio_url.clone(),
            }
        })
        .collect();

    // Tallest first by short edge, then by bitrate so a duplicate rung keeps the
    // better encode.
    out.sort_by_key(|r| {
        (
            std::cmp::Reverse(r.height.min(r.width.max(1))),
            std::cmp::Reverse(r.bandwidth),
        )
    });
    out.dedup_by_key(|r| r.height.min(r.width.max(1)));
    out
}

/// Renditions above the HLS ceiling, cached alongside the parsed master so the
/// menu keeps listing them on a quality change without re-resolving.
static HIGHS: Lazy<std::sync::Mutex<std::collections::HashMap<String, (Vec<HighRendition>, std::time::Instant)>>> =
    Lazy::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

/// Prepend the above-1080p renditions to an HLS menu.
///
/// This is what makes the two transports read as one list. Without it the high
/// rungs would only be visible once you were already watching one, since the
/// HLS path builds its menu purely from the master playlist.
fn with_highs(video_id: &str, hls: Vec<PlaybackQuality>) -> Vec<PlaybackQuality> {
    let Some(highs) = cached_highs(video_id) else {
        return hls;
    };
    let mut menu: Vec<PlaybackQuality> = highs
        .iter()
        .map(|r| PlaybackQuality {
            name: r.name.clone(),
            // Addressed by name through the relay, never fetched from here.
            url: String::new(),
            width: Some(r.width),
            height: Some(r.height),
            fps: Some(r.fps),
            bandwidth: Some(r.bandwidth),
        })
        .collect();
    menu.extend(hls);
    menu
}

fn cached_highs(video_id: &str) -> Option<Vec<HighRendition>> {
    let map = HIGHS.lock().ok()?;
    let (v, at) = map.get(video_id)?;
    // Gated urls die of old age far sooner than the master does, so a cached
    // set of them is only worth handing back inside that shorter window.
    // Serving a 110-second-old one would start playback on a url that is
    // already refusing.
    let ttl = if v.iter().any(|r| crate::services::youtube_dash::is_gated(&r.video_url)) {
        crate::services::youtube_dash::ROTATE_AFTER
    } else {
        MASTER_TTL
    };
    (at.elapsed() < ttl).then(|| v.clone())
}

/// One resolve per video at a time, so concurrent callers share it.
///
/// Two callers race on every tile start: the playback resolve (`try_high`) and
/// the quality menu (`qualities`). Both check the cache, both miss it on a cold
/// open, and both then drive the resolver webview behind the resolver lock,
/// costing seconds of hidden-webview work before first frame.
///
/// A cache alone cannot fix that, because nothing is in it yet when they race.
/// Same shape as the segment single-flight in youtube_dash. It is also why a
/// broadcast with NO rungs above 1080p was the expensive case: the answer is
/// "nothing", and every racing caller paid full price to learn it.
type Gate = std::sync::Arc<tokio::sync::Mutex<()>>;
static HIGHS_INFLIGHT: Lazy<std::sync::Mutex<std::collections::HashMap<String, Gate>>> =
    Lazy::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

/// `high_renditions`, coalesced and cached.
///
/// Prefer this everywhere except the post-refusal retry, which deliberately wants
/// a FRESH resolve after rebuilding the session and must not be served a cached
/// answer.
///
/// No negative TTL is needed: an empty result is cached by `store_highs` like any
/// other, for MASTER_TTL. A separate negative cache would mean picking a number
/// that could lock a broadcast out of a rung it gained mid-stream.
pub async fn high_renditions_cached(video_id: &str, above: u32) -> Vec<HighRendition> {
    if let Some(h) = cached_highs(video_id) {
        return h;
    }
    let gate = {
        let Ok(mut m) = HIGHS_INFLIGHT.lock() else {
            // Poisoned: resolve unguarded rather than refusing to play.
            let h = high_renditions(video_id, above).await;
            store_highs(video_id, &h);
            return h;
        };
        Gate::clone(
            m.entry(video_id.to_string())
                .or_insert_with(|| Gate::new(tokio::sync::Mutex::new(()))),
        )
    };
    let _held = gate.lock().await;

    // Whoever went first has stored the answer, including when the answer is
    // "nothing above the floor".
    if let Some(h) = cached_highs(video_id) {
        if let Ok(mut m) = HIGHS_INFLIGHT.lock() {
            m.remove(video_id);
        }
        return h;
    }

    let h = high_renditions(video_id, above).await;
    store_highs(video_id, &h);
    if let Ok(mut m) = HIGHS_INFLIGHT.lock() {
        m.remove(video_id);
    }
    h
}

fn store_highs(video_id: &str, v: &[HighRendition]) {
    if let Ok(mut map) = HIGHS.lock() {
        map.retain(|_, (_, at)| at.elapsed() < MASTER_TTL);
        map.insert(video_id.to_string(), (v.to_vec(), std::time::Instant::now()));
    }
}

/// Does this error mean "the credential was refused" rather than "playback is
/// impossible"? Only those are worth rebuilding the session and retrying for.
fn is_refusal(e: &anyhow::Error) -> bool {
    let msg = e.to_string();
    msg.contains("403") || msg.contains("no longer authorised") || msg.contains("401")
}

/// The HLS ladder for a video, from cache or resolved on demand.
///
/// `try_high` needs this even when it is about to serve a DASH rendition,
/// because the menu it returns IS the quality picker. It previously defaulted to
/// an EMPTY ladder on a cache miss, and a cache miss is precisely the case where
/// the ordinary HLS path never ran to populate it. The picker then collapsed to
/// the single relayed rendition, so choosing 1440p left no way to step back down.
async fn hls_ladder(video_id: &str) -> Vec<PlaybackQuality> {
    if let Some(cached) = cached_master(video_id) {
        return cached;
    }
    for client in CLIENTS {
        let Ok(player) = player_response(video_id, client).await else {
            continue;
        };
        if playability_error(&player).is_some() {
            continue;
        }
        let Some(manifest) = player
            .pointer("/streamingData/hlsManifestUrl")
            .and_then(|v| v.as_str())
        else {
            continue;
        };
        let Ok(resp) = HTTP.get(manifest).send().await else {
            continue;
        };
        let Ok(master) = resp.text().await else {
            continue;
        };
        let base = manifest.rsplit_once('/').map(|(b, _)| b).unwrap_or("");
        let qualities = hls_master::parse(&master, base);
        if !qualities.is_empty() {
            store_master(video_id, &qualities);
            return qualities;
        }
    }
    // Genuinely nothing to add. The high rendition alone is still playable, so
    // this degrades to the old behaviour rather than failing the start.
    log::warn!(
        "[YouTube] '{}' could not resolve an HLS ladder for the quality menu",
        video_id
    );
    Vec::new()
}

impl YouTubeSource {
    /// If `quality` names a rendition above the HLS ceiling, serve it through the
    /// DASH-to-HLS relay. Returns `None` for everything else so the caller falls
    /// through to the ordinary HLS path.
    async fn try_high(
        &self,
        stream_id: &str,
        video_id: &str,
        quality: &str,
    ) -> Option<ResolvedPlayback> {
        let highs = match cached_highs(video_id) {
            Some(h) => h,
            None => {
                // 1080 is the HLS ceiling on every live broadcast measured, and
                // it is fixed by the muxed itag family rather than by the client.
                // Coalesced: the quality menu races this on every tile start.
                high_renditions_cached(video_id, 1080).await
            }
        };
        if highs.is_empty() {
            return None;
        }
        // "best" should mean the best there is, not the best HLS can carry.
        let want = if quality.eq_ignore_ascii_case("best") {
            highs.first()
        } else {
            highs.iter().find(|r| r.name.eq_ignore_ascii_case(quality))
        }?;

        let menu = with_highs(video_id, hls_ladder(video_id).await);
        let wanted_name = want.name.clone();
        let first = crate::services::youtube_dash::start(stream_id, video_id, 1080, want).await;

        let started = match first {
            Ok(url) => Ok(url),
            Err(e) if is_refusal(&e) => {
                // A refusal means these urls were signed with a credential that
                // has since gone stale. Dropping to HLS here would cost the
                // viewer the resolution they actually chose, for a reason that is
                // entirely recoverable: rebuild the resolver session, re-resolve
                // the renditions, and try again AT THE SAME QUALITY. Only a
                // second failure is worth falling back for.
                log::info!(
                    "[YouTube] '{}' refused; rebuilding the session and retrying at the same quality",
                    wanted_name
                );
                crate::services::youtube_potoken::invalidate_session();
                let fresh = high_renditions(video_id, 1080).await;
                store_highs(video_id, &fresh);
                match fresh
                    .iter()
                    .find(|r| r.name.eq_ignore_ascii_case(&wanted_name))
                    .or_else(|| fresh.first())
                {
                    Some(retry_want) => {
                        crate::services::youtube_dash::start(stream_id, video_id, 1080, retry_want).await
                    }
                    None => Err(anyhow!("no renditions after re-resolving")),
                }
            }
            Err(e) => Err(e),
        };

        match started {
            Ok(url) => Some(ResolvedPlayback {
                kind: PlaybackKind::LocalHls,
                url,
                quality: wanted_name,
                qualities: menu,
            }),
            Err(e) => {
                // Fall back to HLS rather than failing the whole start: a 1080p
                // stream is much better than an error dialog.
                log::warn!(
                    "[YouTube] '{}' could not start the DASH relay, falling back to HLS: {}",
                    wanted_name,
                    e
                );
                None
            }
        }
    }

}

/// Turn a playability status into a message worth showing the user. YouTube puts
/// the real reason here — members-only, geo-blocked, not started yet — and it is
/// far more useful than a generic failure.
fn playability_error(player: &Value) -> Option<String> {
    let status = player
        .pointer("/playabilityStatus/status")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if status == "OK" {
        return None;
    }
    let reason = player
        .pointer("/playabilityStatus/reason")
        .and_then(|v| v.as_str())
        .or_else(|| {
            player
                .pointer("/playabilityStatus/errorScreen/playerErrorMessageRenderer/reason/simpleText")
                .and_then(|v| v.as_str())
        })
        .unwrap_or(status);
    Some(reason.to_string())
}

/// Whether a refusal describes the VIDEO (members-only, geo-blocked, not started)
/// rather than the client we happened to ask with (bot checks, embed refusals).
/// The former is worth showing the user; the latter is noise from a client we are
/// about to stop using anyway.
/// How long a parsed master answers a quality switch before we re-resolve. Short,
/// because the variant URLs are signed and expire.
const MASTER_TTL: Duration = Duration::from_secs(110);

struct CachedMaster {
    qualities: Vec<PlaybackQuality>,
    at: std::time::Instant,
}

/// Parsed masters by VIDEO id. See the note at the call site for why not by channel.
static MASTERS: Lazy<std::sync::Mutex<std::collections::HashMap<String, CachedMaster>>> =
    Lazy::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

fn cached_master(video_id: &str) -> Option<Vec<PlaybackQuality>> {
    let map = MASTERS.lock().ok()?;
    let hit = map.get(video_id)?;
    (hit.at.elapsed() < MASTER_TTL).then(|| hit.qualities.clone())
}

fn store_master(video_id: &str, qualities: &[PlaybackQuality]) {
    if let Ok(mut map) = MASTERS.lock() {
        // Expired entries for streams nobody is watching any more would otherwise
        // accumulate for the life of the process.
        map.retain(|_, v| v.at.elapsed() < MASTER_TTL);
        map.insert(
            video_id.to_string(),
            CachedMaster {
                qualities: qualities.to_vec(),
                at: std::time::Instant::now(),
            },
        );
    }
}

/// Log which client won and what it actually offered, plus anything YouTube
/// advertises that we cannot select.
///
/// Why this is at Info and not Debug: the quality menu is built from the HLS
/// master, and YouTube's live HLS master stops at 1080p even on broadcasts that
/// serve 1440p and 2160p. The HLS ladder is the legacy MUXED itag family
/// (91-94, 300, 301), whose top rung is 1080p60 by construction, so no client
/// tweak raises it.
///
/// Those higher renditions are video-only `adaptiveFormats` itags (271/308 for
/// 1440p, 313/315 for 2160p) delivered as manifestless DASH fragments addressed
/// by `&sq=N`. `youtube_dash` rewrites their container and serves them as HLS,
/// so they DO appear in the menu.
///
/// Logged at Info because the two transports are worth telling apart: it records
/// which client won, what HLS could carry, and which itags came the other way.
fn report_ladder(
    channel: &str,
    video_id: &str,
    client: &str,
    player: &Value,
    qualities: &[PlaybackQuality],
) {
    let hls_max = qualities.iter().filter_map(|q| q.height).max();
    let advertised: Vec<(u64, u64)> = player
        .pointer("/streamingData/adaptiveFormats")
        .and_then(|v| v.as_array())
        .map(|formats| {
            formats
                .iter()
                .filter_map(|f| {
                    Some((
                        f.get("height")?.as_u64()?,
                        f.get("itag").and_then(|i| i.as_u64()).unwrap_or(0),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();

    let hls_heights: Vec<u32> = qualities.iter().filter_map(|q| q.height).collect();
    log::info!(
        "[YouTube] resolved '{}' via {} client: HLS heights {:?}",
        channel,
        client,
        hls_heights
    );

    if let Some(cap) = hls_max {
        let mut capped: Vec<u64> = advertised
            .iter()
            .filter(|(h, _)| *h > cap as u64)
            .map(|(_, itag)| *itag)
            .collect();
        if !capped.is_empty() {
            capped.sort_unstable();
            capped.dedup();
            // Say which of the two cases this is. An itag appearing in
            // `adaptiveFormats` does NOT mean it can be fetched: on some
            // broadcasts the high rungs are advertised but carry no URL on any
            // client we can reach, because YouTube serves them only over SABR.
            // Claiming the relay handles them in that case sends the next reader
            // hunting a bug that is not there.
            let reachable = cached_highs(video_id).map(|h| !h.is_empty()).unwrap_or(false);
            if reachable {
                log::info!(
                    "[YouTube] '{}' HLS tops out at {}p; itags {:?} go higher and are served \
                     through the DASH-to-HLS relay",
                    channel,
                    cap,
                    capped
                );
            } else {
                log::info!(
                    "[YouTube] '{}' HLS tops out at {}p; itags {:?} are advertised but carry no \
                     fetchable URL on any reachable client (SABR-only on this broadcast), so the \
                     menu stops at {}p. yt-dlp caps here too",
                    channel,
                    cap,
                    capped,
                    cap
                );
            }
        }
    }
}

fn is_content_refusal(reason: &str) -> bool {
    let r = reason.to_lowercase();
    !(r.contains("bot")
        || r.contains("sign in")
        || r.contains("unavailable")
        || r.contains("playback on other websites"))
}

/// Turn an `OK` player response into playback: fetch its HLS master, pick the
/// variant `quality` asks for, remember the ladder. Shared by the anonymous and
/// the signed-in resolves so the two cannot drift.
async fn hls_playback(
    channel: &str,
    video_id: &str,
    client_name: &str,
    player: &Value,
    quality: &str,
) -> Result<ResolvedPlayback> {
    let manifest = player
        .pointer("/streamingData/hlsManifestUrl")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("{} returned no HLS manifest", client_name))?;
    let master = HTTP.get(manifest).send().await?.text().await?;
    let base = manifest.rsplit_once('/').map(|(b, _)| b).unwrap_or("");
    let qualities = hls_master::parse(&master, base);
    if qualities.is_empty() {
        return Err(anyhow!("master playlist had no variants"));
    }
    let (idx, label) = hls_master::select(&qualities, quality)
        .ok_or_else(|| anyhow!("no usable quality for this stream"))?;
    report_ladder(channel, video_id, client_name, player, &qualities);
    store_master(video_id, &qualities);
    let url = qualities[idx].url.clone();
    Ok(ResolvedPlayback {
        kind: PlaybackKind::Hls,
        url,
        quality: label,
        qualities: with_highs(video_id, qualities),
    })
}

impl YouTubeSource {
    /// Resolve an age-gated broadcast with the user's session. Same loop shape
    /// as the anonymous one; the reason it reports is the LAST client's, because
    /// by now every reason is about the account, not the video.
    async fn resolve_authed(
        &self,
        channel: &str,
        video_id: &str,
        quality: &str,
    ) -> Result<ResolvedPlayback> {
        let visitor_data = youtube::channel_meta(channel).and_then(|m| m.visitor_data);
        let mut last_error = None;
        for client in AUTHED_CLIENTS {
            let player =
                match player_response_authed(video_id, client, visitor_data.as_deref()).await {
                    Ok(p) => p,
                    Err(e) => {
                        log::warn!("[YouTube] {} failed for '{}': {}", client.name, video_id, e);
                        last_error = Some(e.to_string());
                        continue;
                    }
                };
            if let Some(reason) = playability_error(&player) {
                log::warn!(
                    "[YouTube] {} refused '{}': {}",
                    client.name,
                    video_id,
                    reason
                );
                last_error = Some(reason);
                continue;
            }
            match hls_playback(channel, video_id, client.name, &player, quality).await {
                Ok(resolved) => {
                    log::info!(
                        "[YouTube] {} unlocked age-gated '{}'",
                        client.name,
                        video_id
                    );
                    return Ok(resolved);
                }
                Err(e) => {
                    log::warn!(
                        "[YouTube] {} gave no playable manifest for '{}': {}",
                        client.name,
                        video_id,
                        e
                    );
                    last_error = Some(e.to_string());
                }
            }
        }
        Err(anyhow!(last_error.unwrap_or_else(|| {
            "no signed-in client answered".to_string()
        })))
    }
}

#[async_trait]
impl StreamSource for YouTubeSource {
    fn id(&self) -> &'static str {
        "youtube"
    }

    fn caps(&self) -> SourceCaps {
        SourceCaps {
            playback: true,
            directory: true,
            search: true,
            // Subscriptions come from the user's own session when connected.
            native_follows: crate::services::youtube_auth_service::is_connected(),
            live_check: true,
        }
    }

    /// Session-free, unlike the default. Resolving here starts a relay, so the
    /// menu is built from the same two sources `try_high` reads and never goes
    /// near `youtube_dash`. The returned list is byte-identical to what
    /// resolving produced, so the UI sees no change.
    async fn qualities(&self, channel: &str) -> Result<Vec<PlaybackQuality>> {
        let video_id = live_video_id(channel).await?;
        // Warm the highs cache the way try_high does, so a cold open still lists
        // the 1440p/2160p rungs that live outside the HLS ladder. Coalesced,
        // because this races the playback resolve on every tile start and the two
        // used to drive the resolver webview separately for the same answer.
        high_renditions_cached(&video_id, 1080).await;
        Ok(with_highs(&video_id, hls_ladder(&video_id).await))
    }

    async fn resolve_playback(
        &self,
        stream_id: &str,
        channel: &str,
        quality: &str,
    ) -> Result<ResolvedPlayback> {
        let video_id = live_video_id(channel).await?;

        // A quality change re-enters here (change_stream_quality just calls
        // start_stream again), so without this every switch paid for another
        // InnerTube round trip plus a master fetch. Keyed by VIDEO id, not by
        // channel: when a broadcast ends and the channel starts a new one the id
        // changes, so a dead manifest can never be replayed from cache.
        // 1440p/2160p live only outside the HLS ladder. Resolve them first so
        // they are in the menu whether or not the master came from cache, and so
        // a request for one is answered before the HLS selector, which cannot
        // represent them, ever sees it.
        if let Some(resolved) = self.try_high(stream_id, &video_id, quality).await {
            return Ok(resolved);
        }
        // Anything at or below the HLS ceiling is the original path, untouched.
        crate::services::youtube_dash::stop(stream_id).await;

        if let Some(qualities) = cached_master(&video_id) {
            if let Some((idx, label)) = hls_master::select(&qualities, quality) {
                log::info!(
                    "[YouTube] '{}' quality '{}' served from cached master ({} variants)",
                    channel,
                    label,
                    qualities.len()
                );
                let url = qualities[idx].url.clone();
                return Ok(ResolvedPlayback {
                    kind: PlaybackKind::Hls,
                    url,
                    quality: label,
                    qualities: with_highs(&video_id, qualities),
                });
            }
        }

        let mut last_error = None;
        for client in CLIENTS {
            let player = match player_response(&video_id, client).await {
                Ok(p) => p,
                Err(e) => {
                    last_error = Some(e.to_string());
                    continue;
                }
            };
            // A refusal is NOT necessarily the same for every client — measured
            // on one live stream: ANDROID returned OK while VISIONOS returned
            // LOGIN_REQUIRED ("Sign in to confirm you're not a bot") and
            // WEB_EMBEDDED_PLAYER returned ERROR. So record the reason and keep
            // going; only report one if the whole table is exhausted.
            if let Some(reason) = playability_error(&player) {
                log::debug!("[YouTube] {} refused '{}': {}", client.name, video_id, reason);
                // Keep the first reason that sounds like a property of the VIDEO
                // rather than of the client, since that is the useful message.
                if last_error.is_none() || is_content_refusal(&reason) {
                    last_error = Some(reason);
                }
                continue;
            }
            match hls_playback(channel, &video_id, client.name, &player, quality).await {
                Ok(resolved) => return Ok(resolved),
                Err(e) => {
                    last_error = Some(e.to_string());
                    continue;
                }
            }
        }
        let reason = last_error.unwrap_or_else(|| "no client returned a manifest".to_string());

        // An age gate is the one refusal a signed-in account can clear. The
        // anonymous table above never carries the session (and its first two
        // clients would ignore it anyway), so this is a second, smaller table
        // tried only when the video is gated and there is a session to send.
        if is_age_gate(&reason) {
            if !crate::services::youtube_auth_service::is_connected() {
                return Err(anyhow!(
                    "'{}' is age-restricted. Connect YouTube in Settings → Profile → Accounts to watch it",
                    channel
                ));
            }
            return self
                .resolve_authed(channel, &video_id, quality)
                .await
                .map_err(|e| {
                    anyhow!(
                        "'{}' is age-restricted and your YouTube account did not unlock it: {}",
                        channel,
                        e
                    )
                });
        }
        Err(anyhow!(
            "couldn't resolve a YouTube stream for '{}': {}",
            channel,
            reason
        ))
    }

    async fn channel_meta(&self, channel: &str) -> Result<ProviderStream> {
        // Cache-first, but only while the entry is FRESH: this row carries
        // `is_live`, and an ageless cache meant a channel whose chat was once
        // opened stayed "live" to the who's-live poller for the whole session.
        // An open chat re-stores meta on every re-resolve, so it still mostly
        // costs nothing here.
        if let Some(meta) = youtube::channel_meta_fresh(channel, META_TTL) {
            return Ok(row_from_meta(channel, &meta));
        }
        // Stale or absent: fetch the live page AND PARSE IT. Fetching alone does
        // not populate the metadata cache; only the chat connect path writes it.
        // Since `live_check` is built on this method, parsing here is what keeps
        // the who's-live poller and the favourites sweep working for channels
        // whose chat was never opened.
        match youtube::refresh_channel_meta(&HTTP, channel).await {
            Ok(meta) => Ok(row_from_meta(channel, &meta)),
            Err(e) => {
                // A failed fetch is not evidence the channel went offline. Prefer
                // the last known answer over reporting "not live", which is what
                // an error becomes by the time it reaches `live_check`.
                if let Some(stale) = youtube::channel_meta(channel) {
                    log::debug!("[YouTube] meta refresh for '{}' failed ({}); using last known", channel, e);
                    return Ok(row_from_meta(channel, &stale));
                }
                Err(e)
            }
        }
    }

    async fn live_check(&self, channels: &[String]) -> Result<Vec<ProviderStream>> {
        // No batch endpoint exists, so this is per-channel and deliberately
        // staggered — a burst of watch-page fetches is exactly what gets an IP
        // challenged. Callers keep the follow list small for the same reason.
        const MAX_PER_SWEEP: usize = 25;
        if channels.len() > MAX_PER_SWEEP {
            // Say so. This cap is the difference between "you follow nobody who is
            // live" and "we looked at 25 of your 254 channels" — and reported as
            // silence, the two are indistinguishable to the user. A subscriptions
            // account reaches this easily, so anyone debugging an empty Following
            // list needs to see it.
            log::warn!(
                "[YouTube] live_check is only checking {} of {} channels this sweep; \
                 the rest are NOT being reported as offline, they are unchecked",
                MAX_PER_SWEEP,
                channels.len(),
            );
        }
        let mut out = Vec::new();
        for channel in channels.iter().take(MAX_PER_SWEEP) {
            if let Ok(row) = self.channel_meta(channel).await {
                if row.is_live {
                    out.push(row);
                }
            }
            tokio::time::sleep(Duration::from_millis(400)).await;
        }
        Ok(out)
    }

    async fn search(&self, query: &str) -> Result<StreamPage> {
        let (streams, cursor) = live_search_page(query, None).await?;
        Ok(StreamPage { streams, cursor })
    }

    /// The signed-in account's subscriptions that are live right now.
    ///
    /// One authenticated `browse` of the subscriptions feed, filtered to rows
    /// carrying a LIVE badge. Deliberately NOT "list the subscribed channels, then
    /// check each": a subscription list runs to hundreds, `live_check` costs a
    /// watch-page fetch per channel, and a burst of those is exactly what gets an
    /// IP challenged (see `live_check`'s own note). One request is both cheaper and
    /// safer.
    ///
    /// The trade-off worth knowing: the feed is ordered by RECENCY, so a stream
    /// that has been live for days sits below today's uploads. That is why the
    /// scan reads deep into the feed rather than stopping at the first screenful.
    async fn followed_live(&self) -> Result<Vec<ProviderStream>> {
        match self.subscriptions_feed().await {
            Ok(rows) => Ok(rows),
            Err(e) if e.to_string().contains("didn't accept the session") => {
                // `subscriptions_feed` already re-harvested the cookies and told us
                // it worked. Use them NOW: returning the error here made the caller
                // fall back to a partial `live_check` and wait a full 120s cadence
                // before trying the good session — so a recovery the user never saw
                // still cost them two minutes of a wrong, empty Following list.
                if crate::services::youtube_auth_service::is_connected() {
                    log::info!("[YouTube] session recovered; re-reading subscriptions now");
                    match self.subscriptions_feed().await {
                        Ok(rows) => return Ok(rows),
                        Err(again) if again.to_string().contains("didn't accept the session") => {
                            // Second promo shell straight after a "successful"
                            // re-harvest. The re-harvest only proves cookies were
                            // FOUND, not that they work — and if the WebView2
                            // profile is itself logged out, it faithfully copies
                            // expired cookies forever.
                            //
                            // `is_connected()` cannot see this: it checks that
                            // SAPISID and APISID exist, never that YouTube honours
                            // them. YouTube never says 401 either; it answers 200
                            // with signed-out content. So without this the app
                            // shows an empty Following list indefinitely and never
                            // tells the user their login expired.
                            log::warn!(
                                "[YouTube] the stored session is not accepted even after \
                                 re-harvesting; marking YouTube disconnected so the account \
                                 row prompts a reconnect"
                            );
                            crate::services::youtube_auth_service::disconnect();
                            return Err(anyhow!(
                                "Your YouTube sign-in has expired — reconnect YouTube in Settings → Profile → Accounts"
                            ));
                        }
                        Err(again) => return Err(again),
                    }
                }
                Err(e)
            }
            Err(e) => Err(e),
        }
    }

    /// YouTube's own games directory (`/gaming/games`), which is a REAL category
    /// list with box art and live viewer counts, not something synthesised here.
    ///
    /// Read off the page rather than through InnerTube `browse`: the API call for
    /// the same surface comes back empty (measured), while the page carries the
    /// full `gameCardRenderer` grid. `FEgaming` is a 400.
    async fn categories(&self, _cursor: Option<&str>, limit: u32) -> Result<CategoryPage> {
        let html = youtube::fetch_youtube_html(&HTTP, GAMES_URL, "gaming").await?;
        let data = youtube::extract_json(&html, "ytInitialData")
            .ok_or_else(|| anyhow!("couldn't read YouTube's games directory"))?;
        let mut categories = categories_from_game_cards(&data);
        if categories.is_empty() {
            return Err(anyhow!("YouTube's games directory had no entries"));
        }
        categories.truncate(limit.clamp(1, 100) as usize);
        Ok(CategoryPage {
            categories,
            // The page ships one grid; there is no cursor to continue it with.
            cursor: None,
        })
    }

    async fn directory(
        &self,
        category: Option<&str>,
        cursor: Option<&str>,
        limit: u32,
    ) -> Result<StreamPage> {
        let mut page = match category {
            // A category id is a game's TOPIC channel. Its `/live` page is the
            // per-game live grid, which is the closest thing YouTube has to
            // Twitch's "streams in this category".
            Some(id) if is_channel_id_str(id) => {
                let url = format!("https://www.youtube.com/channel/{}/live", id);
                let html = youtube::fetch_youtube_html(&HTTP, &url, id).await?;
                let data = youtube::extract_json(&html, "ytInitialData")
                    .ok_or_else(|| anyhow!("couldn't read the live grid for '{}'", id))?;
                StreamPage {
                    streams: rows_from_renderers(&data),
                    cursor: None,
                }
            }
            // No category (or an id we don't recognise): "what's live" overall,
            // which on YouTube is a search for live content. One search page is
            // only ~20 rows, so follow continuations until the caller's limit is
            // met — a Discover grid asking for 100 should get 100, not 20.
            _ => {
                let want = limit.clamp(1, MAX_DIRECTORY_ROWS) as usize;
                let mut streams: Vec<ProviderStream> = Vec::new();
                let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
                let mut token = cursor.map(String::from);
                for _ in 0..MAX_SEARCH_PAGES {
                    let (rows, next) = live_search_page("live", token.as_deref()).await?;
                    let fetched = rows.len();
                    // Pages can overlap as the live set shifts under us.
                    for row in rows {
                        if seen.insert(row.id.clone()) {
                            streams.push(row);
                        }
                    }
                    token = next;
                    // Stop on: enough rows, no continuation, or a page that added
                    // nothing (which would otherwise spin through every page).
                    if streams.len() >= want || token.is_none() || fetched == 0 {
                        break;
                    }
                }
                StreamPage {
                    streams,
                    cursor: token,
                }
            }
        };
        page.streams.truncate(limit.clamp(1, MAX_DIRECTORY_ROWS) as usize);
        Ok(page)
    }
}

impl YouTubeSource {
    /// One read of the signed-in account's subscriptions feed.
    async fn subscriptions_feed(&self) -> Result<Vec<ProviderStream>> {
        let headers = crate::services::youtube_auth_service::auth_headers()
            .ok_or_else(|| anyhow!("not signed in to YouTube"))?;

        let body = json!({
            "browseId": "FEsubscriptions",
            "context": { "client": { "clientName": "WEB", "clientVersion": "2.20240101.00.00", "hl": "en", "gl": "US" } }
        });
        let mut req = HTTP
            .post(format!("{}/browse?key={}", INNERTUBE, INNERTUBE_KEY))
            .header(reqwest::header::CONTENT_TYPE, "application/json");
        for (k, v) in headers {
            req = req.header(k, v);
        }
        let resp = req.json(&body).send().await?;
        if !resp.status().is_success() {
            return Err(anyhow!("YouTube subscriptions HTTP {}", resp.status()));
        }
        // Keep the raw body: its length is what the log line below reports.
        // Re-serializing the parsed tree just to count bytes cost a second
        // 1-2 MB encode on every poll of a feed that is fetched every two minutes.
        let body = resp.bytes().await?;
        let json: Value = serde_json::from_slice(&body)?;

        // A signed-OUT request still returns 200 WITH a `contents` tree, so
        // "did we get contents" cannot tell a stale session from a quiet one and an
        // expired login would silently read as "nothing live" forever. What YouTube
        // actually swaps in is a `backgroundPromoRenderer` ("Sign in to see updates
        // from your favorite YouTube channels"), verified against a live anonymous
        // request, so that marker is the honest signal.
        let mut rows = rows_from_renderers(&json);
        let bytes = body.len();
        drop(body);
        // One walk of the tree per poll. The item count, the promo probe and
        // the renderer histogram are diagnostics for the empty-feed case; the
        // old code walked the whole tree four times every two minutes and
        // logged a histogram at INFO each time. On 2026-09-06 the decoded feed
        // was ~7.2 MB per poll (112 subscriptions), so that was the largest
        // idle cost in the app for nothing anyone read.
        if rows.is_empty() {
            let content_items = count_feed_items(&json);
            let promo = contains_key(&json, "backgroundPromoRenderer");
            log::info!(
                "[YouTube] subscriptions feed: {} content item(s), none live, promo={}, has_contents={}, bytes~{}; renderers: {}",
                content_items,
                promo,
                json.get("contents").is_some(),
                bytes,
                renderer_histogram(&json),
            );
            if content_items == 0 && promo {
                let verdict = crate::services::youtube_auth_service::validate_session().await;
                log::warn!(
                    "[YouTube] empty subscriptions feed with a promo shell. Auth probe on \
                     account_menu says: {}",
                    match verdict {
                        Some(true) => "AUTH OK — the session works, so this is the browse, not the login",
                        Some(false) => "REJECTED — the session is genuinely dead; reconnect YouTube",
                        None => "inconclusive",
                    }
                );
                // Signed-out content behind a 200. The WebView2 profile is usually still
                // logged in and only the harvested cookies went stale, so try to recover
                // them; the next sweep then succeeds on its own.
                crate::services::youtube_auth_service::recover_stale_session().await;
                return Err(anyhow!(
                    "YouTube didn't accept the session (sign in to YouTube again)"
                ));
            }
        } else {
            log::debug!("[YouTube] subscriptions feed: {} live, bytes~{}", rows.len(), bytes);
        }
        let mut seen = std::collections::HashSet::new();
        rows.retain(|r| seen.insert(r.id.clone()));
        Ok(rows)
    }
}

/// Ceiling on rows one directory call will return, and on how many search pages it
/// will walk to get there. Each page is a network round trip, so this bounds the
/// cost of a large `limit` rather than letting it fan out unchecked.
const MAX_DIRECTORY_ROWS: u32 = 200;
const MAX_SEARCH_PAGES: usize = 12;

/// One page of the live search. Returns its rows plus the token for the next page.
///
/// `EgJAAQ==` is YouTube's "Live" search filter. A continuation request sends ONLY
/// the token (no query/params) — that is the shape the endpoint expects, and
/// sending both gets the first page back again.
async fn live_search_page(
    query: &str,
    continuation: Option<&str>,
) -> Result<(Vec<ProviderStream>, Option<String>)> {
    let context = json!({
        "client": { "clientName": "WEB", "clientVersion": "2.20240101.00.00", "hl": "en", "gl": "US" }
    });
    let body = match continuation {
        Some(token) => json!({ "continuation": token, "context": context }),
        None => json!({ "query": query, "params": "EgJAAQ==", "context": context }),
    };
    let resp = HTTP
        .post(format!("{}/search?key={}", INNERTUBE, INNERTUBE_KEY))
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .json(&body)
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(anyhow!("YouTube search HTTP {}", resp.status()));
    }
    let json: Value = resp.json().await?;
    Ok((rows_from_renderers(&json), continuation_token(&json)))
}

/// The next-page token anywhere in a response.
fn continuation_token(root: &Value) -> Option<String> {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        match node {
            Value::Object(map) => {
                if let Some(t) = map
                    .get("continuationCommand")
                    .and_then(|c| c.get("token"))
                    .and_then(|t| t.as_str())
                {
                    return Some(t.to_string());
                }
                stack.extend(map.values());
            }
            Value::Array(items) => stack.extend(items.iter()),
            _ => {}
        }
    }
    None
}

/// YouTube's games directory. A plain page fetch, for the reason in `categories`.
const GAMES_URL: &str = "https://www.youtube.com/gaming/games";

fn is_channel_id_str(id: &str) -> bool {
    id.len() == 24 && id.starts_with("UC")
}

/// Every `gameCardRenderer` in the games directory, as categories.
///
/// Each card carries exactly what a category grid needs and nothing has to be
/// invented: the game's name, its official box art, the topic channel that lists
/// its streams, and how many people are watching it worldwide.
fn categories_from_game_cards(root: &Value) -> Vec<ProviderCategory> {
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        match node {
            Value::Object(map) => {
                if let Some(card) = map.get("gameCardRenderer") {
                    if let Some(c) = category_from_game_card(card) {
                        out.push(c);
                    }
                }
                stack.extend(map.values());
            }
            Value::Array(items) => stack.extend(items.iter()),
            _ => {}
        }
    }
    out
}

fn category_from_game_card(card: &Value) -> Option<ProviderCategory> {
    let details = card.pointer("/game/gameDetailsRenderer")?;
    let name = details
        .pointer("/title/simpleText")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())?
        .to_string();
    // The topic channel, which is what `directory` lists streams from.
    let id = details
        .pointer("/endpoint/browseEndpoint/browseId")
        .and_then(|v| v.as_str())
        .filter(|id| is_channel_id_str(id))?
        .to_string();
    // Box art urls come back PROTOCOL-RELATIVE ("//yt3.googleusercontent.com/..."),
    // which renders as a broken image in the webview. Give them a scheme.
    let thumbnail = details
        .pointer("/boxArt/thumbnails/0/url")
        .and_then(|v| v.as_str())
        .map(|u| {
            if let Some(rest) = u.strip_prefix("//") {
                format!("https://{}", rest)
            } else {
                u.to_string()
            }
        })
        .unwrap_or_default();
    // "86K watching worldwide" arrives split across runs, so join before parsing.
    let viewer_count = details
        .pointer("/liveViewersText/runs")
        .and_then(|r| r.as_array())
        .map(|runs| {
            runs.iter()
                .filter_map(|r| r.get("text").and_then(|t| t.as_str()))
                .collect::<String>()
        })
        .map(|s| parse_count(&s))
        .unwrap_or(0);

    Some(ProviderCategory {
        provider: "youtube".to_string(),
        id,
        name,
        thumbnail,
        viewer_count,
        // YouTube reports viewers for a game, not how many channels are live in it.
        channel_count: 0,
    })
}

fn row_from_meta(channel: &str, meta: &youtube::YouTubeChannelMeta) -> ProviderStream {
    let login = channel.to_string();
    ProviderStream {
        provider: "youtube".to_string(),
        key: make_key("youtube", &login),
        id: String::new(),
        user_id: meta.user_id.clone().unwrap_or_default(),
        user_login: login.clone(),
        user_name: meta.username.clone().unwrap_or_else(|| login.clone()),
        title: meta.title.clone().unwrap_or_default(),
        viewer_count: meta.viewer_count.unwrap_or(0) as u32,
        game_id: String::new(),
        // YouTube infers the category and often attaches none, so this stays
        // empty rather than guessing; the UI already handles a blank category.
        game_name: meta.game_name.clone().unwrap_or_default(),
        category_thumbnail: None,
        thumbnail_url: String::new(),
        started_at: meta.start_time.clone().unwrap_or_default(),
        profile_image_url: meta.profile_pic.clone(),
        is_live: meta.is_live,
        watch_url: format!("https://www.youtube.com/watch?v={}", login),
        tags: None,
    }
}

/// Both leaf keys are accepted because the same video row is named differently by
/// layout: search returns `videoRenderer`, while a grid feed (subscriptions) can
/// return `gridVideoRenderer` with the same fields. The walk recurses through
/// everything, so the wrapping containers (`richItemRenderer`, `richSectionRenderer`,
/// shelves) need no cases of their own.
const VIDEO_RENDERER_KEYS: [&str; 2] = ["videoRenderer", "gridVideoRenderer"];

/// How many video/lockup CONTENT items the feed carried, regardless of liveness.
///
/// Separates "the account has nothing live" from "the response has no feed at
/// all". Only the second is evidence of a session problem; the first is a normal
/// answer and must not trigger a re-harvest.
fn count_feed_items(root: &Value) -> usize {
    let mut count = 0usize;
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        match node {
            Value::Object(map) => {
                for key in VIDEO_RENDERER_KEYS {
                    if map.contains_key(key) {
                        count += 1;
                    }
                }
                // Only real video lockups count; a lockup can also be a playlist
                // or a shelf, and those say nothing about whether the feed loaded.
                if let Some(lk) = map.get("lockupViewModel") {
                    if lk.get("contentType").and_then(|v| v.as_str())
                        == Some("LOCKUP_CONTENT_TYPE_VIDEO")
                    {
                        count += 1;
                    }
                }
                stack.extend(map.values());
            }
            Value::Array(items) => stack.extend(items.iter()),
            _ => {}
        }
    }
    count
}

/// A compact `name xN` census of every `*Renderer` / `*ViewModel` in a response,
/// most frequent first. Purely diagnostic: it turns an authed response nobody here
/// can reproduce into one readable log line.
fn renderer_histogram(root: &Value) -> String {
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        match node {
            Value::Object(map) => {
                for (k, v) in map {
                    if k.ends_with("Renderer") || k.ends_with("ViewModel") {
                        *counts.entry(k.as_str()).or_insert(0) += 1;
                    }
                    stack.push(v);
                }
            }
            Value::Array(items) => stack.extend(items.iter()),
            _ => {}
        }
    }
    if counts.is_empty() {
        return "none (empty response)".to_string();
    }
    let mut pairs: Vec<_> = counts.into_iter().collect();
    pairs.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    pairs
        .iter()
        .take(12)
        .map(|(k, n)| format!("{} x{}", k, n))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Whether `key` appears anywhere in the response tree. Used to spot marker
/// renderers whose nesting YouTube is free to move.
fn contains_key(root: &Value, key: &str) -> bool {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        match node {
            Value::Object(map) => {
                if map.contains_key(key) {
                    return true;
                }
                stack.extend(map.values());
            }
            Value::Array(items) => stack.extend(items.iter()),
            _ => {}
        }
    }
    false
}

/// Every LIVE row anywhere in an InnerTube response.
///
/// TWO generations of row shape are read, because YouTube is mid-migration and
/// which one you get depends on the surface: the classic `*Renderer` rows (still
/// what search returns) and `lockupViewModel`, which is what the SUBSCRIPTIONS
/// feed returns now.
///
/// Reading only the classic shape makes the followed-live list come back empty
/// while the account has plenty live: a subscriptions feed can carry lockups and
/// zero `videoRenderer`, so the walk goes straight past every row.
///
/// The walk recurses through everything, so the wrapping containers
/// (`richItemRenderer`, `richSectionRenderer`, shelves) need no cases of their own.
fn rows_from_renderers(root: &Value) -> Vec<ProviderStream> {
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        match node {
            Value::Object(map) => {
                for key in VIDEO_RENDERER_KEYS {
                    if let Some(row) = map.get(key).and_then(row_from_renderer) {
                        out.push(row);
                    }
                }
                if let Some(row) = map.get("lockupViewModel").and_then(row_from_lockup) {
                    out.push(row);
                }
                for v in map.values() {
                    stack.push(v);
                }
            }
            Value::Array(items) => stack.extend(items.iter()),
            _ => {}
        }
    }
    out
}

/// One `lockupViewModel` as a live stream row, or None when it isn't a live video.
///
/// Liveness comes from the thumbnail BADGE STYLE
/// (`THUMBNAIL_OVERLAY_BADGE_STYLE_LIVE`) and nothing else. Titles are not usable:
/// a channel that streams keeps "LIVE" in the title of every past broadcast, so
/// matching on text reports a stream that ended two days ago as live. An ended one
/// carries a duration badge in the DEFAULT style instead. Both shapes were read off
/// the same channel's feed.
fn row_from_lockup(lk: &Value) -> Option<ProviderStream> {
    if lk.get("contentType").and_then(|v| v.as_str()) != Some("LOCKUP_CONTENT_TYPE_VIDEO") {
        return None;
    }
    let video_id = lk.get("contentId").and_then(|v| v.as_str())?;
    if video_id.is_empty() || !has_live_badge(lk) {
        return None;
    }

    let meta = lk.pointer("/metadata/lockupMetadataViewModel");
    let title = meta
        .and_then(|m| m.pointer("/title/content"))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();

    // Metadata rows run [channel name] then [viewers]. Read them by CONTENT rather
    // than by index: a surface that omits the author row (a channel's own tab,
    // where it would be redundant) shifts every other row up.
    let parts: Vec<&str> = meta
        .and_then(|m| m.pointer("/metadata/contentMetadataViewModel/metadataRows"))
        .and_then(|r| r.as_array())
        .map(|rows| {
            rows.iter()
                .filter_map(|r| r.get("metadataParts").and_then(|p| p.as_array()))
                .flatten()
                .filter_map(|p| p.pointer("/text/content").and_then(|c| c.as_str()))
                .collect()
        })
        .unwrap_or_default();
    let viewer_count = parts
        .iter()
        .find(|p| p.contains("watching"))
        .map(|p| parse_count(p))
        .unwrap_or(0);
    let author = parts
        .iter()
        .find(|p| !p.contains("watching") && !p.contains("views") && !p.contains("ago"))
        .map(|p| p.to_string())
        .unwrap_or_default();

    let thumbnail = lk
        .pointer("/contentImage/thumbnailViewModel/image/sources")
        .and_then(|s| s.as_array())
        .and_then(|arr| arr.last())
        .and_then(|t| t.get("url"))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    // The channel avatar, so a card isn't bare. Lockups nest it under the newer
    // view-model chain rather than the classic `channelThumbnail...` renderer.
    let avatar = meta.and_then(|m| {
        largest_url(m.pointer("/image/decoratedAvatarViewModel/avatar/avatarViewModel/image/sources"))
    });

    Some(ProviderStream {
        provider: "youtube".to_string(),
        key: make_key("youtube", video_id),
        id: video_id.to_string(),
        user_id: first_channel_id(lk).unwrap_or_default(),
        // Addressed by video id: that is what resolves to this broadcast.
        user_login: video_id.to_string(),
        user_name: if author.is_empty() { title.clone() } else { author },
        title,
        viewer_count,
        game_id: String::new(),
        game_name: String::new(),
        category_thumbnail: None,
        thumbnail_url: thumbnail,
        started_at: String::new(),
        profile_image_url: avatar,
        is_live: true,
        watch_url: format!("https://www.youtube.com/watch?v={}", video_id),
        tags: None,
    })
}

/// The largest image url in a YouTube `thumbnails` / `sources` array (they are
/// ordered smallest first). None when the array is absent or carries no url, so a
/// caller gets `None` rather than an empty string that would render a broken image.
fn largest_url(arr: Option<&Value>) -> Option<String> {
    let url = arr?
        .as_array()?
        .last()?
        .get("url")?
        .as_str()?;
    (!url.is_empty()).then(|| url.to_string())
}

/// Whether any thumbnail badge in this subtree marks the video as live now.
fn has_live_badge(lk: &Value) -> bool {
    let mut stack = vec![lk];
    while let Some(node) = stack.pop() {
        match node {
            Value::Object(map) => {
                if let Some(style) = map
                    .get("thumbnailBadgeViewModel")
                    .and_then(|b| b.get("badgeStyle"))
                    .and_then(|s| s.as_str())
                {
                    if style.contains("LIVE") {
                        return true;
                    }
                }
                stack.extend(map.values());
            }
            Value::Array(items) => stack.extend(items.iter()),
            _ => {}
        }
    }
    false
}

/// The first channel id referenced inside a lockup, which is its author.
fn first_channel_id(lk: &Value) -> Option<String> {
    let mut stack = vec![lk];
    while let Some(node) = stack.pop() {
        match node {
            Value::Object(map) => {
                if let Some(id) = map
                    .get("browseEndpoint")
                    .and_then(|e| e.get("browseId"))
                    .and_then(|v| v.as_str())
                {
                    if id.len() == 24 && id.starts_with("UC") {
                        return Some(id.to_string());
                    }
                }
                stack.extend(map.values());
            }
            Value::Array(items) => stack.extend(items.iter()),
            _ => {}
        }
    }
    None
}

/// Parse a YouTube count, which is ABBREVIATED on the newer surfaces:
/// "481 watching" -> 481, "11K watching" -> 11000, "1.2M views" -> 1200000.
///
/// A digits-only read turns "11K watching" into 11, which would rank a huge stream
/// below a tiny one everywhere viewer count is used for sorting.
fn parse_count(s: &str) -> u32 {
    let mut num = String::new();
    let mut suffix = 0u8;
    for c in s.chars() {
        if c.is_ascii_digit() {
            num.push(c);
        } else if c == ',' && !num.is_empty() {
            // Thousands separator in the UNABBREVIATED form ("1,234 watching"),
            // which the classic renderers still use. Dropping it is why this can't
            // just stop at the first non-digit.
            continue;
        } else if c == '.' && !num.is_empty() && !num.contains('.') {
            num.push(c);
        } else if num.is_empty() {
            continue; // leading words before the number
        } else {
            // First character past the number. Only a letter glued straight to it
            // scales it ("11K"); a space means the number already ended.
            if c.is_ascii_alphabetic() {
                suffix = c.to_ascii_uppercase() as u8;
            }
            break;
        }
    }
    let base: f64 = num.parse().unwrap_or(0.0);
    let scaled = match suffix {
        b'K' => base * 1_000.0,
        b'M' => base * 1_000_000.0,
        b'B' => base * 1_000_000_000.0,
        _ => base,
    };
    scaled.clamp(0.0, u32::MAX as f64) as u32
}

/// One classic `videoRenderer` row as a live stream, or None when it is not live.
///
/// The shape is deeply nested and differs between surfaces, so callers walk the
/// tree for these rather than hard-coding a path.
fn row_from_renderer(r: &Value) -> Option<ProviderStream> {
    let video_id = r.get("videoId").and_then(|v| v.as_str())?;
    // Only LIVE entries belong in a live grid; the badge is how YouTube marks them.
    let is_live = r
        .pointer("/badges")
        .and_then(|b| b.as_array())
        .map(|badges| {
            badges.iter().any(|b| {
                b.pointer("/metadataBadgeRenderer/style")
                    .and_then(|s| s.as_str())
                    .map(|s| s.contains("LIVE"))
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
        || r.pointer("/thumbnailOverlays")
            .map(|o| o.to_string().contains("LIVE"))
            .unwrap_or(false);
    if !is_live {
        return None;
    }
    let title = r
        .pointer("/title/runs/0/text")
        .or_else(|| r.pointer("/title/simpleText"))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let author = r
        .pointer("/ownerText/runs/0/text")
        .or_else(|| r.pointer("/longBylineText/runs/0/text"))
        // A game topic channel's live grid carries only the short byline.
        .or_else(|| r.pointer("/shortBylineText/runs/0/text"))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    // "1,234 watching" -> 1234, "11K watching" -> 11000.
    let viewers = r
        .pointer("/viewCountText/runs/0/text")
        .or_else(|| r.pointer("/viewCountText/simpleText"))
        .and_then(|v| v.as_str())
        .map(parse_count)
        .unwrap_or(0);
    let thumbnail = r
        .pointer("/thumbnail/thumbnails")
        .and_then(|t| t.as_array())
        .and_then(|arr| arr.last())
        .and_then(|t| t.get("url"))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let avatar = largest_url(
        r.pointer("/channelThumbnailSupportedRenderers/channelThumbnailWithLinkRenderer/thumbnail/thumbnails"),
    );
    // The owning channel. A category grid ships no avatar, so this id is what lets
    // one be resolved later for the cards actually on screen.
    let channel_id = r
        .pointer("/shortBylineText/runs/0/navigationEndpoint/browseEndpoint/browseId")
        .or_else(|| r.pointer("/ownerText/runs/0/navigationEndpoint/browseEndpoint/browseId"))
        .or_else(|| r.pointer("/longBylineText/runs/0/navigationEndpoint/browseEndpoint/browseId"))
        .and_then(|v| v.as_str())
        .filter(|id| is_channel_id_str(id))
        .unwrap_or_default()
        .to_string();

    Some(ProviderStream {
        provider: "youtube".to_string(),
        key: make_key("youtube", video_id),
        id: video_id.to_string(),
        user_id: channel_id,
        // Addressed by video id: that is what resolves to this broadcast.
        user_login: video_id.to_string(),
        user_name: if author.is_empty() { title.clone() } else { author },
        title,
        viewer_count: viewers,
        game_id: String::new(),
        game_name: String::new(),
        category_thumbnail: None,
        thumbnail_url: thumbnail,
        started_at: String::new(),
        profile_image_url: avatar,
        is_live: true,
        watch_url: format!("https://www.youtube.com/watch?v={}", video_id),
        tags: None,
    })
}

// ---------------------------------------------------------------------------
// SABR session construction
// ---------------------------------------------------------------------------

/// Everything a SABR session needs, all scraped from ONE watch page.
///
/// They must come from one page on purpose: the abr url, the ustreamer config
/// and the visitor identity the PO token binds to are minted together, and
/// mixing them across sessions is refused with a 403. Measured 2026-08-21.
pub struct SabrInputs {
    pub session: crate::services::youtube_sabr::SabrSession,
    /// What the menu should call this rendition.
    pub name: String,
    pub video_itag: u32,
    pub audio_itag: u32,
}

/// Build a SABR session for a video, choosing the tallest servable rendition.
///
/// Audio is deliberately AAC-in-MP4 (itag 140) rather than the Opus-in-WebM the
/// browser picks: SABR hands both back as-is, and only the MP4 one can be served
/// to hls.js without another transmux.
pub async fn sabr_session_for(video_id: &str) -> Result<SabrInputs> {
    use crate::services::youtube_sabr::{FormatId, SabrSession};

    let url = format!(
        "https://www.youtube.com/watch?v={}&bpctr=9999999999&has_verified=1",
        video_id
    );
    let html = HTTP
        .get(&url)
        .header(reqwest::header::COOKIE, "PREF=hl=en&tz=UTC; SOCS=CAI")
        .send()
        .await?
        .text()
        .await?;

    let player = youtube::extract_json(&html, "ytInitialPlayerResponse")
        .ok_or_else(|| anyhow!("watch page carried no player response"))?;

    let abr_url = player
        .pointer("/streamingData/serverAbrStreamingUrl")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("no serverAbrStreamingUrl; this broadcast is not on SABR"))?
        .to_string();

    let cfg_b64 = youtube::json_str_after(&html, "\"videoPlaybackUstreamerConfig\":\"")
        .map(|v| youtube::decode_json_escapes(&v))
        .ok_or_else(|| anyhow!("no videoPlaybackUstreamerConfig"))?;
    let ustreamer_config = {
        use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
        use base64::Engine;
        URL_SAFE_NO_PAD
            .decode(cfg_b64.trim_end_matches('='))
            .or_else(|_| STANDARD.decode(&cfg_b64))
            .map_err(|e| anyhow!("ustreamer config was not base64: {}", e))?
    };

    let visitor_data = youtube::json_str_after(&html, "\"visitorData\":\"")
        .map(|v| youtube::decode_json_escapes(&v))
        .ok_or_else(|| anyhow!("no visitorData to bind the token to"))?;
    let client_version = youtube::json_str_after(&html, "\"INNERTUBE_CLIENT_VERSION\":\"")
        .unwrap_or_else(|| "2.20260820.08.00".to_string());

    let formats = player
        .pointer("/streamingData/adaptiveFormats")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow!("no adaptiveFormats"))?;

    let as_format = |f: &Value| FormatId {
        itag: f.get("itag").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
        // The server checks this against what it advertised, so a format id
        // without it is rejected even when the itag is right.
        last_modified: f
            .get("lastModified")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<u64>().ok())
            .or_else(|| f.get("lastModified").and_then(|v| v.as_u64()))
            .unwrap_or(0),
        xtags: f.get("xtags").and_then(|v| v.as_str()).map(String::from),
    };

    // Tallest video by the SHORT edge, so a vertical 1080x1920 stream is not
    // mistaken for a 1920p one.
    let video = formats
        .iter()
        .filter(|f| f.get("height").is_some())
        .max_by_key(|f| {
            let h = f.get("height").and_then(|v| v.as_u64()).unwrap_or(0);
            let w = f.get("width").and_then(|v| v.as_u64()).unwrap_or(0);
            let short = if w > 0 { h.min(w) } else { h };
            (short, f.get("bitrate").and_then(|v| v.as_u64()).unwrap_or(0))
        })
        .ok_or_else(|| anyhow!("no video formats"))?;

    let audio = formats
        .iter()
        .filter(|f| {
            f.get("height").is_none()
                && f.get("mimeType")
                    .and_then(|m| m.as_str())
                    .is_some_and(|m| m.starts_with("audio/mp4"))
        })
        .max_by_key(|f| f.get("bitrate").and_then(|v| v.as_u64()).unwrap_or(0))
        .ok_or_else(|| anyhow!("no AAC audio track to pair with"))?;

    let height = video.get("height").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let width = video.get("width").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let fps = video.get("fps").and_then(|v| v.as_f64()).unwrap_or(30.0);
    let short = if width > 0 { height.min(width) } else { height };
    let name = if fps >= 50.0 {
        format!("{}p{}", short, fps.round() as u32)
    } else {
        format!("{}p", short)
    };

    let v = as_format(video);
    let a = as_format(audio);
    log::info!(
        "[YouTubeSabr] session for '{}': {} video itag {} + audio itag {}, config {} B, binding {} chars",
        video_id,
        name,
        v.itag,
        a.itag,
        ustreamer_config.len(),
        visitor_data.len()
    );

    Ok(SabrInputs {
        video_itag: v.itag,
        audio_itag: a.itag,
        name,
        session: SabrSession::new(
            abr_url,
            ustreamer_config,
            v,
            a,
            width,
            height,
            client_version,
            visitor_data,
        ),
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn age_gate_reasons_are_recognised() {
        // Both wordings YouTube uses, as measured 2026-09-03 on a gated broadcast.
        assert!(super::is_age_gate(
            "This video may be inappropriate for some users."
        ));
        assert!(super::is_age_gate("Sign in to confirm your age"));
        assert!(super::is_age_gate("AGE_VERIFICATION_REQUIRED"));
        // The bot check also says "sign in" and must NOT trigger a signed-in retry.
        assert!(!super::is_age_gate(
            "Sign in to confirm you\u{2019}re not a bot"
        ));
        assert!(!super::is_age_gate("This video is unavailable"));
        assert!(!super::is_age_gate(
            "Join this channel to get access to members-only content"
        ));
    }

    use super::*;

    /// A signed-OUT subscriptions request returns 200 WITH a `contents` tree, so
    /// only the promo marker distinguishes it from a signed-in account that simply
    /// has nothing live. Shape captured from a real anonymous request.
    #[test]
    fn spots_the_signed_out_subscriptions_shell() {
        let signed_out = json!({
            "contents": { "twoColumnBrowseResultsRenderer": { "tabs": [ { "tabRenderer": {
                "content": { "sectionListRenderer": { "contents": [ { "itemSectionRenderer": {
                    "contents": [ { "backgroundPromoRenderer": {
                        "title": { "runs": [ { "text": "Don't miss new videos" } ] },
                        "bodyText": { "runs": [ { "text":
                            "Sign in to see updates from your favorite YouTube channels" } ] }
                    } } ]
                } } ] } }
            } } ] } }
        });
        assert!(contains_key(&signed_out, "backgroundPromoRenderer"));
        assert!(rows_from_renderers(&signed_out).is_empty());

        // A signed-in feed with nothing live also yields no rows, but carries NO
        // promo, so the two are distinguishable and only one is an error.
        let quiet = json!({ "contents": { "twoColumnBrowseResultsRenderer": { "tabs": [] } } });
        assert!(!contains_key(&quiet, "backgroundPromoRenderer"));
        assert!(rows_from_renderers(&quiet).is_empty());
    }

    /// Shapes captured VERBATIM from the real authed subscriptions feed. The feed
    /// returns `lockupViewModel`, not `videoRenderer`, which is why the followed-live
    /// list came back empty while the account had streams live.
    #[test]
    fn parses_a_live_lockup_from_the_subscriptions_feed() {
        let live = json!({ "contents": { "lockupViewModel": {
            "contentId": "xtlt8BVmawk",
            "contentType": "LOCKUP_CONTENT_TYPE_VIDEO",
            "contentImage": { "thumbnailViewModel": {
                "image": { "sources": [
                    { "url": "https://i.ytimg.com/vi/xtlt8BVmawk/small.jpg" },
                    { "url": "https://i.ytimg.com/vi/xtlt8BVmawk/large.jpg" }
                ] },
                "overlays": [ { "thumbnailBottomOverlayViewModel": { "badges": [
                    { "thumbnailBadgeViewModel": { "text": "LIVE",
                      "badgeStyle": "THUMBNAIL_OVERLAY_BADGE_STYLE_LIVE" } }
                ] } } ]
            } },
            "metadata": { "lockupMetadataViewModel": {
                "title": { "content": "100 HOUR CS2 MARATHON" },
                "image": { "decoratedAvatarViewModel": {} },
                "metadata": { "contentMetadataViewModel": { "metadataRows": [
                    { "metadataParts": [ { "text": { "content": "Ludwig" } } ] },
                    { "metadataParts": [ { "text": { "content": "11K watching" } } ] }
                ] } }
            } },
            "rendererContext": { "commandContext": { "onTap": { "innertubeCommand": {
                "browseEndpoint": { "browseId": "UCrPseYLGpNygVi34QpGNqpA" } } } } }
        } } });
        let rows = rows_from_renderers(&live);
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!(r.id, "xtlt8BVmawk");
        assert_eq!(r.user_login, "xtlt8BVmawk", "addressed by video id");
        assert_eq!(r.user_name, "Ludwig", "the channel, not the title");
        assert_eq!(r.title, "100 HOUR CS2 MARATHON");
        assert_eq!(r.viewer_count, 11_000, "abbreviated counts must scale");
        assert_eq!(r.user_id, "UCrPseYLGpNygVi34QpGNqpA");
        assert_eq!(r.thumbnail_url, "https://i.ytimg.com/vi/xtlt8BVmawk/large.jpg");
        assert!(r.is_live);
    }

    /// An ENDED broadcast keeps "LIVE" in its title and differs only by badge
    /// style, so title text must never be the liveness signal. Captured from the
    /// same channel's feed, where a two-day-old VOD still reads "LIVE | ...".
    #[test]
    fn an_ended_stream_is_not_live_despite_its_title() {
        let ended = json!({ "contents": { "lockupViewModel": {
            "contentId": "1YRBlxS43xY",
            "contentType": "LOCKUP_CONTENT_TYPE_VIDEO",
            "contentImage": { "thumbnailViewModel": { "overlays": [
                { "thumbnailBottomOverlayViewModel": { "badges": [
                    { "thumbnailBadgeViewModel": { "text": "9:02:35",
                      "badgeStyle": "THUMBNAIL_OVERLAY_BADGE_STYLE_DEFAULT" } }
                ] } }
            ] } },
            "metadata": { "lockupMetadataViewModel": {
                "title": { "content": "LIVE | ESCAPE FROM TARKOV" },
                "metadata": { "contentMetadataViewModel": { "metadataRows": [
                    { "metadataParts": [ { "text": { "content": "586K views" } },
                                         { "text": { "content": "Streamed 18 hours ago" } } ] }
                ] } }
            } }
        } } });
        assert!(rows_from_renderers(&ended).is_empty());
    }

    /// A playlist or channel lockup is not a stream.
    #[test]
    fn ignores_non_video_lockups() {
        let playlist = json!({ "lockupViewModel": {
            "contentId": "PL1234567890",
            "contentType": "LOCKUP_CONTENT_TYPE_PLAYLIST",
            "contentImage": { "thumbnailViewModel": { "overlays": [
                { "thumbnailBottomOverlayViewModel": { "badges": [
                    { "thumbnailBadgeViewModel": { "badgeStyle": "THUMBNAIL_OVERLAY_BADGE_STYLE_LIVE" } }
                ] } }
            ] } }
        } });
        assert!(rows_from_renderers(&playlist).is_empty());
    }

    /// Shape captured VERBATIM from /gaming/games. Every field is YouTube's own:
    /// name, official box art, the topic channel that lists the game's streams, and
    /// worldwide live viewers. Nothing here is synthesised.
    #[test]
    fn reads_a_game_card_as_a_category() {
        let page = json!({ "contents": [ { "gameCardRenderer": { "game": {
            "gameDetailsRenderer": {
                "title": { "simpleText": "Dota 2" },
                "boxArt": { "thumbnails": [
                    { "url": "//yt3.googleusercontent.com/TJ0klyMkeKSw22EPOqqGcQ" }
                ] },
                "endpoint": { "browseEndpoint": {
                    "browseId": "UCjkem1Rik-q4xKeETu9geUw",
                    "params": "EgRsaXZl",
                    "canonicalBaseUrl": "/channel/UCjkem1Rik-q4xKeETu9geUw"
                } },
                "liveViewersText": { "runs": [
                    { "text": "86K" }, { "text": " watching worldwide" }
                ] }
            }
        } } } ] });
        let cats = categories_from_game_cards(&page);
        assert_eq!(cats.len(), 1);
        let c = &cats[0];
        assert_eq!(c.name, "Dota 2");
        assert_eq!(c.id, "UCjkem1Rik-q4xKeETu9geUw", "the topic channel is the id");
        assert_eq!(c.viewer_count, 86_000, "runs are joined before parsing");
        assert_eq!(
            c.thumbnail, "https://yt3.googleusercontent.com/TJ0klyMkeKSw22EPOqqGcQ",
            "protocol-relative box art would render broken in the webview",
        );
    }

    /// A malformed card contributes nothing rather than a blank tile.
    #[test]
    fn skips_game_cards_missing_their_essentials() {
        let bad = json!({ "contents": [
            { "gameCardRenderer": { "game": { "gameDetailsRenderer": {
                "title": { "simpleText": "No endpoint" } } } } },
            { "gameCardRenderer": { "game": { "gameDetailsRenderer": {
                "endpoint": { "browseEndpoint": { "browseId": "UCjkem1Rik-q4xKeETu9geUw" } } } } } }
        ] });
        assert!(categories_from_game_cards(&bad).is_empty());
    }

    /// A game's live grid uses the CLASSIC gridVideoRenderer, and carries only the
    /// short byline for the channel. Captured from a topic channel's /live page.
    #[test]
    fn reads_a_game_live_grid_row() {
        let grid = json!({ "contents": { "gridVideoRenderer": {
            "videoId": "cwgPyxlrH_Y",
            "title": { "runs": [ { "text": "Iron Wing vs. BoomBoys" } ] },
            "shortBylineText": { "runs": [ { "text": "dota2" } ] },
            "viewCountText": { "simpleText": "49,451 watching" },
            "badges": [ { "metadataBadgeRenderer": { "style": "BADGE_STYLE_TYPE_LIVE_NOW" } } ]
        } } });
        let rows = rows_from_renderers(&grid);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].user_name, "dota2", "short byline is the only author here");
        assert_eq!(rows[0].viewer_count, 49_451);
    }

    /// Both row shapes carry the channel avatar in DIFFERENT places, verified live:
    /// search rows use the classic `channelThumbnail...` renderer, lockups use the
    /// newer decorated-avatar view model. Cards were rendering bare because neither
    /// was read.
    #[test]
    fn reads_the_channel_avatar_from_both_row_shapes() {
        let classic = json!({ "videoRenderer": {
            "videoId": "abc12345678",
            "title": { "runs": [ { "text": "A stream" } ] },
            "badges": [ { "metadataBadgeRenderer": { "style": "BADGE_STYLE_TYPE_LIVE_NOW" } } ],
            "channelThumbnailSupportedRenderers": { "channelThumbnailWithLinkRenderer": {
                "thumbnail": { "thumbnails": [
                    { "url": "https://yt3.ggpht.com/small=s48" },
                    { "url": "https://yt3.ggpht.com/big=s68" }
                ] }
            } }
        } });
        let rows = rows_from_renderers(&classic);
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].profile_image_url.as_deref(),
            Some("https://yt3.ggpht.com/big=s68"),
            "largest source wins",
        );

        let lockup = json!({ "lockupViewModel": {
            "contentId": "def12345678",
            "contentType": "LOCKUP_CONTENT_TYPE_VIDEO",
            "contentImage": { "thumbnailViewModel": { "overlays": [
                { "thumbnailBottomOverlayViewModel": { "badges": [
                    { "thumbnailBadgeViewModel": { "badgeStyle": "THUMBNAIL_OVERLAY_BADGE_STYLE_LIVE" } }
                ] } }
            ] } },
            "metadata": { "lockupMetadataViewModel": {
                "title": { "content": "Another stream" },
                "image": { "decoratedAvatarViewModel": { "avatar": { "avatarViewModel": {
                    "image": { "sources": [ { "url": "https://yt3.ggpht.com/avatar=s68" } ] }
                } } } }
            } }
        } });
        let rows = rows_from_renderers(&lockup);
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].profile_image_url.as_deref(),
            Some("https://yt3.ggpht.com/avatar=s68"),
        );
    }

    /// A row without an avatar reports None, never an empty string: an empty src
    /// renders as a broken image rather than falling back to the placeholder.
    #[test]
    fn a_missing_avatar_is_none_not_empty() {
        let bare = json!({ "gridVideoRenderer": {
            "videoId": "ghi12345678",
            "title": { "simpleText": "No avatar here" },
            "badges": [ { "metadataBadgeRenderer": { "style": "BADGE_STYLE_TYPE_LIVE_NOW" } } ]
        } });
        let rows = rows_from_renderers(&bare);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].profile_image_url, None);
    }

    #[test]
    fn parses_abbreviated_counts() {
        assert_eq!(parse_count("481 watching"), 481);
        assert_eq!(parse_count("11K watching"), 11_000);
        assert_eq!(parse_count("1.2M views"), 1_200_000);
        assert_eq!(parse_count("1,234 watching"), 1_234, "commas are separators, not terminators");
        assert_eq!(parse_count("no digits here"), 0);
    }

    /// The subscriptions grid can name a row `gridVideoRenderer` where search uses
    /// `videoRenderer`. Both carry the same fields, so both must parse.
    #[test]
    fn parses_both_video_renderer_containers() {
        let live_row = |key: &str| {
            json!({ "contents": { key: {
                "videoId": "abc12345678",
                "title": { "runs": [ { "text": "A live stream" } ] },
                "ownerText": { "runs": [ { "text": "Some Channel" } ] },
                "viewCountText": { "simpleText": "1,234 watching" },
                "badges": [ { "metadataBadgeRenderer": { "style": "BADGE_STYLE_TYPE_LIVE_NOW" } } ]
            } } })
        };
        for key in VIDEO_RENDERER_KEYS {
            let rows = rows_from_renderers(&live_row(key));
            assert_eq!(rows.len(), 1, "{} should parse", key);
            assert_eq!(rows[0].id, "abc12345678");
            assert_eq!(rows[0].viewer_count, 1234);
            assert!(rows[0].is_live);
        }

        // A non-live upload in the same feed is skipped: this is a LIVE list.
        let upload = json!({ "contents": { "gridVideoRenderer": {
            "videoId": "def12345678",
            "title": { "runs": [ { "text": "Yesterday's upload" } ] }
        } } });
        assert!(rows_from_renderers(&upload).is_empty());
    }
}

// --- Channel avatars, resolved on demand -----------------------------------
//
// A game's live grid ships NO avatar (measured: no lockups, no
// `channelThumbnailSupportedRenderers`, nothing), unlike search and the
// subscriptions feed which both carry one. So category cards can only be
// decorated by asking per channel, and one ask is a ~530KB browse response.
//
// That cost is why this is demand-driven and cached rather than resolved while
// building the page: the caller passes only the channels currently on screen, a
// channel is fetched at most once per session, and a channel with no avatar is
// remembered as such so it is never retried in a loop.

/// Session cache. `None` means "asked, and there is nothing", which must be
/// remembered or every render would re-ask for the same missing avatar.
static AVATARS: Lazy<tokio::sync::Mutex<std::collections::HashMap<String, Option<String>>>> =
    Lazy::new(|| tokio::sync::Mutex::new(std::collections::HashMap::new()));

/// Most channels resolved per call. The caller sends what is on screen; this caps
/// how much a single scroll can cost even if that list is unexpectedly long.
const AVATAR_BATCH_MAX: usize = 24;
/// Resolved in small waves rather than all at once. A burst of requests is what
/// gets an IP challenged (the same reason `live_check` staggers), and nothing here
/// is urgent enough to risk that.
const AVATAR_CONCURRENCY: usize = 3;

/// Channel avatars for `channel_ids`, as `{ id: url }`. Ids that are unknown, have
/// no avatar, or fail are simply absent, so the caller keeps its placeholder.
pub async fn channel_avatars(channel_ids: &[String]) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    let mut wanted: Vec<String> = Vec::new();
    {
        let cache = AVATARS.lock().await;
        for id in channel_ids {
            if !is_channel_id_str(id) {
                continue;
            }
            match cache.get(id) {
                // Already known, either a url or a remembered blank.
                Some(Some(url)) => {
                    out.insert(id.clone(), url.clone());
                }
                Some(None) => {}
                None => {
                    if !wanted.contains(id) {
                        wanted.push(id.clone());
                    }
                }
            }
        }
    }
    wanted.truncate(AVATAR_BATCH_MAX);
    if wanted.is_empty() {
        return out;
    }

    for wave in wanted.chunks(AVATAR_CONCURRENCY) {
        let results = futures::future::join_all(wave.iter().map(|id| async move {
            (id.clone(), fetch_channel_avatar(id).await)
        }))
        .await;
        let mut cache = AVATARS.lock().await;
        for (id, url) in results {
            if let Some(u) = &url {
                out.insert(id.clone(), u.clone());
            }
            // Store the miss too, so a channel without an avatar is asked once.
            cache.insert(id, url);
        }
    }
    out
}

/// A chatter's channel profile.
///
/// Only the CHANNEL-level facts come from here. Everything that is per-room —
/// member tier and tenure, moderator, owner, verified, a Super Chat amount — is
/// already parsed onto the message by the chat adapter, so the card reads those
/// from the message it was opened from rather than paying for a second lookup
/// that could not answer them anyway.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct YouTubeUserProfile {
    pub channel_id: String,
    /// `@handle`, when the channel has one.
    pub handle: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub avatar_url: Option<String>,
    pub banner_url: Option<String>,
    /// As YouTube renders it ("1.2M subscribers"); it publishes no exact count.
    pub subscriber_count: Option<String>,
}

/// Fetch one channel's profile via the same unauthenticated InnerTube `browse`
/// the avatar resolver already uses — no new client, no session required.
pub async fn user_profile(channel_id: &str) -> Result<YouTubeUserProfile> {
    if !is_channel_id_str(channel_id) {
        return Err(anyhow!("'{}' is not a YouTube channel id", channel_id));
    }
    let body = json!({
        "browseId": channel_id,
        "context": { "client": { "clientName": "WEB", "clientVersion": "2.20240101.00.00", "hl": "en", "gl": "US" } }
    });
    let resp = HTTP
        .post(format!("{}/browse?key={}", INNERTUBE, INNERTUBE_KEY))
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .json(&body)
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(anyhow!("YouTube returned {}", resp.status()));
    }
    let v: Value = resp.json().await?;

    let meta = v.pointer("/metadata/channelMetadataRenderer");
    let header = v.pointer("/header/pageHeaderRenderer/content/pageHeaderViewModel");
    let text_at = |p: Option<&Value>, key: &str| -> Option<String> {
        p?.get(key)
            .and_then(|x| x.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };

    Ok(YouTubeUserProfile {
        channel_id: channel_id.to_string(),
        // The vanity handle lives on the canonical URL, not as its own field.
        handle: text_at(meta, "vanityChannelUrl")
            .and_then(|u| u.rsplit('/').next().map(str::to_string))
            .filter(|h| h.starts_with('@')),
        title: text_at(meta, "title"),
        description: text_at(meta, "description"),
        avatar_url: largest_url(v.pointer("/metadata/channelMetadataRenderer/avatar/thumbnails"))
            .or_else(|| {
                largest_url(v.pointer("/microformat/microformatDataRenderer/thumbnail/thumbnails"))
            }),
        banner_url: largest_url(v.pointer("/header/pageHeaderRenderer/content/pageHeaderViewModel/banner/imageBannerViewModel/image/sources")),
        subscriber_count: header
            .and_then(|h| h.pointer("/metadata/contentMetadataViewModel/metadataRows/0/metadataParts/0/text/content"))
            .and_then(|x| x.as_str())
            .map(str::to_string),
    })
}

/// One channel's avatar via InnerTube `browse`.
///
/// `metadata/channelMetadataRenderer/avatar` is the stable home for it; the
/// microformat copy is a fallback for a response shaped differently. A
/// params-scoped tab returns the same ~530KB, and the ANDROID client 400s on a
/// channel browse, so neither is a cheaper route.
async fn fetch_channel_avatar(channel_id: &str) -> Option<String> {
    let body = json!({
        "browseId": channel_id,
        "context": { "client": { "clientName": "WEB", "clientVersion": "2.20240101.00.00", "hl": "en", "gl": "US" } }
    });
    let resp = HTTP
        .post(format!("{}/browse?key={}", INNERTUBE, INNERTUBE_KEY))
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .json(&body)
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let json: Value = resp.json().await.ok()?;
    largest_url(json.pointer("/metadata/channelMetadataRenderer/avatar/thumbnails"))
        .or_else(|| largest_url(json.pointer("/microformat/microformatDataRenderer/thumbnail/thumbnails")))
}

#[cfg(test)]
mod high_rendition_tests {
    use super::*;

    /// Hit the live InnerTube API and confirm the VISIONOS path still returns
    /// renditions above the HLS ceiling. Opt-in because it needs the network and
    /// a currently-live 1440p+ broadcast:
    ///
    /// ```text
    /// STREAMNOOK_YT_LIVE_ID=<videoId> \
    ///   cargo test high_renditions_from_a_live_broadcast -- --ignored --nocapture
    /// ```
    ///
    /// Worth running whenever YouTube changes something: this call is the part
    /// of the 1440p path most likely to rot, because it depends on a client
    /// context and a visitor token rather than on a documented API.
    #[tokio::test]
    #[ignore = "needs the network and a live 1440p+ broadcast"]
    async fn high_renditions_from_a_live_broadcast() {
        let id = std::env::var("STREAMNOOK_YT_LIVE_ID").expect("STREAMNOOK_YT_LIVE_ID");
        let r = high_renditions(&id, 1080).await;
        for x in &r {
            println!(
                "{:>8}  itag={:<4} {}x{}@{}  {} kbps",
                x.name,
                x.itag,
                x.width,
                x.height,
                x.fps,
                x.bandwidth / 1000
            );
        }
        assert!(!r.is_empty(), "expected at least one rendition above 1080p");
        assert!(r.iter().all(|x| x.height > 1080));
        assert!(r.iter().all(|x| !x.video_url.is_empty() && !x.audio_url.is_empty()));
        // Sorted tallest first, which is the order the quality menu wants.
        assert!(r.windows(2).all(|w| w[0].height >= w[1].height));
    }
}

#[cfg(test)]
mod menu_tests {
    use super::*;
    use crate::services::providers::source::StreamSource;

    /// Print the EXACT quality list the frontend receives, which is what the
    /// player's menu is built from. If 1440p/2160p are missing from the menu in
    /// the app but present here, the app is running an older build.
    ///
    /// ```text
    /// STREAMNOOK_YT_LIVE_ID=<videoId> \
    ///   cargo test menu_for_a_live_broadcast -- --ignored --nocapture
    /// ```
    #[tokio::test]
    #[ignore = "needs the network and a live broadcast"]
    async fn menu_for_a_live_broadcast() {
        let id = std::env::var("STREAMNOOK_YT_LIVE_ID").expect("STREAMNOOK_YT_LIVE_ID");
        let src = YouTubeSource;
        let r = src
            .resolve_playback("test", &id, "1080p60")
            .await
            .expect("resolve");
        println!("kind={:?} served={}", r.kind, r.quality);
        println!(
            "MENU: {:?}",
            crate::services::providers::hls_master::quality_names(&r.qualities)
        );
    }
}

#[cfg(test)]
mod directory_scan_tests {
    use super::*;
    use crate::services::providers::source::StreamSource;

    /// Walk the SAME YouTube directory the app's Home grid shows and report which
    /// entries actually expose renditions above 1080p.
    ///
    /// Exists because there is no way to type a video id into the app: streams are
    /// started from the grid, the sidebar or the command palette. So "go test
    /// 1440p" needs a channel name someone can click, not a video id.
    ///
    /// ```text
    /// cargo test scan_directory_for_high_renditions -- --ignored --nocapture
    /// ```
    #[tokio::test]
    #[ignore = "needs the network; walks the live directory"]
    async fn scan_directory_for_high_renditions() {
        let src = YouTubeSource;
        let page = src.directory(None, None, 30).await.expect("directory");
        println!("directory returned {} streams\n", page.streams.len());
        let mut hits = 0;
        for s in &page.streams {
            let id = match live_video_id(&s.user_login).await {
                Ok(v) => v,
                Err(_) => continue,
            };
            let highs = high_renditions(&id, 1080).await;
            if highs.is_empty() {
                continue;
            }
            hits += 1;
            println!(
                "CLICKABLE  {:<34} {:<14} {}",
                s.user_name,
                s.user_login,
                highs.iter().map(|r| r.name.as_str()).collect::<Vec<_>>().join(" ")
            );
        }
        println!("\n{} of {} directory entries expose >1080p", hits, page.streams.len());
    }
}

#[cfg(test)]
mod sabr_session_tests {
    use super::*;

    /// Build a real SABR session from a live watch page and report every input.
    ///
    /// This deliberately stops short of minting a token, because minting needs
    /// the app's webview. It answers the separable question: are we scraping the
    /// right things? If any field here is missing, no token would help.
    ///
    /// ```text
    /// STREAMNOOK_YT_LIVE_ID=<videoId> \
    ///   cargo test builds_a_sabr_session_from_a_live_page -- --ignored --nocapture
    /// ```
    #[tokio::test]
    #[ignore = "needs the network and a live broadcast"]
    async fn builds_a_sabr_session_from_a_live_page() {
        let id = std::env::var("STREAMNOOK_YT_LIVE_ID").expect("STREAMNOOK_YT_LIVE_ID");
        let inputs = sabr_session_for(&id).await.expect("build session");
        let s = &inputs.session;
        println!("rendition        {}", inputs.name);
        println!("video itag       {}", inputs.video_itag);
        println!("audio itag       {}", inputs.audio_itag);
        println!("dimensions       {}x{}", s.width, s.height);
        println!("client version   {}", s.client_version);
        println!("abr url          {} chars", s.abr_url.len());
        println!("ustreamer config {} bytes", s.ustreamer_config.len());
        println!("content binding  {} chars", s.content_binding.len());
        println!("video lastMod    {}", s.video.last_modified);
        println!("audio lastMod    {}", s.audio.last_modified);

        assert!(s.abr_url.contains("videoplayback"), "abr url should be a videoplayback url");
        assert!(!s.ustreamer_config.is_empty(), "ustreamer config is required");
        assert!(!s.content_binding.is_empty(), "the token needs something to bind to");
        // lastModified is deliberately absent here: the watch page does NOT
        // publish it for live formats. The first SABR request goes out without
        // one and `SabrSession::learn_formats` adopts what the server echoes
        // back in FORMAT_INITIALIZATION_METADATA. Asserting it were present here
        // would be asserting a thing YouTube does not provide.
        assert_eq!(s.video.last_modified, 0, "not knowable before the first response");
        // Audio must be the MP4 track, or the feed cannot serve it to hls.js.
        assert_eq!(inputs.audio_itag, 140, "expected AAC-in-MP4 audio");
    }
}

#[cfg(test)]
mod rendition_parsing_tests {
    use super::*;
    use serde_json::json;

    /// Shaped like a real `streamingData`, including the two traps that cost the
    /// most time to find: high itags that carry no `url` at all, and a vertical
    /// broadcast whose tall edge is its width.
    fn player(formats: Value) -> Value {
        json!({ "streamingData": { "adaptiveFormats": formats } })
    }

    fn video(itag: u64, w: u32, h: u32, fps: f64, url: bool) -> Value {
        let mut f = json!({
            "itag": itag, "width": w, "height": h, "fps": fps,
            "bitrate": 9016000, "mimeType": "video/webm; codecs=\"vp9\""
        });
        if url {
            f["url"] = json!(format!("https://r1.googlevideo.com/videoplayback?itag={}", itag));
        }
        f
    }

    fn aac(bitrate: u64) -> Value {
        json!({
            "itag": 140, "bitrate": bitrate,
            "mimeType": "audio/mp4; codecs=\"mp4a.40.2\"",
            "url": "https://r1.googlevideo.com/videoplayback?itag=140"
        })
    }

    #[test]
    fn takes_only_renditions_that_carry_a_url() {
        // android and ios advertise 308 with the field missing, because they are
        // expected to fetch it over SABR. Listing those would put a rung in the
        // menu that can never play.
        let p = player(json!([
            aac(144000),
            video(308, 2560, 1440, 60.0, false),
            video(299, 1920, 1080, 60.0, true),
        ]));
        assert!(renditions_from_player(&p, 1080).is_empty());
    }

    #[test]
    fn ranks_by_short_edge_so_vertical_is_not_mistaken_for_tall() {
        let p = player(json!([
            aac(144000),
            // A 1080x1920 portrait broadcast is a 1080p stream, not a 1920p one.
            video(308, 1080, 1920, 30.0, true),
        ]));
        assert!(
            renditions_from_player(&p, 1080).is_empty(),
            "portrait 1080 must not clear a 1080 floor"
        );
    }

    #[test]
    fn labels_high_frame_rates_and_sorts_tallest_first() {
        let p = player(json!([
            aac(144000),
            video(308, 2560, 1440, 60.0, true),
            video(315, 3840, 2160, 60.0, true),
            video(271, 2560, 1440, 30.0, true),
        ]));
        let out = renditions_from_player(&p, 1080);
        let names: Vec<&str> = out.iter().map(|r| r.name.as_str()).collect();
        // 1440p60 and 1440p30 are the same rung; the better encode wins it.
        assert_eq!(names, vec!["2160p60", "1440p60"]);
        assert!(out.iter().all(|r| r.audio_url.contains("itag=140")));
    }

    #[test]
    fn without_an_aac_track_there_is_nothing_to_pair_with() {
        // Opus in WebM would need a second transmux, so it does not count.
        let p = player(json!([
            json!({ "itag": 251, "bitrate": 160000,
                    "mimeType": "audio/webm; codecs=\"opus\"",
                    "url": "https://r1.googlevideo.com/videoplayback?itag=251" }),
            video(315, 3840, 2160, 60.0, true),
        ]));
        assert!(renditions_from_player(&p, 1080).is_empty());
    }
}


/// A YouTube channel's chrome metadata, re-resolving it when the cached copy has
/// aged out.
///
/// The plain `youtube::channel_meta` is an ageless cache read, so a surface that
/// polls it (the MultiChat viewer counter) re-read the value captured when chat
/// first resolved and never changed for the life of the session. This applies the
/// same freshness rule `StreamSource::channel_meta` uses, and keeps the last known
/// answer when a refetch fails rather than reporting nothing.
pub async fn channel_meta_refreshed(identifier: &str) -> Option<youtube::YouTubeChannelMeta> {
    if let Some(meta) = youtube::channel_meta_fresh(identifier, META_TTL) {
        return Some(meta);
    }
    let url = youtube::live_page_url(identifier);
    if youtube::fetch_youtube_html(&HTTP, &url, identifier).await.is_err() {
        return youtube::channel_meta(identifier);
    }
    youtube::channel_meta(identifier)
}
