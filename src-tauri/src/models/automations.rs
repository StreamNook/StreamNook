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
/// The longest text a Speak action reads aloud.
pub const MAX_SPEAK_CHARS: usize = 300;

/// "#rrggbb" (what the color swatches write).
pub fn is_hex_color(text: &str) -> bool {
    text.len() == 7 && text.starts_with('#') && text[1..].chars().all(|c| c.is_ascii_hexdigit())
}
/// The fewest actions an hour a rule may be held to.
pub const MIN_PER_HOUR: u32 = 1;
/// The most actions an hour a rule may take.
pub const MAX_PER_HOUR: u32 = 120;
/// The longest a rule waits for a send slot.
pub const MAX_WAIT_SECONDS: u32 = 130;
/// The tray's "Set away" text when no away message was ever written.
pub const DEFAULT_AWAY_MESSAGE: &str = "Back soon";
/// A Timer's interval, in seconds (5 minutes to 12 hours).
pub const TIMER_MIN_SECONDS: u32 = 300;
pub const TIMER_MAX_SECONDS: u32 = 43_200;
/// Uptime and Delay seconds.
pub const TIMED_MAX_SECONDS: u32 = 172_800;
/// A Timeout's length (never a permanent ban).
pub const TIMEOUT_MAX_SECONDS: u32 = 1_209_600;
/// A Pin's length when it has one (Twitch's range).
pub const PIN_MIN_SECONDS: u32 = 30;
pub const PIN_MAX_SECONDS: u32 = 1800;
/// A Wait's length.
pub const WAIT_MAX_SECONDS: u32 = 300;
/// Characters in a Timeout reason and an Announce.
pub const MAX_REASON_CHARS: usize = 500;
/// Chats a rule may keep joined with JoinChat, counted in the 50 cap.
pub const MAX_ANNOUNCE_CHARS: usize = 500;

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
    /// People a rule may whisper in a day (1 to 30, so your own whispers keep room under Twitch's 40).
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
            whisper_people_per_day: self.whisper_people_per_day.clamp(1, 30),
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
    /// A whisper to the owner from someone you have whispered with before.
    Whisper {
        #[serde(default)]
        text: Option<TextMatch>,
    },
    /// Every `every_seconds` (300 to 43,200) per chat, skipped below
    /// `min_messages` other people's lines since its last run.
    Timer {
        #[serde(default = "default_timer_seconds")]
        every_seconds: u32,
        #[serde(default)]
        min_messages: u32,
    },
    /// Local time `at` ("HH:MM") on `days` (empty = every day), once per date.
    Clock {
        #[serde(default)]
        at: String,
        #[serde(default)]
        days: Vec<Weekday>,
    },
    /// Once per broadcast at `seconds` of uptime; `late_ok` (set by the
    /// reminder convert) also fires any time after it, once per broadcast.
    Uptime {
        #[serde(default)]
        seconds: u32,
        #[serde(default)]
        late_ok: bool,
    },
    /// Once, `seconds` after a window joined the chat.
    Delay {
        #[serde(default)]
        seconds: u32,
    },
    /// P3: someone follows the owner's channel (`channel.follow` v2).
    Follow {},
    /// P3: a channel point reward is redeemed in the owner's channel;
    /// `reward_id` None means any reward.
    RewardRedeemed {
        #[serde(default)]
        reward_id: Option<String>,
    },
    /// P3: the owner starts a poll (`channel.poll.begin`).
    PollStarted {},
    /// P3: the owner starts a prediction (`channel.prediction.begin`).
    PredictionStarted {},
    /// P3: a channel in the rule's `These` list goes live (`online`) or offline.
    LiveChange {
        #[serde(default = "yes")]
        online: bool,
    },
}

fn default_timer_seconds() -> u32 {
    900
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
    /// YouTube.
    SuperChat,
    SuperSticker,
    Membership,
    MemberGift,
    /// TikTok.
    #[serde(rename = "tiktok_gift")]
    TikTokGift,
    #[serde(rename = "tiktok_follow")]
    TikTokFollow,
    #[serde(rename = "tiktok_share")]
    TikTokShare,
}

impl EventKind {
    /// Whether this event happens on `platform` (Kick: subs and gifts).
    pub fn allowed_on(self, platform: Platform) -> bool {
        use EventKind::*;
        match platform {
            Platform::Twitch => matches!(self, Sub | Resub | Gift | GiftBomb | Raid | Cheer | Announcement),
            Platform::Kick => matches!(self, Sub | Resub | Gift | GiftBomb),
            Platform::Youtube => matches!(self, SuperChat | SuperSticker | Membership | MemberGift),
            Platform::Tiktok => matches!(self, TikTokGift | TikTokFollow | TikTokShare),
        }
    }
}

/// A chat platform a rule listens on. A rule that posts names exactly one,
/// and only Twitch and Kick can be posted to (YouTube and TikTok rules
/// notify, log or speak).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    Twitch,
    Kick,
    Youtube,
    Tiktok,
}

impl Platform {
    /// The provider id the rest of the app uses.
    pub fn as_str(self) -> &'static str {
        match self {
            Platform::Twitch => "twitch",
            Platform::Kick => "kick",
            Platform::Youtube => "youtube",
            Platform::Tiktok => "tiktok",
        }
    }

    /// The name in the app's words.
    pub fn label(self) -> &'static str {
        match self {
            Platform::Twitch => "Twitch",
            Platform::Kick => "Kick",
            Platform::Youtube => "YouTube",
            Platform::Tiktok => "TikTok",
        }
    }

    pub fn parse(provider: &str) -> Option<Platform> {
        match provider.trim().to_ascii_lowercase().as_str() {
            "" | "twitch" => Some(Platform::Twitch),
            "kick" => Some(Platform::Kick),
            "youtube" => Some(Platform::Youtube),
            "tiktok" => Some(Platform::Tiktok),
            _ => None,
        }
    }

    /// Whether a rule can post there.
    pub fn can_post(self) -> bool {
        matches!(self, Platform::Twitch | Platform::Kick)
    }
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

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(default)]
pub struct Conditions {
    /// The platforms the rule listens on; Twitch alone by default. A rule
    /// with a posting action names exactly one.
    pub platforms: Vec<Platform>,
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
    /// The saved-filter language (`ChatRules::validate_filter`), chat rows only.
    pub expression: Option<String>,
    /// Post only in a chat a window holds, or that the stream warm-up joined
    /// (`IrcService::open_for_reminders`, no adopted set), and never join a
    /// chat for the rule. Offered for every rule; "Make it a rule" sets it.
    pub only_while_open: bool,
    /// Set only by "Make it a rule" for a "mods" keyword reminder: a lead
    /// moderator does not pass the From check, as the reminder reads it.
    /// Backend-owned: a save keeps the stored value.
    pub lead_moderator_excluded: bool,
    /// Set only by "Make it a rule" for an uptime reminder pinned to a chat:
    /// the chat must also be the one being watched, as the reminder requires
    /// (D293: a converted-rule-only condition; the editor never offers it).
    /// Backend-owned: a save keeps the stored value.
    pub only_while_watching: bool,
}

impl Default for Conditions {
    fn default() -> Self {
        Conditions {
            platforms: vec![Platform::Twitch],
            channels: ChannelScope::default(),
            roles: Vec::new(),
            skip_users: Vec::new(),
            only_users: Vec::new(),
            only_live: false,
            only_away: false,
            shared_chat: false,
            hours: None,
            expression: None,
            only_while_open: false,
            lead_moderator_excluded: false,
            only_while_watching: false,
        }
    }
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
    /// One typed Helix command (`command::parse_command`); the stored line
    /// is canonical (alias resolved).
    Command {
        #[serde(default)]
        id: String,
        #[serde(default)]
        line: String,
    },
    /// A whisper to `to` (a person you have whispered with, by id); on a
    /// Whisper trigger an empty `to` is the sender (the Reply form).
    Whisper {
        #[serde(default)]
        id: String,
        #[serde(default)]
        to: Option<WhisperTarget>,
        #[serde(default)]
        texts: Vec<Line>,
        #[serde(default)]
        pick: Pick,
    },
    Timeout {
        #[serde(default)]
        id: String,
        #[serde(default)]
        seconds: u32,
        #[serde(default)]
        reason: String,
    },
    Delete {
        #[serde(default)]
        id: String,
    },
    Pin {
        #[serde(default)]
        id: String,
        /// None pins until the stream ends.
        #[serde(default)]
        seconds: Option<u32>,
    },
    Announce {
        #[serde(default)]
        id: String,
        #[serde(default)]
        text: String,
        /// "primary", "blue", "green", "orange" or "purple".
        #[serde(default = "primary_color")]
        color: String,
    },
    Shoutout {
        #[serde(default)]
        id: String,
    },
    JoinChat {
        #[serde(default)]
        id: String,
        #[serde(default)]
        channel: ChannelRef,
    },
    LeaveChat {
        #[serde(default)]
        id: String,
        #[serde(default)]
        channel: ChannelRef,
    },
    Wait {
        #[serde(default)]
        id: String,
        #[serde(default)]
        seconds: u32,
    },
    Log {
        #[serde(default)]
        id: String,
        #[serde(default)]
        to_file: bool,
    },
    /// P3: read `text` aloud on this device (one player speaks). A local
    /// action: no claim, runs in Test and Live, never posts.
    Speak {
        #[serde(default)]
        id: String,
        #[serde(default)]
        text: String,
        /// Also while streamer mode is on.
        #[serde(default)]
        on_stream: bool,
    },
    /// P3: tint the matches of a Message rule in chat. Not run per event:
    /// compile hands it to `chat_rules` (`AutoHighlight`).
    Highlight {
        #[serde(default)]
        id: String,
        /// "#rrggbb".
        #[serde(default)]
        color: String,
    },
}

fn primary_color() -> String {
    "primary".to_string()
}

/// A person a Whisper action writes to.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(default)]
pub struct WhisperTarget {
    pub user_id: String,
    pub login: String,
    pub name: String,
}

impl Action {
    pub fn id(&self) -> &str {
        match self {
            Action::Reply { id, .. }
            | Action::Send { id, .. }
            | Action::Notify { id, .. }
            | Action::Command { id, .. }
            | Action::Whisper { id, .. }
            | Action::Timeout { id, .. }
            | Action::Delete { id }
            | Action::Pin { id, .. }
            | Action::Announce { id, .. }
            | Action::Shoutout { id }
            | Action::JoinChat { id, .. }
            | Action::LeaveChat { id, .. }
            | Action::Wait { id, .. }
            | Action::Log { id, .. }
            | Action::Speak { id, .. }
            | Action::Highlight { id, .. } => id,
        }
    }

    fn id_mut(&mut self) -> &mut String {
        match self {
            Action::Reply { id, .. }
            | Action::Send { id, .. }
            | Action::Notify { id, .. }
            | Action::Command { id, .. }
            | Action::Whisper { id, .. }
            | Action::Timeout { id, .. }
            | Action::Delete { id }
            | Action::Pin { id, .. }
            | Action::Announce { id, .. }
            | Action::Shoutout { id }
            | Action::JoinChat { id, .. }
            | Action::LeaveChat { id, .. }
            | Action::Wait { id, .. }
            | Action::Log { id, .. }
            | Action::Speak { id, .. }
            | Action::Highlight { id, .. } => id,
        }
    }

    /// Whether the action acts outside this device (and so claims). Notify,
    /// Log, JoinChat, LeaveChat, Wait, Speak and Highlight are local or
    /// control.
    pub fn is_outward(&self) -> bool {
        !matches!(
            self,
            Action::Notify { .. }
                | Action::Log { .. }
                | Action::JoinChat { .. }
                | Action::LeaveChat { .. }
                | Action::Wait { .. }
                | Action::Speak { .. }
                | Action::Highlight { .. }
        )
    }

    /// Whether the action is a control or local step that never claims.
    pub fn is_control(&self) -> bool {
        !self.is_outward()
    }

    /// The lines a Reply, Send or Whisper picks from; none otherwise.
    pub fn lines(&self) -> &[Line] {
        match self {
            Action::Reply { texts, .. } | Action::Send { texts, .. } | Action::Whisper { texts, .. } => texts,
            _ => &[],
        }
    }

    fn lines_mut(&mut self) -> Option<&mut Vec<Line>> {
        match self {
            Action::Reply { texts, .. } | Action::Send { texts, .. } | Action::Whisper { texts, .. } => Some(texts),
            _ => None,
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

/// What "Make it a rule" stores on a rule so it claims exactly as the
/// reminder it came from, and collides with that reminder still running on
/// another device (PLAN § 4.8, D281). The elements themselves are rebuilt at
/// claim time from these parts by `reminder_claim::converted_elements`, the
/// same builder the reminder uses, so the two can never drift.
///
/// It applies only while the rule still has the shape it was converted with:
/// the same trigger kind, chat scope, Send target and owner. A text edit keeps
/// it; any of those changing clears it (a save drops it, and the claim path
/// re-checks, so a hand-edited file cannot keep it either).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(default)]
pub struct ClaimAs {
    /// The claim account at convert time (`ChatRules::own_identity()`'s id).
    pub account_id: String,
    /// `interval`, `delay`, `clock`, `uptime` or `keyword`.
    pub kind: String,
    /// 16 hex over the reminder's message and target.
    pub family_hash: String,
    /// The interval or delay in seconds the window derives from; 0 otherwise.
    pub seconds: u64,
    /// A keyword reminder's cooldown in seconds (its rail); 0 = none.
    pub cooldown_s: u32,
    /// The rule's trigger `kind` and chat scope at convert time.
    pub trigger_kind: String,
    pub scope_kind: String,
    /// `These`: the chats' keys joined by `,`; empty otherwise.
    pub scope_key: String,
    /// The first Send's chat key; None = the chat the rule fired in.
    pub send_to: Option<String>,
}

impl ClaimAs {
    /// The words `applies_to` compares: (trigger kind, scope kind, scope key,
    /// the first Send's chat key). The trigger kind is read through the rule's
    /// JSON so this compiles with whatever triggers a phase has added.
    pub fn shape_of(rule: &Rule) -> (String, String, String, Option<String>) {
        let trigger = serde_json::to_value(&rule.when)
            .ok()
            .and_then(|v| v.get("kind").and_then(Value::as_str).map(str::to_string))
            .unwrap_or_default();
        let (scope, key) = match &rule.only_if.channels {
            ChannelScope::Own {} => ("own", String::new()),
            ChannelScope::These { list } => ("these", list.iter().map(|c| c.key.as_str()).collect::<Vec<_>>().join(",")),
            ChannelScope::AnyOpen {} => ("any_open", String::new()),
            ChannelScope::Watching {} => ("watching", String::new()),
        };
        let send_to = rule
            .actions
            .iter()
            .find_map(|a| match a {
                Action::Send { to, .. } => Some(to.as_ref().map(|c| c.key.clone())),
                _ => None,
            })
            .flatten();
        (trigger, scope.to_string(), key, send_to)
    }

    /// Whether the rule still has the shape it was converted with.
    pub fn applies_to(&self, rule: &Rule) -> bool {
        let (trigger, scope, key, send_to) = ClaimAs::shape_of(rule);
        !self.account_id.is_empty()
            && self.account_id == rule.owner.id
            && self.trigger_kind == trigger
            && self.scope_kind == scope
            && self.scope_key == key
            && self.send_to == send_to
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
    /// Backend-owned: set only by "Make it a rule" (see [`ClaimAs`]); a client
    /// never writes it.
    pub claim_as: Option<ClaimAs>,
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
            claim_as: None,
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

    /// P3: fires from the owner's own-channel EventSub topics.
    pub fn is_own_event_rule(&self) -> bool {
        matches!(
            self.when,
            Trigger::Follow {} | Trigger::RewardRedeemed { .. } | Trigger::PollStarted {} | Trigger::PredictionStarted {}
        )
    }

    /// P3: fires when a watched channel goes live or offline.
    pub fn is_live_rule(&self) -> bool {
        matches!(self.when, Trigger::LiveChange { .. })
    }

    /// P3: fires from a source that is not a chat row. It joins no chat.
    pub fn is_edge_rule(&self) -> bool {
        self.is_own_event_rule() || self.is_live_rule()
    }

    /// Timer, Clock, Uptime and Delay: fired by time, not by a row.
    pub fn is_timed(&self) -> bool {
        matches!(self.when, Trigger::Timer { .. } | Trigger::Clock { .. } | Trigger::Uptime { .. } | Trigger::Delay { .. })
    }

    pub fn is_whisper_rule(&self) -> bool {
        matches!(self.when, Trigger::Whisper { .. })
    }

    /// A chat row starts it (Message, Mention, FirstMessage, ChannelEvent).
    pub fn is_chat_rule(&self) -> bool {
        !self.is_badge_rule() && !self.is_timed() && !self.is_whisper_rule() && !self.is_edge_rule()
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
    if n == "message" || n == "input" || n == "*" {
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
    if rule.is_edge_rule() {
        return Vec::new();
    }
    match &rule.only_if.channels {
        ChannelScope::Own {} if is_kick_sender(rule) && !rule.send_as.login.trim().is_empty() => {
            vec![composite_key(&format!("kick:{}", rule.send_as.login.trim()))]
        }
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

/// `provider:channel` for a chat, a bare name being Twitch. A YouTube id keeps
/// its case (it names one channel), every other name is lowercase: the same
/// key `providers::key::make_key` gives the chat bus.
pub fn composite_key(raw: &str) -> String {
    let parsed = crate::services::providers::key::parse_key(raw.trim().trim_start_matches('#'));
    crate::services::providers::key::make_key(&parsed.provider, &parsed.channel)
}

/// The provider part of a normalized chat key.
pub fn provider_of_key(key: &str) -> &str {
    key.split_once(':').map_or("twitch", |(provider, _)| provider)
}

/// The rule posts as a Kick account.
pub fn is_kick_sender(rule: &Rule) -> bool {
    rule.send_as.provider.trim().eq_ignore_ascii_case("kick")
}

fn kind_of(value: &impl Serialize) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.get("kind").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_default()
}

/// An action's `kind` word (`reply`, `send`, `notify`, and later phases'),
/// read from its serialized form so a platform check never names a variant
/// another phase owns.
pub fn action_kind(action: &Action) -> String {
    kind_of(action)
}

/// A trigger's `kind` word, as above.
pub fn trigger_kind(when: &Trigger) -> String {
    kind_of(when)
}

/// The timers: no stream state exists off Twitch for them to read.
pub fn timer_family(kind: &str) -> bool {
    matches!(kind, "timer" | "clock" | "uptime" | "delay")
}

/// What an action of `kind` may do on `platform`: Twitch everything; Kick
/// posts, notifies, logs, speaks, waits, highlights, times out and deletes;
/// YouTube and TikTok only notify, log and speak.
pub fn action_fits(platform: Platform, kind: &str) -> bool {
    match platform {
        Platform::Twitch => true,
        Platform::Kick => {
            matches!(kind, "reply" | "send" | "notify" | "log" | "speak" | "wait" | "highlight" | "timeout" | "delete")
        }
        Platform::Youtube | Platform::Tiktok => matches!(kind, "notify" | "log" | "speak"),
    }
}

const POST_ONE_PLATFORM: &str = "A rule that posts runs on one platform";
const NOTIFY_ONLY_WORDS: &str = "YouTube and TikTok rules can notify you, log or speak, not post";
const TWITCH_ONLY_ACTION: &str = "This action works on Twitch only";
const TWITCH_ONLY_TRIGGER: &str = "This trigger works on Twitch only";
const NO_TIMERS_WORDS: &str = "Timers need a Twitch stream, so they can't be used with other platforms";
const NO_LIVE_WORDS: &str = "Only Twitch tells StreamNook when a stream is live, so this can't be used with other platforms";
const NO_CHANNEL_WORDS: &str = "{title}, {game} and {uptime} come from Twitch, so they can't be used with other platforms";
const WATCHING_WORDS: &str = "The stream you're watching is a Twitch stream";
const OWNER_WORDS: &str = "Sign in with Twitch to use automations";
const WRONG_EVENT_WORDS: &str = "That event doesn't happen on the platforms you picked";
const WRONG_CHAT_WORDS: &str = "That channel is on a platform this rule doesn't use";
const WRONG_SENDER_WORDS: &str = "Send as the account on this platform";

fn platform_text(path: &str, text: &str, errors: &mut Errors) {
    if placeholders(text).iter().any(|n| is_channel_state(n) || n.eq_ignore_ascii_case("uptime")) {
        errors.push(path, NO_CHANNEL_WORDS);
    }
}

/// The checks the platforms add: one platform when the rule posts, who it
/// sends as, what each platform can do, and the Twitch-only fields.
fn check_platforms(rule: &Rule, errors: &mut Errors) {
    let platforms = &rule.only_if.platforms;
    let other = platforms.iter().any(|p| *p != Platform::Twitch);
    let posts = rule.actions.iter().any(Action::is_outward);
    if !rule.owner.provider.trim().eq_ignore_ascii_case("twitch") {
        errors.push("owner", OWNER_WORDS);
    }
    if posts && platforms.len() > 1 {
        errors.push("only_if.platforms", POST_ONE_PLATFORM);
    }
    if let (true, [platform]) = (posts, platforms.as_slice()) {
        if !rule.send_as.provider.trim().eq_ignore_ascii_case(platform.as_str()) {
            errors.push("send_as", WRONG_SENDER_WORDS);
        }
    }
    for (i, action) in rule.actions.iter().enumerate() {
        let kind = action_kind(action);
        if let Some(bad) = platforms.iter().find(|p| !action_fits(**p, &kind)) {
            let words = if matches!(bad, Platform::Youtube | Platform::Tiktok) { NOTIFY_ONLY_WORDS } else { TWITCH_ONLY_ACTION };
            errors.push(format!("actions[{i}]"), words);
        }
        if let Action::Send { to: Some(chat), .. } = action {
            if !chat.key.trim().is_empty() && !platforms.iter().any(|p| p.as_str() == provider_of_key(&chat.key)) {
                errors.push(format!("actions[{i}].to"), WRONG_CHAT_WORDS);
            }
        }
    }
    if let Trigger::ChannelEvent { events, .. } = &rule.when {
        if events.iter().any(|kind| !platforms.iter().any(|p| kind.allowed_on(*p))) {
            errors.push("when.events", WRONG_EVENT_WORDS);
        }
    }
    if let ChannelScope::These { list } = &rule.only_if.channels {
        for (i, chat) in list.iter().enumerate() {
            if !chat.key.trim().is_empty() && !platforms.iter().any(|p| p.as_str() == provider_of_key(&chat.key)) {
                errors.push(format!("only_if.channels.list[{i}]"), WRONG_CHAT_WORDS);
            }
        }
    }
    if !other {
        return;
    }
    let when = trigger_kind(&rule.when);
    if timer_family(&when) {
        errors.push("when", NO_TIMERS_WORDS);
    } else if !matches!(when.as_str(), "message" | "mention" | "first_message" | "channel_event") {
        errors.push("when", TWITCH_ONLY_TRIGGER);
    }
    if rule.only_if.only_live {
        errors.push("only_if.only_live", NO_LIVE_WORDS);
    }
    if matches!(rule.only_if.channels, ChannelScope::Watching {}) && !platforms.contains(&Platform::Twitch) {
        errors.push("only_if.channels", WATCHING_WORDS);
    }
    for (i, action) in rule.actions.iter().enumerate() {
        match action {
            Action::Notify { title, body, .. } => {
                platform_text(&format!("actions[{i}].title"), title, errors);
                platform_text(&format!("actions[{i}].body"), body, errors);
            }
            other => {
                for (j, line) in other.lines().iter().enumerate() {
                    platform_text(&format!("actions[{i}].texts[{j}].text"), &line.text, errors);
                }
            }
        }
    }
}

fn normalize_platforms(platforms: &mut Vec<Platform>) {
    let mut seen = HashSet::new();
    platforms.retain(|p| seen.insert(*p));
    if platforms.is_empty() {
        platforms.push(Platform::Twitch);
    }
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
    chat.key = if raw.is_empty() { String::new() } else { composite_key(raw) };
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
    normalize_platforms(&mut rule.only_if.platforms);
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
    // A Whisper rule has no chat: its scope reads as your channel (the
    // owner's), and its expression is cleared.
    if rule.is_whisper_rule() {
        rule.only_if.channels = ChannelScope::Own {};
    }
    if !rule.is_chat_rule() {
        rule.only_if.expression = None;
    }
    // Shoutout is forced only-live (Twitch needs the channel live).
    if rule.actions.iter().any(|a| matches!(a, Action::Shoutout { .. })) {
        rule.only_if.only_live = true;
    }
    if let Some(e) = &rule.only_if.expression {
        if e.trim().is_empty() {
            rule.only_if.expression = None;
        }
    }
    for action in rule.actions.iter_mut() {
        match action {
            Action::Command { line, .. } => {
                if let Ok(canonical) = crate::services::account_automations::command::canonical_line(line) {
                    *line = canonical;
                }
            }
            Action::Announce { color, .. } => {
                *color = color.trim().to_ascii_lowercase();
                if color.is_empty() {
                    *color = "primary".into();
                }
            }
            Action::JoinChat { channel, .. } | Action::LeaveChat { channel, .. } => normalize_chat(channel),
            Action::Whisper { to: Some(target), .. } => target.login = target.login.trim().trim_start_matches('@').to_ascii_lowercase(),
            Action::Speak { text, .. } => *text = text.trim().to_string(),
            Action::Highlight { color, .. } => *color = color.trim().to_ascii_lowercase(),
            _ => {}
        }
    }
    // P3: an own-channel event happens in your channel, to nobody with a role.
    if rule.is_own_event_rule() {
        rule.only_if.channels = ChannelScope::Own {};
        rule.only_if.roles.clear();
        rule.only_if.shared_chat = false;
    }
    // P3: a live change watches the channels the rule names, and nothing about people.
    if rule.is_live_rule() {
        if !matches!(rule.only_if.channels, ChannelScope::These { .. }) {
            rule.only_if.channels = ChannelScope::These { list: Vec::new() };
        }
        rule.only_if.roles.clear();
        rule.only_if.skip_users.clear();
        rule.only_if.only_users.clear();
        rule.only_if.only_live = false;
        rule.only_if.shared_chat = false;
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

const TIMER_WORDS: &str = "A timed rule needs a chat: pick your channel, these channels or the stream you're watching";
const NO_USER_WORDS: &str = "A timed rule has no {user}: nobody started it";
const WHISPER_RULE_WORDS: &str = "A whisper rule can reply, whisper, post to a chat you pick, notify you, log or wait";
const CHAT_ONLY_WORDS: &str = "This works only on a chat message";

/// P2 trigger checks (called from `validate` after the Message check).
fn check_p2_trigger(rule: &Rule, errors: &mut Errors) {
    match &rule.when {
        Trigger::Whisper { text } => {
            if let Some(text) = text {
                if !text.pattern.trim().is_empty() {
                    let mut probe = text.clone();
                    probe.pattern = probe.pattern.trim().to_string();
                    let mut inner = Errors(Vec::new());
                    check_pattern(&probe, &mut inner);
                    for e in inner.0 {
                        errors.push("when.text.pattern", &e.message);
                    }
                }
            }
        }
        Trigger::Timer { every_seconds, min_messages } => {
            if !(TIMER_MIN_SECONDS..=TIMER_MAX_SECONDS).contains(every_seconds) {
                errors.push("when.every_seconds", "Pick 5 minutes to 12 hours");
            }
            if *min_messages > 1000 {
                errors.push("when.min_messages", "Keep it to 1000 messages or fewer");
            }
            // Chats x rate is not a save error: `validate` has no `sends_per_minute` (S7.1's
            // `ValidateCtx` carries none) and the preference is per device. The pace rail
            // refuses what the rules may not send, per device and across devices, at fire;
            // a timed rule in many chats also spreads its posts by a per-chat offset.
        }
        Trigger::Clock { at, .. } => {
            if parse_hhmm(at).is_none() {
                errors.push("when.at", "Use a time like 19:30");
            }
        }
        Trigger::Uptime { seconds, .. } | Trigger::Delay { seconds } => {
            if !(1..=TIMED_MAX_SECONDS).contains(seconds) {
                errors.push("when.seconds", "Pick 1 second to 48 hours");
            }
        }
        _ => {}
    }
    if rule.is_timed() {
        match rule.only_if.channels {
            ChannelScope::Own {} | ChannelScope::These { .. } | ChannelScope::Watching {} => {}
            ChannelScope::AnyOpen {} => errors.push("only_if.channels", TIMER_WORDS),
        }
        if !matches!(rule.when, Trigger::Delay { .. }) && !rule.only_if.only_live && rule.origin.as_deref().map_or(true, |o| !o.starts_with("reminder:")) {
            // Timer-family rules are only-live by default; a converted reminder may turn it off.
        }
    }
    if let Some(expression) = &rule.only_if.expression {
        if !rule.is_chat_rule() {
            errors.push("only_if.expression", CHAT_ONLY_WORDS);
        } else if expression.chars().count() > 500 {
            errors.push("only_if.expression", "Keep the expression to 500 characters");
        } else if let Err(e) = crate::services::chat_rules::ChatRules::compile_expression(expression) {
            errors.push("only_if.expression", &e);
        }
    }
}

/// P2 action checks (called from the action loop for every kind but Reply,
/// Send and Notify).
fn check_p2_action(rule: &Rule, action: &Action, at: &str, scope: &TextScope, errors: &mut Errors) {
    let chat_msg = matches!(rule.when, Trigger::Message { .. } | Trigger::Mention { .. } | Trigger::FirstMessage {});
    let raid = matches!(&rule.when, Trigger::ChannelEvent { events, .. } if events.contains(&EventKind::Raid));
    let whisper_rule = rule.is_whisper_rule();
    let send_as = rule.send_as.id.trim();
    let own_is_actor = !send_as.is_empty() && rule.owner.id.trim() == send_as;
    let staff_only = |errors: &mut Errors, name: &str| {
        if whisper_rule {
            errors.push(at, WHISPER_RULE_WORDS);
        } else if !chat_msg {
            errors.push(at, &format!("{name} works on a chat message"));
        }
    };
    match action {
        Action::Command { line, .. } => {
            if whisper_rule {
                errors.push(at, WHISPER_RULE_WORDS);
            }
            let timed_own = matches!(rule.when, Trigger::Timer { .. } | Trigger::Clock { .. })
                && matches!(rule.only_if.channels, ChannelScope::Own {});
            match crate::services::account_automations::command::check_line(line, &crate::services::account_automations::command::SaveCtx {
                timed_own,
                owner_is_send_as: own_is_actor,
                chatter_text_ok: scope.chatter_ok,
            }) {
                Ok(()) => {}
                Err(words) => errors.push(format!("{at}.line"), &words),
            }
        }
        Action::Whisper { to, texts, .. } => {
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
                }
                check_outward_text(&path, &line.text, scope, errors);
            }
            if !whisper_rule && to.as_ref().map_or(true, |t| t.user_id.trim().is_empty()) {
                errors.push(format!("{at}.to"), "Pick a person you have whispered with");
            }
            if rule.is_timed() {
                if let Some(t) = to {
                    if t.user_id.trim().is_empty() {
                        errors.push(format!("{at}.to"), "Pick a person you have whispered with");
                    }
                }
            }
        }
        Action::Timeout { seconds, reason, .. } => {
            staff_only(errors, "Timeout");
            if !(1..=TIMEOUT_MAX_SECONDS).contains(seconds) {
                errors.push(format!("{at}.seconds"), "Pick 1 second to 14 days");
            }
            if is_kick_sender(rule) && *seconds > 604_800 {
                errors.push(format!("{at}.seconds"), "Kick timeouts last up to 7 days");
            }
            if reason.chars().count() > MAX_REASON_CHARS {
                errors.push(format!("{at}.reason"), "Keep the reason to 500 characters");
            }
            check_outward_text(&format!("{at}.reason"), reason, scope, errors);
        }
        Action::Delete { .. } => staff_only(errors, "Delete"),
        Action::Pin { seconds, .. } => {
            staff_only(errors, "Pin");
            if let Some(s) = seconds {
                if !(PIN_MIN_SECONDS..=PIN_MAX_SECONDS).contains(s) {
                    errors.push(format!("{at}.seconds"), "A pin lasts 30 seconds to 30 minutes");
                }
            }
        }
        Action::Announce { text, color, .. } => {
            staff_only(errors, "Announce");
            let t = text.trim();
            if t.is_empty() {
                errors.push(format!("{at}.text"), "Write the announcement");
            } else if text.chars().count() > MAX_ANNOUNCE_CHARS {
                errors.push(format!("{at}.text"), "Keep it to 500 characters");
            }
            if !matches!(color.as_str(), "primary" | "blue" | "green" | "orange" | "purple") {
                errors.push(format!("{at}.color"), "Pick one of the announcement colors");
            }
            check_outward_text(&format!("{at}.text"), text, scope, errors);
        }
        Action::Shoutout { .. } => {
            if whisper_rule {
                errors.push(at, WHISPER_RULE_WORDS);
            } else if !(chat_msg || raid) {
                errors.push(at, "Shoutout works on a chat message or a raid");
            }
        }
        Action::JoinChat { channel, .. } | Action::LeaveChat { channel, .. } => {
            if login_of(&channel.key).is_empty() {
                errors.push(format!("{at}.channel"), "Pick a channel");
            }
        }
        Action::Wait { seconds, .. } => {
            if !(1..=WAIT_MAX_SECONDS).contains(seconds) {
                errors.push(format!("{at}.seconds"), "Wait 1 to 300 seconds");
            }
        }
        // Speak and Highlight are checked by the P3 rules below.
        Action::Log { .. }
        | Action::Reply { .. }
        | Action::Send { .. }
        | Action::Notify { .. }
        | Action::Speak { .. }
        | Action::Highlight { .. } => {}
    }
    // `{user}` has no meaning when nothing a person did started the rule.
    if rule.is_timed() {
        let mut texts: Vec<&str> = action.lines().iter().map(|l| l.text.as_str()).collect();
        match action {
            Action::Announce { text, .. } => texts.push(text),
            Action::Command { line, .. } => texts.push(line),
            _ => {}
        }
        if texts.iter().any(|t| placeholders(t).iter().any(|n| matches!(n.to_ascii_lowercase().as_str(), "user" | "user.name" | "user.login"))) {
            errors.push(at, NO_USER_WORDS);
        }
    }
    // A Whisper rule's Send needs an explicit chat.
    if whisper_rule {
        if let Action::Send { to: None, .. } = action {
            errors.push(format!("{at}.to"), "Pick the chat to post in");
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
        Trigger::RewardRedeemed { reward_id: Some(id) }
            if id.trim().is_empty() || id.chars().count() > 64 || id.chars().any(char::is_control) =>
        {
            errors.push("when.reward_id", "Pick a reward");
        }
        _ => {}
    }
    check_p2_trigger(rule, &mut errors);

    if let ChannelScope::These { list } = &rule.only_if.channels {
        let unfinished_template =
            !rule.enabled && rule.origin.as_deref() == Some("template:friend-live") && rule.is_live_rule();
        if list.is_empty() && !unfinished_template {
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

    check_platforms(rule, &mut errors);
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
        if rule.is_edge_rule() && matches!(action, Action::Reply { .. }) {
            errors.push(at.clone(), "There is no message to reply to here; post a message instead");
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
            Action::Speak { text, .. } => {
                let path = format!("{at}.text");
                if text.trim().is_empty() {
                    errors.push(path.clone(), "Write what to say");
                } else if text.chars().count() > MAX_SPEAK_CHARS {
                    errors.push(path.clone(), "Keep it to 300 characters");
                }
                // The chatter-text rule applies to what is read aloud (PLAN § 11.2).
                check_outward_text(&path, text, &scope, &mut errors);
            }
            Action::Highlight { color, .. } => {
                if !is_hex_color(color) {
                    errors.push(format!("{at}.color"), "Pick a color");
                }
                if !matches!(rule.when, Trigger::Message { .. }) {
                    errors.push(at.clone(), "A highlight works only when someone says certain words");
                } else {
                    // Only what the read loop can judge for one message.
                    if matches!(rule.only_if.channels, ChannelScope::Watching {}) {
                        errors.push("only_if.channels", "A highlight can't follow the stream you're watching");
                    }
                    if rule.only_if.only_live {
                        errors.push("only_if.only_live", "A highlight can't wait for the stream to be live");
                    }
                    if rule.only_if.only_away {
                        errors.push("only_if.only_away", "A highlight can't wait for you to be away");
                    }
                    if rule.only_if.hours.is_some() {
                        errors.push("only_if.hours", "A highlight can't use hours");
                    }
                }
            }
            other => check_p2_action(rule, other, &at, &scope, &mut errors),
        }
    }
    if rule.actions.iter().any(|a| matches!(a, Action::Command { .. } | Action::Whisper { .. } | Action::Timeout { .. } | Action::Delete { .. } | Action::Pin { .. } | Action::Announce { .. } | Action::Shoutout { .. }))
        && rule.send_as.id.trim().is_empty()
    {
        errors.push("send_as", "Pick who acts");
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
        let newer = json!({ "id": "r-000000000001", "name": "Polled", "when": { "kind": "stream_poll", "every_seconds": 900 }, "actions": [] });
        assert!(Rule::from_value(&newer).is_err());
        let newer_action = json!({ "id": "r-000000000002", "actions": [{ "kind": "skywrite", "id": "a-00000001" }] });
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
        assert_eq!((wild.sends_per_minute, wild.other_channel_gap_seconds, wild.whisper_people_per_day), (1, 600, 30));
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

    fn kick_me() -> AccountRef {
        AccountRef { provider: "kick".into(), id: "5001".into(), login: "mykick".into(), name: "MyKick".into() }
    }

    fn on(platforms: &[Platform], mut rule: Rule) -> Rule {
        rule.only_if.platforms = platforms.to_vec();
        rule
    }

    fn kick_rule(action: Action) -> Rule {
        let mut r = on(&[Platform::Kick], rule_with(action));
        r.send_as = kick_me();
        r
    }

    #[test]
    fn platforms_default_to_twitch_and_an_empty_list_reads_as_twitch() {
        assert_eq!(Rule::from_value(&json!({})).unwrap().only_if.platforms, vec![Platform::Twitch]);
        assert_eq!(Conditions::default().platforms, vec![Platform::Twitch]);
        let stored: Conditions = serde_json::from_value(json!({ "platforms": [] })).unwrap();
        assert!(stored.platforms.is_empty(), "a stored empty list stays empty until it is saved");
        let mut r = rule_with(reply("hi"));
        r.only_if.platforms = Vec::new();
        assert!(validate(&mut r, &ValidateCtx::default()).is_empty());
        assert_eq!(r.only_if.platforms, vec![Platform::Twitch]);
        let mut twice = on(&[Platform::Twitch, Platform::Twitch], rule_with(reply("hi")));
        validate(&mut twice, &ValidateCtx::default());
        assert_eq!(twice.only_if.platforms, vec![Platform::Twitch]);
        assert_eq!(serde_json::to_value(Platform::Youtube).unwrap(), json!("youtube"));
        assert_eq!(serde_json::to_value(EventKind::TikTokGift).unwrap(), json!("tiktok_gift"));
        assert_eq!(serde_json::to_value(EventKind::SuperChat).unwrap(), json!("super_chat"));
    }

    #[test]
    fn a_rule_that_posts_names_one_platform() {
        let mut both = on(&[Platform::Twitch, Platform::Kick], rule_with(reply("hi")));
        assert!(has(&validate(&mut both, &ValidateCtx::default()), "only_if.platforms"));
        let mut quiet = on(&[Platform::Twitch, Platform::Kick], rule_with(notify("hi")));
        assert!(validate(&mut quiet, &ValidateCtx::default()).is_empty(), "a notify-only rule may watch both");
    }

    #[test]
    fn youtube_and_tiktok_rules_cannot_post() {
        for platform in [Platform::Youtube, Platform::Tiktok] {
            let mut posting = on(&[platform], rule_with(reply("hi")));
            posting.send_as.provider = platform.as_str().into();
            let errors = validate(&mut posting, &ValidateCtx::default());
            assert!(has(&errors, "actions[0]"), "{platform:?}");
            let mut quiet = on(&[platform], rule_with(notify("hi")));
            assert!(validate(&mut quiet, &ValidateCtx::default()).is_empty(), "{platform:?}");
        }
        for kind in ["reply", "send"] {
            assert!(!action_fits(Platform::Youtube, kind) && !action_fits(Platform::Tiktok, kind));
        }
        assert!(action_fits(Platform::Youtube, "log") && action_fits(Platform::Tiktok, "speak"));
    }

    #[test]
    fn a_kick_rule_sends_as_the_kick_account_and_owns_a_twitch_one() {
        let mut ok = kick_rule(reply("hi"));
        assert!(validate(&mut ok, &ValidateCtx::default()).is_empty());
        let mut as_twitch = on(&[Platform::Kick], rule_with(reply("hi")));
        assert!(has(&validate(&mut as_twitch, &ValidateCtx::default()), "send_as"));
        let mut kick_owner = kick_rule(reply("hi"));
        kick_owner.owner = kick_me();
        assert!(has(&validate(&mut kick_owner, &ValidateCtx::default()), "owner"), "the claim account is a Twitch account");
        assert_eq!(listened_chats(&kick_rule(reply("hi"))), vec!["kick:mykick".to_string()]);
    }

    #[test]
    fn kick_has_no_live_source_so_these_fields_are_errors() {
        let mut live = kick_rule(reply("hi"));
        live.only_if.only_live = true;
        assert!(has(&validate(&mut live, &ValidateCtx::default()), "only_if.only_live"));
        let mut title = kick_rule(reply("now playing {game}, up {uptime}"));
        let errors = validate(&mut title, &ValidateCtx::default());
        assert!(has(&errors, "actions[0].texts[0].text"));
        let mut quiet = kick_rule(notify("{title}"));
        assert!(has(&validate(&mut quiet, &ValidateCtx::default()), "actions[0].title"));
        assert!(timer_family("timer") && timer_family("clock") && timer_family("uptime") && timer_family("delay"));
        assert!(!timer_family("message"));
        let mut watching = kick_rule(reply("hi"));
        watching.only_if.channels = ChannelScope::Watching {};
        assert!(has(&validate(&mut watching, &ValidateCtx::default()), "only_if.channels"));
    }

    #[test]
    fn events_belong_to_their_platform() {
        let event = |platform: Platform, kind: EventKind| {
            let mut r = on(&[platform], rule_with(notify("x")));
            r.when = Trigger::ChannelEvent { events: vec![kind], min_months: 0, min_gifts: 0, min_viewers: 0, min_bits: 0 };
            has(&validate(&mut r, &ValidateCtx::default()), "when.events")
        };
        assert!(!event(Platform::Kick, EventKind::Sub) && !event(Platform::Kick, EventKind::GiftBomb));
        assert!(event(Platform::Kick, EventKind::Raid) && event(Platform::Kick, EventKind::Cheer));
        assert!(!event(Platform::Youtube, EventKind::SuperChat) && !event(Platform::Youtube, EventKind::MemberGift));
        assert!(event(Platform::Youtube, EventKind::Sub));
        assert!(!event(Platform::Tiktok, EventKind::TikTokFollow) && event(Platform::Tiktok, EventKind::SuperChat));
        assert!(event(Platform::Twitch, EventKind::SuperChat) && !event(Platform::Twitch, EventKind::Raid));
    }

    #[test]
    fn chat_keys_keep_their_platform_and_a_youtube_id_its_case() {
        assert_eq!(composite_key("kick:Alpha"), "kick:alpha");
        assert_eq!(composite_key("youtube:UCabcDEF"), "youtube:UCabcDEF");
        assert_eq!(composite_key("#XQC"), "twitch:xqc");
        assert_eq!(composite_key("tiktok:Some.Name"), "tiktok:some.name");
        assert_eq!(provider_of_key("kick:alpha"), "kick");
        assert_eq!(provider_of_key("lirik"), "twitch");
        let mut r = kick_rule(reply("hi"));
        r.only_if.channels = ChannelScope::These { list: vec![chat("kick:Alpha"), chat("twitch:lirik")] };
        let errors = validate(&mut r, &ValidateCtx::default());
        let ChannelScope::These { list } = &r.only_if.channels else { unreachable!() };
        assert_eq!(list[0].key, "kick:alpha");
        assert!(!has(&errors, "only_if.channels.list[0]"));
        assert!(has(&errors, "only_if.channels.list[1]"), "a Twitch chat in a Kick rule");
        let mut send = kick_rule(Action::Send {
            id: String::new(),
            to: Some(chat("twitch:lirik")),
            texts: vec![line("hi")],
            pick: Pick::Random,
        });
        assert!(has(&validate(&mut send, &ValidateCtx::default()), "actions[0].to"));
    }

    fn p3_rule(when: Trigger, action: Action) -> Rule {
        Rule { name: "P3".into(), owner: me(), send_as: me(), when, actions: vec![action], ..Rule::default() }
    }

    fn speak(text: &str) -> Action {
        Action::Speak { id: String::new(), text: text.into(), on_stream: false }
    }

    #[test]
    fn p3_triggers_and_actions_round_trip_with_explicit_nulls() {
        for (when, kind) in [
            (Trigger::Follow {}, "follow"),
            (Trigger::RewardRedeemed { reward_id: None }, "reward_redeemed"),
            (Trigger::PollStarted {}, "poll_started"),
            (Trigger::PredictionStarted {}, "prediction_started"),
            (Trigger::LiveChange { online: false }, "live_change"),
        ] {
            let v = serde_json::to_value(&when).unwrap();
            assert_eq!(v["kind"], kind);
            assert_eq!(serde_json::from_value::<Trigger>(v.clone()).unwrap(), when);
            if kind == "reward_redeemed" {
                assert!(v.get("reward_id").is_some_and(Value::is_null), "an explicit null clears the reward");
            }
        }
        assert_eq!(serde_json::from_value::<Trigger>(json!({ "kind": "live_change" })).unwrap(), Trigger::LiveChange { online: true });
        let s = serde_json::to_value(speak("hi")).unwrap();
        assert_eq!((s["kind"].as_str(), s["on_stream"].as_bool()), (Some("speak"), Some(false)));
        let h: Action = serde_json::from_value(json!({ "kind": "highlight", "color": "#ff8800" })).unwrap();
        assert_eq!(h.id(), "");
        assert!(!speak("x").is_outward() && !h.is_outward());
    }

    #[test]
    fn a_p3_rule_is_forced_into_its_shape() {
        let mut follow = p3_rule(Trigger::Follow {}, notify("x"));
        follow.only_if.channels = ChannelScope::AnyOpen {};
        follow.only_if.roles = vec![SenderRole::Moderator];
        assert!(validate(&mut follow, &ValidateCtx::default()).is_empty());
        assert_eq!(follow.only_if.channels, ChannelScope::Own {});
        assert!(follow.only_if.roles.is_empty());
        assert!(listened_chats(&follow).is_empty(), "an own-channel event joins no chat");

        let mut live = p3_rule(Trigger::LiveChange { online: true }, notify("x"));
        live.only_if.only_live = true;
        let errors = validate(&mut live, &ValidateCtx::default());
        assert!(has(&errors, "only_if.channels"), "a live change needs a channel");
        assert!(!live.only_if.only_live && listened_chats(&live).is_empty());
        live.only_if.channels = ChannelScope::These { list: vec![chat("friend")] };
        assert!(validate(&mut live, &ValidateCtx::default()).is_empty());
    }

    #[test]
    fn p3_actions_are_checked() {
        let ctx = ValidateCtx::default();
        let mut reply = p3_rule(Trigger::Follow {}, reply("hi"));
        assert!(has(&validate(&mut reply, &ctx), "actions[0]"), "no message to reply to");
        let mut send = p3_rule(Trigger::Follow {}, Action::Send { id: String::new(), to: None, texts: vec![line("Welcome {user}")], pick: Pick::Random });
        assert!(validate(&mut send, &ctx).is_empty(), "a Send posts in your own channel");

        let mut empty = p3_rule(Trigger::Follow {}, speak("  "));
        assert!(has(&validate(&mut empty, &ctx), "actions[0].text"));
        let mut long = p3_rule(Trigger::Follow {}, speak(&"a".repeat(MAX_SPEAK_CHARS + 1)));
        assert!(has(&validate(&mut long, &ctx), "actions[0].text"));
        let mut exact = p3_rule(Trigger::Follow {}, speak(&"a".repeat(MAX_SPEAK_CHARS)));
        assert!(validate(&mut exact, &ctx).is_empty());
        let mut input = p3_rule(Trigger::RewardRedeemed { reward_id: None }, speak("{input}"));
        assert!(has(&validate(&mut input, &ctx), "actions[0].text"), "{{input}} is chatter text");
        let mut input_note = p3_rule(Trigger::RewardRedeemed { reward_id: None }, Action::Notify { id: String::new(), title: "{reward}".into(), body: "{input}".into(), toast: true, system: false, sound: None, volume: 1.0 });
        assert!(validate(&mut input_note, &ctx).is_empty(), "a Notify may show it");

        let mut bad_reward = p3_rule(Trigger::RewardRedeemed { reward_id: Some("  ".into()) }, notify("x"));
        assert!(has(&validate(&mut bad_reward, &ctx), "when.reward_id"));
    }

    #[test]
    fn a_highlight_is_for_message_rules_with_conditions_the_read_loop_can_judge() {
        let ctx = ValidateCtx::default();
        let hl = |color: &str| Action::Highlight { id: String::new(), color: color.into() };
        let message = |text: &str| Trigger::Message { text: TextMatch { mode: MatchMode::Contains, pattern: text.into(), case_sensitive: false } };
        let mut ok = p3_rule(message("hi"), hl("#FF8800"));
        assert!(validate(&mut ok, &ctx).is_empty());
        assert!(matches!(&ok.actions[0], Action::Highlight { color, .. } if color == "#ff8800"), "stored lowercase");
        for bad in ["", "ff8800", "#ff88", "#gg0000", "red"] {
            let mut r = p3_rule(message("hi"), hl(bad));
            assert!(has(&validate(&mut r, &ctx), "actions[0].color"), "{bad}");
        }
        let mut mention = p3_rule(Trigger::Mention { replies_too: false }, hl("#ff8800"));
        assert!(has(&validate(&mut mention, &ctx), "actions[0]"));
        let mut watching = p3_rule(message("hi"), hl("#ff8800"));
        watching.only_if.channels = ChannelScope::Watching {};
        watching.only_if.only_live = true;
        watching.only_if.only_away = true;
        watching.only_if.hours = Some(Hours { from: "09:00".into(), to: "17:00".into(), days: vec![] });
        let errors = validate(&mut watching, &ctx);
        for path in ["only_if.channels", "only_if.only_live", "only_if.only_away", "only_if.hours"] {
            assert!(has(&errors, path), "{path}");
        }
    }

    #[test]
    fn the_friend_template_may_be_saved_unfinished_and_nothing_else_may() {
        let mut template = p3_rule(Trigger::LiveChange { online: true }, notify("{channel} is live"));
        template.enabled = false;
        template.origin = Some("template:friend-live".into());
        assert!(validate(&mut template, &ValidateCtx::default()).is_empty());
        template.enabled = true;
        assert!(has(&validate(&mut template, &ValidateCtx::default()), "only_if.channels"));
        let mut other = p3_rule(Trigger::LiveChange { online: true }, notify("x"));
        other.enabled = false;
        assert!(has(&validate(&mut other, &ValidateCtx::default()), "only_if.channels"));
    }

    #[test]
    fn input_is_chatter_text_and_reward_is_not() {
        assert!(is_chatter_text("input") && is_chatter_text("INPUT"));
        assert!(!is_chatter_text("reward") && !is_chatter_text("poll") && !is_chatter_text("prediction"));
    }
}
