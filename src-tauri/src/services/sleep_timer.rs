//! The phones' sleep timer: stop watching after a chosen time, for streams people fall asleep to.
//!
//! The core owns it so both phones behave the same, and so it keeps running with the screen locked
//! and the app in the background (background play keeps the process alive; a paused app has nothing
//! to stop). `FADE_SECS` before the end it announces the fade (`phase: "fading"`) and the player
//! lowers its own audio; at the end it announces `"fired"` and stops playback itself, the same stop
//! as the player's own (relay, watch session, drops and points), so the network stops even if the
//! page is slow to react. The page then releases its player and its Now Playing or media
//! notification, as after any stop. It never quits the app.
//!
//! One timer per run, not a setting. A stop the viewer makes cancels it; switching channels keeps it.
//! Every change is pushed as `sleep-timer://state`, and every label is worded here.

use crate::rt::AppHandle;
use once_cell::sync::Lazy;
use serde::Serialize;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{Emitter, Manager};

/// How long the audio fades before playback stops.
pub const FADE_SECS: u64 = 10;
/// The longest timer: a day.
const MAX_MINUTES: u32 = 24 * 60;
/// The menu's presets, in minutes.
const PRESETS: [u32; 4] = [15, 30, 60, 120];

#[derive(Debug, Clone, Copy, PartialEq)]
struct Timer {
    ends_at_ms: i64,
    generation: u64,
    fading: bool,
}

static TIMER: Lazy<Mutex<Option<Timer>>> = Lazy::new(|| Mutex::new(None));
static GENERATION: Lazy<Mutex<u64>> = Lazy::new(|| Mutex::new(0));

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Preset {
    pub minutes: u32,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SleepTimerState {
    /// "off", "running", "fading" or "fired".
    pub phase: &'static str,
    pub ends_at_ms: Option<i64>,
    pub remaining_secs: Option<u64>,
    /// The menu row while one runs: "Stops in 42 min".
    pub row_label: Option<String>,
    /// How long the fade before the stop lasts.
    pub fade_secs: u64,
    pub presets: Vec<Preset>,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// "15 min", "1 h", "1 h 5 min", "less than a minute".
pub fn duration_label(secs: u64) -> String {
    if secs < 60 {
        return "less than a minute".to_string();
    }
    let minutes = secs.div_ceil(60);
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

fn presets() -> Vec<Preset> {
    PRESETS.iter().map(|&m| Preset { minutes: m, label: duration_label(u64::from(m) * 60) }).collect()
}

fn snapshot(timer: Option<Timer>, now: i64, phase_override: Option<&'static str>) -> SleepTimerState {
    let remaining = timer.map(|t| ((t.ends_at_ms - now).max(0) / 1000) as u64);
    SleepTimerState {
        phase: phase_override.unwrap_or(match timer {
            None => "off",
            Some(t) if t.fading => "fading",
            Some(_) => "running",
        }),
        ends_at_ms: timer.map(|t| t.ends_at_ms),
        remaining_secs: remaining,
        row_label: remaining.map(|r| format!("Stops in {}", duration_label(r))),
        fade_secs: FADE_SECS,
        presets: presets(),
    }
}

pub fn state() -> SleepTimerState {
    let timer = *TIMER.lock().unwrap_or_else(|e| e.into_inner());
    snapshot(timer, now_ms(), None)
}

fn push(app: &AppHandle, state: &SleepTimerState) {
    let _ = app.emit("sleep-timer://state", state);
}

/// Start (or restart) the timer for `minutes`, clamped to one minute through a day.
pub fn start(app: &AppHandle, minutes: u32) -> SleepTimerState {
    let minutes = minutes.clamp(1, MAX_MINUTES);
    let generation = {
        let mut g = GENERATION.lock().unwrap_or_else(|e| e.into_inner());
        *g += 1;
        *g
    };
    let ends_at_ms = now_ms() + i64::from(minutes) * 60_000;
    *TIMER.lock().unwrap_or_else(|e| e.into_inner()) = Some(Timer { ends_at_ms, generation, fading: false });
    log::info!("[SleepTimer] set for {}", duration_label(u64::from(minutes) * 60));
    let current = state();
    push(app, &current);
    let app = app.clone();
    tauri::async_runtime::spawn(async move { run(app, generation, ends_at_ms).await });
    current
}

/// Turn the timer off. A timer already off answers off and pushes nothing.
pub fn cancel(app: &AppHandle) -> SleepTimerState {
    let had = TIMER.lock().unwrap_or_else(|e| e.into_inner()).take().is_some();
    let current = state();
    if had {
        log::info!("[SleepTimer] off");
        push(app, &current);
    }
    current
}

fn still_current(generation: u64) -> bool {
    TIMER.lock().unwrap_or_else(|e| e.into_inner()).is_some_and(|t| t.generation == generation)
}

async fn run(app: AppHandle, generation: u64, ends_at_ms: i64) {
    let fade_at = ends_at_ms - (FADE_SECS as i64) * 1000;
    let wait = (fade_at - now_ms()).max(0) as u64;
    tokio::time::sleep(Duration::from_millis(wait)).await;
    {
        let mut timer = TIMER.lock().unwrap_or_else(|e| e.into_inner());
        match timer.as_mut() {
            Some(t) if t.generation == generation => t.fading = true,
            _ => return,
        }
    }
    push(&app, &state());
    let wait = (ends_at_ms - now_ms()).max(0) as u64;
    tokio::time::sleep(Duration::from_millis(wait)).await;
    if !still_current(generation) {
        return;
    }
    // Off before the stop, so the stop's own cancel finds nothing to cancel.
    TIMER.lock().unwrap_or_else(|e| e.into_inner()).take();
    log::info!("[SleepTimer] fired: stopping playback");
    push(&app, &snapshot(None, now_ms(), Some("fired")));
    let owner = app.clone();
    let app_state = owner.state::<crate::models::settings::AppState>();
    crate::services::watch_start::stop(app.clone(), app_state, false).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_read_as_words() {
        assert_eq!(duration_label(30), "less than a minute");
        assert_eq!(duration_label(60), "1 min");
        assert_eq!(duration_label(61), "2 min");
        assert_eq!(duration_label(15 * 60), "15 min");
        assert_eq!(duration_label(60 * 60), "1 h");
        assert_eq!(duration_label(65 * 60), "1 h 5 min");
        assert_eq!(duration_label(120 * 60), "2 h");
    }

    #[test]
    fn presets_are_worded() {
        let labels: Vec<String> = presets().into_iter().map(|p| p.label).collect();
        assert_eq!(labels, ["15 min", "30 min", "1 h", "2 h"]);
    }

    #[test]
    fn a_running_timer_says_when_it_stops() {
        let t = Timer { ends_at_ms: 10_000 + 42 * 60_000, generation: 1, fading: false };
        let s = snapshot(Some(t), 10_000, None);
        assert_eq!(s.phase, "running");
        assert_eq!(s.row_label.as_deref(), Some("Stops in 42 min"));
        let fading = snapshot(Some(Timer { fading: true, ..t }), 10_000, None);
        assert_eq!(fading.phase, "fading");
        assert_eq!(snapshot(None, 0, None).phase, "off");
        assert_eq!(snapshot(None, 0, Some("fired")).row_label, None);
    }
}
