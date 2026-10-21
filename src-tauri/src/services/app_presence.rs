//! What the phone shells tell the core about the app: in front or not, system
//! battery saver, and whether the main player is parked (paused past
//! earning_gate::PARK_AFTER). Rust pollers read it; the shells read it back
//! (`get_app_presence`, `app-presence`) so both UIs act on the core's truth.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use serde::Serialize;
use tauri::{Emitter, Manager};

use crate::rt::AppHandle;
use crate::services::window_visibility;

pub const EVENT: &str = "app-presence";

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct AppPresence {
    pub foreground: bool,
    pub low_power: bool,
    pub main_parked: bool,
    /// Unix ms the main player parked.
    pub parked_at: Option<u64>,
}

static LOW_POWER: AtomicBool = AtomicBool::new(false);
/// 0 = not parked.
static PARKED_AT: AtomicU64 = AtomicU64::new(0);

pub fn snapshot() -> AppPresence {
    let at = PARKED_AT.load(Ordering::Relaxed);
    AppPresence {
        foreground: !window_visibility::all_hidden(),
        low_power: low_power(),
        main_parked: at != 0,
        parked_at: (at != 0).then_some(at),
    }
}

pub fn low_power() -> bool {
    LOW_POWER.load(Ordering::Relaxed)
}

pub fn main_parked() -> bool {
    PARKED_AT.load(Ordering::Relaxed) != 0
}

/// Background and nothing earning: the only time earning-side pollers rest.
pub fn phone_quiet(app: &AppHandle) -> bool {
    window_visibility::phone_idle()
        && !app
            .try_state::<crate::models::settings::AppState>()
            .is_some_and(|s| s.watch_heartbeat.is_playing())
}

#[cfg(mobile)]
pub fn set_foreground(foreground: bool) {
    let changed = window_visibility::all_hidden() == foreground;
    window_visibility::set_app_foreground(foreground);
    if !foreground {
        // The last reliable moment before either OS may end the process.
        crate::flush_debounced_stores();
    }
    if changed {
        log::info!("[AppPresence] foreground={foreground}");
        announce();
    }
}

#[cfg(mobile)]
pub fn set_low_power(on: bool) {
    if LOW_POWER.swap(on, Ordering::Relaxed) != on {
        log::info!("[AppPresence] low_power={on}");
        announce();
    }
}

pub(crate) fn set_main_parked(parked: bool) {
    if main_parked() == parked {
        return;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(1);
    PARKED_AT.store(if parked { now.max(1) } else { 0 }, Ordering::Relaxed);
    announce();
}

fn announce() {
    if let Some(app) = crate::services::providers::app_handle() {
        let _ = app.emit(EVENT, snapshot());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_snapshot_keeps_the_field_names_the_shells_read() {
        let p = AppPresence { foreground: true, low_power: false, main_parked: true, parked_at: Some(5) };
        assert_eq!(
            serde_json::to_value(p).unwrap(),
            serde_json::json!({"foreground": true, "low_power": false, "main_parked": true, "parked_at": 5})
        );
    }
}
