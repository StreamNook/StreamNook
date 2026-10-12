//! Relay-local segment ids keyed by media sequence number.
//!
//! A relay that rewrites a media playlist's segment URIs to its own short paths
//! (`pseg/<n>`, `up/<n>`) must hand the player the SAME path for the same
//! segment on every refresh. hls.js checks exactly that: when a refreshed
//! playlist names a different URI (queries stripped) for a sequence number it
//! already holds, it rejects the refresh as a fatal `levelParsingError`
//! ("media sequence mismatch").
//!
//! Keying the relay path by the upstream URL breaks that rule whenever the
//! upstream re-signs its segment URLs. Kick's IVS does it every few minutes: a
//! new `playlist-creation` DATERANGE arrives and every segment URL in the window
//! carries a fresh `dna=` signature, so each one looked new and got a new path
//! while keeping its sequence number. The sequence number is what identifies a
//! segment in HLS, so that is the key here, and the URL behind an id is simply
//! replaced with the freshest signature on every refresh.
//!
//! The relayed sequence never runs backwards across an upstream restart: when
//! a playlist's whole window sits below the previous one (the upstream started
//! numbering again), the relay offsets the new numbers to continue above the
//! highest one it has already served.

use std::collections::HashMap;

/// Where a media playlist's segments sit in the upstream sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeqScan {
    /// `#EXT-X-MEDIA-SEQUENCE` of the playlist: the first segment's number.
    pub first: u64,
    /// Number of segment URI lines.
    pub count: usize,
}

/// True for a line that names a URI rather than a tag or a comment.
pub fn is_uri_line(trimmed: &str) -> bool {
    !trimmed.is_empty() && !trimmed.starts_with('#')
}

/// Read a media playlist's sequence position.
///
/// `None` for a master playlist, an empty one, or one that does not state its
/// `#EXT-X-MEDIA-SEQUENCE`. The spec defaults a missing tag to 0, but a live
/// window that never names its position cannot be keyed by it: every refresh
/// would claim to start at 0 and different segments would share a key, so
/// callers fall back to keying by URL instead.
pub fn scan(playlist: &str) -> Option<SeqScan> {
    let mut first: Option<u64> = None;
    let mut has_extinf = false;
    let mut count = 0usize;
    for line in playlist.lines() {
        let t = line.trim();
        if let Some(v) = t.strip_prefix("#EXT-X-MEDIA-SEQUENCE:") {
            first = v.trim().parse().ok();
        } else if t.starts_with("#EXTINF") {
            has_extinf = true;
        } else if is_uri_line(t) {
            count += 1;
        }
    }
    match (first, has_extinf && count > 0) {
        (Some(first), true) => Some(SeqScan { first, count }),
        _ => None,
    }
}

/// Upper bound on remembered sequence numbers; a live window is a handful.
const MAX_TRACKED: usize = 512;

/// Relayed sequence number -> relay id, plus the offset that keeps the relayed
/// sequence monotonic across an upstream restart.
#[derive(Default, Debug)]
pub struct SeqIds {
    by_sn: HashMap<u64, u32>,
    /// Added to every upstream sequence number. Zero until an upstream restart,
    /// so the relayed numbers normally equal the upstream's.
    offset: u64,
    /// Upstream first sequence number of the newest window seen.
    newest_first: Option<u64>,
    /// Highest relayed sequence number handed out so far.
    max_relayed: Option<u64>,
}

impl SeqIds {
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Register a refreshed playlist and return the relayed sequence number of
    /// its first segment (what its `#EXT-X-MEDIA-SEQUENCE` line must say).
    pub fn begin(&mut self, scan: &SeqScan) -> u64 {
        let upstream_last = scan.first + scan.count.saturating_sub(1) as u64;
        match (self.newest_first, self.max_relayed) {
            // The whole window sits below where the previous one started: the
            // upstream restarted its numbering. Continue above everything
            // already served so no relayed number is reused for new media. A
            // merely stale copy of an older window still overlaps the newest
            // one and keeps its numbers.
            (Some(prev_first), Some(max)) if upstream_last < prev_first => {
                self.offset = (max + 1).saturating_sub(scan.first);
                self.by_sn.clear();
                self.newest_first = Some(scan.first);
            }
            (Some(prev_first), _) => {
                self.newest_first = Some(prev_first.max(scan.first));
            }
            (None, _) => self.newest_first = Some(scan.first),
        }
        let relayed_first = scan.first + self.offset;
        let relayed_last = relayed_first + scan.count.saturating_sub(1) as u64;
        self.max_relayed = Some(self.max_relayed.map_or(relayed_last, |m| m.max(relayed_last)));
        relayed_first
    }

    /// The id already given to a relayed sequence number.
    pub fn get(&self, relayed_sn: u64) -> Option<u32> {
        self.by_sn.get(&relayed_sn).copied()
    }

    /// Remember the id given to a relayed sequence number.
    pub fn insert(&mut self, relayed_sn: u64, id: u32) {
        self.by_sn.insert(relayed_sn, id);
        if self.by_sn.len() > MAX_TRACKED {
            let floor = self
                .max_relayed
                .unwrap_or(relayed_sn)
                .saturating_sub((MAX_TRACKED / 2) as u64);
            self.by_sn.retain(|sn, _| *sn > floor);
        }
    }

    /// Forget ids at or below `cutoff`, mirroring the owner's URL-map trim.
    pub fn retain_ids_above(&mut self, cutoff: u32) {
        self.by_sn.retain(|_, id| *id > cutoff);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_reads_a_media_playlist_and_skips_tags_and_prefetch_lines() {
        let pl = "#EXTM3U\n#EXT-X-MEDIA-SEQUENCE:321\n#EXTINF:4.1,live\nhttps://a/1.ts\n\
                  #EXTINF:4.1,live\nhttps://a/2.ts\n#EXT-X-PREFETCH:https://a/3.ts\n";
        assert_eq!(scan(pl), Some(SeqScan { first: 321, count: 2 }));
    }

    #[test]
    fn scan_declines_master_and_unnumbered_playlists() {
        let master = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1\nhttps://a/v.m3u8\n";
        assert_eq!(scan(master), None);
        let unnumbered = "#EXTM3U\n#EXTINF:2.0,\nhttps://a/1.ts\n";
        assert_eq!(scan(unnumbered), None);
    }

    #[test]
    fn a_stale_window_keeps_its_numbers_and_a_restart_continues_upward() {
        let mut ids = SeqIds::default();
        assert_eq!(ids.begin(&SeqScan { first: 321, count: 7 }), 321);
        assert_eq!(ids.begin(&SeqScan { first: 322, count: 7 }), 322);
        // A CDN edge serving the previous window: overlaps, no offset.
        assert_eq!(ids.begin(&SeqScan { first: 321, count: 7 }), 321);
        // The upstream starts numbering again from 0: continue above 328.
        assert_eq!(ids.begin(&SeqScan { first: 0, count: 3 }), 329);
        assert_eq!(ids.begin(&SeqScan { first: 1, count: 3 }), 330);
    }
}
