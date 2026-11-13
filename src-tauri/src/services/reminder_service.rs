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
use crate::services::account_automations::activity::EntryKind;
use crate::services::account_automations::reminder_claim::{self, ClaimInput, Decision, ReminderDoor, ReminderSlot, ReminderSpec};
use crate::services::chat_send_limiter::{self, CopyResult, PacedIo};
use regex::Regex;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};
use crate::rt::AppHandle;

pub mod convert;

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
    /// The name the Reminders list shows (the editor's "Reminder name"); the
    /// Activity row and the Automations group use it, else the message.
    #[serde(default)]
    label: Option<String>,
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
/// The chats the keyword gate refused a moment ago, by login: a busy gated-out chat does not
/// spawn a check per message. Only refusals are kept; an "open" answer is never cached, so a
/// keyword reminder can never post in a chat closed under a second ago.
static GATE_REFUSALS: Mutex<Option<HashMap<String, Instant>>> = Mutex::new(None);
/// When each reminder last posted (epoch ms), for the Reminders list.
static LAST_FIRED: Mutex<Option<HashMap<String, i64>>> = Mutex::new(None);

/// How long a refused chat stays refused.
const GATE_REFUSAL_TTL: Duration = Duration::from_secs(1);

fn with_refusals<R>(f: impl FnOnce(&mut HashMap<String, Instant>) -> R) -> R {
    let mut guard = GATE_REFUSALS.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(HashMap::new))
}

/// Whether `refusals` holds a refusal for `login` younger than the TTL at `now`.
fn refusal_fresh(refusals: &HashMap<String, Instant>, login: &str, now: Instant) -> bool {
    refusals.get(login).is_some_and(|at| now.saturating_duration_since(*at) < GATE_REFUSAL_TTL)
}

fn gate_refused_recently(login: &str) -> bool {
    with_refusals(|m| refusal_fresh(m, login, Instant::now()))
}

fn gate_note_refusal(login: &str) {
    with_refusals(|m| {
        let now = Instant::now();
        m.retain(|_, at| now.saturating_duration_since(*at) < GATE_REFUSAL_TTL);
        m.insert(login.to_string(), now);
    });
}

/// A chat was opened (a window claimed it, or the stream warm-up joined it): forget its refusal.
pub fn gate_forget(login: &str) {
    with_refusals(|m| {
        m.remove(&login.to_lowercase());
    });
}

/// Claims were reset (a fresh start or a stop re-holds every chat): forget every refusal.
pub fn gate_forget_all() {
    with_refusals(|m| m.clear());
}

/// When this reminder last posted, if it has since the app started.
pub(crate) fn last_fired(id: &str) -> Option<i64> {
    LAST_FIRED.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(id).copied())
}

fn note_fired(id: &str, at_ms: i64) {
    LAST_FIRED.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(id.to_string(), at_ms);
}

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

/// The compiled keyword list and the timed list for these settings. A reminder whose id is in
/// `settings.automations.converted_reminders` is in neither, whatever its `enabled`: the list is
/// read from the settings here and in `fire`, never from the engine's compiled rules, so a convert
/// followed at once by a tick never fires, and a stale `enabled: true` written back (then the rule
/// deleted) still leaves the reminder quiet.
fn compile_lists(settings: &Settings) -> (Vec<CompiledKeyword>, Vec<Reminder>) {
    let converted = &settings.automations.converted_reminders;
    let reminders: Vec<Reminder> = reminders_in(settings).into_iter().filter(|r| !converted.contains(&r.id)).collect();
    let keywords = reminders.iter().filter_map(CompiledKeyword::compile).collect();
    let timed = reminders.into_iter().filter(Reminder::is_timed).collect();
    (keywords, timed)
}

/// Recompile from settings: the keyword list, the timed list, and whether the
/// tick should run at all.
pub fn refresh(settings: &Settings) {
    let (keywords, timed) = compile_lists(settings);
    if let Ok(mut slot) = KEYWORDS.write() {
        *slot = (!keywords.is_empty()).then(|| Arc::new(keywords));
    }
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

/// A reminder found due, and the slot it fires for.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Fire {
    pub id: String,
    pub login: String,
    pub is_current: bool,
    pub slot: ReminderSlot,
    /// A keyword fire: the cooldown stamp it set and the one before it, so a gated-out fire can
    /// put it back (compare-and-restore).
    pub stamp: Option<(i64, i64)>,
}

/// How one fire ended (what tests read).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum FireEnd {
    /// Nothing to do: nobody is signed in, or the text came out empty.
    Quiet,
    /// The keyword gate refused: the chat is not open in a window.
    Gated,
    Skipped(String),
    /// It posted. `anyway`: the check could not be answered. `released`: the claim was given back
    /// because nothing can have posted.
    Posted { anyway: bool, released: bool },
}

/// What one fire needs from outside, so a test runs it with no network, no IRC and no real send.
#[async_trait::async_trait]
pub(crate) trait FireEnv: Send + Sync {
    /// (login, user id) of the account `post_text` posts as.
    fn identity(&self) -> Option<(String, String)>;
    /// The keyword gate: a chat a reminder may post in. Caches its refusals for 1 s.
    async fn gate_open(&self, login: &str) -> bool;
    /// Put a keyword fire's cooldown stamp back, only when it is still the one that fire set.
    fn restore_stamp(&self, id: &str, stamp: Option<(i64, i64)>);
    /// The chat's room id, for the claim's class; `pinned_id` is the id a pinned reminder carries.
    async fn room_id(&self, channel: &str, pinned_id: Option<&str>) -> Option<String>;
    /// The door to streamnook.app; None only before `Engine::init` sets the singleton.
    fn door(&self) -> Option<Arc<dyn ReminderDoor>>;
    /// `post_text`.
    async fn post(&self, channel: &str, channel_id: Option<&str>, template: &str, copies: u32) -> (bool, Vec<CopyResult>);
    fn note_posted(&self, id: &str, name: &str, login: &str, text: &str);
    fn note_skipped(&self, id: &str, name: &str, login: &str, words: &str);
}

/// The real environment. Reminder posting never goes through `Outward`: `post_text` calls the
/// paced send directly, as it always has, and `/remind now` needs no engine.
struct RealEnv;

#[async_trait::async_trait]
impl FireEnv for RealEnv {
    fn identity(&self) -> Option<(String, String)> {
        crate::services::chat_rules::ChatRules::own_identity()
    }

    async fn gate_open(&self, login: &str) -> bool {
        let open = crate::services::irc_service::IrcService::open_for_reminders(login).await;
        if !open {
            gate_note_refusal(login);
        }
        open
    }

    fn restore_stamp(&self, id: &str, stamp: Option<(i64, i64)>) {
        with_state(|s| {
            if let Some(rt) = s.runtimes.get_mut(id) {
                restore_stamp_in(rt, stamp);
            }
        });
    }

    async fn room_id(&self, channel: &str, pinned_id: Option<&str>) -> Option<String> {
        if let Some(id) = pinned_id {
            return Some(id.to_string());
        }
        crate::services::slash_commands::user_by_login(channel)
            .await
            .ok()
            .flatten()
            .and_then(|u| u["id"].as_str().map(str::to_string))
    }

    fn door(&self) -> Option<Arc<dyn ReminderDoor>> {
        reminder_claim::engine_door()
    }

    async fn post(&self, channel: &str, channel_id: Option<&str>, template: &str, copies: u32) -> (bool, Vec<CopyResult>) {
        post_text(channel, channel_id, template, copies).await
    }

    fn note_posted(&self, id: &str, name: &str, login: &str, text: &str) {
        note_fired(id, now_ms());
        reminder_claim::log_reminder(EntryKind::Acted, id, name, login, text);
    }

    fn note_skipped(&self, id: &str, name: &str, login: &str, words: &str) {
        reminder_claim::log_reminder(EntryKind::Skipped, id, name, login, words);
    }
}

/// The reminder as it may fire now: present, on, and not turned into a rule.
fn find_active(settings: &Settings, id: &str) -> Option<Reminder> {
    if settings.automations.converted_reminders.iter().any(|c| c == id) {
        return None;
    }
    reminders_in(settings).into_iter().find(|r| r.id == id).filter(|r| r.enabled)
}

/// The name Activity and the Automations group show for a reminder.
fn reminder_name(r: &Reminder) -> String {
    let label = r.label.as_deref().unwrap_or_default().trim();
    cut(if label.is_empty() { r.message.trim() } else { label }, 39, 40)
}

/// What a claim for this reminder is built from: its family (message and target) and, for a
/// keyword reminder, its cooldown in seconds (the rail's window).
fn spec_of(r: &Reminder) -> ReminderSpec {
    let target = r.pinned_channel().unwrap_or_else(|| "current".to_string());
    let cooldown_s = if r.trigger == "keyword" {
        (r.keyword_cooldown_minutes.unwrap_or(DEFAULT_KEYWORD_COOLDOWN_MINUTES).max(0.0).floor() as u32).saturating_mul(60)
    } else {
        0
    };
    ReminderSpec { hash: reminder_claim::family_hash(r.message.trim(), &target), cooldown_s }
}

/// Put a keyword stamp back, but only when it is still the one that fire set (a later fire's
/// stamp is never overwritten).
fn restore_stamp_in(rt: &mut Runtime, stamp: Option<(i64, i64)>) {
    if let Some((set, before)) = stamp {
        if rt.keyword_last_ms == set {
            rt.keyword_last_ms = before;
        }
    }
}

/// The id a keyword message claims under (`chat:<id>`): the Shared Chat `source-id` when the row
/// has one, else its own `id` tag, else the row's id; the same choice `facts::from_chat` makes.
pub(crate) fn message_key_id(source_id: Option<&str>, own_tag_id: Option<&str>, msg_id: &str) -> String {
    let pick = |v: Option<&str>| v.map(str::trim).filter(|s| !s.is_empty());
    pick(source_id).or(pick(own_tag_id)).unwrap_or_else(|| msg_id.trim()).to_string()
}

/// What a local time is in a time zone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Zone {
    /// One UTC offset (seconds).
    Single(i32),
    /// A repeated hour: the first, then the second offset.
    Ambiguous(i32, i32),
    /// A time that does not exist (the hour skipped when the clocks go forward).
    Gap,
}

fn zone_of(naive: chrono::NaiveDateTime) -> Zone {
    use chrono::{Local, LocalResult, TimeZone};
    match Local.from_local_datetime(&naive) {
        LocalResult::Single(t) => Zone::Single(t.offset().local_minus_utc()),
        LocalResult::Ambiguous(a, b) => Zone::Ambiguous(a.offset().local_minus_utc(), b.offset().local_minus_utc()),
        LocalResult::None => Zone::Gap,
    }
}

/// A clock reminder's slot: the scheduled local date and time, and the UTC offset in force at that
/// time (D281, D289), so two devices across a DST change build one key. In a repeated hour the
/// offset is the one this device is in now when it is either of them (two devices in different
/// halves give two keys), else the first. For a local time that does not exist it is the offset in
/// force before the gap.
pub(crate) fn clock_slot(
    date: chrono::NaiveDate,
    h: u32,
    m: u32,
    now_offset_s: i32,
    zone: &dyn Fn(chrono::NaiveDateTime) -> Zone,
) -> ReminderSlot {
    let time = format!("{h:02}:{m:02}");
    let day = date.format("%Y-%m-%d").to_string();
    let Some(naive) = date.and_hms_opt(h, m, 0) else {
        return ReminderSlot::Clock { date: day, time, offset_s: now_offset_s };
    };
    let offset_s = match zone(naive) {
        Zone::Single(o) => o,
        Zone::Ambiguous(first, second) => {
            if now_offset_s == second {
                second
            } else {
                first
            }
        }
        // Three hours earlier is before any gap (gaps are an hour or less).
        Zone::Gap => match zone(naive - chrono::Duration::hours(3)) {
            Zone::Single(o) | Zone::Ambiguous(o, _) => o,
            Zone::Gap => now_offset_s,
        },
    };
    ReminderSlot::Clock { date: day, time, offset_s }
}

fn clock_slot_now(local: &chrono::DateTime<chrono::Local>, h: u32, m: u32) -> ReminderSlot {
    clock_slot(local.date_naive(), h, m, local.offset().local_minus_utc(), &zone_of)
}

/// The slot of a Clock fire on `date` at `h:m` (what a converted Clock rule's job claims as), with
/// this device's offset right now as the tie-break in a repeated hour. Called by
/// `account_automations::executor::reminder_slot_of`.
pub(crate) fn clock_slot_at(date: chrono::NaiveDate, h: u32, m: u32) -> ReminderSlot {
    clock_slot(date, h, m, chrono::Local::now().offset().local_minus_utc(), &zone_of)
}

/// Post the reminder found due into the room it resolved to. A reminder deleted, turned off or made
/// into a rule since is skipped.
fn fire(f: Fire) {
    let Some(app) = APP.get().cloned() else { return };
    tauri::async_runtime::spawn(async move {
        let reminder = {
            use tauri::Manager;
            let state = app.state::<crate::models::settings::AppState>();
            let settings = match state.settings.lock() {
                Ok(s) => s.clone(),
                Err(_) => return,
            };
            find_active(&settings, &f.id)
        };
        let Some(r) = reminder else { return };
        run_fire(&RealEnv, &r, &f).await;
    });
}

/// One fire, start to end: the keyword gate (inside this spawned task, never on the IRC read
/// loop), the room id, the claim (one 15 s deadline for the token check and the claim), the post,
/// and the release when nothing can have posted. The claim is final on a win; a reminder that
/// posted anyway never releases.
pub(crate) async fn run_fire(env: &dyn FireEnv, r: &Reminder, f: &Fire) -> FireEnd {
    let Some((login, own_id)) = env.identity() else { return FireEnd::Quiet };
    if matches!(f.slot, ReminderSlot::Keyword { .. }) && !env.gate_open(&f.login).await {
        // A gated-out fire never uses up the cooldown.
        env.restore_stamp(&f.id, f.stamp);
        return FireEnd::Gated;
    }
    let name = reminder_name(r);
    // A pinned reminder carries the room id it was made with; the watched channel's is looked up
    // from its login (a `current` reminder has no id, `channel_id: None` in `post_text`).
    let pinned_id = r.pinned_channel().and(r.channel_id.clone()).filter(|s| !s.is_empty());
    let post_id = if f.is_current { None } else { r.channel_id.clone().filter(|s| !s.is_empty()) };
    let room_id = env.room_id(&f.login, pinned_id.as_deref()).await;
    let (door, decision) = match env.door() {
        Some(door) => {
            let input = ClaimInput {
                owner_id: own_id.clone(),
                owner_login: login,
                spec: spec_of(r),
                slot: f.slot.clone(),
                chat_key: crate::services::chat_rules::channel_filter_key("twitch", &f.login),
                room_id,
            };
            let deadline = tokio::time::Instant::now() + reminder_claim::CLAIM_DEADLINE;
            let decision = reminder_claim::claim_step(door.as_ref(), &input, deadline).await;
            (Some(door), decision)
        }
        // Before the engine starts: the check cannot be answered.
        None => (None, Decision::PostAnyway),
    };
    let (call, anyway) = match decision {
        Decision::Skip { words } => {
            env.note_skipped(&f.id, &name, &f.login, &words);
            return FireEnd::Skipped(words);
        }
        Decision::Post { call } => (call, false),
        Decision::PostAnyway => (None, true),
    };
    let (posted_any, copies) = env.post(&f.login, post_id.as_deref(), &r.message, clamp_repeat(r.repeat_count)).await;
    // Release only when every attempted copy failed before it could leave, or Twitch answered
    // `is_sent: false` with a reason: nothing may have posted. A `PacedFail::After`, any sent copy
    // and a refusal with no reason keep the claim. A win the cache answered has no call to give back.
    let released = match (&call, &door) {
        (Some(call), Some(door)) if chat_send_limiter::release_after(&copies) => {
            door.release(&own_id, call);
            true
        }
        _ => false,
    };
    if !posted_any {
        return FireEnd::Quiet;
    }
    env.note_posted(&f.id, &name, &f.login, if anyway { reminder_claim::POSTED_ANYWAY } else { "Posted" });
    FireEnd::Posted { anyway, released }
}

/// Post a reminder's text into `channel` (bare login) `copies` times: the
/// template expanded for that room, each copy after the first made unique with
/// the invisible duplicate-bypass suffix, each waiting for a send slot. A
/// refusal stops the run and prints why in that room. Answers whether anything
/// was attempted (false when no one is signed in or the text came out empty)
/// and how each attempted copy went (copies never attempted do not count), so
/// `fire` can tell whether anything may have posted (`release_after`). This
/// makes no claim: `/remind now` (`post_reminder`) is a direct user send and
/// claims nothing.
pub async fn post_text(channel: &str, channel_id: Option<&str>, template: &str, copies: u32) -> (bool, Vec<CopyResult>) {
    post_text_with(&chat_send_limiter::RealPacedIo, channel, channel_id, template, copies).await
}

/// `post_text` over a paced-send seam: every copy goes through the same booking, Helix send with
/// its IRC fallback and counting `chat_send_paced` always had (it wraps this same `paced_send`).
async fn post_text_with<I: PacedIo>(
    io: &I,
    channel: &str,
    channel_id: Option<&str>,
    template: &str,
    copies: u32,
) -> (bool, Vec<CopyResult>) {
    if crate::services::chat_rules::ChatRules::own_identity().is_none() {
        return (false, Vec::new());
    }
    let text = crate::services::slash_dispatch::expand_for_room(template, channel, channel_id.unwrap_or("")).await;
    let text = text.trim().to_string();
    if text.is_empty() {
        return (false, Vec::new());
    }
    let mut results: Vec<CopyResult> = Vec::new();
    for j in 0..copies {
        let copy = with_duplicate_bypass(&text, j);
        let result = chat_send_limiter::paced_send(io, channel, channel_id.map(str::to_string), &copy).await;
        let refusal = result.as_ref().err().map(chat_send_limiter::fail_text);
        results.push(result);
        if let Some(refusal) = refusal {
            crate::services::slash_dispatch::post_room_notice(channel, &format!("Reminder stopped: {refusal}")).await;
            break;
        }
    }
    (true, results)
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
                // A chat a window holds, or the stream warm-up joined: never merely "joined", which the
                // engine's own claims and close-to-tray adoption would keep true for chats you never opened.
                joined.insert(login.clone(), crate::services::irc_service::IrcService::open_for_reminders(login).await);
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
    for f in fires {
        fire(f);
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
) -> Option<Fire> {
    use chrono::{Datelike, Timelike};
    // (Re)arm when the target changes. Resets the relative timers; the per-day
    // and per-stream guards intentionally persist.
    if rt.armed_channel.as_deref() != Some(login) {
        rt.armed_channel = Some(login.to_string());
        rt.armed_at_ms = now;
        rt.last_interval_ms = now;
        rt.delay_fired = false;
    }
    let fire = |slot: ReminderSlot| Some(Fire { id: r.id.clone(), login: login.to_string(), is_current, slot, stamp: None });
    match r.trigger.as_str() {
        "interval" => {
            let secs = r.interval();
            if secs >= 1 && now - rt.last_interval_ms >= secs as i64 * 1000 {
                rt.last_interval_ms = now;
                return fire(ReminderSlot::Interval { secs });
            }
        }
        "delay" => {
            let secs = r.delay();
            if secs >= 1 && !rt.delay_fired && now - rt.armed_at_ms >= secs as i64 * 1000 {
                rt.delay_fired = true;
                return fire(ReminderSlot::Delay { secs });
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
                // The scheduled minute (not the minute a late tick ran) and the offset in force then.
                return fire(clock_slot_now(local, h, m));
            }
        }
        "uptime" => {
            // Needs the watched stream's start time.
            let started = started_at.filter(|s| is_current && !s.is_empty())?;
            let start_ms = chrono::DateTime::parse_from_rfc3339(started).ok()?.timestamp_millis();
            let secs = r.uptime();
            if secs >= 1 && rt.uptime_fired_for.as_deref() != Some(started) && (now - start_ms) / 1000 >= secs as i64 {
                rt.uptime_fired_for = Some(started.to_string());
                return fire(ReminderSlot::Uptime { started_s: start_ms.div_euclid(1000), secs });
            }
        }
        _ => {}
    }
    None
}

/// A chat message arrived: fire any keyword reminder it satisfies. Cheap when no
/// keyword reminder exists (one lock read). This runs on the IRC read loop and never awaits:
/// the open-chat gate runs inside `fire`'s spawned task; here only a refusal cached for the
/// last second is read (a plain map lookup).
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
    let msg_id = message_key_id(msg.tags.get("source-id").map(String::as_str), msg.tags.get("id").map(String::as_str), &msg.id);
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
            // A chat the gate refused a moment ago: nothing to check, and nothing stamped.
            if gate_refused_recently(&target) {
                continue;
            }
            let rt = s.runtimes.entry(kw.id.clone()).or_default();
            if rt.keyword_last_ms > 0 && now - rt.keyword_last_ms < kw.cooldown_ms {
                continue;
            }
            let before = rt.keyword_last_ms;
            rt.keyword_last_ms = now;
            fires.push(Fire {
                id: kw.id.clone(),
                login: target,
                is_current,
                slot: ReminderSlot::Keyword { msg_id: msg_id.clone() },
                stamp: Some((now, before)),
            });
        }
    });
    for f in fires {
        fire(f);
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

    // ---- P5: slots, the gate, and the claim in `fire` ----------------------------------------

    use crate::services::account_automations::activity::{EntryKind, Filter, Log, NewEntry};
    use crate::services::account_automations::reminder_claim::EngineDoor;
    use crate::services::account_automations::testkit::{self, FakeHttp, FakeToken, Kit};
    use crate::models::automations::{AccountRef, Rule, ValidateCtx};
    use crate::services::account_automations::claim_client::{CallKind, ClaimRequest, ClaimResult};
    use crate::services::chat_send_limiter::{PacedFail, PacedSend};

    fn sent() -> CopyResult {
        Ok(PacedSend { is_sent: true, drop_reason: None })
    }
    fn twitch_refused() -> CopyResult {
        Ok(PacedSend { is_sent: false, drop_reason: Some("Twitch refused".into()) })
    }
    fn unsaid_refusal() -> CopyResult {
        Ok(PacedSend { is_sent: false, drop_reason: None })
    }
    fn before() -> CopyResult {
        Err(PacedFail::Before("Sign in to Twitch to send messages".into()))
    }
    fn after() -> CopyResult {
        Err(PacedFail::After("The request timed out".into()))
    }

    /// A fake environment: nothing real is reached. `door` is None before the engine starts.
    struct FakeEnv {
        identity: Option<(String, String)>,
        open: bool,
        room: Option<String>,
        door: Option<Arc<dyn ReminderDoor>>,
        post_result: (bool, Vec<CopyResult>),
        posts: Mutex<Vec<(String, Option<String>, u32)>>,
        restored: Mutex<Vec<Option<(i64, i64)>>>,
        logs: Mutex<Vec<(String, String)>>,
    }

    impl FakeEnv {
        fn alone(copies: Vec<CopyResult>) -> FakeEnv {
            FakeEnv {
                identity: Some(("me".into(), "100".into())),
                open: true,
                room: Some("4242".into()),
                door: None,
                post_result: (true, copies),
                posts: Mutex::new(Vec::new()),
                restored: Mutex::new(Vec::new()),
                logs: Mutex::new(Vec::new()),
            }
        }
        /// An environment on one device's engine (its claim client, token door and fake server).
        fn on(kit: &Kit, copies: Vec<CopyResult>) -> FakeEnv {
            let mut env = FakeEnv::alone(copies);
            env.identity = Some(("me".into(), testkit::main_id()));
            env.door = Some(Arc::new(EngineDoor::over(kit.engine.clone())));
            env
        }
        fn posted_times(&self) -> usize {
            self.posts.lock().unwrap().len()
        }
    }

    #[async_trait::async_trait]
    impl FireEnv for FakeEnv {
        fn identity(&self) -> Option<(String, String)> {
            self.identity.clone()
        }
        async fn gate_open(&self, _login: &str) -> bool {
            self.open
        }
        fn restore_stamp(&self, _id: &str, stamp: Option<(i64, i64)>) {
            self.restored.lock().unwrap().push(stamp);
        }
        async fn room_id(&self, _channel: &str, _pinned_id: Option<&str>) -> Option<String> {
            self.room.clone()
        }
        fn door(&self) -> Option<Arc<dyn ReminderDoor>> {
            self.door.clone()
        }
        async fn post(&self, channel: &str, channel_id: Option<&str>, _template: &str, copies: u32) -> (bool, Vec<CopyResult>) {
            self.posts.lock().unwrap().push((channel.to_string(), channel_id.map(str::to_string), copies));
            self.post_result.clone()
        }
        fn note_posted(&self, _id: &str, _name: &str, _login: &str, text: &str) {
            self.logs.lock().unwrap().push(("posted".into(), text.to_string()));
        }
        fn note_skipped(&self, _id: &str, _name: &str, _login: &str, words: &str) {
            self.logs.lock().unwrap().push(("skipped".into(), words.to_string()));
        }
    }

    fn pinned(trigger: &str, extra: serde_json::Value) -> Reminder {
        let mut v = json!({ "id": "r", "enabled": true, "trigger": trigger, "message": "Plug the Discord",
            "channel_scope": "specific", "channel_login": testkit::chan(), "channel_id": "4242" });
        if let (Some(map), Some(more)) = (v.as_object_mut(), extra.as_object()) {
            map.extend(more.clone());
        }
        reminder(v)
    }

    fn fire_of(slot: ReminderSlot) -> Fire {
        Fire { id: "r".into(), login: testkit::chan(), is_current: false, slot, stamp: None }
    }

    #[test]
    fn evaluate_builds_each_slot() {
        let start = "2026-09-24T10:00:00Z";
        let start_ms = chrono::DateTime::parse_from_rfc3339(start).unwrap().timestamp_millis();
        let t = at(20, 31);
        // `arm`: interval and delay arm on a first look at time 0 and fire later; clock and uptime
        // fire on the one look.
        let slot_of = |def: serde_json::Value, arm: bool, now: i64, started: Option<&str>| {
            let r = reminder(def);
            let mut rt = Runtime::default();
            if arm {
                assert!(evaluate(&r, &mut rt, "chan", true, started, 0, &t).is_none());
            }
            evaluate(&r, &mut rt, "chan", true, started, now, &t).map(|f| f.slot)
        };
        assert_eq!(
            slot_of(json!({"id":"i","enabled":true,"trigger":"interval","interval_seconds":60}), true, 60_000, None),
            Some(ReminderSlot::Interval { secs: 60 })
        );
        assert_eq!(
            slot_of(json!({"id":"d","enabled":true,"trigger":"delay","delay_seconds":10}), true, 10_000, None),
            Some(ReminderSlot::Delay { secs: 10 })
        );
        match slot_of(json!({"id":"c","enabled":true,"trigger":"clock","clock_time":"20:30"}), false, 0, None) {
            Some(ReminderSlot::Clock { date, time, .. }) => assert_eq!((date.as_str(), time.as_str()), ("2026-09-24", "20:30")),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            slot_of(json!({"id":"u","enabled":true,"trigger":"uptime","uptime_seconds":3600}), false, start_ms + 3_600_000, Some(start)),
            Some(ReminderSlot::Uptime { started_s: start_ms / 1000, secs: 3600 })
        );
    }

    /// A zone for a test: US Eastern in 2026. The clocks go forward at 02:00 on March 8 (02:00 to
    /// 03:00 does not exist, offset -5 h before and -4 h after) and back at 02:00 on November 1
    /// (01:00 to 02:00 happens twice, first at -4 h, then at -5 h).
    fn eastern(n: chrono::NaiveDateTime) -> Zone {
        let on = |m, d, h| chrono::NaiveDate::from_ymd_opt(2026, m, d).unwrap().and_hms_opt(h, 0, 0).unwrap();
        if n >= on(3, 8, 2) && n < on(3, 8, 3) {
            Zone::Gap
        } else if n >= on(11, 1, 1) && n < on(11, 1, 2) {
            Zone::Ambiguous(-14_400, -18_000)
        } else if n >= on(3, 8, 3) && n < on(11, 1, 1) {
            Zone::Single(-14_400)
        } else {
            Zone::Single(-18_000)
        }
    }

    #[test]
    fn a_clock_slot_carries_the_offset_in_force_at_the_scheduled_time() {
        let date = |m, d| chrono::NaiveDate::from_ymd_opt(2026, m, d).unwrap();
        let offset = |slot: ReminderSlot| match slot {
            ReminderSlot::Clock { offset_s, .. } => offset_s,
            other => panic!("{other:?}"),
        };
        // An ordinary day: the offset in force then, whatever the device's offset is "now".
        assert_eq!(offset(clock_slot(date(7, 1), 21, 0, -18_000, &eastern)), -14_400);
        // Across the spring change, two devices (one still on the old offset, one on the new) agree on a
        // time that does not exist: the offset in force before the gap.
        assert_eq!(offset(clock_slot(date(3, 8), 2, 30, -18_000, &eastern)), -18_000);
        assert_eq!(offset(clock_slot(date(3, 8), 2, 30, -14_400, &eastern)), -18_000);
        // A repeated hour gives two keys: each device is in one half.
        let first = clock_slot(date(11, 1), 1, 30, -14_400, &eastern);
        let second = clock_slot(date(11, 1), 1, 30, -18_000, &eastern);
        assert_eq!((offset(first.clone()), offset(second.clone())), (-14_400, -18_000));
        assert_ne!(first, second);
        // The key spells the offset.
        let key = |slot: &ReminderSlot| match reminder_claim::reminder_elements(
            &ReminderSpec { hash: "0".repeat(16), cooldown_s: 0 },
            slot,
            "twitch:x",
            "1",
        )
        .unwrap()
        .remove(0)
        {
            crate::services::account_automations::claims::Element::Event { key, .. } => key,
            other => panic!("{other:?}"),
        };
        assert_eq!(key(&first), "reminder:twitch:x:2026-11-01T01:30-04:00");
        assert_eq!(key(&second), "reminder:twitch:x:2026-11-01T01:30-05:00");
    }

    #[test]
    fn a_keyword_slot_names_the_message_the_engine_claims() {
        assert_eq!(message_key_id(Some("src"), Some("own"), "row"), "src", "a Shared Chat copy claims its source id");
        assert_eq!(message_key_id(Some("  "), Some("own"), "row"), "own");
        assert_eq!(message_key_id(None, None, " row "), "row");
        assert_eq!(
            crate::services::account_automations::claims::chat_key(&message_key_id(None, Some("m0"), "x")),
            "chat:m0",
            "the event element is `chat:<uuid>`, as a rule's Message trigger claims"
        );
    }

    #[test]
    fn a_refused_chat_is_remembered_for_a_second_and_an_open_one_never() {
        let mut refusals = HashMap::new();
        let t0 = Instant::now();
        refusals.insert("chan".to_string(), t0);
        assert!(refusal_fresh(&refusals, "chan", t0 + Duration::from_millis(900)));
        assert!(!refusal_fresh(&refusals, "chan", t0 + Duration::from_millis(1000)));
        assert!(!refusal_fresh(&refusals, "other", t0));
        // The real cache: a refusal is cached, forgetting one chat or all of them clears it,
        // and nothing but a refusal is ever written (there is no function that records "open").
        gate_note_refusal("p5-gate-chat");
        assert!(gate_refused_recently("p5-gate-chat"));
        gate_forget("P5-Gate-Chat");
        assert!(!gate_refused_recently("p5-gate-chat"));
        gate_note_refusal("p5-gate-a");
        gate_note_refusal("p5-gate-b");
        gate_forget_all();
        assert!(!gate_refused_recently("p5-gate-a") && !gate_refused_recently("p5-gate-b"));
    }

    #[test]
    fn a_gated_out_fire_puts_its_stamp_back_only_when_it_is_still_the_one_it_set() {
        let mut rt = Runtime { keyword_last_ms: 5_000, ..Runtime::default() };
        restore_stamp_in(&mut rt, Some((5_000, 100)));
        assert_eq!(rt.keyword_last_ms, 100, "the next matching message fires");
        // A later fire stamped meanwhile: it is never overwritten.
        let mut rt = Runtime { keyword_last_ms: 9_000, ..Runtime::default() };
        restore_stamp_in(&mut rt, Some((5_000, 100)));
        assert_eq!(rt.keyword_last_ms, 9_000);
        restore_stamp_in(&mut rt, None);
        assert_eq!(rt.keyword_last_ms, 9_000);
    }

    #[tokio::test]
    async fn the_keyword_gate_runs_in_the_fire_and_a_timed_fire_never_asks_it() {
        let mut env = FakeEnv::alone(vec![sent()]);
        env.open = false;
        let kw = pinned("keyword", json!({"keyword":"hello"}));
        let mut f = fire_of(ReminderSlot::Keyword { msg_id: "m0".into() });
        f.stamp = Some((5_000, 100));
        assert_eq!(run_fire(&env, &kw, &f).await, FireEnd::Gated);
        assert_eq!(env.restored.lock().unwrap().as_slice(), &[Some((5_000, 100))]);
        assert_eq!(env.posted_times(), 0);
        // `tick` already gated a timed reminder; `run_fire` does not ask again (no engine here: it posts anyway).
        let interval = pinned("interval", json!({"interval_seconds": 600}));
        let end = run_fire(&env, &interval, &fire_of(ReminderSlot::Interval { secs: 600 })).await;
        assert!(matches!(end, FireEnd::Posted { anyway: true, .. }), "{end:?}");
    }

    #[test]
    fn a_converted_reminder_is_in_neither_list_and_never_fires() {
        let mut settings = Settings::default();
        settings.extra.insert(
            "reminders".into(),
            json!({ "reminders": [
                {"id":"a","enabled":true,"trigger":"interval","interval_seconds":600,"message":"hi"},
                {"id":"k","enabled":true,"trigger":"keyword","keyword":"x","message":"hi"},
                {"id":"b","enabled":true,"trigger":"interval","interval_seconds":600,"message":"hi"}
            ]}),
        );
        let (keywords, timed) = compile_lists(&settings);
        assert_eq!((keywords.len(), timed.iter().map(|r| r.id.as_str()).collect::<Vec<_>>()), (1, vec!["a", "b"]));
        settings.automations.converted_reminders = vec!["a".into(), "k".into()];
        let (keywords, timed) = compile_lists(&settings);
        assert_eq!((keywords.len(), timed.iter().map(|r| r.id.as_str()).collect::<Vec<_>>()), (0, vec!["b"]));
        assert!(find_active(&settings, "a").is_none(), "listed: off, whatever its enabled says");
        assert!(find_active(&settings, "b").is_some());
        assert!(find_active(&settings, "gone").is_none());
    }

    #[tokio::test]
    async fn the_same_reminder_on_two_devices_posts_once_for_every_slot() {
        let slots = vec![
            ("interval", pinned("interval", json!({"interval_seconds": 600})), ReminderSlot::Interval { secs: 600 }),
            ("delay", pinned("delay", json!({"delay_seconds": 60})), ReminderSlot::Delay { secs: 60 }),
            (
                "clock",
                pinned("clock", json!({"clock_time": "21:00"})),
                ReminderSlot::Clock { date: "2026-11-13".into(), time: "21:00".into(), offset_s: -18_000 },
            ),
            ("uptime", pinned("uptime", json!({"uptime_seconds": 3600})), ReminderSlot::Uptime { started_s: 1_790_000_000, secs: 3600 }),
            ("keyword", pinned("keyword", json!({"keyword": "hello"})), ReminderSlot::Keyword { msg_id: "m0".into() }),
        ];
        for (name, r, slot) in slots {
            let a = Kit::new();
            let b = a.second_device();
            let (env_a, env_b) = (FakeEnv::on(&a, vec![sent()]), FakeEnv::on(&b, vec![sent()]));
            let first = run_fire(&env_a, &r, &fire_of(slot.clone())).await;
            let second = run_fire(&env_b, &r, &fire_of(slot)).await;
            assert!(matches!(first, FireEnd::Posted { anyway: false, released: false }), "{name}: {first:?}");
            assert!(matches!(&second, FireEnd::Skipped(w) if w.starts_with("Another device posted this reminder")), "{name}: {second:?}");
            assert_eq!((env_a.posted_times(), env_b.posted_times()), (1, 0), "{name}");
            // Reminder calls spend the reminder budget (rule traffic can never 429 them).
            assert!(a.outward.bodies().iter().any(|b| b["reminder"] == json!(true)), "{name}");
        }
        // A reminder only one device has always posts.
        let solo = Kit::new();
        let env = FakeEnv::on(&solo, vec![sent()]);
        let r = pinned("interval", json!({"interval_seconds": 600}));
        assert!(matches!(run_fire(&env, &r, &fire_of(ReminderSlot::Interval { secs: 600 })).await, FireEnd::Posted { .. }));
    }

    #[tokio::test]
    async fn keyword_reminders_keep_their_own_cooldown_rail() {
        let hello = pinned("keyword", json!({"keyword":"hello","keyword_cooldown_minutes": 3}));
        // The m0 and m1 case: A posts for m0, B sees m1 later inside the cooldown. B wins `chat:m1`,
        // loses the rail, and nothing of B's call is written.
        let a = Kit::new();
        let b = a.second_device();
        let (env_a, env_b) = (FakeEnv::on(&a, vec![sent()]), FakeEnv::on(&b, vec![sent()]));
        assert!(matches!(run_fire(&env_a, &hello, &fire_of(ReminderSlot::Keyword { msg_id: "m0".into() })).await, FireEnd::Posted { .. }));
        let lost = run_fire(&env_b, &hello, &fire_of(ReminderSlot::Keyword { msg_id: "m1".into() })).await;
        assert!(matches!(&lost, FireEnd::Skipped(w) if w.starts_with("Cooldown from")), "{lost:?}");
        assert_eq!(env_b.posted_times(), 0);
        // Two different keyword reminders in one chat, one message matching both: both post, as today.
        let rules = pinned("keyword", json!({"id":"r2","keyword":"hello","message":"Read the rules","keyword_cooldown_minutes": 3}));
        let c = Kit::new();
        let (env_1, env_2) = (FakeEnv::on(&c, vec![sent()]), FakeEnv::on(&c, vec![sent()]));
        let slot = ReminderSlot::Keyword { msg_id: "m0".into() };
        assert!(matches!(run_fire(&env_1, &hello, &fire_of(slot.clone())).await, FireEnd::Posted { .. }));
        let mut f2 = fire_of(slot);
        f2.id = "r2".into();
        assert!(matches!(run_fire(&env_2, &rules, &f2).await, FireEnd::Posted { .. }), "each carries its own rail");
    }

    #[tokio::test]
    async fn a_converted_keyword_rule_and_the_same_reminder_post_once() {
        // The reminder, made into a rule on device A by the real convert (a keyword rule needs only P1's triggers).
        let def = json!({ "id": "r1", "enabled": true, "trigger": "keyword", "keyword": "hello", "message": "Plug the Discord",
                          "channel_scope": "current", "keyword_cooldown_minutes": 3 });
        let mut settings = Settings::default();
        settings.extra.insert("reminders".into(), json!({ "reminders": [def.clone()] }));
        let env = convert::ConvertEnv {
            owner: AccountRef { provider: "twitch".into(), id: testkit::main_id(), login: "me".into(), name: "me".into() },
            now_ms: 1_000,
            room_id: None,
            other_channels_on: false,
        };
        let no_context = |_: &crate::models::automations::Automations, _: &str| ValidateCtx::default();
        convert::convert_in(&mut settings, "r1", &env, &no_context).expect("converts");
        let rule = Rule::from_value(&settings.automations.rules[0]).expect("the stored rule reads");
        let chat_key = format!("twitch:{}", testkit::chan());
        let converted = |msg: &str| {
            reminder_claim::converted_elements(&rule, "4242", &chat_key, &ReminderSlot::Keyword { msg_id: msg.into() })
                .expect("claim_as applies")
                .expect("builds")
        };
        // Device A's rule wins m0 through the engine's claim client, as a rule claim under the reminder's keys.
        let a = Kit::new();
        let b = a.second_device();
        let request = ClaimRequest {
            owner_id: testkit::main_id(),
            bearer: testkit::token_of(&testkit::main_id()),
            elements: converted("m0"),
            kind: CallKind::Rule,
            budget: Duration::from_secs(2),
            settle_ms: 60_000,
            next_fire_in: None,
        };
        match a.engine.claims.claim(request).await {
            ClaimResult::Answered { answer, .. } => assert!(answer.elements[0].won),
            other => panic!("{other:?}"),
        }
        // Device B still runs the reminder: the same message loses the event, a later one inside the cooldown loses the rail.
        let reminder = reminder(def);
        let mut f = fire_of(ReminderSlot::Keyword { msg_id: "m0".into() });
        f.is_current = true;
        let env_b = FakeEnv::on(&b, vec![sent()]);
        let same = run_fire(&env_b, &reminder, &f).await;
        assert!(matches!(&same, FireEnd::Skipped(w) if w.starts_with("Another device posted this reminder")), "{same:?}");
        f.slot = ReminderSlot::Keyword { msg_id: "m1".into() };
        let later = run_fire(&env_b, &reminder, &f).await;
        assert!(matches!(&later, FireEnd::Skipped(w) if w.starts_with("Cooldown from")), "{later:?}");
        assert_eq!(env_b.posted_times(), 0, "one post across the rule and the reminder");
    }

    #[tokio::test]
    async fn the_claim_is_given_back_only_when_nothing_can_have_posted() {
        // (copies, released): another device posts afterwards only when it was released.
        let cases: Vec<(&str, Vec<CopyResult>, bool)> = vec![
            ("failed before it left", vec![before()], true),
            ("Twitch said no with a reason", vec![twitch_refused()], true),
            ("failed after it may have left", vec![after()], false),
            ("a copy was sent", vec![sent()], false),
            ("a refusal with no reason", vec![unsaid_refusal()], false),
            ("one sent, then a failure", vec![sent(), before()], false),
        ];
        for (name, copies, released) in cases {
            let a = Kit::new();
            let b = a.second_device();
            let r = pinned("interval", json!({"interval_seconds": 600}));
            let slot = ReminderSlot::Interval { secs: 600 };
            let env_a = FakeEnv::on(&a, copies);
            let first = run_fire(&env_a, &r, &fire_of(slot.clone())).await;
            assert!(matches!(first, FireEnd::Posted { released: got, .. } if got == released), "{name}: {first:?}");
            a.settle_claims().await;
            let env_b = FakeEnv::on(&b, vec![sent()]);
            let second = run_fire(&env_b, &r, &fire_of(slot)).await;
            assert_eq!(matches!(second, FireEnd::Posted { .. }), released, "{name}: {second:?}");
        }
        // post_text's early returns attempt nothing: `release_after(&[])` is true, so the claim goes back.
        let a = Kit::new();
        let b = a.second_device();
        let r = pinned("delay", json!({"delay_seconds": 60}));
        let mut env_a = FakeEnv::on(&a, vec![]);
        env_a.post_result = (false, Vec::new());
        assert_eq!(run_fire(&env_a, &r, &fire_of(ReminderSlot::Delay { secs: 60 })).await, FireEnd::Quiet);
        a.settle_claims().await;
        let env_b = FakeEnv::on(&b, vec![sent()]);
        assert!(matches!(run_fire(&env_b, &r, &fire_of(ReminderSlot::Delay { secs: 60 })).await, FireEnd::Posted { .. }));
    }

    #[tokio::test]
    async fn ten_copies_post_under_one_claim_and_the_other_device_loses_throughout() {
        let a = Kit::new();
        let b = a.second_device();
        let r = pinned("interval", json!({"interval_seconds": 600, "repeat_count": 10}));
        let env_a = FakeEnv::on(&a, vec![sent(); 10]);
        assert!(matches!(run_fire(&env_a, &r, &fire_of(ReminderSlot::Interval { secs: 600 })).await, FireEnd::Posted { .. }));
        assert_eq!(env_a.posts.lock().unwrap()[0].2, 10);
        assert_eq!(a.outward.claim_calls(), 1, "one claim for all ten copies");
        let env_b = FakeEnv::on(&b, vec![sent()]);
        assert!(matches!(run_fire(&env_b, &r, &fire_of(ReminderSlot::Interval { secs: 600 })).await, FireEnd::Skipped(_)));
    }

    #[tokio::test]
    async fn a_reminder_that_could_not_check_posts_anyway_and_never_releases() {
        let r = pinned("interval", json!({"interval_seconds": 600}));
        let slot = ReminderSlot::Interval { secs: 600 };
        // A 503 is not a valid claim answer.
        let a = Kit::new();
        a.outward.claim_answers(vec![FakeHttp::Status(503, String::new())]);
        let env = FakeEnv::on(&a, vec![before()]);
        let end = run_fire(&env, &r, &fire_of(slot.clone())).await;
        assert_eq!(end, FireEnd::Posted { anyway: true, released: false });
        a.settle_claims().await;
        assert!(a.outward.releases().is_empty(), "a reminder that posted anyway never releases");
        assert!(env.logs.lock().unwrap().iter().any(|(_, text)| text == "Couldn't check with StreamNook; posted anyway"));
        // No engine yet (before `Engine::init`): the same.
        let none = FakeEnv::alone(vec![sent()]);
        assert_eq!(run_fire(&none, &r, &fire_of(slot.clone())).await, FireEnd::Posted { anyway: true, released: false });
        // A current reminder whose room lookup failed: posts anyway, unclaimed, no claim call at all.
        let c = Kit::new();
        let mut env = FakeEnv::on(&c, vec![sent()]);
        env.room = None;
        let current = reminder(json!({"id":"r","enabled":true,"trigger":"interval","interval_seconds":600,"message":"hi","channel_scope":"current"}));
        let mut f = fire_of(slot);
        f.is_current = true;
        assert_eq!(run_fire(&env, &current, &f).await, FireEnd::Posted { anyway: true, released: false });
        assert_eq!(c.outward.claim_calls(), 0);
        assert_eq!(env.posts.lock().unwrap()[0].1, None, "a current reminder posts with no channel id, as today");
    }

    #[tokio::test]
    async fn a_429_that_fits_is_waited_once_and_one_that_does_not_posts_anyway() {
        let r = pinned("interval", json!({"interval_seconds": 600}));
        let slot = ReminderSlot::Interval { secs: 600 };
        let busy = |s: u64| FakeHttp::Status(429, format!(r#"{{"error":"busy","retry_after_s":{s}}}"#));
        // A 429 whose wait is 0 s fits: the client asks again and the real answer wins.
        let a = Kit::new();
        a.outward.claim_answers(vec![busy(0)]);
        let env = FakeEnv::on(&a, vec![sent()]);
        assert_eq!(run_fire(&env, &r, &fire_of(slot.clone())).await, FireEnd::Posted { anyway: false, released: false });
        assert_eq!(a.outward.claim_calls(), 2);
        // A second 429 posts anyway.
        let b = Kit::new();
        b.outward.claim_answers(vec![busy(0), busy(0)]);
        let env = FakeEnv::on(&b, vec![sent()]);
        assert_eq!(run_fire(&env, &r, &fire_of(slot.clone())).await, FireEnd::Posted { anyway: true, released: false });
        assert_eq!(b.outward.claim_calls(), 2);
        // A Retry-After past the time left posts anyway at once.
        let c = Kit::new();
        c.outward.claim_answers(vec![busy(60)]);
        let env = FakeEnv::on(&c, vec![sent()]);
        assert_eq!(run_fire(&env, &r, &fire_of(slot)).await, FireEnd::Posted { anyway: true, released: false });
        assert_eq!(c.outward.claim_calls(), 1);
    }

    #[tokio::test]
    async fn the_claim_is_made_as_the_account_that_posts_and_ignores_run_state() {
        let r = pinned("interval", json!({"interval_seconds": 600}));
        let slot = ReminderSlot::Interval { secs: 600 };
        // Two devices with run Off post once (the Engine singleton lives while run is Off).
        let a = Kit::new();
        let b = a.second_device();
        a.settings(crate::models::automations::RunState::Off, vec![]).await;
        b.settings(crate::models::automations::RunState::Off, vec![]).await;
        let (env_a, env_b) = (FakeEnv::on(&a, vec![sent()]), FakeEnv::on(&b, vec![sent()]));
        assert!(matches!(run_fire(&env_a, &r, &fire_of(slot.clone())).await, FireEnd::Posted { anyway: false, .. }));
        assert!(matches!(run_fire(&env_b, &r, &fire_of(slot.clone())).await, FireEnd::Skipped(_)));
        // The bearer is the poster's verified token.
        assert_eq!(a.outward.claim_bearers(), vec![testkit::token_of(&testkit::main_id())]);
        // A dead sign-in skips; an account Twitch cannot confirm posts anyway.
        let c = Kit::new();
        c.outward.set_token(&testkit::main_id(), FakeToken::NoToken);
        let env = FakeEnv::on(&c, vec![sent()]);
        assert!(matches!(run_fire(&env, &r, &fire_of(slot.clone())).await, FireEnd::Skipped(w) if w == "Sign in as @me again"));
        for how in [FakeToken::Unreachable, FakeToken::WrongAccount("999".into())] {
            let d = Kit::new();
            d.outward.set_token(&testkit::main_id(), how);
            let env = FakeEnv::on(&d, vec![sent()]);
            assert_eq!(run_fire(&env, &r, &fire_of(slot.clone())).await, FireEnd::Posted { anyway: true, released: false });
            assert_eq!(d.outward.claim_calls(), 0, "no claim without a verified owner");
        }
    }

    #[tokio::test]
    async fn a_fifth_concurrent_reminder_claim_waits_for_a_pool_slot() {
        let a = Kit::new();
        a.outward.park_claims();
        let r = pinned("keyword", json!({"keyword":"hello","keyword_cooldown_minutes": 0}));
        let mut tasks = Vec::new();
        for n in 0..5 {
            let env = FakeEnv::on(&a, vec![sent()]);
            let r = r.clone();
            tasks.push(tokio::spawn(async move {
                run_fire(&env, &r, &fire_of(ReminderSlot::Keyword { msg_id: format!("m{n}") })).await
            }));
        }
        a.outward.wait_for_claims_started(4).await;
        assert_eq!(a.outward.claim_calls(), 4, "the pool is 4: the fifth waits inside its 15 s");
        a.outward.release_claims();
        for task in tasks {
            assert!(matches!(task.await.unwrap(), FireEnd::Posted { anyway: false, .. }));
        }
        assert_eq!(a.outward.claim_calls(), 5);
    }

    #[test]
    fn fires_collapse_into_one_activity_row_and_never_push_rule_entries_out() {
        let mut log = Log::default();
        let rule_entry = |n: usize| NewEntry {
            kind: EntryKind::Acted,
            rule_id: format!("r-{n}"),
            rule_name: format!("Rule {n}"),
            channel: "twitch:me".into(),
            text: "Replied".into(),
        };
        let fire_entry = || NewEntry {
            kind: EntryKind::Acted,
            rule_id: "reminder:abc".into(),
            rule_name: "Plug the Discord".into(),
            channel: "twitch:me".into(),
            text: "Posted".into(),
        };
        for n in 0..60 {
            log.push(rule_entry(n), 1_000 + n as i64);
        }
        for i in 0..360 {
            log.push(fire_entry(), 100_000 + i * 10_000);
            if i % 6 == 0 {
                log.push(rule_entry(100 + i as usize), 100_000 + i * 10_000 + 1);
            }
        }
        let all = log.query(Filter::All, 1_000, None);
        let reminder_rows: Vec<_> = all.iter().filter(|e| e.rule_id == "reminder:abc").collect();
        assert_eq!(reminder_rows.len(), 1);
        assert_eq!(reminder_rows[0].count, 360);
        assert_eq!(reminder_rows[0].at_ms, 100_000 + 359 * 10_000);
        assert_eq!(all.len(), 60 + 60 + 1, "every rule entry is still in the ring");
        // A skip folds by chat and words however old its twin is; another reason is its own row.
        let skip = |text: &str| NewEntry {
            kind: EntryKind::Skipped,
            rule_id: "reminder:abc".into(),
            rule_name: "Plug the Discord".into(),
            channel: "twitch:me".into(),
            text: text.into(),
        };
        log.push(skip("Another device posted this reminder (Windows PC)"), 10_000_000);
        log.push(skip("Another device posted this reminder (Windows PC)"), 90_000_000);
        log.push(skip("Cooldown from Windows PC"), 90_000_001);
        let skipped = log.query(Filter::Skipped, 100, None);
        assert_eq!(skipped.len(), 2);
        assert_eq!(skipped.iter().map(|e| e.count).sum::<u32>(), 3);
        // A reminder row is never a rule's last outcome.
        assert!(log.last_for_rule("abc").is_none());
    }

    // ---- source pins ----------------------------------------------------------------------------

    fn this_file() -> String {
        include_str!("reminder_service.rs").split("#[cfg(test)]").next().unwrap_or_default().to_string()
    }

    fn body_of<'a>(src: &'a str, start: &str) -> &'a str {
        let at = src.find(start).unwrap_or_else(|| panic!("{start} missing"));
        let rest = &src[at..];
        &rest[..rest.find("\n}\n").unwrap_or(rest.len())]
    }

    #[test]
    fn fire_claims_before_post_text_and_post_text_claims_nothing() {
        let src = this_file();
        let run = body_of(&src, "pub(crate) async fn run_fire(");
        let at = |needle: &str| run.find(needle).unwrap_or_else(|| panic!("{needle} missing:\n{run}"));
        assert!(at("reminder_claim::claim_step(") < at("env.post("), "the claim comes before the post");
        assert!(at("env.gate_open(") < at("reminder_claim::claim_step("), "the keyword gate comes first");
        assert!(at("env.room_id(") < at("reminder_claim::claim_step("), "the room id is resolved before the claim");
        assert!(at("chat_send_limiter::release_after(&copies)") > at("env.post("), "release reads the copies after posting");
        let post = body_of(&src, "async fn post_text_with<I: PacedIo>(");
        assert!(!post.contains("claim") && !post.contains("door"), "post_text makes no claim, so /remind now claims nothing");
        assert!(post.contains("chat_send_limiter::paced_send(io,"), "every copy is the same paced send chat_send_paced wraps");
        assert!(!src.contains("Outward::") && !src.contains("outward."), "reminder posting never goes through Outward");
    }

    #[test]
    fn tick_gates_on_open_for_reminders_and_on_message_never_awaits_the_gate() {
        let src = this_file();
        let tick = body_of(&src, "async fn tick() {");
        assert!(tick.contains("open_for_reminders(login)") && !tick.contains("is_joined("));
        let on_message = body_of(&src, "pub fn on_message(");
        assert!(!on_message.contains(".await") && !on_message.contains("open_for_reminders"));
        assert!(on_message.contains("gate_refused_recently("));
        // The claim client is never asked about run state or the foreground.
        let claim = include_str!("account_automations/reminder_claim.rs").split("#[cfg(test)]").next().unwrap_or_default();
        assert!(!claim.contains("run_state(") && !claim.contains("foreground("), "run Off and a backgrounded phone never change a reminder claim");
    }

    #[test]
    fn the_settings_writer_takes_the_lock_changes_a_copy_then_writes_flushes_and_says_so() {
        let src = include_str!("../commands/settings.rs");
        let body = body_of(src, "pub(crate) fn commit_settings(");
        let at = |needle: &str| body.find(needle).unwrap_or_else(|| panic!("{needle} missing:\n{body}"));
        assert!(at("state.settings.lock()") < at("change(&mut copy)"));
        assert!(at("change(&mut copy)") < at("*state_settings = copy;"));
        assert!(at("after_settings_change(&settings, false, None)") < at("write_settings_to_disk(&settings)?"));
        assert!(at("write_settings_to_disk(&settings)?") < at("flush_now_if_phone_idle();"));
        assert!(at("flush_now_if_phone_idle();") < at("SETTINGS_UPDATED_EVENT, SettingsUpdated { source: None, keys }"));
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
