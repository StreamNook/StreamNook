use crate::models::settings::AppState;
use crate::services::auth_proxy;
use crate::services::providers::hls_master;
use crate::services::providers::source::PlaybackKind;
use crate::services::providers::watch_urls::{self, WatchTarget};
use crate::services::stream_server::StreamServer;
use crate::services::twitch_resolver as tr;
use log::debug;
use serde::Serialize;
use serde_json::json;
use tauri::State;

/// The hook a resolution-owning plugin fills (see docs/plugins/HOOKS.md): the
/// host invokes this action with the channel and quality, and the plugin
/// answers with a master playlist for the relay to serve.
pub(crate) const PLAYBACK_RESOLVE_HOOK: &str = "playback.resolve";

/// Hand a non-entitled live resolution to an installed playback plugin, when
/// one provides the `playback.resolve` hook.
///
/// `core` is the core resolver's own result: entitled resolutions are never
/// delegated (Turbo or a channel sub is already ad-free), and a successful
/// core master rides along in the action args so the plugin can graft the
/// above-1080p tiers the viewer's login unlocks onto whatever master it
/// resolves. Returns `None` whenever the plugin path does not produce a
/// playable result, so the caller falls back to the core resolution.
pub(crate) async fn resolve_via_plugin(
    state: &State<'_, AppState>,
    stream_id: &str,
    channel: &str,
    quality: &str,
    core: &Result<tr::ResolvedLive, anyhow::Error>,
) -> Option<tr::ResolvedLive> {
    if core.as_ref().map(|r| r.status.entitled).unwrap_or(false) {
        return None;
    }
    state.plugin_host.provides(PLAYBACK_RESOLVE_HOOK).await?;
    let auth_master = core.as_ref().ok().map(|r| r.master.clone());
    let args = json!({
        "stream_id": stream_id,
        "channel": channel,
        "quality": quality,
        "auth_master": auth_master,
    });
    let answer = match state
        .plugin_host
        .invoke_action(PLAYBACK_RESOLVE_HOOK, args)
        .await
    {
        Ok(v) => v,
        Err(e) => {
            debug!("[Streaming] {} plugin resolve failed: {}", channel, e);
            return None;
        }
    };
    if answer
        .get("declined")
        .and_then(|d| d.as_bool())
        .unwrap_or(false)
    {
        return None;
    }
    let master = answer.get("master")?.as_str()?.to_string();
    let base = answer
        .get("base")
        .and_then(|b| b.as_str())
        .map(String::from);
    let region = answer
        .get("region")
        .and_then(|r| r.as_str())
        .map(String::from);
    match tr::resolve_from_master(channel, master, quality, base, region) {
        Ok(r) => {
            debug!(
                "[Streaming] {} resolved by a playback plugin (region={:?})",
                channel, r.status.proxy_region
            );
            Some(r)
        }
        Err(e) => {
            debug!(
                "[Streaming] {} plugin master unusable ({}); using the core resolution",
                channel, e
            );
            None
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct StreamStartResult {
    /// Local proxy URL (or direct MP4 for clips) the player should load.
    pub url: String,
    /// The literal quality the resolver served. May differ from the requested
    /// quality if the requested one wasn't offered for this stream (closest-match
    /// fallback). The frontend compares this against the user's saved preference
    /// to decide whether to notify.
    pub quality: String,
    /// How the resolver served this live stream:
    /// "turbo" | "subscribed" | "proxy" | "auth-only". None for VOD/clips.
    /// Drives the UI's ad-source badge.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// True when playing Twitch's own ad-free entitlement, with no proxy in use
    /// (i.e. the viewer's Turbo or channel subscription is doing the work).
    pub entitled: bool,
    /// Proxy region label (e.g. "EU") when the ad-block proxy path was used.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proxy_region: Option<String>,
    /// Clips only: where this clip sits inside its source broadcast, so the chat
    /// panel can replay the chat from that moment. Playback never uses it. Absent
    /// for live/VOD, and for a clip whose parent VOD has expired.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clip_source: Option<tr::ClipSource>,
    /// The quality menu the resolver discovered for this stream (variant names
    /// plus best/worst). The player builds its quality selector from this, so it
    /// always matches what was actually resolved — no separate probe needed.
    #[serde(default)]
    pub available: Vec<String>,
    /// How the player should ingest `url`: absent (the default) and "hls" both
    /// mean an HLS playlist, "flv" means an FLV stream needing the mpegts path.
    /// Absent for every Twitch path, so existing consumers are unaffected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// VOD starts only: status, length, timeline anchor, where to begin, and
    /// whether this VOD is standing in for a live broadcast the viewer
    /// rewound. Absent for live and clips.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vod: Option<VodStartInfo>,
}

/// What the player needs to run a VOD correctly (a recording VOD is a growing
/// EVENT playlist that must not be treated as live) and where to start it.
#[derive(Debug, Clone, Serialize)]
pub struct VodStartInfo {
    pub video_id: String,
    /// "recording" while the broadcast is live, "recorded" when finished,
    /// "unknown" when the metadata lookup failed (the player then sniffs the
    /// playlist itself).
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub length_seconds: Option<u32>,
    /// Broadcast time at VOD position 0 (ISO 8601).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recorded_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_login: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail_url: Option<String>,
    /// Where playback should begin: the stored resume position, or the mapped
    /// broadcast position for a live rewind. None = from the top.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_position_secs: Option<f64>,
    /// True when the viewer rewound a live broadcast into this recording; the
    /// live session stays up and the player offers "Back to live".
    pub rewound_from_live: bool,
    /// Audio ranges Twitch muted, for the seekbar marks. Empty when none are
    /// known; on a `recording` VOD that means not yet determined, not "none".
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub muted_segments: Vec<crate::services::muted_segments::MutedRange>,
}

/// What we actually KNOW about a channel's liveness.
///
/// Three-state on purpose. A refused request and a genuine ending are
/// indistinguishable from a failed call alone, and absence from the batch
/// response is not proof of an ending either: the row is missing both when the
/// channel does not exist and when the request was refused. Only a row that says
/// so explicitly counts as an answer.
#[derive(PartialEq, Clone, Copy, Debug)]
pub(crate) enum Liveness {
    Live,
    Offline,
    Unknown,
}

/// Read a batch response as a verdict about ONE channel.
///
/// Split from the request so the rule that matters is testable without a network:
/// a row present and explicit is an answer, anything else is not.
fn verdict_for(rows: &[crate::models::provider_stream::ProviderStream], channel: &str) -> Liveness {
    match rows
        .iter()
        .find(|r| r.user_login.eq_ignore_ascii_case(channel))
    {
        Some(row) => {
            if row.is_live {
                Liveness::Live
            } else {
                Liveness::Offline
            }
        }
        None => Liveness::Unknown,
    }
}

/// One liveness probe against the batched channels endpoint.
async fn probe_kick_liveness(channel: &str) -> Liveness {
    use crate::services::providers::source::StreamSource;
    let one = [channel.to_string()];
    match crate::services::providers::kick_media::KickSource::new()
        .live_check(&one)
        .await
    {
        Ok(rows) => verdict_for(&rows, channel),
        Err(_) => Liveness::Unknown,
    }
}

/// Probe until the answer is definitive, instead of concluding from a single
/// inconclusive result.
///
/// Ending someone's stream is not a call to make on a maybe, and neither is
/// leaving a genuinely-ended stream frozen on its last frame. Retries are cheap
/// next to being wrong in either direction. Still Unknown after all of them hands
/// the decision to the liveness poll rather than inventing one.
pub(crate) async fn confirm_kick_liveness(channel: &str) -> Liveness {
    const ATTEMPTS: u32 = 3;
    for attempt in 0..ATTEMPTS {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(400 * u64::from(attempt))).await;
        }
        match probe_kick_liveness(channel).await {
            Liveness::Unknown => continue,
            verdict => return verdict,
        }
    }
    log::warn!(
        "[Kick] could not determine whether '{}' is live after {} probes; deferring to the liveness poll",
        channel,
        ATTEMPTS
    );
    Liveness::Unknown
}


/// Resolve and serve a non-Twitch live stream.
///
/// The platform adapter hands back a media-playlist (or direct stream) URL; from
/// there the existing relay does the work, in its generic profile: Twitch's SSAI
/// ad detection, segment projection and LL origin are all skipped, while the
/// TARGETDURATION retarget stays on because it is platform-agnostic and these
/// platforms over-declare it exactly the way Twitch does.
async fn start_provider_stream(
    provider: &'static str,
    channel: &str,
    quality: &str,
) -> Result<StreamStartResult, String> {
    let source = crate::services::providers::registry()
        .await
        .get_source(provider)
        .ok_or_else(|| format!("{} streams aren't supported in this build yet", provider))?;

    let resolved = source
        .resolve_playback(crate::services::stream_server::SOLO_STREAM_ID, channel, quality)
        .await
        .map_err(|e| e.to_string())?;

    log::info!(
        "[Streaming] {}:{} '{}' → '{}' ({:?}) variants={}",
        provider,
        channel,
        quality,
        resolved.quality,
        resolved.kind,
        resolved.qualities.len()
    );

    // Deliberately no `auth_proxy::set_status` and no solo-session registration:
    // both address the Twitch-contractual plugin resolution protocol
    // (auth_master, ad windows, entitlement badge). A provider stream reports
    // `mode: None`, so the UI shows no ad-source badge, which is the truth.
    match resolved.kind {
        PlaybackKind::Hls => {
            crate::services::stream_server::set_upstream_profile(
                crate::services::stream_server::UpstreamProfile::GenericHls,
            );
            // Teach the relay how to re-sign this stream. Kick's master url
            // carries a JWT that expires mid-session; without this the first
            // refusal after expiry ends playback for good.
            if provider == "kick" {
                let ch = channel.to_string();
                let q = quality.to_string();
                crate::services::stream_server::set_manifest_refresher(Some(std::sync::Arc::new(
                    move || {
                        let ch = ch.clone();
                        let q = q.clone();
                        Box::pin(async move {
                            match crate::services::providers::kick_media::KickSource::new()
                                .resign(&ch, &q)
                                .await
                            {
                                Ok(url) => Some(url),
                                Err(e) => {
                                    let msg = e.to_string();
                                    if msg.contains("not live") {
                                        // "not live" here is a CLAIM, not a fact.
                                        // `resign` forces a fresh resolve, which is
                                        // the request most likely to be refused by
                                        // Kick's bot defense, and a refused payload
                                        // has no `stream` object, so it parses
                                        // identically to a stream that genuinely
                                        // ended. Verify against the batched channels
                                        // endpoint (a different request path) and
                                        // retry until the answer is definitive.
                                        match confirm_kick_liveness(&ch).await {
                                            Liveness::Offline => {
                                                log::info!(
                                                    "[Kick] '{}' has ended (verified); signalling offline",
                                                    ch
                                                );
                                                if let Some(app) =
                                                    crate::services::providers::app_handle()
                                                {
                                                    use tauri::Emitter;
                                                    let _ = app.emit(
                                                        "provider-stream-offline",
                                                        serde_json::json!({
                                                            "provider": "kick",
                                                            "channel": ch,
                                                        }),
                                                    );
                                                }
                                                None
                                            }
                                            Liveness::Live => {
                                                // Verified still live, so the refusal
                                                // was transient. Don't just decline to
                                                // eject and leave playback broken on a
                                                // 403 loop: re-sign again now, which is
                                                // the whole point of knowing it is up.
                                                log::info!(
                                                    "[Kick] '{}' is still live; retrying the re-sign",
                                                    ch
                                                );
                                                crate::services::providers::kick_media::KickSource::new()
                                                    .resign(&ch, &q)
                                                    .await
                                                    .map_err(|e| {
                                                        log::warn!(
                                                            "[Kick] re-sign retry for '{}' failed: {}",
                                                            ch,
                                                            e
                                                        );
                                                    })
                                                    .ok()
                                            }
                                            // Unknown already logged why. Changing
                                            // nothing is the honest move: the liveness
                                            // poll is the backstop and it does not
                                            // depend on this call succeeding.
                                            Liveness::Unknown => None,
                                        }
                                    } else {
                                        log::warn!("[Kick] could not re-sign '{}': {}", ch, msg);
                                        None
                                    }
                                }
                            }
                        })
                    },
                )))
                .await;
            } else {
                crate::services::stream_server::set_manifest_refresher(None).await;
            }
            let port = StreamServer::start_proxy_server(resolved.url)
                .await
                .map_err(|e| e.to_string())?;
            Ok(StreamStartResult {
                url: local_player_url(port),
                quality: resolved.quality,
                mode: None,
                entitled: false,
                proxy_region: None,
                available: hls_master::quality_names(&resolved.qualities),
                clip_source: None,
                vod: None,
                kind: Some("hls".to_string()),
            })
        }
        // Already localhost HLS, produced by the adapter itself (YouTube's
        // DASH-backed 1440p/2160p renditions). Sending it through the relay
        // would proxy a local server through another local server and rewrite
        // playlists that are already exactly what the player needs.
        PlaybackKind::LocalHls => {
            // The relay is not used on this path, so a refresher registered by
            // an EARLIER stream would otherwise stay installed for the rest of
            // the session: `StreamServer::stop` clears the proxy url but not
            // the refresher. Inert while nothing re-signs, and wrong the moment
            // something does.
            crate::services::stream_server::set_manifest_refresher(None).await;
            Ok(StreamStartResult {
                url: resolved.url,
                quality: resolved.quality,
                mode: None,
                entitled: false,
                proxy_region: None,
                available: hls_master::quality_names(&resolved.qualities),
                clip_source: None,
                vod: None,
                kind: Some("hls".to_string()),
            })
        }
        // Nothing returns these: an adapter whose upstream is FLV serves it
        // through its own local relay and reports `LocalHls`, so the player
        // never needs a second media engine.
        other => Err(format!(
            "{} playback kind {:?} is not wired up yet",
            provider, other
        )),
    }
}

/// Extract the channel login from a twitch.tv live URL (e.g.
/// `https://twitch.tv/shroud` → `shroud`). Returns None for VOD/clip URLs and
/// anything that isn't a plain channel path.
fn channel_from_url(url: &str) -> Option<String> {
    let after = url.split("twitch.tv/").nth(1)?;
    let seg = after.split(['/', '?', '#']).next()?.trim();
    if seg.is_empty() || seg == "videos" || seg == "directory" {
        return None;
    }
    Some(seg.to_lowercase())
}

/// The localhost URL the player polls, with a cache-busting timestamp.
fn local_player_url(port: u16) -> String {
    format!(
        "http://localhost:{}/stream.m3u8?t={}",
        port,
        chrono::Utc::now().timestamp_millis()
    )
}

/// Resolve a Twitch clip to its signed MP4 URL WITHOUT touching any global live-
/// stream state (no solo-session reset, no proxy server, no `currentStream`
/// swap). The in-chat clip modal plays that MP4 directly in its own `<video>`,
/// so the main stream/chat keeps running underneath and the user lands back
/// exactly where they were when the modal closes.
#[tauri::command]
pub async fn resolve_clip_media(
    url: String,
    quality: String,
    state: State<'_, AppState>,
) -> Result<StreamStartResult, String> {
    let slug = tr::clip_slug_from_url(&url).ok_or_else(|| format!("Not a clip URL: {}", url))?;
    let oauth = state.twitch_auth.get_token().await.ok();
    let r = tr::resolve_clip(&slug, oauth.as_deref(), &quality)
        .await
        .map_err(|e| e.to_string())?;
    debug!("[Streaming] clip modal {} → '{}'", slug, r.quality);
    Ok(StreamStartResult {
        url: r.url,
        quality: r.quality,
        mode: None,
        entitled: false,
        proxy_region: None,
        available: r.available,
        clip_source: r.clip_source,
        vod: None,
        kind: None,
    })
}

#[tauri::command]
pub async fn start_stream(
    url: String,
    quality: String,
    state: State<'_, AppState>,
) -> Result<StreamStartResult, String> {
    debug!("[Streaming] start_stream called for URL: {}", url);

    // Clear the prior solo session up front; only a live resolve below
    // re-registers it (keeps a stale session off clip/VOD playback).
    crate::services::stream_server::set_solo_session(None);

    // Non-Twitch platform → its own StreamSource adapter. Dispatching on the URL
    // keeps this the single playback entry point, so quality changes, restarts
    // and session resume all keep working with no new command surface.
    if let WatchTarget::Provider { provider, channel } = watch_urls::classify(&url) {
        return start_provider_stream(provider, &channel, &quality).await;
    }

    // Everything below this point is Twitch, so restore the relay's full
    // Twitch behaviour (ad detection, segment projection, LL probe) in case the
    // previous stream was a provider one.
    crate::services::stream_server::set_upstream_profile(
        crate::services::stream_server::UpstreamProfile::Twitch,
    );

    let streamlink_settings = { state.settings.lock().unwrap().streamlink.clone() };
    let oauth = state.twitch_auth.get_token().await.ok();

    // Clip → signed MP4, loaded directly by the player (no HLS proxy).
    if let Some(slug) = tr::clip_slug_from_url(&url) {
        let r = tr::resolve_clip(&slug, oauth.as_deref(), &quality)
            .await
            .map_err(|e| e.to_string())?;
        debug!("[Streaming] clip {} → '{}'", slug, r.quality);
        return Ok(StreamStartResult {
            url: r.url,
            quality: r.quality,
            mode: None,
            entitled: false,
            proxy_region: None,
            available: r.available,
            clip_source: r.clip_source,
            vod: None,
            kind: None,
        });
    }

    // VOD → HLS media playlist, relayed through the local stream server.
    if let Some(vod_id) = tr::vod_id_from_url(&url) {
        return start_vod(&vod_id, oauth.as_deref(), &quality, &state, None).await;
    }

    // Live channel.
    let channel =
        channel_from_url(&url).ok_or_else(|| format!("Unrecognized Twitch URL: {}", url))?;
    // retry_streams = delay between attempts, stream_timeout = total budget, so a
    // channel that just went live connects once its playlist appears.
    let core = tr::resolve_live_resilient(
        &channel,
        oauth.as_deref(),
        &quality,
        streamlink_settings.retry_streams,
        streamlink_settings.stream_timeout,
    )
    .await;

    // A resolution-owning plugin takes the non-entitled case when installed;
    // otherwise (or when it declines or fails) the core resolution serves.
    let resolved = resolve_via_plugin(
        &state,
        crate::services::stream_server::SOLO_STREAM_ID,
        &channel,
        &quality,
        &core,
    )
    .await;

    // Android can't host a resolution plugin (it would be a spawned native
    // process), so ad-free playback occupies the same seam in-core. It obeys
    // the same contract: it declines the entitled case and any failure, and
    // declining means the core resolution serves.
    #[cfg(target_os = "android")]
    let resolved = match resolved {
        Some(r) => Some(r),
        None => {
            let video = { state.settings.lock().unwrap().video_player.clone() };
            crate::services::ad_bypass::resolve_master(
                &channel,
                &quality,
                core.as_ref().ok(),
                &video,
                state.twitch_auth.clone(),
            )
            .await
        }
    };

    let r = match resolved {
        Some(resolved) => resolved,
        None => core.map_err(|e| e.to_string())?,
    };

    log::info!(
        "[Streaming] {} '{}' → '{}' (mode={}) available={:?}",
        channel,
        quality,
        r.quality,
        r.status.mode,
        r.available
    );
    auth_proxy::set_status(r.status.clone());
    let port = StreamServer::start_proxy_server(r.url)
        .await
        .map_err(|e| e.to_string())?;
    // Register the live solo session AFTER the relay is serving it, so the
    // plugin protocol's "solo" stream id (set_upstream, on_ad_window) always
    // addresses a live relay.
    crate::services::stream_server::set_solo_session(Some(channel.clone()));
    Ok(StreamStartResult {
        url: local_player_url(port),
        quality: r.quality,
        mode: Some(r.status.mode),
        entitled: r.status.entitled,
        proxy_region: r.status.proxy_region,
        available: r.available,
        clip_source: None,
        vod: None,
        kind: None,
    })
}

/// Resolve a VOD, relay it, and decide where it starts. `start_override`
/// (a live rewind's mapped position) wins over the stored resume position;
/// otherwise the progress store answers, when the resume setting is on.
async fn start_vod(
    vod_id: &str,
    oauth: Option<&str>,
    quality: &str,
    state: &State<'_, AppState>,
    start_override: Option<f64>,
) -> Result<StreamStartResult, String> {
    let (resolved, info) = tokio::join!(
        tr::resolve_vod(vod_id, oauth, quality),
        tr::fetch_vod_info(vod_id)
    );
    let r = resolved.map_err(|e| e.to_string())?;
    let port = StreamServer::start_proxy_server_with(r.url, false)
        .await
        .map_err(|e| e.to_string())?;
    let resume_enabled = state.settings.lock().unwrap().video_player.resume_vod_playback;
    let start_position = match start_override {
        Some(p) => Some(p),
        None if resume_enabled => {
            let id = vod_id.to_string();
            tokio::task::spawn_blocking(move || {
                crate::services::vod_progress_service::resume_position(&id)
            })
            .await
            .ok()
            .flatten()
        }
        None => None,
    };
    debug!(
        "[Streaming] vod {} → '{}' status={} len={:?} start={:?}",
        vod_id, r.quality, info.status, info.length_seconds, start_position
    );
    Ok(StreamStartResult {
        url: local_player_url(port),
        quality: r.quality,
        mode: None,
        entitled: false,
        proxy_region: None,
        available: r.available,
        clip_source: None,
        kind: None,
        vod: Some(VodStartInfo {
            video_id: vod_id.to_string(),
            status: info.status,
            length_seconds: info.length_seconds,
            recorded_at: info.recorded_at,
            channel_login: info.owner_login,
            title: info.title,
            thumbnail_url: info.thumbnail_url,
            start_position_secs: start_position,
            rewound_from_live: false,
            muted_segments: info.muted_segments,
        }),
    })
}

/// Whether a live broadcast can be rewound: Twitch only keeps a recording
/// while the channel has VODs enabled, and `archiveVideo` is absent otherwise.
#[derive(Debug, Clone, Serialize)]
pub struct LiveRewindInfo {
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub video_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recorded_at: Option<String>,
}

/// Per-channel answer cache so the player can ask at every live start
/// without a GQL round trip each time. A channel that has just gone live can
/// take a little while to get its recording, so a negative answer expires
/// sooner than a positive one.
static LIVE_REWIND_CACHE: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<String, (LiveRewindInfo, std::time::Instant)>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));
const LIVE_REWIND_CACHE_POSITIVE: std::time::Duration = std::time::Duration::from_secs(30 * 60);
const LIVE_REWIND_CACHE_NEGATIVE: std::time::Duration = std::time::Duration::from_secs(2 * 60);

/// Can this live channel be rewound into its recording? Cached; a lookup
/// failure reports `available: false` without caching, so the next ask
/// retries.
#[tauri::command]
pub async fn get_live_rewind_info(channel: String) -> Result<LiveRewindInfo, String> {
    let channel = channel.trim().trim_start_matches('#').to_lowercase();
    if channel.is_empty() {
        return Err("No channel".to_string());
    }
    if let Ok(cache) = LIVE_REWIND_CACHE.lock() {
        if let Some((info, at)) = cache.get(&channel) {
            let ttl = if info.available {
                LIVE_REWIND_CACHE_POSITIVE
            } else {
                LIVE_REWIND_CACHE_NEGATIVE
            };
            if at.elapsed() < ttl {
                return Ok(info.clone());
            }
        }
    }
    let info = match tr::fetch_live_archive(&channel).await {
        Ok(Some(archive)) => LiveRewindInfo {
            available: true,
            video_id: Some(archive.video_id),
            recorded_at: archive.recorded_at,
        },
        Ok(None) => LiveRewindInfo {
            available: false,
            video_id: None,
            recorded_at: None,
        },
        Err(e) => {
            debug!("[Streaming] live rewind lookup for {} failed: {}", channel, e);
            return Ok(LiveRewindInfo {
                available: false,
                video_id: None,
                recorded_at: None,
            });
        }
    };
    if let Ok(mut cache) = LIVE_REWIND_CACHE.lock() {
        cache.insert(channel, (info.clone(), std::time::Instant::now()));
    }
    Ok(info)
}

/// Rewind a live broadcast: switch the solo relay onto the channel's
/// recording VOD, either at an absolute broadcast position (`position_secs`,
/// what the timeline scrubber sends) or `behind_secs` behind now (None =
/// from the start). The live chat session is untouched; the drops heartbeat
/// target is cleared because watching the recording is not watching live,
/// and "Back to live" (a normal live start) re-registers it.
#[tauri::command]
pub async fn rewind_live_stream(
    channel: String,
    behind_secs: Option<f64>,
    position_secs: Option<f64>,
    quality: String,
    state: State<'_, AppState>,
) -> Result<StreamStartResult, String> {
    let channel = channel.trim().trim_start_matches('#').to_lowercase();
    if channel.is_empty() {
        return Err("No channel to rewind".to_string());
    }
    let archive = tr::fetch_live_archive(&channel)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| {
            format!(
                "{} has no recording of this broadcast to rewind into (VODs may be disabled)",
                channel
            )
        })?;
    let position = match position_secs {
        Some(p) => archive.clamp_position(p),
        None => archive.position_for(chrono::Utc::now(), behind_secs),
    };
    state.watch_heartbeat.clear_target().await;
    crate::services::stream_server::set_solo_session(None);
    crate::services::stream_server::set_upstream_profile(
        crate::services::stream_server::UpstreamProfile::Twitch,
    );
    let oauth = state.twitch_auth.get_token().await.ok();
    let mut result = start_vod(
        &archive.video_id,
        oauth.as_deref(),
        &quality,
        &state,
        Some(position),
    )
    .await?;
    if let Some(v) = result.vod.as_mut() {
        v.rewound_from_live = true;
    }
    log::info!(
        "[Streaming] rewind {} → vod {} at {:.0}s (behind={:?}, position={:?})",
        channel,
        archive.video_id,
        position,
        behind_secs,
        position_secs
    );
    Ok(result)
}

#[tauri::command]
pub async fn stop_stream() -> Result<(), String> {
    // Tear down the YouTube DASH relay for this stream too. Unconditional: it is
    // a no-op for an id it does not know, which is every non-YouTube stream.
    // Without it a stopped stream's session stayed registered, held its init
    // segments and cache, and counted against the relay cap.
    crate::services::youtube_dash::stop(crate::services::stream_server::SOLO_STREAM_ID).await;
    // Same for the TikTok relay, and same reasoning: unknown ids are a no-op.
    crate::services::tiktok_relay::stop(crate::services::stream_server::SOLO_STREAM_ID);
    StreamServer::stop().await.map_err(|e| e.to_string())
}

/// Whether the relay's LL-HLS origin is actively serving parts for this stream.
/// True ⇒ hls.js should run in `lowLatencyMode` (a real `#EXT-X-PART` playlist with
/// blocking reload is being served). The player reads this once at construction to
/// pick the hls.js mode. NOT the same as "the channel is low-latency" — see
/// `get_stream_prefetch_present`.
#[tauri::command]
pub fn get_stream_low_latency() -> bool {
    crate::services::stream_server::is_low_latency()
}

/// Enable or disable the experimental parts-based low-latency origin at runtime.
/// Default is DISABLED: the stable whole-segment path serves every stream, which plays
/// cleanly on all channels and hardware. Turning this on lets the synthesized spec
/// LL-HLS origin (`#EXT-X-PART` + blocking reload, ~Twitch latency) take over on
/// low-latency channels. Takes effect on the next stream start (the origin is probed at
/// start), so the caller should restart the active stream after toggling. Kept as a
/// runtime switch (not a compile flag) so it can be A/B tested and proven per machine
/// before it is ever made the default.
#[tauri::command]
pub fn set_experimental_low_latency(enabled: bool) {
    crate::services::ll_origin::set_disabled(!enabled);
}

/// Report which video codecs this machine can decode and the user allows (families:
/// "av1","hevc","h264"), most-preferred first. The frontend probes
/// `MediaSource.isTypeSupported` and the `enhanced_codecs` setting and calls this at
/// startup and whenever the setting changes. The resolver then prefers the most
/// efficient decodable codec at a given resolution (which also routes AV1/HEVC CMAF
/// streams through the low-latency origin). H.264 is always kept as the fallback, so
/// selection can never resolve to a codec this machine can't play.
#[tauri::command]
pub fn set_codec_preference(prefs: Vec<String>) {
    crate::services::twitch_resolver::set_codec_preference(prefs);
}

/// Report the tallest rendition this display can actually show, in pixels.
/// 0 clears the cap.
///
/// Mobile only. A phone was resolving to Twitch's 2560x1440 source and scaling
/// it into a band roughly 1080x608 - several times the pixels the screen can
/// display, at about double the bitrate, which is decoder power, memory
/// bandwidth, GPU scaling and radio time all spent on pixels that get thrown
/// away. The resolver had no concept of the display, and hls.js cannot help
/// because we resolve to a single variant before the player sees a manifest.
///
/// Re-callable: an Android device can switch display mode (FHD+ / QHD+) at
/// runtime, so the shell pushes this again on every display change. It applies
/// at the NEXT resolve; no reload is forced on an already-playing stream.
///
/// Only affects "best"/auto. An explicitly chosen rendition still wins.
#[tauri::command]
pub fn set_max_video_height(height: u32) {
    crate::services::twitch_resolver::set_max_video_height(height);
}

/// Start a low-latency diagnostic recording session. Returns the full path of the
/// JSONL file the frontend (and origin) will append timestamped records to, so a
/// live drift/A-V-sync session can be analyzed from recorded facts. Rotates files.
#[tauri::command]
pub fn start_ll_diag(label: String) -> Result<String, String> {
    crate::services::ll_diagnostics::start_session(&label)
        .map(|p| p.to_string_lossy().to_string())
        .map_err(|e| e.to_string())
}

/// Append a batch of already-serialized JSON-line records to the diagnostic
/// session identified by `path` (records from superseded sessions are dropped,
/// see `ll_diagnostics::append_lines`). Best-effort; never errors playback.
#[tauri::command]
pub fn append_ll_diag(lines: Vec<String>, path: String) {
    crate::services::ll_diagnostics::append_lines(&lines, &path);
}

/// End the diagnostic session identified by `path` (no-op if superseded).
#[tauri::command]
pub fn stop_ll_diag(path: String) {
    crate::services::ll_diagnostics::stop_session(&path);
}

/// Current ad-detection state for the live stream the local player is pulling.
/// The detector scans every media-playlist poll for Twitch ad-stitch markers,
/// so this reflects whether ads are slipping through the proxy right now.
#[tauri::command]
pub async fn get_ad_detection() -> crate::services::stream_server::AdDetectionState {
    crate::services::stream_server::ad_state()
}

#[tauri::command]
pub async fn get_stream_qualities(
    url: String,
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    // Provider streams answer from their own adapter (whose parsed master is
    // cached, so this costs no extra webview resolve before start_stream).
    if let WatchTarget::Provider { provider, channel } = watch_urls::classify(&url) {
        let source = crate::services::providers::registry()
            .await
            .get_source(provider)
            .ok_or_else(|| format!("{} streams aren't supported in this build yet", provider))?;
        return source
            .qualities(&channel)
            .await
            .map(|q| hls_master::quality_names(&q))
            .map_err(|e| e.to_string());
    }

    let oauth = state.twitch_auth.get_token().await.ok();

    // Resolve once at "best" and surface the variant menu it discovered. The
    // 20s master cache means the subsequent start_stream is a cache hit.
    if let Some(slug) = tr::clip_slug_from_url(&url) {
        tr::resolve_clip(&slug, oauth.as_deref(), "best")
            .await
            .map(|r| r.available)
            .map_err(|e| e.to_string())
    } else if let Some(vod_id) = tr::vod_id_from_url(&url) {
        tr::resolve_vod(&vod_id, oauth.as_deref(), "best")
            .await
            .map(|r| r.available)
            .map_err(|e| e.to_string())
    } else {
        let channel =
            channel_from_url(&url).ok_or_else(|| format!("Unrecognized Twitch URL: {}", url))?;
        tr::resolve_live(&channel, oauth.as_deref(), "best")
            .await
            .map(|r| r.available)
            .map_err(|e| e.to_string())
    }
}

#[tauri::command]
pub async fn change_stream_quality(
    url: String,
    quality: String,
    state: State<'_, AppState>,
) -> Result<StreamStartResult, String> {
    // Don't stop the server - just update the stream URL.
    // The server keeps running on the same port.
    start_stream(url, quality, state).await
}

#[tauri::command]
pub async fn register_active_channel(
    channel_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let bg_service = state.background_service.lock().await;
    let ws_service_mutex = bg_service.websocket_service.clone();
    let ws_service = ws_service_mutex.lock().await;
    ws_service.register_active_channel(&channel_id).await;
    Ok(())
}

#[tauri::command]
pub async fn unregister_active_channel(
    channel_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let bg_service = state.background_service.lock().await;
    let ws_service_mutex = bg_service.websocket_service.clone();
    let ws_service = ws_service_mutex.lock().await;
    ws_service.unregister_active_channel(&channel_id).await;
    Ok(())
}

/// One-shot probe: resolve playable URLs in the webview, then actually fetch a
/// fragment from each track.
///
/// The raw per-itag URLs on the watch page are refused (untransformed `n`, no PO
/// token, `alr=yes`); these have been through youtubei.js `decipher` with a
/// token attached, the same treatment Invidious applies.
///
/// ```js
/// await sn.sabrProbe('UvAxI_BUfqQ')
/// ```
#[tauri::command]
pub async fn youtube_sabr_probe(video_id: String) -> Result<String, String> {
    let s = crate::services::youtube_potoken::resolve_streams(&video_id, 1080)
        .await
        .map_err(|e| format!("could not resolve streams: {}", e))?;

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| e.to_string())?;

    // No `&sq=`, which asks for the live edge and is also the cheapest way to
    // learn the head sequence number.
    async fn probe_one(
        http: &reqwest::Client,
        url: &str,
        label: &str,
    ) -> Result<String, String> {
        let started = std::time::Instant::now();
        let resp = http
            .get(url)
            .send()
            .await
            .map_err(|e| format!("{}: request failed: {}", label, e))?;
        let status = resp.status();
        let head = resp
            .headers()
            .get("x-head-seqnum")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("?")
            .to_string();
        let ctype = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("?")
            .to_string();
        let bytes = resp.bytes().await.unwrap_or_default();
        let kind = if bytes.len() > 8 && &bytes[4..8] == b"ftyp" {
            "MP4/ftyp"
        } else if bytes.len() > 8 && &bytes[4..8] == b"moof" {
            "MP4/moof"
        } else if bytes.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
            "WebM/EBML"
        } else if bytes.starts_with(&[0x1F, 0x43, 0xB6, 0x75]) {
            "WebM/Cluster"
        } else {
            "unknown"
        };
        let line = format!(
            "{}: HTTP {} {} {} bytes ({}) head_seq={} in {:.1}s",
            label,
            status.as_u16(),
            ctype,
            bytes.len(),
            kind,
            head,
            started.elapsed().as_secs_f64()
        );
        log::info!("[YTProbe] {}", line);
        if !status.is_success() {
            return Err(line);
        }
        Ok(line)
    }

    let mut lines: Vec<String> = Vec::new();
    for v in &s.videos {
        lines.push(probe_one(&http, &v.url, &format!("video {}", v.name)).await?);
    }
    lines.push(probe_one(&http, &s.audio_url, "audio").await?);

    let summary = format!(
        "{}: {} | {}",
        video_id,
        s.videos
            .iter()
            .map(|v| v.name.as_str())
            .collect::<Vec<_>>()
            .join(", "),
        lines.join(" | ")
    );
    log::info!("[YTProbe] {}", summary);
    Ok(summary)
}

#[cfg(test)]
mod liveness_tests {
    use super::*;
    use crate::models::provider_stream::ProviderStream;

    fn row(slug: &str, is_live: bool) -> ProviderStream {
        serde_json::from_value(serde_json::json!({
            "provider": "kick",
            "key": format!("kick:{}", slug),
            "user_login": slug,
            "user_name": slug,
            "is_live": is_live,
            "watch_url": format!("https://kick.com/{}", slug),
        }))
        .expect("fixture row")
    }

    #[test]
    fn a_live_row_is_an_answer() {
        assert_eq!(verdict_for(&[row("bigaust", true)], "bigaust"), Liveness::Live);
    }

    #[test]
    fn an_explicitly_offline_row_is_an_answer() {
        assert_eq!(
            verdict_for(&[row("bigaust", false)], "bigaust"),
            Liveness::Offline
        );
    }

    /// The whole point of the three-state split: a channel MISSING from the batch
    /// response is not evidence it ended. The row is absent both when the channel
    /// does not exist and when the request was refused, and treating that as
    /// "offline" is what threw a viewer off a stream that was still running.
    #[test]
    fn an_absent_row_is_not_evidence_of_an_ending() {
        assert_eq!(verdict_for(&[], "bigaust"), Liveness::Unknown);
        assert_eq!(
            verdict_for(&[row("someone_else", true)], "bigaust"),
            Liveness::Unknown
        );
    }

    #[test]
    fn slug_match_is_case_insensitive() {
        assert_eq!(verdict_for(&[row("bigaust", true)], "BigAust"), Liveness::Live);
    }
}
