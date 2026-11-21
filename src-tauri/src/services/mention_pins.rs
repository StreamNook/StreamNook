//! Pinned mentions at the top of the chat you are reading (SN-284).
//!
//! `mention_inbox` owns what a pin IS (the rule: kinds, runs, Most at once,
//! ended is final, the 24 h cap, the stamp). This module owns where a pin
//! shows and when its clock runs:
//!
//! - **Surfaces.** Each window reports its mounted stacks in one call
//!   (`set_pin_surfaces`): which chats each shows, whether it is on screen,
//!   and the ids of the pins actually visible. Rust keeps the latest report per
//!   window whatever the app is doing ("store the report, gate the effect"),
//!   clears a window's surfaces when its page starts loading, when the window
//!   is destroyed and when Pin mentions turns off.
//! - **Homes.** A pin has one home (window, surface), chosen once and sticky:
//!   a surface showing its chat, else the primary. It moves only when its
//!   surface has been gone or not shown for 3 s, or its window has been idle
//!   10 s while another window with a qualifying surface has newer input. A
//!   move never ends a pin.
//! - **The clock.** One 1 s tick, running only while a live pin exists and the
//!   app is in front. A pin counts when it is visible on its home, the app has
//!   been in front for 250 ms, and its home window had input within 60 s that
//!   came after the pin showed and after the app came to the front. Counting
//!   credits `min(now - previous tick, 2 s)`, so sleep and clock jumps neither
//!   end nor stretch a pin. Not counting holds the pin.
//! - **The answer.** `get_mention_pins` serves a cache this module rebuilds on
//!   every change; the revision (and the `mention-pins-changed` event, emitted
//!   after every lock is released) moves only when what a shell draws moves,
//!   never for the time left.
//!
//! Lock order: settings, performance_mode SENT, INPUTS, claims, inbox, then the
//! pins state here. Nothing holding the pins state calls into the inbox; the
//! inbox side may take the pins state (inbox, then pins). `reading_inputs_changed`
//! takes only the pins state.

use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::Instant;

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use tauri::Emitter;

use crate::services::mention_inbox::{self, Change, MentionEntry, PinCfg, PinCredit, PinRun};
use crate::services::mention_pin_row::{self, PinRow};

/// Emitted (no payload) after the pins revision moved: every shell asks again.
pub const PINS_EVENT: &str = "mention-pins-changed";

/// Milliseconds on this process's monotonic clock; 0 means "never".
pub type Mono = u64;

/// The most one tick credits a pin, whatever the gap since the last (sleep).
const CREDIT_CAP_MS: u64 = 2_000;
/// A window's input counts toward a pin's clock for this long.
const INPUT_WINDOW_MS: u64 = 60_000;
/// The app must have been in front this long before clocks count.
const FRONT_SETTLE_MS: u64 = 250;
/// A home whose surface is gone or not shown moves after this long.
const HOME_GRACE_MS: u64 = 3_000;
/// A window with no input this long yields homes to a window with newer input.
const WINDOW_IDLE_MS: u64 = 10_000;
/// Retired page ids remembered per window, to refuse their late calls.
const RETIRED_KEPT: usize = 8;

static T0: Lazy<Instant> = Lazy::new(Instant::now);

/// The monotonic clock the rules run on (never 0).
pub fn mono_now() -> Mono {
    T0.elapsed().as_millis() as u64 + 1
}

fn wall_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

// ---------------------------------------------------------------------------
// What a shell sends
// ---------------------------------------------------------------------------

/// One chat a surface shows, as the shell knows it. Rust builds the room key.
#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct RoomRef {
    pub provider: String,
    pub channel: String,
}

/// One mounted stack in one window (`set_pin_surfaces`).
#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct SurfaceIn {
    /// `main-chat`, `dock`, `pane:<leaf>`, `blended`, `phone-watch`, `phone-chats`...
    pub id: String,
    /// The mount's token (the shell's own bookkeeping; kept for logs).
    pub token: String,
    pub rooms: Vec<RoomRef>,
    pub active: bool,
    /// Drawn and not covered by the app's own UI.
    pub shown: bool,
    /// `stacked`, `landscape`, `vertical`, `over-video` on the phone watch screen.
    pub mode: String,
    pub strips_only: bool,
    pub over_video: bool,
    /// The platforms a tap on a pin here can open.
    pub opens: Vec<String>,
    /// Every pin id on screen now; empty whenever not shown.
    pub visible: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Surface {
    pub id: String,
    pub token: String,
    /// Room keys, lowercased.
    pub rooms: Vec<String>,
    pub active: bool,
    pub shown: bool,
    pub mode: String,
    pub strips_only: bool,
    pub over_video: bool,
    pub opens: Vec<String>,
    pub visible: Vec<String>,
}

impl Surface {
    fn from_in(s: SurfaceIn) -> Self {
        Surface {
            id: s.id,
            token: s.token,
            rooms: s.rooms.iter().map(|r| mention_inbox::room_key(&r.provider, &r.channel).to_lowercase()).collect(),
            active: s.active,
            // Nothing is visible on a surface that is not shown.
            visible: if s.shown { s.visible } else { Vec::new() },
            shown: s.shown,
            mode: s.mode,
            strips_only: s.strips_only,
            over_video: s.over_video,
            opens: s.opens.iter().map(|o| o.to_lowercase()).collect(),
        }
    }

    fn shows(&self, room: &str) -> bool {
        self.rooms.iter().any(|r| r == room)
    }
}

// ---------------------------------------------------------------------------
// The state
// ---------------------------------------------------------------------------

#[derive(Default, Debug)]
struct WindowState {
    /// The page whose calls are applied; None after its page started loading.
    page: Option<String>,
    seq: u64,
    retired: VecDeque<String>,
    surfaces: Vec<Surface>,
}

impl WindowState {
    fn retire(&mut self, page: &str) {
        if !self.retired.iter().any(|p| p == page) {
            self.retired.push_back(page.to_string());
            while self.retired.len() > RETIRED_KEPT {
                self.retired.pop_front();
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Home {
    window: String,
    surface: String,
    missing_since: Option<Mono>,
    unshown_since: Option<Mono>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Clock {
    /// The pin's time is counting now.
    Running,
    /// Timed, and not counting (not visible, or the viewer is not using StreamNook).
    Held,
    /// An until-closed pin has no clock.
    None,
}

impl Clock {
    fn word(self) -> &'static str {
        match self {
            Clock::Running => "running",
            Clock::Held => "held",
            Clock::None => "none",
        }
    }
}

/// One pin as the answer cache holds it.
#[derive(Clone, Debug)]
struct AnswerPin {
    run: PinRun,
    row: PinRow,
    home: Option<(String, String)>,
    clock: Clock,
}

#[derive(Default, Debug)]
pub struct PinState {
    windows: HashMap<String, WindowState>,
    /// The last `note_viewer_input` per window, 0 = never.
    window_input: HashMap<String, Mono>,
    homes: HashMap<String, Home>,
    /// When a pin entered its home's `visible`.
    visible_since: HashMap<String, Mono>,
    clock: HashMap<String, Clock>,
    /// The app is in front (desktop: Performance mode's verdict; phones: reported
    /// interactive and not idle), and since when.
    front: bool,
    front_since: Mono,
    prev_tick: Option<Mono>,
    revision: u64,
    signature: u64,
    answer: Vec<AnswerPin>,
    max: usize,
}

impl PinState {
    // ---- surfaces ---------------------------------------------------------

    /// Apply a window's report when it is newer than the last applied for the
    /// window (by page, then sequence). Answers whether it was applied.
    pub fn set_surfaces(&mut self, label: &str, page: &str, seq: u64, surfaces: Vec<SurfaceIn>) -> bool {
        let w = self.windows.entry(label.to_string()).or_default();
        if w.retired.iter().any(|p| p == page) {
            return false;
        }
        match w.page.clone() {
            Some(current) if current == page => {
                if seq <= w.seq {
                    return false;
                }
            }
            Some(current) => w.retire(&current),
            None => {}
        }
        w.page = Some(page.to_string());
        w.seq = seq;
        let mut list: Vec<Surface> = Vec::new();
        for s in surfaces {
            // One entry per id; the last report of an id wins.
            list.retain(|x| x.id != s.id);
            list.push(Surface::from_in(s));
        }
        w.surfaces = list;
        true
    }

    /// The window started loading a new page: its page retires, its surfaces go.
    pub fn page_started(&mut self, label: &str) -> bool {
        match self.windows.get_mut(label) {
            Some(w) => {
                if let Some(p) = w.page.take() {
                    w.retire(&p);
                }
                let had = !w.surfaces.is_empty();
                w.surfaces.clear();
                had
            }
            None => false,
        }
    }

    /// The window was destroyed.
    pub fn forget_window(&mut self, label: &str) -> bool {
        let had = self.windows.get(label).is_some_and(|w| !w.surfaces.is_empty());
        self.page_started(label);
        self.window_input.remove(label);
        had
    }

    /// Pin mentions turned off: every surface, home and clock goes.
    pub fn clear_all(&mut self) {
        for w in self.windows.values_mut() {
            if let Some(p) = w.page.take() {
                w.retire(&p);
            }
            w.surfaces.clear();
        }
        self.homes.clear();
        self.visible_since.clear();
        self.clock.clear();
        self.answer.clear();
        self.prev_tick = None;
    }

    pub fn note_input(&mut self, label: &str, now: Mono) {
        self.window_input.insert(label.to_string(), now);
    }

    /// Whether the app is in front changed. Leaving stops every clock at once
    /// (the next tick restarts from nothing); returning starts the 250 ms.
    pub fn sync_front(&mut self, front: bool, now: Mono) -> bool {
        if self.front == front {
            return false;
        }
        self.front = front;
        if front {
            self.front_since = now;
        } else {
            self.prev_tick = None;
            for c in self.clock.values_mut() {
                if *c == Clock::Running {
                    *c = Clock::Held;
                }
            }
        }
        true
    }

    fn surface(&self, window: &str, id: &str) -> Option<&Surface> {
        self.windows.get(window)?.surfaces.iter().find(|s| s.id == id)
    }

    fn input_of(&self, label: &str) -> Mono {
        self.window_input.get(label).copied().unwrap_or(0)
    }

    // ---- homes ------------------------------------------------------------

    /// The best home for a pin of `room` right now: a shown surface showing the
    /// room, else the primary. Ties: the window with the most recent input,
    /// the active pane, `main-chat`, `dock`, the phones' watch before their
    /// chats, then the surface id.
    fn pick(&self, room: &str, platform: &str) -> Option<(String, String)> {
        let shown: Vec<(&String, &Surface)> =
            self.windows.iter().flat_map(|(w, ws)| ws.surfaces.iter().map(move |s| (w, s))).filter(|(_, s)| s.shown).collect();
        let showing: Vec<&(&String, &Surface)> = shown.iter().filter(|(_, s)| s.shows(room)).collect();
        let pool: Vec<&(&String, &Surface)> = if showing.is_empty() {
            shown.iter().filter(|(_, s)| primary_ok(s, platform)).collect()
        } else {
            showing
        };
        pool.into_iter()
            .min_by_key(|(w, s)| (std::cmp::Reverse(self.input_of(w)), !s.active, id_rank(&s.id), s.id.clone(), (*w).clone()))
            .map(|(w, s)| ((*w).clone(), s.id.clone()))
    }

    fn assign_home(&mut self, run: &PinRun, now: Mono) {
        let room = run.entry.room.to_lowercase();
        let platform = platform_of(&room);
        let Some(current) = self.homes.get(&run.id).cloned() else {
            if let Some((window, surface)) = self.pick(&room, &platform) {
                self.homes.insert(run.id.clone(), Home { window, surface, missing_since: None, unshown_since: None });
            }
            return;
        };
        let mut home = current.clone();
        let mut move_to: Option<(String, String)> = None;
        match self.surface(&home.window, &home.surface) {
            None => {
                let since = *home.missing_since.get_or_insert(now);
                if now.saturating_sub(since) >= HOME_GRACE_MS {
                    move_to = self.pick(&room, &platform);
                }
            }
            Some(s) => {
                home.missing_since = None;
                if s.shown {
                    home.unshown_since = None;
                } else {
                    let since = *home.unshown_since.get_or_insert(now);
                    if now.saturating_sub(since) >= HOME_GRACE_MS {
                        move_to = self.pick(&room, &platform).filter(|p| *p != (home.window.clone(), home.surface.clone()));
                    }
                }
            }
        }
        // The viewer went to another window: the home follows once its own
        // window has been idle and the other has newer input.
        if move_to.is_none() {
            let own = self.input_of(&home.window);
            let idle = own == 0 || now.saturating_sub(own) >= WINDOW_IDLE_MS;
            if idle {
                if let Some(p) = self.pick(&room, &platform) {
                    if p.0 != home.window && self.input_of(&p.0) > own {
                        move_to = Some(p);
                    }
                }
            }
        }
        match move_to {
            Some((window, surface)) => {
                self.visible_since.remove(&run.id);
                self.homes.insert(run.id.clone(), Home { window, surface, missing_since: None, unshown_since: None });
            }
            None => {
                self.homes.insert(run.id.clone(), home);
            }
        }
    }

    fn ensure_homes(&mut self, runs: &[PinRun], now: Mono) {
        let live: std::collections::HashSet<&str> = runs.iter().map(|r| r.id.as_str()).collect();
        self.homes.retain(|id, _| live.contains(id.as_str()));
        self.visible_since.retain(|id, _| live.contains(id.as_str()));
        for run in runs {
            self.assign_home(run, now);
        }
    }

    fn reconcile_visible(&mut self, runs: &[PinRun], now: Mono) {
        for run in runs {
            let visible = self
                .homes
                .get(&run.id)
                .and_then(|h| self.surface(&h.window, &h.surface))
                .is_some_and(|s| s.shown && s.visible.iter().any(|v| *v == run.id));
            if visible {
                self.visible_since.entry(run.id.clone()).or_insert(now);
            } else {
                self.visible_since.remove(&run.id);
            }
        }
    }

    // ---- the clock --------------------------------------------------------

    /// Whether a pin's clock counts at `now`.
    fn counts(&self, id: &str, now: Mono) -> bool {
        if !self.front || now.saturating_sub(self.front_since) < FRONT_SETTLE_MS {
            return false;
        }
        let Some(home) = self.homes.get(id) else { return false };
        let Some(surface) = self.surface(&home.window, &home.surface) else { return false };
        if !(surface.shown && surface.visible.iter().any(|v| v == id)) {
            return false;
        }
        let Some(since) = self.visible_since.get(id).copied() else { return false };
        let input = self.input_of(&home.window);
        input != 0 && now.saturating_sub(input) <= INPUT_WINDOW_MS && input > since && input > self.front_since
    }

    /// One pass over the live pins: homes, what is visible, and (when
    /// `credit`) what each pin earned since the last tick.
    pub fn step(&mut self, runs: &[PinRun], cfg: &PinCfg, now: Mono, credit: bool) -> Vec<PinCredit> {
        self.ensure_homes(runs, now);
        self.reconcile_visible(runs, now);
        let credit_ms = if credit {
            self.prev_tick.map(|p| now.saturating_sub(p).min(CREDIT_CAP_MS)).unwrap_or(0) as i64
        } else {
            0
        };
        let mut out = Vec::new();
        self.clock.clear();
        for run in runs {
            let counts = self.counts(&run.id, now);
            if credit {
                out.push(PinCredit { id: run.id.clone(), counts, credit_ms });
            }
            let clock = if cfg.expire_ms.is_none() {
                Clock::None
            } else if counts {
                Clock::Running
            } else {
                Clock::Held
            };
            self.clock.insert(run.id.clone(), clock);
        }
        if credit {
            self.prev_tick = Some(now);
        }
        out
    }

    // ---- the answer -------------------------------------------------------

    /// Rebuild the answer cache and move the revision when what a shell draws
    /// moved. Answers whether the revision moved.
    fn publish(&mut self, runs: &[PinRun], rows: Vec<Option<PinRow>>, cfg: &PinCfg, today: i32) -> bool {
        self.max = cfg.max;
        self.answer = runs
            .iter()
            .zip(rows)
            .filter_map(|(run, row)| {
                Some(AnswerPin {
                    run: run.clone(),
                    row: row?,
                    home: self.homes.get(&run.id).map(|h| (h.window.clone(), h.surface.clone())),
                    clock: self.clock.get(&run.id).copied().unwrap_or(if cfg.expire_ms.is_none() { Clock::None } else { Clock::Held }),
                })
            })
            .collect();
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (cfg.max, cfg.expire_ms.is_none(), today).hash(&mut h);
        for p in &self.answer {
            (&p.run.id, &p.home, p.clock, p.row.rev, p.run.ids.len()).hash(&mut h);
        }
        let signature = h.finish();
        if signature != self.signature {
            self.signature = signature;
            self.revision += 1;
            return true;
        }
        false
    }

    fn bump(&mut self) {
        self.revision += 1;
    }
}

/// The platform a room key names: `kick:slug` is Kick, a bare login Twitch.
fn platform_of(room: &str) -> String {
    room.split_once(':').map(|(p, _)| p).unwrap_or("twitch").to_string()
}

fn id_rank(id: &str) -> u8 {
    match id {
        "main-chat" => 0,
        "dock" => 1,
        "phone-watch" => 2,
        "phone-chats" => 3,
        _ => 4,
    }
}

/// A surface that may take a pin whose chat no surface shows. The lower third
/// and over-video are never primary; a phone surface only for a platform it
/// opens, except YouTube and TikTok pins, which may home there and open the
/// Mentions sheet at the entry.
fn primary_ok(s: &Surface, platform: &str) -> bool {
    if s.over_video || s.strips_only {
        return false;
    }
    if s.id.starts_with("phone-") {
        return s.opens.iter().any(|o| o == platform) || matches!(platform, "youtube" | "tiktok");
    }
    true
}

static STATE: Lazy<Mutex<PinState>> = Lazy::new(|| Mutex::new(PinState::default()));

fn state() -> MutexGuard<'static, PinState> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

fn with_state<R>(f: impl FnOnce(&mut PinState) -> R) -> R {
    f(&mut state())
}

static PINS_ON: AtomicBool = AtomicBool::new(false);
static LIVE_PINS: AtomicUsize = AtomicUsize::new(0);
static TICKING: AtomicBool = AtomicBool::new(false);
/// The phones' own "interactive reported" flag: false until the first report
/// (whisper_claims' default stays as it was).
static PHONE_REPORTED: AtomicBool = AtomicBool::new(false);
static PHONE_INTERACTIVE: AtomicBool = AtomicBool::new(false);

/// Whether Pin mentions is on (a lock-free read for pollers).
pub fn pins_on() -> bool {
    PINS_ON.load(Ordering::Relaxed)
}

/// The phone shell reported the app interactive (or not): in front, not in
/// picture-in-picture. Followed by `reading_inputs_changed`.
pub fn note_interactive(interactive: bool) {
    PHONE_INTERACTIVE.store(interactive, Ordering::SeqCst);
    PHONE_REPORTED.store(true, Ordering::SeqCst);
}

/// The phone's front rule, pure: reported, interactive and not idle.
pub fn phone_front(reported: bool, interactive: bool, idle: bool) -> bool {
    reported && interactive && !idle
}

/// Whether StreamNook is in front right now. Desktop: Performance mode's
/// verdict (no verdict yet counts as away). Phones: the pins' own report.
fn app_front_now() -> bool {
    #[cfg(desktop)]
    {
        crate::services::performance_mode::in_front() == Some(true)
    }
    #[cfg(not(desktop))]
    {
        phone_front(
            PHONE_REPORTED.load(Ordering::SeqCst),
            PHONE_INTERACTIVE.load(Ordering::SeqCst),
            crate::services::window_visibility::phone_idle(),
        )
    }
}

// ---------------------------------------------------------------------------
// The answer a shell asks for
// ---------------------------------------------------------------------------

/// Where a pin's chat is, when it is not the surface's own chat.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ChannelMark {
    pub room: String,
    pub platform: String,
    pub channel: String,
}

/// One pin as a shell draws it.
#[derive(Serialize, Clone, Debug)]
pub struct PinOut {
    pub entry: MentionEntry,
    /// The `ChatMessage`-shaped row (`services::mention_pin_row`).
    pub row: serde_json::Value,
    pub channel_mark: Option<ChannelMark>,
    /// `running`, `held` or `none` (until closed).
    pub clock: &'static str,
    /// The use time left; null for an until-closed pin.
    pub left_ms: Option<i64>,
    /// How many copies of the same message this pin stands for.
    pub run: u32,
    /// The row's hash: the shell redraws when it changes.
    pub rev: u64,
    /// The row shows the day: it was not filed today.
    pub day: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct MentionPins {
    pub revision: u64,
    /// Oldest first.
    pub pins: Vec<PinOut>,
    /// Most at once.
    pub max: u32,
}

fn day_number(ms: i64) -> i32 {
    use chrono::{Datelike, TimeZone};
    chrono::Local
        .timestamp_millis_opt(ms)
        .single()
        .map(|d| d.date_naive().num_days_from_ce())
        .unwrap_or(0)
}

/// The pins for one surface of one window. Never writes and never emits.
/// `page` must be the window's current page, else the answer is empty (a
/// retired page's late call).
pub fn answer_for(label: &str, page: Option<&str>, surface: Option<&str>) -> MentionPins {
    answer_at(label, page, surface, wall_ms())
}

fn answer_at(label: &str, page: Option<&str>, surface: Option<&str>, now_wall: i64) -> MentionPins {
    let today = day_number(now_wall);
    with_state(|st| {
        let page_ok = match (page, st.windows.get(label)) {
            (Some(p), Some(w)) => w.page.as_deref() == Some(p),
            (Some(_), None) => false,
            (None, _) => true,
        };
        let pins = match (page_ok, surface) {
            (true, Some(surface)) => st.pins_for(label, surface, today),
            _ => Vec::new(),
        };
        MentionPins { revision: st.revision, pins, max: st.max.max(1) as u32 }
    })
}

impl PinState {
    fn pins_for(&self, label: &str, surface: &str, today: i32) -> Vec<PinOut> {
        let rooms: Vec<String> = self.surface(label, surface).map(|s| s.rooms.clone()).unwrap_or_default();
        self.answer
            .iter()
            .filter(|p| p.home.as_ref().is_some_and(|(w, s)| w == label && s == surface))
            .map(|p| {
                let room = p.run.entry.room.to_lowercase();
                let entry = &p.run.entry;
                PinOut {
                    entry: entry.clone(),
                    row: p.row.row.clone(),
                    channel_mark: (!rooms.contains(&room)).then(|| ChannelMark {
                        room: entry.room.clone(),
                        platform: entry.platform.clone(),
                        channel: entry.channel_login.clone(),
                    }),
                    clock: p.clock.word(),
                    left_ms: p.run.left_ms,
                    run: p.run.ids.len() as u32,
                    rev: p.row.rev,
                    day: day_number(p.run.from_ms) != today,
                }
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Passes: sweep, homes, clocks, answer
// ---------------------------------------------------------------------------

/// One pass over an inbox and the pin state: what time and rules end, Most at
/// once, homes, and (when `credit`) the clock step applied. Pure over its
/// arguments, so the clock is tested with injected ticks. Answers the live
/// pins after the pass and how much the inbox changed: a pin that ended is
/// saved soon, plain clock time only at the next stop event (neither is
/// announced as an inbox change).
pub fn pass_core(
    inbox: &mut mention_inbox::Inbox,
    st: &mut PinState,
    cfg: &PinCfg,
    wall: i64,
    now: Mono,
    credit: bool,
) -> (Vec<PinRun>, Change) {
    let mut dirty = inbox.pin_sweep(cfg, wall);
    dirty |= inbox.pin_enforce_max(cfg, wall);
    let runs = inbox.pin_live_runs(cfg, wall);
    let credits = st.step(&runs, cfg, now, credit);
    let (changed, ended) = inbox.pin_apply_credits(cfg, &credits);
    dirty |= !ended.is_empty();
    let level = if dirty {
        Change::Dirty
    } else if changed {
        Change::Clock
    } else {
        Change::None
    };
    (inbox.pin_live_runs(cfg, wall), level)
}

/// Run a pass against the live inbox and publish the answer.
fn run_pass(now: Mono, credit: bool) {
    let cfg = mention_inbox::pin_cfg();
    if !cfg.on {
        return;
    }
    let wall = wall_ms();
    let Some(runs) = mention_inbox::with_pin_inbox(|inbox| {
        // The front verdict is read inside the pass, so a clock never
        // credits time the app was not in front.
        with_state(|st| {
            st.sync_front(app_front_now(), now);
            pass_core(inbox, st, &cfg, wall, now, credit)
        })
    }) else {
        return;
    };
    // Every second, off the tick's own work: rebuild what late data changed.
    let rows_changed = credit && mention_pin_row::refresh_rows(&runs);
    finish(&runs, &cfg, rows_changed);
}

/// Publish what a pass found: build the rows (outside every lock), then the
/// answer and revision under the pins state, then the event after unlock.
fn finish(runs: &[PinRun], cfg: &PinCfg, force_bump: bool) {
    mention_pin_row::retain_live(runs);
    let rows: Vec<Option<PinRow>> = runs.iter().map(|r| mention_pin_row::cached(&r.entry)).collect();
    let today = day_number(wall_ms());
    let moved = with_state(|st| {
        let moved = st.publish(runs, rows, cfg, today);
        if force_bump && !moved {
            st.bump();
            return true;
        }
        moved
    });
    LIVE_PINS.store(runs.len(), Ordering::SeqCst);
    if let Ok(mut rooms) = LIVE_ROOMS.write() {
        *rooms = runs.iter().map(|r| r.entry.room.to_lowercase()).collect();
    }
    if moved {
        emit_changed();
    }
    ensure_tick();
}

fn emit_changed() {
    if let Some(app) = crate::services::providers::app_handle() {
        let _ = app.emit(PINS_EVENT, ());
    }
}

/// Run a pass on the blocking pool (never on a chat lane, the main thread or
/// an async command's thread: the inbox may load from disk).
fn schedule_pass(credit: bool) {
    if !pins_on() {
        return;
    }
    tauri::async_runtime::spawn_blocking(move || run_pass(mono_now(), credit));
}

fn should_tick() -> bool {
    pins_on() && LIVE_PINS.load(Ordering::SeqCst) > 0 && app_front_now()
}

/// Start the 1 s tick when it should run and is not. It stops by itself when
/// no live pin remains or the app leaves the front.
fn ensure_tick() {
    if !should_tick() || TICKING.swap(true, Ordering::SeqCst) {
        return;
    }
    tauri::async_runtime::spawn(async {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            if !should_tick() {
                TICKING.store(false, Ordering::SeqCst);
                with_state(|st| st.prev_tick = None);
                // The clocks stopped: save what they earned.
                flush_clock();
                // A change landed between the check and the store: keep going.
                if should_tick() && !TICKING.swap(true, Ordering::SeqCst) {
                    continue;
                }
                break;
            }
            let _ = tokio::task::spawn_blocking(|| run_pass(mono_now(), true)).await;
        }
    });
}

// ---------------------------------------------------------------------------
// What the rest of the core calls
// ---------------------------------------------------------------------------

/// Every input of the counting rule can change here (Performance mode's front
/// verdict, `app_presence::set_foreground` both ways, `set_app_interactive`):
/// takes only the pins state, never an inbox, so it is safe from any thread
/// and from under the settings save hook. Going away stops every clock at
/// once; coming back arms the 250 ms and the tick.
pub fn reading_inputs_changed() {
    if !pins_on() {
        return;
    }
    let front = app_front_now();
    let changed = with_state(|st| st.sync_front(front, mono_now()));
    if changed && !front {
        // A stop event: the clocks are stopped above, and what they earned is saved.
        flush_clock();
    }
    ensure_tick();
}

/// Save the pin clock state now, on the blocking pool (`flush_now` folds and writes).
fn flush_clock() {
    if LIVE_PINS.load(Ordering::SeqCst) > 0 {
        tauri::async_runtime::spawn_blocking(|| {
            let _ = mention_inbox::flush_now();
        });
    }
}

/// Rooms holding a live pin, lowercased: the hot chat paths (a timeout, a
/// `/clear`) consult this before taking the inbox lock.
static LIVE_ROOMS: Lazy<std::sync::RwLock<std::collections::HashSet<String>>> =
    Lazy::new(|| std::sync::RwLock::new(std::collections::HashSet::new()));

/// Whether `room` (a Rust room key) holds a live pin right now. Lock-free of
/// the inbox: a CLEARCHAT or `/clear` for any other room costs nothing.
pub fn room_has_live_pin(room: &str) -> bool {
    pins_on() && LIVE_ROOMS.read().map(|r| r.contains(&room.to_lowercase())).unwrap_or(false)
}

/// Time earned since the last tick is folded into the entries before every
/// save (`mention_inbox::flush_now`). A no-op unless a clock is running.
pub fn fold_for_flush() {
    if pins_on() && TICKING.load(Ordering::SeqCst) && LIVE_PINS.load(Ordering::SeqCst) > 0 {
        run_pass(mono_now(), true);
    }
}

/// Whether any window reports a pin surface drawn and not covered: the row
/// owner rebuilds rows only while one is.
pub fn any_surface_shown() -> bool {
    pins_on() && with_state(|st| st.windows.values().any(|w| w.surfaces.iter().any(|s| s.shown)))
}

/// A pin's row was built or changed (no lock held): publish the answer again.
pub fn rows_changed() {
    schedule_pass(false);
}

/// A live pin was just filed (inbox lock released): publish it.
pub fn on_filed() {
    schedule_pass(false);
}

/// A pin ended, was dismissed or removed (inbox lock released).
pub fn on_pins_ended() {
    schedule_pass(false);
}

/// An inbox just loaded: pins it holds are published.
pub fn inbox_loaded() {
    schedule_pass(false);
}

/// The surface a newly filed pin homes on (called inside the inbox lock; takes
/// only the pins state). None when Pin mentions is off or no surface can take it.
pub fn home_for_new_pin(entry: &MentionEntry, runs: &[PinRun]) -> Option<String> {
    if !pins_on() {
        return None;
    }
    let run = runs.iter().find(|r| r.ids.iter().any(|i| *i == entry.id))?;
    with_state(|st| {
        st.assign_home(run, mono_now());
        st.homes.get(&run.id).map(|h| h.surface.clone())
    })
}

/// The settings that matter to pins changed (`mention_inbox::refresh`).
pub fn config_changed(old: &PinCfg, new: &PinCfg, look_changed: bool) {
    PINS_ON.store(new.on, Ordering::SeqCst);
    if !new.on {
        with_state(|st| {
            st.clear_all();
            if old.on {
                st.bump();
            }
        });
        LIVE_PINS.store(0, Ordering::SeqCst);
        if let Ok(mut rooms) = LIVE_ROOMS.write() {
            rooms.clear();
        }
        if old.on {
            emit_changed();
        }
        return;
    }
    let front = app_front_now();
    with_state(|st| {
        st.sync_front(front, mono_now());
        if look_changed {
            st.bump();
        }
    });
    if look_changed {
        emit_changed();
    }
    // The use time, the maximum, the stamp or a muted chat changed: the
    // sweep and Most at once run now, and the answer follows.
    schedule_pass(false);
}

/// A window reported its stacks. Applied when newer than the last report;
/// answers whether it was.
pub fn set_surfaces(label: &str, page: &str, seq: u64, surfaces: Vec<SurfaceIn>) -> bool {
    if !pins_on() {
        // Off: nothing is kept (the page's start call is sent only while on).
        with_state(|st| st.clear_all());
        return false;
    }
    let applied = with_state(|st| st.set_surfaces(label, page, seq, surfaces));
    if applied {
        schedule_pass(false);
    }
    applied
}

/// The window's page started loading: its stacks are gone with it.
pub fn window_page_started(label: &str) {
    if with_state(|st| st.page_started(label)) {
        schedule_pass(false);
    }
}

/// The window was destroyed.
pub fn forget_window(label: &str) {
    if with_state(|st| st.forget_window(label)) {
        schedule_pass(false);
    }
}

/// The viewer used `label` (pointer, key, wheel, touch, composer): stamped on
/// the monotonic clock at receipt. `window_front` is whether that window is
/// the OS foreground one (a covered window's stray pointer never counts).
pub fn note_viewer_input(label: &str, window_front: bool) {
    if !pins_on() || !window_front {
        return;
    }
    with_state(|st| st.note_input(label, mono_now()));
    if LIVE_PINS.load(Ordering::SeqCst) > 0 {
        schedule_pass(false);
    }
    ensure_tick();
}

/// Whether `window` is the OS foreground window, for desktop input. The emote
/// palette's input counts for its host window.
#[cfg(desktop)]
pub fn window_has_os_focus(window: &crate::rt::Window) -> bool {
    #[cfg(windows)]
    {
        let front = crate::services::performance_mode::front_hwnd();
        front != 0 && window.hwnd().map(|h| h.0 as isize).is_ok_and(|h| h == front)
    }
    #[cfg(not(windows))]
    {
        crate::services::whisper_claims::window_focused(window.label())
    }
}

/// Phones: every window is the app.
#[cfg(not(desktop))]
pub fn window_has_os_focus(_window: &crate::rt::Window) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::mention_inbox::{Inbox, MentionKind};

    const NOW_WALL: i64 = 1_800_000_000_000;
    /// A made-up point on the monotonic clock.
    const T: Mono = 1_000_000;

    fn cfg() -> PinCfg {
        PinCfg { on: true, expire_ms: Some(60_000), max: 10, enabled_at_ms: Some(NOW_WALL - 3_600_000), muted: Default::default() }
    }

    fn pin_entry(id: &str, room: &str, sender: &str, from: i64) -> MentionEntry {
        MentionEntry {
            id: id.into(),
            platform: room.split_once(':').map(|(p, _)| p).unwrap_or("twitch").into(),
            room: room.into(),
            channel_login: room.split_once(':').map(|(_, c)| c).unwrap_or(room).into(),
            open_key: room.into(),
            watch_url: String::new(),
            channel_id: Some("1000".into()),
            sender_id: sender.into(),
            sender_login: format!("user{sender}"),
            sender_name: format!("User{sender}"),
            sender_color: Some("#ff0000".into()),
            text: format!("hi @brandon {id}"),
            kind: MentionKind::Mention,
            reply_parent_id: None,
            reply_parent_text: None,
            at_ms: from - 7_000,
            read: false,
            removed: false,
            parts: Vec::new(),
            seen_live: false,
            pin_from_ms: Some(from),
            pin_dismissed: false,
            pin_ended: false,
            pin_left_ms: None,
            pin_seen: false,
            raw_color: Some("#ff0000".into()),
        }
    }

    fn surface(id: &str, rooms: &[&str], shown: bool, visible: &[&str]) -> SurfaceIn {
        SurfaceIn {
            id: id.into(),
            token: format!("tok-{id}"),
            rooms: rooms.iter().map(|r| RoomRef { provider: "twitch".into(), channel: (*r).into() }).collect(),
            shown,
            visible: visible.iter().map(|v| v.to_string()).collect(),
            opens: vec!["twitch".into(), "kick".into()],
            ..Default::default()
        }
    }

    /// An inbox, a pin state and the rule's settings, stepped with made-up ticks.
    struct Rig {
        inbox: Inbox,
        st: PinState,
        cfg: PinCfg,
        seq: u64,
    }

    impl Rig {
        fn new(entries: Vec<MentionEntry>) -> Self {
            let mut inbox = Inbox::default();
            for e in entries {
                inbox.file(e, NOW_WALL);
            }
            Rig { inbox, st: PinState::default(), cfg: cfg(), seq: 0 }
        }

        fn surfaces(&mut self, label: &str, list: Vec<SurfaceIn>) {
            self.seq += 1;
            assert!(self.st.set_surfaces(label, "page-1", self.seq, list));
        }

        fn pass(&mut self, now: Mono, credit: bool) -> Vec<PinRun> {
            self.st.sync_front(self.st.front, now);
            pass_core(&mut self.inbox, &mut self.st, &self.cfg, NOW_WALL, now, credit).0
        }

        fn tick(&mut self, now: Mono) -> Vec<PinRun> {
            self.pass(now, true)
        }

        fn left(&self, id: &str) -> Option<i64> {
            self.inbox.get(id).and_then(|e| e.pin_left_ms)
        }

        fn seen(&self, id: &str) -> bool {
            self.inbox.get(id).is_some_and(|e| e.pin_seen)
        }

        fn ended(&self, id: &str) -> bool {
            self.inbox.get(id).is_some_and(|e| e.pin_ended)
        }
    }

    /// One pin `a` in room xqc, shown and visible on `main-chat`, the app in
    /// front since `T`, with input at `T + 500`: counting from the first tick.
    fn counting_rig() -> Rig {
        let mut rig = Rig::new(vec![pin_entry("a", "xqc", "1", NOW_WALL - 1_000)]);
        rig.st.sync_front(true, T);
        rig.surfaces("main", vec![surface("main-chat", &["xqc"], true, &["a"])]);
        rig.pass(T, false);
        rig.st.note_input("main", T + 500);
        rig
    }

    // ---- the clock ----------------------------------------------------------

    #[test]
    fn the_clock_counts_with_the_pin_visible_the_app_in_front_and_input_after_it_showed() {
        let mut rig = counting_rig();
        rig.tick(T + 1_000);
        assert!(rig.seen("a"), "it would have counted: seen");
        assert_eq!(rig.left("a"), Some(60_000), "the first tick has no previous tick to credit from");
        rig.tick(T + 2_000);
        assert_eq!(rig.left("a"), Some(59_000));
        rig.tick(T + 3_000);
        assert_eq!(rig.left("a"), Some(58_000));
        let runs = rig.pass(T + 3_000, false);
        assert_eq!(rig.st.clock.get(&runs[0].id), Some(&Clock::Running));
    }

    #[test]
    fn a_pin_off_screen_or_without_fresh_input_or_with_the_app_away_is_held() {
        // Not visible on its home.
        let mut rig = counting_rig();
        rig.surfaces("main", vec![surface("main-chat", &["xqc"], true, &[])]);
        rig.tick(T + 1_000);
        rig.tick(T + 2_000);
        assert!(!rig.seen("a") && rig.left("a").is_none());
        // Visible again, but nothing touched StreamNook since it showed.
        rig.surfaces("main", vec![surface("main-chat", &["xqc"], true, &["a"])]);
        rig.tick(T + 3_000);
        rig.tick(T + 4_000);
        assert!(!rig.seen("a"), "input from before it showed does not count");
        // Input after it showed: counting.
        rig.st.note_input("main", T + 4_500);
        rig.tick(T + 5_000);
        rig.tick(T + 6_000);
        assert_eq!(rig.left("a"), Some(58_000));
        // The app goes away: held at once, nothing credited.
        rig.st.sync_front(false, T + 6_500);
        rig.tick(T + 7_000);
        rig.tick(T + 8_000);
        assert_eq!(rig.left("a"), Some(58_000));
        let runs = rig.pass(T + 8_000, false);
        assert_eq!(rig.st.clock.get(&runs[0].id), Some(&Clock::Held));
    }

    #[test]
    fn input_older_than_a_minute_holds_the_pin_and_resumes_with_the_left_time() {
        let mut rig = counting_rig();
        rig.cfg.expire_ms = Some(120_000);
        // The viewer's last input is at T+500; the app stays in front a long time.
        for k in 1..=70 {
            rig.tick(T + k * 1_000);
        }
        // Counted through T+60 s (input within 60 s), then held, however long the app stays in front.
        assert_eq!(rig.left("a"), Some(61_000));
        let frozen = rig.left("a");
        let runs = rig.pass(T + 70_000, false);
        assert_eq!(rig.st.clock.get(&runs[0].id), Some(&Clock::Held));
        // Touch StreamNook again: it resumes from where it was.
        rig.st.note_input("main", T + 70_500);
        rig.tick(T + 71_000);
        rig.tick(T + 72_000);
        assert_eq!(rig.left("a"), frozen.map(|l| l - 2_000));
    }

    #[test]
    fn a_sleep_gap_credits_at_most_two_seconds() {
        let mut rig = counting_rig();
        rig.tick(T + 1_000);
        rig.tick(T + 2_000);
        let before = rig.left("a").unwrap();
        // An 8 h sleep, then input and a tick: 2 s at most, and nothing ends.
        let wake = T + 8 * 3_600_000;
        rig.st.note_input("main", wake - 100);
        rig.tick(wake);
        assert!(!rig.ended("a"));
        assert_eq!(before - rig.left("a").unwrap(), 2_000);
    }

    #[test]
    fn the_app_must_have_been_in_front_a_moment_before_clocks_count() {
        let mut rig = counting_rig();
        rig.tick(T + 100);
        assert!(!rig.seen("a"), "front for 100 ms is not yet 250 ms");
        rig.tick(T + 600);
        assert!(rig.seen("a"));
    }

    #[test]
    fn seen_sets_kept_while_away_count_on_return_once_the_viewer_touches_it() {
        let mut rig = Rig::new(vec![pin_entry("a", "xqc", "1", NOW_WALL - 1_000)]);
        // The report lands while the app is away: stored, the effect gated.
        rig.surfaces("main", vec![surface("main-chat", &["xqc"], true, &["a"])]);
        rig.tick(T + 1_000);
        rig.st.note_input("main", T + 1_500);
        rig.tick(T + 2_000);
        assert!(!rig.seen("a"));
        // The app comes back: input from before does not count, input after does.
        rig.st.sync_front(true, T + 10_000);
        rig.tick(T + 11_000);
        assert!(!rig.seen("a"));
        rig.st.note_input("main", T + 11_500);
        rig.tick(T + 12_000);
        assert!(rig.seen("a"));
    }

    #[test]
    fn an_until_closed_pin_is_seen_without_a_clock() {
        let mut rig = counting_rig();
        rig.cfg.expire_ms = None;
        rig.tick(T + 1_000);
        rig.tick(T + 2_000);
        assert!(rig.seen("a") && rig.left("a").is_none());
        let runs = rig.pass(T + 2_000, false);
        assert_eq!(rig.st.clock.get(&runs[0].id), Some(&Clock::None));
    }

    #[test]
    fn a_pin_ends_when_its_time_is_used_and_never_comes_back() {
        let mut rig = counting_rig();
        rig.cfg.expire_ms = Some(3_000);
        for i in 1..=6 {
            rig.tick(T + i * 1_000);
        }
        assert!(rig.ended("a"));
        assert!(rig.pass(T + 7_000, false).is_empty());
        // Neither a longer For nor until-closed brings it back.
        rig.cfg.expire_ms = Some(3_600_000);
        assert!(rig.pass(T + 8_000, false).is_empty());
        rig.cfg.expire_ms = None;
        assert!(rig.pass(T + 9_000, false).is_empty());
        // Off, then on again with a newer stamp: still gone, and older mentions stay out.
        rig.cfg.on = false;
        assert!(rig.pass(T + 10_000, false).is_empty());
        rig.cfg.on = true;
        rig.cfg.enabled_at_ms = Some(NOW_WALL + 10);
        rig.cfg.expire_ms = Some(60_000);
        assert!(rig.pass(T + 11_000, false).is_empty());
    }

    // ---- filing: Most at once, runs, backfill -----------------------------------

    #[test]
    fn the_oldest_end_for_good_at_most_at_once_and_a_run_ends_together() {
        let mut entries: Vec<MentionEntry> =
            (0..12).map(|i| pin_entry(&format!("m{i:02}"), "xqc", &i.to_string(), NOW_WALL - 60_000 + i)).collect();
        let mut copy = pin_entry("copy", "xqc", "11", NOW_WALL - 5_000);
        copy.text = "hi @brandon m11".into();
        entries.push(copy);
        let mut rig = Rig::new(entries);
        let runs = rig.pass(T, false);
        assert_eq!(runs.len(), 10);
        assert_eq!(runs[0].id, "m02", "the oldest two ended");
        assert!(rig.ended("m00") && rig.ended("m01"));
        // m11 and its copy are one run: dismissing or ending one is both.
        let last = runs.last().unwrap();
        assert_eq!(last.ids.len(), 2);
        // Lowering the maximum ends the excess at once.
        rig.cfg.max = 3;
        let runs = rig.pass(T + 1_000, false);
        assert_eq!(runs.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), vec!["m09", "m10", "m11"]);
    }

    #[test]
    fn cold_boot_keeps_saved_pins_and_the_stamp_is_not_a_launch() {
        let rig = Rig::new(vec![pin_entry("a", "xqc", "1", NOW_WALL - 1_000)]);
        // The inbox saves and loads again: the pin is still live, with its clock.
        let mut saved = rig.inbox;
        saved.pin_apply_credits(&cfg(), &[PinCredit { id: "a".into(), counts: true, credit_ms: 4_000 }]);
        let text = serde_json::to_string(&saved.page(None, 10, false).0).unwrap();
        let back: Vec<MentionEntry> = serde_json::from_str(&text).unwrap();
        let mut reloaded = Inbox::default();
        for e in back {
            reloaded.file(e, NOW_WALL);
        }
        let runs = reloaded.pin_live_runs(&cfg(), NOW_WALL);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].left_ms, Some(56_000));
        // A launch never moves the stamp.
        let mut s = crate::models::settings::Settings::default();
        s.chat_design.pin_mentions = true;
        s.pin_mentions_enabled_at_ms = Some(123);
        assert!(!s.fill_pin_mentions_stamp(NOW_WALL), "a saved stamp is left alone");
        assert_eq!(s.pin_mentions_enabled_at_ms, Some(123));
    }

    #[test]
    fn backfilled_entries_never_pin() {
        let msg = pin_entry("h", "xqc", "1", NOW_WALL);
        // on_backfill files entry_for(): no pin stamp is ever put on it.
        let mut filed = msg.clone();
        filed.pin_from_ms = None;
        let rig = Rig::new(vec![filed]);
        assert!(rig.inbox.pin_live_runs(&cfg(), NOW_WALL).is_empty());
        let src = include_str!("mention_inbox.rs").replace("\r\n", "\n");
        let start = src.find("pub fn on_backfill").expect("on_backfill exists");
        let body = &src[start..start + src[start..].find("\n}\n").unwrap()];
        assert!(!body.contains("pin_from_ms") && !body.contains("pin_stamp"), "a backlog row gets no pin stamp");
    }

    #[test]
    fn clock_time_alone_saves_lazily_and_an_ended_pin_saves_soon() {
        let mut rig = counting_rig();
        rig.cfg.expire_ms = Some(2_000);
        let step = |rig: &mut Rig, now: Mono| pass_core(&mut rig.inbox, &mut rig.st, &rig.cfg.clone(), NOW_WALL, now, true).1;
        assert_eq!(step(&mut rig, T + 1_000), Change::Clock, "seen, nothing ended: no announcement, no periodic write");
        assert_eq!(step(&mut rig, T + 2_000), Change::Clock);
        assert_eq!(step(&mut rig, T + 3_000), Change::Dirty, "ended is final: saved by the next flush");
        assert_eq!(step(&mut rig, T + 4_000), Change::None);
        let inbox = include_str!("mention_inbox.rs").replace("\r\n", "\n");
        assert!(inbox.contains("spawn_blocking(|| flush(false))"), "the 5 s flusher leaves clock-only changes alone");
        assert!(inbox.contains("pub fn flush_now() -> Result<(), String> {"));
    }

    // ---- surfaces ----------------------------------------------------------------

    #[test]
    fn an_older_call_a_retired_page_and_a_page_start_never_leave_stale_surfaces() {
        let mut st = PinState::default();
        assert!(st.set_surfaces("main", "p1", 0, vec![]));
        assert!(st.set_surfaces("main", "p1", 2, vec![surface("main-chat", &["xqc"], true, &[])]));
        assert!(!st.set_surfaces("main", "p1", 1, vec![]), "older sequence dropped");
        assert!(!st.set_surfaces("main", "p1", 2, vec![]), "an equal sequence is not newer");
        assert_eq!(st.windows["main"].surfaces.len(), 1);
        // A reload: the surfaces go, the old page's late calls are refused, the new page applies.
        assert!(st.page_started("main"));
        assert!(st.windows["main"].surfaces.is_empty());
        assert!(!st.set_surfaces("main", "p1", 3, vec![surface("main-chat", &["xqc"], true, &[])]));
        assert!(st.set_surfaces("main", "p2", 0, vec![]));
        assert!(st.set_surfaces("main", "p2", 1, vec![surface("dock", &["xqc"], true, &[])]));
        // A page from timeOrigin can go back: any non-retired page is accepted.
        assert!(st.set_surfaces("main", "p0", 0, vec![]));
        assert!(!st.set_surfaces("main", "p2", 9, vec![]), "p2 retired when p0 took over");
        // Destroyed windows leave nothing, and what is not shown shows nothing.
        st.set_surfaces("popout", "q1", 1, vec![surface("blended", &["xqc"], false, &["a"])]);
        assert!(st.windows["popout"].surfaces[0].visible.is_empty(), "a hidden surface has nothing visible");
        st.forget_window("popout");
        assert!(st.windows["popout"].surfaces.is_empty());
    }

    #[test]
    fn rooms_become_rust_room_keys() {
        let mut st = PinState::default();
        st.set_surfaces(
            "main",
            "p",
            1,
            vec![SurfaceIn {
                id: "dock".into(),
                rooms: vec![
                    RoomRef { provider: "twitch".into(), channel: "XQC".into() },
                    RoomRef { provider: "kick".into(), channel: "SomeSlug".into() },
                ],
                shown: true,
                ..Default::default()
            }],
        );
        assert_eq!(st.windows["main"].surfaces[0].rooms, vec!["xqc".to_string(), "kick:someslug".to_string()]);
    }

    #[test]
    fn turning_pins_off_clears_every_surface_home_and_clock() {
        let mut rig = counting_rig();
        rig.tick(T + 1_000);
        rig.st.clear_all();
        assert!(rig.st.windows["main"].surfaces.is_empty());
        assert!(rig.st.homes.is_empty() && rig.st.clock.is_empty() && rig.st.answer.is_empty());
        // The old page cannot paint them back.
        assert!(!rig.st.set_surfaces("main", "page-1", 99, vec![surface("main-chat", &["xqc"], true, &["a"])]));
    }

    // ---- homes -------------------------------------------------------------------

    fn home_of(rig: &Rig, id: &str) -> Option<(String, String)> {
        rig.st.homes.get(id).map(|h| (h.window.clone(), h.surface.clone()))
    }

    #[test]
    fn a_surface_showing_the_chat_wins_else_the_primary_and_ties_follow_the_order() {
        let mut rig = Rig::new(vec![pin_entry("a", "xqc", "1", NOW_WALL - 1_000), pin_entry("b", "other", "2", NOW_WALL - 900)]);
        rig.surfaces(
            "main",
            vec![surface("dock", &["xqc"], true, &[]), surface("main-chat", &["xqc"], true, &[]), surface("pane:z", &["xqc"], true, &[])],
        );
        rig.pass(T, false);
        assert_eq!(home_of(&rig, "a"), Some(("main".into(), "main-chat".into())), "main-chat before dock before the rest");
        // Nobody shows `other`: the primary, by the same order.
        assert_eq!(home_of(&rig, "b"), Some(("main".into(), "main-chat".into())));
    }

    #[test]
    fn the_lower_third_and_over_video_are_never_primary_and_a_phone_follows_what_it_opens() {
        let mut rig = Rig::new(vec![
            pin_entry("a", "other", "1", NOW_WALL - 1_000),
            pin_entry("y", "youtube:chan", "2", NOW_WALL - 900),
            pin_entry("k", "somethingelse:chan", "3", NOW_WALL - 800),
        ]);
        let mut third = surface("phone-watch", &["xqc"], true, &[]);
        third.strips_only = true;
        let mut chats = surface("phone-chats", &["xqc"], true, &[]);
        chats.opens = vec!["twitch".into()];
        rig.surfaces("main", vec![third, chats]);
        rig.pass(T, false);
        assert_eq!(home_of(&rig, "a"), Some(("main".into(), "phone-chats".into())), "the strips-only surface is never primary");
        assert_eq!(home_of(&rig, "y"), Some(("main".into(), "phone-chats".into())), "YouTube pins may home on a phone");
        assert_eq!(home_of(&rig, "k"), None, "a platform no phone surface opens has no home");
    }

    #[test]
    fn a_home_is_sticky_and_a_remount_inside_three_seconds_keeps_it() {
        let mut rig = Rig::new(vec![pin_entry("a", "xqc", "1", NOW_WALL - 1_000)]);
        rig.surfaces("main", vec![surface("dock", &["xqc"], true, &[])]);
        rig.pass(T, false);
        assert_eq!(home_of(&rig, "a"), Some(("main".into(), "dock".into())));
        // A better surface appears: the home stays.
        rig.surfaces("main", vec![surface("dock", &["xqc"], true, &[]), surface("main-chat", &["xqc"], true, &[])]);
        rig.pass(T + 1_000, false);
        assert_eq!(home_of(&rig, "a"), Some(("main".into(), "dock".into())));
        // The dock unmounts and remounts inside the grace.
        rig.surfaces("main", vec![surface("main-chat", &["xqc"], true, &[])]);
        rig.pass(T + 2_000, false);
        rig.pass(T + 4_000, false);
        assert_eq!(home_of(&rig, "a"), Some(("main".into(), "dock".into())), "gone for 2 s only");
        rig.surfaces("main", vec![surface("dock", &["xqc"], true, &[]), surface("main-chat", &["xqc"], true, &[])]);
        rig.pass(T + 4_500, false);
        assert_eq!(home_of(&rig, "a"), Some(("main".into(), "dock".into())));
        // Gone for 3 s: it moves, and the pin lives.
        rig.surfaces("main", vec![surface("main-chat", &["xqc"], true, &[])]);
        rig.pass(T + 5_000, false);
        let runs = rig.pass(T + 8_100, false);
        assert_eq!(home_of(&rig, "a"), Some(("main".into(), "main-chat".into())));
        assert_eq!(runs.len(), 1, "a home change never ends a pin");
        assert!(!rig.ended("a"));
    }

    #[test]
    fn a_home_that_stays_unshown_three_seconds_moves_to_another_shown_surface() {
        let mut rig = Rig::new(vec![pin_entry("a", "xqc", "1", NOW_WALL - 1_000)]);
        rig.surfaces("main", vec![surface("main-chat", &["xqc"], true, &[]), surface("dock", &["xqc"], true, &[])]);
        rig.pass(T, false);
        assert_eq!(home_of(&rig, "a"), Some(("main".into(), "main-chat".into())));
        rig.surfaces("main", vec![surface("main-chat", &["xqc"], false, &[]), surface("dock", &["xqc"], true, &[])]);
        rig.pass(T + 1_000, false);
        assert_eq!(home_of(&rig, "a"), Some(("main".into(), "main-chat".into())), "not yet");
        rig.pass(T + 4_100, false);
        assert_eq!(home_of(&rig, "a"), Some(("main".into(), "dock".into())));
    }

    #[test]
    fn the_home_follows_the_window_the_viewer_moved_to_after_ten_idle_seconds() {
        let mut rig = Rig::new(vec![pin_entry("a", "xqc", "1", NOW_WALL - 1_000)]);
        rig.surfaces("main", vec![surface("main-chat", &["xqc"], true, &[])]);
        rig.surfaces("popout", vec![surface("blended", &["xqc"], true, &[])]);
        rig.st.note_input("main", T);
        rig.pass(T + 1, false);
        assert_eq!(home_of(&rig, "a"), Some(("main".into(), "main-chat".into())), "the window with the most recent input");
        // The viewer works in the popout; main goes quiet for 10 s.
        rig.st.note_input("popout", T + 5_000);
        rig.pass(T + 6_000, false);
        assert_eq!(home_of(&rig, "a"), Some(("main".into(), "main-chat".into())), "main had input 6 s ago");
        rig.st.note_input("popout", T + 11_000);
        rig.pass(T + 11_100, false);
        assert_eq!(home_of(&rig, "a"), Some(("popout".into(), "blended".into())));
        assert!(!rig.ended("a"));
    }

    #[test]
    fn only_the_home_surfaces_visible_set_counts() {
        let mut rig = Rig::new(vec![pin_entry("a", "xqc", "1", NOW_WALL - 1_000)]);
        rig.st.sync_front(true, T);
        // The pin is visible on the dock, but its home is main-chat.
        rig.surfaces("main", vec![surface("main-chat", &["xqc"], true, &[]), surface("dock", &["xqc"], true, &["a"])]);
        rig.pass(T, false);
        rig.st.note_input("main", T + 500);
        rig.tick(T + 1_000);
        rig.tick(T + 2_000);
        assert!(!rig.seen("a"));
    }

    // ---- the phones' flag --------------------------------------------------------

    #[test]
    fn a_phone_starts_not_counting_and_reads_through_on_the_first_true_in_either_order() {
        assert!(!phone_front(false, false, true), "nothing reported yet");
        assert!(!phone_front(false, true, false), "false until the first explicit report");
        // interactive lands first, the foreground flag after (the phone is still idle in between)...
        assert!(!phone_front(true, true, true));
        assert!(phone_front(true, true, false));
        // ...or the foreground first.
        assert!(!phone_front(true, false, false));
        assert!(phone_front(true, true, false));
        assert!(!phone_front(true, true, true), "backgrounded again");
    }

    // ---- ordering pins ------------------------------------------------------------

    #[test]
    fn going_away_stops_the_clocks_before_any_store_is_flushed() {
        let src = include_str!("app_presence.rs").replace("\r\n", "\n");
        let start = src.find("pub fn set_foreground").unwrap();
        let body = &src[start..start + src[start..].find("\n}\n").unwrap()];
        let hook = body.find("mention_pins::reading_inputs_changed()").expect("the pins hear every report");
        assert!(hook < body.find("flush_debounced_stores").unwrap(), "stopped before the stores are written");
        let inbox = include_str!("mention_inbox.rs").replace("\r\n", "\n");
        let flush = &inbox[inbox.find("pub fn flush_now").unwrap()..];
        assert!(
            flush.find("mention_pins::fold_for_flush()").unwrap() < flush.find("st.dirty").unwrap(),
            "the running clocks fold into the entries before the snapshot is taken"
        );
        let claims = include_str!("whisper_claims.rs").replace("\r\n", "\n");
        let start = claims.find("pub fn set_app_interactive").unwrap();
        let body = &claims[start..start + claims[start..].find("\n}\n").unwrap()];
        assert!(body.contains("mention_pins::reading_inputs_changed()") && body.contains("spawn_blocking"));
        let perf = include_str!("performance_mode.rs").replace("\r\n", "\n");
        let start = perf.find("fn apply(gen").unwrap();
        let body = &perf[start..start + perf[start..].find("\n}\n").unwrap()];
        assert!(body.find("publish();").unwrap() < body.find("reading_inputs_changed").unwrap(), "after publish");
        assert!(perf.contains("pins_on()"), "the poll runs while pins are on");
    }

    #[test]
    fn the_pins_event_is_defined_where_it_is_emitted_and_pages_clear_their_stacks() {
        assert_eq!(PINS_EVENT, "mention-pins-changed");
        let inbox = include_str!("mention_inbox.rs");
        assert!(!inbox.contains("emit(PINS_EVENT"), "only mention_pins emits it");
        let lib = include_str!("../lib.rs");
        assert!(lib.contains("services::mention_pins::window_page_started(webview.label())"));
        assert!(lib.contains("services::mention_pins::forget_window(&label)"));
    }

    // ---- the answer ----------------------------------------------------------------

    /// A state with two pins homed on `main-chat` (one from another chat),
    /// published the way `finish` does.
    fn published() -> (PinState, Vec<PinRun>) {
        let mut rig = Rig::new(vec![
            pin_entry("a", "xqc", "1", NOW_WALL - 20_000),
            pin_entry("b", "otherchat", "2", NOW_WALL - 10_000),
        ]);
        rig.st.sync_front(true, T);
        rig.surfaces("main", vec![surface("main-chat", &["xqc"], true, &["a", "b"])]);
        rig.pass(T, false);
        rig.st.note_input("main", T + 500);
        rig.tick(T + 1_000);
        rig.tick(T + 2_000);
        let runs = rig.pass(T + 2_000, false);
        let rows = runs.iter().map(|r| mention_pin_row::cached(&r.entry)).collect();
        assert!(rig.st.publish(&runs, rows, &rig.cfg, day_number(NOW_WALL)));
        (rig.st, runs)
    }

    #[test]
    fn the_answer_marks_other_chats_and_orders_oldest_first() {
        let (st, _) = published();
        let pins = st.pins_for("main", "main-chat", day_number(NOW_WALL));
        assert_eq!(pins.iter().map(|p| p.entry.id.as_str()).collect::<Vec<_>>(), vec!["a", "b"]);
        assert_eq!(pins[0].channel_mark, None, "this surface's own chat");
        assert_eq!(pins[1].channel_mark.as_ref().map(|m| m.room.as_str()), Some("otherchat"));
        assert_eq!((pins[0].clock, pins[0].left_ms), ("running", Some(59_000)));
        assert!(st.pins_for("main", "dock", day_number(NOW_WALL)).is_empty(), "a pin is on its home only");
        assert!(st.pins_for("popout", "main-chat", day_number(NOW_WALL)).is_empty());
    }

    #[test]
    fn the_revision_moves_for_what_is_drawn_and_not_for_the_time_left() {
        let mut rig = Rig::new(vec![pin_entry("a", "xqc", "1", NOW_WALL - 1_000)]);
        rig.st.sync_front(true, T);
        rig.surfaces("main", vec![surface("main-chat", &["xqc"], true, &["a"])]);
        rig.pass(T, false);
        rig.st.note_input("main", T + 500);
        let today = day_number(NOW_WALL);
        let publish = |rig: &mut Rig, now: Mono| {
            let runs = rig.tick(now);
            let rows = runs.iter().map(|r| mention_pin_row::cached(&r.entry)).collect();
            rig.st.publish(&runs, rows, &rig.cfg.clone(), today)
        };
        assert!(publish(&mut rig, T + 1_000), "it appeared");
        let first = rig.st.revision;
        assert!(!publish(&mut rig, T + 2_000), "the time left moved: no new revision");
        assert!(!publish(&mut rig, T + 3_000));
        assert_eq!(rig.st.revision, first);
        // Held, then a different day, then a different maximum: each moves it.
        rig.st.sync_front(false, T + 3_500);
        assert!(publish(&mut rig, T + 4_000), "running became held");
        let runs = rig.inbox.pin_live_runs(&rig.cfg, NOW_WALL);
        let rows = runs.iter().map(|r| mention_pin_row::cached(&r.entry)).collect();
        assert!(rig.st.publish(&runs, rows, &rig.cfg.clone(), today + 1), "midnight");
    }

    #[test]
    fn the_answer_matches_the_golden_shape() {
        let (st, _) = published();
        // "Today" is the pins' own day (the 20 s before NOW_WALL can straddle a
        // local midnight in some time zone), so the golden holds in every zone.
        let today = day_number(NOW_WALL - 20_000);
        let out = MentionPins { revision: st.revision, pins: st.pins_for("main", "main-chat", today), max: st.max as u32 };
        let mut value = serde_json::to_value(&out).unwrap();
        // The row hash is a std hasher's: its value is not part of the shape.
        for pin in value["pins"].as_array_mut().unwrap() {
            assert_eq!(pin["rev"], serde_json::json!(mention_pin_row::row_hash(&pin["row"])));
            pin["rev"] = serde_json::json!(0);
        }
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join("mention-pins-golden.json");
        let fresh = serde_json::to_string_pretty(&value).unwrap() + "\n";
        if std::env::var("UPDATE_GOLDEN").is_ok_and(|v| v == "1") {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, &fresh).unwrap();
        }
        let stored = std::fs::read_to_string(&path).expect("run once with UPDATE_GOLDEN=1");
        assert!(!stored.trim().is_empty());
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&stored).unwrap(),
            value,
            "the answer's shape moved: every shell decodes it. Regenerate with UPDATE_GOLDEN=1 and review the diff"
        );
        assert!(value["pins"].as_array().is_some_and(|p| !p.is_empty()), "the golden holds pins");
    }
}
