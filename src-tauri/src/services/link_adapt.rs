//! Live playback on a link that changes under it ("Auto" on every platform).
//!
//! The relay resolves one rendition and serves it for the whole session. On a
//! home connection that is right: the source rendition (8 to 9 Mbit/s at
//! 1080p60) arrives many times faster than real time. On a cellular, weak
//! Wi-Fi or busy home link it often cannot arrive in real time at all, and
//! nothing ever stepped down, so the player sat on a frozen frame for as long
//! as the link stayed bad. This module is the one place that decides, for each
//! live stream (the solo player and every MultiNook tile, on every platform):
//!
//! - **which rung of the ladder to play**, from measured throughput (each whole
//!   segment the player or the origin downloads is a sample) and from stalls
//!   the player reports and tails the origin loses. Down quickly, up slowly,
//!   and each up that fails doubles the wait before the next one;
//! - **how much extra distance from the live edge** to keep: one second per
//!   stall, given back half a second at a time while playback stays clean, so a
//!   good link returns to the tight edge and a bad one stops stalling. Only
//!   the phones' players follow it; the desktop players learn their own
//!   cushion per channel.
//!
//! Trouble with no fresh throughput to judge it by (the parts path, where the
//! player reads from the relay over loopback) is measured before it moves
//! anything: one whole published segment of the rung playing, timed. A link
//! with headroom had a hiccup, not a rate problem, and keeps its rung.
//!
//! Only "Auto" (`best`) sessions change rendition; a quality the viewer picked
//! by name stays put, and only the cushion moves. A link that could not hold a
//! rung is remembered for a while (the solo player only: a tile's share of the
//! link says nothing about the whole of it), so a restart, an audio-only
//! screen-off swap or the next channel starts at a rung the link can carry
//! instead of freezing on the source again.
//!
//! The decisions are pure (`Governor`, time passed in) and unit-tested; the
//! runtime half below applies them through the stream's `Route`: a seamless
//! switch of the upstream media playlist on the whole-segment path, or a relay
//! restart otherwise, announced to the player as `playback-adapt`.

use log::info;
use once_cell::sync::Lazy;
use serde::Serialize;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// One rung of the ladder: a video rendition.
#[derive(Debug, Clone, PartialEq)]
pub struct Rung {
    pub name: String,
    pub bandwidth: u64,
    pub url: String,
    /// The video codec family (`avc1`, `hvc1`, `av01`). A switch inside one
    /// family can be spliced into the running playlist; across families the
    /// player needs a fresh decoder, so the relay restarts instead.
    pub family: String,
}

/// EWMA half-lives in samples. hls.js uses 3 and 9 for live; the estimate is
/// the lower of the two, so a sudden drop counts at once and a sudden rise
/// counts only once it holds.
const FAST_HALF_LIFE: f64 = 3.0;
const SLOW_HALF_LIFE: f64 = 9.0;
/// A sample smaller than this is mostly time to first byte (a 160p segment, a
/// part) and says little about the link's rate.
const MIN_SAMPLE_BYTES: u64 = 64 * 1024;
/// A rung is sustainable when the link carries it this much faster than real
/// time. Below it every segment eats into the buffer.
const SUSTAIN_FACTOR: f64 = 1.2;
/// A step down picks the best rung whose bitrate fits in this share of the
/// estimate, leaving room for the link to wobble.
const PICK_FACTOR: f64 = 0.7;
/// A step up needs the estimate to carry the next rung at this share.
const UP_FACTOR: f64 = 0.7;
/// Samples on the current rung before its throughput is trusted for a step up.
const UP_MIN_SAMPLES: u32 = 4;
/// Clean playback before the first step up is tried.
const UP_HOLD_BASE: f64 = 30.0;
/// The longest the step-up wait grows to after repeated failed ups.
const UP_HOLD_MAX: f64 = 480.0;
/// A step down this soon after a step up marks the up as failed.
const UP_FAIL_WINDOW: f64 = 60.0;
/// After any switch, stalls and samples are ignored this long: the switch
/// itself can cost a short rebuffer, and that must not cascade.
const SETTLE: f64 = 8.0;
/// A single stall this long steps down on its own.
const LONG_STALL: f64 = 3.0;
/// Two stalls (or lost tails) within this window step down.
const STALL_WINDOW: f64 = 60.0;
/// With throughput this far above the rung's bitrate, a stall is jitter, not
/// a rate problem: the cushion grows and the rung stays.
const JITTER_HEADROOM: f64 = 2.0;
/// Cushion added per stall and per lost tail, and its ceiling.
const CUSHION_PER_STALL: f64 = 1.0;
const CUSHION_PER_TAIL: f64 = 0.5;
const CUSHION_MAX: f64 = 4.0;
/// Clean playback that gives back one decay step of cushion.
const CUSHION_DECAY_AFTER: f64 = 45.0;
const CUSHION_DECAY_STEP: f64 = 0.5;
/// How long a link that could not hold a rung is remembered across sessions.
const REMEMBER_FOR: Duration = Duration::from_secs(600);
/// Throughput older than this no longer describes the link: trouble is
/// measured afresh instead of judged by it, and the next sample starts a new
/// estimate.
const FRESH_FOR: f64 = 30.0;
/// At most one measurement of the playing rung in this long.
const MEASURE_EVERY: f64 = 15.0;

/// What the governor wants done.
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// Play rung `to` (an index into the ladder, 0 = best).
    Switch { to: usize, why: &'static str },
    /// Measure the link on rung `rung` before stepping up to it (the parts
    /// path streams in real time, so its own transfers never show headroom).
    Probe { rung: usize },
    /// Time one whole segment of the rung playing now before acting on
    /// `why`: with no fresh throughput, a stall cannot tell a link too slow
    /// for the rung from a hiccup on a fast one.
    Measure { why: &'static str },
}

/// The pure decision core. Times are seconds on any monotonic clock.
#[derive(Debug, Clone)]
pub struct Governor {
    ladder: Vec<Rung>,
    current: usize,
    /// False when the viewer picked a rendition by name: only the cushion moves.
    auto: bool,
    fast: Option<f64>,
    slow: Option<f64>,
    /// When the newest throughput sample landed.
    last_sample: Option<f64>,
    last_measure: Option<f64>,
    /// Samples taken since the last switch.
    samples: u32,
    /// Stalls and lost tails, by time, inside `STALL_WINDOW`.
    troubles: Vec<f64>,
    last_switch: f64,
    last_up: Option<f64>,
    up_hold: f64,
    last_trouble: f64,
    last_probe: Option<f64>,
    cushion: f64,
    last_decay: f64,
    /// The best rung bitrate this session proved the link could not hold.
    ceiling_bw: Option<u64>,
}

impl Governor {
    pub fn new(ladder: Vec<Rung>, current: usize, auto: bool, now: f64) -> Self {
        let current = current.min(ladder.len().saturating_sub(1));
        Self {
            ladder,
            current,
            auto,
            fast: None,
            slow: None,
            last_sample: None,
            last_measure: None,
            samples: 0,
            troubles: Vec::new(),
            last_switch: now,
            last_up: None,
            up_hold: UP_HOLD_BASE,
            last_trouble: now,
            last_probe: None,
            cushion: 0.0,
            last_decay: now,
            ceiling_bw: None,
        }
    }

    pub fn current(&self) -> usize {
        self.current
    }

    pub fn ladder(&self) -> &[Rung] {
        &self.ladder
    }

    /// Extra seconds behind the live edge the player should keep.
    pub fn cushion(&self) -> f64 {
        self.cushion
    }

    /// The throughput estimate in bits per second, once there is one.
    pub fn estimate(&self) -> Option<f64> {
        match (self.fast, self.slow) {
            (Some(f), Some(s)) => Some(f.min(s)),
            _ => None,
        }
    }

    /// The highest rung bitrate this session found the link cannot hold.
    pub fn ceiling(&self) -> Option<u64> {
        self.ceiling_bw
    }

    pub fn auto(&self) -> bool {
        self.auto
    }

    fn settling(&self, now: f64) -> bool {
        now - self.last_switch < SETTLE
    }

    /// One whole transfer: `bytes` in `secs`.
    pub fn note_throughput(&mut self, bytes: u64, secs: f64, now: f64) -> Option<Decision> {
        if !self.add_sample(bytes, secs, now) {
            return None;
        }
        self.decide(now)
    }

    /// Folds one transfer into the estimate; false when it is too small to
    /// say anything about the link's rate.
    fn add_sample(&mut self, bytes: u64, secs: f64, now: f64) -> bool {
        if bytes < MIN_SAMPLE_BYTES || secs <= 0.0 {
            return false;
        }
        // Samples from minutes ago measured a different link (the parts path
        // samples rarely): start over rather than average with them.
        if self.last_sample.is_none_or(|t| now - t > FRESH_FOR) {
            self.fast = None;
            self.slow = None;
        }
        let bps = bytes as f64 * 8.0 / secs.max(0.001);
        let ewma = |prev: Option<f64>, half: f64| {
            let alpha = 1.0 - 0.5f64.powf(1.0 / half);
            Some(prev.map_or(bps, |p| p + (bps - p) * alpha))
        };
        self.fast = ewma(self.fast, FAST_HALF_LIFE);
        self.slow = ewma(self.slow, SLOW_HALF_LIFE);
        self.last_sample = Some(now);
        self.samples += 1;
        true
    }

    /// The estimate, while its newest sample still describes the link.
    fn fresh_estimate(&self, now: f64) -> Option<f64> {
        self.last_sample
            .filter(|t| now - t <= FRESH_FOR)
            .and(self.estimate())
    }

    /// A stall the player reported: `secs` long so far (`ended` false while it
    /// is still frozen).
    pub fn note_stall(&mut self, secs: f64, ended: bool, now: f64) -> Option<Decision> {
        if self.settling(now) {
            return None;
        }
        if ended && secs >= 0.5 {
            self.add_trouble(now, CUSHION_PER_STALL);
        }
        if !self.auto {
            return None;
        }
        if secs >= LONG_STALL {
            return self.on_trouble("a long stall", now);
        }
        if self.recent_troubles(now) >= 2 {
            return self.on_trouble("repeated stalls", now);
        }
        None
    }

    /// The origin gave up on a segment's tail (slower than real time, or a
    /// dead transfer): the player will meet a hole.
    pub fn note_tail_loss(&mut self, now: f64) -> Option<Decision> {
        if self.settling(now) {
            return None;
        }
        self.add_trouble(now, CUSHION_PER_TAIL);
        if self.auto && self.recent_troubles(now) >= 2 {
            return self.on_trouble("lost segment tails", now);
        }
        None
    }

    /// Trouble that would step down, judged by the link: a fresh estimate
    /// with headroom over the rung means jitter (the cushion handles it), one
    /// without means the rate, and no fresh estimate means measure first.
    fn on_trouble(&mut self, why: &'static str, now: f64) -> Option<Decision> {
        if self.current + 1 >= self.ladder.len() {
            return None;
        }
        match self.fresh_estimate(now) {
            Some(est) if est >= self.bw(self.current) as f64 * JITTER_HEADROOM => None,
            Some(_) => self.step_down(why),
            None if self.last_measure.is_none_or(|t| now - t >= MEASURE_EVERY) => {
                self.last_measure = Some(now);
                Some(Decision::Measure { why })
            }
            // Measured moments ago and still nothing fresh: the measurement
            // itself failed, which is the answer.
            None => self.step_down(why),
        }
    }

    /// The measurement a `Measure` asked for: one whole segment of the
    /// playing rung, `bytes` in `secs`, or `None` when it could not be fetched
    /// at all (a link that cannot deliver one segment in time).
    pub fn note_measured(&mut self, sample: Option<(u64, f64)>, why: &'static str, now: f64) -> Option<Decision> {
        self.last_measure = Some(now);
        if self.settling(now) || !self.auto {
            return None;
        }
        if let Some((bytes, secs)) = sample {
            self.add_sample(bytes, secs, now);
        }
        match self.fresh_estimate(now) {
            Some(est) if est >= self.bw(self.current) as f64 * JITTER_HEADROOM => None,
            _ => self.step_down(why),
        }
    }

    /// Before the player has loaded anything: when the relay's own first
    /// downloads (the parts origin's backfill) already show the link cannot
    /// carry this rung, the rung to start on instead. Moving then costs no
    /// reload; the same switch a moment after the first frame is a restart
    /// and a frozen picture. Settling does not apply: nothing has played.
    pub fn start_check(&mut self, now: f64) -> Option<usize> {
        if !self.auto || self.samples < 2 {
            return None;
        }
        let est = self.fresh_estimate(now)?;
        if est >= self.bw(self.current) as f64 * SUSTAIN_FACTOR {
            return None;
        }
        match self.step_down("throughput below the bitrate")? {
            Decision::Switch { to, .. } => Some(to),
            _ => None,
        }
    }

    /// A periodic look with no new evidence: decays the cushion and considers a
    /// step up.
    pub fn tick(&mut self, now: f64) -> Option<Decision> {
        self.decay(now);
        self.decide(now)
    }

    /// The relay switched to rung `to` (a decision applied, or a probe's step).
    pub fn switched(&mut self, to: usize, now: f64) {
        let up = to < self.current;
        if !up {
            if let Some(t) = self.last_up {
                if now - t < UP_FAIL_WINDOW {
                    self.up_hold = (self.up_hold * 2.0).min(UP_HOLD_MAX);
                }
            }
            self.ceiling_bw = Some(
                self.ceiling_bw
                    .map_or(self.bw(self.current), |c| c.min(self.bw(self.current))),
            );
        } else {
            self.last_up = Some(now);
        }
        self.current = to.min(self.ladder.len().saturating_sub(1));
        self.last_switch = now;
        self.samples = 0;
        // The estimate carries over: it measured the link, not the rung.
        self.troubles.clear();
        self.last_trouble = now;
    }

    /// The link carried a probe of rung `rung`: `bytes` in `secs`.
    pub fn note_probe(&mut self, rung: usize, bytes: u64, secs: f64, now: f64) -> Option<Decision> {
        self.last_probe = Some(now);
        if secs <= 0.0 || rung >= self.ladder.len() {
            return None;
        }
        let bps = bytes as f64 * 8.0 / secs;
        if rung < self.current && bps * UP_FACTOR >= self.bw(rung) as f64 && self.up_ready(now) {
            return Some(Decision::Switch { to: rung, why: "the link carries more" });
        }
        // A probe costs a whole segment of the richer rung: on a link that
        // cannot carry it, ask less and less often.
        self.up_hold = (self.up_hold * 2.0).min(UP_HOLD_MAX);
        None
    }

    fn bw(&self, i: usize) -> u64 {
        self.ladder.get(i).map_or(0, |r| r.bandwidth)
    }

    fn add_trouble(&mut self, now: f64, cushion: f64) {
        self.troubles.push(now);
        self.last_trouble = now;
        self.last_decay = now;
        self.cushion = (self.cushion + cushion).min(CUSHION_MAX);
    }

    fn recent_troubles(&mut self, now: f64) -> usize {
        self.troubles.retain(|t| now - t <= STALL_WINDOW);
        self.troubles.len()
    }

    fn decay(&mut self, now: f64) {
        while self.cushion > 0.0
            && now - self.last_trouble >= CUSHION_DECAY_AFTER
            && now - self.last_decay >= CUSHION_DECAY_AFTER
        {
            self.cushion = (self.cushion - CUSHION_DECAY_STEP).max(0.0);
            self.last_decay += CUSHION_DECAY_AFTER;
        }
    }

    fn up_ready(&self, now: f64) -> bool {
        self.auto
            && self.current > 0
            && !self.settling(now)
            && now - self.last_trouble >= self.up_hold
            && now - self.last_switch >= self.up_hold.min(UP_HOLD_BASE)
    }

    /// One rung down, or straight to what the estimate fits when that is lower.
    fn step_down(&mut self, why: &'static str) -> Option<Decision> {
        let last = self.ladder.len().saturating_sub(1);
        if self.current >= last {
            return None;
        }
        let mut to = self.current + 1;
        if let Some(est) = self.estimate() {
            let fit = (to..=last)
                .find(|&i| self.bw(i) as f64 <= est * PICK_FACTOR)
                .unwrap_or(last);
            to = to.max(fit);
        }
        Some(Decision::Switch { to, why })
    }

    fn decide(&mut self, now: f64) -> Option<Decision> {
        if !self.auto || self.ladder.len() < 2 || self.settling(now) {
            return None;
        }
        if let Some(est) = self.fresh_estimate(now) {
            let cur = self.bw(self.current) as f64;
            if self.samples >= 2 && est < cur * SUSTAIN_FACTOR {
                return self.step_down("throughput below the bitrate");
            }
            if self.up_ready(now) && self.samples >= UP_MIN_SAMPLES {
                let next = self.current - 1;
                if est * UP_FACTOR >= self.bw(next) as f64 {
                    return Some(Decision::Switch { to: next, why: "the link carries more" });
                }
                return None;
            }
        }
        // No transfers of our own to judge by (the parts path): measure the
        // next rung once the clean-playback hold has passed.
        if self.up_ready(now)
            && self.samples < UP_MIN_SAMPLES
            && self.last_probe.is_none_or(|t| now - t >= self.up_hold)
        {
            self.last_probe = Some(now);
            return Some(Decision::Probe { rung: self.current - 1 });
        }
        None
    }
}

/// The video codec family of a rendition's `CODECS` (`avc1`, `hvc1`, `av01`).
pub fn codec_family(codecs: Option<&str>) -> String {
    codecs
        .and_then(|c| c.split(',').map(str::trim).find(|c| !c.starts_with("mp4a")))
        .map(|c| c.split('.').next().unwrap_or(c).to_ascii_lowercase())
        .unwrap_or_default()
}

/// The ladder a session adapts over: the selected rendition, then every video
/// rendition below its bitrate, best first. The selected one is the top, so
/// "Auto" never climbs above what the screen cap chose.
pub fn ladder_from(
    variants: &[crate::services::twitch_resolver::Variant],
    selected_url: &str,
) -> (Vec<Rung>, usize) {
    let Some(sel) = variants.iter().find(|v| v.url == selected_url) else {
        return (Vec::new(), 0);
    };
    let sel_bw = sel.bandwidth.unwrap_or(u64::MAX);
    let mut rungs: Vec<Rung> = variants
        .iter()
        .filter(|v| v.height.is_some() && v.bandwidth.is_some())
        .filter(|v| v.url == selected_url || v.bandwidth.unwrap_or(0) < sel_bw)
        .map(|v| Rung {
            name: v.name.clone(),
            bandwidth: v.bandwidth.unwrap_or(0),
            url: v.url.clone(),
            family: codec_family(v.codecs.as_deref()),
        })
        .collect();
    rungs.sort_by(|a, b| b.bandwidth.cmp(&a.bandwidth));
    // One rung per name. Where two renditions share one (a tier offered in
    // two codecs), the resolver's pick keeps it, so starting a session never
    // trades the codec it chose.
    let mut out: Vec<Rung> = Vec::with_capacity(rungs.len());
    for r in rungs {
        match out.iter_mut().find(|p| p.name == r.name) {
            Some(kept) if r.url == selected_url => *kept = r,
            Some(_) => {}
            None => out.push(r),
        }
    }
    let current = out.iter().position(|r| r.url == selected_url).unwrap_or(0);
    (out, current)
}

/// The ladder for a platform adapter's quality list (best first), with the
/// pick known by name. A rendition the platform lists without a bitrate (or
/// as 0) gets one estimated from its pixels against the tallest one that has
/// it, so YouTube's 1440p and 2160p tiers can be climbed down from.
pub fn ladder_from_qualities(
    qualities: &[crate::services::providers::source::PlaybackQuality],
    selected_name: &str,
) -> (Vec<Rung>, usize) {
    let known = qualities
        .iter()
        .filter(|q| q.bandwidth.is_some_and(|b| b > 0))
        .filter_map(|q| Some((q.height?, q.width, q.fps, q.bandwidth?)))
        .max_by_key(|(h, ..)| *h);
    let pixels = |h: u32, w: Option<u32>| w.unwrap_or(h * 16 / 9) as f64 * h as f64;
    let variants: Vec<crate::services::twitch_resolver::Variant> = qualities
        .iter()
        .filter(|q| q.height.is_some())
        .filter_map(|q| {
            let h = q.height?;
            let bandwidth = match q.bandwidth.filter(|b| *b > 0) {
                Some(b) => b,
                None => {
                    let (kh, kw, kfps, kb) = known?;
                    let area = pixels(h, q.width) / pixels(kh, kw);
                    let rate = (q.fps.unwrap_or(30.0) / kfps.unwrap_or(30.0)).max(0.5);
                    (kb as f64 * area.powf(0.75) * rate.sqrt()) as u64
                }
            };
            Some(crate::services::twitch_resolver::Variant {
                name: q.name.clone(),
                group_id: q.name.clone(),
                width: q.width,
                height: Some(h),
                fps: q.fps,
                codecs: None,
                bandwidth: Some(bandwidth),
                // Rungs are told apart by name: an adapter switches by name,
                // and a tier it serves through its own relay has no URL.
                url: format!("{}#{}", q.url, q.name),
            })
        })
        .collect();
    let Some(sel) = variants.iter().find(|v| v.name.eq_ignore_ascii_case(selected_name)) else {
        return (Vec::new(), 0);
    };
    let (mut ladder, current) = ladder_from(&variants, &sel.url.clone());
    for r in &mut ladder {
        if let Some((url, _)) = r.url.rsplit_once('#') {
            r.url = url.to_string();
        }
    }
    (ladder, current)
}

/// The rung a new session should start on: the selected one, unless a link
/// remembered from moments ago could not hold it.
pub fn start_rung(ladder: &[Rung], selected: usize, remembered_bw: Option<u64>) -> usize {
    let Some(limit) = remembered_bw else {
        return selected;
    };
    (selected..ladder.len())
        .find(|&i| ladder[i].bandwidth < limit)
        .unwrap_or(ladder.len().saturating_sub(1))
}

// ── Runtime ────────────────────────────────────────────────────────────────

/// The solo player's stream id; MultiNook tiles use their cell ids.
pub const SOLO: &str = crate::services::stream_server::SOLO_STREAM_ID;

/// Lets the shaped-network harness (`link_soak`) run with the governor off,
/// for the "before" numbers.
static OFF: AtomicBool = AtomicBool::new(false);
/// Lets the harness run the phones' rules on a desktop test build.
static PHONE_RULES: AtomicBool = AtomicBool::new(false);

#[cfg(test)]
pub fn force_enabled(on: bool) {
    OFF.store(!on, Ordering::Relaxed);
}

#[cfg(test)]
pub fn force_phone_rules(on: bool) {
    PHONE_RULES.store(on, Ordering::Relaxed);
}

/// Whether live streams adapt (every platform; off only in the harness).
pub fn enabled() -> bool {
    !OFF.load(Ordering::Relaxed)
}

/// The phones' players follow the cushion, and their origin waits longer for
/// a late tail. The desktop players learn their own cushion per channel and
/// keep the origin's home deadlines.
pub fn phone_rules() -> bool {
    cfg!(mobile) || PHONE_RULES.load(Ordering::Relaxed)
}

/// What applying a switch returns: the URL the player reloads on, or None
/// when the switch was spliced into the playlist it is already reading.
pub type SwitchFuture = Pin<Box<dyn Future<Output = anyhow::Result<Option<String>>> + Send>>;

/// How one stream is served. The relay knows how to move it to another rung
/// (splice or restart) and whether its player's own downloads cross the
/// internet; this module only decides when and to what.
pub trait Route: Send + Sync {
    /// Serve rung `to` from now on. `same_decoder`: it shares the playing
    /// rung's codec family.
    fn switch(&self, to: Rung, same_decoder: bool) -> SwitchFuture;

    /// Whether the player's whole-segment downloads time the real link. Not
    /// when a local relay hands them over from memory or prefetches them
    /// (its own fetches are reported instead).
    fn player_measures_link(&self) -> bool;
}

struct Session {
    gov: Governor,
    epoch: Instant,
    /// Unique per `begin`, so a switch applied after the session changed is
    /// dropped.
    generation: u64,
    applying: bool,
    last_cushion_sent: f64,
    route: Arc<dyn Route>,
}

static SESSIONS: Lazy<Mutex<HashMap<String, Session>>> = Lazy::new(|| Mutex::new(HashMap::new()));
static GENERATION: AtomicU64 = AtomicU64::new(0);
/// The best rung bitrate the link could not hold, and when that was learned.
static REMEMBERED: Lazy<Mutex<Option<(Instant, u64)>>> = Lazy::new(|| Mutex::new(None));

fn sessions() -> std::sync::MutexGuard<'static, HashMap<String, Session>> {
    SESSIONS.lock().unwrap_or_else(|e| e.into_inner())
}

/// What the players hear about: the stream it concerns, the rung now
/// playing, the extra cushion, and the URL to reload on after a restart.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdaptEvent {
    pub stream_id: String,
    pub quality: String,
    pub auto: bool,
    pub extra_cushion: f64,
    pub url: Option<String>,
    pub reason: String,
}

fn remembered() -> Option<u64> {
    let mut slot = REMEMBERED.lock().unwrap_or_else(|e| e.into_inner());
    match *slot {
        Some((at, bw)) if at.elapsed() < REMEMBER_FOR => Some(bw),
        Some(_) => {
            *slot = None;
            None
        }
        None => None,
    }
}

fn remember(bw: Option<u64>) {
    if let Some(bw) = bw {
        *REMEMBERED.lock().unwrap_or_else(|e| e.into_inner()) = Some((Instant::now(), bw));
    }
}

/// A live session is starting for `stream_id` on `variants` with
/// `selected_url` chosen. Returns the rung to actually serve: the selected
/// one, or for the solo player in "Auto" a lower one when the link remembered
/// from moments ago could not hold it. None when there is no ladder to adapt
/// over; the caller then serves its own pick and nothing adapts.
pub fn begin(
    stream_id: &str,
    variants: &[crate::services::twitch_resolver::Variant],
    selected_url: &str,
    auto: bool,
    route: Arc<dyn Route>,
) -> Option<Rung> {
    let (ladder, selected) = ladder_from(variants, selected_url);
    begin_ladder(stream_id, ladder, selected, auto, route)
}

/// `begin` for a platform adapter's quality list, where the pick is known by
/// name (YouTube's 1440p and up have no playlist URL of their own).
pub fn begin_named(
    stream_id: &str,
    qualities: &[crate::services::providers::source::PlaybackQuality],
    selected_name: &str,
    auto: bool,
    route: Arc<dyn Route>,
) -> Option<Rung> {
    let (ladder, selected) = ladder_from_qualities(qualities, selected_name);
    begin_ladder(stream_id, ladder, selected, auto, route)
}

fn begin_ladder(
    stream_id: &str,
    ladder: Vec<Rung>,
    selected: usize,
    auto: bool,
    route: Arc<dyn Route>,
) -> Option<Rung> {
    end(stream_id);
    if !enabled() {
        return None;
    }
    if ladder.is_empty() {
        return None;
    }
    let start = if auto && stream_id == SOLO {
        start_rung(&ladder, selected, remembered())
    } else {
        selected
    };
    let rung = ladder[start].clone();
    if start != selected {
        info!(
            "[LinkAdapt] starting on {} instead of {}: the link held no more moments ago",
            ladder[start].name, ladder[selected].name
        );
    }
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    sessions().insert(
        stream_id.to_string(),
        Session {
            gov: Governor::new(ladder, start, auto, 0.0),
            epoch: Instant::now(),
            generation,
            applying: false,
            last_cushion_sent: 0.0,
            route,
        },
    );
    tauri::async_runtime::spawn(ticker(stream_id.to_string(), generation));
    // The players show the rung "Auto" plays; Rust says what it is from the
    // start, so a label never outlives its stream.
    announce(stream_id, None, "start");
    Some(rung)
}

/// Looks every few seconds while the session lives: the parts path reports
/// nothing during clean playback, and that is exactly when the cushion should
/// shrink and a step up be tried.
async fn ticker(stream_id: String, generation: u64) {
    loop {
        tokio::time::sleep(Duration::from_secs(5)).await;
        let live = sessions()
            .get(&stream_id)
            .is_some_and(|s| s.generation == generation);
        if !live {
            return;
        }
        tick(&stream_id);
    }
}

/// The stream ended (stop, a VOD, another channel, a tile closed, or an
/// upstream this module has no ladder for). What the solo player's link
/// learned stays remembered for the next start.
pub fn end(stream_id: &str) {
    let old = sessions().remove(stream_id);
    if let Some(s) = old {
        if stream_id == SOLO {
            remember(s.gov.ceiling());
        }
        // Nothing adapts this stream any more (an empty quality says so).
        crate::services::stream_server::emit_playback_adapt(&AdaptEvent {
            stream_id: stream_id.to_string(),
            quality: String::new(),
            auto: false,
            extra_cushion: 0.0,
            url: None,
            reason: "end".to_string(),
        });
    }
}

/// The session `begin` just made for `stream_id`, to hand to `start_check`.
pub fn generation(stream_id: &str) -> Option<u64> {
    sessions().get(stream_id).map(|s| s.generation)
}

/// Right after the relay came up and before the player loads: the rung to
/// restart the relay on when its own first downloads say the link cannot carry
/// the one it started with (`Governor::start_check`). Already recorded as the
/// rung playing; None keeps the start. Only for the session this start began
/// (`generation`): a start that a newer one overtook must not move the newer
/// stream.
pub fn start_check(stream_id: &str, generation: Option<u64>) -> Option<Rung> {
    let generation = generation?;
    let rung = {
        let mut guard = sessions();
        let s = guard
            .get_mut(stream_id)
            .filter(|s| s.generation == generation && !s.applying)?;
        let now = s.epoch.elapsed().as_secs_f64();
        let est = s.gov.estimate().unwrap_or(0.0);
        let to = s.gov.start_check(now)?;
        let from = s.gov.ladder().get(s.gov.current())?.name.clone();
        s.gov.switched(to, now);
        if stream_id == SOLO {
            remember(s.gov.ceiling());
        }
        let rung = s.gov.ladder().get(to)?.clone();
        info!(
            "[LinkAdapt] {}: starting on {} instead of {}: the first downloads measured {:.2} Mbit/s",
            stream_id,
            rung.name,
            from,
            est / 1e6
        );
        rung
    };
    announce(stream_id, None, "start");
    Some(rung)
}

/// Every MultiNook tile's session ends (the grid closed or was promoted).
pub fn end_tiles() {
    sessions().retain(|id, _| id == SOLO);
}

/// A tile became the solo player without resolving again: its session, rung
/// and what it learned about the link carry over under `to`, now applied
/// through `route`. Returns false when the tile was not adapting.
pub fn handover(from: &str, to: &str, route: Arc<dyn Route>) -> bool {
    let Some(mut s) = sessions().remove(from) else {
        return false;
    };
    end(to);
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    s.generation = generation;
    s.applying = false;
    s.route = route;
    sessions().insert(to.to_string(), s);
    tauri::async_runtime::spawn(ticker(to.to_string(), generation));
    announce(to, None, "handover");
    true
}

/// The rung playing now and the extra cushion, for logs and the harness.
pub fn snapshot(stream_id: &str) -> Option<(String, f64)> {
    let guard = sessions();
    let s = guard.get(stream_id)?;
    Some((s.gov.ladder()[s.gov.current()].name.clone(), s.gov.cushion()))
}

/// The codec family of the rung playing now (`hvc1`, `av01`, `avc1`), so a
/// player that cannot decode it can say which one failed.
pub fn playing_family(stream_id: &str) -> Option<String> {
    let guard = sessions();
    let s = guard.get(stream_id)?;
    Some(s.gov.ladder().get(s.gov.current())?.family.clone())
}

/// The name of the rung playing now, while the stream adapts.
pub fn current_quality(stream_id: &str) -> Option<String> {
    snapshot(stream_id).map(|(name, _)| name)
}

/// Extra seconds behind the live edge to keep: 0 with no session, and on the
/// desktop, whose players keep their own learned cushion.
pub fn extra_cushion(stream_id: &str) -> f64 {
    if !phone_rules() {
        return 0.0;
    }
    sessions().get(stream_id).map_or(0.0, |s| s.gov.cushion())
}

fn with_session(stream_id: &str, f: impl FnOnce(&mut Governor, f64) -> Option<Decision>) {
    let (decision, generation, cushion_moved) = {
        let mut guard = sessions();
        let Some(s) = guard.get_mut(stream_id) else { return };
        let now = s.epoch.elapsed().as_secs_f64();
        let d = if s.applying { None } else { f(&mut s.gov, now) };
        if d.is_some() {
            s.applying = true;
        }
        let moved = phone_rules() && (s.gov.cushion() - s.last_cushion_sent).abs() >= 0.5;
        if moved {
            s.last_cushion_sent = s.gov.cushion();
        }
        (d, s.generation, moved)
    };
    match decision {
        Some(d) => {
            tauri::async_runtime::spawn(apply(stream_id.to_string(), d, generation));
        }
        None if cushion_moved => announce(stream_id, None, "cushion"),
        None => {}
    }
}

/// A whole segment a relay downloaded from the internet itself (the parts
/// origin's backfill and catch-up, a provider relay's segment fetches).
pub fn note_throughput(stream_id: &str, bytes: u64, secs: f64) {
    with_session(stream_id, |g, now| g.note_throughput(bytes, secs, now));
}

/// A whole segment the player downloaded. Counted only where that download
/// crossed the internet; behind a relay that serves from memory it timed
/// loopback, and the relay reports its own fetches instead.
pub fn note_player_segment(stream_id: &str, bytes: u64, secs: f64) {
    // The route is asked with the sessions lock released: it may look at the
    // origin, whose own lock is sometimes held while it reads the cushion.
    let route = sessions().get(stream_id).map(|s| s.route.clone());
    if route.is_some_and(|r| r.player_measures_link()) {
        note_throughput(stream_id, bytes, secs);
    }
}

/// A stall the player reported.
pub fn note_stall(stream_id: &str, secs: f64, ended: bool) {
    with_session(stream_id, |g, now| g.note_stall(secs, ended, now));
}

/// The parts origin gave up on a segment's tail.
pub fn note_tail_loss(stream_id: &str) {
    with_session(stream_id, |g, now| g.note_tail_loss(now));
}

/// Periodic look (the player's reports drive it; this catches clean stretches
/// with nothing to report).
pub fn tick(stream_id: &str) {
    with_session(stream_id, |g, now| g.tick(now));
}

fn announce(stream_id: &str, url: Option<String>, reason: &str) {
    let event = {
        let guard = sessions();
        let Some(s) = guard.get(stream_id) else { return };
        let rung = &s.gov.ladder()[s.gov.current()];
        AdaptEvent {
            stream_id: stream_id.to_string(),
            quality: rung.name.clone(),
            auto: s.gov.auto(),
            extra_cushion: if phone_rules() { s.gov.cushion() } else { 0.0 },
            url,
            reason: reason.to_string(),
        }
    };
    crate::services::stream_server::emit_playback_adapt(&event);
}

async fn apply(stream_id: String, decision: Decision, generation: u64) {
    match decision {
        Decision::Switch { to, why } => switch_to(&stream_id, to, why, generation).await,
        Decision::Probe { rung } => probe(&stream_id, rung, generation).await,
        Decision::Measure { why } => measure(&stream_id, why, generation).await,
    }
    if let Some(s) = sessions().get_mut(&stream_id) {
        if s.generation == generation {
            s.applying = false;
        }
    }
}

/// Rung `i`, the rung playing now and the stream's route, while `generation`
/// is still the session.
fn rung_of(stream_id: &str, generation: u64, i: usize) -> Option<(Rung, Rung, Arc<dyn Route>)> {
    let guard = sessions();
    let s = guard.get(stream_id).filter(|s| s.generation == generation)?;
    let from = s.gov.ladder().get(s.gov.current())?.clone();
    Some((s.gov.ladder().get(i)?.clone(), from, s.route.clone()))
}

async fn switch_to(stream_id: &str, to: usize, why: &'static str, generation: u64) {
    let Some((rung, from, route)) = rung_of(stream_id, generation, to) else { return };
    info!("[LinkAdapt] {}: {} -> {} ({})", stream_id, from.name, rung.name, why);
    let same_decoder = rung.family == from.family;
    let name = rung.name.clone();
    let reload = match route.switch(rung, same_decoder).await {
        Ok(reload) => reload,
        Err(e) => {
            log::warn!("[LinkAdapt] {}: switch to {} failed: {}", stream_id, name, e);
            return;
        }
    };
    {
        let mut guard = sessions();
        let Some(s) = guard.get_mut(stream_id).filter(|s| s.generation == generation) else {
            return;
        };
        let now = s.epoch.elapsed().as_secs_f64();
        s.gov.switched(to, now);
        if stream_id == SOLO && (to > 0 || s.gov.ceiling().is_some()) {
            remember(s.gov.ceiling());
        }
    }
    announce(stream_id, reload, why);
}

/// Times one whole, already-published segment of rung `rung`'s playlist.
async fn probe(stream_id: &str, rung: usize, generation: u64) {
    let Some((target, _, _)) = rung_of(stream_id, generation, rung) else { return };
    let measured = measure_segment(&target.url).await;
    let decision = {
        let mut guard = sessions();
        let Some(s) = guard.get_mut(stream_id).filter(|s| s.generation == generation) else {
            return;
        };
        let now = s.epoch.elapsed().as_secs_f64();
        match measured {
            Some((bytes, secs)) => {
                info!(
                    "[LinkAdapt] {}: probe {}: {:.2} Mbit/s",
                    stream_id,
                    target.name,
                    bytes as f64 * 8.0 / secs / 1e6
                );
                s.gov.note_probe(rung, bytes, secs, now)
            }
            None => s.gov.note_probe(rung, 0, 0.0, now),
        }
    };
    if let Some(Decision::Switch { to, why }) = decision {
        switch_to(stream_id, to, why, generation).await;
    }
}

/// Times one whole segment of the rung playing now, then lets the governor
/// judge the trouble that asked for it.
async fn measure(stream_id: &str, why: &'static str, generation: u64) {
    let current = {
        let guard = sessions();
        guard
            .get(stream_id)
            .filter(|s| s.generation == generation)
            .and_then(|s| s.gov.ladder().get(s.gov.current()).cloned())
    };
    let Some(rung) = current else { return };
    let measured = measure_segment(&rung.url).await;
    if let Some((bytes, secs)) = measured {
        info!(
            "[LinkAdapt] {}: measured {} after {}: {:.2} Mbit/s",
            stream_id,
            rung.name,
            why,
            bytes as f64 * 8.0 / secs / 1e6
        );
    } else {
        info!("[LinkAdapt] {}: {} after {}: no segment arrived", stream_id, rung.name, why);
    }
    let decision = {
        let mut guard = sessions();
        let Some(s) = guard.get_mut(stream_id).filter(|s| s.generation == generation) else {
            return;
        };
        let now = s.epoch.elapsed().as_secs_f64();
        s.gov.note_measured(measured, why, now)
    };
    if let Some(Decision::Switch { to, why }) = decision {
        switch_to(stream_id, to, why, generation).await;
    }
}

/// Downloads the newest whole segment `playlist_url` lists: `(bytes, secs)`.
async fn measure_segment(playlist_url: &str) -> Option<(u64, f64)> {
    let client = crate::services::http::client();
    let text = client
        .get(playlist_url)
        .timeout(Duration::from_secs(8))
        .send()
        .await
        .ok()?
        .text()
        .await
        .ok()?;
    let line = text
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))?;
    let seg = reqwest::Url::parse(playlist_url).ok()?.join(line).ok()?;
    let t0 = Instant::now();
    let bytes = client
        .get(seg)
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .bytes()
        .await
        .ok()?;
    Some((bytes.len() as u64, t0.elapsed().as_secs_f64()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ladder() -> Vec<Rung> {
        [("1080p60", 8_200_000), ("720p60", 3_420_000), ("480p30", 1_430_000), ("360p30", 630_000), ("160p30", 230_000)]
            .iter()
            .map(|(n, b)| Rung {
                name: n.to_string(),
                bandwidth: *b,
                url: format!("u/{n}"),
                family: "avc1".into(),
            })
            .collect()
    }

    /// A 2 s segment of `bw` bits/s downloaded at `link` bits/s.
    fn seg(bw: u64, link: f64) -> (u64, f64) {
        let bytes = bw / 8 * 2;
        (bytes, bytes as f64 * 8.0 / link)
    }

    #[test]
    fn a_fast_home_link_never_moves() {
        let mut g = Governor::new(ladder(), 0, true, 0.0);
        let mut t = 10.0;
        for _ in 0..300 {
            let (b, s) = seg(8_200_000, 200e6);
            assert_eq!(g.note_throughput(b, s, t), None);
            assert_eq!(g.tick(t), None);
            t += 2.0;
        }
        assert_eq!(g.current(), 0);
        assert_eq!(g.cushion(), 0.0);
    }

    #[test]
    fn a_link_below_the_bitrate_steps_straight_to_what_fits() {
        let mut g = Governor::new(ladder(), 0, true, 0.0);
        let (b, s) = seg(8_200_000, 4e6);
        assert_eq!(g.note_throughput(b, s, 10.0), None, "one sample is not enough");
        // 4 Mbit/s * 0.7 = 2.8: 720p60 (3.42) does not fit, 480p30 does.
        assert_eq!(
            g.note_throughput(b, s, 12.0),
            Some(Decision::Switch { to: 2, why: "throughput below the bitrate" })
        );
    }

    #[test]
    fn a_long_stall_with_no_throughput_known_measures_then_steps_down() {
        let mut g = Governor::new(ladder(), 0, true, 0.0);
        assert_eq!(g.note_stall(1.0, false, 20.0), None);
        assert_eq!(g.note_stall(3.2, false, 22.0), Some(Decision::Measure { why: "a long stall" }));
        // 4 Mbit/s cannot carry 8.2 twice over: down, straight to what fits.
        let (b, s) = seg(8_200_000, 4e6);
        assert_eq!(
            g.note_measured(Some((b, s)), "a long stall", 26.0),
            Some(Decision::Switch { to: 2, why: "a long stall" })
        );
    }

    #[test]
    fn a_stall_on_a_fast_link_measured_as_fast_keeps_the_rung() {
        // The parts path at home: no transfers of its own, then a CDN hiccup.
        let mut g = Governor::new(ladder(), 0, true, 0.0);
        assert_eq!(g.note_stall(3.5, false, 40.0), Some(Decision::Measure { why: "a long stall" }));
        let (b, s) = seg(8_200_000, 300e6);
        assert_eq!(g.note_measured(Some((b, s)), "a long stall", 41.0), None);
        // The measurement is fresh now: the next trouble is judged by it.
        assert_eq!(g.note_stall(3.5, false, 50.0), None);
        assert_eq!(g.current(), 0);
    }

    #[test]
    fn a_measurement_that_fails_steps_down() {
        let mut g = Governor::new(ladder(), 0, true, 0.0);
        assert_eq!(g.note_stall(4.0, false, 20.0), Some(Decision::Measure { why: "a long stall" }));
        assert_eq!(
            g.note_measured(None, "a long stall", 30.0),
            Some(Decision::Switch { to: 1, why: "a long stall" })
        );
    }

    #[test]
    fn measurements_are_rate_limited() {
        let mut g = Governor::new(ladder(), 0, true, 0.0);
        assert_eq!(g.note_stall(4.0, false, 20.0), Some(Decision::Measure { why: "a long stall" }));
        // Still nothing fresh 5 s later (the measurement failed and was never
        // reported): no second measurement, the trouble steps down.
        assert_eq!(
            g.note_stall(4.0, false, 25.0),
            Some(Decision::Switch { to: 1, why: "a long stall" })
        );
    }

    #[test]
    fn stale_throughput_is_not_trusted() {
        let mut g = Governor::new(ladder(), 0, true, 0.0);
        for i in 0..4 {
            let (b, s) = seg(8_200_000, 300e6);
            g.note_throughput(b, s, 10.0 + i as f64);
        }
        // A minute later the link may be another one: measure, do not assume.
        assert_eq!(g.note_stall(3.5, false, 80.0), Some(Decision::Measure { why: "a long stall" }));
        // A new sample starts a new estimate instead of averaging with the old.
        let (b, s) = seg(8_200_000, 4e6);
        assert_eq!(
            g.note_measured(Some((b, s)), "a long stall", 82.0),
            Some(Decision::Switch { to: 2, why: "a long stall" })
        );
    }

    #[test]
    fn two_short_stalls_in_a_minute_step_down_and_grow_the_cushion() {
        let mut g = Governor::new(ladder(), 0, true, 0.0);
        assert_eq!(g.note_stall(0.8, true, 20.0), None);
        assert_eq!(g.cushion(), 1.0);
        assert_eq!(
            g.note_stall(0.9, true, 50.0),
            Some(Decision::Measure { why: "repeated stalls" })
        );
        assert_eq!(g.cushion(), 2.0);
        let (b, s) = seg(8_200_000, 9e6);
        assert_eq!(
            g.note_measured(Some((b, s)), "repeated stalls", 52.0),
            Some(Decision::Switch { to: 1, why: "repeated stalls" })
        );
    }

    #[test]
    fn stalls_on_a_link_with_headroom_only_grow_the_cushion() {
        let mut g = Governor::new(ladder(), 1, true, 0.0);
        for i in 0..4 {
            let (b, s) = seg(3_420_000, 30e6);
            g.note_throughput(b, s, 10.0 + i as f64);
        }
        assert_eq!(g.note_stall(1.0, true, 20.0), None);
        assert_eq!(g.note_stall(5.0, true, 25.0), None);
        assert_eq!(g.current(), 1);
        assert_eq!(g.cushion(), 2.0);
    }

    #[test]
    fn the_cushion_comes_back_while_playback_stays_clean() {
        let mut g = Governor::new(ladder(), 0, false, 0.0);
        g.note_stall(1.0, true, 10.0);
        g.note_stall(1.0, true, 12.0);
        assert_eq!(g.cushion(), 2.0);
        g.tick(12.0 + 44.0);
        assert_eq!(g.cushion(), 2.0);
        g.tick(12.0 + 45.0);
        assert_eq!(g.cushion(), 1.5);
        g.tick(12.0 + 4.0 * 45.0);
        assert_eq!(g.cushion(), 0.0);
    }

    #[test]
    fn a_named_quality_never_switches() {
        let mut g = Governor::new(ladder(), 0, false, 0.0);
        let (b, s) = seg(8_200_000, 1e6);
        for i in 0..5 {
            assert_eq!(g.note_throughput(b, s, 10.0 + i as f64 * 2.0), None);
        }
        assert_eq!(g.note_stall(10.0, false, 30.0), None);
    }

    #[test]
    fn switches_settle_before_the_next_decision() {
        let mut g = Governor::new(ladder(), 0, true, 0.0);
        g.switched(1, 20.0);
        assert_eq!(g.note_stall(5.0, false, 24.0), None);
        assert_eq!(g.note_stall(5.0, false, 29.0), Some(Decision::Measure { why: "a long stall" }));
        // A measurement that lands inside a new switch's settle is dropped.
        g.switched(2, 30.0);
        assert_eq!(g.note_measured(None, "a long stall", 31.0), None);
    }

    #[test]
    fn steps_up_after_clean_playback_with_headroom_and_backs_off_when_it_fails() {
        let mut g = Governor::new(ladder(), 0, true, 0.0);
        g.switched(2, 10.0); // down to 480p30
        let mut t = 20.0;
        let mut up = None;
        while t < 200.0 && up.is_none() {
            let (b, s) = seg(1_430_000, 6e6); // 6 * 0.7 = 4.2 >= 3.42
            up = g.note_throughput(b, s, t);
            t += 2.0;
        }
        assert_eq!(up, Some(Decision::Switch { to: 1, why: "the link carries more" }));
        assert!(t - 10.0 >= UP_HOLD_BASE);
        g.switched(1, t);
        // It fails within the window: the next up waits twice as long.
        g.switched(2, t + 20.0);
        assert_eq!(g.up_hold, UP_HOLD_BASE * 2.0);
    }

    #[test]
    fn the_parts_path_probes_before_stepping_up() {
        let mut g = Governor::new(ladder(), 0, true, 0.0);
        g.switched(1, 5.0);
        assert_eq!(g.tick(20.0), None);
        assert_eq!(g.tick(40.0), Some(Decision::Probe { rung: 0 }));
        assert_eq!(g.tick(41.0), None, "one probe per hold");
        let (b, s) = seg(8_200_000, 5e6);
        assert_eq!(g.note_probe(0, b, s, 42.0), None, "5 Mbit/s cannot carry the source");
        assert_eq!(g.up_hold, UP_HOLD_BASE * 2.0, "a failed probe waits longer for the next");
        assert_eq!(g.tick(42.0 + UP_HOLD_BASE + 1.0), None);
        assert_eq!(g.tick(42.0 + UP_HOLD_BASE * 2.0), Some(Decision::Probe { rung: 0 }));
        let (b, s) = seg(8_200_000, 20e6);
        assert_eq!(
            g.note_probe(0, b, s, 110.0),
            Some(Decision::Switch { to: 0, why: "the link carries more" })
        );
    }

    #[test]
    fn lost_tails_count_as_trouble() {
        let mut g = Governor::new(ladder(), 0, true, 0.0);
        assert_eq!(g.note_tail_loss(20.0), None);
        assert_eq!(g.cushion(), 0.5);
        assert_eq!(
            g.note_tail_loss(30.0),
            Some(Decision::Measure { why: "lost segment tails" })
        );
        // The origin's own backfill had measured a slow link moments before.
        let mut g = Governor::new(ladder(), 0, true, 0.0);
        let (b, s) = seg(8_200_000, 5e6);
        g.note_throughput(b, s, 12.0);
        g.note_tail_loss(20.0);
        // 5 * 0.7 = 3.5: 720p60 (3.42) fits.
        assert_eq!(
            g.note_tail_loss(30.0),
            Some(Decision::Switch { to: 1, why: "lost segment tails" })
        );
    }

    #[test]
    fn the_first_downloads_pick_the_start_rung_before_anything_plays() {
        // Two backfill segments at 6 Mbit/s against a 8.2 Mbit/s source.
        let mut g = Governor::new(ladder(), 0, true, 0.0);
        let (b, s) = seg(8_200_000, 6e6);
        g.note_throughput(b, s, 1.0);
        g.note_throughput(b, s, 2.0);
        // Still settling, which a running stream would wait out; a start does not.
        assert_eq!(g.start_check(2.5), Some(1), "6 * 0.7 = 4.2: 720p60 fits");
        // A link that carries the rung keeps it.
        let mut g = Governor::new(ladder(), 0, true, 0.0);
        let (b, s) = seg(8_200_000, 40e6);
        g.note_throughput(b, s, 1.0);
        g.note_throughput(b, s, 2.0);
        assert_eq!(g.start_check(2.5), None);
        // One sample, or a named quality, decides nothing.
        let mut g = Governor::new(ladder(), 0, true, 0.0);
        let (b, s) = seg(8_200_000, 2e6);
        g.note_throughput(b, s, 1.0);
        assert_eq!(g.start_check(1.5), None);
        let mut g = Governor::new(ladder(), 0, false, 0.0);
        g.note_throughput(b, s, 1.0);
        g.note_throughput(b, s, 2.0);
        assert_eq!(g.start_check(2.5), None);
    }

    /// A route that applies nothing; these tests only read the decisions.
    struct Still;

    impl Route for Still {
        fn switch(&self, _to: Rung, _same_decoder: bool) -> SwitchFuture {
            Box::pin(async { Ok(None) })
        }

        fn player_measures_link(&self) -> bool {
            false
        }
    }

    #[test]
    fn a_start_overtaken_by_a_newer_one_cannot_move_it() {
        // Two starts of one stream overlap (a named start, then an Auto swap a
        // moment later). The older start's first-download check must not
        // restart the relay the newer start is already serving.
        let id = "test-overtaken-start";
        begin_ladder(id, ladder(), 0, true, Arc::new(Still));
        let older = generation(id);
        begin_ladder(id, ladder(), 0, true, Arc::new(Still));
        let newer = generation(id);
        assert_ne!(older, newer);
        let (b, s) = seg(8_200_000, 6e6);
        note_throughput(id, b, s);
        note_throughput(id, b, s);
        assert!(start_check(id, older).is_none(), "the older start moved the newer stream");
        assert_eq!(start_check(id, newer).map(|r| r.name).as_deref(), Some("720p60"));
        end(id);
    }

    #[test]
    fn the_bottom_rung_has_nowhere_to_go() {
        let mut g = Governor::new(ladder(), 4, true, 0.0);
        assert_eq!(g.note_stall(9.0, false, 20.0), None);
    }

    #[test]
    fn the_pick_keeps_its_name_against_a_richer_twin() {
        use crate::services::twitch_resolver::Variant;
        let v = |url: &str, bw: u64, codecs: &str| Variant {
            name: "1080p60".into(),
            group_id: url.into(),
            width: None,
            height: Some(1080),
            fps: None,
            codecs: Some(codecs.into()),
            bandwidth: Some(bw),
            url: url.into(),
        };
        let variants = vec![
            v("u/h264", 8_000_000, "avc1.64002A,mp4a.40.2"),
            v("u/hevc", 6_000_000, "hvc1.1.2.L150,mp4a.40.2"),
            Variant { name: "720p60".into(), height: Some(720), ..v("u/720", 3_400_000, "avc1.4D401F") },
        ];
        let (rungs, cur) = ladder_from(&variants, "u/hevc");
        assert_eq!(rungs[cur].url, "u/hevc", "the resolver chose HEVC; the ladder keeps it");
        assert_eq!(rungs.len(), 2);
    }

    #[test]
    fn a_provider_ladder_is_named_and_estimates_missing_bitrates() {
        use crate::services::providers::source::PlaybackQuality;
        let q = |name: &str, h: u32, w: u32, bw: Option<u64>, url: &str| PlaybackQuality {
            name: name.into(),
            url: url.into(),
            width: Some(w),
            height: Some(h),
            fps: Some(60.0),
            bandwidth: bw,
        };
        let qualities = vec![
            q("2160p60", 2160, 3840, Some(0), ""),
            q("1440p60", 1440, 2560, None, ""),
            q("1080p60", 1080, 1920, Some(6_000_000), "https://yt/1080.m3u8"),
            q("720p60", 720, 1280, Some(3_000_000), "https://yt/720.m3u8"),
            PlaybackQuality { height: None, ..q("audio_only", 0, 0, Some(128_000), "https://yt/a.m3u8") },
        ];
        let (rungs, cur) = ladder_from_qualities(&qualities, "2160p60");
        let names: Vec<_> = rungs.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["2160p60", "1440p60", "1080p60", "720p60"]);
        assert_eq!(cur, 0);
        assert!(rungs[0].bandwidth > rungs[1].bandwidth && rungs[1].bandwidth > 6_000_000);
        assert_eq!(rungs[2].url, "https://yt/1080.m3u8", "URLs come back without the name key");
        assert_eq!(rungs[0].url, "");
        let (rungs, cur) = ladder_from_qualities(&qualities, "1080p60");
        assert_eq!((rungs.len(), cur), (2, 0));
        assert!(ladder_from_qualities(&qualities, "480p").0.is_empty());
    }

    #[test]
    fn a_remembered_link_starts_below_the_rung_it_could_not_hold() {
        let l = ladder();
        assert_eq!(start_rung(&l, 0, None), 0);
        assert_eq!(start_rung(&l, 0, Some(8_200_000)), 1);
        assert_eq!(start_rung(&l, 0, Some(3_420_000)), 2);
        assert_eq!(start_rung(&l, 2, Some(8_200_000)), 2);
        assert_eq!(start_rung(&l, 0, Some(100)), 4);
    }

    #[test]
    fn the_ladder_never_climbs_above_the_pick_and_skips_audio() {
        use crate::services::twitch_resolver::Variant;
        let v = |name: &str, h: u32, bw: u64, codecs: &str| Variant {
            name: name.into(),
            group_id: name.into(),
            width: None,
            height: Some(h),
            fps: None,
            codecs: Some(codecs.into()),
            bandwidth: Some(bw),
            url: format!("u/{name}"),
        };
        let variants = vec![
            v("1440p60", 1440, 9_000_000, "hvc1.1.2.L150,mp4a.40.2"),
            v("1080p60", 1080, 8_000_000, "avc1.64002A,mp4a.40.2"),
            v("720p60", 720, 3_400_000, "avc1.4D401F,mp4a.40.2"),
            v("480p30", 480, 1_400_000, "avc1.4D401F,mp4a.40.2"),
            Variant { height: None, codecs: Some("mp4a.40.2".into()), ..v("audio_only", 0, 160_000, "") },
        ];
        let (rungs, cur) = ladder_from(&variants, "u/1080p60");
        let names: Vec<_> = rungs.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["1080p60", "720p60", "480p30"]);
        assert_eq!(cur, 0);
        let (rungs, cur) = ladder_from(&variants, "u/720p60");
        assert_eq!(rungs.len(), 2);
        assert_eq!(cur, 0);
        let (rungs, _) = ladder_from(&variants, "u/1440p60");
        let families: Vec<_> = rungs.iter().map(|r| r.family.as_str()).collect();
        assert_eq!(families, ["hvc1", "avc1", "avc1", "avc1"], "an HEVC pick steps down into H.264");
        assert!(ladder_from(&variants, "u/missing").0.is_empty());
    }
}
