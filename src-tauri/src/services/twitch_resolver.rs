// Native Twitch stream resolution (no Streamlink subprocess).
//
// This is the in-Rust replacement for what Streamlink did with `--stream-url`:
// turn a channel + quality into the media-playlist URL the player loads.
//
// The fragile core (GQL PlaybackAccessToken, usher fetch, out-of-band
// entitlement) lives in `auth_proxy` and is reused here as plain function
// calls. The genuinely new pieces are the HLS master playlist parser
// (`parse_master`) and variant selection (`select_variant`), which mirror what
// Streamlink was doing after it received the master.

use crate::services::auth_proxy::{self, PlaybackStatus};
use crate::services::muted_segments::MutedRange;
use crate::services::quality::{pick_closest_index, pick_closest_quality, short_edge_label, sort_qualities_descending};
use crate::services::twitch_view::{self, View};
use crate::services::vod_chapters::{self, Chapter, ChapterNode};
use crate::services::vod_storyboard::{self, Storyboard};
use anyhow::{anyhow, Context, Result};
use log::debug;
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Ordered list of video codec families the player may resolve to (most-preferred
/// first), set by the frontend after probing `MediaSource.isTypeSupported` and the
/// `enhanced_codecs` setting. Empty until reported = H.264-only (the always-safe
/// baseline). This is the gate that makes the AV1/HEVC selection capability-aware:
/// StreamNook serves a single variant with NO in-player codec fallback, so resolving
/// to a codec this machine can't decode would be a black screen. Selection never
/// returns a codec outside this list, and H.264 is always kept in it.
static CODEC_PREF: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Codec families a player could not play on a channel this session although
/// the probe said this machine decodes them, as (channel, family). Scoped to
/// the channel: one broadcast's bad encode says nothing about the codec on
/// every other channel, so selection drops it there alone.
static FAILED_CODECS: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

/// Set the allowed/decodable video codec families ("av1" | "hevc" | "h264"),
/// most-preferred first. H.264 is always appended so an undecodable codec can never
/// be the only eligible option. Called from the `set_codec_preference` command.
pub fn set_codec_preference(prefs: Vec<String>) {
    *CODEC_PREF.lock().unwrap() = allowed_codecs(prefs, &[]);
}

/// Whether the player has reported what this machine decodes yet.
fn codec_pref_reported() -> bool {
    !CODEC_PREF.lock().unwrap().is_empty()
}

/// The codec families that failed to play on `channel` this session.
fn failed_in_session(channel: &str) -> Vec<String> {
    FAILED_CODECS
        .lock()
        .unwrap()
        .iter()
        .filter(|(c, _)| c.eq_ignore_ascii_case(channel))
        .map(|(_, f)| f.clone())
        .collect()
}

/// The codec families selection skips on `channel`: those that failed to
/// play there this session, and those held off it now.
fn failed_for(channel: &str) -> Vec<String> {
    let mut failed = failed_in_session(channel);
    for family in held_at(channel, Instant::now()) {
        if !failed.contains(&family) {
            failed.push(family);
        }
    }
    failed
}

/// The selection preference on `channel`: what this machine decodes, less
/// what failed to play on that channel.
fn pref_for(channel: &str) -> Vec<String> {
    allowed_codecs(effective_codec_pref(), &failed_for(channel))
}

/// `prefs` limited to known families and stripped of every `failed` one, with
/// H.264 always present.
fn allowed_codecs(mut prefs: Vec<String>, failed: &[String]) -> Vec<String> {
    prefs.retain(|s| matches!(s.as_str(), "av1" | "hevc" | "h264") && !failed.contains(s));
    if !prefs.iter().any(|s| s == "h264") {
        prefs.push("h264".to_string());
    }
    prefs
}

/// The video codec family named in a CODECS list or a MIME type that selection
/// can step away from: AV1 or HEVC. None for H.264, audio or nothing known,
/// since H.264 is the floor every tier falls back to.
pub fn avoidable_codec_family(codecs: &str) -> Option<&'static str> {
    let list = codecs.split_once("codecs=").map_or(codecs, |(_, c)| c);
    list.split(',')
        .map(|c| c.trim().trim_matches(|ch| ch == '"' || ch == '\''))
        .find_map(|c| match codec_family(Some(c)) {
            family @ ("av1" | "hevc") => Some(family),
            _ => None,
        })
}

/// A player on `channel` could not play the codec it was given: the failed
/// SourceBuffer type when it got that far (`mime_type`), else the codecs it
/// was serving. The family named there leaves selection on that channel for
/// the session; returned so the caller can decide whether a restart gets away
/// from it. None for H.264, audio or nothing known: there is nothing better to
/// fall back to.
pub fn fail_codec(channel: &str, mime_type: Option<&str>, serving: Option<&str>) -> Option<&'static str> {
    let family = match mime_type {
        Some(mime) => avoidable_codec_family(mime),
        None => serving.and_then(avoidable_codec_family),
    }?;
    avoid_codec_family(channel, family);
    Some(family)
}

/// Stop selecting `family` on `channel` for the rest of the session: the
/// player could not play it there. H.264 is never dropped, and no other
/// channel is touched.
pub fn avoid_codec_family(channel: &str, family: &str) {
    if family == "h264" {
        return;
    }
    let channel = channel.to_ascii_lowercase();
    let mut failed = FAILED_CODECS.lock().unwrap();
    if !failed.iter().any(|(c, f)| *c == channel && f == family) {
        failed.push((channel.clone(), family.to_string()));
        log::warn!("[Resolver] {family} failed to play on {channel}; not selecting it there again this session");
    }
}

/// Codec families held off a channel until an instant, as (channel, family,
/// until): a stream that keeps reloading there plays in H.264 for a while.
/// HEVC and AV1 always take the low-latency origin, which has no stable
/// whole-segment path for them, so only H.264 lets the reload hold-off reach
/// them. Unlike `FAILED_CODECS` the families come back when the hold ends.
static HELD_CODECS: Mutex<Vec<(String, String, Instant)>> = Mutex::new(Vec::new());

/// The families a hold covers: every one selection can step away from.
const HELD_FAMILIES: [&str; 2] = ["hevc", "av1"];

/// Hold HEVC and AV1 off `channel` for `d` from now, replacing an earlier
/// hold there.
pub fn hold_codecs(channel: &str, d: Duration) {
    let channel = channel.to_ascii_lowercase();
    let now = Instant::now();
    let mut held = HELD_CODECS.lock().unwrap_or_else(|e| e.into_inner());
    held.retain(|(c, _, until)| *until > now && *c != channel);
    for family in HELD_FAMILIES {
        held.push((channel.clone(), family.to_string(), now + d));
    }
    log::warn!(
        "[Resolver] HEVC and AV1 held off {channel} for {} min after repeated reloads",
        d.as_secs() / 60
    );
}

/// End `channel`'s codec hold: the viewer picked a quality there.
pub fn release_codec_hold(channel: &str) {
    let now = Instant::now();
    let mut held = HELD_CODECS.lock().unwrap_or_else(|e| e.into_inner());
    let releases = held
        .iter()
        .any(|(c, _, until)| *until > now && c.eq_ignore_ascii_case(channel));
    held.retain(|(c, _, until)| *until > now && !c.eq_ignore_ascii_case(channel));
    if releases {
        debug!("[Resolver] {channel}'s codec hold ends: a quality was picked");
    }
}

/// The families held off `channel` at `now`. Reads only, so a test may ask
/// about a later instant without touching another test's holds.
fn held_at(channel: &str, now: Instant) -> Vec<String> {
    HELD_CODECS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .filter(|(c, _, until)| *until > now && c.eq_ignore_ascii_case(channel))
        .map(|(_, f, _)| f.clone())
        .collect()
}

/// Whether a rendition with these CODECS is held off `channel` now.
pub fn codec_held(channel: &str, codecs: Option<&str>) -> bool {
    codecs
        .and_then(avoidable_codec_family)
        .is_some_and(|family| held_at(channel, Instant::now()).iter().any(|f| f.as_str() == family))
}

/// A selection preference from the codecs a phone decodes in hardware, as its
/// shell names them (`av1`/`av01`, `hevc`/`h265`/`hvc1`/`hev1`, `h264`/`avc1`).
/// H.264 leads, so a tier Twitch offers in H.264 keeps the transport stream the
/// low-latency route is built on; HEVC, then AV1, serve the tiers only Enhanced
/// Broadcasting offers, when that setting is on and the phone decodes them. A
/// codec the phone has no hardware decoder for is never selected, so its tiers
/// drop out of the quality menu and of `best`.
pub fn device_codec_preference(hardware: &[String], enhanced: bool) -> Vec<String> {
    let has = |family: &str| {
        hardware.iter().any(|c| {
            let c = c.trim().to_ascii_lowercase();
            match family {
                "av1" => c == "av1" || c.starts_with("av01"),
                "hevc" => c == "hevc" || c == "h265" || c.starts_with("hvc1") || c.starts_with("hev1"),
                _ => false,
            }
        })
    };
    let mut prefs = vec!["h264".to_string()];
    if enhanced {
        for family in ["hevc", "av1"] {
            if has(family) {
                prefs.push(family.to_string());
            }
        }
    }
    prefs
}

/// Apply the phone's hardware codecs (passed to `start` by the shell) and the
/// Enhanced Broadcasting setting to selection. Run at startup and after every
/// settings save, since the setting can change.
#[cfg(target_os = "ios")]
pub fn refresh_device_codecs(settings: &crate::models::settings::Settings) {
    let prefs = device_codec_preference(&crate::ios_ffi::launch_codecs(), settings.streamlink.enhanced_codecs);
    if *CODEC_PREF.lock().unwrap() != prefs {
        log::info!("[Resolver] video codecs this phone may select: {}", prefs.join(", "));
        set_codec_preference(prefs);
    }
}

/// Whether a variant can play here: audio, or video in an allowed codec.
fn decodable(v: &Variant, pref: &[String]) -> bool {
    v.height.is_none() || codec_rank(v, pref) != usize::MAX
}

/// The renditions of `master` this device may play: audio, and video in a
/// codec it decodes and the AV1 & h265 setting allows. StreamNook serves one
/// rendition with no player-side fallback, so a tier outside that list is a
/// black picture; it never reaches `best`, a quality pick or the quality menu.
/// Twitch serves its 1440p tier in HEVC, so a machine or phone without an HEVC decoder
/// plays the 1080p H.264 tier as `best` instead of failing on 1440p.
///
/// Until the player reports what this machine decodes (and while nothing has
/// failed on `channel`), every tier stays, exactly as before the filter
/// existed; `best` still picks with the H.264-first default.
pub fn playable_variants(master: &str, channel: &str) -> Vec<Variant> {
    if !codec_pref_reported() && failed_for(channel).is_empty() {
        return parse_master(master);
    }
    retain_playable(parse_master(master), &pref_for(channel))
}

/// The quality menu's tiers on `channel`: `playable_variants` without the
/// codec holds, so a held tier stays listed for the viewer to pick (which
/// ends the hold).
fn menu_variants(master: &str, channel: &str) -> Vec<Variant> {
    let failed = failed_in_session(channel);
    if !codec_pref_reported() && failed.is_empty() {
        return parse_master(master);
    }
    retain_playable(parse_master(master), &allowed_codecs(effective_codec_pref(), &failed))
}

/// `variants` without the video tiers `pref` rules out. A master with no
/// playable video at all is kept whole: trying it beats serving audio only.
fn retain_playable(variants: Vec<Variant>, pref: &[String]) -> Vec<Variant> {
    if !variants.iter().any(|v| v.height.is_some() && decodable(v, pref)) {
        return variants;
    }
    variants.into_iter().filter(|v| decodable(v, pref)).collect()
}

/// The effective preference (never empty: defaults to H.264-only before the
/// frontend reports).
fn effective_codec_pref() -> Vec<String> {
    let p = CODEC_PREF.lock().unwrap().clone();
    if p.is_empty() {
        vec!["h264".to_string()]
    } else {
        p
    }
}

/// Tallest rendition the display can actually show, in pixels. 0 = uncapped.
///
/// Mobile only. Desktop never sets this, so desktop keeps resolving to source
/// exactly as before - a monitor generally CAN show the source rendition, and a
/// desktop is not on a battery budget. See `capped_source_index`.
static MAX_VIDEO_HEIGHT: Mutex<u32> = Mutex::new(0);

/// Set the display's usable video height. Called from `set_max_video_height`,
/// which the mobile shell invokes at boot and again whenever the display mode
/// changes (a phone can switch between FHD+ and QHD+ at runtime, so a value
/// sampled once at startup goes stale).
pub fn set_max_video_height(height: u32) {
    *MAX_VIDEO_HEIGHT.lock().unwrap() = height;
}

/// The cap, or None when unset/uncapped.
///
/// Floored at 360 so a bad or half-initialised value can never pin every stream
/// to the bottom of the ladder.
fn effective_max_height() -> Option<u32> {
    let h = *MAX_VIDEO_HEIGHT.lock().unwrap();
    if h == 0 { None } else { Some(h.max(360)) }
}

/// Whether one `CODECS` entry names an audio codec.
pub(crate) fn is_audio_codec(codec: &str) -> bool {
    let c = codec.trim().to_ascii_lowercase();
    ["mp4a", "ac-3", "ec-3", "opus", "flac", "alac", "mp3"]
        .iter()
        .any(|a| c.starts_with(a))
}

/// Classify a `CODECS="..."` attribute's video codec into a family name: the
/// first entry that is not audio, wherever it sits in the list. An empty or
/// missing codec is treated as H.264 (the safe, universally-decodable default).
fn codec_family(codecs: Option<&str>) -> &'static str {
    let video = codecs
        .into_iter()
        .flat_map(|c| c.split(','))
        .map(str::trim)
        .find(|c| !c.is_empty() && !is_audio_codec(c))
        .unwrap_or("");
    if video.starts_with("av01") {
        "av1"
    } else if video.starts_with("hev1") || video.starts_with("hvc1") {
        "hevc"
    } else if video.starts_with("avc1") || video.starts_with("avc3") || video.is_empty() {
        "h264"
    } else {
        "other"
    }
}

/// Position of a variant's codec family in `pref` (lower = more preferred);
/// `usize::MAX` when the codec isn't allowed/decodable on this machine.
fn codec_rank(v: &Variant, pref: &[String]) -> usize {
    let fam = codec_family(v.codecs.as_deref());
    pref.iter().position(|p| p == fam).unwrap_or(usize::MAX)
}

/// Two renditions are the "same quality" when height matches and fps is within a
/// frame (Twitch emits identical "60.000" strings, but tolerate float noise).
fn same_quality(a: &Variant, height: Option<u32>, fps: Option<f64>) -> bool {
    a.height == height
        && match (a.fps, fps) {
            (Some(x), Some(y)) => (x - y).abs() < 1.0,
            (None, None) => true,
            _ => false,
        }
}

/// Among the variants at `(height, fps)` whose codec is allowed, the index of the
/// most-preferred codec (ties broken by higher bandwidth). `None` when none are
/// eligible (e.g. that resolution is offered only in a codec this machine can't
/// decode), so callers fall back to a guaranteed-decodable rendition.
fn prefer_codec_at(
    variants: &[Variant],
    height: Option<u32>,
    fps: Option<f64>,
    pref: &[String],
) -> Option<usize> {
    variants
        .iter()
        .enumerate()
        .filter(|(_, v)| same_quality(v, height, fps) && codec_rank(v, pref) != usize::MAX)
        .min_by(|(_, a), (_, b)| {
            codec_rank(a, pref)
                .cmp(&codec_rank(b, pref))
                .then(b.bandwidth.cmp(&a.bandwidth))
        })
        .map(|(i, _)| i)
}

/// Clip access-token GQL op (`VideoAccessToken_Clip`). Sent as the full inline
/// query rather than a persisted-query hash: Twitch rotates the stored hashes and
/// returns `PersistedQueryNotFound` when ours goes stale, which would null out
/// `data.clip` and surface as a bogus "clip not found". The inline query has no
/// hash to look up, so it survives those rotations.
const CLIP_ACCESS_TOKEN_QUERY: &str = "\
query VideoAccessToken_Clip($slug: ID!) {\n\
  clip(slug: $slug) {\n\
    id\n\
    durationSeconds\n\
    videoOffsetSeconds\n\
    video {\n\
      id\n\
    }\n\
    game {\n\
      id\n\
      name\n\
    }\n\
    broadcaster {\n\
      login\n\
    }\n\
    playbackAccessToken(params: {platform: \"web\", playerBackend: \"mediaplayer\", playerType: \"site\"}) {\n\
      signature\n\
      value\n\
    }\n\
    videoQualities {\n\
      frameRate\n\
      quality\n\
      sourceURL\n\
    }\n\
  }\n\
}";

/// One rendition from a Twitch master playlist (an `EXT-X-MEDIA` /
/// `EXT-X-STREAM-INF` / url triple).
#[derive(Debug, Clone)]
pub struct Variant {
    /// The `NAME` attribute, e.g. "1080p60", "480p", "audio_only". This is the
    /// quality label Streamlink surfaces; Twitch ships two shapes in the wild
    /// ("480p" vs "480p30"), so we keep it verbatim and let `pick_closest_quality`
    /// reconcile against the user's saved preference.
    pub name: String,
    /// The `GROUP-ID`. The source tier is `"chunked"`; that's what "best" maps to.
    pub group_id: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f64>,
    pub codecs: Option<String>,
    pub bandwidth: Option<u64>,
    /// The media-playlist URL (absolute Twitch CDN / weaver URL). This is exactly
    /// what Streamlink printed under `--stream-url`.
    pub url: String,
}

/// Result of a native live resolution, shaped to fill `StreamStartResult`.
#[derive(Debug, Clone)]
pub struct ResolvedLive {
    /// Selected variant's media-playlist URL (what `StreamServer` should relay).
    pub url: String,
    /// The quality label actually served (sentinel kept for best/worst/audio_only;
    /// the matched variant name for numeric requests, so fallback notices fire).
    pub quality: String,
    /// Quality menu (variant names + best/worst), sorted highest-first.
    pub available: Vec<String>,
    /// Entitlement / proxy decision, for the UI ad-source badge.
    pub status: PlaybackStatus,
    /// The master playlist we parsed (kept for the agreement harness / debugging).
    /// A vertical result holds the NARROWED portrait text, so everything that
    /// re-parses it (`link_adapt`'s ladder, the relay) sees one picture only.
    pub master: String,
    /// The picture this resolution serves (SN-40). Landscape unless a
    /// vertical one was asked for and v2 had a playable portrait group.
    pub view: View,
    /// v2 had a playable portrait group by the time this resolved, whichever
    /// picture is served. Always false when v2 was not asked for.
    pub vertical_available: bool,
    /// The served ladder's video tiers are taller than wide; None when no
    /// tier says.
    pub portrait: Option<bool>,
}

/// What a phone's solo start asks of the v2 usher (SN-40). A resolve without
/// it never fetches v2: tiles, Cast, the ad pivot, the soaks, the desktop.
#[derive(Debug, Clone, Copy)]
pub struct DualAsk {
    /// The view the start wants (Remembered already resolved).
    pub want: View,
    /// The screen-off audio swap: never waits for v2.
    pub audio_only: bool,
}

/// A v2 answer still on its way when the start returned. `settle_late_v2`
/// reads it once the start has answered.
pub struct LateV2 {
    fetch: auth_proxy::DualFetch,
    /// The dual-format note's key (a channel, or a VOD).
    key: String,
    /// The codec preference the portrait group must be playable under.
    pref: Option<Vec<String>>,
}

impl LateV2 {
    /// Nobody will read this answer.
    pub fn abort(self) {
        self.fetch.abort();
    }
}

/// The preference `playable_variants` filters with on `channel`, or None
/// while it keeps every tier (nothing reported, nothing failed).
fn selection_pref(channel: &str) -> Option<Vec<String>> {
    (codec_pref_reported() || !failed_for(channel).is_empty()).then(|| pref_for(channel))
}

/// Whether a v2 master carries a portrait picture this device can play (X9):
/// a dual-format master (tagged portrait AND tagged landscape video, never by
/// shape) with at least one portrait video tier that survives the codec
/// preference (the iPhone's decodable filter rides `parse_master`, Android's
/// codec holds ride `pref`). `pref` None keeps every tier, as
/// `playable_variants` does before the player reports.
pub(crate) fn vertical_playable(v2: &str, pref: Option<&[String]>) -> bool {
    if !twitch_view::is_dual_format(v2) {
        return false;
    }
    parse_master(&twitch_view::narrow(v2, View::Vertical))
        .iter()
        .any(|v| v.height.is_some() && pref.is_none_or(|p| decodable(v, p)))
}

/// What v2 said by the time a start serves: its master when it answered, the
/// availability of its portrait group, and the fetch itself when it is still
/// running (the start does not wait for it; see `twitch_view::v2_wait_until`).
/// Every answer replaces the channel's dual-format note.
async fn read_v2(
    key: &str,
    dual: Option<auth_proxy::DualFetch>,
    ask: Option<DualAsk>,
    pref: Option<&[String]>,
) -> (Option<String>, bool, Option<auth_proxy::DualFetch>) {
    let (Some(ask), Some(mut fetch)) = (ask, dual) else {
        return (None, false, None);
    };
    let v1_done = Instant::now();
    let until = twitch_view::v2_wait_until(ask.want, twitch_view::dual_note(key), ask.audio_only, fetch.started, v1_done);
    match tokio::time::timeout(until.saturating_duration_since(Instant::now()), &mut fetch.handle).await {
        Ok(joined) => {
            let master = joined.ok().flatten();
            let available = master.as_deref().is_some_and(|m| vertical_playable(m, pref));
            if master.is_some() {
                twitch_view::note_dual(key, available);
            }
            (master, available, None)
        }
        Err(_) => (None, false, Some(fetch)),
    }
}

/// A late v2 answer, read after the start returned: whether its portrait
/// picture is playable (None when v2 never answered). Replaces the note.
pub async fn settle_late_v2(late: LateV2) -> Option<bool> {
    let master = late.fetch.handle.await.ok().flatten()?;
    let available = vertical_playable(&master, late.pref.as_deref());
    twitch_view::note_dual(&late.key, available);
    Some(available)
}

/// Parse a Twitch master playlist into its video renditions.
///
/// Anchored on `#EXT-X-STREAM-INF` (one per rendition), each paired with the next
/// non-comment URI line. This mirrors what Streamlink's HLS parser did and is the
/// reason it tolerated both master layouts Twitch ships:
///   - Legacy: a `#EXT-X-MEDIA:TYPE=VIDEO,NAME="1080p60",GROUP-ID="chunked"` tag
///     precedes each `#EXT-X-STREAM-INF` (the label lives in the MEDIA tag, linked
///     by the STREAM-INF's `VIDEO="..."` group).
///   - Modern (the "Transcode-ELT" / IVS stack served over the web/proxy path):
///     no `#EXT-X-MEDIA` tags at all; the label rides on the STREAM-INF itself as
///     `IVS-NAME="1080p60"` / `STABLE-VARIANT-ID` and the source rendition is
///     flagged `IVS-VARIANT-SOURCE="source"` instead of `GROUP-ID="chunked"`.
///
/// The previous version keyed off `#EXT-X-MEDIA:TYPE=VIDEO` and so silently parsed
/// to zero variants on the modern, MEDIA-less masters — the "master playlist had
/// no variants" failure.
pub fn parse_master(master: &str) -> Vec<Variant> {
    let lines: Vec<&str> = master.lines().collect();

    // Pass 1: legacy `#EXT-X-MEDIA:TYPE=VIDEO` renditions, keyed GROUP-ID → NAME.
    // Empty on modern masters; that's fine, the STREAM-INF carries the label there.
    let mut media_names: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    for line in &lines {
        let l = line.trim();
        if l.starts_with("#EXT-X-MEDIA:") && l.contains("TYPE=VIDEO") {
            if let Some(gid) = auth_proxy::extract_attr(l, "GROUP-ID") {
                let name = auth_proxy::extract_attr(l, "NAME").unwrap_or_default();
                media_names.insert(gid, name);
            }
        }
    }

    // Pass 2: every `#EXT-X-STREAM-INF` + its following URI line is one variant.
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let inf = lines[i].trim();
        if !inf.starts_with("#EXT-X-STREAM-INF:") {
            i += 1;
            continue;
        }
        // The URL is the next non-comment, non-empty line.
        let mut j = i + 1;
        while j < lines.len() {
            let u = lines[j].trim();
            if u.is_empty() || u.starts_with('#') {
                j += 1;
                continue;
            }
            break;
        }
        if j >= lines.len() {
            break; // dangling STREAM-INF with no URL; nothing more to parse
        }

        let (width, height) = parse_resolution(inf);
        let fps = auth_proxy::extract_attr(inf, "FRAME-RATE").and_then(|s| s.parse().ok());
        let codecs = auth_proxy::extract_attr(inf, "CODECS");
        let bandwidth = auth_proxy::extract_attr(inf, "BANDWIDTH").and_then(|s| s.parse().ok());
        let video_group = auth_proxy::extract_attr(inf, "VIDEO");

        // The source ("best") rendition: legacy `GROUP-ID="chunked"` (via the
        // STREAM-INF's VIDEO link) or the modern `IVS-VARIANT-SOURCE="source"`.
        let is_source = video_group.as_deref() == Some("chunked")
            || auth_proxy::extract_attr(inf, "IVS-VARIANT-SOURCE")
                .is_some_and(|s| s.eq_ignore_ascii_case("source"));

        // Quality label, in priority: legacy MEDIA NAME (by group) → modern
        // IVS-NAME → STABLE-VARIANT-ID → derived from resolution → group id.
        let name = video_group
            .as_ref()
            .and_then(|g| media_names.get(g).cloned())
            .filter(|n| !n.is_empty())
            .or_else(|| auth_proxy::extract_attr(inf, "IVS-NAME"))
            .or_else(|| auth_proxy::extract_attr(inf, "STABLE-VARIANT-ID"))
            .or_else(|| derive_name(height, fps, codecs.as_deref()))
            .or_else(|| video_group.clone())
            .unwrap_or_default();

        let group_id = if is_source {
            "chunked".to_string()
        } else {
            video_group.clone().unwrap_or_default()
        };

        out.push(Variant {
            name,
            group_id,
            width,
            height,
            fps,
            codecs,
            bandwidth,
            url: lines[j].trim().to_string(),
        });
        i = j + 1;
    }
    // The iPhone never sees a tier it cannot decode in hardware: not in the
    // quality menu, not for `best`, not as a fallback.
    #[cfg(target_os = "ios")]
    {
        let pref = effective_codec_pref();
        out.retain(|v| decodable(v, &pref));
    }
    out
}

/// Synthesize a quality label when the master carries no explicit name (neither a
/// legacy MEDIA NAME nor a modern IVS label). Resolution-bearing renditions become
/// `"1080p60"` / `"480p"`; a resolution-less audio rendition becomes `"audio_only"`.
fn derive_name(height: Option<u32>, fps: Option<f64>, codecs: Option<&str>) -> Option<String> {
    match height {
        Some(h) => {
            let f = fps.unwrap_or(30.0).round() as u32;
            // Twitch suffixes the framerate only for high-fps tiers ("1080p60"),
            // leaving 30fps tiers bare ("480p"), matching its own NAME shapes.
            if f >= 50 {
                Some(format!("{}p{}", h, f))
            } else {
                Some(format!("{}p", h))
            }
        }
        None => {
            let audio = codecs.is_some_and(|c| {
                c.contains("mp4a")
                    && !c.contains("avc")
                    && !c.contains("av01")
                    && !c.contains("hvc")
                    && !c.contains("hev")
            });
            audio.then(|| "audio_only".to_string())
        }
    }
}

/// Parse `RESOLUTION=1920x1080` into `(Some(1920), Some(1080))`.
fn parse_resolution(inf_line: &str) -> (Option<u32>, Option<u32>) {
    match auth_proxy::extract_attr(inf_line, "RESOLUTION") {
        Some(res) => {
            let mut parts = res.split(['x', 'X']);
            let w = parts.next().and_then(|p| p.trim().parse().ok());
            let h = parts.next().and_then(|p| p.trim().parse().ok());
            (w, h)
        }
        None => (None, None),
    }
}

/// The quality menu the player should show: every variant name plus the
/// `best`/`worst` aliases Streamlink synthesizes, sorted highest-first using the
/// existing ordering logic.
pub fn quality_names(variants: &[Variant]) -> Vec<String> {
    let mut names: Vec<String> = variants.iter().map(|v| v.name.clone()).collect();
    if !names.iter().any(|n| n.eq_ignore_ascii_case("best")) {
        names.push("best".to_string());
    }
    if !names.iter().any(|n| n.eq_ignore_ascii_case("worst")) {
        names.push("worst".to_string());
    }
    // The desktop offers Auto as its own choice beside the fixed source
    // ("best"); on the phones "best" is their Auto.
    if !cfg!(mobile) && !names.iter().any(|n| n.eq_ignore_ascii_case("auto")) {
        names.push("auto".to_string());
    }
    sort_qualities_descending(&mut names);
    names
}

/// Map a requested quality to a variant index plus the label actually served.
///
/// Mirrors Streamlink's behavior: `best`/`source` → the source ("chunked")
/// rendition (highest), `auto` → the same rendition to start on (the link
/// decides from there, `link_adapt`), `worst` → lowest video, `audio_only` →
/// the audio rendition, and a numeric request goes through
/// `pick_closest_quality` so the two Twitch naming shapes ("480p" vs "480p30")
/// reconcile against the user's saved preference. Returns
/// `(index, label_to_report)`.
pub fn select_variant(variants: &[Variant], requested: &str) -> Option<(usize, String)> {
    select_variant_with(variants, requested, &effective_codec_pref())
}

/// `select_variant` on `channel`: never a codec that failed to play there.
pub fn select_variant_for(variants: &[Variant], requested: &str, channel: &str) -> Option<(usize, String)> {
    select_variant_with(variants, requested, &pref_for(channel))
}

fn select_variant_with(variants: &[Variant], requested: &str, pref: &[String]) -> Option<(usize, String)> {
    if variants.is_empty() {
        return None;
    }
    let req = requested.trim().to_lowercase();

    if req == "best" || req == "source" {
        return best_index_with(variants, pref).map(|i| (i, "best".to_string()));
    }
    if req == "auto" {
        return best_index_with(variants, pref).map(|i| (i, "auto".to_string()));
    }
    if req == "worst" {
        return worst_index(variants).map(|i| (i, "worst".to_string()));
    }
    if req == "audio_only" || req == "audio-only" || req == "audio" {
        return audio_index(variants)
            .map(|i| (i, "audio_only".to_string()))
            .or_else(|| best_index_with(variants, pref).map(|i| (i, "best".to_string())));
    }

    // Numeric request: reuse the exact matching the Streamlink path used.
    let names: Vec<String> = variants.iter().map(|v| v.name.clone()).collect();
    let idx = match portrait_numeric_index(variants, &req, &names) {
        Some(idx) => idx,
        None => {
            let chosen = pick_closest_quality(&req, &names)?;
            variants.iter().position(|v| v.name.eq_ignore_ascii_case(&chosen))?
        }
    };
    // At the matched resolution, prefer the most-efficient decodable codec (same
    // capability gate as "best"); keep the matched rendition when none is preferable.
    let idx = prefer_codec_at(variants, variants[idx].height, variants[idx].fps, pref).unwrap_or(idx);
    Some((idx, variants[idx].name.clone()))
}

/// A video tier's short edge (its height on a landscape tier, its width on a
/// portrait one), which is what a tier is measured by against the display and
/// against another tier. None for audio.
fn short_edge(v: &Variant) -> Option<u32> {
    twitch_view::short_edge(v.width, v.height)
}

/// Tiers ordered by (short edge, fps, bandwidth). On a landscape ladder the
/// short edge is the height, so this is the order selection always used.
fn by_short_edge(a: &Variant, b: &Variant) -> std::cmp::Ordering {
    short_edge(a)
        .cmp(&short_edge(b))
        .then(a.fps.partial_cmp(&b.fps).unwrap_or(std::cmp::Ordering::Equal))
        .then(a.bandwidth.cmp(&b.bandwidth))
}

/// The source rendition (codec-agnostic): the one video tier Twitch flags
/// (`GROUP-ID="chunked"` / `IVS-VARIANT-SOURCE="source"`). When several are
/// flagged (the v2 usher flags every tier, in a shuffled order) or none is,
/// the highest (short edge, fps, bandwidth) among the flagged ones, else among
/// every video tier.
fn source_index(variants: &[Variant]) -> Option<usize> {
    let flagged: Vec<usize> = variants
        .iter()
        .enumerate()
        .filter(|(_, v)| v.height.is_some() && v.group_id.eq_ignore_ascii_case("chunked"))
        .map(|(i, _)| i)
        .collect();
    if let [only] = flagged.as_slice() {
        return Some(*only);
    }
    let pool: Vec<usize> = if flagged.is_empty() {
        (0..variants.len()).filter(|&i| variants[i].height.is_some()).collect()
    } else {
        flagged
    };
    pool.into_iter()
        .max_by(|&a, &b| by_short_edge(&variants[a], &variants[b]))
        .or(Some(0))
}

/// Whether a ladder's video tiers are taller than wide.
fn ladder_is_portrait(variants: &[Variant]) -> bool {
    twitch_view::ladder_portrait(variants.iter().map(|v| (v.width, v.height))) == Some(true)
}

/// A numeric request on a portrait ladder, matched by the tiers' short edge
/// and fps rather than their names. A vertical-only stream on the legacy v1
/// shape names its rungs by height (`720p` is 404x720, its 720x1280 source is
/// `1280p (source)`), so a saved "720p60" means its 720-wide tier, not the
/// 404-wide one. A tier named exactly as asked (a pick from this ladder's own
/// menu) is still that tier. None on a landscape ladder, which keeps matching
/// by name.
fn portrait_numeric_index(variants: &[Variant], req: &str, names: &[String]) -> Option<usize> {
    if !ladder_is_portrait(variants) || names.iter().any(|n| n.eq_ignore_ascii_case(req)) {
        return None;
    }
    let video: Vec<usize> = (0..variants.len()).filter(|&i| short_edge(&variants[i]).is_some()).collect();
    let labels: Vec<String> = video
        .iter()
        .map(|&i| short_edge_label(short_edge(&variants[i]).unwrap_or(0), variants[i].fps))
        .collect();
    pick_closest_index(req, &labels).map(|at| video[at])
}

/// "best" = the source quality, but in the most-preferred DECODABLE codec offered at
/// that same resolution. This is the `enhanced_codecs` win: AV1/HEVC are more
/// efficient at equal resolution and ship as CMAF, so picking them also routes the
/// stream through the low-latency origin. It never trades resolution for codec.
/// The resolver hands it `playable_variants`, so a source tier in a codec this
/// device cannot play is already gone and the tallest playable tier is the source.
fn best_index(variants: &[Variant]) -> Option<usize> {
    best_index_with(variants, &effective_codec_pref())
}

fn best_index_with(variants: &[Variant], pref: &[String]) -> Option<usize> {
    let src = capped_source_index(variants, effective_max_height())?;
    let (h, f) = (variants[src].height, variants[src].fps);
    Some(prefer_codec_at(variants, h, f, pref).unwrap_or(src))
}

/// `source_index`, but never taller than `cap`.
///
/// A phone was decoding a 2560x1440 rendition onto a display whose short edge is
/// 1080 - more than five times the pixels the portrait player band can show, at
/// roughly double the bitrate. Nothing in the resolver had any notion of the
/// screen it was drawing on, and hls.js cannot compensate because StreamNook
/// resolves to a SINGLE variant before the player ever sees a manifest, so there
/// is no ABR ladder left for `capLevelToPlayerSize` to act on.
///
/// This deliberately short-circuits the `chunked` fast path in `source_index`:
/// "chunked" IS the oversized source, so honouring it first would defeat the cap
/// on exactly the streams that need it.
///
/// Only the "best"/auto path reaches here. An explicitly requested rendition is
/// name-matched in `select_variant` and bypasses the cap entirely, which is what
/// makes the quality menu a real override.
fn capped_source_index(variants: &[Variant], cap: Option<u32>) -> Option<usize> {
    let Some(cap) = cap else {
        return source_index(variants);
    };

    // `height.is_some()` throughout is load-bearing: audio_only carries no
    // resolution and sits at index 0 in modern masters, so a bare `first()` or
    // an `.or(Some(0))` fallback here would silently serve an audio-only stream
    // on any ladder whose every tier is above the cap.
    //
    // A tier is measured by its short edge, like the display: a 1080x1920
    // portrait tier fits a 1080 cap, where its height would read as 1920.
    let ordering = by_short_edge;

    // The highest rendition that fits.
    if let Some((i, _)) = variants
        .iter()
        .enumerate()
        .filter(|(_, v)| short_edge(v).is_some_and(|e| e <= cap))
        .max_by(|(_, a), (_, b)| ordering(a, b))
    {
        return Some(i);
    }

    // Nothing fits: take the LOWEST rendition above the cap, not the highest.
    // Falling back to the highest would re-select the very source tier the cap
    // exists to avoid.
    if let Some((i, _)) = variants
        .iter()
        .enumerate()
        .filter(|(_, v)| v.height.is_some())
        .min_by(|(_, a), (_, b)| ordering(a, b))
    {
        return Some(i);
    }

    // No video renditions at all (an audio-only master). Defer to the uncapped
    // path rather than inventing a choice.
    source_index(variants)
}

/// "worst" = the lowest video rendition by short edge, then fps (audio_only has
/// no resolution and is excluded). Falls back to the first rendition if none
/// carry a resolution.
fn worst_index(variants: &[Variant]) -> Option<usize> {
    variants
        .iter()
        .enumerate()
        .filter(|(_, v)| v.height.is_some())
        .min_by(|(_, a), (_, b)| {
            short_edge(a).cmp(&short_edge(b)).then(
                a.fps
                    .partial_cmp(&b.fps)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
        })
        .map(|(i, _)| i)
        .or(Some(0))
}

fn audio_index(variants: &[Variant]) -> Option<usize> {
    variants
        .iter()
        .position(|v| v.name.to_lowercase().contains("audio"))
}

/// Resolve a live channel to a media-playlist URL, fully in Rust.
///
/// Entitlement-first: a Turbo or channel-subscribed viewer gets the
/// authenticated master directly (ad-free, full quality, credits the
/// streamer). Everyone else gets the same direct master the web player would
/// serve them, authenticated when logged in and anonymous otherwise. The core
/// applies no ad handling to it; a playback plugin that owns resolution can
/// take over the non-entitled case at the command layer (see
/// `commands::streaming::resolve_via_plugin`).
pub async fn resolve_live(
    channel: &str,
    oauth_token: Option<&str>,
    quality: &str,
) -> Result<ResolvedLive> {
    resolve_live_with(channel, oauth_token, quality, None).await.map(|(r, _)| r)
}

/// `resolve_live`, and with `ask` (a phone's solo start, SN-40) also the v2
/// master on the same token: a vertical start whose v2 has a playable
/// portrait group is built from that group alone. A v2 still on its way when
/// the start serves comes back as `LateV2`.
pub async fn resolve_live_with(
    channel: &str,
    oauth_token: Option<&str>,
    quality: &str,
    ask: Option<DualAsk>,
) -> Result<(ResolvedLive, Option<LateV2>)> {
    let channel = channel.to_lowercase();

    // Entitlement-first (only when logged in).
    if let Some(token) = oauth_token {
        let reason = if auth_proxy::account_has_turbo(token).await {
            Some("turbo")
        } else if auth_proxy::is_subscribed(&channel, token).await {
            Some("subscribed")
        } else {
            None
        };
        if let Some(reason) = reason {
            match auth_proxy::fetch_auth_masters(&channel, Some(token), ask.is_some()).await {
                Ok((master, dual)) => {
                    debug!(
                        "[Resolver] {} entitled ({}) → authed master direct",
                        channel, reason
                    );
                    let status = PlaybackStatus {
                        channel: channel.clone(),
                        mode: reason.to_string(),
                        entitled: true,
                        proxy_base: None,
                        proxy_region: None,
                        reason: None,
                    };
                    return build_live(channel, master, dual, status, quality, ask).await;
                }
                Err(e) => debug!(
                    "[Resolver] {} entitled ({}) but auth master failed ({}); falling through",
                    channel, reason, e
                ),
            }
        }
    }

    // Non-entitled: the direct master, with the viewer's own credential when
    // present and anonymously otherwise (same as the logged-out web player).
    let (master, dual) = auth_proxy::fetch_auth_masters(&channel, oauth_token, ask.is_some()).await?;
    let status = PlaybackStatus {
        channel: channel.clone(),
        mode: "auth-only".to_string(),
        entitled: false,
        proxy_base: None,
        proxy_region: None,
        reason: None,
    };
    build_live(channel, master, dual, status, quality, ask).await
}

/// Build a live result from the v1 master and, when asked, what v2 said: the
/// portrait group when the start wants Vertical and v2 has a playable one,
/// else the v1 master exactly as before (landscape never depends on v2).
async fn build_live(
    channel: String,
    v1: String,
    dual: Option<auth_proxy::DualFetch>,
    status: PlaybackStatus,
    quality: &str,
    ask: Option<DualAsk>,
) -> Result<(ResolvedLive, Option<LateV2>)> {
    let pref = selection_pref(&channel);
    let (v2, available, late) = read_v2(&channel, dual, ask, pref.as_deref()).await;
    let late = late.map(|fetch| LateV2 { fetch, key: channel.clone(), pref });
    let want = ask.map_or(View::Landscape, |a| a.want);
    build_served(channel, v1, v2, available, status, quality, want).map(|r| (r, late))
}

/// The served picture, given what v2 said: the narrowed portrait group when
/// `want` is Vertical and v2 had a playable one, else the v1 master as is. A
/// failed or late v2 (`v2` None) is landscape and never an error.
fn build_served(
    channel: String,
    v1: String,
    v2: Option<String>,
    available: bool,
    status: PlaybackStatus,
    quality: &str,
    want: View,
) -> Result<ResolvedLive> {
    let vertical = match v2 {
        Some(v2) if available && want == View::Vertical => {
            match build(channel.clone(), twitch_view::narrow(&v2, View::Vertical), status.clone(), quality, View::Vertical) {
                Ok(r) => Some(r),
                Err(e) => {
                    debug!("[Resolver] {} portrait group unusable ({}); serving landscape", channel, e);
                    None
                }
            }
        }
        _ => None,
    };
    let mut r = match vertical {
        Some(r) => r,
        None => build(channel, v1, status, quality, View::Landscape)?,
    };
    r.vertical_available = available;
    Ok(r)
}

/// Build a `ResolvedLive` from a master playlist a playback plugin supplied
/// through the `playback.resolve` hook. Same parse and variant selection as a
/// core-fetched master; only the status provenance differs.
pub fn resolve_from_master(
    channel: &str,
    master: String,
    quality: &str,
    base: Option<String>,
    region: Option<String>,
) -> Result<ResolvedLive> {
    let channel = channel.to_lowercase();
    let status = PlaybackStatus {
        channel: channel.clone(),
        mode: "plugin".to_string(),
        entitled: false,
        proxy_base: base,
        proxy_region: region,
        reason: None,
    };
    // A plugin or relay master is v1-shaped: landscape (an untagged ladder).
    build(channel, master, status, quality, View::Landscape)
}

/// Retry `resolve_live` until it succeeds or the time budget elapses — the
/// native equivalent of Streamlink's `--retry-streams` (which retries fetching
/// the stream list, waiting `retry_delay_secs` between attempts). This restores
/// the "open a channel the instant it goes live and connect once the playlist
/// appears" behavior: Twitch's GQL live-status can lag the real playlist by up
/// to a minute, so the first resolve attempt may legitimately find nothing.
///
/// `retry_delay_secs == 0` makes a single attempt (Streamlink's default of no
/// retries). `budget_secs` caps the total wait — Streamlink's retry is uncapped
/// unless `--retry-max` is set, and StreamNook historically bounded it with the
/// subprocess timeout, so we reuse `stream_timeout` as that ceiling. The first
/// attempt always runs even if the budget is tiny.
pub async fn resolve_live_resilient(
    channel: &str,
    oauth_token: Option<&str>,
    quality: &str,
    retry_delay_secs: u32,
    budget_secs: u32,
) -> Result<ResolvedLive> {
    resolve_live_resilient_with(channel, oauth_token, quality, retry_delay_secs, budget_secs, None)
        .await
        .map(|(r, _)| r)
}

/// `resolve_live_resilient` with a phone solo start's v2 ask: every retry
/// asks v2 again beside v1 (a stream that just went live).
pub async fn resolve_live_resilient_with(
    channel: &str,
    oauth_token: Option<&str>,
    quality: &str,
    retry_delay_secs: u32,
    budget_secs: u32,
    ask: Option<DualAsk>,
) -> Result<(ResolvedLive, Option<LateV2>)> {
    let start = Instant::now();
    let mut attempt = 0u32;
    let mut budget_secs = budget_secs;
    let mut asked_live = false;
    loop {
        attempt += 1;
        match resolve_live_with(channel, oauth_token, quality, ask).await {
            Ok(r) => return Ok(r),
            Err(e) => {
                // No playlist, and Twitch says no stream either: the channel
                // is offline, so a short wait covers a stream that is starting
                // instead of the whole budget. Asked once per start.
                if retry_delay_secs > 0 && !asked_live && is_playlist_not_found(&e) {
                    asked_live = true;
                    let login = channel.to_lowercase();
                    match crate::services::twitch_service::TwitchService::get_live_public(&[], &[login]).await {
                        Ok(rows) => budget_secs = retry_budget_when_offline(budget_secs, !rows.is_empty()),
                        Err(err) => debug!("[Resolver] {} live check failed ({}); keeping the full retry budget", channel, err),
                    }
                }
                let elapsed = start.elapsed().as_secs() as u32;
                // Stop if retries are disabled, or sleeping again would exceed
                // the budget.
                if retry_delay_secs == 0 || elapsed.saturating_add(retry_delay_secs) >= budget_secs
                {
                    return Err(e);
                }
                debug!(
                    "[Resolver] {} resolve attempt {} failed ({}); retrying in {}s",
                    channel, attempt, e, retry_delay_secs
                );
                tokio::time::sleep(Duration::from_secs(retry_delay_secs as u64)).await;
            }
        }
    }
}

/// Seconds a start keeps retrying a channel Twitch says is offline: enough for
/// a stream that is just starting to publish its playlist, far short of the
/// full budget an offline channel used to wait out.
pub const OFFLINE_RETRY_BUDGET_SECS: u32 = 8;

/// The usher answer for a channel with no playlist (offline, or not started).
fn is_playlist_not_found(e: &anyhow::Error) -> bool {
    format!("{e:#}").contains("usher returned 404")
}

/// The retry budget once Twitch has said whether the channel is live.
pub fn retry_budget_when_offline(budget_secs: u32, live: bool) -> u32 {
    if live {
        budget_secs
    } else {
        budget_secs.min(OFFLINE_RETRY_BUDGET_SECS)
    }
}

/// Parse a fetched master, select the variant, and assemble the result.
/// `view` is the picture `master` holds (a vertical master is already
/// narrowed to its portrait group).
fn build(
    channel: String,
    master: String,
    status: PlaybackStatus,
    quality: &str,
    view: View,
) -> Result<ResolvedLive> {
    let variants = playable_variants(&master, &channel);
    if variants.is_empty() {
        // Log enough to tell apart the failure modes (empty body, error page,
        // unrecognized master shape) without dumping the whole playlist.
        let first = master.lines().next().unwrap_or("").trim();
        debug!(
            "[Resolver] {} no variants: {} bytes, stream-inf={}, first line: {:?}",
            channel,
            master.len(),
            master.matches("#EXT-X-STREAM-INF").count(),
            first
        );
        return Err(anyhow!("{}: master playlist had no variants", channel));
    }
    let (idx, label) = select_variant_for(&variants, quality, &channel)
        .ok_or_else(|| anyhow!("{}: no variant matched '{}'", channel, quality))?;
    let available = quality_names(&menu_variants(&master, &channel));
    let portrait = twitch_view::ladder_portrait(variants.iter().map(|v| (v.width, v.height)));
    Ok(ResolvedLive {
        url: variants[idx].url.clone(),
        quality: label,
        available,
        status,
        master,
        view,
        vertical_available: false,
        portrait,
    })
}

// ----- VOD + clip resolution (Phase 2) -----

/// A resolved VOD or clip: the URL the player should load plus the quality menu.
/// VODs resolve to an HLS media-playlist URL (served through `StreamServer` like
/// live); clips resolve to a signed MP4 URL the player loads directly.
/// Where a clip sits inside its source broadcast. Used ONLY to address chat replay
/// (Twitch's comment API is video-id + offset; there is no clip-addressed one). The
/// clip still plays from its own signed MP4 — none of this touches playback. Every
/// field is optional because a clip whose parent VOD has expired keeps playing fine,
/// it just has no chat to show.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ClipSource {
    pub video_id: Option<String>,
    pub vod_offset: Option<u32>,
    pub duration: Option<f32>,
    pub broadcaster_login: Option<String>,
}

/// The Twitch category a clip or VOD was recorded under, named the way Helix
/// names a live stream's (`game_id` / `game_name`), so the player can show
/// and open it like a live one.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MediaCategory {
    pub id: String,
    pub name: String,
}

/// A GQL `game { id name }` object as a category. A video with no category
/// comes back `null`, and one without a name has nothing to show, so both
/// give None.
pub fn media_category(game: Option<&Value>) -> Option<MediaCategory> {
    let game = game.filter(|g| !g.is_null())?;
    let name = str_field(game, "name")?.trim().to_string();
    if name.is_empty() {
        return None;
    }
    Some(MediaCategory {
        id: str_field(game, "id").unwrap_or_default(),
        name,
    })
}

#[derive(Debug, Clone)]
pub struct ResolvedMedia {
    pub url: String,
    pub quality: String,
    pub available: Vec<String>,
    /// Clip resolution only; None for a live stream or a VOD.
    pub clip_source: Option<ClipSource>,
    /// Clip resolution only (a VOD's comes from `fetch_vod_info`).
    pub category: Option<MediaCategory>,
    /// The picture a VOD serves (SN-40); Landscape for a clip.
    pub view: View,
    /// The VOD's v2 had a playable portrait group (false for a clip).
    pub vertical_available: bool,
    /// The served ladder is taller than wide; None for a clip (its qualities
    /// carry no dimensions).
    pub portrait: Option<bool>,
}

/// Extract the numeric VOD id from a `.../videos/123456789` URL.
pub fn vod_id_from_url(url: &str) -> Option<String> {
    let after = url.split("/videos/").nth(1)?;
    let id: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
    if id.is_empty() {
        None
    } else {
        Some(id)
    }
}

/// Extract a clip slug from either `clips.twitch.tv/SLUG` or
/// `twitch.tv/<channel>/clip/SLUG` (with optional query/fragment).
pub fn clip_slug_from_url(url: &str) -> Option<String> {
    let raw = url
        .split("clips.twitch.tv/")
        .nth(1)
        .or_else(|| url.split("/clip/").nth(1))?;
    let slug = raw.split(['/', '?', '#']).next()?.trim();
    // Guard against the `clips.twitch.tv/embed?clip=...` shape landing here as
    // "embed"; that form carries the slug in the query and isn't handled.
    if slug.is_empty() || slug.eq_ignore_ascii_case("embed") {
        None
    } else {
        Some(slug.to_string())
    }
}

/// GQL `PlaybackAccessToken` (VOD variant) → usher `/vod/{id}` master playlist.
/// Same persisted-query hash as live; the response carries
/// `videoPlaybackAccessToken` (not `streamPlaybackAccessToken`) and usher uses
/// the `nauthsig`/`nauth` param names. No client-integrity needed for VODs.
/// With `want_dual` the `/vod/v2/` master (every ladder, SN-40) is fetched
/// beside it on the same token.
async fn fetch_vod_masters(
    vod_id: &str,
    oauth_token: Option<&str>,
    want_dual: bool,
) -> Result<(String, Option<auth_proxy::DualFetch>)> {
    let client = auth_proxy::web_client();

    let gql_body = json!({
        "operationName": "PlaybackAccessToken",
        "extensions": {
            "persistedQuery": {
                "version": 1,
                "sha256Hash": auth_proxy::PLAYBACK_ACCESS_TOKEN_HASH,
            }
        },
        "variables": {
            "isLive": false,
            "login": "",
            "isVod": true,
            "vodID": vod_id,
            "playerType": "embed",
            "platform": "web",
        }
    });

    let mut req = client
        .post("https://gql.twitch.tv/gql")
        .header("Client-ID", auth_proxy::TWITCH_WEB_CLIENT_ID)
        .json(&gql_body);
    if let Some(t) = oauth_token {
        req = req.header("Authorization", format!("OAuth {}", t));
    }
    let gql_resp: Value = req
        .send()
        .await
        .context("VOD GQL request failed")?
        .json()
        .await
        .context("VOD GQL response not JSON")?;

    let pat = gql_resp
        .pointer("/data/videoPlaybackAccessToken")
        .ok_or_else(|| anyhow!("GQL missing videoPlaybackAccessToken: {}", gql_resp))?;
    let sig = pat
        .get("signature")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("no VOD sig"))?;
    let value = pat
        .get("value")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("no VOD token value"))?;

    let query = vod_usher_query(sig, value);
    let usher = format!("https://usher.ttvnw.net/vod/{}?{}", vod_id, query);
    let dual = want_dual.then(|| {
        auth_proxy::spawn_v2(
            format!("https://usher.ttvnw.net/vod/v2/{}.m3u8?{}&multigroup_video=true", vod_id, query),
            format!("VOD {vod_id}"),
        )
    });

    let fetched = async {
        let master = client
            .get(&usher)
            .header("Referer", "https://player.twitch.tv")
            .header("Origin", "https://player.twitch.tv")
            .send()
            .await
            .context("VOD usher request failed")?;
        if !master.status().is_success() {
            return Err(anyhow!("VOD usher returned {}", master.status()));
        }
        Ok::<String, anyhow::Error>(master.text().await?)
    };
    match fetched.await {
        Ok(master) => Ok((master, dual)),
        Err(e) => {
            if let Some(dual) = &dual {
                dual.abort();
            }
            Err(e)
        }
    }
}

/// The VOD usher query with this playback token (v1 and v2 send the same).
fn vod_usher_query(sig: &str, token: &str) -> String {
    format!(
        "platform=web&player_type=embed&allow_source=true\
         &allow_audio_only=true&playlist_include_framerate=true&supported_codecs=av1,h264,h265\
         &nauthsig={sig}&nauth={tok}",
        sig = sig,
        tok = urlencoding::encode(token),
    )
}

/// Resolve a VOD id to a media-playlist URL (parsed + variant-selected, same as
/// live). Served through `StreamServer` by the caller.
pub async fn resolve_vod(
    vod_id: &str,
    oauth_token: Option<&str>,
    quality: &str,
) -> Result<ResolvedMedia> {
    resolve_vod_with(vod_id, oauth_token, quality, None).await.map(|(r, _)| r)
}

/// `resolve_vod`, and with `ask` (a phone's VOD open, SN-40) also the
/// `/vod/v2/` master: a vertical open whose v2 has a playable portrait group
/// is built from that group alone, exactly as live.
pub async fn resolve_vod_with(
    vod_id: &str,
    oauth_token: Option<&str>,
    quality: &str,
    ask: Option<DualAsk>,
) -> Result<(ResolvedMedia, Option<LateV2>)> {
    let (v1, dual) = fetch_vod_masters(vod_id, oauth_token, ask.is_some()).await?;
    let key = twitch_view::vod_key(vod_id);
    let pref = selection_pref("");
    let (v2, available, late) = read_v2(&key, dual, ask, pref.as_deref()).await;
    let late = late.map(|fetch| LateV2 { fetch, key, pref });
    let (master, view) = match v2 {
        Some(v2) if available && ask.is_some_and(|a| a.want == View::Vertical) => {
            (twitch_view::narrow(&v2, View::Vertical), View::Vertical)
        }
        _ => (v1, View::Landscape),
    };
    let variants = playable_variants(&master, "");
    if variants.is_empty() {
        return Err(anyhow!("VOD {}: master playlist had no variants", vod_id));
    }
    let (idx, label) = select_variant(&variants, quality)
        .ok_or_else(|| anyhow!("VOD {}: no variant matched '{}'", vod_id, quality))?;
    let portrait = twitch_view::ladder_portrait(variants.iter().map(|v| (v.width, v.height)));
    Ok((
        ResolvedMedia {
            url: variants[idx].url.clone(),
            quality: label,
            available: quality_names(&variants),
            clip_source: None,
            category: None,
            view,
            vertical_available: available,
            portrait,
        },
        late,
    ))
}

/// Pick a clip quality. Clips ship a flat list of `<height>p<fps>` MP4 qualities
/// (no `chunked`/audio aliases), so best = highest, worst = lowest, numeric goes
/// through the shared closest-match.
fn pick_clip_quality(requested: &str, names: &[String]) -> Option<String> {
    let mut sorted = names.to_vec();
    sort_qualities_descending(&mut sorted);
    match requested.trim().to_lowercase().as_str() {
        "best" | "source" | "auto" => sorted
            .iter()
            .find(|q| crate::services::quality::parse_quality_height(q).is_some())
            .or(sorted.first())
            .cloned(),
        "worst" => sorted.last().cloned(),
        _ => pick_closest_quality(requested, names),
    }
}

/// Append the clip access token to a clip MP4 `sourceURL` as `sig`/`token` query
/// params (the clip equivalent of usher's signed playlist URL).
fn with_clip_token(src: &str, sig: &str, token: &str) -> String {
    let sep = if src.contains('?') { '&' } else { '?' };
    format!(
        "{}{}sig={}&token={}",
        src,
        sep,
        urlencoding::encode(sig),
        urlencoding::encode(token)
    )
}

/// Resolve a clip slug to a signed MP4 URL. Clips use the `VideoAccessToken_Clip`
/// GQL op (not usher/HLS): it returns the access token plus a list of MP4
/// `videoQualities`, and the chosen MP4 is signed by appending the token.
pub async fn resolve_clip(
    slug: &str,
    oauth_token: Option<&str>,
    quality: &str,
) -> Result<ResolvedMedia> {
    let client = auth_proxy::web_client();

    let body = json!({
        "operationName": "VideoAccessToken_Clip",
        "query": CLIP_ACCESS_TOKEN_QUERY,
        "variables": { "slug": slug }
    });

    let mut req = client
        .post("https://gql.twitch.tv/gql")
        .header("Client-ID", auth_proxy::TWITCH_WEB_CLIENT_ID)
        .json(&body);
    if let Some(t) = oauth_token {
        req = req.header("Authorization", format!("OAuth {}", t));
    }
    let resp: Value = req
        .send()
        .await
        .context("clip GQL request failed")?
        .json()
        .await
        .context("clip GQL response not JSON")?;

    // A GQL-level error (e.g. `PersistedQueryNotFound`, a rejected op, an auth
    // problem) nulls out `data`, which is a different failure from a clip that
    // genuinely doesn't exist. Surface it as-is so it isn't misread as "clip not
    // found".
    if let Some(errors) = resp.get("errors").and_then(|v| v.as_array()) {
        if !errors.is_empty() {
            let msg = errors
                .iter()
                .filter_map(|e| e.get("message").and_then(|m| m.as_str()))
                .collect::<Vec<_>>()
                .join("; ");
            return Err(anyhow!("clip GQL error for {}: {}", slug, msg));
        }
    }

    let clip = resp
        .pointer("/data/clip")
        .filter(|c| !c.is_null())
        .ok_or_else(|| anyhow!("clip not found: {}", slug))?;
    let sig = clip
        .pointer("/playbackAccessToken/signature")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("clip missing signature"))?;
    let token = clip
        .pointer("/playbackAccessToken/value")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("clip missing token value"))?;
    let qualities = clip
        .get("videoQualities")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow!("clip missing videoQualities"))?;

    // (name, sourceURL) pairs, named like the player UI ("1080p60").
    let mut variants: Vec<(String, String)> = Vec::new();
    for q in qualities {
        let src = q.get("sourceURL").and_then(|v| v.as_str()).unwrap_or("");
        let qual = q.get("quality").and_then(|v| v.as_str()).unwrap_or("");
        let fr = q.get("frameRate").and_then(|v| v.as_f64()).unwrap_or(0.0) as i64;
        if src.is_empty() || qual.is_empty() {
            continue;
        }
        variants.push((format!("{}p{}", qual, fr), src.to_string()));
    }
    if variants.is_empty() {
        return Err(anyhow!("clip {} had no playable qualities", slug));
    }

    let names: Vec<String> = variants.iter().map(|(n, _)| n.clone()).collect();
    let chosen = pick_clip_quality(quality, &names)
        .ok_or_else(|| anyhow!("clip {}: no quality matched '{}'", slug, quality))?;
    let src = variants
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(&chosen))
        .map(|(_, u)| u.clone())
        .ok_or_else(|| anyhow!("clip {}: chosen quality vanished", slug))?;

    let mut available = names;
    sort_qualities_descending(&mut available);
    // Chat-replay coordinates. Every field is best-effort: a clip whose parent VOD has
    // expired still plays, it just gets no chat.
    let clip_source = ClipSource {
        video_id: clip
            .pointer("/video/id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string()),
        vod_offset: clip
            .get("videoOffsetSeconds")
            .and_then(|v| v.as_u64())
            .map(|n| n as u32),
        duration: clip
            .get("durationSeconds")
            .and_then(|v| v.as_f64())
            .map(|n| n as f32),
        broadcaster_login: clip
            .pointer("/broadcaster/login")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_lowercase()),
    };

    Ok(ResolvedMedia {
        url: with_clip_token(&src, sig, token),
        quality: chosen,
        available,
        clip_source: Some(clip_source),
        category: media_category(clip.get("game")),
        view: View::Landscape,
        vertical_available: false,
        portrait: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_phone_selects_only_codecs_it_decodes_in_hardware() {
        let all = vec!["avc1".to_string(), "hvc1".to_string(), "av01".to_string()];
        assert_eq!(device_codec_preference(&all, true), ["h264", "hevc", "av1"]);
        let no_av1 = vec!["h264".to_string(), "hevc".to_string()];
        assert_eq!(device_codec_preference(&no_av1, true), ["h264", "hevc"]);
        assert_eq!(device_codec_preference(&["H265".to_string()], true), ["h264", "hevc"]);
        assert_eq!(device_codec_preference(&[], true), ["h264"]);
        assert_eq!(device_codec_preference(&all, false), ["h264"], "Enhanced Broadcasting off keeps H.264");
    }

    // A realistic Twitch master: source ("chunked") + standard tiers + audio_only,
    // in the nickmercs shape (fps suffix on every tier).
    const MASTER_NICKMERCS: &str = "#EXTM3U\n\
#EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID=\"chunked\",NAME=\"1080p60\",AUTOSELECT=YES,DEFAULT=YES\n\
#EXT-X-STREAM-INF:BANDWIDTH=8000000,RESOLUTION=1920x1080,CODECS=\"avc1.4D402A,mp4a.40.2\",VIDEO=\"chunked\",FRAME-RATE=60.000\n\
https://video-weaver.example/chunked.m3u8\n\
#EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID=\"720p60\",NAME=\"720p60\",AUTOSELECT=YES,DEFAULT=NO\n\
#EXT-X-STREAM-INF:BANDWIDTH=3500000,RESOLUTION=1280x720,CODECS=\"avc1.4D401F,mp4a.40.2\",VIDEO=\"720p60\",FRAME-RATE=60.000\n\
https://video-weaver.example/720p60.m3u8\n\
#EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID=\"480p30\",NAME=\"480p30\",AUTOSELECT=YES,DEFAULT=NO\n\
#EXT-X-STREAM-INF:BANDWIDTH=1500000,RESOLUTION=852x480,CODECS=\"avc1.4D401F,mp4a.40.2\",VIDEO=\"480p30\",FRAME-RATE=30.000\n\
https://video-weaver.example/480p30.m3u8\n\
#EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID=\"160p30\",NAME=\"160p30\",AUTOSELECT=YES,DEFAULT=NO\n\
#EXT-X-STREAM-INF:BANDWIDTH=300000,RESOLUTION=284x160,CODECS=\"avc1.4D400C,mp4a.40.2\",VIDEO=\"160p30\",FRAME-RATE=30.000\n\
https://video-weaver.example/160p30.m3u8\n\
#EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID=\"audio_only\",NAME=\"audio_only\",AUTOSELECT=NO,DEFAULT=NO\n\
#EXT-X-STREAM-INF:BANDWIDTH=160000,CODECS=\"mp4a.40.2\",VIDEO=\"audio_only\"\n\
https://video-weaver.example/audio_only.m3u8\n";

    // The modern "Transcode-ELT" / IVS master Twitch serves over the web/proxy
    // path: NO `#EXT-X-MEDIA` tags at all; the label rides on the STREAM-INF as
    // `IVS-NAME` and the source rendition is flagged `IVS-VARIANT-SOURCE="source"`.
    // Trimmed from a real `forsen` proxy response. The old parser parsed this to
    // zero variants ("master playlist had no variants").
    const MASTER_MODERN_IVS: &str = "#EXTM3U\n\
#EXT-X-SESSION-DATA:DATA-ID=\"NODE\",VALUE=\"x.cloudfront.hls.ttvnw.net\"\n\
#EXT-X-SESSION-DATA:DATA-ID=\"TRANSCODESTACK\",VALUE=\"2025-Transcode-ELT-V1\"\n\
#EXT-X-STREAM-INF:BANDWIDTH=160000,CODECS=\"mp4a.40.2\",STABLE-VARIANT-ID=\"audio_only\",IVS-NAME=\"audio_only\",IVS-VARIANT-SOURCE=\"transcode\"\n\
https://euc11.playlist.ttvnw.net/audio.m3u8\n\
#EXT-X-STREAM-INF:BANDWIDTH=6895917,RESOLUTION=1920x1080,CODECS=\"avc1.64002A,mp4a.40.2\",FRAME-RATE=60.000,STABLE-VARIANT-ID=\"1080p60\",IVS-NAME=\"1080p60\",IVS-VARIANT-SOURCE=\"source\"\n\
https://euc11.playlist.ttvnw.net/1080.m3u8\n\
#EXT-X-STREAM-INF:BANDWIDTH=3422999,RESOLUTION=1280x720,CODECS=\"avc1.4D401F,mp4a.40.2\",FRAME-RATE=60.000,STABLE-VARIANT-ID=\"720p60\",IVS-NAME=\"720p60\",IVS-VARIANT-SOURCE=\"transcode\"\n\
https://euc11.playlist.ttvnw.net/720.m3u8\n\
#EXT-X-STREAM-INF:BANDWIDTH=1427999,RESOLUTION=852x480,CODECS=\"avc1.4D401F,mp4a.40.2\",FRAME-RATE=30.000,STABLE-VARIANT-ID=\"480p30\",IVS-NAME=\"480p\",IVS-VARIANT-SOURCE=\"transcode\"\n\
https://euc11.playlist.ttvnw.net/480.m3u8\n\
#EXT-X-STREAM-INF:BANDWIDTH=630000,RESOLUTION=640x360,CODECS=\"avc1.4D401F,mp4a.40.2\",FRAME-RATE=30.000,STABLE-VARIANT-ID=\"360p30\",IVS-NAME=\"360p\",IVS-VARIANT-SOURCE=\"transcode\"\n\
https://euc11.playlist.ttvnw.net/360.m3u8\n\
#EXT-X-STREAM-INF:BANDWIDTH=230000,RESOLUTION=284x160,CODECS=\"avc1.4D401F,mp4a.40.2\",FRAME-RATE=30.000,STABLE-VARIANT-ID=\"160p30\",IVS-NAME=\"160p\",IVS-VARIANT-SOURCE=\"transcode\"\n\
https://euc11.playlist.ttvnw.net/160.m3u8\n";

    fn parse() -> Vec<Variant> {
        parse_master(MASTER_NICKMERCS)
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn parses_all_renditions() {
        let v = parse();
        assert_eq!(v.len(), 5);
        assert_eq!(v[0].name, "1080p60");
        assert_eq!(v[0].group_id, "chunked");
        assert_eq!(v[0].height, Some(1080));
        assert_eq!(v[0].fps, Some(60.0));
        assert_eq!(v[0].url, "https://video-weaver.example/chunked.m3u8");
        assert_eq!(v[4].name, "audio_only");
        assert_eq!(v[4].height, None);
    }

    #[test]
    fn best_maps_to_chunked_source() {
        let v = parse();
        let (idx, label) = select_variant(&v, "best").unwrap();
        assert_eq!(label, "best");
        assert_eq!(v[idx].group_id, "chunked");
        assert_eq!(v[idx].url, "https://video-weaver.example/chunked.m3u8");
    }

    #[test]
    fn worst_maps_to_lowest_video_not_audio() {
        let v = parse();
        let (idx, label) = select_variant(&v, "worst").unwrap();
        assert_eq!(label, "worst");
        assert_eq!(v[idx].name, "160p30");
    }

    #[test]
    fn audio_only_selects_audio() {
        let v = parse();
        let (idx, label) = select_variant(&v, "audio_only").unwrap();
        assert_eq!(label, "audio_only");
        assert_eq!(v[idx].name, "audio_only");
    }

    #[test]
    fn exact_numeric_match() {
        let v = parse();
        let (idx, label) = select_variant(&v, "720p60").unwrap();
        assert_eq!(label, "720p60");
        assert_eq!(v[idx].url, "https://video-weaver.example/720p60.m3u8");
    }

    #[test]
    fn closest_when_requested_tier_absent() {
        // 1440p60 saved but channel maxes at 1080p60 → fall to 1080p60 (source).
        let v = parse();
        let (idx, label) = select_variant(&v, "1440p60").unwrap();
        assert_eq!(label, "1080p60");
        assert_eq!(v[idx].name, "1080p60");
    }

    #[test]
    fn reconciles_fps_suffix_mismatch() {
        // User saved "480p" (caedrel shape) but this channel ships "480p30".
        let v = parse();
        let (idx, _label) = select_variant(&v, "480p").unwrap();
        assert_eq!(v[idx].name, "480p30");
    }

    #[test]
    fn quality_menu_includes_aliases_sorted() {
        let v = parse();
        let names = quality_names(&v);
        // The desktop's Auto leads the menu; the phones' "best" is their Auto.
        let first = if cfg!(mobile) { "1080p60" } else { "auto" };
        assert_eq!(names.first().map(String::as_str), Some(first));
        let first_tier = names.iter().find(|n| n.as_str() != "auto").map(String::as_str);
        assert_eq!(first_tier, Some("1080p60"));
        assert!(names.iter().any(|n| n == "best"));
        assert!(names.iter().any(|n| n == "worst"));
        assert!(names.iter().any(|n| n == "audio_only"));
    }

    #[test]
    fn an_offline_channel_gets_a_short_retry_budget() {
        // No playlist and no stream: a few seconds, not the whole minute.
        assert_eq!(retry_budget_when_offline(60, false), OFFLINE_RETRY_BUDGET_SECS);
        // Live per Twitch (the playlist is late): the full budget stays.
        assert_eq!(retry_budget_when_offline(60, true), 60);
        // A budget already shorter is never raised.
        assert_eq!(retry_budget_when_offline(5, false), 5);
        // Only usher's "no playlist" answer asks Twitch whether it is live.
        assert!(is_playlist_not_found(&anyhow!("usher returned 404 Not Found")));
        assert!(is_playlist_not_found(&anyhow!("usher returned 404 Not Found").context("resolve")));
        assert!(!is_playlist_not_found(&anyhow!("usher returned 403 Forbidden")));
        assert!(!is_playlist_not_found(&anyhow!("GQL request failed")));
    }

    // --- Screen-size cap (mobile) -----------------------------------------
    // Every one of these asserts against `capped_source_index` directly rather
    // than through the global, so they cannot race each other: the cap lives in
    // a process-wide Mutex and `cargo test` runs tests in parallel.

    #[test]
    fn cap_picks_the_tallest_rendition_that_fits() {
        let v = parse();
        let i = capped_source_index(&v, Some(720)).unwrap();
        assert!(v[i].height.unwrap() <= 720, "picked {}", v[i].name);
        // And the TALLEST that fits, not just any.
        assert_eq!(v[i].height, Some(720));
    }

    #[test]
    fn cap_ignores_the_chunked_shortcut() {
        // `source_index` returns the "chunked" group immediately. chunked IS the
        // oversized source, so a cap that honoured it first would never bite.
        let v = parse();
        let i = capped_source_index(&v, Some(480)).unwrap();
        assert_ne!(v[i].group_id, "chunked");
        assert!(v[i].height.unwrap() <= 480);
    }

    #[test]
    fn uncapped_matches_source_index() {
        let v = parse();
        assert_eq!(capped_source_index(&v, None), source_index(&v));
    }

    #[test]
    fn cap_below_every_tier_never_yields_audio_only() {
        // THE trap this cap could ship: audio_only carries no resolution and is
        // the LAST entry here (index 0 in some real masters). A `first()` or
        // `.or(Some(0))` fallback would hand the viewer a black screen with
        // sound. Cap below the whole ladder and demand a real video rendition.
        let v = parse();
        let i = capped_source_index(&v, Some(144)).unwrap();
        assert!(v[i].height.is_some(), "picked {} (audio!)", v[i].name);
        assert_ne!(v[i].name, "audio_only");
        // And specifically the LOWEST above the cap, not the source again.
        let lowest = v
            .iter()
            .filter(|x| x.height.is_some())
            .map(|x| x.height.unwrap())
            .min()
            .unwrap();
        assert_eq!(v[i].height, Some(lowest));
    }

    #[test]
    fn cap_at_or_above_source_is_a_no_op() {
        let v = parse();
        let i = capped_source_index(&v, Some(1080)).unwrap();
        assert_eq!(v[i].height, Some(1080));
        assert_eq!(v[i].name, "1080p60");
    }

    #[test]
    fn audio_only_master_does_not_panic_under_a_cap() {
        let v: Vec<Variant> = parse()
            .into_iter()
            .filter(|x| x.height.is_none())
            .collect();
        assert!(!v.is_empty(), "fixture should have an audio rendition");
        // No video to choose: must defer rather than panic or index out of range.
        assert!(capped_source_index(&v, Some(720)).is_some());
    }

    #[test]
    fn empty_master_yields_no_variants() {
        assert!(parse_master("#EXTM3U\n").is_empty());
        assert!(select_variant(&[], "best").is_none());
    }

    #[test]
    fn parses_modern_media_less_master() {
        // The regression case: a master with zero `#EXT-X-MEDIA` tags.
        let v = parse_master(MASTER_MODERN_IVS);
        assert_eq!(v.len(), 6, "audio_only + 5 video tiers");
        // audio_only first, no resolution.
        assert_eq!(v[0].name, "audio_only");
        assert_eq!(v[0].height, None);
        // Source rendition flagged by IVS-VARIANT-SOURCE="source", not GROUP-ID.
        assert_eq!(v[1].name, "1080p60");
        assert_eq!(v[1].group_id, "chunked");
        assert_eq!(v[1].height, Some(1080));
        assert_eq!(v[1].fps, Some(60.0));
        assert_eq!(v[1].url, "https://euc11.playlist.ttvnw.net/1080.m3u8");
        // Label comes from IVS-NAME ("480p"), not STABLE-VARIANT-ID ("480p30").
        assert_eq!(v[3].name, "480p");
    }

    #[test]
    fn modern_best_worst_audio_select() {
        let v = parse_master(MASTER_MODERN_IVS);
        let (bi, _) = select_variant(&v, "best").unwrap();
        assert_eq!(v[bi].name, "1080p60", "best → IVS source rendition");
        let (wi, _) = select_variant(&v, "worst").unwrap();
        assert_eq!(v[wi].name, "160p", "worst → lowest video, not audio");
        let (ai, _) = select_variant(&v, "audio_only").unwrap();
        assert_eq!(v[ai].name, "audio_only");
    }

    #[test]
    fn anonymous_master_no_source_flag_still_picks_best() {
        // The anonymous proxy master (e.g. symfuhny via TTVLOL): every tier is a
        // "transcode" — none is flagged IVS-VARIANT-SOURCE="source" and there's no
        // GROUP-ID="chunked". "best" must still resolve to the highest video tier,
        // not audio_only or nothing.
        let m = "#EXTM3U\n\
#EXT-X-STREAM-INF:BANDWIDTH=8042999,RESOLUTION=1920x1080,FRAME-RATE=60.000,IVS-NAME=\"1080p60\",IVS-VARIANT-SOURCE=\"transcode\"\n\
https://x/1080.m3u8\n\
#EXT-X-STREAM-INF:BANDWIDTH=3322199,RESOLUTION=1280x720,FRAME-RATE=60.000,IVS-NAME=\"720p60\",IVS-VARIANT-SOURCE=\"transcode\"\n\
https://x/720.m3u8\n\
#EXT-X-STREAM-INF:BANDWIDTH=160000,CODECS=\"mp4a.40.2\",IVS-NAME=\"audio_only\",IVS-VARIANT-SOURCE=\"transcode\"\n\
https://x/audio.m3u8\n";
        let v = parse_master(m);
        assert_eq!(v.len(), 3);
        assert!(
            v.iter().all(|x| x.group_id != "chunked"),
            "none source-flagged"
        );
        let (bi, _) = select_variant(&v, "best").unwrap();
        assert_eq!(
            v[bi].name, "1080p60",
            "best falls back to highest video tier"
        );
        let (wi, _) = select_variant(&v, "worst").unwrap();
        assert_eq!(v[wi].name, "720p60", "worst is lowest video, not audio");
    }

    #[test]
    fn accepts_arbitrary_custom_resolution() {
        // Enhanced broadcasting lets streamers output non-standard resolutions in
        // OBS; the parser must accept any STREAM-INF, not an allow-list of tiers.
        let m = "#EXTM3U\n\
#EXT-X-STREAM-INF:BANDWIDTH=20000000,RESOLUTION=2560x1440,CODECS=\"av01.0.13M.08,mp4a.40.2\",FRAME-RATE=60.000,IVS-NAME=\"1440p60\",IVS-VARIANT-SOURCE=\"source\"\n\
https://x/1440.m3u8\n\
#EXT-X-STREAM-INF:BANDWIDTH=5000000,RESOLUTION=1664x936,FRAME-RATE=60.000,CODECS=\"avc1.4D401F,mp4a.40.2\"\n\
https://x/936.m3u8\n";
        let v = parse_master(m);
        assert_eq!(v.len(), 2);
        // 1440p source: labeled from IVS-NAME, flagged as the source rendition.
        assert_eq!(v[0].name, "1440p60");
        assert_eq!(v[0].group_id, "chunked");
        assert_eq!(v[0].height, Some(1440));
        // Custom 936p tier carries no label → derived from RESOLUTION + FRAME-RATE.
        assert_eq!(v[1].name, "936p60");
        assert_eq!(v[1].height, Some(936));
        // best → the 1440 source; the custom tier is still selectable by name.
        let (bi, _) = select_variant(&v, "best").unwrap();
        assert_eq!(v[bi].name, "1440p60");
        let (ci, _) = select_variant(&v, "936p60").unwrap();
        assert_eq!(v[ci].name, "936p60");
    }

    // ── codec-aware selection (enhanced_codecs) ──

    /// A channel offering both H.264 and a more-efficient AV1 at 1080p60, plus a
    /// 720p AV1 tier, to exercise "prefer codec at the same resolution, never
    /// downgrade resolution for codec".
    fn codec_master() -> Vec<Variant> {
        parse_master(
            "#EXTM3U\n\
#EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID=\"chunked\",NAME=\"1080p60\"\n\
#EXT-X-STREAM-INF:BANDWIDTH=8000000,RESOLUTION=1920x1080,CODECS=\"avc1.4D402A,mp4a.40.2\",VIDEO=\"chunked\",FRAME-RATE=60.000\n\
https://x/h264-1080.m3u8\n\
#EXT-X-STREAM-INF:BANDWIDTH=5000000,RESOLUTION=1920x1080,CODECS=\"av01.0.08M.08,mp4a.40.2\",FRAME-RATE=60.000,IVS-NAME=\"1080p60-av1\"\n\
https://x/av1-1080.m3u8\n\
#EXT-X-STREAM-INF:BANDWIDTH=3000000,RESOLUTION=1280x720,CODECS=\"hev1.1.6.L93.B0,mp4a.40.2\",FRAME-RATE=60.000,IVS-NAME=\"720p60-hevc\"\n\
https://x/hevc-720.m3u8\n",
        )
    }

    #[test]
    fn codec_family_classifies_fourccs() {
        assert_eq!(codec_family(Some("av01.0.08M.08,mp4a.40.2")), "av1");
        assert_eq!(codec_family(Some("hev1.1.6.L93.B0")), "hevc");
        assert_eq!(codec_family(Some("hvc1.1.6.L93.B0")), "hevc");
        assert_eq!(codec_family(Some("avc1.4D402A,mp4a.40.2")), "h264");
        assert_eq!(codec_family(None), "h264"); // unlabeled → safe default
        assert_eq!(codec_family(Some("vp09.00.10.08")), "other");
    }

    #[test]
    fn best_prefers_av1_at_source_resolution_when_decodable() {
        let v = codec_master();
        let pref = vec!["av1".to_string(), "hevc".to_string(), "h264".to_string()];
        let i = best_index_with(&v, &pref).unwrap();
        // Same 1080p resolution as the H.264 source, but the AV1 rendition.
        assert_eq!(v[i].url, "https://x/av1-1080.m3u8");
        assert_eq!(codec_family(v[i].codecs.as_deref()), "av1");
    }

    #[test]
    fn best_falls_back_to_h264_when_enhanced_off() {
        let v = codec_master();
        let pref = vec!["h264".to_string()]; // enhanced off / nothing else decodable
        let i = best_index_with(&v, &pref).unwrap();
        assert_eq!(v[i].url, "https://x/h264-1080.m3u8");
    }

    #[test]
    fn best_never_downgrades_resolution_for_a_fancier_codec() {
        // AV1/HEVC only exist BELOW the source resolution here (source is H.264 1080).
        let v = parse_master(
            "#EXTM3U\n\
#EXT-X-STREAM-INF:BANDWIDTH=8000000,RESOLUTION=1920x1080,CODECS=\"avc1.4D402A\",VIDEO=\"chunked\",FRAME-RATE=60.000\n\
https://x/h264-1080.m3u8\n\
#EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID=\"chunked\",NAME=\"1080p60\"\n\
#EXT-X-STREAM-INF:BANDWIDTH=4000000,RESOLUTION=1280x720,CODECS=\"av01.0.08M.08\",FRAME-RATE=60.000,IVS-NAME=\"720p60\"\n\
https://x/av1-720.m3u8\n",
        );
        let pref = vec!["av1".to_string(), "hevc".to_string(), "h264".to_string()];
        let i = best_index_with(&v, &pref).unwrap();
        // Stays at 1080p H.264; does NOT drop to 720p just because it's AV1.
        assert_eq!(v[i].height, Some(1080));
        assert_eq!(codec_family(v[i].codecs.as_deref()), "h264");
    }

    #[test]
    fn hevc_chosen_only_when_allowed() {
        // At 720p the only option is HEVC. With HEVC allowed it's eligible; without,
        // prefer_codec_at finds nothing decodable there (caller keeps its fallback).
        let v = codec_master();
        let with_hevc = vec!["hevc".to_string(), "h264".to_string()];
        assert!(prefer_codec_at(&v, Some(720), Some(60.0), &with_hevc).is_some());
        let no_hevc = vec!["h264".to_string()];
        assert!(prefer_codec_at(&v, Some(720), Some(60.0), &no_hevc).is_none());
    }

    /// A geo-blocked viewer's master after the region unlock: their own H.264
    /// tiers, with the region relay's 1440p HEVC source tier grafted on. The
    /// 1080p tier is a transcode, so the grafted tier is the only source.
    const MASTER_REGION_UNLOCKED: &str = "#EXTM3U\n\
#EXT-X-STREAM-INF:BANDWIDTH=160000,CODECS=\"mp4a.40.2\",IVS-NAME=\"audio_only\",IVS-VARIANT-SOURCE=\"transcode\"\n\
https://x/audio.m3u8\n\
#EXT-X-STREAM-INF:BANDWIDTH=6000000,RESOLUTION=1920x1080,CODECS=\"avc1.4D402A,mp4a.40.2\",FRAME-RATE=60.000,IVS-NAME=\"1080p60\",IVS-VARIANT-SOURCE=\"transcode\"\n\
https://x/1080.m3u8\n\
#EXT-X-STREAM-INF:BANDWIDTH=3400000,RESOLUTION=1280x720,CODECS=\"avc1.4D401F,mp4a.40.2\",FRAME-RATE=60.000,IVS-NAME=\"720p60\",IVS-VARIANT-SOURCE=\"transcode\"\n\
https://x/720.m3u8\n\
#EXT-X-STREAM-INF:BANDWIDTH=9000000,RESOLUTION=2560x1440,CODECS=\"hev1.1.6.L150.90,mp4a.40.2\",FRAME-RATE=60.000,IVS-NAME=\"1440p60\",IVS-VARIANT-SOURCE=\"source\"\n\
https://relay.example/quality-media?region=weur&u=1440\n";

    #[test]
    fn a_source_tier_this_device_cannot_decode_never_plays() {
        let pref = s(&["av1", "h264"]); // no HEVC decoder
        let v = retain_playable(parse_master(MASTER_REGION_UNLOCKED), &pref);
        let i = best_index_with(&v, &pref).unwrap();
        assert_eq!(v[i].name, "1080p60", "best plays the tallest H.264 tier");
        let menu = quality_names(&v);
        assert!(!menu.iter().any(|q| q == "1440p60"), "the menu does not offer it: {menu:?}");
        let (j, label) = select_variant(&v, "1440p60").unwrap();
        assert_eq!((v[j].height, label.as_str()), (Some(1080), "1080p60"), "a saved 1440p pick lands on 1080p");
    }

    #[test]
    fn a_source_tier_this_device_decodes_stays_best() {
        let pref = s(&["hevc", "h264"]);
        let v = retain_playable(parse_master(MASTER_REGION_UNLOCKED), &pref);
        let i = best_index_with(&v, &pref).unwrap();
        assert_eq!(v[i].name, "1440p60");
        assert!(quality_names(&v).iter().any(|q| q == "1440p60"));
    }

    #[test]
    fn a_master_with_no_playable_video_is_kept_whole() {
        let only_hevc = "#EXTM3U\n\
#EXT-X-STREAM-INF:BANDWIDTH=160000,CODECS=\"mp4a.40.2\",IVS-NAME=\"audio_only\"\n\
https://x/audio.m3u8\n\
#EXT-X-STREAM-INF:BANDWIDTH=9000000,RESOLUTION=2560x1440,CODECS=\"hev1.1.6.L150.90,mp4a.40.2\",FRAME-RATE=60.000,IVS-NAME=\"1440p60\",IVS-VARIANT-SOURCE=\"source\"\n\
https://x/1440.m3u8\n";
        let pref = s(&["h264"]);
        let v = retain_playable(parse_master(only_hevc), &pref);
        assert_eq!(v.len(), 2, "trying HEVC beats serving audio only");
        let (i, _) = select_variant(&v, "best").unwrap();
        assert_eq!(v[i].height, Some(1440));
    }

    #[test]
    fn a_failed_codec_stays_off_the_list_and_h264_never_leaves() {
        let failed = s(&["hevc"]);
        assert_eq!(allowed_codecs(s(&["av1", "hevc", "h264"]), &failed), ["av1", "h264"]);
        assert_eq!(allowed_codecs(s(&["hevc"]), &failed), ["h264"]);
        assert_eq!(allowed_codecs(s(&["av1", "h264"]), &s(&["h264"])), ["av1", "h264"]);
    }

    #[test]
    fn the_failed_family_is_read_from_a_mime_type_or_a_codecs_list() {
        assert_eq!(avoidable_codec_family("video/mp4;codecs=hvc1.1.6.L150.90"), Some("hevc"));
        assert_eq!(avoidable_codec_family("video/mp4; codecs=\"av01.0.08M.08\""), Some("av1"));
        assert_eq!(avoidable_codec_family("hev1.1.6.L150.90,mp4a.40.2"), Some("hevc"));
        assert_eq!(avoidable_codec_family("mp4a.40.2,hev1.1.6.L150.90"), Some("hevc"));
        assert_eq!(avoidable_codec_family("video/mp4;codecs=avc1.4D402A"), None, "H.264 is the floor");
        assert_eq!(avoidable_codec_family("audio/mp4;codecs=mp4a.40.2"), None);
        assert_eq!(avoidable_codec_family(""), None);
    }

    #[test]
    fn a_failure_on_h264_or_nothing_known_drops_no_codec() {
        assert_eq!(
            fail_codec("test-h264-floor", Some("video/mp4;codecs=avc1.64002A"), Some("hvc1")),
            None,
            "the failed type wins"
        );
        assert_eq!(fail_codec("test-h264-floor", None, Some("avc1")), None);
        assert_eq!(fail_codec("test-h264-floor", None, None), None);
        assert!(failed_for("test-h264-floor").is_empty());
    }

    #[test]
    fn a_codec_failure_is_scoped_to_its_channel() {
        assert_eq!(fail_codec("Test-Banned-Here", None, Some("hvc1.1.6.L150.90")), Some("hevc"));
        assert_eq!(failed_for("test-banned-here"), ["hevc"]);
        assert!(failed_for("test-somewhere-else").is_empty());
        let pref = allowed_codecs(s(&["hevc", "h264"]), &failed_for("test-banned-here"));
        assert_eq!(pref, ["h264"]);
        // Reported twice, kept once.
        fail_codec("test-banned-here", Some("video/mp4;codecs=hev1.1.6.L150.90"), None);
        assert_eq!(failed_for("test-banned-here").len(), 1);
    }

    #[test]
    fn a_codec_hold_drops_hevc_and_av1_on_its_channel_until_it_ends() {
        hold_codecs("Test-Hold-Here", Duration::from_secs(600));
        let mut held = failed_for("test-hold-here");
        held.sort();
        assert_eq!(held, ["av1", "hevc"]);
        assert!(failed_for("test-hold-elsewhere").is_empty());
        let pref = allowed_codecs(s(&["av1", "hevc", "h264"]), &failed_for("test-hold-here"));
        assert_eq!(pref, ["h264"], "H.264 stays the floor");
        let later = Instant::now() + Duration::from_secs(601);
        assert!(held_at("test-hold-here", later).is_empty(), "the hold ends");
    }

    #[test]
    fn a_hold_stays_out_of_the_session_failures() {
        hold_codecs("test-hold-menu", Duration::from_secs(600));
        assert!(failed_in_session("test-hold-menu").is_empty(), "the quality menu keeps held tiers");
    }

    #[test]
    fn releasing_a_hold_brings_the_codecs_back() {
        hold_codecs("test-hold-release", Duration::from_secs(600));
        assert_eq!(held_at("test-hold-release", Instant::now()).len(), 2);
        release_codec_hold("Test-Hold-Release");
        assert!(held_at("test-hold-release", Instant::now()).is_empty());
    }

    #[test]
    fn a_held_codec_string_reads_as_held() {
        hold_codecs("test-hold-tile", Duration::from_secs(600));
        assert!(codec_held("test-hold-tile", Some("hev1.1.6.L150.90,mp4a.40.2")));
        assert!(codec_held("test-hold-tile", Some("av01.0.08M.08,mp4a.40.2")));
        assert!(!codec_held("test-hold-tile", Some("avc1.4D402A,mp4a.40.2")));
        assert!(!codec_held("test-hold-tile", None));
        assert!(!codec_held("test-hold-nowhere", Some("hev1.1.6.L150.90")));
    }

    #[test]
    fn before_the_device_reports_every_tier_stays() {
        // Nothing reported and nothing failed on this channel: the whole ladder,
        // HEVC source included, as every release before the filter served.
        if codec_pref_reported() {
            return; // another test holds a preference right now
        }
        let v = playable_variants(MASTER_REGION_UNLOCKED, "test-unreported");
        assert!(v.iter().any(|x| x.name == "1440p60"));
    }

    #[test]
    fn auto_starts_where_best_would() {
        let v = parse();
        let (bi, best) = select_variant(&v, "best").unwrap();
        let (ai, auto) = select_variant(&v, "auto").unwrap();
        assert_eq!((ai, auto.as_str()), (bi, "auto"));
        assert_eq!(best, "best");
    }

    #[test]
    fn the_codec_family_is_the_video_entry_wherever_it_sits() {
        assert_eq!(codec_family(Some("mp4a.40.2,hev1.1.6.L93.B0")), "hevc");
        assert_eq!(codec_family(Some("ec-3,av01.0.08M.08")), "av1");
        assert_eq!(codec_family(Some("mp4a.40.2, avc1.4D402A")), "h264");
        assert_eq!(codec_family(Some("mp4a.40.2")), "h264");
    }

    #[test]
    fn set_codec_preference_filters_junk_and_always_keeps_h264() {
        set_codec_preference(vec!["av1".into(), "vp9".into(), "hevc".into()]);
        assert_eq!(effective_codec_pref(), vec!["av1", "hevc", "h264"]); // vp9 dropped
        // Reset to the default-equivalent so any concurrent test sees H.264-only.
        set_codec_preference(vec![]);
        assert_eq!(effective_codec_pref(), vec!["h264"]);
    }

    #[test]
    fn derives_label_from_resolution_when_unlabeled() {
        // No MEDIA tag and no IVS-NAME — fall back to RESOLUTION + FRAME-RATE.
        let m = "#EXTM3U\n\
#EXT-X-STREAM-INF:BANDWIDTH=8000000,RESOLUTION=1920x1080,FRAME-RATE=60.000,CODECS=\"avc1.4D402A\"\n\
https://x/1080.m3u8\n\
#EXT-X-STREAM-INF:BANDWIDTH=1500000,RESOLUTION=852x480,FRAME-RATE=30.000,CODECS=\"avc1.4D401F\"\n\
https://x/480.m3u8\n";
        let v = parse_master(m);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].name, "1080p60");
        assert_eq!(v[1].name, "480p");
    }

    #[test]
    fn tolerates_lines_between_streaminf_and_url() {
        // A vendor comment / blank line between the STREAM-INF and its URL must
        // not drop the rendition (the old i+1/i+2 adjacency would have).
        let m = "#EXTM3U\n\
#EXT-X-STREAM-INF:BANDWIDTH=8000000,RESOLUTION=1920x1080,FRAME-RATE=60.000,IVS-NAME=\"1080p60\",IVS-VARIANT-SOURCE=\"source\"\n\
\n\
#VENDOR-TAG:foo\n\
https://x/1080.m3u8\n";
        let v = parse_master(m);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].name, "1080p60");
        assert_eq!(v[0].group_id, "chunked");
        assert_eq!(v[0].url, "https://x/1080.m3u8");
    }

    // --- Landscape goldens (SN-40) ----------------------------------------
    // Captured from the selection code as it stood before the vertical view
    // (short-edge measure, the several-source-flags rule, portrait numeric
    // matching). Every landscape ladder must keep choosing exactly these
    // renditions; "landscape unchanged" is claimed by these rows alone.

    /// Best, worst, audio, a few numeric picks (label=url) and every cap step.
    fn golden(v: &[Variant], pref: &[String]) -> Vec<String> {
        let pick = |o: Option<(usize, String)>| match o {
            Some((i, label)) => format!("{label}={}", v[i].url),
            None => "none".to_string(),
        };
        let mut out = Vec::new();
        for req in ["best", "worst", "audio_only", "1440p60", "720p60", "480p", "160p"] {
            out.push(format!("{req}:{}", pick(select_variant_with(v, req, pref))));
        }
        for cap in [1080, 720, 480, 144] {
            let url = capped_source_index(v, Some(cap)).map(|i| v[i].url.clone()).unwrap_or_default();
            out.push(format!("cap{cap}:{url}"));
        }
        out
    }

    /// The menu, with the desktop's Auto in front where it applies.
    fn menu(names: &[&str]) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        if !cfg!(mobile) {
            out.push("auto".into());
        }
        out.extend(names.iter().map(|n| n.to_string()));
        out
    }

    #[test]
    fn landscape_golden_legacy_nickmercs() {
        let v = parse();
        let w = "https://video-weaver.example";
        assert_eq!(
            golden(&v, &s(&["h264"])),
            vec![
                format!("best:best={w}/chunked.m3u8"),
                format!("worst:worst={w}/160p30.m3u8"),
                format!("audio_only:audio_only={w}/audio_only.m3u8"),
                format!("1440p60:1080p60={w}/chunked.m3u8"),
                format!("720p60:720p60={w}/720p60.m3u8"),
                format!("480p:480p30={w}/480p30.m3u8"),
                format!("160p:160p30={w}/160p30.m3u8"),
                format!("cap1080:{w}/chunked.m3u8"),
                format!("cap720:{w}/720p60.m3u8"),
                format!("cap480:{w}/480p30.m3u8"),
                format!("cap144:{w}/160p30.m3u8"),
            ]
        );
        assert_eq!(
            quality_names(&v),
            menu(&["1080p60", "720p60", "480p30", "160p30", "best", "audio_only", "worst"])
        );
    }

    #[test]
    fn landscape_golden_modern_ivs() {
        let v = parse_master(MASTER_MODERN_IVS);
        let w = "https://euc11.playlist.ttvnw.net";
        assert_eq!(
            golden(&v, &s(&["h264"])),
            vec![
                format!("best:best={w}/1080.m3u8"),
                format!("worst:worst={w}/160.m3u8"),
                format!("audio_only:audio_only={w}/audio.m3u8"),
                format!("1440p60:1080p60={w}/1080.m3u8"),
                format!("720p60:720p60={w}/720.m3u8"),
                format!("480p:480p={w}/480.m3u8"),
                format!("160p:160p={w}/160.m3u8"),
                format!("cap1080:{w}/1080.m3u8"),
                format!("cap720:{w}/720.m3u8"),
                format!("cap480:{w}/480.m3u8"),
                format!("cap144:{w}/160.m3u8"),
            ]
        );
        assert_eq!(
            quality_names(&v),
            menu(&["1080p60", "720p60", "480p", "360p", "160p", "best", "audio_only", "worst"])
        );
    }

    #[test]
    fn landscape_golden_region_unlocked() {
        let relay = "https://relay.example/quality-media?region=weur&u=1440";
        let hevc = s(&["hevc", "h264"]);
        let v = retain_playable(parse_master(MASTER_REGION_UNLOCKED), &hevc);
        assert_eq!(
            golden(&v, &hevc),
            vec![
                format!("best:best={relay}"),
                "worst:worst=https://x/720.m3u8".to_string(),
                "audio_only:audio_only=https://x/audio.m3u8".to_string(),
                format!("1440p60:1440p60={relay}"),
                "720p60:720p60=https://x/720.m3u8".to_string(),
                "480p:720p60=https://x/720.m3u8".to_string(),
                "160p:720p60=https://x/720.m3u8".to_string(),
                "cap1080:https://x/1080.m3u8".to_string(),
                "cap720:https://x/720.m3u8".to_string(),
                "cap480:https://x/720.m3u8".to_string(),
                "cap144:https://x/720.m3u8".to_string(),
            ]
        );
        assert_eq!(quality_names(&v), menu(&["1440p60", "1080p60", "720p60", "best", "audio_only", "worst"]));

        let h264 = s(&["h264"]);
        let v = retain_playable(parse_master(MASTER_REGION_UNLOCKED), &h264);
        assert_eq!(
            golden(&v, &h264),
            vec![
                "best:best=https://x/1080.m3u8".to_string(),
                "worst:worst=https://x/720.m3u8".to_string(),
                "audio_only:audio_only=https://x/audio.m3u8".to_string(),
                "1440p60:1080p60=https://x/1080.m3u8".to_string(),
                "720p60:720p60=https://x/720.m3u8".to_string(),
                "480p:720p60=https://x/720.m3u8".to_string(),
                "160p:720p60=https://x/720.m3u8".to_string(),
                "cap1080:https://x/1080.m3u8".to_string(),
                "cap720:https://x/720.m3u8".to_string(),
                "cap480:https://x/720.m3u8".to_string(),
                "cap144:https://x/720.m3u8".to_string(),
            ]
        );
    }

    #[test]
    fn landscape_golden_unflagged_and_codec_ladders() {
        let anon = parse_master(
            "#EXTM3U\n\
#EXT-X-STREAM-INF:BANDWIDTH=8042999,RESOLUTION=1920x1080,FRAME-RATE=60.000,IVS-NAME=\"1080p60\",IVS-VARIANT-SOURCE=\"transcode\"\n\
https://x/1080.m3u8\n\
#EXT-X-STREAM-INF:BANDWIDTH=3322199,RESOLUTION=1280x720,FRAME-RATE=60.000,IVS-NAME=\"720p60\",IVS-VARIANT-SOURCE=\"transcode\"\n\
https://x/720.m3u8\n\
#EXT-X-STREAM-INF:BANDWIDTH=160000,CODECS=\"mp4a.40.2\",IVS-NAME=\"audio_only\",IVS-VARIANT-SOURCE=\"transcode\"\n\
https://x/audio.m3u8\n",
        );
        assert_eq!(
            golden(&anon, &s(&["h264"])),
            vec![
                "best:best=https://x/1080.m3u8",
                "worst:worst=https://x/720.m3u8",
                "audio_only:audio_only=https://x/audio.m3u8",
                "1440p60:1080p60=https://x/1080.m3u8",
                "720p60:720p60=https://x/720.m3u8",
                "480p:720p60=https://x/720.m3u8",
                "160p:720p60=https://x/720.m3u8",
                "cap1080:https://x/1080.m3u8",
                "cap720:https://x/720.m3u8",
                "cap480:https://x/720.m3u8",
                "cap144:https://x/720.m3u8",
            ]
        );

        let v = codec_master();
        assert_eq!(
            golden(&v, &s(&["av1", "hevc", "h264"])),
            vec![
                "best:best=https://x/av1-1080.m3u8",
                "worst:worst=https://x/hevc-720.m3u8",
                "audio_only:best=https://x/av1-1080.m3u8",
                "1440p60:1080p60-av1=https://x/av1-1080.m3u8",
                "720p60:720p60-hevc=https://x/hevc-720.m3u8",
                "480p:720p60-hevc=https://x/hevc-720.m3u8",
                "160p:720p60-hevc=https://x/hevc-720.m3u8",
                "cap1080:https://x/h264-1080.m3u8",
                "cap720:https://x/hevc-720.m3u8",
                "cap480:https://x/hevc-720.m3u8",
                "cap144:https://x/hevc-720.m3u8",
            ]
        );
        assert_eq!(quality_names(&v), menu(&["1080p60", "1080p60-av1", "720p60-hevc", "best", "worst"]));
    }

    // --- Portrait ladders (SN-40) -------------------------------------------

    use crate::services::twitch_view::tests::{fixture, three_shuffles};

    const V2: &str = "https://v2.example";

    #[test]
    fn the_v2_source_is_the_top_tier_on_every_line_order() {
        // v2 flags every tier (audio too) and shuffles them: the first flagged
        // line was a random tier, sometimes the audio.
        for m in three_shuffles(&fixture("dual_v2")) {
            let tall = parse_master(&twitch_view::narrow(&m, twitch_view::View::Vertical));
            let i = source_index(&tall).unwrap();
            assert_eq!(tall[i].url, format!("{V2}/1080p60-portrait.m3u8"), "{m}");
            let land = parse_master(&twitch_view::narrow(&m, twitch_view::View::Landscape));
            let i = source_index(&land).unwrap();
            assert_eq!(land[i].url, format!("{V2}/1080p60.m3u8"), "{m}");

            // The top portrait tier is HEVC: an H.264-only phone plays 720x1280.
            let h264 = s(&["h264"]);
            let playable = retain_playable(tall.clone(), &h264);
            let (i, label) = select_variant_with(&playable, "best", &h264).unwrap();
            assert_eq!((playable[i].url.clone(), label), (format!("{V2}/720p60-portrait.m3u8"), "best".into()));
            let hevc = s(&["h264", "hevc"]);
            let playable = retain_playable(tall.clone(), &hevc);
            let (i, _) = select_variant_with(&playable, "best", &hevc).unwrap();
            assert_eq!(playable[i].url, format!("{V2}/1080p60-portrait.m3u8"));
            // Worst is the smallest portrait rung, never the audio.
            let (i, _) = select_variant_with(&tall, "worst", &hevc).unwrap();
            assert_eq!(tall[i].url, format!("{V2}/160p30-portrait.m3u8"));
        }
    }

    #[test]
    fn one_flagged_source_keeps_the_fast_path() {
        // The v1 shapes flag exactly one tier; it wins even below another tier.
        let m = "#EXTM3U\n\
#EXT-X-STREAM-INF:BANDWIDTH=9000000,RESOLUTION=2560x1440,IVS-NAME=\"1440p60\",IVS-VARIANT-SOURCE=\"transcode\"\nhttps://x/1440.m3u8\n\
#EXT-X-STREAM-INF:BANDWIDTH=6000000,RESOLUTION=1920x1080,IVS-NAME=\"1080p60\",IVS-VARIANT-SOURCE=\"source\"\nhttps://x/1080.m3u8\n";
        let v = parse_master(m);
        assert_eq!(v[source_index(&v).unwrap()].url, "https://x/1080.m3u8");
    }

    #[test]
    fn the_phone_cap_measures_the_short_edge() {
        // The legacy vertical-only shape (lultarek): 720x1280 source named
        // "1280p (source)", rungs named by height (720p = 404x720). Its height
        // 1280 read as above a 1080 display and played the 404x720 rung.
        let v = parse_master(&fixture("vertical_only_v1"));
        let i = capped_source_index(&v, Some(1080)).unwrap();
        assert_eq!((v[i].width, v[i].height, v[i].name.as_str()), (Some(720), Some(1280), "1280p (source)"));
        let i = capped_source_index(&v, Some(720)).unwrap();
        assert_eq!(v[i].url, "https://v1.example/chunked.m3u8");
        let i = capped_source_index(&v, Some(480)).unwrap();
        assert_eq!(v[i].url, "https://v1.example/720p30.m3u8", "404 wide fits 480");
        let (i, _) = select_variant_with(&v, "worst", &s(&["h264"])).unwrap();
        assert_eq!(v[i].url, "https://v1.example/160p30.m3u8");

        // The dual-format portrait ladder (zerator): 1080x1920 fits a 1080 cap.
        for m in three_shuffles(&fixture("dual_v2")) {
            let tall = parse_master(&twitch_view::narrow(&m, twitch_view::View::Vertical));
            let i = capped_source_index(&tall, Some(1080)).unwrap();
            assert_eq!(tall[i].url, format!("{V2}/1080p60-portrait.m3u8"));
            let h264 = retain_playable(tall.clone(), &s(&["h264"]));
            let i = capped_source_index(&h264, Some(1080)).unwrap();
            assert_eq!(h264[i].url, format!("{V2}/720p60-portrait.m3u8"));
            let i = capped_source_index(&tall, Some(720)).unwrap();
            assert_eq!(tall[i].url, format!("{V2}/720p60-portrait.m3u8"));
        }
    }

    #[test]
    fn a_saved_quality_matches_a_portrait_ladder_by_short_edge() {
        let h264 = s(&["h264"]);
        let v = parse_master(&fixture("vertical_only_v1"));
        let pick = |q: &str| select_variant_with(&v, q, &h264).map(|(i, l)| (v[i].url.clone(), l));
        // "720p60" is the 720-wide tier, not the rung this ladder calls 720p.
        assert_eq!(pick("720p60"), Some(("https://v1.example/chunked.m3u8".into(), "1280p (source)".into())));
        assert_eq!(pick("1080p60"), Some(("https://v1.example/chunked.m3u8".into(), "1280p (source)".into())));
        // A pick from this ladder's own menu is still the tier it names.
        assert_eq!(pick("720p"), Some(("https://v1.example/720p30.m3u8".into(), "720p".into())));
        assert_eq!(pick("1280p (source)"), Some(("https://v1.example/chunked.m3u8".into(), "1280p (source)".into())));

        // The v2 portrait ladder names by short edge already.
        let tall = parse_master(&twitch_view::narrow(&fixture("dual_v2"), twitch_view::View::Vertical));
        let (i, label) = select_variant_with(&tall, "720p60", &h264).unwrap();
        assert_eq!((tall[i].width, tall[i].height, label.as_str()), (Some(720), Some(1280), "720p60"));
        // 1080p60 there is HEVC; without it, the closest by short edge is 720x1280.
        let playable = retain_playable(tall, &h264);
        let (i, _) = select_variant_with(&playable, "1080p60", &h264).unwrap();
        assert_eq!(playable[i].url, format!("{V2}/720p60-portrait.m3u8"));
    }

    fn status(channel: &str) -> PlaybackStatus {
        PlaybackStatus {
            channel: channel.into(),
            mode: "auth-only".into(),
            entitled: false,
            proxy_base: None,
            proxy_region: None,
            reason: None,
        }
    }

    #[test]
    fn vertical_needs_a_playable_portrait_group() {
        let h264 = s(&["h264"]);
        let dual = fixture("dual_v2");
        assert!(vertical_playable(&dual, None));
        for m in three_shuffles(&dual) {
            assert!(vertical_playable(&m, Some(h264.as_slice())), "720x1280 is H.264");
        }
        // A portrait group that is HEVC only, for a phone without HEVC.
        let hevc_only = "#EXTM3U\n\
#EXT-X-STREAM-INF:BANDWIDTH=7635509,CODECS=\"avc1.4D402A,mp4a.40.2\",RESOLUTION=1920x1080,FRAME-RATE=60.000,IVS-NAME=\"1080p60\",IVS-GROUPS=\"landscape\",IVS-VARIANT-SOURCE=\"source\"\nhttps://x/1080.m3u8\n\
#EXT-X-STREAM-INF:BANDWIDTH=5224532,CODECS=\"hev1.1.6.L123.90,mp4a.40.2\",RESOLUTION=1080x1920,FRAME-RATE=60.000,IVS-NAME=\"1080p60\",IVS-GROUPS=\"portrait\",IVS-VARIANT-SOURCE=\"source\"\nhttps://x/1080-portrait.m3u8\n";
        assert!(!vertical_playable(hevc_only, Some(h264.as_slice())));
        assert!(vertical_playable(hevc_only, Some(s(&["h264", "hevc"]).as_slice())));
        // Vertical-only (untagged portrait) and v1 masters are not dual-format.
        assert!(!vertical_playable(&fixture("vertical_only_v2"), None));
        assert!(!vertical_playable(&fixture("vertical_only_v1"), None));
        assert!(!vertical_playable(MASTER_MODERN_IVS, None));
    }

    #[test]
    fn the_served_picture_follows_the_want_and_what_v2_said() {
        let dual = fixture("dual_v2");
        let v1 = MASTER_MODERN_IVS.to_string();
        let land_720 = "https://euc11.playlist.ttvnw.net/720.m3u8";
        let serve = |v2: Option<String>, available: bool, want: View| {
            build_served("test-served".into(), v1.clone(), v2, available, status("test-served"), "720p60", want).unwrap()
        };

        // v2 failed or was late: landscape from v1, no error.
        let r = serve(None, false, View::Vertical);
        assert_eq!((r.view, r.vertical_available, r.url.as_str()), (View::Landscape, false, land_720));
        assert_eq!((r.master.as_str(), r.portrait), (v1.as_str(), Some(false)));
        // v2 answered with no playable portrait group: landscape.
        let r = serve(Some(dual.clone()), false, View::Vertical);
        assert_eq!((r.view, r.url.as_str()), (View::Landscape, land_720));
        // Dual-format, wanted landscape: landscape, and the pill can show.
        let r = serve(Some(dual.clone()), true, View::Landscape);
        assert_eq!((r.view, r.vertical_available, r.url.as_str()), (View::Landscape, true, land_720));
        assert_eq!(r.master, v1, "landscape never reads v2's bytes");
        // Dual-format, wanted vertical: the narrowed portrait group.
        let r = serve(Some(dual.clone()), true, View::Vertical);
        assert_eq!((r.view, r.vertical_available, r.portrait), (View::Vertical, true, Some(true)));
        assert_eq!(r.url, format!("{V2}/720p60-portrait.m3u8"));
        assert_eq!(r.master, twitch_view::narrow(&dual, View::Vertical), "the result's master is the narrowed text");
        let mut seen = std::collections::HashSet::new();
        assert!(r.available.iter().all(|n| seen.insert(n.clone())), "one name per tier: {:?}", r.available);

        // A vertical-only stream on v1 is portrait with no view to pick.
        let r = build_served(
            "test-served-vo".into(),
            fixture("vertical_only_v1"),
            None,
            false,
            status("test-served-vo"),
            "best",
            View::Vertical,
        )
        .unwrap();
        assert_eq!((r.view, r.vertical_available, r.portrait), (View::Landscape, false, Some(true)));
        assert_eq!(r.url, "https://v1.example/chunked.m3u8");
    }

    fn answered(text: Option<String>) -> auth_proxy::DualFetch {
        auth_proxy::DualFetch { handle: tokio::spawn(async move { text }), started: Instant::now() }
    }

    fn never() -> auth_proxy::DualFetch {
        auth_proxy::DualFetch {
            handle: tokio::spawn(async {
                tokio::time::sleep(Duration::from_secs(3600)).await;
                None
            }),
            started: Instant::now(),
        }
    }

    #[tokio::test]
    async fn a_vertical_start_reads_v2_and_every_answer_replaces_the_note() {
        let tall = Some(DualAsk { want: View::Vertical, audio_only: false });
        let (m, available, late) = read_v2("test-read-dual", Some(answered(Some(fixture("dual_v2")))), tall, None).await;
        assert!(m.is_some() && available && late.is_none());
        assert_eq!(twitch_view::dual_note("test-read-dual"), Some(true));

        let (_, available, _) = read_v2("test-read-single", Some(answered(Some(MASTER_MODERN_IVS.into()))), tall, None).await;
        assert!(!available);
        assert_eq!(twitch_view::dual_note("test-read-single"), Some(false));

        // A failed v2 is no answer: the note stays as it was.
        let (m, available, late) = read_v2("test-read-failed", Some(answered(None)), tall, None).await;
        assert!(m.is_none() && !available && late.is_none());
        assert_eq!(twitch_view::dual_note("test-read-failed"), None);
    }

    #[tokio::test]
    async fn a_landscape_start_never_waits_for_v2() {
        let begun = Instant::now();
        let land = Some(DualAsk { want: View::Landscape, audio_only: false });
        let (m, available, late) = read_v2("test-read-land", Some(never()), land, None).await;
        assert!(m.is_none() && !available);
        late.expect("v2 is left running").abort();
        // Known not dual-format, or the audio swap: a vertical start does not wait either.
        twitch_view::note_dual("test-read-nodual", false);
        let tall = Some(DualAsk { want: View::Vertical, audio_only: false });
        let (_, _, late) = read_v2("test-read-nodual", Some(never()), tall, None).await;
        late.expect("not waited for").abort();
        let audio = Some(DualAsk { want: View::Vertical, audio_only: true });
        let (_, _, late) = read_v2("test-read-audio", Some(never()), audio, None).await;
        late.expect("not waited for").abort();
        assert!(begun.elapsed() < Duration::from_millis(900), "nothing waited");
    }

    #[tokio::test]
    async fn a_late_v2_settles_after_the_start_and_notes_it() {
        let late = LateV2 { fetch: answered(Some(fixture("dual_v2"))), key: "test-late".into(), pref: None };
        assert_eq!(settle_late_v2(late).await, Some(true));
        assert_eq!(twitch_view::dual_note("test-late"), Some(true));
        let h264 = Some(s(&["h264"]));
        let late = LateV2 { fetch: answered(Some(MASTER_MODERN_IVS.into())), key: "test-late-2".into(), pref: h264 };
        assert_eq!(settle_late_v2(late).await, Some(false));
        let late = LateV2 { fetch: answered(None), key: "test-late-3".into(), pref: None };
        assert_eq!(settle_late_v2(late).await, None, "no answer");
        assert_eq!(twitch_view::dual_note("test-late-3"), None);
    }

    #[test]
    fn extracts_vod_id() {
        assert_eq!(
            vod_id_from_url("https://www.twitch.tv/videos/1234567890").as_deref(),
            Some("1234567890")
        );
        assert_eq!(
            vod_id_from_url("https://twitch.tv/videos/987654321?t=1h2m3s").as_deref(),
            Some("987654321")
        );
        assert_eq!(vod_id_from_url("https://twitch.tv/shroud"), None);
    }

    #[test]
    fn extracts_clip_slug_both_shapes() {
        assert_eq!(
            clip_slug_from_url("https://clips.twitch.tv/FunnySlugName-abc123").as_deref(),
            Some("FunnySlugName-abc123")
        );
        assert_eq!(
            clip_slug_from_url("https://www.twitch.tv/somechannel/clip/CoolSlug-xyz?featured=true")
                .as_deref(),
            Some("CoolSlug-xyz")
        );
        assert_eq!(clip_slug_from_url("https://twitch.tv/shroud"), None);
        assert_eq!(clip_slug_from_url("https://twitch.tv/videos/123"), None);
    }

    #[test]
    fn clip_quality_best_worst_numeric() {
        let names = s(&["360p30", "480p30", "720p60", "1080p60"]);
        assert_eq!(
            pick_clip_quality("best", &names).as_deref(),
            Some("1080p60")
        );
        assert_eq!(
            pick_clip_quality("worst", &names).as_deref(),
            Some("360p30")
        );
        assert_eq!(
            pick_clip_quality("720p60", &names).as_deref(),
            Some("720p60")
        );
    }

    #[test]
    fn clip_token_appends_query_params() {
        assert_eq!(
            with_clip_token("https://clips-media.example/clip.mp4", "SIG", "TOK"),
            "https://clips-media.example/clip.mp4?sig=SIG&token=TOK"
        );
        assert!(
            with_clip_token("https://x.example/clip.mp4?foo=bar", "SIG", "TOK")
                .contains("?foo=bar&sig=SIG&token=TOK")
        );
    }
}

// ---------------------------------------------------------------------------
// VOD metadata (status / length / timeline anchors), web GQL
// ---------------------------------------------------------------------------

/// What the player needs to know about a VOD beyond its playlist: whether the
/// broadcast is still being recorded (the playlist is a growing EVENT), how
/// long it is right now, and where its timeline starts relative to the live
/// broadcast (`recorded_at`).
#[derive(Debug, Clone, Serialize)]
pub struct VodInfo {
    pub video_id: String,
    /// "recording" while live, "recorded" when finished, "unknown" if GQL
    /// gave nothing usable.
    pub status: String,
    pub length_seconds: Option<u32>,
    pub recorded_at: Option<String>,
    pub owner_login: Option<String>,
    pub title: Option<String>,
    pub thumbnail_url: Option<String>,
    /// The category the video is filed under. None when it has none.
    pub category: Option<MediaCategory>,
    /// Audio ranges Twitch muted, merged and clamped. Empty means none are
    /// KNOWN: on a recording VOD Twitch has not run audio recognition yet,
    /// which is not the same as "no mutes" (the GQL connection comes back
    /// `null` in both cases; probed 2026-09-12).
    pub muted_segments: Vec<MutedRange>,
    /// Category changes over the broadcast, sorted and clamped. Empty when
    /// Twitch reports none (one category, highlight, upload).
    pub chapters: Vec<Chapter>,
    /// Seek-preview sprite sheets. Only a finished VOD has them.
    pub storyboard: Option<Storyboard>,
}

impl VodInfo {
    pub fn is_recording(&self) -> bool {
        self.status == "recording"
    }
}

/// The recording VOD of a channel that is live right now, with the broadcast
/// start so a "behind live" distance can be mapped onto the VOD timeline.
#[derive(Debug, Clone)]
pub struct LiveArchive {
    pub video_id: String,
    pub length_seconds: u32,
    /// Broadcast time at VOD position 0 (ISO 8601). Measured 2026-09-06 at
    /// ~4 s after `stream.createdAt`.
    pub recorded_at: Option<String>,
    pub stream_created_at: Option<String>,
}

impl LiveArchive {
    /// Seconds of broadcast elapsed at `now`, on the VOD timeline.
    pub fn elapsed_secs(&self, now: chrono::DateTime<chrono::Utc>) -> Option<f64> {
        let anchor = self
            .recorded_at
            .as_deref()
            .or(self.stream_created_at.as_deref())?;
        let start = chrono::DateTime::parse_from_rfc3339(anchor).ok()?;
        let secs = (now - start.with_timezone(&chrono::Utc)).num_milliseconds() as f64 / 1000.0;
        Some(secs.max(0.0))
    }

    /// Clamp an absolute broadcast position onto what the recording can
    /// serve (its tail runs ~30-40 s behind live, and the reported length
    /// lags too).
    pub fn clamp_position(&self, position_secs: f64) -> f64 {
        let max_pos = (self.length_seconds as f64 - 15.0).max(0.0);
        if !position_secs.is_finite() {
            return 0.0;
        }
        position_secs.clamp(0.0, max_pos)
    }

    /// Map a "this far behind live" request onto a VOD position, keeping clear
    /// of the tail (the recording runs ~30-40 s behind the live edge, and its
    /// reported length lags too). `behind_secs` None means "from the start".
    pub fn position_for(&self, now: chrono::DateTime<chrono::Utc>, behind_secs: Option<f64>) -> f64 {
        let tail_guard = 15.0;
        let max_pos = (self.length_seconds as f64 - tail_guard).max(0.0);
        let Some(behind) = behind_secs else {
            return 0.0;
        };
        let Some(elapsed) = self.elapsed_secs(now) else {
            return 0.0;
        };
        (elapsed - behind.max(0.0)).clamp(0.0, max_pos)
    }
}

/// One anonymous web-GQL query. VOD metadata is public, so no OAuth is sent:
/// the answer is the same for everyone and the token stays out of an extra
/// request.
async fn gql_web_query(query: &str, variables: Value) -> Result<Value> {
    let client = auth_proxy::web_client();
    let body = json!({ "query": query, "variables": variables });
    let resp: Value = client
        .post("https://gql.twitch.tv/gql")
        .header("Client-ID", auth_proxy::TWITCH_WEB_CLIENT_ID)
        .json(&body)
        .send()
        .await
        .context("GQL request failed")?
        .json()
        .await
        .context("GQL response not JSON")?;
    Ok(resp)
}

fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// Status / length / anchors for one VOD. Never fails playback: a GQL hiccup
/// yields `status: "unknown"` and the player falls back to sniffing the
/// playlist itself.
pub async fn fetch_vod_info(vod_id: &str) -> VodInfo {
    let unknown = VodInfo {
        video_id: vod_id.to_string(),
        status: "unknown".to_string(),
        length_seconds: None,
        recorded_at: None,
        owner_login: None,
        title: None,
        thumbnail_url: None,
        category: None,
        muted_segments: Vec::new(),
        chapters: Vec::new(),
        storyboard: None,
    };
    // `muteInfo` is free here: it rides the call this function already makes.
    // Verified 2026-09-12 that it resolves under the WEB client id this
    // transport uses, and on ARCHIVE / HIGHLIGHT / UPLOAD alike, so adding it
    // cannot turn a non-archive video into the `unknown` sentinel below.
    // `moments` and `seekPreviewsURL` were verified the same way 2026-09-21
    // under the same client id on ARCHIVE / HIGHLIGHT / UPLOAD.
    let query = r#"query StreamNookVodInfo($id: ID!) {
        video(id: $id) {
            id status lengthSeconds recordedAt title
            owner { login }
            game { id name }
            previewThumbnailURL(width: 440, height: 248)
            muteInfo { mutedSegmentConnection { nodes { offset duration } } }
            seekPreviewsURL
            moments(momentRequestType: VIDEO_CHAPTER_MARKERS) {
                edges { node {
                    positionMilliseconds durationMilliseconds description
                    details { ... on GameChangeMomentDetails { game { id boxArtURL } } }
                } }
            }
        }
    }"#;
    let resp = match gql_web_query(query, json!({ "id": vod_id })).await {
        Ok(r) => r,
        Err(e) => {
            debug!("[Resolver] vod info {} unavailable: {}", vod_id, e);
            return unknown;
        }
    };
    let Some(video) = resp.pointer("/data/video").filter(|v| !v.is_null()) else {
        return unknown;
    };
    let length_seconds = video
        .get("lengthSeconds")
        .and_then(|v| v.as_u64())
        .map(|n| n.min(u32::MAX as u64) as u32);
    // A null connection is the normal shape for an unmuted (or still
    // recording) video, so it yields an empty list rather than an error.
    let mute_nodes: Vec<(i64, i64)> = video
        .pointer("/muteInfo/mutedSegmentConnection/nodes")
        .and_then(|n| n.as_array())
        .map(|nodes| {
            nodes
                .iter()
                .filter_map(|n| {
                    Some((n.get("offset")?.as_i64()?, n.get("duration")?.as_i64()?))
                })
                .collect()
        })
        .unwrap_or_default();
    let status = str_field(video, "status")
        .map(|s| s.to_lowercase())
        .unwrap_or_else(|| "unknown".to_string());
    let chapter_nodes: Vec<ChapterNode> = video
        .pointer("/moments/edges")
        .and_then(|e| e.as_array())
        .map(|edges| {
            edges
                .iter()
                .filter_map(|e| {
                    let n = e.get("node")?;
                    let game = n.pointer("/details/game");
                    Some(ChapterNode {
                        position_ms: n.get("positionMilliseconds")?.as_i64()?,
                        duration_ms: n
                            .get("durationMilliseconds")
                            .and_then(|d| d.as_i64())
                            .unwrap_or(0),
                        title: str_field(n, "description").unwrap_or_default(),
                        game_id: game.and_then(|g| str_field(g, "id")),
                        box_art_url: game.and_then(|g| str_field(g, "boxArtURL")),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    // Sheets exist only once the broadcast has ended; a recording's manifest
    // URL is advertised but answers 403 (probed 2026-09-21), so do not ask.
    let storyboard = match (status.as_str(), str_field(video, "seekPreviewsURL")) {
        ("recorded", Some(url)) => vod_storyboard::fetch(&url).await,
        _ => None,
    };
    VodInfo {
        video_id: vod_id.to_string(),
        status,
        length_seconds,
        recorded_at: str_field(video, "recordedAt"),
        owner_login: video
            .get("owner")
            .and_then(|o| str_field(o, "login"))
            .map(|s| s.to_lowercase()),
        title: str_field(video, "title"),
        thumbnail_url: str_field(video, "previewThumbnailURL"),
        category: media_category(video.get("game")),
        muted_segments: crate::services::muted_segments::normalize(&mute_nodes, length_seconds),
        chapters: vod_chapters::normalize(&chapter_nodes, length_seconds),
        storyboard,
    }
}

/// The recording VOD of a live channel, or None when the channel is offline
/// or has VODs disabled (Twitch then returns no `archiveVideo`).
pub async fn fetch_live_archive(login: &str) -> Result<Option<LiveArchive>> {
    let query = r#"query StreamNookLiveArchive($login: String!) {
        user(login: $login) {
            stream {
                createdAt
                archiveVideo { id status lengthSeconds recordedAt }
            }
        }
    }"#;
    let resp = gql_web_query(query, json!({ "login": login })).await?;
    let Some(stream) = resp.pointer("/data/user/stream").filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let Some(archive) = stream.get("archiveVideo").filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let Some(video_id) = str_field(archive, "id") else {
        return Ok(None);
    };
    Ok(Some(LiveArchive {
        video_id,
        length_seconds: archive
            .get("lengthSeconds")
            .and_then(|v| v.as_u64())
            .map(|n| n.min(u32::MAX as u64) as u32)
            .unwrap_or(0),
        recorded_at: str_field(archive, "recordedAt"),
        stream_created_at: str_field(stream, "createdAt"),
    }))
}

#[cfg(test)]
mod live_archive_tests {
    use super::*;

    fn archive() -> LiveArchive {
        LiveArchive {
            video_id: "1".into(),
            length_seconds: 24511,
            recorded_at: Some("2026-09-06T00:14:59Z".into()),
            stream_created_at: Some("2026-09-06T00:14:55Z".into()),
        }
    }

    fn at(s: &str) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339(s)
            .unwrap()
            .with_timezone(&chrono::Utc)
    }

    #[test]
    fn behind_maps_onto_the_vod_timeline() {
        // 07:03:59 - 00:14:59 = 24540 s elapsed; 30 min behind = 22740.
        let now = at("2026-09-06T07:03:59Z");
        let pos = archive().position_for(now, Some(1800.0));
        assert!((pos - 22740.0).abs() < 0.5, "got {pos}");
    }

    #[test]
    fn from_start_is_zero() {
        assert_eq!(archive().position_for(at("2026-09-06T07:03:59Z"), None), 0.0);
    }

    #[test]
    fn small_behind_clamps_clear_of_the_tail() {
        // 10 s behind live would land past the tail of the recording; clamp.
        let pos = archive().position_for(at("2026-09-06T07:03:59Z"), Some(10.0));
        assert_eq!(pos, 24511.0 - 15.0);
    }

    #[test]
    fn missing_anchor_falls_back_to_start() {
        let mut a = archive();
        a.recorded_at = None;
        a.stream_created_at = None;
        assert_eq!(a.position_for(at("2026-09-06T07:03:59Z"), Some(60.0)), 0.0);
    }
}

#[cfg(test)]
mod media_category_tests {
    use super::*;

    #[test]
    fn a_named_game_is_the_category() {
        let game = json!({ "id": "32982", "name": "Grand Theft Auto V" });
        assert_eq!(
            media_category(Some(&game)),
            Some(MediaCategory {
                id: "32982".into(),
                name: "Grand Theft Auto V".into()
            })
        );
    }

    #[test]
    fn no_game_or_no_name_is_none() {
        assert_eq!(media_category(None), None);
        assert_eq!(media_category(Some(&Value::Null)), None);
        assert_eq!(media_category(Some(&json!({ "id": "1", "name": "  " }))), None);
        assert_eq!(media_category(Some(&json!({ "id": "1" }))), None);
    }

    #[test]
    fn a_name_without_an_id_still_shows() {
        let got = media_category(Some(&json!({ "name": "Just Chatting" }))).unwrap();
        assert_eq!((got.id.as_str(), got.name.as_str()), ("", "Just Chatting"));
    }

    #[test]
    fn the_clip_document_asks_for_the_game() {
        let compact: String = CLIP_ACCESS_TOKEN_QUERY.split_whitespace().collect();
        assert!(compact.contains("game{idname}"), "{compact}");
    }
}
