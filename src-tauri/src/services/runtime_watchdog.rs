//! Runtime stall detector. Answers "did the backend freeze, which layer, and
//! for how long" with measured numbers instead of inference.
//!
//! Two heartbeats run forever, each measuring its own scheduling lag:
//!
//!  - a TOKIO TASK: if it asks to sleep 250ms but is polled back 2s late, a
//!    tokio worker thread was BLOCKED (sync I/O on the async runtime, a mutex
//!    held across `.await`, a long CPU loop). This is the exact class the
//!    diagnostics-recorder freeze belonged to; if others remain in chat /
//!    badges / emotes / irc, this catches them.
//!  - an OS THREAD: same measurement on a real thread tokio can't starve. If
//!    BOTH lag, the whole PROCESS stalled (GC pause in a linked lib, swap, CPU
//!    saturation); if only the tokio one lags, it's tokio-specific blocking.
//!
//! Stalls are written to the active ll_diagnostics capture as `rt_stall`
//! events, so they line up on one timeline with the player's ticks and the
//! frontend's `jsblock` events — a freeze becomes one labelled row naming the
//! side and the duration. They also go to the file log, at most one line per
//! side every few seconds, so a "chat feels slow" report can be read off it.

use std::time::{Duration, Instant};

/// Heartbeat cadence. Short enough to localize a stall, long enough to be free.
const TICK: Duration = Duration::from_millis(250);
/// Lag past the cadence before it counts as a stall worth recording (filters
/// ordinary scheduler jitter; a real freeze is hundreds of ms to seconds).
const STALL_THRESHOLD: Duration = Duration::from_millis(300);
/// A gap this long is the machine sleeping, not a stall.
const SLEEP_GAP: Duration = Duration::from_secs(30);
/// At most one log line per side this often; the stalls between are counted
/// into the next one.
const LOG_EVERY: Duration = Duration::from_secs(5);

/// One side's stalls, paced for the log.
struct Pacer {
    side: &'static str,
    last: Option<Instant>,
    skipped: u32,
}

impl Pacer {
    fn new(side: &'static str) -> Self {
        Self { side, last: None, skipped: 0 }
    }

    fn lag(&mut self, lag: Duration) {
        if lag < STALL_THRESHOLD || lag >= SLEEP_GAP {
            return;
        }
        if self.last.is_some_and(|t| t.elapsed() < LOG_EVERY) {
            self.skipped += 1;
            return;
        }
        record(self.side, lag.as_millis(), self.skipped);
        self.last = Some(Instant::now());
        self.skipped = 0;
    }
}

fn record(side: &str, lag_ms: u128, skipped: u32) {
    log::warn!("[Watchdog] {side} runtime stalled ~{lag_ms}ms ({skipped} more since the last line)");
    crate::services::ll_diagnostics::event(&format!(
        "\"ev\":\"rt_stall\",\"side\":\"{side}\",\"ms\":{lag_ms}"
    ));
}

/// Master switch. On for desktop: two quarter-second heartbeats cost nothing
/// and answer "did the backend stall" in the ordinary log. Off on the phones,
/// where waking four times a second keeps the CPU out of its low-power states.
const WATCHDOG_ENABLED: bool = cfg!(desktop);

/// Start both heartbeats. Call once from Tauri `setup`. The setup closure runs
/// OUTSIDE the tokio runtime context, so the tokio heartbeat is spawned via
/// `tauri::async_runtime::spawn` (Tauri's managed runtime — the same one every
/// command and `ll_origin` task runs on, which is exactly what we want to
/// measure), NOT bare `tokio::spawn` (panics: "no reactor running").
pub fn start() {
    if !WATCHDOG_ENABLED {
        return;
    }
    // Tokio-scheduling heartbeat. If the runtime is fully jammed it cannot run
    // to measure in real time, but the sleep returns late once unjammed, so the
    // stall is reported retroactively with its true duration.
    tauri::async_runtime::spawn(async {
        let mut pacer = Pacer::new("tokio");
        loop {
            let t0 = Instant::now();
            tokio::time::sleep(TICK).await;
            pacer.lag(t0.elapsed().saturating_sub(TICK));
        }
    });

    // Whole-process heartbeat on a dedicated OS thread. Now that it runs on
    // every desktop start, a refused thread is logged, never a startup panic.
    let spawned = std::thread::Builder::new()
        .name("rt-watchdog".into())
        .spawn(|| {
            let mut pacer = Pacer::new("process");
            loop {
                let t0 = Instant::now();
                std::thread::sleep(TICK);
                pacer.lag(t0.elapsed().saturating_sub(TICK));
            }
        });
    if let Err(e) = spawned {
        log::warn!("[Watchdog] process heartbeat not started: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jitter_and_sleep_are_not_stalls() {
        let mut p = Pacer::new("tokio");
        p.lag(Duration::from_millis(100));
        p.lag(Duration::from_secs(45));
        assert!(p.last.is_none());
    }

    #[test]
    fn stalls_close_together_share_one_line() {
        let mut p = Pacer::new("tokio");
        p.lag(Duration::from_millis(400));
        assert!(p.last.is_some());
        assert_eq!(p.skipped, 0);
        p.lag(Duration::from_millis(700));
        p.lag(Duration::from_millis(500));
        assert_eq!(p.skipped, 2);
    }
}
