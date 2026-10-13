//! How far a player that cannot see the relay's live edge sits behind it.
//!
//! hls.js reports its own edge distance (`hls.latency`), but AVPlayer only
//! exposes the programme date of the frame it shows (`currentDate()`). The
//! programme date is the broadcaster's clock, so it cannot be compared with the
//! device clock directly. It can be compared with the programme date of the
//! newest media the relay holds, which is stamped by the same broadcaster:
//! both ends of the subtraction are on one timeline and the skew cancels.
//!
//! The edge reader reports every live part as it lands (`note_edge`): its end
//! programme date and the device time it arrived. "Live now" on the programme
//! timeline is a part's end date plus the time since it arrived, taken over the
//! freshest arrival in a short window (a part that queued behind a slow read
//! would otherwise drag the edge back). The player side hands in its current
//! programme date and gets the distance back (`report`).
//!
//! The relay also records each player request (`note_fetch`), so a measurement
//! shows which segment or part the player was on alongside the distance.
//!
//! Off until the first `report`: nothing is recorded for players that never ask.

use once_cell::sync::Lazy;
use serde::Serialize;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// Arrivals older than this no longer vote on where the edge is.
const EDGE_WINDOW_MS: i64 = 20_000;
/// Player requests held between two reports (the player asks once a second).
const MAX_FETCHES: usize = 200;
/// Catch-up rate offered while the player sits too far behind its target.
const CATCH_UP_RATE: f64 = 1.08;
/// Above the target by this much starts a catch-up...
const CATCH_UP_START_MS: i64 = 500;
/// ...which runs until the player is within this much of the target.
const CATCH_UP_STOP_MS: i64 = 100;

static ACTIVE: AtomicBool = AtomicBool::new(false);

struct EdgeArrival {
    sn: u64,
    part: usize,
    end_pdt_ms: i64,
    arrived_ms: i64,
}

#[derive(Default)]
struct State {
    edges: VecDeque<EdgeArrival>,
    fetches: Vec<String>,
    catching_up: bool,
}

static STATE: Lazy<Mutex<State>> = Lazy::new(|| Mutex::new(State::default()));

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Whether anyone is measuring; callers check it before formatting a record.
pub fn is_active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

/// Forget the previous stream's edge and requests.
pub fn reset() {
    if let Ok(mut s) = STATE.lock() {
        *s = State::default();
    }
}

/// A live part landed: `end_pdt_ms` is the programme date its media ends at.
pub fn note_edge(sn: u64, part: usize, end_pdt_ms: i64) {
    if !is_active() {
        return;
    }
    let arrived_ms = now_ms();
    if let Ok(mut s) = STATE.lock() {
        s.edges.push_back(EdgeArrival {
            sn,
            part,
            end_pdt_ms,
            arrived_ms,
        });
        while s
            .edges
            .front()
            .is_some_and(|e| arrived_ms - e.arrived_ms > EDGE_WINDOW_MS)
        {
            s.edges.pop_front();
        }
    }
}

/// A player request, already formatted (`part 123/4`, `pl 123/5 210ms`, ...).
pub fn note_fetch(what: String) {
    if !is_active() {
        return;
    }
    if let Ok(mut s) = STATE.lock() {
        if s.fetches.len() < MAX_FETCHES {
            let at = now_ms() % 100_000;
            s.fetches.push(format!("{at} {what}"));
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LiveDistance {
    /// Live now on the programme timeline minus the player's programme date,
    /// from the freshest arrival in the window.
    pub behind_ms: Option<i64>,
    /// The same from the newest part alone.
    pub behind_newest_ms: Option<i64>,
    /// The newest part the relay holds and how long ago it arrived.
    pub edge_sn: Option<u64>,
    pub edge_part: Option<usize>,
    pub edge_age_ms: Option<i64>,
    /// The playback rate to run at: above 1 while catching up to `target_ms`.
    pub rate: f64,
    /// Player requests since the previous report, oldest first.
    pub fetches: Vec<String>,
}

/// Where live is on the programme timeline at `now`, and from the newest part
/// alone. `None` before any part arrived.
fn edge_at(edges: &VecDeque<EdgeArrival>, now: i64) -> Option<(i64, i64)> {
    let newest = edges.back()?;
    let freshest = edges
        .iter()
        .map(|e| e.end_pdt_ms + (now - e.arrived_ms))
        .max()?;
    Some((freshest, newest.end_pdt_ms + (now - newest.arrived_ms)))
}

/// Measure `player_pdt_ms` against the edge and drain the request log. With a
/// `target_ms`, the answer's `rate` runs a gentle catch-up with hysteresis;
/// without one it stays 1.
pub fn report(player_pdt_ms: Option<f64>, target_ms: Option<i64>) -> LiveDistance {
    ACTIVE.store(true, Ordering::Relaxed);
    let now = now_ms();
    let mut s = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let edge = edge_at(&s.edges, now);
    let player = player_pdt_ms.filter(|p| p.is_finite()).map(|p| p as i64);
    let (behind_ms, behind_newest_ms) = match (edge, player) {
        (Some((freshest, newest)), Some(p)) => (Some(freshest - p), Some(newest - p)),
        _ => (None, None),
    };
    let catching_up = match (behind_ms, target_ms) {
        (Some(b), Some(t)) if s.catching_up => b > t + CATCH_UP_STOP_MS,
        (Some(b), Some(t)) => b > t + CATCH_UP_START_MS,
        _ => false,
    };
    s.catching_up = catching_up;
    let fetches = std::mem::take(&mut s.fetches);
    let newest = s.edges.back();
    LiveDistance {
        behind_ms,
        behind_newest_ms,
        edge_sn: newest.map(|e| e.sn),
        edge_part: newest.map(|e| e.part),
        edge_age_ms: newest.map(|e| now - e.arrived_ms),
        rate: if catching_up { CATCH_UP_RATE } else { 1.0 },
        fetches,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arrival(sn: u64, end_pdt_ms: i64, arrived_ms: i64) -> EdgeArrival {
        EdgeArrival {
            sn,
            part: 0,
            end_pdt_ms,
            arrived_ms,
        }
    }

    #[test]
    fn the_edge_is_the_freshest_arrival_carried_forward() {
        let mut edges = VecDeque::new();
        // Programme dates run 5 s ahead of the device clock (broadcaster skew).
        edges.push_back(arrival(1, 105_000, 100_000));
        // This one queued 300 ms behind a slow read.
        edges.push_back(arrival(1, 105_100, 100_400));
        let (freshest, newest) = edge_at(&edges, 101_000).unwrap();
        assert_eq!(freshest, 106_000);
        assert_eq!(newest, 105_700);
    }

    #[test]
    fn no_edge_before_the_first_part() {
        assert_eq!(edge_at(&VecDeque::new(), 1), None);
    }
}
