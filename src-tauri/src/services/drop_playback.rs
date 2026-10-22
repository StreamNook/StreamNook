//! Player-shaped traffic for the channel on screen. Twitch counts drop
//! progress only while the stream's media segments are requested on a
//! playback token signed for the viewer; minute-watched events alone credit
//! nothing. What the app's own player fetches does not always qualify: an
//! ad-free relay serves the video from elsewhere, and the token a stream was
//! resolved with need not be the viewer's. So while the watch heartbeat has a
//! channel that is playing, this polls that channel's lowest-quality video
//! playlist on a token signed with the Drops credential and sends a HEAD for
//! each new segment. No audio or video is downloaded.

use anyhow::{anyhow, Result};
use log::debug;
use reqwest::{Client, Url};
use serde_json::{json, Value};
use std::collections::{HashSet, VecDeque};
use std::time::{Duration, Instant};

const CLIENT_ID: &str = env!("TWITCH_ANDROID_CLIENT_ID");

/// Playback tokens expire, so the playlist is re-resolved this often.
const RESOLVE_EVERY: Duration = Duration::from_secs(10 * 60);
/// Segment URIs remembered, so a segment is HEADed once.
const SEEN_CAP: usize = 256;

const ACCESS_TOKEN_QUERY: &str = "query PlaybackAccessToken($login: String!) { streamPlaybackAccessToken(channelName: $login, params: { platform: \"web\", playerBackend: \"mediaplayer\", playerType: \"site\" }) { value signature } }";

/// What the loop should be following right now: the login on screen while it
/// plays, or `None`.
pub trait Follow: Send + Sync + 'static {
    fn following(&self) -> impl std::future::Future<Output = Option<String>> + Send;
    /// Resolves when what `following` answers may have changed.
    fn changed(&self) -> impl std::future::Future<Output = ()> + Send;
}

/// Runs for the life of the app. Follows whatever `follow` names, switching
/// as soon as a channel change, pause or close is reported (no idle polling).
pub async fn run<F: Follow>(client: Client, follow: std::sync::Arc<F>) {
    let mut failures: u32 = 0;
    let mut failed_on: Option<String> = None;
    loop {
        let Some(login) = follow.following().await else {
            failures = 0;
            follow.changed().await;
            continue;
        };
        if failed_on.as_deref() != Some(login.as_str()) {
            failures = 0;
        }
        let Ok(token) = crate::services::drops_auth_service::DropsAuthService::get_token().await else {
            // No Drops credential: nothing could be credited anyway.
            wait_while_following(&*follow, &login, Duration::from_secs(30)).await;
            continue;
        };
        let outcome = match variant_url(&client, &token, &login).await {
            Ok(url) => {
                debug!("[DropPlayback] following {login}");
                pump(&client, &url, &*follow, &login).await
            }
            Err(e) => Err(e),
        };
        match outcome {
            Ok(heads) => {
                debug!("[DropPlayback] {login}: {heads} segments");
                failures = 0;
            }
            Err(e) => {
                debug!("[DropPlayback] {login} unavailable: {e}");
                failures += 1;
                failed_on = Some(login.clone());
                // Offline channels and refusals back off, up to five minutes.
                let wait = 15u64.saturating_mul(1 << failures.min(4)).min(300);
                wait_while_following(&*follow, &login, Duration::from_secs(wait)).await;
            }
        }
    }
}

/// Sleeps up to `max`, waking early once `follow` stops naming `login`.
async fn wait_while_following<F: Follow>(follow: &F, login: &str, max: Duration) {
    let until = Instant::now() + max;
    while Instant::now() < until {
        let _ = tokio::time::timeout(until.saturating_duration_since(Instant::now()), follow.changed()).await;
        if follow.following().await.as_deref() != Some(login) {
            return;
        }
    }
}

/// The lowest-bandwidth video rendition of a live channel, signed for the
/// Drops credential's viewer so the traffic counts for that account.
pub async fn variant_url(client: &Client, token: &str, login: &str) -> Result<Url> {
    let body: Value = client
        .post("https://gql.twitch.tv/gql")
        .header("Client-Id", CLIENT_ID)
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
pub fn lowest_video_variant(master: &str) -> Option<String> {
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

/// The media segments a playlist lists, resolved against its own URL, and
/// its target duration in seconds.
pub fn parse_media_playlist(base: &Url, text: &str) -> (Vec<Url>, u64) {
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

/// Polls one media playlist, HEADing each new segment, until the token is
/// due for renewal or `follow` stops naming `login`. Errors when the
/// playlist stops answering.
async fn pump<F: Follow>(client: &Client, url: &Url, follow: &F, login: &str) -> Result<u32> {
    let started = Instant::now();
    let mut seen: HashSet<String> = HashSet::new();
    let mut order: VecDeque<String> = VecDeque::new();
    let mut heads = 0;
    while started.elapsed() < RESOLVE_EVERY {
        if follow.following().await.as_deref() != Some(login) {
            break;
        }
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

    const MASTER: &str = "#EXTM3U\n\
#EXT-X-STREAM-INF:BANDWIDTH=6000000,RESOLUTION=1920x1080,VIDEO=\"chunked\"\nhttps://a.example/chunked.m3u8\n\
#EXT-X-STREAM-INF:BANDWIDTH=160000,VIDEO=\"audio_only\"\nhttps://a.example/audio.m3u8\n\
#EXT-X-STREAM-INF:BANDWIDTH=230000,RESOLUTION=284x160,VIDEO=\"160p30\"\nhttps://a.example/160p.m3u8\n\
#EXT-X-STREAM-INF:BANDWIDTH=630000,RESOLUTION=640x360,VIDEO=\"360p30\"\nhttps://a.example/360p.m3u8\n";

    #[test]
    fn picks_the_lowest_video_rendition_not_audio_only() {
        assert_eq!(lowest_video_variant(MASTER).as_deref(), Some("https://a.example/160p.m3u8"));
    }

    #[test]
    fn media_playlist_segments_resolve_and_skip_tags() {
        let base = Url::parse("https://b.example/v1/playlist/x.m3u8").unwrap();
        let text = "#EXTM3U\n#EXT-X-TARGETDURATION:6\n#EXT-X-TWITCH-PREFETCH:https://b.example/p.ts\n\
#EXTINF:2.000,live\nhttps://b.example/seg1.ts\n#EXTINF:2.000,live\nseg2.ts\n";
        let (segs, target) = parse_media_playlist(&base, text);
        assert_eq!(target, 6);
        assert_eq!(
            segs.iter().map(Url::as_str).collect::<Vec<_>>(),
            vec!["https://b.example/seg1.ts", "https://b.example/v1/playlist/seg2.ts"]
        );
    }

    #[test]
    fn missing_target_duration_polls_every_two_seconds() {
        let base = Url::parse("https://b.example/x.m3u8").unwrap();
        assert_eq!(parse_media_playlist(&base, "#EXTM3U\n").1, 2);
    }
}
