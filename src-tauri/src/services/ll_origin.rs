//! Low-latency HLS origin for Twitch.
//!
//! Twitch low-latency channels ship chunked CMAF: each ~2s segment is ~19
//! `moof`+`mdat` fragment pairs (~105ms each), and the in-progress segment is
//! delivered progressively (the connection is held open while it encodes). The
//! `#EXT-X-TWITCH-PREFETCH` tag points at that in-progress segment.
//!
//! This module turns that into a real LL-HLS origin so hls.js (in `lowLatencyMode`)
//! can play ~2s from live instead of waiting a whole segment behind it (the ~5-6s
//! floor of whole-segment promotion). A background "edge reader" streams the
//! in-progress segment, splits it into parts the instant each `moof`+`mdat` lands,
//! and serves an LL-HLS playlist with `#EXT-X-PART` + blocking reload. hls.js fetches
//! each part as it appears.
//!
//! Verified hls.js 1.6.15 contract this implements:
//! - Low latency comes from BLOCKING the playlist reload (`_HLS_msn`/`_HLS_part`),
//!   then plain GETs of listed `#EXT-X-PART` URIs. `#EXT-X-PRELOAD-HINT` is ignored
//!   by hls.js, so we don't emit it.
//! - Only ever list a part whose bytes we already hold (a listed part that 404s with
//!   no alternate quality is a fatal, non-recovering freeze). Same for the blocking
//!   wait: bounded, then return the current playlist rather than hang.
//! - Always keep >=1 complete `#EXTINF` segment (an all-parts playlist trips
//!   "not enough fragments to start").
//! - Part URLs need not be stable across refreshes (parts match by (sn, partIndex)).
//!   The init segment (`#EXT-X-MAP`) is left as the stable absolute upstream URL.
//!
//! One `LlOrigin` instance serves one upstream stream. The solo relay uses the
//! shared module-level instance (facade functions at the bottom); MultiNook builds
//! one per tile so any number of low-latency grid tiles can run concurrently.

use log::{debug, info, warn};
use once_cell::sync::Lazy;
use reqwest::{Client, Response};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Notify;
use tokio::task::JoinHandle;

/// Max segments retained in the live window (a few complete + the in-progress one).
const MAX_SEGMENTS: usize = 6;
/// Smaller live window for MultiNook tiles: the window is held entirely in memory
/// and multiplies across every tile of a grid, and tiles ride a slightly looser
/// cushion than solo so they never need the deeper history.
pub(crate) const TILE_MAX_SEGMENTS: usize = 4;
/// Declared `PART-TARGET` (max part duration). Generously above Twitch's ~0.105s
/// chunks so every real part is comfortably under it (spec requires that), and so
/// hls.js's edge clamp (`edge - partTarget`) leaves headroom. It also sets hls.js's
/// low-latency playlist-reload timeout cap (`PART-TARGET * 3`): 0.667 puts that
/// at 2.0s so a held blocking reload plus transit and queueing stays under it.
/// A 1.6s cap was not enough once a playlist response queued behind a part fetch
/// on a shared keep-alive connection. Costs ~0.167s of edge latency.
const PART_TARGET: f64 = 0.667;
/// Fallback per-part duration for CMAF parts whose real span could not be
/// measured (init unavailable / parse failure). NOT merely advisory: hls.js
/// SUMS declared playlist durations to place fragments, and a systematic
/// declared-vs-real error compounds into playlist-vs-buffer drift. Real Twitch
/// CMAF parts run ~0.105s, and declaring 0.1 drifts the playlist edge seconds
/// behind played media within an hour, which surfaces as hls.js currentTime
/// rewinds and a MediaSource-ended recovery loop. Measured durations from the
/// part's own sample tables are the primary path.
const NOMINAL_PART_DUR: f64 = 0.1;
const TARGET_DURATION: u64 = 2;
/// How long a blocking reload waits for the requested part before returning the
/// current playlist anyway (hls.js then retries; never hang indefinitely). MUST
/// stay under hls.js's low-latency reload timeout, which it caps at
/// `max(PART-TARGET * 3, TARGETDURATION * 0.8)`, which is 2.0s for this origin
/// at PART-TARGET 0.667. That budget covers this 1.0s hold plus roughly 1.0s of
/// transit and queueing; longer holds trip `levelLoadTimeOut` once a playlist
/// response queues behind a part fetch on a shared keep-alive connection.
///
/// During a real part drought a shorter hold only means hls.js re-polls sooner,
/// so nothing is lost. If timeouts persist the next lever is removing the
/// shared-connection head-of-line blocking (a dedicated playlist connection, or
/// HTTP/2 multiplexing), not this value.
const BLOCK_TIMEOUT: Duration = Duration::from_millis(1000);
/// How long the reader waits for a preopened next-segment connection before
/// abandoning it for the poll path. Normally ready instantly (the previous
/// segment just finished, so the next is already producing).
const PREOPEN_WAIT: Duration = Duration::from_secs(4);
/// How many trailing published segments to backfill (fully) at start so the first
/// served playlist already has complete `#EXTINF` segments.
const BACKFILL_SEGMENTS: usize = 2;
/// Bound on one-shot fetches (probe, backfill, playlist polls). The streaming
/// in-progress GET is intentionally NOT bounded this way (it's long-lived); it uses a
/// per-chunk read timeout instead. Without these, a hung fetch could block stream
/// start or freeze the reader.
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
/// Max wait for the FIRST chunk of an in-progress segment. Generous on purpose:
/// the CDN holds a preopened GET until the segment starts producing, so a slow
/// segment start legitimately spends a couple of seconds here (2.5s+ shell gaps
/// observed on healthy streams).
const FIRST_CHUNK_TIMEOUT: Duration = Duration::from_secs(4);
/// Max MID-STREAM silence before the segment is abandoned (flush what arrived,
/// mark complete, move to the next). Chunks normally land every ~0.3s or
/// faster; multi-second silence means the upstream transfer stalled, and
/// waiting it out is exactly the part drought that starves the player: a
/// multi-second production gap drains the buffer into a stall.
/// The next segment is usually already preopened and producing, so abandoning
/// the dead tail converts a 5-10s freeze into a sub-frame skip (the transmuxer
/// resets its half-assembled PES; the next segment leads with an IDR).
const MIDSTREAM_CHUNK_TIMEOUT: Duration = Duration::from_secs(2);
/// Wall-clock grace past the segment's own duration before a TRICKLING
/// transfer is abandoned too. Silence detection misses slow-but-alive
/// transfers: a 2s segment taking 5s to deliver opens a multi-second part gap
/// and stalls, while the following segments sit complete upstream.
/// A live in-progress segment finishes ~its duration after
/// its first chunk; running this far past that means delivery is slower than
/// real time and the famine only grows. Measured from the FIRST chunk (a
/// preopened connection legitimately idles until the segment starts).
const SEGMENT_DEADLINE_GRACE: f64 = 1.5;
/// Master switch for the MPEG-TS low-latency origin. Twitch serves H.264 (the
/// "chunked"/source quality on the vast majority of channels) as MPEG-TS with NO
/// `#EXT-X-MAP`; only HEVC/AV1 variants are CMAF. The CMAF origin silently no-ops
/// on TS, so without this the true-2s path never runs for most streams and the
/// player falls back to whole-segment promotion (~3-3.5s). Unlike the CMAF path,
/// the TS part demuxer in hls.js needs a real-stream sign-off, so this stays a
/// one-line opt-in until a build is curl+playback verified. When false (or if a TS
/// stream can't be chunked), the origin stays inactive and the stable promotion
/// fallback serves the stream — never worse than before.
const ENABLE_TS_LL_ORIGIN: bool = true;
/// Target real part duration for the TS chunker, in 90 kHz PTS ticks (~0.30s).
/// Kept well under `PART_TARGET` (0.5s) so every emitted part is spec-legal even
/// when a frame straddles the boundary. CMAF parts come pre-split by Twitch
/// (~0.105s) so this only governs TS.
const TS_PART_PTS: u64 = 27_000;
/// Transmux TS parts to fMP4 before publishing them, so hls.js takes its
/// passthrough path (explicit per-sample timestamps, no stateful JS remux).
/// This is the structural fix for the duplicate-append A/V fork: hls.js
/// occasionally re-fetches a fragment it already buffered (trigger unknown, but
/// frequent enough to matter), and on the raw
/// TS path each re-append inserted 2s of duplicate video at the buffer end
/// while audio coalesced, desyncing A/V by -2s per occurrence. With fMP4 the
/// same re-append overwrites the same time range and is harmless. When false,
/// raw TS parts are served exactly as before (kill switch).
const ENABLE_TS_TRANSMUX: bool = true;

/// Segment container. Twitch ships CMAF (fMP4, with `#EXT-X-MAP`) for HEVC/AV1 and
/// MPEG-TS (no init segment) for H.264. The two split into low-latency parts
/// differently and render slightly different playlists (TS has no `#EXT-X-MAP`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Container {
    /// fMP4: parts are `moof`+`mdat` pairs; an `#EXT-X-MAP` init segment is required.
    Cmaf,
    /// MPEG-TS: parts are runs of 188-byte packets cut at video-PES boundaries; no
    /// init segment (TS is self-initializing — PAT/PMT lead each segment).
    Ts,
}

/// Container-tagged streaming splitter: turns a (possibly partial) segment byte
/// stream into low-latency parts. CMAF cuts on box pairs; TS cuts on packet-aligned
/// video-PES boundaries.
enum Chunker {
    Cmaf(BoxChunker),
    Ts(TsChunker),
}

impl Chunker {
    fn new(container: Container) -> Self {
        match container {
            Container::Cmaf => Chunker::Cmaf(BoxChunker::new()),
            Container::Ts => Chunker::Ts(TsChunker::new()),
        }
    }
    /// Each part is returned with its media duration in seconds. CMAF parts come
    /// pre-split by Twitch (~0.105s) and the nominal value is fine (≈19 parts ≈ 2s).
    /// TS parts MUST carry their real PTS-derived duration: with only ~7 parts per
    /// segment, a nominal 0.1s would undercount the segment to ~0.7s, and hls.js then
    /// re-downloads the whole 2s segment on top of the parts (double-appended video =
    /// the repeat-and-drift boomerang). Real durations make the parts tile the segment.
    fn push(&mut self, data: &[u8]) -> Vec<(Vec<u8>, f64)> {
        match self {
            Chunker::Cmaf(c) => c
                .push(data)
                .into_iter()
                .map(|b| (b, NOMINAL_PART_DUR))
                .collect(),
            Chunker::Ts(c) => c.push(data),
        }
    }
    fn flush(&mut self) -> Option<(Vec<u8>, f64)> {
        match self {
            Chunker::Cmaf(c) => c.flush().map(|b| (b, NOMINAL_PART_DUR)),
            Chunker::Ts(c) => c.flush(),
        }
    }
}

/// What `start()` found, so the relay can tell the player apart three ways: a real
/// LL-HLS origin is serving parts (`active` → hls.js `lowLatencyMode`), or it isn't
/// but the upstream is a low-latency broadcast (`has_prefetch` → the player rides a
/// tighter promotion cushion), or neither (normal-latency → the wide cushion).
#[derive(Clone, Copy, Debug, Default)]
pub struct StartOutcome {
    pub active: bool,
    pub has_prefetch: bool,
}

#[derive(Clone)]
struct Part {
    duration: f64,
    bytes: Arc<Vec<u8>>,
}

struct Segment {
    sn: u64,
    pdt: Option<String>,
    complete: bool,
    duration: f64,
    parts: Vec<Part>,
    /// Whole-segment bytes, assembled lazily on the first `seg/` fetch and
    /// handed out by refcount after that. Stays `None` in LL steady state
    /// (the player fetches parts only), so the parts path pays nothing.
    assembled: Option<bytes::Bytes>,
}

struct LiveEdge {
    /// CMAF init segment URL. Empty for `Container::Ts` (TS needs no `#EXT-X-MAP`).
    init_url: String,
    /// Origin-generated init segment (TS transmux path). When set, the playlist
    /// advertises `#EXT-X-MAP:URI="init.mp4"` and the relay serves these bytes;
    /// parts are fMP4 regardless of `container` (which still selects the
    /// CHUNKER for the upstream byte stream).
    init_bytes: Option<Arc<Vec<u8>>>,
    container: Container,
    target_duration: u64,
    part_target: f64,
    segments: VecDeque<Segment>,
}

impl LiveEdge {
    /// Whether the parts served to the player are fMP4 (native CMAF upstream or
    /// the TS transmux), which decides the `#EXT-X-MAP` line and part extension.
    fn cmaf_presented(&self) -> bool {
        self.container == Container::Cmaf || self.init_bytes.is_some()
    }
}

/// One live-edge origin. Solo and MultiNook each route their relay traffic to an
/// instance of this; all state below is per-stream.
pub struct LlOrigin {
    live_edge: Mutex<Option<LiveEdge>>,
    /// Wakes blocked playlist reloads whenever the edge gains a part or segment.
    notify: Notify,
    /// Monotonic change marker for the served playlist: bumped by `wake_serves`
    /// alongside every notify. Directive-less playlist holds compare against it
    /// to mean "anything changed since the request arrived".
    edge_version: AtomicU64,
    reader_task: Mutex<Option<JoinHandle<()>>>,
    /// Generation counter: bumped on every start/stop so a lingering reader task can
    /// detect it has been superseded and exit even before its `abort()` lands.
    generation: AtomicU64,
    /// Live window size (complete segments + the in-progress one).
    max_segments: usize,
    /// TS-to-fMP4 transmuxer, present only on the TS path with
    /// `ENABLE_TS_TRANSMUX`. Behind its own lock (not `live_edge`) so per-part
    /// CPU work never contends with playlist/part serving. Access is naturally
    /// sequential: activation backfill, then the single reader task.
    transmux: Mutex<Option<crate::services::ts_fmp4::Transmuxer>>,
    /// CMAF path only: the video track's (track_id, timescale) from the
    /// upstream init segment, fetched once at activation. Lets every published
    /// part carry its MEASURED duration instead of `NOMINAL_PART_DUR` (the
    /// nominal lie compounds into playlist-vs-buffer drift; see that const).
    cmaf_video: Mutex<Option<(u32, u32)>>,
    /// Retirement grace for segments leaving the window (eviction or rebuild):
    /// a playlist already in the player's hands may still reference them, and
    /// a 404 on a LISTED part breaks the immutability contract — hls.js
    /// penalizes the "pathway", marks the fragment as a GAP, and spirals
    /// (observed live: 404 on part/10193/0 during a famine rebuild, then a
    /// fragGap retry storm). Retired segments stay servable until they age out
    /// of this small ring.
    retired: Mutex<VecDeque<Segment>>,
}

/// Hard kill switch: when true, no origin activates and the relays fall back to the
/// stable whole-segment path (plain playlist to hls.js with the relay's target-duration
/// rewrite + prefetch promotion). Global on purpose: it turns the feature off everywhere
/// at once.
///
/// Default ON (disabled), so the shipped default is the stable whole-segment path that
/// plays cleanly on every channel and hardware. The synthesized parts path (true
/// Twitch-like low latency) is opt-in at runtime via `set_disabled(false)`, driven by the
/// "experimental low latency" setting, so it can be tested and proven per machine before
/// it ever becomes the default.
static DISABLED: AtomicBool = AtomicBool::new(true);

fn http_client() -> Client {
    Client::builder()
        .tcp_keepalive(Duration::from_secs(15))
        .pool_idle_timeout(Duration::from_secs(30))
        // No overall timeout: the in-progress segment GET is intentionally long-lived
        // (it streams for ~the segment duration). Per-read progress is what matters.
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .build()
        .expect("ll_origin http client")
}

// ──────────────────────────── CMAF box chunker ────────────────────────────

/// Splits a CMAF byte stream into parts, one per `moof`+`mdat` pair. Any boxes
/// before the first `moof` (e.g. a leading `emsg`/`styp`) ride with the next part.
/// Feed bytes incrementally; each `push` returns whatever parts completed.
struct BoxChunker {
    buf: Vec<u8>,
    current: Vec<u8>,
}

impl BoxChunker {
    fn new() -> Self {
        Self {
            buf: Vec::new(),
            current: Vec::new(),
        }
    }

    fn push(&mut self, data: &[u8]) -> Vec<Vec<u8>> {
        self.buf.extend_from_slice(data);
        let mut parts = Vec::new();
        loop {
            if self.buf.len() < 8 {
                break;
            }
            let size32 = u32::from_be_bytes([self.buf[0], self.buf[1], self.buf[2], self.buf[3]]);
            let is_mdat = &self.buf[4..8] == b"mdat";
            let (box_len, header) = if size32 == 1 {
                if self.buf.len() < 16 {
                    break;
                }
                let large = u64::from_be_bytes(self.buf[8..16].try_into().unwrap()) as usize;
                (large, 16)
            } else {
                (size32 as usize, 8)
            };
            // size 0 ("to end of stream") or a corrupt tiny size: wait for flush().
            if box_len < header {
                break;
            }
            if self.buf.len() < box_len {
                break;
            }
            self.current.extend_from_slice(&self.buf[..box_len]);
            self.buf.drain(..box_len);
            if is_mdat {
                parts.push(std::mem::take(&mut self.current));
            }
        }
        parts
    }

    /// Stream ended: DROP any leftovers. A published part must be a complete
    /// `moof`+`mdat` group or nothing — a clean CMAF segment ends exactly at an
    /// mdat boundary (push() already emitted everything), so leftovers only
    /// exist when the transfer was ABANDONED mid-box. Appending a truncated
    /// box makes the browser run MSE's append-error algorithm, which ENDS the
    /// MediaSource outright, and a complete moof without its mdat leaves the
    /// SourceBuffer stuck in PARSING_MEDIA_SEGMENT (the timestampOffset
    /// error). The dropped tail is just the abandoned segment's lost media,
    /// which the playlist already accounts for.
    fn flush(&mut self) -> Option<Vec<u8>> {
        if !self.buf.is_empty() || !self.current.is_empty() {
            log::info!(
                "[LLOrigin] dropping {} leftover CMAF bytes (incomplete fragment at stream end)",
                self.buf.len() + self.current.len()
            );
        }
        self.buf.clear();
        self.current.clear();
        None
    }
}

// ──────────────────────── CMAF duration measurement ────────────────────────

/// Iterate top-level ISO-BMFF boxes in `data` as (fourcc, body) pairs.
fn iter_boxes(data: &[u8]) -> impl Iterator<Item = (&[u8], &[u8])> {
    let mut i = 0usize;
    std::iter::from_fn(move || {
        if i + 8 > data.len() {
            return None;
        }
        let size = u32::from_be_bytes(data[i..i + 4].try_into().unwrap()) as usize;
        if size < 8 || i + size > data.len() {
            return None;
        }
        let item = (&data[i + 4..i + 8], &data[i + 8..i + size]);
        i += size;
        Some(item)
    })
}

fn find_box<'a>(data: &'a [u8], fourcc: &[u8; 4]) -> Option<&'a [u8]> {
    iter_boxes(data).find(|(k, _)| *k == fourcc).map(|(_, b)| b)
}

/// Extract the VIDEO track's (track_id, timescale) from a CMAF init segment.
/// This is what converts a part's `trun` sample durations into seconds.
fn parse_cmaf_video_track(init: &[u8]) -> Option<(u32, u32)> {
    let moov = find_box(init, b"moov")?;
    for (kind, trak) in iter_boxes(moov) {
        if kind != b"trak" {
            continue;
        }
        let Some(mdia) = find_box(trak, b"mdia") else {
            continue;
        };
        let Some(hdlr) = find_box(mdia, b"hdlr") else {
            continue;
        };
        if hdlr.len() < 12 || &hdlr[8..12] != b"vide" {
            continue;
        }
        let tkhd = find_box(trak, b"tkhd")?;
        let track_id_off = if tkhd.first() == Some(&1) { 20 } else { 12 };
        let mdhd = find_box(mdia, b"mdhd")?;
        let timescale_off = if mdhd.first() == Some(&1) { 20 } else { 12 };
        if tkhd.len() < track_id_off + 4 || mdhd.len() < timescale_off + 4 {
            continue;
        }
        let track_id = u32::from_be_bytes(tkhd[track_id_off..track_id_off + 4].try_into().unwrap());
        let timescale =
            u32::from_be_bytes(mdhd[timescale_off..timescale_off + 4].try_into().unwrap());
        if timescale > 0 {
            return Some((track_id, timescale));
        }
    }
    None
}

/// Measure a CMAF part's video span in seconds from its own sample tables:
/// the sum of the video traf's `trun` sample durations (or the `tfhd` default
/// duration times the sample count), divided by the init's timescale.
fn cmaf_part_duration(part: &[u8], track_id: u32, timescale: u32) -> Option<f64> {
    let mut total_ticks: u64 = 0;
    for (kind, moof) in iter_boxes(part) {
        if kind != b"moof" {
            continue;
        }
        for (k2, traf) in iter_boxes(moof) {
            if k2 != b"traf" {
                continue;
            }
            let Some(tfhd) = find_box(traf, b"tfhd") else {
                continue;
            };
            if tfhd.len() < 8 {
                continue;
            }
            let flags = u32::from_be_bytes(tfhd[0..4].try_into().unwrap()) & 0x00FF_FFFF;
            if u32::from_be_bytes(tfhd[4..8].try_into().unwrap()) != track_id {
                continue;
            }
            // Optional tfhd fields, in flag order, after the track id.
            let mut off = 8usize;
            if flags & 0x1 != 0 {
                off += 8; // base_data_offset
            }
            if flags & 0x2 != 0 {
                off += 4; // sample_description_index
            }
            let default_dur = if flags & 0x8 != 0 && tfhd.len() >= off + 4 {
                Some(u32::from_be_bytes(tfhd[off..off + 4].try_into().unwrap()))
            } else {
                None
            };
            for (k3, trun) in iter_boxes(traf) {
                if k3 != b"trun" || trun.len() < 8 {
                    continue;
                }
                let tflags = u32::from_be_bytes(trun[0..4].try_into().unwrap()) & 0x00FF_FFFF;
                let count = u32::from_be_bytes(trun[4..8].try_into().unwrap());
                if tflags & 0x100 != 0 {
                    let mut p = 8usize;
                    if tflags & 0x1 != 0 {
                        p += 4; // data_offset
                    }
                    if tflags & 0x4 != 0 {
                        p += 4; // first_sample_flags
                    }
                    let per = [0x100u32, 0x200, 0x400, 0x800]
                        .iter()
                        .filter(|&&f| tflags & f != 0)
                        .count()
                        * 4;
                    for _ in 0..count {
                        if trun.len() < p + 4 {
                            break;
                        }
                        total_ticks +=
                            u32::from_be_bytes(trun[p..p + 4].try_into().unwrap()) as u64;
                        p += per;
                    }
                } else if let Some(d) = default_dur {
                    total_ticks += d as u64 * count as u64;
                }
            }
        }
    }
    if total_ticks == 0 {
        None
    } else {
        Some(total_ticks as f64 / timescale as f64)
    }
}

// ──────────────────────────── MPEG-TS chunker ────────────────────────────

const TS_PACKET: usize = 188;
const TS_SYNC: u8 = 0x47;

/// Splits an MPEG-TS byte stream into low-latency parts. Cuts only on 188-byte
/// packet boundaries (never mid-packet) and only at the start of a video PES
/// (payload-unit-start on a `0xE0..=0xEF` stream), so every part begins on an access
/// unit. A new part is cut once ~`TS_PART_PTS` of video has accumulated since the
/// part started. Because a Twitch TS segment leads with PAT/PMT and a keyframe, the
/// first part of each segment is the independently-decodable one (the playlist marks
/// only part 0 `INDEPENDENT=YES`).
///
/// Robustness: re-syncs on a lost `0x47`, tolerates partial trailing packets (held
/// until the next push), and ignores non-video PIDs for cut decisions (they ride
/// along in whichever part they fall in — valid TS, harmless to the demuxer).
struct TsChunker {
    buf: Vec<u8>,
    current: Vec<u8>,
    /// PTS (90 kHz) at which the in-progress part started; None until the first
    /// video PES of the part is seen.
    part_start_pts: Option<u64>,
    /// Most recent video PTS seen (used to estimate the final part's duration at flush,
    /// when there is no following PES to measure against).
    last_pts: Option<u64>,
}

/// 90 kHz ticks per second (MPEG-TS PTS clock).
const TS_CLOCK: f64 = 90_000.0;

impl TsChunker {
    fn new() -> Self {
        Self {
            buf: Vec::new(),
            current: Vec::new(),
            part_start_pts: None,
            last_pts: None,
        }
    }

    fn push(&mut self, data: &[u8]) -> Vec<(Vec<u8>, f64)> {
        self.buf.extend_from_slice(data);
        let mut parts = Vec::new();
        let mut i = 0;
        while i + TS_PACKET <= self.buf.len() {
            // Re-sync: a packet must begin with the sync byte. If not, scan forward
            // to the next 0x47 and realign (a dropped/garbled packet otherwise
            // poisons every subsequent boundary).
            if self.buf[i] != TS_SYNC {
                match self.buf[i + 1..].iter().position(|&b| b == TS_SYNC) {
                    Some(off) => {
                        i += 1 + off;
                        continue;
                    }
                    None => {
                        i = self.buf.len(); // no sync in the rest; drop it
                        break;
                    }
                }
            }
            let pkt = &self.buf[i..i + TS_PACKET];
            if let Some(pts) = video_pes_pts(pkt) {
                self.last_pts = Some(pts);
                // A video PES (access unit) starts here. Cut the previous part BEFORE
                // this packet once enough has accumulated SINCE THE PART'S FIRST PES,
                // so the new part starts on this access unit.
                //
                // The first video PES of a part NEVER triggers a cut (`None` ⇒ false):
                // a Twitch TS segment leads with PAT/PMT then the keyframe PES, so the
                // first PES must stay with that lead in part 0 — otherwise part 0 (which
                // the playlist marks INDEPENDENT=YES) would hold only PAT/PMT and the
                // keyframe would land in part 1, starting hls.js mid-GOP (garbage
                // frames / apparent stall).
                let should_cut = match self.part_start_pts {
                    Some(start) => {
                        // `wrapping_sub` guards a backwards PTS (rare splice/wrap): a
                        // huge delta (>= 2^32 ticks) is treated as not-yet-elapsed.
                        let e = pts.wrapping_sub(start);
                        (TS_PART_PTS..(1u64 << 32)).contains(&e)
                    }
                    None => false,
                };
                if !self.current.is_empty() && should_cut {
                    // The completed part spans [part_start_pts, pts): its real duration
                    // is that PTS delta. This is what makes the parts tile the segment.
                    let start = self.part_start_pts.unwrap_or(pts);
                    let dur = pts.wrapping_sub(start) as f64 / TS_CLOCK;
                    parts.push((std::mem::take(&mut self.current), dur));
                    self.part_start_pts = Some(pts);
                } else if self.part_start_pts.is_none() {
                    self.part_start_pts = Some(pts);
                }
            }
            self.current.extend_from_slice(pkt);
            i += TS_PACKET;
        }
        self.buf.drain(..i);
        parts
    }

    /// Stream ended: emit any accumulated packets as the final part. Its duration is
    /// estimated from the last video PTS (plus ~one 60 fps frame, since the part runs
    /// to the end of that last frame); falls back to the nominal when no PTS was seen.
    fn flush(&mut self) -> Option<(Vec<u8>, f64)> {
        if !self.buf.is_empty() {
            // A trailing partial packet is unusual but shouldn't be dropped; the
            // demuxer ignores a short final packet.
            let rest = std::mem::take(&mut self.buf);
            self.current.extend_from_slice(&rest);
        }
        let dur = match (self.part_start_pts, self.last_pts) {
            (Some(start), Some(last)) if last > start => {
                (last - start) as f64 / TS_CLOCK + (1.0 / 60.0)
            }
            _ => NOMINAL_PART_DUR,
        };
        self.part_start_pts = None;
        self.last_pts = None;
        if self.current.is_empty() {
            None
        } else {
            Some((std::mem::take(&mut self.current), dur))
        }
    }
}

/// If `pkt` (one 188-byte TS packet) carries the start of a video PES
/// (`0x000001` start code, stream_id `0xE0..=0xEF`) with a PTS, return the 33-bit
/// PTS. Returns None for continuation packets, non-video PIDs, and PES without PTS.
fn video_pes_pts(pkt: &[u8]) -> Option<u64> {
    if pkt.len() < TS_PACKET || pkt[0] != TS_SYNC {
        return None;
    }
    // payload_unit_start_indicator
    if pkt[1] & 0x40 == 0 {
        return None;
    }
    let adaptation = (pkt[3] >> 4) & 0x3;
    if adaptation == 0 || adaptation == 2 {
        return None; // no payload (adaptation-only/reserved)
    }
    let mut p = 4;
    if adaptation == 3 {
        let af_len = pkt[4] as usize;
        p = 5 + af_len;
    }
    // PES start code + stream_id
    if p + 9 > pkt.len() || pkt[p] != 0x00 || pkt[p + 1] != 0x00 || pkt[p + 2] != 0x01 {
        return None;
    }
    let stream_id = pkt[p + 3];
    if !(0xE0..=0xEF).contains(&stream_id) {
        return None; // not a video stream
    }
    let pts_dts_flags = (pkt[p + 7] >> 6) & 0x3;
    if pts_dts_flags == 0 {
        return None; // PES present but carries no PTS
    }
    let b = &pkt[p + 9..];
    if b.len() < 5 {
        return None;
    }
    // 33-bit PTS packed across 5 bytes with marker bits (ISO 13818-1).
    let pts = (((b[0] as u64 >> 1) & 0x07) << 30)
        | ((b[1] as u64) << 22)
        | (((b[2] as u64 >> 1) & 0x7F) << 15)
        | ((b[3] as u64) << 7)
        | ((b[4] as u64 >> 1) & 0x7F);
    Some(pts)
}

// ──────────────────────────── upstream playlist parse ────────────────────────────

struct Upstream {
    init_url: Option<String>,
    /// (sn, pdt, absolute url) for published segments.
    published: Vec<(u64, Option<String>, String)>,
    /// Absolute prefetch URLs, oldest first (the first is the actively-producing one).
    prefetch: Vec<String>,
}

fn base_of(url: &str) -> String {
    let no_q = url.split('?').next().unwrap_or(url);
    match no_q.rfind('/') {
        Some(i) => no_q[..=i].to_string(),
        None => String::new(),
    }
}

fn absolutize(uri: &str, base: &str) -> String {
    if uri.starts_with("http://") || uri.starts_with("https://") {
        uri.to_string()
    } else {
        format!("{base}{uri}")
    }
}

fn parse_upstream(text: &str, base: &str) -> Upstream {
    let mut up = Upstream {
        init_url: None,
        published: Vec::new(),
        prefetch: Vec::new(),
    };
    let mut sn: u64 = 0;
    let mut pending_pdt: Option<String> = None;
    let mut expect_uri = false;
    for line in text.lines() {
        let t = line.trim();
        if let Some(v) = t.strip_prefix("#EXT-X-MEDIA-SEQUENCE:") {
            sn = v.trim().parse().unwrap_or(0);
        } else if let Some(rest) = t.strip_prefix("#EXT-X-MAP:") {
            // URI="..."
            if let Some(uri) = extract_attr(rest, "URI") {
                up.init_url = Some(absolutize(&uri, base));
            }
        } else if let Some(v) = t.strip_prefix("#EXT-X-PROGRAM-DATE-TIME:") {
            pending_pdt = Some(v.trim().to_string());
        } else if let Some(url) = t.strip_prefix("#EXT-X-TWITCH-PREFETCH:") {
            up.prefetch.push(absolutize(url.trim(), base));
        } else if t.starts_with("#EXTINF:") {
            expect_uri = true;
        } else if expect_uri && !t.is_empty() && !t.starts_with('#') {
            expect_uri = false;
            up.published
                .push((sn, pending_pdt.take(), absolutize(t, base)));
            sn += 1;
        }
    }
    up
}

/// Extract `KEY="value"` from an attribute list fragment.
fn extract_attr(attrs: &str, key: &str) -> Option<String> {
    let needle = format!("{key}=\"");
    let start = attrs.find(&needle)? + needle.len();
    let rest = &attrs[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

// ──────────────────────────── lifecycle ────────────────────────────

pub fn set_disabled(disabled: bool) {
    DISABLED.store(disabled, Ordering::Relaxed);
}

impl LlOrigin {
    pub fn new(max_segments: usize) -> Arc<Self> {
        Arc::new(Self {
            live_edge: Mutex::new(None),
            notify: Notify::new(),
            edge_version: AtomicU64::new(0),
            reader_task: Mutex::new(None),
            generation: AtomicU64::new(0),
            max_segments,
            transmux: Mutex::new(None),
            cmaf_video: Mutex::new(None),
            retired: Mutex::new(VecDeque::new()),
        })
    }

    /// Move segments leaving the live window into the retirement ring.
    fn retire(&self, segs: impl IntoIterator<Item = Segment>) {
        const RETIRED_CAP: usize = 8;
        let mut r = self.retired.lock().unwrap();
        for s in segs {
            r.push_back(s);
        }
        while r.len() > RETIRED_CAP {
            r.pop_front();
        }
    }

    /// Mark the served playlist as changed and wake every blocked reload.
    fn wake_serves(&self) {
        self.edge_version.fetch_add(1, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    pub fn is_active(&self) -> bool {
        self.live_edge.lock().unwrap().is_some()
    }

    /// The container of the active edge (defaults to CMAF when inactive; callers
    /// only consult it while streaming a known-active edge).
    fn container(&self) -> Container {
        self.live_edge
            .lock()
            .unwrap()
            .as_ref()
            .map(|e| e.container)
            .unwrap_or(Container::Cmaf)
    }

    /// Deactivate the live edge because the upstream is permanently gone (stream
    /// offline, or the usher-signed playlist/segment URLs expired after hours).
    /// Unlike `stop()` this does NOT bump the generation or abort the reader (the
    /// reader is the caller and returns right after): it just clears the edge so
    /// `is_active()` goes false, blocking holds release with `None`, and the relay
    /// falls through to the plain upstream path — which 4xx's into a fatal hls.js
    /// error (offline UI) on true death, or a graceful non-LL serve if the upstream
    /// recovered. Without this the reader spun forever serving the last-good
    /// playlist while `edge_version` never advanced (a silent permanent freeze).
    fn deactivate_offline(&self, reason: &str) {
        warn!("[LLOrigin] deactivating origin (upstream gone): {reason}");
        crate::services::ll_diagnostics::event(&format!(
            "\"ev\":\"o_offline\",\"reason\":{reason:?}"
        ));
        *self.live_edge.lock().unwrap() = None;
        *self.transmux.lock().unwrap() = None;
        *self.cmaf_video.lock().unwrap() = None;
        self.wake_serves();
    }

    /// Stop any running reader and clear state. Called on stream stop / restart.
    pub fn stop(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        if let Some(h) = self.reader_task.lock().unwrap().take() {
            h.abort();
        }
        *self.live_edge.lock().unwrap() = None;
        *self.transmux.lock().unwrap() = None;
        *self.cmaf_video.lock().unwrap() = None;
        self.retired.lock().unwrap().clear();
        self.wake_serves();
    }

    /// Probe the upstream and, if it's a low-latency broadcast we can serve, build the
    /// initial live edge (backfilling a couple of complete segments) and spawn the
    /// streaming reader. Returns `{active, has_prefetch}`: `active` when the origin took
    /// over (LL channel), `has_prefetch` whenever the upstream carries PREFETCH hints
    /// (so the relay can still flag a low-latency channel for the promotion fallback even
    /// when the origin declined). Awaited from stream start so the player can read the
    /// result before constructing hls.js.
    pub async fn start(self: Arc<Self>, upstream_playlist_url: String) -> StartOutcome {
        self.stop();
        // The experimental-low-latency kill switch (`DISABLED`, default on) gates the
        // TS parts path, which has a clean fallback (the stable whole-segment path
        // plays H.264/TS fine). CMAF (HEVC/AV1) has no working fallback: the stable
        // path promotes the in-progress prefetch hint as a whole segment, but a
        // progressive fMP4 fragment is incomplete and trips an MSE append error, so the
        // stream won't play at all. So we probe first, then honor the switch for TS only
        // and always let CMAF through (native passthrough, the proven path). This is
        // what lets a region-unlocked 1440p tier play without the user turning on the
        // setting.
        let force_disabled = DISABLED.load(Ordering::Relaxed);
        let gen = self.generation.load(Ordering::SeqCst);
        let client = http_client();

        let text = match client
            .get(&upstream_playlist_url)
            .timeout(FETCH_TIMEOUT)
            .send()
            .await
        {
            Ok(r) => r.text().await.unwrap_or_default(),
            Err(e) => {
                warn!("[LLOrigin] initial playlist fetch failed: {e}");
                return StartOutcome::default();
            }
        };
        let base = base_of(&upstream_playlist_url);
        let up = parse_upstream(&text, &base);
        let has_prefetch = !up.prefetch.is_empty();
        let inactive = StartOutcome {
            active: false,
            has_prefetch,
        };

        // Not a low-latency broadcast (no prefetch hints): leave the origin inactive so
        // the relay uses the stable whole-segment path.
        if up.prefetch.is_empty() || up.published.is_empty() {
            debug!("[LLOrigin] not a low-latency stream (no prefetch); origin inactive");
            return inactive;
        }

        // Pick the container from the upstream shape: `#EXT-X-MAP` ⇒ CMAF (fMP4),
        // otherwise MPEG-TS. TS is the H.264 (source/"chunked") case — the common one.
        let (container, init_url) = match up.init_url.clone() {
            // CMAF (HEVC/AV1): the origin is the only path that serves it cleanly, so it
            // runs even when the experimental switch is on. Native fMP4 passthrough.
            Some(u) => (Container::Cmaf, u),
            None => {
                // MPEG-TS (H.264): the stable whole-segment path serves this fine, so
                // the TS parts path stays opt-in behind the experimental switch (and its
                // own build-time sign-off). Return the pre-probe default so the common
                // case's latency cushion is byte-for-byte what it was before this probe.
                if force_disabled {
                    return StartOutcome::default();
                }
                if !ENABLE_TS_LL_ORIGIN {
                    debug!(
                        "[LLOrigin] low-latency MPEG-TS stream; TS origin disabled, using fallback"
                    );
                    return inactive;
                }
                (Container::Ts, String::new())
            }
        };

        // The transmuxer must exist before backfill so the backfilled segments
        // are already fMP4 and the init segment is ready before the first
        // playlist render (hls.js fetches `#EXT-X-MAP` before anything else).
        *self.transmux.lock().unwrap() = if container == Container::Ts && ENABLE_TS_TRANSMUX {
            Some(crate::services::ts_fmp4::Transmuxer::new())
        } else {
            None
        };

        // CMAF path: fetch the init once for the video track's timescale, so
        // every published part carries its measured duration (must happen
        // before backfill — backfilled parts get durations too).
        *self.cmaf_video.lock().unwrap() = None;
        if container == Container::Cmaf && !init_url.is_empty() {
            match client.get(&init_url).timeout(FETCH_TIMEOUT).send().await {
                Ok(resp) => {
                    let bytes = resp.bytes().await.map(|b| b.to_vec()).unwrap_or_default();
                    let parsed = parse_cmaf_video_track(&bytes);
                    if parsed.is_none() {
                        warn!("[LLOrigin] CMAF init parse failed; parts fall back to nominal durations");
                    }
                    *self.cmaf_video.lock().unwrap() = parsed;
                }
                Err(e) => {
                    warn!("[LLOrigin] CMAF init fetch failed ({e}); parts fall back to nominal durations");
                }
            }
        }

        // Backfill the last few complete segments so the first playlist has #EXTINF.
        let mut segments: VecDeque<Segment> = VecDeque::new();
        let backfill: Vec<_> = up
            .published
            .iter()
            .rev()
            .take(BACKFILL_SEGMENTS)
            .rev()
            .cloned()
            .collect();
        for (sn, pdt, url) in backfill {
            match client.get(&url).timeout(FETCH_TIMEOUT).send().await.ok() {
                Some(resp) => {
                    let bytes = resp.bytes().await.map(|b| b.to_vec()).unwrap_or_default();
                    let parts = self.convert_parts(split_complete(&bytes, container));
                    if !parts.is_empty() {
                        segments.push_back(make_segment(sn, pdt, true, parts));
                    }
                }
                None => continue,
            }
        }
        if segments.is_empty() {
            warn!("[LLOrigin] backfill produced no complete segments; origin inactive");
            return inactive;
        }

        // With the transmux on, the init segment must exist before going live; a
        // stream whose backfill never showed an SPS/PPS can't be presented as
        // CMAF, so decline activation (the promotion fallback serves it).
        let init_bytes = {
            let mut g = self.transmux.lock().unwrap();
            match g.as_mut() {
                Some(t) => match t.init_segment() {
                    Some(b) => Some(Arc::new(b)),
                    None => {
                        warn!("[LLOrigin] transmux found no H.264 config in backfill; origin inactive");
                        *g = None;
                        return inactive;
                    }
                },
                None => None,
            }
        };

        *self.live_edge.lock().unwrap() = Some(LiveEdge {
            init_url,
            init_bytes,
            container,
            // Declare the real ~2s segment size, NOT Twitch's inflated TARGETDURATION:6.
            // hls.js uses targetduration for reload cadence and tune-in goal math; an
            // inflated value makes it mis-time part requests.
            target_duration: TARGET_DURATION,
            part_target: PART_TARGET,
            segments,
        });
        // Query string stripped: it is most of the length and can carry signed
        // playback params that have no business in a log people send around.
        info!(
            "[LLOrigin] activated ({container:?} low-latency origin) for {}",
            upstream_playlist_url.split('?').next().unwrap_or("")
        );

        let handle = tokio::spawn(run_reader(self.clone(), upstream_playlist_url, client, gen));
        *self.reader_task.lock().unwrap() = Some(handle);
        StartOutcome {
            active: true,
            has_prefetch: true,
        }
    }
}

fn make_segment(
    sn: u64,
    pdt: Option<String>,
    complete: bool,
    part_list: Vec<(Vec<u8>, f64)>,
) -> Segment {
    let total: f64 = part_list.iter().map(|(_, d)| d).sum();
    let parts = part_list
        .into_iter()
        .map(|(b, duration)| Part {
            duration,
            bytes: Arc::new(b),
        })
        .collect();
    Segment {
        sn,
        pdt,
        complete,
        // Sample-measured parts sum to the real span; the raw path's even split
        // sums to exactly TARGET_DURATION (same value as before).
        duration: if total > 0.0 {
            total
        } else {
            TARGET_DURATION as f64
        },
        parts,
        assembled: None,
    }
}

/// Split a fully-downloaded segment into parts (CMAF: one per moof+mdat; TS: runs of
/// packets cut at video-PES boundaries).
fn split_complete(bytes: &[u8], container: Container) -> Vec<Vec<u8>> {
    let mut chunker = Chunker::new(container);
    let mut parts: Vec<Vec<u8>> = chunker.push(bytes).into_iter().map(|(b, _)| b).collect();
    if let Some((tail, _)) = chunker.flush() {
        parts.push(tail);
    }
    // Chunker durations are dropped here: `convert_parts` pairs each part with
    // its real sample-measured duration (transmux path) or an even
    // TARGET_DURATION/count split (raw path), summing to the segment either way.
    parts
}

// ──────────────────────────── the streaming reader ────────────────────────────

async fn run_reader(
    origin: Arc<LlOrigin>,
    upstream_playlist_url: String,
    client: Client,
    gen: u64,
) {
    let base = base_of(&upstream_playlist_url);
    // Connection to the NEXT in-progress segment, opened by `preopen_next` while
    // the current one was still streaming. Consumed by the fast path below;
    // discarded whenever the world doesn't match (the poll path then re-syncs).
    let mut preopened: Option<(u64, JoinHandle<Option<(Response, Option<String>, String)>>)> = None;
    // Consecutive failing playlist polls; reset on any good poll. Drives offline
    // deactivation so a permanently-dead upstream stops being served forever.
    let mut consec_playlist_fail: u32 = 0;
    loop {
        if gen != origin.generation.load(Ordering::SeqCst) {
            return;
        }

        // Fast path: stream the preopened next segment. No playlist round trip, no
        // time-to-first-byte, no waiting for Twitch to publish the previous segment.
        // Without this the origin publishes NOTHING for the poll + publish-lag +
        // TTFB at every segment boundary (~1-2.5s), which is most of a 2s cushion:
        // the player drains right as it reaches the live edge and stalls.
        if let Some((sn, mut handle)) = preopened.take() {
            let contiguous = {
                let g = origin.live_edge.lock().unwrap();
                match g.as_ref() {
                    Some(e) => e.segments.back().is_some_and(|s| s.sn + 1 == sn),
                    None => return,
                }
            };
            if contiguous {
                match tokio::time::timeout(PREOPEN_WAIT, &mut handle).await {
                    Ok(Ok(Some((resp, pdt, seg_url)))) => {
                        match origin.push_shell(sn, pdt.clone()) {
                            Some(true) => {
                                crate::services::ll_diagnostics::event(&format!(
                                    "\"ev\":\"o_shell\",\"path\":\"fast\",\"sn\":{sn},\"pdt\":{}",
                                    pdt.as_deref()
                                        .map(|p| format!("\"{p}\""))
                                        .unwrap_or_else(|| "null".into())
                                ));
                                origin.wake_serves();
                                let next = tokio::spawn(preopen_next(
                                    client.clone(),
                                    upstream_playlist_url.clone(),
                                    sn + 1,
                                ));
                                stream_response(
                                    &origin,
                                    resp,
                                    sn,
                                    gen,
                                    Some((&client, seg_url.as_str())),
                                )
                                .await;
                                if !origin.finish_segment(sn) {
                                    return;
                                }
                                crate::services::ll_diagnostics::event(&format!(
                                    "\"ev\":\"o_finish\",\"path\":\"fast\",\"sn\":{sn}"
                                ));
                                origin.wake_serves();
                                preopened = Some((sn + 1, next));
                                continue;
                            }
                            Some(false) => {} // refused: the poll path re-syncs
                            None => return,
                        }
                    }
                    Ok(_) => {} // preopen failed (no hint / ad window): poll path
                    Err(_) => handle.abort(), // not ready in time: poll path
                }
            }
        }

        // Re-fetch the playlist to find the current in-progress segment. reqwest
        // returns Ok for any HTTP status, so a 403/404 (stream offline, or an
        // expired usher-signed URL) would otherwise parse to an empty playlist and
        // loop forever; inspect the status and count failures toward deactivation.
        let text = match client
            .get(&upstream_playlist_url)
            .timeout(FETCH_TIMEOUT)
            .send()
            .await
        {
            Ok(r) => {
                let status = r.status();
                if !status.is_success() {
                    consec_playlist_fail += 1;
                    warn!("[LLOrigin] reader playlist HTTP {status} ({consec_playlist_fail})");
                    if consec_playlist_fail >= PLAYLIST_OFFLINE_LIMIT {
                        origin.deactivate_offline(&format!("playlist HTTP {status}"));
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    continue;
                }
                r.text().await.unwrap_or_default()
            }
            Err(e) => {
                consec_playlist_fail += 1;
                warn!("[LLOrigin] reader playlist fetch failed: {e} ({consec_playlist_fail})");
                if consec_playlist_fail >= PLAYLIST_OFFLINE_LIMIT {
                    origin.deactivate_offline("playlist fetch repeatedly failing");
                    return;
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
                continue;
            }
        };
        let up = parse_upstream(&text, &base);
        let last_published_sn = match up.published.last() {
            Some((sn, _, _)) => {
                consec_playlist_fail = 0; // a healthy poll
                *sn
            }
            None => {
                consec_playlist_fail += 1;
                if consec_playlist_fail >= PLAYLIST_OFFLINE_LIMIT {
                    origin.deactivate_offline("playlist has no segments");
                    return;
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
                continue;
            }
        };
        let inprogress_sn = last_published_sn + 1;
        // CMAF passthrough: keep the served `#EXT-X-MAP` URI in sync with the
        // upstream. The init URL is captured once at activation and hls.js caches
        // the init bytes keyed by that URL, so if the broadcaster's encoder
        // reconfigures mid-stream (SSAI ad splice with different parameter sets, a
        // codec reset) Twitch publishes a NEW init URL while our frozen one keeps
        // hls.js decoding the new bitstream against stale config (corrupt frames or
        // a MediaSource append error). Writing the new URL through makes hls.js see
        // a new MAP URI and re-fetch. TS-transmux serves its own immutable
        // `init.mp4` (avc3 carries parameter-set changes in-band), so this only
        // applies to native CMAF passthrough (`init_bytes` is None there).
        let init_changed = {
            let mut g = origin.live_edge.lock().unwrap();
            match g.as_mut() {
                Some(edge) => match init_url_update(edge, up.init_url.as_deref()) {
                    Some(new_init) => {
                        warn!("[LLOrigin] upstream init segment changed; updating #EXT-X-MAP to force re-fetch");
                        edge.init_url = new_init;
                        true
                    }
                    None => false,
                },
                None => return,
            }
        };
        if init_changed {
            origin.wake_serves();
        }
        let inprogress_url = match up.prefetch.first() {
            Some(u) => u.clone(),
            None => {
                // Lost low-latency hints (e.g. an ad window): step back and retry.
                tokio::time::sleep(Duration::from_millis(500)).await;
                continue;
            }
        };
        // The in-progress segment's wall-clock start is the LAST PUBLISHED segment's
        // PROGRAM-DATE-TIME plus one segment duration. Reusing the published PDT
        // verbatim would anchor the freshest content a full segment in the past and
        // skew any PDT-derived latency readout by ~2s. Re-derived from upstream on
        // every poll, so the nominal-vs-real duration error never accumulates. If the
        // timestamp doesn't parse the tag is omitted; hls.js extrapolates a missing
        // PDT from the previous segment, which is exactly right.
        let pdt = up
            .published
            .last()
            .and_then(|(_, p, _)| p.as_deref())
            .and_then(advance_pdt);

        // Skip if we've already ingested this sn.
        let already = {
            let g = origin.live_edge.lock().unwrap();
            g.as_ref()
                .map(|e| e.segments.iter().any(|s| s.sn >= inprogress_sn))
                .unwrap_or(true) // gone -> stop
        };
        if origin.live_edge.lock().unwrap().is_none() {
            return;
        }
        if already {
            tokio::time::sleep(Duration::from_millis(250)).await;
            continue;
        }

        // Bring the window up to date BEFORE opening the in-progress stream. The
        // rendered window must stay CONTIGUOUS: hls.js numbers segments by POSITION
        // from #EXT-X-MEDIA-SEQUENCE, so a hole shifts every later segment's number
        // away from its `seg/<sn>.ts` URI, and as the window slides the same URI
        // changes number across refreshes, which hls.js rejects as a fatal
        // "media sequence mismatch", freezing playback. A hole opens
        // whenever a segment finalizes outside the reader's sight: most commonly one
        // finalizing between the activation backfill and the first poll here (a
        // segment boundary falls inside that window on most stream starts), or any
        // poll/read hiccup that makes the reader skip ahead.
        let newest_in_window = {
            let g = origin.live_edge.lock().unwrap();
            match g.as_ref() {
                Some(e) => e.segments.back().map(|s| s.sn).unwrap_or(0),
                None => return,
            }
        };
        if newest_in_window + 1 < inprogress_sn {
            let oldest_published = match up.published.first() {
                Some((sn, _, _)) => *sn,
                None => unreachable!("published is non-empty (checked above)"),
            };
            let (rebuild, fetch) = plan_catch_up(newest_in_window, oldest_published, inprogress_sn);
            // A rebuild jumps over segments the transmuxer never saw; completing
            // its half-assembled pre-gap PES with post-gap bytes would emit one
            // garbage sample, so drop that state before converting anything.
            if rebuild {
                if let Some(t) = origin.transmux.lock().unwrap().as_mut() {
                    t.reset_assembly();
                }
            }
            // Fetch every missing segment BEFORE touching the window, so a blocking
            // reload can never observe an empty or partially rebuilt playlist.
            let mut fetched: Vec<Segment> = Vec::new();
            let mut filled = true;
            for sn in fetch {
                let found = up.published.iter().find(|(s, _, _)| *s == sn);
                let Some((_, seg_pdt, url)) = found else {
                    filled = false;
                    break;
                };
                match fetch_published(&client, sn, seg_pdt.clone(), url, &origin).await {
                    Some(seg) => fetched.push(seg),
                    None => {
                        filled = false;
                        break;
                    }
                }
            }
            if !filled {
                // A catch-up fetch failed; rendering a hole would freeze the player,
                // so retry the whole poll shortly instead.
                tokio::time::sleep(Duration::from_millis(300)).await;
                continue;
            }
            {
                let mut g = origin.live_edge.lock().unwrap();
                match g.as_mut() {
                    Some(edge) => {
                        if rebuild {
                            // The hole can't be filled adjacently (it predates the
                            // upstream window, or is deeper than ours): swap in a
                            // fresh backfill from the live edge. The MEDIA-SEQUENCE
                            // jump is a legal sliding-window advance; hls.js
                            // re-anchors via PROGRAM-DATE-TIME.
                            warn!(
                                "[LLOrigin] window resync: newest held {newest_in_window}, upstream starts at {oldest_published}, in-progress {inprogress_sn}"
                            );
                            crate::services::ll_diagnostics::event(&format!(
                                "\"ev\":\"o_rebuild\",\"newest\":{newest_in_window},\"oldest\":{oldest_published},\"inprogress\":{inprogress_sn}"
                            ));
                            let old: Vec<Segment> = edge.segments.drain(..).collect();
                            origin.retire(old);
                        } else if !fetched.is_empty() {
                            crate::services::ll_diagnostics::event(&format!(
                                "\"ev\":\"o_catchup\",\"n\":{},\"upto\":{inprogress_sn}",
                                fetched.len()
                            ));
                        }
                        edge.segments.extend(fetched);
                        while edge.segments.len() > origin.max_segments {
                            if let Some(s) = edge.segments.pop_front() {
                                origin.retire([s]);
                            }
                        }
                    }
                    None => return,
                }
            }
            origin.wake_serves();
        }

        // Create the in-progress segment shell and stream its parts in, preopening
        // the following segment's connection so the next boundary is seamless.
        match origin.push_shell(inprogress_sn, pdt.clone()) {
            Some(true) => {}
            Some(false) => continue,
            None => return,
        }
        crate::services::ll_diagnostics::event(&format!(
            "\"ev\":\"o_shell\",\"path\":\"poll\",\"sn\":{inprogress_sn},\"pdt\":{}",
            pdt.as_deref()
                .map(|p| format!("\"{p}\""))
                .unwrap_or_else(|| "null".into())
        ));
        origin.wake_serves();

        let next = tokio::spawn(preopen_next(
            client.clone(),
            upstream_playlist_url.clone(),
            inprogress_sn + 1,
        ));
        stream_segment(&origin, &client, &inprogress_url, inprogress_sn, gen).await;

        if !origin.finish_segment(inprogress_sn) {
            return;
        }
        crate::services::ll_diagnostics::event(&format!(
            "\"ev\":\"o_finish\",\"path\":\"poll\",\"sn\":{inprogress_sn}"
        ));
        origin.wake_serves();
        preopened = Some((inprogress_sn + 1, next));
    }
}

/// While one in-progress segment streams, open the connection for the NEXT one.
/// Twitch advertises it as a later PREFETCH hint, and the CDN holds the request
/// until that segment starts producing, so by the time the previous segment ends
/// the response is ready to read with zero ramp-up. Also returns the
/// upstream-derived PROGRAM-DATE-TIME for `next_sn`, so the reader's fast path
/// re-anchors to the playlist every segment instead of compounding
/// nominal-duration drift.
async fn preopen_next(
    client: Client,
    playlist_url: String,
    next_sn: u64,
) -> Option<(Response, Option<String>, String)> {
    let base = base_of(&playlist_url);
    for _ in 0..6 {
        if let Ok(r) = client
            .get(&playlist_url)
            .timeout(FETCH_TIMEOUT)
            .send()
            .await
        {
            let text = r.text().await.unwrap_or_default();
            let up = parse_upstream(&text, &base);
            if let Some((last_sn, last_pdt, _)) = up.published.last() {
                // Hints are consecutive after the last published segment, so the
                // hint index for next_sn follows from their sn distance.
                let Some(idx) = next_sn.checked_sub(last_sn + 1) else {
                    // Already published: the poll path fetches it whole instead.
                    return None;
                };
                if let Some(url) = up.prefetch.get(idx as usize) {
                    let steps = (next_sn - last_sn) as i64;
                    let pdt = last_pdt.as_deref().and_then(|p| advance_pdt_by(p, steps));
                    let resp = client.get(url).send().await.ok()?;
                    return Some((resp, pdt, url.clone()));
                }
                // Hint not advertised yet: poll again shortly.
            }
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    None
}

/// Advance an RFC3339 PROGRAM-DATE-TIME by one nominal segment duration.
fn advance_pdt(pdt: &str) -> Option<String> {
    advance_pdt_by(pdt, 1)
}

/// Advance an RFC3339 PROGRAM-DATE-TIME by `steps` nominal segment durations.
fn advance_pdt_by(pdt: &str, steps: i64) -> Option<String> {
    let t = chrono::DateTime::parse_from_rfc3339(pdt).ok()?;
    Some(
        (t + chrono::Duration::seconds(TARGET_DURATION as i64 * steps))
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
    )
}

/// Decide how the reader brings a stale window up to date before the next
/// in-progress segment. Returns `(clear_window_first, sns_to_fetch)`. The fetch
/// range is always consecutive and ends at `inprogress_sn - 1`, and when not
/// rebuilding it starts right after `window_newest`, so appending the fetched
/// segments keeps the window contiguous at every intermediate render.
/// How many missing segments the reader will fill ADJACENTLY before declaring
/// a rebuild instead. Catch-up fetches are serial whole-segment downloads; on
/// a connection delivering near the stream bitrate each one costs about a
/// segment of real time, so a deep fill can never gain ground: it downloads one
/// segment per segment produced and stands still.
/// One or two segments covers the startup race and a single hiccup;
/// beyond that, jumping to the live edge keeps the stream playable and the
/// player re-anchors via the media-sequence advance.
const CATCH_UP_MAX_SEGMENTS: u64 = 2;
/// Rebuild backfill: one complete segment before the in-progress one. Smaller
/// than the activation backfill on purpose — a mid-session rebuild happens
/// while the player is already starving, and every fetched segment delays the
/// first fresh part by up to a segment of transfer time. One complete segment
/// plus the in-progress shell is enough for the playlist contract (the lone
/// complete segment gets its EXTINF from having the shell as successor).
const REBUILD_BACKFILL: u64 = 1;
/// Consecutive failing playlist polls (HTTP 4xx/5xx, fetch error, or a body that
/// parses to zero published segments) before the upstream is judged permanently
/// gone and the origin deactivates. Each failing poll sleeps ~500ms, so this is
/// ~6s of sustained failure — far longer than any transient DNS/network blip or
/// ad window (ad segments still publish, so an ad never zeroes `published`),
/// short enough that a truly offline stream surfaces quickly instead of freezing
/// forever. A single good poll resets the count.
const PLAYLIST_OFFLINE_LIMIT: u32 = 12;

/// CMAF passthrough only: the new `#EXT-X-MAP` URI to adopt when the upstream's
/// init segment changed (an encoder/parameter-set reconfig publishes a fresh init
/// URL), else `None`. Returns `None` for the TS-transmux path (`init_bytes` is
/// `Some`; its own `init.mp4` is immutable and avc3 carries parameter-set changes
/// in-band), when the upstream advertises no init, and when nothing changed.
fn init_url_update(edge: &LiveEdge, upstream_init: Option<&str>) -> Option<String> {
    let new = upstream_init?;
    if edge.init_bytes.is_none() && !edge.init_url.is_empty() && edge.init_url != new {
        Some(new.to_string())
    } else {
        None
    }
}

fn plan_catch_up(
    window_newest: u64,
    oldest_published: u64,
    inprogress_sn: u64,
) -> (bool, std::ops::Range<u64>) {
    let first_missing = window_newest + 1;
    let gap = inprogress_sn.saturating_sub(first_missing);
    if first_missing < oldest_published || gap > CATCH_UP_MAX_SEGMENTS {
        // The hole predates the upstream window, or is too deep to fill while
        // gaining ground: rebuild from the live edge with a minimal backfill.
        let start = inprogress_sn
            .saturating_sub(REBUILD_BACKFILL)
            .max(oldest_published);
        (true, start..inprogress_sn)
    } else {
        (false, first_missing..inprogress_sn)
    }
}

/// Fetch a published segment whole and build its complete window entry.
/// Returns None on any failure (caller retries the poll).
async fn fetch_published(
    client: &Client,
    sn: u64,
    pdt: Option<String>,
    url: &str,
    origin: &LlOrigin,
) -> Option<Segment> {
    let resp = match client.get(url).timeout(FETCH_TIMEOUT).send().await {
        Ok(r) => r,
        Err(e) => {
            warn!("[LLOrigin] catch-up fetch failed for sn {sn}: {e}");
            return None;
        }
    };
    let bytes = match resp.bytes().await {
        Ok(b) => b.to_vec(),
        Err(e) => {
            warn!("[LLOrigin] catch-up body read failed for sn {sn}: {e}");
            return None;
        }
    };
    let parts = origin.convert_parts(split_complete(&bytes, origin.container()));
    if parts.is_empty() {
        warn!("[LLOrigin] catch-up segment sn {sn} produced no parts");
        return None;
    }
    Some(make_segment(sn, pdt, true, parts))
}

/// Stream one in-progress segment, publishing each completed moof+mdat as a part.
async fn stream_segment(origin: &LlOrigin, client: &Client, url: &str, sn: u64, gen: u64) {
    let resp = match client.get(url).send().await {
        Ok(r) => r,
        Err(e) => {
            warn!("[LLOrigin] in-progress GET failed for sn {sn}: {e}");
            return;
        }
    };
    stream_response(origin, resp, sn, gen, Some((client, url))).await;
}

/// Read an already-open in-progress response, publishing parts as chunks land.
///
/// On a mid-transfer stall (silence or read error), ONE byte-exact resume is
/// attempted on a fresh ranged connection before the tail is abandoned: the
/// stall is usually that connection's problem while the segment keeps growing
/// on the CDN, and a `Range: bytes=<received>-` pickup continues the byte
/// stream seamlessly through the SAME chunker and demux state — no media lost,
/// no bridge needed. `resume` carries the client and segment URL when known.
async fn stream_response(
    origin: &LlOrigin,
    first: Response,
    sn: u64,
    gen: u64,
    resume: Option<(&Client, &str)>,
) {
    let mut chunker = Chunker::new(origin.container());
    // True when the body ended cleanly; a read error or timeout means the tail
    // bytes of this segment never arrived.
    let mut completed = false;
    // First chunk gets the long leash (CDN holds the request until the segment
    // starts); after that, silence means a stalled transfer.
    let mut first_chunk_at: Option<tokio::time::Instant> = None;
    // A trickling transfer never trips the silence timeout, so it also gets a
    // wall-clock deadline: the segment's own duration plus grace. The deadline
    // spans resumes too — total wall time is what starves the player.
    let deadline_secs = origin.expected_segment_secs() + SEGMENT_DEADLINE_GRACE;
    let mut received: u64 = 0;
    let mut resp = first;
    let mut resume_left = resume.is_some();
    loop {
        if gen != origin.generation.load(Ordering::SeqCst) {
            return;
        }
        let limit = match first_chunk_at {
            None => FIRST_CHUNK_TIMEOUT,
            Some(t0) => {
                let remaining = (deadline_secs - t0.elapsed().as_secs_f64()).max(0.0);
                if remaining <= 0.0 {
                    warn!("[LLOrigin] sn {sn} exceeded its delivery deadline; abandoning the tail");
                    break;
                }
                MIDSTREAM_CHUNK_TIMEOUT.min(Duration::from_secs_f64(remaining))
            }
        };
        match tokio::time::timeout(limit, resp.chunk()).await {
            Ok(Ok(Some(bytes))) => {
                first_chunk_at.get_or_insert_with(tokio::time::Instant::now);
                received += bytes.len() as u64;
                for (part, dur) in chunker.push(&bytes) {
                    let Some((part, dur)) = origin.convert_part(part, dur) else {
                        continue;
                    };
                    if !origin.append_part(sn, part, dur) {
                        return; // edge gone
                    }
                    origin.wake_serves();
                }
            }
            Ok(Ok(None)) => {
                completed = true;
                break;
            }
            outcome => {
                let why = match &outcome {
                    Ok(Err(e)) => format!("read error: {e}"),
                    _ => "read timed out".to_string(),
                };
                if resume_left {
                    resume_left = false;
                    if let Some((client, url)) = resume {
                        if let Some(r2) = range_resume(client, url, received, sn, &why).await {
                            resp = r2;
                            continue;
                        }
                    }
                }
                warn!("[LLOrigin] in-progress {why} for sn {sn}; abandoning the tail");
                break;
            }
        }
    }
    if let Some((tail, dur)) = chunker.flush() {
        if let Some((tail, dur)) = origin.convert_part(tail, dur) {
            if origin.append_part(sn, tail, dur) {
                origin.wake_serves();
            }
        }
    }
    if !completed {
        // The segment's tail is missing: a PES left half-assembled in the
        // transmuxer would be completed by the NEXT segment's unrelated bytes
        // and emit one garbage sample. Drop the partial state — but KEEP the
        // timeline expectations, so the lost media is bridged into a seamless
        // output instead of a served hole (the playlist stays sn-contiguous
        // and cannot express the gap).
        if let Some(t) = origin.transmux.lock().unwrap().as_mut() {
            t.drop_partial_input();
        }
    }
}

/// Byte-exact pickup of a stalled segment transfer on a fresh connection.
/// Only a 206 continues the stream; a 200 would restart the body (duplicate
/// bytes) and anything else is a refusal — both fall back to abandoning.
async fn range_resume(
    client: &Client,
    url: &str,
    received: u64,
    sn: u64,
    why: &str,
) -> Option<Response> {
    let resp = client
        .get(url)
        .header(reqwest::header::RANGE, format!("bytes={received}-"))
        .timeout(Duration::from_millis(1500))
        .send()
        .await
        .ok()?;
    if resp.status() == reqwest::StatusCode::PARTIAL_CONTENT {
        info!("[LLOrigin] sn {sn}: {why} at byte {received}; resumed on a fresh ranged connection");
        crate::services::ll_diagnostics::event(&format!(
            "\"ev\":\"o_resume\",\"sn\":{sn},\"at\":{received}"
        ));
        Some(resp)
    } else {
        warn!(
            "[LLOrigin] sn {sn}: range resume refused ({})",
            resp.status()
        );
        None
    }
}

impl LlOrigin {
    /// Append the in-progress segment shell. Returns `None` if the edge is gone,
    /// `Some(false)` if appending would break window contiguity (refused; the
    /// reader's catch-up should make that impossible), `Some(true)` on success.
    fn push_shell(&self, sn: u64, pdt: Option<String>) -> Option<bool> {
        let mut g = self.live_edge.lock().unwrap();
        let edge = g.as_mut()?;
        if edge.segments.back().is_some_and(|s| s.sn + 1 != sn) {
            warn!("[LLOrigin] refusing non-contiguous shell sn {sn}");
            return Some(false);
        }
        edge.segments.push_back(Segment {
            sn,
            pdt,
            complete: false,
            duration: TARGET_DURATION as f64,
            parts: Vec::new(),
            assembled: None,
        });
        while edge.segments.len() > self.max_segments {
            if let Some(s) = edge.segments.pop_front() {
                self.retire([s]);
            }
        }
        Some(true)
    }

    /// Mark `sn` complete and set its `#EXTINF` to the SUM of its (immutable) part
    /// durations, so the parts tile the segment exactly without ever rewriting a
    /// published part's duration.
    ///
    /// Parts MUST be immutable once served: a part whose listed duration changes
    /// across a playlist refresh is treated by hls.js as a new part and RE-FETCHED,
    /// and re-appending an identical TS part makes hls.js extend the video timeline
    /// while the audio coalesces, giving progressive A/V drift of roughly half a
    /// second per re-fetch. The earlier even-split rewrite here was the cause.
    /// Returns false if the edge is gone.
    fn finish_segment(&self, sn: u64) -> bool {
        let mut g = self.live_edge.lock().unwrap();
        match g.as_mut() {
            Some(edge) => {
                if let Some(seg) = edge.segments.iter_mut().find(|s| s.sn == sn) {
                    seg.complete = true;
                    let sum: f64 = seg.parts.iter().map(|p| p.duration).sum();
                    if sum > 0.0 {
                        seg.duration = sum; // EXTINF == Σ part durations; parts untouched
                    }
                }
                true
            }
            None => false,
        }
    }

    /// Run one TS part through the transmuxer when it is active. `None` means
    /// the part yielded no complete samples (e.g. a PAT/PMT-only tail) and must
    /// not be published: a listed part has to serve bytes a demuxer can use.
    /// Passthrough (bytes and the caller's duration unchanged) when the
    /// transmux is off.
    ///
    /// On the transmux path the published duration is the transmuxer's sample-
    /// measured span, NOT `dur` from the chunker: the chunker measures
    /// presentation-timestamp deltas at its cut points, which B-frame arrival
    /// order systematically inflates by tens of milliseconds per segment.
    /// hls.js sums the declared durations to place fragments, so that bias
    /// compounds until its part lookup drifts past tolerance and it re-fetches
    /// parts it already buffered (mid-GOP rewrites at the playhead = visible
    /// artifacting). Sample-measured durations keep playlist arithmetic glued
    /// to buffer reality.
    fn convert_part(&self, bytes: Vec<u8>, dur: f64) -> Option<(Vec<u8>, f64)> {
        let mut g = self.transmux.lock().unwrap();
        match g.as_mut() {
            Some(t) => t.push_part(&bytes),
            None => {
                // CMAF passthrough: replace the caller's nominal duration with
                // the part's measured video span (same playlist-honesty rule
                // as the transmux path; nominal-vs-real error compounds into
                // playlist-vs-buffer drift).
                let measured = self
                    .cmaf_video
                    .lock()
                    .unwrap()
                    .and_then(|(tid, ts)| cmaf_part_duration(&bytes, tid, ts));
                Some((bytes, measured.unwrap_or(dur)))
            }
        }
    }

    /// Convert a complete segment's split parts, pairing each published part
    /// with its duration: sample-measured on the transmux path, an even
    /// `TARGET_DURATION/count` split on the raw path (unchanged behavior — raw
    /// backfill durations are normalized so they sum to the segment exactly).
    fn convert_parts(&self, parts: Vec<Vec<u8>>) -> Vec<(Vec<u8>, f64)> {
        let even = TARGET_DURATION as f64 / parts.len().max(1) as f64;
        parts
            .into_iter()
            .filter_map(|p| self.convert_part(p, even))
            .collect()
    }

    /// Origin-generated init segment bytes (TS transmux path only).
    pub fn get_init(&self) -> Option<Arc<Vec<u8>>> {
        self.live_edge.lock().unwrap().as_ref()?.init_bytes.clone()
    }

    /// Duration of the most recent complete segment, the wall-clock yardstick
    /// for the in-progress delivery deadline (segment sizes are per-channel:
    /// ~2s on most, ~4.2s on some).
    fn expected_segment_secs(&self) -> f64 {
        let g = self.live_edge.lock().unwrap();
        g.as_ref()
            .and_then(|e| {
                e.segments
                    .iter()
                    .rev()
                    .find(|s| s.complete)
                    .map(|s| s.duration)
            })
            .filter(|d| d.is_finite() && *d > 0.5)
            .unwrap_or(TARGET_DURATION as f64)
    }

    /// Append a part (with its real media duration) to the segment with `sn`. Returns
    /// false if the edge is gone.
    fn append_part(&self, sn: u64, bytes: Vec<u8>, duration: f64) -> bool {
        let mut g = self.live_edge.lock().unwrap();
        match g.as_mut() {
            Some(edge) => {
                if let Some(seg) = edge.segments.iter_mut().find(|s| s.sn == sn) {
                    seg.parts.push(Part {
                        duration,
                        bytes: Arc::new(bytes),
                    });
                }
                true
            }
            None => false,
        }
    }
}

// ──────────────────────────── serving ────────────────────────────

fn has_part_locked(edge: &LiveEdge, sn: u64, part: u64) -> bool {
    edge.segments
        .iter()
        .any(|s| s.sn == sn && (s.parts.len() as u64) > part)
}

/// Whether a blocking reload for `(sn, part)` can be released. Beyond the plain
/// "that part exists" case, the LL-HLS spec requires a request for a part index
/// past the final part of a COMPLETED segment to be treated as a request for part
/// 0 of the FOLLOWING segment. The client's boundary request (last seen sn, final
/// part index + 1) can only ever be satisfied through that rule; without it every
/// segment hand-off burned a full blocking hold.
fn blocking_satisfied(edge: &LiveEdge, sn: u64, part: u64) -> bool {
    if has_part_locked(edge, sn, part) {
        return true;
    }
    edge.segments
        .iter()
        .any(|s| s.sn == sn && s.complete && (s.parts.len() as u64) <= part)
        && has_part_locked(edge, sn + 1, 0)
}

impl LlOrigin {
    /// Serve the LL-HLS playlist, honoring a blocking reload for `(msn, part)`.
    pub async fn serve_playlist(&self, msn: Option<u64>, part: Option<u64>) -> Option<String> {
        let deadline = tokio::time::Instant::now() + BLOCK_TIMEOUT;
        if let (Some(m), Some(p)) = (msn, part) {
            // Blocking reload: hold until the requested part exists.
            loop {
                let notified = self.notify.notified();
                tokio::pin!(notified);
                // Register the waiter BEFORE the satisfaction check. `Notify`
                // only registers on first poll (or `enable`), so the old
                // check-then-await pattern lost any notify that landed between
                // the check and the await and slept the full hold for nothing.
                notified.as_mut().enable();
                {
                    let g = self.live_edge.lock().unwrap();
                    match g.as_ref() {
                        Some(edge) if blocking_satisfied(edge, m, p) => break,
                        Some(_) => {}
                        None => return None,
                    }
                }
                let now = tokio::time::Instant::now();
                if now >= deadline {
                    break;
                }
                if tokio::time::timeout(deadline - now, notified)
                    .await
                    .is_err()
                {
                    break;
                }
            }
        } else {
            // Directive-less reload: hls.js drops its blocking directives after
            // any MISS (it only sends _HLS_msn when the previous response
            // ADVANCED) and falls back to plain polling, adding up to a second
            // of blind discovery delay right when a famine ends. We own both
            // ends, so plain reloads get hanging-GET semantics too: hold until
            // the edge changes AT ALL, then respond instantly with the fresh
            // playlist — recovery is discovered the moment it happens. The
            // budget for plain reloads is hls.js's normal (non-LL-capped)
            // playlist policy, far above this hold.
            let v0 = self.edge_version.load(Ordering::SeqCst);
            loop {
                let notified = self.notify.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                if self.edge_version.load(Ordering::SeqCst) != v0 {
                    break;
                }
                if self.live_edge.lock().unwrap().is_none() {
                    return None;
                }
                let now = tokio::time::Instant::now();
                if now >= deadline {
                    break;
                }
                if tokio::time::timeout(deadline - now, notified)
                    .await
                    .is_err()
                {
                    break;
                }
            }
        }
        let g = self.live_edge.lock().unwrap();
        g.as_ref().map(render_locked)
    }
}

fn render_locked(edge: &LiveEdge) -> String {
    let mut s = String::with_capacity(2048);
    s.push_str("#EXTM3U\n#EXT-X-VERSION:9\n");
    s.push_str(&format!("#EXT-X-TARGETDURATION:{}\n", edge.target_duration));
    s.push_str(&format!(
        "#EXT-X-PART-INF:PART-TARGET={:.3}\n",
        edge.part_target
    ));
    s.push_str("#EXT-X-SERVER-CONTROL:CAN-BLOCK-RELOAD=YES,PART-HOLD-BACK=1.500\n");
    let media_seq = edge.segments.front().map(|s| s.sn).unwrap_or(0);
    s.push_str(&format!("#EXT-X-MEDIA-SEQUENCE:{media_seq}\n"));
    // CMAF needs its init segment; raw MPEG-TS is self-initializing (PAT/PMT
    // lead each segment) and must NOT carry an `#EXT-X-MAP`. The transmux path
    // serves its own generated init from the relay instead of the upstream URL.
    if edge.init_bytes.is_some() {
        s.push_str("#EXT-X-MAP:URI=\"init.mp4\"\n");
    } else if edge.container == Container::Cmaf {
        s.push_str(&format!("#EXT-X-MAP:URI=\"{}\"\n", edge.init_url));
    }
    let part_ext = if edge.cmaf_presented() { "mp4" } else { "ts" };
    let n = edge.segments.len();
    for (i, seg) in edge.segments.iter().enumerate() {
        if let Some(pdt) = &seg.pdt {
            s.push_str(&format!("#EXT-X-PROGRAM-DATE-TIME:{pdt}\n"));
        }
        // List parts for the last THREE segments. A client can still be mid-way
        // through the previous segment's parts when the next one starts; removing
        // parts it hasn't fetched yet forces it to drop that tail on the floor
        // (observed live as a ~0.2-0.3s buffer hole + bufferSeekOverHole at segment
        // boundaries when only two were listed). Spec guidance: keep parts listed
        // until they are at least three target durations from the edge. Older
        // complete segments are fetched whole.
        if i + 3 >= n {
            for (k, p) in seg.parts.iter().enumerate() {
                // Only part 0 of a segment is independently decodable (Twitch segments
                // are GOP-aligned: part 0 carries the IDR keyframe, parts 1+ are
                // P-frames). Marking a P-frame part INDEPENDENT would let hls.js start
                // decoding mid-GOP — garbage frames, clock doesn't advance, looks like a
                // stall. With only part 0 marked, hls.js starts at a real keyframe.
                let independent = if k == 0 { ",INDEPENDENT=YES" } else { "" };
                s.push_str(&format!(
                    "#EXT-X-PART:DURATION={:.3},URI=\"part/{}/{}.{}\"{}\n",
                    p.duration, seg.sn, k, part_ext, independent
                ));
            }
        }
        // A complete segment renders its EXTINF only once a SUCCESSOR exists in the
        // window. Flipping a segment from in-progress to complete in the same
        // refresh that first reveals its final part lets the client decide the
        // segment is done before fetching that part and advance past it, leaving a
        // one-part (~85-105ms) hole in its buffer at the boundary, which surfaces
        // as repeating bufferStalledError and bufferSeekOverHole pairs.
        // Deferring the EXTINF guarantees at least one refresh in which the final
        // part is visible on a still-in-progress segment. The lone-segment
        // exception keeps a minimal window startable.
        if seg.complete && (i + 1 < n || n == 1) {
            s.push_str(&format!(
                "#EXTINF:{:.3},live\nseg/{}.ts\n",
                seg.duration, seg.sn
            ));
        }
    }
    s
}

impl LlOrigin {
    /// Bytes for a single part (`part/<sn>/<k>.mp4`).
    pub fn get_part(&self, sn: u64, idx: usize) -> Option<Arc<Vec<u8>>> {
        {
            let g = self.live_edge.lock().unwrap();
            let edge = g.as_ref()?;
            if let Some(p) = edge
                .segments
                .iter()
                .find(|s| s.sn == sn)
                .and_then(|s| s.parts.get(idx))
            {
                return Some(p.bytes.clone());
            }
        }
        // Retirement grace: the requester is holding a playlist that listed
        // this part before an eviction/rebuild. Serving the retired bytes
        // keeps the listed-parts-never-404 contract.
        let r = self.retired.lock().unwrap();
        r.iter()
            .find(|s| s.sn == sn)
            .and_then(|s| s.parts.get(idx))
            .map(|p| p.bytes.clone())
    }

    /// Bytes for a complete segment (`seg/<sn>.ts`), assembled from its parts in
    /// memory. Assembly runs at most once per segment (memoized on the segment,
    /// under the same locks as before); repeat fetches clone the handle.
    pub fn get_segment(&self, sn: u64) -> Option<bytes::Bytes> {
        let assemble = |seg: &mut Segment| {
            if let Some(b) = &seg.assembled {
                return b.clone();
            }
            let total: usize = seg.parts.iter().map(|p| p.bytes.len()).sum();
            let mut out = Vec::with_capacity(total);
            for p in &seg.parts {
                out.extend_from_slice(&p.bytes);
            }
            let b = bytes::Bytes::from(out);
            seg.assembled = Some(b.clone());
            b
        };
        {
            let mut g = self.live_edge.lock().unwrap();
            let edge = g.as_mut()?;
            if let Some(seg) = edge.segments.iter_mut().find(|s| s.sn == sn && s.complete) {
                return Some(assemble(seg));
            }
        }
        // Retirement grace (see get_part).
        let mut r = self.retired.lock().unwrap();
        r.iter_mut().find(|s| s.sn == sn && s.complete).map(assemble)
    }
}

// ──────────────────────────── solo facade ────────────────────────────

/// The solo player's shared instance (one solo stream at a time). MultiNook tiles
/// each construct their own origin via `LlOrigin::new`; these functions keep the
/// solo relay's call sites on a single global origin.
static SOLO: Lazy<Arc<LlOrigin>> = Lazy::new(|| LlOrigin::new(MAX_SEGMENTS));

pub fn is_active() -> bool {
    SOLO.is_active()
}

/// True when the experimental low-latency engine is OFF (the hard kill switch is
/// engaged), so no origin will ever serve a parts/`seg/` playlist this session.
/// The whole-segment stable projection keys on this: it must run ONLY when the
/// engine is off, never alongside an active (or about-to-activate) origin, or the
/// player gets two URL schemes for the same media sequence and the refresh check
/// fails. Stable for the session, unlike the per-poll `is_active()`.
pub fn engine_disabled() -> bool {
    DISABLED.load(Ordering::Relaxed)
}

pub fn stop() {
    SOLO.stop()
}

pub async fn start(upstream_playlist_url: String) -> StartOutcome {
    SOLO.clone().start(upstream_playlist_url).await
}

pub fn get_init() -> Option<Arc<Vec<u8>>> {
    SOLO.get_init()
}

pub async fn serve_playlist(msn: Option<u64>, part: Option<u64>) -> Option<String> {
    SOLO.serve_playlist(msn, part).await
}

pub fn get_part(sn: u64, idx: usize) -> Option<Arc<Vec<u8>>> {
    SOLO.get_part(sn, idx)
}

pub fn get_segment(sn: u64) -> Option<bytes::Bytes> {
    SOLO.get_segment(sn)
}

// ──────────────────────────── relay routing helpers ────────────────────────────
// Shared by both relays (`stream_server`, `multi_nook_server`) so their LL-HLS
// routes stay byte-identical.

/// Optional raw query-string filter: yields the query string, or empty if absent.
pub(crate) fn opt_raw_query() -> warp::filters::BoxedFilter<(String,)> {
    use warp::Filter;
    warp::query::raw()
        .or(warp::any().map(String::new))
        .unify()
        .boxed()
}

/// Parse a numeric LL-HLS directive (`_HLS_msn` / `_HLS_part`) from a raw query string.
pub(crate) fn parse_directive(query: &str, key: &str) -> Option<u64> {
    query.split('&').find_map(|pair| {
        let mut it = pair.splitn(2, '=');
        if it.next() == Some(key) {
            it.next().and_then(|v| v.parse().ok())
        } else {
            None
        }
    })
}

/// Parse `part/<sn>/<k>.<ext>` -> (sn, k). The extension (`.mp4` for CMAF, `.ts` for
/// MPEG-TS) is cosmetic — hls.js picks the demuxer from the bytes — so accept either.
pub(crate) fn parse_part_path(rest: &str) -> Option<(u64, usize)> {
    let rest = rest
        .strip_suffix(".mp4")
        .or_else(|| rest.strip_suffix(".ts"))
        .unwrap_or(rest);
    let mut it = rest.splitn(2, '/');
    let sn = it.next()?.parse().ok()?;
    let k = it.next()?.parse().ok()?;
    Some((sn, k))
}

/// Zero-copy bridge: present a shared part/init buffer as a response body
/// without copying it (the per-part memcpy this replaces ran ~9-10 times a
/// second per LL stream).
struct SharedBuf(std::sync::Arc<Vec<u8>>);
impl AsRef<[u8]> for SharedBuf {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}
pub(crate) fn arc_bytes(buf: std::sync::Arc<Vec<u8>>) -> bytes::Bytes {
    bytes::Bytes::from_owner(SharedBuf(buf))
}

pub(crate) fn media_response(bytes: impl Into<bytes::Bytes>) -> warp::http::Response<bytes::Bytes> {
    let bytes: bytes::Bytes = bytes.into();
    // Sniff the container so a TS part is labelled MP2T (CMAF stays mp4). hls.js
    // demuxes from the bytes regardless, but the honest content-type avoids any
    // strict-MIME edge cases.
    let content_type = if bytes.first() == Some(&TS_SYNC) {
        "video/MP2T"
    } else {
        "video/mp4"
    };
    warp::http::Response::builder()
        .status(200)
        .header("Content-Type", content_type)
        .header("Access-Control-Allow-Origin", "*")
        .header("Access-Control-Allow-Methods", "GET, OPTIONS")
        .header("Access-Control-Allow-Headers", "*")
        .header(
            "Cache-Control",
            "no-cache, no-store, must-revalidate, max-age=0",
        )
        .body(bytes)
        .unwrap()
}

pub(crate) fn playlist_response(bytes: Vec<u8>) -> warp::http::Response<bytes::Bytes> {
    warp::http::Response::builder()
        .status(200)
        .header("Content-Type", "application/x-mpegURL")
        .header("Access-Control-Allow-Origin", "*")
        .header("Access-Control-Allow-Methods", "GET, OPTIONS")
        .header("Access-Control-Allow-Headers", "*")
        .header(
            "Cache-Control",
            "no-cache, no-store, must-revalidate, max-age=0",
        )
        // Fresh connection per playlist request. Playlist responses are the
        // only HELD responses the relay serves, and captures show the webview
        // queueing the NEXT playlist request invisibly for seconds during
        // upstream famines (sent per hls.js stats, never reaching the handler,
        // arriving in a flood the moment the famine ends). Keep-alive reuse
        // against held responses is the prime suspect; closing costs nothing
        // on loopback and isolates the layer. Parts/segments keep keep-alive.
        .header("Connection", "close")
        .body(bytes.into())
        .unwrap()
}

pub(crate) fn empty_cors(status: u16) -> warp::http::Response<bytes::Bytes> {
    warp::http::Response::builder()
        .status(status)
        .header("Access-Control-Allow-Origin", "*")
        .body(bytes::Bytes::new())
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_box(typ: &[u8; 4], version: u8, payload: &[u8]) -> Vec<u8> {
        let mut body = vec![version, 0, 0, 0];
        body.extend_from_slice(payload);
        make_box(typ, &body)
    }

    #[test]
    fn cmaf_durations_measured_from_sample_tables() {
        // Init: moov > trak > (tkhd v0 id=1, mdia > (mdhd v0 timescale=90000,
        // hdlr 'vide')).
        let mut tkhd = vec![0u8; 8]; // creation+modification
        tkhd.extend_from_slice(&1u32.to_be_bytes()); // track_id
        let tkhd = full_box(b"tkhd", 0, &tkhd);
        let mut mdhd = vec![0u8; 8];
        mdhd.extend_from_slice(&90_000u32.to_be_bytes()); // timescale
        mdhd.extend_from_slice(&0u32.to_be_bytes()); // duration
        let mdhd = full_box(b"mdhd", 0, &mdhd);
        let mut hdlr_p = vec![0u8; 4]; // pre_defined
        hdlr_p.extend_from_slice(b"vide");
        let hdlr = full_box(b"hdlr", 0, &hdlr_p);
        let mdia = make_box(b"mdia", &[mdhd, hdlr].concat());
        let trak = make_box(b"trak", &[tkhd, mdia].concat());
        let moov = make_box(b"moov", &trak);
        let init = [make_box(b"ftyp", b"isom"), moov].concat();
        let (tid, ts) = parse_cmaf_video_track(&init).expect("video track parsed");
        assert_eq!((tid, ts), (1, 90_000));

        // Part: moof > traf > (tfhd id=1 with default_sample_duration=1500,
        // trun count=6 without per-sample durations) + mdat. 6 x 1500 / 90000.
        let mut tfhd_p = vec![0u8, 0, 0, 0x08]; // version 0, flags: default-duration present
        tfhd_p.extend_from_slice(&1u32.to_be_bytes());
        tfhd_p.extend_from_slice(&1500u32.to_be_bytes());
        let tfhd = make_box(b"tfhd", &tfhd_p);
        let mut trun_p = vec![0u8, 0, 0, 0]; // version 0, no optional fields
        trun_p.extend_from_slice(&6u32.to_be_bytes());
        let trun = make_box(b"trun", &trun_p);
        let traf = make_box(b"traf", &[tfhd, trun].concat());
        let moof = make_box(b"moof", &traf);
        let part = [moof, make_box(b"mdat", &[0u8; 4])].concat();
        let d = cmaf_part_duration(&part, 1, 90_000).expect("measured");
        assert!((d - 0.1).abs() < 1e-9, "6x1500/90000 = 0.1, got {d}");
    }

    /// Pair fixture part bytes with the raw path's even duration split.
    fn even_parts(bytes: Vec<Vec<u8>>) -> Vec<(Vec<u8>, f64)> {
        let dur = TARGET_DURATION as f64 / bytes.len().max(1) as f64;
        bytes.into_iter().map(|b| (b, dur)).collect()
    }

    /// Build a fake CMAF byte stream: a leading emsg, then `n` moof+mdat pairs.
    fn make_box(typ: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let size = (8 + payload.len()) as u32;
        let mut b = Vec::new();
        b.extend_from_slice(&size.to_be_bytes());
        b.extend_from_slice(typ);
        b.extend_from_slice(payload);
        b
    }

    fn fake_segment(n: usize) -> (Vec<u8>, Vec<Vec<u8>>) {
        let mut full = Vec::new();
        let emsg = make_box(b"emsg", b"meta");
        full.extend_from_slice(&emsg);
        let mut expected_parts: Vec<Vec<u8>> = Vec::new();
        for i in 0..n {
            let moof = make_box(b"moof", &[i as u8; 12]);
            let mdat = make_box(b"mdat", &[i as u8; 40]);
            let mut part = Vec::new();
            if i == 0 {
                part.extend_from_slice(&emsg); // leading emsg rides with the first part
            }
            part.extend_from_slice(&moof);
            part.extend_from_slice(&mdat);
            expected_parts.push(part);
            full.extend_from_slice(&moof);
            full.extend_from_slice(&mdat);
        }
        (full, expected_parts)
    }

    #[test]
    fn chunker_splits_on_moof_mdat_pairs() {
        let (full, expected) = fake_segment(19);
        let parts = split_complete(&full, Container::Cmaf);
        assert_eq!(parts.len(), 19);
        assert_eq!(parts, expected);
        // Parts reassemble into the original byte stream exactly.
        let rejoined: Vec<u8> = parts.concat();
        assert_eq!(rejoined, full);
    }

    #[test]
    fn chunker_handles_split_across_feeds() {
        // Feed the stream one byte at a time; parts must still come out identical.
        let (full, expected) = fake_segment(5);
        let mut chunker = BoxChunker::new();
        let mut got: Vec<Vec<u8>> = Vec::new();
        for byte in &full {
            got.extend(chunker.push(&[*byte]));
        }
        if let Some(tail) = chunker.flush() {
            got.push(tail);
        }
        assert_eq!(got, expected);
    }

    // ── MPEG-TS chunker ──

    /// Build one 188-byte TS packet on PID 0x100. `pts` (90 kHz) is encoded as a
    /// video PES header when `pusi` is set; otherwise it's a payload-only
    /// continuation packet. Remaining payload is zero-filled to 188 bytes.
    fn ts_packet(pusi: bool, stream_id: u8, pts: Option<u64>, cc: u8) -> Vec<u8> {
        let pid: u16 = 0x100;
        let mut p = vec![0u8; TS_PACKET];
        p[0] = TS_SYNC;
        p[1] = (if pusi { 0x40 } else { 0x00 }) | ((pid >> 8) as u8 & 0x1F);
        p[2] = (pid & 0xFF) as u8;
        p[3] = 0x10 | (cc & 0x0F); // adaptation=01 (payload only)
        if pusi {
            let mut h = vec![0x00, 0x00, 0x01, stream_id, 0x00, 0x00, 0x80];
            if let Some(pts) = pts {
                h.push(0x80); // PTS_DTS_flags = '10'
                h.push(0x05); // PES_header_data_length
                h.push(0x20 | (((pts >> 30) & 0x07) << 1) as u8 | 0x01);
                h.push(((pts >> 22) & 0xFF) as u8);
                h.push(((((pts >> 15) & 0x7F) << 1) | 0x01) as u8);
                h.push(((pts >> 7) & 0xFF) as u8);
                h.push((((pts & 0x7F) << 1) | 0x01) as u8);
            } else {
                h.push(0x00); // PTS_DTS_flags = '00'
                h.push(0x00);
            }
            p[4..4 + h.len()].copy_from_slice(&h);
        }
        p
    }

    #[test]
    fn ts_pts_parses_and_rejects_non_video() {
        // A known PTS round-trips through the parser.
        let pkt = ts_packet(true, 0xE0, Some(123_456), 0);
        assert_eq!(video_pes_pts(&pkt), Some(123_456));
        // Audio PES (0xC0) is not a cut point.
        assert_eq!(video_pes_pts(&ts_packet(true, 0xC0, Some(99), 0)), None);
        // A continuation packet (no PUSI) is not a cut point.
        assert_eq!(video_pes_pts(&ts_packet(false, 0xE0, None, 1)), None);
        // A video PES with no PTS is not a cut point.
        assert_eq!(video_pes_pts(&ts_packet(true, 0xE0, None, 0)), None);
    }

    #[test]
    fn ts_chunker_cuts_on_pes_boundaries_by_pts() {
        // 12 access units, one TS packet each, spaced 0.1s (9000 ticks). With a
        // 0.3s (27000-tick) part target, a cut falls every 3rd PES.
        let mut full = Vec::new();
        let mut pkts = Vec::new();
        for i in 0..12u64 {
            let pkt = ts_packet(true, 0xE0, Some(i * 9000), (i % 16) as u8);
            full.extend_from_slice(&pkt);
            pkts.push(pkt);
        }
        let mut chunker = TsChunker::new();
        let mut raw = chunker.push(&full);
        if let Some(tail) = chunker.flush() {
            raw.push(tail);
        }
        // Cuts before PES 3, 6, 9 → parts [0..3),[3..6),[6..9),[9..12).
        assert_eq!(raw.len(), 4);
        // Each cut part spans 3 access units × 0.1s = 0.3s of real PTS time.
        for (_, dur) in &raw[..3] {
            assert!(
                (*dur - 0.3).abs() < 0.001,
                "cut part duration is the real PTS span"
            );
        }
        let parts: Vec<Vec<u8>> = raw.into_iter().map(|(b, _)| b).collect();
        for part in &parts {
            assert_eq!(part.len() % TS_PACKET, 0, "parts are packet-aligned");
            assert_eq!(part.len(), 3 * TS_PACKET, "each part holds 3 access units");
        }
        // Part 0 begins at the first packet (the keyframe/PAT-PMT lead is never cut).
        assert_eq!(&parts[0][..TS_PACKET], pkts[0].as_slice());
        // Lossless: parts reassemble into the original byte stream.
        assert_eq!(parts.concat(), full);
    }

    #[test]
    fn ts_chunker_part_durations_sum_to_the_segment() {
        // The boomerang fix: parts must tile the segment, not undercount it. 10 access
        // units at 0.1s each = a 1.0s span; the part durations must add up to ~that.
        let mut full = Vec::new();
        for i in 0..10u64 {
            full.extend_from_slice(&ts_packet(true, 0xE0, Some(i * 9000), (i % 16) as u8));
        }
        let mut chunker = TsChunker::new();
        let mut raw = chunker.push(&full);
        if let Some(tail) = chunker.flush() {
            raw.push(tail);
        }
        let total: f64 = raw.iter().map(|(_, d)| d).sum();
        // 3 cut parts of 0.3s + the single-frame flush tail's nominal estimate ≈ 1.0s
        // (the point: ~the 0.9s real span, NOT the ~0.4s a fixed-nominal would give).
        assert!(
            total > 0.85 && total < 1.15,
            "parts tile the segment span (got {total})"
        );
    }

    #[test]
    fn ts_chunker_keeps_continuation_packets_in_their_part() {
        // PES(pts0) + 2 continuation packets, then PES(pts=27000): the continuations
        // ride in part 0; the cut lands at the second PES.
        let mut full = Vec::new();
        full.extend_from_slice(&ts_packet(true, 0xE0, Some(0), 0));
        full.extend_from_slice(&ts_packet(false, 0xE0, None, 1));
        full.extend_from_slice(&ts_packet(false, 0xE0, None, 2));
        full.extend_from_slice(&ts_packet(true, 0xE0, Some(27_000), 3));
        let mut chunker = TsChunker::new();
        let mut raw = chunker.push(&full);
        if let Some(tail) = chunker.flush() {
            raw.push(tail);
        }
        let parts: Vec<Vec<u8>> = raw.into_iter().map(|(b, _)| b).collect();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].len(), 3 * TS_PACKET); // PES + 2 continuations
        assert_eq!(parts[1].len(), TS_PACKET);
        assert_eq!(parts.concat(), full);
    }

    #[test]
    fn ts_chunker_reassembles_under_byte_split_feeds() {
        // Feeding the same stream one byte at a time must produce identical parts
        // (the reader receives arbitrary network chunk boundaries).
        let mut full = Vec::new();
        for i in 0..9u64 {
            full.extend_from_slice(&ts_packet(true, 0xE0, Some(i * 9000), (i % 16) as u8));
        }
        let mut whole = TsChunker::new();
        let mut want = whole.push(&full);
        if let Some(t) = whole.flush() {
            want.push(t);
        }
        let mut drip = TsChunker::new();
        let mut got: Vec<(Vec<u8>, f64)> = Vec::new();
        for b in &full {
            got.extend(drip.push(&[*b]));
        }
        if let Some(t) = drip.flush() {
            got.push(t);
        }
        assert_eq!(got, want);
        let bytes: Vec<u8> = got.into_iter().flat_map(|(b, _)| b).collect();
        assert_eq!(bytes, full);
    }

    #[test]
    fn ts_chunker_keeps_psi_lead_with_keyframe_in_part0() {
        // A real segment leads with non-PES packets (PAT/PMT) before the keyframe PES.
        // Those must ride in part 0 WITH the first keyframe — never split off into a
        // keyframe-less independent part 0. Simulate the lead with two no-PTS packets
        // (video_pes_pts returns None for them), then PES(0), PES(27000), PES(54000).
        let mut full = Vec::new();
        full.extend_from_slice(&ts_packet(false, 0xE0, None, 0)); // PAT-like
        full.extend_from_slice(&ts_packet(false, 0xE0, None, 1)); // PMT-like
        let keyframe = ts_packet(true, 0xE0, Some(0), 2);
        full.extend_from_slice(&keyframe);
        full.extend_from_slice(&ts_packet(true, 0xE0, Some(27_000), 3));
        full.extend_from_slice(&ts_packet(true, 0xE0, Some(54_000), 4));
        let mut chunker = TsChunker::new();
        let mut raw = chunker.push(&full);
        if let Some(t) = chunker.flush() {
            raw.push(t);
        }
        let parts: Vec<Vec<u8>> = raw.into_iter().map(|(b, _)| b).collect();
        // part 0 = [lead, lead, keyframe PES] (3 packets), then a cut at pts 27000.
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0].len(), 3 * TS_PACKET);
        // The keyframe PES sits in part 0 (third packet), NOT split off into part 1.
        assert_eq!(&parts[0][2 * TS_PACKET..3 * TS_PACKET], keyframe.as_slice());
        assert_eq!(parts.concat(), full);
    }

    #[test]
    fn ts_chunker_resyncs_after_garbage() {
        // A run of non-sync bytes ahead of a valid packet is skipped, not absorbed
        // into a part (a dropped/garbled packet otherwise poisons every boundary).
        let pkt = ts_packet(true, 0xE0, Some(0), 0);
        let mut stream = vec![0xAB, 0xCD, 0xEF]; // junk, no 0x47
        stream.extend_from_slice(&pkt);
        let mut chunker = TsChunker::new();
        let parts = chunker.push(&stream);
        assert!(parts.is_empty()); // single PES, no cut yet
        let (tail, _dur) = chunker.flush().unwrap();
        assert_eq!(tail, pkt); // the junk was dropped; the packet survives intact
    }

    #[test]
    fn parse_upstream_reads_segments_prefetch_and_map() {
        let pl = "#EXTM3U\n\
#EXT-X-TARGETDURATION:2\n\
#EXT-X-MEDIA-SEQUENCE:100\n\
#EXT-X-MAP:URI=\"https://cdn/init.mp4\"\n\
#EXT-X-PROGRAM-DATE-TIME:2026-06-09T03:02:33.166Z\n\
#EXTINF:2.000,live\nhttps://cdn/a100.mp4\n\
#EXTINF:2.000,live\nhttps://cdn/a101.mp4\n\
#EXT-X-TWITCH-PREFETCH:https://cdn/a102.mp4\n\
#EXT-X-TWITCH-PREFETCH:https://cdn/a103.mp4\n";
        let up = parse_upstream(pl, "https://cdn/");
        assert_eq!(up.init_url.as_deref(), Some("https://cdn/init.mp4"));
        assert_eq!(up.published.len(), 2);
        assert_eq!(up.published[0].0, 100);
        assert_eq!(up.published[1].0, 101);
        assert_eq!(
            up.published[0].1.as_deref(),
            Some("2026-06-09T03:02:33.166Z")
        );
        assert_eq!(
            up.prefetch,
            vec!["https://cdn/a102.mp4", "https://cdn/a103.mp4"]
        );
    }

    #[test]
    fn catch_up_fills_a_small_hole_adjacently() {
        // The startup race: backfill held ..8551, segment 8552 finalized before the
        // first poll, in-progress is 8553. The plan must fetch exactly 8552 without
        // clearing, so the window never renders a hole.
        let (rebuild, fetch) = plan_catch_up(8551, 8540, 8553);
        assert!(!rebuild);
        assert_eq!(fetch.collect::<Vec<_>>(), vec![8552]);
    }

    #[test]
    fn catch_up_rebuilds_when_hole_is_unfillable_or_deep() {
        // Hole predates the upstream window: rebuild with the minimal backfill.
        let (rebuild, fetch) = plan_catch_up(8500, 8540, 8553);
        assert!(rebuild);
        assert_eq!(fetch.collect::<Vec<_>>(), vec![8552]);

        // Hole deeper than CATCH_UP_MAX_SEGMENTS: serial fills can't gain
        // ground on a constrained connection, so jump to the edge instead.
        let (rebuild, fetch) = plan_catch_up(8549, 8540, 8553);
        assert!(rebuild);
        assert_eq!(fetch.collect::<Vec<_>>(), vec![8552]);

        // Empty window (newest sentinel 0) behaves like a rebuild too.
        let (rebuild, fetch) = plan_catch_up(0, 8546, 8553);
        assert!(rebuild);
        assert_eq!(fetch.collect::<Vec<_>>(), vec![8552]);
    }

    #[test]
    fn newest_complete_segment_defers_extinf_until_successor() {
        // 101 is internally complete but has no successor yet: it must render as
        // still-in-progress (parts only), so a client never learns "complete" in
        // the same refresh that first shows the final part.
        let mut segs = VecDeque::new();
        segs.push_back(make_segment(
            100,
            None,
            true,
            even_parts(vec![vec![1], vec![2]]),
        ));
        segs.push_back(make_segment(
            101,
            None,
            true,
            even_parts(vec![vec![3], vec![4]]),
        ));
        let edge = LiveEdge {
            init_url: "https://cdn/init.mp4".into(),
            init_bytes: None,
            container: Container::Cmaf,
            target_duration: 2,
            part_target: PART_TARGET,
            segments: segs,
        };
        let pl = render_locked(&edge);
        assert!(pl.contains("seg/100.ts"));
        assert!(!pl.contains("seg/101.ts"));
        assert!(pl.contains("part/101/1.mp4"));

        // A lone complete segment still renders EXTINF (a playlist with zero
        // complete segments cannot start playback).
        let mut lone = VecDeque::new();
        lone.push_back(make_segment(100, None, true, even_parts(vec![vec![1]])));
        let edge = LiveEdge {
            init_url: "https://cdn/init.mp4".into(),
            init_bytes: None,
            container: Container::Cmaf,
            target_duration: 2,
            part_target: PART_TARGET,
            segments: lone,
        };
        assert!(render_locked(&edge).contains("seg/100.ts"));
    }

    #[test]
    fn boundary_blocking_request_rolls_to_next_segment() {
        let mut segs = VecDeque::new();
        segs.push_back(make_segment(
            100,
            None,
            true,
            even_parts(vec![vec![1], vec![2]]),
        ));
        segs.push_back(Segment {
            sn: 101,
            pdt: None,
            complete: false,
            duration: 2.0,
            parts: vec![Part {
                duration: 0.1,
                bytes: Arc::new(vec![3]),
            }],
            assembled: None,
        });
        let edge = LiveEdge {
            init_url: "https://cdn/init.mp4".into(),
            init_bytes: None,
            container: Container::Cmaf,
            target_duration: 2,
            part_target: PART_TARGET,
            segments: segs,
        };
        // Plain existing-part requests.
        assert!(blocking_satisfied(&edge, 100, 1));
        assert!(blocking_satisfied(&edge, 101, 0));
        // Beyond the final part of COMPLETE 100: rolls to part 0 of 101 (spec rule).
        assert!(blocking_satisfied(&edge, 100, 2));
        // Beyond the newest part of the still-in-progress 101: must keep blocking.
        assert!(!blocking_satisfied(&edge, 101, 1));
    }

    #[test]
    fn render_always_has_extinf_and_lists_edge_parts() {
        let mut segs = VecDeque::new();
        segs.push_back(make_segment(
            99,
            Some("PDT9".into()),
            true,
            even_parts(vec![vec![0], vec![9]]),
        ));
        segs.push_back(make_segment(
            100,
            Some("PDT0".into()),
            true,
            even_parts(vec![vec![1], vec![2]]),
        ));
        segs.push_back(make_segment(
            101,
            Some("PDT1".into()),
            true,
            even_parts(vec![vec![3], vec![4]]),
        ));
        // in-progress segment with 3 parts, not complete
        segs.push_back(Segment {
            sn: 102,
            pdt: Some("PDT2".into()),
            complete: false,
            duration: 2.0,
            parts: vec![
                Part {
                    duration: 0.1,
                    bytes: Arc::new(vec![5]),
                },
                Part {
                    duration: 0.1,
                    bytes: Arc::new(vec![6]),
                },
                Part {
                    duration: 0.1,
                    bytes: Arc::new(vec![7]),
                },
            ],
            assembled: None,
        });
        let edge = LiveEdge {
            init_url: "https://cdn/init.mp4".into(),
            init_bytes: None,
            container: Container::Cmaf,
            target_duration: 2,
            part_target: PART_TARGET,
            segments: segs,
        };
        let pl = render_locked(&edge);
        assert!(pl.contains("#EXT-X-SERVER-CONTROL:CAN-BLOCK-RELOAD=YES"));
        assert!(pl.contains("#EXT-X-PART-INF:PART-TARGET="));
        assert!(pl.contains("#EXT-X-MEDIA-SEQUENCE:99"));
        assert!(pl.contains("#EXT-X-MAP:URI=\"https://cdn/init.mp4\""));
        // At least one complete segment (avoids "not enough fragments").
        assert!(pl.contains("#EXTINF:"));
        assert!(pl.contains("seg/100.ts"));
        // The last THREE segments list parts (a client may still be mid-way through
        // the previous segment's parts when a new one starts); older ones do not.
        assert!(pl.contains("part/102/0.mp4"));
        assert!(pl.contains("part/101/0.mp4"));
        assert!(pl.contains("part/100/0.mp4"));
        assert!(!pl.contains("part/99/0.mp4"));
        // In-progress segment has no EXTINF yet.
        assert!(!pl.contains("seg/102.ts"));
        // has_part: the in-progress segment exposes its 3 parts.
        assert!(has_part_locked(&edge, 102, 2));
        assert!(!has_part_locked(&edge, 102, 3));
    }

    #[test]
    fn render_omits_map_and_uses_ts_parts_for_mpeg_ts() {
        let mut segs = VecDeque::new();
        segs.push_back(make_segment(
            50,
            Some("PDT0".into()),
            true,
            even_parts(vec![vec![1], vec![2]]),
        ));
        segs.push_back(Segment {
            sn: 51,
            pdt: Some("PDT1".into()),
            complete: false,
            duration: 2.0,
            parts: vec![Part {
                duration: 0.3,
                bytes: Arc::new(vec![3]),
            }],
            assembled: None,
        });
        let edge = LiveEdge {
            init_url: String::new(),
            init_bytes: None,
            container: Container::Ts,
            target_duration: 2,
            part_target: PART_TARGET,
            segments: segs,
        };
        let pl = render_locked(&edge);
        // TS is self-initializing: NO #EXT-X-MAP.
        assert!(!pl.contains("#EXT-X-MAP"));
        // Parts and complete segments use the .ts extension.
        assert!(pl.contains("part/51/0.ts"));
        assert!(pl.contains("seg/50.ts"));
        // Still a valid LL-HLS playlist shape.
        assert!(pl.contains("#EXT-X-SERVER-CONTROL:CAN-BLOCK-RELOAD=YES"));
        assert!(pl.contains("#EXT-X-PART:DURATION="));
        assert!(pl.contains("INDEPENDENT=YES"));
    }

    #[test]
    fn render_transmuxed_ts_presents_cmaf() {
        // A TS edge with origin-generated init bytes (the transmux path) must
        // advertise the relay-served init and .mp4 parts: the player consumes
        // fMP4 even though the upstream container is TS.
        let mut segs = VecDeque::new();
        segs.push_back(make_segment(
            50,
            None,
            true,
            even_parts(vec![vec![1], vec![2]]),
        ));
        segs.push_back(make_segment(51, None, true, even_parts(vec![vec![3]])));
        let edge = LiveEdge {
            init_url: String::new(),
            init_bytes: Some(Arc::new(vec![0, 0, 0, 8, b'f', b't', b'y', b'p'])),
            container: Container::Ts,
            target_duration: 2,
            part_target: PART_TARGET,
            segments: segs,
        };
        let pl = render_locked(&edge);
        assert!(pl.contains("#EXT-X-MAP:URI=\"init.mp4\""));
        assert!(pl.contains("part/51/0.mp4"));
        assert!(!pl.contains("part/51/0.ts"));
    }

    fn ts_edge_with_one_segment() -> LiveEdge {
        let mut segs = VecDeque::new();
        segs.push_back(make_segment(1, None, true, even_parts(vec![vec![1]])));
        LiveEdge {
            init_url: String::new(),
            init_bytes: None,
            container: Container::Ts,
            target_duration: 2,
            part_target: PART_TARGET,
            segments: segs,
        }
    }

    #[test]
    fn deactivate_offline_clears_the_active_edge() {
        // The permanent-death exit: when the upstream is judged gone, the edge is
        // cleared so is_active() goes false and serve_playlist returns None (the
        // relay then falls through instead of serving a stale playlist forever).
        let origin = LlOrigin::new(6);
        *origin.live_edge.lock().unwrap() = Some(ts_edge_with_one_segment());
        assert!(origin.is_active());
        let before = origin.edge_version.load(Ordering::SeqCst);
        origin.deactivate_offline("test offline");
        assert!(!origin.is_active());
        // serve_playlist must report inactive, not a stale playlist.
        let served = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap()
            .block_on(origin.serve_playlist(None, None));
        assert!(served.is_none());
        // The change was published so any blocked reload wakes.
        assert!(origin.edge_version.load(Ordering::SeqCst) > before);
    }

    #[test]
    fn init_url_update_only_fires_on_a_cmaf_change() {
        // CMAF passthrough: a changed upstream MAP URI is adopted (forces hls.js
        // to re-fetch the init); an unchanged one is not.
        let cmaf = LiveEdge {
            init_url: "https://cdn/init_A.mp4".into(),
            init_bytes: None,
            container: Container::Cmaf,
            target_duration: 2,
            part_target: PART_TARGET,
            segments: VecDeque::new(),
        };
        assert_eq!(
            init_url_update(&cmaf, Some("https://cdn/init_B.mp4")),
            Some("https://cdn/init_B.mp4".to_string())
        );
        assert_eq!(init_url_update(&cmaf, Some("https://cdn/init_A.mp4")), None);
        assert_eq!(init_url_update(&cmaf, None), None);

        // TS-transmux path (init_bytes Some): never touched — its init.mp4 is
        // immutable and avc3 handles parameter-set changes in-band.
        let transmux = LiveEdge {
            init_url: String::new(),
            init_bytes: Some(Arc::new(vec![1, 2, 3])),
            container: Container::Ts,
            target_duration: 2,
            part_target: PART_TARGET,
            segments: VecDeque::new(),
        };
        assert_eq!(
            init_url_update(&transmux, Some("https://cdn/whatever.mp4")),
            None
        );
    }
}
