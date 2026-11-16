//! Ad-segment stripper for the live media playlist.
//!
//! The native port of the Streamlink TTV-LOL plugin's `should_filter_segment`,
//! carried forward from StreamNook 7.8.6 (`services/ad_detect.rs` at `2bf9720`).
//! A Twitch SSAI ad pod is an `#EXT-X-DATERANGE CLASS="twitch-stitched-ad"`
//! (id `stitched-ad-…`) carrying `X-TV-TWITCH-AD-*` metadata, or an `#EXTINF`
//! segment whose title contains "Amazon". Both forms are removed here before
//! the player ever sees them, which is what makes a leaked ad disappear with no
//! reload and no stall.
//!
//! `services::ad_detect` stays detect-only (whole-document substring matching
//! for the UI counter); a stripper needs real line structure, so it lives here.

use chrono::{DateTime, FixedOffset};

/// What one filter pass did. `dropped == 0` means the caller should serve the
/// original bytes untouched.
#[derive(Debug, Clone, Copy, Default)]
pub struct FilterOutcome {
    /// Segments left out of this playlist: the ads, plus any content that sat
    /// before them once content resumed (see `filter_ad_segments`).
    pub dropped: u32,
    /// Real (non-ad) segments still in the served playlist.
    pub real: u32,
}

impl FilterOutcome {
    /// True when the whole window was ads, so stripping left nothing playable.
    /// Filtering cannot cope with this; it is the pivot's cue to re-resolve
    /// through a different region.
    pub fn all_ads(&self) -> bool {
        self.dropped > 0 && self.real == 0
    }
}

/// True for an `#EXT-X-DATERANGE` line that marks a Twitch ad pod.
fn is_ad_daterange(line: &str) -> bool {
    line.contains("twitch-stitched-ad") || line.contains("stitched-ad-")
}

/// True when a match at `pos` starts a whole attribute rather than the tail of a
/// longer one. Without this, looking up `DURATION` finds `PLANNED-DURATION`,
/// which is a real attribute on Twitch's ad dateranges, and the ad window ends
/// up computed from the wrong number.
fn at_attr_boundary(line: &str, pos: usize) -> bool {
    match line[..pos].chars().next_back() {
        None => true,
        Some(c) => c == ',' || c == ':' || c == ' ',
    }
}

/// Pull an attribute value out of an HLS tag line (quoted or unquoted form).
fn tag_attr(line: &str, key: &str) -> Option<String> {
    let quoted = format!("{}=\"", key);
    if let Some(pos) = line.match_indices(&quoted).find_map(|(p, _)| {
        at_attr_boundary(line, p).then_some(p)
    }) {
        let rest = &line[pos + quoted.len()..];
        return rest.find('"').map(|end| rest[..end].to_string());
    }
    let bare = format!("{}=", key);
    let pos = line
        .match_indices(&bare)
        .find_map(|(p, _)| at_attr_boundary(line, p).then_some(p))?;
    let rest = &line[pos + bare.len()..];
    let end = rest.find(',').unwrap_or(rest.len());
    Some(rest[..end].to_string())
}

/// Strip Twitch SSAI ad segments from a live media playlist. A segment is an ad
/// if its `#EXTINF` title contains "Amazon" or its `#EXT-X-PROGRAM-DATE-TIME`
/// falls inside an ad daterange. Those segments (and their buffered
/// discontinuity/PDT/EXTINF prefix tags) plus the ad daterange tags come out.
///
/// Only the NEWEST unbroken run of real segments is served. A media sequence
/// number is a segment's identity (hls.js and `hls_projection` key by it), so
/// an ad dropped from the middle of the window would slide every later segment
/// down onto the ad's numbers, and a few polls later, with the ads leading the
/// window, the same segment came back under its true number: one number naming
/// two segments on every midroll, played as a jump, replay or stall (SN-276).
/// So while a break runs, the content before it is served and the ads after it
/// are cut; once content resumes, the window moves straight to it.
///
/// Both sequence headers are then re-based on one invariant: **every served
/// segment keeps its true upstream media sequence and its true upstream
/// discontinuity sequence** (the served run is unbroken, so the first one
/// decides the rest). `#EXT-X-MEDIA-SEQUENCE` is bumped by the
/// segments dropped ahead of it and `#EXT-X-DISCONTINUITY-SEQUENCE` by the
/// `#EXT-X-DISCONTINUITY` tags dropped ahead of it, so the two accountings move
/// in lockstep. `hls_projection::stabilize` (which runs after this and only
/// rewrites segment URLs) relies on being handed playlists that are already
/// self-consistent this way.
///
/// Returns the filtered playlist and what it did. Callers should only swap in
/// the filtered output when `dropped > 0`, so an ad-free playlist passes
/// through byte-for-byte untouched.
pub fn filter_ad_segments(playlist: &str) -> (String, FilterOutcome) {
    // Pass 1: collect ad time windows from ad dateranges (best-effort; a parse
    // failure just means we fall back to the "Amazon" title signal).
    let mut windows: Vec<(DateTime<FixedOffset>, DateTime<FixedOffset>)> = Vec::new();
    for line in playlist.lines() {
        let t = line.trim();
        if t.starts_with("#EXT-X-DATERANGE") && is_ad_daterange(t) {
            let start =
                tag_attr(t, "START-DATE").and_then(|s| DateTime::parse_from_rfc3339(&s).ok());
            if let Some(start) = start {
                let end = tag_attr(t, "END-DATE")
                    .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                    .or_else(|| {
                        tag_attr(t, "DURATION")
                            .and_then(|d| d.parse::<f64>().ok())
                            .map(|secs| {
                                start + chrono::Duration::milliseconds((secs * 1000.0) as i64)
                            })
                    });
                if let Some(end) = end {
                    windows.push((start, end));
                }
            }
        }
    }
    let in_ad_window =
        |pdt: &DateTime<FixedOffset>| windows.iter().any(|(s, e)| pdt >= s && pdt < e);

    // Pass 2: which segments are ads, in order.
    let mut is_ad: Vec<bool> = Vec::new();
    {
        let mut cur_pdt: Option<DateTime<FixedOffset>> = None;
        let mut cur_title: Option<String> = None;
        for raw in playlist.lines() {
            let t = raw.trim();
            if let Some(v) = t.strip_prefix("#EXT-X-PROGRAM-DATE-TIME:") {
                cur_pdt = DateTime::parse_from_rfc3339(v.trim()).ok();
            } else if t.starts_with("#EXTINF:") {
                cur_title = t.split_once(',').map(|(_, title)| title.to_string());
            } else if !t.is_empty() && !t.starts_with('#') {
                is_ad.push(
                    cur_title.as_deref().is_some_and(|title| title.contains("Amazon"))
                        || cur_pdt.as_ref().is_some_and(in_ad_window),
                );
                cur_pdt = None;
                cur_title = None;
            }
        }
    }
    // The newest unbroken run of real segments: [keep_from, keep_to].
    let (keep_from, keep_to) = match is_ad.iter().rposition(|ad| !*ad) {
        Some(last) => (is_ad[..last].iter().rposition(|ad| *ad).map_or(0, |i| i + 1), last),
        None => (is_ad.len(), 0), // nothing real: serve no segment
    };

    // Pass 3: walk lines, serving only that run.
    let mut out: Vec<String> = Vec::new();
    let mut pending: Vec<String> = Vec::new(); // buffered prefix tags for the next segment
    let mut seg_idx = 0usize;
    let mut pending_discontinuities = 0u32;
    let mut dropped = 0u32;
    let mut real = 0u32;
    let mut leading_dropped = 0u32;
    let mut leading_disc_dropped = 0u32;
    let mut mediaseq_idx: Option<usize> = None;
    let mut mediaseq_val: Option<u64> = None;
    let mut discseq_idx: Option<usize> = None;
    let mut discseq_val: Option<u64> = None;

    for raw in playlist.lines() {
        let t = raw.trim();
        if t.is_empty() {
            continue;
        }
        if t.starts_with("#EXT-X-DATERANGE") {
            if is_ad_daterange(t) {
                continue; // drop the ad daterange tag
            }
            out.push(raw.to_string());
            continue;
        }
        if let Some(v) = t.strip_prefix("#EXT-X-MEDIA-SEQUENCE:") {
            mediaseq_val = v.trim().parse::<u64>().ok();
            mediaseq_idx = Some(out.len());
            out.push(raw.to_string()); // patched after the walk if leading ads were dropped
            continue;
        }
        if let Some(v) = t.strip_prefix("#EXT-X-DISCONTINUITY-SEQUENCE:") {
            discseq_val = v.trim().parse::<u64>().ok();
            discseq_idx = Some(out.len());
            out.push(raw.to_string()); // patched the same way, for the cc accounting
            continue;
        }
        if t.starts_with("#EXT-X-PROGRAM-DATE-TIME:") || t.starts_with("#EXTINF:") {
            pending.push(raw.to_string());
            continue;
        }
        if t == "#EXT-X-DISCONTINUITY" {
            pending_discontinuities += 1;
            pending.push(raw.to_string());
            continue;
        }
        if t.starts_with("#EXT-X-BYTERANGE") {
            pending.push(raw.to_string());
            continue;
        }
        // `#EXT-X-MAP` and `#EXT-X-KEY` are NOT per-segment: each applies to every
        // segment that follows until the next one of its kind. Buffering them with
        // a segment means dropping an ad takes the initialization segment with it,
        // and then nothing after it can be decoded at all. That turns a cosmetic ad
        // leak into a dead stream, and it is reachable: the tiers grafted from the
        // viewer's own master are the CMAF ones, which are exactly the tiers that
        // carry `#EXT-X-MAP`.
        if t.starts_with("#EXT-X-MAP") || t.starts_with("#EXT-X-KEY") {
            out.push(raw.to_string());
            continue;
        }
        if t.starts_with('#') {
            // Any other tag is playlist-level (header, ENDLIST, ...).
            out.push(raw.to_string());
            continue;
        }

        // Non-comment line = the segment URI; this closes a segment.
        if seg_idx >= keep_from && seg_idx <= keep_to {
            out.append(&mut pending);
            out.push(raw.to_string());
            real += 1;
        } else {
            dropped += 1;
            // Ahead of the served run, its numbers and discontinuities move into
            // the headers. After it (a break still running), nothing served
            // follows, so nothing needs accounting for.
            if seg_idx < keep_from {
                leading_dropped += 1;
                leading_disc_dropped += pending_discontinuities;
            }
        }
        pending.clear();
        pending_discontinuities = 0;
        seg_idx += 1;
    }

    if leading_dropped > 0 {
        if let (Some(idx), Some(val)) = (mediaseq_idx, mediaseq_val) {
            // Saturating, not wrapping: the value is parsed from relay-supplied
            // text, and a wrapped sequence number would poison the projection map
            // (or panic outright in a debug build, inside the request handler).
            out[idx] = format!(
                "#EXT-X-MEDIA-SEQUENCE:{}",
                val.saturating_add(leading_dropped as u64)
            );
        }
    }
    // Twitch's live playlists carry no `#EXT-X-DISCONTINUITY-SEQUENCE` at all
    // (checked against the live edge on 2026-08-06), so only patching an existing
    // header would never once fire in production. When the tag that opens an ad
    // pod is dropped off the front of the window the header has to be INSERTED,
    // or every surviving segment reads one discontinuity lower than it should.
    if leading_disc_dropped > 0 {
        let bumped = format!(
            "#EXT-X-DISCONTINUITY-SEQUENCE:{}",
            discseq_val.unwrap_or(0).saturating_add(leading_disc_dropped as u64)
        );
        match discseq_idx {
            Some(idx) => out[idx] = bumped,
            // Insert right after the media sequence so the two headers stay
            // together. `mediaseq_idx` is still valid: the patch above only
            // replaced a line, it did not move any.
            None => {
                let at = mediaseq_idx.map(|i| i + 1).unwrap_or_else(|| out.len().min(1));
                out.insert(at, bumped);
            }
        }
    }

    let mut joined = out.join("\n");
    joined.push('\n');
    (joined, FilterOutcome { dropped, real })
}

#[cfg(test)]
mod tests {
    use super::*;

    const AD_DATERANGE: &str = "#EXT-X-DATERANGE:ID=\"stitched-ad-1\",CLASS=\"twitch-stitched-ad\",START-DATE=\"2026-01-01T00:00:00.000Z\",DURATION=4.0,X-TV-TWITCH-AD-ROLL-TYPE=\"MIDROLL\"";

    #[test]
    fn drops_amazon_titled_segments() {
        let pl = "#EXTM3U\n#EXT-X-MEDIA-SEQUENCE:10\n\
#EXTINF:2.0,Amazon|123\nhttps://cdn/ad0.ts\n\
#EXTINF:2.0,live\nhttps://cdn/real0.ts\n";
        let (out, o) = filter_ad_segments(pl);
        assert_eq!((o.dropped, o.real), (1, 1));
        assert!(!out.contains("ad0.ts"));
        assert!(out.contains("real0.ts"));
    }

    #[test]
    fn drops_segments_inside_an_ad_daterange() {
        let pl = format!(
            "#EXTM3U\n#EXT-X-MEDIA-SEQUENCE:10\n{}\n\
#EXT-X-PROGRAM-DATE-TIME:2026-01-01T00:00:01.000Z\n#EXTINF:2.0,live\nhttps://cdn/ad0.ts\n\
#EXT-X-PROGRAM-DATE-TIME:2026-01-01T00:00:05.000Z\n#EXTINF:2.0,live\nhttps://cdn/real0.ts\n",
            AD_DATERANGE
        );
        let (out, o) = filter_ad_segments(&pl);
        assert_eq!((o.dropped, o.real), (1, 1));
        assert!(!out.contains("ad0.ts"));
        assert!(!out.contains("twitch-stitched-ad"), "ad daterange must go too");
        assert!(out.contains("real0.ts"));
    }

    #[test]
    fn ad_free_playlist_drops_nothing() {
        let pl = "#EXTM3U\n#EXT-X-MEDIA-SEQUENCE:10\n\
#EXTINF:2.0,live\nhttps://cdn/a.ts\n#EXTINF:2.0,live\nhttps://cdn/b.ts\n";
        let (_, o) = filter_ad_segments(pl);
        assert_eq!((o.dropped, o.real), (0, 2));
    }

    #[test]
    fn leading_ads_bump_media_sequence() {
        let pl = "#EXTM3U\n#EXT-X-MEDIA-SEQUENCE:100\n\
#EXTINF:2.0,Amazon|1\nhttps://cdn/ad0.ts\n\
#EXTINF:2.0,Amazon|2\nhttps://cdn/ad1.ts\n\
#EXTINF:2.0,live\nhttps://cdn/real0.ts\n";
        let (out, o) = filter_ad_segments(pl);
        assert_eq!((o.dropped, o.real), (2, 1));
        assert!(
            out.contains("#EXT-X-MEDIA-SEQUENCE:102"),
            "first surviving segment keeps its true upstream sequence: {out}"
        );
    }

    #[test]
    fn trailing_ads_leave_media_sequence_alone() {
        let pl = "#EXTM3U\n#EXT-X-MEDIA-SEQUENCE:100\n\
#EXTINF:2.0,live\nhttps://cdn/real0.ts\n\
#EXTINF:2.0,Amazon|1\nhttps://cdn/ad0.ts\n";
        let (out, o) = filter_ad_segments(pl);
        assert_eq!((o.dropped, o.real), (1, 1));
        assert!(out.contains("#EXT-X-MEDIA-SEQUENCE:100"));
    }

    #[test]
    fn leading_ad_discontinuity_bumps_discontinuity_sequence() {
        // The pod's entering #EXT-X-DISCONTINUITY belongs to the dropped ad
        // segment, so it leaves with it; without the header bump every
        // surviving segment's cc would read one lower than upstream.
        let pl = "#EXTM3U\n#EXT-X-MEDIA-SEQUENCE:100\n#EXT-X-DISCONTINUITY-SEQUENCE:5\n\
#EXT-X-DISCONTINUITY\n#EXTINF:2.0,Amazon|1\nhttps://cdn/ad0.ts\n\
#EXT-X-DISCONTINUITY\n#EXTINF:2.0,live\nhttps://cdn/real0.ts\n";
        let (out, o) = filter_ad_segments(pl);
        assert_eq!((o.dropped, o.real), (1, 1));
        assert!(out.contains("#EXT-X-MEDIA-SEQUENCE:101"), "{out}");
        assert!(out.contains("#EXT-X-DISCONTINUITY-SEQUENCE:6"), "{out}");
        // The surviving segment keeps its own leaving-the-pod discontinuity.
        assert_eq!(out.matches("#EXT-X-DISCONTINUITY\n").count(), 1, "{out}");
    }

    #[test]
    fn a_dropped_leading_discontinuity_creates_the_header_when_upstream_has_none() {
        // This is the shape Twitch actually serves: no
        // #EXT-X-DISCONTINUITY-SEQUENCE anywhere. Patching alone would be a
        // no-op and the surviving segment would read one discontinuity low.
        let pl = "#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:100\n\
#EXT-X-DISCONTINUITY\n#EXTINF:2.0,Amazon|1\nhttps://cdn/ad0.ts\n\
#EXTINF:2.0,live\nhttps://cdn/real0.ts\n";
        let (out, o) = filter_ad_segments(pl);
        assert_eq!((o.dropped, o.real), (1, 1));
        assert!(out.contains("#EXT-X-DISCONTINUITY-SEQUENCE:1"), "{out}");
        // Directly after the media sequence, so the two headers stay together
        // and both still precede the first segment.
        let lines: Vec<&str> = out.lines().collect();
        let ms = lines
            .iter()
            .position(|l| l.starts_with("#EXT-X-MEDIA-SEQUENCE:"))
            .unwrap();
        assert_eq!(lines[ms + 1], "#EXT-X-DISCONTINUITY-SEQUENCE:1", "{out}");
    }

    #[test]
    fn an_ad_free_playlist_never_grows_a_discontinuity_header() {
        let pl = "#EXTM3U\n#EXT-X-MEDIA-SEQUENCE:100\n\
#EXT-X-DISCONTINUITY\n#EXTINF:2.0,live\nhttps://cdn/a.ts\n";
        let (out, o) = filter_ad_segments(pl);
        assert_eq!(o.dropped, 0);
        assert!(!out.contains("#EXT-X-DISCONTINUITY-SEQUENCE"), "{out}");
    }

    #[test]
    fn content_resuming_after_a_midroll_keeps_its_true_numbers() {
        // real0 (sn 100, cc 5), the pod (sn 101, cc 6), real1 (sn 102, cc 7).
        // Content resumed, so the window moves to real1 under its own numbers;
        // serving real0 too would have slid real1 onto the ad's sn 101 (SN-276).
        let pl = "#EXTM3U\n#EXT-X-MEDIA-SEQUENCE:100\n#EXT-X-DISCONTINUITY-SEQUENCE:5\n\
#EXTINF:2.0,live\nhttps://cdn/real0.ts\n\
#EXT-X-DISCONTINUITY\n#EXTINF:2.0,Amazon|1\nhttps://cdn/ad0.ts\n\
#EXT-X-DISCONTINUITY\n#EXTINF:2.0,live\nhttps://cdn/real1.ts\n";
        let (out, o) = filter_ad_segments(pl);
        assert_eq!((o.dropped, o.real), (2, 1));
        assert_eq!(frags(&out), vec![(102, "https://cdn/real1.ts".to_string(), 7)], "{out}");
    }

    #[test]
    fn a_running_break_serves_the_content_before_it() {
        let pl = "#EXTM3U\n#EXT-X-MEDIA-SEQUENCE:100\n#EXT-X-DISCONTINUITY-SEQUENCE:5\n\
#EXTINF:2.0,live\nhttps://cdn/real0.ts\n#EXTINF:2.0,live\nhttps://cdn/real1.ts\n\
#EXT-X-DISCONTINUITY\n#EXTINF:2.0,Amazon|1\nhttps://cdn/ad0.ts\n";
        let (out, o) = filter_ad_segments(pl);
        assert_eq!((o.dropped, o.real), (1, 2));
        assert!(!o.all_ads());
        assert_eq!(
            frags(&out),
            vec![(100, "https://cdn/real0.ts".to_string(), 5), (101, "https://cdn/real1.ts".to_string(), 5)],
            "{out}"
        );
    }

    /// (sn, url, cc) for each served segment, assigned the way hls.js does.
    fn frags(playlist: &str) -> Vec<(u64, String, u64)> {
        let (mut sn, mut cc) = (0u64, 0u64);
        let mut out = Vec::new();
        for line in playlist.lines().map(str::trim) {
            if let Some(v) = line.strip_prefix("#EXT-X-MEDIA-SEQUENCE:") {
                sn = v.parse().unwrap_or(0);
            } else if let Some(v) = line.strip_prefix("#EXT-X-DISCONTINUITY-SEQUENCE:") {
                cc = v.parse().unwrap_or(0);
            } else if line == "#EXT-X-DISCONTINUITY" {
                cc += 1;
            } else if !line.is_empty() && !line.starts_with('#') {
                out.push((sn, line.to_string(), cc));
                sn += 1;
            }
        }
        out
    }

    /// A whole midroll sliding through Twitch's window (no discontinuity header,
    /// as Twitch serves it), one poll per segment: every served segment keeps
    /// the sequence number and discontinuity count it has upstream, so no number
    /// ever names two segments across polls and no ad is ever served.
    #[test]
    fn across_a_whole_midroll_every_segment_keeps_its_numbers() {
        let ad = |n: u64| (110..=117).contains(&n);
        // Upstream truth: discontinuities open (110) and close (118) the pod.
        let true_cc = |n: u64| (n >= 110) as u64 + (n >= 118) as u64;
        let mut seen: std::collections::HashMap<u64, String> = std::collections::HashMap::new();
        for head in 100u64..=124 {
            let mut pl = format!("#EXTM3U\n#EXT-X-MEDIA-SEQUENCE:{head}\n");
            // A tag rolled off the front is gone from the window, as on Twitch;
            // the window's own count starts again at 0 without a header.
            let base = true_cc(head);
            for n in head..head + 6 {
                if (n == 110 || n == 118) && n != head {
                    pl.push_str("#EXT-X-DISCONTINUITY\n");
                }
                let title = if ad(n) { "Amazon|1" } else { "live" };
                pl.push_str(&format!("#EXTINF:2.0,{title}\nhttps://cdn/s{n}.ts\n"));
            }
            let (out, _) = filter_ad_segments(&pl);
            for (sn, url, cc) in frags(&out) {
                let n: u64 = url.trim_start_matches("https://cdn/s").trim_end_matches(".ts").parse().unwrap();
                assert!(!ad(n), "ad served at head {head}: {out}");
                assert_eq!(sn, n, "head {head}: {url} served as {sn}\n{out}");
                assert_eq!(cc + base, true_cc(n), "head {head}: {url} cc {cc}\n{out}");
                if let Some(prev) = seen.insert(sn, url.clone()) {
                    assert_eq!(prev, url, "sequence {sn} named two segments");
                }
            }
        }
    }

    #[test]
    fn all_ad_window_reports_itself() {
        let pl = "#EXTM3U\n#EXT-X-MEDIA-SEQUENCE:100\n\
#EXTINF:2.0,Amazon|1\nhttps://cdn/ad0.ts\n\
#EXTINF:2.0,Amazon|2\nhttps://cdn/ad1.ts\n";
        let (_, o) = filter_ad_segments(pl);
        assert!(o.all_ads());
    }

    #[test]
    fn the_init_segment_survives_dropping_the_ad_next_to_it() {
        // #EXT-X-MAP applies to every segment after it, not just the next one.
        // Losing it with a dropped ad leaves the rest of the window undecodable,
        // which is worse than the ad. Reachable on the CMAF tiers grafted from
        // the viewer's own master.
        let pl = "#EXTM3U\n#EXT-X-MEDIA-SEQUENCE:10\n\
#EXT-X-MAP:URI=\"https://cdn/init.mp4\"\n\
#EXTINF:2.0,Amazon|1\nhttps://cdn/ad0.m4s\n\
#EXTINF:2.0,live\nhttps://cdn/real0.m4s\n";
        let (out, o) = filter_ad_segments(pl);
        assert_eq!((o.dropped, o.real), (1, 1));
        assert!(out.contains("#EXT-X-MAP:URI=\"https://cdn/init.mp4\""), "{out}");
        assert!(!out.contains("ad0.m4s"), "{out}");
        assert!(out.contains("real0.m4s"), "{out}");
    }

    #[test]
    fn the_decryption_key_survives_dropping_the_ad_next_to_it() {
        let pl = "#EXTM3U\n#EXT-X-MEDIA-SEQUENCE:10\n\
#EXT-X-KEY:METHOD=AES-128,URI=\"https://cdn/key.bin\"\n\
#EXTINF:2.0,Amazon|1\nhttps://cdn/ad0.ts\n\
#EXTINF:2.0,live\nhttps://cdn/real0.ts\n";
        let (out, _) = filter_ad_segments(pl);
        assert!(out.contains("#EXT-X-KEY:METHOD=AES-128"), "{out}");
    }

    #[test]
    fn planned_duration_does_not_masquerade_as_duration() {
        // PLANNED-DURATION is a real attribute on Twitch's ad dateranges, and a
        // substring search for "DURATION=" finds it first, which would compute
        // the ad window from the wrong number.
        let line = "#EXT-X-DATERANGE:ID=\"x\",PLANNED-DURATION=30.0,DURATION=4.0";
        assert_eq!(tag_attr(line, "DURATION").as_deref(), Some("4.0"));
        assert_eq!(tag_attr(line, "PLANNED-DURATION").as_deref(), Some("30.0"));
    }

    #[test]
    fn playlist_level_tags_survive() {
        let pl = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:6\n#EXT-X-MEDIA-SEQUENCE:1\n\
#EXTINF:2.0,live\nhttps://cdn/a.ts\n#EXT-X-ENDLIST\n";
        let (out, _) = filter_ad_segments(pl);
        for tag in [
            "#EXTM3U",
            "#EXT-X-VERSION:3",
            "#EXT-X-TARGETDURATION:6",
            "#EXT-X-ENDLIST",
        ] {
            assert!(out.contains(tag), "{tag} missing from {out}");
        }
    }
}
