use crate::commands::streaming::StreamStartResult;
use crate::models::settings::AppState;
use crate::rt::AppHandle;
use crate::services::multi_nook_server::{
    MultiNookServer, TileProfile, TilePromotion, TileRefresher,
};
use crate::services::stream_server::StreamServer;
use crate::services::providers::source::PlaybackKind;
use crate::services::providers::watch_urls::WatchTarget;
use crate::services::twitch_resolver as tr;
use log::debug;
use tauri::State;

/// Maximum number of concurrent streams allowed
const MAX_STREAMS: usize = 25;

/// Extract the channel login from a twitch.tv live URL. MultiNook tiles are
/// always live channels, so this is enough.
fn channel_from_url(url: &str) -> Option<String> {
    let after = url.split("twitch.tv/").nth(1)?;
    let seg = after.split(['/', '?', '#']).next()?.trim();
    if seg.is_empty() || seg == "videos" || seg == "directory" {
        return None;
    }
    Some(seg.to_lowercase())
}

/// The quality a tile asks its resolver for: its own pick, held to about
/// `cap` lines while it is a small tile. A height is what both resolvers'
/// pickers understand; a channel without a rendition near it gets the
/// nearest it has. "worst" and audio-only already sit under any cap.
pub fn tile_quality(own: &str, cap: Option<u32>) -> String {
    let Some(cap) = cap else { return own.to_string() };
    let lower = own.trim().to_ascii_lowercase();
    if lower == "worst" || lower.starts_with("audio") {
        return own.to_string();
    }
    match crate::services::quality::parse_quality_height(&lower) {
        Some(h) if h <= cap => own.to_string(),
        _ => format!("{cap}p"),
    }
}

/// Start a stream for multi-stream mode. Each tile resolves natively (same
/// pipeline as the solo player) and gets its own proxy server.
#[tauri::command]
pub async fn start_multi_nook(
    stream_id: String,
    url: String,
    quality: String,
    // Which platform this tile is on. Absent means Twitch, matching the
    // frontend's bare-key convention, so older callers keep working.
    provider: Option<String>,
    // The tile starts as one of the small tiles of a main layout, so the
    // small-tile quality cap applies. Absent means a full-size tile.
    small: Option<bool>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let provider = provider.unwrap_or_else(|| "twitch".to_string());
    let cap = cap_for(&state, small)?;
    let quality = tile_quality(&quality, cap);
    debug!(
        "[MultiNook] start_multi_nook called: id='{}', provider='{}', url='{}', quality='{}'",
        stream_id, provider, url, quality
    );

    let current_count = MultiNookServer::active_count().await;
    if current_count >= MAX_STREAMS {
        return Err(format!(
            "Maximum of {} concurrent streams reached",
            MAX_STREAMS
        ));
    }

    if provider != "twitch" {
        return start_provider_tile(&stream_id, &provider, &url, &quality, cap).await;
    }

    let (channel, r) = resolve_twitch_tile(&state, &stream_id, &url, &quality).await?;
    let codecs = variant_codecs(&r.master, &r.url, &r.quality);

    let port = MultiNookServer::start_proxy(&stream_id, r.url, TileProfile::Twitch, None)
        .await
        .map_err(|e| e.to_string())?;

    // Keep what the resolve learned, so this tile can be handed to the solo
    // player later without asking usher again. Costs one clone per tile start.
    MultiNookServer::set_promotion(
        &stream_id,
        TilePromotion {
            quality: r.quality.clone(),
            available: r.available.clone(),
            status: Some(r.status.clone()),
            capped: cap,
            codecs,
        },
    )
    .await;

    // Tag the proxy URL when the tile's relay activated its LL-HLS origin (settled
    // inside start_proxy, before this point). The player must choose its hls.js mode
    // at construction, and riding the flag on the URL it already consumes keeps the
    // two atomic: a refreshed URL always carries the matching mode.
    let low_latency = MultiNookServer::is_low_latency(&stream_id).await;
    let proxy_url = format!(
        "http://localhost:{}/stream.m3u8?t={}{}",
        port,
        chrono::Utc::now().timestamp_millis(),
        if low_latency { "&ll=1" } else { "" }
    );

    debug!(
        "[MultiNook] '{}' ({}) → {} (mode={})",
        stream_id, channel, proxy_url, r.status.mode
    );

    Ok(proxy_url)
}

/// What a tile's change of size did to its stream.
#[derive(serde::Serialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Retier {
    /// The relay now serves the new quality under the same local URL; the
    /// player keeps playing.
    Swapped,
    /// The tile needs a full restart (another platform, another codec family,
    /// a changed low-latency mode, or no relay to swap).
    Restart,
}

/// A tile became small or stopped being small while its small-tile cap
/// applies: serve its stream at the quality its new size wants. A Twitch tile
/// is re-resolved and swapped in place on its own relay, the path an ad pivot
/// already uses mid-playback, so the player is not rebuilt. Anything that
/// would confuse a running decoder asks for a restart instead.
#[tauri::command]
pub async fn retier_multi_nook_tile(
    stream_id: String,
    url: String,
    quality: String,
    provider: Option<String>,
    small: Option<bool>,
    state: State<'_, AppState>,
) -> Result<Retier, String> {
    if provider.as_deref().unwrap_or("twitch") != "twitch" {
        return Ok(Retier::Restart);
    }
    let Some((_, current)) = MultiNookServer::promotion_target(&stream_id).await else {
        return Ok(Retier::Restart);
    };
    let cap = cap_for(&state, small)?;
    let quality = tile_quality(&quality, cap);
    let was_low_latency = MultiNookServer::is_low_latency(&stream_id).await;

    let (_, r) = resolve_twitch_tile(&state, &stream_id, &url, &quality).await?;
    let codecs = variant_codecs(&r.master, &r.url, &r.quality);
    if !same_codec_family(current.codecs.as_deref(), codecs.as_deref()) {
        debug!("[MultiNook] '{stream_id}' changes codec at its new size; restarting it");
        return Ok(Retier::Restart);
    }

    // The new rendition has its own segment URLs; drop the old projection map.
    crate::services::hls_projection::reset(&stream_id);
    MultiNookServer::start_proxy(&stream_id, r.url.clone(), TileProfile::Twitch, None)
        .await
        .map_err(|e| e.to_string())?;
    MultiNookServer::set_promotion(
        &stream_id,
        TilePromotion {
            quality: r.quality.clone(),
            available: r.available.clone(),
            status: Some(r.status.clone()),
            capped: cap,
            codecs,
        },
    )
    .await;
    // The player picked its hls.js mode at construction from the URL's `ll`
    // flag; a mode change needs a fresh player.
    if MultiNookServer::is_low_latency(&stream_id).await != was_low_latency {
        return Ok(Retier::Restart);
    }
    debug!("[MultiNook] '{stream_id}' now plays {} in place", r.quality);
    Ok(Retier::Swapped)
}

/// The small-tile cap from settings when the tile is small, else none.
fn cap_for(state: &State<'_, AppState>, small: Option<bool>) -> Result<Option<u32>, String> {
    if !small.unwrap_or(false) {
        return Ok(None);
    }
    Ok(state.settings.lock().map_err(|e| e.to_string())?.multi_nook_layout.caps_small_tiles())
}

/// The CODECS of the rendition a resolve picked: by its URL, else by its
/// name, else the source tier for "best".
fn variant_codecs(master: &str, url: &str, quality: &str) -> Option<String> {
    let variants = tr::parse_master(master);
    let by_url = variants.iter().find(|v| v.url == url);
    let by_name = || variants.iter().find(|v| v.name.eq_ignore_ascii_case(quality));
    let source = || {
        if !matches!(quality.to_ascii_lowercase().as_str(), "best" | "source") {
            return None;
        }
        variants
            .iter()
            .find(|v| v.group_id.eq_ignore_ascii_case("chunked"))
            .or_else(|| variants.iter().filter(|v| v.height.is_some()).max_by_key(|v| v.height))
    };
    by_url.or_else(by_name).or_else(source).and_then(|v| v.codecs.clone())
}

/// The video codec family of a CODECS attribute ("avc1.64002A,mp4a.40.2" is
/// "avc"). Audio entries are skipped.
fn codec_family(codecs: &str) -> Option<&'static str> {
    codecs.split(',').map(str::trim).find_map(|c| {
        let c = c.to_ascii_lowercase();
        if c.starts_with("avc1") || c.starts_with("avc3") {
            Some("avc")
        } else if c.starts_with("hvc1") || c.starts_with("hev1") {
            Some("hevc")
        } else if c.starts_with("av01") {
            Some("av1")
        } else if c.starts_with("vp09") || c.starts_with("vp9") {
            Some("vp9")
        } else {
            None
        }
    })
}

/// Whether two renditions decode alike. Unknown on either side is not safe.
fn same_codec_family(a: Option<&str>, b: Option<&str>) -> bool {
    match (a.and_then(codec_family), b.and_then(codec_family)) {
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

/// Resolve one Twitch tile at `quality`, through a resolution-owning plugin
/// when one takes it, exactly as the solo player resolves.
async fn resolve_twitch_tile(
    state: &State<'_, AppState>,
    stream_id: &str,
    url: &str,
    quality: &str,
) -> Result<(String, tr::ResolvedLive), String> {
    let stream_timeout = { state.settings.lock().unwrap().streamlink.stream_timeout };

    let channel =
        channel_from_url(url).ok_or_else(|| format!("Unrecognized Twitch URL: {}", url))?;
    let oauth = state.twitch_auth.get_token().await.ok();

    // MultiNook resolves each tile with a SINGLE attempt (retry_delay = 0). Unlike
    // the solo player, a grid tile is expected to be live, so the solo path's
    // retry-until-live loop is wrong here: it would keep an offline channel
    // hammering usher / GQL every `retry_streams` seconds for the full
    // `stream_timeout` budget (60s by default), saturating the network and
    // stalling the OTHER tiles' playback. Failing fast lets an offline tile show
    // its overlay right away; the per-tile Retry button (frontend) covers the
    // rare "channel just went live" case. `stream_timeout` is still passed as
    // the budget but is moot at retry_delay = 0 (single attempt).
    let core =
        tr::resolve_live_resilient(&channel, oauth.as_deref(), quality, 0, stream_timeout).await;

    // Same hand-off as the solo player: a resolution-owning plugin takes the
    // non-entitled tile when installed, addressed by this tile's stream id.
    let r = match crate::commands::streaming::resolve_via_plugin(
        state, stream_id, &channel, quality, &core,
    )
    .await
    {
        Some(plugin_resolved) => plugin_resolved,
        None => core.map_err(|e| e.to_string())?,
    };
    Ok((channel, r))
}

/// Resolve and serve one non-Twitch tile.
///
/// The platform adapter hands back a media-playlist URL and the per-tile relay
/// serves it in its generic profile, exactly as `start_provider_stream` does for
/// the solo player: no SSAI ad detection, no segment projection, no LL-HLS
/// origin, while the platform-agnostic TARGETDURATION retarget stays on.
async fn start_provider_tile(
    stream_id: &str,
    provider: &str,
    url: &str,
    quality: &str,
    cap: Option<u32>,
) -> Result<String, String> {
    // The frontend addresses tiles by URL, so recover the channel from it rather
    // than inventing a second addressing scheme. Note NO lowercasing here: the
    // Twitch helper above folds case because Twitch logins are case-insensitive,
    // and doing that for every platform would destroy case-sensitive ids.
    let channel = provider_channel_from_url(provider, url)
        .ok_or_else(|| format!("Unrecognized {} URL: {}", provider, url))?;

    let source = crate::services::providers::registry()
        .await
        .get_source(provider)
        .ok_or_else(|| format!("{} streams aren't supported in this build yet", provider))?;

    let resolved = source
        .resolve_playback(&stream_id, &channel, quality)
        .await
        .map_err(|e| e.to_string())?;

    match resolved.kind {
        PlaybackKind::Hls => {
            let port = MultiNookServer::start_proxy(
                stream_id,
                resolved.url,
                TileProfile::GenericHls,
                kick_refresher(provider, &channel, quality),
            )
            .await
            .map_err(|e| e.to_string())?;
            MultiNookServer::set_promotion(
                stream_id,
                TilePromotion {
                    quality: resolved.quality.clone(),
                    available: crate::services::providers::hls_master::quality_names(
                        &resolved.qualities,
                    ),
                    // No ad-source badge on a provider stream, so nothing to carry.
                    status: None,
                    capped: cap,
                    codecs: None,
                },
            )
            .await;
            // No `&ll=1`: the LL origin is Twitch-only and is not probed for this
            // profile, so the player must not select its low-latency mode.
            let proxy_url = format!(
                "http://localhost:{}/stream.m3u8?t={}",
                port,
                chrono::Utc::now().timestamp_millis()
            );
            debug!(
                "[MultiNook] '{}' ({}:{}) -> {}",
                stream_id, provider, channel, proxy_url
            );
            Ok(proxy_url)
        }
        // Already localhost HLS produced by the adapter itself, so it is handed
        // to the player untouched: proxying would put one local server in front
        // of another.
        //
        // This is the LIVE path for a YouTube tile, not a defensive branch. It
        // became reachable when youtube_dash was keyed by stream id and the
        // grid's YouTube refusal was lifted; the url already carries this tile's
        // own /s/{stream_id}/ prefix, which is what keeps two YouTube tiles from
        // serving each other's fragments.
        PlaybackKind::LocalHls => Ok(resolved.url),
        other => Err(format!(
            "{} playback kind {:?} is not supported in the grid yet",
            provider, other
        )),
    }
}

/// Kick's master url carries a JWT that expires mid-session, so a tile needs to
/// be able to re-sign it. Same logic as the solo relay's refresher, including
/// the second opinion on "not live": `resign` forces a fresh resolve, which is
/// the request most likely to be refused by Kick's bot defense, and a refused
/// payload has no `stream` object, so it parses identically to a broadcast that
/// genuinely ended. Other providers get None (nothing to re-sign).
fn kick_refresher(provider: &str, channel: &str, quality: &str) -> Option<TileRefresher> {
    if provider != "kick" {
        return None;
    }
    let ch = channel.to_string();
    let q = quality.to_string();
    Some(std::sync::Arc::new(move || {
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
                        match crate::commands::streaming::confirm_kick_liveness(&ch).await {
                            crate::commands::streaming::Liveness::Offline => {
                                log::info!("[MultiNook] Kick '{}' has ended (verified)", ch);
                            }
                            _ => {
                                log::warn!(
                                    "[MultiNook] Kick '{}' re-sign was refused but the channel looks live; will retry on the next refused request",
                                    ch
                                );
                            }
                        }
                    } else {
                        log::warn!("[MultiNook] Kick '{}' re-sign failed: {}", ch, msg);
                    }
                    None
                }
            }
        })
    }))
}

/// Extract a channel from a provider watch URL, preserving case.
/// The channel a provider tile should resolve, recovered from its watch URL.
///
/// Delegates to `watch_urls::classify`, which is the ONE place that knows each
/// platform's URL shapes and is unit-tested against them. This used to be a
/// bespoke "take the first path segment" reader, which is right for Kick
/// (`kick.com/<slug>`) and for a YouTube `@handle`, and silently wrong for every
/// other YouTube shape the app actually produces:
///
///   youtube.com/watch?v=<id>        -> "watch"
///   youtube.com/live/<id>           -> "live"
///   youtube.com/channel/UC.../live  -> "channel"
///
/// The failure was invisible at the call site and surfaced as the RESOLVER
/// complaining that a channel named "watch" isn't live, so a YouTube tile added
/// from Discover showed "Offline or unreachable" while the same stream played
/// fine in the solo player and its chat connected normally.
///
/// Falls back to the first path segment only for a provider `classify` does not
/// know, so adding a platform cannot be broken by this delegation.
fn provider_channel_from_url(provider: &str, url: &str) -> Option<String> {
    if let WatchTarget::Provider { provider: p, channel } =
        crate::services::providers::watch_urls::classify(url)
    {
        if p == provider && !channel.is_empty() {
            return Some(channel);
        }
    }
    let after = url.split("://").nth(1)?;
    let path = after.split_once('/').map(|(_, p)| p)?;
    let seg = path.split(['/', '?', '#']).next()?.trim();
    if seg.is_empty() {
        return None;
    }
    Some(seg.to_string())
}

#[cfg(test)]
mod tile_quality_tests {
    use super::{codec_family, same_codec_family, tile_quality, variant_codecs};

    const MASTER: &str = "#EXTM3U\n\
#EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID=\"chunked\",NAME=\"1080p60 (source)\",AUTOSELECT=YES,DEFAULT=YES\n\
#EXT-X-STREAM-INF:BANDWIDTH=8000000,RESOLUTION=1920x1080,CODECS=\"hvc1.1.2.L123.90,mp4a.40.2\",VIDEO=\"chunked\",FRAME-RATE=60.000\n\
https://cdn.example/source.m3u8\n\
#EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID=\"480p30\",NAME=\"480p\",AUTOSELECT=YES,DEFAULT=YES\n\
#EXT-X-STREAM-INF:BANDWIDTH=1400000,RESOLUTION=852x480,CODECS=\"avc1.4D401F,mp4a.40.2\",VIDEO=\"480p30\",FRAME-RATE=30.000\n\
https://cdn.example/480.m3u8\n";

    #[test]
    fn the_codec_family_ignores_audio_and_profiles() {
        assert_eq!(codec_family("avc1.4D401F,mp4a.40.2"), Some("avc"));
        assert_eq!(codec_family("mp4a.40.2, hev1.1.6.L93.B0"), Some("hevc"));
        assert_eq!(codec_family("av01.0.08M.08"), Some("av1"));
        assert_eq!(codec_family("mp4a.40.2"), None);
    }

    #[test]
    fn a_swap_in_place_needs_one_known_codec_family() {
        assert!(same_codec_family(Some("avc1.64002A"), Some("avc1.4D401F,mp4a.40.2")));
        assert!(!same_codec_family(Some("hvc1.1.2.L123.90"), Some("avc1.4D401F")), "HEVC source to an H.264 transcode restarts");
        assert!(!same_codec_family(None, Some("avc1.4D401F")), "unknown is never safe");
    }

    #[test]
    fn the_served_rendition_is_found_by_url_name_or_source() {
        assert_eq!(variant_codecs(MASTER, "https://cdn.example/480.m3u8", "x").as_deref(), Some("avc1.4D401F,mp4a.40.2"));
        assert_eq!(variant_codecs(MASTER, "https://proxy/elsewhere", "480p").as_deref(), Some("avc1.4D401F,mp4a.40.2"));
        assert_eq!(variant_codecs(MASTER, "https://proxy/elsewhere", "best").as_deref(), Some("hvc1.1.2.L123.90,mp4a.40.2"));
        assert_eq!(variant_codecs(MASTER, "https://proxy/elsewhere", "720p60"), None);
    }

    #[test]
    fn a_full_size_tile_keeps_its_own_quality() {
        assert_eq!(tile_quality("best", None), "best");
        assert_eq!(tile_quality("720p60", None), "720p60");
    }

    #[test]
    fn a_small_tile_is_held_to_the_cap() {
        assert_eq!(tile_quality("best", Some(480)), "480p");
        assert_eq!(tile_quality("source", Some(480)), "480p");
        assert_eq!(tile_quality("1080p60", Some(720)), "720p");
        assert_eq!(tile_quality("", Some(360)), "360p");
    }

    #[test]
    fn a_pick_already_under_the_cap_stays() {
        assert_eq!(tile_quality("360p30", Some(480)), "360p30");
        assert_eq!(tile_quality("480p", Some(480)), "480p");
        assert_eq!(tile_quality("worst", Some(480)), "worst");
        assert_eq!(tile_quality("audio_only", Some(480)), "audio_only");
    }
}

#[cfg(test)]
mod provider_url_tests {
    use super::provider_channel_from_url;

    #[test]
    fn youtube_tiles_resolve_the_video_not_the_url_keyword() {
        // The regression: a Discover row's watch URL used to yield "watch".
        assert_eq!(
            provider_channel_from_url("youtube", "https://www.youtube.com/watch?v=3C1mkvtGiJw"),
            Some("3C1mkvtGiJw".to_string())
        );
        assert_eq!(
            provider_channel_from_url("youtube", "https://www.youtube.com/live/jfKfPfyJRdk"),
            Some("jfKfPfyJRdk".to_string())
        );
        // A favourite is keyed by UC id, so this shape is on the tile path too.
        assert_eq!(
            provider_channel_from_url(
                "youtube",
                "https://www.youtube.com/channel/UCXuqSBlHAE6Xw-yeJA0Tunw/live"
            ),
            Some("UCXuqSBlHAE6Xw-yeJA0Tunw".to_string())
        );
        assert_eq!(
            provider_channel_from_url("youtube", "https://www.youtube.com/@somechannel/live"),
            Some("@somechannel".to_string())
        );
    }

    #[test]
    fn other_platforms_keep_their_existing_readings() {
        assert_eq!(
            provider_channel_from_url("kick", "https://kick.com/xqc"),
            Some("xqc".to_string())
        );
        assert_eq!(
            provider_channel_from_url("tiktok", "https://www.tiktok.com/@someone/live"),
            Some("someone".to_string())
        );
        // A provider `classify` does not know still falls back rather than failing.
        assert_eq!(
            provider_channel_from_url("rumble", "https://rumble.com/c/somechannel"),
            Some("c".to_string())
        );
    }
}

/// Stop a specific stream in multi-stream mode
#[tauri::command]
pub async fn stop_multi_nook(stream_id: String) -> Result<(), String> {
    debug!("[MultiNook] Stopping stream: {}", stream_id);
    // No-op unless this tile was a YouTube one holding a DASH relay.
    crate::services::youtube_dash::stop(&stream_id).await;
    // Likewise for a TikTok tile's relay session.
    crate::services::tiktok_relay::stop(&stream_id);
    MultiNookServer::stop_instance(&stream_id)
        .await
        .map_err(|e| e.to_string())
}

/// Stop all streams in multi-stream mode (cleanup)
#[tauri::command]
pub async fn stop_all_multi_nooks() -> Result<(), String> {
    debug!("[MultiNook] Stopping all multi-stream instances");
    // Everything EXCEPT the solo player's. Leaving the grid must not kill a
    // stream playing behind it: the store keeps streamUrl across the toggle and
    // the solo player remounts on it.
    crate::services::youtube_dash::stop_all_except(crate::services::stream_server::SOLO_STREAM_ID)
        .await;
    crate::services::tiktok_relay::stop_all_except(
        crate::services::stream_server::SOLO_STREAM_ID,
    );
    MultiNookServer::stop_all().await.map_err(|e| e.to_string())
}

/// Hand ONE grid tile to the solo player and tear the rest of the grid down.
///
/// The point is that nothing is resolved again. The tile's relay is already
/// serving an upstream media playlist it fetched, authorized and (for Kick)
/// signed; `promotion_target` reads that live url back out and the solo relay
/// is pointed straight at it. No usher call, no GQL, no token mint, no
/// entitlement probe: the expensive half of `start_stream` is skipped outright,
/// which is the whole difference between this and closing the grid and starting
/// the channel again.
///
/// Order is deliberate. The solo relay comes up FIRST and only then is the grid
/// stopped, so a failure anywhere in here leaves the grid exactly as it was
/// rather than half torn down with nothing playing.
///
/// Returns `None` when the tile has nothing to hand over: it never resolved, it
/// was closed while this was in flight, or it is a YouTube tile, whose adapter
/// serves its own local relay and never registers with `MultiNookServer`. The
/// caller starts that channel the ordinary way instead.
#[tauri::command]
pub async fn promote_multi_nook_tile(
    stream_id: String,
) -> Result<Option<StreamStartResult>, String> {
    let Some((upstream, promotion)) = MultiNookServer::promotion_target(&stream_id).await else {
        debug!(
            "[MultiNook] '{}' has no promotion target; caller falls back to a normal start",
            stream_id
        );
        return Ok(None);
    };
    // A small tile's stream was resolved under the small-tile cap; the solo
    // player resolves afresh at the viewer's own quality instead.
    if promotion.capped.is_some() {
        debug!("[MultiNook] '{}' plays under the small-tile cap; promoting with a fresh resolve", stream_id);
        return Ok(None);
    }

    // The badge reads from here, exactly as the solo path sets it. A provider
    // tile carries no status and leaves whatever the last Twitch stream wrote,
    // which is the same thing `start_provider_stream` does.
    let status = promotion.status.clone();
    if let Some(ref s) = status {
        crate::services::auth_proxy::set_status(s.clone());
    }

    // The relay re-probes the low-latency origin against this upstream, so the
    // solo player picks its hls.js mode from a settled answer just as it does on
    // a cold start. Probing is skipped for a non-Twitch upstream inside the
    // relay itself (the origin only speaks Twitch's playlist shape).
    let port = StreamServer::start_proxy_server(upstream)
        .await
        .map_err(|e| e.to_string())?;

    // Serving now, so the grid can go. Everything EXCEPT the tile we just
    // promoted still has a relay to stop.
    crate::services::youtube_dash::stop_all_except(crate::services::stream_server::SOLO_STREAM_ID)
        .await;
    crate::services::tiktok_relay::stop_all_except(
        crate::services::stream_server::SOLO_STREAM_ID,
    );
    MultiNookServer::stop_all().await.map_err(|e| e.to_string())?;

    // Register the solo session only once the relay is serving, matching
    // `start_live`: the plugin protocol's "solo" stream id must always address a
    // live relay.
    if let Some(ref s) = status {
        crate::services::stream_server::set_solo_session(Some(s.channel.clone()));
    }

    debug!(
        "[MultiNook] '{}' promoted to the solo player on port {} without resolving again",
        stream_id, port
    );

    Ok(Some(StreamStartResult {
        url: crate::commands::streaming::local_player_url(port),
        quality: promotion.quality,
        mode: status.as_ref().map(|s| s.mode.clone()),
        entitled: status.as_ref().map(|s| s.entitled).unwrap_or(false),
        proxy_region: status.and_then(|s| s.proxy_region),
        available: promotion.available,
        clip_source: None,
        vod: None,
        kind: None,
    }))
}

/// Get a list of active multi-stream IDs
#[tauri::command]
pub async fn get_active_multi_nooks() -> Result<Vec<String>, String> {
    Ok(MultiNookServer::get_active_streams().await)
}

/// Declare the grid's Twitch channels, by user id, so a raid out of any of them
/// reaches its tile as `multi-nook://raid`. An empty list closes the socket.
#[tauri::command]
pub fn set_multi_nook_raid_channels(app: AppHandle, channel_ids: Vec<String>) {
    crate::services::multi_nook_raids::set_channels(&app, channel_ids);
}

/// The grid's Twitch channels, for the live title and category Rust polls
/// for them (services::multi_nook_meta). An empty list ends the poll.
#[tauri::command]
pub fn set_multi_nook_meta_channels(app: AppHandle, logins: Vec<String>) {
    crate::services::multi_nook_meta::set_channels(&app, logins);
}

/// One tile's volume and mute as the viewer changes them, written into the
/// saved grid in place. The debounced settings writer persists it; no other
/// window shows tile volume, so nothing is broadcast. Saving the whole grid
/// through `patch_settings` instead ran a full settings round trip and made
/// every open window re-read its settings, once per scroll notch.
/// The delay a Resync lines MultiNook tiles up at (seconds behind live), or
/// None when no sync is held. Tile playlists then reach back that far, so
/// hls.js never snaps a held tile forward to live.
#[tauri::command]
pub async fn set_multi_nook_sync_delay(seconds: Option<f64>) -> Result<(), String> {
    let delay = seconds.filter(|s| s.is_finite() && *s > 0.0 && *s <= 60.0);
    MultiNookServer::set_sync_delay(delay).await;
    Ok(())
}

#[tauri::command]
pub fn set_multi_nook_slot_audio(
    slot_id: String,
    volume: f32,
    muted: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut settings = state.settings.lock().map_err(|e| e.to_string())?;
    let Some(slot) = settings.multi_nook_slots.iter_mut().find(|s| s.id == slot_id) else {
        return Ok(());
    };
    let volume = if volume.is_finite() { volume.clamp(0.0, 1.0) } else { slot.volume };
    if slot.volume == volume && slot.muted == muted {
        return Ok(());
    }
    slot.volume = volume;
    slot.muted = muted;
    crate::commands::settings::write_settings_to_disk(&settings)
}

/// The quality menu for one tile, so every tile in the grid can offer its own
/// selector without resolving the channel again.
///
/// A tile's relay keeps the menu its resolve discovered, which answers every
/// Twitch and Kick tile for free. A YouTube tile serves its own relay and never
/// registers, so it asks its platform adapter instead, whose parsed master is
/// already cached from starting the tile. A Twitch tile still resolving answers
/// empty rather than paying for a second resolve.
#[tauri::command]
pub async fn get_multi_nook_tile_qualities(
    stream_id: String,
    url: String,
    provider: Option<String>,
) -> Result<Vec<String>, String> {
    if let Some(qualities) = MultiNookServer::tile_qualities(&stream_id).await {
        return Ok(qualities);
    }
    let provider = provider.unwrap_or_else(|| "twitch".to_string());
    if provider == "twitch" {
        return Ok(Vec::new());
    }
    let Some(channel) = provider_channel_from_url(&provider, &url) else {
        return Ok(Vec::new());
    };
    let Some(source) = crate::services::providers::registry()
        .await
        .get_source(&provider)
    else {
        return Ok(Vec::new());
    };
    source
        .qualities(&channel)
        .await
        .map(|q| crate::services::providers::hls_master::quality_names(&q))
        .map_err(|e| e.to_string())
}
