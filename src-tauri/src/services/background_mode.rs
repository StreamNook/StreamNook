//! Autopilot's Missing badges mode, as the app shell sees it: StreamNook keeps
//! running in the tray with every window closed, the tray says what the mode
//! is doing, a notice reaches the viewer while no window is up, and the app
//! ends itself when nothing is left to keep it alive.
//!
//! Core knows nothing about farming or mining here. It reads four settings
//! (Missing badges, Keep running when closed, Start with the computer, the
//! told flag), whether an enabled plugin provides `drops.missing-badges` with
//! its Twitch consent granted, and the `paused` flag of that plugin's last
//! status. Every input is a value in memory, so a close on the event-loop
//! thread never waits on disk or on the plugin.
//!
//! The close decision is made in one place (`on_user_windows_gone`, or
//! `exit_requested_decision` for the runtime's own request); see the 3.C
//! section of the plan for the reasoning behind each flag below.

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use once_cell::sync::{Lazy, OnceCell};
use serde::Serialize;
use serde_json::Value;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIcon;
use tauri::{Emitter, Manager};

use crate::commands::notification_mirror::{deliver, MirroredNotice, NoticeAction};
use crate::models::drops::DropsSettings;
use crate::models::settings::{AppState, CloseToTrayMode};
use crate::plugin_host::registry::Registry;
use crate::rt::{AppHandle, Rt};
use crate::services::app_exit::{self, SHUTDOWN};

/// The tray icon's id.
pub const TRAY_ID: &str = "streamnook-tray";
/// Sent (no payload) whenever the autostart state or keep-alive may have changed.
pub const STATE_EVENT: &str = "background-state-changed";

const PROVIDER_FEATURE: &str = "drops.missing-badges";
const PROVIDER_CONSENT_KIND: &str = "twitch.android";
const WAITING_LINE: &str = "Waiting for missing badges";
const CLOSING_NOTICE: &str = "Missing badges stopped, so StreamNook is closing.";
const TOLD_NOTICE: &str =
    "StreamNook keeps collecting missing badges in the tray. Quit from the tray icon to stop.";
const LINE_CAP: usize = 64;
const FALLBACK_DESTROY_AFTER: Duration = Duration::from_secs(5);
const CLOSING_NOTICE_WAIT: Duration = Duration::from_secs(2);
/// The Linux background launch waits this many 1 s steps for a tray host.
const LAUNCH_WAIT_STEPS: u32 = 30;

// ---------------------------------------------------------------- state

/// Whether an enabled plugin can run Missing badges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    /// The registry has not been read yet, or could not be.
    Unknown,
    Ready,
    NotReady,
}

impl Provider {
    fn as_u8(self) -> u8 {
        match self {
            Provider::Unknown => 0,
            Provider::Ready => 1,
            Provider::NotReady => 2,
        }
    }

    fn from_u8(value: u8) -> Provider {
        match value {
            1 => Provider::Ready,
            2 => Provider::NotReady,
            _ => Provider::Unknown,
        }
    }
}

static APP: OnceCell<AppHandle> = OnceCell::new();
static PROVIDER: AtomicU8 = AtomicU8::new(0);
static PROVIDER_ID: Mutex<Option<String>> = Mutex::new(None);
/// The provider's registry name (data, for the tray's consent line).
static PROVIDER_NAME: Mutex<Option<String>> = Mutex::new(None);
/// The provider's last `drops.status` value; cleared when it stops.
static LAST_STATUS: Mutex<Option<Value>> = Mutex::new(None);
/// Linux: a tray host owns `org.kde.StatusNotifierWatcher` on the session bus.
/// Starts false (no host, today's quit) and follows the bus.
#[cfg(target_os = "linux")]
static TRAY_HOST: AtomicBool = AtomicBool::new(false);
/// The app stayed alive with every window gone only because of keep-alive.
static STAYING_FOR_KEEP_ALIVE: AtomicBool = AtomicBool::new(false);
/// One exit task at a time.
static EXIT_PENDING: AtomicBool = AtomicBool::new(false);
/// Counts main windows: bumped by every show and every destroy of main.
static MAIN_LIFETIME: AtomicU64 = AtomicU64::new(0);
/// The `MAIN_LIFETIME` a keep-alive close was made in; `u64::MAX` is no tag.
static KEEP_ALIVE_HIDE: AtomicU64 = AtomicU64::new(u64::MAX);
/// A `--background` launch kept the app windowless.
static BACKGROUND_STARTED: AtomicBool = AtomicBool::new(false);
static TELLING: AtomicBool = AtomicBool::new(false);
static BACKGROUND_LAUNCH: Lazy<bool> =
    Lazy::new(|| argv_is_background(&std::env::args().collect::<Vec<_>>()));

/// Stores the handle; called once in setup.
pub fn init(app: &AppHandle) {
    let _ = APP.set(app.clone());
}

pub fn provider() -> Provider {
    Provider::from_u8(PROVIDER.load(Ordering::Acquire))
}

pub fn staying_for_keep_alive() -> bool {
    STAYING_FOR_KEEP_ALIVE.load(Ordering::Acquire)
}

/// The provider's registry state. Called after every registry load and save.
pub fn note_registry(registry: &Registry) {
    let (provider, found) = registry_provider(registry);
    PROVIDER.store(provider.as_u8(), Ordering::Release);
    let (id, name) = match found {
        Some((id, name)) => (Some(id), Some(name)),
        None => (None, None),
    };
    {
        let mut current = PROVIDER_ID.lock().unwrap_or_else(|e| e.into_inner());
        if *current != id {
            *LAST_STATUS.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
        *current = id;
    }
    *PROVIDER_NAME.lock().unwrap_or_else(|e| e.into_inner()) = name;
    if let Some(app) = APP.get() {
        crate::services::autostart::sync(app);
        refresh_tray(app);
        reevaluate(app);
    }
}

/// A plugin process ended. Whatever status the provider sent no longer holds.
pub fn note_provider_stopped(plugin_id: &str) {
    if !is_provider(plugin_id) {
        return;
    }
    *LAST_STATUS.lock().unwrap_or_else(|e| e.into_inner()) = None;
    if let Some(app) = APP.get() {
        refresh_tray(app);
        reevaluate(app);
    }
}

/// A plugin pushed its `drops.status`. Only the provider's is kept.
pub fn note_drops_status(plugin_id: &str, value: &Value) {
    if !is_provider(plugin_id) {
        return;
    }
    *LAST_STATUS.lock().unwrap_or_else(|e| e.into_inner()) = Some(value.clone());
    if let Some(app) = APP.get() {
        refresh_tray(app);
        reevaluate(app);
    }
}

fn is_provider(plugin_id: &str) -> bool {
    let id = PROVIDER_ID.lock().unwrap_or_else(|e| e.into_inner());
    keeps_status(id.as_deref(), plugin_id)
}

fn status_paused() -> bool {
    LAST_STATUS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .is_some_and(status_is_paused)
}

/// Missing badges, Keep running when closed, Start at login.
fn switches(app: &AppHandle) -> (bool, bool, bool) {
    match app.try_state::<AppState>() {
        Some(state) => {
            let s = state.settings.lock().unwrap_or_else(|e| e.into_inner());
            (
                s.drops.collect_missing_badges,
                s.drops.keep_running_when_closed,
                s.drops.start_at_login,
            )
        }
        None => (false, false, false),
    }
}

fn drops_inputs(app: &AppHandle) -> (bool, bool) {
    let (collect, keep, _) = switches(app);
    (collect, keep)
}

fn close_mode(app: &AppHandle) -> CloseToTrayMode {
    app.try_state::<AppState>()
        .and_then(|s| s.settings.lock().ok().map(|g| g.close_to_tray))
        .unwrap_or_default()
}

fn platform_ok(tray_host_check: bool) -> bool {
    #[cfg(target_os = "linux")]
    {
        crate::linux_cef::session_bus_available()
            && (!tray_host_check || TRAY_HOST.load(Ordering::Acquire))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = tray_host_check;
        true
    }
}

/// Whether StreamNook should keep running with no window. Includes the Linux
/// tray-host check, so it is the question for ENTERING windowless mode.
pub fn keep_alive(app: &AppHandle) -> bool {
    let (collect, keep) = drops_inputs(app);
    wants_keep_alive(
        collect,
        keep,
        provider(),
        status_paused(),
        platform_ok(true),
    )
}

/// `keep_alive` without the Linux tray-host check, for re-evaluation: a
/// plasmashell restart must never quit the app.
fn keep_alive_now(app: &AppHandle) -> bool {
    let (collect, keep) = drops_inputs(app);
    wants_keep_alive(
        collect,
        keep,
        provider(),
        status_paused(),
        platform_ok(false),
    )
}

// ---------------------------------------------------------------- pure rules

/// Pure: every input must hold. `Unknown` counts as not ready.
pub fn wants_keep_alive(
    collect: bool,
    keep_running: bool,
    provider: Provider,
    paused: bool,
    platform_ok: bool,
) -> bool {
    collect && keep_running && provider == Provider::Ready && !paused && platform_ok
}

/// Pure: whether the runtime's own exit request (`code: None`) is refused.
/// (tao's macOS Cmd+Q never reaches `ExitRequested`, so it always quits.)
pub fn stays_alive(close_mode: CloseToTrayMode, code: Option<i32>, keep_alive: bool) -> bool {
    code.is_none() && (close_mode == CloseToTrayMode::Always || keep_alive)
}

/// Pure: whether the main window's close hides it to the tray.
pub fn close_hides(mode: CloseToTrayMode, popouts_open: bool, keep_alive: bool) -> bool {
    mode == CloseToTrayMode::Always
        || (mode == CloseToTrayMode::WithPopouts && popouts_open)
        || keep_alive
}

/// Pure: the close that made this `ended` main window was a keep-alive close.
pub fn keep_alive_close(tag: u64, ended: u64) -> bool {
    tag == ended
}

/// Pure: a teardown for the main window that ended as lifetime `ended` is
/// still the current one: no main exists, nothing was shown or destroyed
/// since, and the app is not shutting down.
pub fn teardown_current(main_present: bool, lifetime: u64, ended: u64, shutdown: bool) -> bool {
    !main_present && ended.checked_add(1) == Some(lifetime) && !shutdown
}

/// Pure: the 5 s fallback destroys main only if it is the same hidden window.
pub fn fallback_destroy_due(
    captured: u64,
    current: u64,
    main_present: bool,
    main_hidden: bool,
    shutdown: bool,
) -> bool {
    captured == current && main_present && main_hidden && !shutdown
}

/// Pure: the re-evaluation exit is due.
pub fn exit_due_from(staying: bool, keep_alive_now: bool, shutdown: bool) -> bool {
    staying && !keep_alive_now && !shutdown
}

/// Pure: one exit task wins the right to run.
pub fn try_begin_exit(pending: &AtomicBool) -> bool {
    pending
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
}

/// Pure: a task whose check failed gives up its place, reads the inputs once
/// more, and goes again only after winning the exchange again (no lost
/// wake-up, never two tasks).
pub fn finish_failed_round(pending: &AtomicBool, due_now: impl FnOnce() -> bool) -> bool {
    pending.store(false, Ordering::Release);
    due_now() && try_begin_exit(pending)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExitStep {
    Recheck,
    Notice,
    FinalCheck,
    Exit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExitNext {
    Go(ExitStep),
    Abort,
}

/// Pure step machine: check, notice, check, exit. A failed check aborts.
fn next_exit_step(step: ExitStep, still_due: bool) -> ExitNext {
    match (step, still_due) {
        (ExitStep::Recheck, true) => ExitNext::Go(ExitStep::Notice),
        (ExitStep::Notice, _) => ExitNext::Go(ExitStep::FinalCheck),
        (ExitStep::FinalCheck, true) => ExitNext::Go(ExitStep::Exit),
        _ => ExitNext::Abort,
    }
}

/// What the runtime's exit request becomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitVerdict {
    Proceed,
    Stay,
    StayAndReevaluate,
}

/// Pure: `code: Some(_)` (tray Quit, the updater, the exit task) always
/// proceeds. Inside `code: None`: the staying flag keeps the app and asks for
/// a re-evaluation, computing nothing; otherwise a decision recorded for this
/// last window is used, and anything else is computed. Never proceeds by default.
pub fn exit_requested_decision(
    code: Option<i32>,
    flag: bool,
    recorded: impl FnOnce() -> Option<bool>,
    compute: impl FnOnce() -> bool,
) -> ExitVerdict {
    if code.is_some() {
        return ExitVerdict::Proceed;
    }
    if flag {
        return ExitVerdict::StayAndReevaluate;
    }
    let stay = recorded().unwrap_or_else(compute);
    if stay {
        ExitVerdict::Stay
    } else {
        ExitVerdict::Proceed
    }
}

/// Pure: only the provider's status is kept.
pub fn keeps_status(provider_id: Option<&str>, plugin_id: &str) -> bool {
    provider_id == Some(plugin_id)
}

/// Pure: the provider's last status says the mode is paused.
pub fn status_is_paused(status: &Value) -> bool {
    status
        .get("paused")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// Pure: the provider among the registry's plugins: an enabled process plugin
/// that provides `drops.missing-badges` and has its Twitch consent granted.
pub fn registry_provider(registry: &Registry) -> (Provider, Option<(String, String)>) {
    let found = registry.plugins.iter().find(|p| {
        p.enabled
            && p.kind == "process"
            && p.granted.provides.iter().any(|f| f == PROVIDER_FEATURE)
            && p.credential_consent
                .get(PROVIDER_CONSENT_KIND)
                .map(String::as_str)
                == Some("always")
    });
    match found {
        Some(p) => (Provider::Ready, Some((p.id.clone(), p.name.clone()))),
        None => (Provider::NotReady, None),
    }
}

/// Pure: a `--background` argument after the program name.
pub fn argv_is_background(args: &[String]) -> bool {
    args.iter().skip(1).any(|a| a == "--background")
}

/// Whether this launch carries `--background`.
pub fn background_launch() -> bool {
    *BACKGROUND_LAUNCH
}

/// What a `--background` launch does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchDecision {
    /// Main is not shown; the app stays in the tray.
    Hide { sets_staying_flag: bool },
    /// Missing badges is off or its provider is gone: remove the login entry and exit.
    RemoveAndExit,
    /// Show a normal window (registry unreadable, or no Linux tray host).
    Visible,
}

/// Pure. `keep_alive` is the full question (the Linux tray host included) and `wanted`
/// is `collect && keep_running && start_at_login`. With keep-alive false and
/// a known provider, switches turned off or a missing provider remove the
/// entry and exit before keep-alive is read (a Linux machine with no tray
/// host and the switches off must not keep an entry the viewer turned off);
/// an unreadable registry shows a window and keeps the entry; so does a
/// wanted entry that keep-alive refuses (no Linux tray host).
pub fn background_launch_decision(
    background: bool,
    keep_alive: bool,
    wanted: bool,
    provider: Provider,
    mode: CloseToTrayMode,
) -> LaunchDecision {
    if !background {
        return LaunchDecision::Visible;
    }
    match provider {
        Provider::Unknown => LaunchDecision::Visible,
        Provider::NotReady => LaunchDecision::RemoveAndExit,
        // Switches off: the entry goes before keep-alive or the tray host is read.
        Provider::Ready if !wanted => LaunchDecision::RemoveAndExit,
        Provider::Ready if keep_alive => LaunchDecision::Hide {
            sets_staying_flag: mode != CloseToTrayMode::Always,
        },
        // Wanted and ready but not kept alive (no Linux tray host: `keep_alive`
        // already includes that check): a normal window, entry kept.
        Provider::Ready => LaunchDecision::Visible,
    }
}

/// Pure: the posted Linux decision is dropped when main already exists (the
/// viewer opened StreamNook during the wait).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn launch_decision_dropped(main_exists: bool) -> bool {
    main_exists
}

/// Pure: stop waiting for a Linux tray host.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn launch_wait_done(tray_host: bool, waited_steps: u32) -> bool {
    tray_host || waited_steps >= LAUNCH_WAIT_STEPS
}

/// Pure: a plugin notice goes through `deliver` only while main is not on screen.
pub fn should_deliver_notice(main_on_screen: bool) -> bool {
    !main_on_screen
}

/// Pure: the told flag is saved only when the notice was delivered and, for a
/// system notification, the OS permission is granted.
pub fn told_flag_should_save(delivered: Option<&str>, system_granted: bool) -> bool {
    match delivered {
        None => false,
        Some("system") => system_granted,
        Some(_) => true,
    }
}

/// Pure: the tray's one-line state from the provider's status. `provider_name`
/// is the plugin's registry name (data), used in the consent line.
pub fn tray_line(status: &Value, provider_name: &str) -> String {
    let working = status
        .get("is_active")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let drop_name = status
        .get("drop_name")
        .and_then(Value::as_str)
        .filter(|n| !n.is_empty());
    let text = match (working, drop_name) {
        (true, Some(name)) => {
            let cur = status
                .get("current_minutes")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            let req = status
                .get("required_minutes")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            format!("Collecting {name}, {cur} of {req} min")
        }
        _ if status_is_paused(status) => "Missing badges paused".to_string(),
        _ => match status.get("blocked").and_then(Value::as_str) {
            Some("signed_out") => "Sign in to Drops to collect badges".to_string(),
            Some("consent") => format!("{provider_name} needs your OK to use Twitch"),
            _ => WAITING_LINE.to_string(),
        },
    };
    cap_line(&text)
}

fn cap_line(text: &str) -> String {
    if text.chars().count() <= LINE_CAP {
        return text.to_string();
    }
    let mut capped: String = text.chars().take(LINE_CAP - 1).collect();
    capped.push('\u{2026}');
    capped
}

/// Pure: muda reads `&` as a mnemonic, so the menu text doubles it. The
/// tooltip keeps the plain line.
pub fn menu_text(line: &str) -> String {
    line.replace('&', "&&")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayMode {
    Normal,
    Windowless,
}

/// Pure: windowless when no user window is on screen, keep-alive holds, and
/// the app is past its boot gate (main was shown once, or a background
/// launch kept it hidden on purpose).
pub fn tray_mode(window_visible: bool, keep_alive: bool, booted: bool) -> TrayMode {
    if !window_visible && keep_alive && booted {
        TrayMode::Windowless
    } else {
        TrayMode::Normal
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuKind {
    Normal,
    Windowless,
    WindowlessUpdate,
}

/// Pure: which prebuilt menu the tray shows.
pub fn tray_menu_kind(mode: TrayMode, update_ready: bool) -> MenuKind {
    match (mode, update_ready) {
        (TrayMode::Normal, _) => MenuKind::Normal,
        (TrayMode::Windowless, false) => MenuKind::Windowless,
        (TrayMode::Windowless, true) => MenuKind::WindowlessUpdate,
    }
}

// ---------------------------------------------------------------- main window lifetime

/// `show_main_window` was called: a new main (or a shown one) starts a new
/// lifetime, so a teardown or fallback destroy from the old one stands down.
pub fn note_main_window_shown() {
    MAIN_LIFETIME.fetch_add(1, Ordering::AcqRel);
}

/// A show cleared the staying flag (called through `app_exit::note_main_shown`).
pub fn note_shown() {
    if STAYING_FOR_KEEP_ALIVE.swap(false, Ordering::AcqRel) {
        if let Some(app) = APP.get() {
            refresh_tray(app);
        }
    }
}

/// The user's close hides main under keep-alive: tag this lifetime so its
/// `Destroyed` ends the session, and arm the 5 s fallback destroy.
pub fn note_keep_alive_close(app: &AppHandle) {
    let lifetime = MAIN_LIFETIME.load(Ordering::Acquire);
    KEEP_ALIVE_HIDE.store(lifetime, Ordering::Release);
    arm_fallback_destroy(app, lifetime);
}

fn arm_fallback_destroy(app: &AppHandle, lifetime: u64) {
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(FALLBACK_DESTROY_AFTER);
        let on_main = app.clone();
        let _ = app.run_on_main_thread(move || {
            let main = on_main.get_webview_window("main");
            let hidden = main
                .as_ref()
                .is_some_and(|w| !w.is_visible().unwrap_or(true));
            if fallback_destroy_due(
                lifetime,
                MAIN_LIFETIME.load(Ordering::Acquire),
                main.is_some(),
                hidden,
                SHUTDOWN.load(Ordering::Acquire),
            ) {
                if let Some(window) = main {
                    let _ = window.destroy();
                }
            }
        });
    });
}

/// The main `Destroyed` arm calls this for every main window, always.
pub fn note_main_destroyed(app: &AppHandle) {
    let ended = MAIN_LIFETIME.fetch_add(1, Ordering::AcqRel);
    let keep_alive_close = keep_alive_close(KEEP_ALIVE_HIDE.load(Ordering::Acquire), ended);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        main_teardown(app, ended, keep_alive_close).await;
    });
}

/// Whether the teardown for the main window that ended as `ended` is current.
pub fn teardown_still_current(app: &AppHandle, ended: u64) -> bool {
    teardown_current(
        crate::services::window_visibility::window_open(app, "main"),
        MAIN_LIFETIME.load(Ordering::Acquire),
        ended,
        SHUTDOWN.load(Ordering::Acquire),
    )
}

async fn main_teardown(app: AppHandle, ended: u64, end_session: bool) {
    let check = app.clone();
    let still_current = move || teardown_still_current(&check, ended);
    crate::services::watch_start::stop_for_closed_main(app, still_current, end_session).await;
}

// ---------------------------------------------------------------- the close decision

/// Every user window is gone (main hidden in the tray counts as gone for the
/// popout arm). Decides once whether the app stays, records it, and returns
/// it; the caller exits on false.
pub fn on_user_windows_gone(app: &AppHandle, closed_label: &str) -> bool {
    let mode = close_mode(app);
    let stay = stays_alive(mode, None, keep_alive(app));
    app_exit::note_close_decision(closed_label, stay);
    if stay {
        record_stay(app, mode != CloseToTrayMode::Always);
    }
    stay
}

/// The shared store for "the app stays": with `sets_flag` (close mode is not
/// Always) it sets the staying flag, then refreshes the tray, flushes the
/// stores, says the one-time notice, and re-evaluates once so a keep-alive
/// input that flipped between the read and the store is still seen.
pub fn record_stay(app: &AppHandle, sets_flag: bool) {
    if sets_flag {
        STAYING_FOR_KEEP_ALIVE.store(true, Ordering::Release);
    }
    refresh_tray(app);
    tauri::async_runtime::spawn_blocking(crate::flush_debounced_stores);
    if keep_alive_now(app) {
        tauri::async_runtime::spawn(tell_once(app.clone()));
    }
    if sets_flag {
        reevaluate(app);
    }
}

/// Re-evaluation: after every registry save, provider stop, provider status
/// and drops save. Acts only while the staying flag is set.
pub fn reevaluate(app: &AppHandle) {
    if SHUTDOWN.load(Ordering::Acquire) {
        return;
    }
    if exit_due(app) && try_begin_exit(&EXIT_PENDING) {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            exit_task(app).await;
        });
    }
}

fn exit_due(app: &AppHandle) -> bool {
    exit_due_from(
        STAYING_FOR_KEEP_ALIVE.load(Ordering::Acquire),
        keep_alive_now(app),
        SHUTDOWN.load(Ordering::Acquire),
    )
}

async fn exit_task(app: AppHandle) {
    loop {
        if run_exit_steps(&app).await {
            return;
        }
        if !finish_failed_round(&EXIT_PENDING, || exit_due(&app)) {
            return;
        }
    }
}

/// Check, notice (awaited at most 2 s), check, exit. True once exit was called.
async fn run_exit_steps(app: &AppHandle) -> bool {
    let mut step = ExitStep::Recheck;
    loop {
        let due = match step {
            ExitStep::Recheck | ExitStep::FinalCheck => exit_due(app),
            ExitStep::Notice => {
                let notice = MirroredNotice {
                    id: "autopilot-closing".into(),
                    kind: "drops".into(),
                    title: "StreamNook".into(),
                    body: CLOSING_NOTICE.into(),
                    quote: None,
                    image_url: None,
                    round_image: false,
                    action_label: None,
                    action: None,
                    reply: None,
                };
                let _ =
                    tokio::time::timeout(CLOSING_NOTICE_WAIT, deliver(app.clone(), notice)).await;
                true
            }
            ExitStep::Exit => {
                app_exit::exit(app, 0, "Missing badges stopped; nothing keeps the app open");
                return true;
            }
        };
        match next_exit_step(step, due) {
            ExitNext::Go(next) => step = next,
            ExitNext::Abort => return false,
        }
    }
}

// ---------------------------------------------------------------- the background launch

/// Setup, after the registry load and `init`. A `--background` launch decides
/// once whether to stay in the tray, remove its login entry, or show a window.
pub fn decide_launch(app: &AppHandle) {
    if !background_launch() {
        return;
    }
    #[cfg(not(target_os = "linux"))]
    apply_launch_decision(app, decision_now(app));
    #[cfg(target_os = "linux")]
    wait_for_tray_host_then_decide(app);
}

fn decision_now(app: &AppHandle) -> LaunchDecision {
    let (collect, keep, login) = switches(app);
    background_launch_decision(
        true,
        keep_alive(app),
        collect && keep && login,
        provider(),
        close_mode(app),
    )
}

fn apply_launch_decision(app: &AppHandle, decision: LaunchDecision) {
    match decision {
        LaunchDecision::Hide { sets_staying_flag } => {
            BACKGROUND_STARTED.store(true, Ordering::Release);
            KEEP_ALIVE_HIDE.store(MAIN_LIFETIME.load(Ordering::Acquire), Ordering::Release);
            record_stay(app, sets_staying_flag);
            // Windows and macOS built main from the config, hidden: it goes
            // before its first reveal. Linux never built one.
            if let Some(main) = app.get_webview_window("main") {
                let _ = main.destroy();
            }
        }
        LaunchDecision::RemoveAndExit => {
            crate::services::autostart::remove();
            let app = app.clone();
            // After setup returns, so the event loop is running.
            tauri::async_runtime::spawn(async move {
                app_exit::exit(
                    &app,
                    0,
                    "background launch with Missing badges off; login entry removed",
                );
            });
        }
        LaunchDecision::Visible => {
            #[cfg(target_os = "linux")]
            crate::show_main_window(app);
        }
    }
}

#[cfg(target_os = "linux")]
fn wait_for_tray_host_then_decide(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let bus = crate::linux_cef::session_bus_available();
        let mut waited = 0u32;
        while bus && !launch_wait_done(TRAY_HOST.load(Ordering::Acquire), waited) {
            std::thread::sleep(Duration::from_secs(1));
            waited += 1;
        }
        let post = app.clone();
        if let Err(e) = app.run_on_main_thread(move || post_linux_launch_decision(&post)) {
            log::warn!("[Background] could not post the launch decision: {e}");
        }
    });
}

#[cfg(target_os = "linux")]
fn post_linux_launch_decision(app: &AppHandle) {
    if launch_decision_dropped(crate::services::window_visibility::window_open(app, "main")) {
        return;
    }
    apply_launch_decision(app, decision_now(app));
}

/// Linux: reads the tray host's owner once (so no close sees an unread
/// value), then a thread follows `NameOwnerChanged` for that name.
#[cfg(target_os = "linux")]
pub fn start_tray_host_watch() {
    if !crate::linux_cef::session_bus_available() {
        return;
    }
    let (first_read, first_read_done) = std::sync::mpsc::channel::<()>();
    let spawned = std::thread::Builder::new()
        .name("tray-host-watch".into())
        .spawn(move || tray_host_loop(first_read));
    if spawned.is_ok() {
        let _ = first_read_done.recv_timeout(Duration::from_secs(2));
    }
}

#[cfg(target_os = "linux")]
fn tray_host_loop(first_read: std::sync::mpsc::Sender<()>) {
    use zbus::blocking::{fdo::DBusProxy, Connection};
    use zbus::names::BusName;
    const WATCHER: &str = "org.kde.StatusNotifierWatcher";
    let read = |proxy: &DBusProxy| -> bool {
        BusName::try_from(WATCHER)
            .ok()
            .and_then(|name| proxy.name_has_owner(name).ok())
            .unwrap_or(false)
    };
    let proxy = match Connection::session().and_then(|c| DBusProxy::new(&c)) {
        Ok(proxy) => proxy,
        Err(e) => {
            log::warn!("[Background] no session bus for the tray host watch: {e}");
            let _ = first_read.send(());
            return;
        }
    };
    // Subscribe before the first read so a change in between is not missed.
    let changes = proxy.receive_name_owner_changed();
    TRAY_HOST.store(read(&proxy), Ordering::Release);
    let _ = first_read.send(());
    let Ok(changes) = changes else { return };
    for change in changes {
        let Ok(args) = change.args() else { continue };
        if args.name().as_str() == WATCHER {
            TRAY_HOST.store(read(&proxy), Ordering::Release);
        }
    }
}

// ---------------------------------------------------------------- notices

fn main_on_screen(app: &AppHandle) -> bool {
    app.get_webview_window("main")
        .is_some_and(|w| w.is_visible().unwrap_or(false) && !w.is_minimized().unwrap_or(true))
}

/// A plugin notice reaches the viewer through `deliver` (system notification
/// or a MultiChat card) only while main is not on screen; the in-app toast the
/// broker already sent covers the rest.
pub async fn deliver_plugin_notice(app: AppHandle, plugin_name: String, message: String) {
    if !should_deliver_notice(main_on_screen(&app)) {
        return;
    }
    let notice = MirroredNotice {
        id: format!("plugin-notice-{}", uuid::Uuid::new_v4()),
        kind: "drops".into(),
        title: plugin_name,
        body: message,
        quote: None,
        image_url: None,
        round_image: false,
        action_label: Some("Open".into()),
        action: Some(NoticeAction::Drops),
        reply: None,
    };
    let _ = deliver(app, notice).await;
}

async fn tell_once(app: AppHandle) {
    if TELLING.swap(true, Ordering::AcqRel) {
        return;
    }
    tell_inner(&app).await;
    TELLING.store(false, Ordering::Release);
}

async fn tell_inner(app: &AppHandle) {
    let told = match app.try_state::<AppState>() {
        Some(state) => {
            state
                .settings
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .drops
                .told_keep_running
        }
        None => return,
    };
    if told {
        return;
    }
    let notice = MirroredNotice {
        id: "autopilot-tray-told".into(),
        kind: "drops".into(),
        title: "StreamNook".into(),
        body: TOLD_NOTICE.into(),
        quote: None,
        image_url: None,
        round_image: false,
        action_label: None,
        action: None,
        reply: None,
    };
    let route = deliver(app.clone(), notice).await;
    let granted = match route.as_deref() {
        Some("system") => {
            crate::services::system_notify::permission(false).await
                == crate::services::system_notify::Permission::Granted
        }
        _ => false,
    };
    if told_flag_should_save(route.as_deref(), granted) {
        mark_told(app.clone()).await;
    }
}

/// The only writer of `told_keep_running`: a copy of the live drops settings
/// with the field set, saved through `apply_drops_settings` with the override.
pub async fn mark_told(app: AppHandle) {
    let _write = crate::commands::drops::DROPS_SETTINGS_WRITE.lock().await;
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let mut next: DropsSettings = state
        .settings
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .drops
        .clone();
    if next.told_keep_running {
        return;
    }
    next.told_keep_running = true;
    if let Err(e) =
        crate::commands::drops::apply_drops_settings(&app, &state, next, Some(true)).await
    {
        log::warn!("[Background] could not save the told flag: {e}");
    }
}

/// `apply_drops_settings` finished a save.
pub fn on_drops_settings_saved(app: &AppHandle, previous: &DropsSettings, next: &DropsSettings) {
    if crate::services::autostart::takeover_requested(previous.start_at_login, next.start_at_login)
    {
        crate::services::autostart::request_takeover();
    }
    if !next.start_at_login {
        crate::services::autostart::clear_takeover();
    }
    if !previous.collect_missing_badges && next.collect_missing_badges {
        // macOS shows its prompt now, while the viewer is in the settings.
        tauri::async_runtime::spawn(async {
            let _ = crate::services::system_notify::permission(true).await;
        });
    }
    crate::services::autostart::sync(app);
    refresh_tray(app);
    reevaluate(app);
}

pub fn emit_state_changed(app: &AppHandle) {
    let _ = app.emit(STATE_EVENT, ());
}

// ---------------------------------------------------------------- the tray

#[derive(Debug, Clone, PartialEq, Eq)]
struct TrayView {
    kind: MenuKind,
    line: String,
}

/// The tray's handles, managed once the tray is built.
pub struct TrayParts {
    tray: TrayIcon<Rt>,
    normal: Menu<Rt>,
    windowless: Menu<Rt>,
    windowless_update: Menu<Rt>,
    status_items: [MenuItem<Rt>; 2],
    applied: Mutex<Option<TrayView>>,
}

/// Builds the two windowless menus, keeps every handle, and applies the first state.
pub fn install_tray(app: &AppHandle, tray: TrayIcon<Rt>, normal: Menu<Rt>) -> tauri::Result<()> {
    let build = |with_update: bool| -> tauri::Result<(Menu<Rt>, MenuItem<Rt>)> {
        let status = MenuItem::with_id(app, "bg_status", WAITING_LINE, false, None::<&str>)?;
        let update = MenuItem::with_id(app, "bg_update", "Update ready", false, None::<&str>)?;
        let sep = PredefinedMenuItem::separator(app)?;
        let open = MenuItem::with_id(app, "bg_open", "Open StreamNook", true, None::<&str>)?;
        let quit = MenuItem::with_id(app, "bg_quit", "Quit StreamNook", true, None::<&str>)?;
        let menu = if with_update {
            Menu::with_items(app, &[&status, &update, &sep, &open, &quit])?
        } else {
            Menu::with_items(app, &[&status, &sep, &open, &quit])?
        };
        Ok((menu, status))
    };
    let (windowless, status_a) = build(false)?;
    let (windowless_update, status_b) = build(true)?;
    app.manage(TrayParts {
        tray,
        normal,
        windowless,
        windowless_update,
        status_items: [status_a, status_b],
        applied: Mutex::new(None),
    });
    refresh_tray(app);
    Ok(())
}

fn any_user_window_visible(app: &AppHandle) -> bool {
    app.webview_windows()
        .iter()
        .any(|(label, w)| app_exit::is_user_window(label) && w.is_visible().unwrap_or(false))
}

fn current_line() -> String {
    let name = PROVIDER_NAME
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .unwrap_or_else(|| "Autopilot".to_string());
    let status = LAST_STATUS.lock().unwrap_or_else(|e| e.into_inner());
    match status.as_ref() {
        Some(value) => tray_line(value, &name),
        None => tray_line(&Value::Null, &name),
    }
}

fn desired_tray(app: &AppHandle) -> TrayView {
    let booted = app_exit::main_revealed() || BACKGROUND_STARTED.load(Ordering::Acquire);
    let mode = tray_mode(any_user_window_visible(app), keep_alive_now(app), booted);
    let kind = tray_menu_kind(mode, crate::services::update_watch::update_ready());
    let line = if mode == TrayMode::Windowless {
        current_line()
    } else {
        String::new()
    };
    TrayView { kind, line }
}

/// Posts one closure to the main thread, which reads the latest state and
/// applies it. Nothing is locked across a menu call.
pub fn refresh_tray(app: &AppHandle) {
    if SHUTDOWN.load(Ordering::Acquire) || app.try_state::<TrayParts>().is_none() {
        return;
    }
    let on_main = app.clone();
    if let Err(e) = app.run_on_main_thread(move || apply_tray(&on_main)) {
        log::debug!("[Background] tray refresh not posted: {e}");
    }
}

fn apply_tray(app: &AppHandle) {
    if SHUTDOWN.load(Ordering::Acquire) {
        return;
    }
    let Some(parts) = app.try_state::<TrayParts>() else {
        return;
    };
    let want = desired_tray(app);
    let applied = parts
        .applied
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    if applied.as_ref() == Some(&want) {
        return;
    }
    let mut ok = true;
    if applied.as_ref().map(|a| a.kind) != Some(want.kind) {
        let menu = match want.kind {
            MenuKind::Normal => &parts.normal,
            MenuKind::Windowless => &parts.windowless,
            MenuKind::WindowlessUpdate => &parts.windowless_update,
        };
        if let Err(e) = parts.tray.set_menu(Some(menu.clone())) {
            log::warn!("[Background] tray menu not set: {e}");
            ok = false;
        }
    }
    if ok
        && want.kind != MenuKind::Normal
        && applied.as_ref().map(|a| a.line.as_str()) != Some(want.line.as_str())
    {
        for item in &parts.status_items {
            if let Err(e) = item.set_text(menu_text(&want.line)) {
                log::warn!("[Background] tray line not set: {e}");
                ok = false;
            }
        }
    }
    if ok {
        let tooltip = if want.kind == MenuKind::Normal {
            "StreamNook".to_string()
        } else {
            format!("StreamNook: {}", want.line)
        };
        if let Err(e) = parts.tray.set_tooltip(Some(tooltip)) {
            log::debug!("[Background] tray tooltip not set: {e}");
        }
        *parts.applied.lock().unwrap_or_else(|e| e.into_inner()) = Some(want);
    }
}

// ---------------------------------------------------------------- the command

#[derive(Debug, Clone, Serialize)]
pub struct BackgroundState {
    /// "on" | "off" | "blocked" | "other_copy" | "error" | "unsupported"
    pub autostart: String,
    pub autostart_error: Option<String>,
    pub keep_alive: bool,
    pub platform_ok: bool,
    pub system_notifications: bool,
}

/// What the Autopilot page shows on its rows: the real login-entry state, and
/// whether StreamNook would stay running if closed now.
#[tauri::command]
pub fn get_background_state(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<BackgroundState, String> {
    let autostart = crate::services::autostart::state();
    let system_notifications = {
        let s = state.settings.lock().map_err(|e| e.to_string())?;
        s.live_notifications.enabled && s.live_notifications.use_native_notifications
    };
    Ok(BackgroundState {
        autostart: autostart.code().to_string(),
        autostart_error: autostart.error(),
        keep_alive: keep_alive(&app),
        platform_ok: platform_ok(true),
        system_notifications,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin_host::registry::{GrantedCaps, InstalledPlugin};
    use serde_json::json;

    fn plugin(provides: &[&str], consent: &str, enabled: bool, kind: &str) -> InstalledPlugin {
        let mut credential_consent = std::collections::HashMap::new();
        credential_consent.insert("twitch.android".to_string(), consent.to_string());
        InstalledPlugin {
            id: "community.drops-farmer".into(),
            name: "Autopilot".into(),
            version: "0.6.0".into(),
            author: "StreamNook".into(),
            tier: "C".into(),
            description: String::new(),
            homepage: None,
            enabled,
            source: "local-dev".into(),
            kind: kind.into(),
            dir: String::new(),
            entry: String::new(),
            ui_entry: None,
            args: vec![],
            granted: GrantedCaps {
                provides: provides.iter().map(|s| s.to_string()).collect(),
                ..Default::default()
            },
            credential_consent,
        }
    }

    // 21
    #[test]
    fn wants_keep_alive_needs_all_inputs() {
        assert!(wants_keep_alive(true, true, Provider::Ready, false, true));
        assert!(!wants_keep_alive(false, true, Provider::Ready, false, true));
        assert!(!wants_keep_alive(true, false, Provider::Ready, false, true));
        assert!(!wants_keep_alive(
            true,
            true,
            Provider::Unknown,
            false,
            true
        ));
        assert!(!wants_keep_alive(
            true,
            true,
            Provider::NotReady,
            false,
            true
        ));
        assert!(!wants_keep_alive(true, true, Provider::Ready, true, true));
        assert!(!wants_keep_alive(true, true, Provider::Ready, false, false));
    }

    // 22
    #[test]
    fn stays_alive_needs_no_code_and_always_or_keep_alive() {
        use CloseToTrayMode::*;
        assert!(stays_alive(Always, None, false));
        assert!(stays_alive(WithPopouts, None, true));
        assert!(stays_alive(Never, None, true));
        assert!(!stays_alive(WithPopouts, None, false));
        assert!(!stays_alive(Never, None, false));
        assert!(!stays_alive(Always, Some(0), true));
        assert!(!stays_alive(WithPopouts, Some(0), true));
    }

    // 23
    #[test]
    fn close_hides_under_keep_alive() {
        use CloseToTrayMode::*;
        assert!(close_hides(Never, false, true));
        assert!(close_hides(Never, true, true));
        assert!(close_hides(WithPopouts, false, true));
        assert!(close_hides(WithPopouts, true, false));
        assert!(!close_hides(WithPopouts, false, false));
        assert!(!close_hides(Never, true, false));
        assert!(close_hides(Always, false, false));
    }

    // 22a
    #[test]
    fn reevaluate_exits_only_after_staying_for_keep_alive() {
        assert!(exit_due_from(true, false, false));
        // An ordinary close never set the flag: no notice, no exit.
        assert!(!exit_due_from(false, false, false));
        assert!(!exit_due_from(true, true, false));
        assert!(!exit_due_from(true, false, true));
    }

    // 22c
    #[test]
    fn the_fallback_destroy_skips_a_reopened_main() {
        assert!(fallback_destroy_due(4, 4, true, true, false));
        // A show bumped the lifetime.
        assert!(!fallback_destroy_due(4, 5, true, true, false));
        assert!(!fallback_destroy_due(4, 4, false, false, false));
        assert!(!fallback_destroy_due(4, 4, true, false, false));
        assert!(!fallback_destroy_due(4, 4, true, true, true));
    }

    // 22g (pure part)
    #[test]
    fn teardown_stops_when_a_new_main_exists_or_the_lifetime_moved_or_shutdown() {
        assert!(teardown_current(false, 3, 2, false));
        assert!(!teardown_current(true, 3, 2, false));
        // A show (recreate) moved the lifetime on.
        assert!(!teardown_current(false, 4, 2, false));
        assert!(!teardown_current(false, 3, 2, true));
        assert!(!teardown_current(false, 2, 2, false));
        assert!(!teardown_current(false, 0, u64::MAX, false));
    }

    // 22h (pure part)
    #[test]
    fn only_a_keep_alive_close_ends_the_session() {
        assert!(keep_alive_close(2, 2));
        // A stale tag from an older lifetime.
        assert!(!keep_alive_close(1, 2));
        // No close ever tagged: the first main (lifetime 0) never matches.
        assert!(!keep_alive_close(u64::MAX, 0));
    }

    // 22k (pure part)
    #[test]
    fn the_last_popout_stays_for_keep_alive() {
        use crate::services::app_exit::{exit_when_last_popout_closes, MainWindow};
        // Main hidden in the tray, a helper still open: the arm asks, and a
        // keep-alive stay holds the app.
        assert!(exit_when_last_popout_closes(
            MainWindow::Hidden,
            true,
            true,
            false
        ));
        assert!(stays_alive(CloseToTrayMode::WithPopouts, None, true));
        // No stay: the arm exits.
        assert!(!stays_alive(CloseToTrayMode::WithPopouts, None, false));
        // Under Always the arm never asks.
        assert!(!exit_when_last_popout_closes(
            MainWindow::Hidden,
            true,
            true,
            true
        ));
        // A destroyed main counts as gone.
        assert!(exit_when_last_popout_closes(
            MainWindow::Gone,
            true,
            false,
            false
        ));
    }

    // 22l
    #[test]
    fn exit_requested_decision_cases() {
        use ExitVerdict::*;
        let never = || -> Option<bool> { panic!("recorded must not be read") };
        let never_compute = || -> bool { panic!("compute must not run") };
        // A code always proceeds, even with the flag set.
        assert_eq!(
            exit_requested_decision(Some(0), true, never, never_compute),
            Proceed
        );
        // The flag stays and re-evaluates, computing nothing.
        assert_eq!(
            exit_requested_decision(None, true, never, never_compute),
            StayAndReevaluate
        );
        // A recorded decision is used.
        assert_eq!(
            exit_requested_decision(None, false, || Some(true), never_compute),
            Stay
        );
        assert_eq!(
            exit_requested_decision(None, false, || Some(false), never_compute),
            Proceed
        );
        // Nothing recorded (or its label differed: the take returns None): compute.
        assert_eq!(exit_requested_decision(None, false, || None, || true), Stay);
        assert_eq!(
            exit_requested_decision(None, false, || None, || false),
            Proceed
        );
    }

    // 22m (pure part)
    #[test]
    fn a_flip_before_the_flag_still_exits() {
        // keep_alive was true at the read, so the app stays and the flag is stored.
        let stay = stays_alive(CloseToTrayMode::WithPopouts, None, true);
        assert!(stay);
        let flag = stay;
        // It flipped to false after the read: the re-check after the store exits.
        assert!(exit_due_from(flag, false, false));
    }

    // 22f
    #[test]
    fn exit_steps_recheck_before_and_after_the_notice() {
        use ExitNext::*;
        use ExitStep::*;
        // Check, notice, check, exit.
        assert_eq!(next_exit_step(Recheck, true), Go(Notice));
        assert_eq!(next_exit_step(Notice, true), Go(FinalCheck));
        assert_eq!(next_exit_step(FinalCheck, true), Go(Exit));
        // A failed first check never reaches the notice.
        assert_eq!(next_exit_step(Recheck, false), Abort);
        // The notice does not decide; the third check does.
        assert_eq!(next_exit_step(Notice, false), Go(FinalCheck));
        assert_eq!(next_exit_step(FinalCheck, false), Abort);
    }

    #[test]
    fn a_failed_check_reruns_after_a_second_compare_exchange() {
        let pending = AtomicBool::new(false);
        // The first task wins; a second never does.
        assert!(try_begin_exit(&pending));
        assert!(!try_begin_exit(&pending));
        // The check failed and nothing is due now: give up the place.
        assert!(!finish_failed_round(&pending, || false));
        assert!(!pending.load(Ordering::Acquire));
        // Fail, the inputs flip, the second exchange is won, exit goes again.
        assert!(try_begin_exit(&pending));
        assert!(finish_failed_round(&pending, || true));
        assert!(pending.load(Ordering::Acquire));
        // Another task already took the place: this one stops.
        assert!(!try_begin_exit(&pending));
    }

    // 22b
    #[test]
    fn background_launch_decision_cases() {
        use CloseToTrayMode::*;
        use LaunchDecision::*;
        use Provider::*;
        // Not a background launch: a normal window.
        assert_eq!(
            background_launch_decision(false, true, true, Ready, WithPopouts),
            Visible
        );
        // Keep-alive: hide, and the staying flag is set unless Always.
        assert_eq!(
            background_launch_decision(true, true, true, Ready, WithPopouts),
            Hide {
                sets_staying_flag: true
            }
        );
        assert_eq!(
            background_launch_decision(true, true, true, Ready, Never),
            Hide {
                sets_staying_flag: true
            }
        );
        assert_eq!(
            background_launch_decision(true, true, true, Ready, Always),
            Hide {
                sets_staying_flag: false
            }
        );
        // Keep-alive true but the switches off (a stale input): the entry goes, no Hide.
        assert_eq!(
            background_launch_decision(true, true, false, Ready, WithPopouts),
            RemoveAndExit
        );
        // An unreadable registry is never read as "not ready".
        assert_eq!(
            background_launch_decision(true, false, true, Unknown, WithPopouts),
            Visible
        );
        assert_eq!(
            background_launch_decision(true, false, false, Unknown, WithPopouts),
            Visible
        );
        assert_eq!(
            background_launch_decision(true, false, true, NotReady, WithPopouts),
            RemoveAndExit
        );
        // Ready but a switch is off (start_at_login off, say, while another
        // copy owns the entry: `autostart::remove` removes nothing that is
        // not this copy's, and the app exits).
        assert_eq!(
            background_launch_decision(true, false, false, Ready, WithPopouts),
            RemoveAndExit
        );
        // Linux, switches off, no tray host: the known provider decides first.
        assert_eq!(
            background_launch_decision(true, false, false, Ready, WithPopouts),
            RemoveAndExit
        );
        assert_eq!(
            background_launch_decision(true, false, false, NotReady, WithPopouts),
            RemoveAndExit
        );
        // Wanted and ready, but no Linux tray host: a visible launch, entry kept.
        assert_eq!(
            background_launch_decision(true, false, true, Ready, WithPopouts),
            Visible
        );
    }

    // 22e
    #[test]
    fn a_posted_linux_decision_is_dropped_when_main_exists() {
        assert!(launch_decision_dropped(true));
        assert!(!launch_decision_dropped(false));
        assert!(!launch_wait_done(false, 29));
        assert!(launch_wait_done(false, 30));
        assert!(launch_wait_done(true, 0));
    }

    // 24
    #[test]
    fn tray_line_maps_every_status() {
        let working = json!({ "is_active": true, "drop_name": "Golden Frog Badge",
            "current_minutes": 7, "required_minutes": 15 });
        assert_eq!(
            tray_line(&working, "Autopilot"),
            "Collecting Golden Frog Badge, 7 of 15 min"
        );
        assert_eq!(
            tray_line(&json!({ "paused": true }), "Autopilot"),
            "Missing badges paused"
        );
        assert_eq!(
            tray_line(&json!({ "blocked": "signed_out" }), "Autopilot"),
            "Sign in to Drops to collect badges"
        );
        // The consent line carries the provider's registry name, whatever it is.
        assert_eq!(
            tray_line(&json!({ "blocked": "consent" }), "Autopilot"),
            "Autopilot needs your OK to use Twitch"
        );
        assert_eq!(
            tray_line(&json!({ "blocked": "consent" }), "Badge Bot"),
            "Badge Bot needs your OK to use Twitch"
        );
        assert_eq!(
            tray_line(&json!({ "is_active": false }), "Autopilot"),
            "Waiting for missing badges"
        );
        assert_eq!(
            tray_line(&Value::Null, "Autopilot"),
            "Waiting for missing badges"
        );
        // A very long badge name is capped.
        let long = json!({ "is_active": true, "drop_name": "x".repeat(200),
            "current_minutes": 1, "required_minutes": 2 });
        assert_eq!(tray_line(&long, "Autopilot").chars().count(), 64);
        assert!(tray_line(&long, "Autopilot").ends_with('\u{2026}'));
    }

    #[test]
    fn menu_text_doubles_ampersands() {
        assert_eq!(
            menu_text("Collecting Fish & Chips, 1 of 2 min"),
            "Collecting Fish && Chips, 1 of 2 min"
        );
        assert_eq!(menu_text("plain"), "plain");
    }

    // 25
    #[test]
    fn tray_mode_follows_windows_and_keep_alive() {
        assert_eq!(tray_mode(false, true, true), TrayMode::Windowless);
        assert_eq!(tray_mode(true, true, true), TrayMode::Normal);
        // Keep-alive turned off: back to today's menu.
        assert_eq!(tray_mode(false, false, true), TrayMode::Normal);
        // Still behind the boot gate.
        assert_eq!(tray_mode(false, true, false), TrayMode::Normal);
        assert_eq!(tray_menu_kind(TrayMode::Normal, true), MenuKind::Normal);
        assert_eq!(
            tray_menu_kind(TrayMode::Windowless, false),
            MenuKind::Windowless
        );
        assert_eq!(
            tray_menu_kind(TrayMode::Windowless, true),
            MenuKind::WindowlessUpdate
        );
    }

    // 27
    #[test]
    fn background_launch_reads_the_flag() {
        let args = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(argv_is_background(&args(&[
            "streamnook.exe",
            "--background"
        ])));
        assert!(!argv_is_background(&args(&["streamnook.exe"])));
        assert!(!argv_is_background(&args(&["--background"])));
        assert!(!argv_is_background(&args(&[
            "streamnook.exe",
            "--backgrounds"
        ])));
    }

    #[test]
    fn a_second_background_launch_is_ignored() {
        // The single-instance callbacks use the same reader on the second
        // launch's argv: a plain second launch still shows main.
        let second = vec!["streamnook.exe".to_string(), "--background".to_string()];
        assert!(argv_is_background(&second));
        let plain = vec![
            "streamnook.exe".to_string(),
            "streamnook://watch/x".to_string(),
        ];
        assert!(!argv_is_background(&plain));
    }

    // 28
    #[test]
    fn provider_ready_needs_missing_badges_and_always_consent() {
        let mut reg = Registry::default();
        assert_eq!(registry_provider(&reg), (Provider::NotReady, None));
        reg.plugins
            .push(plugin(&["drops.missing-badges"], "always", true, "process"));
        assert_eq!(
            registry_provider(&reg),
            (
                Provider::Ready,
                Some((
                    "community.drops-farmer".to_string(),
                    "Autopilot".to_string()
                ))
            )
        );
        for (provides, consent, enabled, kind) in [
            (&["drops.automation"][..], "always", true, "process"),
            (&["drops.missing-badges"][..], "ask", true, "process"),
            (&["drops.missing-badges"][..], "revoked", true, "process"),
            (&["drops.missing-badges"][..], "always", false, "process"),
            (&["drops.missing-badges"][..], "always", true, "ui"),
        ] {
            let mut reg = Registry::default();
            reg.plugins.push(plugin(provides, consent, enabled, kind));
            assert_eq!(
                registry_provider(&reg).0,
                Provider::NotReady,
                "{provides:?} {consent} {enabled} {kind}"
            );
        }
    }

    // 29
    #[test]
    fn only_the_provider_status_is_kept() {
        assert!(keeps_status(
            Some("community.drops-farmer"),
            "community.drops-farmer"
        ));
        assert!(!keeps_status(
            Some("community.drops-farmer"),
            "other.plugin"
        ));
        assert!(!keeps_status(None, "community.drops-farmer"));
        assert!(status_is_paused(&json!({ "paused": true })));
        assert!(!status_is_paused(&json!({ "paused": false })));
        assert!(!status_is_paused(&json!({})));
    }

    // 32a
    #[test]
    fn deliver_only_when_main_is_not_on_screen() {
        assert!(should_deliver_notice(false));
        assert!(!should_deliver_notice(true));
    }

    // 32b
    #[test]
    fn the_told_flag_is_saved_only_when_delivered() {
        assert!(!told_flag_should_save(None, true));
        assert!(told_flag_should_save(Some("system"), true));
        assert!(!told_flag_should_save(Some("system"), false));
        assert!(told_flag_should_save(Some("multichat-1"), false));
    }
}

/// Source pins: each names a full statement, in the order the design needs.
/// Pins never spell the emit call (`tests/event_registry.rs` counts that text).
#[cfg(test)]
mod pins {
    const LIB_RS: &str = include_str!("../lib.rs");
    const APP_EXIT_RS: &str = include_str!("app_exit.rs");
    const THIS_RS: &str = include_str!("background_mode.rs");
    const UPDATE_WATCH_RS: &str = include_str!("update_watch.rs");
    const BROKER_RS: &str = include_str!("../plugin_host/broker.rs");
    const NSH: &str = include_str!("../../windows/installer-hooks.nsh");
    const AUTOSTART_RS: &str = include_str!("autostart.rs");

    /// CRLF folded to LF, then every run of whitespace removed.
    fn squash(src: &str) -> String {
        src.replace("\r\n", "\n").split_whitespace().collect()
    }

    /// The squashed text from `start` up to the next `end` after it.
    fn between(src: &str, start: &str, end: &str) -> String {
        let s = squash(src);
        let (a, b) = (squash(start), squash(end));
        let from = s.find(&a).unwrap_or_else(|| panic!("`{start}` not found"));
        let rest = &s[from..];
        let to = rest[a.len()..]
            .find(&b)
            .map(|i| i + a.len())
            .unwrap_or_else(|| panic!("`{end}` not found after `{start}`"));
        rest[..to].to_string()
    }

    /// The squashed text of a top-level function: signature to the closing
    /// brace at column 0.
    fn fn_text(src: &str, signature: &str) -> String {
        let src = src.replace("\r\n", "\n");
        let from = src
            .find(signature)
            .unwrap_or_else(|| panic!("`{signature}` not found"));
        let rest = &src[from..];
        let end = rest.find("\n}\n").unwrap_or(rest.len());
        squash(&rest[..end])
    }

    /// Every part appears, each after the one before.
    fn in_order(text: &str, parts: &[&str]) {
        let mut at = 0;
        for part in parts {
            let part = squash(part);
            let i = text[at..]
                .find(&part)
                .unwrap_or_else(|| panic!("`{part}` missing or out of order in:\n{text}"));
            at += i + part.len();
        }
    }

    fn has(text: &str, part: &str) -> bool {
        text.contains(&squash(part))
    }

    const EXIT_REQUESTED_START: &str =
        "if let tauri::RunEvent::ExitRequested { code, api, .. } = &event {";
    const EXIT_START: &str = "if let tauri::RunEvent::Exit = event {";
    const CLOSE_REQUESTED_START: &str = "if let WindowEvent::CloseRequested { api, .. } = event {";
    const POPOUT_ARM_START: &str =
        "} else if services::app_exit::is_user_window(&label) && label != \"main\" {";

    // 22 and 23
    #[test]
    fn lib_rs_uses_the_close_rules() {
        let exit = between(LIB_RS, EXIT_REQUESTED_START, EXIT_START);
        assert!(has(&exit, "exit_requested_decision("));
        assert!(has(
            &exit,
            "stays_alive(close_mode, None, keep_alive(app_handle))"
        ));
        let close = between(LIB_RS, CLOSE_REQUESTED_START, POPOUT_ARM_START);
        assert!(has(
            &close,
            "let hide_to_tray = services::background_mode::close_hides(mode, popouts_open, keep_alive);"
        ));
        // The popout arm asks the shared decision before it exits.
        let popout = between(
            LIB_RS,
            POPOUT_ARM_START,
            ".build(tauri::generate_context!())",
        );
        in_order(
            &popout,
            &[
                "on_user_windows_gone(&app_handle, &label)",
                "services::app_exit::exit(",
            ],
        );
    }

    // 22h (CloseRequested tag and note_main_destroyed)
    #[test]
    fn a_keep_alive_close_is_tagged_and_destroy_reads_the_tag() {
        let close = between(LIB_RS, CLOSE_REQUESTED_START, POPOUT_ARM_START);
        in_order(
            &close,
            &[
                "let keep_alive = services::background_mode::keep_alive(&app_handle);",
                "if keep_alive {",
                "services::background_mode::note_keep_alive_close(&app_handle);",
                "api.prevent_close();",
            ],
        );
        let destroyed = fn_text(THIS_RS, "pub fn note_main_destroyed(");
        in_order(
            &destroyed,
            &[
                "let ended = MAIN_LIFETIME.fetch_add(1, Ordering::AcqRel);",
                "keep_alive_close(KEEP_ALIVE_HIDE.load(Ordering::Acquire), ended)",
                "main_teardown(app, ended, keep_alive_close)",
            ],
        );
        let tag = fn_text(THIS_RS, "pub fn note_keep_alive_close(");
        in_order(
            &tag,
            &[
                "KEEP_ALIVE_HIDE.store(lifetime, Ordering::Release);",
                "arm_fallback_destroy(",
            ],
        );
    }

    // 22c
    #[test]
    fn main_lifetime_is_bumped_on_show_and_on_destroy() {
        let show = fn_text(LIB_RS, "pub(crate) fn show_main_window(");
        assert!(has(
            &show,
            "services::background_mode::note_main_window_shown();"
        ));
        let shown = fn_text(THIS_RS, "pub fn note_main_window_shown(");
        assert!(has(&shown, "MAIN_LIFETIME.fetch_add(1, Ordering::AcqRel);"));
        let destroyed = between(
            LIB_RS,
            "if label == \"main\" {\n                    use tauri::Emitter;",
            POPOUT_ARM_START,
        );
        assert!(has(
            &destroyed,
            "services::background_mode::note_main_destroyed(&app_handle);"
        ));
    }

    // 22j and 22m (main arm)
    #[test]
    fn the_main_destroyed_arm_decides_once() {
        let arm = between(
            LIB_RS,
            "services::background_mode::note_main_destroyed(&app_handle);",
            "services::background_mode::refresh_tray(&app_handle);",
        );
        in_order(
            &arm,
            &[
                "!services::background_mode::staying_for_keep_alive()",
                "if !services::background_mode::on_user_windows_gone(&app_handle, \"main\") {",
                "services::app_exit::exit(",
            ],
        );
        // The teardown is spawned before and regardless of the decision.
        let all = squash(LIB_RS);
        let note = all
            .find(&squash(
                "services::background_mode::note_main_destroyed(&app_handle);",
            ))
            .unwrap();
        let gone = all
            .find(&squash(
                "services::background_mode::on_user_windows_gone(&app_handle, \"main\")",
            ))
            .unwrap();
        assert!(note < gone);
    }

    // 22j
    #[test]
    fn a_show_clears_the_decision_and_the_flag() {
        let shown = fn_text(APP_EXIT_RS, "pub fn note_main_shown(");
        in_order(
            &shown,
            &[
                "clear_close_decision();",
                "crate::services::background_mode::note_shown();",
            ],
        );
        let take = fn_text(APP_EXIT_RS, "pub fn take_close_decision(");
        assert!(has(&take, "LAST_DESTROYED"));
        let flag = fn_text(THIS_RS, "pub fn note_shown(");
        assert!(has(
            &flag,
            "STAYING_FOR_KEEP_ALIVE.swap(false, Ordering::AcqRel)"
        ));
    }

    // 22j (shutdown)
    #[test]
    fn exit_sets_shutdown() {
        let begin = fn_text(APP_EXIT_RS, "pub fn begin_shutdown(");
        assert!(has(&begin, "SHUTDOWN.store(true, Ordering::Release);"));
        let exit = fn_text(APP_EXIT_RS, "pub fn exit(");
        in_order(&exit, &["begin_shutdown();", "app.exit(code);"]);
        let requested = between(LIB_RS, EXIT_REQUESTED_START, EXIT_START);
        assert!(has(
            &requested,
            "} else { services::app_exit::begin_shutdown(); }"
        ));
        let exit_arm = between(LIB_RS, EXIT_START, "let shutdown_started");
        in_order(
            &exit_arm,
            &[
                "services::app_exit::begin_shutdown();",
                "services::app_exit::on_run_exit();",
            ],
        );
    }

    // 22k
    #[test]
    fn the_stay_store_is_followed_by_a_recheck() {
        let gone = fn_text(THIS_RS, "pub fn on_user_windows_gone(");
        assert!(has(
            &gone,
            "record_stay(app, mode != CloseToTrayMode::Always);"
        ));
        let stay = fn_text(THIS_RS, "pub fn record_stay(");
        in_order(
            &stay,
            &[
                "STAYING_FOR_KEEP_ALIVE.store(true, Ordering::Release);",
                "reevaluate(app);",
            ],
        );
    }

    // 22l
    #[test]
    fn the_flag_check_sits_inside_the_code_none_branch() {
        let decision = fn_text(THIS_RS, "pub fn exit_requested_decision(");
        in_order(
            &decision,
            &[
                "if code.is_some() {",
                "if flag {",
                "recorded().unwrap_or_else(compute)",
            ],
        );
    }

    // 22m
    #[test]
    fn the_popout_arm_covers_every_user_window_and_gates_the_emit() {
        let lib = squash(LIB_RS);
        assert!(lib.contains(&squash(
            "} else if services::app_exit::is_user_window(&label) && label != \"main\" {"
        )));
        let popout = between(
            LIB_RS,
            POPOUT_ARM_START,
            ".build(tauri::generate_context!())",
        );
        in_order(
            &popout,
            &[
                "if label.starts_with(\"multichat-\") || label.starts_with(\"overlay-\") {",
                "\"multichat-popout-closed\", &label",
            ],
        );
        // CloseRequested's popouts_open still matches only popouts.
        let close = between(LIB_RS, CLOSE_REQUESTED_START, POPOUT_ARM_START);
        assert!(has(
            &close,
            ".any(|(l, _)| l.starts_with(\"multichat-\") || l.starts_with(\"overlay-\"))"
        ));
    }

    // 22b and 22m
    #[test]
    fn the_launch_decision_stores_through_the_shared_helper() {
        let apply = fn_text(THIS_RS, "fn apply_launch_decision(");
        in_order(
            &apply,
            &[
                "KEEP_ALIVE_HIDE.store(MAIN_LIFETIME.load(Ordering::Acquire), Ordering::Release);",
                "record_stay(app, sets_staying_flag);",
                "main.destroy()",
            ],
        );
    }

    // 22e
    #[test]
    fn the_linux_poster_checks_for_main_first() {
        let post = fn_text(THIS_RS, "fn post_linux_launch_decision(");
        in_order(
            &post,
            &["launch_decision_dropped(", "apply_launch_decision("],
        );
    }

    // 22f
    #[test]
    fn reevaluate_uses_the_compare_exchange_and_the_step_machine() {
        let re = fn_text(THIS_RS, "pub fn reevaluate(");
        assert!(has(
            &re,
            "if exit_due(app) && try_begin_exit(&EXIT_PENDING) {"
        ));
        let begin = fn_text(THIS_RS, "pub fn try_begin_exit(");
        assert!(has(
            &begin,
            "compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)"
        ));
        let steps = fn_text(THIS_RS, "async fn run_exit_steps(");
        assert!(has(&steps, "next_exit_step(step, due)"));
        let task = fn_text(THIS_RS, "async fn exit_task(");
        assert!(has(&task, "finish_failed_round(&EXIT_PENDING"));
    }

    // 22g: the teardown reads the current lifetime, never a constant
    #[test]
    fn the_teardown_is_wired_to_the_lifetime_check() {
        let teardown = fn_text(THIS_RS, "async fn main_teardown(");
        in_order(
            &teardown,
            &[
                "let still_current = move || teardown_still_current(&check, ended);",
                "crate::services::watch_start::stop_for_closed_main(app, still_current, end_session).await;",
            ],
        );
        let current = fn_text(THIS_RS, "pub fn teardown_still_current(");
        assert!(has(
            &current,
            "teardown_current(
                crate::services::window_visibility::window_open(app, \"main\"),
                MAIN_LIFETIME.load(Ordering::Acquire),
                ended,
                SHUTDOWN.load(Ordering::Acquire),
            )"
        ));
    }

    // 3-M8: re-evaluation never asks the Linux tray host, entering windowless mode does
    #[test]
    fn only_entering_windowless_mode_asks_the_tray_host() {
        let now = fn_text(THIS_RS, "fn keep_alive_now(");
        assert!(has(
            &now,
            // No closing paren: rustfmt may wrap the call and add a trailing comma.
            "wants_keep_alive(collect, keep, provider(), status_paused(), platform_ok(false)"
        ));
        let entering = fn_text(THIS_RS, "pub fn keep_alive(");
        assert!(has(
            &entering,
            "wants_keep_alive(collect, keep, provider(), status_paused(), platform_ok(true)"
        ));
    }

    // 2-C1: an unknown provider never removes the login entry (Windows worker)
    #[test]
    fn the_windows_worker_removes_only_with_removal_allowed() {
        let a = squash(AUTOSTART_RS);
        assert!(a.contains(&squash("if ours && w.removal_allowed && w.writes {")));
        // Remove deletes only an entry that is this copy's or stale.
        let remove = between(
            AUTOSTART_RS,
            "pub(super) fn remove() -> Result<(), String> {",
            "#[cfg(target_os = \"macos\")]",
        );
        assert!(has(
            &remove,
            ".is_some_and(|c| entry_owner(&c, &exe, exists) != Owner::Other);"
        ));
        assert!(has(&remove, "if ours { delete(RUN_KEY, ENTRY_NAME) }"));
    }

    // M3: the launch decision reads the switches, whole call
    #[test]
    fn the_launch_decision_is_given_the_switches() {
        let run = fn_text(THIS_RS, "fn decision_now(");
        assert!(has(
            &run,
            "background_launch_decision(true, keep_alive(app), collect && keep && login, provider(), close_mode(app),)"
        ));
    }

    // 27
    #[test]
    fn both_single_instance_callbacks_ignore_a_background_launch() {
        let lib = squash(LIB_RS);
        let needle = squash("if services::background_mode::argv_is_background(&argv) {");
        assert_eq!(lib.matches(&needle).count(), 2);
    }

    // 22a (call sites in files this lane does not own; 7.3b pins them too)
    #[test]
    fn re_evaluation_triggers_exist() {
        let drops = squash(include_str!("../commands/drops.rs"));
        assert!(drops.contains(&squash("on_drops_settings_saved(")));
        assert!(squash(include_str!("../plugin_host/registry.rs"))
            .contains(&squash("background_mode::note_registry(")));
        assert!(squash(include_str!("../plugin_host/process.rs"))
            .contains(&squash("background_mode::note_provider_stopped(")));
        assert!(squash(BROKER_RS).contains(&squash("background_mode::note_drops_status(")));
    }

    // 32a
    #[test]
    fn the_broker_hands_notices_to_the_gate() {
        assert!(squash(BROKER_RS).contains(&squash("background_mode::deliver_plugin_notice(")));
        let deliver = fn_text(THIS_RS, "pub async fn deliver_plugin_notice(");
        assert!(has(
            &deliver,
            "if !should_deliver_notice(main_on_screen(&app)) {"
        ));
    }

    // update ready
    #[test]
    fn the_update_check_feeds_the_tray() {
        let w = squash(UPDATE_WATCH_RS);
        assert!(w.contains(&squash(
            "UPDATE_READY.store(status.update_available, Ordering::Release);"
        )));
        assert!(w.contains(&squash(
            "crate::services::background_mode::refresh_tray(&app);"
        )));
    }

    // NSIS
    #[test]
    fn the_uninstaller_removes_the_login_entry() {
        let hook = squash(NSH);
        in_order(
            &hook,
            &[
                "${If} $UpdateMode <> 1",
                "DeleteRegValue HKCU \"Software\\Microsoft\\Windows\\CurrentVersion\\Run\" \"StreamNook\"",
                "DeleteRegValue HKCU \"Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run\" \"StreamNook\"",
                "${EndIf}",
            ],
        );
    }

    // the event is emitted from one place
    #[test]
    fn the_state_event_is_sent_from_this_file_only() {
        assert!(has(
            &fn_text(THIS_RS, "pub fn emit_state_changed("),
            "(STATE_EVENT, ())"
        ));
    }
}
