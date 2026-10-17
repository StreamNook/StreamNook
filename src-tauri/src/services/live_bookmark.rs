//! Where the viewer left a live Twitch broadcast, kept as a quiet bookmark on
//! that broadcast's recording.
//!
//! Live watching never files a Continue Watching card. A bookmark is a
//! `source: live` progress entry, which the rail skips: it shows only on the
//! recording itself (the VOD list's progress bar, the offline room's Resume,
//! a mark on the live timeline when the same broadcast is reopened), and it
//! becomes an ordinary entry once the viewer deliberately plays that VOD.
//!
//! The session follows the main player's live PLAYBACK, not the watch
//! session: a close to the tray stops the stream but leaves the watch session
//! up. It ends on `stop_stream`, a start of anything other than the same
//! channel, the main window being destroyed, or app exit. A broadcast that
//! ends while it is watched files nothing, and clears what earlier sessions
//! filed for it.
//!
//! The position is the broadcast's wall clock: a recording's own length
//! trailed wall clock since `recordedAt` by under 8 s on every broadcast
//! probed, up to 8.5 h in. While rewound into the recording, the player's own
//! playhead is used instead.

use crate::services::vod_progress_service;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// Watching less than this files nothing, so zapping through channels
/// leaves no trace.
pub const MIN_WATCH_SECS: f64 = 600.0;
/// A bookmark lands this far before the last thing seen: the player sits a
/// few seconds behind live, and resuming a little early beats skipping.
pub const BACKOFF_SECS: f64 = 20.0;
/// A live pause shorter than this resumes behind the edge by the pause;
/// longer, and hls.js seeks back to live by itself (`liveMaxLatencyDuration`).
pub const LIVE_SEEK_WINDOW_SECS: f64 = 55.0;
/// A stop then a start of the same channel inside this window (a restart, a
/// quality change, an ad pivot, a quick trip Home) keeps counting the same
/// session's watch time.
const SAME_SESSION_GRACE_MS: i64 = 15 * 60 * 1000;
/// A recording that began this long before the broadcast is a previous
/// broadcast's (the per-channel recording cache can outlive a restart).
const STALE_ARCHIVE_SLACK_MS: i64 = 2 * 60 * 1000;
/// The timeline mark is hidden this close to live: there is nothing to go
/// back for.
pub const MARK_MIN_BEHIND_SECS: f64 = 120.0;
/// VOD starts remembered for classifying the player's reports. A report
/// arrives within seconds of its start; this only bounds the list.
const PLAYS_KEPT: usize = 32;

/// A broadcast's recording: its VOD id and the wall clock at position 0.
#[derive(Debug, Clone, PartialEq)]
pub struct Archive {
    pub video_id: String,
    pub recorded_at_ms: i64,
}

impl Archive {
    pub fn parse(video_id: &str, recorded_at: Option<&str>) -> Option<Archive> {
        let video_id = video_id.trim();
        let at = chrono::DateTime::parse_from_rfc3339(recorded_at?).ok()?;
        (!video_id.is_empty()).then(|| Archive {
            video_id: video_id.to_string(),
            recorded_at_ms: at.timestamp_millis(),
        })
    }
}

/// Why a VOD is playing. Only a deliberate open may write the progress store
/// as an ordinary entry; a rewind is the live session looking back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Deliberate,
    Rewind,
}

/// What ending a session asks the store to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Nothing,
    /// File the bookmark.
    Write { login: String, video_id: String, position_secs: f64 },
    /// The recording is not known yet: look it up, then map `left_ms`.
    Resolve { login: String, left_ms: i64, behind_secs: f64 },
    /// The broadcast was watched to its end: forget earlier bookmarks of it.
    Forget { video_id: String },
}

#[derive(Debug, Clone, PartialEq)]
struct Rewound {
    video_id: String,
    position_secs: Option<f64>,
}

#[derive(Debug, Clone)]
struct Session {
    login: String,
    /// The resume setting when the session began.
    enabled: bool,
    /// When this playback began; the timeline mark only shows what was filed
    /// before it.
    begun_ms: i64,
    archive: Option<Archive>,
    played_ms: i64,
    playing_since: Option<i64>,
    paused_at: Option<i64>,
    /// How far behind live a short pause left the player.
    behind_secs: f64,
    rewound: Option<Rewound>,
    /// Twitch said the broadcast ended while it was watched live.
    ended: bool,
    active: bool,
    stopped_ms: Option<i64>,
}

impl Session {
    fn new(login: &str, enabled: bool, now: i64) -> Session {
        Session {
            login: login.to_string(),
            enabled,
            begun_ms: now,
            archive: None,
            played_ms: 0,
            playing_since: None,
            paused_at: None,
            behind_secs: 0.0,
            rewound: None,
            ended: false,
            active: true,
            stopped_ms: None,
        }
    }

    fn settle(&mut self, now: i64) {
        if let Some(since) = self.playing_since.take() {
            self.played_ms += (now - since).max(0);
        }
    }
}

/// The live session, pure: every method takes the clock so it can be tested.
#[derive(Debug, Default)]
pub struct Tracker {
    session: Option<Session>,
}

/// Map a moment of the broadcast onto its recording, backed off a little.
/// None when it lands too early in the recording to be worth resuming.
pub fn map_position(archive: &Archive, left_ms: i64, behind_secs: f64) -> Option<f64> {
    let secs = (left_ms - archive.recorded_at_ms) as f64 / 1000.0 - behind_secs.max(0.0) - BACKOFF_SECS;
    (secs.is_finite() && secs >= vod_progress_service::RESUME_MIN_SECS).then_some(secs)
}

/// Where the live timeline marks "you left here", or None. Only a position
/// filed before this playback began, and only one far enough from live to be
/// worth going back for.
pub fn mark_position(
    entry_position_secs: f64,
    entry_updated_secs: i64,
    begun_ms: Option<i64>,
    elapsed_secs: f64,
) -> Option<f64> {
    if !entry_position_secs.is_finite() || entry_position_secs < vod_progress_service::RESUME_MIN_SECS {
        return None;
    }
    if let Some(begun) = begun_ms {
        if entry_updated_secs * 1000 >= begun {
            return None;
        }
    }
    (elapsed_secs - entry_position_secs >= MARK_MIN_BEHIND_SECS).then_some(entry_position_secs)
}

impl Tracker {
    fn active_for(&mut self, login: &str) -> Option<&mut Session> {
        self.session
            .as_mut()
            .filter(|s| s.active && s.login.eq_ignore_ascii_case(login))
    }

    /// Something is about to start in the main player. A live start of the
    /// same channel keeps the session; anything else ends it.
    pub fn starting(&mut self, live_login: Option<&str>, now: i64, broadcast_started_ms: Option<i64>) -> Outcome {
        let same = match (live_login, self.session.as_ref()) {
            (Some(login), Some(s)) => s.active && s.login.eq_ignore_ascii_case(login),
            _ => false,
        };
        if same {
            Outcome::Nothing
        } else {
            self.end(now, broadcast_started_ms)
        }
    }

    /// The main player is now serving `login` live.
    pub fn begin_live(&mut self, login: &str, enabled: bool, now: i64) {
        let login = login.to_lowercase();
        if let Some(s) = self.session.as_mut().filter(|s| s.login == login) {
            let resumable = s.active || s.stopped_ms.is_some_and(|at| now - at <= SAME_SESSION_GRACE_MS);
            if resumable {
                if !s.active {
                    // Back after a stop: what was filed then is "before" now.
                    s.begun_ms = now;
                }
                if s.ended {
                    // A broadcast that ended and came back is a new recording.
                    s.archive = None;
                    s.ended = false;
                }
                s.active = true;
                s.stopped_ms = None;
                s.enabled = enabled;
                s.rewound = None;
                s.paused_at = None;
                s.behind_secs = 0.0;
                return;
            }
        }
        self.session = Some(Session::new(&login, enabled, now));
    }

    /// The recording of the broadcast `login` is playing.
    pub fn note_archive(&mut self, login: &str, archive: Archive) {
        if let Some(s) = self.active_for(login) {
            s.archive = Some(archive);
        }
    }

    /// The live session swapped onto its recording.
    pub fn begin_rewind(&mut self, login: &str, archive: Archive) {
        if let Some(s) = self.active_for(login) {
            s.rewound = Some(Rewound { video_id: archive.video_id.clone(), position_secs: None });
            s.archive = Some(archive);
            s.paused_at = None;
            s.behind_secs = 0.0;
        }
    }

    /// The player's playhead while rewound.
    pub fn note_rewind_position(&mut self, video_id: &str, position_secs: f64) -> bool {
        if !position_secs.is_finite() || position_secs < 0.0 {
            return false;
        }
        match self.session.as_mut().and_then(|s| s.rewound.as_mut().filter(|_| s.active)) {
            Some(r) if r.video_id == video_id => {
                r.position_secs = Some(position_secs);
                true
            }
            _ => false,
        }
    }

    /// The main player started or stopped playing.
    pub fn set_playing(&mut self, playing: bool, now: i64) {
        let Some(s) = self.session.as_mut().filter(|s| s.active) else { return };
        if playing {
            if s.playing_since.is_none() {
                s.playing_since = Some(now);
            }
            if let Some(paused) = s.paused_at.take() {
                if s.rewound.is_none() {
                    let gap = (now - paused) as f64 / 1000.0;
                    s.behind_secs = if gap < LIVE_SEEK_WINDOW_SECS {
                        (s.behind_secs + gap).min(LIVE_SEEK_WINDOW_SECS)
                    } else {
                        0.0
                    };
                }
            }
        } else {
            s.settle(now);
            if s.paused_at.is_none() {
                s.paused_at = Some(now);
            }
        }
    }

    /// Twitch says `login`'s broadcast ended. Only counts while it is watched
    /// live: a viewer rewound into the recording is still somewhere in it.
    pub fn note_ended(&mut self, login: &str) {
        if let Some(s) = self.active_for(login) {
            if s.rewound.is_none() {
                s.ended = true;
            }
        }
    }

    /// When the current session began, if it is `login`'s.
    pub fn begun_ms(&self, login: &str) -> Option<i64> {
        self.session
            .as_ref()
            .filter(|s| s.active && s.login.eq_ignore_ascii_case(login))
            .map(|s| s.begun_ms)
    }

    /// The main player stopped. `broadcast_started_ms` (the watch session's
    /// broadcast start, when known) catches a recording left over from a
    /// previous broadcast.
    pub fn end(&mut self, now: i64, broadcast_started_ms: Option<i64>) -> Outcome {
        let Some(s) = self.session.as_mut().filter(|s| s.active) else {
            return Outcome::Nothing;
        };
        s.settle(now);
        s.active = false;
        s.stopped_ms = Some(now);
        if !s.enabled {
            return Outcome::Nothing;
        }
        let archive = s
            .archive
            .clone()
            .filter(|a| broadcast_started_ms.is_none_or(|started| a.recorded_at_ms >= started - STALE_ARCHIVE_SLACK_MS));
        if s.ended {
            return match archive {
                Some(a) => Outcome::Forget { video_id: a.video_id },
                None => Outcome::Nothing,
            };
        }
        if (s.played_ms as f64) / 1000.0 < MIN_WATCH_SECS {
            return Outcome::Nothing;
        }
        if let Some(r) = &s.rewound {
            return match r.position_secs {
                Some(p) if p >= vod_progress_service::RESUME_MIN_SECS => Outcome::Write {
                    login: s.login.clone(),
                    video_id: r.video_id.clone(),
                    position_secs: p,
                },
                _ => Outcome::Nothing,
            };
        }
        let left_ms = s.paused_at.unwrap_or(now);
        match archive {
            Some(a) => match map_position(&a, left_ms, s.behind_secs) {
                Some(position_secs) => Outcome::Write {
                    login: s.login.clone(),
                    video_id: a.video_id,
                    position_secs,
                },
                None => Outcome::Nothing,
            },
            None => Outcome::Resolve {
                login: s.login.clone(),
                left_ms,
                behind_secs: s.behind_secs,
            },
        }
    }
}

static TRACKER: Mutex<Tracker> = Mutex::new(Tracker { session: None });

fn with_tracker<R>(f: impl FnOnce(&mut Tracker) -> R) -> Option<R> {
    TRACKER.lock().ok().map(|mut t| f(&mut t))
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// The watch session's broadcast start for `login`, if it is the one watched.
fn broadcast_started_ms(login: Option<&str>) -> Option<i64> {
    let (watched, started) = crate::services::watch_session::current_twitch_watch()?;
    if let Some(login) = login {
        if !watched.eq_ignore_ascii_case(login) {
            return None;
        }
    }
    chrono::DateTime::parse_from_rfc3339(&started).ok().map(|d| d.timestamp_millis())
}

fn session_login() -> Option<String> {
    with_tracker(|t| t.session.as_ref().filter(|s| s.active).map(|s| s.login.clone())).flatten()
}

/// How an end may finish its work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndKind {
    /// From a command: a missing recording id may be looked up.
    Async,
    /// The window is gone or the process is exiting: no network, no await.
    Sync,
}

fn apply(outcome: Outcome, kind: EndKind) {
    match outcome {
        Outcome::Nothing => {}
        Outcome::Write { login, video_id, position_secs } => {
            log::debug!("[LiveBookmark] {login} left at {position_secs:.0}s of {video_id}");
            if let Err(e) = vod_progress_service::record_live(&video_id, position_secs, &login) {
                log::debug!("[LiveBookmark] could not file {video_id}: {e}");
            }
        }
        Outcome::Forget { video_id } => {
            if let Err(e) = vod_progress_service::remove_live(&video_id) {
                log::debug!("[LiveBookmark] could not forget {video_id}: {e}");
            }
        }
        Outcome::Resolve { login, left_ms, behind_secs } => {
            if kind == EndKind::Sync {
                log::debug!("[LiveBookmark] {login}: recording unknown at exit, nothing filed");
                return;
            }
            tauri::async_runtime::spawn(async move {
                let archive = match crate::services::twitch_resolver::fetch_live_archive(&login).await {
                    Ok(Some(a)) => Archive::parse(&a.video_id, a.recorded_at.as_deref()),
                    _ => None,
                };
                let Some(archive) = archive else {
                    log::debug!("[LiveBookmark] {login}: no recording to file a bookmark on");
                    return;
                };
                if let Some(position_secs) = map_position(&archive, left_ms, behind_secs) {
                    apply(
                        Outcome::Write { login, video_id: archive.video_id, position_secs },
                        EndKind::Sync,
                    );
                }
            });
        }
    }
}

/// Something is about to start in the main player: `live_login` for a live
/// Twitch channel, None for a VOD, clip or another platform.
pub fn starting(live_login: Option<&str>) {
    let started = broadcast_started_ms(session_login().as_deref());
    if let Some(out) = with_tracker(|t| t.starting(live_login, now_ms(), started)) {
        apply(out, EndKind::Async);
    }
}

/// The main player is serving `login` live. `enabled` is the resume setting.
pub fn begin_live(login: &str, enabled: bool) {
    with_tracker(|t| t.begin_live(login, enabled, now_ms()));
}

/// The recording of `login`'s broadcast (the player asks at every live start).
pub fn note_archive(login: &str, video_id: &str, recorded_at: Option<&str>) {
    if let Some(a) = Archive::parse(video_id, recorded_at) {
        with_tracker(|t| t.note_archive(login, a));
    }
}

/// The live session swapped onto its recording.
pub fn begin_rewind(login: &str, video_id: &str, recorded_at: Option<&str>) {
    if let Some(a) = Archive::parse(video_id, recorded_at) {
        with_tracker(|t| t.begin_rewind(login, a));
    }
}

/// The main player's play state, once the earning gate has decided it
/// (`report_player_state`).
pub fn set_playing(playing: bool) {
    with_tracker(|t| t.set_playing(playing, now_ms()));
}

/// Twitch confirmed `login`'s broadcast is over.
pub fn note_ended(login: &str) {
    with_tracker(|t| t.note_ended(login));
}

/// The main player stopped.
pub fn end(kind: EndKind) {
    let started = broadcast_started_ms(session_login().as_deref());
    if let Some(out) = with_tracker(|t| t.end(now_ms(), started)) {
        apply(out, kind);
    }
}

/// When the live session on `login` began, for the timeline mark.
pub fn begun_ms(login: &str) -> Option<i64> {
    with_tracker(|t| t.begun_ms(login)).flatten()
}

// ── VOD starts: who opened this VOD ─────────────────────────────────────

static NEXT_PLAY: AtomicU64 = AtomicU64::new(1);
static PLAYS: Mutex<VecDeque<(u64, Origin)>> = Mutex::new(VecDeque::new());

fn push_play(plays: &mut VecDeque<(u64, Origin)>, id: u64, origin: Origin) {
    plays.push_back((id, origin));
    while plays.len() > PLAYS_KEPT {
        plays.pop_front();
    }
}

/// Stamp a VOD start. The player sends the id back with every position
/// report, so Rust (not the page) decides what the report may write.
pub fn register_play(origin: Origin) -> u64 {
    let id = NEXT_PLAY.fetch_add(1, Ordering::Relaxed);
    if let Ok(mut plays) = PLAYS.lock() {
        push_play(&mut plays, id, origin);
    }
    id
}

/// Who opened the VOD a report is for. None for an id this run never issued,
/// which must write nothing: failing open would file a rail card for a live
/// session, the exact thing this module exists to prevent.
pub fn origin_of(play_id: Option<u64>) -> Option<Origin> {
    let id = play_id?;
    PLAYS
        .lock()
        .ok()?
        .iter()
        .rev()
        .find(|(p, _)| *p == id)
        .map(|(_, o)| *o)
}

/// A rewound player's playhead. Held in memory and filed when the session
/// ends, so a casual scrub back writes nothing to disk.
pub fn note_rewind_position(video_id: &str, position_secs: f64) {
    with_tracker(|t| t.note_rewind_position(video_id, position_secs));
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60_000;
    const T0: i64 = 1_760_000_000_000;

    fn archive(at: i64) -> Archive {
        Archive { video_id: "v1".into(), recorded_at_ms: at }
    }

    /// Live on `login` from T0, playing, the recording started an hour before.
    fn watching(t: &mut Tracker) {
        t.begin_live("caedrel", true, T0);
        t.note_archive("caedrel", archive(T0 - 60 * MIN));
        t.set_playing(true, T0);
    }

    fn write_pos(o: Outcome) -> f64 {
        match o {
            Outcome::Write { position_secs, video_id, .. } => {
                assert_eq!(video_id, "v1");
                position_secs
            }
            other => panic!("expected a write, got {other:?}"),
        }
    }

    #[test]
    fn closing_after_twenty_minutes_files_where_it_was_left() {
        let mut t = Tracker::default();
        watching(&mut t);
        let pos = write_pos(t.end(T0 + 20 * MIN, None));
        assert!((pos - (80.0 * 60.0 - BACKOFF_SECS)).abs() < 0.001, "{pos}");
    }

    #[test]
    fn zapping_files_nothing() {
        let mut t = Tracker::default();
        watching(&mut t);
        assert_eq!(t.end(T0 + 5 * MIN, None), Outcome::Nothing);
    }

    #[test]
    fn only_playing_time_counts_toward_the_minimum() {
        let mut t = Tracker::default();
        watching(&mut t);
        t.set_playing(false, T0 + 5 * MIN);
        // Paused for an hour, then closed: five minutes watched.
        assert_eq!(t.end(T0 + 65 * MIN, None), Outcome::Nothing);
    }

    #[test]
    fn closing_while_paused_files_the_pause() {
        let mut t = Tracker::default();
        watching(&mut t);
        t.set_playing(false, T0 + 30 * MIN);
        let pos = write_pos(t.end(T0 + 90 * MIN, None));
        assert!((pos - (90.0 * 60.0 - BACKOFF_SECS)).abs() < 0.001, "{pos}");
    }

    #[test]
    fn a_short_pause_leaves_the_player_behind_live() {
        let mut t = Tracker::default();
        watching(&mut t);
        t.set_playing(false, T0 + 20 * MIN);
        t.set_playing(true, T0 + 20 * MIN + 30_000);
        let pos = write_pos(t.end(T0 + 30 * MIN, None));
        assert!((pos - (90.0 * 60.0 - 30.0 - BACKOFF_SECS)).abs() < 0.001, "{pos}");
    }

    #[test]
    fn a_long_pause_snaps_back_to_live() {
        let mut t = Tracker::default();
        watching(&mut t);
        t.set_playing(false, T0 + 20 * MIN);
        t.set_playing(true, T0 + 25 * MIN);
        let pos = write_pos(t.end(T0 + 40 * MIN, None));
        assert!((pos - (100.0 * 60.0 - BACKOFF_SECS)).abs() < 0.001, "{pos}");
    }

    #[test]
    fn a_broadcast_watched_to_its_end_forgets_its_bookmarks() {
        let mut t = Tracker::default();
        watching(&mut t);
        t.note_ended("caedrel");
        assert_eq!(t.end(T0 + 60 * MIN, None), Outcome::Forget { video_id: "v1".into() });
    }

    #[test]
    fn ending_while_rewound_keeps_the_rewind_position() {
        let mut t = Tracker::default();
        watching(&mut t);
        t.begin_rewind("caedrel", archive(T0 - 60 * MIN));
        assert!(t.note_rewind_position("v1", 1234.0));
        t.note_ended("caedrel");
        assert_eq!(write_pos(t.end(T0 + 20 * MIN, None)), 1234.0);
    }

    #[test]
    fn a_rewind_with_no_report_files_nothing() {
        let mut t = Tracker::default();
        watching(&mut t);
        t.begin_rewind("caedrel", archive(T0 - 60 * MIN));
        assert_eq!(t.end(T0 + 20 * MIN, None), Outcome::Nothing);
    }

    #[test]
    fn rewind_reports_for_another_video_are_ignored() {
        let mut t = Tracker::default();
        watching(&mut t);
        t.begin_rewind("caedrel", archive(T0 - 60 * MIN));
        assert!(!t.note_rewind_position("other", 99.0));
    }

    #[test]
    fn back_to_live_uses_the_live_position_again() {
        let mut t = Tracker::default();
        watching(&mut t);
        t.begin_rewind("caedrel", archive(T0 - 60 * MIN));
        t.note_rewind_position("v1", 600.0);
        t.begin_live("caedrel", true, T0 + 15 * MIN);
        let pos = write_pos(t.end(T0 + 20 * MIN, None));
        assert!((pos - (80.0 * 60.0 - BACKOFF_SECS)).abs() < 0.001, "{pos}");
    }

    #[test]
    fn an_unknown_recording_is_resolved_later() {
        let mut t = Tracker::default();
        t.begin_live("caedrel", true, T0);
        t.set_playing(true, T0);
        assert_eq!(
            t.end(T0 + 20 * MIN, None),
            Outcome::Resolve { login: "caedrel".into(), left_ms: T0 + 20 * MIN, behind_secs: 0.0 }
        );
    }

    #[test]
    fn a_previous_broadcasts_recording_is_not_trusted() {
        let mut t = Tracker::default();
        watching(&mut t);
        // The broadcast began 30 min ago; the recording an hour ago.
        let out = t.end(T0 + 20 * MIN, Some(T0 - 30 * MIN));
        assert!(matches!(out, Outcome::Resolve { .. }), "{out:?}");
    }

    #[test]
    fn resume_setting_off_files_nothing() {
        let mut t = Tracker::default();
        t.begin_live("caedrel", false, T0);
        t.note_archive("caedrel", archive(T0 - 60 * MIN));
        t.set_playing(true, T0);
        assert_eq!(t.end(T0 + 20 * MIN, None), Outcome::Nothing);
    }

    #[test]
    fn a_restart_of_the_same_channel_keeps_counting() {
        let mut t = Tracker::default();
        watching(&mut t);
        assert_eq!(t.end(T0 + 6 * MIN, None), Outcome::Nothing);
        t.begin_live("caedrel", true, T0 + 6 * MIN);
        t.set_playing(true, T0 + 6 * MIN);
        // Six minutes before the restart, five after: over the minimum.
        assert!(matches!(t.end(T0 + 11 * MIN, None), Outcome::Write { .. }));
    }

    #[test]
    fn same_channel_start_does_not_end_the_session() {
        let mut t = Tracker::default();
        watching(&mut t);
        assert_eq!(t.starting(Some("CaEdReL"), T0 + 20 * MIN, None), Outcome::Nothing);
        assert!(matches!(t.starting(None, T0 + 20 * MIN, None), Outcome::Write { .. }));
    }

    #[test]
    fn ending_twice_files_once() {
        let mut t = Tracker::default();
        watching(&mut t);
        assert!(matches!(t.end(T0 + 20 * MIN, None), Outcome::Write { .. }));
        assert_eq!(t.end(T0 + 21 * MIN, None), Outcome::Nothing);
    }

    #[test]
    fn the_first_half_minute_is_not_worth_resuming() {
        // 40 s in, backed off 20 s: 20 s, under the resume minimum.
        assert!(map_position(&archive(T0), T0 + 40_000, 0.0).is_none());
        assert!(map_position(&archive(T0), T0 + 60_000, 0.0).is_some());
    }

    #[test]
    fn mark_shows_only_older_positions_far_from_live() {
        // Filed an hour before this playback began, two hours behind live.
        assert_eq!(mark_position(3600.0, (T0 - 60 * MIN) / 1000, Some(T0), 3.0 * 3600.0), Some(3600.0));
        // Filed during this playback: the viewer's own rewind.
        assert_eq!(mark_position(3600.0, T0 / 1000 + 5, Some(T0), 3.0 * 3600.0), None);
        // A minute behind live.
        assert_eq!(mark_position(3600.0, (T0 - MIN) / 1000, Some(T0), 3660.0), None);
        // Too early to resume.
        assert_eq!(mark_position(10.0, (T0 - MIN) / 1000, Some(T0), 3660.0), None);
    }

    #[test]
    fn unknown_play_ids_write_nothing() {
        assert_eq!(origin_of(None), None);
        assert_eq!(origin_of(Some(u64::MAX)), None);
        let rewind = register_play(Origin::Rewind);
        let open = register_play(Origin::Deliberate);
        assert_eq!(origin_of(Some(rewind)), Some(Origin::Rewind));
        assert_eq!(origin_of(Some(open)), Some(Origin::Deliberate));
    }

    #[test]
    fn the_play_list_is_bounded() {
        let mut plays = VecDeque::new();
        for i in 0..(PLAYS_KEPT as u64 + 10) {
            push_play(&mut plays, i, Origin::Deliberate);
        }
        assert_eq!(plays.len(), PLAYS_KEPT);
        assert_eq!(plays.front().map(|p| p.0), Some(10));
    }
}
