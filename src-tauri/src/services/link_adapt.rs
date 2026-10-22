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
//!   the old phone rules (tests) hand it out; every app's player learns its
//!   own cushion per channel.
//!
//! Two rule sets (`Rules`). Every app runs the desktop's; the governor the
//! phones first shipped stays for its unit tests. The desktop's is held to
//! "never hitch": it judges only warm samples (never the
//! first transfer of a session or one taken while a switch settles), steps
//! down only on the median of at least four of them against the rate the
//! rung's content measurably runs at, measures twice and keeps the faster
//! segment before blaming the link for a stall, never steps down for a lost
//! tail, and moves only where the switch is seamless (spliced into the
//! playlist the player reads). Where it would reload the player, Auto holds
//! the rung it started on.
//!
//! Only "Auto" sessions change rendition (`auto` on every platform, and the
//! phones' "best", which is what they shipped as Auto); a quality the viewer
//! picked by name, and the desktop's "best", stay put. A start picks its rung
//! from two timed downloads before the first frame, so a weak link starts on a
//! rung it can carry. A MultiNook tile never moves its own rung: the grid
//! shares one link.
//!
//! The decisions are pure (`Governor`, time passed in) and unit-tested; the
//! runtime half below applies them through the stream's `Route`: a seamless
//! switch of the upstream media playlist on the whole-segment path, or on the
//! phones a relay restart otherwise, announced to the player as
//! `playback-adapt`.

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
/// measured afresh instead of judged by it.
const FRESH_FOR: f64 = 30.0;
/// At most one measurement of the playing rung in this long.
const MEASURE_EVERY: f64 = 15.0;
/// Desktop rules: the warm samples the median needs before it may move the
/// rung, and how many recent ones it keeps.
const WARM_MIN: usize = 4;
const WARM_KEEP: usize = 8;
/// Desktop: the start pick moves off the rung only when even the faster of
/// its first downloads ran below this share of what the rung's content
/// measurably needs. A link that carries the rung, or nearly does, keeps it.
const START_FACTOR: f64 = 0.8;
/// Downloads the start pick needs, each timed from its first byte.
const START_SAMPLES: usize = 2;
/// How long the start pick reads each segment it downloads itself.
const START_READ: Duration = Duration::from_millis(1500);
/// How long one Twitch live segment plays. Twitch cuts every rendition into
/// segments of this length, so a relay's own download of one also says what
/// the rung's content really runs at.
pub const TWITCH_SEGMENT_SECS: f64 = 2.0;

/// Which decisions a session makes.
///
/// `Phone` is the governor the phones first shipped (kept for its tests):
/// stalls and lost tails step down at once, judged by an EWMA of every
/// transfer, and a relay restart is an acceptable way to switch. `Desktop` is held to a player that must never
/// hitch: it judges only warm samples, steps down only when the measured link
/// cannot carry what the rung measurably needs, and moves only where the
/// switch is seamless (spliced into the playlist the player already reads).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rules {
    Phone,
    Desktop,
}

/// What the governor wants done.
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// Play rung `to` (an index into the ladder, 0 = best).
    Switch { to: usize, why: &'static str },
    /// Measure the link on rung `rung` before stepping up to it (the parts
    /// path streams in real time, so its own transfers never show headroom).
    Probe { rung: usize },
    /// Time whole segments of the rung playing now before acting on `why`:
    /// with no warm throughput, a stall cannot tell a link too slow for the
    /// rung from a hiccup on a fast one.
    Measure { why: &'static str },
}

/// One timed download of a whole segment: `bytes` in `secs`, and how long
/// that segment plays when the playlist says (`content_secs`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Measured {
    pub bytes: u64,
    pub secs: f64,
    pub content_secs: Option<f64>,
}

impl Measured {
    /// The link's rate over this download, in bits per second.
    pub fn bps(&self) -> f64 {
        self.bytes as f64 * 8.0 / self.secs.max(0.001)
    }
}

/// One warm desktop sample: the link's rate, and the rate the rung's content
/// actually runs at when the segment's length is known.
#[derive(Debug, Clone, Copy)]
struct Warm {
    bps: f64,
    content_bps: Option<f64>,
    at: f64,
}

/// The middle value (the mean of the two middle ones for an even count).
fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = values.len();
    if n == 0 {
        return 0.0;
    }
    if n % 2 == 1 {
        values[n / 2]
    } else {
        (values[n / 2 - 1] + values[n / 2]) / 2.0
    }
}

/// The pure decision core. Times are seconds on any monotonic clock.
#[derive(Debug, Clone)]
pub struct Governor {
    ladder: Vec<Rung>,
    current: usize,
    /// False when the viewer picked a rendition by name: only the cushion moves.
    auto: bool,
    rules: Rules,
    /// Phone rules: the EWMA estimate of every transfer.
    fast: Option<f64>,
    slow: Option<f64>,
    /// Desktop rules: warm samples since the last switch, newest last.
    warm: Vec<Warm>,
    /// Desktop rules: the next transfer rides a connection that has not
    /// warmed up yet (the first of a session).
    cold_pending: bool,
    /// When the newest kept sample landed.
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
    /// A MultiNook tile: it shares the link with the rest of the grid, so its
    /// own share never moves its rung.
    tile: bool,
    /// Whether a switch now would be seamless (spliced, no reload).
    seamless: bool,
    /// Desktop: the downloads before the first frame (the start pick's
    /// evidence), until the pick is made.
    start_samples: Vec<Measured>,
    start_done: bool,
    /// Why automatic switches are held now, once logged for this episode.
    held: Option<&'static str>,
}

impl Governor {
    /// A governor on the desktop's rules.
    pub fn new(ladder: Vec<Rung>, current: usize, auto: bool, now: f64) -> Self {
        Self::with_rules(ladder, current, auto, Rules::Desktop, now)
    }

    pub fn with_rules(ladder: Vec<Rung>, current: usize, auto: bool, rules: Rules, now: f64) -> Self {
        let current = current.min(ladder.len().saturating_sub(1));
        Self {
            ladder,
            current,
            auto,
            rules,
            fast: None,
            slow: None,
            warm: Vec::new(),
            cold_pending: true,
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
            tile: false,
            seamless: true,
            start_samples: Vec::new(),
            start_done: false,
            held: None,
        }
    }

    pub fn current(&self) -> usize {
        self.current
    }

    pub fn ladder(&self) -> &[Rung] {
        &self.ladder
    }

    pub fn rules(&self) -> Rules {
        self.rules
    }

    /// Extra seconds behind the live edge the player should keep.
    pub fn cushion(&self) -> f64 {
        self.cushion
    }

    /// The phone rules' throughput estimate in bits per second, once there
    /// is one.
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

    /// A MultiNook tile never moves its own rung (see `tile`).
    pub fn set_tile(&mut self, tile: bool) {
        self.tile = tile;
    }

    /// Whether a switch now would be seamless; the runtime asks the stream's
    /// route before every decision.
    pub fn set_seamless(&mut self, seamless: bool) {
        self.seamless = seamless;
    }

    fn settling(&self, now: f64) -> bool {
        now - self.last_switch < SETTLE
    }

    /// Whether automatic switches are off for now: a tile never moves, and on
    /// the desktop nothing moves where the switch would reload the player.
    fn holds(&self) -> bool {
        self.tile || (self.rules == Rules::Desktop && !self.seamless)
    }

    /// Why automatic switches are held, while they are.
    fn hold_reason(&self) -> Option<&'static str> {
        if self.tile {
            Some("a MultiNook tile never moves its own rung")
        } else if self.rules == Rules::Desktop && !self.seamless {
            Some("a switch here would reload the player")
        } else {
            None
        }
    }

    /// The hold logged for this episode, if one is running.
    pub fn holding(&self) -> Option<&'static str> {
        self.held
    }

    /// A decision the hold stopped: logged once per hold episode.
    fn hold(&mut self, would: &str) -> Option<Decision> {
        let reason = self.hold_reason()?;
        if self.held != Some(reason) {
            self.held = Some(reason);
            let rung = self.ladder.get(self.current).map_or("", |r| r.name.as_str());
            info!("[LinkAdapt] holding {rung}: {reason} ({would})");
        }
        None
    }

    /// One whole transfer: `bytes` in `secs`, of a segment that plays
    /// `content_secs` when that is known.
    pub fn note_throughput(&mut self, bytes: u64, secs: f64, content_secs: Option<f64>, now: f64) -> Option<Decision> {
        self.note_start_sample(Measured { bytes, secs, content_secs });
        match self.rules {
            Rules::Phone => {
                if !self.add_ewma(bytes, secs) {
                    return None;
                }
                self.samples += 1;
                self.decide_phone(now)
            }
            Rules::Desktop => {
                if !self.add_warm(bytes, secs, content_secs, now) {
                    return None;
                }
                self.decide_desktop(now)
            }
        }
    }

    /// Phone rules: folds one transfer into the EWMAs; false when it is too
    /// small to say anything about the link's rate.
    fn add_ewma(&mut self, bytes: u64, secs: f64) -> bool {
        if bytes < MIN_SAMPLE_BYTES || secs <= 0.0 {
            return false;
        }
        let bps = bytes as f64 * 8.0 / secs.max(0.001);
        let ewma = |prev: Option<f64>, half: f64| {
            let alpha = 1.0 - 0.5f64.powf(1.0 / half);
            Some(prev.map_or(bps, |p| p + (bps - p) * alpha))
        };
        self.fast = ewma(self.fast, FAST_HALF_LIFE);
        self.slow = ewma(self.slow, SLOW_HALF_LIFE);
        true
    }

    /// Desktop rules: keeps one transfer when it says something about the
    /// warm link. The first transfer of a session rides a cold connection
    /// (slow start), and one taken while a switch settles still carries the
    /// switch; both are dropped, never judged.
    fn add_warm(&mut self, bytes: u64, secs: f64, content_secs: Option<f64>, now: f64) -> bool {
        if bytes < MIN_SAMPLE_BYTES || secs <= 0.0 {
            return false;
        }
        if self.cold_pending {
            self.cold_pending = false;
            return false;
        }
        if self.settling(now) {
            return false;
        }
        let bps = bytes as f64 * 8.0 / secs.max(0.001);
        let content_bps = content_secs.filter(|s| *s > 0.0).map(|s| bytes as f64 * 8.0 / s);
        self.warm.retain(|w| now - w.at <= FRESH_FOR);
        self.warm.push(Warm { bps, content_bps, at: now });
        if self.warm.len() > WARM_KEEP {
            self.warm.remove(0);
        }
        self.last_sample = Some(now);
        self.samples += 1;
        true
    }

    /// Desktop rules: the median link rate of the fresh warm samples, once
    /// there are at least `WARM_MIN` of them.
    fn warm_median(&self, now: f64) -> Option<f64> {
        let fresh: Vec<f64> = self
            .warm
            .iter()
            .filter(|w| now - w.at <= FRESH_FOR)
            .map(|w| w.bps)
            .collect();
        if fresh.len() < WARM_MIN {
            return None;
        }
        Some(median(fresh))
    }

    /// The rate the playing rung's content really runs at: measured from the
    /// warm samples whose segment length is known, else what it declares.
    fn content_rate(&self) -> f64 {
        let known: Vec<f64> = self.warm.iter().filter_map(|w| w.content_bps).collect();
        if known.len() >= 2 {
            median(known)
        } else {
            self.bw(self.current) as f64
        }
    }

    /// How far the playing rung's real rate sits from the bitrate it declares,
    /// so the other rungs' declared bitrates are read the same way.
    fn content_scale(&self) -> f64 {
        let declared = self.bw(self.current) as f64;
        if declared > 0.0 {
            self.content_rate() / declared
        } else {
            1.0
        }
    }

    /// Desktop rules: the best rung below the playing one that `link` carries
    /// with room to spare, in the playing rung's codec family (crossing
    /// families needs a fresh decoder, which is a reload). The lowest such
    /// rung when none fits; None when there is nowhere seamless to go.
    fn fit_below(&self, link: f64) -> Option<usize> {
        let family = &self.ladder.get(self.current)?.family;
        let scale = self.content_scale();
        let below: Vec<usize> = (self.current + 1..self.ladder.len())
            .filter(|&i| &self.ladder[i].family == family)
            .collect();
        let lowest = *below.last()?;
        Some(
            below
                .iter()
                .copied()
                .find(|&i| self.bw(i) as f64 * scale <= link * PICK_FACTOR)
                .unwrap_or(lowest),
        )
    }

    /// Desktop rules: the next rung up in the playing rung's codec family.
    fn next_up(&self) -> Option<usize> {
        let family = &self.ladder.get(self.current)?.family;
        (0..self.current).rev().find(|&i| &self.ladder[i].family == family)
    }

    fn step_down_desktop(&self, link: f64, why: &'static str) -> Option<Decision> {
        self.fit_below(link).map(|to| Decision::Switch { to, why })
    }

    /// Desktop rules: trouble steps down only when the measured link cannot
    /// carry what the rung measurably needs. Warm samples answer that when
    /// there are enough; otherwise the link is measured first, at most every
    /// `MEASURE_EVERY`, and nothing moves until it is.
    fn judge_desktop(&mut self, why: &'static str, now: f64) -> Option<Decision> {
        if let Some(link) = self.warm_median(now) {
            if link >= self.content_rate() * SUSTAIN_FACTOR {
                return None;
            }
            return self.step_down_desktop(link, why);
        }
        if self.last_measure.is_none_or(|t| now - t >= MEASURE_EVERY) {
            self.last_measure = Some(now);
            return Some(Decision::Measure { why });
        }
        None
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
        if self.holds() {
            if self.rules == Rules::Desktop && (secs >= LONG_STALL || self.recent_troubles(now) >= 2) {
                return self.hold("after stalls");
            }
            return None;
        }
        match self.rules {
            Rules::Phone => {
                let jitter = self
                    .estimate()
                    .is_some_and(|e| e >= self.bw(self.current) as f64 * JITTER_HEADROOM);
                if jitter {
                    return None;
                }
                if secs >= LONG_STALL {
                    return self.step_down("a long stall");
                }
                if self.recent_troubles(now) >= 2 {
                    return self.step_down("repeated stalls");
                }
                None
            }
            Rules::Desktop => {
                let why = if secs >= LONG_STALL {
                    "a long stall"
                } else if self.recent_troubles(now) >= 2 {
                    "repeated stalls"
                } else {
                    return None;
                };
                self.judge_desktop(why, now)
            }
        }
    }

    /// The origin gave up on a segment's tail (slower than real time, or a
    /// dead transfer): the player will meet a hole.
    pub fn note_tail_loss(&mut self, now: f64) -> Option<Decision> {
        if self.settling(now) {
            return None;
        }
        match self.rules {
            Rules::Phone => {
                self.add_trouble(now, CUSHION_PER_TAIL);
                if self.auto && !self.holds() && self.recent_troubles(now) >= 2 {
                    return self.step_down("lost segment tails");
                }
                None
            }
            // A lost tail is as often the broadcaster's ingest as the link, so
            // on the desktop it never moves the rung; it only restarts the
            // clean-playback clock a step up waits for.
            Rules::Desktop => {
                self.last_trouble = now;
                None
            }
        }
    }

    /// The measurement a `Measure` asked for: the faster of two whole warm
    /// segments of the playing rung, or `None` when none arrived.
    pub fn note_measured(&mut self, sample: Option<Measured>, why: &'static str, now: f64) -> Option<Decision> {
        self.last_measure = Some(now);
        if self.settling(now) || !self.auto || self.holds() {
            return None;
        }
        // Nothing arrived: no evidence the link is the cause (a broadcast can
        // stall just as well), so the rung stays.
        let m = sample?;
        if m.bytes < MIN_SAMPLE_BYTES || m.secs <= 0.0 {
            return None;
        }
        let link = m.bps();
        let rate = m
            .content_secs
            .filter(|s| *s > 0.0)
            .map_or_else(|| self.content_rate(), |s| m.bytes as f64 * 8.0 / s);
        if link >= rate * SUSTAIN_FACTOR {
            return None;
        }
        self.step_down_desktop(link, why)
    }

    /// A download before the first frame, timed from its first byte: the
    /// start pick's evidence (desktop only, until the pick is made).
    pub fn note_start_sample(&mut self, sample: Measured) {
        if self.rules == Rules::Desktop
            && !self.start_done
            && self.start_samples.len() < 2 * START_SAMPLES
            && sample.bytes >= MIN_SAMPLE_BYTES
            && sample.secs > 0.0
        {
            self.start_samples.push(sample);
        }
    }

    /// Whether the start pick still needs downloads of its own (the
    /// whole-segment path's relay downloads nothing before the player).
    pub fn start_wants_samples(&self) -> bool {
        self.rules == Rules::Desktop
            && !self.start_done
            && self.auto
            && !self.tile
            && self.start_samples.len() < START_SAMPLES
    }

    /// The faster of the start pick's downloads, in bits per second.
    pub fn start_link(&self) -> Option<f64> {
        self.start_samples.iter().map(|m| m.bps()).reduce(f64::max)
    }

    /// The start pick, before the player has loaded anything: the rung to
    /// start on instead when the link clearly cannot carry this one. Switching
    /// now costs no freeze on any path, so it is made where later switches are
    /// held, but never for a tile (the grid shares one link). It needs
    /// `START_SAMPLES` downloads timed from their first byte and moves only
    /// when even the faster one ran under `START_FACTOR` of the rung's
    /// measured content rate, so a healthy link, one slow cold download or a
    /// borderline link all keep the start. Any rung may be picked: nothing
    /// has been decoded yet. Made once; the phones never move here.
    pub fn start_check(&mut self, _now: f64) -> Option<usize> {
        let samples = std::mem::take(&mut self.start_samples);
        self.start_done = true;
        if self.rules == Rules::Phone || !self.auto || self.tile || samples.len() < START_SAMPLES {
            return None;
        }
        let link = samples.iter().map(|m| m.bps()).fold(0.0, f64::max);
        let measured: Vec<f64> = samples
            .iter()
            .filter_map(|m| m.content_secs.filter(|s| *s > 0.0).map(|s| m.bytes as f64 * 8.0 / s))
            .collect();
        let declared = self.bw(self.current) as f64;
        let content = if measured.is_empty() { declared } else { median(measured) };
        if link >= content * START_FACTOR {
            return None;
        }
        let scale = if declared > 0.0 { content / declared } else { 1.0 };
        let last = self.ladder.len().checked_sub(1)?;
        if self.current >= last {
            return None;
        }
        Some(
            (self.current + 1..=last)
                .find(|&i| self.bw(i) as f64 * scale <= link * PICK_FACTOR)
                .unwrap_or(last),
        )
    }

    /// A periodic look with no new evidence: decays the cushion and considers a
    /// step up.
    pub fn tick(&mut self, now: f64) -> Option<Decision> {
        self.decay(now);
        match self.rules {
            Rules::Phone => self.decide_phone(now),
            Rules::Desktop => self.decide_desktop(now),
        }
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
        // The phones' estimate carries over: it measured the link, not the
        // rung. The desktop judges each rung by samples taken on it.
        self.warm.clear();
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
        let need = match self.rules {
            Rules::Phone => self.bw(rung) as f64,
            Rules::Desktop => self.bw(rung) as f64 * self.content_scale(),
        };
        if rung < self.current && !self.holds() && bps * UP_FACTOR >= need && self.up_ready(now) {
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

    /// Phone rules: one rung down, or straight to what the estimate fits when
    /// that is lower.
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

    /// Phone rules: the decisions the phones shipped.
    fn decide_phone(&mut self, now: f64) -> Option<Decision> {
        if !self.auto || self.ladder.len() < 2 || self.settling(now) || self.holds() {
            return None;
        }
        if let Some(est) = self.estimate() {
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

    /// Desktop rules: step down only on the median of warm samples against
    /// what the rung measurably needs; step up only with room to spare, so the
    /// new rung clears the bar with margin and the two never alternate.
    fn decide_desktop(&mut self, now: f64) -> Option<Decision> {
        if !self.auto || self.ladder.len() < 2 || self.settling(now) {
            return None;
        }
        if self.holds() {
            // What it would do, to say so once.
            let falls = self.warm_median(now).is_some_and(|link| link < self.content_rate() * SUSTAIN_FACTOR);
            return if falls { self.hold("the link cannot carry it") } else { None };
        }
        self.held = None;
        if let Some(link) = self.warm_median(now) {
            if link < self.content_rate() * SUSTAIN_FACTOR {
                return self.step_down_desktop(link, "throughput below the bitrate");
            }
            if self.up_ready(now) {
                if let Some(next) = self.next_up() {
                    if link * UP_FACTOR >= self.bw(next) as f64 * self.content_scale() {
                        return Some(Decision::Switch { to: next, why: "the link carries more" });
                    }
                }
            }
            return None;
        }
        // No warm samples of its own yet: measure the next rung up once the
        // clean-playback hold has passed.
        if self.up_ready(now) && self.last_probe.is_none_or(|t| now - t >= self.up_hold) {
            if let Some(next) = self.next_up() {
                self.last_probe = Some(now);
                return Some(Decision::Probe { rung: next });
            }
        }
        None
    }
}

/// Whether one `CODECS` entry names an audio codec.
fn is_audio_codec(codec: &str) -> bool {
    let c = codec.trim().to_ascii_lowercase();
    ["mp4a", "ac-3", "ec-3", "opus", "flac", "alac", "mp3"]
        .iter()
        .any(|a| c.starts_with(a))
}

/// The video codec family of a rendition's `CODECS` (`avc1`, `hvc1`, `av01`):
/// the first entry that is not audio, wherever it sits in the list.
pub fn codec_family(codecs: Option<&str>) -> String {
    codecs
        .into_iter()
        .flat_map(|c| c.split(','))
        .map(str::trim)
        .find(|c| !c.is_empty() && !is_audio_codec(c))
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

/// The old phone rules, kept only for their unit tests. Every app runs the
/// desktop's rules: its players learn their own cushion per channel and the
/// origin keeps its home deadlines.
pub fn phone_rules() -> bool {
    PHONE_RULES.load(Ordering::Relaxed)
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

    /// Whether moving to another rung of the playing codec family now is
    /// seamless: spliced into the playlist the player reads, no reload. The
    /// desktop moves only then.
    fn seamless(&self) -> bool {
        false
    }

    /// How long one of this stream's segments plays when the platform fixes
    /// it, so the relay's own downloads also time what the rung needs.
    fn segment_secs(&self) -> Option<f64> {
        None
    }

    /// Whether an ad is playing: its segments say nothing about the rung.
    fn ads_playing(&self) -> bool {
        false
    }
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
    /// The discontinuity counter of the player's last reported segment.
    last_cc: Option<u64>,
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
    // The phones start below a rung the link just failed to hold. The
    // desktop starts at its pick: where a switch would reload the player it
    // never climbs back, so a guess low would last the whole session.
    let start = if auto && stream_id == SOLO && phone_rules() {
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
    let rules = if phone_rules() { Rules::Phone } else { Rules::Desktop };
    let mut gov = Governor::with_rules(ladder, start, auto, rules, 0.0);
    // A tile's share of a link the grid splits says nothing about the link.
    gov.set_tile(stream_id != SOLO);
    sessions().insert(
        stream_id.to_string(),
        Session {
            gov,
            epoch: Instant::now(),
            generation,
            applying: false,
            last_cushion_sent: 0.0,
            route,
            last_cc: None,
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
/// restart the relay on when warm downloads say the link cannot carry the one
/// it started with (`Governor::start_check`). Already recorded as the rung
/// playing; None keeps the start. Only for the session this start began
/// (`generation`): a start that a newer one overtook must not move the newer
/// stream. Never written to the remembered link: a start's guess is not
/// evidence a later session should start below.
pub fn start_check(stream_id: &str, generation: Option<u64>) -> Option<Rung> {
    let generation = generation?;
    let rung = {
        let mut guard = sessions();
        let s = guard
            .get_mut(stream_id)
            .filter(|s| s.generation == generation && !s.applying)?;
        let now = s.epoch.elapsed().as_secs_f64();
        let est = s.gov.start_link().unwrap_or(0.0);
        let to = s.gov.start_check(now)?;
        let from = s.gov.ladder().get(s.gov.current())?.name.clone();
        s.gov.switched(to, now);
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

/// Before `start_check`, on a stream whose relay downloaded nothing of its own
/// (the whole-segment path; the parts origin's backfill already timed two
/// segments): download the starting rung's newest segments so the start pick
/// has its evidence before the first frame. Desktop "Auto" solo only.
pub async fn start_measure(stream_id: &str, generation: Option<u64>) {
    let Some(generation) = generation else { return };
    let url = {
        let guard = sessions();
        let Some(s) = guard.get(stream_id).filter(|s| s.generation == generation) else { return };
        if !s.gov.start_wants_samples() {
            return;
        }
        match s.gov.ladder().get(s.gov.current()) {
            Some(rung) => rung.url.clone(),
            None => return,
        }
    };
    let samples = measure_start(&url).await;
    let mut guard = sessions();
    if let Some(s) = guard.get_mut(stream_id).filter(|s| s.generation == generation) {
        for m in samples {
            s.gov.note_start_sample(m);
        }
    }
}

/// The start pick's own downloads: the newest `START_SAMPLES` segments of
/// `playlist_url`, one after the other, each timed from its first byte (its
/// connection's setup left out) and read for at most `START_READ`. A part read
/// carries the share of the segment's length its bytes are, from the length
/// header, so what the rung's content runs at is known too.
async fn measure_start(playlist_url: &str) -> Vec<Measured> {
    let client = crate::services::http::client();
    let Ok(resp) = client.get(playlist_url).timeout(Duration::from_secs(8)).send().await else {
        return Vec::new();
    };
    let Ok(text) = resp.text().await else { return Vec::new() };
    let Ok(base) = reqwest::Url::parse(playlist_url) else { return Vec::new() };
    let mut out = Vec::new();
    for (uri, length) in newest_segments(&text, START_SAMPLES) {
        let Ok(url) = base.join(&uri) else { continue };
        let Ok(resp) = client.get(url).timeout(Duration::from_secs(10)).send().await else { continue };
        let Ok(mut resp) = resp.error_for_status() else { continue };
        let total = resp.content_length();
        let started = Instant::now();
        let mut first_at: Option<Instant> = None;
        let mut bytes: u64 = 0;
        while let Some(left) = START_READ.checked_sub(started.elapsed()) {
            match tokio::time::timeout(left, resp.chunk()).await {
                Ok(Ok(Some(chunk))) => {
                    if first_at.is_none() {
                        first_at = Some(Instant::now());
                    } else {
                        bytes += chunk.len() as u64;
                    }
                }
                _ => break,
            }
        }
        let secs = first_at.map_or(0.0, |t| t.elapsed().as_secs_f64());
        let content_secs = match (total, length) {
            (Some(total), Some(len)) if total > 0 => Some(len * bytes as f64 / total as f64),
            _ => None,
        };
        out.push(Measured { bytes, secs, content_secs });
    }
    out
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
    s.gov.set_tile(to != SOLO);
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
    // Asked with the sessions lock released: the route may look at the
    // origin, whose own lock is sometimes held while it reads the cushion.
    let Some(route) = sessions().get(stream_id).map(|s| s.route.clone()) else { return };
    let seamless = route.seamless();
    let (decision, generation, cushion_moved) = {
        let mut guard = sessions();
        let Some(s) = guard.get_mut(stream_id) else { return };
        s.gov.set_seamless(seamless);
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

/// Relay downloads in flight per stream, and how many have started, so a
/// download that ran alone (it timed the link) can be told from one that
/// shared it with others (each of those timed a share of the link).
static TRANSFERS: Lazy<Mutex<HashMap<String, (usize, u64)>>> = Lazy::new(|| Mutex::new(HashMap::new()));

/// One relay download timed for the governor (`begin_transfer`). It becomes
/// a sample only when no other download of the same stream overlapped it.
pub struct Transfer {
    stream_id: String,
    /// The stream's start count when this one began.
    seq: u64,
    alone_at_start: bool,
    t0: Instant,
    open: bool,
}

/// A relay download of `stream_id` is starting now.
pub fn begin_transfer(stream_id: &str) -> Transfer {
    let mut map = TRANSFERS.lock().unwrap_or_else(|e| e.into_inner());
    let entry = map.entry(stream_id.to_string()).or_insert((0, 0));
    let alone_at_start = entry.0 == 0;
    entry.0 += 1;
    entry.1 += 1;
    Transfer {
        stream_id: stream_id.to_string(),
        seq: entry.1,
        alone_at_start,
        t0: Instant::now(),
        open: true,
    }
}

impl Transfer {
    /// The download finished with `bytes`: a sample when nothing overlapped it.
    pub fn finish(mut self, bytes: u64) {
        if self.close() {
            note_throughput(&self.stream_id, bytes, self.t0.elapsed().as_secs_f64());
        }
    }

    /// Leaves the in-flight count; true when no other download of the stream
    /// was running when this one began or started while it ran.
    fn close(&mut self) -> bool {
        if !self.open {
            return false;
        }
        self.open = false;
        let mut map = TRANSFERS.lock().unwrap_or_else(|e| e.into_inner());
        let Some(entry) = map.get_mut(&self.stream_id) else { return false };
        entry.0 = entry.0.saturating_sub(1);
        let alone = self.alone_at_start && entry.1 == self.seq;
        if entry.0 == 0 {
            map.remove(&self.stream_id);
        }
        alone
    }
}

impl Drop for Transfer {
    fn drop(&mut self) {
        self.close();
    }
}

/// A whole segment a relay downloaded from the internet itself (the parts
/// origin's backfill and catch-up, a provider relay's segment fetches).
pub fn note_throughput(stream_id: &str, bytes: u64, secs: f64) {
    let route = sessions().get(stream_id).map(|s| s.route.clone());
    let content = route.and_then(|r| r.segment_secs());
    with_session(stream_id, |g, now| g.note_throughput(bytes, secs, content, now));
}

/// A whole segment the player downloaded, `content_secs` long, on
/// discontinuity run `cc`. Counted only where that download crossed the
/// internet; behind a relay that serves from memory it timed loopback, and the
/// relay reports its own fetches instead. On the desktop an ad's segments, and
/// the first segment of each new discontinuity run (an ad's edge, a timeline
/// the player is still joining), are left out.
pub fn note_player_segment(stream_id: &str, bytes: u64, secs: f64, content_secs: Option<f64>, cc: Option<u64>) {
    // The route is asked with the sessions lock released: it may look at the
    // origin, whose own lock is sometimes held while it reads the cushion.
    let Some(route) = sessions().get(stream_id).map(|s| s.route.clone()) else { return };
    if !route.player_measures_link() {
        return;
    }
    let desktop = !phone_rules();
    if desktop && route.ads_playing() {
        return;
    }
    let first_of_run = {
        let mut guard = sessions();
        let Some(s) = guard.get_mut(stream_id) else { return };
        let changed = cc.is_some() && s.last_cc.is_some() && s.last_cc != cc;
        if cc.is_some() {
            s.last_cc = cc;
        }
        changed
    };
    if desktop && first_of_run {
        return;
    }
    let content = content_secs.filter(|c| *c > 0.0).or_else(|| route.segment_secs());
    with_session(stream_id, |g, now| g.note_throughput(bytes, secs, content, now));
}

/// A stall the player reported.
pub fn note_stall(stream_id: &str, secs: f64, ended: bool) {
    if ended {
        info!("[LinkAdapt] {}: stall ended after {:.0} ms", stream_id, secs * 1000.0);
    }
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
    let same_decoder = rung.family == from.family;
    let name = rung.name.clone();
    if !phone_rules() && !(same_decoder && route.seamless()) {
        info!("[LinkAdapt] {}: holding {}: a switch to {} would reload the player", stream_id, from.name, name);
        return;
    }
    info!("[LinkAdapt] {}: {} -> {} ({})", stream_id, from.name, rung.name, why);
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
    let count = if phone_rules() { 1 } else { 2 };
    let measured = measure_segment(&target.url, count).await;
    let decision = {
        let mut guard = sessions();
        let Some(s) = guard.get_mut(stream_id).filter(|s| s.generation == generation) else {
            return;
        };
        let now = s.epoch.elapsed().as_secs_f64();
        match measured {
            Some(m) => {
                info!("[LinkAdapt] {}: probe {}: {:.2} Mbit/s", stream_id, target.name, m.bps() / 1e6);
                s.gov.note_probe(rung, m.bytes, m.secs, now)
            }
            None => s.gov.note_probe(rung, 0, 0.0, now),
        }
    };
    if let Some(Decision::Switch { to, why }) = decision {
        switch_to(stream_id, to, why, generation).await;
    }
}

/// Times two whole segments of the rung playing now (the first warms the
/// connection), then lets the governor judge the trouble by the faster one.
async fn measure(stream_id: &str, why: &'static str, generation: u64) {
    let current = {
        let guard = sessions();
        guard
            .get(stream_id)
            .filter(|s| s.generation == generation)
            .and_then(|s| s.gov.ladder().get(s.gov.current()).cloned())
    };
    let Some(rung) = current else { return };
    let measured = measure_segment(&rung.url, 2).await;
    if let Some(m) = measured {
        info!("[LinkAdapt] {}: measured {} after {}: {:.2} Mbit/s", stream_id, rung.name, why, m.bps() / 1e6);
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

/// The newest `count` whole segments a media playlist lists, oldest first:
/// each URI with how long it plays (`#EXTINF`).
fn newest_segments(playlist: &str, count: usize) -> Vec<(String, Option<f64>)> {
    let mut out: Vec<(String, Option<f64>)> = Vec::new();
    let mut dur: Option<f64> = None;
    for line in playlist.lines().map(str::trim) {
        if let Some(v) = line.strip_prefix("#EXTINF:") {
            dur = v.split(',').next().and_then(|d| d.trim().parse().ok());
        } else if !line.is_empty() && !line.starts_with('#') {
            out.push((line.to_string(), dur.take()));
        }
    }
    let skip = out.len().saturating_sub(count);
    out.split_off(skip)
}

/// Downloads the newest `count` whole segments `playlist_url` lists, one
/// after the other on the same client, and returns the fastest.
async fn measure_segment(playlist_url: &str, count: usize) -> Option<Measured> {
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
    let base = reqwest::Url::parse(playlist_url).ok()?;
    let mut best: Option<Measured> = None;
    for (uri, content_secs) in newest_segments(&text, count) {
        let Ok(url) = base.join(&uri) else { continue };
        let t0 = Instant::now();
        let fetched = async {
            client
                .get(url)
                .timeout(Duration::from_secs(10))
                .send()
                .await
                .ok()?
                .error_for_status()
                .ok()?
                .bytes()
                .await
                .ok()
        }
        .await;
        let Some(bytes) = fetched else { continue };
        let m = Measured {
            bytes: bytes.len() as u64,
            secs: t0.elapsed().as_secs_f64(),
            content_secs,
        };
        if best.is_none_or(|b| m.bps() > b.bps()) {
            best = Some(m);
        }
    }
    best
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

    /// A governor on the phones' rules.
    fn phone(current: usize, auto: bool) -> Governor {
        Governor::with_rules(ladder(), current, auto, Rules::Phone, 0.0)
    }

    /// A governor on the desktop's rules.
    fn desk(current: usize) -> Governor {
        Governor::with_rules(ladder(), current, true, Rules::Desktop, 0.0)
    }

    /// Feeds `n` warm 2 s segments of `bw` at `link` from `t`, two seconds
    /// apart, after spending the session's cold first sample. Returns the
    /// first decision and the time after the last sample.
    fn feed(g: &mut Governor, bw: u64, link: f64, mut t: f64, n: usize) -> (Option<Decision>, f64) {
        let (b, s) = seg(bw, link);
        if g.cold_pending {
            g.note_throughput(b, s, Some(2.0), t);
            t += 2.0;
        }
        let mut first = None;
        for _ in 0..n {
            let d = g.note_throughput(b, s, Some(2.0), t);
            first = first.or(d);
            t += 2.0;
        }
        (first, t)
    }

    // ── Phone rules: what the phones shipped (ios/v2), unchanged ──

    #[test]
    fn phone_a_fast_home_link_never_moves() {
        let mut g = phone(0, true);
        let mut t = 10.0;
        for _ in 0..300 {
            let (b, s) = seg(8_200_000, 200e6);
            assert_eq!(g.note_throughput(b, s, None, t), None);
            assert_eq!(g.tick(t), None);
            t += 2.0;
        }
        assert_eq!(g.current(), 0);
        assert_eq!(g.cushion(), 0.0);
    }

    #[test]
    fn phone_a_link_below_the_bitrate_steps_straight_to_what_fits() {
        let mut g = phone(0, true);
        let (b, s) = seg(8_200_000, 4e6);
        assert_eq!(g.note_throughput(b, s, None, 10.0), None, "one sample is not enough");
        // 4 Mbit/s * 0.7 = 2.8: 720p60 (3.42) does not fit, 480p30 does.
        assert_eq!(
            g.note_throughput(b, s, None, 12.0),
            Some(Decision::Switch { to: 2, why: "throughput below the bitrate" })
        );
    }

    #[test]
    fn phone_a_long_stall_steps_down_with_no_throughput_known() {
        let mut g = phone(0, true);
        assert_eq!(g.note_stall(1.0, false, 20.0), None);
        assert_eq!(
            g.note_stall(3.2, false, 22.0),
            Some(Decision::Switch { to: 1, why: "a long stall" })
        );
    }

    #[test]
    fn phone_two_short_stalls_in_a_minute_step_down_and_grow_the_cushion() {
        let mut g = phone(0, true);
        assert_eq!(g.note_stall(0.8, true, 20.0), None);
        assert_eq!(g.cushion(), 1.0);
        assert_eq!(
            g.note_stall(0.9, true, 50.0),
            Some(Decision::Switch { to: 1, why: "repeated stalls" })
        );
        assert_eq!(g.cushion(), 2.0);
    }

    #[test]
    fn phone_stalls_on_a_link_with_headroom_only_grow_the_cushion() {
        let mut g = phone(1, true);
        for i in 0..4 {
            let (b, s) = seg(3_420_000, 30e6);
            g.note_throughput(b, s, None, 10.0 + i as f64);
        }
        assert_eq!(g.note_stall(1.0, true, 20.0), None);
        assert_eq!(g.note_stall(5.0, true, 25.0), None);
        assert_eq!(g.current(), 1);
        assert_eq!(g.cushion(), 2.0);
    }

    #[test]
    fn phone_the_cushion_comes_back_while_playback_stays_clean() {
        let mut g = phone(0, false);
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
    fn phone_a_named_quality_never_switches() {
        let mut g = phone(0, false);
        let (b, s) = seg(8_200_000, 1e6);
        for i in 0..5 {
            assert_eq!(g.note_throughput(b, s, None, 10.0 + i as f64 * 2.0), None);
        }
        assert_eq!(g.note_stall(10.0, false, 30.0), None);
    }

    #[test]
    fn phone_switches_settle_before_the_next_decision() {
        let mut g = phone(0, true);
        g.switched(1, 20.0);
        assert_eq!(g.note_stall(5.0, false, 24.0), None);
        assert_eq!(
            g.note_stall(5.0, false, 29.0),
            Some(Decision::Switch { to: 2, why: "a long stall" })
        );
    }

    #[test]
    fn phone_steps_up_after_clean_playback_with_headroom_and_backs_off_when_it_fails() {
        let mut g = phone(0, true);
        g.switched(2, 10.0); // down to 480p30
        let mut t = 20.0;
        let mut up = None;
        while t < 200.0 && up.is_none() {
            let (b, s) = seg(1_430_000, 6e6); // 6 * 0.7 = 4.2 >= 3.42
            up = g.note_throughput(b, s, None, t);
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
    fn phone_the_parts_path_probes_before_stepping_up() {
        let mut g = phone(0, true);
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
    fn phone_lost_tails_count_as_trouble() {
        let mut g = phone(0, true);
        assert_eq!(g.note_tail_loss(20.0), None);
        assert_eq!(g.cushion(), 0.5);
        assert_eq!(
            g.note_tail_loss(30.0),
            Some(Decision::Switch { to: 1, why: "lost segment tails" })
        );
    }

    #[test]
    fn phone_the_bottom_rung_has_nowhere_to_go() {
        let mut g = phone(4, true);
        assert_eq!(g.note_stall(9.0, false, 20.0), None);
    }

    #[test]
    fn phone_never_moves_at_the_start_and_never_measures() {
        let mut g = phone(0, true);
        let (b, s) = seg(8_200_000, 2e6);
        g.note_throughput(b, s, None, 1.0);
        g.note_throughput(b, s, None, 2.0);
        assert_eq!(g.start_check(2.5), None);
        // Trouble on the phone steps down at once, as it always did.
        assert!(matches!(g.note_stall(4.0, false, 20.0), Some(Decision::Switch { .. })));
    }

    // ── Desktop rules: never hitch ──

    #[test]
    fn desktop_a_fast_home_link_never_moves() {
        let mut g = desk(0);
        let mut t = 10.0;
        for _ in 0..300 {
            let (b, s) = seg(8_200_000, 200e6);
            assert_eq!(g.note_throughput(b, s, Some(2.0), t), None);
            assert_eq!(g.tick(t), None);
            t += 2.0;
        }
        assert_eq!(g.current(), 0);
    }

    #[test]
    fn desktop_never_judges_cold_or_settling_samples() {
        let mut g = desk(0);
        let (b, s) = seg(8_200_000, 4e6);
        // The session's first transfer (cold), then three inside the start's
        // settle: none of them count.
        for t in [1.0, 3.0, 5.0, 7.0] {
            assert_eq!(g.note_throughput(b, s, Some(2.0), t), None);
        }
        assert!(g.warm.is_empty());
        // After it, four warm samples before anything moves.
        for t in [10.0, 12.0, 14.0] {
            assert_eq!(g.note_throughput(b, s, Some(2.0), t), None);
        }
        // 4 Mbit/s * 0.7 = 2.8: 720p60 (3.42) does not fit, 480p30 does.
        assert_eq!(
            g.note_throughput(b, s, Some(2.0), 16.0),
            Some(Decision::Switch { to: 2, why: "throughput below the bitrate" })
        );
    }

    #[test]
    fn desktop_the_first_sample_of_a_session_is_dropped_even_after_settle() {
        let mut g = desk(0);
        let (b, s) = seg(8_200_000, 4e6);
        for t in [10.0, 12.0, 14.0, 16.0] {
            assert_eq!(g.note_throughput(b, s, Some(2.0), t), None);
        }
        assert_eq!(g.warm.len(), 3, "the first was cold");
        assert!(g.note_throughput(b, s, Some(2.0), 18.0).is_some());
    }

    #[test]
    fn desktop_one_slow_sample_among_fast_ones_moves_nothing() {
        let mut g = desk(0);
        let (fb, fs) = seg(8_200_000, 20e6);
        let (sb, ss) = seg(8_200_000, 2e6);
        g.note_throughput(fb, fs, Some(2.0), 9.0); // cold
        for (i, (b, s)) in [(fb, fs), (fb, fs), (sb, ss), (fb, fs), (fb, fs)].into_iter().enumerate() {
            assert_eq!(g.note_throughput(b, s, Some(2.0), 10.0 + 2.0 * i as f64), None);
        }
        assert_eq!(g.current(), 0);
    }

    #[test]
    fn desktop_judges_the_measured_content_rate_not_the_declared_one() {
        // The rung declares 8.2 Mbit/s but its segments run at 6; the link
        // carries 8, which is 1.33x what the content needs.
        let bytes = 1_500_000u64;
        let secs = bytes as f64 * 8.0 / 8e6;
        let mut g = desk(0);
        g.note_throughput(bytes, secs, Some(2.0), 9.0); // cold
        for i in 0..6 {
            assert_eq!(g.note_throughput(bytes, secs, Some(2.0), 10.0 + 2.0 * i as f64), None);
        }
        // Judged by the declared bitrate instead, 8 < 8.2 * 1.2 would step down.
        let mut h = desk(0);
        h.note_throughput(bytes, secs, None, 9.0); // cold
        let mut moved = None;
        for i in 0..4 {
            moved = moved.or(h.note_throughput(bytes, secs, None, 10.0 + 2.0 * i as f64));
        }
        assert!(moved.is_some());
    }

    #[test]
    fn desktop_a_stall_on_a_warm_fast_link_keeps_the_rung() {
        let mut g = desk(0);
        let (_, t) = feed(&mut g, 8_200_000, 40e6, 10.0, 4);
        assert_eq!(g.note_stall(5.0, false, t), None);
        assert_eq!(g.note_stall(1.0, true, t + 1.0), None);
        assert_eq!(g.note_stall(1.0, true, t + 2.0), None);
        assert_eq!(g.current(), 0);
    }

    #[test]
    fn desktop_a_stall_with_no_warm_samples_measures_first() {
        let mut g = desk(0);
        assert_eq!(g.note_stall(1.0, false, 20.0), None, "a short stall alone is not trouble");
        assert_eq!(g.note_stall(3.5, false, 40.0), Some(Decision::Measure { why: "a long stall" }));
        // The faster of the two segments came in at 160 Mbit/s: a hiccup.
        let fast = Measured { bytes: 2_050_000, secs: 0.1, content_secs: Some(2.0) };
        assert_eq!(g.note_measured(Some(fast), "a long stall", 41.0), None);
        assert_eq!(g.current(), 0);
    }

    #[test]
    fn desktop_a_measured_link_that_cannot_carry_the_rung_steps_down() {
        let mut g = desk(0);
        assert_eq!(g.note_stall(4.0, false, 20.0), Some(Decision::Measure { why: "a long stall" }));
        // 8.2 Mbit/s of content arriving at 4 Mbit/s.
        let slow = Measured { bytes: 2_050_000, secs: 2_050_000.0 * 8.0 / 4e6, content_secs: Some(2.0) };
        assert_eq!(
            g.note_measured(Some(slow), "a long stall", 22.0),
            Some(Decision::Switch { to: 2, why: "a long stall" })
        );
        // 1.1x the content's rate is not enough headroom either.
        let mut h = desk(0);
        h.note_stall(4.0, false, 20.0);
        let tight = Measured { bytes: 2_050_000, secs: 2_050_000.0 * 8.0 / (8.2e6 * 1.1), content_secs: Some(2.0) };
        assert!(h.note_measured(Some(tight), "a long stall", 22.0).is_some());
    }

    #[test]
    fn desktop_a_measurement_that_brought_nothing_keeps_the_rung() {
        let mut g = desk(0);
        assert_eq!(g.note_stall(4.0, false, 20.0), Some(Decision::Measure { why: "a long stall" }));
        assert_eq!(g.note_measured(None, "a long stall", 30.0), None);
        // And it does not measure again inside the window: no evidence, no move.
        assert_eq!(g.note_stall(4.0, false, 32.0), None);
        assert_eq!(g.note_stall(4.0, false, 46.0), Some(Decision::Measure { why: "a long stall" }));
        assert_eq!(g.current(), 0);
    }

    #[test]
    fn desktop_lost_tails_never_step_down() {
        let mut g = desk(0);
        for t in [20.0, 25.0, 30.0, 35.0, 40.0] {
            assert_eq!(g.note_tail_loss(t), None);
        }
        assert_eq!(g.current(), 0);
        // Even on a slow link: the throughput decides that, never the tails.
        let (_, t) = feed(&mut g, 8_200_000, 4e6, 50.0, 4);
        assert_eq!(g.note_tail_loss(t), None);
        assert_eq!(g.note_tail_loss(t + 1.0), None);
    }

    #[test]
    fn desktop_holds_where_a_switch_would_reload_the_player() {
        let mut g = desk(0);
        g.set_seamless(false);
        let (d, t) = feed(&mut g, 8_200_000, 4e6, 10.0, 6);
        assert_eq!(d, None);
        assert_eq!(g.note_stall(5.0, false, t), None);
        assert_eq!(g.tick(t + 100.0), None);
        // The same evidence moves it once the switch can be spliced.
        g.set_seamless(true);
        assert!(matches!(feed(&mut g, 8_200_000, 4e6, t + 102.0, 4).0, Some(Decision::Switch { .. })));
    }

    #[test]
    fn desktop_a_tile_never_moves_its_own_rung() {
        let mut g = desk(0);
        g.set_tile(true);
        let (d, t) = feed(&mut g, 8_200_000, 2e6, 10.0, 8);
        assert_eq!(d, None);
        assert_eq!(g.note_stall(9.0, false, t), None);
        assert_eq!(g.start_check(t), None);
    }

    #[test]
    fn desktop_steps_down_only_inside_the_codec_family() {
        let r = |name: &str, bw: u64, family: &str| Rung {
            name: name.into(),
            bandwidth: bw,
            url: format!("u/{name}"),
            family: family.into(),
        };
        let mixed = vec![
            r("1440p60", 9_000_000, "hvc1"),
            r("1080p60h", 5_000_000, "hvc1"),
            r("1080p60", 8_000_000, "avc1"),
            r("720p60", 3_400_000, "avc1"),
        ];
        let mut g = Governor::with_rules(mixed.clone(), 0, true, Rules::Desktop, 0.0);
        let (d, _) = feed(&mut g, 9_000_000, 4e6, 10.0, 4);
        assert_eq!(d, Some(Decision::Switch { to: 1, why: "throughput below the bitrate" }));
        // An HEVC source with nothing else HEVC below it has nowhere seamless to go.
        let lone = vec![mixed[0].clone(), mixed[2].clone(), mixed[3].clone()];
        let mut h = Governor::with_rules(lone, 0, true, Rules::Desktop, 0.0);
        assert_eq!(feed(&mut h, 9_000_000, 2e6, 10.0, 6).0, None);
    }

    #[test]
    fn desktop_steps_up_with_room_and_never_alternates() {
        let mut g = desk(0);
        g.switched(2, 10.0); // on 480p30
        g.cold_pending = false;
        let mut t = 20.0;
        let mut up = None;
        while t < 200.0 && up.is_none() {
            let (b, s) = seg(1_430_000, 6e6); // 6 * 0.7 = 4.2 >= 3.42
            up = g.note_throughput(b, s, Some(2.0), t);
            t += 2.0;
        }
        assert_eq!(up, Some(Decision::Switch { to: 1, why: "the link carries more" }));
        g.switched(1, t);
        // The same 6 Mbit/s carries 720p60 with 1.75x to spare and cannot reach
        // the source: nothing moves either way for minutes.
        for _ in 0..150 {
            let (b, s) = seg(3_420_000, 6e6);
            assert_eq!(g.note_throughput(b, s, Some(2.0), t), None);
            assert_eq!(g.tick(t), None);
            t += 2.0;
        }
        assert_eq!(g.current(), 1);
    }

    #[test]
    fn desktop_a_weak_start_picks_the_rung_that_fits_before_the_first_frame() {
        // The parts origin's two backfill segments of 8.2 Mbit/s content, both
        // at 4 Mbit/s: 4 * 0.7 = 2.8 carries 480p30 (1.43), not 720p60.
        let mut g = desk(0);
        let (b, s) = seg(8_200_000, 4e6);
        g.note_throughput(b, s, Some(2.0), 1.0);
        g.note_throughput(b, s, Some(2.0), 2.0);
        assert_eq!(g.start_check(2.5), Some(2));
        // Made once: later evidence goes to the running rules.
        assert_eq!(g.start_check(3.0), None);
    }

    #[test]
    fn desktop_a_healthy_start_keeps_the_top_even_after_one_slow_cold_download() {
        let mut g = desk(0);
        let (b, cold) = seg(8_200_000, 3e6);
        let (_, warm) = seg(8_200_000, 270e6);
        g.note_throughput(b, cold, Some(2.0), 1.0);
        g.note_throughput(b, warm, Some(2.0), 2.0);
        assert_eq!(g.start_check(2.5), None);
        // One download is not enough to move at all.
        let mut h = desk(0);
        h.note_throughput(b, cold, Some(2.0), 1.0);
        assert!(h.start_wants_samples(), "the start pick downloads its own");
        assert_eq!(h.start_check(2.5), None);
    }

    #[test]
    fn desktop_a_borderline_start_keeps_the_top() {
        // 7 Mbit/s for 8.2 Mbit/s of content: 85%, above the 80% bar.
        let mut g = desk(0);
        let (b, s) = seg(8_200_000, 7e6);
        g.note_throughput(b, s, Some(2.0), 1.0);
        g.note_throughput(b, s, Some(2.0), 2.0);
        assert_eq!(g.start_check(2.5), None);
        // Judged by what the content measurably runs at: 6 Mbit/s of real
        // content (declared 8.2) arriving at 5.5 is 92% of it.
        let mut h = desk(0);
        let bytes = 1_500_000u64;
        let secs = bytes as f64 * 8.0 / 5.5e6;
        h.note_throughput(bytes, secs, Some(2.0), 1.0);
        h.note_throughput(bytes, secs, Some(2.0), 2.0);
        assert_eq!(h.start_check(2.5), None);
    }

    #[test]
    fn desktop_the_start_pick_runs_where_later_switches_hold_but_never_for_a_tile() {
        let (b, s) = seg(8_200_000, 2e6);
        let mut g = desk(0);
        g.set_seamless(false); // the parts path: no seamless switch after the start
        g.note_throughput(b, s, Some(2.0), 1.0);
        g.note_throughput(b, s, Some(2.0), 2.0);
        // 2 * 0.7 = 1.4: 360p30 (0.63) is the best that fits, any family.
        assert_eq!(g.start_check(2.5), Some(3));
        let mut t = desk(0);
        t.set_tile(true);
        t.note_throughput(b, s, Some(2.0), 1.0);
        t.note_throughput(b, s, Some(2.0), 2.0);
        assert!(!t.start_wants_samples());
        assert_eq!(t.start_check(2.5), None);
        let mut p = phone(0, true);
        p.note_throughput(b, s, None, 1.0);
        p.note_throughput(b, s, None, 2.0);
        assert_eq!(p.start_check(2.5), None, "the phones never move here");
    }

    #[test]
    fn desktop_a_hold_is_logged_once_per_episode() {
        let mut g = desk(0);
        g.set_seamless(false);
        let (d, t) = feed(&mut g, 8_200_000, 4e6, 10.0, 6);
        assert_eq!(d, None);
        assert_eq!(g.holding(), Some("a switch here would reload the player"));
        // Still held: the episode is not logged again.
        g.held = Some("a switch here would reload the player");
        assert_eq!(g.note_stall(5.0, false, t), None);
        assert_eq!(g.holding(), Some("a switch here would reload the player"));
        // The switch becomes seamless: it moves, and the episode ends.
        g.set_seamless(true);
        assert!(matches!(feed(&mut g, 8_200_000, 4e6, t + 2.0, 4).0, Some(Decision::Switch { .. })));
        assert_eq!(g.holding(), None);
        // A healthy held stream logs nothing.
        let mut h = desk(0);
        h.set_seamless(false);
        assert_eq!(feed(&mut h, 8_200_000, 40e6, 10.0, 6).0, None);
        assert_eq!(h.holding(), None);
    }

    #[test]
    fn desktop_probes_the_next_rung_when_it_has_no_samples() {
        let mut g = desk(0);
        g.switched(1, 5.0);
        assert_eq!(g.tick(20.0), None);
        assert_eq!(g.tick(40.0), Some(Decision::Probe { rung: 0 }));
        let (b, s) = seg(8_200_000, 5e6);
        assert_eq!(g.note_probe(0, b, s, 42.0), None);
        assert_eq!(g.up_hold, UP_HOLD_BASE * 2.0);
    }

    #[test]
    fn desktop_the_bottom_rung_has_nowhere_to_go() {
        let mut g = desk(4);
        let (d, _) = feed(&mut g, 230_000, 50_000.0, 10.0, 6);
        assert_eq!(d, None);
    }

    // ── The runtime half ──

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

    /// A whole-segment route whose player times the link; `ads` says an ad
    /// is playing.
    struct Direct {
        ads: bool,
    }

    impl Route for Direct {
        fn switch(&self, _to: Rung, _same_decoder: bool) -> SwitchFuture {
            Box::pin(async { Ok(None) })
        }

        fn player_measures_link(&self) -> bool {
            true
        }

        fn seamless(&self) -> bool {
            true
        }

        fn segment_secs(&self) -> Option<f64> {
            Some(TWITCH_SEGMENT_SECS)
        }

        fn ads_playing(&self) -> bool {
            self.ads
        }
    }

    /// A session well past its start (settled and warm), on `route`.
    fn settled_session(id: &str, route: Arc<dyn Route>) {
        let mut gov = Governor::with_rules(ladder(), 0, true, Rules::Desktop, 0.0);
        gov.cold_pending = false;
        let epoch = Instant::now().checked_sub(Duration::from_secs(60)).unwrap_or_else(Instant::now);
        sessions().insert(
            id.to_string(),
            Session {
                gov,
                epoch,
                generation: 0,
                applying: false,
                last_cushion_sent: 0.0,
                route,
                last_cc: None,
            },
        );
    }

    fn warm_count(id: &str) -> usize {
        sessions().get(id).map_or(0, |s| s.gov.warm.len())
    }

    #[test]
    fn a_start_overtaken_by_a_newer_one_cannot_move_it() {
        // Two starts of one stream overlap (a named start, then an Auto swap a
        // moment later). The older start's check must not restart the relay
        // the newer start is already serving, and a cold start moves nothing.
        let id = "test-overtaken-start";
        begin_ladder(id, ladder(), 0, true, Arc::new(Still));
        let older = generation(id);
        begin_ladder(id, ladder(), 0, true, Arc::new(Still));
        let newer = generation(id);
        assert_ne!(older, newer);
        // Any id but the solo player's starts as a tile, which never gets a
        // start pick: play the solo player's part without touching its session.
        sessions().get_mut(id).unwrap().gov.set_tile(false);
        let (b, s) = seg(8_200_000, 2e6);
        note_throughput(id, b, s);
        note_throughput(id, b, s);
        assert!(start_check(id, older).is_none(), "the older start moved the newer stream");
        assert_eq!(
            start_check(id, newer).map(|r| r.name),
            Some("360p30".to_string()),
            "the start it owns is picked"
        );
        end(id);
    }

    #[test]
    fn ad_segments_and_the_first_of_a_new_run_are_left_out() {
        let (b, s) = seg(8_200_000, 40e6);
        let ads = "test-ad-segments";
        settled_session(ads, Arc::new(Direct { ads: true }));
        note_player_segment(ads, b, s, Some(2.0), Some(0));
        note_player_segment(ads, b, s, Some(2.0), Some(0));
        assert_eq!(warm_count(ads), 0);
        end(ads);

        let runs = "test-discontinuity-runs";
        settled_session(runs, Arc::new(Direct { ads: false }));
        note_player_segment(runs, b, s, Some(2.0), Some(3));
        note_player_segment(runs, b, s, Some(2.0), Some(3));
        assert_eq!(warm_count(runs), 2);
        // A discontinuity: its first segment is dropped, the next counts.
        note_player_segment(runs, b, s, Some(2.0), Some(4));
        assert_eq!(warm_count(runs), 2);
        note_player_segment(runs, b, s, Some(2.0), Some(4));
        assert_eq!(warm_count(runs), 3);
        end(runs);
    }

    #[test]
    fn only_a_download_that_ran_alone_counts() {
        let id = "test-overlapping-transfers";
        // Alone: in and out with nothing else running.
        let a = begin_transfer(id);
        assert!(a.alone_at_start);
        let mut a = a;
        assert!(a.close());
        // Two overlapping: neither ran alone.
        let mut b = begin_transfer(id);
        let mut c = begin_transfer(id);
        assert!(!c.close(), "it began while another ran");
        assert!(!b.close(), "another began while it ran");
        // A dropped transfer leaves the count, so the next one is alone again.
        drop(begin_transfer(id));
        let mut d = begin_transfer(id);
        assert!(d.close());
    }

    #[test]
    fn newest_segments_reads_the_tail_of_the_playlist_with_lengths() {
        let pl = "#EXTM3U\n#EXT-X-MEDIA-SEQUENCE:9\n#EXTINF:2.000,live\na.ts\n#EXTINF:2.000,live\nb.ts\n\
                  #EXTINF:1.700,live\nc.ts\n#EXT-X-TWITCH-PREFETCH:https://cdn/d.ts\n";
        assert_eq!(
            newest_segments(pl, 2),
            vec![("b.ts".to_string(), Some(2.0)), ("c.ts".to_string(), Some(1.7))]
        );
        assert_eq!(newest_segments(pl, 9).len(), 3);
        assert!(newest_segments("#EXTM3U\n", 2).is_empty());
    }

    #[test]
    fn the_codec_family_is_the_video_entry_wherever_it_sits() {
        assert_eq!(codec_family(Some("avc1.64002A,mp4a.40.2")), "avc1");
        assert_eq!(codec_family(Some("mp4a.40.2,avc1.64002A")), "avc1");
        assert_eq!(codec_family(Some("mp4a.40.2, hvc1.1.2.L150")), "hvc1");
        assert_eq!(codec_family(Some("ec-3,av01.0.08M.08")), "av01");
        assert_eq!(codec_family(Some("mp4a.40.2")), "");
        assert_eq!(codec_family(None), "");
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
