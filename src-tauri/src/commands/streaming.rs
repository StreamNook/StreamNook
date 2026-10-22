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
    /// mean an HLS playlist, "flv" means an FLV stream needing the mpegts path,
    /// "cast" (Android) means a Cast device plays it and `url` is a placeholder.
    /// Absent for every other Twitch path, so existing consumers are unaffected.
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
    /// Stamp for this start. The player's position reports carry it back, so
    /// Rust decides what each report may write (`live_bookmark::origin_of`).
    /// 0 is never issued, so an unstamped start writes nothing.
    pub play_id: u64,
    /// Audio ranges Twitch muted, for the seekbar marks. Empty when none are
    /// known; on a `recording` VOD that means not yet determined, not "none".
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub muted_segments: Vec<crate::services::muted_segments::MutedRange>,
    /// Category changes over the broadcast, for the chapter marks and list.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub chapters: Vec<crate::services::vod_chapters::Chapter>,
    /// Seek-preview sprite sheets. Finished VODs only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storyboard: Option<crate::services::vod_storyboard::Storyboard>,
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


/// Why `start_stream` failed. A plain message serializes as the bare string it
/// always was, so every existing caller reads it unchanged; a stream the
/// platform only serves to a signed-in account arrives as
/// `{ "sign_in": { provider, channel, message } }`, which the player turns into
/// a sign-in prompt.
#[derive(Debug, serde::Serialize)]
#[serde(untagged)]
pub enum StartError {
    Message(String),
    SignIn {
        sign_in: crate::services::providers::source::SignInRequired,
    },
}

impl From<String> for StartError {
    fn from(message: String) -> Self {
        StartError::Message(message)
    }
}

impl std::fmt::Display for StartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StartError::Message(m) => f.write_str(m),
            StartError::SignIn { sign_in } => f.write_str(&sign_in.message),
        }
    }
}

impl StartError {
    fn from_resolve(e: anyhow::Error) -> Self {
        let sign_in = e
            .chain()
            .find_map(|c| c.downcast_ref::<crate::services::providers::source::SignInRequired>());
        match sign_in {
            Some(s) => StartError::SignIn { sign_in: s.clone() },
            None => StartError::Message(e.to_string()),
        }
    }
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
) -> Result<StreamStartResult, StartError> {
    let source = crate::services::providers::registry()
        .await
        .get_source(provider)
        .ok_or_else(|| format!("{} streams aren't supported in this build yet", provider))?;

    let resolved = source
        .resolve_playback(crate::services::stream_server::SOLO_STREAM_ID, channel, quality)
        .await
        .map_err(StartError::from_resolve)?;

    log::info!(
        "[Streaming] {}:{} '{}' → '{}' ({:?}) variants={}",
        provider,
        channel,
        quality,
        resolved.quality,
        resolved.kind,
        resolved.qualities.len()
    );

    // "Auto" follows the link from here on, like Twitch. A link that just
    // failed to hold this rung starts lower, which for an adapter means
    // resolving again at that rung's name.
    let mut resolved = resolved;
    let adapts = provider != "tiktok";
    if adapts {
        let route = std::sync::Arc::new(ProviderSoloRoute {
            provider,
            channel: channel.to_string(),
        });
        let start = crate::services::link_adapt::begin_named(
            crate::services::link_adapt::SOLO,
            &resolved.qualities,
            &resolved.quality,
            is_auto_quality(quality),
            route,
        );
        if let Some(rung) = start.filter(|r| !r.name.eq_ignore_ascii_case(&resolved.quality)) {
            match source.resolve_playback(crate::services::stream_server::SOLO_STREAM_ID, channel, &rung.name).await {
                Ok(lower) => resolved = lower,
                Err(e) => log::warn!("[Streaming] {}:{} could not start on {}: {}", provider, channel, rung.name, e),
            }
        }
    }

    let url = serve_provider_playback(provider, channel, quality, &resolved).await?;
    Ok(StreamStartResult {
        url,
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

/// "Auto": the quality the link decides. `auto` on every platform. On the
/// phones "best" is also Auto, which is what they shipped, whatever rules the
/// governor runs; on the desktop "best" and "source" stay the fixed source
/// rendition they always were.
pub(crate) fn is_auto_quality(quality: &str) -> bool {
    match quality.trim().to_ascii_lowercase().as_str() {
        "auto" => true,
        "best" | "source" => cfg!(mobile),
        _ => false,
    }
}

/// The solo player's Kick or YouTube stream as `link_adapt` moves it: the
/// adapter resolves the rung by name and the stream is served again, so the
/// player reloads. Its own segment downloads never measure the link: the
/// relay prefetches them (and times those fetches itself), and YouTube's
/// 1440p and up come from a local relay that does the same.
struct ProviderSoloRoute {
    provider: &'static str,
    channel: String,
}

impl crate::services::link_adapt::Route for ProviderSoloRoute {
    fn switch(&self, to: crate::services::link_adapt::Rung, _same_decoder: bool) -> crate::services::link_adapt::SwitchFuture {
        let (provider, channel) = (self.provider, self.channel.clone());
        Box::pin(async move {
            let source = crate::services::providers::registry()
                .await
                .get_source(provider)
                .ok_or_else(|| anyhow::anyhow!("no {provider} source"))?;
            let resolved = source
                .resolve_playback(crate::services::stream_server::SOLO_STREAM_ID, &channel, &to.name)
                .await?;
            let url = serve_provider_playback(provider, &channel, &to.name, &resolved)
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))?;
            Ok(Some(url))
        })
    }

    fn player_measures_link(&self) -> bool {
        false
    }
}

/// Hand a provider's resolved stream to the solo player: through the relay,
/// or as is when the adapter already serves it from localhost. Returns the
/// URL the player loads.
async fn serve_provider_playback(
    provider: &'static str,
    channel: &str,
    quality: &str,
    resolved: &crate::services::providers::source::ResolvedPlayback,
) -> Result<String, StartError> {
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
                        // The rung playing now: "Auto" may have moved off the
                        // quality this stream started on, and a re-sign must
                        // not quietly undo that.
                        let q = crate::services::link_adapt::current_quality(
                            crate::services::link_adapt::SOLO,
                        )
                        .unwrap_or_else(|| q.clone());
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
            let port = StreamServer::start_proxy_server(resolved.url.clone())
                .await
                .map_err(|e| e.to_string())?;
            Ok(local_player_url(port))
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
            Ok(resolved.url.clone())
        }
        // Nothing returns these: an adapter whose upstream is FLV serves it
        // through its own local relay and reports `LocalHls`, so the player
        // never needs a second media engine.
        other => Err(format!(
            "{} playback kind {:?} is not wired up yet",
            provider, other
        )
        .into()),
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
pub(crate) fn local_player_url(port: u16) -> String {
    format!(
        "http://{}:{}/stream.m3u8?t={}",
        LOOPBACK_HOST,
        port,
        chrono::Utc::now().timestamp_millis()
    )
}

/// The relay's host in player URLs. The webview players and their CSP name
/// `localhost`; AVPlayer gets the address itself, so the iPhone never resolves
/// a name or tries IPv6 first for a relay bound to IPv4.
#[cfg(not(target_os = "ios"))]
const LOOPBACK_HOST: &str = "localhost";
#[cfg(target_os = "ios")]
const LOOPBACK_HOST: &str = "127.0.0.1";

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
) -> Result<StreamStartResult, StartError> {
    let result = start_stream_inner(url, quality, state).await;
    // TV screen mode plays whatever the phone would: every start, restart and
    // quality change passes through here.
    #[cfg(target_os = "android")]
    if let Ok(started) = &result {
        crate::services::tv_screen::on_local_stream(Some(&started.url));
    }
    result
}

async fn start_stream_inner(
    url: String,
    quality: String,
    state: State<'_, AppState>,
) -> Result<StreamStartResult, StartError> {
    debug!("[Streaming] start_stream called for URL: {}", url);

    // Clear the prior solo session up front; only a live resolve below
    // re-registers it (keeps a stale session off clip/VOD playback).
    crate::services::stream_server::set_solo_session(None);
    // Whatever adapted before (another channel, a clip, a VOD) stops here;
    // only a live start below begins again.
    crate::services::link_adapt::end(crate::services::link_adapt::SOLO);

    // Non-Twitch platform → its own StreamSource adapter. Dispatching on the URL
    // keeps this the single playback entry point, so quality changes, restarts
    // and session resume all keep working with no new command surface.
    if let WatchTarget::Provider { provider, channel } = watch_urls::classify(&url) {
        crate::services::live_bookmark::starting(None);
        #[cfg(target_os = "android")]
        if crate::services::cast_session::takes_stream(provider) {
            return Ok(cast_placeholder(provider, &channel, &quality));
        }
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
        crate::services::live_bookmark::starting(None);
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
        crate::services::live_bookmark::starting(None);
        let mut result = start_vod(&vod_id, oauth.as_deref(), &quality, &state, None)
            .await
            .map_err(StartError::from)?;
        if let Some(v) = result.vod.as_mut() {
            v.play_id = crate::services::live_bookmark::register_play(
                crate::services::live_bookmark::Origin::Deliberate,
            );
        }
        return Ok(result);
    }

    // Live channel.
    let channel =
        channel_from_url(&url).ok_or_else(|| format!("Unrecognized Twitch URL: {}", url))?;
    // A restart or quality change of the same channel keeps the live
    // bookmark's session; any other channel ends it.
    crate::services::live_bookmark::starting(Some(&channel));

    #[cfg(target_os = "android")]
    if crate::services::cast_session::takes_stream("twitch") {
        return Ok(cast_placeholder("twitch", &channel, &quality));
    }
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

    // A phone can't host a resolution plugin (it would be a spawned native
    // process), so ad-free playback occupies the same seam in-core. It obeys
    // the same contract: it declines the entitled case and any failure, and
    // declining means the core resolution serves.
    #[cfg(mobile)]
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
    // The link decides the rung from here on ("Auto" only), never past what
    // this device decodes; a link that just failed to hold this rung starts
    // lower.
    let serve_url = crate::services::link_adapt::begin(
        crate::services::link_adapt::SOLO,
        &tr::playable_variants(&r.master, &channel),
        &r.url,
        is_auto_quality(&quality),
        crate::services::stream_server::solo_twitch_route(),
    )
    .map_or_else(|| r.url.clone(), |rung| rung.url);
    // Read with no await since `begin`: the session this start owns.
    let generation = crate::services::link_adapt::generation(crate::services::link_adapt::SOLO);
    let port = StreamServer::start_proxy_server(serve_url)
        .await
        .map_err(|e| e.to_string())?;
    // The parts origin's first downloads just measured the link (on the
    // whole-segment path the start pick downloads two segments itself). When
    // they say this rung clearly cannot be carried, restart on one that can
    // before the player loads anything: a longer load instead of freezes.
    crate::services::link_adapt::start_measure(crate::services::link_adapt::SOLO, generation).await;
    let port = match crate::services::link_adapt::start_check(
        crate::services::link_adapt::SOLO,
        generation,
    ) {
        Some(rung) => StreamServer::start_proxy_server(rung.url)
            .await
            .map_err(|e| e.to_string())?,
        None => port,
    };
    // Register the live solo session AFTER the relay is serving it, so the
    // plugin protocol's "solo" stream id (set_upstream, on_ad_window) always
    // addresses a live relay.
    crate::services::stream_server::set_solo_session(Some(channel.clone()));
    // The iPhone plays the audio-only rendition through a master that says it
    // is audio only (see `stream_server::AUDIO_PLAYLIST`).
    #[cfg(target_os = "ios")]
    let url = if r.quality.contains("audio") {
        local_player_url(port).replacen("/stream.m3u8", "/audio.m3u8", 1)
    } else {
        local_player_url(port)
    };
    #[cfg(not(target_os = "ios"))]
    let url = local_player_url(port);
    let resume = state
        .settings
        .lock()
        .map(|s| s.video_player.resume_vod_playback)
        .unwrap_or(false);
    crate::services::live_bookmark::begin_live(&channel, resume);
    Ok(StreamStartResult {
        url,
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

/// A live stream that a Cast device is playing. Nothing is resolved or relayed
/// on the phone: the watch session hands the channel to `cast_session`, which
/// looks it up for the TV, and the page shows the casting card in place of the
/// video. `kind: "cast"` is what tells it so.
#[cfg(target_os = "android")]
fn cast_placeholder(provider: &str, channel: &str, quality: &str) -> StreamStartResult {
    StreamStartResult {
        url: format!("cast://{}/{}", provider, channel.to_lowercase()),
        quality: quality.to_string(),
        mode: None,
        entitled: false,
        proxy_region: None,
        available: Vec::new(),
        clip_source: None,
        vod: None,
        kind: Some("cast".to_string()),
    }
}

/// How long a VOD open waits for the viewer's Twitch-side position.
const VOD_REMOTE_POSITION_BUDGET: std::time::Duration = std::time::Duration::from_millis(1500);

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
    let resume_enabled = state.settings.lock().unwrap().video_player.resume_vod_playback;
    // A position saved on another device lands in the store while the VOD
    // resolves, so the resume read below sees it. Bounded: a slow answer only
    // means this device's own position wins.
    let remote_read = async {
        if resume_enabled && start_override.is_none() {
            let _ = tokio::time::timeout(
                VOD_REMOTE_POSITION_BUDGET,
                crate::services::vod_sync::sync_on_open(vod_id),
            )
            .await;
        }
    };
    let (resolved, info, ()) = tokio::join!(
        tr::resolve_vod(vod_id, oauth, quality),
        tr::fetch_vod_info(vod_id),
        remote_read
    );
    let r = resolved.map_err(|e| e.to_string())?;
    let port = StreamServer::start_proxy_server_with(r.url, false)
        .await
        .map_err(|e| e.to_string())?;
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
            play_id: 0,
            muted_segments: info.muted_segments,
            chapters: info.chapters,
            storyboard: info.storyboard,
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
    /// Where the viewer left this broadcast before this playback, for the
    /// timeline's "you left here" mark. Never cached: it changes as they watch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub left_off_secs: Option<f64>,
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
/// retries. Also hands the recording to the live bookmark, and says where the
/// viewer left this broadcast before, for the timeline mark.
#[tauri::command]
pub async fn get_live_rewind_info(channel: String) -> Result<LiveRewindInfo, String> {
    let channel = channel.trim().trim_start_matches('#').to_lowercase();
    if channel.is_empty() {
        return Err("No channel".to_string());
    }
    let mut info = live_rewind_lookup(&channel).await;
    if let (true, Some(id)) = (info.available, info.video_id.clone()) {
        crate::services::live_bookmark::note_archive(&channel, &id, info.recorded_at.as_deref());
        info.left_off_secs = left_off_secs(&channel, &id, info.recorded_at.as_deref()).await;
    }
    Ok(info)
}

/// The "you left here" mark for this broadcast's recording, decided by
/// `live_bookmark::mark_position`.
async fn left_off_secs(channel: &str, video_id: &str, recorded_at: Option<&str>) -> Option<f64> {
    let start = chrono::DateTime::parse_from_rfc3339(recorded_at?).ok()?;
    let elapsed = (chrono::Utc::now() - start.with_timezone(&chrono::Utc)).num_milliseconds() as f64 / 1000.0;
    let id = video_id.to_string();
    let entry = tokio::task::spawn_blocking(move || crate::services::vod_progress_service::lookup(&id))
        .await
        .ok()
        .flatten()
        .filter(|e| !e.completed)?;
    crate::services::live_bookmark::mark_position(
        entry.position_secs,
        entry.updated_at,
        crate::services::live_bookmark::begun_ms(channel),
        elapsed,
    )
}

async fn live_rewind_lookup(channel: &str) -> LiveRewindInfo {
    if let Ok(cache) = LIVE_REWIND_CACHE.lock() {
        if let Some((info, at)) = cache.get(channel) {
            let ttl = if info.available {
                LIVE_REWIND_CACHE_POSITIVE
            } else {
                LIVE_REWIND_CACHE_NEGATIVE
            };
            if at.elapsed() < ttl {
                return info.clone();
            }
        }
    }
    let info = match tr::fetch_live_archive(channel).await {
        Ok(Some(archive)) => LiveRewindInfo {
            available: true,
            video_id: Some(archive.video_id),
            recorded_at: archive.recorded_at,
            left_off_secs: None,
        },
        Ok(None) => LiveRewindInfo {
            available: false,
            video_id: None,
            recorded_at: None,
            left_off_secs: None,
        },
        Err(e) => {
            debug!("[Streaming] live rewind lookup for {} failed: {}", channel, e);
            return LiveRewindInfo {
                available: false,
                video_id: None,
                recorded_at: None,
                left_off_secs: None,
            };
        }
    };
    if let Ok(mut cache) = LIVE_REWIND_CACHE.lock() {
        cache.insert(channel.to_string(), (info.clone(), std::time::Instant::now()));
    }
    info
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
        v.play_id = crate::services::live_bookmark::register_play(
            crate::services::live_bookmark::Origin::Rewind,
        );
    }
    crate::services::live_bookmark::begin_rewind(&channel, &archive.video_id, archive.recorded_at.as_deref());
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
    // Every close of a live stream comes through here, the tray close too
    // (it stops the stream but not the watch session).
    crate::services::live_bookmark::end(crate::services::live_bookmark::EndKind::Async);
    #[cfg(target_os = "android")]
    crate::services::tv_screen::on_local_stream(None);
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

/// Whether the broadcast itself is low-latency (Twitch sends PREFETCH hints),
/// independent of whether the parts origin took it over. With the origin off
/// the player still rides such a channel on promoted in-progress segments,
/// which is a tighter, thinner ride than a normal-latency broadcast, so the
/// automatic live-edge gap needs the distinction. Read once at construction,
/// after `get_stream_low_latency`.
#[tauri::command]
pub fn get_stream_prefetch_present() -> bool {
    crate::services::ll_origin::upstream_has_prefetch()
}

/// How far the player sits behind the relay's live edge, for a player that
/// cannot measure it itself (AVPlayer): it passes the programme date of the
/// frame it shows, in Unix milliseconds. With `target_ms` the answer carries a
/// catch-up rate toward that distance. Also drains the relay's record of the
/// player's requests since the previous call. `playlist_behind_ms` (how far the
/// player sits behind its playlist's end) words the readout when the relay has
/// no parts to measure against (the whole-segment route).
#[tauri::command]
pub fn get_relay_live_distance(
    player_pdt_ms: Option<f64>,
    target_ms: Option<i64>,
    playlist_behind_ms: Option<f64>,
) -> crate::services::relay_probe::LiveDistance {
    crate::services::relay_probe::report(player_pdt_ms, target_ms, playlist_behind_ms)
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

/// The live channel the solo player last restarted for a failed codec, and the
/// codec families it already stepped away from there.
static SOLO_CODEC_STEPS: std::sync::Mutex<(String, Vec<&'static str>)> =
    std::sync::Mutex::new((String::new(), Vec::new()));

/// The solo player could not decode its live Twitch stream, though the probe
/// said this machine or phone plays that codec. The codec leaves selection for the
/// session, and the answer says whether restarting the stream now gets away
/// from it. `mime_type` is the SourceBuffer type the player failed to create,
/// when it got that far; without it the rung playing now counts. Once per codec
/// per channel: a second failure means the restart found nothing else to play.
#[tauri::command]
pub fn report_solo_codec_failure(mime_type: Option<String>) -> bool {
    // Only a live Twitch resolve honors the codec list.
    let Some(channel) = crate::services::stream_server::solo_channel() else {
        return false;
    };
    let serving = crate::services::link_adapt::playing_family(crate::services::link_adapt::SOLO);
    let Some(family) = tr::fail_codec(&channel, mime_type.as_deref(), serving.as_deref()) else {
        return false;
    };
    let mut steps = SOLO_CODEC_STEPS.lock().unwrap_or_else(|e| e.into_inner());
    if steps.0 != channel {
        *steps = (channel.clone(), Vec::new());
    }
    let restart = !steps.1.contains(&family);
    if restart {
        steps.1.push(family);
    }
    debug!("[Streaming] {channel} could not decode {family}; restart={restart}");
    restart
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

/// A player's view of the link, for `link_adapt`: the core decides the
/// rendition (and on the phones the cushion), the player only reports what it
/// saw. `stream_id` is the MultiNook tile reporting; none is the solo player.
///
/// - `frag`: a whole segment the player downloaded itself (`bytes` in `ms`).
///   Only the whole-segment path sends these; on the parts path the player
///   reads from the relay over loopback and the origin's own reads count.
///   Rust also drops them where a local relay served the segment from memory.
/// - `stall`: playback has been frozen `ms` so far (sent while it lasts).
/// - `stall_end`: a stall ended after `ms`.
///
/// Decisions come back as the `playback-adapt` event.
#[tauri::command]
pub fn report_playback_health(
    event: String,
    bytes: Option<u64>,
    ms: Option<f64>,
    stream_id: Option<String>,
    cc: Option<u64>,
    media_ms: Option<f64>,
) {
    let id = stream_id.as_deref().unwrap_or(crate::services::link_adapt::SOLO);
    let secs = ms.unwrap_or(0.0).max(0.0) / 1000.0;
    match event.as_str() {
        // `cc` is the segment's discontinuity counter and `media_ms` how long
        // it plays: Rust leaves the first segment of a new run out, and times
        // what the rung's content really needs.
        "frag" => {
            if let Some(bytes) = bytes {
                let content = media_ms.filter(|m| *m > 0.0).map(|m| m / 1000.0);
                crate::services::link_adapt::note_player_segment(id, bytes, secs, content, cc);
            }
        }
        "stall" => crate::services::link_adapt::note_stall(id, secs, false),
        "stall_end" => crate::services::link_adapt::note_stall(id, secs, true),
        _ => {}
    }
}

/// Extra seconds behind the live edge the player should keep right now, read
/// when a player is built (later changes arrive as `playback-adapt`).
#[tauri::command]
pub fn get_live_edge_cushion() -> f64 {
    crate::services::link_adapt::extra_cushion(crate::services::link_adapt::SOLO)
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
    start_stream(url, quality, state).await.map_err(|e| e.to_string())
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

    // Shared client; the probe's 60 s deadline rides on each request.
    let http = crate::services::http::client();

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
            .timeout(std::time::Duration::from_secs(60))
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

#[cfg(test)]
mod start_error_tests {
    use super::*;
    use crate::services::providers::source::SignInRequired;

    #[test]
    fn a_plain_failure_is_still_a_bare_string_on_the_wire() {
        let e: StartError = "someone isn't live right now".to_string().into();
        assert_eq!(serde_json::to_value(&e).unwrap(), serde_json::json!("someone isn't live right now"));
    }

    #[test]
    fn a_sign_in_is_typed_on_the_wire_even_under_context() {
        let err = anyhow::Error::new(SignInRequired {
            provider: "tiktok",
            channel: "someone".into(),
            message: "@someone's LIVE is 18+. Sign in to TikTok to watch it.".into(),
        })
        .context("resolving someone");
        let e = StartError::from_resolve(err);
        assert_eq!(
            serde_json::to_value(&e).unwrap(),
            serde_json::json!({ "sign_in": {
                "provider": "tiktok",
                "channel": "someone",
                "message": "@someone's LIVE is 18+. Sign in to TikTok to watch it."
            }})
        );
        // Callers that only want words still get the message.
        assert_eq!(e.to_string(), "@someone's LIVE is 18+. Sign in to TikTok to watch it.");
    }
}
