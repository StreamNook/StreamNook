//! Local HLS relay for TikTok LIVE.
//!
//! Every TikTok stream is served from here, which is why `TikTokSource` always
//! reports `PlaybackKind::LocalHls` rather than the plain `Hls` kind. The
//! reason is the grid: `multi_nook_server` answers 404 for any path that is not
//! `stream.m3u8`, so it can only carry a platform whose segment URLs are both
//! absolute and served by a CORS-open CDN. Kick's IVS is; TikTok's pull CDN is
//! not known to be, and its playlists may carry relative URIs. Serving TikTok
//! from here makes the question moot and makes solo and grid playback the same
//! code path.
//!
//! One server for the process, one session per `stream_id`, addressed by the
//! `/s/{id}/` path prefix. That prefix is what keeps two tiles from serving each
//! other's bytes: it is in every URL the player is ever handed, so there is no
//! shared mutable "current stream" to get out of step.
//!
//! ```text
//! /s/{id}/stream.m3u8   master, one variant (every mode)
//!
//! HLS upstream
//! /s/{id}/live.m3u8     the media playlist, URIs rewritten to `up/{n}`
//! /s/{id}/up/{n}        one upstream segment or init, relayed
//!
//! DASH upstream, segments held because the CDN keeps only the newest
//! /s/{id}/video.m3u8    /s/{id}/audio.m3u8
//! /s/{id}/iv, /ia       inits          /s/{id}/v/{n}, /a/{n}   segments
//!
//! FLV upstream, cut into fMP4 here
//! /s/{id}/flv.m3u8      the media playlist, audio and video muxed
//! /s/{id}/fi/{epoch}    init per codec configuration
//! /s/{id}/f/{n}         segments
//! ```

use crate::services::flv_fmp4;
use anyhow::{anyhow, Result};
use bytes::Bytes;
use once_cell::sync::Lazy;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;
use warp::Filter;

/// How long a fetched media playlist is reused. Short enough to ride the
/// player's own poll cadence rather than adding a timer of our own.
const PLAYLIST_TTL: Duration = Duration::from_millis(2000);
/// Interned upstream URIs kept per session. A live window is a few dozen; this
/// bounds a pathological playlist without ever evicting one still in the window.
const MAX_URIS: usize = 512;
/// Relayed segment bodies kept per session.
const KEEP_SEGMENTS: usize = 8;
/// A session with no request for this long has lost its player: a destroyed
/// webview never runs its cleanup, so this is the backstop that stops the work.
/// It is NOT the primary teardown, which is `stop` from the command layer.
const IDLE_ABORT: Duration = Duration::from_secs(20);

/// Hosts a TikTok playlist may point at, beyond its own host.
///
/// The primary rule is same-host-as-the-playlist, which covers every CDN TikTok
/// actually uses without needing this list to be complete. This exists so a
/// playlist that legitimately spreads across sibling CDN hosts still plays.
const CDN_SUFFIXES: [&str; 7] = [
    ".tiktokcdn.com",
    ".tiktokcdn-us.com",
    ".tiktokcdn-eu.com",
    ".tiktokcdn-in.com",
    ".ibytedtos.com",
    ".byteoversea.com",
    ".tiktokv.com",
];

/// Interned upstream URIs for one session, plus the bodies already fetched.
#[derive(Default)]
struct UriMap {
    next: u32,
    by_index: HashMap<u32, String>,
    by_url: HashMap<String, u32>,
    cache: HashMap<u32, Bytes>,
}

impl UriMap {
    /// Stable index for `url`. A URI that survives a playlist refresh keeps its
    /// index, so the player's in-flight request for `up/7` stays valid.
    fn intern(&mut self, url: &str) -> u32 {
        if let Some(i) = self.by_url.get(url) {
            return *i;
        }
        let idx = self.next;
        self.next = self.next.wrapping_add(1);
        self.by_index.insert(idx, url.to_string());
        self.by_url.insert(url.to_string(), idx);
        if self.by_index.len() > MAX_URIS {
            // Drop the oldest indices, which are behind the live window.
            let cutoff = idx.saturating_sub(MAX_URIS as u32);
            self.by_index.retain(|k, _| *k > cutoff);
            self.by_url.retain(|_, v| *v > cutoff);
        }
        idx
    }

    fn target(&self, idx: u32) -> Option<String> {
        self.by_index.get(&idx).cloned()
    }

    fn remember(&mut self, idx: u32, bytes: Bytes) {
        self.cache.insert(idx, bytes);
        if self.cache.len() > KEEP_SEGMENTS {
            let cutoff = idx.saturating_sub(KEEP_SEGMENTS as u32);
            self.cache.retain(|k, _| *k > cutoff);
        }
    }
}

/// How this session's upstream is packaged.
///
/// TikTok publishes three and only one of them is reliably usable. Measured
/// against live rooms: the `hls` endpoint answers 504 on most of its fleet and
/// serves a frozen playlist on the rest, whose segments 404 even when fetched
/// straight from the CDN. The `cmaf` field is not HLS at all, it is a DASH
/// manifest, and it works: the manifest updates and its segments fetch clean.
///
/// The saving grace is that TikTok's DASH segments are ALREADY fMP4, `avc1`
/// video and `mp4a` audio with an `init...mp4`. So serving DASH as HLS is a
/// manifest translation and nothing else: no container rewriting, no bitstream
/// work, no second media engine in the page.
///
/// FLV, though, starts far sooner, which is why the adapter asks for it first
/// (see `tiktok_media::delivery_order`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Packaging {
    /// An upstream `.m3u8` we fetch, rewrite and relay.
    Hls,
    /// An upstream `.mpd` we translate into HLS.
    Dash,
    /// An upstream FLV pull we cut into fMP4 segments.
    Flv,
}

/// One elementary stream inside a DASH manifest.
#[derive(Clone, Debug, Default, PartialEq)]
struct DashTrack {
    /// `initialization` template, relative to the manifest.
    init: String,
    /// `media` template, still carrying `$Number$`.
    media: String,
    /// Number of the newest segment the manifest describes.
    newest: u64,
    /// Segment duration in seconds.
    duration: f64,
    codecs: String,
    bandwidth: u64,
    width: Option<u32>,
    height: Option<u32>,
    /// `frameRate`. The manifest is the only place TikTok publishes one: the
    /// room's `sdk_params` carry codec, resolution and bitrate but no fps.
    fps: Option<f64>,
}

/// The segments one track has taken, and what became of the rest.
#[derive(Default)]
struct TrackHold {
    /// Bodies by segment number, each taken while it was the newest.
    held: BTreeMap<u64, Bytes>,
    /// Numbers that could not be taken in time. Kept so the playlist marks
    /// them as gaps rather than closing ranks, which would silently renumber
    /// every segment after them.
    missed: BTreeSet<u64>,
    /// The newest number a capture has been started for, so a download still
    /// in flight is never started twice and a skipped number is noticed.
    started_through: Option<u64>,
    /// The init segment, under the init name that produced it.
    init: Option<(String, Bytes)>,
}

impl TrackHold {
    /// Oldest number still inside the window.
    fn floor(&self) -> Option<u64> {
        self.started_through
            .map(|t| t.saturating_sub(DASH_WINDOW - 1))
    }

    /// Forget everything behind the window, so memory is bounded by the window
    /// rather than by how long the stream has run.
    fn prune(&mut self) {
        if let Some(floor) = self.floor() {
            self.held.retain(|n, _| *n >= floor);
            self.missed.retain(|n| *n >= floor);
        }
    }

    /// What the playlist may list right now, as (number, is_gap), in order.
    ///
    /// Only a contiguous run of RESOLVED numbers: each one either held or
    /// definitively missed. A number whose download is still in flight ends
    /// the run, because announcing it as a gap would make the player skip it
    /// for good even though it is about to arrive. Leading and trailing gaps
    /// are dropped, since the player can do nothing with them.
    fn listable(&self) -> Vec<(u64, bool)> {
        let Some(&first) = self.held.keys().next() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut n = first;
        loop {
            if self.held.contains_key(&n) {
                out.push((n, false));
            } else if self.missed.contains(&n) {
                out.push((n, true));
            } else {
                break;
            }
            n += 1;
        }
        while matches!(out.last(), Some((_, true))) {
            out.pop();
        }
        out
    }
}

/// What the manifest said last, and what this relay holds because of it.
#[derive(Default)]
struct DashState {
    video: Option<DashTrack>,
    audio: Option<DashTrack>,
    v: TrackHold,
    a: TrackHold,
    /// Consecutive manifest failures. The capture loop gives up past a bound,
    /// which is how a broadcast that has ended stops costing anything.
    failures: u32,
}

struct Session {
    /// The manifest we relay, and the base relative URIs resolve against.
    upstream: String,
    /// Scheme and host of `upstream`, the primary allowlist entry.
    upstream_host: String,
    packaging: Packaging,
    handle: String,
    tier: String,
    width: Option<u32>,
    height: Option<u32>,
    fps: Option<f64>,
    bandwidth: Option<u64>,
    uris: Mutex<UriMap>,
    /// Last fetched playlist and when, so a burst of polls costs one fetch.
    playlist: Mutex<Option<(Instant, String)>>,
    /// DASH only: the manifest's latest view and the segments held because of it.
    dash: Mutex<DashState>,
    /// FLV only: the segments cut from the pull so far.
    flv: Mutex<FlvState>,
    /// DASH and FLV: the loop that takes each segment as it appears. Aborted
    /// the moment the session is stopped or replaced.
    capture_task: Mutex<Option<JoinHandle<()>>>,
    last_seen: Mutex<Instant>,
}

impl Session {
    fn touch(&self) {
        if let Ok(mut t) = self.last_seen.lock() {
            *t = Instant::now();
        }
    }

    fn idle_for(&self) -> Duration {
        self.last_seen
            .lock()
            .map(|t| t.elapsed())
            .unwrap_or_else(|_| Duration::ZERO)
    }
}

static REGISTRY: Lazy<Mutex<HashMap<String, Arc<Session>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
static PORT: Lazy<Mutex<Option<u16>>> = Lazy::new(|| Mutex::new(None));
static SERVER: Lazy<Mutex<Option<JoinHandle<()>>>> = Lazy::new(|| Mutex::new(None));

/// What a caller needs to start a session: the chosen rendition's playlist plus
/// the metadata the master playlist advertises.
pub struct Upstream {
    pub url: String,
    pub packaging: Packaging,
    pub tier: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f64>,
    pub bandwidth: Option<u64>,
}

/// Begin (or replace) the session for `stream_id` and return the master URL.
///
/// Replacing is the quality-switch path: the frontend re-enters `start_stream`,
/// which lands here with the same id. The previous session is removed FIRST so
/// its work stops now rather than when the idle watchdog notices; an in-flight
/// request still holding the old `Arc` finishes against the tier it started on.
pub async fn start(stream_id: &str, handle: &str, up: Upstream) -> Result<String> {
    if !valid_id(stream_id) {
        return Err(anyhow!("invalid stream id"));
    }
    let parsed = reqwest::Url::parse(&up.url).map_err(|e| anyhow!("bad upstream url: {}", e))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| anyhow!("upstream url has no host"))?
        .to_ascii_lowercase();

    let is_dash = matches!(up.packaging, Packaging::Dash);
    let is_flv = matches!(up.packaging, Packaging::Flv);
    let session = Arc::new(Session {
        upstream: up.url.clone(),
        upstream_host: host,
        packaging: up.packaging,
        handle: handle.to_string(),
        tier: up.tier.clone(),
        width: up.width,
        height: up.height,
        fps: up.fps,
        bandwidth: up.bandwidth,
        uris: Mutex::new(UriMap::default()),
        playlist: Mutex::new(None),
        dash: Mutex::new(DashState::default()),
        flv: Mutex::new(FlvState::default()),
        capture_task: Mutex::new(None),
        last_seen: Mutex::new(Instant::now()),
    });

    let port = ensure_server()?;

    // Read the manifest BEFORE handing out a url, so a stream that cannot be
    // served fails here, with a reason, instead of as an empty playlist the
    // player retries until it gives up.
    let first_jobs = if is_dash {
        let (video, audio) = fetch_manifest(&session).await?;
        apply_manifest(&session, video, audio)
    } else {
        Vec::new()
    };

    reap_idle();
    if let Ok(mut reg) = REGISTRY.lock() {
        // A quality change lands here with the same id. The session it
        // replaces stops now, not when its loop next notices it was replaced.
        if let Some(old) = reg.insert(stream_id.to_string(), session.clone()) {
            retire(&old);
        }
    }

    if is_dash {
        let task = tokio::spawn(capture_loop(stream_id.to_string(), session.clone()));
        if let Ok(mut slot) = session.capture_task.lock() {
            *slot = Some(task);
        }
        for (is_video, name, n) in first_jobs {
            tokio::spawn(capture_segment(session.clone(), is_video, name, n));
        }
    }

    if is_flv {
        let task = tokio::spawn(flv_pump(stream_id.to_string(), session.clone()));
        if let Ok(mut slot) = session.capture_task.lock() {
            *slot = Some(task);
        }
        // Same reasoning as the DASH manifest read above: a pull that cannot
        // be played fails here, with its reason, not as a playlist that never
        // fills. The codecs arrive within the first tags, well inside this.
        if let Err(e) = flv_first_init(&session, FLV_FIRST_INIT_WAIT).await {
            remove_if_same(stream_id, &session);
            retire(&session);
            return Err(e);
        }
    }

    log::info!(
        "[TikTokRelay] '{}' -> @{} [{}] {} on port {}",
        stream_id,
        handle,
        up.tier,
        if is_dash {
            "dash"
        } else if is_flv {
            "flv"
        } else {
            "hls"
        },
        port
    );
    // A fresh url for every start, although the path is the same. The player
    // reloads its source when the url CHANGES; a quality switch or a jump to
    // another creator reuses this id, and an identical url would leave the
    // player on its old source, reading the new session's segments against the
    // old tier's init. The query is ignored by the router, which matches paths.
    Ok(format!(
        "http://127.0.0.1:{}/s/{}/stream.m3u8?t={}",
        port,
        stream_id,
        chrono::Utc::now().timestamp_millis()
    ))
}

/// Stop a session's background work. Idempotent.
fn retire(s: &Arc<Session>) {
    if let Ok(mut slot) = s.capture_task.lock() {
        if let Some(task) = slot.take() {
            task.abort();
        }
    }
}

/// Drop one session. Called from `stop_stream` and `stop_multi_nook`.
pub fn stop(stream_id: &str) {
    let removed = REGISTRY.lock().ok().and_then(|mut reg| reg.remove(stream_id));
    if let Some(old) = removed {
        retire(&old);
        log::info!("[TikTokRelay] '{}' stopped", stream_id);
    }
}

/// Drop every session except one. The grid teardown, mirroring the shape the
/// DASH relay uses when a tile is promoted to the solo player.
pub fn stop_all_except(keep: &str) {
    let removed: Vec<Arc<Session>> = match REGISTRY.lock() {
        Ok(mut reg) => {
            let ids: Vec<String> = reg.keys().filter(|id| *id != keep).cloned().collect();
            ids.into_iter().filter_map(|id| reg.remove(&id)).collect()
        }
        Err(_) => Vec::new(),
    };
    for s in &removed {
        retire(s);
    }
}

/// Remove sessions whose player has gone away without telling us.
fn reap_idle() {
    let removed: Vec<(String, Arc<Session>)> = match REGISTRY.lock() {
        Ok(mut reg) => {
            let idle: Vec<String> = reg
                .iter()
                .filter(|(_, s)| s.idle_for() >= IDLE_ABORT)
                .map(|(id, _)| id.clone())
                .collect();
            idle.into_iter()
                .filter_map(|id| reg.remove(&id).map(|s| (id, s)))
                .collect()
        }
        Err(_) => Vec::new(),
    };
    for (id, s) in &removed {
        retire(s);
        log::info!("[TikTokRelay] '{}' idle, releasing", id);
    }
}

/// Whether `s` is still the session registered under `id`. A loop whose
/// session was stopped or replaced must end, or it keeps pulling a stream
/// nobody is watching.
fn still_registered(id: &str, s: &Arc<Session>) -> bool {
    REGISTRY
        .lock()
        .ok()
        .and_then(|reg| reg.get(id).map(|cur| Arc::ptr_eq(cur, s)))
        .unwrap_or(false)
}

/// Remove `s` from the registry, but only if it is still the one there.
fn remove_if_same(id: &str, s: &Arc<Session>) {
    if let Ok(mut reg) = REGISTRY.lock() {
        if reg.get(id).map(|cur| Arc::ptr_eq(cur, s)).unwrap_or(false) {
            reg.remove(id);
        }
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Whether the relay may fetch `url` for a session whose playlist lives on
/// `playlist_host`.
///
/// Deliberately stricter than "any http(s) URL a playlist named". The relay
/// returns whatever it fetches to the page under a permissive CORS header, so
/// an unchecked URL turns it into a fetch proxy for anything reachable from
/// this machine, including loopback and the local network.
fn fetch_allowed(url: &str, playlist_host: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return false;
    };
    if parsed.scheme() != "https" {
        return false;
    }
    let Some(host) = parsed.host_str() else {
        return false;
    };
    let host = host.to_ascii_lowercase();

    // An address literal is never a CDN. Refuse the private ranges outright and
    // refuse the rest too, since TikTok always names a host.
    if host.parse::<IpAddr>().is_ok() {
        return false;
    }
    if host == playlist_host {
        return true;
    }
    CDN_SUFFIXES.iter().any(|s| host.ends_with(s))
}

fn ensure_server() -> Result<u16> {
    let mut port_guard = PORT.lock().map_err(|_| anyhow!("port poisoned"))?;
    if let Some(p) = *port_guard {
        // A recorded port with nothing listening behind it is unrecoverable
        // without this check: every later request fails to connect while the
        // port still looks live, so playback stays broken for the rest of the
        // session. Rebind instead. Sessions registered against the old port are
        // unreachable either way, so they go with it.
        let ended = SERVER
            .lock()
            .ok()
            .and_then(|g| g.as_ref().map(|h| h.is_finished()))
            .unwrap_or(false);
        if !ended {
            return Ok(p);
        }
        log::warn!("[TikTokRelay] server on port {} ended; rebinding", p);
        *port_guard = None;
        let orphaned: Vec<Arc<Session>> = REGISTRY
            .lock()
            .map(|mut reg| reg.drain().map(|(_, s)| s).collect())
            .unwrap_or_default();
        for s in &orphaned {
            retire(s);
        }
    }
    let route = warp::path::full()
        .and_then(|p: warp::path::FullPath| async move { handle(p.as_str().to_string()).await })
        .boxed();

    let socket = tokio::net::TcpSocket::new_v4()?;
    socket.bind(SocketAddr::from(([127, 0, 0, 1], 0)))?;
    let port = socket.local_addr()?.port();
    let listener = socket.listen(512)?;
    let joined = tokio::spawn(async move {
        warp::serve(route).incoming(listener).run().await;
    });
    *SERVER.lock().map_err(|_| anyhow!("server poisoned"))? = Some(joined);
    *port_guard = Some(port);
    log::info!("[TikTokRelay] serving on port {}", port);
    Ok(port)
}

/// Split `/s/{id}/{tail}` once, so every arm below matches on the tail alone.
fn route_of(path: &str) -> Option<(&str, &str)> {
    let rest = path.trim_start_matches('/').strip_prefix("s/")?;
    let (id, tail) = rest.split_once('/')?;
    (!id.is_empty() && !tail.is_empty()).then_some((id, tail))
}

fn session_of(id: &str) -> Option<Arc<Session>> {
    REGISTRY.lock().ok()?.get(id).cloned()
}

async fn handle(path: String) -> Result<warp::http::Response<Bytes>, warp::Rejection> {
    let Some((id, tail)) = route_of(&path) else {
        return Ok(fail_because(404, "not a stream path".into()));
    };
    let Some(session) = session_of(id) else {
        return Ok(fail_because(404, format!("no session '{}'", id)));
    };
    session.touch();

    const M3U8: &str = "application/vnd.apple.mpegurl";

    match session.packaging {
        Packaging::Hls => {
            if tail == "stream.m3u8" {
                return Ok(cors(master_playlist(&session, id), M3U8));
            }
            if tail == "live.m3u8" {
                return Ok(match media_playlist(&session).await {
                    Ok(body) => cors(body, M3U8),
                    Err(e) => fail_because(502, format!("playlist: {}", e)),
                });
            }
            if let Some(n) = tail.strip_prefix("up/").and_then(|s| s.parse::<u32>().ok()) {
                return Ok(match upstream_bytes(&session, n).await {
                    Ok(bytes) => media_response(bytes),
                    Err(e) => fail_because(502, format!("segment: {}", e)),
                });
            }
        }
        Packaging::Dash => {
            if tail == "stream.m3u8" {
                return Ok(match dash_master(&session, id).await {
                    Ok(body) => cors(body, M3U8),
                    Err(e) => fail_because(502, format!("manifest: {}", e)),
                });
            }
            // `video`/`audio` are separate renditions here because DASH keeps
            // them as separate adaptation sets, which is also how hls.js wants
            // them: one variant plus an audio group.
            for (name, want_video) in [("video.m3u8", true), ("audio.m3u8", false)] {
                if tail == name {
                    return Ok(match dash_media(&session, id, want_video).await {
                        Ok(body) => cors(body, M3U8),
                        Err(e) => fail_because(502, format!("playlist: {}", e)),
                    });
                }
            }
            // `iv`/`ia` are the init segments, `v/{n}`/`a/{n}` the media ones.
            for (prefix, want_video) in [("v/", true), ("a/", false)] {
                if let Some(n) = tail.strip_prefix(prefix).and_then(|s| s.parse::<u64>().ok()) {
                    return Ok(match dash_segment(&session, want_video, Some(n)).await {
                        Ok(bytes) => media_response(bytes),
                        Err(e) => fail_because(502, format!("segment: {}", e)),
                    });
                }
            }
            for (name, want_video) in [("iv", true), ("ia", false)] {
                if tail == name {
                    return Ok(match dash_segment(&session, want_video, None).await {
                        Ok(bytes) => media_response(bytes),
                        Err(e) => fail_because(502, format!("init: {}", e)),
                    });
                }
            }
        }
        Packaging::Flv => {
            if tail == "stream.m3u8" {
                return Ok(match flv_master(&session, id).await {
                    Ok(body) => cors(body, M3U8),
                    Err(e) => fail_because(502, format!("stream: {}", e)),
                });
            }
            if tail == "flv.m3u8" {
                return Ok(match flv_media(&session, id).await {
                    Ok(body) => cors(body, M3U8),
                    Err(e) => fail_because(502, format!("playlist: {}", e)),
                });
            }
            // `fi/{epoch}` is an init, `f/{n}` a segment.
            if let Some(epoch) = tail.strip_prefix("fi/").and_then(|s| s.parse::<u32>().ok()) {
                let init = session.flv.lock().ok().and_then(|st| st.inits.get(&epoch).cloned());
                return Ok(match init {
                    Some(bytes) => media_response(bytes),
                    None => fail_because(404, format!("no init {}", epoch)),
                });
            }
            if let Some(n) = tail.strip_prefix("f/").and_then(|s| s.parse::<u64>().ok()) {
                let seg = session.flv.lock().ok().and_then(|st| st.segs.get(&n).map(|g| g.bytes.clone()));
                return Ok(match seg {
                    Some(bytes) => media_response(bytes),
                    None => fail_because(404, format!("segment {} is outside the window", n)),
                });
            }
        }
    }
    Ok(fail_because(404, format!("unknown path '{}'", tail)))
}

/// One variant, pointing at our own media playlist. Built from the rendition
/// metadata so the player's stats read the real geometry rather than guessing.
fn master_playlist(s: &Session, id: &str) -> String {
    let mut attrs = format!("BANDWIDTH={}", s.bandwidth.unwrap_or(2_000_000));
    if let (Some(w), Some(h)) = (s.width, s.height) {
        attrs.push_str(&format!(",RESOLUTION={}x{}", w, h));
    }
    if let Some(f) = s.fps {
        attrs.push_str(&format!(",FRAME-RATE={:.3}", f));
    }
    format!(
        "#EXTM3U\n#EXT-X-VERSION:6\n#EXT-X-STREAM-INF:{},NAME=\"{}\"\n/s/{}/live.m3u8\n",
        attrs, s.tier, id
    )
}

async fn media_playlist(s: &Session) -> Result<String> {
    if let Ok(guard) = s.playlist.lock() {
        if let Some((at, body)) = guard.as_ref() {
            if at.elapsed() < PLAYLIST_TTL {
                return Ok(body.clone());
            }
        }
    }
    let res = crate::services::http::client()
        .get(&s.upstream)
        .header("Referer", "https://www.tiktok.com/")
        .send()
        .await?;
    if !res.status().is_success() {
        return Err(anyhow!("upstream {} for @{}", res.status(), s.handle));
    }
    let text = res.text().await?;
    let rewritten = rewrite_playlist(s, &text);
    if let Ok(mut guard) = s.playlist.lock() {
        *guard = Some((Instant::now(), rewritten.clone()));
    }
    Ok(rewritten)
}

/// Point every URI in a media playlist back at this relay.
///
/// Three shapes carry a URI, and missing any one of them breaks playback in a
/// way that looks like a CDN problem:
///   * a bare absolute line
///   * a bare RELATIVE line, resolved against the playlist's own URL
///   * a `URI="..."` attribute, which is how `#EXT-X-MAP` names an fMP4 init
///     segment and how `#EXT-X-PART` names a partial one
fn rewrite_playlist(s: &Session, body: &str) -> String {
    let base = reqwest::Url::parse(&s.upstream).ok();
    let mut out = String::with_capacity(body.len());

    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            out.push('\n');
            continue;
        }
        if let Some(stripped) = trimmed.strip_prefix('#') {
            out.push_str(&rewrite_attr_uri(s, base.as_ref(), line, stripped));
            out.push('\n');
            continue;
        }
        match absolutize(base.as_ref(), trimmed) {
            Some(abs) => {
                let idx = match s.uris.lock() {
                    Ok(mut m) => m.intern(&abs),
                    Err(_) => {
                        out.push_str(line);
                        out.push('\n');
                        continue;
                    }
                };
                out.push_str(&format!("up/{}", idx));
            }
            None => out.push_str(line),
        }
        out.push('\n');
    }
    out
}

/// Rewrite a `URI="..."` attribute on a tag line, leaving everything else as is.
fn rewrite_attr_uri(s: &Session, base: Option<&reqwest::Url>, line: &str, tag: &str) -> String {
    if !tag.starts_with("EXT-X-MAP") && !tag.starts_with("EXT-X-PART") {
        return line.to_string();
    }
    let Some(open) = line.find("URI=\"") else {
        return line.to_string();
    };
    let value_start = open + 5;
    let Some(close_rel) = line[value_start..].find('"') else {
        return line.to_string();
    };
    let close = value_start + close_rel;
    let raw = &line[value_start..close];
    let Some(abs) = absolutize(base, raw) else {
        return line.to_string();
    };
    let idx = match s.uris.lock() {
        Ok(mut m) => m.intern(&abs),
        Err(_) => return line.to_string(),
    };
    format!("{}up/{}{}", &line[..value_start], idx, &line[close..])
}

/// Resolve a playlist URI to an absolute URL, or `None` when it is not a URI we
/// should touch.
fn absolutize(base: Option<&reqwest::Url>, raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    if raw.starts_with("http://") || raw.starts_with("https://") {
        return Some(raw.to_string());
    }
    let base = base?;
    let mut joined = base.join(raw).ok()?;
    // TikTok signs the DIRECTORY and carries the signature in the MANIFEST's
    // query string, which `Url::join` drops. A segment fetched without it comes
    // back 404, which reads as a missing segment rather than as an unsigned
    // request. Only for a URI that brought no query of its own, so a CDN that
    // signs each file individually is left alone.
    if joined.query().is_none() {
        if let Some(q) = base.query() {
            joined.set_query(Some(q));
        }
    }
    Some(joined.to_string())
}

// --- DASH to HLS ------------------------------------------------------------
//
// TikTok's DASH segments are already fMP4, so none of this touches media bytes:
// it reads the manifest, works out which segment numbers exist, and writes the
// equivalent HLS. The player, Plyr chrome and quality menu stay exactly as they
// are and simply gain a platform.
//
// The one part that is not translation is OWNING the window. TikTok's CDN
// serves a segment only while it is the newest one: measured, the moment a newer
// number appears the previous one answers 404, whatever shift buffer the
// manifest advertises. So this relay takes each segment while it is newest and
// holds it, on a timer of its own. Capture driven by the player's polling would
// lose a segment every time a poll landed late, and a lost segment is gone.

/// Segments held per track, which is also the window the playlist advertises:
/// thirty seconds at TikTok's 2 second segments, bounded however long the
/// stream runs (about 2.5 MB a session).
///
/// Sized from the PLAYERS, not from TikTok's manifest. Grid tiles sit eight
/// seconds behind live by policy, and a twelve second window put a tile's
/// playhead right at the window's floor: any delay and it asked for a segment
/// that had just been pruned, got a 502, and was left with a hole in its buffer
/// that its media-error recovery could not climb out of. Upstream keeps only the
/// newest segment regardless, so this relay is the only place history exists,
/// and it can afford to keep more than the player will ever reach back for.
const DASH_WINDOW: u64 = 15;
/// How often the manifest is read. TikTok declares a one second
/// `minimumUpdatePeriod`, and each segment stays newest for about two, so a
/// one second read sees every segment at least once.
const CAPTURE_EVERY: Duration = Duration::from_millis(1000);
/// How far behind the newest a late read still tries. Older is certainly gone.
const CATCH_UP: u64 = 2;
/// Consecutive manifest failures before the broadcast is treated as over.
const MAX_MANIFEST_FAILURES: u32 = 15;
/// A segment is served while it is being produced, so fetching one can take up
/// to its own length. This bounds a stalled connection, not a normal one.
const SEGMENT_TIMEOUT: Duration = Duration::from_secs(10);
const MANIFEST_TIMEOUT: Duration = Duration::from_secs(5);
/// How many held segments a playlist waits for before it is first served.
///
/// Upstream keeps only the newest segment, so a fresh session has no history,
/// and a player handed a one-segment playlist starts at its beginning: one
/// segment from the edge, with the next arriving just as the buffer runs out.
/// On a room with four second segments that measured as a buffer draining to a
/// tenth of a second before every refill, and a stall whenever delivery was a
/// moment late. Two whole segments buy a full segment of cushion for the rest
/// of the watch; the cost is start-up time, since the segment in progress when
/// the session begins is not taken (see `plan_captures`).
const START_SEGMENTS: usize = 2;
/// The most a playlist request waits for those segments, kept under the
/// players' ten second playlist timeout. A room whose segments are too long to
/// deliver two in time is served what it has instead.
const START_WAIT: Duration = Duration::from_secs(8);

/// Read one attribute out of an XML fragment.
///
/// The name must start at a boundary, which is not fussiness: `width="` is a
/// substring of `bandwidth="`, so a plain search reads a video track's bitrate
/// as its pixel width. Likewise `d="` sits inside `id="`.
fn xml_attr(fragment: &str, key: &str) -> Option<String> {
    let needle = format!("{}=\"", key);
    let mut from = 0usize;
    while let Some(rel) = fragment[from..].find(&needle) {
        let at = from + rel;
        let boundary = at == 0
            || fragment[..at]
                .chars()
                .next_back()
                .map(|c| c.is_whitespace() || c == '<')
                .unwrap_or(false);
        if boundary {
            let rest = &fragment[at + needle.len()..];
            let end = rest.find('"')?;
            return Some(rest[..end].to_string());
        }
        from = at + needle.len();
    }
    None
}

/// `frameRate` is either a plain number or a ratio such as `30000/1001`.
fn parse_frame_rate(raw: &str) -> Option<f64> {
    let rate = match raw.split_once('/') {
        Some((n, d)) => {
            let d: f64 = d.trim().parse().ok()?;
            if d == 0.0 {
                return None;
            }
            n.trim().parse::<f64>().ok()? / d
        }
        None => raw.trim().parse().ok()?,
    };
    (rate.is_finite() && rate > 0.0).then_some(rate)
}

/// Parse the two adaptation sets we care about out of a live MPD.
///
/// Deliberately not a general DASH parser. It reads exactly the shape TikTok
/// publishes: `isoff-live` profile, one video and one audio `Representation`,
/// each with a `SegmentTemplate` addressed by `$Number$` and a `SegmentTimeline`
/// describing the tip of the stream.
fn parse_mpd(body: &str) -> (Option<DashTrack>, Option<DashTrack>) {
    let mut video = None;
    let mut audio = None;

    for chunk in body.split("<AdaptationSet").skip(1) {
        let is_video = chunk.contains("contentType=\"video\"")
            || chunk.contains("mimeType=\"video/mp4\"");
        let Some(tpl_at) = chunk.find("<SegmentTemplate") else {
            continue;
        };
        let tpl = &chunk[tpl_at..];
        let Some(init) = xml_attr(tpl, "initialization") else {
            continue;
        };
        let Some(media) = xml_attr(tpl, "media") else {
            continue;
        };
        let start: u64 = xml_attr(tpl, "startNumber")
            .and_then(|s| s.parse().ok())
            .unwrap_or(1);
        // Each `<S>` is one more segment beyond `startNumber`, and `r` repeats
        // it. The newest number is what the playlist must end on.
        let mut count: u64 = 0;
        for s in tpl.split("<S ").skip(1) {
            let repeat: u64 = xml_attr(s, "r").and_then(|v| v.parse().ok()).unwrap_or(0);
            count += 1 + repeat;
        }
        let newest = start + count.saturating_sub(1);
        let timescale: f64 = xml_attr(tpl, "timescale")
            .and_then(|s| s.parse().ok())
            .unwrap_or(1000.0);
        let d: f64 = tpl
            .split("<S ")
            .nth(1)
            .and_then(|s| xml_attr(s, "d"))
            .and_then(|s| s.parse().ok())
            .unwrap_or(2000.0);

        let track = DashTrack {
            init,
            media,
            newest,
            duration: (d / timescale).max(0.1),
            codecs: xml_attr(chunk, "codecs").unwrap_or_default(),
            bandwidth: xml_attr(chunk, "bandwidth")
                .and_then(|s| s.parse().ok())
                .unwrap_or(1_000_000),
            width: xml_attr(chunk, "width").and_then(|s| s.parse().ok()),
            height: xml_attr(chunk, "height").and_then(|s| s.parse().ok()),
            fps: xml_attr(chunk, "frameRate").and_then(|s| parse_frame_rate(&s)),
        };
        if is_video {
            video = Some(track);
        } else {
            audio = Some(track);
        }
    }
    (video, audio)
}

/// Which segment numbers to start taking now, and which to write off.
///
/// `started_through` is the newest number a capture was already started for.
/// Numbers between it and `newest` that are within `catch_up` of the newest are
/// still worth one request; anything older has already been dropped upstream
/// and is recorded as missed, bounded to the window so a long stall cannot turn
/// into a long list.
fn plan_captures(started_through: Option<u64>, newest: u64, catch_up: u64) -> (Vec<u64>, Vec<u64>) {
    match started_through {
        // Cold start takes nothing: the newest segment is already part way
        // through production, and the CDN hands it over from the chunk it is
        // on, so it arrives without its opening keyframe (measured on fresh
        // sessions: 44 of 47 audio chunks, 58 of 60 video frames). A player
        // cannot decode that GOP, skips to the next keyframe two seconds on,
        // and so throws away exactly the cushion the start hold waited for.
        // Every later segment is requested as it begins and arrives whole.
        None => (Vec::new(), Vec::new()),
        Some(done) if newest <= done => (Vec::new(), Vec::new()),
        Some(done) => {
            let first = done + 1;
            let try_from = newest.saturating_sub(catch_up).max(first);
            let write_off_from = newest.saturating_sub(DASH_WINDOW - 1).max(first);
            let missed = (write_off_from..try_from).collect();
            let start = (try_from..=newest).collect();
            (start, missed)
        }
    }
}

/// Fetch and parse the manifest.
async fn fetch_manifest(s: &Session) -> Result<(Option<DashTrack>, Option<DashTrack>)> {
    let res = crate::services::http::client()
        .get(&s.upstream)
        .header("Referer", "https://www.tiktok.com/")
        .timeout(MANIFEST_TIMEOUT)
        .send()
        .await?;
    if !res.status().is_success() {
        return Err(anyhow!("manifest {} for @{}", res.status(), s.handle));
    }
    let body = res.text().await?;
    let (video, audio) = parse_mpd(&body);
    if video.is_none() && audio.is_none() {
        return Err(anyhow!("no playable adaptation set in the manifest"));
    }
    Ok((video, audio))
}

/// Record what the manifest now says and return the captures to start, as
/// (is_video, segment name, number).
fn apply_manifest(
    s: &Session,
    video: Option<DashTrack>,
    audio: Option<DashTrack>,
) -> Vec<(bool, String, u64)> {
    let Ok(mut guard) = s.dash.lock() else {
        return Vec::new();
    };
    let st = &mut *guard;
    st.failures = 0;
    let mut jobs = Vec::new();

    for (is_video, track, previous) in [
        (true, video.as_ref(), st.video.as_ref()),
        (false, audio.as_ref(), st.audio.as_ref()),
    ] {
        let Some(track) = track else { continue };
        let hold = if is_video { &mut st.v } else { &mut st.a };

        // A new media template, or numbering far behind where it was, means
        // the encoder restarted. What is held belongs to the old numbering and
        // would be mis-sequenced against the new one, so it is dropped.
        let restarted = previous.map(|p| p.media != track.media).unwrap_or(false)
            || hold
                .started_through
                .map(|t| track.newest + DASH_WINDOW * 2 < t)
                .unwrap_or(false);
        if restarted {
            log::info!(
                "[TikTokRelay] @{} {} numbering restarted; dropping the held window",
                s.handle,
                if is_video { "video" } else { "audio" }
            );
            *hold = TrackHold::default();
        }

        let (start, missed) = plan_captures(hold.started_through, track.newest, CATCH_UP);
        hold.missed.extend(missed);
        for n in &start {
            jobs.push((is_video, track.media.replace("$Number$", &n.to_string()), *n));
        }
        // A cold start takes nothing but still has to record where it began,
        // or every read would be a cold start and nothing would ever be taken.
        let through = start.last().copied().unwrap_or(track.newest);
        hold.started_through = Some(hold.started_through.map_or(through, |t| t.max(through)));
        hold.prune();
    }

    st.video = video;
    st.audio = audio;
    jobs
}

/// Take one segment and hold it, or record that it could not be taken.
async fn capture_segment(s: Arc<Session>, is_video: bool, name: String, n: u64) {
    let got = fetch_segment(&s, &name).await;
    let Ok(mut st) = s.dash.lock() else { return };
    let hold = if is_video { &mut st.v } else { &mut st.a };
    match got {
        Ok(bytes) => {
            hold.missed.remove(&n);
            hold.held.insert(n, bytes);
        }
        Err(e) => {
            log::debug!(
                "[TikTokRelay] @{} {} segment {} not taken: {}",
                s.handle,
                if is_video { "video" } else { "audio" },
                n,
                e
            );
            if !hold.held.contains_key(&n) {
                hold.missed.insert(n);
            }
        }
    }
    hold.prune();
}

/// Read the manifest on a timer and take each segment while it is the newest.
///
/// Ends when the session is stopped or replaced, when nobody has requested
/// anything for `IDLE_ABORT` (a destroyed webview never says goodbye), or when
/// the manifest has been unreachable long enough that the broadcast is over.
async fn capture_loop(id: String, s: Arc<Session>) {
    let mut tick = tokio::time::interval(CAPTURE_EVERY);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // The first tick fires at once, and `start` has just read the manifest.
    tick.tick().await;
    loop {
        tick.tick().await;
        if !still_registered(&id, &s) {
            return;
        }
        if s.idle_for() >= IDLE_ABORT {
            log::info!("[TikTokRelay] '{}' idle, releasing", id);
            remove_if_same(&id, &s);
            return;
        }
        match fetch_manifest(&s).await {
            Ok((video, audio)) => {
                for (is_video, name, n) in apply_manifest(&s, video, audio) {
                    tokio::spawn(capture_segment(s.clone(), is_video, name, n));
                }
            }
            Err(e) => {
                let failures = s
                    .dash
                    .lock()
                    .map(|mut st| {
                        st.failures += 1;
                        st.failures
                    })
                    .unwrap_or(MAX_MANIFEST_FAILURES);
                if failures >= MAX_MANIFEST_FAILURES {
                    log::info!(
                        "[TikTokRelay] '{}' manifest unreachable {} times in a row ({}); ending",
                        id,
                        failures,
                        e
                    );
                    remove_if_same(&id, &s);
                    return;
                }
            }
        }
    }
}

/// Fetch one file from the manifest's directory, under the host policy.
async fn fetch_segment(s: &Session, name: &str) -> Result<Bytes> {
    let base = reqwest::Url::parse(&s.upstream)?;
    let url = absolutize(Some(&base), name).ok_or_else(|| anyhow!("bad segment name"))?;
    if !fetch_allowed(&url, &s.upstream_host) {
        log::warn!("[TikTokRelay] refused upstream host for @{}", s.handle);
        return Err(anyhow!("upstream host not allowed"));
    }
    let res = crate::services::http::client()
        .get(&url)
        .header("Referer", "https://www.tiktok.com/")
        .timeout(SEGMENT_TIMEOUT)
        .send()
        .await?;
    if !res.status().is_success() {
        return Err(anyhow!("{}", res.status()));
    }
    Ok(res.bytes().await?)
}

/// One video variant plus an audio rendition group, which is the shape hls.js
/// expects when the two arrive as separate tracks.
async fn dash_master(s: &Session, id: &str) -> Result<String> {
    let (v, a) = {
        let st = s.dash.lock().map_err(|_| anyhow!("poisoned"))?;
        (st.video.clone(), st.audio.clone())
    };
    let (v, a) = (v.as_ref(), a.as_ref());
    if v.is_none() && a.is_none() {
        return Err(anyhow!("manifest not read yet"));
    }

    let codecs = [
        v.map(|t| t.codecs.clone()).unwrap_or_default(),
        a.map(|t| t.codecs.clone()).unwrap_or_default(),
    ]
    .into_iter()
    .filter(|c| !c.is_empty())
    .collect::<Vec<_>>()
    .join(",");

    // The audio-only tier publishes a manifest with no video track. Pointing the
    // player at a video playlist there would fail on the first request, so the
    // audio rendition becomes the variant itself.
    if v.is_none() {
        let bandwidth = a.map(|t| t.bandwidth).unwrap_or(0).max(1);
        let mut attrs = format!("BANDWIDTH={}", bandwidth);
        if !codecs.is_empty() {
            attrs.push_str(&format!(",CODECS=\"{}\"", codecs));
        }
        return Ok(format!(
            "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-STREAM-INF:{},NAME=\"{}\"\n/s/{}/audio.m3u8\n",
            attrs, s.tier, id
        ));
    }

    let mut out = String::from("#EXTM3U\n#EXT-X-VERSION:7\n");
    if a.is_some() {
        out.push_str(&format!(
            "#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aud\",NAME=\"Audio\",DEFAULT=YES,\
AUTOSELECT=YES,URI=\"/s/{}/audio.m3u8\"\n",
            id
        ));
    }
    let mut attrs = format!(
        "BANDWIDTH={}",
        v.map(|t| t.bandwidth).unwrap_or(1_000_000) + a.map(|t| t.bandwidth).unwrap_or(0)
    );
    if let Some(w) = v.and_then(|t| t.width).or(s.width) {
        if let Some(h) = v.and_then(|t| t.height).or(s.height) {
            attrs.push_str(&format!(",RESOLUTION={}x{}", w, h));
        }
    }
    if let Some(f) = v.and_then(|t| t.fps).or(s.fps) {
        attrs.push_str(&format!(",FRAME-RATE={:.3}", f));
    }
    if !codecs.is_empty() {
        attrs.push_str(&format!(",CODECS=\"{}\"", codecs));
    }
    if a.is_some() {
        attrs.push_str(",AUDIO=\"aud\"");
    }
    out.push_str(&format!(
        "#EXT-X-STREAM-INF:{},NAME=\"{}\"\n/s/{}/video.m3u8\n",
        attrs, s.tier, id
    ));
    Ok(out)
}

/// A live media playlist over `items`, as produced by `TrackHold::listable`.
///
/// The media sequence IS the segment number, always: a number that could not be
/// taken stays in place as an `#EXT-X-GAP` rather than being closed up, because
/// HLS sequence numbers are positional and closing a gap would relabel every
/// segment after it.
fn dash_media_body(id: &str, kind: &str, init: &str, duration: f64, items: &[(u64, bool)]) -> String {
    let target = duration.ceil().max(1.0) as u64;
    // `EXT-X-GAP` arrived in protocol version 8.
    let version = if items.iter().any(|(_, gap)| *gap) { 8 } else { 7 };
    let mut out = format!(
        "#EXTM3U\n#EXT-X-VERSION:{}\n#EXT-X-TARGETDURATION:{}\n#EXT-X-MEDIA-SEQUENCE:{}\n\
#EXT-X-MAP:URI=\"/s/{}/{}\"\n",
        version,
        target,
        items.first().map(|(n, _)| *n).unwrap_or(0),
        id,
        init
    );
    for (n, gap) in items {
        out.push_str(&format!("#EXTINF:{:.3},\n", duration));
        if *gap {
            out.push_str("#EXT-X-GAP\n");
        }
        out.push_str(&format!("/s/{}/{}/{}\n", id, kind, n));
    }
    out
}

/// The media playlist for one track. While the session is new it waits for
/// `START_SEGMENTS` held segments, so the player starts with a cushion; once
/// the hold has them, every request answers at once.
async fn dash_media(s: &Session, id: &str, want_video: bool) -> Result<String> {
    dash_media_within(s, id, want_video, START_WAIT).await
}

async fn dash_media_within(s: &Session, id: &str, want_video: bool, wait: Duration) -> Result<String> {
    let (kind, init) = if want_video { ("v", "iv") } else { ("a", "ia") };
    let deadline = Instant::now() + wait;
    loop {
        let (duration, items) = {
            let st = s.dash.lock().map_err(|_| anyhow!("poisoned"))?;
            let track = if want_video { st.video.as_ref() } else { st.audio.as_ref() }
                .ok_or_else(|| anyhow!("no {} track", if want_video { "video" } else { "audio" }))?;
            let hold = if want_video { &st.v } else { &st.a };
            (track.duration, hold.listable())
        };
        // Gaps are skipped by the player, so only held segments are cushion.
        let held = items.iter().filter(|(_, gap)| !*gap).count();
        let out_of_time = Instant::now() >= deadline;
        if held >= START_SEGMENTS || (out_of_time && held > 0) {
            return Ok(dash_media_body(id, kind, init, duration, &items));
        }
        if out_of_time {
            return Err(anyhow!("no segment taken yet"));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Serve one DASH segment, or the track's init when `number` is `None`.
///
/// Held segments first: upstream has already dropped them, so the copy taken
/// while each was newest is the only one there is.
async fn dash_segment(s: &Session, want_video: bool, number: Option<u64>) -> Result<Bytes> {
    let (name, cached_init) = {
        let st = s.dash.lock().map_err(|_| anyhow!("poisoned"))?;
        let track = if want_video { st.video.as_ref() } else { st.audio.as_ref() }
            .ok_or_else(|| anyhow!("no such track"))?;
        let hold = if want_video { &st.v } else { &st.a };
        match number {
            Some(n) => {
                if let Some(hit) = hold.held.get(&n) {
                    return Ok(hit.clone());
                }
                if hold.missed.contains(&n) {
                    return Err(anyhow!("segment {} was not taken in time", n));
                }
                (track.media.replace("$Number$", &n.to_string()), None)
            }
            None => (track.init.clone(), hold.init.clone()),
        }
    };

    if number.is_none() {
        if let Some((held_name, bytes)) = cached_init {
            if held_name == name {
                return Ok(bytes);
            }
        }
        let bytes = fetch_segment(s, &name).await?;
        if let Ok(mut st) = s.dash.lock() {
            let hold = if want_video { &mut st.v } else { &mut st.a };
            hold.init = Some((name, bytes.clone()));
        }
        return Ok(bytes);
    }

    // Not held and not written off: a capture still in flight, or a request
    // for something outside the window. Upstream answers if it is still the
    // newest, which is the only case where it can.
    fetch_segment(s, &name).await
}

async fn upstream_bytes(s: &Session, idx: u32) -> Result<Bytes> {
    if let Ok(m) = s.uris.lock() {
        if let Some(hit) = m.cache.get(&idx) {
            return Ok(hit.clone());
        }
    }
    let target = s
        .uris
        .lock()
        .ok()
        .and_then(|m| m.target(idx))
        .ok_or_else(|| anyhow!("unknown segment {}", idx))?;

    if !fetch_allowed(&target, &s.upstream_host) {
        log::warn!("[TikTokRelay] refused upstream host for @{}", s.handle);
        return Err(anyhow!("upstream host not allowed"));
    }

    let res = crate::services::http::client()
        .get(&target)
        .header("Referer", "https://www.tiktok.com/")
        .send()
        .await?;
    if !res.status().is_success() {
        return Err(anyhow!("{}", res.status()));
    }
    let bytes = res.bytes().await?;
    if let Ok(mut m) = s.uris.lock() {
        m.remember(idx, bytes.clone());
    }
    Ok(bytes)
}

/// Sniff fMP4 vs MPEG-TS so the player is told what it is actually receiving.
// --- FLV to HLS -------------------------------------------------------------
//
// For the rooms that publish only an FLV pull. The pull is one endless HTTP
// response; `flv_fmp4` cuts it into fMP4 segments on its keyframes, and this
// keeps the newest of them as the window, exactly as the DASH path keeps the
// segments TikTok's CDN has already dropped. The player sees the same HLS
// either way.

/// A pull silent for this long is dead and is reopened. Live FLV never pauses
/// anywhere near this long (an audio tag arrives every 43 ms), and every second
/// of it is a second the player sits frozen before the reconnect can help.
const FLV_STALL: Duration = Duration::from_secs(6);
/// How long a new pull may take to name its codecs before the start gives up
/// on FLV and the adapter moves on to the next delivery. They come in the
/// CDN's opening burst, measured at a tenth to half a second.
const FLV_FIRST_INIT_WAIT: Duration = Duration::from_secs(4);
/// Media a new session's first playlist waits to hold. The player starts at
/// the first segment and learns of each new one within about a second (its
/// reload interval tracks the one second segments), so three seconds keeps a
/// second in reserve at the worst moment. The CDN's opening burst usually
/// covers some of it, which is why an FLV start is quick.
const FLV_START_SECONDS: f64 = 3.0;
/// Segments held per FLV session: thirty seconds of one second segments, the
/// same span the DASH window keeps.
const FLV_WINDOW: usize = 30;
/// Reopens in a row, none of them producing a segment, before the broadcast
/// is treated as over.
const FLV_MAX_RETRIES: u32 = 5;

/// The FLV pull's own client. Not the shared one: its thirty second TOTAL
/// timeout would cut a live pull every half minute. This one bounds the
/// connect and leaves the body to `FLV_STALL`, and every redirect hop is held
/// to the same fetch posture as the first request, since a redirect is
/// otherwise a way around it.
static FLV_CLIENT: Lazy<reqwest::Client> = Lazy::new(|| {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 {
                attempt.error("too many redirects")
            } else if redirect_allowed(attempt.url()) {
                attempt.follow()
            } else {
                let host = attempt.url().host_str().unwrap_or("").to_string();
                attempt.error(format!("redirected to {}, which the relay does not fetch from", host))
            }
        }))
        .build()
        .unwrap_or_default()
});

fn redirect_allowed(url: &reqwest::Url) -> bool {
    if url.scheme() != "https" {
        return false;
    }
    let Some(host) = url.host_str().map(|h| h.to_ascii_lowercase()) else {
        return false;
    };
    host.parse::<IpAddr>().is_err()
        && !host.starts_with('[')
        && CDN_SUFFIXES.iter().any(|s| host.ends_with(s))
}

/// What one session's pull has produced.
#[derive(Default)]
struct FlvState {
    /// Init segments by epoch; a new epoch is a new codec configuration.
    inits: BTreeMap<u32, Bytes>,
    codecs: Option<String>,
    width: Option<u16>,
    height: Option<u16>,
    segs: BTreeMap<u64, FlvSeg>,
    /// Timeline breaks so far, which is what numbers each segment's
    /// discontinuity sequence.
    breaks: u64,
    /// Why the pull stopped for good, once it has.
    ended: Option<String>,
}

struct FlvSeg {
    epoch: u32,
    disc_seq: u64,
    discontinuity: bool,
    duration: f64,
    frames: u32,
    bytes: Bytes,
}

impl FlvState {
    fn take(&mut self, out: flv_fmp4::Output) {
        match out {
            flv_fmp4::Output::Init {
                epoch,
                bytes,
                codecs,
                width,
                height,
            } => {
                self.inits.insert(epoch, Bytes::from(bytes));
                self.codecs = Some(codecs);
                self.width = width;
                self.height = height;
            }
            flv_fmp4::Output::Segment(seg) => {
                if seg.discontinuity {
                    self.breaks += 1;
                }
                self.segs.insert(
                    seg.number,
                    FlvSeg {
                        epoch: seg.epoch,
                        disc_seq: self.breaks,
                        discontinuity: seg.discontinuity,
                        duration: seg.duration,
                        frames: seg.video_frames,
                        bytes: Bytes::from(seg.bytes),
                    },
                );
                // The same thirty seconds as DASH, for the same reason: the
                // player's cushion has to sit well inside it.
                while self.segs.len() > FLV_WINDOW {
                    self.segs.pop_first();
                }
                // An init no listed segment names is dead weight, except the
                // newest, which the next segment will name.
                if let Some(oldest) = self.segs.values().next().map(|g| g.epoch) {
                    let newest = self.inits.keys().next_back().copied();
                    self.inits.retain(|e, _| *e >= oldest || Some(*e) == newest);
                }
            }
        }
    }
}

/// Why one connection to the pull ended.
enum FlvStop {
    /// The network or the CDN: reopen.
    Retry(String),
    /// The stream itself: reopening would only fail the same way.
    Unplayable(String),
}

/// Hold the pull open for the session's lifetime, reopening it when the CDN
/// drops it. One muxer across every connection, so a reopen's cache replay is
/// recognised as frames already held rather than written twice.
async fn flv_pump(id: String, s: Arc<Session>) {
    let mut muxer = flv_fmp4::Muxer::new();
    let mut failures = 0u32;
    loop {
        if !still_registered(&id, &s) {
            return;
        }
        if s.idle_for() >= IDLE_ABORT {
            log::info!("[TikTokRelay] '{}' idle, releasing", id);
            remove_if_same(&id, &s);
            return;
        }
        let alive = || still_registered(&id, &s) && s.idle_for() < IDLE_ABORT;
        match flv_connection(&s, &mut muxer, &alive).await {
            Ok(produced) => {
                if produced > 0 {
                    failures = 0;
                }
            }
            Err(FlvStop::Unplayable(why)) => {
                // A warning, not info: the file log keeps only warnings unless
                // Diagnostics is on, and this is the one line that says which
                // format a room was refused for.
                log::warn!("[TikTokRelay] '{}' cannot play @{}: {}", id, s.handle, why);
                end_flv(&s, format!("@{} is streaming in a format StreamNook can't play", s.handle));
                remove_if_same(&id, &s);
                return;
            }
            Err(FlvStop::Retry(why)) => {
                log::info!("[TikTokRelay] '{}' pull for @{} dropped: {}", id, s.handle, why);
            }
        }
        failures += 1;
        if failures >= FLV_MAX_RETRIES {
            log::info!("[TikTokRelay] '{}' pull for @{} failed {} times in a row; ending", id, s.handle, failures);
            end_flv(&s, format!("@{}'s stream stopped", s.handle));
            remove_if_same(&id, &s);
            return;
        }
        tokio::time::sleep(Duration::from_secs(failures as u64)).await;
    }
}

/// An error with every cause behind it. reqwest's own text names only the
/// kind of failure ("error following redirect"), and the cause is the part
/// that says which host was refused.
fn error_chain(e: &dyn std::error::Error) -> String {
    let mut out = e.to_string();
    let mut cause = e.source();
    while let Some(c) = cause {
        out.push_str(": ");
        out.push_str(&c.to_string());
        cause = c.source();
    }
    out
}

fn end_flv(s: &Session, why: String) {
    if let Ok(mut st) = s.flv.lock() {
        st.ended.get_or_insert(why);
    }
}

/// One connection to the pull, read until it ends, stalls, or the session no
/// longer wants it. Returns how many segments it produced.
async fn flv_connection(
    s: &Session,
    muxer: &mut flv_fmp4::Muxer,
    alive: &(dyn Fn() -> bool + Sync),
) -> std::result::Result<usize, FlvStop> {
    let mut res = FLV_CLIENT
        .get(&s.upstream)
        .header("Referer", "https://www.tiktok.com/")
        .send()
        .await
        .map_err(|e| FlvStop::Retry(format!("connect: {}", error_chain(&e))))?;
    if !res.status().is_success() {
        return Err(FlvStop::Retry(format!("upstream {}", res.status())));
    }
    let mut demux = flv_fmp4::Demuxer::default();
    let mut produced = 0usize;
    loop {
        if !alive() {
            return Ok(produced);
        }
        let chunk = match tokio::time::timeout(FLV_STALL, res.chunk()).await {
            Err(_) => return Err(FlvStop::Retry(format!("no data for {} s", FLV_STALL.as_secs()))),
            Ok(Err(e)) => return Err(FlvStop::Retry(format!("read: {}", e))),
            Ok(Ok(None)) => return Ok(produced),
            Ok(Ok(Some(c))) => c,
        };
        let frames = demux.push(&chunk).map_err(|e| FlvStop::Unplayable(e.to_string()))?;
        let mut outs = Vec::new();
        for f in frames {
            outs.extend(muxer.push(f).map_err(|e| FlvStop::Unplayable(e.to_string()))?);
        }
        if outs.is_empty() {
            continue;
        }
        let mut st = s.flv.lock().map_err(|_| FlvStop::Unplayable("state poisoned".into()))?;
        for o in outs {
            if matches!(o, flv_fmp4::Output::Segment(_)) {
                produced += 1;
            }
            st.take(o);
        }
    }
}

/// Wait for the pull's first init, or for the reason it will never come.
async fn flv_first_init(s: &Session, wait: Duration) -> Result<()> {
    let deadline = Instant::now() + wait;
    loop {
        {
            let st = s.flv.lock().map_err(|_| anyhow!("poisoned"))?;
            if st.codecs.is_some() {
                return Ok(());
            }
            if let Some(why) = st.ended.clone() {
                return Err(anyhow!(why));
            }
        }
        if Instant::now() >= deadline {
            return Err(anyhow!("@{}'s stream sent nothing playable", s.handle));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// One variant, described from what the pull itself carried.
///
/// Waits briefly for the first segment so the frame rate can be stated. That
/// costs nothing: the media playlist the player asks for next waits for two
/// segments anyway.
async fn flv_master(s: &Session, id: &str) -> Result<String> {
    let deadline = Instant::now() + START_WAIT;
    loop {
        {
            let st = s.flv.lock().map_err(|_| anyhow!("poisoned"))?;
            let out_of_time = Instant::now() >= deadline;
            if let Some(codecs) = st.codecs.clone() {
                if !st.segs.is_empty() || out_of_time || st.ended.is_some() {
                    return Ok(flv_master_body(s, id, &st, &codecs));
                }
            } else if let Some(why) = st.ended.clone() {
                return Err(anyhow!(why));
            } else if out_of_time {
                return Err(anyhow!("nothing playable yet"));
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn flv_master_body(s: &Session, id: &str, st: &FlvState, codecs: &str) -> String {
    let mut attrs = format!("BANDWIDTH={}", s.bandwidth.filter(|b| *b > 0).unwrap_or(2_000_000));
    if let (Some(w), Some(h)) = (st.width, st.height) {
        attrs.push_str(&format!(",RESOLUTION={}x{}", w, h));
    }
    if let Some(seg) = st.segs.values().next_back().filter(|g| g.frames > 0 && g.duration > 0.0) {
        attrs.push_str(&format!(",FRAME-RATE={:.3}", seg.frames as f64 / seg.duration));
    }
    attrs.push_str(&format!(",CODECS=\"{}\"", codecs));
    // No `#EXT-X-INDEPENDENT-SEGMENTS`: segments after the first may open
    // mid-GOP (see `flv_fmp4`), so the claim would be false.
    format!(
        "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-STREAM-INF:{},NAME=\"{}\"\n/s/{}/flv.m3u8\n",
        attrs, s.tier, id
    )
}

async fn flv_media(s: &Session, id: &str) -> Result<String> {
    flv_media_within(s, id, START_WAIT).await
}

/// The media playlist, holding a new session's first answer until it has
/// `FLV_START_SECONDS` of media to offer.
async fn flv_media_within(s: &Session, id: &str, wait: Duration) -> Result<String> {
    let deadline = Instant::now() + wait;
    loop {
        {
            let st = s.flv.lock().map_err(|_| anyhow!("poisoned"))?;
            let held = st.segs.len();
            let held_seconds: f64 = st.segs.values().map(|g| g.duration).sum();
            let out_of_time = Instant::now() >= deadline;
            // A pull that has ended will cut nothing more, so there is no
            // cushion worth waiting for.
            if held_seconds >= FLV_START_SECONDS || (held > 0 && (out_of_time || st.ended.is_some())) {
                return Ok(flv_media_body(id, &st));
            }
            if held == 0 {
                if let Some(why) = st.ended.clone() {
                    return Err(anyhow!(why));
                }
                if out_of_time {
                    return Err(anyhow!("no segment cut yet"));
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// A live media playlist over the held window.
///
/// A segment that begins a new timeline is marked `#EXT-X-DISCONTINUITY`,
/// and names its init again when the codec configuration changed with it.
/// Once such a segment is the first one listed its mark is dropped and the
/// discontinuity sequence carries the count instead, which is how HLS keeps
/// timelines numbered across a sliding window.
fn flv_media_body(id: &str, st: &FlvState) -> String {
    // EXTINF rounded to the nearest second may not exceed the target.
    let target = st
        .segs
        .values()
        .map(|g| g.duration.round() as u64)
        .max()
        .unwrap_or(2)
        .max(1);
    let (first, disc_seq) = st
        .segs
        .iter()
        .next()
        .map(|(n, g)| (*n, g.disc_seq))
        .unwrap_or((0, 0));
    let mut out = format!(
        "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:{}\n#EXT-X-MEDIA-SEQUENCE:{}\n#EXT-X-DISCONTINUITY-SEQUENCE:{}\n",
        target, first, disc_seq
    );
    let mut epoch: Option<u32> = None;
    for (n, g) in &st.segs {
        if g.discontinuity && epoch.is_some() {
            out.push_str("#EXT-X-DISCONTINUITY\n");
        }
        if epoch != Some(g.epoch) {
            out.push_str(&format!("#EXT-X-MAP:URI=\"/s/{}/fi/{}\"\n", id, g.epoch));
            epoch = Some(g.epoch);
        }
        out.push_str(&format!("#EXTINF:{:.3},\n/s/{}/f/{}\n", g.duration, id, n));
    }
    out
}

fn media_response(bytes: Bytes) -> warp::http::Response<Bytes> {
    // A fragment cut here starts at its `moof`; TikTok's own start at `styp`.
    let is_mp4 = bytes.len() > 8 && matches!(&bytes[4..8], b"ftyp" | b"styp" | b"moof");
    let ct = if is_mp4 {
        "video/mp4"
    } else {
        "video/mp2t"
    };
    cors(bytes, ct)
}

fn cors(body: impl Into<Bytes>, content_type: &str) -> warp::http::Response<Bytes> {
    warp::http::Response::builder()
        .status(200)
        .header("Content-Type", content_type)
        .header("Access-Control-Allow-Origin", "*")
        .header("Cache-Control", "no-cache")
        .body(body.into())
        .unwrap_or_else(|_| warp::http::Response::new(Bytes::new()))
}

/// Answer with the reason attached, so a failure is visible in the browser
/// console where the person debugging is already looking.
fn fail_because(code: u16, why: String) -> warp::http::Response<Bytes> {
    let mut b = warp::http::Response::builder()
        .status(code)
        .header("Access-Control-Allow-Origin", "*")
        .header("Cache-Control", "no-store");
    if !why.is_empty() {
        let one_line: String = why
            .chars()
            .map(|c| if c.is_ascii_graphic() || c == ' ' { c } else { ' ' })
            .take(400)
            .collect();
        b = b.header("X-SN-Reason", one_line);
    }
    b.body(Bytes::from(why.into_bytes()))
        .unwrap_or_else(|_| warp::http::Response::new(Bytes::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(upstream: &str) -> Session {
        Session {
            upstream: upstream.to_string(),
            packaging: Packaging::Hls,
            upstream_host: reqwest::Url::parse(upstream)
                .ok()
                .and_then(|u| u.host_str().map(|h| h.to_ascii_lowercase()))
                .unwrap_or_default(),
            handle: "someone".into(),
            tier: "hd".into(),
            width: Some(720),
            height: Some(1280),
            fps: Some(30.0),
            bandwidth: Some(1_200_000),
            uris: Mutex::new(UriMap::default()),
            playlist: Mutex::new(None),
            dash: Mutex::new(DashState::default()),
            flv: Mutex::new(FlvState::default()),
            capture_task: Mutex::new(None),
            last_seen: Mutex::new(Instant::now()),
        }
    }

    #[test]
    fn routes_split_on_the_stream_id() {
        assert_eq!(route_of("/s/solo/live.m3u8"), Some(("solo", "live.m3u8")));
        assert_eq!(route_of("/s/tile-2/up/7"), Some(("tile-2", "up/7")));
        assert_eq!(route_of("/live.m3u8"), None);
        assert_eq!(route_of("/s/solo/"), None);
    }

    #[test]
    fn stream_ids_cannot_escape_their_prefix() {
        assert!(valid_id("solo"));
        assert!(valid_id("tile_3-a"));
        assert!(!valid_id("../etc"));
        assert!(!valid_id("a/b"));
        assert!(!valid_id(""));
    }

    #[test]
    fn rewrites_absolute_relative_and_attribute_uris() {
        let s = session("https://pull-hls.tiktokcdn.com/live/room/index.m3u8");
        let body = concat!(
            "#EXTM3U\n",
            "#EXT-X-TARGETDURATION:4\n",
            "#EXT-X-MAP:URI=\"init.mp4\"\n",
            "#EXTINF:4.000,\n",
            "seg1.m4s\n",
            "#EXTINF:4.000,\n",
            "https://pull-hls.tiktokcdn.com/live/room/seg2.m4s\n"
        );
        let out = rewrite_playlist(&s, body);
        // The init segment is a URI attribute, which a line-prefix rule misses.
        assert!(out.contains("#EXT-X-MAP:URI=\"up/0\""), "got: {}", out);
        // Relative and absolute media lines both come home.
        assert!(out.contains("\nup/1\n"), "got: {}", out);
        assert!(out.contains("\nup/2\n"), "got: {}", out);
        assert!(!out.contains("tiktokcdn.com"), "no upstream url may survive");
        // Tags that carry no URI are untouched.
        assert!(out.contains("#EXT-X-TARGETDURATION:4"));
    }

    #[test]
    fn an_index_is_stable_across_a_refresh() {
        let s = session("https://pull-hls.tiktokcdn.com/live/room/index.m3u8");
        let first = rewrite_playlist(&s, "#EXTINF:4.000,\nseg1.m4s\n");
        let again = rewrite_playlist(&s, "#EXTINF:4.000,\nseg1.m4s\n#EXTINF:4.000,\nseg2.m4s\n");
        assert!(first.contains("up/0"));
        // seg1 keeps index 0, so a request already in flight stays valid.
        assert!(again.contains("up/0") && again.contains("up/1"), "got: {}", again);
    }

    /// TikTok signs the directory, not the file, and the signature lives in the
    /// manifest's query. Dropping it turns every segment into a 404 that reads
    /// like a missing segment rather than an unsigned request.
    #[test]
    fn a_relative_segment_inherits_the_manifests_signature() {
        let base = reqwest::Url::parse(
            "https://pull-hls.tiktokcdn.com/stage/stream-1_hd/index.m3u8?expire=123&sign=abc",
        )
        .unwrap();
        let got = absolutize(Some(&base), "seg-7.ts").unwrap();
        assert_eq!(
            got,
            "https://pull-hls.tiktokcdn.com/stage/stream-1_hd/seg-7.ts?expire=123&sign=abc"
        );

        // A nested path resolves against the directory, still signed.
        let got = absolutize(Some(&base), "sub/seg-8.ts").unwrap();
        assert!(got.ends_with("/sub/seg-8.ts?expire=123&sign=abc"), "{got}");

        // A URI that brought its own query keeps it untouched, so a CDN that
        // signs each file individually is not rewritten into something wrong.
        let got = absolutize(Some(&base), "seg-9.ts?token=own").unwrap();
        assert!(got.ends_with("seg-9.ts?token=own"), "{got}");

        // An absolute URI is already complete.
        let got = absolutize(Some(&base), "https://other.tiktokcdn.com/a.ts").unwrap();
        assert_eq!(got, "https://other.tiktokcdn.com/a.ts");
    }

    #[test]
    fn the_relay_refuses_to_fetch_anything_but_the_cdn() {
        let host = "pull-hls.tiktokcdn.com";
        assert!(fetch_allowed("https://pull-hls.tiktokcdn.com/a.m4s", host));
        // A sibling CDN host is allowed by suffix.
        assert!(fetch_allowed("https://other.tiktokcdn-us.com/a.m4s", host));
        // Everything that would turn the relay into a proxy is refused.
        assert!(!fetch_allowed("http://pull-hls.tiktokcdn.com/a.m4s", host));
        assert!(!fetch_allowed("https://127.0.0.1/secret", host));
        assert!(!fetch_allowed("https://192.168.1.10/secret", host));
        assert!(!fetch_allowed("https://localhost/secret", host));
        assert!(!fetch_allowed("https://evil.example.com/a.m4s", host));
        assert!(!fetch_allowed("file:///etc/passwd", host));
    }

    /// A stand-in for TikTok's CDN: one media playlist carrying every URI shape
    /// that matters, plus a segment body to fetch.
    async fn mock_upstream() -> (u16, tokio::task::JoinHandle<()>) {
        let body: String = concat!(
            "#EXTM3U\n",
            "#EXT-X-VERSION:7\n",
            "#EXT-X-TARGETDURATION:4\n",
            "#EXT-X-MEDIA-SEQUENCE:100\n",
            "#EXT-X-MAP:URI=\"init.mp4\"\n",
            "#EXTINF:4.000,\n",
            "seg1.m4s\n",
            "#EXTINF:4.000,\n",
            "sub/seg2.m4s\n",
        )
        .to_string();
        let playlist = warp::path("index.m3u8")
            .map(move || {
                warp::reply::with_header(
                    body.clone(),
                    "Content-Type",
                    "application/vnd.apple.mpegurl",
                )
            })
            .boxed();
        let socket = tokio::net::TcpSocket::new_v4().unwrap();
        socket.bind(SocketAddr::from(([127, 0, 0, 1], 0))).unwrap();
        let port = socket.local_addr().unwrap().port();
        let listener = socket.listen(64).unwrap();
        let h = tokio::spawn(async move {
            warp::serve(playlist).incoming(listener).run().await;
        });
        (port, h)
    }

    async fn get(url: &str) -> (u16, String) {
        let r = reqwest::get(url).await.expect("request");
        let status = r.status().as_u16();
        (status, r.text().await.unwrap_or_default())
    }

    fn upstream_for(port: u16, tier: &str) -> Upstream {
        Upstream {
            url: format!("http://127.0.0.1:{}/index.m3u8", port),
            packaging: Packaging::Hls,
            tier: tier.to_string(),
            width: Some(720),
            height: Some(1280),
            fps: Some(30.0),
            bandwidth: Some(1_200_000),
        }
    }

    /// The whole relay, driven over real HTTP: rewriting, per-stream isolation,
    /// the refusal to proxy an address it was never meant to reach, teardown.
    #[tokio::test]
    async fn serves_rewrites_isolates_and_refuses() {
        let (up_port, _mock) = mock_upstream().await;

        let master_a = start("tile-a", "someone", upstream_for(up_port, "hd"))
            .await
            .expect("session a");
        let master_b = start("tile-b", "other", upstream_for(up_port, "sd"))
            .await
            .expect("session b");

        // The master names this session's own media playlist, not a shared one.
        let (status, body) = get(&master_a).await;
        assert_eq!(status, 200);
        assert!(body.contains("/s/tile-a/live.m3u8"), "master: {body}");
        assert!(body.contains("RESOLUTION=720x1280"), "master: {body}");
        assert!(body.contains("NAME=\"hd\""), "master: {body}");

        let base_a = master_a
            .split('?')
            .next()
            .unwrap()
            .trim_end_matches("stream.m3u8")
            .to_string();
        let (status, body) = get(&format!("{base_a}live.m3u8")).await;
        assert_eq!(status, 200, "live.m3u8: {body}");
        // Every URI shape comes home, including the one a line-prefix rule misses.
        assert!(body.contains("#EXT-X-MAP:URI=\"up/0\""), "playlist: {body}");
        assert!(body.contains("\nup/1\n"), "relative segment: {body}");
        assert!(body.contains("\nup/2\n"), "nested relative segment: {body}");
        // Nothing may leak the upstream, or the player would fetch it directly
        // and the whole reason this relay exists would be bypassed.
        assert!(!body.contains("127.0.0.1"), "upstream leaked: {body}");
        // Tags that carry no URI pass through untouched.
        assert!(body.contains("#EXT-X-MEDIA-SEQUENCE:100"), "playlist: {body}");

        // The security property, end to end: the playlist named a loopback
        // address, and the relay declines to become a proxy for it.
        let (status, body) = get(&format!("{base_a}up/1")).await;
        assert_eq!(status, 502, "loopback must be refused, got body: {body}");
        assert!(body.contains("not allowed"), "reason: {body}");

        // Two sessions, two tiers, no shared state.
        let (_, body_b) = get(&master_b).await;
        assert!(body_b.contains("NAME=\"sd\""), "master b: {body_b}");
        assert!(body_b.contains("/s/tile-b/live.m3u8"), "master b: {body_b}");

        // An id nobody started is a 404, not a panic and not someone else's stream.
        let (status, _) = get(&format!("{base_a}../tile-zzz/live.m3u8")).await;
        assert!(status == 404 || status == 400, "unknown id status {status}");

        // Teardown is immediate, not deferred to the idle sweep.
        stop("tile-a");
        let (status, _) = get(&format!("{base_a}live.m3u8")).await;
        assert_eq!(status, 404, "stopped session must stop answering");
        // Stopping one must not touch the other.
        let (status, _) = get(&master_b).await;
        assert_eq!(status, 200, "sibling session survived");

        stop_all_except("nothing");
        let (status, _) = get(&master_b).await;
        assert_eq!(status, 404, "stop_all_except cleared the rest");

        // A quality change re-enters `start` with the SAME id. The new tier has
        // to be what is served from that moment, with no window where the old
        // one still wins.
        //
        // Deliberately part of this test rather than its own: the relay's
        // server, port and registry are process-global, and a second
        // `#[tokio::test]` gets its own runtime whose drop kills the shared
        // server task out from under whichever test is still running. One
        // runtime here mirrors the app, which has exactly one for its lifetime.
        let first = start("solo", "someone", upstream_for(up_port, "sd"))
            .await
            .expect("first");
        let (_, body) = get(&first).await;
        assert!(body.contains("NAME=\"sd\""), "{body}");

        // Different milliseconds, so the two urls cannot collide by accident.
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        let second = start("solo", "someone", upstream_for(up_port, "origin"))
            .await
            .expect("second");
        // Same path, different url. The player reloads its source only when the
        // url changes, so a quality switch that handed back the SAME url would
        // leave it on the old source reading the new session's segments.
        let path = |u: &str| u.split('?').next().unwrap().to_string();
        assert_eq!(path(&first), path(&second), "the session keeps its path");
        assert_ne!(first, second, "each start must hand the player a new url");
        let (_, body) = get(&second).await;
        assert!(body.contains("NAME=\"origin\""), "{body}");
        assert!(!body.contains("NAME=\"sd\""), "old tier still served: {body}");
        stop("solo");
    }

    /// A real manifest, captured from a live room, so the parser is pinned to
    /// the shape TikTok actually publishes rather than to a guess about DASH.
    const REAL_MPD: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="dynamic" minimumUpdatePeriod="PT1.000S" timeShiftBufferDepth="PT12.000S" minBufferTime="PT2.000S" profiles="urn:mpeg:dash:profile:isoff-live:2011">
	<Period start="PT0.000S" id="dash">
		<AdaptationSet id="1" contentType="video" segmentAlignment="true" bitstreamSwitching="true">
			<Representation id="1-0" mimeType="video/mp4" codecs="avc1.64001f" bandwidth="1000000" width="720" height="1280" frameRate="25">
				<SegmentTemplate timescale="1000" availabilityTimeOffset="1.950" initialization="init-stream1-0origin.mp4" media="1790105964-chunk-stream1-0origin-$Number$.m4v" startNumber="1112">
					<SegmentTimeline>
						<S t="0" d="2000"/>
					</SegmentTimeline>
				</SegmentTemplate>
			</Representation>
		</AdaptationSet>
		<AdaptationSet id="0" contentType="audio" segmentAlignment="true" bitstreamSwitching="true">
			<Representation id="0-0" mimeType="audio/mp4" codecs="mp4a.40.2" bandwidth="65536" audioSamplingRate="24000">
				<SegmentTemplate timescale="1000" availabilityTimeOffset="1.950" initialization="init-stream0-0origin.mp4" media="1790105964-chunk-stream0-0origin-$Number$.m4a" startNumber="1112">
					<SegmentTimeline>
						<S t="0" d="2000"/>
					</SegmentTimeline>
				</SegmentTemplate>
			</Representation>
		</AdaptationSet>
	</Period>
</MPD>"#;

    #[test]
    fn reads_both_tracks_out_of_a_real_manifest() {
        let (video, audio) = parse_mpd(REAL_MPD);
        let v = video.expect("video track");
        let a = audio.expect("audio track");

        assert_eq!(v.init, "init-stream1-0origin.mp4");
        assert_eq!(v.media, "1790105964-chunk-stream1-0origin-$Number$.m4v");
        assert_eq!(v.newest, 1112, "one timeline entry means startNumber itself");
        assert_eq!(v.duration, 2.0, "2000 ticks at timescale 1000");
        assert_eq!(v.codecs, "avc1.64001f");
        assert_eq!((v.width, v.height), (Some(720), Some(1280)));

        assert_eq!(a.init, "init-stream0-0origin.mp4");
        assert_eq!(a.codecs, "mp4a.40.2");
        // Audio carries no frame size, and inventing one would put a bogus
        // RESOLUTION on the variant.
        assert_eq!((a.width, a.height), (None, None));
    }

    #[test]
    fn a_repeated_timeline_entry_counts_every_segment_it_stands_for() {
        // `r="3"` means the entry plus three more, so four segments from 1112.
        let mpd = REAL_MPD.replace(r#"<S t="0" d="2000"/>"#, r#"<S t="0" d="2000" r="3"/>"#);
        let (video, _) = parse_mpd(&mpd);
        assert_eq!(video.unwrap().newest, 1115);
    }

    #[test]
    fn the_manifest_is_where_the_frame_rate_comes_from() {
        let (video, audio) = parse_mpd(REAL_MPD);
        assert_eq!(video.unwrap().fps, Some(25.0));
        assert_eq!(audio.unwrap().fps, None, "audio has no frame rate");
        assert_eq!(parse_frame_rate("30000/1001").map(|f| (f * 1000.0).round()), Some(29970.0));
        assert_eq!(parse_frame_rate("0"), None);
        assert_eq!(parse_frame_rate("30/0"), None);
        assert_eq!(parse_frame_rate("junk"), None);
    }

    #[test]
    fn capture_planning_starts_the_tip_and_writes_off_what_is_gone() {
        // Cold start: the newest is already in production and would arrive
        // without its keyframe, so nothing is taken yet.
        assert_eq!(plan_captures(None, 100, 2), (vec![], vec![]));
        // Same tip as last time: nothing new.
        assert_eq!(plan_captures(Some(100), 100, 2), (vec![], vec![]));
        // The normal case: one new segment.
        assert_eq!(plan_captures(Some(100), 101, 2), (vec![101], vec![]));
        // A late read: the ones within reach are tried.
        assert_eq!(plan_captures(Some(100), 103, 2), (vec![101, 102, 103], vec![]));
        // Very late: the unreachable ones are written off, and only within the
        // window, so a long stall cannot produce an unbounded list.
        let (start, missed) = plan_captures(Some(100), 200, 2);
        assert_eq!(start, vec![198, 199, 200]);
        // Written off only as far back as the window reaches.
        assert_eq!(missed, (200 - (DASH_WINDOW - 1)..198).collect::<Vec<_>>());
        assert!(missed.len() < DASH_WINDOW as usize);
    }

    #[test]
    fn a_session_starts_with_the_first_segment_it_can_take_whole() {
        let s = session("https://pull-f5-tt01.tiktokcdn.com/stage/stream-1_hd/index.mpd");
        let (video, audio) = parse_mpd(REAL_MPD);
        let (video, audio) = (video.expect("video"), audio.expect("audio"));
        let tip = video.newest;

        // The first read takes nothing, for either track, and lists nothing.
        let jobs = apply_manifest(&s, Some(video.clone()), Some(audio.clone()));
        assert!(jobs.is_empty(), "took the in-progress segment: {jobs:?}");
        {
            let st = s.dash.lock().unwrap();
            assert_eq!(st.v.started_through, Some(tip));
            assert!(st.v.listable().is_empty() && st.a.listable().is_empty());
        }

        // The next segment is taken on both tracks as soon as it appears.
        let next = |t: &DashTrack| DashTrack { newest: t.newest + 1, ..t.clone() };
        let jobs = apply_manifest(&s, Some(next(&video)), Some(next(&audio)));
        let numbers: Vec<(bool, u64)> = jobs.iter().map(|(v, _, n)| (*v, *n)).collect();
        assert_eq!(numbers, vec![(true, tip + 1), (false, tip + 1)]);
        assert!(jobs[0].1.ends_with(&format!("{}.m4v", tip + 1)), "{jobs:?}");
    }

    fn hold_with(held: &[u64], missed: &[u64], started_through: u64) -> TrackHold {
        TrackHold {
            held: held.iter().map(|n| (*n, Bytes::from_static(b"x"))).collect(),
            missed: missed.iter().copied().collect(),
            started_through: Some(started_through),
            init: None,
        }
    }

    #[test]
    fn only_a_resolved_run_is_listed() {
        // Clean: everything held.
        let h = hold_with(&[10, 11, 12], &[], 12);
        assert_eq!(h.listable(), vec![(10, false), (11, false), (12, false)]);

        // A written-off segment stays in place as a gap.
        let h = hold_with(&[10, 12], &[11], 12);
        assert_eq!(h.listable(), vec![(10, false), (11, true), (12, false)]);

        // A segment still in flight ends the run: announcing it as a gap would
        // make the player skip it for good although it is about to arrive.
        let h = hold_with(&[10, 12], &[], 12);
        assert_eq!(h.listable(), vec![(10, false)]);

        // Trailing gaps are dropped; there is nothing after them to play.
        let h = hold_with(&[10], &[11, 12], 12);
        assert_eq!(h.listable(), vec![(10, false)]);

        // Nothing held yet means nothing to list.
        let h = hold_with(&[], &[10], 10);
        assert!(h.listable().is_empty());
    }

    #[test]
    fn the_window_is_bounded_whatever_the_stream_length() {
        let mut h = hold_with(&(1..=100).collect::<Vec<_>>(), &[], 100);
        h.prune();
        assert_eq!(h.held.len(), DASH_WINDOW as usize);
        assert_eq!(h.held.keys().next(), Some(&(100 - DASH_WINDOW + 1)));
    }

    #[test]
    fn the_media_sequence_is_the_segment_number_even_across_a_gap() {
        let body = dash_media_body("t1", "v", "iv", 2.0, &[(10, false), (11, true), (12, false)]);
        assert!(body.contains("#EXT-X-MEDIA-SEQUENCE:10"), "{body}");
        assert!(body.contains("#EXT-X-VERSION:8"), "a gap needs protocol 8: {body}");
        assert!(body.contains("#EXT-X-MAP:URI=\"/s/t1/iv\""), "{body}");
        // The gap keeps its position, so segment 12 is still the third entry.
        let entries: Vec<&str> = body.lines().filter(|l| l.starts_with("/s/")).collect();
        assert_eq!(entries, vec!["/s/t1/v/10", "/s/t1/v/11", "/s/t1/v/12"]);
        let gap_at = body.find("#EXT-X-GAP").expect("gap tag");
        assert!(gap_at < body.find("/s/t1/v/11").unwrap(), "the tag belongs to segment 11");
        assert!(gap_at > body.find("/s/t1/v/10").unwrap());

        let clean = dash_media_body("t1", "a", "ia", 2.0, &[(5, false), (6, false)]);
        assert!(clean.contains("#EXT-X-VERSION:7"), "no gap, no version bump: {clean}");
        assert!(!clean.contains("GAP"));
    }

    #[tokio::test]
    async fn an_audio_only_manifest_gets_an_audio_only_master() {
        let s = session("https://pull-f5-tt01.tiktokcdn.com/stage/stream-1_ao/index.mpd");
        let (_, audio) = parse_mpd(REAL_MPD);
        s.dash.lock().unwrap().audio = audio;
        let master = dash_master(&s, "t1").await.expect("master");
        assert!(master.contains("/s/t1/audio.m3u8"), "{master}");
        // No video playlist to point at, and no separate audio group: the audio
        // rendition IS the variant.
        assert!(!master.contains("video.m3u8"), "would 404 on the first request: {master}");
        assert!(!master.contains("#EXT-X-MEDIA"), "{master}");
        assert!(master.contains("CODECS=\"mp4a.40.2\""), "{master}");
    }

    #[tokio::test]
    async fn a_new_session_holds_its_first_playlist_until_there_is_a_cushion() {
        let s = Arc::new(session("https://pull-f5-tt01.tiktokcdn.com/stage/stream-1_hd/index.mpd"));
        let (video, _) = parse_mpd(REAL_MPD);
        {
            let mut st = s.dash.lock().unwrap();
            st.video = video;
            st.v.held.insert(100, Bytes::from_static(b"seg100"));
        }
        // The second segment lands while the request is waiting for it.
        let late = s.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            late.dash.lock().unwrap().v.held.insert(101, Bytes::from_static(b"seg101"));
        });
        let asked = Instant::now();
        let body = dash_media_within(&s, "t1", true, Duration::from_secs(5)).await.expect("playlist");
        assert!(asked.elapsed() >= Duration::from_millis(250), "answered with one segment: {body}");
        assert!(body.contains("/s/t1/v/100") && body.contains("/s/t1/v/101"), "{body}");

        // Once the hold has its cushion, a reload answers at once.
        let asked = Instant::now();
        dash_media_within(&s, "t1", true, Duration::from_secs(5)).await.expect("reload");
        assert!(asked.elapsed() < Duration::from_millis(100), "a warm reload waited");
    }

    #[tokio::test]
    async fn a_room_too_slow_for_a_cushion_is_served_what_it_has() {
        let s = session("https://pull-f5-tt01.tiktokcdn.com/stage/stream-1_hd/index.mpd");
        let (video, _) = parse_mpd(REAL_MPD);
        {
            let mut st = s.dash.lock().unwrap();
            st.video = video.clone();
            st.v.held.insert(100, Bytes::from_static(b"seg100"));
        }
        let body = dash_media_within(&s, "t1", true, Duration::from_millis(250)).await.expect("served");
        assert!(body.contains("/s/t1/v/100"), "{body}");

        // Nothing held at all stays an error rather than an empty playlist.
        let empty = session("https://pull-f5-tt01.tiktokcdn.com/stage/stream-1_hd/index.mpd");
        empty.dash.lock().unwrap().video = video;
        assert!(dash_media_within(&empty, "t1", true, Duration::from_millis(250)).await.is_err());
    }

    fn flv_session(upstream: &str) -> Session {
        let mut s = session(upstream);
        s.packaging = Packaging::Flv;
        s
    }

    /// A throwaway HTTP server answering each path with a fixed response.
    async fn mock_http(routes: Vec<(&'static str, u16, Vec<(&'static str, String)>, Vec<u8>)>) -> u16 {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let routes = Arc::new(routes);
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                let routes = routes.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 4096];
                    let n = sock.read(&mut buf).await.unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]).to_string();
                    let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
                    let (status, headers, body) = routes
                        .iter()
                        .find(|r| r.0 == path)
                        .map(|r| (r.1, r.2.clone(), r.3.clone()))
                        .unwrap_or((404, Vec::new(), Vec::new()));
                    let mut head = format!(
                        "HTTP/1.1 {} X\r\nContent-Length: {}\r\nConnection: close\r\n",
                        status,
                        body.len()
                    );
                    for (k, v) in headers {
                        head.push_str(&format!("{}: {}\r\n", k, v));
                    }
                    head.push_str("\r\n");
                    let _ = sock.write_all(head.as_bytes()).await;
                    let _ = sock.write_all(&body).await;
                    let _ = sock.shutdown().await;
                });
            }
        });
        port
    }

    #[tokio::test]
    async fn an_flv_pull_becomes_a_playable_window() {
        let port = mock_http(vec![
            (
                "/live.flv",
                200,
                vec![("Content-Type", "video/x-flv".into())],
                flv_fmp4::tests::sample_flv(6_000_000, 6_008_100),
            ),
            ("/hevc.flv", 200, Vec::new(), flv_fmp4::tests::hevc_flv()),
            (
                "/bounce.flv",
                302,
                vec![("Location", "http://127.0.0.1:9/elsewhere.flv".into())],
                Vec::new(),
            ),
        ])
        .await;

        let s = flv_session(&format!("http://127.0.0.1:{}/live.flv", port));
        let mut muxer = flv_fmp4::Muxer::new();
        let produced = match flv_connection(&s, &mut muxer, &|| true).await {
            Ok(n) => n,
            Err(FlvStop::Retry(e)) | Err(FlvStop::Unplayable(e)) => panic!("pull failed: {e}"),
        };
        // Eight one second segments out of four GOPs; the ninth is still open
        // when the body ends, and is kept for the reconnect rather than cut short.
        assert_eq!(produced, 8);
        {
            let st = s.flv.lock().unwrap();
            let codecs = st.codecs.clone().expect("codecs from the pull");
            assert_eq!(codecs, "avc1.640020,mp4a.40.2");
            let master = flv_master_body(&s, "t1", &st, &codecs);
            assert!(master.contains("RESOLUTION=720x1280"), "{master}");
            assert!(master.contains("FRAME-RATE=25.000"), "{master}");
            assert!(master.contains("CODECS=\"avc1.640020,mp4a.40.2\""), "{master}");
            assert!(master.contains("/s/t1/flv.m3u8"), "{master}");
            let body = flv_media_body("t1", &st);
            assert!(body.contains("#EXT-X-MAP:URI=\"/s/t1/fi/0\""), "{body}");
            assert!(body.contains("#EXT-X-TARGETDURATION:1\n"), "{body}");
            assert!(body.contains("/s/t1/f/0\n") && body.contains("/s/t1/f/7\n"), "{body}");
            assert!(!master.contains("INDEPENDENT-SEGMENTS"), "segments may open mid-GOP: {master}");
            assert!(!body.contains("DISCONTINUITY\n"), "one timeline: {body}");
            assert_eq!(&st.segs[&0].bytes[4..8], b"moof");
            assert_eq!(&st.inits[&0][4..8], b"ftyp");
        }
        // A new session's playlist already has its cushion, so it answers now.
        let asked = Instant::now();
        flv_media_within(&s, "t1", Duration::from_secs(5)).await.expect("playlist");
        assert!(asked.elapsed() < Duration::from_millis(100));

        // A codec the page cannot be handed is the stream's fault, not the
        // network's: reopening would only fail the same way.
        let hevc = flv_session(&format!("http://127.0.0.1:{}/hevc.flv", port));
        let mut m2 = flv_fmp4::Muxer::new();
        assert!(matches!(
            flv_connection(&hevc, &mut m2, &|| true).await,
            Err(FlvStop::Unplayable(_))
        ));

        // A redirect off TikTok's CDN is refused, which is the fetch posture
        // holding past the first request.
        let bounce = flv_session(&format!("http://127.0.0.1:{}/bounce.flv", port));
        let mut m3 = flv_fmp4::Muxer::new();
        match flv_connection(&bounce, &mut m3, &|| true).await {
            Err(FlvStop::Retry(why)) => assert!(why.contains("does not fetch from"), "{why}"),
            Ok(_) | Err(FlvStop::Unplayable(_)) => panic!("followed a redirect off the CDN"),
        }
    }

    fn flv_seg(epoch: u32, discontinuity: bool) -> flv_fmp4::Output {
        // Numbered by the caller through `take`'s map key, so this only
        // describes the segment.
        flv_fmp4::Output::Segment(flv_fmp4::Segment {
            number: 0,
            epoch,
            discontinuity,
            duration: 2.0,
            video_frames: 50,
            bytes: b"....moof".to_vec(),
        })
    }

    #[test]
    fn a_timeline_break_is_marked_and_counted_across_the_window() {
        let mut st = FlvState::default();
        for (n, (epoch, disc)) in [(0, false), (0, false), (1, true), (1, false)].into_iter().enumerate() {
            let flv_fmp4::Output::Segment(mut seg) = flv_seg(epoch, disc) else { unreachable!() };
            seg.number = 10 + n as u64;
            st.take(flv_fmp4::Output::Segment(seg));
        }
        let body = flv_media_body("t1", &st);
        assert!(body.contains("#EXT-X-MEDIA-SEQUENCE:10\n#EXT-X-DISCONTINUITY-SEQUENCE:0\n"), "{body}");
        assert!(body.starts_with("#EXTM3U"));
        assert!(
            body.contains("/s/t1/f/11\n#EXT-X-DISCONTINUITY\n#EXT-X-MAP:URI=\"/s/t1/fi/1\"\n#EXTINF:2.000,\n/s/t1/f/12\n"),
            "{body}"
        );

        // Once the break is the first segment listed, its mark goes and the
        // sequence carries it instead.
        st.segs.remove(&10);
        st.segs.remove(&11);
        let body = flv_media_body("t1", &st);
        assert!(body.contains("#EXT-X-MEDIA-SEQUENCE:12\n#EXT-X-DISCONTINUITY-SEQUENCE:1\n"), "{body}");
        assert!(!body.contains("#EXT-X-DISCONTINUITY\n"), "{body}");
        assert!(body.contains("#EXT-X-MAP:URI=\"/s/t1/fi/1\""), "{body}");
    }

    #[test]
    fn the_flv_window_is_bounded_and_drops_inits_nothing_names() {
        let mut st = FlvState::default();
        for epoch in 0..3u32 {
            st.take(flv_fmp4::Output::Init {
                epoch,
                bytes: b"....ftyp".to_vec(),
                codecs: "avc1.640020,mp4a.40.2".into(),
                width: Some(720),
                height: Some(1280),
            });
        }
        for n in 0..40u64 {
            let flv_fmp4::Output::Segment(mut seg) = flv_seg(if n < 30 { 1 } else { 2 }, n == 30) else {
                unreachable!()
            };
            seg.number = n;
            st.take(flv_fmp4::Output::Segment(seg));
        }
        assert_eq!(st.segs.len(), FLV_WINDOW);
        assert_eq!(st.segs.keys().next(), Some(&(40 - FLV_WINDOW as u64)));
        // Epoch 0 is named by nothing left; 1 and 2 still are.
        assert_eq!(st.inits.keys().copied().collect::<Vec<_>>(), vec![1, 2]);
    }

    #[tokio::test]
    async fn a_full_manifest_gets_a_variant_plus_an_audio_group() {
        let s = session("https://pull-f5-tt01.tiktokcdn.com/stage/stream-1_hd/index.mpd");
        let (video, audio) = parse_mpd(REAL_MPD);
        {
            let mut st = s.dash.lock().unwrap();
            st.video = video;
            st.audio = audio;
        }
        let master = dash_master(&s, "t1").await.expect("master");
        assert!(master.contains("#EXT-X-MEDIA:TYPE=AUDIO"), "{master}");
        assert!(master.contains("/s/t1/video.m3u8"), "{master}");
        assert!(master.contains("RESOLUTION=720x1280"), "{master}");
        // The frame rate the manifest carries, since sdk_params never does.
        assert!(master.contains("FRAME-RATE=25.000"), "{master}");
        assert!(master.contains("CODECS=\"avc1.64001f,mp4a.40.2\""), "{master}");
    }

    #[test]
    fn a_manifest_with_nothing_playable_is_an_error_not_an_empty_success() {
        let (v, a) = parse_mpd("<MPD></MPD>");
        assert!(v.is_none() && a.is_none());
    }

    #[test]
    fn the_master_advertises_the_real_geometry() {
        let s = session("https://pull-hls.tiktokcdn.com/live/room/index.m3u8");
        let m = master_playlist(&s, "solo");
        assert!(m.contains("RESOLUTION=720x1280"), "portrait, got: {}", m);
        assert!(m.contains("FRAME-RATE=30.000"));
        assert!(m.contains("/s/solo/live.m3u8"));
    }
}
