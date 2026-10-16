//! When reminders fire. A reminder is a message StreamNook posts into a
//! streamer's chat on the user's behalf, on one of five triggers: a repeating
//! interval, a one-time delay after joining, a clock time, a stream-uptime
//! threshold, or a keyword appearing in chat.
//!
//! Rust decides when a reminder fires and posts it too: the message template is
//! expanded for the target room (`slash_dispatch::expand_for_room`) and every
//! copy waits for a slot from the chat send limiter, then goes out as the
//! signed-in account. A refusal stops the run and says so in that room's chat.
//! No window has to be open on the room for it to post.
//!
//! Reminders live in settings (`reminders.reminders`); `refresh` recompiles them
//! on every settings change. The one-second tick runs only while an enabled
//! time-based reminder exists. The `/remind` command's parser and the lines it
//! prints live here too.

use crate::models::chat_layout::ChatMessage;
use crate::models::settings::Settings;
use regex::Regex;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Duration;
use crate::rt::AppHandle;

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
    #[serde(default)]
    message: String,
    #[serde(default)]
    repeat_count: Option<f64>,
    #[serde(default)]
    channel_id: Option<String>,
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

/// Post the reminder found due into the room it resolved to. A reminder
/// deleted or turned off since is skipped.
fn fire(id: &str, channel: &str, is_current: bool) {
    let Some(app) = APP.get().cloned() else { return };
    let id = id.to_string();
    let channel = channel.to_string();
    tauri::async_runtime::spawn(async move {
        let reminder = {
            use tauri::Manager;
            let state = app.state::<crate::models::settings::AppState>();
            let settings = match state.settings.lock() {
                Ok(s) => s.clone(),
                Err(_) => return,
            };
            reminders_in(&settings).into_iter().find(|r| r.id == id)
        };
        let Some(r) = reminder.filter(|r| r.enabled) else { return };
        // The watched channel's id is looked up from its login; a pinned
        // reminder carries the id it was made with.
        let channel_id = if is_current { None } else { r.channel_id.clone().filter(|s| !s.is_empty()) };
        post_text(&channel, channel_id.as_deref(), &r.message, clamp_repeat(r.repeat_count)).await;
    });
}

/// Post a reminder's text into `channel` (bare login) `copies` times: the
/// template expanded for that room, each copy after the first made unique with
/// the invisible duplicate-bypass suffix, each waiting for a send slot. A
/// refusal stops the run and prints why in that room. False when nothing was
/// posted because no one is signed in or the text came out empty.
pub async fn post_text(channel: &str, channel_id: Option<&str>, template: &str, copies: u32) -> bool {
    if crate::services::chat_rules::ChatRules::own_identity().is_none() {
        return false;
    }
    let text = crate::services::slash_dispatch::expand_for_room(template, channel, channel_id.unwrap_or("")).await;
    let text = text.trim().to_string();
    if text.is_empty() {
        return false;
    }
    for j in 0..copies {
        let copy = with_duplicate_bypass(&text, j);
        if let Err(refusal) =
            crate::services::chat_send_limiter::chat_send_paced(channel.to_string(), channel_id.map(str::to_string), copy).await
        {
            crate::services::slash_dispatch::post_room_notice(channel, &format!("Reminder stopped: {refusal}")).await;
            break;
        }
    }
    true
}

// ───────────────────────────── the /remind command ─────────────────────────────

/// How many back-to-back copies one reminder may post.
pub const MAX_REPEAT: u32 = 10;

/// A repeat count kept between 1 and [`MAX_REPEAT`].
pub fn clamp_repeat(n: Option<f64>) -> u32 {
    match n.map(f64::floor) {
        Some(v) if v.is_finite() && v >= 1.0 => (v as u32).min(MAX_REPEAT),
        _ => 1,
    }
}

/// Copy `copy_index` of a repeated send: the first is the plain text, later
/// ones end in a growing run of the invisible tag character Twitch's
/// duplicate check ignores.
pub fn with_duplicate_bypass(text: &str, copy_index: u32) -> String {
    if copy_index == 0 {
        return text.to_string();
    }
    let tag = char::from_u32(0xE0000).unwrap_or(' ');
    format!("{text} {}", std::iter::repeat(tag).take(copy_index as usize).collect::<String>())
}

/// What a `/remind` line asks for.
#[derive(Debug, Clone, PartialEq)]
pub enum RemindParse {
    List,
    Help,
    Clear,
    Manage { op: &'static str, index: usize },
    SendNow { message: String, repeat: u32 },
    Create { reminder: Value, summary: String },
    Error(String),
}

const REMIND_USAGE: &str = "Usage: /remind every 20m <message> · in 30s · at 21:00 · uptime 2h · on <word> · now. Times accept s/m/h (30s, 90s, 1h30m). Add x3 to repeat. /remind list to manage, /remind help for everything.";

/// The leading integer of `raw`, the way JavaScript's `parseInt` reads it.
fn leading_int(raw: &str) -> Option<i64> {
    let s = raw.trim_start();
    let (neg, digits) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let end = digits.find(|c: char| !c.is_ascii_digit()).unwrap_or(digits.len());
    if end == 0 {
        return None;
    }
    let n: i64 = digits[..end].parse().unwrap_or(i64::MAX);
    Some(if neg { -n } else { n })
}

/// Seconds from "30s", "20m", "2h", "1h30m", or a bare number of minutes.
pub fn parse_duration_seconds(raw: &str) -> Option<u64> {
    let s = raw.trim().to_lowercase();
    if s.is_empty() {
        return None;
    }
    if s.chars().all(|c| c.is_ascii_digit()) {
        return Some(s.parse::<u64>().unwrap_or(u64::MAX / 60).saturating_mul(60));
    }
    let mut total: u64 = 0;
    let mut rest = s.as_str();
    let mut any = false;
    for (unit, scale) in [('h', 3600u64), ('m', 60), ('s', 1)] {
        let end = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
        if end > 0 && rest[end..].starts_with(unit) {
            let n: u64 = rest[..end].parse().unwrap_or(u64::MAX);
            total = total.saturating_add(n.saturating_mul(scale));
            rest = &rest[end + 1..];
            any = true;
        }
    }
    (any && rest.is_empty()).then_some(total)
}

/// "21:00" or "9:05" as "HH:MM".
pub fn parse_clock(raw: &str) -> Option<String> {
    let (h, m) = raw.trim().split_once(':')?;
    if h.is_empty() || h.len() > 2 || m.len() != 2 || !h.chars().chain(m.chars()).all(|c| c.is_ascii_digit()) {
        return None;
    }
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (h <= 23 && m <= 59).then(|| format!("{h:02}:{m:02}"))
}

/// 30 as "30s", 90 as "1m30s", 1200 as "20m".
pub fn format_duration(total_seconds: f64) -> String {
    let total = total_seconds.max(0.0).round() as u64;
    if total < 60 {
        return format!("{total}s");
    }
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    let mut out = String::new();
    if h > 0 {
        out.push_str(&format!("{h}h"));
    }
    if m > 0 {
        out.push_str(&format!("{m}m"));
    }
    if s > 0 {
        out.push_str(&format!("{s}s"));
    }
    if out.is_empty() {
        "0s".to_string()
    } else {
        out
    }
}

fn seconds_of(r: &Value, secs: &str, minutes: &str) -> f64 {
    r[secs].as_f64().or_else(|| r[minutes].as_f64().map(|m| m * 60.0)).unwrap_or(0.0)
}

/// "every 20m", "in 30s", "at 21:00", "at 2h uptime", "on \"word\"".
fn trigger_phrase(r: &Value) -> String {
    match r["trigger"].as_str().unwrap_or("") {
        "interval" => format!("every {}", format_duration(seconds_of(r, "interval_seconds", "interval_minutes"))),
        "delay" => format!("in {}", format_duration(seconds_of(r, "delay_seconds", "delay_minutes"))),
        "clock" => format!("at {}", r["clock_time"].as_str().unwrap_or("undefined")),
        "uptime" => format!("at {} uptime", format_duration(seconds_of(r, "uptime_seconds", "uptime_minutes"))),
        "keyword" => format!("on \"{}\"", r["keyword"].as_str().unwrap_or("undefined")),
        _ => String::new(),
    }
}

fn repeat_of(r: &Value) -> u32 {
    clamp_repeat(r["repeat_count"].as_f64())
}

fn pinned_login(r: &Value) -> Option<&str> {
    (r["channel_scope"].as_str() == Some("specific"))
        .then(|| r["channel_login"].as_str())
        .flatten()
        .filter(|s| !s.is_empty())
}

fn describe_reminder(r: &Value) -> String {
    let repeat = repeat_of(r);
    let times = if repeat > 1 { format!(" ×{repeat}") } else { String::new() };
    let place = pinned_login(r).map(|l| format!(" in #{l}")).unwrap_or_default();
    format!("Reminder set: {}{times}{place}", trigger_phrase(r))
}

fn cut(text: &str, keep: usize, over: usize) -> String {
    if text.chars().count() > over {
        format!("{}…", text.chars().take(keep).collect::<String>())
    } else {
        text.to_string()
    }
}

/// The numbered list `/remind list` prints.
pub fn format_reminder_list(reminders: &[Value]) -> Vec<String> {
    if reminders.is_empty() {
        return vec!["No reminders yet. Example: /remind every 20m Plug the Discord. /remind help lists every option.".to_string()];
    }
    let mut lines = vec![format!("Your reminders ({}):", reminders.len())];
    for (i, r) in reminders.iter().enumerate() {
        let repeat = repeat_of(r);
        let times = if repeat > 1 { format!(" ×{repeat}") } else { String::new() };
        let place = pinned_login(r).map(|l| format!("#{l}")).unwrap_or_else(|| "current channel".to_string());
        let state = if r["enabled"].as_bool().unwrap_or(false) { "on" } else { "off" };
        let msg = cut(r["message"].as_str().unwrap_or(""), 57, 60);
        lines.push(format!("#{} [{state}] {}{times} · {place} · \"{msg}\"", i + 1, trigger_phrase(r)));
    }
    lines.push("Manage: /remind remove <#> · /remind disable <#> · /remind enable <#> · /remind clear".to_string());
    lines
}

/// What `/remind help` prints.
pub fn remind_help_lines() -> Vec<String> {
    [
        "Reminders auto-post a message into chat to nudge the streamer. Make one:",
        "/remind every 20m <message>: repeat every 20 minutes (also 30s, 90s, 1h30m)",
        "/remind in 30s <message>: once, 30 seconds from now",
        "/remind at 21:00 <message>: once a day at a set time (24h)",
        "/remind uptime 2h <message>: once when the stream has been live 2h",
        "/remind on <word> <message>: when that word shows up in chat",
        "/remind now <message>: post it right now (not saved)",
        "Times take s/m/h: 45s, 90s, 5m, 1h30m. Add x3 anywhere to post it 3 times.",
        "Manage: /remind list · /remind remove <#> · /remind enable <#> · /remind disable <#> · /remind clear",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

fn repeat_token(t: &str) -> Option<u32> {
    let lower = t.to_ascii_lowercase();
    let digits = lower
        .strip_prefix("x:")
        .or_else(|| lower.strip_prefix('x'))
        .or_else(|| lower.strip_prefix("repeat:"))?;
    (!digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()))
        .then(|| clamp_repeat(Some(digits.parse::<f64>().unwrap_or(f64::MAX))))
}

/// Parse the words after `/remind`, typed in the room `login` (id `id`).
/// `new_id` names a reminder this creates.
pub fn parse_remind_command(args: &[&str], login: &str, id: &str, new_id: &str) -> RemindParse {
    let tokens: Vec<&str> = args.iter().copied().filter(|t| !t.is_empty()).collect();
    if tokens.is_empty() {
        return RemindParse::List;
    }
    let mut repeat = 1;
    let mut rest: Vec<&str> = Vec::new();
    for t in tokens {
        match repeat_token(t) {
            Some(n) => repeat = n,
            None => rest.push(t),
        }
    }
    if rest.is_empty() {
        return RemindParse::Error(REMIND_USAGE.to_string());
    }
    let base = |trigger: &str| -> serde_json::Map<String, Value> {
        let mut m = serde_json::Map::new();
        m.insert("id".into(), json!(new_id));
        m.insert("enabled".into(), json!(true));
        m.insert("message".into(), json!(""));
        m.insert("repeat_count".into(), json!(repeat));
        // Typed in a streamer's chat, so it posts to that channel.
        m.insert("channel_scope".into(), json!(if login.is_empty() { "current" } else { "specific" }));
        if !login.is_empty() {
            m.insert("channel_login".into(), json!(login));
        }
        if !id.is_empty() {
            m.insert("channel_id".into(), json!(id));
        }
        m.insert("trigger".into(), json!(trigger));
        m
    };
    let finish = |mut m: serde_json::Map<String, Value>, message: String| -> RemindParse {
        let message = message.trim().to_string();
        if message.is_empty() {
            return RemindParse::Error("Add the message to post, e.g. /remind every 20m Plug the Discord".to_string());
        }
        m.insert("message".into(), json!(message));
        let reminder = Value::Object(m);
        let summary = describe_reminder(&reminder);
        RemindParse::Create { reminder, summary }
    };
    let index = |raw: Option<&&str>| raw.and_then(|r| leading_int(r)).filter(|n| *n >= 1).map(|n| n as usize);
    let mode = rest[0].to_lowercase();
    let tail = |from: usize| rest.get(from..).map(|r| r.join(" ")).unwrap_or_default();
    match mode.as_str() {
        "help" | "?" => RemindParse::Help,
        "list" | "ls" | "all" => RemindParse::List,
        "clear" | "clearall" | "removeall" => RemindParse::Clear,
        "remove" | "delete" | "rm" | "del" => match index(rest.get(1)) {
            Some(index) => RemindParse::Manage { op: "remove", index },
            None => RemindParse::Error("Which one? Run /remind list for the numbers, then e.g. /remind remove 2".to_string()),
        },
        "enable" | "disable" | "toggle" => match index(rest.get(1)) {
            Some(index) => RemindParse::Manage {
                op: match mode.as_str() {
                    "enable" => "enable",
                    "disable" => "disable",
                    _ => "toggle",
                },
                index,
            },
            None => RemindParse::Error(format!("Which one? Run /remind list, then e.g. /remind {mode} 2")),
        },
        "now" => {
            let message = tail(1).trim().to_string();
            if message.is_empty() {
                RemindParse::Error("Usage: /remind now <message>".to_string())
            } else {
                RemindParse::SendNow { message, repeat }
            }
        }
        "every" | "in" | "uptime" => {
            let (trigger, field, error) = match mode.as_str() {
                "every" => ("interval", "interval_seconds", "How often? e.g. /remind every 20m or every 30s <message>"),
                "in" => ("delay", "delay_seconds", "How long from now? e.g. /remind in 30s or in 5m <message>"),
                _ => ("uptime", "uptime_seconds", "At what uptime? e.g. /remind uptime 2h or 90s <message>"),
            };
            match parse_duration_seconds(rest.get(1).copied().unwrap_or("")).filter(|s| *s >= 1) {
                Some(secs) => {
                    let mut m = base(trigger);
                    m.insert(field.into(), json!(secs));
                    finish(m, tail(2))
                }
                None => RemindParse::Error(error.to_string()),
            }
        }
        "at" => match parse_clock(rest.get(1).copied().unwrap_or("")) {
            Some(clock) => {
                let mut m = base("clock");
                m.insert("clock_time".into(), json!(clock));
                finish(m, tail(2))
            }
            None => RemindParse::Error("What time? Use 24-hour HH:MM, e.g. /remind at 21:00 <message>".to_string()),
        },
        "on" => match rest.get(1) {
            Some(keyword) => {
                let mut m = base("keyword");
                m.insert("keyword".into(), json!(keyword));
                m.insert("keyword_match".into(), json!("contains"));
                m.insert("keyword_from".into(), json!("anyone"));
                m.insert("keyword_cooldown_minutes".into(), json!(5));
                finish(m, tail(2))
            }
            None => RemindParse::Error("On what word? e.g. /remind on giveaway <message>".to_string()),
        },
        _ => {
            // Shorthands with no verb: "/remind 20m <message>", "/remind 21:00 <message>".
            if let Some(clock) = parse_clock(&mode) {
                let mut m = base("clock");
                m.insert("clock_time".into(), json!(clock));
                return finish(m, tail(1));
            }
            match parse_duration_seconds(&mode).filter(|s| *s >= 1) {
                Some(secs) => {
                    let mut m = base("interval");
                    m.insert("interval_seconds".into(), json!(secs));
                    finish(m, tail(1))
                }
                None => RemindParse::Error(REMIND_USAGE.to_string()),
            }
        }
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

    fn parse(line: &str) -> RemindParse {
        let words: Vec<&str> = line.split(' ').collect();
        parse_remind_command(&words, "chan", "42", "rid")
    }

    fn created(line: &str) -> (Value, String) {
        match parse(line) {
            RemindParse::Create { reminder, summary } => (reminder, summary),
            other => panic!("{line} parsed as {other:?}"),
        }
    }

    #[test]
    fn remind_durations_and_clock_times() {
        assert_eq!(parse_duration_seconds("30s"), Some(30));
        assert_eq!(parse_duration_seconds("90s"), Some(90));
        assert_eq!(parse_duration_seconds("1h30m"), Some(5400));
        assert_eq!(parse_duration_seconds("20"), Some(1200));
        assert_eq!(parse_duration_seconds("1h30"), None);
        assert_eq!(parse_duration_seconds("soon"), None);
        assert_eq!(parse_clock("9:05"), Some("09:05".to_string()));
        assert_eq!(parse_clock("21:00"), Some("21:00".to_string()));
        assert_eq!(parse_clock("24:00"), None);
        assert_eq!(parse_clock("21:7"), None);
        assert_eq!(format_duration(30.0), "30s");
        assert_eq!(format_duration(90.0), "1m30s");
        assert_eq!(format_duration(1200.0), "20m");
        assert_eq!(format_duration(3600.0), "1h");
    }

    #[test]
    fn remind_creates_each_trigger() {
        let (r, summary) = created("every 20m Drink water");
        assert_eq!(r["trigger"], json!("interval"));
        assert_eq!(r["interval_seconds"], json!(1200));
        assert_eq!(r["channel_scope"], json!("specific"));
        assert_eq!(r["channel_login"], json!("chan"));
        assert_eq!(r["channel_id"], json!("42"));
        assert_eq!(summary, "Reminder set: every 20m in #chan");
        let (r, summary) = created("x3 in 30s Check the queue");
        assert_eq!((r["trigger"].clone(), r["delay_seconds"].clone(), r["repeat_count"].clone()), (json!("delay"), json!(30), json!(3)));
        assert_eq!(summary, "Reminder set: in 30s ×3 in #chan");
        let (r, _) = created("at 21:30 Raid time");
        assert_eq!(r["clock_time"], json!("21:30"));
        let (r, summary) = created("uptime 2h Stretch");
        assert_eq!(r["uptime_seconds"], json!(7200));
        assert_eq!(summary, "Reminder set: at 2h uptime in #chan");
        let (r, summary) = created("on !discord The link is in the panel");
        assert_eq!(r["keyword"], json!("!discord"));
        assert_eq!(r["keyword_cooldown_minutes"], json!(5));
        assert_eq!(summary, "Reminder set: on \"!discord\" in #chan");
        // Shorthands with no verb.
        assert_eq!(created("20m hi").0["interval_seconds"], json!(1200));
        assert_eq!(created("21:00 hi").0["clock_time"], json!("21:00"));
        // Repeats are capped.
        assert_eq!(created("every 1m x99 hi").0["repeat_count"], json!(10));
    }

    #[test]
    fn remind_reads_management_and_says_what_is_missing() {
        assert_eq!(parse_remind_command(&[], "chan", "42", "rid"), RemindParse::List);
        assert_eq!(parse("help"), RemindParse::Help);
        assert_eq!(parse("ls"), RemindParse::List);
        assert_eq!(parse("clearall"), RemindParse::Clear);
        assert_eq!(parse("rm 2"), RemindParse::Manage { op: "remove", index: 2 });
        assert_eq!(parse("toggle 1"), RemindParse::Manage { op: "toggle", index: 1 });
        assert_eq!(parse("now hello x2"), RemindParse::SendNow { message: "hello".into(), repeat: 2 });
        assert_eq!(
            parse("remove"),
            RemindParse::Error("Which one? Run /remind list for the numbers, then e.g. /remind remove 2".into())
        );
        assert_eq!(parse("enable x"), RemindParse::Error("Which one? Run /remind list, then e.g. /remind enable 2".into()));
        assert_eq!(parse("every 20m"), RemindParse::Error("Add the message to post, e.g. /remind every 20m Plug the Discord".into()));
        assert_eq!(parse("x3"), RemindParse::Error(REMIND_USAGE.into()));
        assert_eq!(parse("whenever hi"), RemindParse::Error(REMIND_USAGE.into()));
        assert_eq!(parse("now"), RemindParse::Error("Usage: /remind now <message>".into()));
    }

    #[test]
    fn remind_lists_number_the_reminders() {
        assert_eq!(
            format_reminder_list(&[]),
            vec!["No reminders yet. Example: /remind every 20m Plug the Discord. /remind help lists every option."]
        );
        let long = "x".repeat(70);
        let list = format_reminder_list(&[
            json!({ "enabled": true, "trigger": "interval", "interval_minutes": 2, "message": "hi", "channel_scope": "current" }),
            json!({ "enabled": false, "trigger": "keyword", "keyword": "gg", "message": long, "channel_scope": "specific", "channel_login": "chan", "repeat_count": 2 }),
        ]);
        assert_eq!(list[0], "Your reminders (2):");
        assert_eq!(list[1], "#1 [on] every 2m · current channel · \"hi\"");
        assert_eq!(list[2], format!("#2 [off] on \"gg\" ×2 · #chan · \"{}…\"", "x".repeat(57)));
        assert!(list[3].starts_with("Manage: "));
        assert_eq!(remind_help_lines().len(), 9);
    }

    #[test]
    fn repeats_are_made_unique_and_clamped() {
        assert_eq!(with_duplicate_bypass("hi", 0), "hi");
        let tag = char::from_u32(0xE0000).unwrap();
        assert_eq!(with_duplicate_bypass("hi", 2), format!("hi {tag}{tag}"));
        assert_eq!(clamp_repeat(None), 1);
        assert_eq!(clamp_repeat(Some(0.0)), 1);
        assert_eq!(clamp_repeat(Some(3.7)), 3);
        assert_eq!(clamp_repeat(Some(50.0)), 10);
    }
}
