//! When reminders fire. A reminder is a message StreamNook posts into a
//! streamer's chat on the user's behalf, on one of five triggers: a repeating
//! interval, a one-time delay after joining, a clock time, a stream-uptime
//! threshold, or a keyword appearing in chat.
//!
//! This used to tick on a webview timer, which throttles while the window is
//! hidden, and matched keywords inside every window's chat store, so a
//! channel-specific keyword reminder fired once per window showing that
//! channel. Rust now decides when; `reminders://fire { id, channel, is_current }`
//! goes to the main window, which expands the message template and posts it
//! (the sent row lives in its chat store).
//!
//! Reminders live in settings (`reminders.reminders`); `refresh` recompiles them
//! on every settings change. The one-second tick runs only while an enabled
//! time-based reminder exists.

use crate::models::chat_layout::ChatMessage;
use crate::models::settings::Settings;
use regex::Regex;
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

pub const FIRE_EVENT: &str = "reminders://fire";

const TICK: Duration = Duration::from_secs(1);
/// A clock reminder still fires this many minutes late (a missed tick, or a
/// join just after the time), but not hours late.
const CLOCK_CATCH_UP_MINUTES: u32 = 2;
const DEFAULT_KEYWORD_COOLDOWN_MINUTES: f64 = 5.0;

#[derive(Debug, Clone, Deserialize, Default)]
struct Reminder {
    id: String,
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    trigger: String,
    #[serde(default)]
    channel_scope: Option<String>,
    #[serde(default)]
    channel_login: Option<String>,
    #[serde(default)]
    interval_seconds: Option<f64>,
    #[serde(default)]
    interval_minutes: Option<f64>,
    #[serde(default)]
    delay_seconds: Option<f64>,
    #[serde(default)]
    delay_minutes: Option<f64>,
    #[serde(default)]
    clock_time: Option<String>,
    #[serde(default)]
    uptime_seconds: Option<f64>,
    #[serde(default)]
    uptime_minutes: Option<f64>,
    #[serde(default)]
    keyword: Option<String>,
    #[serde(default)]
    keyword_match: Option<String>,
    #[serde(default)]
    keyword_from: Option<String>,
    #[serde(default)]
    keyword_cooldown_minutes: Option<f64>,
}

impl Reminder {
    /// Seconds from the new field, else the older minutes field.
    fn seconds(secs: Option<f64>, minutes: Option<f64>) -> u64 {
        secs.or(minutes.map(|m| m * 60.0)).unwrap_or(0.0).max(0.0).floor() as u64
    }
    fn interval(&self) -> u64 {
        Self::seconds(self.interval_seconds, self.interval_minutes)
    }
    fn delay(&self) -> u64 {
        Self::seconds(self.delay_seconds, self.delay_minutes)
    }
    fn uptime(&self) -> u64 {
        Self::seconds(self.uptime_seconds, self.uptime_minutes)
    }
    /// The login a `specific` reminder posts to (None for `current`).
    fn pinned_channel(&self) -> Option<String> {
        (self.channel_scope.as_deref() == Some("specific"))
            .then(|| self.channel_login.clone().unwrap_or_default().to_lowercase())
    }
    fn is_timed(&self) -> bool {
        self.enabled && matches!(self.trigger.as_str(), "interval" | "delay" | "clock" | "uptime")
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum KeywordMode {
    Contains,
    Exact,
    Word,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum KeywordFrom {
    Anyone,
    Mods,
    Broadcaster,
}

#[derive(Debug, Clone)]
struct CompiledKeyword {
    id: String,
    keyword: String,
    mode: KeywordMode,
    word: Option<Regex>,
    from: KeywordFrom,
    cooldown_ms: i64,
    pinned_channel: Option<String>,
}

impl CompiledKeyword {
    fn compile(r: &Reminder) -> Option<Self> {
        if !r.enabled || r.trigger != "keyword" {
            return None;
        }
        let keyword = r.keyword.as_deref().unwrap_or_default().trim().to_lowercase();
        if keyword.is_empty() {
            return None;
        }
        let mode = match r.keyword_match.as_deref() {
            Some("exact") => KeywordMode::Exact,
            Some("word") => KeywordMode::Word,
            _ => KeywordMode::Contains,
        };
        // Compiled once here, never per message.
        let word = if mode == KeywordMode::Word {
            let pattern = format!(r"(?i)(?:^|[^\p{{L}}\p{{N}}_]){}(?:$|[^\p{{L}}\p{{N}}_])", regex::escape(&keyword));
            Some(Regex::new(&pattern).ok()?)
        } else {
            None
        };
        let from = match r.keyword_from.as_deref() {
            Some("mods") => KeywordFrom::Mods,
            Some("broadcaster") => KeywordFrom::Broadcaster,
            _ => KeywordFrom::Anyone,
        };
        let cooldown_minutes = r.keyword_cooldown_minutes.unwrap_or(DEFAULT_KEYWORD_COOLDOWN_MINUTES).max(0.0).floor();
        Some(Self {
            id: r.id.clone(),
            keyword,
            mode,
            word,
            from,
            cooldown_ms: (cooldown_minutes * 60_000.0) as i64,
            pinned_channel: r.pinned_channel(),
        })
    }

    fn matches(&self, content: &str) -> bool {
        match self.mode {
            KeywordMode::Exact => content.trim().to_lowercase() == self.keyword,
            KeywordMode::Word => self.word.as_ref().is_some_and(|re| re.is_match(content)),
            KeywordMode::Contains => content.to_lowercase().contains(&self.keyword),
        }
    }

    fn sender_passes(&self, badges: &[&str]) -> bool {
        let broadcaster = badges.contains(&"broadcaster");
        match self.from {
            KeywordFrom::Anyone => true,
            KeywordFrom::Broadcaster => broadcaster,
            KeywordFrom::Mods => broadcaster || badges.contains(&"moderator"),
        }
    }
}

/// Per-reminder firing state. Kept across settings changes (keyed by id) so an
/// edit to one reminder does not reset the others' timers.
#[derive(Debug, Default, Clone)]
struct Runtime {
    /// The channel this reminder is armed for; interval and delay timers
    /// restart when it changes (a channel switch, a reconnect).
    armed_channel: Option<String>,
    armed_at_ms: i64,
    last_interval_ms: i64,
    delay_fired: bool,
    /// Local date the clock trigger last fired on (once per day).
    clock_fired_date: Option<String>,
    /// Stream start the uptime trigger last fired for (once per stream).
    uptime_fired_for: Option<String>,
    keyword_last_ms: i64,
}

#[derive(Default)]
struct State {
    timed: Vec<Reminder>,
    runtimes: HashMap<String, Runtime>,
    tick: Option<tauri::async_runtime::JoinHandle<()>>,
}

static APP: OnceLock<AppHandle> = OnceLock::new();
static STATE: Mutex<Option<State>> = Mutex::new(None);
static KEYWORDS: RwLock<Option<Arc<Vec<CompiledKeyword>>>> = RwLock::new(None);

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(State::default))
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn reminders_in(settings: &Settings) -> Vec<Reminder> {
    settings
        .extra
        .get("reminders")
        .and_then(|v| v.get("reminders"))
        .and_then(|v| serde_json::from_value::<Vec<serde_json::Value>>(v.clone()).ok())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|v| serde_json::from_value::<Reminder>(v).ok())
        .collect()
}

pub fn init(app: AppHandle, settings: &Settings) {
    let _ = APP.set(app);
    refresh(settings);
}

/// Recompile from settings: the keyword list, the timed list, and whether the
/// tick should run at all.
pub fn refresh(settings: &Settings) {
    let reminders = reminders_in(settings);
    let keywords: Vec<CompiledKeyword> = reminders.iter().filter_map(CompiledKeyword::compile).collect();
    if let Ok(mut slot) = KEYWORDS.write() {
        *slot = (!keywords.is_empty()).then(|| Arc::new(keywords));
    }
    let timed: Vec<Reminder> = reminders.into_iter().filter(Reminder::is_timed).collect();
    let start = with_state(|s| {
        s.timed = timed;
        if s.timed.is_empty() {
            if let Some(task) = s.tick.take() {
                task.abort();
            }
            false
        } else {
            s.tick.is_none()
        }
    });
    if start {
        let task = tauri::async_runtime::spawn(tick_loop());
        with_state(|s| {
            if s.tick.is_none() && !s.timed.is_empty() {
                s.tick = Some(task);
            } else {
                task.abort();
            }
        });
    }
}

fn fire(id: &str, channel: &str, is_current: bool) {
    if let Some(app) = APP.get() {
        let _ = app.emit(FIRE_EVENT, json!({ "id": id, "channel": channel, "is_current": is_current }));
    }
}

async fn tick_loop() {
    loop {
        tokio::time::sleep(TICK).await;
        tick().await;
    }
}

/// Where a reminder posts right now: its pinned channel, or the channel being
/// watched. The watched stream's start time rides along for uptime triggers.
fn resolve_target(r: &Reminder, watched: Option<&(String, String)>) -> Option<(String, bool, Option<String>)> {
    match r.pinned_channel() {
        Some(login) if login.is_empty() => None,
        Some(login) => {
            let current = watched.filter(|(w, _)| *w == login);
            Some((login.clone(), current.is_some(), current.map(|(_, s)| s.clone())))
        }
        None => watched.map(|(login, started)| (login.clone(), true, Some(started.clone()))),
    }
}

async fn tick() {
    if crate::services::chat_rules::ChatRules::own_identity().is_none() {
        return;
    }
    let watched = crate::services::watch_session::current_twitch_watch();
    let timed = with_state(|s| s.timed.clone());
    let mut joined: HashMap<String, bool> = HashMap::new();
    for r in &timed {
        let target = resolve_target(r, watched.as_ref());
        if let Some((login, _, _)) = &target {
            if !joined.contains_key(login) {
                joined.insert(login.clone(), crate::services::irc_service::IrcService::is_joined(login).await);
            }
        }
    }
    let now = now_ms();
    let local = chrono::Local::now();
    let mut fires = Vec::new();
    with_state(|s| {
        for r in &timed {
            let rt = s.runtimes.entry(r.id.clone()).or_default();
            let Some((login, is_current, started_at)) = resolve_target(r, watched.as_ref()) else {
                rt.armed_channel = None;
                continue;
            };
            if !joined.get(&login).copied().unwrap_or(false) {
                rt.armed_channel = None;
                continue;
            }
            if let Some(decision) = evaluate(r, rt, &login, is_current, started_at.as_deref(), now, &local) {
                fires.push(decision);
            }
        }
    });
    for (id, login, is_current) in fires {
        fire(&id, &login, is_current);
    }
}

/// Arm, then decide whether this reminder fires on this tick.
fn evaluate(
    r: &Reminder,
    rt: &mut Runtime,
    login: &str,
    is_current: bool,
    started_at: Option<&str>,
    now: i64,
    local: &chrono::DateTime<chrono::Local>,
) -> Option<(String, String, bool)> {
    use chrono::{Datelike, Timelike};
    // (Re)arm when the target changes. Resets the relative timers; the per-day
    // and per-stream guards intentionally persist.
    if rt.armed_channel.as_deref() != Some(login) {
        rt.armed_channel = Some(login.to_string());
        rt.armed_at_ms = now;
        rt.last_interval_ms = now;
        rt.delay_fired = false;
    }
    let fire = || Some((r.id.clone(), login.to_string(), is_current));
    match r.trigger.as_str() {
        "interval" => {
            let secs = r.interval();
            if secs >= 1 && now - rt.last_interval_ms >= secs as i64 * 1000 {
                rt.last_interval_ms = now;
                return fire();
            }
        }
        "delay" => {
            let secs = r.delay();
            if secs >= 1 && !rt.delay_fired && now - rt.armed_at_ms >= secs as i64 * 1000 {
                rt.delay_fired = true;
                return fire();
            }
        }
        "clock" => {
            let (h, m) = r.clock_time.as_deref().and_then(|t| {
                let mut parts = t.split(':');
                Some((parts.next()?.trim().parse::<u32>().ok()?, parts.next()?.trim().parse::<u32>().ok()?))
            })?;
            let today = format!("{}-{}-{}", local.year(), local.month(), local.day());
            let now_minutes = local.hour() * 60 + local.minute();
            let target_minutes = h * 60 + m;
            if rt.clock_fired_date.as_deref() != Some(today.as_str())
                && now_minutes >= target_minutes
                && now_minutes - target_minutes <= CLOCK_CATCH_UP_MINUTES
            {
                rt.clock_fired_date = Some(today);
                return fire();
            }
        }
        "uptime" => {
            // Needs the watched stream's start time.
            let started = started_at.filter(|s| is_current && !s.is_empty())?;
            let start_ms = chrono::DateTime::parse_from_rfc3339(started).ok()?.timestamp_millis();
            let secs = r.uptime();
            if secs >= 1 && rt.uptime_fired_for.as_deref() != Some(started) && (now - start_ms) / 1000 >= secs as i64 {
                rt.uptime_fired_for = Some(started.to_string());
                return fire();
            }
        }
        _ => {}
    }
    None
}

/// A chat message arrived: fire any keyword reminder it satisfies. Cheap when no
/// keyword reminder exists (one lock read).
pub fn on_message(msg: &ChatMessage) {
    let Some(keywords) = KEYWORDS.read().ok().and_then(|k| k.clone()) else { return };
    let Some((_, own_id)) = crate::services::chat_rules::ChatRules::own_identity() else { return };
    if msg.user_id.is_empty() || msg.user_id == own_id || msg.content.is_empty() {
        return;
    }
    let channel = msg.channel.trim_start_matches('#').to_lowercase();
    let watched = crate::services::watch_session::current_twitch_watch();
    let badges: Vec<&str> = msg.badges.iter().map(|b| b.name.as_str()).collect();
    let now = now_ms();
    let mut fires = Vec::new();
    with_state(|s| {
        for kw in keywords.iter() {
            let (target, is_current) = match &kw.pinned_channel {
                Some(login) => (login.clone(), watched.as_ref().is_some_and(|(w, _)| w == login)),
                None => match &watched {
                    Some((login, _)) => (login.clone(), true),
                    None => continue,
                },
            };
            if target != channel || !kw.sender_passes(&badges) || !kw.matches(&msg.content) {
                continue;
            }
            let rt = s.runtimes.entry(kw.id.clone()).or_default();
            if rt.keyword_last_ms > 0 && now - rt.keyword_last_ms < kw.cooldown_ms {
                continue;
            }
            rt.keyword_last_ms = now;
            fires.push((kw.id.clone(), target, is_current));
        }
    });
    for (id, login, is_current) in fires {
        fire(&id, &login, is_current);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn reminder(v: serde_json::Value) -> Reminder {
        serde_json::from_value(v).expect("reminder")
    }

    fn at(h: u32, m: u32) -> chrono::DateTime<chrono::Local> {
        chrono::Local.with_ymd_and_hms(2026, 9, 24, h, m, 0).earliest().unwrap()
    }

    #[test]
    fn durations_prefer_seconds_then_legacy_minutes() {
        assert_eq!(reminder(json!({"id":"a","interval_seconds":90})).interval(), 90);
        assert_eq!(reminder(json!({"id":"a","interval_minutes":2})).interval(), 120);
        assert_eq!(reminder(json!({"id":"a"})).interval(), 0);
    }

    #[test]
    fn an_interval_fires_after_its_period_from_arming() {
        let r = reminder(json!({"id":"i","enabled":true,"trigger":"interval","interval_seconds":60}));
        let mut rt = Runtime::default();
        let t = at(12, 0);
        assert!(evaluate(&r, &mut rt, "chan", true, None, 0, &t).is_none()); // arms
        assert!(evaluate(&r, &mut rt, "chan", true, None, 59_000, &t).is_none());
        assert!(evaluate(&r, &mut rt, "chan", true, None, 60_000, &t).is_some());
        // A channel change re-arms, so the period restarts.
        assert!(evaluate(&r, &mut rt, "other", true, None, 130_000, &t).is_none());
        assert!(evaluate(&r, &mut rt, "other", true, None, 190_000, &t).is_some());
    }

    #[test]
    fn a_delay_fires_once_per_arming() {
        let r = reminder(json!({"id":"d","enabled":true,"trigger":"delay","delay_seconds":10}));
        let mut rt = Runtime::default();
        let t = at(12, 0);
        evaluate(&r, &mut rt, "chan", true, None, 0, &t);
        assert!(evaluate(&r, &mut rt, "chan", true, None, 10_000, &t).is_some());
        assert!(evaluate(&r, &mut rt, "chan", true, None, 20_000, &t).is_none());
    }

    #[test]
    fn a_clock_reminder_fires_once_a_day_within_the_catch_up_window() {
        let r = reminder(json!({"id":"c","enabled":true,"trigger":"clock","clock_time":"20:30"}));
        let mut rt = Runtime::default();
        assert!(evaluate(&r, &mut rt, "chan", true, None, 0, &at(20, 29)).is_none());
        assert!(evaluate(&r, &mut rt, "chan", true, None, 0, &at(20, 31)).is_some());
        assert!(evaluate(&r, &mut rt, "chan", true, None, 0, &at(20, 32)).is_none());
        let mut late = Runtime::default();
        assert!(evaluate(&r, &mut late, "chan", true, None, 0, &at(20, 40)).is_none());
    }

    #[test]
    fn an_uptime_reminder_fires_once_per_stream_and_only_when_watched() {
        let r = reminder(json!({"id":"u","enabled":true,"trigger":"uptime","uptime_seconds":3600}));
        let start = "2026-09-24T10:00:00Z";
        let start_ms = chrono::DateTime::parse_from_rfc3339(start).unwrap().timestamp_millis();
        let t = at(12, 0);
        let mut rt = Runtime::default();
        assert!(evaluate(&r, &mut rt, "chan", false, Some(start), start_ms + 3_700_000, &t).is_none());
        assert!(evaluate(&r, &mut rt, "chan", true, Some(start), start_ms + 3_500_000, &t).is_none());
        assert!(evaluate(&r, &mut rt, "chan", true, Some(start), start_ms + 3_600_000, &t).is_some());
        assert!(evaluate(&r, &mut rt, "chan", true, Some(start), start_ms + 3_700_000, &t).is_none());
    }

    #[test]
    fn keyword_matching_follows_each_mode() {
        let kw = |mode: &str| {
            CompiledKeyword::compile(&reminder(json!({
                "id":"k","enabled":true,"trigger":"keyword","keyword":"Drop","keyword_match":mode
            })))
            .unwrap()
        };
        assert!(kw("contains").matches("any drops today?"));
        assert!(kw("exact").matches("  DROP "));
        assert!(!kw("exact").matches("drop it"));
        assert!(kw("word").matches("where is the drop?"));
        assert!(!kw("word").matches("any drops today?"));
        assert!(kw("word").matches("drop"));
    }

    #[test]
    fn sender_filters_read_badges() {
        let kw = |from: &str| {
            CompiledKeyword::compile(&reminder(json!({
                "id":"k","enabled":true,"trigger":"keyword","keyword":"x","keyword_from":from
            })))
            .unwrap()
        };
        assert!(kw("anyone").sender_passes(&[]));
        assert!(kw("mods").sender_passes(&["moderator"]));
        assert!(kw("mods").sender_passes(&["broadcaster"]));
        assert!(!kw("mods").sender_passes(&["vip"]));
        assert!(!kw("broadcaster").sender_passes(&["moderator"]));
    }

    #[test]
    fn disabled_or_empty_keywords_do_not_compile() {
        assert!(CompiledKeyword::compile(&reminder(json!({"id":"k","enabled":false,"trigger":"keyword","keyword":"x"}))).is_none());
        assert!(CompiledKeyword::compile(&reminder(json!({"id":"k","enabled":true,"trigger":"keyword","keyword":"  "}))).is_none());
    }
}
