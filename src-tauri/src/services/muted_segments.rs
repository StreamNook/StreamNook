//! Muted audio ranges on a Twitch VOD, normalized once for the seekbar.
//!
//! Twitch runs audio recognition over a finished broadcast and mutes the
//! matching audio in 180-second blocks, reporting them through GQL as
//! `video.muteInfo.mutedSegmentConnection.nodes` (`offset` / `duration`, both
//! whole seconds). Probed live 2026-09-12 against xqc VOD 2871438321: 13
//! nodes, every one `duration: 180`.
//!
//! Two facts from that probe shape this module.
//!
//! 1. **Adjacent blocks arrive separately.** Offsets 16201 / 16381 / 16561
//!    are three back-to-back 180 s blocks, which would draw as three touching
//!    marks instead of one nine-minute band. They have to be merged, and the
//!    offsets sit on a grid that is a second or two off true multiples, hence
//!    the epsilon. Real gaps are never bridged by the exact pass.
//! 2. **`mutedSegmentConnection` is `null`, never `[]`.** On a `RECORDED`
//!    video that means genuinely unmuted; on a `RECORDING` one it means
//!    Twitch has not determined them yet. So an empty result here is "none
//!    KNOWN", and the caller must not present it as "no muted sections" while
//!    a broadcast is still recording.

use serde::Serialize;

/// Gap (seconds) within which two ranges count as contiguous. Twitch's
/// offsets are a second or two off exact 180 s multiples.
const ADJACENT_EPSILON: f64 = 1.0;

/// Marks the seekbar will draw. Past this the bands are coalesced with a
/// wider gap: a long music broadcast can mute an alternating pattern that
/// survives exact merging, and hundreds of absolutely positioned divs do not
/// belong in a control bar.
const MAX_MARKS: usize = 200;

/// One contiguous stretch of muted audio, in seconds from the VOD start.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct MutedRange {
    pub start_secs: f64,
    pub end_secs: f64,
}

/// Merge Twitch's mute blocks into contiguous bands, clamped to the video
/// length. `nodes` are `(offset, duration)` pairs straight off GQL.
///
/// The result is sorted, non-overlapping, free of zero-width entries, and no
/// longer than `MAX_MARKS`.
pub fn normalize(nodes: &[(i64, i64)], length_secs: Option<u32>) -> Vec<MutedRange> {
    let mut ranges: Vec<MutedRange> = nodes
        .iter()
        .filter(|(offset, duration)| *offset >= 0 && *duration > 0)
        .map(|(offset, duration)| MutedRange {
            start_secs: *offset as f64,
            end_secs: (*offset + *duration) as f64,
        })
        .collect();
    if ranges.is_empty() {
        return ranges;
    }
    ranges.sort_by(|a, b| a.start_secs.total_cmp(&b.start_secs));
    let mut merged = fold(&ranges, ADJACENT_EPSILON);

    if let Some(len) = length_secs.filter(|n| *n > 0) {
        let len = len as f64;
        merged.retain(|r| r.start_secs < len);
        for r in merged.iter_mut() {
            r.end_secs = r.end_secs.min(len);
        }
    }
    merged.retain(|r| r.end_secs > r.start_secs);

    if merged.len() > MAX_MARKS {
        match length_secs.filter(|n| *n > 0) {
            // Collapse gaps narrower than one mark's own width: they are not
            // separable on screen anyway, so this loses nothing visible.
            Some(len) => {
                let epsilon = (len as f64 / MAX_MARKS as f64).max(ADJACENT_EPSILON);
                merged = fold(&merged, epsilon);
                merged.truncate(MAX_MARKS);
            }
            // No length to scale against; keep the earliest marks.
            None => merged.truncate(MAX_MARKS),
        }
    }
    merged
}

/// Fold a sorted range list, merging any pair separated by `epsilon` or less.
fn fold(sorted: &[MutedRange], epsilon: f64) -> Vec<MutedRange> {
    let mut out: Vec<MutedRange> = Vec::with_capacity(sorted.len());
    for r in sorted {
        match out.last_mut() {
            Some(prev) if r.start_secs <= prev.end_secs + epsilon => {
                prev.end_secs = prev.end_secs.max(r.end_secs);
            }
            _ => out.push(*r),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real GQL response for xqc VOD 2871438321 (43,691 s), probed
    /// 2026-09-12. Note 16201 / 16381 / 16561 are three contiguous blocks.
    const XQC: &[(i64, i64)] = &[
        (0, 180),
        (14041, 180),
        (15301, 180),
        (15661, 180),
        (16021, 180),
        (16201, 180),
        (16381, 180),
        (16561, 180),
        (16921, 180),
        (17101, 180),
        (17281, 180),
        (17461, 180),
        (17641, 180),
    ];

    #[test]
    fn contiguous_blocks_merge_into_one_band() {
        let out = normalize(XQC, Some(43691));
        // 13 nodes become 6 bands. Two runs are contiguous on the 180 s grid:
        // 16021 + 16201 + 16381 + 16561 -> 16021..16741 (12 min), and
        // 16921 + 17101 + 17281 + 17461 + 17641 -> 16921..17821 (15 min).
        assert_eq!(out.len(), 6, "{out:?}");
        assert!(
            out.contains(&MutedRange { start_secs: 16021.0, end_secs: 16741.0 }),
            "the four back-to-back blocks must become one band: {out:?}"
        );
        assert!(
            out.contains(&MutedRange { start_secs: 16921.0, end_secs: 17821.0 }),
            "the five back-to-back blocks must become one band: {out:?}"
        );
        // Merging must not invent or lose muted time: 13 blocks x 180 s.
        let total: f64 = out.iter().map(|r| r.end_secs - r.start_secs).sum();
        assert_eq!(total, 13.0 * 180.0);
        // Still sorted and non-overlapping.
        for pair in out.windows(2) {
            assert!(pair[0].end_secs < pair[1].start_secs);
        }
    }

    #[test]
    fn a_real_gap_is_not_bridged() {
        // 15661..15841 then 16021: a 180 s unmuted gap between them.
        let out = normalize(&[(15661, 180), (16021, 180)], Some(43691));
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn empty_in_empty_out() {
        assert!(normalize(&[], Some(100)).is_empty());
        assert!(normalize(&[], None).is_empty());
    }

    #[test]
    fn zero_and_negative_entries_are_dropped() {
        assert!(normalize(&[(0, 0), (10, -5), (-10, 180)], Some(100)).is_empty());
    }

    #[test]
    fn ranges_clamp_to_the_video_length() {
        let out = normalize(&[(0, 180)], Some(100));
        assert_eq!(out, vec![MutedRange { start_secs: 0.0, end_secs: 100.0 }]);
    }

    #[test]
    fn a_range_starting_past_the_end_is_dropped() {
        assert!(normalize(&[(500, 180)], Some(100)).is_empty());
    }

    #[test]
    fn an_unknown_length_leaves_ranges_unclamped() {
        let out = normalize(&[(0, 180)], None);
        assert_eq!(out, vec![MutedRange { start_secs: 0.0, end_secs: 180.0 }]);
    }

    #[test]
    fn the_mark_count_is_bounded() {
        // 400 isolated 10 s mutes, 100 s apart, over a 40,000 s VOD.
        let nodes: Vec<(i64, i64)> = (0..400).map(|i| (i * 100, 10)).collect();
        let out = normalize(&nodes, Some(40_000));
        assert!(out.len() <= MAX_MARKS, "got {} marks", out.len());
        for pair in out.windows(2) {
            assert!(pair[0].start_secs < pair[1].start_secs, "still sorted");
            assert!(pair[0].end_secs <= pair[1].start_secs, "non-overlapping");
        }
    }

    #[test]
    fn the_mark_count_is_bounded_without_a_length() {
        let nodes: Vec<(i64, i64)> = (0..400).map(|i| (i * 100, 10)).collect();
        assert_eq!(normalize(&nodes, None).len(), MAX_MARKS);
    }
}
