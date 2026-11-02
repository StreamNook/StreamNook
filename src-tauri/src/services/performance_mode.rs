//! Performance mode (Settings > Interface > Performance mode): while it is
//! active every StreamNook window draws the cheap look. No backdrop blur, no
//! player glow, decorative loops still and entrances instant, and with Freeze
//! emotes the chat's emotes, paints and badges on their first frame. The video
//! is never touched.
//!
//! Rust decides when it is active because "In the background" depends on
//! whether ANY StreamNook window is the one in front, which no single page can
//! see: a click into a MultiChat popout blurs the main window without leaving
//! the app. Every window applies what this module emits (`performance-state`)
//! and asks once on load (`get_performance_state`).
//!
//! "In front" comes from the OS, never from one window's focus flag:
//! - Windows: a foreground hook, because wry keeps focus in the WebView2 child,
//!   so leaving the app sends the window no focus event at all. WebView2's own
//!   dialogs (the file chooser, DevTools, print) belong to its browser
//!   process, a child of this one, so those count as ours.
//! - macOS: `NSApplication.isActive`, true while our own panels and sheets are
//!   key.
//! - Linux: the X11 active window, ours or transient for ours (the CEF runtime
//!   always runs on X11). Portal file dialogs are another process and are not
//!   transient for us, so StreamNook counts its own open dialogs and reads as
//!   in front while one is open (every OS).
//!
//! Leaving turns the cheap look on after `AWAY_DELAY`, so a quick look at
//! another app changes nothing; coming back turns it off at once.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;
use serde::Serialize;
use tauri::{Emitter, Manager};

use crate::models::settings::{AppState, PerformanceMode, Settings};
use crate::rt::AppHandle;

pub const EVENT: &str = "performance-state";

/// How long StreamNook has to be out of front before the cheap look comes on.
/// Still emotes swap every image source, so flipping on a quick alt-tab would
/// reload them all for nothing.
const AWAY_DELAY: Duration = Duration::from_secs(10);
/// The away timer wakes this much after the delay, so its check never lands a
/// hair before the moment `decide` counts as long enough.
const TIMER_SLACK: Duration = Duration::from_millis(50);
/// How long a lost focus or a closed dialog waits before the OS is asked who
/// is in front: by then it knows where focus landed.
const FOCUS_SETTLE: Duration = Duration::from_millis(150);

/// What every window draws. `still_chat` is only ever true while `active`.
#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct PerformanceState {
    pub active: bool,
    pub still_chat: bool,
}

/// What an answer did: whether anything changed, and the leave time a new
/// away timer has to be armed for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Answer {
    changed: bool,
    arm: Option<Instant>,
}

/// Everything the decision reads. Every transition is a method taking the
/// instant it happens at, so the rules are tested with made-up instants.
#[derive(Debug)]
struct Inputs {
    mode: PerformanceMode,
    still_chat: bool,
    /// None until the first answer.
    in_front: Option<bool>,
    /// When StreamNook left the front. A second app switch while away keeps
    /// the first leave time.
    away_since: Option<Instant>,
    /// Bumped by every focus event, hook call and poll; an answer to an older
    /// question is dropped.
    gen: u64,
    /// StreamNook's own open dialogs, per window label.
    dialogs: HashMap<String, u32>,
}

impl Inputs {
    fn new() -> Self {
        Self {
            mode: PerformanceMode::Off,
            still_chat: false,
            in_front: None,
            away_since: None,
            gen: 0,
            dialogs: HashMap::new(),
        }
    }

    fn bump(&mut self) -> u64 {
        self.gen = self.gen.wrapping_add(1);
        self.gen
    }

    fn dialog_open(&self) -> bool {
        self.dialogs.values().any(|&n| n > 0)
    }

    /// An answer to the question tagged `gen`. While one of our dialogs is
    /// open the app is in front whatever the OS says.
    fn answer(&mut self, gen: u64, front: bool, now: Instant) -> Answer {
        if gen != self.gen {
            return Answer {
                changed: false,
                arm: None,
            };
        }
        let front = front || self.dialog_open();
        let before = (self.in_front, self.away_since);
        self.in_front = Some(front);
        let mut arm = None;
        if front {
            self.away_since = None;
        } else if self.away_since.is_none() {
            self.away_since = Some(now);
            arm = Some(now);
        }
        Answer {
            changed: before != (self.in_front, self.away_since),
            arm,
        }
    }

    fn decide(&self, now: Instant) -> PerformanceState {
        let active = match self.mode {
            PerformanceMode::Off => false,
            PerformanceMode::Always => true,
            PerformanceMode::Background => {
                self.in_front == Some(false)
                    && !self.dialog_open()
                    && self
                        .away_since
                        .is_some_and(|t| now.saturating_duration_since(t) >= AWAY_DELAY)
            }
        };
        PerformanceState {
            active,
            still_chat: active && self.still_chat,
        }
    }

    /// The away timer armed at `t0` still stands for the current absence.
    fn timer_due(&self, t0: Instant) -> bool {
        self.away_since == Some(t0)
    }

    fn open_dialog(&mut self, label: &str) {
        *self.dialogs.entry(label.to_string()).or_insert(0) += 1;
    }

    fn close_dialog(&mut self, label: &str) {
        if let Some(n) = self.dialogs.get_mut(label) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                self.dialogs.remove(label);
            }
        }
    }

    fn forget_window(&mut self, label: &str) {
        self.dialogs.remove(label);
    }
}

static APP: OnceLock<AppHandle> = OnceLock::new();
static INPUTS: Lazy<Mutex<Inputs>> = Lazy::new(|| Mutex::new(Inputs::new()));
/// The state last emitted. Compared and emitted under this one lock, so two
/// publishers can never send out of order. Taken before INPUTS, never after,
/// and nobody holds either while asking the OS.
static SENT: Mutex<PerformanceState> = Mutex::new(PerformanceState {
    active: false,
    still_chat: false,
});

fn inputs() -> MutexGuard<'static, Inputs> {
    INPUTS.lock().unwrap_or_else(|p| p.into_inner())
}

/// The state to send, if it differs from the last one sent (and records it).
fn next_to_send(sent: &mut PerformanceState, now: PerformanceState) -> Option<PerformanceState> {
    if *sent == now {
        return None;
    }
    *sent = now;
    Some(now)
}

fn publish() {
    let mut sent = SENT.lock().unwrap_or_else(|p| p.into_inner());
    let now = inputs().decide(Instant::now());
    let Some(state) = next_to_send(&mut sent, now) else {
        return;
    };
    log::info!(
        "[Performance] active={} still_chat={}",
        state.active,
        state.still_chat
    );
    if let Some(app) = APP.get() {
        if let Err(e) = app.emit(EVENT, state) {
            log::debug!("[Performance] emit failed: {e}");
        }
    }
}

/// What windows were last told, for one that opens after it was sent.
pub fn sent() -> PerformanceState {
    *SENT.lock().unwrap_or_else(|p| p.into_inner())
}

fn apply(gen: u64, front: bool) {
    let answer = inputs().answer(gen, front, Instant::now());
    if let Some(t0) = answer.arm {
        arm_timer(t0);
    }
    publish();
}

fn arm_timer(t0: Instant) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(AWAY_DELAY + TIMER_SLACK).await;
        let due = inputs().timer_due(t0);
        if due {
            publish();
        }
    });
}

/// Ask again once focus has settled, tagged with a fresh question.
fn ask_after_settle() {
    let gen = inputs().bump();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FOCUS_SETTLE).await;
        if let Some(app) = APP.get() {
            ask(app, gen);
        }
    });
}

/// Called once from the setup hook, on the main thread (which the Windows
/// foreground hook needs).
pub fn start(app: AppHandle) {
    let read = app.try_state::<AppState>().and_then(|s| {
        s.settings
            .lock()
            .ok()
            .map(|g| (g.performance_mode, g.performance_still_chat))
    });
    let _ = APP.set(app);
    if let Some((mode, still)) = read {
        let mut i = inputs();
        i.mode = mode;
        i.still_chat = still;
    }
    #[cfg(windows)]
    install_foreground_hook();
    publish();
    let gen = inputs().bump();
    tauri::async_runtime::spawn(async move {
        if let Some(app) = APP.get() {
            ask(app, gen);
        }
    });
}

/// A save changed the settings (any window). `after_settings_change` hands
/// over Rust's live copy, so two saves never apply out of order.
pub fn refresh(settings: &Settings) {
    let background = {
        let mut i = inputs();
        i.mode = settings.performance_mode;
        i.still_chat = settings.performance_still_chat;
        i.mode == PerformanceMode::Background
    };
    publish();
    if background {
        tauri::async_runtime::spawn(async { poll() });
    }
}

/// Ask the OS whether StreamNook is in front. Only "In the background" needs
/// it; called every second by the window poller. Never on the main thread.
pub fn poll() {
    let gen = {
        let mut i = inputs();
        if i.mode != PerformanceMode::Background {
            return;
        }
        i.bump()
    };
    if let Some(app) = APP.get() {
        ask(app, gen);
    }
}

/// A StreamNook window gained or lost focus (from `on_window_event`, on the
/// main thread). A gain means the app is in front now; a loss is checked
/// against the OS once focus has settled. Windows watches the foreground
/// window itself instead.
#[cfg(not(windows))]
pub fn note_focus(focused: bool) {
    if focused {
        let gen = inputs().bump();
        apply(gen, true);
    } else {
        ask_after_settle();
    }
}

/// A window closed: dialogs it had open are gone with it.
pub fn note_window_destroyed(label: &str) {
    inputs().forget_window(label);
}

/// One of StreamNook's own dialogs opened over the window labelled `label`.
pub fn dialog_opened(label: &str) {
    let gen = {
        let mut i = inputs();
        i.open_dialog(label);
        i.bump()
    };
    apply(gen, true);
}

/// That dialog closed; the OS says where focus went.
pub fn dialog_closed(label: &str) {
    inputs().close_dialog(label);
    ask_after_settle();
}

/// Holds a dialog open for as long as it lives, for Rust's own dialogs.
pub struct DialogGuard(String);

impl DialogGuard {
    pub fn new(label: &str) -> Self {
        dialog_opened(label);
        Self(label.to_string())
    }
}

impl Drop for DialogGuard {
    fn drop(&mut self) {
        dialog_closed(&self.0);
    }
}

// ---------------------------------------------------------------- Windows

#[cfg(windows)]
const WEBVIEW_EXE: &str = "msedgewebview2.exe";
/// How long a "is this pid ours" verdict is trusted.
#[cfg(windows)]
const VERDICT_TTL: Duration = Duration::from_secs(60);
#[cfg(windows)]
static VERDICTS: Lazy<Mutex<HashMap<(u32, u64), (bool, Instant)>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// One process as the judge sees it.
#[derive(Debug, Clone)]
#[cfg_attr(not(windows), allow(dead_code))]
struct ProcFacts {
    parent: u32,
    /// Lowercased, no path.
    name: String,
    /// Creation time (FILETIME as one number); None when it cannot be read.
    created: Option<u64>,
}

/// Whether `pid` is StreamNook: this process, or a WebView2 process started
/// after it whose parent is this process or such a WebView2 process. A
/// browser StreamNook opened a link in is `msedge.exe` or `chrome.exe` and
/// never counts; a WebView2 process older than us is someone else's, whose
/// dead parent's pid happened to be reused.
#[cfg_attr(not(windows), allow(dead_code))]
fn judge(pid: u32, own: u32, own_created: u64, facts: impl Fn(u32) -> Option<ProcFacts>) -> bool {
    if pid == own {
        return true;
    }
    let webview_after_us = |f: &ProcFacts| {
        f.name == "msedgewebview2.exe" && f.created.is_some_and(|c| c >= own_created)
    };
    let Some(me) = facts(pid) else { return false };
    if !webview_after_us(&me) {
        return false;
    }
    if me.parent == own {
        return true;
    }
    let Some(parent) = facts(me.parent) else {
        return false;
    };
    webview_after_us(&parent) && parent.parent == own
}

#[cfg(windows)]
fn window_pid(hwnd: windows::Win32::Foundation::HWND) -> Option<u32> {
    use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
    if hwnd.0.is_null() {
        return None;
    }
    let mut pid = 0u32;
    // SAFETY: a plain query on a window handle the system just gave us.
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
    }
    (pid != 0).then_some(pid)
}

#[cfg(windows)]
fn filetime(t: windows::Win32::Foundation::FILETIME) -> u64 {
    (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime)
}

#[cfg(windows)]
fn creation_time(pid: u32) -> Option<u64> {
    use windows::Win32::Foundation::{CloseHandle, FILETIME};
    use windows::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    // SAFETY: a limited query handle opened and closed here.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let (mut created, mut exited, mut kernel, mut user) = (
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
        );
        let ok = GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user).is_ok();
        let _ = CloseHandle(handle);
        ok.then(|| filetime(created))
    }
}

#[cfg(windows)]
fn own_creation() -> u64 {
    static OWN: OnceLock<u64> = OnceLock::new();
    *OWN.get_or_init(|| {
        use windows::Win32::Foundation::FILETIME;
        use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
        let (mut created, mut exited, mut kernel, mut user) = (
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
        );
        // SAFETY: the pseudo handle of this process needs no closing.
        let ok = unsafe {
            GetProcessTimes(
                GetCurrentProcess(),
                &mut created,
                &mut exited,
                &mut kernel,
                &mut user,
            )
            .is_ok()
        };
        if ok {
            filetime(created)
        } else {
            0
        }
    })
}

/// Whether `pid` is StreamNook, cached per (pid, creation time). None when
/// the process list cannot be read (keep the last answer).
#[cfg(windows)]
fn win_is_ours(pid: u32) -> Option<bool> {
    let own = std::process::id();
    if pid == own {
        return Some(true);
    }
    let key = (pid, creation_time(pid).unwrap_or(0));
    let now = Instant::now();
    {
        let mut cache = VERDICTS.lock().unwrap_or_else(|p| p.into_inner());
        cache.retain(|_, (_, at)| now.saturating_duration_since(*at) < VERDICT_TTL);
        if let Some((verdict, _)) = cache.get(&key) {
            return Some(*verdict);
        }
    }
    let procs = crate::platform::process::tree();
    if procs.is_empty() {
        return None;
    }
    let facts = |p: u32| {
        procs.iter().find(|x| x.pid == p).map(|x| ProcFacts {
            parent: x.parent,
            name: x.name.clone(),
            created: creation_time(p),
        })
    };
    let verdict = judge(pid, own, own_creation(), facts);
    VERDICTS
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(key, (verdict, now));
    Some(verdict)
}

#[cfg(windows)]
fn ask(_app: &AppHandle, gen: u64) {
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
    // SAFETY: no arguments; a null answer is handled by window_pid.
    let Some(pid) = window_pid(unsafe { GetForegroundWindow() }) else {
        return;
    };
    if let Some(front) = win_is_ours(pid) {
        apply(gen, front);
    }
}

/// Every foreground change on the desktop, ours or not. Runs on the main
/// thread inside its message loop: it only reads the pid and hands the rest
/// to a blocking task.
#[cfg(windows)]
unsafe extern "system" fn on_foreground(
    _hook: windows::Win32::UI::Accessibility::HWINEVENTHOOK,
    _event: u32,
    hwnd: windows::Win32::Foundation::HWND,
    _object: i32,
    _child: i32,
    _thread: u32,
    _time: u32,
) {
    let Some(pid) = window_pid(hwnd) else { return };
    let gen = inputs().bump();
    tauri::async_runtime::spawn_blocking(move || {
        if let Some(front) = win_is_ours(pid) {
            apply(gen, front);
        }
    });
}

#[cfg(windows)]
fn install_foreground_hook() {
    use windows::Win32::UI::Accessibility::SetWinEventHook;
    use windows::Win32::UI::WindowsAndMessaging::{EVENT_SYSTEM_FOREGROUND, WINEVENT_OUTOFCONTEXT};
    // SAFETY: an out-of-context hook on this (main) thread, whose loop pumps
    // it; the system removes it when the thread exits.
    let hook = unsafe {
        SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            None,
            Some(on_foreground),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        )
    };
    if hook.0.is_null() {
        log::warn!("[Performance] foreground hook failed; the 1 s poll stands in");
    }
}

// ---------------------------------------------------------------- macOS

#[cfg(target_os = "macos")]
fn ask(app: &AppHandle, gen: u64) {
    // AppKit answers only on the main thread; the dispatch does not wait.
    let _ = app.run_on_main_thread(move || {
        let Some(mtm) = objc2::MainThreadMarker::new() else {
            return;
        };
        let active = objc2_app_kit::NSApplication::sharedApplication(mtm).isActive();
        apply(gen, active);
    });
}

// ---------------------------------------------------------------- Linux

/// Whether the X11 active window is ours or a dialog transient for ours.
/// No active window (XWayland reports none while a Wayland app has focus)
/// is not ours.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn judge_x11(active: Option<u64>, transient_for: Option<u64>, ours: &[u64]) -> bool {
    active.is_some_and(|w| ours.contains(&w)) || transient_for.is_some_and(|w| ours.contains(&w))
}

#[cfg(target_os = "linux")]
fn ask(app: &AppHandle, gen: u64) {
    let ours: Vec<u64> = app
        .webview_windows()
        .values()
        .filter_map(|w| crate::linux_x11::xid(w).ok())
        .collect();
    let active = crate::linux_x11::active_window();
    let transient = active.and_then(crate::linux_x11::transient_for);
    apply(gen, judge_x11(active, transient, &ours));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(base: Instant, secs: f64) -> Instant {
        base + Duration::from_secs_f64(secs)
    }

    #[test]
    fn decide_table() {
        let t0 = Instant::now();
        for still in [false, true] {
            let mut i = Inputs::new();
            i.still_chat = still;
            for mode in [
                PerformanceMode::Off,
                PerformanceMode::Background,
                PerformanceMode::Always,
            ] {
                i.mode = mode;
                for front in [None, Some(true), Some(false)] {
                    i.in_front = front;
                    i.away_since = (front == Some(false)).then_some(t0);
                    for (secs, long) in [(9.9, false), (10.0, true)] {
                        let expect = match mode {
                            PerformanceMode::Off => false,
                            PerformanceMode::Always => true,
                            PerformanceMode::Background => front == Some(false) && long,
                        };
                        assert_eq!(
                            i.decide(at(t0, secs)),
                            PerformanceState {
                                active: expect,
                                still_chat: expect && still
                            },
                            "{mode:?} front={front:?} {secs}s still={still}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn sends_only_changes() {
        let mut sent = PerformanceState::default();
        assert_eq!(next_to_send(&mut sent, PerformanceState::default()), None);
        let on = PerformanceState {
            active: true,
            still_chat: false,
        };
        assert_eq!(next_to_send(&mut sent, on), Some(on));
        assert_eq!(next_to_send(&mut sent, on), None);
        assert_eq!(
            next_to_send(&mut sent, PerformanceState::default()),
            Some(PerformanceState::default())
        );
    }

    #[test]
    fn publish_emits_inside_the_sent_guard() {
        let src = include_str!("performance_mode.rs").replace("\r\n", "\n");
        let body = &src[src.find("fn publish() {").expect("publish exists")..];
        let body = &body[..body.find("\n}\n").expect("publish ends")];
        let lock = body.find("SENT.lock()").expect("publish takes SENT");
        let emit = body.find("emit(EVENT").expect("publish emits the event");
        assert!(lock < emit, "the emit happens under the SENT guard");
        assert!(
            !body.contains("drop(sent)"),
            "the guard is held through the emit"
        );
    }

    #[test]
    fn a_second_switch_while_away_keeps_the_first_leave_time() {
        let t0 = Instant::now();
        let mut i = Inputs::new();
        i.mode = PerformanceMode::Background;
        let g = i.bump();
        assert_eq!(
            i.answer(g, false, t0),
            Answer {
                changed: true,
                arm: Some(t0)
            }
        );
        let g = i.bump();
        assert_eq!(
            i.answer(g, false, at(t0, 3.0)),
            Answer {
                changed: false,
                arm: None
            }
        );
        assert!(!i.decide(at(t0, 9.9)).active);
        assert!(i.decide(at(t0, 10.0)).active);
        assert!(i.timer_due(t0));
    }

    #[test]
    fn coming_back_clears_at_once_and_disarms_the_timer() {
        let t0 = Instant::now();
        let mut i = Inputs::new();
        i.mode = PerformanceMode::Background;
        let g = i.bump();
        i.answer(g, false, t0);
        assert!(i.decide(at(t0, 12.0)).active);
        let g = i.bump();
        assert_eq!(
            i.answer(g, true, at(t0, 12.0)),
            Answer {
                changed: true,
                arm: None
            }
        );
        assert!(!i.decide(at(t0, 12.0)).active);
        assert!(!i.timer_due(t0));
        // Leaving again starts a new absence with its own timer.
        let g = i.bump();
        let t1 = at(t0, 20.0);
        assert_eq!(i.answer(g, false, t1).arm, Some(t1));
        assert!(!i.decide(at(t0, 25.0)).active);
        assert!(i.decide(at(t0, 30.0)).active);
    }

    #[test]
    fn a_stale_answer_is_dropped() {
        let t0 = Instant::now();
        let mut i = Inputs::new();
        i.mode = PerformanceMode::Background;
        let old = i.bump();
        let new = i.bump();
        assert_eq!(
            i.answer(new, true, t0),
            Answer {
                changed: true,
                arm: None
            }
        );
        assert_eq!(
            i.answer(old, false, t0),
            Answer {
                changed: false,
                arm: None
            }
        );
        assert_eq!(i.in_front, Some(true));
    }

    #[test]
    fn answers_while_off_count_once_background_is_picked() {
        let t0 = Instant::now();
        let mut i = Inputs::new();
        assert_eq!(i.in_front, None);
        let g = i.bump();
        i.answer(g, false, t0);
        assert!(
            !i.decide(at(t0, 30.0)).active,
            "Off never draws the cheap look"
        );
        i.mode = PerformanceMode::Background;
        assert!(i.decide(at(t0, 30.0)).active);
    }

    #[test]
    fn an_open_dialog_keeps_the_app_in_front() {
        let t0 = Instant::now();
        let mut i = Inputs::new();
        i.mode = PerformanceMode::Background;
        i.open_dialog("main");
        let g = i.bump();
        i.answer(g, false, t0);
        assert_eq!(i.in_front, Some(true));
        assert!(!i.decide(at(t0, 60.0)).active);
        i.open_dialog("main");
        i.close_dialog("main");
        assert!(i.dialog_open(), "two opens need two closes");
        i.close_dialog("main");
        assert!(!i.dialog_open());
        i.open_dialog("multichat-1");
        i.forget_window("multichat-1");
        assert!(
            !i.dialog_open(),
            "a closed window takes its dialogs with it"
        );
    }

    fn proc(parent: u32, name: &str, created: u64) -> ProcFacts {
        ProcFacts {
            parent,
            name: name.into(),
            created: Some(created),
        }
    }

    #[test]
    fn webview_children_are_ours_and_launched_browsers_are_not() {
        let own = 100;
        let own_created = 1_000;
        let table: HashMap<u32, ProcFacts> = [
            (200, proc(own, "msedgewebview2.exe", 2_000)),
            (300, proc(200, "msedgewebview2.exe", 2_100)),
            (400, proc(own, "msedge.exe", 3_000)),
            (500, proc(own, "msedgewebview2.exe", 500)),
            (600, proc(999, "msedgewebview2.exe", 2_000)),
            (700, proc(own, "minecraft.exe", 4_000)),
        ]
        .into_iter()
        .collect();
        let facts = |p: u32| table.get(&p).cloned();
        assert!(judge(own, own, own_created, facts), "this process");
        assert!(
            judge(200, own, own_created, facts),
            "the WebView2 browser process"
        );
        assert!(
            judge(300, own, own_created, facts),
            "a WebView2 child of it"
        );
        assert!(
            !judge(400, own, own_created, facts),
            "a browser StreamNook launched"
        );
        assert!(
            !judge(500, own, own_created, facts),
            "older than us: a reused parent pid"
        );
        assert!(
            !judge(600, own, own_created, facts),
            "someone else's WebView2"
        );
        assert!(!judge(700, own, own_created, facts), "a game");
        assert!(!judge(800, own, own_created, facts), "an unknown pid");
    }

    #[test]
    fn x11_judges_by_window_id() {
        let ours = [0x10, 0x20];
        assert!(judge_x11(Some(0x10), None, &ours));
        assert!(
            judge_x11(Some(0x99), Some(0x20), &ours),
            "a dialog transient for ours"
        );
        assert!(!judge_x11(Some(0x99), Some(0x98), &ours));
        assert!(!judge_x11(Some(0x99), None, &ours));
        assert!(
            !judge_x11(None, None, &ours),
            "no active window is not ours"
        );
    }

    #[test]
    fn linux_runs_the_cef_runtime_on_x11() {
        let src = include_str!("../linux_cef.rs").replace("\r\n", "\n");
        assert!(
            src.contains("linux_windowing: tauri_runtime_cef::LinuxWindowing::X11"),
            "the Linux in-front check reads X11 window properties"
        );
    }
}
