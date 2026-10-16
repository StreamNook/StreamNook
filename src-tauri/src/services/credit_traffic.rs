//! Player-shaped traffic that lets drops credit the stream being watched.
//!
//! Since about 2026-10-07 Twitch credits drop minutes only while the viewer's
//! own playback requests the stream's media segments; minute-watched alone
//! credits nothing (channel points are unaffected). A stream served through an
//! ad-free relay or a resolution plugin pulls its playlist and segments on the
//! relay's anonymous token, so none of that traffic is the viewer's. While the
//! main heartbeat has a playing target served that way, this polls the
//! lowest-quality video playlist signed for the drops account at the playlist's
//! own segment interval and sends a HEAD for each new segment, the way the
//! Autopilot plugin does for mining. No audio or video is downloaded.

use anyhow::{anyhow, Result};
use reqwest::{Client, Url};
use serde_json::{json, Value};
use std::collections::{HashSet, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::services::drops_auth_service::DropsAuthService;

/// Playback tokens expire, so the playlist is re-resolved this often.
const RESOLVE_EVERY: Duration = Duration::from_secs(10 * 60);
/// Segment URIs remembered, so a segment is HEADed once.
const SEEN_CAP: usize = 256;

const ACCESS_TOKEN_QUERY: &str = "query PlaybackAccessToken($login: String!) { streamPlaybackAccessToken(channelName: $login, params: { platform: \"web\", playerBackend: \"mediaplayer\", playerType: \"site\" }) { value signature } }";

/// Serving modes whose playback already rides the viewer's own token, so its
/// segment requests count for them.
const VIEWER_BOUND_MODES: [&str; 3] = ["turbo", "subscribed", "auth-only"];

static FOLLOWING: Mutex<Option<(String, tauri::async_runtime::JoinHandle<()>)>> = Mutex::new(None);

/// Whether the main stream for `login` needs this traffic: it was served by a
/// relay or a plugin (or its serving mode is unknown).
pub fn needed_for(login: &str) -> bool {
    crate::services::auth_proxy::get_status(&login.to_lowercase())
        .map(|status| !VIEWER_BOUND_MODES.contains(&status.mode.as_str()))
        .unwrap_or(true)
}

/// Follows `login`, or stops for `None`. A no-op while already following it.
pub fn follow(login: Option<&str>) {
    let mut current = FOLLOWING.lock().unwrap_or_else(|e| e.into_inner());
    if let (Some(login), Some((following, _))) = (login, current.as_ref()) {
        if following == login {
            return;
        }
    }
    if let Some((was, task)) = current.take() {
        task.abort();
        log::info!("[CreditTraffic] stopped for {was}");
    }
    let Some(login) = login else { return };
    let owned = login.to_string();
    log::info!("[CreditTraffic] following {owned} (its playback is not the viewer's own)");
    let task = tauri::async_runtime::spawn(run(owned.clone()));
    *current = Some((owned, task));
}

async fn run(login: String) {
    let client = crate::services::http::client().clone();
    let mut failures: u32 = 0;
    loop {
        match DropsAuthService::get_token().await {
            // No drops sign-in yet: nothing can count; look again in a minute.
            Err(_) => {
                tokio::time::sleep(Duration::from_secs(60)).await;
                continue;
            }
            Ok(token) => match variant_url(&client, &token, &login).await {
                Ok(url) => {
                    failures = 0;
                    match pump(&client, &url, RESOLVE_EVERY).await {
                        Ok(heads) => log::debug!("[CreditTraffic] {login}: {heads} segments in this window"),
                        Err(e) => {
                            log::info!("[CreditTraffic] {login} interrupted: {e}");
                            failures += 1;
                        }
                    }
                }
                Err(e) => {
                    log::info!("[CreditTraffic] {login} unavailable: {e}");
                    failures += 1;
                }
            },
        }
        if failures > 0 {
            let wait = 15u64.saturating_mul(1 << failures.min(4)).min(300);
            tokio::time::sleep(Duration::from_secs(wait)).await;
        }
    }
}

/// The lowest-bandwidth video rendition of a live channel, signed for the
/// drops account so the traffic counts for that viewer.
async fn variant_url(client: &Client, token: &str, login: &str) -> Result<Url> {
    let body: Value = client
        .post("https://gql.twitch.tv/gql")
        .header("Client-Id", DropsAuthService::client_id())
        .header("Authorization", format!("OAuth {token}"))
        .json(&json!({ "query": ACCESS_TOKEN_QUERY, "variables": { "login": login } }))
        .timeout(Duration::from_secs(10))
        .send()
        .await?
        .json()
        .await?;
    let pat = &body["data"]["streamPlaybackAccessToken"];
    let (Some(value), Some(sig)) = (pat["value"].as_str(), pat["signature"].as_str()) else {
        return Err(anyhow!("no playback token: {}", body["errors"]));
    };
    let p = (uuid::Uuid::new_v4().as_u128() % 9_999_999).to_string();
    let master_url = Url::parse_with_params(
        &format!("https://usher.ttvnw.net/api/channel/hls/{login}.m3u8"),
        &[
            ("platform", "web"),
            ("player_type", "site"),
            ("allow_source", "true"),
            ("playlist_include_framerate", "true"),
            ("p", p.as_str()),
            ("sig", sig),
            ("token", value),
        ],
    )?;
    let resp = client.get(master_url).timeout(Duration::from_secs(10)).send().await?;
    if !resp.status().is_success() {
        return Err(anyhow!("usher HTTP {}", resp.status()));
    }
    let master = resp.text().await?;
    let variant = lowest_video_variant(&master).ok_or_else(|| anyhow!("no video rendition"))?;
    Ok(Url::parse(&variant)?)
}

/// The lowest-bandwidth rendition that carries video (audio-only is skipped:
/// a real player never picks it for a visible stream).
fn lowest_video_variant(master: &str) -> Option<String> {
    let lines: Vec<&str> = master.lines().collect();
    let mut best: Option<(u64, String)> = None;
    for (i, line) in lines.iter().enumerate() {
        let Some(attrs) = line.strip_prefix("#EXT-X-STREAM-INF:") else { continue };
        if attrs.contains("audio_only") {
            continue;
        }
        let bandwidth = attrs
            .split(',')
            .find_map(|a| a.strip_prefix("BANDWIDTH="))
            .and_then(|v| v.parse().ok())
            .unwrap_or(u64::MAX);
        let Some(url) = lines.get(i + 1).map(|l| l.trim()).filter(|l| !l.is_empty() && !l.starts_with('#')) else {
            continue;
        };
        if best.as_ref().is_none_or(|(b, _)| bandwidth < *b) {
            best = Some((bandwidth, url.to_string()));
        }
    }
    best.map(|(_, url)| url)
}

/// The media segments a playlist lists, resolved against its own URL, and its
/// target duration in seconds.
fn parse_media_playlist(base: &Url, text: &str) -> (Vec<Url>, u64) {
    let target = text
        .lines()
        .find_map(|l| l.strip_prefix("#EXT-X-TARGETDURATION:"))
        .and_then(|v| v.trim().parse::<f64>().ok())
        .map_or(2, |s| s.ceil() as u64)
        .clamp(1, 10);
    let segments = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| base.join(l).ok())
        .collect();
    (segments, target)
}

/// Polls one media playlist until `for_how_long` passes, HEADing each new
/// segment. Returns early with an error when the playlist stops answering.
async fn pump(client: &Client, url: &Url, for_how_long: Duration) -> Result<u32> {
    let started = Instant::now();
    let mut seen: HashSet<String> = HashSet::new();
    let mut order: VecDeque<String> = VecDeque::new();
    let mut heads = 0;
    while started.elapsed() < for_how_long {
        let resp = client.get(url.clone()).timeout(Duration::from_secs(10)).send().await?;
        if !resp.status().is_success() {
            return Err(anyhow!("playlist HTTP {}", resp.status()));
        }
        let text = resp.text().await?;
        let (segments, target) = parse_media_playlist(url, &text);
        for seg in segments {
            let key = seg.to_string();
            if !seen.insert(key.clone()) {
                continue;
            }
            order.push_back(key);
            if order.len() > SEEN_CAP {
                if let Some(old) = order.pop_front() {
                    seen.remove(&old);
                }
            }
            if client.head(seg).timeout(Duration::from_secs(10)).send().await.is_ok() {
                heads += 1;
            }
        }
        tokio::time::sleep(Duration::from_secs(target)).await;
    }
    Ok(heads)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lowest_video_rendition_is_picked_and_audio_only_skipped() {
        let master = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=8000000,VIDEO=\"chunked\"\nhttps://a/source.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=160000,VIDEO=\"audio_only\"\nhttps://a/audio.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=230000,VIDEO=\"160p30\"\nhttps://a/160.m3u8\n";
        assert_eq!(lowest_video_variant(master).as_deref(), Some("https://a/160.m3u8"));
    }

    #[test]
    fn a_media_playlist_gives_its_segments_and_target() {
        let base = Url::parse("https://cdn/v1/playlist/x.m3u8").unwrap();
        let (segments, target) = parse_media_playlist(&base, "#EXTM3U\n#EXT-X-TARGETDURATION:6\n#EXTINF:2.0,\nseg1.ts\n#EXTINF:2.0,\nhttps://cdn/seg2.ts\n");
        assert_eq!(target, 6);
        assert_eq!(segments[0].as_str(), "https://cdn/v1/playlist/seg1.ts");
        assert_eq!(segments.len(), 2);
    }

    #[test]
    fn viewer_bound_playback_needs_no_extra_traffic() {
        crate::services::auth_proxy::set_status(crate::services::auth_proxy::PlaybackStatus {
            channel: "creditmode_sub".into(),
            mode: "subscribed".into(),
            entitled: true,
            proxy_base: None,
            proxy_region: None,
        });
        assert!(!needed_for("creditmode_sub"));
        crate::services::auth_proxy::set_status(crate::services::auth_proxy::PlaybackStatus {
            channel: "creditmode_relay".into(),
            mode: "plugin".into(),
            entitled: false,
            proxy_base: Some("relay".into()),
            proxy_region: None,
        });
        assert!(needed_for("creditmode_relay"));
        assert!(needed_for("creditmode_unknown"));
    }
}
