//! Live playback on a link that changes under it (the phones away from home).
//!
//! The relay resolves one rendition and serves it for the whole session. On a
//! home connection that is right: the source rendition (8 to 9 Mbit/s at
//! 1080p60) arrives many times faster than real time. On a cellular or weak
//! Wi-Fi link it often cannot arrive in real time at all, and nothing ever
//! stepped down, so the player sat on a frozen frame for as long as the link
//! stayed bad. This module is the one place that decides, for the solo live
//! stream:
//!
//! - **which rung of the ladder to play**, from measured throughput (each whole
//!   segment the player or the origin downloads is a sample) and from stalls
//!   the player reports and tails the origin loses. Down quickly, up slowly,
//!   and each up that fails doubles the wait before the next one;
//! - **how much extra distance from the live edge** to keep: one second per
//!   stall, given back half a second at a time while playback stays clean, so a
//!   good link returns to the tight edge and a bad one stops stalling.
//!
//! Only "Auto" (`best`) sessions change rendition; a quality the viewer picked
//! by name stays put, and only the cushion moves. A link that could not hold a
//! rung is remembered for a while, so a restart, an audio-only screen-off swap
//! or the next channel starts at a rung the link can carry instead of freezing
//! on the source again.
//!
//! The decisions are pure (`Governor`, time passed in) and unit-tested; the
//! runtime half below applies them through the relay: a seamless switch of the
//! upstream media playlist on the whole-segment path, or an origin restart on
//! the parts path, announced to the player as `playback-adapt`.

use log::info;
use once_cell::sync::Lazy;
use serde::Serialize;
use std::sync::Mutex;
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

/// What the governor wants done.
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// Play rung `to` (an index into the ladder, 0 = best).
    Switch { to: usize, why: &'static str },
    /// Measure the link on rung `rung` before stepping up to it (the parts
    /// path streams in real time, so its own transfers never show headroom).
    Probe { rung: usize },
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

    fn settling(&self, now: f64) -> bool {
        now - self.last_switch < SETTLE
    }

    /// One whole transfer: `bytes` in `secs`.
    pub fn note_throughput(&mut self, bytes: u64, secs: f64, now: f64) -> Option<Decision> {
        if bytes < MIN_SAMPLE_BYTES || secs <= 0.0 {
            return None;
        }
        let bps = bytes as f64 * 8.0 / secs.max(0.001);
        let ewma = |prev: Option<f64>, half: f64| {
            let alpha = 1.0 - 0.5f64.powf(1.0 / half);
            Some(prev.map_or(bps, |p| p + (bps - p) * alpha))
        };
        self.fast = ewma(self.fast, FAST_HALF_LIFE);
        self.slow = ewma(self.slow, SLOW_HALF_LIFE);
        self.samples += 1;
        self.decide(now)
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

    /// The origin gave up on a segment's tail (slower than real time, or a
    /// dead transfer): the player will meet a hole.
    pub fn note_tail_loss(&mut self, now: f64) -> Option<Decision> {
        if self.settling(now) {
            return None;
        }
        self.add_trouble(now, CUSHION_PER_TAIL);
        if self.auto && self.recent_troubles(now) >= 2 {
            return self.step_down("lost segment tails");
        }
        None
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
    rungs.dedup_by(|a, b| a.name == b.name);
    let current = rungs.iter().position(|r| r.url == selected_url).unwrap_or(0);
    (rungs, current)
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

/// Adaptation runs on the phones. Desktop sits on wired or home links and its
/// player reports nothing, so it keeps exactly the behaviour it had.
pub const ENABLED: bool = cfg!(mobile);

/// Lets the shaped-network harness (`link_soak`) run adaptation on a desktop
/// test build.
#[cfg(test)]
static FORCED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(test)]
pub fn force_enabled(on: bool) {
    FORCED.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// Whether this build adapts (always on the phones).
pub fn enabled() -> bool {
    #[cfg(test)]
    if FORCED.load(std::sync::atomic::Ordering::Relaxed) {
        return true;
    }
    ENABLED
}

struct Session {
    gov: Governor,
    epoch: Instant,
    /// Bumped by every begin/end, so a switch applied after the session
    /// changed is dropped.
    generation: u64,
    applying: bool,
    last_cushion_sent: f64,
}

static SESSION: Lazy<Mutex<Option<Session>>> = Lazy::new(|| Mutex::new(None));
static GENERATION: Lazy<Mutex<u64>> = Lazy::new(|| Mutex::new(0));
/// The best rung bitrate the link could not hold, and when that was learned.
static REMEMBERED: Lazy<Mutex<Option<(Instant, u64)>>> = Lazy::new(|| Mutex::new(None));

/// What the players hear about: the rung now playing, the extra cushion, and
/// (on the parts path) the URL to reload on.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdaptEvent {
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

/// A live Twitch session is starting on `variants` with `selected_url`
/// chosen. Returns the URL to actually serve: the selected one, or a lower rung
/// when the link remembered from moments ago could not hold it.
pub fn begin(
    variants: &[crate::services::twitch_resolver::Variant],
    selected_url: &str,
    auto: bool,
) -> String {
    end();
    if !enabled() {
        return selected_url.to_string();
    }
    let (ladder, selected) = ladder_from(variants, selected_url);
    if ladder.is_empty() {
        return selected_url.to_string();
    }
    let start = if auto { start_rung(&ladder, selected, remembered()) } else { selected };
    let url = ladder[start].url.clone();
    if start != selected {
        info!(
            "[LinkAdapt] starting on {} instead of {}: the link held no more moments ago",
            ladder[start].name, ladder[selected].name
        );
    }
    let generation = {
        let mut g = GENERATION.lock().unwrap_or_else(|e| e.into_inner());
        *g += 1;
        *g
    };
    *SESSION.lock().unwrap_or_else(|e| e.into_inner()) = Some(Session {
        gov: Governor::new(ladder, start, auto, 0.0),
        epoch: Instant::now(),
        generation,
        applying: false,
        last_cushion_sent: 0.0,
    });
    tauri::async_runtime::spawn(ticker(generation));
    url
}

/// Looks every few seconds while the session lives: the parts path reports
/// nothing during clean playback, and that is exactly when the cushion should
/// shrink and a step up be tried.
async fn ticker(generation: u64) {
    loop {
        tokio::time::sleep(Duration::from_secs(5)).await;
        let live = SESSION
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .is_some_and(|s| s.generation == generation);
        if !live {
            return;
        }
        tick();
    }
}

/// The session ended (stop, a VOD, another platform). The link it learned
/// stays remembered for the next start.
pub fn end() {
    let old = SESSION.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(s) = old {
        remember(s.gov.ceiling());
    }
    *GENERATION.lock().unwrap_or_else(|e| e.into_inner()) += 1;
}

/// The rung playing now and the extra cushion, for logs and the harness.
pub fn snapshot() -> Option<(String, f64)> {
    let guard = SESSION.lock().unwrap_or_else(|e| e.into_inner());
    let s = guard.as_ref()?;
    Some((s.gov.ladder()[s.gov.current()].name.clone(), s.gov.cushion()))
}

/// The codec family of the rung playing now (`hvc1`, `av01`, `avc1`), so a
/// player that cannot decode it can say which one failed.
pub fn playing_family() -> Option<String> {
    let guard = SESSION.lock().unwrap_or_else(|e| e.into_inner());
    let s = guard.as_ref()?;
    Some(s.gov.ladder().get(s.gov.current())?.family.clone())
}

/// Extra seconds behind the live edge to keep (0 with no session).
pub fn extra_cushion() -> f64 {
    SESSION
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map_or(0.0, |s| s.gov.cushion())
}

fn with_session(f: impl FnOnce(&mut Governor, f64) -> Option<Decision>) {
    let (decision, generation, cushion_moved) = {
        let mut guard = SESSION.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = guard.as_mut() else { return };
        let now = s.epoch.elapsed().as_secs_f64();
        let d = if s.applying { None } else { f(&mut s.gov, now) };
        if d.is_some() {
            s.applying = true;
        }
        let moved = (s.gov.cushion() - s.last_cushion_sent).abs() >= 0.5;
        if moved {
            s.last_cushion_sent = s.gov.cushion();
        }
        (d, s.generation, moved)
    };
    match decision {
        Some(d) => {
            tauri::async_runtime::spawn(apply(d, generation));
        }
        None if cushion_moved => announce(None, "cushion"),
        None => {}
    }
}

/// A whole segment the player (or the origin) downloaded.
pub fn note_throughput(bytes: u64, secs: f64) {
    with_session(|g, now| g.note_throughput(bytes, secs, now));
}

/// A stall the player reported.
pub fn note_stall(secs: f64, ended: bool) {
    with_session(|g, now| g.note_stall(secs, ended, now));
}

/// The parts origin gave up on a segment's tail.
pub fn note_tail_loss() {
    with_session(|g, now| g.note_tail_loss(now));
}

/// Periodic look (the player's reports drive it; this catches clean stretches
/// with nothing to report).
pub fn tick() {
    with_session(|g, now| g.tick(now));
}

fn announce(url: Option<String>, reason: &str) {
    let event = {
        let guard = SESSION.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = guard.as_ref() else { return };
        let rung = &s.gov.ladder()[s.gov.current()];
        AdaptEvent {
            quality: rung.name.clone(),
            auto: s.gov.auto,
            extra_cushion: s.gov.cushion(),
            url,
            reason: reason.to_string(),
        }
    };
    crate::services::stream_server::emit_playback_adapt(&event);
}

async fn apply(decision: Decision, generation: u64) {
    match decision {
        Decision::Switch { to, why } => switch_to(to, why, generation).await,
        Decision::Probe { rung } => probe(rung, generation).await,
    }
    if let Some(s) = SESSION.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        if s.generation == generation {
            s.applying = false;
        }
    }
}

/// Rung `i` and the rung playing now, while `generation` is still the session.
fn rung_of(generation: u64, i: usize) -> Option<(Rung, Rung)> {
    let guard = SESSION.lock().unwrap_or_else(|e| e.into_inner());
    let s = guard.as_ref().filter(|s| s.generation == generation)?;
    let from = s.gov.ladder().get(s.gov.current())?.clone();
    Some((s.gov.ladder().get(i)?.clone(), from))
}

async fn switch_to(to: usize, why: &'static str, generation: u64) {
    let Some((rung, from)) = rung_of(generation, to) else { return };
    info!("[LinkAdapt] {} -> {} ({})", from.name, rung.name, why);
    let same_decoder = rung.family == from.family;
    let reload = match crate::services::stream_server::adapt_upstream(&rung.url, same_decoder).await {
        Ok(reload) => reload,
        Err(e) => {
            log::warn!("[LinkAdapt] switch to {} failed: {}", rung.name, e);
            return;
        }
    };
    {
        let mut guard = SESSION.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = guard.as_mut().filter(|s| s.generation == generation) else { return };
        let now = s.epoch.elapsed().as_secs_f64();
        s.gov.switched(to, now);
        if to > 0 || s.gov.ceiling().is_some() {
            remember(s.gov.ceiling());
        }
    }
    announce(reload, why);
}

/// Times one whole, already-published segment of rung `rung`'s playlist.
async fn probe(rung: usize, generation: u64) {
    let Some((target, _)) = rung_of(generation, rung) else { return };
    let measured = measure_segment(&target.url).await;
    let decision = {
        let mut guard = SESSION.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = guard.as_mut().filter(|s| s.generation == generation) else { return };
        let now = s.epoch.elapsed().as_secs_f64();
        match measured {
            Some((bytes, secs)) => {
                info!(
                    "[LinkAdapt] probe {}: {:.2} Mbit/s",
                    target.name,
                    bytes as f64 * 8.0 / secs / 1e6
                );
                s.gov.note_probe(rung, bytes, secs, now)
            }
            None => s.gov.note_probe(rung, 0, 0.0, now),
        }
    };
    if let Some(Decision::Switch { to, why }) = decision {
        switch_to(to, why, generation).await;
    }
}

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
    let seg = text
        .lines()
        .rev()
        .find(|l| !l.is_empty() && !l.starts_with('#'))?
        .to_string();
    let t0 = Instant::now();
    let bytes = client
        .get(&seg)
        .timeout(Duration::from_secs(10))
        .send()
        .await
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
    fn a_long_stall_steps_down_with_no_throughput_known() {
        let mut g = Governor::new(ladder(), 0, true, 0.0);
        assert_eq!(g.note_stall(1.0, false, 20.0), None);
        assert_eq!(
            g.note_stall(3.2, false, 22.0),
            Some(Decision::Switch { to: 1, why: "a long stall" })
        );
    }

    #[test]
    fn two_short_stalls_in_a_minute_step_down_and_grow_the_cushion() {
        let mut g = Governor::new(ladder(), 0, true, 0.0);
        assert_eq!(g.note_stall(0.8, true, 20.0), None);
        assert_eq!(g.cushion(), 1.0);
        assert_eq!(
            g.note_stall(0.9, true, 50.0),
            Some(Decision::Switch { to: 1, why: "repeated stalls" })
        );
        assert_eq!(g.cushion(), 2.0);
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
        assert_eq!(
            g.note_stall(5.0, false, 29.0),
            Some(Decision::Switch { to: 2, why: "a long stall" })
        );
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
            Some(Decision::Switch { to: 1, why: "lost segment tails" })
        );
    }

    #[test]
    fn the_bottom_rung_has_nowhere_to_go() {
        let mut g = Governor::new(ladder(), 4, true, 0.0);
        assert_eq!(g.note_stall(9.0, false, 20.0), None);
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
