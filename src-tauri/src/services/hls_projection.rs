//! Stable Live Projection: makes the whole-segment live playlists the core
//! serves refresh-stable for hls.js, independent of upstream URL re-signing.
//!
//! hls.js reconciles every refresh of the SAME playlist URL by matching
//! fragments on their media-sequence number and requiring, for any fragment
//! present in both the old and new playlist, an identical URL path (the trailing
//! `?query` stripped) AND an identical discontinuity counter. Twitch re-signs a
//! segment's URL PATH on every poll (the signing token rides in the path, not
//! the query), so forwarding those URLs raw breaks the match: hls.js aborts the
//! refresh with `playlistParsingError` ("media sequence mismatch"), then
//! re-syncs and replays around the live edge. That is the ad-time replay/glitch.
//!
//! This module pins every segment to a stable synthetic `vseg/<sid>/<sn>.ts`
//! whose path never changes for a given media-sequence number, recording the
//! freshest real URL so a fetch of that synthetic path 302-redirects to it. The
//! synthetic path is the player-visible identity; the re-signing is invisible.
//!
//! Ad-neutral by construction: it knows nothing about ads. It guarantees an
//! hls.js invariant on whatever playlist it is handed, the same category of
//! transform as the `#EXT-X-TARGETDURATION` retarget the core already applies.
//! The experimental low-latency origin satisfies the same invariant a different
//! way (it owns segment bytes in memory and emits its own synthetic paths); this
//! is the stable whole-segment path's equivalent, and only ever runs when that
//! origin is inactive.
//!
//! Per SESSION (not global): the map is keyed by a session id (`"solo"` for the
//! solo relay, the tile id for each MultiNook tile) so two streams serving the
//! same media-sequence number never resolve to each other's segments.

use once_cell::sync::Lazy;
use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

/// Synthetic segment path prefix. Deliberately NOT `seg/` (owned by the
/// low-latency origin) so the two schemes can never be confused in routing.
pub const VSEG_PREFIX: &str = "vseg/";

/// Media-sequence numbers to retain per session behind the live edge. The live
/// window is ~12-15 segments and the player rides a few behind; 120 is a
/// generous margin so a just-rolled-off segment the player still wants resolves,
/// while the map stays tiny.
const RETAIN: u64 = 120;

/// One session's projection state: media-sequence number -> freshest real
/// (absolute) segment URL, plus the last window start seen so a stream restart
/// (the broadcast's sequence resets) clears stale entries.
#[derive(Default)]
struct SessionProj {
    map: BTreeMap<u64, String>,
    last_media_seq: Option<u64>,
    /// The newest sequence number the player has fetched (through a redirect).
    requested: Option<u64>,
    /// The first sequence number of the rendition switched to last
    /// (`begin_switch`); recorded segments below it keep their rendition.
    switch_floor: Option<u64>,
}

static PROJECTIONS: Lazy<Mutex<HashMap<String, SessionProj>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Forget a session's projection state (stream start/stop, or a pivot that
/// recreates the player). The next `stabilize` for this sid starts an empty
/// window. Safe to call when no state exists.
pub fn reset(sid: &str) {
    PROJECTIONS.lock().unwrap().remove(sid);
}

/// The relay is about to poll a different rendition of the same broadcast
/// (`link_adapt`). Every segment the player has already fetched keeps its
/// rendition; every later one, listed or not, resolves to the new rendition
/// from the next playlist recorded on. Nothing the player can see changes:
/// the synthetic paths, sequence numbers and discontinuity counts stay the
/// same, so hls.js accepts every refresh. That holds because Twitch's
/// renditions are cut from one timeline (the same sequence numbers, programme
/// dates and presentation timestamps), and hls.js builds a new init segment
/// when the resolution changes inside one discontinuity run. Starting at the
/// player's position rather than after the newest listed segment matters on a
/// slow link: a player that has fallen behind would otherwise still owe
/// several segments of the rendition it could not carry.
pub fn begin_switch(sid: &str) {
    let mut projections = PROJECTIONS.lock().unwrap();
    let proj = projections.entry(sid.to_string()).or_default();
    proj.switch_floor = Some(proj.requested.map_or(0, |r| r + 1));
}

/// Resolve a (possibly relative) segment URI against the upstream manifest base.
/// Twitch segments are absolute CDN URLs (returned as-is); the join covers
/// proxied playlists that use relative chunk paths.
fn resolve_segment_url(uri: &str, base_url: &str) -> String {
    if uri.starts_with("http://") || uri.starts_with("https://") {
        uri.to_string()
    } else {
        format!("{base_url}{uri}")
    }
}

/// The base URL (scheme + host + path up to the last `/`) a relative segment URI
/// resolves against, derived from the upstream manifest URL with its query
/// stripped. Mirrors how a browser resolves a relative playlist entry.
pub fn base_url_of(manifest_url: &str) -> String {
    let no_query = manifest_url.split('?').next().unwrap_or(manifest_url);
    match no_query.rfind('/') {
        Some(i) => no_query[..=i].to_string(),
        None => no_query.to_string(),
    }
}

/// Rewrite every media-segment URI in a live media playlist to a stable
/// synthetic `vseg/<sid>/<sn>.ts`, recording sn -> freshest real URL for the
/// redirect handler. `#EXT-X-MAP` (the init segment) and every tag pass through
/// untouched: hls.js's refresh check never compares the init segment, and the
/// player fetches it direct from the CDN. Returns the rewritten playlist.
///
/// No-op shape preserved: a playlist with no `#EXTINF` segments (a master, or an
/// empty ad-break coast) comes back unchanged except for trivial whitespace.
pub fn stabilize(sid: &str, playlist: &str, base_url: &str) -> String {
    let mut projections = PROJECTIONS.lock().unwrap();
    let proj = projections.entry(sid.to_string()).or_default();

    let mut sn: u64 = 0;
    let mut max_sn: Option<u64> = None;
    let mut expect_uri = false;
    let mut out = String::with_capacity(playlist.len() + 64);

    for line in playlist.lines() {
        let trimmed = line.trim();
        if let Some(v) = trimmed.strip_prefix("#EXT-X-MEDIA-SEQUENCE:") {
            sn = v.trim().parse().unwrap_or(0);
            // A new broadcast restarts the media-sequence: if the window start
            // moved BACKWARDS, the recorded URLs belong to a previous stream, so
            // a synthetic path could resolve to dead content. Clear and reseed.
            if proj.last_media_seq.is_some_and(|prev| sn < prev) {
                proj.map.clear();
                proj.requested = None;
                proj.switch_floor = None;
            }
            proj.last_media_seq = Some(sn);
            out.push_str(line);
            out.push('\n');
        } else if trimmed.starts_with("#EXTINF:") {
            expect_uri = true;
            out.push_str(line);
            out.push('\n');
        } else if expect_uri && !trimmed.is_empty() && !trimmed.starts_with('#') {
            // The segment URI for the preceding #EXTINF: record the real URL,
            // emit the stable synthetic path in its place.
            expect_uri = false;
            // Below a rendition switch, a segment the player already has keeps
            // the URL it was fetched from.
            let fetched = proj.switch_floor.is_some_and(|f| sn < f) && proj.map.contains_key(&sn);
            if !fetched {
                proj.map.insert(sn, resolve_segment_url(trimmed, base_url));
            }
            max_sn = Some(sn);
            out.push_str(&format!("{VSEG_PREFIX}{sid}/{sn}.ts\n"));
            sn += 1;
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }

    // Prune sns well behind the edge so a session's map can't grow unbounded.
    if let Some(edge) = max_sn {
        let keep_from = edge.saturating_sub(RETAIN);
        let stale: Vec<u64> = proj.map.range(..keep_from).map(|(k, _)| *k).collect();
        for k in stale {
            proj.map.remove(&k);
        }
    }

    out
}

/// Parse a synthetic segment request path (`vseg/<sid>/<sn>.ts`) into its parts.
/// Returns `None` for anything that is not a projection segment request, so the
/// caller falls through to normal handling.
pub fn parse_vseg_path(request_path: &str) -> Option<(String, u64)> {
    let rest = request_path.strip_prefix(VSEG_PREFIX)?;
    let (sid, file) = rest.rsplit_once('/')?;
    if sid.is_empty() {
        return None;
    }
    let sn: u64 = file.strip_suffix(".ts").unwrap_or(file).parse().ok()?;
    Some((sid.to_string(), sn))
}

/// The freshest real CDN URL recorded for `(sid, sn)`, for a 302 redirect.
/// `None` if the session/sn is unknown (the request 404s, and hls.js retries).
pub fn redirect_target(sid: &str, sn: u64) -> Option<String> {
    let mut projections = PROJECTIONS.lock().unwrap();
    let proj = projections.get_mut(sid)?;
    let url = proj.map.get(&sn).cloned()?;
    proj.requested = Some(proj.requested.map_or(sn, |r| r.max(sn)));
    Some(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clear(sid: &str) {
        reset(sid);
    }

    // ── URL stability: a segment keeps ONE synthetic path across re-signing ──

    #[test]
    fn each_sequence_gets_a_stable_synthetic_url() {
        clear("t1");
        let poll_n = "#EXTM3U\n\
#EXT-X-TARGETDURATION:2\n\
#EXT-X-MEDIA-SEQUENCE:100\n\
#EXT-X-MAP:URI=\"https://cdn/init.mp4\"\n\
#EXTINF:2.000,live\nhttps://cdn/a100.ts?dna=AAA\n\
#EXTINF:2.000,live\nhttps://cdn/a101.ts?dna=AAA\n";
        let out_n = stabilize("t1", poll_n, "https://cdn/");
        assert!(out_n.contains("vseg/t1/100.ts"));
        assert!(out_n.contains("vseg/t1/101.ts"));
        // The init segment passes through untouched (fetched direct from the CDN).
        assert!(out_n.contains("#EXT-X-MAP:URI=\"https://cdn/init.mp4\""));
        // Raw segment URLs no longer appear in the served playlist.
        assert!(!out_n.contains("a100.ts"));
        assert_eq!(
            redirect_target("t1", 100).as_deref(),
            Some("https://cdn/a100.ts?dna=AAA")
        );

        // Poll N+1: window advanced one; sn 101 re-signed (?dna=BBB). hls.js needs
        // sn 101 to keep the SAME path across refreshes; the synthetic path is
        // identical while the redirect target updates to the fresh signed URL.
        let poll_n1 = "#EXTM3U\n\
#EXT-X-TARGETDURATION:2\n\
#EXT-X-MEDIA-SEQUENCE:101\n\
#EXTINF:2.000,live\nhttps://cdn/a101.ts?dna=BBB\n\
#EXTINF:2.000,live\nhttps://cdn/a102.ts?dna=BBB\n";
        let out_n1 = stabilize("t1", poll_n1, "https://cdn/");
        assert!(out_n.contains("vseg/t1/101.ts") && out_n1.contains("vseg/t1/101.ts"));
        // Redirect target for sn 101 refreshed to the latest signed URL.
        assert_eq!(
            redirect_target("t1", 101).as_deref(),
            Some("https://cdn/a101.ts?dna=BBB")
        );
        clear("t1");
    }

    #[test]
    fn sessions_do_not_collide_on_shared_sequence_numbers() {
        clear("a");
        clear("b");
        let pl_a = "#EXTM3U\n#EXT-X-MEDIA-SEQUENCE:50\n#EXTINF:2.0,live\nhttps://cdn/aaa50.ts\n";
        let pl_b = "#EXTM3U\n#EXT-X-MEDIA-SEQUENCE:50\n#EXTINF:2.0,live\nhttps://cdn/bbb50.ts\n";
        stabilize("a", pl_a, "https://cdn/");
        stabilize("b", pl_b, "https://cdn/");
        // Same sn 50 in two sessions resolves to each session's own segment.
        assert_eq!(redirect_target("a", 50).as_deref(), Some("https://cdn/aaa50.ts"));
        assert_eq!(redirect_target("b", 50).as_deref(), Some("https://cdn/bbb50.ts"));
        clear("a");
        clear("b");
    }

    #[test]
    fn relative_segment_uris_resolve_against_base() {
        clear("rel");
        let pl = "#EXTM3U\n#EXT-X-MEDIA-SEQUENCE:7\n#EXTINF:2.0,live\nchunk7.ts\n";
        stabilize("rel", pl, "https://cdn/live/");
        assert_eq!(
            redirect_target("rel", 7).as_deref(),
            Some("https://cdn/live/chunk7.ts")
        );
        clear("rel");
    }

    #[test]
    fn restart_clears_stale_entries() {
        clear("rs");
        stabilize(
            "rs",
            "#EXTM3U\n#EXT-X-MEDIA-SEQUENCE:900\n#EXTINF:2.0,live\nhttps://cdn/x900.ts\n",
            "https://cdn/",
        );
        assert!(redirect_target("rs", 900).is_some());
        // A new broadcast restarts at a far lower media-sequence: the old entry
        // must not survive (its synthetic path would resolve to dead content).
        stabilize(
            "rs",
            "#EXTM3U\n#EXT-X-MEDIA-SEQUENCE:3\n#EXTINF:2.0,live\nhttps://cdn/y3.ts\n",
            "https://cdn/",
        );
        assert!(redirect_target("rs", 900).is_none());
        assert_eq!(redirect_target("rs", 3).as_deref(), Some("https://cdn/y3.ts"));
        clear("rs");
    }

    #[test]
    fn master_or_empty_playlist_unchanged() {
        clear("m");
        let master = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1000000\nchunked.m3u8\n";
        let out = stabilize("m", master, "https://cdn/");
        assert!(out.contains("chunked.m3u8"));
        assert!(!out.contains("vseg/"));
        clear("m");
    }

    #[test]
    fn vseg_path_parsing() {
        assert_eq!(parse_vseg_path("vseg/solo/12.ts"), Some(("solo".into(), 12)));
        assert_eq!(parse_vseg_path("vseg/tile-3/0.ts"), Some(("tile-3".into(), 0)));
        // Not a projection request.
        assert_eq!(parse_vseg_path("seg/12.ts"), None);
        assert_eq!(parse_vseg_path("stream.m3u8"), None);
        assert_eq!(parse_vseg_path("vseg/solo/notnum.ts"), None);
        assert_eq!(parse_vseg_path("vseg//5.ts"), None);
    }

    // ── Refresh-invariant harness (models hls.js mapFragmentIntersection) ──
    // Assign each segment its (sn, url-path-without-query, cc) exactly as hls.js
    // does: sn seeds from #EXT-X-MEDIA-SEQUENCE and bumps per URI; cc seeds from
    // #EXT-X-DISCONTINUITY-SEQUENCE and bumps on each #EXT-X-DISCONTINUITY. A
    // fragment present in two consecutive polls must agree on url-path AND cc.

    fn parse_segments(playlist: &str) -> Vec<(u64, String, u64)> {
        let lines: Vec<&str> = playlist.lines().collect();
        let mut sn: u64 = 0;
        let mut cc: u64 = 0;
        let mut out = Vec::new();
        let mut i = 0;
        while i < lines.len() {
            let t = lines[i].trim();
            if let Some(v) = t.strip_prefix("#EXT-X-MEDIA-SEQUENCE:") {
                sn = v.trim().parse().unwrap_or(0);
            } else if let Some(v) = t.strip_prefix("#EXT-X-DISCONTINUITY-SEQUENCE:") {
                cc = v.trim().parse().unwrap_or(0);
            } else if t == "#EXT-X-DISCONTINUITY" {
                cc += 1;
            } else if t.starts_with("#EXTINF:") {
                let mut j = i + 1;
                while j < lines.len()
                    && (lines[j].trim().is_empty() || lines[j].trim().starts_with('#'))
                {
                    j += 1;
                }
                if j < lines.len() {
                    let u = lines[j].trim();
                    let path = u.split('?').next().unwrap_or(u).to_string();
                    out.push((sn, path, cc));
                    sn += 1;
                    i = j;
                }
            }
            i += 1;
        }
        out
    }

    fn assert_refresh_consistent(a: &str, b: &str, label: &str) {
        let sa = parse_segments(a);
        let sb = parse_segments(b);
        let mut overlap = 0;
        for (sn_a, url_a, cc_a) in &sa {
            if let Some((_, url_b, cc_b)) = sb.iter().find(|(sn_b, _, _)| sn_b == sn_a) {
                overlap += 1;
                assert_eq!(url_a, url_b, "{label}: url path mismatch at sn {sn_a}");
                assert_eq!(cc_a, cc_b, "{label}: cc mismatch at sn {sn_a} ({cc_a}!={cc_b})");
            }
        }
        assert!(overlap >= 1, "{label}: expected SN overlap, got none");
    }

    #[test]
    fn url_invariant_holds_across_resigned_polls() {
        clear("inv");
        // Two polls of the same window, segment URLs fully re-signed (path token
        // rotates, not just the query). Raw, this trips hls.js; stabilized, the
        // synthetic paths are identical.
        let raw_n = "#EXTM3U\n#EXT-X-MEDIA-SEQUENCE:200\n\
#EXTINF:2.0,live\nhttps://cdn/TOKa/200.ts?s=1\n\
#EXTINF:2.0,live\nhttps://cdn/TOKa/201.ts?s=1\n";
        let raw_n1 = "#EXTM3U\n#EXT-X-MEDIA-SEQUENCE:201\n\
#EXTINF:2.0,live\nhttps://cdn/TOKb/201.ts?s=2\n\
#EXTINF:2.0,live\nhttps://cdn/TOKb/202.ts?s=2\n";
        // Sanity: raw IS inconsistent (sn 201 path TOKa/201 vs TOKb/201).
        let raw = std::panic::catch_unwind(|| assert_refresh_consistent(raw_n, raw_n1, "raw"));
        assert!(raw.is_err(), "raw polls should be inconsistent (proves the trap)");
        // Stabilized is consistent.
        let s_n = stabilize("inv", raw_n, "https://cdn/");
        let s_n1 = stabilize("inv", raw_n1, "https://cdn/");
        assert_refresh_consistent(&s_n, &s_n1, "stabilized");
        clear("inv");
    }

    // The core projection's cc job is to PRESERVE whatever discontinuity
    // accounting it is handed (it never touches discontinuity tags or
    // DISCONTINUITY-SEQUENCE; it only rewrites segment URLs). Keeping the upstream
    // cc-consistent across the discontinuity-rolloff transition is the filter's
    // responsibility (it drops ad discontinuities and must maintain
    // DISCONTINUITY-SEQUENCE the way it maintains MEDIA-SEQUENCE). This proves the
    // projection does not itself introduce a cc mismatch on already-consistent,
    // re-signed input.
    #[test]
    fn stabilize_preserves_discontinuity_accounting() {
        clear("cc");
        // A discontinuity in-window, segment URLs re-signed each poll (path token
        // rotates). Across the slide the disc rolls off and DISCONTINUITY-SEQUENCE
        // advances correctly, so sn 11's cc is 3 in both polls.
        let f_n = "#EXTM3U\n#EXT-X-MEDIA-SEQUENCE:10\n#EXT-X-DISCONTINUITY-SEQUENCE:2\n\
#EXTINF:2.0,live\nhttps://cdn/TOKa/10.ts?s=1\n\
#EXT-X-DISCONTINUITY\n\
#EXTINF:2.0,live\nhttps://cdn/TOKa/11.ts?s=1\n\
#EXTINF:2.0,live\nhttps://cdn/TOKa/12.ts?s=1\n";
        let f_n1 = "#EXTM3U\n#EXT-X-MEDIA-SEQUENCE:11\n#EXT-X-DISCONTINUITY-SEQUENCE:3\n\
#EXTINF:2.0,live\nhttps://cdn/TOKb/11.ts?s=2\n\
#EXTINF:2.0,live\nhttps://cdn/TOKb/12.ts?s=2\n\
#EXTINF:2.0,live\nhttps://cdn/TOKb/13.ts?s=2\n";
        let s_n = stabilize("cc", f_n, "https://cdn/");
        let s_n1 = stabilize("cc", f_n1, "https://cdn/");
        // Both checks hold after stabilization: url-path AND cc agree for sn 11/12.
        assert_refresh_consistent(&s_n, &s_n1, "cc-preserve");
        clear("cc");
    }
    // ── Rendition switches (link_adapt) ──

    fn window(first: u64, n: u64, tag: &str) -> String {
        let mut pl = format!("#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:{first}\n");
        for sn in first..first + n {
            pl.push_str(&format!("#EXTINF:2.000,live\nhttps://cdn/{tag}/{sn}.ts?s={first}\n"));
        }
        pl
    }

    #[test]
    fn a_switch_starts_right_after_what_the_player_fetched() {
        clear("sw");
        let a = stabilize("sw", &window(100, 6, "hi"), "https://cdn/");
        // The player has fetched up to 102; 103..105 are listed but not fetched.
        assert!(redirect_target("sw", 101).is_some());
        assert!(redirect_target("sw", 102).is_some());
        begin_switch("sw");
        let b = stabilize("sw", &window(101, 6, "lo"), "https://cdn/");
        // The player sees nothing change: same paths, no discontinuity.
        assert_refresh_consistent(&a, &b, "switch");
        assert!(!b.contains("DISCONTINUITY"));
        assert_eq!(redirect_target("sw", 102).as_deref(), Some("https://cdn/hi/102.ts?s=100"));
        assert_eq!(redirect_target("sw", 103).as_deref(), Some("https://cdn/lo/103.ts?s=101"));
        assert_eq!(redirect_target("sw", 106).as_deref(), Some("https://cdn/lo/106.ts?s=101"));
        // Later polls keep the fetched ones as they were.
        let c = stabilize("sw", &window(102, 6, "lo"), "https://cdn/");
        assert_refresh_consistent(&b, &c, "slide");
        assert_eq!(redirect_target("sw", 102).as_deref(), Some("https://cdn/hi/102.ts?s=100"));
        clear("sw");
    }

    #[test]
    fn a_switch_before_any_fetch_moves_everything() {
        clear("sw0");
        stabilize("sw0", &window(10, 3, "hi"), "https://cdn/");
        begin_switch("sw0");
        stabilize("sw0", &window(10, 3, "lo"), "https://cdn/");
        assert_eq!(redirect_target("sw0", 10).as_deref(), Some("https://cdn/lo/10.ts?s=10"));
        clear("sw0");
    }

    #[test]
    fn switching_back_keeps_each_fetched_segment_on_its_own_rendition() {
        clear("sw2");
        stabilize("sw2", &window(1, 4, "a"), "https://cdn/");
        redirect_target("sw2", 2);
        begin_switch("sw2");
        stabilize("sw2", &window(1, 4, "b"), "https://cdn/");
        redirect_target("sw2", 3);
        begin_switch("sw2");
        stabilize("sw2", &window(2, 4, "c"), "https://cdn/");
        assert_eq!(redirect_target("sw2", 2).as_deref(), Some("https://cdn/a/2.ts?s=1"));
        assert_eq!(redirect_target("sw2", 3).as_deref(), Some("https://cdn/b/3.ts?s=1"));
        assert_eq!(redirect_target("sw2", 4).as_deref(), Some("https://cdn/c/4.ts?s=2"));
        clear("sw2");
    }

    // ── Continuity: a switch or a resync never drops or repeats a segment ──

    /// A live window whose segments carry programme dates two seconds apart:
    /// sequence number `sn` starts at second `2 * sn` of the hour.
    fn dated_window(first: u64, n: u64, tag: &str) -> String {
        let mut pl = format!("#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:{first}\n");
        for sn in first..first + n {
            let at = sn * 2;
            pl.push_str(&format!(
                "#EXT-X-PROGRAM-DATE-TIME:2026-10-20T00:{:02}:{:02}.000Z\n#EXTINF:2.000,live\nhttps://cdn/{tag}/{sn}.ts\n",
                (at / 60) % 60,
                at % 60
            ));
        }
        pl
    }

    /// The (sn, programme second, synthetic path) of every segment a served
    /// playlist lists.
    fn listed(playlist: &str) -> Vec<(u64, u64, String)> {
        let mut out = Vec::new();
        let mut sn: u64 = 0;
        let mut at: Option<u64> = None;
        let mut expect = false;
        for line in playlist.lines().map(str::trim) {
            if let Some(v) = line.strip_prefix("#EXT-X-MEDIA-SEQUENCE:") {
                sn = v.parse().unwrap();
            } else if let Some(v) = line.strip_prefix("#EXT-X-PROGRAM-DATE-TIME:") {
                // ...T00:MM:SS.000Z
                let time = v.split('T').nth(1).unwrap().trim_end_matches('Z');
                let mut fields = time.split(':');
                let _hour = fields.next();
                let minutes: u64 = fields.next().unwrap().parse().unwrap();
                let seconds: f64 = fields.next().unwrap().parse().unwrap();
                at = Some(minutes * 60 + seconds as u64);
            } else if line.starts_with("#EXTINF:") {
                expect = true;
            } else if expect && !line.is_empty() && !line.starts_with('#') {
                expect = false;
                out.push((sn, at.take().unwrap(), line.to_string()));
                sn += 1;
            }
        }
        out
    }

    /// hls.js's order of play over a run of served playlists: it starts at the
    /// first listed segment, fetches up to `per_poll` new ones after each poll,
    /// and resyncs to the newest listed one when it fell out of the window.
    /// Every fetch goes through the redirect, as the player's does.
    struct Player {
        next: Option<u64>,
        /// What played: (sn, programme second, real URL).
        played: Vec<(u64, u64, String)>,
        resyncs: usize,
    }

    impl Player {
        fn new() -> Self {
            Self { next: None, played: Vec::new(), resyncs: 0 }
        }

        fn poll(&mut self, sid: &str, served: &str, per_poll: usize) {
            let segs = listed(served);
            let first = segs.first().expect("a live window").0;
            let next = match self.next {
                None => first,
                Some(n) if n < first => {
                    self.resyncs += 1;
                    segs.last().unwrap().0
                }
                Some(n) => n,
            };
            self.next = Some(next);
            for (sn, at, path) in segs.iter().filter(|s| s.0 >= next).take(per_poll) {
                let (psid, psn) = parse_vseg_path(path).unwrap();
                assert_eq!((psid.as_str(), psn), (sid, *sn), "the path names its own segment");
                let url = redirect_target(sid, *sn).expect("every listed segment resolves");
                self.played.push((*sn, *at, url));
                self.next = Some(sn + 1);
            }
        }
    }

    /// No segment repeated or dropped, by sequence number and programme date,
    /// except across a counted resync, where the date jumps with the number.
    fn assert_continuous(played: &[(u64, u64, String)], resyncs: usize) {
        let mut jumps = 0;
        for pair in played.windows(2) {
            let (a, b) = (&pair[0], &pair[1]);
            assert!(b.0 > a.0, "segment {} played again after {}", b.0, a.0);
            assert_eq!(b.1 - a.1, (b.0 - a.0) * 2, "programme date and sequence disagree at {}", b.0);
            if b.0 != a.0 + 1 {
                jumps += 1;
            }
        }
        assert_eq!(jumps, resyncs, "a segment was dropped outside a resync");
    }

    #[test]
    fn a_switch_drops_and_repeats_nothing() {
        let sid = "cont-switch";
        clear(sid);
        let mut player = Player::new();
        let mut first = 100;
        for _ in 0..2 {
            let served = stabilize(sid, &dated_window(first, 5, "hi"), "https://cdn/");
            player.poll(sid, &served, 1);
            first += 1;
        }
        begin_switch(sid);
        for _ in 0..12 {
            let served = stabilize(sid, &dated_window(first, 5, "lo"), "https://cdn/");
            player.poll(sid, &served, 1);
            first += 1;
        }
        assert_continuous(&player.played, player.resyncs);
        assert_eq!(player.resyncs, 0);
        assert_eq!(player.played.len(), 14);
        // The old rendition up to the switch, the new one after it, never back.
        let tags: Vec<&str> = player
            .played
            .iter()
            .map(|p| if p.2.contains("/hi/") { "hi" } else { "lo" })
            .collect();
        let flip = tags.iter().position(|t| *t == "lo").expect("it switched");
        assert_eq!(flip, 2, "the first segment after the last one fetched");
        assert!(tags[flip..].iter().all(|t| *t == "lo"));
        clear(sid);
    }

    #[test]
    fn a_resync_then_a_switch_drop_and_repeat_nothing_more() {
        let sid = "cont-resync";
        clear(sid);
        let mut player = Player::new();
        let served = stabilize(sid, &dated_window(200, 4, "hi"), "https://cdn/");
        player.poll(sid, &served, 1);
        // The player stalled while the window moved past it, and resyncs.
        let served = stabilize(sid, &dated_window(220, 4, "hi"), "https://cdn/");
        player.poll(sid, &served, 1);
        assert_eq!(player.resyncs, 1);
        begin_switch(sid);
        let mut first = 221;
        for _ in 0..8 {
            let served = stabilize(sid, &dated_window(first, 4, "lo"), "https://cdn/");
            player.poll(sid, &served, 1);
            first += 1;
        }
        assert_continuous(&player.played, player.resyncs);
        // The resync point keeps the rendition it was fetched from; the switch
        // starts right after it.
        assert!(player.played.iter().any(|p| p.0 == 223 && p.2.contains("/hi/")));
        assert!(player.played.iter().filter(|p| p.0 > 223).all(|p| p.2.contains("/lo/")));
        assert_eq!(player.played.iter().filter(|p| p.0 > 223).count(), 8);
        clear(sid);
    }

    #[test]
    fn a_new_broadcast_forgets_the_switch_point() {
        let sid = "cont-restart";
        clear(sid);
        stabilize(sid, &dated_window(500, 4, "hi"), "https://cdn/");
        redirect_target(sid, 502);
        begin_switch(sid);
        // The broadcast restarted: its sequence begins again, and an old
        // recorded segment must never shadow a new one.
        stabilize(sid, &dated_window(3, 4, "new"), "https://cdn/");
        for sn in 3..7 {
            assert!(redirect_target(sid, sn).unwrap().contains("/new/"));
        }
        clear(sid);
    }
}
