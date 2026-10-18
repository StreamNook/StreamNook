//! VOD chat replay: a recorded broadcast's chat, dripped in step with the player.
//!
//! VODs have no live chat feed. Twitch's recorded chat is read in pages by
//! offset (`vod_comments_at`) and parsed into the same rows live chat uses, so a
//! phone's chat pane draws replay with its ordinary row views. This module owns
//! the clock: the player reports its position about once a second
//! (`vod_replay_tick`) and gets back the rows to add, or a reset after a seek.
//! The buffer, the refill rules and seek handling all live here, so every app
//! replays the same way.
//!
//! The rules match the desktop's replay engine: a fetch starts `LEAD_SECS` before
//! the playhead, the buffer refills when fewer than `REFILL_WITHIN` rows are
//! left, a jump back past `SEEK_BACK_SECS` or forward past `SEEK_FWD_SECS` is a
//! seek, a seek the buffer can answer is answered from it at once, and one it
//! cannot waits for the scrubbing to settle (a tick with no further jump) before
//! fetching. A clip replays a window of its source VOD: positions arrive in the
//! clip's own clock and are shifted by `start_offset`.
//!
//! One session at a time (the phones play one VOD at a time).

use crate::models::chat_layout::ChatMessage;
use crate::rt::AppHandle;
use once_cell::sync::Lazy;
use serde::Serialize;
use std::collections::HashSet;
use std::sync::Mutex;
use tauri::Manager;

/// Rows a pane holds at most; a reset sends no more than this.
const VISIBLE_MAX: usize = 200;
/// A fetch starts this far before the playhead, so the screen is not empty at the start.
const LEAD_SECS: f64 = 15.0;
/// Refill when fewer than this many fetched rows are still ahead of the playhead.
const REFILL_WITHIN: usize = 40;
/// Any rewind past jitter is the viewer scrubbing back.
const SEEK_BACK_SECS: f64 = 0.75;
/// A forward jump past this is a seek, not playback.
const SEEK_FWD_SECS: f64 = 20.0;

/// A replayed row: anything with an id the buffer can dedupe on.
pub trait ReplayRow: Clone {
    fn row_id(&self) -> &str;
}

impl ReplayRow for ChatMessage {
    fn row_id(&self) -> &str {
        &self.id
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayStep<M = ChatMessage> {
    /// Drop every replay row before adding `rows` (a seek, or the first rows).
    pub reset: bool,
    pub rows: Vec<M>,
    /// Chat for this point is still being fetched.
    pub loading: bool,
    pub error: Option<String>,
}

impl<M> Default for ReplayStep<M> {
    fn default() -> Self {
        ReplayStep { reset: false, rows: Vec::new(), loading: false, error: None }
    }
}

struct Engine<M = ChatMessage> {
    vod_id: String,
    login: String,
    /// Sorted by offset.
    buffer: Vec<(f64, M)>,
    /// The next row not yet shown.
    ptr: usize,
    seen: HashSet<String>,
    last_time: f64,
    fetching: bool,
    next_fetch: f64,
    /// The last fetch added nothing: wait for the playhead to pass it.
    frontier: bool,
    /// A seek the buffer could not answer, waiting for a tick with no further jump.
    seek_pending: bool,
    start_offset: f64,
    end_offset: f64,
    /// Bumped on every resync, so a fetch that started before it is dropped.
    generation: u64,
    /// The pane has not been reset since the session began.
    fresh: bool,
    error: Option<String>,
}

/// What a tick asks the caller to fetch: the generation it belongs to and the offset.
type Fetch = Option<(u64, f64)>;

impl<M: ReplayRow> Engine<M> {
    fn new(vod_id: &str, login: &str, start_offset: f64, end_offset: Option<f64>, position: f64) -> (Self, Fetch) {
        let start_offset = start_offset.max(0.0);
        let mut engine = Engine {
            vod_id: vod_id.to_string(),
            login: login.to_lowercase(),
            buffer: Vec::new(),
            ptr: 0,
            seen: HashSet::new(),
            last_time: position,
            fetching: false,
            next_fetch: 0.0,
            frontier: false,
            seek_pending: false,
            start_offset,
            end_offset: end_offset.unwrap_or(f64::INFINITY),
            generation: 0,
            fresh: true,
            error: None,
        };
        engine.next_fetch = (engine.vod_time(position) - LEAD_SECS).max(start_offset);
        let fetch = engine.start_fetch(engine.next_fetch);
        (engine, fetch)
    }

    fn vod_time(&self, t: f64) -> f64 {
        t + self.start_offset
    }

    fn start_fetch(&mut self, offset: f64) -> Fetch {
        if self.fetching {
            return None;
        }
        self.fetching = true;
        Some((self.generation, offset.max(self.start_offset)))
    }

    /// A page arrived for `generation`.
    fn absorb(&mut self, generation: u64, page: Result<Vec<(f64, M)>, String>) {
        if generation != self.generation {
            return;
        }
        self.fetching = false;
        let page = match page {
            Ok(page) => page,
            Err(e) => {
                self.error = Some(e);
                return;
            }
        };
        self.error = None;
        let mut added = 0;
        let mut newest = self.next_fetch;
        for (offset, message) in page {
            if !self.seen.insert(message.row_id().to_string()) {
                continue;
            }
            newest = newest.max(offset);
            self.buffer.push((offset, message));
            added += 1;
        }
        if added > 0 {
            // New pages land at or after the newest offset held, so the shown head stays put.
            let shown = self.ptr;
            self.buffer[shown..].sort_by(|a, b| a.0.total_cmp(&b.0));
        }
        self.next_fetch = newest;
        self.frontier = added == 0;
    }

    /// Show everything up to `vt` again from the buffer. None when the buffer does not reach
    /// it: it starts after `vt`, or `vt` is past what has been fetched.
    fn reseek(&mut self, vt: f64) -> Option<ReplayStep<M>> {
        let first = self.buffer.first()?.0;
        if vt < first - 1.0 || vt > self.next_fetch + 1.0 {
            return None;
        }
        self.ptr = self.buffer.iter().take_while(|(o, _)| *o <= vt).count();
        let from = self.ptr.saturating_sub(VISIBLE_MAX);
        Some(ReplayStep {
            reset: true,
            rows: self.buffer[from..self.ptr].iter().map(|(_, m)| m.clone()).collect(),
            ..Default::default()
        })
    }

    fn resync(&mut self, vt: f64) -> Fetch {
        self.generation += 1;
        self.buffer.clear();
        self.ptr = 0;
        self.seen.clear();
        self.fetching = false;
        self.frontier = false;
        self.seek_pending = false;
        self.next_fetch = (vt - LEAD_SECS).max(self.start_offset);
        self.start_fetch(self.next_fetch)
    }

    fn tick(&mut self, t: f64) -> (ReplayStep<M>, Fetch) {
        let vt = self.vod_time(t);
        let jumped = t < self.last_time - SEEK_BACK_SECS || t > self.last_time + SEEK_FWD_SECS;
        self.last_time = t;
        if jumped {
            if let Some(step) = self.reseek(vt) {
                self.seek_pending = false;
                self.fresh = false;
                return (step, None);
            }
            // Clear the pane now; fetch once the scrubbing stops.
            self.seek_pending = true;
            self.fresh = false;
            return (ReplayStep { reset: true, loading: true, ..Default::default() }, None);
        }
        if self.seek_pending {
            let fetch = self.resync(vt);
            return (ReplayStep { loading: true, ..Default::default() }, fetch);
        }

        let start = self.ptr;
        while self.ptr < self.buffer.len() && self.buffer[self.ptr].0 <= vt {
            self.ptr += 1;
        }
        let mut step = ReplayStep {
            reset: std::mem::take(&mut self.fresh),
            rows: self.buffer[start..self.ptr].iter().map(|(_, m)| m.clone()).collect(),
            loading: self.fetching && self.ptr >= self.buffer.len(),
            error: self.error.clone(),
        };
        if step.rows.len() > VISIBLE_MAX {
            step.rows.drain(..step.rows.len() - VISIBLE_MAX);
        }

        let within = self.next_fetch < self.end_offset;
        let fetch = if !self.fetching && within && !self.frontier && self.buffer.len() - self.ptr < REFILL_WITHIN {
            self.start_fetch(self.next_fetch)
        } else if !self.fetching && within && self.frontier && self.ptr >= self.buffer.len() && vt > self.next_fetch + 1.0 {
            // Drained, and the playhead has moved past the empty frontier: look again from here.
            self.frontier = false;
            self.start_fetch(vt.floor())
        } else {
            None
        };
        (step, fetch)
    }
}

static SESSION: Lazy<Mutex<Option<Engine<ChatMessage>>>> = Lazy::new(|| Mutex::new(None));

/// Start replaying `vod_id` from `position` (seconds on the player's clock). A clip passes
/// its window in the VOD (`start_offset`, `end_offset`). Replaces any session.
pub fn begin(app: &AppHandle, vod_id: &str, login: &str, start_offset: f64, end_offset: Option<f64>, position: f64) {
    let (engine, fetch) = Engine::new(vod_id, login, start_offset, end_offset, position);
    if let Ok(mut s) = SESSION.lock() {
        *s = Some(engine);
    }
    run_fetch(app, vod_id, login, fetch);
}

/// The rows to add for the player at `position`. Nothing for a VOD that is not the session's.
pub fn tick(app: &AppHandle, vod_id: &str, position: f64) -> ReplayStep {
    let (step, fetch, login) = {
        let Ok(mut s) = SESSION.lock() else { return ReplayStep::default() };
        let Some(engine) = s.as_mut().filter(|e| e.vod_id == vod_id) else {
            return ReplayStep::default();
        };
        let (step, fetch) = engine.tick(position);
        (step, fetch, engine.login.clone())
    };
    run_fetch(app, vod_id, &login, fetch);
    step
}

pub fn stop() {
    if let Ok(mut s) = SESSION.lock() {
        *s = None;
    }
}

fn run_fetch(app: &AppHandle, vod_id: &str, login: &str, fetch: Fetch) {
    let Some((generation, offset)) = fetch else { return };
    let emotes = app.state::<crate::models::settings::AppState>().emote_service.clone();
    let (vod_id, login) = (vod_id.to_string(), login.to_string());
    tauri::async_runtime::spawn(async move {
        let page = crate::commands::twitch::vod_comments_at(&vod_id, &login, offset, emotes)
            .await
            .map(|comments| comments.into_iter().map(|c| (c.content_offset_seconds, c.message)).collect())
            .map_err(|e| {
                log::warn!("[VodReplay] {vod_id} at {offset}: {e}");
                "Could not load chat replay".to_string()
            });
        if let Ok(mut s) = SESSION.lock() {
            if let Some(engine) = s.as_mut().filter(|e| e.vod_id == vod_id) {
                engine.absorb(generation, page);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct Row(String);

    impl ReplayRow for Row {
        fn row_id(&self) -> &str {
            &self.0
        }
    }

    type E = Engine<Row>;

    fn page(offsets: &[(f64, &str)]) -> Result<Vec<(f64, Row)>, String> {
        Ok(offsets.iter().map(|(o, id)| (*o, Row(id.to_string()))).collect())
    }

    fn ids(step: &ReplayStep<Row>) -> Vec<String> {
        step.rows.iter().map(|m| m.0.clone()).collect()
    }

    #[test]
    fn the_first_fetch_leads_the_playhead() {
        let (_, fetch) = E::new("v", "Chan", 0.0, None, 100.0);
        assert_eq!(fetch, Some((0, 85.0)));
        let (_, fetch) = E::new("v", "chan", 0.0, None, 5.0);
        assert_eq!(fetch, Some((0, 0.0)));
    }

    #[test]
    fn rows_drip_as_the_playhead_passes_them() {
        let (mut e, _) = E::new("v", "c", 0.0, None, 100.0);
        e.absorb(0, page(&[(90.0, "a"), (99.0, "b"), (101.0, "c"), (103.0, "d")]));
        let (step, _) = e.tick(100.0);
        assert!(step.reset);
        assert_eq!(ids(&step), ["a", "b"]);
        let (step, _) = e.tick(101.5);
        assert!(!step.reset);
        assert_eq!(ids(&step), ["c"]);
    }

    #[test]
    fn a_rewind_inside_the_buffer_takes_rows_back_at_once() {
        let (mut e, _) = E::new("v", "c", 0.0, None, 100.0);
        e.absorb(0, page(&[(90.0, "a"), (99.0, "b"), (101.0, "c")]));
        e.tick(100.0);
        e.tick(101.5);
        let (step, fetch) = e.tick(95.0);
        assert!(step.reset);
        assert_eq!(ids(&step), ["a"]);
        assert_eq!(fetch, None);
    }

    #[test]
    fn a_seek_outside_the_buffer_clears_then_fetches_once_settled() {
        let (mut e, _) = E::new("v", "c", 0.0, None, 100.0);
        e.absorb(0, page(&[(90.0, "a"), (99.0, "b")]));
        e.tick(100.0);
        let (step, fetch) = e.tick(2_000.0);
        assert!(step.reset && step.rows.is_empty() && step.loading);
        assert_eq!(fetch, None);
        let (_, fetch) = e.tick(2_001.0);
        assert_eq!(fetch, Some((1, 1_986.0)));
        // A page from before the seek is dropped.
        e.absorb(0, page(&[(150.0, "old")]));
        assert!(e.buffer.is_empty());
    }

    #[test]
    fn refills_stop_at_a_clips_window() {
        let (mut e, first) = E::new("v", "c", 2_781.0, Some(2_811.0), 0.0);
        assert_eq!(first, Some((0, 2_781.0)));
        e.absorb(0, page(&[(2_781.0, "a"), (2_790.0, "b"), (2_812.0, "z")]));
        let (step, fetch) = e.tick(10.0);
        assert_eq!(ids(&step), ["a", "b"]);
        assert_eq!(fetch, None, "the next fetch would start past the clip's end");
    }

    #[test]
    fn duplicate_rows_are_dropped() {
        let (mut e, _) = E::new("v", "c", 0.0, None, 0.0);
        e.absorb(0, page(&[(1.0, "a"), (2.0, "b")]));
        e.fetching = true;
        e.absorb(0, page(&[(2.0, "b"), (3.0, "c")]));
        assert_eq!(e.buffer.len(), 3);
    }
}
