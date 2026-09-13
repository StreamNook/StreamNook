//! Serves YouTube's video-only DASH renditions to hls.js as ordinary HLS.
//!
//! YouTube publishes 1440p and 2160p live only as video-only `adaptiveFormats`
//! itags, delivered as manifestless DASH fragments addressed by `&sq=N`, in
//! WebM. hls.js plays neither WebM nor DASH. Rather than put a second media
//! engine in the frontend, this rewrites the container (`webm_fmp4`) and the
//! packaging, so the player, Plyr chrome and quality menu stay exactly as they
//! are and simply gain rungs.
//!
//! What is served:
//!
//! ```text
//! /stream.m3u8   master: one video variant plus an audio rendition group
//! /video.m3u8    rolling live media playlist, EXT-X-MAP + fMP4 segments
//! /audio.m3u8    the same for audio
//! /init_v.mp4    generated from the WebM track header
//! /init_a.mp4    YouTube's own audio ftyp+moov, split out of a fragment
//! /v/<sq>.m4s    fetched, transmuxed WebM -> fMP4
//! /a/<sq>.m4s    fetched, moof+mdat sliced out of a self-contained fragment
//! ```
//!
//! This is a separate server from `stream_server` on purpose. That relay is the
//! Twitch low-latency path and is load-bearing; nothing here needs its ad
//! detection, segment projection or LL origin, and keeping them apart means a
//! bug in this path cannot reach Twitch playback.
//!
//! Timestamps are passed through absolutely rather than rebased. YouTube gives
//! video (millisecond ticks) and audio (44100 ticks) positions on one shared
//! media timeline, so leaving them alone is what keeps the two tracks in sync
//! without this module having to understand either clock.

use crate::services::providers::youtube_media::HighRendition;
use crate::services::webm_fmp4::{self, Parsed, Vp9Config};
use anyhow::{anyhow, Context, Result};
use bytes::Bytes;
use once_cell::sync::Lazy;
use rand::Rng;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use warp::Filter;

/// itag 140 is AAC-LC in an MP4 container, which is the only audio these
/// renditions are ever paired with.
const AUDIO_CODEC: &str = "mp4a.40.2";

/// How much wall-clock the advertised window should cover.
///
/// Counted in SECONDS rather than segments because segment length varies by
/// broadcast: a fixed count is a 30s window at 5s segments and a 4s window at
/// 0.66s ones, and the short case is far too shallow to survive a hiccup.
const WINDOW_SECONDS: f64 = 30.0;

/// Floor on the number of advertised segments, so a long-segment stream still
/// offers the player somewhere to seek.
const WINDOW_MIN: u64 = 6;

fn window_for(target: f64) -> u64 {
    if target <= 0.0 {
        return WINDOW_MIN;
    }
    ((WINDOW_SECONDS / target).ceil() as u64).max(WINDOW_MIN)
}

/// How stale the live edge may be before a playlist request refreshes it. The
/// player polls the playlist about every target duration, so this rides its
/// cadence rather than adding a timer of its own.
const HEAD_TTL: Duration = Duration::from_secs(2);

/// How far behind `X-Head-Seqnum` the newest ADVERTISED segment starts.
///
/// The head is the sequence being written, and the one before it has only just
/// closed, so neither is reliably retrievable. Measured on a live broadcast:
/// 16388 fetched fine while 16390 through 16392 all 404ed, putting the real edge
/// around head-3.
const EDGE_BACKOFF_START: u64 = 3;

/// Ceiling for the adaptive backoff. Each step costs one segment of latency, so
/// this caps the damage when something else entirely is wrong.
const EDGE_BACKOFF_MAX: u64 = 8;

/// The upstream status, attached to segment errors so the handler can tell a
/// sequence that has not been written yet (404) from one we are no longer
/// allowed to fetch (403). They need opposite responses and conflating them cost
/// several debugging rounds.
#[derive(Debug, Clone, Copy)]
struct UpstreamStatus(u16);

impl std::fmt::Display for UpstreamStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "upstream {}", self.0)
    }
}

impl std::error::Error for UpstreamStatus {}

fn upstream_status(e: &anyhow::Error) -> Option<u16> {
    e.chain()
        .find_map(|c| c.downcast_ref::<UpstreamStatus>())
        .map(|s| s.0)
}

static HTTP: Lazy<reqwest::Client> = Lazy::new(|| {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
});

struct Session {
    /// Kept so an expired url can be re-issued without unwinding playback.
    video_id: String,
    /// The floor the renditions were resolved against, so a refresh asks the
    /// same question and gets the same rungs back.
    above: u32,
    /// Identifies which rendition to pick out of a refreshed ladder.
    itag: u64,
    /// When the urls below were issued.
    issued: Instant,
    video_url: String,
    audio_url: String,
    width: u32,
    height: u32,
    fps: f64,
    bandwidth: u64,
    video_codec: String,
    /// `Bytes`, so serving an init is a refcount clone rather than a copy.
    init_video: Bytes,
    init_audio: Bytes,
    /// Segment length in seconds, from the media itself.
    target: f64,
    /// Head sequence per track. These are NOT the same number: measured on a
    /// live broadcast, video reported 13305 while audio reported 13306. Sharing
    /// one head makes the lagging track ask for a segment that does not exist
    /// yet, and every request for it 404s until the stream is restarted.
    video_head: u64,
    audio_head: u64,
    video_head_at: Instant,
    audio_head_at: Instant,
    /// Segments held back from the edge, per track, grown when the origin proves
    /// the edge is further back than assumed. Self-tuning because the right
    /// value differs per broadcast and a fixed constant is just a guess that
    /// happens to work on the stream it was measured against.
    video_backoff: u64,
    audio_backoff: u64,
}

/// Everything ONE relayed stream owns.
///
/// This used to be a set of module statics, which meant exactly one YouTube
/// stream could exist per process. A second `start` overwrote the session while
/// the first player kept polling the same port, so it was served the second
/// broadcast's fragments: no error anywhere, just the wrong video. That is why
/// every field below is per-stream rather than shared.
struct DashStream {
    /// Registry key, and the `/s/{id}/` segment on the wire.
    id: String,
    /// Playback identity, minted once per stream. YouTube reads it as "this is
    /// one viewing session of one video", so two concurrent streams must not
    /// share one.
    cpn: String,
    /// Request counter, incremented per media request to match what YouTube's own
    /// player sends alongside `cpn`.
    rn: AtomicU64,
    force_rotate: std::sync::atomic::AtomicBool,
    rotating: std::sync::atomic::AtomicBool,
    session: Mutex<Session>,
    /// Recently served segments, already remuxed. Values are `Bytes` so a hit
    /// serves by refcount instead of copying. Per-stream because sequence
    /// numbers restart per broadcast: two streams that both reach video 13305
    /// would otherwise serve each other's bytes against the wrong init segment.
    segments: Mutex<HashMap<(bool, u64), Bytes>>,
    /// Segments being fetched right now, each behind its own gate.
    ///
    /// A set of keys was not enough: it told a caller that someone else was
    /// already fetching, but gave it nothing to WAIT on, so the foreground
    /// request just fetched the same segment again. Since prefetch warms sq+1
    /// and the player asks for sq+1 next, that was the common case, not a rare
    /// race: every segment was downloaded and remuxed twice.
    inflight: Mutex<HashMap<(bool, u64), Arc<tokio::sync::Mutex<()>>>>,
    last_segment_warn: Mutex<Option<Instant>>,
    /// Throttle for the segment cost breakdown below. Separate from
    /// `last_segment_warn` on purpose: one throttles failures, this throttles a
    /// diagnostic about successes, and sharing one would hide whichever is rarer.
    last_cost_log: Mutex<Option<Instant>>,
    /// Touched by every request. The reaper reads it; nothing here runs on a
    /// timer, so a stream that went away without a stop is collected lazily.
    last_seen: Mutex<Instant>,
}

static REGISTRY: Lazy<Mutex<HashMap<String, Arc<DashStream>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Concurrent relayed streams. Above this a stream stays on the 1080p HLS
/// ladder rather than queueing behind the one shared resolver webview.
const MAX_DASH_STREAMS: usize = 4;

/// A stream nothing has requested for this long is gone: the player closed
/// without a stop, or the window was destroyed. Swept on the next `start`
/// rather than by a ticker, because a ticker would turn every leaked entry into
/// a hidden webview resolve every fifteen seconds.
const IDLE_EVICT: Duration = Duration::from_secs(120);

/// Ids reach the URL path, so keep them to something that needs no escaping and
/// cannot contain the separator `route_of` splits on.
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
}

fn lookup(id: &str) -> Option<Arc<DashStream>> {
    REGISTRY.lock().ok()?.get(id).cloned()
}

/// Mint a playback nonce. One per `DashStream`, generated at construction, so
/// two concurrent streams never share one.
fn mint_nonce() -> String {
    const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut r = rand::rng();
    (0..16).map(|_| A[r.random_range(0..A.len())] as char).collect()
}
static PORT: Lazy<Mutex<Option<u16>>> = Lazy::new(|| Mutex::new(None));
static SERVER: Lazy<Mutex<Option<tokio::task::JoinHandle<()>>>> = Lazy::new(|| Mutex::new(None));

/// Fetch one fragment. `sq` of `None` asks for the live edge, which is also the
/// cheapest way to learn the current head sequence number.
///
/// Free rather than a method, and it returns the head EVEN ON FAILURE, because
/// `start` runs its probes before any stream exists to record them against. The
/// caller decides what to do with the head; recording it is the instance's job.
/// `age` is the current session's url age, purely for the 403 log line.
async fn fetch_fragment(
    cpn: &str,
    rn: &AtomicU64,
    base: &str,
    sq: Option<u64>,
    video: bool,
    age: Option<Duration>,
) -> (Option<u64>, Result<Vec<u8>>) {
    // Every media request carries the playback session identity, the way the
    // real player does.
    let rn = rn.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let url = match sq {
        Some(n) => format!("{}&sq={}&cpn={}&rn={}", base, n, cpn, rn),
        None => format!("{}&cpn={}&rn={}", base, cpn, rn),
    };
    let resp = match HTTP.get(&url).send().await {
        Ok(r) => r,
        Err(e) => return (None, Err(e.into())),
    };
    let head = resp
        .headers()
        .get("x-head-seqnum")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok());
    let status = resp.status();
    // The head is returned from EVERY response, including failures. A 404 for a
    // sequence past the edge still reports where the edge actually is, and that
    // correction is the only way an over-eager head recovers. Recording it is
    // the caller's job now, since this runs before a stream exists.
    if !status.is_success() {
        // Logged, not just returned. A 403 here means the urls stopped being
        // authorised — the difference between "this segment is not written yet"
        // (404, normal at the edge) and "playback is about to die". Returning it
        // silently meant the only symptom was the player giving up.
        if status.as_u16() == 403 {
            // The credential that signed these urls has been refused, so the next
            // resolve must rebuild the resolver's warm session rather than hand
            // back more urls signed with the same dead one. Without this the
            // relay never recovered: it fell back to HLS (1080p ceiling) and
            // every later attempt 403'd instantly on its first fetch.
            crate::services::youtube_potoken::invalidate_session();
            log::warn!(
                "[YouTubeDash] UPSTREAM 403 for {} sq={:?} — urls no longer authorised (age {:?})",
                if video { "video" } else { "audio" },
                sq,
                age
            );
        } else if status.as_u16() != 404 {
            log::warn!(
                "[YouTubeDash] upstream {} for {} sq={:?}",
                status,
                if video { "video" } else { "audio" },
                sq
            );
        }
        // The exact text matters: try_high's `is_refusal` retry matches on
        // "403" appearing in it.
        return (
            head,
            Err(anyhow!(
                "segment {:?} returned {} (origin says head={:?})",
                sq,
                status,
                head
            )
            .context(UpstreamStatus(status.as_u16()))),
        );
    }
    // `Bytes` is already one contiguous buffer; `to_vec` here copied every
    // megabyte of every segment for nothing.
    match resp.bytes().await {
        Ok(b) => (head, Ok(b.into())),
        Err(e) => (head, Err(e.into())),
    }
}

/// Ask for the live edge purely to read `X-Head-Seqnum`. Sequence 0 is always
/// outside the window, so this answers with a 404 and no body.
async fn probe_head(base: &str) -> Option<u64> {
    let resp = HTTP.get(format!("{}&sq=0", base)).send().await.ok()?;
    resp.headers()
        .get("x-head-seqnum")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
}

impl DashStream {
    /// The two media urls, cloned, so no lock is held across an await. That rule
    /// is why this exists rather than callers reading the session inline.
    fn urls(&self) -> Option<(String, String)> {
        let g = self.session.lock().ok()?;
        Some((g.video_url.clone(), g.audio_url.clone()))
    }

    fn touch(&self) {
        if let Ok(mut g) = self.last_seen.lock() {
            *g = Instant::now();
        }
    }

    fn idle_for(&self) -> Duration {
        self.last_seen.lock().ok().map(|g| g.elapsed()).unwrap_or_default()
    }

    /// One fragment, carrying this stream's playback identity and recording the
    /// head it reports.
    ///
    /// A 403 arms a forced rotation for THIS stream only. It used to set a
    /// process-global flag, so one stream's refusal was consumed by whichever
    /// stream rotated next, and the stream that was actually refused never
    /// force-rotated: it kept its dead credential and died on it.
    async fn fragment(&self, base: &str, sq: Option<u64>, video: bool) -> Result<Vec<u8>> {
        let age = self.session.lock().ok().map(|g| g.issued.elapsed());
        let (head, out) = fetch_fragment(&self.cpn, &self.rn, base, sq, video, age).await;
        self.note_head(video, head);
        if let Err(e) = &out {
            if upstream_status(e) == Some(403) {
                // Recover on the NEXT segment request rather than waiting out the
                // rotation clock, so the viewer keeps the resolution they picked
                // instead of watching it stall and drop to the HLS ceiling.
                self.force_rotate.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }
        out
    }

    fn note_head(&self, video: bool, head: Option<u64>) {
    if let (Some(h), Ok(mut g)) = (head, self.session.lock()) {
        {
            let s = &mut *g;
            let (cur, at) = if video {
                (&mut s.video_head, &mut s.video_head_at)
            } else {
                (&mut s.audio_head, &mut s.audio_head_at)
            };
            // Follow the origin in BOTH directions. Taking the maximum makes a
            // single over-reported head permanent, and every later request for a
            // segment past the real edge then 404s forever.
            //
            // A BACKWARDS step is the interesting case: the playlist shrinks under
            // the player, which it sees as the live edge moving away from it. One
            // suspect is the url rotation — the next probe after a swap asks a
            // freshly-issued url, which can disagree with the old one about the
            // edge. Logged with the delta so a stutter can be matched to it.
            if h < *cur {
                log::warn!(
                    "[YouTubeDash] {} head went BACKWARDS {} -> {} ({} segments, rotation in flight: {})",
                    if video { "video" } else { "audio" },
                    *cur,
                    h,
                    *cur - h,
                    self.rotating.load(std::sync::atomic::Ordering::SeqCst)
                );
            } else if h > *cur + 3 {
                log::warn!(
                    "[YouTubeDash] {} head JUMPED {} -> {} (+{} segments)",
                    if video { "video" } else { "audio" },
                    *cur,
                    h,
                    h - *cur
                );
            }
            *cur = h;
            *at = Instant::now();
        }
    }
}

/// A segment we advertised turned out not to exist, so hold back one more.
///
/// Only counts failures at or past the edge we published: an older sequence
/// failing means it aged out of the DVR window, which says nothing about how
/// close to the head is safe.
fn widen_backoff(&self, video: bool, sq: u64) {
    if let Ok(mut g) = self.session.lock() {
        {
            let s = &mut *g;
            let (head, backoff) = if video {
                (s.video_head, &mut s.video_backoff)
            } else {
                (s.audio_head, &mut s.audio_backoff)
            };
            if sq + 1 >= head.saturating_sub(*backoff) && *backoff < EDGE_BACKOFF_MAX {
                *backoff += 1;
                log::info!(
                    "[YouTubeDash] {} edge backoff -> {} (sq {} was not there yet, head {})",
                    if video { "video" } else { "audio" },
                    *backoff,
                    sq,
                    head
                );
            }
        }
    }
}

}

/// Point the relay at a rendition for one stream, priming everything the
/// playlists need.
///
/// Returns the local URL the player should load. Safe to call again for a
/// quality change on the SAME `stream_id`: it replaces that stream's entry and
/// leaves every other stream alone.
pub async fn start(
    stream_id: &str,
    video_id: &str,
    above: u32,
    r: &HighRendition,
) -> Result<String> {
    // The id reaches the URL path, so anything needing escaping is refused here
    // rather than mangled later. The message must not contain "403" or
    // "no longer authorised": try_high's `is_refusal` matches on those and would
    // rebuild the potoken session and retry for nothing.
    if !valid_id(stream_id) {
        return Err(anyhow!("unusable stream id '{}'", stream_id));
    }
    // Lazy reaper. Nothing in this module runs on a timer (a ticker would turn
    // every leaked entry into a hidden webview resolve every fifteen seconds),
    // so abandoned streams are collected the next time one starts.
    reap_idle();
    {
        let reg = REGISTRY.lock().map_err(|_| anyhow!("registry poisoned"))?;
        if !reg.contains_key(stream_id) && reg.len() >= MAX_DASH_STREAMS {
            return Err(anyhow!(
                "already relaying {} streams; this one stays on the HLS ladder",
                reg.len()
            ));
        }
    }
    // One nonce per stream, minted here. This used to be a process-global that
    // was cleared on every start, which rewrote the nonce a running stream was
    // mid-playback on.
    let cpn = mint_nonce();
    let rn = AtomicU64::new(0);
    // The bare url answers with the live EDGE, which is a chunk of whatever the
    // origin has buffered rather than one addressable segment. It is the cheapest
    // way to learn the head sequence, and that is all it is used for.
    let (vhead, edge) = fetch_fragment(&cpn, &rn, &r.video_url, None, true, None).await;
    edge?;
    let head = vhead.unwrap_or(0);
    if head == 0 {
        return Err(anyhow!("origin reported no head sequence"));
    }

    // Everything else comes from a REAL numbered segment.
    //
    // Never take the duration from the edge chunk: it covers several segments, so
    // EXTINF comes out far too long, the player paces itself slower than the origin
    // produces, and it falls behind until it is chasing sequences that do not
    // exist. The symptom looks like an edge problem (404s near the head); the cause
    // is the advertised segment duration.
    let probe_sq = head.saturating_sub(EDGE_BACKOFF_START);
    let (_, vbytes) = fetch_fragment(&cpn, &rn, &r.video_url, Some(probe_sq), true, None).await;
    let vbytes = vbytes?;
    let parsed: Parsed = webm_fmp4::parse(&vbytes)?;
    let cfg = Vp9Config::from_keyframe(
        &parsed.samples[0].data,
        parsed.track.width,
        parsed.track.height,
        r.fps,
    );
    let init_video = webm_fmp4::init_segment(&parsed.track, &cfg);
    let target = (parsed.duration() as f64 / parsed.track.timescale as f64).max(0.2);

    let (ahead, abytes) = fetch_fragment(&cpn, &rn, &r.audio_url, Some(probe_sq), false, None).await;
    let abytes = abytes?;
    let (init_audio, _) = webm_fmp4::split_audio_fragment(&abytes)?;
    if init_audio.is_empty() {
        return Err(anyhow!("audio fragment carried no init"));
    }

    let session = Session {
        video_id: video_id.to_string(),
        above,
        itag: r.itag,
        issued: Instant::now(),
        video_url: r.video_url.clone(),
        audio_url: r.audio_url.clone(),
        width: r.width,
        height: r.height,
        fps: r.fps,
        bandwidth: r.bandwidth.max(1),
        video_codec: cfg.codec_string(),
        init_video: init_video.into(),
        init_audio: init_audio.into(),
        target,
        video_head: head,
        audio_head: ahead.unwrap_or(head),
        video_head_at: Instant::now(),
        audio_head_at: Instant::now(),
        video_backoff: EDGE_BACKOFF_START,
        audio_backoff: EDGE_BACKOFF_START,
    };

    // A brand new instance, never a mutation of one already registered: an
    // in-flight request holding the old Arc finishes against the rendition it
    // started on rather than being handed a different one's bytes mid-segment.
    // Sequence numbers restart per broadcast, so a fresh cache comes with it.
    let stream = Arc::new(DashStream {
        id: stream_id.to_string(),
        cpn,
        rn,
        force_rotate: std::sync::atomic::AtomicBool::new(false),
        rotating: std::sync::atomic::AtomicBool::new(false),
        session: Mutex::new(session),
        segments: Mutex::new(HashMap::new()),
        inflight: Mutex::new(HashMap::new()),
        last_segment_warn: Mutex::new(None),
        last_cost_log: Mutex::new(None),
        last_seen: Mutex::new(Instant::now()),
    });
    REGISTRY
        .lock()
        .map_err(|_| anyhow!("registry poisoned"))?
        .insert(stream_id.to_string(), stream);

    log::info!(
        "[YouTubeDash] {}: {} ({}x{}@{}) itag={} codec={} segment={:.3}s head={}",
        stream_id,
        r.name,
        r.width,
        r.height,
        r.fps,
        r.itag,
        cfg.codec_string(),
        target,
        head
    );

    let port = ensure_server()?;
    // The id lives in the PATH, not the query. Every reference the playlists emit
    // is a bare relative name (`video.m3u8`, `init_v.mp4`, `v/97.m4s`), and a
    // player resolves those against the playlist's own URL, so a path prefix is
    // inherited by every child request for free and the playlist bodies need no
    // change at all. A query parameter would be dropped by that same resolution,
    // and `warp::path::full()` cannot see one anyway.
    //
    // `?t=` stays: it is the React effect key. Without it a quality change that
    // reuses the same id returns a byte-identical URL, the effect never re-runs,
    // and the menu relabels while playback stays on the old rendition.
    Ok(format!(
        "http://localhost:{}/s/{}/stream.m3u8?t={}",
        port,
        stream_id,
        chrono::Utc::now().timestamp_millis()
    ))
}

/// Drop streams nothing has asked about for a while.
///
/// A leaked entry is inert (every clock in this module is read inside `handle`
/// on an incoming request), but it holds two init segments plus up to six
/// remuxed fragments, and it counts against MAX_DASH_STREAMS, so the next real
/// stream would be refused and silently fall back to 1080p.
fn reap_idle() {
    let Ok(mut reg) = REGISTRY.lock() else { return };
    reg.retain(|id, st| {
        let keep = st.idle_for() < IDLE_EVICT;
        if !keep {
            log::info!("[YouTubeDash] {}: idle, dropping the relay", id);
        }
        keep
    });
}

/// Drop ONE stream. A missing id is a deliberate no-op: this is called from the
/// ordinary sub-1080p resolve path far more often than a stream actually ends.
///
/// The shared server keeps running; it is inert for an id it does not know.
pub async fn stop(stream_id: &str) {
    let removed = REGISTRY
        .lock()
        .ok()
        .and_then(|mut r| r.remove(stream_id))
        .is_some();
    if removed {
        log::info!("[YouTubeDash] {}: relay stopped", stream_id);
    }
    // Only once nothing is left. The resolver webview is shared, and every
    // running stream rotates through it: closing it because ONE tile went away
    // would leave the others playing for about fifteen seconds and then failing
    // one at a time, minutes after the action that caused it.
    close_resolver_if_idle(removed).await;
}

/// Grid teardown: drop every stream except the one named, normally the solo
/// player's. Exiting the grid must not kill a stream playing behind it.
pub async fn stop_all_except(keep: &str) {
    let dropped = {
        let Ok(mut reg) = REGISTRY.lock() else { return };
        let before = reg.len();
        reg.retain(|id, _| id == keep);
        before != reg.len()
    };
    close_resolver_if_idle(dropped).await;
}

async fn close_resolver_if_idle(removed_something: bool) {
    if !removed_something {
        return;
    }
    let empty = REGISTRY.lock().map(|r| r.is_empty()).unwrap_or(false);
    if empty {
        crate::services::youtube_potoken::close_resolver().await;
    }
}

/// One server for the process, serving every stream by path.
///
/// The route captures nothing per-stream, so there is nothing a second listener
/// would isolate. A port per stream would instead mean N chances to lose a bind
/// race, and a failed start here degrades silently to the 1080p HLS ceiling.
///
/// The PORT guard is held across the whole function: there are no awaits inside,
/// so two concurrent starts cannot both bind and leak one of the listeners.
fn ensure_server() -> Result<u16> {
    let mut port_guard = PORT.lock().map_err(|_| anyhow!("port poisoned"))?;
    if let Some(p) = *port_guard {
        return Ok(p);
    }

    let route = warp::path::full()
        .and_then(|p: warp::path::FullPath| async move { handle(p.as_str().to_string()).await })
        .boxed();

    // Port 0 lets the OS pick a free one, which removes the collision class
    // entirely. Nothing bakes in a port range: the CSP is connect-src *, and the
    // frontend only ever follows the URL this returns.
    let socket = tokio::net::TcpSocket::new_v4()?;
    socket.bind(SocketAddr::from(([127, 0, 0, 1], 0)))?;
    let port = socket.local_addr()?.port();
    let listener = socket.listen(512)?;
    let handle = tokio::spawn(async move {
        warp::serve(route).incoming(listener).run().await;
    });
    *SERVER.lock().map_err(|_| anyhow!("server poisoned"))? = Some(handle);
    *port_guard = Some(port);
    log::info!("[YouTubeDash] serving on port {}", port);
    Ok(port)
}

/// Split `/s/{id}/{tail}` once, at the top, so every match arm below keeps the
/// exact pattern it had when there was only one stream. `v/97.m4s` has to reach
/// the segment arm unchanged, or the playlists serve while every segment 404s.
fn route_of(path: &str) -> Option<(&str, &str)> {
    let rest = path.trim_start_matches('/').strip_prefix("s/")?;
    let (id, tail) = rest.split_once('/')?;
    (!id.is_empty() && !tail.is_empty()).then_some((id, tail))
}

fn cors(body: impl Into<Bytes>, content_type: &str) -> warp::http::Response<Bytes> {
    warp::http::Response::builder()
        .status(200)
        .header("Content-Type", content_type)
        .header("Access-Control-Allow-Origin", "*")
        .header("Cache-Control", "no-cache")
        .body(body.into())
        .unwrap_or_else(|_| warp::http::Response::new(Bytes::new()))
}

fn fail(code: u16) -> warp::http::Response<Bytes> {
    fail_because(code, String::new())
}

/// Answer with the REASON attached, in the body and in a header.
///
/// The relay's own logs go to the Rust terminal while the person debugging is
/// usually looking at the browser console, where a bare 404 says nothing. This
/// puts the cause where the failure is already visible: hls.js prints the status,
/// and the Network tab shows the body.
fn fail_because(code: u16, why: String) -> warp::http::Response<Bytes> {
    let mut b = warp::http::Response::builder()
        .status(code)
        .header("Access-Control-Allow-Origin", "*")
        .header("Cache-Control", "no-store");
    if !why.is_empty() {
        // Header values cannot carry newlines or non-ASCII.
        let one_line: String = why
            .chars()
            .map(|c| if c.is_ascii_graphic() || c == ' ' { c } else { ' ' })
            .take(400)
            .collect();
        b = b.header("X-SN-Reason", one_line);
    }
    b.body(Bytes::from(why.into_bytes()))
        .unwrap_or_else(|_| warp::http::Response::new(Bytes::new()))
}

impl DashStream {
    fn head_is_stale(&self, video: bool) -> bool {
        self.session
            .lock()
            .ok()
            .map(|s| {
                let (h, at) = if video {
                    (s.video_head, s.video_head_at)
                } else {
                    (s.audio_head, s.audio_head_at)
                };
                at.elapsed() > HEAD_TTL || h == 0
            })
            .unwrap_or(false)
    }
}

/// Gated urls stop working about thirty seconds after they are issued, so they
/// are swapped out before they get there. It is an AGE limit, not a request
/// quota: the same wall clock expires them whatever the request count.
///
/// Fifteen leaves roughly half the window as headroom, because the refresh runs
/// in the background and a webview resolve is not instant. Playback keeps using
/// the current urls the whole time it is in flight.
pub(crate) const ROTATE_AFTER: Duration = Duration::from_secs(15);

/// Rotate on the next segment request regardless of url age.
///
/// Age is the right trigger for the ordinary case (urls expire on a clock), but
/// a REFUSED url can be seconds old, and waiting out the rest of `ROTATE_AFTER`
/// with a credential the origin has already rejected is dead air. Set on a 403 so
/// recovery starts on the next request instead of at the next tick.
static FORCE_ROTATE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Only urls carrying a Proof-of-Origin token expire this way. The ones the
/// visionos client hands out are plain and last as long as their `expire` says,
/// so they are left alone rather than being churned for nothing.
pub(crate) fn is_gated(url: &str) -> bool {
    url.contains("&pot=") || url.contains("?pot=")
}

/// Re-issue the media urls if the ones in hand are about to go stale.
///
/// Playback continues across the swap untouched: only the base urls change, and
/// the sequence numbers, heads and backoffs all carry forward, because the new
/// urls address the same segments of the same broadcast.
/// One refresh at a time. Requests arrive far faster than a re-resolve
/// completes, so without this every segment in the window would start its own.
static ROTATING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

// Rate limiting for the slow-serve warning: it is per-request and otherwise
// floods the app log on any sluggish stream.
const SLOW_SERVE_LOG_EVERY_MS: u64 = 30_000;
static SLOW_SERVE_EPOCH: OnceLock<Instant> = OnceLock::new();
static SLOW_SERVE_LAST_LOG_MS: AtomicU64 = AtomicU64::new(0);
static SLOW_SERVE_SUPPRESSED: AtomicU64 = AtomicU64::new(0);

/// Start a refresh if the urls in hand are getting old. Returns immediately.
///
/// This deliberately does NOT wait for the new urls. A re-resolve takes seconds
/// (on the gated path it spawns a webview), and blocking a segment request on it
/// stalls playback for exactly as long as it takes, which shows up as a stutter
/// on a fixed cadence. The gap between `ROTATE_AFTER` and the ~30s wall is the
/// headroom that lets the swap land while the current urls still work.
impl DashStream {
    fn rotation_due(&self) -> Option<(String, u32, u64)> {
        let g = self.session.lock().ok()?;
        let forced = self.force_rotate.load(std::sync::atomic::Ordering::SeqCst);
        let due = is_gated(&g.video_url) && (forced || g.issued.elapsed() >= ROTATE_AFTER);
        due.then(|| (g.video_id.clone(), g.above, g.itag))
    }

    fn rotate_if_stale(self: &Arc<Self>) {
        let Some((video_id, above, itag)) = self.rotation_due() else {
            return;
        };

        use std::sync::atomic::Ordering;
        if self.rotating.swap(true, Ordering::SeqCst) {
            return;
        }
        // Cleared only once a rotation is actually under way, so a refusal can never
        // be swallowed by a concurrent caller that then declined to rotate.
        self.force_rotate.store(false, Ordering::SeqCst);

        let me = Arc::clone(self);
        tokio::spawn(async move {
            let outcome = me.refresh_urls(&video_id, above, itag).await;
            me.rotating.store(false, Ordering::SeqCst);
            if let Err(e) = outcome {
                log::warn!(
                    "[YouTubeDash] {}: could not re-issue urls for itag {}: {}",
                    me.id,
                    itag,
                    e
                );
            }
        });
    }
}

/// Swap the host of a videoplayback url, keeping everything else byte for byte.
///
/// The signature covers query parameters, not the host, so an edge in the same
/// pool serves the same url happily. Verified 2026-08-21: a url issued for
/// `rr1---sn-iv` returned 200 from `rr9---sn-iv`.
fn with_host(url: &str, host: &str) -> Option<String> {
    let rest = url.strip_prefix("https://")?;
    let slash = rest.find('/')?;
    Some(format!("https://{}{}", host, &rest[slash..]))
}

fn host_of(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("https://")?;
    Some(&rest[..rest.find('/')?])
}

/// Confirm a url plays, and leave a warm connection to its host behind.
///
/// `HEAD` answers in about a tenth of a second with no body, so this costs
/// almost nothing, and it runs off the playback path.
async fn probe(url: &str) -> bool {
    match HTTP.head(url).send().await {
        Ok(r) => r.status().is_success(),
        Err(_) => false,
    }
}

/// Choose between a freshly issued url and the same url pointed back at the host
/// already in use.
///
/// Every resolve lands on a different edge (`rr1` -> `rr13` -> `rr10` observed
/// back to back). Following it means a DNS lookup, a TCP connect and a TLS
/// handshake before the very next segment, which is a visible hitch mid
/// playback. Staying put keeps the pooled connection.
async fn pick_url(fresh: &str, current: &str) -> String {
    if let Some(host) = host_of(current) {
        if host_of(fresh) != Some(host) {
            if let Some(pinned) = with_host(fresh, host) {
                if probe(&pinned).await {
                    return pinned;
                }
                log::debug!("[YouTubeDash] host {} would not serve the new url", host);
            }
        }
    }
    // Either the host was unchanged, or it refused: warm the new one instead so
    // the handshake is paid here rather than on the next segment.
    probe(fresh).await;
    fresh.to_string()
}

impl DashStream {
    async fn refresh_urls(&self, video_id: &str, above: u32, itag: u64) -> Result<()> {
    // Bounded by AGE, never by invalidating first.
    //
    // Invalidating before the resolver lock lets two surfaces on ONE broadcast each
    // wipe the other's just-stored result, so the coalescing re-check inside that
    // lock never hits and both pay a full hidden-webview resolve. The registry is
    // keyed by stream id precisely so one broadcast CAN be open twice, which makes
    // that a supported case rather than an exotic one.
    //
    // REUSE_WITHIN + ROTATE_AFTER must stay under the ~30s gated-url wall, because
    // a stream adopting an entry of that age records `issued` as now and will not
    // rotate again for another ROTATE_AFTER. 5 + 15 = 20s leaves ~10s of headroom.
    // Straight to the gated resolver rather than through `high_renditions`.
    // Nothing but a gated url is ever rotated, so re-asking the visionos client
    // every fifteen seconds would only re-learn that it has nothing to offer,
    // at the cost of a watch-page scrape and a player call each time.
    const REUSE_WITHIN: Duration = Duration::from_secs(5);
    let resolved =
        crate::services::youtube_potoken::resolve_streams_fresh(video_id, above, REUSE_WITHIN)
            .await?;
    let fresh = resolved
        .videos
        .iter()
        .find(|v| v.itag as u64 == itag)
        .ok_or_else(|| anyhow!("itag {} is no longer offered by '{}'", itag, video_id))?;

    let (cur_video, cur_audio) = self
        .urls()
        .ok_or_else(|| anyhow!("session ended mid-refresh"))?;
    // Both settled BEFORE the swap, so the urls that go live are already proven
    // and already have a connection open.
    let video_url = pick_url(&fresh.url, &cur_video).await;
    let audio_url = pick_url(&resolved.audio_url, &cur_audio).await;

    let mut g = self.session.lock().map_err(|_| anyhow!("session poisoned"))?;
    // The session may have been restarted onto a different rendition while this
    // was in flight; overwriting it then would silently switch quality.
    if g.itag != itag {
        return Err(anyhow!("session moved to itag {} mid-refresh", g.itag));
    }
    g.video_url = video_url;
    g.audio_url = audio_url;
    g.issued = Instant::now();
    log::info!("[YouTubeDash] {}: re-issued urls for itag {}", self.id, itag);
    Ok(())
    }
}

async fn handle(full: String) -> Result<warp::http::Response<Bytes>, std::convert::Infallible> {
    let started = Instant::now();
    // No `?` handling: warp::path::full() yields the path only, never the query.
    let Some((id, tail)) = route_of(&full) else {
        return Ok(fail_because(404, format!("unroutable path {}", full)));
    };
    let path = tail.to_string();

    let Some(st) = lookup(id) else {
        // 503, not 404: this is what an in-flight request gets after its stream
        // stopped, and hls.js treats the two differently.
        return Ok(fail_because(503, format!("no session for stream {}", id)));
    };
    // Proof of life for the reaper.
    st.touch();
    // Every request goes through here, so the refresh rides the player's own
    // cadence instead of needing a timer of its own. It does not block: the
    // current urls stay good for another ten seconds or so.
    let was_rotating = st.rotating.load(std::sync::atomic::Ordering::SeqCst);
    st.rotate_if_stale();

    // Anything the player waits on for more than a beat is a candidate for the
    // visible hitch, so name it with whether a url rotation was in flight. A
    // media playlist is served from memory and should be sub-millisecond; the
    // only awaits on this path are the live-edge head probes.
    let report = |kind: &str| {
        let ms = started.elapsed().as_millis();
        if ms >= 250 {
            // Rate limited: this fires per request and buried the app log
            // (thousands of lines an hour) under a stream that is merely slow,
            // which is exactly the noise that hides a real chat fault. One line
            // per window, carrying however many were suppressed, keeps the
            // signal without the flood.
            let suppressed = SLOW_SERVE_SUPPRESSED.fetch_add(1, Ordering::Relaxed);
            let now_ms = SLOW_SERVE_EPOCH
                .get_or_init(std::time::Instant::now)
                .elapsed()
                .as_millis() as u64;
            let last = SLOW_SERVE_LAST_LOG_MS.load(Ordering::Relaxed);
            // `last == 0` means nothing has been reported yet. Without this the
            // very first slow serve after startup, usually the most telling one,
            // was swallowed for the first 30 seconds of the process.
            let due = last == 0 || now_ms.saturating_sub(last) >= SLOW_SERVE_LOG_EVERY_MS;
            // Stamp at least 1 so the "never reported" sentinel cannot recur.
            let stamp = now_ms.max(1);
            if due
                && SLOW_SERVE_LAST_LOG_MS
                    .compare_exchange(last, stamp, Ordering::Relaxed, Ordering::Relaxed)
                    .is_ok()
            {
                SLOW_SERVE_SUPPRESSED.store(0, Ordering::Relaxed);
                log::warn!(
                    "[YouTubeDash] {}: slow serve: {} took {}ms (rotation in flight: {}; {} more suppressed since the previous report)",
                    st.id,
                    kind,
                    ms,
                    was_rotating || st.rotating.load(Ordering::SeqCst),
                    suppressed
                );
            }
        }
    };

    let res = match path.as_str() {
        "stream.m3u8" => st
            .session
            .lock()
            .map(|g| cors(master(&g).into_bytes(), "application/vnd.apple.mpegurl"))
            .map_err(|_| anyhow!("session poisoned")),
        "video.m3u8" | "audio.m3u8" => {
            let video = path.starts_with("video");
            // Refresh the live edge only when it has aged out, so this rides the
            // player's own playlist cadence instead of adding a timer.
            // Each track's own head, from its own url: they advance
            // independently and one cannot stand in for the other.
            if st.head_is_stale(video) {
                if let Some((v, a)) = st.urls() {
                    let base = if video { v } else { a };
                    st.note_head(video, probe_head(&base).await);
                }
            }
            st.session
                .lock()
                .map_err(|_| anyhow!("session poisoned"))
                .and_then(|g| media_playlist(&g, video))
                .map(|m| cors(m.into_bytes(), "application/vnd.apple.mpegurl"))
        }
        "init_v.mp4" | "init_a.mp4" => {
            let video = path.starts_with("init_v");
            st.session
                .lock()
                .ok()
                .map(|g| {
                    if video {
                        g.init_video.clone()
                    } else {
                        g.init_audio.clone()
                    }
                })
                .map(|b| cors(b, "video/mp4"))
                .ok_or_else(|| anyhow!("no init"))
        }
        p if p.starts_with("v/") || p.starts_with("a/") => {
            let video = p.starts_with("v/");
            match p[2..].trim_end_matches(".m4s").parse::<u64>() {
                Ok(sq) => st.segment(video, sq).await.map(|b| cors(b, "video/mp4")),
                Err(_) => Err(anyhow!("bad sequence")),
            }
        }
        _ => Err(anyhow!("not found")),
    };
    report(&path);

    Ok(match res {
        Ok(r) => r,
        Err(e) => {
            // A failing fragment used to log at debug, so the first real stall in
            // the app showed as nothing but hls.js 404 spam with no cause. The
            // upstream status and the head it reported are the whole diagnosis,
            // so say them, throttled to one line a second rather than one per
            // retry.
            if path.starts_with("v/") || path.starts_with("a/") {
                let say = st
                    .last_segment_warn
                    .lock()
                    .ok()
                    .map(|mut g| {
                        let due = g.map(|t| t.elapsed() > Duration::from_secs(1)).unwrap_or(true);
                        if due {
                            *g = Some(Instant::now());
                        }
                        due
                    })
                    .unwrap_or(false);
                if say {
                    log::warn!("[YouTubeDash] {}: {} -> {}", st.id, path, e);
                }
            } else {
                log::debug!("[YouTubeDash] {}: {} -> {}", st.id, path, e);
            }
            fail_because(404, format!("{}: {}", path, e))
        }
    })
}

/// Free, over a borrowed session, so the playlist tests can build one by hand
/// and assert on the text without any shared state to serialise around.
fn master(s: &Session) -> String {
    format!(
        "#EXTM3U\n\
         #EXT-X-VERSION:7\n\
         #EXT-X-INDEPENDENT-SEGMENTS\n\
         #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aud\",NAME=\"Audio\",DEFAULT=YES,AUTOSELECT=YES,URI=\"audio.m3u8\"\n\
         #EXT-X-STREAM-INF:BANDWIDTH={},RESOLUTION={}x{},FRAME-RATE={:.3},CODECS=\"{},{}\",AUDIO=\"aud\"\n\
         video.m3u8\n",
        s.bandwidth, s.width, s.height, s.fps, s.video_codec, AUDIO_CODEC
    )
}

fn media_playlist(s: &Session, video: bool) -> Result<String> {
    let head = if video { s.video_head } else { s.audio_head };
    if head == 0 {
        return Err(anyhow!("live edge unknown"));
    }
    let backoff = if video { s.video_backoff } else { s.audio_backoff };
    let newest = head.saturating_sub(backoff);
    let first = newest.saturating_sub(window_for(s.target) - 1);
    let (dir, init) = if video { ("v", "init_v.mp4") } else { ("a", "init_a.mp4") };

    let mut m = format!(
        "#EXTM3U\n\
         #EXT-X-VERSION:7\n\
         #EXT-X-TARGETDURATION:{}\n\
         #EXT-X-MEDIA-SEQUENCE:{}\n\
         #EXT-X-MAP:URI=\"{}\"\n",
        s.target.ceil() as u64,
        first,
        init
    );
    for n in first..=newest {
        m.push_str(&format!("#EXTINF:{:.3},\n{}/{}.m4s\n", s.target, dir, n));
    }
    Ok(m)
}

impl DashStream {
    async fn segment(self: &Arc<Self>, video: bool, sq: u64) -> Result<Bytes> {
    if let Some(hit) = self.cached_segment(video, sq) {
        return Ok(hit);
    }
    let out = self.fetch_once(video, sq).await?;
    // With this one served, warm the NEXT one so its round trip happens while the
    // player is still chewing on this segment instead of inside its next request.
    self.prefetch_next(video, sq);
    Ok(out)
}

/// Fetch and remux a segment AT MOST ONCE, however many callers want it.
///
/// The gate is a per-key async mutex rather than a notification, deliberately: a
/// waiter that clones a notifier, drops the map lock and only then awaits can
/// miss a wake that lands in between, and the cost of that bug is a stalled
/// segment rather than a duplicated one. Taking a lock has no such window.
///
/// Every path re-checks the cache AFTER acquiring, so the second caller through
/// pays nothing, and a first caller that FAILED leaves the second to try for
/// itself rather than inheriting an error it never saw.
async fn fetch_once(self: &Arc<Self>, video: bool, sq: u64) -> Result<Bytes> {
    let gate = {
        let mut f = self
            .inflight
            .lock()
            .map_err(|_| anyhow!("inflight poisoned"))?;
        Arc::clone(
            f.entry((video, sq))
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
        )
    };
    let _held = gate.lock().await;

    if let Some(hit) = self.cached_segment(video, sq) {
        return Ok(hit);
    }

    // `Bytes::from(Vec)` takes ownership of the remux buffer; the cache insert
    // and the served body then share it by refcount.
    let outcome = self
        .fetch_and_remux(video, sq)
        .await
        .map(Bytes::from)
        .inspect(|out| self.store_segment(video, sq, out.clone()));

    // Dropped only after the bytes are cached, so a caller arriving now takes the
    // cache hit rather than opening a fresh gate. A waiter already blocked holds
    // its own Arc, so removing the entry cannot strand it.
    if let Ok(mut f) = self.inflight.lock() {
        f.remove(&(video, sq));
    }
    outcome
}

/// Fetch one segment upstream and convert it to fMP4.
async fn fetch_and_remux(&self, video: bool, sq: u64) -> Result<Vec<u8>> {
    let (vurl, aurl) = self.urls().ok_or_else(|| anyhow!("no session"))?;
    let base = if video { vurl } else { aurl };
    let fetch_started = Instant::now();
    let bytes = match self.fragment(&base, Some(sq), video).await {
        Ok(v) => v,
        Err(e) => {
            // Only a 404 means "not written yet". A 403 means the URL is no
            // longer authorised, and widening the backoff for it just walks
            // further from the edge while never fixing anything.
            if upstream_status(&e) != Some(403) {
                self.widen_backoff(video, sq);
            }
            return Err(e);
        }
    };
    // Kept on a blocking worker so a parse can never occupy an async worker that
    // the playlist, the other track's fetch or the next segment is waiting on.
    //
    // MEASURED 2026-08-28, because this comment used to claim the parse was
    // "CPU-bound over a multi-megabyte buffer" and that claim sent two separate
    // investigations down the wrong path. Over 22 steady-state 1440p60 segments:
    //
    //     size  median 2146KB      FETCH median 1134ms      REMUX median 1ms
    //
    // The remux is 0.1% of the cost. The parse is effectively free and always
    // was; the entire per-segment time is pulling bytes from googlevideo. Do NOT
    // reason about this path as though transmuxing were expensive. The
    // spawn_blocking stays because it is correct and costs nothing, not because
    // the work behind it is heavy.
    let fetch_ms = fetch_started.elapsed().as_millis();
    let raw_len = bytes.len();
    let remux_started = Instant::now();
    let out: Result<Vec<u8>> = tokio::task::spawn_blocking(move || {
        if video {
            let parsed = webm_fmp4::parse(&bytes)?;
            // Sequence number doubles as the moof sequence, which keeps it unique
            // and monotonic without tracking a separate counter.
            Ok(webm_fmp4::media_segment(sq as u32, &parsed))
        } else {
            let (_, media) = webm_fmp4::split_audio_fragment(&bytes)?;
            Ok(media)
        }
    })
    .await
    .map_err(|e| anyhow!("remux task failed: {}", e))?;

    // WHERE a slow segment actually goes. Logged only when it is slow, and at
    // most once a second per stream, because the point is to answer one question
    // rather than to narrate every segment: is the cost the gated googlevideo
    // fetch, or is it running a 1440p60 VP9 fragment through a full WebM parser
    // for a tile a few hundred pixels wide?
    //
    // This exists because the two costs were only ever measured TOGETHER, and the
    // gated resolve path reports bitrate 0, so even the segment size was unknown.
    // Without the split, "1.3s per serve" cannot be acted on: one answer means the
    // relay is innocent, the other means 1440p60 is simply the wrong rendition to
    // hand a small tile.
    let remux_ms = remux_started.elapsed().as_millis();
    if fetch_ms + remux_ms >= 250 {
        let due = self
            .last_cost_log
            .lock()
            .ok()
            .map(|mut g| {
                let due = g.map(|t| t.elapsed() > Duration::from_secs(1)).unwrap_or(true);
                if due {
                    *g = Some(Instant::now());
                }
                due
            })
            .unwrap_or(false);
        if due {
            log::info!(
                "[YouTubeDash] {}: slow {} sq={} {}KB fetch={}ms remux={}ms",
                self.id,
                if video { "video" } else { "audio" },
                sq,
                raw_len / 1024,
                fetch_ms,
                remux_ms
            );
        }
    }
    out
}

/// A couple of seconds of look-ahead per track is all the player is ever ahead
/// by; more would just hold megabytes for segments it may never ask for.
/// Per INSTANCE now, so N streams cost N of these rather than fighting over one.
const SEGMENT_CACHE_MAX: usize = 6;

fn cached_segment(&self, video: bool, sq: u64) -> Option<Bytes> {
    self.segments.lock().ok()?.get(&(video, sq)).cloned()
}

fn store_segment(&self, video: bool, sq: u64, bytes: Bytes) {
    if let Ok(mut c) = self.segments.lock() {
        // Keyed by sequence, so "oldest" is simply the lowest number on this
        // track. Live playback only ever moves forward.
        if c.len() >= Self::SEGMENT_CACHE_MAX {
            if let Some(&oldest) = c.keys().filter(|(v, _)| *v == video).min_by_key(|(_, n)| *n) {
                c.remove(&oldest);
            }
        }
        c.insert((video, sq), bytes);
    }
}

/// Drop everything cached for this stream.
fn clear_segment_cache(&self) {
    if let Ok(mut c) = self.segments.lock() {
        c.clear();
    }
    if let Ok(mut f) = self.inflight.lock() {
        f.clear();
    }
}

/// Warm `sq + 1` in the background.
///
/// Deliberately conservative: never past the live edge (that 404s, and a 404
/// widens the backoff, which would walk the playlist AWAY from live), never a
/// duplicate of something cached or already in flight, and failures are dropped
/// silently — this is an optimisation, and the real request will report any
/// genuine problem itself.
fn prefetch_next(self: &Arc<Self>, video: bool, sq: u64) {
    let next = sq + 1;
    let head = {
        let Ok(g) = self.session.lock() else { return };
        if video { g.video_head } else { g.audio_head }
    };
    if head == 0 || next >= head {
        return; // not written yet upstream
    }
    if self.cached_segment(video, next).is_some() {
        return;
    }
    // Cheap pre-check only. `fetch_once` is the real guard; this just avoids
    // spawning a task that would immediately block on a gate someone else holds.
    {
        let Ok(f) = self.inflight.lock() else { return };
        if f.contains_key(&(video, next)) {
            return; // already being fetched
        }
    }
    let me = Arc::clone(self);
    tokio::spawn(async move {
        // Through the same gate as a foreground request, which is what lets the
        // player JOIN this fetch instead of starting a second one. Failures stay
        // silent: this is an optimisation, and the real request reports for
        // itself.
        let _ = me.fetch_once(video, next).await;
    });
}
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A session built by hand. There is no shared state to serialise around
    /// any more: the playlist functions take a borrowed Session, so each test
    /// owns its own and they can run in any order or in parallel. The lock this
    /// replaced existed purely because the relay had ONE session per process.
    fn a_session(target: f64, head: u64) -> Session {
        a_session_with_heads(target, head, head)
    }

    fn a_session_with_heads(target: f64, video_head: u64, audio_head: u64) -> Session {
        Session {
            video_id: "vid".into(),
            above: 1080,
            itag: 308,
            issued: Instant::now(),
            video_url: "https://example/v".into(),
            audio_url: "https://example/a".into(),
            width: 2560,
            height: 1440,
            fps: 60.0,
            bandwidth: 9_016_000,
            video_codec: "vp09.00.50.08".into(),
            init_video: Bytes::from(vec![1, 2, 3]),
            init_audio: Bytes::from(vec![4, 5, 6]),
            target,
            video_head,
            audio_head,
            video_head_at: Instant::now(),
            audio_head_at: Instant::now(),
            video_backoff: EDGE_BACKOFF_START,
            audio_backoff: EDGE_BACKOFF_START,
        }
    }


    /// Drive the WHOLE path against a live broadcast: VISIONOS resolve, relay
    /// start, playlist generation, fragment fetch, transmux. Writes the result
    /// so it can be handed to a real decoder, which is the only check that
    /// proves the bytes are playable rather than merely well-shaped.
    ///
    /// ```text
    /// STREAMNOOK_YT_LIVE_ID=<videoId> STREAMNOOK_DASH_OUT=<dir>     ///   cargo test end_to_end_against_a_live_broadcast -- --ignored --nocapture
    /// ```
    #[tokio::test]
    #[ignore = "needs the network and a live 1440p+ broadcast"]
    async fn end_to_end_against_a_live_broadcast() {
        let id = std::env::var("STREAMNOOK_YT_LIVE_ID").expect("STREAMNOOK_YT_LIVE_ID");
        let highs =
            crate::services::providers::youtube_media::high_renditions(&id, 1080).await;
        assert!(!highs.is_empty(), "no renditions above 1080p");
        let r = &highs[0];
        println!("rendition {} itag={} {}x{}", r.name, r.itag, r.width, r.height);

        let url = start("e2e", &id, 1080, r).await.expect("start relay");
        println!("relay url {}", url);
        let base = url.split("/stream.m3u8").next().unwrap().to_string();

        let get = |p: String| {
            let c = HTTP.clone();
            async move { c.get(&p).send().await.expect("get").bytes().await.expect("body").to_vec() }
        };

        let master = String::from_utf8(get(format!("{}/stream.m3u8", base)).await).unwrap();
        println!("--- master ---
{}", master);
        assert!(master.contains("RESOLUTION="));
        assert!(master.contains("AUDIO=\"aud\""));

        let vpl = String::from_utf8(get(format!("{}/video.m3u8", base)).await).unwrap();
        println!("--- video.m3u8 (first lines) ---");
        for line in vpl.lines().take(8) {
            println!("{}", line);
        }
        assert!(vpl.contains("#EXT-X-MAP:URI=\"init_v.mp4\""));

        // Newest listed segment: the one the player would fetch first.
        let seg_name = vpl
            .lines()
            .filter(|l| l.starts_with("v/"))
            .next_back()
            .expect("a segment line")
            .to_string();

        let init_v = get(format!("{}/init_v.mp4", base)).await;
        let seg_v = get(format!("{}/{}", base, seg_name)).await;
        let init_a = get(format!("{}/init_a.mp4", base)).await;
        println!(
            "init_v={}B seg({})={}B init_a={}B",
            init_v.len(),
            seg_name,
            seg_v.len(),
            init_a.len()
        );
        assert!(init_v.len() > 100 && seg_v.len() > 10_000 && init_a.len() > 100);
        assert_eq!(&seg_v[4..8], b"moof", "segment must start with a moof");

        if let Ok(dir) = std::env::var("STREAMNOOK_DASH_OUT") {
            std::fs::create_dir_all(&dir).unwrap();
            let mut joined = init_v.clone();
            joined.extend_from_slice(&seg_v);
            std::fs::write(format!("{}/video.mp4", dir), &joined).unwrap();
            println!("wrote {}/video.mp4 ({} B)", dir, joined.len());
        }
        // Optionally keep serving so a real player can be pointed at it. This
        // is the only way to exercise hls.js and the WebView's own decoder,
        // which is where the remaining risk lives.
        if let Ok(secs) = std::env::var("STREAMNOOK_DASH_HOLD") {
            let secs: u64 = secs.parse().unwrap_or(60);
            println!("HOLDING relay at {} for {}s", base, secs);
            tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
        }
        stop("e2e").await;
    }

    #[test]
    fn master_pairs_the_video_variant_with_an_audio_group() {
        let m = master(&a_session(5.0, 100));
        assert!(m.contains("RESOLUTION=2560x1440"));
        assert!(m.contains("CODECS=\"vp09.00.50.08,mp4a.40.2\""));
        // Without the audio group hls.js would play a silent video-only stream.
        assert!(m.contains("TYPE=AUDIO,GROUP-ID=\"aud\""));
        assert!(m.contains("AUDIO=\"aud\""));
    }

    #[test]
    fn media_playlist_holds_back_from_the_head() {
        // head is being written and head-1 has only just closed. Advertising
        // head-1 races the origin and 404s often enough to stall the player,
        // which is how this first failed in the app.
        let m = media_playlist(&a_session(5.0, 100), true).unwrap();
        assert!(m.contains("v/97.m4s"));
        assert!(!m.contains("v/98.m4s"), "the edge is further back than it looks");
        assert!(!m.contains("v/100.m4s"));
        assert!(m.contains("#EXT-X-MEDIA-SEQUENCE:92"));
        assert_eq!(m.matches("#EXTINF").count(), window_for(5.0) as usize);
        assert!(m.contains("#EXT-X-MAP:URI=\"init_v.mp4\""));
    }

    #[test]
    fn audio_playlist_points_at_its_own_init_and_directory() {
        let m = media_playlist(&a_session(5.0, 100), false).unwrap();
        assert!(m.contains("#EXT-X-MAP:URI=\"init_a.mp4\""));
        assert!(m.contains("a/97.m4s"));
        assert!(!m.contains("v/"));
    }

    /// The route parser is the ONLY guard on the id prefix, and a bug in it is
    /// invisible in the worst way: the master and both media playlists still
    /// serve (they are matched by literal name), while every segment 404s. The
    /// player attaches, reports the right resolution, and never renders a frame.
    #[test]
    fn route_of_splits_the_stream_id_off_every_shape() {
        assert_eq!(route_of("/s/cell-1-0/stream.m3u8"), Some(("cell-1-0", "stream.m3u8")));
        assert_eq!(route_of("/s/cell-1-0/video.m3u8"), Some(("cell-1-0", "video.m3u8")));
        assert_eq!(route_of("/s/cell-1-0/audio.m3u8"), Some(("cell-1-0", "audio.m3u8")));
        assert_eq!(route_of("/s/cell-1-0/init_v.mp4"), Some(("cell-1-0", "init_v.mp4")));
        // The segment tail must survive INTACT: the match arm below tests
        // `starts_with("v/")` and slices `p[2..]`.
        assert_eq!(route_of("/s/cell-1-0/v/97.m4s"), Some(("cell-1-0", "v/97.m4s")));
        assert_eq!(route_of("/s/solo/a/13305.m4s"), Some(("solo", "a/13305.m4s")));
    }

    #[test]
    fn route_of_rejects_anything_it_cannot_attribute() {
        // The old unprefixed shape: it named no stream, so it must not resolve
        // to an arbitrary one.
        assert_eq!(route_of("/stream.m3u8"), None);
        assert_eq!(route_of("/s/"), None);
        assert_eq!(route_of("/s/id"), None); // an id with no tail
        assert_eq!(route_of("/s//video.m3u8"), None); // empty id
        assert_eq!(route_of("/other/id/video.m3u8"), None);
        assert_eq!(route_of(""), None);
    }

    #[test]
    fn stream_ids_that_would_need_escaping_are_refused() {
        // Real ids from both callers.
        assert!(valid_id("solo"));
        assert!(valid_id("cell-1724700000000-3"));
        assert!(valid_id("a.b_c-1"));
        // A slash would split wrong in route_of; the rest would need escaping.
        assert!(!valid_id(""));
        assert!(!valid_id("a/b"));
        assert!(!valid_id("a?b"));
        assert!(!valid_id("a b"));
        assert!(!valid_id("caf\u{e9}"));
        assert!(!valid_id(&"x".repeat(65)));
    }

    fn a_stream(id: &str, session: Session) -> Arc<DashStream> {
        Arc::new(DashStream {
            id: id.to_string(),
            cpn: mint_nonce(),
            rn: AtomicU64::new(0),
            force_rotate: std::sync::atomic::AtomicBool::new(false),
            rotating: std::sync::atomic::AtomicBool::new(false),
            session: Mutex::new(session),
            segments: Mutex::new(HashMap::new()),
            inflight: Mutex::new(HashMap::new()),
            last_segment_warn: Mutex::new(None),
            last_cost_log: Mutex::new(None),
            last_seen: Mutex::new(Instant::now()),
        })
    }

    /// THE bug this whole refactor exists to remove. Sequence numbers restart per
    /// broadcast, so two live streams routinely hold the same `(video, sq)` key.
    /// Shared, one stream served the other's remuxed bytes against its own init
    /// segment, which describes a different resolution and codec config: the
    /// symptom is a decode error or garbage frames, so it gets triaged as a
    /// transmux bug rather than a keying one.
    #[test]
    fn two_streams_do_not_share_a_segment_cache() {
        let a = a_stream("a", a_session(5.0, 13305));
        let b = a_stream("b", a_session(5.0, 13305));
        a.store_segment(true, 13305, Bytes::from_static(b"AAAA"));
        b.store_segment(true, 13305, Bytes::from_static(b"BBBB"));
        assert_eq!(a.cached_segment(true, 13305).unwrap(), Bytes::from_static(b"AAAA"));
        assert_eq!(b.cached_segment(true, 13305).unwrap(), Bytes::from_static(b"BBBB"));
        // And clearing one leaves the other alone.
        a.clear_segment_cache();
        assert!(a.cached_segment(true, 13305).is_none());
        assert!(b.cached_segment(true, 13305).is_some());
    }

    /// The freeze this fixed. `prefetch_next` warms sq+1 and the player asks for
    /// sq+1 next, so "someone is already fetching this" was the COMMON case, not a
    /// rare race. The old code knew that and did nothing with it: `inflight` was a
    /// set of keys, which told a caller to expect a duplicate without giving it
    /// anything to wait on. Every segment was downloaded and remuxed twice, and a
    /// 1440p60 remux is multi-megabyte and CPU-bound.
    ///
    /// Measured before the fix: 774 slow serves in 34 minutes against a 2.0s
    /// segment budget, median 1310ms, max 2786ms.
    #[tokio::test]
    async fn a_request_joins_an_in_flight_fetch_instead_of_duplicating_it() {
        let st = a_stream("a", a_session(5.0, 100));

        // Hold the gate for (video, 97) the way an in-flight prefetch would.
        let gate = {
            let mut f = st.inflight.lock().unwrap();
            Arc::clone(
                f.entry((true, 97))
                    .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
            )
        };
        let held = gate.lock().await;

        // A foreground request for the SAME segment must not proceed while that
        // gate is held. If it does, it is duplicating the download and remux.
        let waiter = {
            let st = Arc::clone(&st);
            tokio::spawn(async move { st.fetch_once(true, 97).await.is_ok() })
        };
        tokio::task::yield_now().await;
        assert!(!waiter.is_finished(), "a second caller must WAIT, not race");

        // The owner finishes and publishes its bytes.
        st.store_segment(true, 97, Bytes::from_static(b"REAL"));
        drop(held);

        // The waiter now takes the cache hit rather than fetching. It must not
        // reach the network, which in this test would fail.
        let ok = tokio::time::timeout(Duration::from_secs(5), waiter)
            .await
            .expect("waiter must wake once the gate drops")
            .expect("waiter task panicked");
        assert!(ok, "the waiter should have been served from cache");
        assert_eq!(st.cached_segment(true, 97).unwrap(), Bytes::from_static(b"REAL"));
    }

    #[test]
    fn a_head_recorded_on_one_stream_does_not_move_another() {
        let a = a_stream("a", a_session(5.0, 100));
        let b = a_stream("b", a_session(5.0, 100));
        a.note_head(true, Some(500));
        assert_eq!(a.session.lock().unwrap().video_head, 500);
        assert_eq!(b.session.lock().unwrap().video_head, 100, "b must not follow a");
    }

    /// A 403 arms a forced rotation. As a process-global this was consumed by
    /// whichever stream rotated first, so the stream that was actually refused
    /// never force-rotated and died on the credential the origin had rejected.
    #[test]
    fn a_forced_rotation_belongs_to_one_stream() {
        let gated = || {
            let mut s = a_session(5.0, 100);
            s.video_url = "https://example/v?pot=abc".into();
            s
        };
        let a = a_stream("a", gated());
        let b = a_stream("b", gated());
        a.force_rotate.store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(a.rotation_due().is_some(), "a was refused, so a rotates");
        assert!(b.rotation_due().is_none(), "b's urls are fresh and unrefused");
    }

    /// Only gated urls perish on a clock, so an ungated one must not rotate just
    /// because another stream's did.
    #[test]
    fn an_ungated_url_never_rotates_on_age() {
        let st = a_stream("a", a_session(5.0, 100)); // plain https://example/v
        st.force_rotate.store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(st.rotation_due().is_none());
    }

    #[test]
    fn each_stream_gets_its_own_playback_nonce() {
        // The origin reads cpn as "one viewing session of one video". It must be
        // stable for a stream's whole life, and it must NOT be shared: this was
        // a process-global that start() cleared, so opening a second stream
        // rewrote the nonce the first was mid-playback on.
        let a = mint_nonce();
        let b = mint_nonce();
        assert_ne!(a, b, "two streams must not share a playback nonce");
        for n in [&a, &b] {
            assert_eq!(n.chars().count(), 16);
            assert!(n.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
        }
    }

    #[test]
    fn the_window_covers_a_fixed_span_not_a_fixed_count() {
        // A count-based window is a 30s buffer at 5s segments and a 4s buffer at
        // 0.66s ones. The short case is what made playback fragile, so the span
        // is what is held constant.
        assert_eq!(window_for(5.0), 6);
        assert_eq!(window_for(2.0), 15);
        assert!(window_for(0.66) >= 45);
        // Degenerate input must not produce an empty playlist.
        assert_eq!(window_for(0.0), WINDOW_MIN);
    }

    #[test]
    fn each_track_uses_its_own_head() {
        // The bug this guards: audio and video number segments independently
        // (measured 13305 vs 13306 on one broadcast). Sharing a head makes the
        // lagging track request a segment that does not exist, and it 404s
        // forever because the head only ever moves forward.
        let s = a_session_with_heads(5.0, 100, 140);
        let v = media_playlist(&s, true).unwrap();
        let a = media_playlist(&s, false).unwrap();
        assert!(v.contains("v/97.m4s"), "video must follow the video head");
        assert!(a.contains("a/137.m4s"), "audio must follow the audio head");
        // The previous assertion here was `!v.contains("138")`, which was
        // vacuous: with audio_head 140 and backoff 3 the audio window is
        // 132..=137, so "138" appears in neither playlist whatever happens.
        // These name the audio window's real numbers instead.
        assert!(!v.contains("a/137.m4s"), "video must not serve audio segments");
        assert!(
            !v.contains("#EXT-X-MEDIA-SEQUENCE:132"),
            "video must not inherit the audio head"
        );
    }

    #[test]
    fn playlist_is_refused_until_the_live_edge_is_known() {
        // Serving a window computed from head 0 would advertise sequence 0, which
        // is always outside the DVR window and 404s every fragment.
        assert!(media_playlist(&a_session(5.0, 0), true).is_err());
    }

    #[test]
    fn target_duration_rounds_up_so_it_never_understates_a_segment() {
        let m = media_playlist(&a_session(4.967, 50), true).unwrap();
        assert!(m.contains("#EXT-X-TARGETDURATION:5"));
        assert!(m.contains("#EXTINF:4.967,"));
    }

    #[test]
    fn window_clamps_at_the_start_of_a_stream() {
        let m = media_playlist(&a_session(5.0, 3), true).unwrap();
        assert!(m.contains("#EXT-X-MEDIA-SEQUENCE:0"));
        assert_eq!(m.matches("#EXTINF").count(), 1);
    }
}

#[cfg(test)]
mod rotation_tests {
    use super::*;

    #[test]
    fn only_proof_of_origin_urls_are_treated_as_perishable() {
        // visionos urls carry no token and last as long as their `expire` says,
        // so churning them would cost a player round trip for nothing.
        assert!(!is_gated("https://r1.googlevideo.com/videoplayback?itag=308&sq=5"));
        assert!(is_gated("https://r1.googlevideo.com/videoplayback?itag=308&pot=ABC"));
        assert!(is_gated("https://r1.googlevideo.com/videoplayback?pot=ABC&itag=308"));
        // `pot` has to be its own parameter, not a prefix of another one.
        assert!(!is_gated("https://r1.googlevideo.com/videoplayback?itag=308&potato=1"));
    }

    #[test]
    fn the_host_swaps_without_disturbing_anything_else() {
        let u = "https://rr1---sn-iv.googlevideo.com/videoplayback?itag=308&pot=A&sq=5";
        assert_eq!(
            with_host(u, "rr9---sn-iv.googlevideo.com").as_deref(),
            Some("https://rr9---sn-iv.googlevideo.com/videoplayback?itag=308&pot=A&sq=5")
        );
        assert_eq!(host_of(u), Some("rr1---sn-iv.googlevideo.com"));
        // A query string carrying slashes must not be mistaken for the path.
        let q = "https://a.example/videoplayback?u=x/y/z";
        assert_eq!(host_of(q), Some("a.example"));
        assert_eq!(
            with_host(q, "b.example").as_deref(),
            Some("https://b.example/videoplayback?u=x/y/z")
        );
    }

    #[test]
    fn a_url_without_a_path_is_left_alone_rather_than_mangled() {
        assert_eq!(with_host("https://a.example", "b.example"), None);
        assert_eq!(host_of("https://a.example"), None);
        assert_eq!(with_host("http://a.example/x", "b.example"), None);
    }

    #[test]
    fn the_window_stays_under_the_measured_wall() {
        // Gated urls were measured refusing at roughly thirty seconds; anything
        // at or above that would rotate only after playback had already broken.
        assert!(ROTATE_AFTER < Duration::from_secs(30));
        // The refresh is asynchronous, so there has to be real room for it to
        // finish while the current urls still work.
        assert!(
            Duration::from_secs(30) - ROTATE_AFTER >= Duration::from_secs(10),
            "not enough headroom for a background refresh to land"
        );
    }
}
