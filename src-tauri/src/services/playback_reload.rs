//! Automatic live reloads, counted per channel.
//!
//! The player reloads a live stream on its own when it stalls (the frozen-
//! playhead watchdog) or gives up (the watch session's `Resume`). Each reload
//! fully resets the relay, so when the cause sits upstream the same thing
//! happens again: viewers saw the stream "keep refreshing" every half minute
//! until they stopped it themselves. This counts those reloads and, on the
//! second inside `WINDOW`, plays the channel through the stable whole-segment
//! path instead of the low-latency origin for `HOLD_OFF`. Every automatic
//! reload is logged at warn level with its reason and count, so a user's log
//! shows the loop without Detailed log on.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;

/// Reloads closer together than this are one bout of trouble.
const WINDOW: Duration = Duration::from_secs(5 * 60);
/// How long the low-latency origin stays off once reloads repeat.
const HOLD_OFF: Duration = Duration::from_secs(10 * 60);
/// The reload, within `WINDOW`, that turns low latency off.
const HOLD_OFF_AT: usize = 2;

static HISTORY: Lazy<Mutex<HashMap<String, VecDeque<Instant>>>> = Lazy::new(Default::default);

/// Record an automatic reload of `login` at `now`; returns how many fell
/// inside `WINDOW`, this one included.
fn record(history: &mut HashMap<String, VecDeque<Instant>>, login: &str, now: Instant) -> usize {
    let times = history.entry(login.to_ascii_lowercase()).or_default();
    while times.front().is_some_and(|t| now.duration_since(*t) > WINDOW) {
        times.pop_front();
    }
    times.push_back(now);
    times.len()
}

/// Note one automatic reload. Returns true when the low-latency origin is
/// now held off, so the reload about to happen uses the stable path.
pub fn note(login: &str, reason: &str) -> bool {
    let count = {
        let mut history = HISTORY.lock().unwrap_or_else(|e| e.into_inner());
        record(&mut history, login, Instant::now())
    };
    let hold = count >= HOLD_OFF_AT;
    if hold {
        crate::services::ll_origin::hold_off(HOLD_OFF);
    }
    log::warn!(
        "[Playback] automatic reload of {login} ({reason}), {count} in the last {} min{}",
        WINDOW.as_secs() / 60,
        if hold {
            format!("; low latency off for {} min, playing the stable stream", HOLD_OFF.as_secs() / 60)
        } else {
            String::new()
        }
    );
    hold
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reloads_count_inside_the_window_per_channel() {
        let mut h = HashMap::new();
        let t0 = Instant::now();
        assert_eq!(record(&mut h, "Caseoh_", t0), 1);
        assert_eq!(record(&mut h, "caseoh_", t0 + Duration::from_secs(40)), 2);
        assert_eq!(record(&mut h, "xqc", t0 + Duration::from_secs(41)), 1, "per channel");
        // Past the window the old ones no longer count.
        assert_eq!(record(&mut h, "caseoh_", t0 + WINDOW + Duration::from_secs(10)), 2);
        assert_eq!(record(&mut h, "caseoh_", t0 + WINDOW * 3), 1);
    }
}
