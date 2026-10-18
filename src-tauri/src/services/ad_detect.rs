//! Shared Twitch SSAI ad DETECTION, the single source of truth for the marker
//! strings, so the solo player (`stream_server`) and MultiNook
//! (`multi_nook_server`) recognize ad windows identically.
//!
//! Detection is read-only by design: the core relay is ad-neutral and never
//! edits ads out of the playlists it serves. The state tracked here drives the
//! UI's ad indicator and the `on_ad_window` plugin event, which is how an
//! installed resolution-owning plugin learns it should swap the relay's
//! upstream (docs/plugins/PROTOCOL.md).
//!
//! Markers were confirmed live: an ad pod is a
//! `#EXT-X-DATERANGE CLASS="twitch-stitched-ad"` (id `stitched-ad-...`)
//! carrying `X-TV-TWITCH-AD-*` metadata, or an `#EXTINF` segment whose title
//! contains "Amazon".

/// High-confidence Twitch ad signatures. `stitched-ad` matches both the
/// DATERANGE class (`twitch-stitched-ad`) and the id form (`stitched-ad-…`);
/// `X-TV-TWITCH-AD` matches the ad metadata attrs; `Amazon` is the ad EXTINF
/// title. `CUE-OUT` / `SCTE35` are intentionally absent — Twitch SSAI doesn't
/// use them.
const STRONG: &[&str] = &["stitched-ad", "X-TV-TWITCH-AD", "Amazon"];

/// Rolling ad-detection state for one stream. Cloneable/serializable so it can
/// be snapshotted for the UI or a Tauri command.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct AdDetectionState {
    /// True when the most recent media playlist carried ad-stitch markers.
    pub ads_present: bool,
    /// Consecutive ad-bearing polls (input to a future pivot debounce).
    pub consecutive_ad_polls: u32,
    /// Marker tags matched on the last scan.
    pub matched_markers: Vec<String>,
    /// Count of distinct ad breaks seen on the current stream.
    pub ad_events: u32,
}

/// Pure scan of a media playlist. Returns `(ads_present, matched_markers)`.
pub fn scan(playlist: &str) -> (bool, Vec<String>) {
    let mut matched = Vec::new();
    for n in STRONG {
        if playlist.contains(n) {
            matched.push((*n).to_string());
        }
    }
    (!matched.is_empty(), matched)
}

/// Fold a fresh scan into a running `state`. Returns `Some(break_number)` the
/// first poll a NEW ad break is seen (so callers can log it once), else `None`.
pub fn update(state: &mut AdDetectionState, playlist: &str) -> Option<u32> {
    let (ads, matched) = scan(playlist);
    let mut new_break = None;
    if ads {
        if !state.ads_present {
            state.ad_events = state.ad_events.saturating_add(1);
            new_break = Some(state.ad_events);
        }
        state.ads_present = true;
        state.consecutive_ad_polls = state.consecutive_ad_polls.saturating_add(1);
    } else {
        state.ads_present = false;
        state.consecutive_ad_polls = 0;
    }
    state.matched_markers = matched;
    new_break
}

/// Lower the playlist's declared `#EXT-X-TARGETDURATION` to the real maximum
/// segment length. Twitch over-declares 6s even though its live segments are ~2s,
/// and hls.js refuses to hold the playhead closer than ~one target duration to the
/// live edge (so a 6s declaration pins viewers ~6-8s back no matter the configured
/// sync target). Rewriting it to `ceil(max #EXTINF)` lets the player ride much
/// closer to live without stalling, on every channel, with no dependence on the
/// low-latency PREFETCH tags (which normal-latency broadcasts don't send).
///
/// Spec-safe: TARGETDURATION must be >= every segment's duration, and this only
/// ever LOWERS it toward that true maximum, never below it. Returns `Some(rewrite)`
/// only when it actually changed the value, so an already-correct playlist (or a
/// master playlist with no segments) passes through untouched (`None`).
pub fn retarget_playlist(playlist: &str) -> Option<String> {
    let mut max_dur = 0.0f64;
    let mut has_extinf = false;
    for line in playlist.lines() {
        if let Some(rest) = line.trim_start().strip_prefix("#EXTINF:") {
            has_extinf = true;
            if let Some(num) = rest.split(',').next() {
                if let Ok(d) = num.trim().parse::<f64>() {
                    if d > max_dur {
                        max_dur = d;
                    }
                }
            }
        }
    }
    // No media segments (master playlist) or unparseable durations: leave it alone.
    if !has_extinf || max_dur <= 0.0 {
        return None;
    }
    let new_td = (max_dur.ceil() as u64).max(1);

    let mut changed = false;
    let mut out = String::with_capacity(playlist.len());
    for line in playlist.lines() {
        if let Some(cur) = line.trim_start().strip_prefix("#EXT-X-TARGETDURATION:") {
            let cur_val = cur.trim().parse::<u64>().unwrap_or(0);
            if cur_val > new_td {
                out.push_str(&format!("#EXT-X-TARGETDURATION:{}", new_td));
                changed = true;
            } else {
                out.push_str(line);
            }
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    if changed {
        Some(out)
    } else {
        None
    }
}

/// Promote Twitch's low-latency `#EXT-X-TWITCH-PREFETCH:<url>` hints into real
/// `#EXTINF` segments so hls.js actually plays them. On a low-latency broadcast
/// Twitch appends the next ~2 in-progress segments as PREFETCH tags; hls.js
/// ignores that proprietary tag, so without this the player can never ride closer
/// than the last fully-published segment (~4s+ back even on a low-latency channel).
/// Each hint becomes `#EXTINF:<dur>,live\n<url>`, extending the live edge ~2s per
/// hint toward real time. The URLs are absolute (same CDN as the normal segments),
/// so the player fetches them directly, exactly as it already does for the others.
///
/// REFRESH-STABILITY (the load-bearing part — getting this wrong is what made the
/// player hard-freeze with a fatal `levelParsingError`):
/// hls.js reconciles every live-playlist refresh by matching segments on their
/// media-sequence number and requiring, for each segment present in both the old
/// and new playlist, that the URL path (after the trailing `?query` is stripped)
/// AND the discontinuity counter `cc` are identical. `cc` is the running count of
/// `#EXT-X-DISCONTINUITY` tags before a segment. Twitch signals a prefetch-boundary
/// discontinuity with the SEPARATE tag `#EXT-X-PREFETCH-DISCONTINUITY`, which hls.js
/// does not understand and silently drops. So a naive promotion (pass the marker
/// through, just rewrite the hint) gives a promoted segment `cc=X` now but `cc=X+1`
/// once Twitch finalizes it carrying a real `#EXT-X-DISCONTINUITY` — same SN, mismatched
/// `cc`, fatal `levelParsingError` on the first refresh, never recovers.
/// Fix: translate `#EXT-X-PREFETCH-DISCONTINUITY` into a real `#EXT-X-DISCONTINUITY`
/// emitted immediately before the promoted segment it precedes, so the promoted and
/// the eventual finalized form agree on `cc`. We deliberately do NOT touch
/// `#EXT-X-MEDIA-SEQUENCE` (appending hints at the tail gives them the same SN they
/// get when finalized) or `#EXT-X-DISCONTINUITY-SEQUENCE` (it seeds the cc of the
/// FIRST segment in the window; tail-appended hints never shift it, and adjusting it
/// would corrupt the already-published segments' cc). No cross-poll state is needed.
///
/// MUST only be called on an ad-free playlist: a prefetch segment landing inside an
/// ad break could be the ad itself, and promoting it would fast-path it past the
/// segment filter. Callers gate on the ad-detection state. Returns `None` when there
/// were no hints (so a normal-latency playlist passes through untouched).
pub fn promote_prefetch(playlist: &str) -> Option<String> {
    if !playlist.contains("#EXT-X-TWITCH-PREFETCH:") {
        return None;
    }
    // Estimate the segment duration from the playlist's own segments (Twitch is a
    // steady 2s; fall back to 2.0 if none parse).
    let dur = playlist
        .lines()
        .filter_map(|l| l.trim().strip_prefix("#EXTINF:"))
        .filter_map(|r| r.split(',').next())
        .filter_map(|n| n.trim().parse::<f64>().ok())
        .next_back()
        .unwrap_or(2.0);

    let mut out = String::with_capacity(playlist.len() + 160);
    let mut promoted = false;
    // Held when we've seen a `#EXT-X-PREFETCH-DISCONTINUITY` and are waiting to emit
    // the real `#EXT-X-DISCONTINUITY` immediately before the next promoted segment.
    let mut pending_discontinuity = false;

    // PROGRAM-DATE-TIME extrapolation for the promoted segments. Twitch stamps every
    // PUBLISHED segment with a PDT (capture wall-clock) but the PREFETCH hints carry
    // none. Without a PDT on the promoted segments, the player can't map the frame it's
    // showing back to a real time when the playhead is riding in that fresh region, so
    // the "behind live" readout falls back to the last published segment's PDT and reads
    // ~one prefetch-span (2-3s) too high. We extrapolate each promoted segment's capture
    // time from the last published segment's PDT plus the running duration, so the
    // readout is the same metric Twitch's "Latency To Broadcaster" shows, accurate at any
    // playhead position. If the upstream carries no PDT, stamping is skipped (graceful).
    let mut pending_pdt: Option<chrono::DateTime<chrono::Utc>> = None;
    let mut last_dur: f64 = dur;
    // Capture time of the end of the last published segment = the start of the first
    // promoted segment.
    let mut frontier: Option<chrono::DateTime<chrono::Utc>> = None;
    let mut promoted_idx: i64 = 0;

    for line in playlist.lines() {
        let trimmed = line.trim();
        // Track the published segments' PDT/duration so we can extrapolate forward.
        if let Some(v) = trimmed.strip_prefix("#EXT-X-PROGRAM-DATE-TIME:") {
            pending_pdt = chrono::DateTime::parse_from_rfc3339(v.trim())
                .ok()
                .map(|d| d.with_timezone(&chrono::Utc));
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("#EXTINF:") {
            if let Some(n) = rest.split(',').next() {
                if let Ok(d) = n.trim().parse::<f64>() {
                    last_dur = d;
                }
            }
            out.push_str(line);
            out.push('\n');
            continue;
        }
        // Match the prefetch-discontinuity marker BEFORE the prefetch hint check.
        // It carries no value; convert it to a real discontinuity in front of the
        // hint it precedes, never pass the proprietary tag through (hls.js drops it).
        if trimmed.starts_with("#EXT-X-PREFETCH-DISCONTINUITY") {
            pending_discontinuity = true;
            continue;
        }
        if let Some(url) = trimmed.strip_prefix("#EXT-X-TWITCH-PREFETCH:") {
            if pending_discontinuity {
                out.push_str("#EXT-X-DISCONTINUITY\n");
                pending_discontinuity = false;
            }
            // Stamp the extrapolated capture time: first promoted segment starts at the
            // frontier (end of last published), each subsequent one a duration later.
            if let Some(f) = frontier {
                let ts =
                    f + chrono::Duration::milliseconds((promoted_idx as f64 * dur * 1000.0) as i64);
                out.push_str(&format!(
                    "#EXT-X-PROGRAM-DATE-TIME:{}\n",
                    ts.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
                ));
            }
            out.push_str(&format!("#EXTINF:{:.3},live\n{}\n", dur, url.trim()));
            promoted = true;
            promoted_idx += 1;
        } else if !trimmed.is_empty() && !trimmed.starts_with('#') {
            // A published segment's URI line: its capture time was `pending_pdt`, so the
            // frontier (end of produced content) advances to that PDT plus the segment's
            // duration. This becomes the start of the first promoted segment.
            if let Some(p) = pending_pdt.take() {
                frontier = Some(p + chrono::Duration::milliseconds((last_dur * 1000.0) as i64));
            }
            out.push_str(line);
            out.push('\n');
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    if promoted {
        Some(out)
    } else {
        None
    }
}

/// `promote_prefetch` limited to the OLDEST hint. Twitch's newer hint names the
/// segment after the in-progress one, which has not begun producing; a player
/// that downloads promoted segments whole (AVPlayer) reaches for it, waits on a
/// response that cannot come yet, and stalls. Same ad-free precondition.
pub fn promote_oldest_prefetch(playlist: &str) -> Option<String> {
    let mut seen = false;
    let mut kept = String::with_capacity(playlist.len());
    for line in playlist.lines() {
        let trimmed = line.trim();
        let hint = trimmed.starts_with("#EXT-X-TWITCH-PREFETCH:");
        if seen && (hint || trimmed.starts_with("#EXT-X-PREFETCH-DISCONTINUITY")) {
            continue;
        }
        seen |= hint;
        kept.push_str(line);
        kept.push('\n');
    }
    promote_prefetch(&kept)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_recognizes_ad_markers() {
        let ad_pl = "#EXTM3U\n\
#EXT-X-DATERANGE:ID=\"stitched-ad-200\",CLASS=\"twitch-stitched-ad\",START-DATE=\"2026-06-02T10:00:02.000Z\",DURATION=4.0,X-TV-TWITCH-AD-ROLL-TYPE=MIDROLL\n\
#EXTINF:2.000,Amazon\n\
ad0.ts\n";
        let (ads, markers) = scan(ad_pl);
        assert!(ads);
        assert!(markers.iter().any(|m| m == "stitched-ad"));
        assert!(markers.iter().any(|m| m == "Amazon"));

        let clean_pl = "#EXTM3U\n#EXTINF:2.000,live\na.ts\n";
        let (ads, markers) = scan(clean_pl);
        assert!(!ads);
        assert!(markers.is_empty());
    }

    #[test]
    fn update_tracks_break_transitions() {
        let ad_pl = "#EXTM3U\n#EXTINF:2.000,Amazon\nad0.ts\n";
        let clean_pl = "#EXTM3U\n#EXTINF:2.000,live\na.ts\n";
        let mut st = AdDetectionState::default();
        // First ad poll opens break #1; the next ad poll is the same break.
        assert_eq!(update(&mut st, ad_pl), Some(1));
        assert_eq!(update(&mut st, ad_pl), None);
        assert_eq!(st.consecutive_ad_polls, 2);
        // A clean poll closes the window; a later ad poll is break #2.
        assert_eq!(update(&mut st, clean_pl), None);
        assert!(!st.ads_present);
        assert_eq!(update(&mut st, ad_pl), Some(2));
    }

    #[test]
    fn retarget_lowers_overdeclared_targetduration() {
        let pl = "#EXTM3U\n\
#EXT-X-VERSION:3\n\
#EXT-X-TARGETDURATION:6\n\
#EXT-X-MEDIA-SEQUENCE:100\n\
#EXTINF:2.000,live\n\
a.ts\n\
#EXTINF:2.000,live\n\
b.ts\n";
        let out = retarget_playlist(pl).expect("should lower 6 -> 2");
        assert!(out.contains("#EXT-X-TARGETDURATION:2"));
        assert!(!out.contains("#EXT-X-TARGETDURATION:6"));
        // Segments and other tags survive untouched.
        assert!(out.contains("a.ts") && out.contains("b.ts"));
        assert!(out.contains("#EXT-X-MEDIA-SEQUENCE:100"));
    }

    #[test]
    fn retarget_noop_when_already_correct() {
        let pl = "#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2.000,live\na.ts\n";
        assert!(retarget_playlist(pl).is_none());
    }

    #[test]
    fn retarget_noop_on_master_playlist() {
        // No #EXTINF (a master/variant playlist) -> never touch the targetduration.
        let pl = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1000000\nchunked.m3u8\n";
        assert!(retarget_playlist(pl).is_none());
    }

    #[test]
    fn retarget_never_raises() {
        // Already-low (or low-latency) declaration must not be raised toward a
        // larger segment; only lower. Here max EXTINF is 2 but TD is 1, leave it.
        let pl = "#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXTINF:2.000,live\na.ts\n";
        assert!(retarget_playlist(pl).is_none());
    }

    #[test]
    fn promote_prefetch_converts_hints_to_segments() {
        let pl = "#EXTM3U\n\
#EXT-X-TARGETDURATION:2\n\
#EXTINF:2.000,live\n\
https://cdn/seg1.ts\n\
#EXT-X-TWITCH-PREFETCH:https://cdn/seg2.ts\n\
#EXT-X-TWITCH-PREFETCH:https://cdn/seg3.ts\n";
        let out = promote_prefetch(pl).expect("should promote hints");
        // Hints are gone, replaced by real segments at the inherited duration.
        assert!(!out.contains("#EXT-X-TWITCH-PREFETCH"));
        assert!(out.contains("#EXTINF:2.000,live\nhttps://cdn/seg2.ts"));
        assert!(out.contains("#EXTINF:2.000,live\nhttps://cdn/seg3.ts"));
        // The original published segment is untouched.
        assert!(out.contains("https://cdn/seg1.ts"));
        // Two promoted + one original = three EXTINF lines.
        assert_eq!(out.matches("#EXTINF:").count(), 3);
    }

    #[test]
    fn promote_stamps_extrapolated_pdt_on_hints() {
        // A published segment captured at 00:00:00 with a 2s duration; the two prefetch
        // hints that follow are the in-progress content. Each promoted segment must get a
        // PROGRAM-DATE-TIME extrapolated forward (00:00:02, then 00:00:04) so the player's
        // frame-to-time mapping stays accurate in the promoted region.
        let pl = "#EXTM3U\n\
#EXT-X-TARGETDURATION:2\n\
#EXT-X-MEDIA-SEQUENCE:100\n\
#EXT-X-PROGRAM-DATE-TIME:2026-06-15T00:00:00.000Z\n\
#EXTINF:2.000,live\nhttps://cdn/seg100.ts\n\
#EXT-X-TWITCH-PREFETCH:https://cdn/seg101.ts\n\
#EXT-X-TWITCH-PREFETCH:https://cdn/seg102.ts\n";
        let out = promote_prefetch(pl).expect("promotes");
        assert!(out.contains(
            "#EXT-X-PROGRAM-DATE-TIME:2026-06-15T00:00:02.000Z\n#EXTINF:2.000,live\nhttps://cdn/seg101.ts"
        ));
        assert!(out.contains(
            "#EXT-X-PROGRAM-DATE-TIME:2026-06-15T00:00:04.000Z\n#EXTINF:2.000,live\nhttps://cdn/seg102.ts"
        ));
    }

    #[test]
    fn promote_without_pdt_skips_stamping() {
        // No PROGRAM-DATE-TIME upstream: promote, but never invent a timestamp.
        let pl = "#EXTM3U\n\
#EXTINF:2.000,live\nhttps://cdn/seg1.ts\n\
#EXT-X-TWITCH-PREFETCH:https://cdn/seg2.ts\n";
        let out = promote_prefetch(pl).expect("promotes");
        assert!(!out.contains("#EXT-X-PROGRAM-DATE-TIME"));
        assert!(out.contains("#EXTINF:2.000,live\nhttps://cdn/seg2.ts"));
    }

    #[test]
    fn promote_prefetch_noop_without_hints() {
        let pl = "#EXTM3U\n#EXTINF:2.000,live\nhttps://cdn/seg1.ts\n";
        assert!(promote_prefetch(pl).is_none());
    }

    // ──── Refresh-stability harness ────
    // Model of how hls.js assigns each segment a (media-sequence number, URL with
    // the trailing `?query` stripped, discontinuity counter) so we can prove its
    // cross-refresh consistency check passes without running hls.js. Rules mirror the
    // bundled parser: SN seeds from `#EXT-X-MEDIA-SEQUENCE` (default 0) and bumps once
    // per segment URI; cc seeds from `#EXT-X-DISCONTINUITY-SEQUENCE` (default 0) and
    // bumps on each `#EXT-X-DISCONTINUITY`; the URL is compared with everything from
    // the first `?` onward removed.
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
                // The URI is the next non-tag line; skip forward to it.
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

    /// Assert that every media-sequence number present in BOTH polls resolves to the
    /// same URL path and the same cc — exactly hls.js's `mapFragmentIntersection`
    /// rule. A violation is what fires the fatal `levelParsingError`.
    fn assert_refresh_consistent(poll_n: &str, poll_n1: &str) {
        let a = parse_segments(poll_n);
        let b = parse_segments(poll_n1);
        for (sn_a, url_a, cc_a) in &a {
            if let Some((_, url_b, cc_b)) = b.iter().find(|(sn_b, _, _)| sn_b == sn_a) {
                assert_eq!(url_a, url_b, "URL path mismatch at sn {sn_a}");
                assert_eq!(cc_a, cc_b, "cc mismatch at sn {sn_a} ({cc_a} != {cc_b})");
            }
        }
        // Guard against an empty overlap silently passing.
        let overlap = a
            .iter()
            .filter(|(sn_a, _, _)| b.iter().any(|(sn_b, _, _)| sn_b == sn_a))
            .count();
        assert!(overlap >= 2, "expected a real SN overlap, got {overlap}");
    }

    #[test]
    fn promote_translates_prefetch_discontinuity() {
        let pl = "#EXTM3U\n\
#EXT-X-TARGETDURATION:2\n\
#EXT-X-MEDIA-SEQUENCE:100\n\
#EXTINF:2.000,live\n\
https://cdn/seg100.ts\n\
#EXT-X-PREFETCH-DISCONTINUITY\n\
#EXT-X-TWITCH-PREFETCH:https://cdn/seg101.ts?token=A\n";
        let out = promote_prefetch(pl).expect("should promote");
        // The proprietary marker is gone, replaced by a real discontinuity placed
        // immediately before the promoted segment.
        assert!(!out.contains("#EXT-X-PREFETCH-DISCONTINUITY"));
        assert!(out.contains("#EXT-X-DISCONTINUITY\n#EXTINF:2.000,live\nhttps://cdn/seg101.ts"));
        // Exactly one discontinuity was introduced (no spurious extras).
        assert_eq!(out.matches("#EXT-X-DISCONTINUITY").count(), 1);
    }

    #[test]
    fn refresh_stable_cc_across_finalize() {
        // Poll N: four published segments, then a prefetch-discontinuity opening the
        // first hint (seg104), then a second hint (seg105).
        let poll_n = "#EXTM3U\n\
#EXT-X-TARGETDURATION:2\n\
#EXT-X-MEDIA-SEQUENCE:100\n\
#EXTINF:2.000,live\nhttps://cdn/seg100.ts\n\
#EXTINF:2.000,live\nhttps://cdn/seg101.ts\n\
#EXTINF:2.000,live\nhttps://cdn/seg102.ts\n\
#EXTINF:2.000,live\nhttps://cdn/seg103.ts\n\
#EXT-X-PREFETCH-DISCONTINUITY\n\
#EXT-X-TWITCH-PREFETCH:https://cdn/seg104.ts?token=A\n\
#EXT-X-TWITCH-PREFETCH:https://cdn/seg105.ts?token=A\n";
        // Poll N+1: window advanced by one (seg100 rolled off). seg104 has finalized,
        // now carrying the REAL discontinuity Twitch promised via the marker. seg105
        // is still a hint; seg106 is the new hint. Queries rotated (token=B) to prove
        // they're ignored by the path comparison.
        let poll_n1 = "#EXTM3U\n\
#EXT-X-TARGETDURATION:2\n\
#EXT-X-MEDIA-SEQUENCE:101\n\
#EXTINF:2.000,live\nhttps://cdn/seg101.ts\n\
#EXTINF:2.000,live\nhttps://cdn/seg102.ts\n\
#EXTINF:2.000,live\nhttps://cdn/seg103.ts\n\
#EXT-X-DISCONTINUITY\n\
#EXTINF:2.000,live\nhttps://cdn/seg104.ts?token=B\n\
#EXT-X-TWITCH-PREFETCH:https://cdn/seg105.ts?token=B\n\
#EXT-X-TWITCH-PREFETCH:https://cdn/seg106.ts?token=B\n";
        let pn = promote_prefetch(poll_n).expect("poll N has hints");
        let pn1 = promote_prefetch(poll_n1).expect("poll N+1 has hints");
        // The promoted poll N must agree with the finalized poll N+1 on every shared
        // SN. This is the exact case the naive promotion failed (seg104 cc 0 vs 1).
        assert_refresh_consistent(&pn, &pn1);
        // Spot-check the headline segment: cc must be 1 in both (one discontinuity
        // before it), proving the marker→discontinuity translation lined them up.
        let seg104_n = parse_segments(&pn)
            .into_iter()
            .find(|(sn, _, _)| *sn == 104)
            .unwrap();
        let seg104_n1 = parse_segments(&pn1)
            .into_iter()
            .find(|(sn, _, _)| *sn == 104)
            .unwrap();
        assert_eq!(seg104_n.2, 1);
        assert_eq!(seg104_n1.2, 1);
    }

    #[test]
    fn refresh_stable_steady_stream_no_discontinuity() {
        // No discontinuity anywhere: cc must stay 0 across the finalize boundary, and
        // promotion must not invent one.
        let poll_n = "#EXTM3U\n\
#EXT-X-MEDIA-SEQUENCE:200\n\
#EXTINF:2.000,live\nhttps://cdn/seg200.ts\n\
#EXTINF:2.000,live\nhttps://cdn/seg201.ts\n\
#EXT-X-TWITCH-PREFETCH:https://cdn/seg202.ts?token=A\n\
#EXT-X-TWITCH-PREFETCH:https://cdn/seg203.ts?token=A\n";
        let poll_n1 = "#EXTM3U\n\
#EXT-X-MEDIA-SEQUENCE:201\n\
#EXTINF:2.000,live\nhttps://cdn/seg201.ts\n\
#EXTINF:2.000,live\nhttps://cdn/seg202.ts?token=B\n\
#EXT-X-TWITCH-PREFETCH:https://cdn/seg203.ts?token=B\n\
#EXT-X-TWITCH-PREFETCH:https://cdn/seg204.ts?token=B\n";
        let pn = promote_prefetch(poll_n).expect("hints");
        let pn1 = promote_prefetch(poll_n1).expect("hints");
        assert!(!pn.contains("#EXT-X-DISCONTINUITY"));
        assert_refresh_consistent(&pn, &pn1);
    }

    #[test]
    fn promote_preserves_published_segment_cc() {
        // A published mid-window discontinuity plus a seeded discontinuity-sequence:
        // promotion must leave the published segments' cc untouched and only add cc at
        // the tail.
        let pl = "#EXTM3U\n\
#EXT-X-MEDIA-SEQUENCE:300\n\
#EXT-X-DISCONTINUITY-SEQUENCE:7\n\
#EXTINF:2.000,live\nhttps://cdn/seg300.ts\n\
#EXT-X-DISCONTINUITY\n\
#EXTINF:2.000,live\nhttps://cdn/seg301.ts\n\
#EXT-X-PREFETCH-DISCONTINUITY\n\
#EXT-X-TWITCH-PREFETCH:https://cdn/seg302.ts?token=A\n";
        let out = promote_prefetch(pl).expect("hints");
        let segs = parse_segments(&out);
        // Published: seg300 cc=7 (seed), seg301 cc=8 (one real discontinuity).
        assert_eq!(segs.iter().find(|(sn, _, _)| *sn == 300).unwrap().2, 7);
        assert_eq!(segs.iter().find(|(sn, _, _)| *sn == 301).unwrap().2, 8);
        // Promoted tail: seg302 cc=9 (the translated prefetch-discontinuity).
        assert_eq!(segs.iter().find(|(sn, _, _)| *sn == 302).unwrap().2, 9);
    }

    #[test]
    fn oldest_promotion_drops_the_newer_hint_and_its_discontinuity() {
        let pl = "#EXTM3U\n\
#EXT-X-TARGETDURATION:2\n\
#EXTINF:2.000,live\n\
https://cdn/seg1.ts\n\
#EXT-X-TWITCH-PREFETCH:https://cdn/seg2.ts\n\
#EXT-X-PREFETCH-DISCONTINUITY\n\
#EXT-X-TWITCH-PREFETCH:https://cdn/seg3.ts\n";
        let out = promote_oldest_prefetch(pl).expect("promotes");
        assert!(out.contains("#EXTINF:2.000,live\nhttps://cdn/seg2.ts"));
        assert!(!out.contains("seg3.ts"));
        assert!(!out.contains("#EXT-X-DISCONTINUITY"));
        assert_eq!(out.matches("#EXTINF:").count(), 2);
        assert!(promote_oldest_prefetch("#EXTM3U\n#EXTINF:2.000,live\na.ts\n").is_none());
    }
}
