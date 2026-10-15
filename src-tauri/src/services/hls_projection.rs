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
    /// The newest sequence number recorded so far.
    newest: Option<u64>,
    /// Rendition switches, as the first sequence number served from the new
    /// rendition (see `begin_switch`). Ascending.
    switches: Vec<u64>,
}

static PROJECTIONS: Lazy<Mutex<HashMap<String, SessionProj>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Forget a session's projection state (stream start/stop, or a pivot that
/// recreates the player). The next `stabilize` for this sid starts an empty
/// window. Safe to call when no state exists.
pub fn reset(sid: &str) {
    PROJECTIONS.lock().unwrap().remove(sid);
}

/// The relay is about to poll a different rendition of the same broadcast.
/// Twitch's renditions share sequence numbers and programme dates, so the
/// window splices: every sequence number already recorded keeps the old
/// rendition's segment (the player may hold or be fetching it), the first new
/// one is served from the new rendition behind an `#EXT-X-DISCONTINUITY` (a new
/// resolution and encoder), and `#EXT-X-DISCONTINUITY-SEQUENCE` counts the
/// switch once that segment rolls out of the window, so every segment keeps
/// one discontinuity count across refreshes. Returns false when nothing has
/// been served yet (there is nothing to splice; the next poll simply starts on
/// the new rendition).
pub fn begin_switch(sid: &str) -> bool {
    let mut projections = PROJECTIONS.lock().unwrap();
    let Some(proj) = projections.get_mut(sid) else {
        return false;
    };
    let Some(newest) = proj.newest else {
        return false;
    };
    let floor = newest + 1;
    if proj.switches.last().is_none_or(|&f| f < floor) {
        proj.switches.push(floor);
    }
    true
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

    // The window's first sequence number, read up front: it decides which
    // switches have rolled out (counted in the discontinuity sequence) and
    // which still show as a tag in the window.
    let first_sn: Option<u64> = playlist
        .lines()
        .find_map(|l| l.trim().strip_prefix("#EXT-X-MEDIA-SEQUENCE:"))
        .and_then(|v| v.trim().parse().ok());
    if let Some(first) = first_sn {
        // A new broadcast restarts the media-sequence: if the window start
        // moved BACKWARDS, the recorded URLs belong to a previous stream, so a
        // synthetic path could resolve to dead content. Clear and reseed.
        if proj.last_media_seq.is_some_and(|prev| first < prev) {
            proj.map.clear();
            proj.switches.clear();
            proj.newest = None;
        }
    }
    let rolled = first_sn.map_or(0, |f| proj.switches.iter().filter(|&&s| s <= f).count() as u64);
    let has_dseq = playlist.contains("#EXT-X-DISCONTINUITY-SEQUENCE:");

    let mut sn: u64 = 0;
    let mut max_sn: Option<u64> = None;
    let mut expect_uri = false;
    let mut out = String::with_capacity(playlist.len() + 64);
    // Lines since the previous segment URI: a segment's own tags, emitted with
    // it so a splice's discontinuity can go in front of them.
    let mut pending = String::new();

    for line in playlist.lines() {
        let trimmed = line.trim();
        if let Some(v) = trimmed.strip_prefix("#EXT-X-MEDIA-SEQUENCE:") {
            sn = v.trim().parse().unwrap_or(0);
            proj.last_media_seq = Some(sn);
            pending.push_str(line);
            pending.push('\n');
            if rolled > 0 && !has_dseq {
                pending.push_str(&format!("#EXT-X-DISCONTINUITY-SEQUENCE:{rolled}\n"));
            }
        } else if let Some(v) = trimmed.strip_prefix("#EXT-X-DISCONTINUITY-SEQUENCE:") {
            if rolled > 0 {
                let base: u64 = v.trim().parse().unwrap_or(0);
                pending.push_str(&format!("#EXT-X-DISCONTINUITY-SEQUENCE:{}\n", base + rolled));
            } else {
                pending.push_str(line);
                pending.push('\n');
            }
        } else if trimmed.starts_with("#EXTINF:") {
            expect_uri = true;
            pending.push_str(line);
            pending.push('\n');
        } else if expect_uri && !trimmed.is_empty() && !trimmed.starts_with('#') {
            // The segment URI for the preceding #EXTINF: record the real URL,
            // emit the stable synthetic path in its place.
            expect_uri = false;
            let floor = proj.switches.last().copied();
            let spliced_in = first_sn.is_some_and(|f| sn > f) && proj.switches.contains(&sn);
            if spliced_in {
                out.push_str("#EXT-X-DISCONTINUITY\n");
            }
            out.push_str(&pending);
            pending.clear();
            // Below the latest switch a recorded URL is the old rendition's:
            // keep it. Everything else takes the freshest signed URL.
            let keep_old = floor.is_some_and(|f| sn < f) && proj.map.contains_key(&sn);
            if !keep_old {
                proj.map.insert(sn, resolve_segment_url(trimmed, base_url));
            }
            max_sn = Some(sn);
            out.push_str(&format!("{VSEG_PREFIX}{sid}/{sn}.ts\n"));
            sn += 1;
        } else {
            pending.push_str(line);
            pending.push('\n');
        }
    }
    out.push_str(&pending);

    // Prune sns well behind the edge so a session's map can't grow unbounded.
    if let Some(edge) = max_sn {
        proj.newest = Some(proj.newest.map_or(edge, |n| n.max(edge)));
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
    PROJECTIONS
        .lock()
        .unwrap()
        .get(sid)
        .and_then(|p| p.map.get(&sn).cloned())
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

    // ── Rendition switches (link_adapt): a seamless splice ──

    fn window(first: u64, n: u64, tag: &str) -> String {
        let mut pl = format!("#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:{first}\n");
        for sn in first..first + n {
            pl.push_str(&format!(
                "#EXT-X-PROGRAM-DATE-TIME:2026-10-14T00:00:{:02}.000Z\n#EXTINF:2.000,live\nhttps://cdn/{tag}/{sn}.ts?s={first}\n",
                sn % 60
            ));
        }
        pl
    }

    #[test]
    fn a_rendition_switch_splices_behind_a_discontinuity() {
        clear("sw");
        let a = stabilize("sw", &window(100, 4, "hi"), "https://cdn/");
        assert!(begin_switch("sw"));
        // The next poll is the new rendition: same sequence numbers, one newer.
        let b = stabilize("sw", &window(101, 4, "lo"), "https://cdn/");
        assert_refresh_consistent(&a, &b, "switch");
        // Recorded segments keep the old rendition; the new one starts at 104.
        assert_eq!(redirect_target("sw", 103).as_deref(), Some("https://cdn/hi/103.ts?s=100"));
        assert_eq!(redirect_target("sw", 104).as_deref(), Some("https://cdn/lo/104.ts?s=101"));
        // The discontinuity sits right before 104's own tags.
        assert!(b.contains("vseg/sw/103.ts\n#EXT-X-DISCONTINUITY\n#EXT-X-PROGRAM-DATE-TIME"));
        assert_eq!(b.matches("#EXT-X-DISCONTINUITY\n").count(), 1);
        // Polls slide over the switch, then past it: every overlap agrees.
        let mut prev = b;
        for first in 102..=110 {
            let next = stabilize("sw", &window(first, 4, "lo"), "https://cdn/");
            assert_refresh_consistent(&prev, &next, &format!("slide {first}"));
            prev = next;
        }
        assert!(prev.contains("#EXT-X-DISCONTINUITY-SEQUENCE:1\n"));
        assert!(!prev.contains("#EXT-X-DISCONTINUITY\n"));
        clear("sw");
    }

    #[test]
    fn a_switch_adds_to_the_upstream_discontinuity_sequence() {
        clear("swd");
        let with_dseq = |first: u64, tag: &str| {
            window(first, 3, tag).replace(
                &format!("#EXT-X-MEDIA-SEQUENCE:{first}\n"),
                &format!("#EXT-X-MEDIA-SEQUENCE:{first}\n#EXT-X-DISCONTINUITY-SEQUENCE:5\n"),
            )
        };
        stabilize("swd", &with_dseq(10, "hi"), "https://cdn/");
        assert!(begin_switch("swd"));
        let mut prev = stabilize("swd", &with_dseq(11, "lo"), "https://cdn/");
        for first in 12..=16 {
            let next = stabilize("swd", &with_dseq(first, "lo"), "https://cdn/");
            assert_refresh_consistent(&prev, &next, &format!("dseq {first}"));
            prev = next;
        }
        assert!(prev.contains("#EXT-X-DISCONTINUITY-SEQUENCE:6\n"));
        assert_eq!(prev.matches("DISCONTINUITY-SEQUENCE").count(), 1);
        clear("swd");
    }

    #[test]
    fn two_switches_count_twice_and_a_switch_before_any_poll_is_a_no_op() {
        clear("sw2");
        assert!(!begin_switch("sw2"));
        stabilize("sw2", &window(1, 3, "a"), "https://cdn/");
        assert!(begin_switch("sw2"));
        let mut prev = stabilize("sw2", &window(2, 3, "b"), "https://cdn/");
        assert!(begin_switch("sw2"));
        for first in 3..=9 {
            let tag = if first == 3 { "b" } else { "c" };
            let next = stabilize("sw2", &window(first, 3, tag), "https://cdn/");
            assert_refresh_consistent(&prev, &next, &format!("two {first}"));
            prev = next;
        }
        assert!(prev.contains("#EXT-X-DISCONTINUITY-SEQUENCE:2\n"));
        clear("sw2");
    }

    #[test]
    fn without_a_switch_the_output_is_unchanged() {
        clear("same");
        let pl = window(40, 3, "x") + "#EXT-X-TWITCH-PREFETCH:https://cdn/x/43.ts\n";
        let out = stabilize("same", &pl, "https://cdn/");
        let expected = pl
            .lines()
            .map(|l| match l.strip_prefix("https://cdn/x/") {
                Some(rest) => format!("vseg/same/{}.ts", rest.split('.').next().unwrap()),
                None => l.to_string(),
            })
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        assert_eq!(out, expected);
        clear("same");
    }
}
