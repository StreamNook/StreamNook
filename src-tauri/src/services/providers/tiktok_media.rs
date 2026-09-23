//! TikTok's browse + watch adapter (`StreamSource`), sibling of the read-only
//! chat adapter in `tiktok.rs`.
//!
//! Resolution is two hops, both anonymous: a profile scrape gives the room id
//! (non-empty only while the creator is live), and `room/info` gives the
//! rendition ladder. Everything the player is handed then comes from
//! `tiktok_relay`, never straight from TikTok's CDN, which is what makes solo
//! and grid playback the same path.
//!
//! Playback always reports `PlaybackKind::LocalHls`. See `tiktok_relay` for why.

use crate::models::provider_stream::{CategoryPage, ProviderStream, StreamPage};
use crate::services::providers::key::make_key;
use crate::services::providers::source::{
    PlaybackKind, PlaybackQuality, ResolvedPlayback, SourceCaps, StreamSource,
};
use crate::services::tiktok_relay;
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tiktok_live::http::api::{fetch_room_info, FetchParams};
use tiktok_live::http::sigi::scrape_profile;
use tiktok_live::http::ttwid::fetch_ttwid;
use tiktok_live::structs::events::TikTokRendition;

/// A device token is good for far longer than this; refetching it costs a full
/// page GET, which the chat adapter currently pays on every metadata refresh.
const TTWID_TTL: Duration = Duration::from_secs(30 * 60);
/// A room id is stable for one broadcast.
const ROOM_TTL: Duration = Duration::from_secs(60);
/// Deliberately short. A creator who just went live must not be invisible for a
/// minute because a poll happened to ask one second too early.
const ROOM_OFFLINE_TTL: Duration = Duration::from_secs(15);
/// Long enough that a quality switch costs no network, short enough that a
/// rotated CDN path is picked up without a restart.
const RENDITION_TTL: Duration = Duration::from_secs(45);
/// Liveness is one page fetch per channel, so a sweep is capped the way the
/// other per-channel platform's is. The remainder is UNCHECKED, not offline.
const MAX_PER_SWEEP: usize = 25;
/// Spacing inside a sweep, so a long follow list is not a burst.
const SWEEP_SPACING: Duration = Duration::from_millis(400);
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

static TTWID: Lazy<Mutex<Option<(Instant, String)>>> = Lazy::new(|| Mutex::new(None));
/// handle -> room id, `None` meaning "checked, not live".
static ROOMS: Lazy<Mutex<HashMap<String, (Instant, Option<String>)>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
/// Keyed by ROOM ID rather than handle: when a broadcast ends and the creator
/// starts a new one the id changes, so a dead ladder can never be replayed.
static RENDITIONS: Lazy<Mutex<HashMap<String, (Instant, Vec<TikTokRendition>)>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

pub struct TikTokSource;

impl TikTokSource {
    pub fn new() -> Self {
        Self
    }
}

impl Default for TikTokSource {
    fn default() -> Self {
        Self::new()
    }
}

fn params<'a>() -> FetchParams<'a> {
    FetchParams {
        timeout: HTTP_TIMEOUT,
        ..Default::default()
    }
}

/// Strip the shapes a handle arrives in: `tiktok:name`, `tiktok/name`, `@name`.
fn clean_handle(channel: &str) -> String {
    let c = channel.trim();
    let c = c
        .strip_prefix("tiktok:")
        .or_else(|| c.strip_prefix("tiktok/"))
        .unwrap_or(c);
    c.trim().trim_start_matches('@').to_lowercase()
}

async fn ttwid() -> Result<String> {
    if let Ok(guard) = TTWID.lock() {
        if let Some((at, t)) = guard.as_ref() {
            if at.elapsed() < TTWID_TTL {
                return Ok(t.clone());
            }
        }
    }
    let fresh = fetch_ttwid(HTTP_TIMEOUT, None, None)
        .await
        .map_err(|e| anyhow!("TikTok device token failed: {}", e))?;
    if let Ok(mut guard) = TTWID.lock() {
        *guard = Some((Instant::now(), fresh.clone()));
    }
    Ok(fresh)
}

/// Record that `handle` is live in `room_id`, learned from somewhere other than
/// a profile lookup (a directory row). A click on that row then resolves
/// straight from the room, skipping the page fetch that would only find it again.
pub fn note_live_room(handle: &str, room_id: &str) {
    let h = clean_handle(handle);
    if h.is_empty() || room_id.is_empty() {
        return;
    }
    if let Ok(mut cache) = ROOMS.lock() {
        cache.insert(h, (Instant::now(), Some(room_id.to_string())));
    }
}

/// Forget everything cached for a handle. Called when a broadcast is observed
/// to have ended, so the next resolve starts clean rather than replaying a
/// ladder whose URLs are already dead.
pub fn invalidate(handle: &str) {
    let h = clean_handle(handle);
    let room = ROOMS
        .lock()
        .ok()
        .and_then(|mut m| m.remove(&h))
        .and_then(|(_, r)| r);
    if let (Some(room), Ok(mut r)) = (room, RENDITIONS.lock()) {
        r.remove(&room);
    }
}

/// The live room for a handle, or `None` when the creator is not live.
///
/// A scrape answers this AND the identity the metadata surfaces need, which is
/// why it is preferred over the narrower room-id endpoint: one request serves
/// liveness, display name, avatar and follower count.
async fn room_id_for(handle: &str) -> Result<Option<String>> {
    if let Ok(cache) = ROOMS.lock() {
        if let Some((at, room)) = cache.get(handle) {
            let ttl = if room.is_some() {
                ROOM_TTL
            } else {
                ROOM_OFFLINE_TTL
            };
            if at.elapsed() < ttl {
                return Ok(room.clone());
            }
        }
    }
    let token = ttwid().await?;
    let profile = scrape_profile(handle, &token, HTTP_TIMEOUT, None, None, None)
        .await
        .map_err(|e| anyhow!("{}", friendly(&e, handle)))?;
    let room = (!profile.room_id.is_empty()).then(|| profile.room_id.clone());
    if let Ok(mut cache) = ROOMS.lock() {
        cache.insert(handle.to_string(), (Instant::now(), room.clone()));
    }
    Ok(room)
}

async fn renditions_for(room_id: &str, handle: &str) -> Result<Vec<TikTokRendition>> {
    use tiktok_live::errors::TikTokLiveError as E;
    if let Ok(cache) = RENDITIONS.lock() {
        if let Some((at, r)) = cache.get(room_id) {
            if at.elapsed() < RENDITION_TTL {
                return Ok(r.clone());
            }
        }
    }
    // Anonymous first, always. The signed-in session is sent only when TikTok
    // has refused THIS room without it, so an account is never attached to a
    // request that did not need one (see `tiktok_auth_service`).
    let info = match fetch_room_info(room_id, params()).await {
        Ok(info) => info,
        Err(E::AgeRestricted(_)) => {
            let Some(cookies) = crate::services::tiktok_auth_service::cookie_header() else {
                return Err(anyhow!(
                    "@{}'s LIVE is age restricted. Sign in to TikTok in Settings, Accounts to watch it",
                    handle
                ));
            };
            let signed = FetchParams {
                cookies: Some(&cookies),
                ..params()
            };
            match fetch_room_info(room_id, signed).await {
                Ok(info) => info,
                Err(E::AgeRestricted(_)) => {
                    return Err(anyhow!(
                        "@{}'s LIVE is age restricted, and your TikTok account can't watch it",
                        handle
                    ))
                }
                Err(e) => return Err(anyhow!("{}", friendly(&e, handle))),
            }
        }
        Err(e) => return Err(anyhow!("{}", friendly(&e, handle))),
    };
    let renditions = info
        .stream_url
        .map(|s| s.renditions)
        .filter(|r| !r.is_empty())
        .ok_or_else(|| anyhow!("this LIVE published no playable rendition"))?;
    if let Ok(mut cache) = RENDITIONS.lock() {
        cache.insert(room_id.to_string(), (Instant::now(), renditions.clone()));
    }
    Ok(renditions)
}

/// Turn a vendored error into something worth showing a person.
fn friendly(e: &tiktok_live::errors::TikTokLiveError, handle: &str) -> String {
    use tiktok_live::errors::TikTokLiveError as E;
    match e {
        E::UserNotFound(_) | E::ProfileNotFound(_) => format!("@{} was not found on TikTok", handle),
        E::ProfilePrivate(_) => format!("@{}'s profile is private", handle),
        E::AgeRestricted(_) => "This TikTok LIVE is age restricted".to_string(),
        E::HostNotOnline(_) | E::RoomIdMissing => {
            format!("@{} isn't live right now", handle)
        }
        other => format!("TikTok did not answer: {}", other),
    }
}

/// Codecs the page cannot be relied on to decode.
///
/// TikTok encodes some top tiers with ByteDance's HEVC variants, which play
/// only where the platform ships an HEVC decoder. They stay SELECTABLE by name,
/// so someone whose machine handles them can pick one, but "best" never lands
/// on one while an H.264 tier exists.
fn risky_codec(codec: Option<&str>) -> bool {
    matches!(codec, Some(c) if c.starts_with("bytevc"))
}

/// Menu row for one tier.
fn quality_of(r: &TikTokRendition) -> PlaybackQuality {
    let audio_only = r.tier == "ao";
    PlaybackQuality {
        // The tier key is the label, which is what TikTok's own player shows.
        // A resolution-derived name would be ambiguous: two tiers can report the
        // same frame size and differ only in bitrate.
        name: if audio_only {
            "audio_only".to_string()
        } else {
            r.tier.clone()
        },
        url: r.hls.clone().unwrap_or_default(),
        width: r.width,
        height: if audio_only { None } else { r.height },
        fps: r.fps,
        bandwidth: r.vbitrate,
    }
}

/// Choose a tier for a requested quality.
///
/// Same ladder as the shared HLS helper with ONE difference, and it is the
/// reason this exists rather than reusing it: TikTok is portrait, so `height` is
/// the LONG side of the frame. Matching "720p" against height picks the tier
/// whose long side is nearest 720, which on a 720x1280 ladder is the worst rung.
/// A numeric request is compared against the SHORT side instead.
fn select_tier(
    menu: &[PlaybackQuality],
    codecs: &[Option<String>],
    requested: &str,
) -> Option<(usize, String)> {
    if menu.is_empty() {
        return None;
    }
    let req = requested.trim().to_ascii_lowercase();
    let named = |i: usize| (i, menu[i].name.clone());

    let short_side = |q: &PlaybackQuality| -> Option<u32> {
        match (q.width, q.height) {
            (Some(w), Some(h)) => Some(w.min(h)),
            (Some(w), None) => Some(w),
            (None, Some(h)) => Some(h),
            (None, None) => None,
        }
    };

    if req.is_empty() || req == "best" || req == "source" {
        // Best PLAYABLE, not merely best. Rows are already sorted best first.
        let idx = menu
            .iter()
            .enumerate()
            .position(|(i, q)| {
                q.height.is_some() && !risky_codec(codecs.get(i).and_then(|c| c.as_deref()))
            })
            .unwrap_or(0);
        return Some(named(idx));
    }
    if req == "worst" {
        let idx = menu
            .iter()
            .enumerate()
            .filter(|(_, q)| q.height.is_some())
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(menu.len() - 1);
        return Some(named(idx));
    }
    if req == "audio_only" || req == "audio-only" || req == "audio" || req == "ao" {
        if let Some((i, _)) = menu.iter().enumerate().find(|(_, q)| q.height.is_none()) {
            return Some(named(i));
        }
        return Some(named(0));
    }
    // Exact tier name, which is how the menu labels itself.
    if let Some(i) = menu.iter().position(|q| q.name.eq_ignore_ascii_case(&req)) {
        return Some(named(i));
    }
    // Numeric, against the short side.
    if let Some(target) = req
        .trim_end_matches(|c: char| !c.is_ascii_digit())
        .split('p')
        .next()
        .and_then(|s| s.parse::<u32>().ok())
    {
        let at_or_below = menu
            .iter()
            .enumerate()
            .filter(|(_, q)| short_side(q).map(|s| s <= target).unwrap_or(false))
            .max_by_key(|(_, q)| short_side(q).unwrap_or(0));
        if let Some((i, _)) = at_or_below {
            return Some(named(i));
        }
        let above = menu
            .iter()
            .enumerate()
            .filter(|(_, q)| short_side(q).is_some())
            .min_by_key(|(_, q)| short_side(q).unwrap_or(u32::MAX));
        if let Some((i, _)) = above {
            return Some(named(i));
        }
    }
    Some(named(0))
}

/// Broadcast start, read out of the room payload. Without this the watch chrome
/// has no uptime to tick, which is the one metadata field TikTok chat could
/// never fill because the socket does not carry it.
fn started_at_from(raw_json: &str) -> String {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(raw_json) else {
        return String::new();
    };
    let secs = v
        .pointer("/data/create_time")
        .and_then(|c| c.as_i64())
        .unwrap_or(0);
    if secs <= 0 {
        return String::new();
    }
    chrono::DateTime::from_timestamp(secs, 0)
        .map(|d| d.to_rfc3339())
        .unwrap_or_default()
}

fn row_from(
    handle: &str,
    profile_user_id: &str,
    display: &str,
    avatar: &str,
    title: &str,
    viewers: u32,
    started_at: String,
    is_live: bool,
) -> ProviderStream {
    ProviderStream {
        provider: "tiktok".to_string(),
        key: make_key("tiktok", handle),
        id: String::new(),
        user_id: profile_user_id.to_string(),
        user_login: handle.to_string(),
        user_name: if display.is_empty() {
            handle.to_string()
        } else {
            display.to_string()
        },
        title: title.to_string(),
        viewer_count: viewers,
        game_id: String::new(),
        // TikTok LIVE has no category taxonomy, so this stays empty rather than
        // being filled with something that looks like one.
        game_name: String::new(),
        category_thumbnail: None,
        thumbnail_url: String::new(),
        started_at,
        profile_image_url: (!avatar.is_empty()).then(|| avatar.to_string()),
        is_live,
        watch_url: format!("https://www.tiktok.com/@{}/live", handle),
        tags: None,
    }
}

impl TikTokSource {
    /// One scrape, optionally followed by a room lookup, into a browse row.
    async fn row_for(&self, handle: &str) -> Result<ProviderStream> {
        let token = ttwid().await?;
        let profile = scrape_profile(handle, &token, HTTP_TIMEOUT, None, None, None)
            .await
            .map_err(|e| anyhow!("{}", friendly(&e, handle)))?;
        let live = !profile.room_id.is_empty();

        // Keep the room cache in step, so a resolve right after a browse row is
        // a cache hit rather than a second scrape.
        if let Ok(mut cache) = ROOMS.lock() {
            cache.insert(
                handle.to_string(),
                (Instant::now(), live.then(|| profile.room_id.clone())),
            );
        }

        let (title, viewers, started_at) = if live {
            match fetch_room_info(&profile.room_id, params()).await {
                Ok(info) => (
                    info.title,
                    info.viewers.max(0) as u32,
                    started_at_from(&info.raw_json),
                ),
                // A room that refuses metadata is still live; saying so with an
                // empty title beats reporting the creator as offline.
                Err(e) => {
                    log::debug!("[TikTok] room info for @{} failed: {}", handle, e);
                    (String::new(), 0, String::new())
                }
            }
        } else {
            (String::new(), 0, String::new())
        };

        Ok(row_from(
            handle,
            &profile.user_id,
            &profile.nickname,
            &profile.avatar_large,
            &title,
            viewers,
            started_at,
            live,
        ))
    }
}

#[async_trait]
impl StreamSource for TikTokSource {
    fn id(&self) -> &'static str {
        "tiktok"
    }

    fn caps(&self) -> SourceCaps {
        SourceCaps {
            playback: true,
            // Read through a hidden page, because TikTok signs its directory
            // requests inside the page and refuses any it did not sign.
            // Anonymous, so no sign-in is needed to browse.
            directory: true,
            // Exact-handle jump, the same shape Kick uses: typing a creator's
            // name takes you to them. There is no anonymous search endpoint.
            search: true,
            // TikTok's own following list needs a session; follows live in
            // `Settings.provider_follows` and are swept by `live_check`.
            native_follows: false,
            live_check: true,
        }
    }

    async fn resolve_playback(
        &self,
        stream_id: &str,
        channel: &str,
        quality: &str,
    ) -> Result<ResolvedPlayback> {
        let handle = clean_handle(channel);
        let room = room_id_for(&handle)
            .await?
            .ok_or_else(|| anyhow!("@{} isn't live right now", handle))?;
        let renditions = renditions_for(&room, &handle).await?;

        let menu: Vec<PlaybackQuality> = renditions.iter().map(quality_of).collect();
        let codecs: Vec<Option<String>> = renditions.iter().map(|r| r.vcodec.clone()).collect();
        let (idx, label) = select_tier(&menu, &codecs, quality)
            .ok_or_else(|| anyhow!("@{} published no playable rendition", handle))?;
        let chosen = &renditions[idx];

        // DASH first, which is the opposite of what the field names suggest.
        //
        // `cmaf` is not HLS: it is an `index.mpd`. It is also the only delivery
        // that reliably works. Measured across live rooms, the `hls` endpoint
        // answers 504 on most of TikTok's fleet, and where it does answer it
        // serves a frozen playlist whose segments 404 even when fetched
        // straight from the CDN with no relay involved. The DASH manifest
        // updates and its segments fetch clean, and because those segments are
        // already fMP4 there is no container work to do.
        //
        // `hls` stays as the fallback rather than being dropped: it is the
        // cheaper path when it works, and TikTok may repair it.
        //
        // FLV last. It is the only packaging some rooms publish (two in eleven
        // of TikTok's Top live, measured), and the relay has to rewrite its
        // container rather than just its manifest, so it is the costliest.
        let (upstream, packaging) = match (&chosen.cmaf, &chosen.hls, &chosen.flv) {
            (Some(mpd), _, _) => (mpd.clone(), tiktok_relay::Packaging::Dash),
            (None, Some(m3u8), _) => (m3u8.clone(), tiktok_relay::Packaging::Hls),
            (None, None, Some(flv)) => (flv.clone(), tiktok_relay::Packaging::Flv),
            (None, None, None) => {
                return Err(anyhow!(
                    "@{} is streaming in a format StreamNook can't play",
                    handle
                ))
            }
        };

        let url = tiktok_relay::start(
            stream_id,
            &handle,
            tiktok_relay::Upstream {
                url: upstream,
                packaging,
                tier: chosen.tier.clone(),
                width: chosen.width,
                height: chosen.height,
                fps: chosen.fps,
                bandwidth: chosen.vbitrate,
            },
        )
        .await?;

        Ok(ResolvedPlayback {
            kind: PlaybackKind::LocalHls,
            url,
            quality: label,
            qualities: menu,
        })
    }

    /// Session free, unlike the trait default.
    ///
    /// The default resolves at "best" and throws the URL away, passing an empty
    /// `stream_id`. Resolving here STARTS a relay session keyed on that id, so
    /// the default would open a session nobody owns every time the quality menu
    /// was enumerated. The ladder comes from the room metadata alone.
    async fn qualities(&self, channel: &str) -> Result<Vec<PlaybackQuality>> {
        let handle = clean_handle(channel);
        let room = room_id_for(&handle)
            .await?
            .ok_or_else(|| anyhow!("@{} isn't live right now", handle))?;
        Ok(renditions_for(&room, &handle)
            .await?
            .iter()
            .map(quality_of)
            .collect())
    }

    async fn channel_meta(&self, channel: &str) -> Result<ProviderStream> {
        self.row_for(&clean_handle(channel)).await
    }

    async fn directory(
        &self,
        category: Option<&str>,
        _cursor: Option<&str>,
        limit: u32,
    ) -> Result<StreamPage> {
        let mut streams = match category.map(str::trim).filter(|c| !c.is_empty()) {
            Some(kw) => crate::services::providers::tiktok_feed::keyword(kw).await?,
            None => crate::services::providers::tiktok_feed::top_live().await?,
        };
        streams.truncate(limit.clamp(1, 100) as usize);
        // No cursor: TikTok's feed has no paging, and asking again mostly
        // returns the same rooms.
        Ok(StreamPage {
            streams,
            cursor: None,
        })
    }

    async fn search(&self, query: &str) -> Result<StreamPage> {
        let handle = clean_handle(query);
        if handle.is_empty() {
            return Ok(StreamPage {
                streams: vec![],
                cursor: None,
            });
        }
        match self.row_for(&handle).await {
            Ok(row) => Ok(StreamPage {
                streams: vec![row],
                cursor: None,
            }),
            // Not found is an empty result, not a failure.
            Err(_) => Ok(StreamPage {
                streams: vec![],
                cursor: None,
            }),
        }
    }

    async fn categories(&self, _cursor: Option<&str>, _limit: u32) -> Result<CategoryPage> {
        Err(anyhow!("categories are not supported on tiktok"))
    }

    async fn live_check(&self, channels: &[String]) -> Result<Vec<ProviderStream>> {
        if channels.is_empty() {
            return Ok(vec![]);
        }
        if channels.len() > MAX_PER_SWEEP {
            log::warn!(
                "[TikTok] {} channels to check, sweeping {}; the rest are UNCHECKED this pass, not offline",
                channels.len(),
                MAX_PER_SWEEP
            );
        }
        let mut out = Vec::new();
        for (i, raw) in channels.iter().take(MAX_PER_SWEEP).enumerate() {
            let handle = clean_handle(raw);
            if handle.is_empty() {
                continue;
            }
            // A channel whose chat socket is open is already reporting its own
            // liveness and viewer count. Trust that rather than paying for a
            // page fetch to learn what is arriving over a live connection.
            // Only the positive case: a cached `false` may simply be stale.
            if let Some(meta) = crate::services::providers::tiktok::channel_meta(&handle) {
                if meta.is_live {
                    out.push(row_from(
                        &handle,
                        meta.user_id.as_deref().unwrap_or_default(),
                        meta.username.as_deref().unwrap_or_default(),
                        meta.profile_pic.as_deref().unwrap_or_default(),
                        meta.title.as_deref().unwrap_or_default(),
                        meta.viewer_count.unwrap_or(0) as u32,
                        meta.start_time.clone().unwrap_or_default(),
                        true,
                    ));
                    continue;
                }
            }
            if i > 0 {
                tokio::time::sleep(SWEEP_SPACING).await;
            }
            match self.row_for(&handle).await {
                Ok(row) => out.push(row),
                // A failed check is not evidence of an ending, so the channel is
                // simply left out of this pass.
                Err(e) => log::debug!("[TikTok] live check for @{} failed: {}", handle, e),
            }
        }
        Ok(out)
    }

    async fn followed_live(&self) -> Result<Vec<ProviderStream>> {
        Err(anyhow!("followed_live is not supported on tiktok"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rung(tier: &str, w: u32, h: u32, codec: &str) -> TikTokRendition {
        TikTokRendition {
            tier: tier.to_string(),
            label: tier.to_string(),
            level: 0,
            hls: Some(format!("https://cdn.example.com/{}.m3u8", tier)),
            width: Some(w),
            height: Some(h),
            vcodec: Some(codec.to_string()),
            ..Default::default()
        }
    }

    fn audio() -> TikTokRendition {
        TikTokRendition {
            tier: "ao".to_string(),
            label: "ao".to_string(),
            flv: Some("https://cdn.example.com/ao.flv".to_string()),
            ..Default::default()
        }
    }

    /// A portrait ladder, best first, as `parse_stream_urls` orders it.
    fn ladder() -> (Vec<PlaybackQuality>, Vec<Option<String>>) {
        let r = vec![
            rung("origin", 1080, 1920, "h264"),
            rung("hd", 720, 1280, "h264"),
            rung("sd", 480, 854, "h264"),
            audio(),
        ];
        (
            r.iter().map(quality_of).collect(),
            r.iter().map(|x| x.vcodec.clone()).collect(),
        )
    }

    #[test]
    fn a_numeric_request_matches_the_short_side() {
        let (menu, codecs) = ladder();
        // The whole reason this helper exists. Matching 720 against `height`
        // would walk the ladder 1920/1280/854 and land on `sd`.
        assert_eq!(select_tier(&menu, &codecs, "720p").unwrap().1, "hd");
        assert_eq!(select_tier(&menu, &codecs, "1080").unwrap().1, "origin");
        assert_eq!(select_tier(&menu, &codecs, "480p").unwrap().1, "sd");
        // Below every rung: take the smallest rather than nothing.
        assert_eq!(select_tier(&menu, &codecs, "144p").unwrap().1, "sd");
    }

    #[test]
    fn best_and_worst_and_audio_resolve() {
        let (menu, codecs) = ladder();
        assert_eq!(select_tier(&menu, &codecs, "best").unwrap().1, "origin");
        assert_eq!(select_tier(&menu, &codecs, "").unwrap().1, "origin");
        // `worst` must never be the audio rung.
        assert_eq!(select_tier(&menu, &codecs, "worst").unwrap().1, "sd");
        assert_eq!(select_tier(&menu, &codecs, "audio_only").unwrap().1, "audio_only");
        assert_eq!(select_tier(&menu, &codecs, "audio").unwrap().1, "audio_only");
    }

    #[test]
    fn best_skips_a_codec_the_page_may_not_decode() {
        let r = vec![
            rung("origin", 1080, 1920, "bytevc1"),
            rung("hd", 720, 1280, "h264"),
        ];
        let menu: Vec<PlaybackQuality> = r.iter().map(quality_of).collect();
        let codecs: Vec<Option<String>> = r.iter().map(|x| x.vcodec.clone()).collect();
        assert_eq!(select_tier(&menu, &codecs, "best").unwrap().1, "hd");
        // Still reachable by name for anyone whose machine handles it.
        assert_eq!(select_tier(&menu, &codecs, "origin").unwrap().1, "origin");
    }

    #[test]
    fn an_all_hevc_ladder_still_plays_something() {
        let r = vec![rung("origin", 1080, 1920, "bytevc1")];
        let menu: Vec<PlaybackQuality> = r.iter().map(quality_of).collect();
        let codecs: Vec<Option<String>> = r.iter().map(|x| x.vcodec.clone()).collect();
        assert_eq!(select_tier(&menu, &codecs, "best").unwrap().1, "origin");
    }

    /// The field named `cmaf` is a DASH manifest, not HLS. Measured against
    /// live rooms: every one of them served an `index.mpd` there. Nothing may
    /// route it to the player as a playlist.
    #[test]
    fn a_dash_url_never_reaches_the_quality_menu() {
        let r = TikTokRendition {
            tier: "hd".into(),
            label: "hd".into(),
            cmaf: Some("https://pull-f5-tt01.tiktokcdn.com/x/index.mpd".into()),
            flv: Some("https://pull-flv.tiktokcdn.com/x.flv".into()),
            width: Some(720),
            height: Some(1280),
            ..Default::default()
        };
        // The menu row must not advertise a url the player cannot use.
        assert_eq!(quality_of(&r).url, "", "a DASH url must not reach the menu");
        assert!(r.hls.is_none());
    }

    #[test]
    fn the_audio_rung_carries_no_height() {
        let q = quality_of(&audio());
        // `quality_names` only offers `worst` when something has a height, and
        // the audio rule finds its row by the absence of one.
        assert_eq!(q.height, None);
        assert_eq!(q.name, "audio_only");
    }

    #[test]
    fn handles_arrive_in_several_shapes() {
        assert_eq!(clean_handle("@Someone"), "someone");
        assert_eq!(clean_handle("tiktok:Someone"), "someone");
        assert_eq!(clean_handle("tiktok/someone"), "someone");
        assert_eq!(clean_handle("  @someone "), "someone");
    }

    /// Everything the adapter does that does NOT need someone to be live,
    /// against real TikTok: identity, the offline answer, search, the batch
    /// sweep, and the error wording a person would actually be shown.
    ///
    /// ```text
    /// SN_TIKTOK_HANDLE=<any real handle> cargo test --lib --no-default-features \
    ///     -- --ignored --nocapture exercises_the_adapter_against_real_tiktok
    /// ```
    #[tokio::test]
    #[ignore = "needs network; set SN_TIKTOK_HANDLE"]
    async fn exercises_the_adapter_against_real_tiktok() {
        let handle = std::env::var("SN_TIKTOK_HANDLE").expect("set SN_TIKTOK_HANDLE");
        let src = TikTokSource::new();

        // Identity, whether or not they are live.
        let meta = src.channel_meta(&handle).await.expect("channel_meta");
        println!(
            "meta: login={} name={:?} id={} live={} avatar={:?}",
            meta.user_login, meta.user_name, meta.user_id, meta.is_live, meta.profile_image_url
        );
        assert_eq!(meta.provider, "tiktok");
        assert_eq!(meta.key, format!("tiktok:{}", clean_handle(&handle)));
        assert!(!meta.user_id.is_empty(), "a real account has a user id");
        assert!(!meta.user_name.is_empty());
        assert!(meta.profile_image_url.is_some(), "avatar should resolve");
        // TikTok has no category system, and the row must say so rather than
        // inventing one.
        assert!(meta.game_name.is_empty());
        assert_eq!(
            meta.watch_url,
            format!("https://www.tiktok.com/@{}/live", clean_handle(&handle))
        );

        // Search is an exact-handle jump; a real handle finds exactly itself.
        let page = src.search(&handle).await.expect("search");
        assert_eq!(page.streams.len(), 1, "exact handle should find one row");
        assert_eq!(page.streams[0].user_login, clean_handle(&handle));

        // A handle that cannot exist is an EMPTY result, never an error: the
        // search box must not show a failure for a typo.
        let empty = src
            .search("this-handle-should-not-exist-9y3x7q")
            .await
            .expect("search must not error on a miss");
        assert!(empty.streams.is_empty());

        // The batch sweep tolerates a bad entry without losing the good ones.
        let rows = src
            .live_check(&[handle.clone(), "this-handle-should-not-exist-9y3x7q".into()])
            .await
            .expect("live_check");
        println!("live_check returned {} row(s)", rows.len());
        assert!(
            rows.iter().any(|r| r.user_login == clean_handle(&handle)),
            "the real handle must survive a sweep containing a bad one"
        );

        // Browse surfaces TikTok cannot serve must refuse explicitly, never
        // return an empty success that reads as "nothing is on". The directory
        // itself is served through the app's hidden window, so it is verified
        // in the running app rather than here.
        assert!(src.categories(None, 20).await.is_err());
        assert!(src.followed_live().await.is_err());

        // And the offline wording, which is what a person actually reads.
        if !meta.is_live {
            let err = src
                .resolve_playback("probe", &handle, "best")
                .await
                .expect_err("an offline creator cannot resolve");
            let text = err.to_string();
            println!("offline error: {text}");
            assert!(text.contains("isn't live"), "unhelpful wording: {text}");
            assert!(!text.contains("Err"), "raw debug leaked: {text}");
        }
    }

    /// The whole stack against a real broadcast: scrape, room info, the
    /// rendition ladder, tier selection, the relay session, the rewritten
    /// playlist, and one real segment fetched through our own origin.
    ///
    /// Ignored by default because it needs a creator who is live at the moment
    /// it runs. Run it with:
    ///
    /// ```text
    /// SN_TIKTOK_HANDLE=<handle> cargo test --lib --no-default-features \
    ///     -- --ignored --nocapture resolves_a_real_live_room
    /// ```
    #[tokio::test]
    #[ignore = "needs a live TikTok room; set SN_TIKTOK_HANDLE"]
    async fn resolves_a_real_live_room() {
        let handle = std::env::var("SN_TIKTOK_HANDLE")
            .expect("set SN_TIKTOK_HANDLE to a creator who is live right now");
        let src = TikTokSource::new();

        let meta = src.channel_meta(&handle).await.expect("channel_meta");
        println!(
            "meta: name={:?} live={} viewers={} title={:?} started_at={:?} avatar={}",
            meta.user_name,
            meta.is_live,
            meta.viewer_count,
            meta.title,
            meta.started_at,
            meta.profile_image_url.is_some()
        );
        assert!(meta.is_live, "@{handle} is not live; pick someone who is");
        assert_eq!(meta.provider, "tiktok");
        assert!(meta.watch_url.contains(&clean_handle(&handle)));
        assert!(
            !meta.started_at.is_empty(),
            "started_at is what finally gives TikTok an uptime tick"
        );

        // The menu must not start a session. If it did, the trait default was
        // being used instead of our override and a session nobody owns would be
        // opened on every quality enumeration.
        let menu = src.qualities(&handle).await.expect("qualities");
        println!("menu: {} rungs", menu.len());
        for q in &menu {
            println!(
                "  {:<12} {:?}x{:?} fps={:?} bw={:?}",
                q.name, q.width, q.height, q.fps, q.bandwidth
            );
        }
        assert!(!menu.is_empty(), "a live room must offer something");
        assert!(
            menu.iter().any(|q| q.height.is_some()),
            "no video rung, so `worst` would vanish from the menu"
        );

        let resolved = src
            .resolve_playback("probe", &handle, "best")
            .await
            .expect("resolve_playback");
        println!("resolved: {:?} {} -> {}", resolved.kind, resolved.quality, resolved.url);
        assert!(matches!(resolved.kind, PlaybackKind::LocalHls));
        assert!(resolved.url.contains("/s/probe/"), "url: {}", resolved.url);

        let client = reqwest::Client::new();
        let master = client.get(&resolved.url).send().await.expect("master");
        assert!(master.status().is_success(), "master {}", master.status());
        let master = master.text().await.unwrap();
        println!("--- master ---\n{master}");

        // Two packagings reach here. A DASH room is translated into a variant
        // plus an audio group (`video.m3u8`); an HLS room is relayed as one
        // rewritten playlist (`live.m3u8`). Follow whichever the master names
        // rather than assuming, so this test covers both.
        let origin = resolved
            .url
            .split("/s/probe/")
            .next()
            .expect("origin")
            .to_string();
        let media_ref = master
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty() && !l.starts_with('#'))
            .expect("the master must name a media playlist");
        let media_url = if media_ref.starts_with('/') {
            format!("{origin}{media_ref}")
        } else {
            resolved.url.replace("stream.m3u8", media_ref)
        };

        let media = client.get(&media_url).send().await.expect("media playlist");
        assert!(
            media.status().is_success(),
            "media playlist {} (reason: {:?})",
            media.status(),
            media.headers().get("x-sn-reason")
        );
        let media = media.text().await.unwrap();
        println!("--- media playlist ---\n{media}");
        assert!(
            !media.contains("tiktokcdn") && !media.contains("https://"),
            "an upstream url leaked, so the player would bypass the relay:\n{media}"
        );
        assert!(media.contains("#EXTINF"), "no segments listed:\n{media}");

        // An fMP4 playlist must name its init segment, or the player has no
        // decoder configuration and every segment is undecodable.
        let mut to_fetch: Vec<String> = Vec::new();
        if let Some(map_line) = media.lines().find(|l| l.starts_with("#EXT-X-MAP")) {
            let uri = map_line
                .split("URI=\"")
                .nth(1)
                .and_then(|r| r.split('"').next())
                .expect("EXT-X-MAP must carry a URI");
            to_fetch.push(uri.to_string());
        }
        to_fetch.push(
            media
                .lines()
                .map(str::trim)
                .find(|l| !l.is_empty() && !l.starts_with('#'))
                .expect("a segment line")
                .to_string(),
        );

        // Real bytes through our own origin. This is what proves the CDN
        // accepts our headers, that the signature survives the rewrite, and
        // that the host allowlist admits the real CDN rather than refusing it.
        for path in to_fetch {
            let url = if path.starts_with('/') {
                format!("{origin}{path}")
            } else {
                resolved.url.replace("stream.m3u8", &path)
            };
            let res = client.get(&url).send().await.expect("segment");
            println!("fetch {path} -> {}", res.status());
            assert!(
                res.status().is_success(),
                "refused: {} (reason: {:?})",
                res.status(),
                res.headers().get("x-sn-reason")
            );
            let bytes = res.bytes().await.unwrap();
            println!("  {} bytes, container={}", bytes.len(), container_of(&bytes));
            assert!(bytes.len() > 200, "suspiciously small body for {path}");
        }

        // The audio rendition, which the master points at separately and which
        // following only the variant would never touch. A silent stream is
        // still a broken stream, so this is not an optional extra.
        if let Some(group) = master.lines().find(|l| l.starts_with("#EXT-X-MEDIA:TYPE=AUDIO")) {
            let uri = group
                .split("URI=\"")
                .nth(1)
                .and_then(|r| r.split('"').next())
                .expect("the audio group must carry a URI");
            let audio_url = format!("{origin}{uri}");
            let audio = client.get(&audio_url).send().await.expect("audio playlist");
            assert!(
                audio.status().is_success(),
                "audio playlist {} (reason: {:?})",
                audio.status(),
                audio.headers().get("x-sn-reason")
            );
            let audio = audio.text().await.unwrap();
            println!("--- audio playlist ---\n{audio}");
            assert!(audio.contains("#EXTINF"), "no audio segments:\n{audio}");
            assert!(
                audio.contains("#EXT-X-MAP"),
                "audio needs its own init or it cannot be decoded:\n{audio}"
            );

            let a_init = audio
                .lines()
                .find(|l| l.starts_with("#EXT-X-MAP"))
                .and_then(|l| l.split("URI=\"").nth(1))
                .and_then(|r| r.split('"').next())
                .expect("audio init uri")
                .to_string();
            let a_seg = audio
                .lines()
                .map(str::trim)
                .find(|l| !l.is_empty() && !l.starts_with('#'))
                .expect("an audio segment")
                .to_string();
            for path in [a_init, a_seg] {
                let res = client
                    .get(&format!("{origin}{path}"))
                    .send()
                    .await
                    .expect("audio fetch");
                println!("audio {path} -> {}", res.status());
                assert!(
                    res.status().is_success(),
                    "audio refused: {} (reason: {:?})",
                    res.status(),
                    res.headers().get("x-sn-reason")
                );
                let bytes = res.bytes().await.unwrap();
                println!("  {} bytes, container={}", bytes.len(), container_of(&bytes));
                assert!(bytes.len() > 100, "empty audio body for {path}");
            }
        }

        // The point of the relay, proven rather than asserted. TikTok's CDN
        // keeps only the newest segment, so a window can exist only because
        // this relay takes each one while it is newest and holds it.
        //
        // Deliberately WITHOUT polling the playlist in between: capture runs on
        // the relay's own timer, and a player whose poll lands late must not
        // cost a segment. Keep the session alive with a cheap master request,
        // wait long enough for several segments to pass, then read the window.
        for _ in 0..5 {
            tokio::time::sleep(std::time::Duration::from_millis(2000)).await;
            let _ = client.get(&resolved.url).send().await;
        }
        let body = client
            .get(&media_url)
            .send()
            .await
            .expect("playlist")
            .text()
            .await
            .unwrap();
        let numbers: Vec<u64> = body
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .filter_map(|l| l.rsplit('/').next()?.parse().ok())
            .collect();
        let gaps = body.matches("#EXT-X-GAP").count();
        println!("window after 10s unpolled: {:?} ({gaps} gaps)", numbers);
        assert!(
            numbers.len() >= 4,
            "the window did not fill while the player was idle, so capture is \
             still tied to polling: {numbers:?}"
        );
        // Consecutive: the media sequence is positional, so a hole the playlist
        // closed up would relabel every segment after it. Any hole must show
        // as an explicit gap instead, and a healthy stream should have none.
        assert!(
            numbers.windows(2).all(|w| w[1] == w[0] + 1),
            "segment numbers are not consecutive: {numbers:?}"
        );
        assert_eq!(gaps, 0, "a healthy stream lost segments:\n{body}");
        let oldest_seen = body
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty() && !l.starts_with('#'))
            .expect("a segment")
            .to_string();

        let url = format!("{origin}{oldest_seen}");
        let res = client.get(&url).send().await.expect("oldest segment");
        println!("reach back to {oldest_seen} -> {}", res.status());
        assert!(
            res.status().is_success(),
            "the oldest advertised segment must still serve: {} (reason: {:?})",
            res.status(),
            res.headers().get("x-sn-reason")
        );
        let bytes = res.bytes().await.unwrap();
        println!("  {} bytes, container={}", bytes.len(), container_of(&bytes));
        assert!(bytes.len() > 200);

        crate::services::tiktok_relay::stop("probe");
    }

    /// Read a media body's container from its bytes, so the test reports what
    /// actually arrived rather than trusting a file extension.
    fn container_of(b: &[u8]) -> String {
        if b.len() < 12 {
            return "too short".into();
        }
        match &b[4..8] {
            b"ftyp" => format!("fMP4 init (brand {})", String::from_utf8_lossy(&b[8..12])),
            b"styp" => "fMP4 segment".into(),
            b"moof" => "fMP4 fragment".into(),
            _ if b[0] == 0x47 => "MPEG-TS".into(),
            _ => format!("unknown {:02x?}", &b[..8]),
        }
    }

    #[test]
    fn start_time_comes_out_of_the_room_payload() {
        let raw = r#"{"data":{"create_time":1750000000}}"#;
        assert!(started_at_from(raw).starts_with("2025-"), "got {}", started_at_from(raw));
        assert_eq!(started_at_from("{}"), "");
        assert_eq!(started_at_from("not json"), "");
    }
}
