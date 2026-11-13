//! Account automations: the stored settings group, the preferences, and the
//! typed reading of one rule.
//!
//! `Settings.automations` is backend-owned (`Settings::adopt_backend_owned`):
//! only the automations commands write it. Its rules are stored as raw JSON
//! values. Each is read into a [`Rule`] when the engine compiles; a rule a
//! newer StreamNook wrote (a trigger or action this build does not know)
//! fails that reading, shows read-only and is never rewritten. A save merges
//! the new typed value over the stored raw value ([`merge_raw`]), so fields
//! this build does not name survive its edits, including inside an action.
//!
//! Every rule type has a serde default for every field, and every `Option`
//! serializes as an explicit `null`, so a save can clear a value.
//!
//! Mirrored in `src/types/index.ts`; the iPhone decodes the same JSON.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

/// The stored shape's version.
pub const AUTOMATIONS_VERSION: u32 = 1;
/// Rules one install keeps.
pub const MAX_RULES: usize = 100;
/// Actions one rule runs.
pub const MAX_ACTIONS: usize = 5;
/// Lines one Reply or Send picks from.
pub const MAX_LINES: usize = 20;
/// Characters in one line (Twitch's message limit).
pub const MAX_LINE_CHARS: usize = 500;
/// Characters in a rule's name.
pub const MAX_NAME_CHARS: usize = 60;
/// Chats one rule names.
pub const MAX_RULE_CHANNELS: usize = 25;
/// Chats the rules keep joined in all.
pub const MAX_ENGINE_CHATS: usize = 50;
/// Characters in a message pattern.
pub const MAX_PATTERN_CHARS: usize = 200;
/// The compiled size a Regex pattern may reach.
pub const REGEX_SIZE_LIMIT: usize = 64 * 1024;
/// The longest cooldown (30 days).
pub const MAX_COOLDOWN_SECONDS: u32 = 2_592_000;
/// Characters in an away message.
pub const MAX_AWAY_CHARS: usize = 200;
/// The fewest actions an hour a rule may be held to.
pub const MIN_PER_HOUR: u32 = 1;
/// The most actions an hour a rule may take.
pub const MAX_PER_HOUR: u32 = 120;
/// The longest a rule waits for a send slot.
pub const MAX_WAIT_SECONDS: u32 = 130;
/// The tray's "Set away" text when no away message was ever written.
pub const DEFAULT_AWAY_MESSAGE: &str = "Back soon";

/// Whether rules run: Off, Test (they log what they would do) or On.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    #[default]
    Off,
    Test,
    On,
}

impl RunState {
    fn from_value(v: &Value) -> Option<RunState> {
        match v.as_str() {
            Some("off") => Some(RunState::Off),
            Some("test") => Some(RunState::Test),
            Some("on") => Some(RunState::On),
            _ => None,
        }
    }
}

impl<'de> Deserialize<'de> for RunState {
    /// Anything but the three known names (a newer build's value, a hand
    /// edit) reads as Off: settings load as one parse, and a value this build
    /// does not know must never fail the file or switch rules on.
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(RunState::from_value(&Value::deserialize(d)?).unwrap_or(RunState::Off))
    }
}

/// This device's away: set by `/afk`, `/back`, the page and the tray on the
/// device they run on, never shared with another device.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Away {
    pub message: String,
    pub since_ms: i64,
}

impl Away {
    /// Read leniently: any object is an away (its message clipped to 200
    /// characters); anything else is no away.
    pub fn from_value(v: &Value) -> Option<Away> {
        let o = v.as_object()?;
        Some(Away {
            message: clip(o.get("message").and_then(Value::as_str).unwrap_or_default().trim(), MAX_AWAY_CHARS),
            since_ms: o.get("since_ms").and_then(Value::as_i64).unwrap_or(0),
        })
    }
}

/// `Settings.automations`: backend-owned, written only through the
/// automations commands.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Automations {
    pub version: u32,
    /// Off at install.
    pub run: RunState,
    /// What Resume returns to; never Off.
    pub last_active: RunState,
    /// This device's away; an explicit null when clear.
    pub away: Option<Away>,
    /// The tray's "Set away" uses it (else "Back soon").
    pub last_away_message: String,
    /// Raw rules, at most 100.
    pub rules: Vec<Value>,
    /// Reminder ids turned off by "Make it a rule"; a listed reminder never
    /// fires, whatever its `enabled`.
    pub converted_reminders: Vec<String>,
    /// Keys a newer build added to this group.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for Automations {
    fn default() -> Self {
        Automations {
            version: AUTOMATIONS_VERSION,
            run: RunState::Off,
            last_active: RunState::Test,
            away: None,
            last_away_message: String::new(),
            rules: Vec::new(),
            converted_reminders: Vec::new(),
            extra: Map::new(),
        }
    }
}

impl Automations {
    /// Read the group field by field, so one damaged field never empties the
    /// rest: a group that is not an object is the default, a `rules` that is
    /// not a list is empty, an unknown run state is Off, and keys this build
    /// does not name are kept in `extra`.
    pub fn from_value(v: Value) -> Automations {
        let Value::Object(mut m) = v else { return Automations::default() };
        let mut out = Automations::default();
        if let Some(n) = m.remove("version").and_then(|v| v.as_u64()) {
            out.version = u32::try_from(n).unwrap_or(u32::MAX);
        }
        out.run = m.remove("run").and_then(|v| RunState::from_value(&v)).unwrap_or(RunState::Off);
        out.last_active = m
            .remove("last_active")
            .and_then(|v| RunState::from_value(&v))
            .filter(|s| *s != RunState::Off)
            .unwrap_or(RunState::Test);
        out.away = m.remove("away").and_then(|v| Away::from_value(&v));
        out.last_away_message = m
            .remove("last_away_message")
            .and_then(|v| v.as_str().map(|s| clip(s.trim(), MAX_AWAY_CHARS)))
            .unwrap_or_default();
        out.rules = match m.remove("rules") {
            Some(Value::Array(rules)) => rules,
            _ => Vec::new(),
        };
        out.converted_reminders = match m.remove("converted_reminders") {
            Some(Value::Array(ids)) => ids.into_iter().filter_map(|v| v.as_str().map(str::to_string)).collect(),
            _ => Vec::new(),
        };
        out.extra = m;
        out
    }

    /// Every stored rule read typed, in order; an `Err` is a rule this build
    /// cannot read (made by a newer StreamNook), kept as it is.
    pub fn typed_rules(&self) -> Vec<Result<Rule, String>> {
        self.rules.iter().map(Rule::from_value).collect()
    }
}

impl<'de> Deserialize<'de> for Automations {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Automations::from_value(Value::deserialize(d)?))
    }
}

/// `Settings.automation_prefs`: ordinary preferences, written field by field.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct AutomationPrefs {
    /// Outward sends across all rules and devices in a rolling minute (1 to 20).
    pub sends_per_minute: u32,
    /// Rules may post in chats that are not the owner's.
    pub other_channels: bool,
    /// At most one automatic message per other chat in this many seconds (10 to 600).
    pub other_channel_gap_seconds: u32,
    /// A line only you see in chat when a rule acts or would act.
    pub chat_notes: bool,
    /// People a rule may whisper in a day (1 to 40).
    pub whisper_people_per_day: u32,
    /// Keys the frontend adds to this group before this struct names them.
    #[serde(flatten)]
    pub extra: HashMap<String, Value>,
}

impl Default for AutomationPrefs {
    fn default() -> Self {
        AutomationPrefs {
            sends_per_minute: 10,
            other_channels: false,
            other_channel_gap_seconds: 30,
            chat_notes: true,
            whisper_people_per_day: 20,
            extra: HashMap::new(),
        }
    }
}

impl AutomationPrefs {
    /// The preferences with every number inside its range, as the engine
    /// reads them.
    pub fn clamped(&self) -> AutomationPrefs {
        AutomationPrefs {
            sends_per_minute: self.sends_per_minute.clamp(1, 20),
            other_channel_gap_seconds: self.other_channel_gap_seconds.clamp(10, 600),
            whisper_people_per_day: self.whisper_people_per_day.clamp(1, 40),
            ..self.clone()
        }
    }
}

/// One account a rule names. `login` and `name` are refreshed by id.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(default)]
pub struct AccountRef {
    pub provider: String,
    pub id: String,
    pub login: String,
    pub name: String,
}

impl Default for AccountRef {
    fn default() -> Self {
        AccountRef { provider: "twitch".into(), id: String::new(), login: String::new(), name: String::new() }
    }
}

/// What makes a rule fire. P2 to P5 add variants; a rule naming one this
/// build does not know fails to read and is held read-only.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Trigger {
    Message {
        #[serde(default)]
        text: TextMatch,
    },
    Mention {
        #[serde(default)]
        replies_too: bool,
    },
    FirstMessage {},
    ChannelEvent {
        #[serde(default)]
        events: Vec<EventKind>,
        #[serde(default)]
        min_months: u32,
        #[serde(default)]
        min_gifts: u32,
        #[serde(default)]
        min_viewers: u32,
        #[serde(default)]
        min_bits: u32,
    },
    BadgeDrop {
        #[serde(default)]
        on: BadgeLeg,
        #[serde(default)]
        link: BadgeLink,
        #[serde(default)]
        batch_text: String,
    },
}

impl Default for Trigger {
    fn default() -> Self {
        Trigger::Mention { replies_too: true }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Sub,
    Resub,
    Gift,
    GiftBomb,
    Raid,
    Cheer,
    Announcement,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum BadgeLeg {
    Announced,
    Earnable,
    #[default]
    Both,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LinkPreset {
    #[default]
    #[serde(rename = "streamnook")]
    StreamNook,
    #[serde(rename = "streamdatabase")]
    StreamDatabase,
    #[serde(rename = "custom")]
    Custom,
    #[serde(rename = "none")]
    None,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LinkFallback {
    #[default]
    #[serde(rename = "streamnook")]
    StreamNook,
    #[serde(rename = "no_link")]
    NoLink,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(default)]
pub struct BadgeLink {
    pub preset: LinkPreset,
    pub custom: String,
    pub if_not_ready: LinkFallback,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum MatchMode {
    AnyWord,
    AllWords,
    Phrase,
    #[default]
    Contains,
    StartsWith,
    Exact,
    Regex,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(default)]
pub struct TextMatch {
    pub mode: MatchMode,
    /// Kept as typed (a Regex is never lowercased).
    pub pattern: String,
    pub case_sensitive: bool,
}

/// Which chats a rule listens in.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChannelScope {
    /// The owner's channel.
    Own {},
    These {
        #[serde(default)]
        list: Vec<ChannelRef>,
    },
    /// Any chat open in a window (or adopted when one closed).
    AnyOpen {},
    /// The chat of this device's current Twitch watch; not for badge rules.
    Watching {},
}

impl Default for ChannelScope {
    fn default() -> Self {
        ChannelScope::Own {}
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SenderRole {
    Broadcaster,
    Moderator,
    Vip,
    Subscriber,
    Viewer,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Weekday {
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
    Sun,
}

/// Local hours a rule runs in; `to` before `from` crosses midnight. No days
/// means every day.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(default)]
pub struct Hours {
    pub from: String,
    pub to: String,
    pub days: Vec<Weekday>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(default)]
pub struct Conditions {
    pub channels: ChannelScope,
    /// Empty = anyone.
    pub roles: Vec<SenderRole>,
    pub skip_users: Vec<String>,
    pub only_users: Vec<String>,
    pub only_live: bool,
    pub only_away: bool,
    /// Include Shared Chat partner chat lines (never events).
    pub shared_chat: bool,
    pub hours: Option<Hours>,
}

/// A chat a rule names. `id` is the element id; `key` is built in Rust
/// (`twitch:login`); `room_id` is resolved when the rule is saved.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(default)]
pub struct ChannelRef {
    pub id: String,
    pub key: String,
    pub room_id: String,
    pub name: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(default)]
pub struct Line {
    pub id: String,
    pub text: String,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Pick {
    #[default]
    Random,
    Shuffle,
    InOrder,
}

fn yes() -> bool {
    true
}

fn full_volume() -> f32 {
    1.0
}

/// What a rule does. P2 to P5 add variants; a rule naming one this build
/// does not know fails to read and is held read-only.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Action {
    Reply {
        #[serde(default)]
        id: String,
        #[serde(default)]
        texts: Vec<Line>,
        #[serde(default)]
        pick: Pick,
        #[serde(default)]
        thread: bool,
    },
    Send {
        #[serde(default)]
        id: String,
        /// None = the chat the rule fired in.
        #[serde(default)]
        to: Option<ChannelRef>,
        #[serde(default)]
        texts: Vec<Line>,
        #[serde(default)]
        pick: Pick,
    },
    Notify {
        #[serde(default)]
        id: String,
        #[serde(default)]
        title: String,
        #[serde(default)]
        body: String,
        #[serde(default = "yes")]
        toast: bool,
        #[serde(default)]
        system: bool,
        #[serde(default)]
        sound: Option<String>,
        /// 0 to 1.
        #[serde(default = "full_volume")]
        volume: f32,
    },
}

impl Action {
    pub fn id(&self) -> &str {
        match self {
            Action::Reply { id, .. } | Action::Send { id, .. } | Action::Notify { id, .. } => id,
        }
    }

    fn id_mut(&mut self) -> &mut String {
        match self {
            Action::Reply { id, .. } | Action::Send { id, .. } | Action::Notify { id, .. } => id,
        }
    }

    /// Whether the action posts outside this device (and so claims).
    pub fn is_outward(&self) -> bool {
        matches!(self, Action::Reply { .. } | Action::Send { .. })
    }

    /// The lines a Reply or Send picks from; none for a Notify.
    pub fn lines(&self) -> &[Line] {
        match self {
            Action::Reply { texts, .. } | Action::Send { texts, .. } => texts,
            Action::Notify { .. } => &[],
        }
    }

    fn lines_mut(&mut self) -> Option<&mut Vec<Line>> {
        match self {
            Action::Reply { texts, .. } | Action::Send { texts, .. } => Some(texts),
            Action::Notify { .. } => None,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(default)]
pub struct Limits {
    /// Per chat; never for badge rules.
    pub rule_cooldown_seconds: u32,
    /// Per person.
    pub user_cooldown_seconds: u32,
    /// 1 to 120.
    pub max_per_hour: u32,
    /// 0 to 130.
    pub max_wait_seconds: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { rule_cooldown_seconds: 30, user_cooldown_seconds: 300, max_per_hour: 30, max_wait_seconds: 10 }
    }
}

/// The typed reading of one raw rule.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Rule {
    /// "r-" + 12 hex, made in Rust.
    pub id: String,
    pub name: String,
    pub enabled: bool,
    /// False = Test.
    pub live: bool,
    /// Whose rules: "your channel", the claim account.
    pub owner: AccountRef,
    /// Who speaks; defaults to the owner.
    pub send_as: AccountRef,
    pub when: Trigger,
    pub only_if: Conditions,
    pub actions: Vec<Action>,
    pub limits: Limits,
    /// Invisible variation on repeats.
    pub vary: bool,
    /// "template:<id>", or "reminder:<id>" for a converted reminder.
    pub origin: Option<String>,
    pub created_ms: i64,
    pub updated_ms: i64,
}

impl Default for Rule {
    fn default() -> Self {
        Rule {
            id: String::new(),
            name: String::new(),
            enabled: true,
            live: false,
            owner: AccountRef::default(),
            send_as: AccountRef::default(),
            when: Trigger::default(),
            only_if: Conditions::default(),
            actions: Vec::new(),
            limits: Limits::default(),
            vary: false,
            origin: None,
            created_ms: 0,
            updated_ms: 0,
        }
    }
}

impl Rule {
    /// Read one raw rule; an `Err` is a rule this build cannot read.
    pub fn from_value(v: &Value) -> Result<Rule, String> {
        serde_json::from_value(v.clone()).map_err(|e| e.to_string())
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    pub fn is_badge_rule(&self) -> bool {
        matches!(self.when, Trigger::BadgeDrop { .. })
    }
}

/// A new rule id: "r-" + 12 hex.
pub fn new_rule_id() -> String {
    format!("r-{:012x}", rand::random::<u64>() & 0xffff_ffff_ffff)
}

/// A new element id: `prefix` + "-" + 8 hex (`a` actions, `l` lines, `c` chats).
pub fn new_element_id(prefix: &str) -> String {
    format!("{prefix}-{:08x}", rand::random::<u32>())
}

/// Give the rule and every action, line and chat an id where it has none,
/// or where an earlier element of the same list already took it.
pub fn fill_ids(rule: &mut Rule) {
    if rule.id.trim().is_empty() {
        rule.id = new_rule_id();
    }
    let mut actions = HashSet::new();
    for action in rule.actions.iter_mut() {
        let id = action.id_mut();
        if id.trim().is_empty() || actions.contains(id.as_str()) {
            *id = new_element_id("a");
        }
        actions.insert(id.clone());
        if let Some(lines) = action.lines_mut() {
            let mut seen = HashSet::new();
            for line in lines.iter_mut() {
                if line.id.trim().is_empty() || seen.contains(line.id.as_str()) {
                    line.id = new_element_id("l");
                }
                seen.insert(line.id.clone());
            }
        }
        if let Action::Send { to: Some(chat), .. } = action {
            if chat.id.trim().is_empty() {
                chat.id = new_element_id("c");
            }
        }
    }
    if let ChannelScope::These { list } = &mut rule.only_if.channels {
        let mut seen = HashSet::new();
        for chat in list.iter_mut() {
            if chat.id.trim().is_empty() || seen.contains(chat.id.as_str()) {
                chat.id = new_element_id("c");
            }
            seen.insert(chat.id.clone());
        }
    }
}

/// Merge a rule's new typed value over its stored raw value:
///
/// - objects merge key by key, recursively, and keep keys the new value
///   does not name (a newer build's fields);
/// - an object whose `kind` changed is replaced whole;
/// - a `null` in the new value replaces (clearing `hours` or `to` works);
/// - a list whose every element carries an `id` merges by `id` in the new
///   order (an element whose id is gone is removed, a new one added);
/// - any other list merges by index where both elements are objects with
///   the same `kind`, else takes the new element;
/// - scalars replace.
pub fn merge_raw(old: &Value, new: &Value) -> Value {
    match (old, new) {
        (Value::Object(o), Value::Object(n)) => {
            if let (Some(old_kind), Some(new_kind)) = (o.get("kind"), n.get("kind")) {
                if old_kind != new_kind {
                    return new.clone();
                }
            }
            let mut out = o.clone();
            for (key, value) in n {
                let merged = match o.get(key) {
                    Some(prior) if !value.is_null() => merge_raw(prior, value),
                    _ => value.clone(),
                };
                out.insert(key.clone(), merged);
            }
            Value::Object(out)
        }
        (Value::Array(o), Value::Array(n)) => Value::Array(merge_list(o, n)),
        _ => new.clone(),
    }
}

fn element_id(v: &Value) -> Option<&str> {
    v.get("id")?.as_str().filter(|id| !id.is_empty())
}

fn merge_list(old: &[Value], new: &[Value]) -> Vec<Value> {
    if !new.is_empty() && new.iter().all(|e| element_id(e).is_some()) {
        return new
            .iter()
            .map(|e| match old.iter().find(|o| element_id(o) == element_id(e)) {
                Some(prior) => merge_raw(prior, e),
                None => e.clone(),
            })
            .collect();
    }
    new.iter()
        .enumerate()
        .map(|(i, e)| match old.get(i) {
            Some(prior) if prior.is_object() && e.is_object() && prior.get("kind").is_some() && prior.get("kind") == e.get("kind") => {
                merge_raw(prior, e)
            }
            _ => e.clone(),
        })
        .collect()
}

/// The `{placeholder}` names in `text`, by the custom-command grammar
/// (`slash_dispatch::expand_with`): `{N}`, `{N+}`, `{*}` and `{name}` (a
/// letter, then letters, digits, `_` or `.`); `{{` and `}}` are literal.
pub fn placeholders(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < text.len() {
        let rest = &text[i..];
        if rest.starts_with("{{") || rest.starts_with("}}") {
            i += 2;
            continue;
        }
        if bytes[i] == b'{' {
            if let Some(close) = rest[1..].find('}').map(|e| e + 1) {
                let inner = &rest[1..close];
                if is_placeholder(inner) {
                    out.push(inner.to_string());
                    i += close + 1;
                    continue;
                }
            }
        }
        i += rest.chars().next().map_or(1, char::len_utf8);
    }
    out
}

fn is_placeholder(inner: &str) -> bool {
    let digits_end = inner.find(|c: char| !c.is_ascii_digit()).unwrap_or(inner.len());
    if digits_end > 0 && (digits_end == inner.len() || &inner[digits_end..] == "+") {
        return true;
    }
    inner == "*"
        || (inner.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
            && inner.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.'))
}

/// Text the chatter wrote: `{message}`, `{N}`, `{N+}`, `{*}`.
pub fn is_chatter_text(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    if n == "message" || n == "*" {
        return true;
    }
    let digits = n.strip_suffix('+').unwrap_or(&n);
    !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit())
}

/// The channel's own words: `{title}`, `{game}`.
pub fn is_channel_state(name: &str) -> bool {
    matches!(name.to_ascii_lowercase().as_str(), "title" | "game")
}

/// A badge drop's details: `{badge}`, `{badge.*}`, `{badges}`.
pub fn is_badge_variable(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n == "badge" || n == "badges" || n.starts_with("badge.")
}

/// One problem with a rule, at the field the editor shows it under.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct FieldError {
    pub path: String,
    pub message: String,
}

/// What `validate` needs from the other rules.
#[derive(Clone, Debug, Default)]
pub struct ValidateCtx {
    /// How many other rules are stored.
    pub other_rules: usize,
    /// The chats the other enabled rules listen in (`listened_chats`).
    pub other_chats: HashSet<String>,
}

/// The chats a rule keeps joined to listen: the owner's for Own, the named
/// ones for These, none for AnyOpen and Watching.
pub fn listened_chats(rule: &Rule) -> Vec<String> {
    match &rule.only_if.channels {
        ChannelScope::Own {} if !rule.owner.login.trim().is_empty() => {
            vec![crate::services::chat_rules::channel_filter_key("twitch", rule.owner.login.trim())]
        }
        ChannelScope::These { list } => list.iter().map(|c| c.key.clone()).filter(|k| !login_of(k).is_empty()).collect(),
        _ => Vec::new(),
    }
}

fn login_of(key: &str) -> &str {
    key.split_once(':').map_or(key, |(_, login)| login).trim()
}

fn clip(text: &str, chars: usize) -> String {
    text.chars().take(chars).collect()
}

/// "HH:MM" as minutes past midnight.
pub fn parse_hhmm(text: &str) -> Option<u32> {
    let (h, m) = text.trim().split_once(':')?;
    if h.len() != 2 || m.len() != 2 {
        return None;
    }
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then_some(h * 60 + m)
}

fn normalize_users(list: &mut Vec<String>) {
    let mut seen = HashSet::new();
    let cleaned: Vec<String> = list
        .iter()
        .map(|u| u.trim().trim_start_matches('@').to_ascii_lowercase())
        .filter(|u| !u.is_empty() && seen.insert(u.clone()))
        .collect();
    *list = cleaned;
}

fn normalize_chat(chat: &mut ChannelRef) {
    let raw = if chat.key.trim().is_empty() { chat.name.trim() } else { chat.key.trim() };
    chat.key = if raw.is_empty() {
        String::new()
    } else {
        crate::services::chat_rules::channel_filter_key("twitch", raw)
    };
}

/// The forced and tidied values every saved rule carries.
fn normalize(rule: &mut Rule) {
    rule.name = rule.name.trim().to_string();
    if rule.owner.provider.trim().is_empty() {
        rule.owner.provider = "twitch".into();
    }
    if rule.send_as.id.trim().is_empty() {
        rule.send_as = rule.owner.clone();
    }
    if rule.send_as.provider.trim().is_empty() {
        rule.send_as.provider = "twitch".into();
    }
    normalize_users(&mut rule.only_if.skip_users);
    normalize_users(&mut rule.only_if.only_users);
    if let ChannelScope::These { list } = &mut rule.only_if.channels {
        list.iter_mut().for_each(normalize_chat);
    }
    if let Some(hours) = &mut rule.only_if.hours {
        let mut seen = HashSet::new();
        hours.days.retain(|d| seen.insert(*d));
    }
    for action in rule.actions.iter_mut() {
        match action {
            Action::Send { to: Some(chat), .. } => normalize_chat(chat),
            Action::Notify { volume, .. } => {
                *volume = if volume.is_finite() { volume.clamp(0.0, 1.0) } else { 1.0 };
            }
            _ => {}
        }
    }
    if rule.is_badge_rule() {
        rule.only_if.channels = ChannelScope::Own {};
        rule.only_if.only_live = true;
        rule.limits.rule_cooldown_seconds = 0;
    }
    fill_ids(rule);
}

struct Errors(Vec<FieldError>);

impl Errors {
    fn push(&mut self, path: impl Into<String>, message: &str) {
        let error = FieldError { path: path.into(), message: message.to_string() };
        if !self.0.contains(&error) {
            self.0.push(error);
        }
    }
}

/// Where a text's variables may go.
struct TextScope {
    own: bool,
    chatter_ok: bool,
    badge_rule: bool,
}

const CHATTER_WORDS: &str =
    "The chatter's words ({message}, {1}, {*}) can be posted only in your own channel, by a rule that answers only you and your moderators";
const CHANNEL_WORDS: &str = "{title} and {game} can be posted only in your own channel";
const BADGE_WORDS: &str = "Badge details work only in badge rules";

fn check_outward_text(path: &str, text: &str, scope: &TextScope, errors: &mut Errors) {
    for name in placeholders(text) {
        if is_chatter_text(&name) && !scope.chatter_ok {
            errors.push(path, CHATTER_WORDS);
        }
        if is_channel_state(&name) && !scope.own {
            errors.push(path, CHANNEL_WORDS);
        }
        if is_badge_variable(&name) && !scope.badge_rule {
            errors.push(path, BADGE_WORDS);
        }
    }
}

fn check_local_text(path: &str, text: &str, scope: &TextScope, errors: &mut Errors) {
    if placeholders(text).iter().any(|n| is_badge_variable(n)) && !scope.badge_rule {
        errors.push(path, BADGE_WORDS);
    }
}

fn check_pattern(text: &TextMatch, errors: &mut Errors) {
    let path = "when.text.pattern";
    if text.pattern.trim().is_empty() {
        errors.push(path, "Enter the words to watch for");
        return;
    }
    if text.pattern.chars().count() > MAX_PATTERN_CHARS {
        errors.push(path, "Keep the pattern to 200 characters");
        return;
    }
    if text.mode == MatchMode::Regex {
        let built = regex::RegexBuilder::new(&text.pattern)
            .case_insensitive(!text.case_sensitive)
            .size_limit(REGEX_SIZE_LIMIT)
            .build();
        if let Err(e) = built {
            let message = match e {
                regex::Error::CompiledTooBig(_) => "This pattern is too large to run",
                _ => "This isn't a valid regular expression",
            };
            errors.push(path, message);
        }
    }
}

/// Tidy and check a rule before it is saved. Forces what a rule kind
/// requires (a badge rule runs in your own channel, only while live, with no
/// rule cooldown), fills missing ids, and answers every field error; an
/// empty list means the rule saves.
pub fn validate(rule: &mut Rule, ctx: &ValidateCtx) -> Vec<FieldError> {
    normalize(rule);
    let mut errors = Errors(Vec::new());
    if ctx.other_rules >= MAX_RULES {
        errors.push("rules", "You can keep up to 100 rules");
    }
    let name_chars = rule.name.chars().count();
    if name_chars == 0 {
        errors.push("name", "Give the rule a name");
    } else if name_chars > MAX_NAME_CHARS {
        errors.push("name", "Keep the name to 60 characters");
    }
    if rule.owner.id.trim().is_empty() {
        errors.push("owner", "Pick whose rules these are");
    }

    let own = matches!(rule.only_if.channels, ChannelScope::Own {});
    let scope = TextScope {
        own,
        chatter_ok: own
            && !rule.only_if.roles.is_empty()
            && rule.only_if.roles.iter().all(|r| matches!(r, SenderRole::Broadcaster | SenderRole::Moderator)),
        badge_rule: rule.is_badge_rule(),
    };

    match &rule.when {
        Trigger::Message { text } => check_pattern(text, &mut errors),
        Trigger::ChannelEvent { events, .. } if events.is_empty() => {
            errors.push("when.events", "Pick at least one event");
        }
        Trigger::BadgeDrop { link, batch_text, .. } => {
            if link.preset == LinkPreset::Custom {
                let custom = link.custom.trim();
                if custom.is_empty() {
                    errors.push("when.link.custom", "Enter the link to post");
                } else if !custom.starts_with("https://") {
                    errors.push("when.link.custom", "Links must start with https://");
                }
            }
            if batch_text.chars().count() > MAX_LINE_CHARS {
                errors.push("when.batch_text", "Keep the message to 500 characters");
            }
            check_outward_text("when.batch_text", batch_text, &scope, &mut errors);
        }
        _ => {}
    }

    if let ChannelScope::These { list } = &rule.only_if.channels {
        if list.is_empty() {
            errors.push("only_if.channels", "Pick at least one channel");
        }
        if list.len() > MAX_RULE_CHANNELS {
            errors.push("only_if.channels", "A rule can watch up to 25 channels");
        }
        for (i, chat) in list.iter().enumerate() {
            if login_of(&chat.key).is_empty() {
                errors.push(format!("only_if.channels.list[{i}]"), "Pick a channel");
            }
        }
    }
    let mut chats = ctx.other_chats.clone();
    chats.extend(listened_chats(rule));
    if chats.len() > MAX_ENGINE_CHATS {
        errors.push("only_if.channels", "Your rules can watch up to 50 chats in all");
    }
    if let Some(hours) = &rule.only_if.hours {
        if parse_hhmm(&hours.from).is_none() {
            errors.push("only_if.hours.from", "Use a time like 09:30");
        }
        if parse_hhmm(&hours.to).is_none() {
            errors.push("only_if.hours.to", "Use a time like 09:30");
        }
    }

    if rule.actions.is_empty() {
        errors.push("actions", "Add at least one action");
    }
    if rule.actions.len() > MAX_ACTIONS {
        errors.push("actions", "A rule can do up to 5 things");
    }
    for (i, action) in rule.actions.iter().enumerate() {
        let at = format!("actions[{i}]");
        if scope.badge_rule && matches!(action, Action::Reply { .. }) {
            errors.push(at.clone(), "Badge rules can post a message or notify you, not reply");
        }
        match action {
            Action::Reply { texts, .. } | Action::Send { texts, .. } => {
                if texts.is_empty() {
                    errors.push(format!("{at}.texts"), "Add at least one line");
                }
                if texts.len() > MAX_LINES {
                    errors.push(format!("{at}.texts"), "Pick from up to 20 lines");
                }
                for (j, line) in texts.iter().enumerate() {
                    let path = format!("{at}.texts[{j}].text");
                    let text = line.text.trim();
                    if text.is_empty() {
                        errors.push(path.clone(), "Write the message");
                    } else if line.text.chars().count() > MAX_LINE_CHARS {
                        errors.push(path.clone(), "Keep each line to 500 characters");
                    } else if text.starts_with('/') || text.starts_with('.') {
                        errors.push(path.clone(), "A line can't start with / or .");
                    }
                    check_outward_text(&path, &line.text, &scope, &mut errors);
                }
            }
            Action::Notify { title, body, .. } => {
                check_local_text(&format!("{at}.title"), title, &scope, &mut errors);
                check_local_text(&format!("{at}.body"), body, &scope, &mut errors);
            }
        }
    }

    let limits = rule.limits;
    if !(MIN_PER_HOUR..=MAX_PER_HOUR).contains(&limits.max_per_hour) {
        errors.push("limits.max_per_hour", "Pick 1 to 120 times an hour");
    }
    if limits.max_wait_seconds > MAX_WAIT_SECONDS {
        errors.push("limits.max_wait_seconds", "Wait at most 130 seconds");
    }
    if limits.rule_cooldown_seconds > MAX_COOLDOWN_SECONDS {
        errors.push("limits.rule_cooldown_seconds", "Keep cooldowns to 30 days or less");
    }
    if limits.user_cooldown_seconds > MAX_COOLDOWN_SECONDS {
        errors.push("limits.user_cooldown_seconds", "Keep cooldowns to 30 days or less");
    }
    errors.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn me() -> AccountRef {
        AccountRef { provider: "twitch".into(), id: "100".into(), login: "me".into(), name: "Me".into() }
    }

    fn line(text: &str) -> Line {
        Line { id: String::new(), text: text.into() }
    }

    fn reply(text: &str) -> Action {
        Action::Reply { id: String::new(), texts: vec![line(text)], pick: Pick::Random, thread: true }
    }

    fn notify(title: &str) -> Action {
        Action::Notify {
            id: String::new(),
            title: title.into(),
            body: String::new(),
            toast: true,
            system: false,
            sound: None,
            volume: 1.0,
        }
    }

    fn chat(login: &str) -> ChannelRef {
        ChannelRef { id: String::new(), key: login.into(), room_id: String::new(), name: login.into() }
    }

    fn rule_with(action: Action) -> Rule {
        Rule {
            name: "Away reply".into(),
            owner: me(),
            send_as: me(),
            when: Trigger::Mention { replies_too: true },
            actions: vec![action],
            ..Rule::default()
        }
    }

    fn has(errors: &[FieldError], path: &str) -> bool {
        errors.iter().any(|e| e.path == path)
    }

    #[test]
    fn every_option_serializes_as_an_explicit_null() {
        let v = serde_json::to_value(Rule::default()).unwrap();
        assert!(v.get("origin").is_some_and(Value::is_null));
        assert!(v["only_if"].get("hours").is_some_and(Value::is_null));
        assert_eq!(v["when"]["kind"], "mention");
        assert_eq!(v["only_if"]["channels"]["kind"], "own");
        let send = serde_json::to_value(Action::Send { id: "a-1".into(), to: None, texts: vec![], pick: Pick::Random }).unwrap();
        assert_eq!(send["kind"], "send");
        assert!(send.get("to").is_some_and(Value::is_null));
        let alert = serde_json::to_value(notify("x")).unwrap();
        assert!(alert.get("sound").is_some_and(Value::is_null));
        let group = serde_json::to_value(Automations::default()).unwrap();
        assert!(group.get("away").is_some_and(Value::is_null));
        assert_eq!(group["run"], "off");
        assert_eq!(group["last_active"], "test");
        assert_eq!(group["version"], 1);
    }

    #[test]
    fn a_rule_reads_from_an_empty_object_with_its_defaults() {
        let r = Rule::from_value(&json!({})).unwrap();
        assert!(r.enabled);
        assert!(!r.live);
        assert_eq!(r.when, Trigger::Mention { replies_too: true });
        assert_eq!(r.limits, Limits { rule_cooldown_seconds: 30, user_cooldown_seconds: 300, max_per_hour: 30, max_wait_seconds: 10 });
        assert_eq!(r.owner.provider, "twitch");
        let alert: Action = serde_json::from_value(json!({ "kind": "notify" })).unwrap();
        assert_eq!(alert, notify(""));
        let link: BadgeLink = serde_json::from_value(json!({ "preset": "streamdatabase", "if_not_ready": "no_link" })).unwrap();
        assert_eq!((link.preset, link.if_not_ready), (LinkPreset::StreamDatabase, LinkFallback::NoLink));
    }

    #[test]
    fn a_rule_from_a_newer_build_is_held_and_kept_as_it_is() {
        let newer = json!({ "id": "r-000000000001", "name": "Timed", "when": { "kind": "timer", "every_seconds": 900 }, "actions": [] });
        assert!(Rule::from_value(&newer).is_err());
        let newer_action = json!({ "id": "r-000000000002", "actions": [{ "kind": "timeout", "id": "a-00000001" }] });
        assert!(Rule::from_value(&newer_action).is_err());
        let group = Automations { rules: vec![newer.clone(), newer_action.clone()], ..Automations::default() };
        let back: Automations = serde_json::from_value(serde_json::to_value(&group).unwrap()).unwrap();
        assert_eq!(back.rules, vec![newer, newer_action]);
        assert!(back.typed_rules().iter().all(Result::is_err));
    }

    #[test]
    fn the_group_reads_field_by_field() {
        let a: Automations = serde_json::from_value(json!({
            "run": "paused",
            "last_active": "off",
            "rules": "not a list",
            "away": { "message": "brb", "since_ms": 5 },
            "converted_reminders": ["rem-1", 7],
            "future_field": { "x": 1 }
        }))
        .unwrap();
        assert_eq!(a.run, RunState::Off);
        assert_eq!(a.last_active, RunState::Test, "Resume never returns to Off");
        assert!(a.rules.is_empty());
        assert_eq!(a.away, Some(Away { message: "brb".into(), since_ms: 5 }));
        assert_eq!(a.converted_reminders, vec!["rem-1".to_string()]);
        let saved = serde_json::to_value(&a).unwrap();
        assert_eq!(saved["future_field"], json!({ "x": 1 }), "a newer build's key survives");
        let garbage: Automations = serde_json::from_value(json!("garbage")).unwrap();
        assert_eq!(garbage, Automations::default());
        let long = Away::from_value(&json!({ "message": "x".repeat(300) })).unwrap();
        assert_eq!(long.message.chars().count(), MAX_AWAY_CHARS);
        let on: Automations = serde_json::from_value(json!({ "run": "on", "last_active": "on" })).unwrap();
        assert_eq!((on.run, on.last_active), (RunState::On, RunState::On));
    }

    #[test]
    fn prefs_default_and_clamp() {
        let p = AutomationPrefs::default();
        assert_eq!((p.sends_per_minute, p.other_channels, p.other_channel_gap_seconds, p.chat_notes, p.whisper_people_per_day), (10, false, 30, true, 20));
        let wild = AutomationPrefs { sends_per_minute: 0, other_channel_gap_seconds: 5000, whisper_people_per_day: 99, ..AutomationPrefs::default() }.clamped();
        assert_eq!((wild.sends_per_minute, wild.other_channel_gap_seconds, wild.whisper_people_per_day), (1, 600, 40));
        let read: AutomationPrefs = serde_json::from_value(json!({ "sends_per_minute": 4, "later_pref": true })).unwrap();
        assert_eq!(read.sends_per_minute, 4);
        assert!(read.chat_notes);
        assert_eq!(serde_json::to_value(&read).unwrap()["later_pref"], true);
    }

    fn stored_rule() -> Value {
        json!({
            "id": "r-000000000001",
            "name": "Old",
            "enabled": true,
            "live": true,
            "owner": { "provider": "twitch", "id": "100", "login": "me", "name": "Me" },
            "send_as": { "provider": "twitch", "id": "100", "login": "me", "name": "Me" },
            "when": { "kind": "mention", "replies_too": true },
            "only_if": {
                "channels": { "kind": "own" },
                "roles": [], "skip_users": [], "only_users": [],
                "only_live": false, "only_away": true, "shared_chat": false,
                "hours": { "from": "22:00", "to": "06:00", "days": [] }
            },
            "actions": [
                {
                    "kind": "reply", "id": "a-00000001", "pick": "random", "thread": true, "future_flag": 7,
                    "texts": [
                        { "id": "l-00000001", "text": "hi", "tone": "warm" },
                        { "id": "l-00000002", "text": "hey", "tone": "cool" }
                    ]
                },
                {
                    "kind": "send", "id": "a-00000002", "pick": "random", "texts": [{ "id": "l-00000003", "text": "bye" }],
                    "to": { "id": "c-00000001", "key": "twitch:friend", "room_id": "9", "name": "friend", "pinned": true }
                },
                { "kind": "notify", "id": "a-00000003", "title": "Ping", "body": "", "toast": true, "system": false, "sound": null, "volume": 1.0, "glow": "red" }
            ],
            "limits": { "rule_cooldown_seconds": 30, "user_cooldown_seconds": 300, "max_per_hour": 30, "max_wait_seconds": 10, "burst_guard": 3 },
            "vary": false,
            "origin": null,
            "created_ms": 1,
            "updated_ms": 1
        })
    }

    #[test]
    fn unknown_fields_survive_a_rule_edit() {
        let stored = stored_rule();
        let mut edited = Rule::from_value(&stored).unwrap();
        edited.name = "New".into();
        if let Action::Reply { texts, .. } = &mut edited.actions[0] {
            texts[0].text = "hello".into();
        }
        let merged = merge_raw(&stored, &edited.to_value());
        assert_eq!(merged["name"], "New");
        assert_eq!(merged["limits"]["burst_guard"], 3);
        assert_eq!(merged["actions"][0]["future_flag"], 7);
        assert_eq!(merged["actions"][0]["texts"][0]["text"], "hello");
        assert_eq!(merged["actions"][0]["texts"][0]["tone"], "warm");
        assert_eq!(merged["actions"][1]["to"]["pinned"], true);
        assert_eq!(merged["actions"][2]["glow"], "red");
        let again = merge_raw(&merged, &Rule::from_value(&merged).unwrap().to_value());
        assert_eq!(again, merged, "a second save changes nothing");
    }

    #[test]
    fn a_null_clears_hours_and_the_send_target() {
        let stored = stored_rule();
        let mut edited = Rule::from_value(&stored).unwrap();
        edited.only_if.hours = None;
        if let Action::Send { to, .. } = &mut edited.actions[1] {
            *to = None;
        }
        let merged = merge_raw(&stored, &edited.to_value());
        assert!(merged["only_if"]["hours"].is_null());
        assert!(merged["actions"][1]["to"].is_null());
        assert_eq!(merged["only_if"]["only_away"], true);
    }

    #[test]
    fn list_elements_merge_by_id_through_a_reorder_and_a_delete() {
        let stored = stored_rule();
        let mut edited = Rule::from_value(&stored).unwrap();
        edited.actions.swap(0, 2);
        if let Action::Reply { texts, .. } = &mut edited.actions[2] {
            texts.remove(0);
        }
        let merged = merge_raw(&stored, &edited.to_value());
        assert_eq!(merged["actions"][0]["id"], "a-00000003");
        assert_eq!(merged["actions"][0]["glow"], "red");
        assert_eq!(merged["actions"][2]["id"], "a-00000001");
        assert_eq!(merged["actions"][2]["future_flag"], 7);
        let texts = merged["actions"][2]["texts"].as_array().unwrap();
        assert_eq!(texts.len(), 1);
        assert_eq!(texts[0]["id"], "l-00000002");
        assert_eq!(texts[0]["tone"], "cool", "the kept line keeps its own field, never its neighbor's");
    }

    #[test]
    fn a_changed_kind_leaves_nothing_of_the_old_kind() {
        let stored = stored_rule();
        let mut edited = Rule::from_value(&stored).unwrap();
        edited.actions[0] = Action::Send { id: "a-00000001".into(), to: None, texts: vec![line("now a send")], pick: Pick::Random };
        let merged = merge_raw(&stored, &edited.to_value());
        let action = merged["actions"][0].as_object().unwrap();
        assert_eq!(action["kind"], "send");
        assert!(!action.contains_key("thread"));
        assert!(!action.contains_key("future_flag"));
        assert_eq!(merged["actions"][0]["texts"][0]["text"], "now a send");
    }

    #[test]
    fn lists_without_ids_merge_by_index_only_when_the_kind_matches() {
        let merged = merge_raw(&json!([{ "kind": "x", "a": 1 }, { "kind": "y", "b": 2 }]), &json!([{ "kind": "x", "c": 3 }, { "kind": "z" }]));
        assert_eq!(merged, json!([{ "kind": "x", "a": 1, "c": 3 }, { "kind": "z" }]));
        assert_eq!(merge_raw(&json!(["a", "b"]), &json!(["c"])), json!(["c"]));
        assert_eq!(merge_raw(&json!({ "a": 1 }), &json!(5)), json!(5));
    }

    #[test]
    fn ids_are_made_in_rust_for_elements_without_one() {
        let mut r = rule_with(reply("hi"));
        r.actions.push(Action::Reply { id: String::new(), texts: vec![line("a"), line("b")], pick: Pick::Shuffle, thread: false });
        r.actions.push(notify("dup"));
        r.only_if.channels = ChannelScope::These { list: vec![chat("one"), chat("two")] };
        fill_ids(&mut r);
        let hex = |s: &str, prefix: &str, len: usize| {
            s.starts_with(prefix) && s.len() == prefix.len() + len && s[prefix.len()..].chars().all(|c| c.is_ascii_hexdigit())
        };
        assert!(hex(&r.id, "r-", 12), "{}", r.id);
        assert!(r.actions.iter().all(|a| hex(a.id(), "a-", 8)));
        assert!(r.actions.iter().flat_map(|a| a.lines()).all(|l| hex(&l.id, "l-", 8)));
        let ChannelScope::These { list } = &r.only_if.channels else { unreachable!() };
        assert!(list.iter().all(|c| hex(&c.id, "c-", 8)));
        // A repeated id in one list gets a fresh one.
        let first = r.actions[0].id().to_string();
        *r.actions[1].id_mut() = first.clone();
        fill_ids(&mut r);
        assert_eq!(r.actions[0].id(), first);
        assert_ne!(r.actions[1].id(), first);
    }

    #[test]
    fn a_good_rule_saves_with_no_errors() {
        let mut r = rule_with(reply("@{user} I'm away right now: {away}"));
        assert!(validate(&mut r, &ValidateCtx::default()).is_empty());
        assert_eq!(r.send_as, me());
    }

    #[test]
    fn chatter_words_go_out_only_in_your_own_channel_for_you_and_your_mods() {
        let ctx = ValidateCtx::default();
        let at = "actions[0].texts[0].text";
        let mut anyone = rule_with(reply("You said {message}"));
        assert!(has(&validate(&mut anyone, &ctx), at), "from anyone");
        let mut mods_elsewhere = rule_with(reply("You said {message}"));
        mods_elsewhere.only_if.roles = vec![SenderRole::Moderator];
        mods_elsewhere.only_if.channels = ChannelScope::These { list: vec![chat("xqc")] };
        assert!(has(&validate(&mut mods_elsewhere, &ctx), at), "from moderators in chosen chats");
        let mut mods_at_home = rule_with(reply("You said {message}"));
        mods_at_home.only_if.roles = vec![SenderRole::Moderator, SenderRole::Broadcaster];
        assert!(validate(&mut mods_at_home, &ctx).is_empty(), "from moderators in your own channel");
        let mut vips_at_home = rule_with(reply("{1} {2+} {*}"));
        vips_at_home.only_if.roles = vec![SenderRole::Moderator, SenderRole::Vip];
        assert!(has(&validate(&mut vips_at_home, &ctx), at), "arguments are the chatter's words too");
        let mut title_anywhere = rule_with(reply("Now playing {game}: {title}"));
        title_anywhere.only_if.channels = ChannelScope::AnyOpen {};
        assert!(has(&validate(&mut title_anywhere, &ctx), at));
        let mut title_at_home = rule_with(reply("Now playing {game}: {title}"));
        assert!(validate(&mut title_at_home, &ctx).is_empty());
        let mut alert = rule_with(notify("{user} said {message} during {title}"));
        alert.only_if.channels = ChannelScope::AnyOpen {};
        assert!(validate(&mut alert, &ctx).is_empty(), "a notice only you see always saves");
        let mut literal = rule_with(reply("Type {{message}} to see it"));
        assert!(validate(&mut literal, &ctx).is_empty(), "doubled braces are literal");
    }

    #[test]
    fn badge_rules_are_forced_and_badge_words_stay_in_them() {
        let ctx = ValidateCtx::default();
        let mut badge = Rule {
            when: Trigger::BadgeDrop { on: BadgeLeg::Both, link: BadgeLink::default(), batch_text: "{badge.count} new: {badges}".into() },
            actions: vec![Action::Send { id: String::new(), to: None, texts: vec![line("New badge: {badge} {badge.link}")], pick: Pick::Random }],
            ..rule_with(notify("x"))
        };
        badge.only_if.channels = ChannelScope::AnyOpen {};
        badge.limits.rule_cooldown_seconds = 30;
        assert!(validate(&mut badge, &ctx).is_empty());
        assert_eq!(badge.only_if.channels, ChannelScope::Own {});
        assert!(badge.only_if.only_live);
        assert_eq!(badge.limits.rule_cooldown_seconds, 0);
        let mut replying = Rule { actions: vec![reply("new badge")], ..badge.clone() };
        assert!(has(&validate(&mut replying, &ctx), "actions[0]"));
        let mut chat_rule = rule_with(reply("{badge}"));
        assert!(has(&validate(&mut chat_rule, &ctx), "actions[0].texts[0].text"));
        let mut chat_notice = rule_with(notify("{badge.how}"));
        assert!(has(&validate(&mut chat_notice, &ctx), "actions[0].title"));
        let mut custom = badge.clone();
        custom.when = Trigger::BadgeDrop { on: BadgeLeg::Both, link: BadgeLink { preset: LinkPreset::Custom, custom: "http://x".into(), if_not_ready: LinkFallback::NoLink }, batch_text: String::new() };
        assert!(has(&validate(&mut custom, &ctx), "when.link.custom"));
    }

    #[test]
    fn caps_are_field_errors() {
        let ctx = ValidateCtx::default();
        let mut r = rule_with(reply("hi"));
        assert!(has(&validate(&mut r, &ValidateCtx { other_rules: MAX_RULES, ..ValidateCtx::default() }), "rules"));
        r.actions = (0..6).map(|_| notify("x")).collect();
        assert!(has(&validate(&mut r, &ctx), "actions"));
        r.actions.clear();
        assert!(has(&validate(&mut r, &ctx), "actions"));
        r.actions = vec![Action::Reply { id: String::new(), texts: (0..21).map(|i| line(&format!("line {i}"))).collect(), pick: Pick::Random, thread: false }];
        assert!(has(&validate(&mut r, &ctx), "actions[0].texts"));
        for bad in ["x".repeat(501), "/me hi".to_string(), ".hi".to_string(), "   ".to_string()] {
            let mut one = rule_with(reply(&bad));
            assert!(has(&validate(&mut one, &ctx), "actions[0].texts[0].text"), "{bad:?}");
        }
        let mut many = rule_with(reply("hi"));
        many.only_if.channels = ChannelScope::These { list: (0..26).map(|i| chat(&format!("c{i}"))).collect() };
        assert!(has(&validate(&mut many, &ctx), "only_if.channels"));
        let mut full = rule_with(reply("hi"));
        let others = ValidateCtx { other_chats: (0..50).map(|i| format!("twitch:c{i}")).collect(), ..ValidateCtx::default() };
        assert!(has(&validate(&mut full, &others), "only_if.channels"), "the 51st chat");
        let mut fits = rule_with(reply("hi"));
        let fits_ctx = ValidateCtx { other_chats: (0..49).map(|i| format!("twitch:c{i}")).collect(), ..ValidateCtx::default() };
        assert!(validate(&mut fits, &fits_ctx).is_empty(), "the 50th chat");
        let mut limits = rule_with(reply("hi"));
        limits.limits = Limits { rule_cooldown_seconds: 0, user_cooldown_seconds: MAX_COOLDOWN_SECONDS + 1, max_per_hour: 0, max_wait_seconds: 131 };
        let errors = validate(&mut limits, &ctx);
        for path in ["limits.user_cooldown_seconds", "limits.max_per_hour", "limits.max_wait_seconds"] {
            assert!(has(&errors, path), "{path}");
        }
        limits.limits = Limits { rule_cooldown_seconds: MAX_COOLDOWN_SECONDS, user_cooldown_seconds: 0, max_per_hour: 121, max_wait_seconds: 130 };
        let errors = validate(&mut limits, &ctx);
        assert!(has(&errors, "limits.max_per_hour") && !has(&errors, "limits.rule_cooldown_seconds"));
        let mut named = rule_with(reply("hi"));
        named.name = "n".repeat(61);
        assert!(has(&validate(&mut named, &ctx), "name"));
        named.name = "   ".into();
        assert!(has(&validate(&mut named, &ctx), "name"));
        let mut hours = rule_with(reply("hi"));
        hours.only_if.hours = Some(Hours { from: "9:30".into(), to: "24:00".into(), days: vec![Weekday::Mon, Weekday::Mon] });
        let errors = validate(&mut hours, &ctx);
        assert!(has(&errors, "only_if.hours.from") && has(&errors, "only_if.hours.to"));
        assert_eq!(hours.only_if.hours.unwrap().days, vec![Weekday::Mon]);
    }

    #[test]
    fn message_patterns_are_checked_and_kept_as_typed() {
        let ctx = ValidateCtx::default();
        let with = |mode: MatchMode, pattern: &str| Rule {
            when: Trigger::Message { text: TextMatch { mode, pattern: pattern.into(), case_sensitive: false } },
            ..rule_with(reply("hi"))
        };
        assert!(validate(&mut with(MatchMode::Contains, &"a".repeat(200)), &ctx).is_empty());
        assert!(has(&validate(&mut with(MatchMode::Contains, &"a".repeat(201)), &ctx), "when.text.pattern"));
        assert!(has(&validate(&mut with(MatchMode::Contains, "  "), &ctx), "when.text.pattern"));
        assert!(has(&validate(&mut with(MatchMode::Regex, "("), &ctx), "when.text.pattern"));
        assert!(has(&validate(&mut with(MatchMode::Regex, "\\w{1000}"), &ctx), "when.text.pattern"), "past the 64 KiB size limit");
        let mut kept = with(MatchMode::Regex, "Hello|World");
        assert!(validate(&mut kept, &ctx).is_empty());
        assert_eq!(kept.when, Trigger::Message { text: TextMatch { mode: MatchMode::Regex, pattern: "Hello|World".into(), case_sensitive: false } });
        let mut events = Rule { when: Trigger::ChannelEvent { events: vec![], min_months: 0, min_gifts: 0, min_viewers: 0, min_bits: 0 }, ..rule_with(reply("hi")) };
        assert!(has(&validate(&mut events, &ctx), "when.events"));
    }

    #[test]
    fn chats_and_people_are_tidied() {
        let mut r = rule_with(reply("hi"));
        r.only_if.channels = ChannelScope::These { list: vec![chat("#XQC"), ChannelRef { key: String::new(), name: "Lirik".into(), ..ChannelRef::default() }] };
        r.only_if.skip_users = vec!["@Nightbot".into(), "nightbot".into(), " ".into()];
        assert!(validate(&mut r, &ValidateCtx::default()).is_empty());
        let ChannelScope::These { list } = &r.only_if.channels else { unreachable!() };
        assert_eq!(list.iter().map(|c| c.key.as_str()).collect::<Vec<_>>(), vec!["twitch:xqc", "twitch:lirik"]);
        assert_eq!(r.only_if.skip_users, vec!["nightbot".to_string()]);
        assert_eq!(listened_chats(&r), vec!["twitch:xqc".to_string(), "twitch:lirik".to_string()]);
        let own = rule_with(reply("hi"));
        assert_eq!(listened_chats(&own), vec!["twitch:me".to_string()]);
    }

    #[test]
    fn placeholders_follow_the_custom_command_grammar() {
        assert_eq!(
            placeholders("{user} {{message}} {1} {2+} {*} {badge.link} {not a name} {} {Title}"),
            vec!["user", "1", "2+", "*", "badge.link", "Title"]
        );
        assert!(is_chatter_text("message") && is_chatter_text("12") && is_chatter_text("3+") && is_chatter_text("*"));
        assert!(!is_chatter_text("user") && !is_chatter_text("+"));
        assert!(is_channel_state("Title") && is_channel_state("game"));
        assert!(is_badge_variable("badges") && is_badge_variable("badge.how") && !is_badge_variable("badger"));
        assert_eq!(parse_hhmm("09:30"), Some(570));
        assert_eq!(parse_hhmm("23:59"), Some(1439));
        assert_eq!(parse_hhmm("24:00"), None);
    }
}
