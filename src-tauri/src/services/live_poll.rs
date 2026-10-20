//! Go-live alerts found by polling: the one check both phones run in the
//! background, and the record that keeps any broadcast from alerting twice.
//!
//! Android runs it from a WorkManager job every 15 minutes
//! (`android_notify.rs`), beside its FCM push lane. The iPhone runs it from a
//! background app refresh (`live_poll_check`), a best-effort fallback beside its
//! APNs push lane: iOS decides when that refresh runs, so it may be late or
//! skipped.
//!
//! The record is `notify_state.json`: per channel, the `started_at` of the
//! broadcast already announced. Every lane checks and writes it, so a push, a
//! poll and a duplicate of either collapse into one alert. The first run after
//! an install finds no file and records what is live without announcing it, so
//! a fresh install does not empty the whole following list into the shade.
//!
//! On the iPhone a push shows without any app code running, so it never
//! reaches this record by itself. The app hands over what it can see instead:
//! the go-live notifications still on screen at each refresh (their
//! identifier is `live-<channel_id>`, the push's collapse id), and every
//! go-live it presents, opens or sees dismissed. A channel with one of those
//! counts as already shown for that broadcast.
//!
//! A push banner swiped away before the refresh leaves no trace at all, so the
//! iPhone also skips (records without announcing) a channel the push server
//! holds for this phone once a push has been seen here lately: the push lane
//! is working and has that channel, and a second banner is the worse failure.
//! Until a push has been seen (no APNs key on the server yet, a token that
//! never registered) and for channels followed outside the app since the last
//! registration, the poll announces as Android's does.
//!
//! The rules here are plain functions over the record, tested on desktop; the
//! file, the lock, the Twitch calls and the commands are phones only.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Announced broadcasts are remembered this long after they were last seen
/// live. Long enough that a channel which drops off the live list and returns
/// is not announced twice, short enough that the file cannot grow without
/// bound.
pub const SHOWN_TTL_SECS: i64 = 7 * 24 * 60 * 60;

/// Two start times this close together are one broadcast. The push carries the
/// start time EventSub reports (with milliseconds), the poll the one Helix
/// reports (whole seconds), so the strings for one broadcast need not match.
const SAME_BROADCAST_SECS: i64 = 5;

/// A push seen within this long proves the push lane works on this phone.
pub const PUSH_PROOF_SECS: i64 = 7 * 24 * 60 * 60;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ShownEntry {
    /// The broadcast's own start time. Keyed on this rather than on "is live"
    /// so a genuinely new broadcast is always announced while an ongoing one
    /// never is, however the live list churns between runs.
    pub started_at: String,
    pub seen_at: i64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct NotifyState {
    #[serde(default)]
    pub shown: HashMap<String, ShownEntry>,
    /// When a go-live push was last seen on this phone (Unix seconds). The
    /// iPhone only; Android's file never carries it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub push_seen_at: Option<i64>,
}

/// The channels the push server holds for this phone, from its last
/// registration.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PushCover {
    pub ids: Vec<String>,
}

/// A followed channel that is live now, as the poll found it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LiveStream {
    pub channel_id: String,
    pub login: String,
    pub name: String,
    pub started_at: String,
    pub title: String,
    pub game: String,
}

/// A go-live notification the iPhone holds or has shown: its identifier and,
/// when it carries them, the broadcast's start time and when it was delivered
/// (Unix seconds). `polled` marks one the poll posted; every other is a push.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct Sighting {
    pub id: String,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub date: Option<i64>,
    #[serde(default)]
    pub polled: bool,
}

/// Notes the pushes among `seen` as proof the push lane works.
fn note_pushes(state: &mut NotifyState, seen: &[Sighting], now: i64) {
    for s in seen.iter().filter(|s| !s.polled && s.id.starts_with("live-")) {
        let at = s.date.unwrap_or(now).min(now);
        if state.push_seen_at.is_none_or(|prev| at > prev) {
            state.push_seen_at = Some(at);
        }
    }
}

/// One go-live to post, worded the way the push words it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LiveAlert {
    pub channel_id: String,
    pub login: String,
    pub started_at: String,
    pub title: String,
    pub body: String,
}

/// The notification identifier (and thread) for a channel's go-live, the same
/// as the push's collapse id, so a poll and a push replace each other.
pub fn notification_id(channel_id: &str) -> String {
    format!("live-{channel_id}")
}

fn parse_time(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s.trim()).ok().map(|t| t.timestamp())
}

/// Whether two start times name the same broadcast.
pub fn same_broadcast(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    match (parse_time(a), parse_time(b)) {
        (Some(x), Some(y)) => (x - y).abs() <= SAME_BROADCAST_SECS,
        _ => false,
    }
}

/// Whether `a` started clearly after `b` (both readable).
fn later(a: &str, b: &str) -> bool {
    match (parse_time(a), parse_time(b)) {
        (Some(x), Some(y)) => x - y > SAME_BROADCAST_SECS,
        _ => false,
    }
}

/// The alert for a stream: "<name> is live", and the stream title (or what it
/// plays) as the body.
pub fn alert_for(stream: &LiveStream) -> LiveAlert {
    let body = if !stream.title.is_empty() {
        stream.title.clone()
    } else if !stream.game.is_empty() {
        format!("Playing {}", stream.game)
    } else {
        String::new()
    };
    LiveAlert {
        channel_id: stream.channel_id.clone(),
        login: stream.login.clone(),
        started_at: stream.started_at.clone(),
        title: format!("{} is live", stream.name),
        body,
    }
}

/// Whether one of `seen` is this stream's go-live for this broadcast. One with
/// no start time counts when it was delivered after the stream started, and
/// when it carries neither it counts: an alert left out is better than one
/// shown twice.
fn already_seen(seen: &[Sighting], stream: &LiveStream) -> bool {
    let id = notification_id(&stream.channel_id);
    seen.iter().filter(|s| s.id == id).any(|s| match s.started_at.as_deref() {
        Some(at) if !at.is_empty() => same_broadcast(at, &stream.started_at),
        _ => match (s.date, parse_time(&stream.started_at)) {
            (Some(date), Some(start)) => date + SAME_BROADCAST_SECS >= start,
            _ => true,
        },
    })
}

/// The poll's decision: which of the live streams to announce, recording each
/// in `state`. Muted logins (lowercase) are skipped and not recorded, so
/// unmuting during a broadcast announces it on the next run. `seeding` (no
/// record existed) records everything and announces nothing. A broadcast in
/// `seen` is recorded and not announced, and so is a channel in `cover` (the
/// iPhone's push registration) once a push was seen within
/// [`PUSH_PROOF_SECS`]. Entries not seen live for [`SHOWN_TTL_SECS`] are
/// dropped.
pub fn decide(
    state: &mut NotifyState,
    seeding: bool,
    streams: &[LiveStream],
    muted: &[String],
    seen: &[Sighting],
    cover: Option<&PushCover>,
    now: i64,
) -> Vec<LiveAlert> {
    note_pushes(state, seen, now);
    let pushes_work = state.push_seen_at.is_some_and(|at| now - at < PUSH_PROOF_SECS);
    let covered = |id: &str| pushes_work && cover.is_some_and(|c| c.ids.iter().any(|c| c == id));
    let mut out = Vec::new();
    for stream in streams {
        if stream.channel_id.is_empty() || muted.contains(&stream.login.to_lowercase()) {
            continue;
        }
        // Already announced THIS broadcast. `seen_at` is bumped rather than
        // left alone so a channel live for longer than the TTL is not pruned
        // and announced again while it still broadcasts.
        if let Some(prev) = state.shown.get_mut(&stream.channel_id) {
            if same_broadcast(&prev.started_at, &stream.started_at) {
                prev.seen_at = now;
                continue;
            }
        }
        state.shown.insert(
            stream.channel_id.clone(),
            ShownEntry { started_at: stream.started_at.clone(), seen_at: now },
        );
        if seeding || already_seen(seen, stream) || covered(&stream.channel_id) {
            continue;
        }
        out.push(alert_for(stream));
    }
    // Age out rather than dropping everything no longer live: a channel that
    // falls off the list and comes back with the same broadcast would
    // otherwise be announced a second time.
    state.shown.retain(|_, e| now - e.seen_at < SHOWN_TTL_SECS);
    out
}

/// A go-live seen outside the poll (Android's push, a banner the iPhone
/// presented): true when it is the first sighting of that broadcast, which is
/// then recorded. An older broadcast never replaces a newer one on record.
pub fn claim(state: &mut NotifyState, channel_id: &str, started_at: &str, now: i64) -> bool {
    if let Some(prev) = state.shown.get_mut(channel_id) {
        if same_broadcast(&prev.started_at, started_at) {
            prev.seen_at = now;
            return false;
        }
        if later(&prev.started_at, started_at) {
            return false;
        }
    }
    state.shown.insert(
        channel_id.to_string(),
        ShownEntry { started_at: started_at.to_string(), seen_at: now },
    );
    true
}

/// Records the go-lives the iPhone saw (presented, opened, dismissed or still
/// on screen), returning the channel ids that were new. A sighting without a
/// start time cannot be keyed to a broadcast and is left out.
pub fn record_seen(state: &mut NotifyState, seen: &[Sighting], now: i64) -> Vec<String> {
    note_pushes(state, seen, now);
    let mut fresh = Vec::new();
    for s in seen {
        let (Some(channel_id), Some(started_at)) = (s.id.strip_prefix("live-"), s.started_at.as_deref()) else {
            continue;
        };
        if channel_id.is_empty() || started_at.is_empty() {
            continue;
        }
        if claim(state, channel_id, started_at, now) {
            fresh.push(channel_id.to_string());
        }
    }
    fresh
}

/// The file, the lock, Twitch and the commands. Phones only.
#[cfg(mobile)]
pub mod poll {
    use super::*;
    use crate::services::cache_service;
    use crate::services::push_alerts::AlertInputs;
    use crate::services::twitch_service::TwitchService;
    use serde_json::Value;

    /// Serializes every read-modify-write of `notify_state.json`. The lanes
    /// run in one process on different threads (Android's WorkManager poll and
    /// its push receiver; the iPhone's refresh and its notification delegate),
    /// and without it a claim landing between a read and a write is dropped,
    /// which resurfaces as a duplicate alert far from the cause.
    static STATE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    pub fn now_secs() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    fn state_path() -> Option<std::path::PathBuf> {
        cache_service::get_app_data_dir().ok().map(|d| d.join("notify_state.json"))
    }

    fn read_state() -> NotifyState {
        state_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Whether the poll has ever run here. See the module doc: the first run
    /// records without announcing.
    fn state_exists() -> bool {
        state_path().map(|p| p.is_file()).unwrap_or(false)
    }

    fn write_state(state: &NotifyState) {
        if let Some(path) = state_path() {
            if let Ok(json) = serde_json::to_string(state) {
                let _ = std::fs::write(path, json);
            }
        }
    }

    /// The saved settings as loose JSON rather than the `Settings` struct:
    /// these paths run with no app around them (Android) or before the app has
    /// loaded anything (an iPhone refresh), and a field they do not care about
    /// failing to parse must not stop them.
    pub fn read_settings() -> Value {
        cache_service::get_app_data_dir()
            .ok()
            .map(|d| d.join("settings.json"))
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(Value::Null)
    }

    /// A `live_notifications` switch, on when never saved (the serde defaults).
    pub fn notif_flag(settings: &Value, name: &str) -> bool {
        settings
            .get("live_notifications")
            .and_then(|n| n.get(name))
            .and_then(|v| v.as_bool())
            .unwrap_or(true)
    }

    fn inputs(settings: &Value) -> AlertInputs {
        AlertInputs::from_json(settings.get("live_notifications"))
    }

    /// Whether go-live alerts are on here: the master switch, Live channels,
    /// and a platform that sends them. The push registration reads the same.
    pub fn live_on(settings: &Value) -> bool {
        let inputs = inputs(settings);
        inputs.enabled && inputs.live && crate::services::alert_types::works("live")
    }

    /// Followed channels that are live right now, fully paged.
    ///
    /// Not `TwitchService::get_followed_streams`: that one takes an `AppState`
    /// (there is none in Android's worker process) and is shared with desktop.
    /// `get_token` refreshes an expired token against the compiled-in secret
    /// and persists it, which is why this runs in Rust and not in the shells.
    async fn fetch_followed_live() -> anyhow::Result<Vec<LiveStream>> {
        const CLIENT_ID: &str = env!("TWITCH_APP_CLIENT_ID");
        let token = TwitchService::get_token().await?;
        let me = TwitchService::get_user_info().await?;
        let client = crate::services::http::client().clone();

        let mut out = Vec::new();
        let mut cursor: Option<String> = None;
        // A hard stop rather than `loop`: a cursor that keeps returning itself
        // would otherwise spin a background job against Twitch forever.
        for _ in 0..10 {
            let mut url = format!("https://api.twitch.tv/helix/streams/followed?user_id={}&first=100", me.id);
            if let Some(c) = &cursor {
                url.push_str(&format!("&after={}", c));
            }
            let resp = client
                .get(&url)
                .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", token))
                .header("Client-Id", CLIENT_ID)
                .send()
                .await?
                .json::<Value>()
                .await?;

            match resp.get("data").and_then(|d| d.as_array()) {
                Some(arr) if !arr.is_empty() => {
                    for v in arr {
                        if let Ok(s) = serde_json::from_value::<crate::models::stream::TwitchStream>(v.clone()) {
                            out.push(LiveStream {
                                channel_id: s.user_id,
                                login: s.user_login,
                                name: s.user_name,
                                started_at: s.started_at,
                                title: s.title,
                                game: s.game_name,
                            });
                        }
                    }
                }
                _ => break,
            }

            cursor = resp
                .get("pagination")
                .and_then(|p| p.get("cursor"))
                .and_then(|c| c.as_str())
                .map(|s| s.to_string());
            if cursor.is_none() {
                break;
            }
        }
        Ok(out)
    }

    /// The live check: the followed channels live now, minus the muted ones
    /// and every broadcast already announced or in `seen`, recorded before
    /// they are returned so a failure after this costs an alert, never a
    /// repeat. The caller checks the switches first. Any failure (not signed
    /// in, no network) is an empty list: a background check stays quiet.
    pub async fn collect(settings: &Value, seen: &[Sighting], cover: Option<&PushCover>) -> Vec<LiveAlert> {
        let muted = inputs(settings).muted;
        // Network happens BEFORE the lock, so the lock never spans an await.
        let Ok(streams) = fetch_followed_live().await else {
            return Vec::new();
        };
        let _guard = STATE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let seeding = !state_exists();
        let mut state = read_state();
        let out = decide(&mut state, seeding, &streams, &muted, seen, cover, now_secs());
        write_state(&state);
        out
    }

    const PUSH_FOLLOWS_FILE: &str = "push_follows.json";

    /// Keeps the channel ids the push server last accepted for this phone
    /// (`None`: the phone was dropped from it). The iPhone's poll reads them;
    /// called by `push_alerts` after each registration.
    pub fn note_registered(ids: Option<&[String]>) {
        let Ok(dir) = cache_service::get_app_data_dir() else { return };
        let path = dir.join(PUSH_FOLLOWS_FILE);
        match ids {
            Some(ids) => {
                if let Ok(json) = serde_json::to_string(&PushCover { ids: ids.to_vec() }) {
                    let _ = std::fs::write(path, json);
                }
            }
            None => {
                let _ = std::fs::remove_file(path);
            }
        }
    }

    fn push_cover() -> Option<PushCover> {
        let dir = cache_service::get_app_data_dir().ok()?;
        let text = std::fs::read_to_string(dir.join(PUSH_FOLLOWS_FILE)).ok()?;
        serde_json::from_str(&text).ok()
    }

    /// Android's push gate: may this go-live show, recording that it did. The
    /// switches and mutes are the poll's.
    ///
    /// With no record yet (a fresh install) it shows and records NOTHING: the
    /// poll's silent first run is keyed on the file's absence, and creating it
    /// here would make that run announce every followed channel already live.
    /// A push is safe to show unrecorded, as it exists because a broadcast just
    /// started, and the first run records that channel without announcing it.
    pub fn claim_push(channel_id: &str, started_at: &str, login: &str) -> bool {
        let settings = read_settings();
        if !notif_flag(&settings, "enabled") || !notif_flag(&settings, "show_live_notifications") {
            return false;
        }
        let login = login.trim().to_lowercase();
        if inputs(&settings).muted.iter().any(|m| *m == login) {
            return false;
        }
        let _guard = STATE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        if !state_exists() {
            return true;
        }
        let mut state = read_state();
        let fresh = claim(&mut state, channel_id, started_at, now_secs());
        write_state(&state);
        fresh
    }

    /// What one iPhone refresh found: the go-lives to post, and whether alerts
    /// are on at all (when off, the app stops scheduling refreshes until it
    /// next goes to the background with them on).
    #[cfg(target_os = "ios")]
    #[derive(Debug, Serialize)]
    pub struct CheckRun {
        pub active: bool,
        pub alerts: Vec<LiveAlert>,
    }

    /// The iPhone's background live check. `delivered` is every go-live
    /// notification the iPhone still shows (`live-<channel_id>`, with the
    /// broadcast's start time when it has one); a channel among them is not
    /// announced again. Returns the go-lives the app should post, already
    /// recorded as shown.
    #[cfg(target_os = "ios")]
    #[tauri::command]
    pub async fn live_poll_check(delivered: Vec<Sighting>) -> Result<CheckRun, String> {
        let settings = read_settings();
        if !live_on(&settings) {
            return Ok(CheckRun { active: false, alerts: Vec::new() });
        }
        let alerts = collect(&settings, &delivered, push_cover().as_ref()).await;
        log::info!("[LivePoll] refresh: {} delivered on screen, {} to post", delivered.len(), alerts.len());
        Ok(CheckRun { active: true, alerts })
    }

    /// Go-lives the iPhone presented, opened, saw dismissed or still shows,
    /// recorded so the poll never announces those broadcasts again. Returns
    /// the channel ids that had not been recorded before (an empty list for a
    /// banner the poll itself already showed). Before the first poll it
    /// records nothing and reports every one as new, like Android's push gate.
    #[cfg(target_os = "ios")]
    #[tauri::command]
    pub async fn live_poll_seen(seen: Vec<Sighting>) -> Result<Vec<String>, String> {
        let _guard = STATE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        if !state_exists() {
            return Ok(seen.iter().filter_map(|s| s.id.strip_prefix("live-").map(str::to_string)).collect());
        }
        let mut state = read_state();
        let fresh = record_seen(&mut state, &seen, now_secs());
        write_state(&state);
        Ok(fresh)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000;

    fn stream(id: &str, login: &str, started_at: &str) -> LiveStream {
        LiveStream {
            channel_id: id.into(),
            login: login.into(),
            name: login.to_uppercase(),
            started_at: started_at.into(),
            title: format!("{login} title"),
            game: String::new(),
        }
    }

    fn ids(alerts: &[LiveAlert]) -> Vec<&str> {
        alerts.iter().map(|a| a.channel_id.as_str()).collect()
    }

    fn sighting(channel: &str, started_at: Option<&str>, date: Option<i64>) -> Sighting {
        Sighting { id: notification_id(channel), started_at: started_at.map(str::to_string), date, polled: false }
    }

    /// The decision without a push registration (Android, or an iPhone that
    /// never registered).
    fn decide(
        state: &mut NotifyState,
        seeding: bool,
        streams: &[LiveStream],
        muted: &[String],
        seen: &[Sighting],
        now: i64,
    ) -> Vec<LiveAlert> {
        super::decide(state, seeding, streams, muted, seen, None, now)
    }

    const T1: &str = "2026-10-19T18:00:00Z";
    const T2: &str = "2026-10-19T22:30:00Z";

    #[test]
    fn the_first_run_records_everything_and_announces_nothing() {
        let mut state = NotifyState::default();
        let live = [stream("1", "alpha", T1), stream("2", "bravo", T1)];
        assert!(decide(&mut state, true, &live, &[], &[], NOW).is_empty());
        assert_eq!(state.shown.len(), 2);
        // The next run knows them both.
        assert!(decide(&mut state, false, &live, &[], &[], NOW + 900).is_empty());
    }

    #[test]
    fn a_new_broadcast_alerts_once_and_never_again() {
        let mut state = NotifyState::default();
        decide(&mut state, true, &[], &[], &[], NOW);
        let live = [stream("1", "alpha", T1)];
        let first = decide(&mut state, false, &live, &[], &[], NOW);
        assert_eq!(first, vec![LiveAlert {
            channel_id: "1".into(),
            login: "alpha".into(),
            started_at: T1.into(),
            title: "ALPHA is live".into(),
            body: "alpha title".into(),
        }]);
        for run in 1..5 {
            assert!(decide(&mut state, false, &live, &[], &[], NOW + run * 900).is_empty(), "run {run}");
        }
        // Gone a while, then a new broadcast: that one alerts.
        assert!(decide(&mut state, false, &[], &[], &[], NOW + 9000).is_empty());
        assert_eq!(ids(&decide(&mut state, false, &[stream("1", "alpha", T2)], &[], &[], NOW + 9900)), vec!["1"]);
    }

    #[test]
    fn a_push_start_time_with_milliseconds_is_the_same_broadcast() {
        assert!(same_broadcast("2026-10-19T18:00:00.123Z", T1));
        assert!(same_broadcast(T1, "2026-10-19T18:00:03Z"));
        assert!(!same_broadcast(T1, T2));
        assert!(same_broadcast("", ""));
        assert!(!same_broadcast("", T1));
        let mut state = NotifyState::default();
        decide(&mut state, true, &[], &[], &[], NOW);
        // Android's push claimed it with EventSub's time; the poll reads Helix's.
        assert!(claim(&mut state, "1", "2026-10-19T18:00:00.123Z", NOW));
        assert!(decide(&mut state, false, &[stream("1", "alpha", T1)], &[], &[], NOW + 900).is_empty());
    }

    #[test]
    fn a_push_still_on_screen_counts_as_shown() {
        let mut state = NotifyState::default();
        decide(&mut state, true, &[], &[], &[], NOW);
        let live = [stream("1", "alpha", T1), stream("2", "bravo", T1), stream("3", "charlie", T1)];
        let delivered = [
            sighting("1", Some("2026-10-19T18:00:00.456Z"), None),
            // No start time: delivered after the stream started, so this broadcast.
            sighting("2", None, Some(parse_time(T1).unwrap() + 4)),
            // Yesterday's banner for charlie is not this broadcast.
            sighting("3", Some("2026-10-18T18:00:00Z"), None),
        ];
        assert_eq!(ids(&decide(&mut state, false, &live, &[], &delivered, NOW)), vec!["3"]);
        // All three are recorded: none alerts on the next run, banners gone or not.
        assert!(decide(&mut state, false, &live, &[], &[], NOW + 900).is_empty());
    }

    #[test]
    fn a_delivered_banner_older_than_the_broadcast_does_not_hide_it() {
        let mut state = NotifyState::default();
        decide(&mut state, true, &[], &[], &[], NOW);
        let old = sighting("1", None, Some(parse_time(T1).unwrap() - 3600));
        assert_eq!(ids(&decide(&mut state, false, &[stream("1", "alpha", T1)], &[], &[old], NOW)), vec!["1"]);
        // Nothing to go on at all: left out rather than risk a second alert.
        let mut state = NotifyState::default();
        decide(&mut state, true, &[], &[], &[], NOW);
        let bare = sighting("1", None, None);
        assert!(decide(&mut state, false, &[stream("1", "alpha", T1)], &[], &[bare], NOW).is_empty());
    }

    #[test]
    fn muted_channels_neither_alert_nor_record() {
        let mut state = NotifyState::default();
        decide(&mut state, true, &[], &[], &[], NOW);
        let live = [stream("1", "Alpha", T1), stream("2", "bravo", T1)];
        assert_eq!(ids(&decide(&mut state, false, &live, &["alpha".into()], &[], NOW)), vec!["2"]);
        assert!(!state.shown.contains_key("1"));
        // Unmuted during the broadcast: it alerts on the next run, once.
        assert_eq!(ids(&decide(&mut state, false, &live, &[], &[], NOW + 900)), vec!["1"]);
        assert!(decide(&mut state, false, &live, &[], &[], NOW + 1800).is_empty());
    }

    #[test]
    fn a_long_broadcast_stays_recorded_past_the_ttl() {
        let mut state = NotifyState::default();
        decide(&mut state, true, &[], &[], &[], NOW);
        let live = [stream("1", "alpha", T1)];
        assert_eq!(decide(&mut state, false, &live, &[], &[], NOW).len(), 1);
        let mut at = NOW;
        while at < NOW + 2 * SHOWN_TTL_SECS {
            at += 6 * 60 * 60;
            assert!(decide(&mut state, false, &live, &[], &[], at).is_empty());
        }
        // Not seen for the TTL: dropped.
        decide(&mut state, false, &[], &[], &[], at + SHOWN_TTL_SECS);
        assert!(state.shown.is_empty());
    }

    #[test]
    fn the_body_is_the_title_then_the_game() {
        let mut s = stream("1", "alpha", T1);
        assert_eq!(alert_for(&s).body, "alpha title");
        s.title.clear();
        s.game = "Chess".into();
        assert_eq!(alert_for(&s).body, "Playing Chess");
        s.game.clear();
        assert_eq!(alert_for(&s).body, "");
        assert_eq!(notification_id("123"), "live-123");
    }

    #[test]
    fn a_claim_is_true_once_per_broadcast() {
        let mut state = NotifyState::default();
        assert!(claim(&mut state, "1", T1, NOW));
        assert!(!claim(&mut state, "1", T1, NOW + 1));
        assert!(!claim(&mut state, "1", "2026-10-19T18:00:00.900Z", NOW + 2));
        assert!(claim(&mut state, "1", T2, NOW + 3), "a new broadcast");
        assert!(!claim(&mut state, "1", T1, NOW + 4), "an older broadcast never replaces a newer one");
        assert_eq!(state.shown["1"].started_at, T2);
    }

    #[test]
    fn banners_the_iphone_saw_keep_the_poll_quiet() {
        let mut state = NotifyState::default();
        decide(&mut state, true, &[], &[], &[], NOW);
        let seen = [
            sighting("1", Some("2026-10-19T18:00:00.250Z"), None),
            sighting("2", None, Some(NOW)),
            Sighting { id: "mention-9".into(), started_at: Some(T1.into()), ..Default::default() },
        ];
        assert_eq!(record_seen(&mut state, &seen, NOW), vec!["1".to_string()]);
        assert_eq!(record_seen(&mut state, &seen, NOW + 1), Vec::<String>::new(), "seen twice is not new");
        // The push banner was dismissed before the refresh ran: still no repeat.
        let live = [stream("1", "alpha", T1), stream("2", "bravo", T1)];
        assert_eq!(ids(&decide(&mut state, false, &live, &[], &[], NOW + 900)), vec!["2"]);
    }

    #[test]
    fn a_banner_the_poll_posted_is_not_new_when_presented() {
        let mut state = NotifyState::default();
        decide(&mut state, true, &[], &[], &[], NOW);
        let posted = decide(&mut state, false, &[stream("1", "alpha", T1)], &[], &[], NOW);
        let back = Sighting { polled: true, ..sighting(&posted[0].channel_id, Some(&posted[0].started_at), Some(NOW)) };
        assert!(record_seen(&mut state, &[back], NOW + 1).is_empty());
        assert_eq!(state.push_seen_at, None, "the poll's own banner proves nothing about pushes");
    }

    #[test]
    fn once_pushes_arrive_the_poll_leaves_registered_channels_to_them() {
        let mut state = NotifyState::default();
        decide(&mut state, true, &[], &[], &[], NOW);
        // A push for some channel was presented: the push lane works here.
        record_seen(&mut state, &[sighting("9", Some(T1), Some(NOW - 60))], NOW);
        assert_eq!(state.push_seen_at, Some(NOW - 60));
        let cover = PushCover { ids: vec!["1".into()] };
        // Alpha is on the push server, so its banner (perhaps swiped away
        // already) came by push; bravo was followed outside the app since.
        let live = [stream("1", "alpha", T2), stream("2", "bravo", T2)];
        assert_eq!(ids(&super::decide(&mut state, false, &live, &[], &[], Some(&cover), NOW + 900)), vec!["2"]);
        assert!(super::decide(&mut state, false, &live, &[], &[], Some(&cover), NOW + 1800).is_empty());
    }

    #[test]
    fn without_a_recent_push_the_poll_covers_registered_channels_too() {
        let cover = PushCover { ids: vec!["1".into(), "2".into()] };
        let live = [stream("1", "alpha", T2), stream("2", "bravo", T2)];

        // Never a push (no APNs key on the server yet, say).
        let mut state = NotifyState::default();
        decide(&mut state, true, &[], &[], &[], NOW);
        assert_eq!(ids(&super::decide(&mut state, false, &live, &[], &[], Some(&cover), NOW)), vec!["1", "2"]);

        // The last push was too long ago.
        let mut state = NotifyState::default();
        decide(&mut state, true, &[], &[], &[], NOW);
        state.push_seen_at = Some(NOW - PUSH_PROOF_SECS);
        assert_eq!(ids(&super::decide(&mut state, false, &live, &[], &[], Some(&cover), NOW)), vec!["1", "2"]);

        // Only banners the poll posted on screen.
        let mut state = NotifyState::default();
        decide(&mut state, true, &[], &[], &[], NOW);
        let ours = Sighting { polled: true, ..sighting("7", Some(T1), Some(NOW)) };
        assert_eq!(ids(&super::decide(&mut state, false, &live, &[], &[ours], Some(&cover), NOW)), vec!["1", "2"]);
    }

    #[test]
    fn a_push_on_screen_at_the_refresh_is_proof_too() {
        let mut state = NotifyState::default();
        decide(&mut state, true, &[], &[], &[], NOW);
        let cover = PushCover { ids: vec!["1".into(), "2".into()] };
        let live = [stream("1", "alpha", T2), stream("2", "bravo", T2)];
        let on_screen = [sighting("1", Some(T2), Some(NOW - 30))];
        assert!(super::decide(&mut state, false, &live, &[], &on_screen, Some(&cover), NOW).is_empty());
    }

    #[test]
    fn an_android_state_file_never_carries_push_proof() {
        let mut state = NotifyState::default();
        decide(&mut state, true, &[stream("1", "alpha", T1)], &[], &[], NOW);
        claim(&mut state, "2", T1, NOW);
        let json = serde_json::to_string(&state).unwrap();
        assert!(!json.contains("push_seen_at"), "{json}");
    }

    #[test]
    fn sightings_read_from_the_iphone() {
        let parsed: Vec<Sighting> = serde_json::from_str(
            r#"[{"id":"live-1","started_at":"2026-10-19T18:00:00Z","date":1800000000},{"id":"live-2"}]"#,
        )
        .unwrap();
        assert_eq!(parsed[0], sighting("1", Some(T1), Some(1_800_000_000)));
        assert_eq!(parsed[1], sighting("2", None, None));
    }

    #[test]
    fn a_state_file_from_an_older_build_reads() {
        let state: NotifyState =
            serde_json::from_str(r#"{"shown":{"1":{"started_at":"2026-10-19T18:00:00Z","seen_at":5}}}"#).unwrap();
        assert_eq!(state.shown["1"], ShownEntry { started_at: T1.into(), seen_at: 5 });
        let empty: NotifyState = serde_json::from_str("{}").unwrap();
        assert!(empty.shown.is_empty());
    }
}
