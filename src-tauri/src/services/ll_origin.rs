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
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
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
/// Ceiling on a tile window widened for a MultiNook sync (`window_for_delay`).
const TILE_SYNC_MAX_SEGMENTS: usize = 8;

/// The tile window that keeps a playhead `delay_secs` behind live inside the
/// playlist. hls.js snaps a playhead that sits before the first listed segment
/// back to live the moment it buffers, so a tile held at a shared MultiNook
/// delay needs complete segments covering that delay plus slack (playlist age
/// and the in-progress segment), and never less than the normal tile window.
pub(crate) fn window_for_delay(delay_secs: f64) -> usize {
    if !delay_secs.is_finite() || delay_secs <= 0.0 {
        return TILE_MAX_SEGMENTS;
    }
    let complete = ((delay_secs + 2.0) / TARGET_DURATION as f64).ceil() as usize;
    (complete + 1).clamp(TILE_MAX_SEGMENTS, TILE_SYNC_MAX_SEGMENTS)
}
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
/// Byte-exact pickups attempted on a fresh connection before a segment's
/// tail is abandoned. Each is bounded by the delivery deadline above, so a
/// second attempt costs nothing on a dead segment and recovers a tail whose
/// first pickup connection died the same way the original did.
const RESUME_ATTEMPTS: u8 = 2;
/// A body that ends cleanly but this far short of the segment's expected
/// duration was cut at a fragment boundary: the CDN closed the in-progress
/// object early while the segment kept growing. Captured 2026-09-21 on a
/// 1440p60 CMAF channel: 16 of 19 fragments, no partial box, no timeout,
/// a 283 ms hole in the player. One fragment of slack: two or more missing
/// fragments earn the pickup, a single one (a 100 ms seam hls.js steps over)
/// does not hold the reader.
const SHORT_SEGMENT_TOLERANCE: f64 = 0.15;
/// The one pickup a clean-but-short body gets. Short, because the reader is
/// sequential and a legitimately short segment must not hold the next one.
const SHORT_PICKUP_TIMEOUT: Duration = Duration::from_millis(700);
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
    fn is_cmaf(&self) -> bool {
        matches!(self, Chunker::Cmaf(_))
    }
    /// Bytes of a fragment the body cut short at the current end of input.
    /// Only the CMAF chunker can tell: a TS tail is cut at PES boundaries by
    /// `flush`.
    fn truncated_fragment_bytes(&self) -> usize {
        match self {
            Chunker::Cmaf(c) => c.truncated_fragment_bytes(),
            Chunker::Ts(_) => 0,
        }
    }
}

/// Type of the last complete box in a run of boxes, or None when the bytes
/// do not parse as whole boxes.
fn last_box_type(bytes: &[u8]) -> Option<[u8; 4]> {
    let mut at = 0usize;
    let mut last = None;
    while bytes.len() - at >= 8 {
        let size32 = u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]) as usize;
        let len = if size32 == 1 {
            if bytes.len() - at < 16 {
                return None;
            }
            u64::from_be_bytes(bytes[at + 8..at + 16].try_into().unwrap()) as usize
        } else {
            size32
        };
        if len < 8 || at + len > bytes.len() {
            return None;
        }
        last = Some([bytes[at + 4], bytes[at + 5], bytes[at + 6], bytes[at + 7]]);
        at += len;
    }
    last
}

/// True when a segment that ended cleanly holds clearly less media than the
/// broadcast's segment length: cut at a fragment boundary, not finished.
fn ended_short(held_secs: f64, expected_secs: f64) -> bool {
    expected_secs > 0.0 && held_secs + SHORT_SEGMENT_TOLERANCE < expected_secs
}

/// How many trailing bytes of what the reader already holds are compared
/// against the same offsets of a full-body pickup before its remainder is
/// trusted. A pickup that restarts the object must be byte-identical up to
/// the held length; if it is not, continuing would splice foreign bytes into
/// the middle of a fragment, which the decoder plays as garbage.
const PICKUP_FINGERPRINT_BYTES: usize = 64;

/// Keep the last `PICKUP_FINGERPRINT_BYTES` of a growing byte stream.
fn remember_tail(tail: &mut Vec<u8>, bytes: &[u8]) {
    tail.extend_from_slice(bytes);
    if tail.len() > PICKUP_FINGERPRINT_BYTES {
        let cut = tail.len() - PICKUP_FINGERPRINT_BYTES;
        tail.drain(..cut);
    }
}

/// Drop the leading bytes of a resumed body that the reader already holds.
/// A pickup answered with a full body (200) restarts the object from byte 0;
/// the first `skip` bytes are the ones already chunked, so the stream stays
/// byte-exact once they are discarded. `skip` counts down across chunks.
fn skip_prefix<'a>(skip: &mut u64, bytes: &'a [u8]) -> &'a [u8] {
    if *skip == 0 {
        return bytes;
    }
    let n = (*skip).min(bytes.len() as u64) as usize;
    *skip -= n as u64;
    &bytes[n..]
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
    /// Live window size (complete segments + the in-progress one). A tile's
    /// widens while a MultiNook sync holds it further back (`set_window`).
    max_segments: AtomicUsize,
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
    /// PART-TARGET of the Apple playlist, fixed by the first one served for
    /// this stream (see `APPLE_PART_TARGET_HEADROOM`).
    apple_part_target: Mutex<Option<f64>>,
    /// The stream `link_adapt` knows this origin by (`solo` or a tile id): its
    /// lost tails and its own segment downloads are reported under it.
    adapt_id: Mutex<String>,
    /// The upstream playlist the reader follows, so a parked origin can restart it.
    upstream: Mutex<Option<String>>,
    /// No player asked for the playlist for `PARK_IDLE_MS`: the reader is
    /// stopped and the edge held until a player asks again (`unpark`).
    parked: AtomicBool,
    /// Whether the live edge holds a window the reader published since the last
    /// unpark. False from an `unpark` until the restarted reader's first
    /// publish (`mark_fresh`), so a playlist request in between waits for it
    /// instead of being handed the window held across the park. True at
    /// creation, after a stop, and while parked.
    fresh_after_unpark: AtomicBool,
    /// Set by `rebuild_at_edge`, taken by the reader it starts: that reader
    /// drops the held window and rebuilds it from the upstream's live edge,
    /// whether or not the window already reaches the upstream.
    force_rebuild: AtomicBool,
    /// When a player last asked for the playlist (`mono_ms`; 0 = not yet). A
    /// player that stopped loading lets the reader park the origin (`PARK_IDLE_MS`).
    last_playlist_ms: AtomicU64,
    /// Held for the whole of `start`, so two starts on one origin run one after
    /// the other instead of interleaving their probes and installs.
    start_lock: tokio::sync::Mutex<()>,
    /// Set by `close`: the stream this origin served is gone for good, so no
    /// start, install or unpark may bring a reader back.
    closed: AtomicBool,
    /// Re-signs the upstream playlist URL when the first polls after an unpark
    /// are refused (`PlaylistHealth`). Set by the relay that owns the stream.
    resigner: Mutex<Option<Resigner>>,
    /// Playlist requests being served right now (`ServeGuard`). The reader never
    /// parks the origin while one is in flight: the request is waiting on the
    /// window it would be handed.
    serving: AtomicUsize,
}

/// One playlist request in flight on an origin, counted in `LlOrigin::serving`
/// from `LlOrigin::serving_guard` until it drops, on every return path and when
/// the request is cancelled.
struct ServeGuard<'a>(&'a AtomicUsize);

impl Drop for ServeGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
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
/// The viewer's chosen distance behind live (`video_player.ll_target_latency`), in
/// milliseconds; 0 is Auto.
static TARGET_GAP_MS: AtomicU64 = AtomicU64::new(0);

/// A player that has not asked for the playlist this long has stopped loading
/// (the desktop players stop 2 s into a pause): the reader parks the origin and
/// the next request wakes it (stream_server, multi_nook_server). The desktop
/// players start over at the live edge after a shorter pause (pausePark.ts
/// RESYNC_AFTER_PAUSE_MS) and ask the origin to rebuild its window first
/// (`rebuild_at_edge`), so a resync never plays the window held across the
/// pause whether or not the origin has parked yet.
const PARK_IDLE_MS: u64 = 30_000;

/// `PARK_IDLE_MS`, or `SN_PARK_IDLE_MS` from the environment when it holds a
/// positive number of milliseconds (test runs use seconds). Read once.
pub(crate) fn park_idle_ms() -> u64 {
    static SPAN: Lazy<u64> = Lazy::new(|| {
        std::env::var("SN_PARK_IDLE_MS")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .filter(|ms| *ms > 0)
            .unwrap_or(PARK_IDLE_MS)
    });
    *SPAN
}

/// Milliseconds on a monotonic clock, for idle spans. Never 0.
pub(crate) fn mono_ms() -> u64 {
    static T0: Lazy<std::time::Instant> = Lazy::new(std::time::Instant::now);
    (T0.elapsed().as_millis() as u64).saturating_add(1)
}

/// Whether an origin last asked at `last` (0 = never) has been idle for
/// `span` at `now`.
pub(crate) fn idle_for_park(last: u64, now: u64, span: u64) -> bool {
    last != 0 && now.saturating_sub(last) >= span
}

/// A plugin swap that arrived while the relay was parked: stored in `slot`
/// (replacing an older one) and `None` returned, so nothing starts. When the
/// relay is not parked the URL comes back to be applied now, and a swap held
/// earlier is dropped as superseded.
pub(crate) fn hold_swap_if_parked(
    slot: &mut Option<String>,
    parked: bool,
    url: String,
) -> Option<String> {
    if parked {
        *slot = Some(url);
        None
    } else {
        *slot = None;
        Some(url)
    }
}

/// The held swap, taken once a player is loading again. A relay that is still
/// parked keeps it.
pub(crate) fn take_held_swap(slot: &mut Option<String>, parked: bool) -> Option<String> {
    if parked {
        None
    } else {
        slot.take()
    }
}

/// How long a playlist request waits, after an unpark, for the restarted
/// reader's first publish before the relay answers 503. Under the main
/// player's 10 s `levelLoadingTimeOut`, so the player retries instead of
/// timing out.
const UNPARK_PUBLISH_TIMEOUT: Duration = Duration::from_secs(5);

/// `UNPARK_PUBLISH_TIMEOUT`, or `SN_UNPARK_PUBLISH_SECS` from the environment
/// when it holds a positive number of seconds. Read once.
fn unpark_publish_timeout() -> Duration {
    static SPAN: Lazy<Duration> = Lazy::new(|| {
        publish_timeout_from(std::env::var("SN_UNPARK_PUBLISH_SECS").ok().as_deref())
    });
    *SPAN
}

fn publish_timeout_from(secs: Option<&str>) -> Duration {
    secs.and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|s| *s > 0)
        .map(Duration::from_secs)
        .unwrap_or(UNPARK_PUBLISH_TIMEOUT)
}

/// What a playlist request came to at the parts origin.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PlaylistServe {
    /// The playlist to answer with.
    Playlist(String),
    /// No edge: the origin is off for this stream and the relay falls through
    /// to the plain upstream path.
    Inactive,
    /// A woken origin whose reader has not published yet. The relay answers
    /// 503 and the player asks again; the held window is never served, and
    /// the request never falls through to the upstream.
    NotReady,
}

/// A callback that re-signs a refused upstream playlist URL: given the stale
/// URL, it yields the fresh one.
pub(crate) type Resigner =
    Arc<dyn Fn(String) -> futures::future::BoxFuture<'static, Option<String>> + Send + Sync>;

/// A spawned task that is aborted when its handle is dropped, so a task never
/// outlives the reader (or the loop pass) that owns it.
struct AbortOnDrop<T>(JoinHandle<T>);

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Playlist polls after an unpark in which a 403 or 404 is read as an expired
/// signature rather than an offline stream.
const RESIGN_POLLS_AFTER_UNPARK: u32 = 3;

/// The reader's tally of failing playlist polls, and its one chance after an
/// unpark to re-sign a refused playlist before a refusal counts toward going
/// offline.
struct PlaylistHealth {
    /// Consecutive failing polls; reset by any good poll.
    fails: u32,
    /// Polls started by this reader.
    polls: u32,
    /// Whether a refusal in the first polls may still be answered by a re-sign.
    resign_left: bool,
}

impl PlaylistHealth {
    fn new(after_unpark: bool) -> Self {
        Self {
            fails: 0,
            polls: 0,
            resign_left: after_unpark,
        }
    }

    fn polled(&mut self) {
        self.polls = self.polls.saturating_add(1);
    }

    fn healthy(&mut self) {
        self.fails = 0;
    }

    /// Counts a failing poll and returns the run of failures so far.
    fn failed(&mut self) -> u32 {
        self.fails += 1;
        self.fails
    }

    /// Whether a refused poll should be answered by a re-sign instead of
    /// being counted: once per unpark, only in the first polls, only for a
    /// 403 or 404.
    fn wants_resign(&mut self, status: u16) -> bool {
        if self.resign_left && self.polls <= RESIGN_POLLS_AFTER_UNPARK && matches!(status, 403 | 404) {
            self.resign_left = false;
            return true;
        }
        false
    }
}

/// How far behind the relay's edge AVPlayer sits past the playlist's hold-back
/// (measured: a 0.936 s hold-back plays 1.44 to 1.71 s behind the edge, a 0.624 s
/// one 1.13 to 1.20 s).
const APPLE_BEHIND_OVER_HOLD: f64 = 0.55;
/// Seconds the parts origin's edge trails the broadcaster, by ear (the desktop's
/// `LL_EDGE_DELAY` in src/utils/latency.ts).
pub(crate) const LL_EDGE_DELAY_SECS: f64 = 0.3;
/// The automatic distance behind live on the parts path: the desktop's and
/// Android's `AUTO_GAP.ll` (src/utils/latency.ts), so every app rides the same.
const AUTO_LL_GAP_SECS: f64 = 1.5;

/// The origin's client, built once. `start` used to build a new one per stream
/// start, which rebuilt the TLS configuration (and on Linux re-parsed the CA
/// bundle) on every channel switch; a clone shares the pool and costs an Arc.
static HTTP_CLIENT: Lazy<Client> = Lazy::new(|| {
    Client::builder()
        .tcp_keepalive(Duration::from_secs(15))
        .pool_idle_timeout(Duration::from_secs(30))
        // No overall timeout: the in-progress segment GET is intentionally long-lived
        // (it streams for ~the segment duration). Per-read progress is what matters.
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .build()
        .expect("ll_origin http client")
});

fn http_client() -> Client {
    HTTP_CLIENT.clone()
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

    /// Bytes of a fragment that has started but not completed: a partial box
    /// in `buf`, or complete boxes (a `moof`) still waiting for their `mdat`.
    /// Non-zero at end of body means the transfer ended mid-fragment.
    fn pending(&self) -> usize {
        self.buf.len() + self.current.len()
    }

    /// Bytes of a fragment the body cut short: a box header whose declared
    /// size exceeds what arrived, or a `moof` still waiting for its `mdat`.
    /// Complete trailing boxes that are not media (an `emsg`, a `prft`) and
    /// sub-header scraps are not a cut fragment; flush() drops those. The
    /// distinction matters: a clean end of body with a cut fragment is worth
    /// a pickup, a clean end with a trailing box must not cost one.
    fn truncated_fragment_bytes(&self) -> usize {
        let partial_box = self.buf.len() >= 8 && {
            let size32 = u32::from_be_bytes([self.buf[0], self.buf[1], self.buf[2], self.buf[3]]) as usize;
            let declared = if size32 == 1 {
                if self.buf.len() >= 16 {
                    u64::from_be_bytes(self.buf[8..16].try_into().unwrap()) as usize
                } else {
                    usize::MAX
                }
            } else {
                size32
            };
            declared > self.buf.len()
        };
        let moof_waiting = last_box_type(&self.current) == Some(*b"moof");
        if partial_box || moof_waiting {
            self.buf.len() + self.current.len()
        } else {
            0
        }
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
            if let Some(pts) = cut_stamp(pkt) {
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
    video_pes_stamps(pkt).map(|(pts, _)| pts)
}

/// The timestamp the TS chunker cuts parts on. Desktop and Android cut on the
/// PTS: hls.js takes parts that wander a frame or two around `TS_PART_PTS`
/// (presentation order is not decode order). AVPlayer refuses a playlist whose
/// non-final parts fall under 85% of its PART-TARGET, so the iPhone cuts on the
/// decode timestamp, which gives every non-final part the same frame count.
#[cfg(not(target_os = "ios"))]
fn cut_stamp(pkt: &[u8]) -> Option<u64> {
    video_pes_pts(pkt)
}

#[cfg(target_os = "ios")]
fn cut_stamp(pkt: &[u8]) -> Option<u64> {
    video_pes_stamps(pkt).map(|(_, dts)| dts)
}

/// `(PTS, DTS)` of a video PES start; the DTS is the PTS when the PES carries
/// none (no reordering).
fn video_pes_stamps(pkt: &[u8]) -> Option<(u64, u64)> {
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
    // 33-bit timestamps packed across 5 bytes with marker bits (ISO 13818-1).
    let stamp = |b: &[u8]| {
        (((b[0] as u64 >> 1) & 0x07) << 30)
            | ((b[1] as u64) << 22)
            | (((b[2] as u64 >> 1) & 0x7F) << 15)
            | ((b[3] as u64) << 7)
            | ((b[4] as u64 >> 1) & 0x7F)
    };
    let pts = stamp(b);
    let dts = if pts_dts_flags == 3 && b.len() >= 10 {
        stamp(&b[5..])
    } else {
        pts
    };
    Some((pts, dts))
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

/// Until when the TS low-latency path is off regardless of the setting
/// (`playback_reload`: automatic reloads kept coming). CMAF still runs, as
/// with the setting off, since it has no other path.
static HOLD_OFF_UNTIL: Mutex<Option<std::time::Instant>> = Mutex::new(None);

/// Serve TS streams through the stable whole-segment path for `d`.
pub fn hold_off(d: Duration) {
    *HOLD_OFF_UNTIL.lock().unwrap_or_else(|e| e.into_inner()) = Some(std::time::Instant::now() + d);
}

fn held_off() -> bool {
    HOLD_OFF_UNTIL
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_some_and(|until| std::time::Instant::now() < until)
}

/// The viewer's target latency (None or a non-positive value is Auto). AVPlayer
/// reads it as the hold-back of the next playlist it loads.
/// The viewer's target latency in seconds, or None at Auto.
pub(crate) fn target_gap_secs() -> Option<f64> {
    let ms = TARGET_GAP_MS.load(Ordering::Relaxed);
    (ms > 0).then(|| ms as f64 / 1000.0)
}

pub fn set_target_gap(gap: Option<f32>) {
    let ms = gap.filter(|g| *g > 0.0).map(|g| (g as f64 * 1000.0).round() as u64).unwrap_or(0);
    TARGET_GAP_MS.store(ms, Ordering::Relaxed);
}

/// Seconds to add to (or take off) the default hold-back so AVPlayer rides the
/// viewer's target gap, or at Auto the same gap as the desktop and Android: never
/// under three part targets (LL-HLS: PART-HOLD-BACK SHOULD be at least 3x).
fn apple_gap_adjust(target: f64, hold_parts: u64) -> f64 {
    gap_adjust(TARGET_GAP_MS.load(Ordering::Relaxed), target, hold_parts)
}

fn gap_adjust(ms: u64, target: f64, hold_parts: u64) -> f64 {
    let gap = if ms == 0 { AUTO_LL_GAP_SECS } else { ms as f64 / 1000.0 };
    let wanted = (gap - LL_EDGE_DELAY_SECS - APPLE_BEHIND_OVER_HOLD).max(target * 3.0);
    wanted - target * hold_parts as f64
}

impl LlOrigin {
    pub fn new(max_segments: usize) -> Arc<Self> {
        Arc::new(Self {
            live_edge: Mutex::new(None),
            notify: Notify::new(),
            edge_version: AtomicU64::new(0),
            reader_task: Mutex::new(None),
            generation: AtomicU64::new(0),
            max_segments: AtomicUsize::new(max_segments),
            transmux: Mutex::new(None),
            cmaf_video: Mutex::new(None),
            retired: Mutex::new(VecDeque::new()),
            apple_part_target: Mutex::new(None),
            adapt_id: Mutex::new(String::new()),
            upstream: Mutex::new(None),
            parked: AtomicBool::new(false),
            fresh_after_unpark: AtomicBool::new(true),
            force_rebuild: AtomicBool::new(false),
            last_playlist_ms: AtomicU64::new(0),
            start_lock: tokio::sync::Mutex::new(()),
            closed: AtomicBool::new(false),
            resigner: Mutex::new(None),
            serving: AtomicUsize::new(0),
        })
    }

    /// Counts a playlist request as in flight until the guard drops.
    fn serving_guard(&self) -> ServeGuard<'_> {
        self.serving.fetch_add(1, Ordering::SeqCst);
        ServeGuard(&self.serving)
    }

    /// Install (or clear) how this origin's upstream URL is re-signed.
    pub(crate) fn set_resigner(&self, resigner: Option<Resigner>) {
        *self.resigner.lock().unwrap() = resigner;
    }

    /// Ask the installed resigner for a fresh URL in place of `stale`. `None`
    /// when no resigner is installed or it found nothing.
    async fn resign(&self, stale: &str) -> Option<String> {
        let resigner = { self.resigner.lock().unwrap().clone() };
        let resigner = resigner?;
        resigner(stale.to_string()).await
    }

    /// Make `url` the playlist an unparked reader of generation `gen` and any
    /// later unpark follow. False when that reader is no longer the current one.
    fn adopt_upstream(&self, gen: u64, url: String) -> bool {
        let _task = self.reader_task.lock().unwrap();
        if self.generation.load(Ordering::SeqCst) != gen {
            return false;
        }
        *self.upstream.lock().unwrap() = Some(url);
        true
    }

    /// Name the stream this origin serves, for `link_adapt`.
    pub(crate) fn set_adapt_id(&self, id: &str) {
        *self.adapt_id.lock().unwrap() = id.to_string();
    }

    fn adapt_id(&self) -> String {
        self.adapt_id.lock().unwrap().clone()
    }

    fn window(&self) -> usize {
        self.max_segments.load(Ordering::Relaxed)
    }

    /// Resize the live window. Growing takes effect as new segments arrive (the
    /// listed history never moves backwards); shrinking trims on the next push.
    pub(crate) fn set_window(&self, segments: usize) {
        self.max_segments.store(segments.max(1), Ordering::Relaxed);
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
    /// A reader that is no longer the current one (`gen` is stale) changes nothing.
    fn deactivate_offline(&self, reason: &str, gen: u64) {
        let task = self.reader_task.lock().unwrap();
        if self.generation.load(Ordering::SeqCst) != gen {
            return;
        }
        warn!("[LLOrigin] deactivating origin (upstream gone): {reason}");
        crate::services::ll_diagnostics::event(&format!(
            "\"ev\":\"o_offline\",\"reason\":{reason:?}"
        ));
        *self.live_edge.lock().unwrap() = None;
        *self.transmux.lock().unwrap() = None;
        *self.cmaf_video.lock().unwrap() = None;
        drop(task);
        self.wake_serves();
    }

    /// Stop any running reader and clear state. Called on stream stop / restart.
    pub fn stop(&self) {
        // Held to the end of the clears, so an `unpark` waiting on the lock
        // finds the origin stopped and starts nothing.
        let task = self.reader_task.lock().unwrap();
        self.stop_locked(task);
    }

    /// The stream this origin served is gone for good (a closed tile): stop it,
    /// and refuse every later start, install and unpark.
    pub fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.stop();
    }

    /// `stop()` unless the origin is parked: a parked origin is left as it is
    /// and false is returned, so a rendition switch cannot restart a relay that
    /// is deliberately idle. True when it stopped.
    pub fn stop_unless_parked(&self) -> bool {
        let task = self.reader_task.lock().unwrap();
        if self.parked.load(Ordering::SeqCst) {
            return false;
        }
        self.stop_locked(task);
        true
    }

    /// The stop itself, run with the `reader_task` lock held; the lock is
    /// released after the clears.
    fn stop_locked(&self, mut task: std::sync::MutexGuard<'_, Option<JoinHandle<()>>>) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        if let Some(h) = task.take() {
            h.abort();
        }
        *self.live_edge.lock().unwrap() = None;
        *self.transmux.lock().unwrap() = None;
        *self.cmaf_video.lock().unwrap() = None;
        self.retired.lock().unwrap().clear();
        *self.apple_part_target.lock().unwrap() = None;
        self.parked.store(false, Ordering::SeqCst);
        self.fresh_after_unpark.store(true, Ordering::SeqCst);
        self.force_rebuild.store(false, Ordering::SeqCst);
        *self.upstream.lock().unwrap() = None;
        self.last_playlist_ms.store(0, Ordering::SeqCst);
        drop(task);
        self.wake_serves();
    }

    /// No player asked for the playlist for `PARK_IDLE_MS`: stop downloading. The reader
    /// ends and the half-read in-progress segment is closed the way an abandoned
    /// tail is (stream_response): what arrived stays, the transmuxer drops its
    /// partial input and bridges the loss, and the segment is finished at the sum
    /// of its parts. So the playlist never shrinks under a client that read it, a
    /// paused player's blocking reloads get it back (serve_playlist_apple), and a
    /// reader restarted soon after (`unpark`, a gap too small to rebuild) carries on
    /// from a complete segment instead of an unfinishable one. A long park rebuilds
    /// at the live edge (plan_catch_up).
    pub fn park(&self) -> bool {
        let mut task = self.reader_task.lock().unwrap();
        if !self.is_active() || self.parked.swap(true, Ordering::SeqCst) {
            return false;
        }
        self.finish_park(&mut task);
        true
    }

    /// The reader's own park, for the reader started as generation `gen`. It
    /// parks only while that reader is still the current one and no player has
    /// asked since the idle span ran out; a request that lands after the
    /// reader decided leaves the origin running.
    fn park_if_idle(&self, gen: u64) -> bool {
        self.park_if_idle_at(gen, mono_ms())
    }

    fn park_if_idle_at(&self, gen: u64, now: u64) -> bool {
        let mut task = self.reader_task.lock().unwrap();
        if self.generation.load(Ordering::SeqCst) != gen
            || !self.is_active()
            || self.parked.swap(true, Ordering::SeqCst)
        {
            return false;
        }
        if !self.idle_park_due_at(now) {
            self.parked.store(false, Ordering::SeqCst);
            return false;
        }
        self.finish_park(&mut task);
        true
    }

    /// The park itself, run with the `reader_task` lock held and `parked` set:
    /// retire the reader, then close the half-read tail.
    fn finish_park(&self, task: &mut Option<JoinHandle<()>>) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        if let Some(h) = task.take() {
            h.abort();
        }
        if let Some(t) = self.transmux.lock().unwrap().as_mut() {
            t.drop_partial_input();
        }
        self.close_tail();
        // A parked origin serves the window it holds, however recently it was
        // woken before.
        self.fresh_after_unpark.store(true, Ordering::SeqCst);
        self.wake_serves();
        info!("[LLOrigin] parked (player paused)");
    }

    /// A player asked for the playlist. The request is noted first, so a reader
    /// deciding to park sees it; `wakes` is whether this request is from a
    /// player that is loading and wants a parked origin reading again.
    pub fn wake_for_request(self: &Arc<Self>, wakes: bool) {
        self.note_playlist_request();
        if wakes && self.is_parked() {
            self.unpark();
        }
    }

    /// Restart a parked reader. The held edge stays in place so the origin never
    /// reads as inactive, but a playlist request waits for the reader's first
    /// publish (`fresh_after_unpark`) rather than being served that stale window.
    pub fn unpark(self: &Arc<Self>) -> bool {
        let mut task = self.reader_task.lock().unwrap();
        if self.closed.load(Ordering::SeqCst) || !self.parked.load(Ordering::SeqCst) {
            return false;
        }
        self.parked.store(false, Ordering::SeqCst);
        self.note_playlist_request();
        let Some(url) = self.upstream.lock().unwrap().clone().filter(|_| self.is_active()) else {
            return false;
        };
        let gen = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.fresh_after_unpark.store(false, Ordering::SeqCst);
        *task = Some(tokio::spawn(run_reader(self.clone(), url, http_client(), gen, true)));
        info!("[LLOrigin] unparked; reader back at the live edge");
        true
    }

    /// Drop the held window and rebuild it from the upstream's live edge, the
    /// rebuild a park and unpark gives, parked or not. The current reader is
    /// replaced by one that rebuilds before it publishes; a playlist request
    /// waits for that publish (`fresh_after_unpark`) and is never handed the
    /// window held before. False when the origin is closed, inactive or has no
    /// upstream to read.
    pub fn rebuild_at_edge(self: &Arc<Self>) -> bool {
        let mut task = self.reader_task.lock().unwrap();
        if self.closed.load(Ordering::SeqCst) || !self.is_active() {
            return false;
        }
        let Some(url) = self.upstream.lock().unwrap().clone() else {
            return false;
        };
        self.parked.store(false, Ordering::SeqCst);
        self.note_playlist_request();
        let gen = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        if let Some(h) = task.take() {
            h.abort();
        }
        if let Some(t) = self.transmux.lock().unwrap().as_mut() {
            t.reset_assembly();
        }
        self.force_rebuild.store(true, Ordering::SeqCst);
        self.fresh_after_unpark.store(false, Ordering::SeqCst);
        *task = Some(tokio::spawn(run_reader(self.clone(), url, http_client(), gen, true)));
        info!("[LLOrigin] page asked for a rebuild; reader back at the live edge");
        true
    }

    /// Whether the reader started as generation `gen` is to rebuild the window
    /// from the live edge. Taken under the reader lock, so a reader that a newer
    /// start or rebuild already replaced never takes the request meant for its
    /// successor.
    fn take_force_rebuild(&self, gen: u64) -> bool {
        let _task = self.reader_task.lock().unwrap();
        self.generation.load(Ordering::SeqCst) == gen && self.force_rebuild.swap(false, Ordering::SeqCst)
    }

    /// The reader started as generation `gen` has brought the window up to the
    /// upstream's live edge: playlist requests held since the unpark go out.
    /// Nothing changes for a reader that is no longer the current one.
    fn mark_fresh(&self, gen: u64) {
        // Under the reader lock, like `adopt_upstream`, so a stale reader's
        // check cannot pass and then store after a newer unpark cleared the flag.
        let _task = self.reader_task.lock().unwrap();
        if self.generation.load(Ordering::SeqCst) != gen {
            return;
        }
        self.fresh_after_unpark.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    /// Waits up to `wait` for the first publish after an unpark. True once the
    /// window is fresh or the edge is gone (the caller answers inactive), false
    /// when `wait` runs out first, or when the origin parked while this request
    /// waited: a park marks the window fresh only so a paused player is handed
    /// what the origin holds, and a request that waited for a new reader must
    /// not be handed that.
    async fn wait_fresh(&self, wait: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + wait;
        let mut waited = false;
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            // Registered before the check, so a publish between the two is not lost.
            notified.as_mut().enable();
            if !self.is_active() {
                return true;
            }
            if self.fresh_after_unpark.load(Ordering::SeqCst) {
                return !(waited && self.is_parked());
            }
            waited = true;
            let now = tokio::time::Instant::now();
            if now >= deadline || tokio::time::timeout(deadline - now, notified).await.is_err() {
                return false;
            }
        }
    }

    pub fn is_parked(&self) -> bool {
        self.parked.load(Ordering::SeqCst)
    }

    /// Marks the origin parked without needing a live edge, for tests outside
    /// this module that only read the flag.
    #[cfg(test)]
    pub(crate) fn mark_parked_for_test(&self) {
        self.parked.store(true, Ordering::SeqCst);
    }

    /// Gives the origin a one-segment live edge and an upstream, for tests
    /// outside this module that exercise the wake path.
    #[cfg(test)]
    pub(crate) fn arm_for_test(&self, upstream: &str) {
        let mut segments = VecDeque::new();
        segments.push_back(make_segment(1, None, true, vec![(vec![1], 1.0)]));
        *self.live_edge.lock().unwrap() = Some(LiveEdge {
            init_url: String::new(),
            init_bytes: None,
            container: Container::Ts,
            target_duration: 2,
            part_target: PART_TARGET,
            segments,
        });
        *self.upstream.lock().unwrap() = Some(upstream.to_string());
    }

    /// Whether a reader task is running, for tests outside this module.
    #[cfg(test)]
    pub(crate) fn has_reader_for_test(&self) -> bool {
        self.reader_task.lock().unwrap().is_some()
    }

    /// The in-progress tail at a park: finished at what it holds, or dropped when
    /// it is still a bare shell (listed only as the preload hint, so no listed
    /// segment disappears).
    fn close_tail(&self) {
        let tail = {
            let mut g = self.live_edge.lock().unwrap();
            let Some(edge) = g.as_mut() else { return };
            let (pop, finish) = match edge.segments.back() {
                Some(s) if !s.complete && s.parts.is_empty() => (edge.segments.len() > 1, None),
                Some(s) if !s.complete => (false, Some(s.sn)),
                _ => (false, None),
            };
            if pop {
                edge.segments.pop_back();
            }
            finish
        };
        if let Some(sn) = tail {
            self.finish_segment(sn, self.generation.load(Ordering::SeqCst));
        }
    }

    /// A player asked for the playlist (either form).
    fn note_playlist_request(&self) {
        self.note_playlist_request_at(mono_ms());
    }

    fn note_playlist_request_at(&self, ms: u64) {
        self.last_playlist_ms.store(ms, Ordering::SeqCst);
    }

    /// No player has asked for the playlist for `PARK_IDLE_MS` as of `now`, and
    /// none is waiting on an answer.
    fn idle_park_due_at(&self, now: u64) -> bool {
        self.serving.load(Ordering::SeqCst) == 0
            && idle_for_park(self.last_playlist_ms.load(Ordering::SeqCst), now, park_idle_ms())
    }

    /// A player on the whole-segment path asked for the playlist, which this
    /// origin does not serve while inactive.
    pub(crate) fn note_plain_request(&self) {
        self.note_playlist_request();
    }

    /// Nothing is being read for this origin's player: the reader is parked, or
    /// the origin is inactive (whole-segment path) and no player has asked for
    /// the playlist for the park span.
    pub(crate) fn parked_or_idle(&self) -> bool {
        self.parked_or_idle_at(mono_ms())
    }

    fn parked_or_idle_at(&self, now: u64) -> bool {
        self.is_parked() || (!self.is_active() && self.idle_park_due_at(now))
    }

    /// Probe the upstream and, if it's a low-latency broadcast we can serve, build the
    /// initial live edge (backfilling a couple of complete segments) and spawn the
    /// streaming reader. Returns `{active, has_prefetch}`: `active` when the origin took
    /// over (LL channel), `has_prefetch` whenever the upstream carries PREFETCH hints
    /// (so the relay can still flag a low-latency channel for the promotion fallback even
    /// when the origin declined). Awaited from stream start so the player can read the
    /// result before constructing hls.js.
    pub async fn start(self: Arc<Self>, upstream_playlist_url: String) -> StartOutcome {
        if self.closed.load(Ordering::SeqCst) {
            return StartOutcome::default();
        }
        // Stopped before the queue, so a start already running ends early at
        // its next check and a stop that lands while this one waits is seen.
        self.stop();
        let requested = self.generation.load(Ordering::SeqCst);
        let _start = self.start_lock.lock().await;
        if self.generation.load(Ordering::SeqCst) != requested {
            return StartOutcome::default();
        }
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
        let held = held_off();
        if held && !DISABLED.load(Ordering::Relaxed) {
            warn!("[LLOrigin] low latency held off after repeated reloads; using the stable path for TS");
        }
        let force_disabled = DISABLED.load(Ordering::Relaxed) || held;
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
        if self.start_superseded(gen) {
            return StartOutcome::default();
        }
        let base = base_of(&upstream_playlist_url);
        let up = parse_upstream(&text, &base);
        let has_prefetch = !up.prefetch.is_empty();
        UPSTREAM_HAS_PREFETCH.store(has_prefetch, Ordering::SeqCst);
        let inactive = StartOutcome {
            active: false,
            has_prefetch,
        };

        // Ad-free playback owns the playlist: this origin synthesizes its own and
        // the relay serves it before the ad filter is ever reached, so the two
        // cannot both run. The kill switch below is no help here, because CMAF
        // (HEVC/AV1) deliberately ignores it, which is exactly where ads would slip
        // through with the feature reporting itself as on.
        #[cfg(mobile)]
        if crate::services::ad_bypass::active() {
            debug!("[LLOrigin] ad-free playback owns this stream; origin inactive");
            return inactive;
        }

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
            if self.start_superseded(gen) {
                return inactive;
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
            let fetched = client.get(&url).timeout(FETCH_TIMEOUT).send().await.ok();
            if self.start_superseded(gen) {
                return inactive;
            }
            match fetched {
                Some(resp) => {
                    // Timed from the response, so the connection's setup (and
                    // the first one's handshake) is left out.
                    let t0 = std::time::Instant::now();
                    let bytes = resp.bytes().await.map(|b| b.to_vec()).unwrap_or_default();
                    // A whole published segment at the link's full rate: the
                    // parts path's best measure of it, and the start pick's.
                    crate::services::link_adapt::note_throughput(
                        &self.adapt_id(),
                        bytes.len() as u64,
                        t0.elapsed().as_secs_f64(),
                    );
                    let parts = self.convert_parts(split_complete(&bytes, container), gen);
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

        let edge = LiveEdge {
            init_url,
            init_bytes,
            container,
            // Declare the real ~2s segment size, NOT Twitch's inflated TARGETDURATION:6.
            // hls.js uses targetduration for reload cadence and tune-in goal math; an
            // inflated value makes it mis-time part requests.
            target_duration: TARGET_DURATION,
            part_target: PART_TARGET,
            segments,
        };
        let installed = self.install_started(gen, edge, upstream_playlist_url.clone(), || {
            tokio::spawn(run_reader(self.clone(), upstream_playlist_url, client, gen, false))
        });
        if !installed {
            debug!("[LLOrigin] start superseded by a stop; nothing installed");
            return inactive;
        }
        StartOutcome {
            active: true,
            has_prefetch: true,
        }
    }

    /// Whether a stop has superseded the start that began as generation `gen`.
    /// When it has, the transmuxer and CMAF track the start prepared are cleared
    /// so nothing of it outlives the stop.
    fn start_superseded(&self, gen: u64) -> bool {
        if self.generation.load(Ordering::SeqCst) == gen {
            return false;
        }
        *self.transmux.lock().unwrap() = None;
        *self.cmaf_video.lock().unwrap() = None;
        true
    }

    /// Take over for a start that probed as generation `gen`: set the live edge
    /// and upstream, note the request and store the reader `spawn` makes, all under
    /// the `reader_task` lock that `stop()` also takes. A start that a `stop()`
    /// superseded since it began installs nothing (the transmuxer and CMAF track it
    /// prepared are cleared) and returns false.
    fn install_started(
        &self,
        gen: u64,
        edge: LiveEdge,
        upstream: String,
        spawn: impl FnOnce() -> JoinHandle<()>,
    ) -> bool {
        let mut task = self.reader_task.lock().unwrap();
        if self.generation.load(Ordering::SeqCst) != gen || self.closed.load(Ordering::SeqCst) {
            *self.transmux.lock().unwrap() = None;
            *self.cmaf_video.lock().unwrap() = None;
            return false;
        }
        let container = edge.container;
        *self.live_edge.lock().unwrap() = Some(edge);
        // Query string stripped: it is most of the length and can carry signed
        // playback params that have no business in a log people send around.
        info!(
            "[LLOrigin] activated ({container:?} low-latency origin) for {}",
            upstream.split('?').next().unwrap_or("")
        );

        *self.upstream.lock().unwrap() = Some(upstream);
        self.note_playlist_request();
        *task = Some(spawn());
        true
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

/// `after_unpark`: this reader replaces one a park stopped, so a refused first
/// poll may be an expired signature (see `PlaylistHealth`).
async fn run_reader(
    origin: Arc<LlOrigin>,
    mut upstream_playlist_url: String,
    client: Client,
    gen: u64,
    after_unpark: bool,
) {
    let mut base = base_of(&upstream_playlist_url);
    // Connection to the NEXT in-progress segment, opened by `preopen_next` while
    // the current one was still streaming. Consumed by the fast path below;
    // discarded whenever the world doesn't match (the poll path then re-syncs).
    // Aborted when dropped, so no pass of this loop and no end of this reader
    // leaves a connection opening behind.
    let mut preopened: Option<(
        u64,
        AbortOnDrop<Option<(Response, Option<String>, String)>>,
    )> = None;
    // Consecutive failing playlist polls; reset on any good poll. Drives offline
    // deactivation so a permanently-dead upstream stops being served forever.
    let mut health = PlaylistHealth::new(after_unpark);
    // The page asked for the window to be rebuilt from the live edge: stays set
    // until the rebuild has replaced the window, across retried polls.
    let mut forced = origin.take_force_rebuild(gen);
    loop {
        if gen != origin.generation.load(Ordering::SeqCst) {
            return;
        }
        // The player stopped asking (paused): stop reading until it asks again.
        if origin.idle_park_due_at(mono_ms()) {
            drop(preopened.take());
            if origin.park_if_idle(gen) {
                return;
            }
            // A request landed after the idle read: this reader keeps running.
            // A superseded or inactive one ends at the next pass.
            if !origin.is_active() {
                return;
            }
            continue;
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
                match tokio::time::timeout(PREOPEN_WAIT, &mut handle.0).await {
                    Ok(Ok(Some((resp, pdt, seg_url)))) => {
                        match origin.push_shell(sn, pdt.clone(), gen) {
                            Some(true) => {
                                crate::services::ll_diagnostics::event(&format!(
                                    "\"ev\":\"o_shell\",\"path\":\"fast\",\"sn\":{sn},\"pdt\":{}",
                                    pdt.as_deref()
                                        .map(|p| format!("\"{p}\""))
                                        .unwrap_or_else(|| "null".into())
                                ));
                                origin.wake_serves();
                                let next = AbortOnDrop(tokio::spawn(preopen_next(
                                    client.clone(),
                                    upstream_playlist_url.clone(),
                                    sn + 1,
                                )));
                                stream_response(
                                    &origin,
                                    resp,
                                    sn,
                                    gen,
                                    Some((&client, seg_url.as_str())),
                                )
                                .await;
                                if !origin.finish_segment(sn, gen) {
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
                    Err(_) => handle.0.abort(), // not ready in time: poll path
                }
            }
        }

        // Re-fetch the playlist to find the current in-progress segment. reqwest
        // returns Ok for any HTTP status, so a 403/404 (stream offline, or an
        // expired usher-signed URL) would otherwise parse to an empty playlist and
        // loop forever; inspect the status and count failures toward deactivation.
        health.polled();
        let text = match client
            .get(&upstream_playlist_url)
            .timeout(FETCH_TIMEOUT)
            .send()
            .await
        {
            Ok(r) => {
                let status = r.status();
                if !status.is_success() {
                    // The first polls after an unpark may meet a signature that
                    // expired while parked: re-sign once and poll again before
                    // the refusal counts toward going offline.
                    if health.wants_resign(status.as_u16()) {
                        warn!("[LLOrigin] reader playlist HTTP {status} right after an unpark; re-signing");
                        if let Some(fresh) = origin.resign(&upstream_playlist_url).await {
                            if !origin.adopt_upstream(gen, fresh.clone()) {
                                return;
                            }
                            base = base_of(&fresh);
                            upstream_playlist_url = fresh;
                            continue;
                        }
                    }
                    let fails = health.failed();
                    warn!("[LLOrigin] reader playlist HTTP {status} ({fails})");
                    if fails >= PLAYLIST_OFFLINE_LIMIT {
                        origin.deactivate_offline(&format!("playlist HTTP {status}"), gen);
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    continue;
                }
                r.text().await.unwrap_or_default()
            }
            Err(e) => {
                let fails = health.failed();
                warn!("[LLOrigin] reader playlist fetch failed: {e} ({fails})");
                if fails >= PLAYLIST_OFFLINE_LIMIT {
                    origin.deactivate_offline("playlist fetch repeatedly failing", gen);
                    return;
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
                continue;
            }
        };
        let up = parse_upstream(&text, &base);
        let last_published_sn = match up.published.last() {
            Some((sn, _, _)) => {
                health.healthy(); // a healthy poll
                *sn
            }
            None => {
                if health.failed() >= PLAYLIST_OFFLINE_LIMIT {
                    origin.deactivate_offline("playlist has no segments", gen);
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
        if already && !forced {
            // The window already reaches the upstream's in-progress segment.
            origin.mark_fresh(gen);
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
        if forced || newest_in_window + 1 < inprogress_sn {
            let oldest_published = match up.published.first() {
                Some((sn, _, _)) => *sn,
                None => unreachable!("published is non-empty (checked above)"),
            };
            let (rebuild, fetch) =
                catch_up_plan(forced, newest_in_window, oldest_published, inprogress_sn);
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
                match fetch_published(&client, sn, seg_pdt.clone(), url, &origin, gen).await {
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
                        while edge.segments.len() > origin.window() {
                            if let Some(s) = edge.segments.pop_front() {
                                origin.retire([s]);
                            }
                        }
                    }
                    None => return,
                }
            }
            origin.wake_serves();
            forced = false;
        }
        // The window now runs up to the upstream's in-progress segment (caught
        // up, rebuilt, or already there): a request held since an unpark may go.
        origin.mark_fresh(gen);

        // Create the in-progress segment shell and stream its parts in, preopening
        // the following segment's connection so the next boundary is seamless.
        match origin.push_shell(inprogress_sn, pdt.clone(), gen) {
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

        let next = AbortOnDrop(tokio::spawn(preopen_next(
            client.clone(),
            upstream_playlist_url.clone(),
            inprogress_sn + 1,
        )));
        stream_segment(&origin, &client, &inprogress_url, inprogress_sn, gen).await;

        if !origin.finish_segment(inprogress_sn, gen) {
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

/// Decide how the reader brings a stale window up to date before the next
/// in-progress segment. Returns `(clear_window_first, sns_to_fetch)`.
///
/// The fetch range is always consecutive and ends at `inprogress_sn - 1`, and
/// when not rebuilding it starts right after `window_newest`, so appending the
/// fetched segments keeps the window contiguous at every intermediate render.
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

/// `plan_catch_up`, except that a `forced` plan (the page asked for a rebuild)
/// treats no segment as held: the window is always rebuilt from the live edge,
/// with the same minimal backfill, however current the held window looks.
fn catch_up_plan(
    forced: bool,
    window_newest: u64,
    oldest_published: u64,
    inprogress_sn: u64,
) -> (bool, std::ops::Range<u64>) {
    if forced {
        let start = inprogress_sn
            .saturating_sub(REBUILD_BACKFILL)
            .max(oldest_published);
        (true, start..inprogress_sn)
    } else {
        plan_catch_up(window_newest, oldest_published, inprogress_sn)
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
    gen: u64,
) -> Option<Segment> {
    let t0 = std::time::Instant::now();
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
    crate::services::link_adapt::note_throughput(&origin.adapt_id(), bytes.len() as u64, t0.elapsed().as_secs_f64());
    let parts = origin.convert_parts(split_complete(&bytes, origin.container()), gen);
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
    // On a phone that has been stalling, the player keeps more distance from
    // the edge (`link_adapt`), so a late tail can still arrive in time: the
    // deadline and the silence limit widen with that cushion. Zero otherwise.
    let adapt_id = origin.adapt_id();
    let cushion = crate::services::link_adapt::extra_cushion(&adapt_id);
    let deadline_secs = origin.expected_segment_secs() + SEGMENT_DEADLINE_GRACE + cushion;
    let midstream_timeout = MIDSTREAM_CHUNK_TIMEOUT + Duration::from_secs_f64(cushion / 2.0);
    let mut received: u64 = 0;
    let mut resp = first;
    let mut resumes_left: u8 = if resume.is_some() { RESUME_ATTEMPTS } else { 0 };
    // Bytes of a resumed full-body reply still to discard (see skip_prefix).
    let mut skip: u64 = 0;
    // A clean-but-short body gets exactly one pickup (see ended_short).
    let mut short_pickup_tried = false;
    // Fingerprint of what was streamed so far, and of the prefix a full-body
    // pickup replays, compared once the skip completes.
    let mut held_tail: Vec<u8> = Vec::new();
    let mut replay_tail: Vec<u8> = Vec::new();
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
                    crate::services::link_adapt::note_tail_loss(&adapt_id);
                    break;
                }
                midstream_timeout.min(Duration::from_secs_f64(remaining))
            }
        };
        // What went wrong this iteration, if anything; None = keep reading.
        let failure: Option<String> = match tokio::time::timeout(limit, resp.chunk()).await {
            Ok(Ok(Some(bytes))) => {
                first_chunk_at.get_or_insert_with(tokio::time::Instant::now);
                let raw: &[u8] = &bytes;
                let skipping = skip > 0;
                let before = skip;
                let rest = skip_prefix(&mut skip, raw);
                if skipping {
                    // The bytes just discarded are the pickup's replay of
                    // what we hold; remember their tail for the check.
                    let n = (before - skip) as usize;
                    remember_tail(&mut replay_tail, &raw[..n]);
                    if skip == 0 && replay_tail != held_tail {
                        warn!("[LLOrigin] sn {sn}: pickup replayed different bytes than held at {received}; abandoning the tail");
                        crate::services::ll_diagnostics::event(&format!(
                            "\"ev\":\"o_abandon\",\"sn\":{sn},\"why\":\"pickup prefix mismatch\",\"received\":{received},\"parts\":{},\"pending\":0",
                            origin.parts_held(sn)
                        ));
                        break;
                    }
                }
                if rest.is_empty() {
                    continue;
                }
                remember_tail(&mut held_tail, rest);
                received += rest.len() as u64;
                for (part, dur) in chunker.push(rest) {
                    let Some((part, dur)) = origin.convert_part(part, dur, gen) else {
                        continue;
                    };
                    if !origin.append_part(sn, part, dur, gen) {
                        return; // edge gone
                    }
                    origin.wake_serves();
                }
                None
            }
            Ok(Ok(None)) if skip > 0 => {
                // The pickup body ended before reaching the bytes we lack.
                Some(format!("pickup body ended {skip} bytes short"))
            }
            Ok(Ok(None)) if chunker.truncated_fragment_bytes() > 0 => {
                // A clean end of body in the middle of a fragment: the CDN
                // closed the in-progress object early while the segment kept
                // growing. Treated like a stalled transfer, so the tail is
                // picked up instead of dropped.
                Some(format!(
                    "body ended mid-fragment ({} bytes cut)",
                    chunker.truncated_fragment_bytes()
                ))
            }
            Ok(Ok(None)) => {
                let held = origin.held_secs(sn);
                let expected = origin.expected_segment_secs();
                // CMAF only: a TS segment's final part is still inside the
                // chunker here (flush() cuts it after the loop), so its held
                // duration always reads one part short at end of body and a
                // pickup would fire on healthy segments (seen on caseoh_).
                if chunker.is_cmaf() && ended_short(held, expected) {
                    let parts = origin.parts_held(sn);
                    if !short_pickup_tried {
                        short_pickup_tried = true;
                        if let Some((client, url)) = resume {
                            let why = format!("body ended after {held:.2}s of {expected:.1}s ({parts} parts)");
                            if let Some((r2, to_skip)) =
                                range_resume(client, url, received, sn, &why, SHORT_PICKUP_TIMEOUT).await
                            {
                                resp = r2;
                                skip = to_skip;
                                replay_tail.clear();
                                continue;
                            }
                        }
                    }
                    // Still short after the pickup (or none possible): the
                    // segment is what it is. Recorded so a capture can tell a
                    // genuinely short segment from a lost tail.
                    info!("[LLOrigin] sn {sn} finished short: {held:.2}s of {expected:.1}s ({parts} parts, {received} bytes)");
                    crate::services::ll_diagnostics::event(&format!(
                        "\"ev\":\"o_short\",\"sn\":{sn},\"held\":{held:.3},\"expected\":{expected:.3},\"parts\":{parts},\"received\":{received}"
                    ));
                }
                completed = true;
                break;
            }
            Ok(Err(e)) => Some(format!("read error: {e}")),
            Err(_) => Some("read timed out".to_string()),
        };
        let Some(why) = failure else {
            continue;
        };
        if resumes_left > 0 {
            resumes_left -= 1;
            if let Some((client, url)) = resume {
                // A pickup's timeout covers its whole body. On a phone the tail
                // may take longer than 1.5 s to arrive, so it gets whatever is
                // left of the delivery deadline.
                let budget = match first_chunk_at {
                    Some(t0) if crate::services::link_adapt::phone_rules() => {
                        (deadline_secs - t0.elapsed().as_secs_f64()).max(1.5)
                    }
                    _ => 1.5,
                };
                if let Some((r2, to_skip)) =
                    range_resume(client, url, received, sn, &why, Duration::from_secs_f64(budget)).await
                {
                    resp = r2;
                    skip = to_skip;
                    replay_tail.clear();
                    continue;
                }
            }
        }
        let parts = origin.parts_held(sn);
        warn!(
            "[LLOrigin] in-progress {why} for sn {sn} after {received} bytes / {parts} parts; abandoning the tail"
        );
        crate::services::link_adapt::note_tail_loss(&adapt_id);
        crate::services::ll_diagnostics::event(&format!(
            "\"ev\":\"o_abandon\",\"sn\":{sn},\"why\":\"{}\",\"received\":{received},\"parts\":{parts},\"pending\":{}",
            why.replace('"', "'"),
            chunker.truncated_fragment_bytes()
        ));
        break;
    }
    if let Some((tail, dur)) = chunker.flush() {
        if let Some((tail, dur)) = origin.convert_part(tail, dur, gen) {
            if origin.append_part(sn, tail, dur, gen) {
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
        origin.drop_partial_input_if_current(gen);
    }
}

/// Byte-exact pickup of a stalled segment transfer on a fresh connection.
/// Returns the response and how many leading bytes of it to discard: 0 for
/// a 206 that continues at `received`, `received` for a 200. The CDN answers
/// a range on a still-growing object with 200 far more often than 206
/// (15 of 20 pickups in one release log), and refusing those threw away
/// tails that were fully retrievable: the body restarts at byte 0, so
/// dropping the bytes already held keeps the stream byte-exact. Anything
/// else is a refusal and falls back to abandoning.
async fn range_resume(
    client: &Client,
    url: &str,
    received: u64,
    sn: u64,
    why: &str,
    timeout: Duration,
) -> Option<(Response, u64)> {
    let resp = client
        .get(url)
        .header(reqwest::header::RANGE, format!("bytes={received}-"))
        .timeout(timeout)
        .send()
        .await
        .ok()?;
    let status = resp.status();
    if status == reqwest::StatusCode::PARTIAL_CONTENT || status == reqwest::StatusCode::OK {
        let skip = if status == reqwest::StatusCode::OK { received } else { 0 };
        info!(
            "[LLOrigin] sn {sn}: {why} at byte {received}; resumed on a fresh connection ({status}, skipping {skip} bytes)"
        );
        crate::services::ll_diagnostics::event(&format!(
            "\"ev\":\"o_resume\",\"sn\":{sn},\"at\":{received},\"status\":{},\"skip\":{skip}",
            status.as_u16()
        ));
        Some((resp, skip))
    } else {
        warn!(
            "[LLOrigin] sn {sn}: range resume refused ({})",
            resp.status()
        );
        None
    }
}

impl LlOrigin {
    /// Append the in-progress segment shell. Returns `None` if the edge is gone
    /// or the reader started as generation `gen` is no longer the current one,
    /// `Some(false)` if appending would break window contiguity (refused; the
    /// reader's catch-up should make that impossible), `Some(true)` on success.
    fn push_shell(&self, sn: u64, pdt: Option<String>, gen: u64) -> Option<bool> {
        let mut g = self.live_edge.lock().unwrap();
        if self.generation.load(Ordering::SeqCst) != gen {
            return None;
        }
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
        while edge.segments.len() > self.window() {
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
    /// Returns false if the edge is gone or the reader started as generation
    /// `gen` is no longer the current one.
    /// Media seconds currently held for `sn`, from its parts' measured
    /// durations (0 when the edge or segment is gone).
    fn held_secs(&self, sn: u64) -> f64 {
        let g = self.live_edge.lock().unwrap();
        g.as_ref()
            .and_then(|edge| edge.segments.iter().find(|s| s.sn == sn))
            .map(|s| s.parts.iter().map(|p| p.duration).sum())
            .unwrap_or(0.0)
    }

    /// Parts currently held for `sn` (0 when the edge or segment is gone).
    fn parts_held(&self, sn: u64) -> usize {
        let g = self.live_edge.lock().unwrap();
        g.as_ref()
            .and_then(|edge| edge.segments.iter().find(|s| s.sn == sn))
            .map(|s| s.parts.len())
            .unwrap_or(0)
    }

    fn finish_segment(&self, sn: u64, gen: u64) -> bool {
        let mut g = self.live_edge.lock().unwrap();
        if self.generation.load(Ordering::SeqCst) != gen {
            return false;
        }
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
    /// On the transmux path the published duration is the transmuxer's decode-
    /// timeline measurement (each part runs from where the previous declared
    /// part ended, so final-sample estimates never sum into drift), NOT `dur`
    /// from the chunker: the chunker measures
    /// presentation-timestamp deltas at its cut points, which B-frame arrival
    /// order systematically inflates by tens of milliseconds per segment.
    /// hls.js sums the declared durations to place fragments, so that bias
    /// compounds until its part lookup drifts past tolerance and it re-fetches
    /// parts it already buffered (mid-GOP rewrites at the playhead = visible
    /// artifacting). Sample-measured durations keep playlist arithmetic glued
    /// to buffer reality.
    ///
    /// The generation is checked inside the transmux lock: a reader that a
    /// stop, park or newer start has retired gets `None` and never feeds the
    /// transmuxer a newer start owns.
    fn convert_part(&self, bytes: Vec<u8>, dur: f64, gen: u64) -> Option<(Vec<u8>, f64)> {
        let mut g = self.transmux.lock().unwrap();
        if self.generation.load(Ordering::SeqCst) != gen {
            return None;
        }
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

    /// Drop the transmuxer's half-assembled input for the reader started as
    /// generation `gen`. The generation is checked inside the transmux lock, so
    /// a reader that a stop, park or newer start has retired leaves a newer
    /// start's transmuxer alone. True when the partial input was dropped.
    fn drop_partial_input_if_current(&self, gen: u64) -> bool {
        let mut g = self.transmux.lock().unwrap();
        if self.generation.load(Ordering::SeqCst) != gen {
            return false;
        }
        match g.as_mut() {
            Some(t) => {
                t.drop_partial_input();
                true
            }
            None => false,
        }
    }

    /// Convert a complete segment's split parts, pairing each published part
    /// with its duration: sample-measured on the transmux path, an even
    /// `TARGET_DURATION/count` split on the raw path (unchanged behavior — raw
    /// backfill durations are normalized so they sum to the segment exactly).
    fn convert_parts(&self, parts: Vec<Vec<u8>>, gen: u64) -> Vec<(Vec<u8>, f64)> {
        let even = TARGET_DURATION as f64 / parts.len().max(1) as f64;
        parts
            .into_iter()
            .filter_map(|p| self.convert_part(p, even, gen))
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
    /// false if the edge is gone or the reader started as generation `gen` is no
    /// longer the current one.
    fn append_part(&self, sn: u64, bytes: Vec<u8>, duration: f64, gen: u64) -> bool {
        let mut g = self.live_edge.lock().unwrap();
        if self.generation.load(Ordering::SeqCst) != gen {
            return false;
        }
        match g.as_mut() {
            Some(edge) => {
                if let Some(seg) = edge.segments.iter_mut().find(|s| s.sn == sn) {
                    seg.parts.push(Part {
                        duration,
                        bytes: Arc::new(bytes),
                    });
                    if crate::services::relay_probe::is_active() {
                        if let Some(start) = seg.pdt.as_deref().and_then(pdt_millis) {
                            let held: f64 = seg.parts.iter().map(|p| p.duration).sum();
                            crate::services::relay_probe::note_edge(
                                sn,
                                seg.parts.len() - 1,
                                start + (held * 1000.0).round() as i64,
                            );
                        }
                    }
                }
                true
            }
            None => false,
        }
    }
}

/// An RFC3339 PROGRAM-DATE-TIME as Unix milliseconds.
fn pdt_millis(pdt: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(pdt)
        .ok()
        .map(|t| t.timestamp_millis())
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
    /// A hold that runs long counts from when it answers as well as when it
    /// began, so it never reads as an idle stretch.
    pub async fn serve_playlist(&self, msn: Option<u64>, part: Option<u64>) -> Option<String> {
        match self.serve_playlist_checked(msn, part).await {
            PlaylistServe::Playlist(pl) => Some(pl),
            PlaylistServe::Inactive | PlaylistServe::NotReady => None,
        }
    }

    /// `serve_playlist`, telling an origin that is not ready (woken, its reader
    /// not yet published) from one that is inactive: the relays answer the first
    /// with a retryable error and only the second falls through to the upstream.
    pub(crate) async fn serve_playlist_checked(
        &self,
        msn: Option<u64>,
        part: Option<u64>,
    ) -> PlaylistServe {
        let _serving = self.serving_guard();
        self.note_playlist_request();
        let served = self.serve_playlist_held(msn, part).await;
        self.note_playlist_request();
        served
    }

    async fn serve_playlist_held(&self, msn: Option<u64>, part: Option<u64>) -> PlaylistServe {
        self.serve_playlist_held_within(msn, part, unpark_publish_timeout()).await
    }

    /// The playlist for a request. After an unpark it first waits up to
    /// `publish_wait` for the restarted reader's first publish, and answers
    /// `NotReady` if there is none, never the window held across the park.
    async fn serve_playlist_held_within(
        &self,
        msn: Option<u64>,
        part: Option<u64>,
        publish_wait: Duration,
    ) -> PlaylistServe {
        if !self.wait_fresh(publish_wait).await {
            return PlaylistServe::NotReady;
        }
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
                        None => return PlaylistServe::Inactive,
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
                    return PlaylistServe::Inactive;
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
        match g.as_ref() {
            Some(edge) => PlaylistServe::Playlist(render_locked(edge)),
            None => PlaylistServe::Inactive,
        }
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

// ──────────────────────────── Apple LL-HLS ────────────────────────────
//
// AVPlayer's low-latency mode wants more than hls.js: a `#EXT-X-PRELOAD-HINT`
// for the next part (whose request the server holds until the part exists), a
// blocking reload that holds until the requested part or segment is really
// there, and, because its playlists arrive through a resource loader on a
// custom scheme, absolute http URIs for everything it fetches itself. All of
// it is opt-in per request (`apple=1` on the query): the hls.js playlist is
// `render_locked` byte for byte, and the Apple one is derived from it.

/// AVPlayer enforces the spec's part-size window: no part above PART-TARGET and
/// no non-final part under 85% of it (a playlist that breaks either is a parse
/// error, not a warning). hls.js's generous `PART_TARGET` fails the second rule,
/// so the Apple playlist declares a target fitted to the stream's own parts:
/// the longest non-final part held when the first Apple playlist is served,
/// plus this headroom, kept for the rest of the stream (the spec forbids the
/// target changing). The window then runs from 0.88 to 1.04 of that part. TS
/// parts are cut on the decode timestamp on the iPhone (`cut_stamp`), so every
/// non-final part spans `TS_PART_PTS` rounded up to a whole frame (0.300 s at
/// 30, 50 and 60 fps, 0.333 at 24) and a segment's first part can run a frame
/// or two short.
const APPLE_PART_TARGET_HEADROOM: f64 = 1.04;
/// The target before any complete segment exists (never kept).
const APPLE_TS_PART_TARGET: f64 = 0.334;
/// Twitch's own CMAF fragments run about 0.105 s.
const APPLE_CMAF_PART_TARGET: f64 = 0.12;
/// Hold-back as a multiple of the part target: the spec's floor is two, Apple
/// asks for three. A request may ask for two (`apple_hold=2`).
pub(crate) const APPLE_HOLD_BACK_PARTS: u64 = 3;

/// The part target fitted to the edge's parts, rounded up to the millisecond.
/// `None` before a complete segment exists.
fn fitted_apple_part_target(edge: &LiveEdge) -> Option<f64> {
    let longest = edge
        .segments
        .iter()
        .filter(|s| s.complete)
        .flat_map(|s| s.parts.split_last().map(|(_, rest)| rest).unwrap_or(&[]))
        .map(|p| p.duration)
        .fold(0.0, f64::max);
    // The epsilon keeps float noise (0.3 * 1.04 = 0.31200000000000006) from
    // rounding up a whole millisecond.
    (longest > 0.0)
        .then(|| (longest * APPLE_PART_TARGET_HEADROOM * 1000.0 - 1e-6).ceil() / 1000.0)
}

fn fallback_apple_part_target(edge: &LiveEdge) -> f64 {
    match edge.container {
        Container::Ts => APPLE_TS_PART_TARGET,
        Container::Cmaf => APPLE_CMAF_PART_TARGET,
    }
}

/// CMAF parts for AVPlayer. Twitch's own fragments run 6 to 9 frames (0.100
/// to 0.150 s at 60 fps), a spread no PART-TARGET fits within its 85% window,
/// so the Apple playlist lists runs of consecutive fragments instead, served
/// as one part of back-to-back moof+mdat pairs. A run closes once it reaches
/// this length: three 6- or 7-frame fragments, 0.300 to 0.350 s, which does
/// fit. An in-progress segment lists only its closed runs; the remainder
/// becomes its final part when it completes.
const APPLE_CMAF_RUN_MIN: f64 = 0.295;
/// The CMAF run target before any complete segment exists (never kept).
const APPLE_CMAF_RUN_TARGET: f64 = 0.351;

/// One segment's fragment runs, as (first fragment, end, duration).
fn apple_runs(seg: &Segment) -> Vec<(usize, usize, f64)> {
    let mut runs = Vec::new();
    let (mut start, mut sum) = (0, 0.0);
    for (i, p) in seg.parts.iter().enumerate() {
        sum += p.duration;
        if sum >= APPLE_CMAF_RUN_MIN {
            runs.push((start, i + 1, sum));
            start = i + 1;
            sum = 0.0;
        }
    }
    if seg.complete && start < seg.parts.len() {
        runs.push((start, seg.parts.len(), sum));
    }
    runs
}

/// The edge as AVPlayer is served it: for CMAF, each segment's parts are its
/// fragment runs (`apple_runs`); `None` for TS, whose parts already fit. With
/// a `target`, declared durations are held inside its window: a segment's
/// first run carries the longer keyframe fragment and can run a frame or two
/// past it, and the segment's EXTINF stays exact either way.
fn apple_view(edge: &LiveEdge, target: Option<f64>) -> Option<LiveEdge> {
    if edge.container != Container::Cmaf {
        return None;
    }
    let empty = Arc::new(Vec::new());
    let segments = edge
        .segments
        .iter()
        .map(|seg| {
            let runs = apple_runs(seg);
            let last = runs.len().saturating_sub(1);
            let parts = runs
                .iter()
                .enumerate()
                .map(|(i, &(_, _, d))| {
                    let duration = match target {
                        None => d,
                        Some(t) if seg.complete && i == last => d.min(t),
                        Some(t) => d.clamp(((t * 0.85) * 1000.0).ceil() / 1000.0, t),
                    };
                    Part { duration, bytes: empty.clone() }
                })
                .collect();
            Segment {
                sn: seg.sn,
                pdt: seg.pdt.clone(),
                complete: seg.complete,
                duration: seg.duration,
                parts,
                assembled: None,
            }
        })
        .collect();
    Some(LiveEdge {
        init_url: edge.init_url.clone(),
        init_bytes: edge.init_bytes.clone(),
        container: edge.container,
        target_duration: edge.target_duration,
        part_target: edge.part_target,
        segments,
    })
}

/// The CMAF run target: the longest run that is neither a segment's first nor
/// its last, rounded up to the millisecond, with no headroom (0.300 s runs
/// must stay within 85% of it). `None` before a complete segment has one.
fn fitted_apple_run_target(view: &LiveEdge) -> Option<f64> {
    let longest = view
        .segments
        .iter()
        .filter(|s| s.complete && s.parts.len() > 2)
        .flat_map(|s| s.parts[1..s.parts.len() - 1].iter())
        .map(|p| p.duration)
        .fold(0.0, f64::max);
    (longest > 0.0).then(|| (longest * 1000.0 - 1e-6).ceil() / 1000.0)
}

/// The bytes of CMAF run `idx` of a segment: its fragments back to back.
fn apple_run_bytes(seg: &Segment, idx: usize) -> Option<Arc<Vec<u8>>> {
    let &(first, end, _) = apple_runs(seg).get(idx)?;
    let mut bytes = Vec::with_capacity(seg.parts[first..end].iter().map(|p| p.bytes.len()).sum());
    for p in &seg.parts[first..end] {
        bytes.extend_from_slice(&p.bytes);
    }
    Some(Arc::new(bytes))
}
/// How long an Apple blocking reload waits for its part before answering with
/// the current playlist. AVPlayer allows a blocking reload about three target
/// durations, far above hls.js's 2 s budget that bounds `BLOCK_TIMEOUT`.
const APPLE_BLOCK_TIMEOUT: Duration = Duration::from_millis(3000);
/// How long a request for a part that does not exist yet (the preload hint) is
/// held before it answers 404.
const APPLE_PART_WAIT: Duration = Duration::from_millis(3000);

/// Whether a reload that names only `_HLS_msn` can be released: the segment is
/// complete, or a later one has begun.
fn segment_satisfied(edge: &LiveEdge, sn: u64) -> bool {
    edge.segments
        .iter()
        .any(|s| s.sn > sn || (s.sn == sn && s.complete))
}

/// Whether part `idx` of `sn` can no longer appear: its segment completed
/// without it, or left the window.
fn part_never_comes(edge: &LiveEdge, sn: u64, idx: usize) -> bool {
    if edge.segments.front().is_some_and(|s| s.sn > sn) {
        return true;
    }
    edge.segments
        .iter()
        .any(|s| s.sn == sn && s.complete && s.parts.len() <= idx)
}

/// The part a preload hint names: the next part of the in-progress segment, or
/// part 0 of the next segment when the newest one is complete.
fn next_part(edge: &LiveEdge) -> Option<(u64, usize)> {
    let newest = edge.segments.back()?;
    Some(if newest.complete {
        (newest.sn + 1, 0)
    } else {
        (newest.sn, newest.parts.len())
    })
}

/// Rewrites the `URI="..."` attribute of a tag line against `base`, adding
/// `query`. Absolute URIs (the upstream CMAF init) stay as they are.
fn absolute_uri_attr(line: &str, base: &str, query: &str) -> String {
    let Some(start) = line.find("URI=\"").map(|i| i + 5) else {
        return line.to_string();
    };
    let Some(len) = line[start..].find('"') else {
        return line.to_string();
    };
    let uri = &line[start..start + len];
    if uri.contains("://") {
        return line.to_string();
    }
    format!(
        "{}{base}{uri}{query}{}",
        &line[..start],
        &line[start + len..]
    )
}

/// The playlist for AVPlayer: `render_locked` with absolute URIs against `base`
/// (`http://127.0.0.1:<port>/`), Apple's part target, a hold-back of
/// `hold_parts` targets plus `extra_hold` seconds (the cushion `link_adapt`
/// keeps after stalls on a poor link), and a preload hint.
fn render_apple(edge: &LiveEdge, base: &str, target: f64, hold_parts: u64, extra_hold: f64) -> String {
    let plain = render_locked(edge);
    let mut s = String::with_capacity(plain.len() * 2);
    for line in plain.lines() {
        if line.starts_with("#EXT-X-PART-INF:") {
            s.push_str(&format!("#EXT-X-PART-INF:PART-TARGET={target:.3}"));
        } else if line.starts_with("#EXT-X-SERVER-CONTROL:") {
            s.push_str(&format!(
                "#EXT-X-SERVER-CONTROL:CAN-BLOCK-RELOAD=YES,PART-HOLD-BACK={:.3}",
                target * hold_parts as f64 + extra_hold
            ));
        } else if line.starts_with("#EXT-X-PART:") {
            s.push_str(&absolute_uri_attr(line, base, "?apple=1"));
        } else if line.starts_with("#EXT-X-MAP:") {
            s.push_str(&absolute_uri_attr(line, base, ""));
        } else if !line.starts_with('#') && !line.is_empty() {
            s.push_str(base);
            s.push_str(line);
        } else {
            s.push_str(line);
        }
        s.push('\n');
    }
    if let Some((sn, k)) = next_part(edge) {
        let ext = if edge.cmaf_presented() { "mp4" } else { "ts" };
        s.push_str(&format!(
            "#EXT-X-PRELOAD-HINT:TYPE=PART,URI=\"{base}part/{sn}/{k}.{ext}?apple=1\"\n"
        ));
    }
    s
}

impl LlOrigin {
    /// Waits until `ready` holds, the edge goes away (false) or `deadline`
    /// passes (true, the caller answers with what it has).
    async fn wait_for(
        &self,
        deadline: tokio::time::Instant,
        ready: impl Fn(&LiveEdge) -> bool,
    ) -> bool {
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let g = self.live_edge.lock().unwrap();
                match g.as_ref() {
                    Some(edge) if ready(edge) => return true,
                    Some(_) => {}
                    None => return false,
                }
            }
            let now = tokio::time::Instant::now();
            if now >= deadline
                || tokio::time::timeout(deadline - now, notified)
                    .await
                    .is_err()
            {
                return true;
            }
        }
    }

    /// The playlist for AVPlayer, honoring its blocking reload. A request
    /// without directives (the first load) answers at once. After an unpark it
    /// waits for the restarted reader's first publish like `serve_playlist_checked`
    /// does, and answers `NotReady` if there is none.
    pub(crate) async fn serve_playlist_apple(
        &self,
        msn: Option<u64>,
        part: Option<u64>,
        base: &str,
        hold_parts: u64,
    ) -> PlaylistServe {
        self.serve_playlist_apple_within(msn, part, base, hold_parts, unpark_publish_timeout())
            .await
    }

    async fn serve_playlist_apple_within(
        &self,
        msn: Option<u64>,
        part: Option<u64>,
        base: &str,
        hold_parts: u64,
        publish_wait: Duration,
    ) -> PlaylistServe {
        let _serving = self.serving_guard();
        self.note_playlist_request();
        let served = if self.wait_fresh(publish_wait).await {
            self.serve_playlist_apple_held(msn, part, base, hold_parts).await
        } else {
            PlaylistServe::NotReady
        };
        self.note_playlist_request();
        served
    }

    async fn serve_playlist_apple_held(
        &self,
        msn: Option<u64>,
        part: Option<u64>,
        base: &str,
        hold_parts: u64,
    ) -> PlaylistServe {
        if let Some(m) = msn {
            let deadline = tokio::time::Instant::now() + APPLE_BLOCK_TIMEOUT;
            let held = self
                .wait_for(deadline, |edge| {
                    let view = apple_view(edge, None);
                    let edge = view.as_ref().unwrap_or(edge);
                    match part {
                        Some(p) => blocking_satisfied(edge, m, p),
                        None => segment_satisfied(edge, m),
                    }
                })
                .await;
            // A hold that runs out answers with the held playlist (wait_for is
            // true at its deadline), so a paused player costs one loopback
            // request per hold and never reaches the upstream relay. Only an
            // edge that went away falls through.
            if !held {
                return PlaylistServe::Inactive;
            }
        }
        let g = self.live_edge.lock().unwrap();
        let Some(edge) = g.as_ref() else {
            return PlaylistServe::Inactive;
        };
        let runs = apple_view(edge, None);
        let target = {
            let mut kept = self.apple_part_target.lock().unwrap();
            match (*kept, &runs) {
                (Some(t), _) => t,
                (None, Some(view)) => match fitted_apple_run_target(view) {
                    Some(t) => *kept.insert(t),
                    None => APPLE_CMAF_RUN_TARGET,
                },
                (None, None) => match fitted_apple_part_target(edge) {
                    Some(t) => *kept.insert(t),
                    None => fallback_apple_part_target(edge),
                },
            }
        };
        let view = runs.and_then(|_| apple_view(edge, Some(target)));
        PlaylistServe::Playlist(render_apple(
            view.as_ref().unwrap_or(edge),
            base,
            target,
            hold_parts,
            crate::services::link_adapt::extra_cushion(&self.adapt_id()) + apple_gap_adjust(target, hold_parts),
        ))
    }

    /// A part for AVPlayer: held while it is still to come (a preload hint).
    /// For CMAF, `idx` names a fragment run (`apple_runs`).
    pub async fn get_part_apple(&self, sn: u64, idx: usize) -> Option<Arc<Vec<u8>>> {
        let deadline = tokio::time::Instant::now() + APPLE_PART_WAIT;
        self.wait_for(deadline, |edge| {
            let view = apple_view(edge, None);
            let edge = view.as_ref().unwrap_or(edge);
            has_part_locked(edge, sn, idx as u64) || part_never_comes(edge, sn, idx)
        })
        .await;
        let cmaf = {
            let g = self.live_edge.lock().unwrap();
            let edge = g.as_ref()?;
            if edge.container == Container::Cmaf {
                Some(edge.segments.iter().find(|s| s.sn == sn).and_then(|s| apple_run_bytes(s, idx)))
            } else {
                None
            }
        };
        match cmaf {
            Some(Some(bytes)) => Some(bytes),
            // Retirement grace, as `get_part` gives single parts.
            Some(None) => self.retired.lock().unwrap().iter().find(|s| s.sn == sn).and_then(|s| apple_run_bytes(s, idx)),
            None => self.get_part(sn, idx),
        }
    }
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
static SOLO: Lazy<Arc<LlOrigin>> = Lazy::new(|| {
    let origin = LlOrigin::new(MAX_SEGMENTS);
    origin.set_adapt_id(crate::services::link_adapt::SOLO);
    origin
});

/// Whether the upstream playlist probed at the last `start()` carried
/// `#EXT-X-TWITCH-PREFETCH` hints: a low-latency broadcast, whether or not
/// this origin took it over. The player picks its cushion tier from it.
static UPSTREAM_HAS_PREFETCH: AtomicBool = AtomicBool::new(false);

pub fn upstream_has_prefetch() -> bool {
    UPSTREAM_HAS_PREFETCH.load(Ordering::SeqCst)
}

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
    // Ad-free playback makes `start` decline above, so no origin can serve a
    // `seg/` playlist for this stream no matter where the kill switch sits. Say
    // so, or the viewer who has the experimental setting on loses the stable
    // projection as well as the origin and ends up worse off than with it off.
    #[cfg(mobile)]
    if crate::services::ad_bypass::active() {
        return true;
    }
    DISABLED.load(Ordering::Relaxed)
}

pub fn stop() {
    SOLO.stop()
}

/// The main player's origin, stopped unless it is parked (see
/// `LlOrigin::stop_unless_parked`).
pub fn stop_unless_parked() -> bool {
    SOLO.stop_unless_parked()
}

/// The main player's origin, parked (see `LlOrigin::park`).
pub fn park() -> bool {
    SOLO.park()
}

pub fn unpark() -> bool {
    SOLO.unpark()
}

/// The main player's origin, rebuilt at the live edge (see
/// `LlOrigin::rebuild_at_edge`). False when none is armed.
pub fn rebuild_at_edge() -> bool {
    SOLO.rebuild_at_edge()
}

pub fn is_parked() -> bool {
    SOLO.is_parked()
}

/// How the main player's origin re-signs a refused playlist after an unpark
/// (see `LlOrigin::set_resigner`).
pub(crate) fn set_resigner(resigner: Option<Resigner>) {
    SOLO.set_resigner(resigner)
}

/// The main player's origin noted a playlist request (see
/// `LlOrigin::wake_for_request`).
pub fn wake_for_request(wakes: bool) {
    SOLO.wake_for_request(wakes)
}

pub async fn start(upstream_playlist_url: String) -> StartOutcome {
    SOLO.clone().start(upstream_playlist_url).await
}

pub fn get_init() -> Option<Arc<Vec<u8>>> {
    SOLO.get_init()
}

/// The main player's playlist, keeping a not-ready origin apart from an
/// inactive one (see `PlaylistServe`).
pub(crate) async fn serve_playlist_checked(msn: Option<u64>, part: Option<u64>) -> PlaylistServe {
    SOLO.serve_playlist_checked(msn, part).await
}

/// The main player's playlist for AVPlayer, keeping a not-ready origin apart
/// from an inactive one (see `PlaylistServe`).
pub(crate) async fn serve_playlist_apple(
    msn: Option<u64>,
    part: Option<u64>,
    base: &str,
    hold_parts: u64,
) -> PlaylistServe {
    SOLO.serve_playlist_apple(msn, part, base, hold_parts).await
}

pub async fn get_part_apple(sn: u64, idx: usize) -> Option<Arc<Vec<u8>>> {
    SOLO.get_part_apple(sn, idx).await
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
    fn chunker_reports_a_fragment_cut_short_at_end_of_body() {
        // A complete moof followed by an mdat whose bytes stop early: the
        // pending count is what a clean end of body must NOT drop silently.
        let mut c = BoxChunker::new();
        let moof = make_box(b"moof", &[1, 2, 3, 4]);
        let mdat = make_box(b"mdat", &[9; 32]);
        assert!(c.push(&moof).is_empty());
        assert_eq!(c.pending(), moof.len(), "a moof waiting for its mdat is pending");
        assert_eq!(c.truncated_fragment_bytes(), moof.len(), "and it is a cut fragment");
        assert!(c.push(&mdat[..20]).is_empty());
        assert_eq!(c.pending(), moof.len() + 20);
        assert_eq!(c.truncated_fragment_bytes(), moof.len() + 20);
        let parts = c.push(&mdat[20..]);
        assert_eq!(parts.len(), 1);
        assert_eq!(c.pending(), 0, "a completed fragment leaves nothing pending");
        assert_eq!(c.truncated_fragment_bytes(), 0);
    }

    #[test]
    fn trailing_metadata_box_is_not_a_cut_fragment() {
        // A complete fragment followed by a whole non-media box (an emsg) and
        // a sub-header scrap: pending, but nothing a pickup could recover.
        let mut c = BoxChunker::new();
        c.push(&make_box(b"moof", &[1; 12]));
        assert_eq!(c.push(&make_box(b"mdat", &[2; 40])).len(), 1);
        c.push(&make_box(b"emsg", &[3; 20]));
        c.push(&[0, 0, 0]);
        assert!(c.pending() > 0);
        assert_eq!(c.truncated_fragment_bytes(), 0);
    }

    #[test]
    fn a_clean_end_well_short_of_the_segment_is_a_cut() {
        // The captured case: 16 of 19 fragments (~1.7 s of 2.0 s).
        assert!(ended_short(1.7, 2.0));
        // Two fragments short still earns the pickup.
        assert!(ended_short(1.8, 2.0));
        // One fragment short does not: a 100 ms seam is not worth holding
        // the reader for.
        assert!(!ended_short(1.9, 2.0));
        assert!(!ended_short(2.0, 2.0));
        // No expectation yet (no playlist seen): never short.
        assert!(!ended_short(0.5, 0.0));
    }

    #[test]
    fn pickup_fingerprint_keeps_only_the_tail_and_detects_a_foreign_prefix() {
        let mut held = Vec::new();
        for chunk in [vec![1u8; 50], vec![2u8; 50], vec![3u8; 10]] {
            remember_tail(&mut held, &chunk);
        }
        assert_eq!(held.len(), PICKUP_FINGERPRINT_BYTES);
        // Bytes 46..110 of the stream: four 1s, fifty 2s, ten 3s.
        assert_eq!(&held[..4], &[1u8; 4][..]);
        assert_eq!(&held[4..54], &[2u8; 50][..]);
        assert_eq!(&held[54..], &[3u8; 10][..]);
        // A replay of the same 110 bytes in different chunking matches...
        let mut replay = Vec::new();
        let same: Vec<u8> = [vec![1u8; 50], vec![2u8; 50], vec![3u8; 10]].concat();
        for piece in same.chunks(7) {
            remember_tail(&mut replay, piece);
        }
        assert_eq!(replay, held);
        // ...and a replay that differs anywhere in the last 64 bytes does not.
        let mut other = Vec::new();
        let mut changed = same.clone();
        changed[100] = 9;
        remember_tail(&mut other, &changed);
        assert_ne!(other, held);
    }

    #[test]
    fn resume_skips_exactly_the_bytes_already_held() {
        let mut skip: u64 = 10;
        assert_eq!(skip_prefix(&mut skip, &[1, 2, 3, 4]), &[] as &[u8]);
        assert_eq!(skip, 6);
        assert_eq!(skip_prefix(&mut skip, &[0, 0, 0, 0, 0, 0, 7, 8]), &[7, 8]);
        assert_eq!(skip, 0);
        assert_eq!(skip_prefix(&mut skip, &[9]), &[9], "nothing left to skip passes through");
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
    fn a_pes_with_a_dts_reports_both_stamps() {
        let mut pkt = ts_packet(true, 0xE0, Some(4_500), 0);
        // Rewrite the header as PTS 4500 + DTS 1500 ('11' flags, 10 header bytes).
        let pack = |prefix: u8, t: u64| {
            [
                prefix | (((t >> 30) & 0x07) << 1) as u8 | 0x01,
                ((t >> 22) & 0xFF) as u8,
                ((((t >> 15) & 0x7F) << 1) | 0x01) as u8,
                ((t >> 7) & 0xFF) as u8,
                (((t & 0x7F) << 1) | 0x01) as u8,
            ]
        };
        pkt[11] = 0xC0;
        pkt[12] = 0x0A;
        pkt[13..18].copy_from_slice(&pack(0x30, 4_500));
        pkt[18..23].copy_from_slice(&pack(0x10, 1_500));
        assert_eq!(video_pes_stamps(&pkt), Some((4_500, 1_500)));
        assert_eq!(video_pes_pts(&pkt), Some(4_500));
        // Without a DTS the decode stamp is the presentation stamp.
        let plain = ts_packet(true, 0xE0, Some(9_000), 0);
        assert_eq!(video_pes_stamps(&plain), Some((9_000, 9_000)));
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
    fn a_forced_catch_up_rebuilds_from_the_live_edge_whatever_the_window_holds() {
        // The window reaches the upstream (newest 8552, in-progress 8553): not
        // forced there is nothing to fetch, forced it is rebuilt behind the edge.
        let (rebuild, fetch) = catch_up_plan(false, 8552, 8540, 8553);
        assert!(!rebuild);
        assert_eq!(fetch.collect::<Vec<_>>(), Vec::<u64>::new());
        let (rebuild, fetch) = catch_up_plan(true, 8552, 8540, 8553);
        assert!(rebuild);
        assert_eq!(fetch.collect::<Vec<_>>(), vec![8552]);

        // The backfill never starts before the oldest published segment.
        let (rebuild, fetch) = catch_up_plan(true, 8552, 8552, 8553);
        assert!(rebuild);
        assert_eq!(fetch.collect::<Vec<_>>(), vec![8552]);

        // Not forced keeps `plan_catch_up`'s answer, small hole and deep hole.
        let (rebuild, fetch) = catch_up_plan(false, 8551, 8540, 8553);
        assert!(!rebuild);
        assert_eq!(fetch.collect::<Vec<_>>(), vec![8552]);
        let (rebuild, fetch) = catch_up_plan(false, 8500, 8540, 8553);
        assert!(rebuild);
        assert_eq!(fetch.collect::<Vec<_>>(), vec![8552]);
    }

    #[tokio::test]
    async fn rebuild_at_edge_replaces_the_reader_and_serves_no_window_held_before() {
        let origin = LlOrigin::new(6);
        let short = Duration::from_millis(50);
        assert!(!origin.rebuild_at_edge(), "an inactive origin has nothing to rebuild");
        assert!(origin.reader_task.lock().unwrap().is_none());

        // Nothing listens here, so the restarted reader publishes nothing.
        origin.arm_for_test("http://127.0.0.1:1/live.m3u8");
        assert!(matches!(
            origin.serve_playlist_held_within(Some(1), Some(0), short).await,
            PlaylistServe::Playlist(_)
        ));
        let generation = origin.generation.load(Ordering::SeqCst);
        assert!(origin.rebuild_at_edge());
        assert!(origin.has_reader_for_test());
        assert!(origin.generation.load(Ordering::SeqCst) > generation);
        assert!(origin.force_rebuild.load(Ordering::SeqCst), "the new reader is told to rebuild");
        assert!(!origin.fresh_after_unpark.load(Ordering::SeqCst));
        assert_eq!(
            origin.serve_playlist_held_within(Some(1), Some(0), short).await,
            PlaylistServe::NotReady,
            "the window held before the rebuild is never served"
        );
        assert!(origin.is_active(), "the origin stays active, so the relay answers 503");

        // A reader that is no longer the current one does not take the request.
        assert!(!origin.take_force_rebuild(generation));
        origin.stop();
        assert!(!origin.force_rebuild.load(Ordering::SeqCst), "a stop clears a pending rebuild");
    }

    #[tokio::test]
    async fn rebuild_at_edge_wakes_a_parked_origin_and_refuses_a_closed_one() {
        let origin = LlOrigin::new(6);
        origin.arm_for_test("http://127.0.0.1:1/live.m3u8");
        assert!(origin.park());
        assert!(origin.rebuild_at_edge());
        assert!(!origin.is_parked(), "the rebuild's reader replaces the parked one");
        assert!(origin.has_reader_for_test());
        origin.stop();

        origin.arm_for_test("http://127.0.0.1:1/live.m3u8");
        origin.close();
        origin.arm_for_test("http://127.0.0.1:1/live.m3u8");
        assert!(!origin.rebuild_at_edge(), "a closed origin never rebuilds");
        assert!(!origin.has_reader_for_test(), "no reader was started");
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
        let gen = origin.generation.load(Ordering::SeqCst);
        origin.deactivate_offline("test offline", gen);
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

    fn held_sns(origin: &LlOrigin) -> Vec<u64> {
        origin.live_edge.lock().unwrap().as_ref().unwrap().segments.iter().map(|s| s.sn).collect()
    }

    #[test]
    fn park_finishes_a_half_read_tail_at_its_parts() {
        let origin = LlOrigin::new(6);
        let mut edge = ts_edge_with_one_segment();
        edge.segments.push_back(make_segment(2, None, false, even_parts(vec![vec![1], vec![2]])));
        *origin.live_edge.lock().unwrap() = Some(edge);
        assert!(origin.park());
        let g = origin.live_edge.lock().unwrap();
        let tail = g.as_ref().unwrap().segments.back().unwrap();
        assert_eq!(tail.sn, 2);
        assert!(tail.complete, "a restarted reader must find a finished segment");
        let sum: f64 = tail.parts.iter().map(|p| p.duration).sum();
        assert!((tail.duration - sum).abs() < 1e-9);
    }

    #[test]
    fn park_drops_a_bare_shell() {
        let origin = LlOrigin::new(6);
        let mut edge = ts_edge_with_one_segment();
        edge.segments.push_back(make_segment(2, None, false, Vec::new()));
        *origin.live_edge.lock().unwrap() = Some(edge);
        assert!(origin.park());
        assert_eq!(held_sns(&origin), vec![1]);
    }

    #[test]
    fn park_holds_the_edge_as_served() {
        let origin = LlOrigin::new(6);
        *origin.live_edge.lock().unwrap() = Some(ts_edge_with_one_segment());
        let before = held_sns(&origin);
        let generation = origin.generation.load(Ordering::SeqCst);
        assert!(origin.park());
        assert!(origin.is_active());
        assert!(origin.is_parked());
        assert!(origin.generation.load(Ordering::SeqCst) > generation);
        assert_eq!(held_sns(&origin), before);
        assert!(!origin.park(), "a second park is a no-op");
    }

    #[test]
    fn unpark_without_an_upstream_clears_the_park_and_starts_nothing() {
        let origin = LlOrigin::new(6);
        *origin.live_edge.lock().unwrap() = Some(ts_edge_with_one_segment());
        assert!(origin.park());
        assert!(!origin.unpark());
        assert!(!origin.is_parked());
        assert!(origin.reader_task.lock().unwrap().is_none());
    }

    #[test]
    fn stop_clears_a_park() {
        let origin = LlOrigin::new(6);
        *origin.live_edge.lock().unwrap() = Some(ts_edge_with_one_segment());
        assert!(origin.park());
        origin.stop();
        assert!(!origin.is_parked());
        assert!(!origin.is_active());
    }

    #[test]
    fn an_unpark_after_a_stop_starts_no_reader() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap();
        let _guard = rt.enter();
        let origin = LlOrigin::new(6);
        *origin.live_edge.lock().unwrap() = Some(ts_edge_with_one_segment());
        *origin.upstream.lock().unwrap() = Some("http://x/playlist.m3u8".to_string());
        assert!(origin.park());
        let generation = origin.generation.load(Ordering::SeqCst);

        // The stop takes the reader lock and the generation bump, then waits on
        // the live edge this thread holds, so it is mid-stop while the checks run.
        let edge_guard = origin.live_edge.lock().unwrap();
        let stopper = {
            let origin = origin.clone();
            std::thread::spawn(move || origin.stop())
        };
        let began = std::time::Instant::now();
        while origin.generation.load(Ordering::SeqCst) == generation {
            assert!(began.elapsed() < Duration::from_secs(5), "the stop never began");
            std::thread::yield_now();
        }
        assert!(origin.reader_task.try_lock().is_err(), "the stop holds the reader lock");
        drop(edge_guard);
        stopper.join().unwrap();

        assert!(!origin.unpark(), "a stopped origin has nothing to wake");
        assert!(origin.reader_task.lock().unwrap().is_none(), "no reader was spawned");
        assert!(!origin.is_parked());
        assert!(!origin.is_active());
    }

    #[test]
    fn a_start_superseded_by_a_stop_installs_nothing() {
        let origin = LlOrigin::new(6);
        let gen = origin.generation.load(Ordering::SeqCst);
        origin.stop();
        *origin.cmaf_video.lock().unwrap() = Some((1, 90_000));
        *origin.transmux.lock().unwrap() = Some(crate::services::ts_fmp4::Transmuxer::new());

        let installed = origin.install_started(
            gen,
            ts_edge_with_one_segment(),
            "http://x/playlist.m3u8".to_string(),
            || -> JoinHandle<()> { panic!("a superseded start spawns no reader") },
        );
        assert!(!installed);
        assert!(!origin.is_active());
        assert!(origin.upstream.lock().unwrap().is_none());
        assert!(origin.cmaf_video.lock().unwrap().is_none(), "the prepared track is cleared");
        assert!(origin.transmux.lock().unwrap().is_none(), "the prepared transmuxer is cleared");
        assert!(origin.reader_task.lock().unwrap().is_none());
    }

    #[test]
    fn a_queued_start_returns_inactive_when_a_stop_lands_while_it_waits() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            // Anything fetched would connect here and sit in the backlog.
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let url = format!("http://{}/live.m3u8", listener.local_addr().unwrap());

            let origin = LlOrigin::new(6);
            let waiting_on = origin.start_lock.lock().await;
            let generation = origin.generation.load(Ordering::SeqCst);
            let starter = tokio::spawn(origin.clone().start(url));
            // The queued start stops first, which moves the generation.
            let began = std::time::Instant::now();
            while origin.generation.load(Ordering::SeqCst) == generation {
                assert!(began.elapsed() < Duration::from_secs(5), "the start never queued");
                tokio::task::yield_now().await;
            }
            origin.stop();
            drop(waiting_on);

            let outcome = starter.await.unwrap();
            assert!(!outcome.active);
            assert!(origin.reader_task.lock().unwrap().is_none());
            assert!(!origin.is_active());
            assert!(listener.accept().is_err(), "a stopped start fetches nothing");
        });
    }

    #[test]
    fn stop_unless_parked_leaves_a_parked_origin_alone() {
        let origin = LlOrigin::new(6);
        *origin.live_edge.lock().unwrap() = Some(ts_edge_with_one_segment());
        assert!(origin.park());
        let generation = origin.generation.load(Ordering::SeqCst);
        assert!(!origin.stop_unless_parked());
        assert!(origin.is_parked());
        assert!(origin.is_active());
        assert_eq!(origin.generation.load(Ordering::SeqCst), generation);

        let running = LlOrigin::new(6);
        *running.live_edge.lock().unwrap() = Some(ts_edge_with_one_segment());
        assert!(running.stop_unless_parked());
        assert!(!running.is_active());
    }

    #[test]
    fn a_stale_reader_changes_nothing() {
        let origin = LlOrigin::new(6);
        let mut edge = ts_edge_with_one_segment();
        edge.segments.push_back(make_segment(2, None, false, even_parts(vec![vec![2]])));
        *origin.live_edge.lock().unwrap() = Some(edge);
        let gen = origin.generation.load(Ordering::SeqCst);
        let stale = gen + 1;
        let held_parts = |origin: &LlOrigin| origin.parts_held(2);

        assert!(!origin.append_part(2, vec![9], 0.5, stale));
        assert_eq!(held_parts(&origin), 1);

        assert_eq!(origin.push_shell(3, None, stale), None);
        assert_eq!(held_sns(&origin), vec![1, 2]);

        assert!(!origin.finish_segment(2, stale));
        assert!(!origin.live_edge.lock().unwrap().as_ref().unwrap().segments[1].complete);

        let before = origin.edge_version.load(Ordering::SeqCst);
        origin.deactivate_offline("stale reader", stale);
        assert!(origin.is_active());
        assert_eq!(origin.edge_version.load(Ordering::SeqCst), before);

        // The current reader's calls take effect.
        assert!(origin.append_part(2, vec![9], 0.5, gen));
        assert_eq!(held_parts(&origin), 2);
        assert_eq!(origin.push_shell(3, None, gen), Some(true));
        assert!(origin.finish_segment(2, gen));
    }

    #[test]
    fn a_stale_reader_leaves_the_transmuxer_alone() {
        let origin = LlOrigin::new(6);
        let gen = origin.generation.load(Ordering::SeqCst);

        assert!(!origin.drop_partial_input_if_current(gen), "no transmuxer, nothing to drop");

        *origin.transmux.lock().unwrap() = Some(crate::services::ts_fmp4::Transmuxer::new());
        assert!(!origin.drop_partial_input_if_current(gen + 1), "a stale reader drops nothing");
        assert!(origin.drop_partial_input_if_current(gen), "the current reader drops the tail");

        origin.stop();
        *origin.transmux.lock().unwrap() = Some(crate::services::ts_fmp4::Transmuxer::new());
        assert!(
            !origin.drop_partial_input_if_current(gen),
            "a reader retired by a stop leaves a newer transmuxer alone"
        );
    }

    #[test]
    fn a_stale_reader_converts_no_part() {
        let origin = LlOrigin::new(6);
        let gen = origin.generation.load(Ordering::SeqCst);

        // Passthrough (no transmuxer): the current reader's part comes back as
        // it went in, a stale reader's does not.
        assert_eq!(origin.convert_part(vec![7], 0.5, gen), Some((vec![7], 0.5)));
        assert_eq!(origin.convert_part(vec![7], 0.5, gen + 1), None);

        // With a transmuxer, a reader retired by a stop never feeds it.
        origin.stop();
        *origin.transmux.lock().unwrap() = Some(crate::services::ts_fmp4::Transmuxer::new());
        assert_eq!(origin.convert_part(vec![7], 0.5, gen), None);
        assert!(origin.convert_parts(vec![vec![7], vec![8]], gen).is_empty());
    }

    #[test]
    fn a_closed_origin_installs_nothing_and_never_unparks() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap();
        let origin = LlOrigin::new(6);
        *origin.live_edge.lock().unwrap() = Some(ts_edge_with_one_segment());
        *origin.upstream.lock().unwrap() = Some("http://x/playlist.m3u8".to_string());
        assert!(origin.park());

        origin.close();
        assert!(!origin.is_active());
        assert!(!origin.unpark(), "a closed origin starts no reader");
        assert!(origin.reader_task.lock().unwrap().is_none());

        // Every other precondition for a wake is back after the close: an
        // edge, an upstream and a park. The close still wins.
        *origin.live_edge.lock().unwrap() = Some(ts_edge_with_one_segment());
        *origin.upstream.lock().unwrap() = Some("http://x/playlist.m3u8".to_string());
        origin.mark_parked_for_test();
        assert!(!origin.unpark(), "a closed origin never wakes");
        assert!(origin.reader_task.lock().unwrap().is_none(), "no reader was started");
        origin.stop();

        let gen = origin.generation.load(Ordering::SeqCst);
        *origin.transmux.lock().unwrap() = Some(crate::services::ts_fmp4::Transmuxer::new());
        let installed = origin.install_started(
            gen,
            ts_edge_with_one_segment(),
            "http://x/playlist.m3u8".to_string(),
            || -> JoinHandle<()> { panic!("a closed origin spawns no reader") },
        );
        assert!(!installed, "a start that probed before the close installs nothing");
        assert!(!origin.is_active());
        assert!(origin.upstream.lock().unwrap().is_none());
        assert!(origin.transmux.lock().unwrap().is_none(), "the prepared transmuxer is cleared");

        let outcome = rt.block_on(origin.clone().start("http://127.0.0.1:1/x.m3u8".to_string()));
        assert!(!outcome.active, "a closed origin does not start");
    }

    #[test]
    fn an_inactive_origin_reads_as_parked_after_the_idle_span_without_a_request() {
        let origin = LlOrigin::new(6);
        let asked = OLD_REQUEST_MS;
        assert!(!origin.parked_or_idle_at(asked + park_idle_ms()), "never asked: not parked");

        origin.note_playlist_request_at(asked);
        assert!(!origin.parked_or_idle_at(asked + park_idle_ms() - 1));
        assert!(origin.parked_or_idle_at(asked + park_idle_ms()));

        origin.note_playlist_request_at(asked + park_idle_ms());
        assert!(!origin.parked_or_idle_at(asked + park_idle_ms()), "a fresh request clears it");

        // An active origin is idle-parked by its reader, not by this reading.
        origin.note_playlist_request_at(asked);
        *origin.live_edge.lock().unwrap() = Some(ts_edge_with_one_segment());
        assert!(!origin.parked_or_idle_at(asked + park_idle_ms()));
        assert!(origin.park());
        assert!(origin.parked_or_idle_at(asked));
    }

    #[test]
    fn a_swap_is_held_while_parked_and_taken_once_after() {
        let mut slot: Option<String> = None;
        assert_eq!(hold_swap_if_parked(&mut slot, true, "a".to_string()), None);
        assert_eq!(slot.as_deref(), Some("a"));
        // A newer swap replaces the held one.
        assert_eq!(hold_swap_if_parked(&mut slot, true, "b".to_string()), None);
        assert_eq!(slot.as_deref(), Some("b"));
        // Still parked: it stays held.
        assert_eq!(take_held_swap(&mut slot, true), None);
        assert_eq!(slot.as_deref(), Some("b"));
        // A player loads again: taken once.
        assert_eq!(take_held_swap(&mut slot, false).as_deref(), Some("b"));
        assert_eq!(take_held_swap(&mut slot, false), None);
        // Not parked: applied now, nothing held.
        assert_eq!(hold_swap_if_parked(&mut slot, false, "c".to_string()).as_deref(), Some("c"));
        assert!(slot.is_none());
        // A swap applied now supersedes one held earlier.
        assert_eq!(hold_swap_if_parked(&mut slot, true, "d".to_string()), None);
        assert_eq!(hold_swap_if_parked(&mut slot, false, "e".to_string()).as_deref(), Some("e"));
        assert!(slot.is_none(), "the older held swap is gone");
    }

    #[test]
    fn a_refusal_right_after_an_unpark_re_signs_once_before_it_counts() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap();
        let origin = LlOrigin::new(6);
        let gen = origin.generation.load(Ordering::SeqCst);
        *origin.upstream.lock().unwrap() = Some("http://x/stale.m3u8".to_string());
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        {
            let calls = calls.clone();
            let seen = seen.clone();
            origin.set_resigner(Some(Arc::new(move |stale: String| {
                calls.fetch_add(1, Ordering::SeqCst);
                seen.lock().unwrap().push(stale);
                Box::pin(async { Some("http://x/fresh.m3u8".to_string()) })
                    as futures::future::BoxFuture<'static, Option<String>>
            })));
        }

        let mut health = PlaylistHealth::new(true);
        health.polled();
        assert!(health.wants_resign(403));
        let fresh = rt.block_on(origin.resign("http://x/stale.m3u8"));
        assert_eq!(fresh.as_deref(), Some("http://x/fresh.m3u8"));
        assert!(origin.adopt_upstream(gen, fresh.unwrap()));
        assert_eq!(
            origin.upstream.lock().unwrap().as_deref(),
            Some("http://x/fresh.m3u8"),
            "the retry follows the fresh URL"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(*seen.lock().unwrap(), vec!["http://x/stale.m3u8".to_string()]);
        assert_eq!(health.fails, 0, "the re-signed refusal did not count");

        // One re-sign per unpark: the next refusal counts.
        health.polled();
        assert!(!health.wants_resign(403));
        assert_eq!(health.failed(), 1);

        // A superseded reader adopts nothing.
        assert!(!origin.adopt_upstream(gen + 1, "http://x/other.m3u8".to_string()));
        assert_eq!(origin.upstream.lock().unwrap().as_deref(), Some("http://x/fresh.m3u8"));
    }

    #[test]
    fn a_refusal_without_a_recent_unpark_counts_as_before() {
        // A reader that did not follow an unpark never re-signs.
        let mut health = PlaylistHealth::new(false);
        health.polled();
        assert!(!health.wants_resign(403));
        assert!(!health.wants_resign(404));
        assert_eq!(health.failed(), 1);

        // After an unpark, only a 403 or 404 in the first polls re-signs.
        let mut late = PlaylistHealth::new(true);
        for _ in 0..=RESIGN_POLLS_AFTER_UNPARK {
            late.polled();
        }
        assert!(!late.wants_resign(403), "past the first polls a refusal counts");
        let mut other = PlaylistHealth::new(true);
        other.polled();
        assert!(!other.wants_resign(500), "a server error is not an expired signature");
        assert!(other.wants_resign(404));

        // A good poll resets the run.
        health.healthy();
        assert_eq!(health.fails, 0);
    }

    /// A local playlist server on loopback: `/stale.m3u8` is refused with a 403
    /// and every other path answers 200 with an empty body. Each path asked for
    /// is recorded.
    struct RefusingServer {
        addr: std::net::SocketAddr,
        paths: Arc<Mutex<Vec<String>>>,
        done: Arc<AtomicBool>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl RefusingServer {
        fn start() -> Self {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let addr = listener.local_addr().unwrap();
            let paths = Arc::new(Mutex::new(Vec::<String>::new()));
            let done = Arc::new(AtomicBool::new(false));
            let thread = {
                let (paths, done) = (paths.clone(), done.clone());
                std::thread::spawn(move || {
                    use std::io::{Read, Write};
                    while !done.load(Ordering::SeqCst) {
                        match listener.accept() {
                            Ok((mut stream, _)) => {
                                let _ = stream.set_nonblocking(false);
                                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                                let mut buf = [0u8; 2048];
                                let n = stream.read(&mut buf).unwrap_or(0);
                                let request = String::from_utf8_lossy(&buf[..n]).to_string();
                                let path = request.split_whitespace().nth(1).unwrap_or("").to_string();
                                let status = if path == "/stale.m3u8" { "403 Forbidden" } else { "200 OK" };
                                paths.lock().unwrap().push(path);
                                let _ = write!(
                                    stream,
                                    "HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                                );
                            }
                            Err(_) => std::thread::sleep(Duration::from_millis(5)),
                        }
                    }
                })
            };
            Self { addr, paths, done, thread: Some(thread) }
        }

        fn url(&self, path: &str) -> String {
            format!("http://{}{}", self.addr, path)
        }

        fn paths(&self) -> Vec<String> {
            self.paths.lock().unwrap().clone()
        }
    }

    impl Drop for RefusingServer {
        fn drop(&mut self) {
            self.done.store(true, Ordering::SeqCst);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    /// A parked origin holding an edge and `upstream`, whose resigner counts its
    /// calls and always answers `fresh`.
    fn parked_origin_with_resigner(
        upstream: &str,
        fresh: &str,
    ) -> (Arc<LlOrigin>, Arc<AtomicUsize>, Arc<Mutex<Vec<String>>>) {
        let origin = LlOrigin::new(6);
        *origin.live_edge.lock().unwrap() = Some(ts_edge_with_one_segment());
        *origin.upstream.lock().unwrap() = Some(upstream.to_string());
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        {
            let (calls, seen, fresh) = (calls.clone(), seen.clone(), fresh.to_string());
            origin.set_resigner(Some(Arc::new(move |stale: String| {
                calls.fetch_add(1, Ordering::SeqCst);
                seen.lock().unwrap().push(stale);
                let fresh = fresh.clone();
                Box::pin(async move { Some(fresh) }) as futures::future::BoxFuture<'static, Option<String>>
            })));
        }
        assert!(origin.park());
        (origin, calls, seen)
    }

    #[test]
    fn an_unpark_re_signs_a_refused_first_poll_once_and_polls_the_fresh_url() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let server = RefusingServer::start();
        let stale = server.url("/stale.m3u8");
        let fresh = server.url("/fresh.m3u8");

        // Woken: the reader's first poll is refused, the resigner is asked
        // once, and the poll after it goes to the fresh URL.
        let (origin, calls, seen) = parked_origin_with_resigner(&stale, &fresh);
        rt.block_on(async {
            assert!(origin.unpark());
            let began = std::time::Instant::now();
            while !server.paths().contains(&"/fresh.m3u8".to_string()) {
                assert!(began.elapsed() < Duration::from_secs(10), "the fresh URL was never polled");
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });
        assert_eq!(calls.load(Ordering::SeqCst), 1, "one re-sign");
        assert_eq!(*seen.lock().unwrap(), vec![stale.clone()]);
        assert_eq!(origin.upstream.lock().unwrap().as_deref(), Some(fresh.as_str()));
        let asked = server.paths();
        assert_eq!(asked.first().map(String::as_str), Some("/stale.m3u8"), "the stale URL went first");
        origin.stop();

        // Never woken: no poll, so nothing is refused and nothing is re-signed.
        let before = server.paths().len();
        let (idle, idle_calls, _) = parked_origin_with_resigner(&stale, &fresh);
        rt.block_on(async { tokio::time::sleep(Duration::from_millis(300)).await });
        assert_eq!(idle_calls.load(Ordering::SeqCst), 0, "a parked origin never re-signs");
        assert_eq!(server.paths().len(), before, "a parked origin polls nothing");
        assert_eq!(idle.upstream.lock().unwrap().as_deref(), Some(stale.as_str()));
    }

    #[test]
    fn a_woken_origin_serves_no_window_until_its_reader_publishes() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let _guard = rt.enter();
        let origin = LlOrigin::new(6);
        assert!(origin.fresh_after_unpark.load(Ordering::SeqCst), "a new origin is fresh");
        *origin.live_edge.lock().unwrap() = Some(ts_edge_with_one_segment());
        // Nothing listens here, so the restarted reader publishes nothing.
        *origin.upstream.lock().unwrap() = Some("http://127.0.0.1:1/live.m3u8".to_string());
        let short = Duration::from_millis(50);
        let serve = |origin: &Arc<LlOrigin>, wait: Duration| {
            rt.block_on(origin.serve_playlist_held_within(Some(1), Some(0), wait))
        };

        // Parked, the held window is the answer.
        assert!(origin.park());
        assert!(matches!(serve(&origin, short), PlaylistServe::Playlist(_)));

        // Woken: the held window is not served while the reader has not published.
        assert!(origin.unpark());
        assert!(!origin.fresh_after_unpark.load(Ordering::SeqCst));
        assert_eq!(serve(&origin, short), PlaylistServe::NotReady);
        assert!(origin.is_active(), "the origin is not inactive, so the relay answers 503");

        // A reader that is no longer the current one publishes nothing.
        let gen = origin.generation.load(Ordering::SeqCst);
        origin.mark_fresh(gen + 1);
        assert_eq!(serve(&origin, short), PlaylistServe::NotReady);

        // The current reader's first publish releases a request that was waiting.
        let served = rt.block_on(async {
            let publisher = {
                let origin = origin.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    origin.mark_fresh(gen);
                })
            };
            let served = origin
                .serve_playlist_held_within(Some(1), Some(0), Duration::from_secs(5))
                .await;
            publisher.await.unwrap();
            served
        });
        assert!(matches!(served, PlaylistServe::Playlist(_)));
        assert!(matches!(serve(&origin, short), PlaylistServe::Playlist(_)));

        // A park or a stop leaves the origin serving what it holds.
        origin.fresh_after_unpark.store(false, Ordering::SeqCst);
        assert!(origin.park());
        assert!(origin.fresh_after_unpark.load(Ordering::SeqCst));
        origin.fresh_after_unpark.store(false, Ordering::SeqCst);
        origin.stop();
        assert!(origin.fresh_after_unpark.load(Ordering::SeqCst));
        assert_eq!(serve(&origin, short), PlaylistServe::Inactive);
    }

    #[test]
    fn the_unpark_publish_wait_reads_seconds_and_stays_under_the_player_timeout() {
        assert_eq!(publish_timeout_from(None), UNPARK_PUBLISH_TIMEOUT);
        assert_eq!(publish_timeout_from(Some("2")), Duration::from_secs(2));
        assert_eq!(publish_timeout_from(Some(" 7 ")), Duration::from_secs(7));
        assert_eq!(publish_timeout_from(Some("0")), UNPARK_PUBLISH_TIMEOUT);
        assert_eq!(publish_timeout_from(Some("soon")), UNPARK_PUBLISH_TIMEOUT);
        assert!(UNPARK_PUBLISH_TIMEOUT < Duration::from_secs(10));
    }

    #[test]
    fn a_dropped_preopen_handle_aborts_its_task() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap();
        rt.block_on(async {
            let task = AbortOnDrop(tokio::spawn(async {
                tokio::time::sleep(Duration::from_secs(60)).await;
            }));
            let probe = task.0.abort_handle();
            drop(task);
            tokio::task::yield_now().await;
            tokio::task::yield_now().await;
            assert!(probe.is_finished(), "dropping the handle ended the task");
        });
    }

    const OLD_REQUEST_MS: u64 = u64::MAX / 2;

    #[test]
    fn serve_playlist_notes_the_request() {
        let origin = LlOrigin::new(6);
        *origin.live_edge.lock().unwrap() = Some(ts_edge_with_one_segment());
        origin.note_playlist_request_at(OLD_REQUEST_MS);
        let rt = tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap();
        // Segment 1 part 0 exists, so the blocking reload answers at once.
        let served = rt.block_on(origin.serve_playlist(Some(1), Some(0)));
        assert!(served.is_some());
        assert_ne!(origin.last_playlist_ms.load(Ordering::SeqCst), OLD_REQUEST_MS);
    }

    #[test]
    fn serve_playlist_apple_notes_the_request() {
        let origin = LlOrigin::new(6);
        *origin.live_edge.lock().unwrap() = Some(ts_edge_with_one_segment());
        origin.note_playlist_request_at(OLD_REQUEST_MS);
        let rt = tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap();
        // A first load carries no directives and answers at once.
        let served = rt.block_on(origin.serve_playlist_apple(None, None, "http://x/", 2));
        assert!(matches!(served, PlaylistServe::Playlist(_)));
        assert_ne!(origin.last_playlist_ms.load(Ordering::SeqCst), OLD_REQUEST_MS);
    }

    #[test]
    fn a_woken_origin_serves_the_apple_route_no_window_until_its_reader_publishes() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let _guard = rt.enter();
        let origin = LlOrigin::new(6);
        *origin.live_edge.lock().unwrap() = Some(ts_edge_with_one_segment());
        // Nothing listens here, so the restarted reader publishes nothing.
        *origin.upstream.lock().unwrap() = Some("http://127.0.0.1:1/live.m3u8".to_string());
        let short = Duration::from_millis(50);
        let serve = |origin: &Arc<LlOrigin>, wait: Duration| {
            rt.block_on(origin.serve_playlist_apple_within(None, None, "http://x/", 2, wait))
        };

        // Parked, the held window is the answer.
        assert!(origin.park());
        assert!(matches!(serve(&origin, short), PlaylistServe::Playlist(_)));

        // Woken: the held window is not served while the reader has not published.
        assert!(origin.unpark());
        assert_eq!(serve(&origin, short), PlaylistServe::NotReady);
        assert!(origin.is_active(), "the origin is not inactive, so the relay answers 503");

        // The current reader's first publish releases a request that was waiting.
        let gen = origin.generation.load(Ordering::SeqCst);
        let served = rt.block_on(async {
            let publisher = {
                let origin = origin.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    origin.mark_fresh(gen);
                })
            };
            let served = origin
                .serve_playlist_apple_within(None, None, "http://x/", 2, Duration::from_secs(5))
                .await;
            publisher.await.unwrap();
            served
        });
        assert!(matches!(served, PlaylistServe::Playlist(_)));
        origin.stop();
    }

    #[test]
    fn a_held_reload_notes_the_request_when_the_hold_ends() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_time()
            .build()
            .unwrap();
        let origin = LlOrigin::new(6);
        *origin.live_edge.lock().unwrap() = Some(ts_edge_with_one_segment());
        origin.note_playlist_request_at(OLD_REQUEST_MS);

        // Segment 1 is complete with one part, so a reload for part 1 of it
        // waits for part 0 of segment 2.
        let held = {
            let origin = origin.clone();
            rt.spawn(async move { origin.serve_playlist_apple(Some(1), Some(1), "http://x/", 2).await })
        };

        // The request is noted as it arrives; put the old value back mid-hold.
        let began = std::time::Instant::now();
        while origin.last_playlist_ms.load(Ordering::SeqCst) == OLD_REQUEST_MS {
            assert!(began.elapsed() < Duration::from_secs(5), "the request was never noted");
            std::thread::sleep(Duration::from_millis(1));
        }
        origin.note_playlist_request_at(OLD_REQUEST_MS);
        assert!(!held.is_finished(), "the reload is still held");

        // The part lands and the hold ends.
        {
            let mut g = origin.live_edge.lock().unwrap();
            let edge = g.as_mut().unwrap();
            edge.segments.push_back(make_segment(2, None, false, even_parts(vec![vec![2]])));
        }
        origin.wake_serves();
        let served = rt.block_on(held).unwrap();
        assert!(matches!(served, PlaylistServe::Playlist(_)));
        assert_ne!(
            origin.last_playlist_ms.load(Ordering::SeqCst),
            OLD_REQUEST_MS,
            "the end of the hold noted the request again"
        );
    }

    #[test]
    fn park_if_idle_parks_only_when_idle_and_current() {
        let origin = LlOrigin::new(6);
        *origin.live_edge.lock().unwrap() = Some(ts_edge_with_one_segment());
        let gen = origin.generation.load(Ordering::SeqCst);
        let idle_at = OLD_REQUEST_MS + park_idle_ms();
        origin.note_playlist_request_at(OLD_REQUEST_MS);

        // A reader that is no longer the current one never parks.
        assert!(!origin.park_if_idle_at(gen + 1, idle_at));
        assert!(!origin.is_parked());

        // A request that landed after the reader decided keeps the origin running.
        origin.note_playlist_request_at(idle_at);
        assert!(!origin.park_if_idle_at(gen, idle_at));
        assert!(!origin.is_parked(), "the failed park leaves the origin unparked");

        // Idle and current: parks, and the reader's generation is retired.
        origin.note_playlist_request_at(OLD_REQUEST_MS);
        assert!(origin.park_if_idle_at(gen, idle_at));
        assert!(origin.is_parked());
        assert!(origin.generation.load(Ordering::SeqCst) > gen);
    }

    #[test]
    fn park_if_idle_refuses_while_a_playlist_request_is_in_flight() {
        let origin = LlOrigin::new(6);
        *origin.live_edge.lock().unwrap() = Some(ts_edge_with_one_segment());
        let gen = origin.generation.load(Ordering::SeqCst);
        let idle_at = OLD_REQUEST_MS + park_idle_ms();
        origin.note_playlist_request_at(OLD_REQUEST_MS);

        let first = origin.serving_guard();
        let second = origin.serving_guard();
        assert!(!origin.idle_park_due_at(idle_at), "a request waiting on its answer is not idle");
        assert!(!origin.park_if_idle_at(gen, idle_at));
        assert!(!origin.is_parked());

        drop(first);
        assert!(!origin.park_if_idle_at(gen, idle_at), "one request is still in flight");
        drop(second);
        assert_eq!(origin.serving.load(Ordering::SeqCst), 0);
        assert!(origin.park_if_idle_at(gen, idle_at), "idle once none is in flight");
        assert!(origin.is_parked());
    }

    #[test]
    fn every_way_a_playlist_request_ends_releases_its_count() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let _guard = rt.enter();
        let origin = LlOrigin::new(6);
        *origin.live_edge.lock().unwrap() = Some(ts_edge_with_one_segment());
        *origin.upstream.lock().unwrap() = Some("http://127.0.0.1:1/live.m3u8".to_string());
        let short = Duration::from_millis(50);

        // Answered at once.
        assert!(matches!(
            rt.block_on(origin.serve_playlist_checked(Some(1), Some(0))),
            PlaylistServe::Playlist(_)
        ));
        assert_eq!(origin.serving.load(Ordering::SeqCst), 0);

        // Not ready: woken, and the reader publishes nothing.
        assert!(origin.park());
        assert!(origin.unpark());
        assert_eq!(
            rt.block_on(origin.serve_playlist_apple_within(None, None, "http://x/", 2, short)),
            PlaylistServe::NotReady
        );
        assert_eq!(origin.serving.load(Ordering::SeqCst), 0);

        // Cancelled while it waits.
        let cancelled = rt.block_on(async {
            tokio::time::timeout(
                Duration::from_millis(20),
                origin.serve_playlist_apple_within(None, None, "http://x/", 2, Duration::from_secs(5)),
            )
            .await
        });
        assert!(cancelled.is_err());
        assert_eq!(origin.serving.load(Ordering::SeqCst), 0);
        origin.stop();
    }

    #[test]
    fn a_park_landing_while_a_request_waits_never_hands_it_the_held_window() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let _guard = rt.enter();
        let origin = LlOrigin::new(6);
        *origin.live_edge.lock().unwrap() = Some(ts_edge_with_one_segment());
        // Nothing listens here, so the restarted reader publishes nothing.
        *origin.upstream.lock().unwrap() = Some("http://127.0.0.1:1/live.m3u8".to_string());

        let park_after_wait = |origin: &Arc<LlOrigin>| {
            let origin = origin.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(20)).await;
                assert!(origin.park());
            })
        };

        // The checked route.
        assert!(origin.park());
        assert!(origin.unpark());
        let served = rt.block_on(async {
            let parker = park_after_wait(&origin);
            let served = origin
                .serve_playlist_held_within(Some(1), Some(0), Duration::from_secs(5))
                .await;
            parker.await.unwrap();
            served
        });
        assert_eq!(served, PlaylistServe::NotReady);
        assert!(origin.is_parked());

        // The Apple route.
        assert!(origin.unpark());
        let served = rt.block_on(async {
            let parker = park_after_wait(&origin);
            let served = origin
                .serve_playlist_apple_within(None, None, "http://x/", 2, Duration::from_secs(5))
                .await;
            parker.await.unwrap();
            served
        });
        assert_eq!(served, PlaylistServe::NotReady);

        // A request that never waited is still handed the held window.
        assert!(matches!(
            rt.block_on(origin.serve_playlist_apple_within(None, None, "http://x/", 2, Duration::from_millis(50))),
            PlaylistServe::Playlist(_)
        ));
        origin.stop();
    }

    #[test]
    fn a_request_wakes_a_parked_origin_through_wake_for_request() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap();
        let _guard = rt.enter();
        let origin = LlOrigin::new(6);
        *origin.live_edge.lock().unwrap() = Some(ts_edge_with_one_segment());
        *origin.upstream.lock().unwrap() = Some("http://x/playlist.m3u8".to_string());
        assert!(origin.park());

        origin.wake_for_request(false);
        assert!(origin.is_parked(), "a request that does not wake leaves it parked");
        assert!(origin.reader_task.lock().unwrap().is_none());

        origin.wake_for_request(true);
        assert!(!origin.is_parked());
        let handle = origin.reader_task.lock().unwrap().take();
        assert!(handle.is_some(), "the wake started a reader");
        handle.unwrap().abort();
    }

    #[test]
    fn idle_parks_only_after_the_span_and_never_before_a_first_request() {
        let span = 30_000;
        assert!(!idle_for_park(0, 10 * span, span), "never asked: nothing to measure");
        assert!(!idle_for_park(1_000, 1_000 + span - 1, span));
        assert!(idle_for_park(1_000, 1_000 + span, span));
        // A clock read before the request (a race) is not idle.
        assert!(!idle_for_park(5_000, 4_000, span));
    }

    #[test]
    fn an_unpark_counts_as_a_request() {
        let origin = LlOrigin::new(6);
        *origin.live_edge.lock().unwrap() = Some(ts_edge_with_one_segment());
        let asked = u64::MAX / 2;
        origin.note_playlist_request_at(asked);
        assert!(!origin.idle_park_due_at(asked + park_idle_ms() - 1));
        assert!(origin.idle_park_due_at(asked + park_idle_ms()), "idle before the unpark");
        assert!(origin.park());
        assert!(!origin.unpark(), "no upstream in a test: nothing restarts");
        let woke = origin.last_playlist_ms.load(Ordering::SeqCst);
        assert_ne!(woke, asked, "the unpark noted a request");
        assert!(!origin.idle_park_due_at(woke), "not idle right after the unpark");
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

    #[test]
    fn a_synced_tile_window_covers_its_delay() {
        assert_eq!(window_for_delay(3.0), TILE_MAX_SEGMENTS, "a low-latency delay needs no more");
        assert_eq!(window_for_delay(10.0), 7, "6 complete segments plus the in-progress one");
        assert_eq!(window_for_delay(40.0), TILE_SYNC_MAX_SEGMENTS, "bounded");
        assert_eq!(window_for_delay(f64::NAN), TILE_MAX_SEGMENTS);
    }

    /// A transmuxed TS edge: one complete segment and one in progress.
    fn apple_fixture() -> LiveEdge {
        let mut segs = VecDeque::new();
        segs.push_back(make_segment(
            50,
            Some("P0".into()),
            true,
            even_parts(vec![vec![1], vec![2]]),
        ));
        segs.push_back(Segment {
            sn: 51,
            pdt: Some("P1".into()),
            complete: false,
            duration: 2.0,
            parts: vec![Part {
                duration: 0.3,
                bytes: Arc::new(vec![3]),
            }],
            assembled: None,
        });
        LiveEdge {
            init_url: String::new(),
            init_bytes: Some(Arc::new(vec![0, 0, 0, 8, b'f', b't', b'y', b'p'])),
            container: Container::Ts,
            target_duration: 2,
            part_target: PART_TARGET,
            segments: segs,
        }
    }

    #[test]
    fn the_hls_js_playlist_is_unchanged_by_the_apple_variant() {
        let expected = "#EXTM3U\n\
#EXT-X-VERSION:9\n\
#EXT-X-TARGETDURATION:2\n\
#EXT-X-PART-INF:PART-TARGET=0.667\n\
#EXT-X-SERVER-CONTROL:CAN-BLOCK-RELOAD=YES,PART-HOLD-BACK=1.500\n\
#EXT-X-MEDIA-SEQUENCE:50\n\
#EXT-X-MAP:URI=\"init.mp4\"\n\
#EXT-X-PROGRAM-DATE-TIME:P0\n\
#EXT-X-PART:DURATION=1.000,URI=\"part/50/0.mp4\",INDEPENDENT=YES\n\
#EXT-X-PART:DURATION=1.000,URI=\"part/50/1.mp4\"\n\
#EXTINF:2.000,live\n\
seg/50.ts\n\
#EXT-X-PROGRAM-DATE-TIME:P1\n\
#EXT-X-PART:DURATION=0.300,URI=\"part/51/0.mp4\",INDEPENDENT=YES\n";
        let edge = apple_fixture();
        assert_eq!(render_locked(&edge), expected);
        // Rendering the Apple variant first leaves the hls.js one untouched.
        let _ = render_apple(&edge, "http://127.0.0.1:9/", 0.334, APPLE_HOLD_BACK_PARTS, 0.0);
        assert_eq!(render_locked(&edge), expected);
        assert!(!expected.contains("PRELOAD-HINT"));
        assert!(!expected.contains("apple=1"));
    }

    #[test]
    fn the_apple_playlist_is_absolute_and_hints_the_next_part() {
        let pl = render_apple(&apple_fixture(), "http://127.0.0.1:9/", 0.334, APPLE_HOLD_BACK_PARTS, 0.0);
        let expected = "#EXTM3U\n\
#EXT-X-VERSION:9\n\
#EXT-X-TARGETDURATION:2\n\
#EXT-X-PART-INF:PART-TARGET=0.334\n\
#EXT-X-SERVER-CONTROL:CAN-BLOCK-RELOAD=YES,PART-HOLD-BACK=1.002\n\
#EXT-X-MEDIA-SEQUENCE:50\n\
#EXT-X-MAP:URI=\"http://127.0.0.1:9/init.mp4\"\n\
#EXT-X-PROGRAM-DATE-TIME:P0\n\
#EXT-X-PART:DURATION=1.000,URI=\"http://127.0.0.1:9/part/50/0.mp4?apple=1\",INDEPENDENT=YES\n\
#EXT-X-PART:DURATION=1.000,URI=\"http://127.0.0.1:9/part/50/1.mp4?apple=1\"\n\
#EXTINF:2.000,live\n\
http://127.0.0.1:9/seg/50.ts\n\
#EXT-X-PROGRAM-DATE-TIME:P1\n\
#EXT-X-PART:DURATION=0.300,URI=\"http://127.0.0.1:9/part/51/0.mp4?apple=1\",INDEPENDENT=YES\n\
#EXT-X-PRELOAD-HINT:TYPE=PART,URI=\"http://127.0.0.1:9/part/51/1.mp4?apple=1\"\n";
        assert_eq!(pl, expected);
    }

    #[test]
    fn the_apple_part_target_fits_the_longest_non_final_part() {
        let mut edge = apple_fixture();
        // Segment 50's parts are 1.0 + 1.0: only the first is non-final.
        assert_eq!(fitted_apple_part_target(&edge), Some(1.04));
        // In-progress segments do not count; with no complete one, nothing fits yet.
        edge.segments.pop_front();
        assert_eq!(fitted_apple_part_target(&edge), None);
        assert_eq!(fallback_apple_part_target(&edge), APPLE_TS_PART_TARGET);
        // 0.300 s parts land on a 0.312 target: 0.265 to 0.312 are accepted.
        edge.segments.push_front(make_segment(
            50,
            None,
            true,
            vec![(vec![1], 0.284), (vec![2], 0.3), (vec![3], 0.2)],
        ));
        assert_eq!(fitted_apple_part_target(&edge), Some(0.312));
    }

    #[test]
    fn an_absolute_cmaf_init_stays_upstream() {
        assert_eq!(
            absolute_uri_attr("#EXT-X-MAP:URI=\"https://cdn/init.mp4\"", "http://127.0.0.1:9/", ""),
            "#EXT-X-MAP:URI=\"https://cdn/init.mp4\""
        );
    }

    #[test]
    fn the_target_gap_moves_the_apple_hold_back() {
        // Auto: three part targets, the default hold-back.
        assert!(gap_adjust(0, 0.312, 3).abs() < 1e-9);
        assert!((gap_adjust(3000, 0.312, 3) - (2.15 - 0.936)).abs() < 1e-9);
        // Never under three part targets, however low the gap.
        assert!(gap_adjust(500, 0.312, 3).abs() < 1e-9);
    }

    #[test]
    fn apple_holds_release_on_the_right_edge_changes() {
        let mut edge = apple_fixture();
        assert!(segment_satisfied(&edge, 50));
        assert!(!segment_satisfied(&edge, 51));
        assert!(!part_never_comes(&edge, 51, 1));
        assert!(part_never_comes(&edge, 50, 2));
        assert!(part_never_comes(&edge, 49, 0));
        assert_eq!(next_part(&edge), Some((51, 1)));
        edge.segments.back_mut().unwrap().complete = true;
        assert!(segment_satisfied(&edge, 51));
        assert!(part_never_comes(&edge, 51, 1));
        assert_eq!(next_part(&edge), Some((52, 0)));
    }

    /// Twitch's CMAF fragment lengths for one 60 fps segment, as the iPhone saw them.
    const CMAF_FRAGMENTS: [f64; 19] = [
        0.134, 0.116, 0.100, 0.100, 0.117, 0.100, 0.117, 0.100, 0.100, 0.116, 0.100, 0.117, 0.100, 0.100,
        0.117, 0.100, 0.116, 0.100, 0.050,
    ];

    /// A native CMAF edge: one complete segment of Twitch fragments, one in progress.
    fn cmaf_fixture() -> LiveEdge {
        let parts = |n: usize| -> Vec<(Vec<u8>, f64)> {
            CMAF_FRAGMENTS.iter().take(n).enumerate().map(|(i, d)| (vec![i as u8], *d)).collect()
        };
        let mut segs = VecDeque::new();
        segs.push_back(make_segment(60, Some("P0".into()), true, parts(19)));
        let mut open = make_segment(61, Some("P1".into()), false, parts(5));
        open.duration = 2.0;
        segs.push_back(open);
        LiveEdge {
            init_url: "https://cdn/init.mp4".into(),
            init_bytes: None,
            container: Container::Cmaf,
            target_duration: 2,
            part_target: PART_TARGET,
            segments: segs,
        }
    }

    #[test]
    fn cmaf_fragments_run_into_parts_that_fit_the_apple_window() {
        let edge = cmaf_fixture();
        let runs = apple_runs(&edge.segments[0]);
        let ends: Vec<usize> = runs.iter().map(|r| r.1).collect();
        assert_eq!(ends, [3, 6, 9, 12, 15, 18, 19]);
        // The open segment lists only its closed run.
        assert_eq!(apple_runs(&edge.segments[1]).len(), 1);

        let view = apple_view(&edge, None).unwrap();
        let target = fitted_apple_run_target(&view).unwrap();
        assert_eq!(target, 0.333);
        let fitted = apple_view(&edge, Some(target)).unwrap();
        let seg = &fitted.segments[0];
        for p in &seg.parts[..seg.parts.len() - 1] {
            assert!(p.duration <= target && p.duration >= target * 0.85, "{} outside the window", p.duration);
        }
        assert!(seg.parts.last().unwrap().duration <= target);
        // TS edges keep their own parts.
        assert!(apple_view(&apple_fixture(), None).is_none());
    }

    #[test]
    fn the_apple_cmaf_playlist_lists_runs_and_the_hls_js_one_is_unchanged() {
        let edge = cmaf_fixture();
        let before = render_locked(&edge);
        assert_eq!(before.matches("#EXT-X-PART:").count(), 24, "every fragment for hls.js");
        let view = apple_view(&edge, Some(0.333)).unwrap();
        let pl = render_apple(&view, "http://127.0.0.1:9/", 0.333, APPLE_HOLD_BACK_PARTS, 0.0);
        assert_eq!(render_locked(&edge), before);
        assert_eq!(pl.matches("#EXT-X-PART:").count(), 8);
        assert!(pl.contains("#EXT-X-PART-INF:PART-TARGET=0.333\n"));
        assert!(pl.contains("#EXT-X-MAP:URI=\"https://cdn/init.mp4\"\n"));
        assert!(pl.contains(
            "#EXT-X-PART:DURATION=0.333,URI=\"http://127.0.0.1:9/part/60/0.mp4?apple=1\",INDEPENDENT=YES\n"
        ));
        assert!(pl.contains("#EXT-X-PART:DURATION=0.050,URI=\"http://127.0.0.1:9/part/60/6.mp4?apple=1\"\n"));
        assert!(pl.ends_with("#EXT-X-PRELOAD-HINT:TYPE=PART,URI=\"http://127.0.0.1:9/part/61/1.mp4?apple=1\"\n"));
    }

    #[test]
    fn a_cmaf_run_is_its_fragments_back_to_back() {
        let edge = cmaf_fixture();
        assert_eq!(apple_run_bytes(&edge.segments[0], 0).unwrap().as_slice(), &[0, 1, 2]);
        assert_eq!(apple_run_bytes(&edge.segments[0], 6).unwrap().as_slice(), &[18]);
        assert!(apple_run_bytes(&edge.segments[0], 7).is_none());
        let view = apple_view(&edge, None).unwrap();
        assert!(part_never_comes(&view, 60, 7));
        assert!(!part_never_comes(&view, 61, 1));
        assert!(blocking_satisfied(&view, 61, 0));
        assert!(!blocking_satisfied(&view, 61, 1));
    }
}
