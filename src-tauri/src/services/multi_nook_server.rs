use crate::services::ad_detect;
use crate::services::ll_origin::{
    self, arc_bytes, empty_cors, media_response, opt_raw_query, parse_directive, parse_part_path,
    playlist_response, LlOrigin,
};
use anyhow::Result;
use log::{debug, info, warn};
use once_cell::sync::Lazy;
use rand::Rng;
use reqwest::Client;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Mutex;
use warp::Filter;

/// What kind of upstream a tile is serving. Twitch gets SSAI ad detection,
/// segment projection and the LL-HLS origin; every other platform gets the
/// plain generic path. Mirrors the solo relay's `UpstreamProfile`, which is
/// process-global and therefore unusable here: tiles need this per instance,
/// since one grid can hold a Twitch tile and a Kick tile at once.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TileProfile {
    Twitch,
    GenericHls,
}

/// Re-signs this tile's manifest url. Same shape as the solo relay's
/// `Refresher`, but held PER TILE: the solo one is a process-global static, so a
/// grid with two Kick tiles would have them overwrite each other's re-signer.
pub type TileRefresher =
    Arc<dyn Fn() -> futures::future::BoxFuture<'static, Option<String>> + Send + Sync>;

/// Represents a single stream proxy instance
struct StreamInstance {
    handle: tokio::task::JoinHandle<()>,
    port: u16,
    proxy_url: Arc<Mutex<Option<String>>>,
    /// Per-tile ad-detection state (shared marker logic via `ad_detect`).
    ad_state: Arc<std::sync::Mutex<ad_detect::AdDetectionState>>,
    /// Per-tile LL-HLS origin, same engine as the solo relay. Active only when the
    /// tile's channel is a low-latency broadcast; inactive tiles use the plain
    /// playlist proxy below.
    ll_origin: Arc<LlOrigin>,
    /// Which upstream treatment this tile gets. See `TileProfile`.
    profile: TileProfile,
    /// Re-signs the upstream url when it starts being refused. Kick's master url
    /// carries a JWT that expires mid-session; without this a tile plays until
    /// expiry and then stops for good.
    refresher: Option<TileRefresher>,
    /// One re-sign at a time per tile. Everyone who queues behind the winner
    /// gets the winner's url rather than stampeding the platform.
    resign_lock: Arc<Mutex<()>>,
}

pub struct MultiNookServer;

/// Registry of all active stream proxy instances, keyed by stream_id
static STREAM_REGISTRY: Lazy<Arc<Mutex<HashMap<String, StreamInstance>>>> =
    Lazy::new(|| Arc::new(Mutex::new(HashMap::new())));

/// Global HTTP client with optimized connection pooling
static HTTP_CLIENT: Lazy<Client> = Lazy::new(|| {
    Client::builder()
        .tcp_keepalive(std::time::Duration::from_secs(15))
        .pool_idle_timeout(std::time::Duration::from_secs(30))
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .expect("Failed to build global HTTP client")
});

impl MultiNookServer {
    /// Start a new proxy server for a specific stream, or update the URL if one already exists
    pub async fn start_proxy(
        stream_id: &str,
        stream_url: String,
        profile: TileProfile,
        refresher: Option<TileRefresher>,
    ) -> Result<u16> {
        // Get-or-create under the registry lock, but probe the LL origin AFTER
        // releasing it: the probe fetches the upstream playlist plus backfill
        // segments, and holding the registry across that would serialize every
        // tile's startup when a preset cold-starts a whole grid.
        let (port, origin) = {
            let mut registry = STREAM_REGISTRY.lock().await;

            if let Some(instance) = registry.get_mut(stream_id) {
                debug!(
                    "[MultiNook] Updating proxy URL for stream '{}' on port {}",
                    stream_id, instance.port
                );
                *instance.proxy_url.lock().await = Some(stream_url.clone());
                // New stream on this tile: clear stale ad-detection state.
                *instance.ad_state.lock().unwrap() = ad_detect::AdDetectionState::default();
                // A tile can be retargeted at a different platform (preset load,
                // swap), so the profile follows the new upstream.
                instance.profile = profile;
                instance.refresher = refresher;
                (instance.port, instance.ll_origin.clone())
            } else {
                // Start a new server on a random port
                let port = rand::rng().random_range(10000..20000);
                let proxy_url: Arc<Mutex<Option<String>>> =
                    Arc::new(Mutex::new(Some(stream_url.clone())));
                let ad_state: Arc<std::sync::Mutex<ad_detect::AdDetectionState>> =
                    Arc::new(std::sync::Mutex::new(ad_detect::AdDetectionState::default()));
                let origin = LlOrigin::new(ll_origin::TILE_MAX_SEGMENTS);

                let addr = SocketAddr::from(([127, 0, 0, 1], port));
                let proxy_url_clone = proxy_url.clone();
                let ad_state_clone = ad_state.clone();
                let origin_clone = origin.clone();
                let sid = stream_id.to_string();
                let profile_for_filter = profile;

                // Wildcard route like the solo relay: when the LL origin is active the
                // playlist references relative `part/...` and `seg/...` URIs that must
                // resolve against this same port.
                let proxy = warp::path::full()
                    .and(warp::method())
                    .and(opt_raw_query())
                    .and(warp::any().map(move || proxy_url_clone.clone()))
                    .and(warp::any().map(move || ad_state_clone.clone()))
                    .and(warp::any().map(move || sid.clone()))
                    .and(warp::any().map(move || origin_clone.clone()))
                    .and(warp::any().map(move || profile_for_filter))
                    .and_then(Self::proxy_handler)
                    .boxed();

                let handle = tokio::spawn(async move {
                    warp::serve(proxy).run(addr).await;
                });

                debug!(
                    "[MultiNook] Started proxy for stream '{}' on port {}",
                    stream_id, port
                );

                registry.insert(
                    stream_id.to_string(),
                    StreamInstance {
                        handle,
                        port,
                        proxy_url,
                        ad_state,
                        ll_origin: origin.clone(),
                        profile,
                        refresher,
                        resign_lock: Arc::new(Mutex::new(())),
                    },
                );
                (port, origin)
            }
        };

        // Probe the upstream: on a low-latency broadcast this builds the live edge and
        // spawns the per-tile reader BEFORE we return, so `is_low_latency` reads a
        // settled answer when `start_multi_nook` tags the proxy URL. On a
        // normal-latency channel the origin stays inactive and the plain playlist
        // proxy serves the tile.
        // Twitch only: the LL-HLS origin speaks Twitch's low-latency playlist
        // shape. Probing a Kick playlist with it wastes a fetch at best and
        // mis-serves the tile at worst.
        if profile == TileProfile::Twitch {
            origin.start(stream_url).await;
        }

        Ok(port)
    }

    /// Re-sign one tile's upstream url after it started being refused, returning
    /// the fresh url. Mirrors the solo relay's blocking re-sign: one at a time,
    /// and a caller that queued behind the winner gets the winner's url instead
    /// of asking the platform again.
    ///
    /// The registry lock is never held across the refresher await, or one tile
    /// re-signing would stall every other tile's requests.
    async fn resign_tile(stream_id: &str, stale: &str) -> Option<String> {
        let (refresher, resign_lock, proxy_url) = {
            let registry = STREAM_REGISTRY.lock().await;
            let inst = registry.get(stream_id)?;
            (
                inst.refresher.clone()?,
                inst.resign_lock.clone(),
                inst.proxy_url.clone(),
            )
        };

        let _guard = resign_lock.lock().await;

        // Someone else re-signed while we queued: take theirs.
        if let Some(current) = proxy_url.lock().await.clone() {
            if current != stale {
                return Some(current);
            }
        }

        let fresh = refresher().await?;
        *proxy_url.lock().await = Some(fresh.clone());
        info!(
            "[MultiNook] '{}' manifest url re-signed after a refused request",
            stream_id
        );
        Some(fresh)
    }

    /// Stop a specific stream's proxy server
    pub async fn stop_instance(stream_id: &str) -> Result<()> {
        let mut registry = STREAM_REGISTRY.lock().await;

        if let Some(instance) = registry.remove(stream_id) {
            debug!(
                "[MultiNook] Stopping proxy for stream '{}' on port {}",
                stream_id, instance.port
            );
            instance.ll_origin.stop();
            instance.handle.abort();
            *instance.proxy_url.lock().await = None;
            crate::services::hls_projection::reset(stream_id);
        } else {
            debug!(
                "[MultiNook] No proxy found for stream '{}', nothing to stop",
                stream_id
            );
        }

        Ok(())
    }

    /// Stop all stream proxy servers (cleanup on exit or mode switch)
    pub async fn stop_all() -> Result<()> {
        let mut registry = STREAM_REGISTRY.lock().await;
        let count = registry.len();

        for (id, instance) in registry.drain() {
            debug!(
                "[MultiNook] Stopping proxy for stream '{}' on port {}",
                id, instance.port
            );
            instance.ll_origin.stop();
            instance.handle.abort();
            *instance.proxy_url.lock().await = None;
            crate::services::hls_projection::reset(&id);
        }

        debug!("[MultiNook] Stopped {} proxy servers", count);
        Ok(())
    }

    /// Get the port for a specific stream
    pub async fn get_port(stream_id: &str) -> Option<u16> {
        let registry = STREAM_REGISTRY.lock().await;
        registry.get(stream_id).map(|i| i.port)
    }

    /// Whether the tile's LL-HLS origin is active (low-latency broadcast). Read by
    /// `start_multi_nook` after `start_proxy` settles the probe, to tag the proxy
    /// URL the player picks its hls.js mode from.
    pub async fn is_low_latency(stream_id: &str) -> bool {
        let registry = STREAM_REGISTRY.lock().await;
        registry
            .get(stream_id)
            .map(|i| i.ll_origin.is_active())
            .unwrap_or(false)
    }

    /// Get a list of all active stream IDs
    pub async fn get_active_streams() -> Vec<String> {
        let registry = STREAM_REGISTRY.lock().await;
        registry.keys().cloned().collect()
    }

    /// Get the count of active stream proxies
    pub async fn active_count() -> usize {
        let registry = STREAM_REGISTRY.lock().await;
        registry.len()
    }

    /// Replace a tile's upstream playlist with one a resolution-owning plugin
    /// supplied via `set_upstream`. Reuses the tile-restart path: the relay
    /// keeps its port, resets its ad state, and re-probes the new upstream.
    pub async fn swap_upstream(stream_id: &str, playlist_url: String) -> Result<()> {
        let exists = STREAM_REGISTRY.lock().await.contains_key(stream_id);
        if !exists {
            return Err(anyhow::anyhow!("no relay session for tile '{stream_id}'"));
        }
        // A pivot points the tile at a new region with its own segment numbering;
        // drop the old region's projection map so no synthetic URL survives it.
        crate::services::hls_projection::reset(stream_id);
        let (profile, refresher) = {
            let registry = STREAM_REGISTRY.lock().await;
            registry
                .get(stream_id)
                .map(|i| (i.profile, i.refresher.clone()))
                .unwrap_or((TileProfile::Twitch, None))
        };
        Self::start_proxy(stream_id, playlist_url, profile, refresher).await?;
        info!(
            "[MultiNook] '{}' upstream swapped by a playback plugin",
            stream_id
        );
        Ok(())
    }

    /// Snapshot every tile's ad-detection state, keyed by stream_id (for a
    /// command / the future auto-pivot). Parity with the solo `ad_state()`.
    pub async fn ad_snapshot() -> HashMap<String, ad_detect::AdDetectionState> {
        let registry = STREAM_REGISTRY.lock().await;
        registry
            .iter()
            .map(|(id, inst)| (id.clone(), inst.ad_state.lock().unwrap().clone()))
            .collect()
    }

    async fn proxy_handler(
        path: warp::path::FullPath,
        method: warp::http::Method,
        raw_query: String,
        proxy_url: Arc<Mutex<Option<String>>>,
        ad_state: Arc<std::sync::Mutex<ad_detect::AdDetectionState>>,
        stream_id: String,
        origin: Arc<LlOrigin>,
        profile: TileProfile,
    ) -> Result<warp::http::Response<bytes::Bytes>, warp::Rejection> {
        // Handle CORS preflight instantly without touching upstream.
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

        // ── LL-HLS origin path: identical routing to the solo relay ──
        // When the tile's origin is live it owns the media playlist, parts, and
        // complete segments (served from memory); the upstream proxy below only
        // handles the non-LL case.
        if origin.is_active() {
            // Origin-generated init segment (TS transmux path).
            if request_path == "init.mp4" {
                if let Some(bytes) = origin.get_init() {
                    return Ok(media_response(arc_bytes(bytes)));
                }
                return Ok(empty_cors(404));
            }
            if let Some(rest) = request_path.strip_prefix("part/") {
                if let Some((sn, k)) = parse_part_path(rest) {
                    if let Some(bytes) = origin.get_part(sn, k) {
                        return Ok(media_response(arc_bytes(bytes)));
                    }
                }
                return Ok(empty_cors(404));
            }
            if let Some(rest) = request_path.strip_prefix("seg/") {
                if let Some(sn) = rest.strip_suffix(".ts").and_then(|s| s.parse::<u64>().ok()) {
                    if let Some(bytes) = origin.get_segment(sn) {
                        return Ok(media_response(bytes));
                    }
                }
                return Ok(empty_cors(404));
            }
            if request_path == "stream.m3u8" || request_path.is_empty() {
                let msn = parse_directive(&raw_query, "_HLS_msn");
                let part = parse_directive(&raw_query, "_HLS_part");
                if let Some(pl) = origin.serve_playlist(msn, part).await {
                    return Ok(playlist_response(pl.into_bytes()));
                }
                // Origin went inactive between the check and now: fall through.
            }
        }

        // Stable whole-segment projection: a synthetic `vseg/<tile>/<sn>.ts`
        // 302-redirects to the freshest real CDN URL (see stream_server). A
        // distinct `vseg/` prefix, so it never collides with the LL origin's
        // `seg/`; the per-tile session id keeps two tiles' equal sequence numbers
        // from resolving to each other.
        if let Some((sid, sn)) = crate::services::hls_projection::parse_vseg_path(request_path) {
            return Ok(
                match crate::services::hls_projection::redirect_target(&sid, sn) {
                    Some(u) => warp::http::Response::builder()
                        .status(302)
                        .header("Location", u)
                        .header("Access-Control-Allow-Origin", "*")
                        .header(
                            "Cache-Control",
                            "no-cache, no-store, must-revalidate, max-age=0",
                        )
                        .body(bytes::Bytes::new())
                        .unwrap(),
                    None => empty_cors(404),
                },
            );
        }

        // Tiles only relay the media playlist on the non-LL path; stabilized
        // segment URLs in it 302 back here via the `vseg/` route above.
        if request_path != "stream.m3u8" && !request_path.is_empty() {
            return Ok(empty_cors(404));
        }

        // A keep-alive connection can deliver one last poll after the tile relay
        // stops (the abort only kills the accept loop). A warp rejection would be
        // a bare 404 without CORS headers, which the webview logs as a CORS error;
        // answer with a CORS'd 404 instead.
        let Some(url) = proxy_url.lock().await.clone() else {
            return Ok(empty_cors(404));
        };

        // At most one re-sign per request. A refused playlist is the signal that
        // the upstream url expired (Kick's master carries a JWT with a lifetime
        // shorter than a viewing session); without this the tile would serve the
        // refusal body and stop for good with no way back.
        let mut url = url;
        let mut resigned = false;
        let (status, mut bytes) = loop {
            let response = match HTTP_CLIENT.get(&url).send().await {
                Ok(res) => res,
                Err(e) => {
                    debug!("[MultiNook] Upstream request failed: {}", e);
                    return Ok(warp::http::Response::builder()
                        .status(502)
                        .header("Access-Control-Allow-Origin", "*")
                        .body(bytes::Bytes::new())
                        .unwrap());
                }
            };

            let status = response.status();
            let body = match response.bytes().await {
                Ok(b) => b.to_vec(),
                Err(e) => {
                    debug!("[MultiNook] Failed to read body bytes: {}", e);
                    return Ok(warp::http::Response::builder()
                        .status(502)
                        .header("Access-Control-Allow-Origin", "*")
                        .body(bytes::Bytes::new())
                        .unwrap());
                }
            };

            if status.is_success() || resigned {
                break (status, body);
            }
            warn!(
                "[MultiNook] '{}' upstream returned {}; attempting to re-sign",
                stream_id, status
            );
            match Self::resign_tile(&stream_id, &url).await {
                Some(fresh) => {
                    url = fresh;
                    resigned = true;
                }
                // No refresher (Twitch tiles), or re-signing failed: serve what
                // the upstream said rather than inventing a different failure.
                None => break (status, body),
            }
        };

        // Detect ad markers for the per-tile state — same shared logic as the solo
        // player, read-only: the tile relay is ad-neutral and serves the upstream's
        // segments untouched. For playback only; the core never reports ads to a plugin.
        // This server only relays the media playlist (not .ts), so every body is a
        // playlist worth scanning; free. The one rewrite is lowering Twitch's
        // over-declared #EXT-X-TARGETDURATION (6s for ~2s segments) to the real segment
        // size: hls.js derives its live-playlist RELOAD cadence from targetduration, so
        // the inflated 6 makes it re-poll too slowly to keep a small per-tile buffer fed
        // and the tile stalls shortly after starting. Tile low latency, when enabled, is
        // served by the per-tile parts origin above and never reaches here.
        if let Ok(text) = std::str::from_utf8(&bytes) {
            // SSAI marker detection is Twitch-specific; other platforms do not
            // stitch ads into the media playlist, so running it there only
            // produces phantom state.
            if profile == TileProfile::Twitch {
                let mut st = ad_state.lock().unwrap();
                if let Some(n) = ad_detect::update(&mut st, text) {
                    info!(
                        "[MultiNook] ad markers detected on '{}' (break #{}): {:?}",
                        stream_id, n, st.matched_markers
                    );
                }
            }
            // Retarget, then pin segment URLs stable across refreshes. Gated like the
            // solo path: only on a live playlist (not VOD/#EXT-X-ENDLIST) and only when
            // the experimental low-latency engine is off, so our `vseg/` rewrite never
            // races the per-tile origin's `seg/` scheme for the same media sequence.
            let is_live = crate::services::hls_kind::classify(text)
                == crate::services::hls_kind::PlaylistKind::Live;
            // Segment projection is part of the Twitch profile too (it exists to
            // survive Twitch's ad-stitched sequence rewrites).
            let stabilize_ok = profile == TileProfile::Twitch
                && is_live
                && crate::services::ll_origin::engine_disabled();
            let work: String =
                ad_detect::retarget_playlist(text).unwrap_or_else(|| text.to_string());
            bytes = if stabilize_ok {
                let base = crate::services::hls_projection::base_url_of(&url);
                crate::services::hls_projection::stabilize(&stream_id, &work, &base).into_bytes()
            } else {
                work.into_bytes()
            };
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
            .body(bytes.into())
            .unwrap())
    }
}
