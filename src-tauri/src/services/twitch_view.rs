//! Which picture of a Twitch stream plays: the landscape ladder or, on a
//! dual-format broadcast, the portrait one (SN-40, the phones' vertical view).
//!
//! Twitch sends a dual-format broadcast's portrait ladder only on the v2 usher
//! (`/api/v2/channel/hls/{ch}.m3u8`, `/vod/v2/{id}.m3u8`, asked with
//! `multigroup_video=true`). There every `#EXT-X-STREAM-INF` names its ladder
//! with `IVS-GROUPS="landscape"` or `"portrait"` (audio carries
//! `"landscape,portrait"`), every tier is flagged as the source, and the line
//! order changes on every request. Both ladders reuse the same names, so they
//! must never meet: a vertical start narrows the v2 master to its portrait
//! group before anything parses it, and every consumer downstream (the quality
//! menu, `link_adapt`'s ladder, the relay) re-parses that narrowed text.
//!
//! A vertical-only stream (a phone broadcast) is different: its ladder itself
//! is portrait, comes on v1 untagged, and needs no view at all. The short edge
//! (`short_edge`) is what makes it select well.
//!
//! Everything here is pure (no I/O, no Tauri), so every platform compiles it
//! and its rules are tested on the desktop.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use crate::services::auth_proxy::extract_attr;

/// The picture a Twitch start serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Landscape,
    Vertical,
}

impl View {
    /// The wire name ("landscape" | "vertical"), any case. Anything else is
    /// no request at all.
    pub fn parse(s: &str) -> Option<View> {
        match s.trim().to_ascii_lowercase().as_str() {
            "landscape" => Some(View::Landscape),
            "vertical" => Some(View::Vertical),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            View::Landscape => "landscape",
            View::Vertical => "vertical",
        }
    }

    /// The `IVS-GROUPS` name of this view's ladder.
    fn group(self) -> &'static str {
        match self {
            View::Landscape => "landscape",
            View::Vertical => "portrait",
        }
    }
}

/// The view a start wants. `request` is what the shell sent; absent (or a
/// name this build does not know) means the remembered pick on a phone
/// (`video_player.vertical_view`) and landscape on the desktop. While a TV
/// owns the picture (Android TV screen mode) it is landscape whatever was
/// asked: a TV is landscape.
pub fn wanted(request: Option<&str>, remembered_vertical: bool, phone: bool, tv_owns: bool) -> View {
    if tv_owns {
        return View::Landscape;
    }
    match request.and_then(View::parse) {
        Some(view) => view,
        None if phone && remembered_vertical => View::Vertical,
        None => View::Landscape,
    }
}

fn is_stream_inf(line: &str) -> bool {
    line.trim_start().starts_with("#EXT-X-STREAM-INF:")
}

/// Whether one `#EXT-X-STREAM-INF` belongs to `view`'s ladder. A line without
/// `IVS-GROUPS` is landscape: every v1 master, every relay master and every
/// master from before dual format.
fn in_group(inf: &str, view: View) -> bool {
    match extract_attr(inf, "IVS-GROUPS") {
        Some(groups) => groups.split(',').any(|g| g.trim().eq_ignore_ascii_case(view.group())),
        None => view == View::Landscape,
    }
}

/// Which ladders a master tags on its VIDEO tiers: (portrait, landscape).
/// Audio sits in both groups and has no `RESOLUTION`, so it never counts.
pub fn tagged_groups(master: &str) -> (bool, bool) {
    let (mut portrait, mut landscape) = (false, false);
    for line in master.lines().filter(|l| is_stream_inf(l) && extract_attr(l, "RESOLUTION").is_some()) {
        let Some(groups) = extract_attr(line, "IVS-GROUPS") else {
            continue;
        };
        for g in groups.split(',') {
            match g.trim().to_ascii_lowercase().as_str() {
                "portrait" => portrait = true,
                "landscape" => landscape = true,
                _ => {}
            }
        }
    }
    (portrait, landscape)
}

/// A v2 master of a dual-format broadcast: tagged portrait video AND tagged
/// landscape video. Never decided by shape: a vertical-only stream's v2 master
/// is untagged portrait and is not dual-format.
pub fn is_dual_format(master: &str) -> bool {
    let (portrait, landscape) = tagged_groups(master);
    portrait && landscape
}

/// `master` with only `view`'s renditions. Every line that is not a rendition
/// is kept; a dropped rendition loses its `#EXT-X-STREAM-INF` and its URI (the
/// next non-empty, non-comment line, as `twitch_resolver::parse_master` pairs
/// them). Untagged renditions are landscape. The input's line ending (CRLF or
/// LF) and its final newline are kept.
pub fn narrow(master: &str, view: View) -> String {
    let eol = if master.contains("\r\n") { "\r\n" } else { "\n" };
    let mut out: Vec<&str> = Vec::new();
    let mut drop_uri = false;
    // `lines()` strips the `\r` of a CRLF line, so the join puts it back.
    for line in master.lines() {
        if is_stream_inf(line) {
            drop_uri = !in_group(line, view);
            if !drop_uri {
                out.push(line);
            }
            continue;
        }
        let t = line.trim();
        if drop_uri && !t.is_empty() && !t.starts_with('#') {
            drop_uri = false;
            continue;
        }
        out.push(line);
    }
    let mut text = out.join(eol);
    if master.ends_with('\n') {
        text.push_str(eol);
    }
    text
}

/// A video tier's short edge: its height on a landscape tier, its width on a
/// portrait one. This is what a tier is measured by against the display and
/// against another tier, so a 1080x1920 tier counts as 1080, never 1920. None
/// for audio (no height).
pub fn short_edge(width: Option<u32>, height: Option<u32>) -> Option<u32> {
    let h = height?;
    Some(width.map_or(h, |w| w.min(h)))
}

/// Whether a ladder's video tiers are taller than wide, from every tier whose
/// width and height are known (the served ladder, not the chosen tier, so the
/// audio-only rendition keeps the answer). None when no tier says.
pub fn ladder_portrait(dims: impl IntoIterator<Item = (Option<u32>, Option<u32>)>) -> Option<bool> {
    let (mut tall, mut wide) = (0u32, 0u32);
    for (w, h) in dims {
        if let (Some(w), Some(h)) = (w, h) {
            if h > w {
                tall += 1;
            } else {
                wide += 1;
            }
        }
    }
    (tall + wide > 0).then_some(tall > wide)
}

/// How a live Twitch start is served.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServePlan {
    /// A phone's landscape start: today's path, the in-core ad relay seam
    /// (after the plugin seam, which a phone never fills).
    Relay,
    /// The desktop's landscape start: today's path, the core master with the
    /// playback plugin's seam.
    Direct,
    /// The portrait ladder straight from Twitch: no plugin, no relay. A relay
    /// fetches v1 and cannot serve it, and the plugin would be handed a
    /// portrait master it was never built for.
    Vertical,
}

/// The plan for a start that wanted `want`, where `vertical_built` says the
/// resolver built the portrait ladder (the v2 master had a playable portrait
/// group).
pub fn serve_plan(want: View, vertical_built: bool, phone: bool) -> ServePlan {
    if want == View::Vertical && vertical_built {
        ServePlan::Vertical
    } else if phone {
        ServePlan::Relay
    } else {
        ServePlan::Direct
    }
}

/// The per-request usher deadline (`auth_proxy::web_client`'s 8 s): the
/// longest a start waits for v2 when the channel is known to be dual-format.
pub const V2_DEADLINE: Duration = Duration::from_secs(8);

/// How long past v1 a start waits for v2 on a channel it knows nothing about.
pub const V2_GRACE: Duration = Duration::from_secs(1);

/// When a start stops waiting for v2 and serves with what it has (Z5).
///
/// A landscape start, and the screen-off audio swap, never wait: v2 runs
/// alongside and its answer is read when the start returns (later answers
/// arrive as `vertical-available`). A vertical start waits by what this
/// channel's note says: known dual-format, up to the request deadline (a
/// portrait picture is the whole point); known not dual-format, not at all;
/// no note, v1 plus a second.
pub fn v2_wait_until(want: View, note: Option<bool>, audio_only: bool, v2_started: Instant, v1_done: Instant) -> Instant {
    if want == View::Landscape || audio_only {
        return v1_done;
    }
    match note {
        Some(true) => (v2_started + V2_DEADLINE).max(v1_done),
        Some(false) => v1_done,
        None => v1_done + V2_GRACE,
    }
}

/// The last v2 answer per channel (Y11): true when its master had a playable
/// portrait group. Every v2 answer replaces it; a v2 that never answered
/// leaves it. In memory only: a broadcast's format can change between
/// sessions, and the note only sizes the next start's wait.
static DUAL_NOTES: LazyLock<Mutex<HashMap<String, bool>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// The note for `key` (a channel login, or `vod_key`).
pub fn dual_note(key: &str) -> Option<bool> {
    DUAL_NOTES.lock().unwrap_or_else(|e| e.into_inner()).get(&key.to_ascii_lowercase()).copied()
}

/// A v2 answer for `key`.
pub fn note_dual(key: &str, available: bool) {
    DUAL_NOTES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key.to_ascii_lowercase(), available);
}

/// The note key of a VOD (its channel is not known when it resolves).
pub fn vod_key(vod_id: &str) -> String {
    format!("vod:{vod_id}")
}

/// The solo player's start ticket. Every start (live, VOD, a live rewind, a
/// promoted tile) mints one through `mint_start_ticket`, the only writer, and
/// returns it; a late v2 answer and a view pick act only for the ticket that
/// is still current.
static START_TICKET: AtomicU64 = AtomicU64::new(0);

/// A new solo start: its ticket, which supersedes every earlier one.
pub fn mint_start_ticket() -> u64 {
    START_TICKET.fetch_add(1, Ordering::AcqRel) + 1
}

/// Whether `ticket` is still the latest solo start. 0 is never issued.
pub fn start_ticket_is_current(ticket: u64) -> bool {
    ticket != 0 && START_TICKET.load(Ordering::Acquire) == ticket
}

/// Whether a view pick is remembered (Z1): only when the start served what
/// was picked and is still the current one. A Vertical pick served Landscape
/// (the broadcast stopped sending portrait) saves nothing.
pub fn saves_pick(picked: View, served: View, ticket_current: bool) -> bool {
    ticket_current && picked == served
}

/// Whether picking Vertical just gave up ad-free playback (Y18, X21): the
/// pick served Vertical, ad-free is switched on, and the landscape session
/// before it (same channel) was relayed for a viewer Twitch does not already
/// serve ad-free. Rotations and restarts never ask.
pub fn gave_up_ad_free(
    picked: View,
    served: Option<View>,
    ad_free_on: bool,
    before_mode: Option<&str>,
    before_entitled: bool,
) -> bool {
    picked == View::Vertical
        && served == Some(View::Vertical)
        && ad_free_on
        && !before_entitled
        && before_mode == Some("proxy")
}

/// A requested quality that is the audio-only rendition.
pub fn is_audio_only(quality: &str) -> bool {
    matches!(quality.trim().to_ascii_lowercase().as_str(), "audio_only" | "audio-only" | "audio")
}

/// Shared with the resolver's and the relay's tests: the fixtures, and the
/// line orders Twitch shuffles v2 into.
#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn fixture(name: &str) -> String {
        let raw = match name {
            "dual_v2" => include_str!("fixtures/twitch_view/dual_v2.m3u8"),
            "vertical_only_v1" => include_str!("fixtures/twitch_view/vertical_only_v1.m3u8"),
            "vertical_only_v2" => include_str!("fixtures/twitch_view/vertical_only_v2.m3u8"),
            other => panic!("no fixture {other}"),
        };
        raw.replace("\r\n", "\n")
    }

    /// `master` with its renditions (each an INF line and its URI) in `order`.
    fn shuffled(master: &str, order: &[usize]) -> String {
        let lines: Vec<&str> = master.lines().collect();
        let first = lines.iter().position(|l| is_stream_inf(l)).expect("a rendition");
        let pairs: Vec<(&str, &str)> = lines[first..].chunks(2).map(|c| (c[0], c[1])).collect();
        assert_eq!(order.len(), pairs.len());
        let mut out: Vec<&str> = lines[..first].to_vec();
        for &i in order {
            out.push(pairs[i].0);
            out.push(pairs[i].1);
        }
        out.join("\n") + "\n"
    }

    /// Three line orders, as Twitch shuffles v2 on every request.
    pub(crate) fn three_shuffles(master: &str) -> Vec<String> {
        let n = master.lines().filter(|l| is_stream_inf(l)).count();
        let reversed: Vec<usize> = (0..n).rev().collect();
        let rotated: Vec<usize> = (0..n).map(|i| (i + 4) % n).collect();
        let odd_even: Vec<usize> = (0..n).filter(|i| i % 2 == 1).chain((0..n).filter(|i| i % 2 == 0)).collect();
        vec![shuffled(master, &reversed), shuffled(master, &rotated), shuffled(master, &odd_even)]
    }

    fn uris(master: &str) -> Vec<String> {
        let lines: Vec<&str> = master.lines().collect();
        lines
            .iter()
            .enumerate()
            .filter(|(_, l)| is_stream_inf(l))
            .map(|(i, _)| lines[i + 1].trim().to_string())
            .collect()
    }

    #[test]
    fn narrow_keeps_one_ladder_and_the_audio_on_every_line_order() {
        let dual = fixture("dual_v2");
        let mut orders = three_shuffles(&dual);
        orders.push(dual.clone());
        for m in orders {
            let land = narrow(&m, View::Landscape);
            let land_uris = uris(&land);
            assert_eq!(land_uris.len(), 6, "5 landscape tiers and the audio: {land}");
            assert!(land_uris.iter().all(|u| !u.contains("-portrait")), "{land}");
            assert!(land_uris.iter().any(|u| u.ends_with("/audio_only.m3u8")));

            let tall = narrow(&m, View::Vertical);
            let tall_uris = uris(&tall);
            assert_eq!(tall_uris.len(), 6, "5 portrait tiers and the audio: {tall}");
            assert!(
                tall_uris.iter().all(|u| u.contains("-portrait") || u.ends_with("/audio_only.m3u8")),
                "{tall}"
            );
            // Lines that are not renditions stay, in both.
            for kept in ["#EXTM3U", "#EXT-X-SESSION-DATA:DATA-ID=\"NODE\"", "TRANSCODESTACK"] {
                assert!(land.contains(kept) && tall.contains(kept), "{kept}");
            }
            assert!(tall.ends_with('\n'));
        }
    }

    #[test]
    fn an_untagged_rendition_is_landscape() {
        let m = "#EXTM3U\n\
#EXT-X-STREAM-INF:BANDWIDTH=1,RESOLUTION=1280x720\n\
https://x/untagged.m3u8\n\
#EXT-X-STREAM-INF:BANDWIDTH=1,RESOLUTION=720x1280,IVS-GROUPS=\"portrait\"\n\
https://x/tall.m3u8\n";
        assert_eq!(uris(&narrow(m, View::Landscape)), ["https://x/untagged.m3u8"]);
        assert_eq!(uris(&narrow(m, View::Vertical)), ["https://x/tall.m3u8"]);
        // A v1 master (no groups at all) is its own landscape narrowing.
        let v1 = fixture("vertical_only_v1");
        assert_eq!(narrow(&v1, View::Landscape), v1);
        assert!(uris(&narrow(&v1, View::Vertical)).is_empty());
    }

    #[test]
    fn narrow_keeps_crlf_line_ends() {
        let lf = fixture("dual_v2");
        let crlf = lf.replace('\n', "\r\n");
        let tall = narrow(&crlf, View::Vertical);
        assert_eq!(tall, narrow(&lf, View::Vertical).replace('\n', "\r\n"));
        assert!(tall.ends_with("\r\n"));
        assert!(!tall.replace("\r\n", "").contains('\n'), "no bare LF in a CRLF master");
        assert_eq!(uris(&tall).len(), 6);
    }

    #[test]
    fn dual_format_needs_both_tagged_ladders() {
        assert!(is_dual_format(&fixture("dual_v2")));
        assert_eq!(tagged_groups(&fixture("dual_v2")), (true, true));
        // Vertical-only v2 carries no IVS-GROUPS: portrait by shape, not dual.
        assert!(!is_dual_format(&fixture("vertical_only_v2")));
        assert!(!is_dual_format(&fixture("vertical_only_v1")));
        // Audio tagged both ways is not a portrait ladder.
        let audio_only_tags = "#EXTM3U\n\
#EXT-X-STREAM-INF:BANDWIDTH=1,RESOLUTION=1920x1080,IVS-GROUPS=\"landscape\"\nhttps://x/1080.m3u8\n\
#EXT-X-STREAM-INF:BANDWIDTH=1,CODECS=\"mp4a.40.2\",IVS-GROUPS=\"landscape,portrait\"\nhttps://x/a.m3u8\n";
        assert_eq!(tagged_groups(audio_only_tags), (false, true));
        assert!(!is_dual_format(audio_only_tags));
    }

    #[test]
    fn the_short_edge_is_the_smaller_side() {
        assert_eq!(short_edge(Some(1920), Some(1080)), Some(1080));
        assert_eq!(short_edge(Some(1080), Some(1920)), Some(1080));
        assert_eq!(short_edge(Some(720), Some(1280)), Some(720));
        assert_eq!(short_edge(None, Some(720)), Some(720), "height alone stands for itself");
        assert_eq!(short_edge(Some(720), None), None, "audio has no height");
        assert_eq!(short_edge(None, None), None);
    }

    #[test]
    fn a_ladder_is_portrait_by_its_tiers() {
        assert_eq!(ladder_portrait([(Some(720), Some(1280)), (Some(404), Some(720)), (None, None)]), Some(true));
        assert_eq!(ladder_portrait([(Some(1920), Some(1080)), (None, None)]), Some(false));
        assert_eq!(ladder_portrait([(None, None)]), None, "only audio: unknown");
        assert_eq!(ladder_portrait(std::iter::empty::<(Option<u32>, Option<u32>)>()), None);
    }

    #[test]
    fn the_wanted_view() {
        // Explicit always wins (except a TV).
        assert_eq!(wanted(Some("vertical"), false, true, false), View::Vertical);
        assert_eq!(wanted(Some("Landscape"), true, true, false), View::Landscape);
        // Absent: remembered on a phone, landscape on the desktop.
        assert_eq!(wanted(None, true, true, false), View::Vertical);
        assert_eq!(wanted(None, false, true, false), View::Landscape);
        assert_eq!(wanted(None, true, false, false), View::Landscape);
        // An unknown name is no request.
        assert_eq!(wanted(Some("sideways"), true, true, false), View::Vertical);
        // TV screen mode: always landscape.
        assert_eq!(wanted(Some("vertical"), true, true, true), View::Landscape);
        assert_eq!(wanted(None, true, true, true), View::Landscape);
    }

    #[test]
    fn the_serve_plan() {
        assert_eq!(serve_plan(View::Vertical, true, true), ServePlan::Vertical);
        assert_eq!(serve_plan(View::Vertical, true, false), ServePlan::Vertical);
        // No portrait ladder built: today's path.
        assert_eq!(serve_plan(View::Vertical, false, true), ServePlan::Relay);
        assert_eq!(serve_plan(View::Vertical, false, false), ServePlan::Direct);
        assert_eq!(serve_plan(View::Landscape, true, true), ServePlan::Relay);
        assert_eq!(serve_plan(View::Landscape, false, false), ServePlan::Direct);
    }

    #[test]
    fn the_v2_wait_rule() {
        let t0 = Instant::now();
        let v2_started = t0 + Duration::from_millis(60);
        let v1_done = t0 + Duration::from_millis(140);
        let at = |want, note, audio| v2_wait_until(want, note, audio, v2_started, v1_done);
        // Landscape never waits past v1, whatever the note.
        for note in [None, Some(true), Some(false)] {
            assert_eq!(at(View::Landscape, note, false), v1_done);
        }
        // Vertical: known dual waits up to the request deadline.
        assert_eq!(at(View::Vertical, Some(true), false), v2_started + V2_DEADLINE);
        // Known not dual: no wait.
        assert_eq!(at(View::Vertical, Some(false), false), v1_done);
        // Unknown: v1 plus a second.
        assert_eq!(at(View::Vertical, None, false), v1_done + Duration::from_secs(1));
        // The audio swap never waits.
        assert_eq!(at(View::Vertical, Some(true), true), v1_done);
        // A v1 slower than the deadline is still waited for (never earlier).
        let late_v1 = v2_started + Duration::from_secs(9);
        assert_eq!(v2_wait_until(View::Vertical, Some(true), false, v2_started, late_v1), late_v1);
    }

    #[test]
    fn every_v2_answer_replaces_the_note() {
        assert_eq!(dual_note("test-note-chan"), None);
        note_dual("Test-Note-Chan", true);
        assert_eq!(dual_note("test-note-chan"), Some(true));
        note_dual("test-note-chan", false);
        assert_eq!(dual_note("TEST-NOTE-CHAN"), Some(false));
        assert_eq!(dual_note(&vod_key("123")), None);
        assert_eq!(vod_key("123"), "vod:123");
    }

    #[test]
    fn a_newer_start_supersedes_the_ticket() {
        let first = mint_start_ticket();
        let second = mint_start_ticket();
        assert!(second > first && first > 0);
        assert!(!start_ticket_is_current(first));
        assert!(!start_ticket_is_current(0), "0 is never a start");
    }

    #[test]
    fn a_pick_is_saved_only_when_served_and_current() {
        assert!(saves_pick(View::Vertical, View::Vertical, true));
        assert!(saves_pick(View::Landscape, View::Landscape, true));
        assert!(!saves_pick(View::Vertical, View::Landscape, true), "Vertical served Landscape saves nothing");
        assert!(!saves_pick(View::Vertical, View::Vertical, false), "a newer start took over");
    }

    #[test]
    fn ad_free_is_given_up_only_by_a_vertical_pick_off_a_relayed_session() {
        let v = View::Vertical;
        assert!(gave_up_ad_free(v, Some(v), true, Some("proxy"), false));
        assert!(!gave_up_ad_free(View::Landscape, Some(View::Landscape), true, Some("proxy"), false));
        assert!(!gave_up_ad_free(v, Some(View::Landscape), true, Some("proxy"), false), "served landscape");
        assert!(!gave_up_ad_free(v, Some(v), false, Some("proxy"), false), "ad-free off");
        assert!(!gave_up_ad_free(v, Some(v), true, Some("subscribed"), true), "entitled");
        assert!(!gave_up_ad_free(v, Some(v), true, Some("auth-only"), false), "no relay answered before");
        assert!(!gave_up_ad_free(v, Some(v), true, None, false), "no session before");
    }

    #[test]
    fn audio_only_names() {
        assert!(is_audio_only("audio_only") && is_audio_only(" Audio ") && is_audio_only("audio-only"));
        assert!(!is_audio_only("best") && !is_audio_only("720p60"));
    }
}
