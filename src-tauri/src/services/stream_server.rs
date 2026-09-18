use crate::services::ll_origin::{
    arc_bytes, empty_cors, media_response, opt_raw_query, parse_directive, parse_part_path,
    playlist_response,
};
use anyhow::Result;
use log::{error, info, warn};
use once_cell::sync::Lazy;
// rand 0.10 moved random_range onto the RngExt extension trait.
use rand::RngExt;
use reqwest::Client;
use std::net::SocketAddr;
use std::sync::Arc;
use tauri::{Emitter};
use tokio::sync::Mutex;
use warp::Filter;

pub struct StreamServer;

static SERVER_HANDLE: Lazy<Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>> =
    Lazy::new(|| Arc::new(Mutex::new(None)));
static PROXY_URL: Lazy<Arc<Mutex<Option<String>>>> = Lazy::new(|| Arc::new(Mutex::new(None)));
static CURRENT_PORT: Lazy<Arc<Mutex<Option<u16>>>> = Lazy::new(|| Arc::new(Mutex::new(None)));

/// How to obtain a FRESH signed manifest url for whatever is playing.
///
/// Kick's playback url is an IVS master carrying a JWT with an expiry. When it
/// lapses mid-session the origin starts refusing every fetch and playback simply
/// stops — there was recovery machinery for this (`qualities_for(slug, force)`)
/// but nothing ever called it with `force`, so an expired url was terminal.
///
/// Registered by the playback starter rather than resolved here, so this file
/// stays provider-agnostic.
type Refresher = Arc<dyn Fn() -> futures::future::BoxFuture<'static, Option<String>> + Send + Sync>;
static REFRESHER: Lazy<Arc<Mutex<Option<Refresher>>>> = Lazy::new(|| Arc::new(Mutex::new(None)));
/// One re-resolve at a time; a refusal usually arrives on several requests at once.
static REFRESHING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Register (or clear) how to re-sign the current stream's manifest url.
pub async fn set_manifest_refresher(f: Option<Refresher>) {
    *REFRESHER.lock().await = f;
}

/// An upstream refused us. If the stream knows how to re-sign itself, do it once
/// and swap the new url in; later requests read it from the same holder.
///
/// Returns whether a refresh was started, purely for logging.
async fn refresh_manifest_after_refusal(status: u16) -> bool {
    use std::sync::atomic::Ordering;
    let Some(refresher) = REFRESHER.lock().await.clone() else {
        return false;
    };
    if REFRESHING.swap(true, Ordering::SeqCst) {
        return false;
    }
    warn!(
        "[StreamServer] upstream returned {}; re-signing the manifest url",
        status
    );
    tokio::spawn(async move {
        match refresher().await {
            Some(fresh) => {
                *PROXY_URL.lock().await = Some(fresh);
                info!("[StreamServer] manifest url re-signed; playback should recover");
            }
            None => warn!("[StreamServer] could not re-sign the manifest url"),
        }
        REFRESHING.store(false, Ordering::SeqCst);
    });
    true
}
/// One re-sign at a time, and the WINNER's url is handed to everyone who queued
/// behind it. Separate from `REFRESHING` (which guards the fire-and-forget
/// segment path) because the playlist path needs the resulting url, not just to
/// know that a refresh started.
static RESIGN_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

/// Re-sign now and hand back the fresh url.
///
/// Unlike `refresh_manifest_after_refusal`, this AWAITS the result so the caller
/// can retry its fetch inline. Callers queued behind the lock re-read the holder
/// first: if the winner already replaced the stale url, they reuse it rather than
/// re-signing again (a refusal usually arrives on several requests at once).
async fn resign_blocking(stale: &str) -> Option<String> {
    let _guard = RESIGN_LOCK.lock().await;
    if let Some(current) = PROXY_URL.lock().await.clone() {
        if current != stale {
            return Some(current);
        }
    }
    let refresher = REFRESHER.lock().await.clone()?;
    let fresh = refresher().await?;
    *PROXY_URL.lock().await = Some(fresh.clone());
    info!("[StreamServer] manifest url re-signed for a refused playlist request");
    Some(fresh)
}

/// Local request path -> upstream url, against a given manifest url.
///
/// Pulled out of the handler so the same mapping can be re-applied to a freshly
/// re-signed manifest on retry; the query string carries the upstream's auth and
/// must ride along on variant and chunk requests too.
fn map_upstream_url(manifest_url: &str, request_path: &str) -> String {
    if request_path == "stream.m3u8" || request_path.is_empty() {
        return manifest_url.to_string();
    }
    let (url_without_query, query) = match manifest_url.find('?') {
        Some(q_idx) => (&manifest_url[..q_idx], &manifest_url[q_idx..]),
        None => (manifest_url, ""),
    };
    let base_url = match url_without_query.rfind('/') {
        Some(last_slash) => &url_without_query[..=last_slash],
        None => url_without_query,
    };
    format!("{}{}{}", base_url, request_path, query)
}

/// Serializes solo `start_proxy_server` calls end to end. The existence check,
/// `ll_origin::start` (a multi-second backfill await), and the SERVER_HANDLE write
/// are otherwise non-atomic, so two concurrent cold starts (e.g. a fast channel
/// re-trigger) could both see "no server", both bind warp, and both run the shared
/// SOLO origin's start — one silently wiping the other's edge mid-setup. Held for
/// the whole function so the second caller observes a fully-initialized server and
/// takes the reuse path.
static SOLO_START_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

/// Global HTTP client with optimized connection pooling
static HTTP_CLIENT: Lazy<Client> = Lazy::new(|| {
    Client::builder()
        .tcp_keepalive(std::time::Duration::from_secs(15))
        .pool_idle_timeout(std::time::Duration::from_secs(30))
        .timeout(std::time::Duration::from_secs(10))
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .build()
        .expect("Failed to build global HTTP client")
});

// Live ad-detection state for the solo stream. The marker logic is shared with
// MultiNook via `ad_detect` (single source of truth). The player re-polls the
// media playlist through this server every few seconds, so scanning those bytes
// is free (no extra requests) and closes the "trust the proxy blindly" gap.
pub use crate::services::ad_detect::AdDetectionState;

static AD_STATE: Lazy<std::sync::Mutex<AdDetectionState>> =
    Lazy::new(|| std::sync::Mutex::new(AdDetectionState::default()));

/// Snapshot the current ad-detection state (for the Tauri command / the pivot).
pub fn ad_state() -> AdDetectionState {
    AD_STATE.lock().unwrap().clone()
}

fn reset_ad_state() {
    *AD_STATE.lock().unwrap() = AdDetectionState::default();
    // Tear down the LL-HLS origin (background reader + live edge) for the old stream.
    crate::services::ll_origin::stop();
    // Drop the stable-projection segment map for the old stream so a synthetic
    // `vseg/<sn>.ts` can never resolve to a previous stream's segment.
    crate::services::hls_projection::reset(SOLO_STREAM_ID);
}

/// Build the relay's playlist/segment response with the headers the player
/// needs. Shared by the Twitch path and the generic-HLS early return.
fn finish_playlist_response(
    status: warp::http::StatusCode,
    request_path: &str,
    bytes: Vec<u8>,
) -> Result<warp::http::Response<bytes::Bytes>, warp::Rejection> {
    let content_type = if request_path.ends_with(".ts") {
        "video/MP2T"
    } else {
        "application/x-mpegURL"
    };
    Ok(warp::http::Response::builder()
        .status(status)
        .header("Content-Type", content_type)
        .header("Access-Control-Allow-Origin", "*")
        .header("Access-Control-Allow-Methods", "GET, OPTIONS")
        .header("Access-Control-Allow-Headers", "*")
        .header(
            "Cache-Control",
            "no-cache, no-store, must-revalidate, max-age=0",
        )
        .header("Pragma", "no-cache")
        .header("Expires", "0")
        .body(bytes.into())
        .unwrap())
}

/// Fold a relayed media playlist into the solo stream's ad-detection state.
/// Read-only and for the core's own playback only: the relay serves the
/// playlist untouched and only RECORDS whether ad markers are present, which
/// gates the low-latency prefetch promotion (`ads_now`) so it never
/// fast-forwards into an ad. The core never acts on ads beyond that and never
/// reports them to a plugin; a resolution-owning plugin detects ads itself.
fn detect_ads_in_playlist(playlist: &str) {
    let mut st = AD_STATE.lock().unwrap();
    if let Some(n) = crate::services::ad_detect::update(&mut st, playlist) {
        info!(
            "[StreamServer] ad markers detected in live playlist (break #{}): {:?}",
            n, st.matched_markers
        );
    }
}

/// The stream id the solo relay session is addressed by in the plugin
/// protocol (`set_upstream`). MultiNook tiles use their own per-tile ids.
pub const SOLO_STREAM_ID: &str = "solo";

/// Which upstream the relay is serving, which decides how much of the
/// playlist pipeline runs.
///
/// `Twitch` is the full path. `GenericHls` (Kick, YouTube) skips the three
/// Twitch-CONTRACTUAL steps — SSAI ad detection, `vseg/` segment projection
/// (which exists to absorb Twitch re-signing its segment paths), and the LL
/// origin probe (keyed on `#EXT-X-TWITCH-PREFETCH`). It deliberately KEEPS the
/// TARGETDURATION retarget: that is pure `#EXTINF` math, and these platforms
/// over-declare it the same way Twitch does (Kick ships `TARGETDURATION:6` for
/// ~2.35s segments), so the latency win applies verbatim.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UpstreamProfile {
    Twitch,
    GenericHls,
}

static UPSTREAM_PROFILE: Lazy<std::sync::Mutex<UpstreamProfile>> =
    Lazy::new(|| std::sync::Mutex::new(UpstreamProfile::Twitch));

/// Set by the start path before the relay is handed a new upstream.
pub fn set_upstream_profile(profile: UpstreamProfile) {
    if let Ok(mut p) = UPSTREAM_PROFILE.lock() {
        *p = profile;
    }
    // A new stream means the previous stream's segment URLs are meaningless.
    if let Ok(mut m) = pseg_map().lock() {
        m.clear();
    }
}

/// Absolute segment URLs, addressed by a relay-local index.
///
/// Twitch's media playlists use RELATIVE segment URIs, so they resolve against
/// this relay and inherit its CORS header for free. YouTube's are ABSOLUTE
/// googlevideo URLs, which the player would fetch directly — and googlevideo
/// sends no `access-control-allow-origin`, so the browser blocks it and playback
/// never starts. Rewriting those to `pseg/<n>` routes the bytes back through
/// here, where the header is ours to set.
///
/// Rust is not a browser and has no CORS rules, so fetching them from this side
/// is exactly the part that is allowed.
#[derive(Default)]
struct PsegMap {
    /// index -> already-downloaded bytes, for segments fetched ahead of the
    /// player. warp 0.4 offers no public streaming body (its `wrap_stream` is
    /// crate-private), so a segment must be whole before it can be served —
    /// which serialises a full download in front of playback and shows up as a
    /// stall while the buffer fills. Prefetching moves that download to BEFORE
    /// the request instead of during it, using the playlist as the schedule: it
    /// names precisely which segments the player is about to ask for.
    /// Bodies are `Bytes`, so a cache hit serves by refcount instead of copying.
    cache: std::collections::HashMap<u32, bytes::Bytes>,
    /// Indices already being fetched, so a prefetch and a live request can't
    /// download the same segment twice.
    inflight: std::collections::HashSet<u32>,
    /// index -> absolute URL.
    by_index: std::collections::HashMap<u32, String>,
    /// absolute URL -> index, so a segment that survives a playlist refresh keeps
    /// the SAME local URI. Handing the player a new URI for a segment it already
    /// has would make it re-download the sliding window on every refresh.
    by_url: std::collections::HashMap<String, u32>,
    next: u32,
}

impl PsegMap {
    fn clear(&mut self) {
        self.by_index.clear();
        self.by_url.clear();
        self.cache.clear();
        self.inflight.clear();
        // `next` deliberately keeps counting: reusing indices across streams
        // could hand the player a cached URI pointing at different bytes.
    }

    fn intern(&mut self, url: &str) -> u32 {
        if let Some(i) = self.by_url.get(url) {
            return *i;
        }
        let idx = self.next;
        self.next = self.next.wrapping_add(1);
        self.by_url.insert(url.to_string(), idx);
        self.by_index.insert(idx, url.to_string());
        // Bound it. A live window is a handful of segments; this keeps enough
        // history for a slow client without growing across a long session.
        if self.by_index.len() > 512 {
            let cutoff = idx.saturating_sub(256);
            self.by_index.retain(|k, _| *k > cutoff);
            self.by_url.retain(|_, v| *v > cutoff);
        }
        idx
    }

    /// Drop prefetched bodies the player has moved past. Bytes are the expensive
    /// thing here, so this window is much tighter than the URL map's: a live
    /// playlist only ever references a handful of segments at once.
    fn trim_cache(&mut self, newest: u32) {
        const KEEP: u32 = 8;
        let cutoff = newest.saturating_sub(KEEP);
        self.cache.retain(|k, _| *k > cutoff);
    }
}

/// Fetch a segment into the cache unless it is already there or already being
/// fetched. Called for every segment a freshly-rewritten playlist names, so the
/// bytes are usually resident before the player asks for them.
fn prefetch_segment(index: u32) {
    let url = {
        let Ok(mut m) = pseg_map().lock() else { return };
        if m.cache.contains_key(&index) || m.inflight.contains(&index) {
            return;
        }
        let Some(url) = m.by_index.get(&index).cloned() else {
            return;
        };
        m.inflight.insert(index);
        url
    };
    tokio::spawn(async move {
        let fetched = match HTTP_CLIENT.get(&url).send().await {
            // The reqwest body is already one contiguous `Bytes`; store it as is.
            Ok(res) if res.status().is_success() => res.bytes().await.ok(),
            // A failure here is not worth reporting: the player will request the
            // segment itself in a moment and that path logs properly.
            _ => None,
        };
        if let Ok(mut m) = pseg_map().lock() {
            m.inflight.remove(&index);
            if let Some(bytes) = fetched {
                m.cache.insert(index, bytes);
                m.trim_cache(index);
            }
        }
    });
}

static PSEG: Lazy<std::sync::Mutex<PsegMap>> = Lazy::new(|| std::sync::Mutex::new(PsegMap::default()));

fn pseg_map() -> &'static std::sync::Mutex<PsegMap> {
    &PSEG
}

/// Rewrite absolute `http(s)://` segment URIs in a media playlist to `pseg/<n>`.
/// Comment lines are left alone, so tags keep whatever absolute URLs they carry
/// (a `#EXT-X-MAP` URI is handled by the same rule when it stands on its own).
fn rewrite_absolute_segments(playlist: &str) -> String {
    let mut out = String::with_capacity(playlist.len());
    let mut seen_indices: Vec<u32> = Vec::new();
    for line in playlist.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
            let idx = match pseg_map().lock() {
                Ok(mut m) => m.intern(trimmed),
                Err(_) => {
                    out.push_str(line);
                    out.push('\n');
                    continue;
                }
            };
            seen_indices.push(idx);
            out.push_str(&format!("pseg/{}", idx));
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }

    // Prefetch ONLY the newest few segments, never the whole playlist. A live
    // YouTube playlist carries the entire DVR window — dozens of segments — and
    // firing a download for each one saturates the connection and starves the
    // single segment the player is actually waiting for. The player consumes
    // from the live edge, so the tail is the only part worth fetching early.
    const PREFETCH_AHEAD: usize = 3;
    let start = seen_indices.len().saturating_sub(PREFETCH_AHEAD);
    for idx in &seen_indices[start..] {
        prefetch_segment(*idx);
    }
    out
}

/// `pseg/<n>` -> the absolute URL it stands for.
fn pseg_target(request_path: &str) -> Option<String> {
    let n: u32 = request_path.strip_prefix("pseg/")?.parse().ok()?;
    pseg_map().lock().ok()?.by_index.get(&n).cloned()
}

pub fn upstream_profile() -> UpstreamProfile {
    UPSTREAM_PROFILE
        .lock()
        .map(|p| *p)
        .unwrap_or(UpstreamProfile::Twitch)
}

/// The channel the solo relay is currently serving, when it is a live stream
/// (None for VOD/clip playback and when stopped). This is what makes the solo
/// session addressable by plugins.
static SOLO_CHANNEL: Lazy<std::sync::Mutex<Option<String>>> =
    Lazy::new(|| std::sync::Mutex::new(None));
static APP_HANDLE: once_cell::sync::OnceCell<tauri::AppHandle> = once_cell::sync::OnceCell::new();
/// True when the LL-HLS origin is actively serving parts for this stream — what the
/// player keys `lowLatencyMode` on. A real spec LL-HLS playlist (`#EXT-X-PART` +
/// blocking reload) is being served, so hls.js's native low-latency controller has
/// parts to consume. Driven by the experimental-low-latency setting (the origin kill
/// switch); false on the stable whole-segment path.
pub fn is_low_latency() -> bool {
    crate::services::ll_origin::is_active()
}

/// Store the app handle so the relay can emit reload events and reach the
/// plugin host for `on_ad_window` notifications.
pub fn set_app_handle(handle: tauri::AppHandle) {
    let _ = APP_HANDLE.set(handle);
}

/// Record (or clear) the live channel the solo relay serves. Live starts set
/// it; VOD/clip starts and stops clear it.
pub fn set_solo_session(channel: Option<String>) {
    *SOLO_CHANNEL.lock().unwrap() = channel;
}

/// True while the solo relay is serving a live channel (the precondition for
/// a plugin to swap its upstream).
pub fn solo_session_active() -> bool {
    SOLO_CHANNEL.lock().unwrap().is_some()
}

/// Replace the solo relay's upstream playlist with one a resolution-owning
/// plugin supplied via `set_upstream`, and tell the player to reload onto it.
/// This is the mid-stream escalation path (e.g. the plugin re-resolved through
/// a different region after a leaked ad window).
pub async fn swap_upstream(playlist_url: String) -> Result<()> {
    let channel = SOLO_CHANNEL
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| anyhow::anyhow!("no live solo relay session"))?;
    let port = StreamServer::start_proxy_server(playlist_url).await?;
    if let Some(app) = APP_HANDLE.get() {
        let url = format!(
            "http://localhost:{}/stream.m3u8?t={}",
            port,
            chrono::Utc::now().timestamp_millis()
        );
        let _ = app.emit(
            "ad-pivot",
            serde_json::json!({ "url": url, "channel": channel }),
        );
    }
    info!(
        "[StreamServer] {} upstream swapped by a playback plugin",
        channel
    );
    Ok(())
}

impl StreamServer {
    /// Serve `stream_url` through the relay, probing the low-latency origin
    /// (the live default).
    pub async fn start_proxy_server(stream_url: String) -> Result<u16> {
        Self::start_proxy_server_with(stream_url, true).await
    }

    /// `probe_ll: false` for VOD playlists: they never carry the prefetch
    /// tags the origin keys on, and a recording VOD playlist is thousands of
    /// lines, so the probe would only pay one big fetch to learn "inactive".
    pub async fn start_proxy_server_with(stream_url: String, probe_ll: bool) -> Result<u16> {
        // The upstream media-playlist URL the LL-HLS origin will poll. `reset_ad_state`
        // (below) stops any prior origin; `ll_origin::start` probes this URL and, if it's
        // a low-latency broadcast, builds the live edge before we return — so the player
        // can read `get_stream_low_latency` and pick the right hls.js mode.
        let upstream = stream_url.clone();

        // Serialize the whole start sequence (TOCTOU guard): without this two
        // concurrent cold starts could both spawn a warp server and both run the
        // shared SOLO origin's start, racing each other's edge setup.
        let _init_guard = SOLO_START_LOCK.lock().await;

        // Check if server is already running
        let server_exists = SERVER_HANDLE.lock().await.is_some();

        if server_exists {
            // Server already running - just update the URL
            *PROXY_URL.lock().await = Some(stream_url);
            // New stream on the existing server: clear stale ad-detection state.
            reset_ad_state();
            // Bring up the parts-based LL-HLS origin for the new stream (a no-op unless
            // the experimental setting enabled it and the channel is low-latency).
            // Skipped outright for non-Twitch upstreams: the origin keys on
            // `#EXT-X-TWITCH-PREFETCH`, which they never emit (Kick ships plain
            // `#EXT-X-PREFETCH`), so probing only costs a multi-second backfill
            // fetch to reach the same "inactive" answer.
            if probe_ll && upstream_profile() == UpstreamProfile::Twitch {
                let outcome = crate::services::ll_origin::start(upstream).await;
                log::debug!(
                    "[StreamServer] LL origin start (reuse): active={}",
                    outcome.active
                );
            } else {
                crate::services::ll_origin::stop();
            }
            // Return the existing port by parsing it from a static variable
            return Self::get_current_port().await;
        }

        // Start new server
        let port = rand::rng().random_range(10000..20000);

        *PROXY_URL.lock().await = Some(stream_url);
        reset_ad_state();
        if probe_ll && upstream_profile() == UpstreamProfile::Twitch {
            let outcome = crate::services::ll_origin::start(upstream).await;
            log::debug!("[StreamServer] LL origin start: active={}", outcome.active);
        }

        // Store the port
        *CURRENT_PORT.lock().await = Some(port);

        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let proxy_url_clone = PROXY_URL.clone();

        // Use a wildcard proxy that catches ALL paths so relative chunks are automatically mapped.
        // The raw query string is captured too, for the LL-HLS blocking-reload directives
        // (`_HLS_msn`/`_HLS_part`).
        let proxy = warp::path::full()
            .and(warp::method())
            .and(opt_raw_query())
            .and(warp::any().map(move || proxy_url_clone.clone()))
            .and_then(Self::dynamic_proxy_handler)
            .boxed();

        // Bind with a deep listen backlog. The relay shares the app's async
        // runtime with the CPU-bound TS->fMP4 transmux, so a transmux burst can
        // briefly keep the acceptor task off a worker thread. With the default
        // (small) backlog the OS then refuses the next connection (an RST, which
        // hls.js reports as ERR_CONNECTION_REFUSED, dropping a part and setting
        // off a fragGap cascade). A deep backlog turns that momentary stall into
        // a sub-second queue wait instead of a hard refusal.
        let socket = tokio::net::TcpSocket::new_v4()?;
        socket.set_reuseaddr(true)?;
        socket.bind(addr)?;
        let listener = socket.listen(1024)?;

        let handle = tokio::spawn(async move {
            warp::serve(proxy).incoming(listener).run().await;
        });

        *SERVER_HANDLE.lock().await = Some(handle);

        Ok(port)
    }
    async fn dynamic_proxy_handler(
        path: warp::path::FullPath,
        method: warp::http::Method,
        raw_query: String,
        proxy_url: Arc<Mutex<Option<String>>>,
    ) -> Result<warp::http::Response<bytes::Bytes>, warp::Rejection> {
        // None means stop() already ran. The abort only kills the accept loop;
        // hyper's per-connection tasks survive it, so the player's keep-alive
        // connection can still deliver a final poll. A warp rejection here would
        // answer it with a bare 404 (no CORS headers), which the webview reports
        // as a CORS error. Answer with a CORS'd 404 so the straggler dies quietly.
        // First touch of every request. During famine incidents, playlist
        // requests provably leave hls.js and produce no o_pl for seconds: this
        // either shows them arriving here (blockage in our routing below) or
        // not arriving at all (blockage in the socket/accept layer or the
        // webview's pool) — the last unattributed hop.
        if crate::services::ll_diagnostics::is_active() {
            crate::services::ll_diagnostics::event(&format!(
                "\"ev\":\"o_req\",\"p\":{:?}",
                path.as_str()
            ));
        }
        let Some(manifest_url) = proxy_url.lock().await.clone() else {
            return Ok(empty_cors(404));
        };

        // Handle CORS Preflight INSTANTLY without hitting Twitch
        if method == warp::http::Method::OPTIONS {
            return Ok(warp::http::Response::builder()
                .status(200)
                .header("Access-Control-Allow-Origin", "*")
                .header("Access-Control-Allow-Methods", "GET, OPTIONS")
                .header("Access-Control-Allow-Headers", "*")
                .header("Access-Control-Max-Age", "86400")
                .body(bytes::Bytes::new())
                .unwrap());
        }

        let request_path = path.as_str().trim_start_matches('/');

        // ── LL-HLS origin path (active only on low-latency channels) ──
        // When the origin is live it owns the media playlist, parts, and complete
        // segments (served from memory). This must come before the non-LL stable-URL
        // redirect, which shares the `seg/` prefix.
        if crate::services::ll_origin::is_active() {
            // Origin-generated init segment (TS transmux path).
            if request_path == "init.mp4" {
                if let Some(bytes) = crate::services::ll_origin::get_init() {
                    crate::services::ll_diagnostics::event(&format!(
                        "\"ev\":\"o_init\",\"len\":{}",
                        bytes.len()
                    ));
                    return Ok(media_response(arc_bytes(bytes)));
                }
                return Ok(empty_cors(404));
            }
            if let Some(rest) = request_path.strip_prefix("part/") {
                if let Some((sn, k)) = parse_part_path(rest) {
                    if let Some(bytes) = crate::services::ll_origin::get_part(sn, k) {
                        let h = if crate::services::ll_diagnostics::is_active() {
                            crate::services::ll_diagnostics::quick_hash(bytes.as_ref())
                        } else {
                            0
                        };
                        crate::services::ll_diagnostics::event(&format!(
                            "\"ev\":\"o_part\",\"sn\":{sn},\"k\":{k},\"len\":{},\"h\":{h}",
                            bytes.len()
                        ));
                        return Ok(media_response(arc_bytes(bytes)));
                    }
                    crate::services::ll_diagnostics::event(&format!(
                        "\"ev\":\"o_part_miss\",\"sn\":{sn},\"k\":{k}"
                    ));
                }
                return Ok(empty_cors(404));
            }
            if let Some(rest) = request_path.strip_prefix("seg/") {
                if let Some(sn) = rest.strip_suffix(".ts").and_then(|s| s.parse::<u64>().ok()) {
                    if let Some(bytes) = crate::services::ll_origin::get_segment(sn) {
                        // Whole-segment fetch (the suspected A/V-skew trigger): record sn
                        // and size so it correlates with the frontend append burst.
                        crate::services::ll_diagnostics::event(&format!(
                            "\"ev\":\"o_seg\",\"sn\":{sn},\"len\":{}",
                            bytes.len()
                        ));
                        return Ok(media_response(bytes));
                    }
                    crate::services::ll_diagnostics::event(&format!(
                        "\"ev\":\"o_seg_miss\",\"sn\":{sn}"
                    ));
                }
                return Ok(empty_cors(404));
            }
            if request_path == "stream.m3u8" || request_path.is_empty() {
                let msn = parse_directive(&raw_query, "_HLS_msn");
                let part = parse_directive(&raw_query, "_HLS_part");
                // Serve-side timing: hls.js reports levelLoadTimeOut pairs before
                // stalls even though the blocking hold is bounded well under its
                // budget; this measures whether the server ever actually exceeds
                // it, separating origin-side delay from client-side accounting.
                let t0 = std::time::Instant::now();
                if let Some(pl) = crate::services::ll_origin::serve_playlist(msn, part).await {
                    if crate::services::ll_diagnostics::is_active() {
                        crate::services::ll_diagnostics::event(&format!(
                            "\"ev\":\"o_pl\",\"ms\":{},\"msn\":{},\"part\":{}",
                            t0.elapsed().as_millis(),
                            msn.map(|v| v as i64).unwrap_or(-1),
                            part.map(|v| v as i64).unwrap_or(-1)
                        ));
                    }
                    return Ok(playlist_response(pl.into_bytes()));
                }
                // Origin went inactive between the check and now: fall through.
            }
        }

        // Stable whole-segment projection: a synthetic `vseg/<sid>/<sn>.ts` is
        // 302-redirected to the freshest real CDN URL recorded when the playlist
        // was stabilized, so the player-visible path never changes even as Twitch
        // re-signs the real URL. Handled before any upstream fetch. The LL origin
        // owns `seg/` when active; this scheme uses a distinct `vseg/` prefix.
        if let Some((sid, sn)) = crate::services::hls_projection::parse_vseg_path(request_path) {
            return Ok(match crate::services::hls_projection::redirect_target(&sid, sn) {
                Some(url) => warp::http::Response::builder()
                    .status(302)
                    .header("Location", url)
                    .header("Access-Control-Allow-Origin", "*")
                    .header(
                        "Cache-Control",
                        "no-cache, no-store, must-revalidate, max-age=0",
                    )
                    .body(bytes::Bytes::new())
                    .unwrap(),
                None => empty_cors(404),
            });
        }

        // A segment we rewrote out of a playlist whose URLs were absolute. Its
        // real URL is already complete and signed, so it is fetched verbatim —
        // NOT joined against the manifest base and NOT given the manifest's query
        // (that convention is Twitch's, and appending it here would corrupt an
        // already-signed googlevideo URL).
        // Already fetched ahead of the request: hand it straight over. This is
        // the path that removes the startup stall.
        if let Some(index) = request_path
            .strip_prefix("pseg/")
            .and_then(|s| s.parse::<u32>().ok())
        {
            let hit = pseg_map()
                .lock()
                .ok()
                .and_then(|m| m.cache.get(&index).cloned());
            if let Some(bytes) = hit {
                return Ok(media_response(bytes));
            }
        }

        if let Some(target) = pseg_target(request_path) {
            return Ok(match HTTP_CLIENT.get(&target).send().await {
                Ok(res) if res.status().is_success() => match res.bytes().await {
                    // `media_response` sniffs TS vs fMP4 and carries the CORS
                    // headers — the reason this detour exists at all.
                    Ok(bytes) => {
                        // Keep it: hls.js re-requests a segment after a recovered
                        // error, and a second surface on the same stream would
                        // otherwise pay for the same download again. The clone is
                        // a refcount, not a copy.
                        if let Some(index) = request_path
                            .strip_prefix("pseg/")
                            .and_then(|s| s.parse::<u32>().ok())
                        {
                            if let Ok(mut m) = pseg_map().lock() {
                                m.cache.insert(index, bytes.clone());
                                m.trim_cache(index);
                            }
                        }
                        media_response(bytes)
                    }
                    Err(e) => {
                        error!("[StreamServer] segment body failed: {}", e);
                        empty_cors(502)
                    }
                },
                Ok(res) => {
                    // A 403 is the one to read carefully. Two very different
                    // causes: a signed URL that simply expired (harmless — the
                    // next playlist refresh replaces it), or YouTube's
                    // Proof-of-Origin enforcement, which lets a live stream play
                    // for roughly 30 seconds and THEN starts refusing. The
                    // sequence number distinguishes them: failures that begin
                    // partway into a stream, at a consistent offset, are the
                    // latter and are not fixable by retrying.
                    let sq = target
                        .split("/sq/")
                        .nth(1)
                        .and_then(|s| s.split('/').next())
                        .unwrap_or("?");
                    error!(
                        "[StreamServer] segment returned {} (sq={})",
                        res.status(),
                        sq
                    );
                    // 403/401 on a SIGNED url is the expiry case, which is
                    // recoverable by re-signing rather than terminal.
                    if matches!(res.status().as_u16(), 401 | 403) {
                        refresh_manifest_after_refusal(res.status().as_u16()).await;
                    }
                    empty_cors(res.status().as_u16())
                }
                Err(e) => {
                    error!("[StreamServer] segment request failed: {}", e);
                    empty_cors(502)
                }
            });
        }

        // Map the local path to the upstream CDN.
        let fetch_url = map_upstream_url(&manifest_url, &request_path);

        let mut response = match HTTP_CLIENT.get(&fetch_url).send().await {
            Ok(res) => res,
            Err(e) => {
                error!("[StreamServer] Upstream request failed: {}", e);
                return Ok(warp::http::Response::builder()
                    .status(502)
                    .header("Access-Control-Allow-Origin", "*")
                    .body(bytes::Bytes::new())
                    .unwrap());
            }
        };

        // An expiring upstream token starts refusing EVERY fetch once it lapses
        // (Kick's IVS master carries a JWT with an expiry). The segment path
        // already re-signed on a refusal, but this playlist path did not, so a
        // lapsed token surfaced to hls.js as a fatal levelLoadError and playback
        // stopped on a channel that was still perfectly live. Re-sign and retry
        // inline so the refusal never reaches the player.
        if matches!(response.status().as_u16(), 401 | 403) {
            if let Some(fresh) = resign_blocking(&manifest_url).await {
                let retry_url = map_upstream_url(&fresh, &request_path);
                match HTTP_CLIENT.get(&retry_url).send().await {
                    Ok(res) => {
                        info!(
                            "[StreamServer] re-signed after {}; retry returned {}",
                            response.status().as_u16(),
                            res.status().as_u16()
                        );
                        response = res;
                    }
                    Err(e) => error!("[StreamServer] re-signed retry failed: {}", e),
                }
            }
        }

        let status = response.status();
        let mut bytes = match response.bytes().await {
            Ok(b) => b.to_vec(),
            Err(e) => {
                error!("[StreamServer] Failed to read body bytes: {}", e);
                return Ok(warp::http::Response::builder()
                    .status(502)
                    .header("Access-Control-Allow-Origin", "*")
                    .body(bytes::Bytes::new())
                    .unwrap());
            }
        };

        // Playlist handling (never on .ts payloads). These bytes are already in hand,
        // and the live media playlist is re-fetched on every live-edge poll, so this is
        // free. Detection is read-only: the relay is ad-neutral and serves the upstream's
        // segments as they are. Ad markers are only RECORDED, for the UI counter and the
        // resolution-owning plugin (which escalates by swapping the upstream; core never
        // edits ads out). The one rewrite is lowering Twitch's over-declared
        // #EXT-X-TARGETDURATION (6s for ~2s segments) to the real segment size, so hls.js
        // rides at its cushion instead of being forced several seconds back. The
        // parts-based low-latency origin (when enabled) owns its own playlist and never
        // reaches here.
        if !request_path.ends_with(".ts") {
            if let Ok(text) = std::str::from_utf8(&bytes) {
                // Non-Twitch upstreams: retarget only. Ad detection would be
                // scanning for markers that platform never emits, and the
                // `vseg/` projection would rewrite segment URLs that are already
                // absolute, stable, and CORS-open (so the player fetches them
                // straight from the CDN and they never come back through here).
                let generic = upstream_profile() == UpstreamProfile::GenericHls;
                if generic {
                    let work = crate::services::ad_detect::retarget_playlist(text)
                        .unwrap_or_else(|| text.to_string());
                    // Absolute segment URLs would be fetched straight from the
                    // CDN by the player. That is fine when the CDN is CORS-open
                    // (Kick's IVS is) and fatal when it is not (googlevideo sends
                    // no CORS header at all), so route them back through here.
                    // Same bytes, same network path — they simply arrive from an
                    // origin whose headers we control.
                    bytes = rewrite_absolute_segments(&work).into_bytes();
                    return finish_playlist_response(status, request_path, bytes);
                }
                detect_ads_in_playlist(text);
                #[cfg(target_os = "android")]
                let filtered = crate::services::ad_bypass::filter_and_escalate(text);
                #[cfg(target_os = "android")]
                let text = filtered.as_str();
                // Lower the over-declared TARGETDURATION, then (LIVE only) pin every
                // segment URL stable across refreshes so Twitch re-signing a path
                // can't trip hls.js into replaying a segment. Gated two ways: a
                // VOD/ended playlist (#EXT-X-ENDLIST) is static and fully seekable so
                // rewriting+pruning it would break seeking; and the experimental
                // low-latency engine, when ON, owns the playlist with its own `seg/`
                // scheme (served from memory) — our `vseg/` rewrite alongside it
                // would hand the player two URLs for one media sequence and trip the
                // exact refresh-mismatch we fix, so we only stabilize when the engine
                // is off (the whole-segment path).
                // `hls_kind` tells a recording VOD (`PLAYLIST-TYPE:EVENT`, no
                // ENDLIST, only grows) apart from a live window: it is a VOD
                // for the relay's purposes, and stabilizing it broke seeking
                // past the projection's retention window (GitHub #216).
                let is_live = crate::services::hls_kind::classify(text)
                    == crate::services::hls_kind::PlaylistKind::Live;
                let stabilize_ok = is_live && crate::services::ll_origin::engine_disabled();
                let work: String = crate::services::ad_detect::retarget_playlist(text)
                    .unwrap_or_else(|| text.to_string());
                bytes = if stabilize_ok {
                    let base = crate::services::hls_projection::base_url_of(&manifest_url);
                    crate::services::hls_projection::stabilize(SOLO_STREAM_ID, &work, &base)
                        .into_bytes()
                } else {
                    work.into_bytes()
                };
            }
        }

        // Determine content-type (chunks are video/MP2T, playlists are x-mpegURL)
        let content_type = if request_path.ends_with(".ts") {
            "video/MP2T"
        } else {
            "application/x-mpegURL"
        };

        Ok(warp::http::Response::builder()
            .status(status)
            .header("Content-Type", content_type)
            .header("Access-Control-Allow-Origin", "*")
            .header("Access-Control-Allow-Methods", "GET, OPTIONS")
            .header("Access-Control-Allow-Headers", "*")
            .header(
                "Cache-Control",
                "no-cache, no-store, must-revalidate, max-age=0",
            )
            .header("Pragma", "no-cache")
            .header("Expires", "0")
            .body(bytes.into()) // Return perfectly preserved source bytes!
            .unwrap())
    }

    async fn proxy_handler() -> Result<warp::http::Response<bytes::Bytes>, warp::Rejection> {
        let url = PROXY_URL
            .lock()
            .await
            .clone()
            .ok_or_else(|| warp::reject::not_found())?;

        let response = match HTTP_CLIENT.get(&url).send().await {
            Ok(res) => res,
            Err(e) => {
                error!("[StreamServer] Upstream request failed: {}", e);
                return Ok(warp::http::Response::builder()
                    .status(502)
                    .header("Access-Control-Allow-Origin", "*")
                    .body(bytes::Bytes::new())
                    .unwrap());
            }
        };

        let status = response.status();
        let bytes = match response.bytes().await {
            Ok(b) => b.to_vec(),
            Err(e) => {
                error!("[StreamServer] Failed to read body bytes: {}", e);
                return Ok(warp::http::Response::builder()
                    .status(502)
                    .header("Access-Control-Allow-Origin", "*")
                    .body(bytes::Bytes::new())
                    .unwrap());
            }
        };

        let mut rewritten_bytes = bytes.clone();

        // Rewrite relative URLs to absolute URLs for VOD/Clip M3U8 manifests
        if let Ok(m3u8_str) = String::from_utf8(bytes) {
            if m3u8_str.starts_with("#EXTM3U") {
                let base_url = if let Some(last_slash) = url.rfind('/') {
                    &url[..=last_slash]
                } else {
                    &url
                };

                let mut new_m3u8 = String::with_capacity(m3u8_str.len() + 1024);
                for line in m3u8_str.lines() {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        new_m3u8.push('\n');
                        continue;
                    }
                    if trimmed.starts_with('#')
                        || trimmed.starts_with("http://")
                        || trimmed.starts_with("https://")
                    {
                        new_m3u8.push_str(line);
                        new_m3u8.push('\n');
                    } else {
                        new_m3u8.push_str(base_url);
                        new_m3u8.push_str(trimmed);
                        new_m3u8.push('\n');
                    }
                }
                rewritten_bytes = new_m3u8.into_bytes();
            }
        }

        Ok(warp::http::Response::builder()
            .status(status)
            .header("Content-Type", "application/x-mpegURL")
            .header("Access-Control-Allow-Origin", "*")
            .header("Access-Control-Allow-Methods", "GET, OPTIONS")
            .header(
                "Cache-Control",
                "no-cache, no-store, must-revalidate, max-age=0",
            )
            .header("Pragma", "no-cache")
            .header("Expires", "0")
            .body(rewritten_bytes.into())
            .unwrap())
    }

    pub async fn stop() -> Result<()> {
        if let Some(handle) = SERVER_HANDLE.lock().await.take() {
            handle.abort();
        }
        *PROXY_URL.lock().await = None;
        *CURRENT_PORT.lock().await = None;
        reset_ad_state();
        set_solo_session(None);
        Ok(())
    }

    async fn get_current_port() -> Result<u16> {
        CURRENT_PORT
            .lock()
            .await
            .ok_or_else(|| anyhow::anyhow!("No server running"))
    }
}
