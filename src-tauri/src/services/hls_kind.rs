//! Media-playlist classification shared by the solo and MultiNook relays.
//!
//! Twitch serves three shapes through the same media-playlist endpoint, and
//! the relay must treat them differently:
//!
//! - `Live`: a sliding window (~12-15 segments) with no `#EXT-X-ENDLIST`.
//!   Segment URLs get re-signed across refreshes, so the relay pins them
//!   stable (`hls_projection`) and probes the low-latency origin.
//! - `Event`: the recording VOD of a broadcast that is still live.
//!   `#EXT-X-PLAYLIST-TYPE:EVENT`, no `ENDLIST`, `MEDIA-SEQUENCE:0`, and it
//!   only ever GROWS (every segment since the start stays listed). It is a
//!   VOD for every purpose the relay cares about: fully seekable, stable
//!   segment names, no prefetch tags. Treating it as live was GitHub #216:
//!   the projection's 120-sequence retention window turned any seek older
//!   than ~20 minutes into a dead `vseg/` lookup.
//! - `Vod`: a finished video, `#EXT-X-ENDLIST` present, static.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaylistKind {
    Live,
    Event,
    Vod,
}

impl PlaylistKind {
    /// True for both VOD shapes: the playlist is seekable and must not be
    /// rewritten or pruned.
    pub fn is_vod(self) -> bool {
        matches!(self, PlaylistKind::Event | PlaylistKind::Vod)
    }
}

/// Classify a media playlist by its tags. Cheap: two substring scans over
/// text the relay already holds.
pub fn classify(playlist: &str) -> PlaylistKind {
    if playlist.contains("#EXT-X-ENDLIST") {
        return PlaylistKind::Vod;
    }
    match playlist_type(playlist) {
        Some(t) if t.eq_ignore_ascii_case("EVENT") => PlaylistKind::Event,
        // `PLAYLIST-TYPE:VOD` promises the playlist never changes, so it is
        // seekable even before its ENDLIST lands.
        Some(t) if t.eq_ignore_ascii_case("VOD") => PlaylistKind::Vod,
        _ => PlaylistKind::Live,
    }
}

fn playlist_type(playlist: &str) -> Option<&str> {
    playlist
        .lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix("#EXT-X-PLAYLIST-TYPE:"))
        .map(str::trim)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Head of a real recording-VOD playlist, captured from Twitch 2026-09-06.
    const RECORDING_VOD: &str = "#EXTM3U\n\
#EXT-X-VERSION:3\n\
#EXT-X-TARGETDURATION:13\n\
#ID3-EQUIV-TDTG:2026-09-06T07:00:35\n\
#EXT-X-PLAYLIST-TYPE:EVENT\n\
#EXT-X-MEDIA-SEQUENCE:0\n\
#EXT-X-TWITCH-ELAPSED-SECS:0.000\n\
#EXT-X-TWITCH-TOTAL-SECS:24311.908\n\
#EXTINF:10.000,\n\
0.ts\n\
#EXTINF:10.000,\n\
1.ts\n";

    const LIVE: &str = "#EXTM3U\n\
#EXT-X-VERSION:3\n\
#EXT-X-TARGETDURATION:6\n\
#EXT-X-MEDIA-SEQUENCE:4021\n\
#EXT-X-TWITCH-ELAPSED-SECS:8042.000\n\
#EXTINF:2.000,live\n\
https://video-edge.example/seg-4021.ts?sig=a\n\
#EXT-X-TWITCH-PREFETCH:https://video-edge.example/seg-4022.ts?sig=b\n";

    #[test]
    fn recording_vod_is_event() {
        assert_eq!(classify(RECORDING_VOD), PlaylistKind::Event);
        assert!(classify(RECORDING_VOD).is_vod());
    }

    #[test]
    fn finished_vod_is_vod() {
        let finished = format!("{}#EXT-X-ENDLIST\n", RECORDING_VOD);
        assert_eq!(classify(&finished), PlaylistKind::Vod);
    }

    #[test]
    fn sliding_window_is_live() {
        assert_eq!(classify(LIVE), PlaylistKind::Live);
        assert!(!classify(LIVE).is_vod());
    }

    #[test]
    fn a_vod_typed_playlist_without_endlist_is_still_a_vod() {
        let vod_typed = RECORDING_VOD.replace("PLAYLIST-TYPE:EVENT", "PLAYLIST-TYPE:VOD");
        assert_eq!(classify(&vod_typed), PlaylistKind::Vod);
    }
}
