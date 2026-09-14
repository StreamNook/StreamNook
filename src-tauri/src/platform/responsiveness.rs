//! Detect a wedged UI thread on platforms that have no Win32 message pump.
//!
//! # Why this is not a port of `ui_hang_watchdog`
//!
//! That module probes the **Win32 message pump** directly, the same way the OS
//! decides whether to paint "(Not Responding)". A message pump is a Windows
//! concept; macOS and Linux have no equivalent to probe, so there is nothing to
//! port. What CAN be ported is the *purpose*: notice when the thread that draws
//! the UI has stopped servicing work, and say so.
//!
//! The portable mechanism is a round-trip. Ask the main thread to flip a flag;
//! if it has not flipped by the deadline, the main thread is not running work.
//! That catches the same class of fault (a blocking call on the UI thread while
//! every other thread keeps going) through a different door.
//!
//! # What it deliberately does not copy
//!
//! The Windows implementation is allocation-free, writing through raw
//! `CreateFileW` because a wedged UI thread can be stuck holding the process
//! heap lock. That constraint comes from *how* it probes — it runs code on the
//! stuck thread. This probe never runs anything on the stuck thread; it only
//! observes whether a flag moved, from a healthy thread. So the allocation
//! gymnastics are unnecessary here, and copying them would be cargo-culting.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// How many consecutive missed probes before a hang is declared.
///
/// One miss is not a hang: a main thread doing a legitimately slow layout or a
/// synchronous file dialog will miss a single round-trip. Requiring several in
/// a row is what separates "busy" from "wedged".
pub const MISSES_BEFORE_HANG: u32 = 3;

/// What a single probe observed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Probe {
    /// The main thread ran our closure inside the deadline.
    Responsive,
    /// It did not.
    Missed,
}

/// What the watchdog should do about the latest probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transition {
    /// Nothing changed worth reporting.
    Steady,
    /// The UI just became unresponsive; report onset.
    HangStarted { missed: u32 },
    /// The UI just recovered; report the duration.
    Recovered { missed: u32 },
}

/// Pure hang-state machine.
///
/// Kept separate from the probing so the decision logic can be tested exactly,
/// without timing, threads, or a running UI. Getting this wrong is expensive in
/// both directions: too eager and every slow layout is logged as a freeze, too
/// lax and a real freeze goes unrecorded.
#[derive(Debug, Default)]
pub struct HangState {
    consecutive_misses: u32,
    in_hang: bool,
}

impl HangState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the UI is currently considered hung.
    pub fn is_hung(&self) -> bool {
        self.in_hang
    }

    pub fn observe(&mut self, probe: Probe) -> Transition {
        match probe {
            Probe::Missed => {
                self.consecutive_misses = self.consecutive_misses.saturating_add(1);
                if !self.in_hang && self.consecutive_misses >= MISSES_BEFORE_HANG {
                    self.in_hang = true;
                    Transition::HangStarted {
                        missed: self.consecutive_misses,
                    }
                } else {
                    Transition::Steady
                }
            }
            Probe::Responsive => {
                let missed = self.consecutive_misses;
                self.consecutive_misses = 0;
                if self.in_hang {
                    self.in_hang = false;
                    Transition::Recovered { missed }
                } else {
                    Transition::Steady
                }
            }
        }
    }
}

/// Ask the main thread to flip a flag, and report whether it did in time.
///
/// Returns [`Probe::Missed`] if the main thread does not service the closure
/// within `deadline`, which is the signal that it is wedged rather than merely
/// busy.
pub fn probe_main_thread<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    deadline: Duration,
) -> Probe {
    let flag = Arc::new(AtomicBool::new(false));
    let echo = flag.clone();

    if app.run_on_main_thread(move || echo.store(true, Ordering::Release)).is_err() {
        // The main thread is gone entirely (shutdown). Not a hang.
        return Probe::Responsive;
    }

    // Poll rather than block: a condvar would need the main thread to signal us,
    // and the whole premise is that it might not.
    let step = Duration::from_millis(10);
    let mut waited = Duration::ZERO;
    while waited < deadline {
        if flag.load(Ordering::Acquire) {
            return Probe::Responsive;
        }
        std::thread::sleep(step);
        waited += step;
    }
    if flag.load(Ordering::Acquire) {
        Probe::Responsive
    } else {
        Probe::Missed
    }
}

/// Count of hangs observed this session, for diagnostics.
static HANGS_SEEN: AtomicU32 = AtomicU32::new(0);

pub fn record_hang() {
    HANGS_SEEN.fetch_add(1, Ordering::Relaxed);
}

pub fn hangs_seen() -> u32 {
    HANGS_SEEN.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_single_miss_is_not_a_hang() {
        let mut s = HangState::new();
        assert_eq!(s.observe(Probe::Missed), Transition::Steady);
        assert!(!s.is_hung(), "one slow frame must not be reported as a freeze");
    }

    #[test]
    fn misses_below_the_threshold_stay_quiet() {
        let mut s = HangState::new();
        for _ in 0..(MISSES_BEFORE_HANG - 1) {
            assert_eq!(s.observe(Probe::Missed), Transition::Steady);
        }
        assert!(!s.is_hung());
    }

    #[test]
    fn the_threshold_miss_declares_a_hang_exactly_once() {
        let mut s = HangState::new();
        for _ in 0..(MISSES_BEFORE_HANG - 1) {
            s.observe(Probe::Missed);
        }
        assert_eq!(
            s.observe(Probe::Missed),
            Transition::HangStarted { missed: MISSES_BEFORE_HANG }
        );
        assert!(s.is_hung());
        // Still hung, but onset must not be reported again — otherwise a long
        // freeze floods the log with duplicate onsets.
        assert_eq!(s.observe(Probe::Missed), Transition::Steady);
        assert!(s.is_hung());
    }

    #[test]
    fn recovery_reports_how_long_it_was_stuck() {
        let mut s = HangState::new();
        for _ in 0..(MISSES_BEFORE_HANG + 2) {
            s.observe(Probe::Missed);
        }
        assert_eq!(
            s.observe(Probe::Responsive),
            Transition::Recovered { missed: MISSES_BEFORE_HANG + 2 }
        );
        assert!(!s.is_hung());
    }

    #[test]
    fn recovery_is_reported_once_not_on_every_healthy_probe() {
        let mut s = HangState::new();
        for _ in 0..MISSES_BEFORE_HANG {
            s.observe(Probe::Missed);
        }
        assert!(matches!(s.observe(Probe::Responsive), Transition::Recovered { .. }));
        assert_eq!(s.observe(Probe::Responsive), Transition::Steady);
        assert_eq!(s.observe(Probe::Responsive), Transition::Steady);
    }

    #[test]
    fn a_healthy_probe_resets_the_streak() {
        let mut s = HangState::new();
        s.observe(Probe::Missed);
        s.observe(Probe::Missed);
        s.observe(Probe::Responsive); // streak broken, no hang was declared
        assert_eq!(s.observe(Probe::Missed), Transition::Steady);
        assert_eq!(s.observe(Probe::Missed), Transition::Steady);
        assert!(
            !s.is_hung(),
            "the counter must restart after a healthy probe, or intermittent \
             slowness accumulates into a false freeze report"
        );
    }

    #[test]
    fn hang_and_recovery_can_repeat() {
        let mut s = HangState::new();
        for _ in 0..2 {
            for _ in 0..MISSES_BEFORE_HANG {
                s.observe(Probe::Missed);
            }
            assert!(s.is_hung());
            assert!(matches!(s.observe(Probe::Responsive), Transition::Recovered { .. }));
            assert!(!s.is_hung());
        }
    }

    #[test]
    fn hang_counter_increments() {
        let before = hangs_seen();
        record_hang();
        assert_eq!(hangs_seen(), before + 1);
    }
}
