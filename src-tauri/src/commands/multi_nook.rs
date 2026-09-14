use crate::models::settings::AppState;
use crate::services::multi_nook_server::{MultiNookServer, TileProfile, TileRefresher};
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
    state: State<'_, AppState>,
) -> Result<String, String> {
    let provider = provider.unwrap_or_else(|| "twitch".to_string());
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
        return start_provider_tile(&stream_id, &provider, &url, &quality).await;
    }

    let stream_timeout = { state.settings.lock().unwrap().streamlink.stream_timeout };

    let channel =
        channel_from_url(&url).ok_or_else(|| format!("Unrecognized Twitch URL: {}", url))?;
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
        tr::resolve_live_resilient(&channel, oauth.as_deref(), &quality, 0, stream_timeout).await;

    // Same hand-off as the solo player: a resolution-owning plugin takes the
    // non-entitled tile when installed, addressed by this tile's stream id.
    let r = match crate::commands::streaming::resolve_via_plugin(
        &state, &stream_id, &channel, &quality, &core,
    )
    .await
    {
        Some(plugin_resolved) => plugin_resolved,
        None => core.map_err(|e| e.to_string())?,
    };

    let port = MultiNookServer::start_proxy(&stream_id, r.url, TileProfile::Twitch, None)
        .await
        .map_err(|e| e.to_string())?;

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
    MultiNookServer::stop_all().await.map_err(|e| e.to_string())
}

/// Get a list of active multi-stream IDs
#[tauri::command]
pub async fn get_active_multi_nooks() -> Result<Vec<String>, String> {
    Ok(MultiNookServer::get_active_streams().await)
}
