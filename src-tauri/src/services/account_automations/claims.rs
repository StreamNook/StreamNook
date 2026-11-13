//! The plain claim keys, families and elements an outward action claims
//! with, built the same way on every device.
//!
//! Right before a rule posts, its device claims the event on streamnook.app,
//! so only one of your devices answers it. A key names the event or the rail;
//! a family names which answers collide, so two rules on two devices with the
//! same family for the same event give one answer. Keys and families go to the
//! site as plain text over TLS, and the site replaces each with an HMAC before
//! the database sees it. Every value built here must match [`PLAIN_SHAPE`]; a
//! call holding one that does not is never sent.
//!
//! Each outward action belongs to one action class (what kind of answer it
//! gives, and where), and each class claims on its own: a reply on one device
//! and a different kind of answer on another both run, two replies never do.
//! Wording, filters, accounts and thresholds are never part of a family.

use std::sync::OnceLock;

use regex::Regex;
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::badges::BadgePhase;
use super::facts::{fold, nfc, Facts, RowKind};
use crate::models::automations::{
    placeholders, provider_of_key, AccountRef, Action, AutomationPrefs, EventKind, Line, MatchMode, Rule, TextMatch,
    Trigger,
};

/// Every plain key and family: a kind of 1 to 16 lowercase letters or `_`, a
/// colon, then 1 to 600 characters with no control characters. The same text
/// as the `plain` pattern of `tests/fixtures/automation_claim_patterns.json`.
pub const PLAIN_SHAPE: &str = r"^[a-z_]{1,16}:[^\x00-\x1f\x7f]{1,600}$";
/// Every rail kind; each rail's family is `rail:<kind>`.
pub const RAIL_KINDS: &[&str] = &["gap", "rulecd", "person", "conv", "pace", "reminder_kw"];
/// Sends one account's rules may address to one person in 5 minutes, across devices.
pub const CONV_LIMIT: u32 = 3;
/// The conversation rail's period.
pub const CONV_PERIOD_S: u32 = 300;
/// The pace rail's period.
pub const PACE_PERIOD_S: u32 = 60;
/// The window of a Kick event's claim (the event has no id another device shares).
pub const KICK_EVENT_WINDOW_S: u32 = 60;
/// The longest a winner may take to send: the longest rule wait plus 5 s.
pub const MAX_SETTLE_MS: u32 = 135_000;
/// The longest window a windowed element carries (30 days).
pub const MAX_WINDOW_S: u32 = 2_592_000;
/// The most sends one event element counts.
pub const MAX_SENDS: u32 = 20;

/// Activity words when a class's claim could not be built.
pub const BUILD_FAILED: &str = "Couldn't build this rule's check with your other devices";
/// Activity words for a Send whose chat has no room id yet.
pub const NO_ROOM: &str = "Pick the channel again so StreamNook can find it";

fn shape() -> &'static Regex {
    static SHAPE: OnceLock<Regex> = OnceLock::new();
    SHAPE.get_or_init(|| Regex::new(PLAIN_SHAPE).expect("the plain key shape compiles"))
}

/// Whether `value` may be sent as a plain key or family.
pub fn plain_ok(value: &str) -> bool {
    shape().is_match(value)
}

/// The first 16 hex of the SHA-256 of `text`.
pub fn hash16(text: &str) -> String {
    Sha256::digest(text.as_bytes())[..8].iter().map(|b| format!("{b:02x}")).collect()
}

/// A claim call's id: 16 random hex.
pub fn new_call_id() -> String {
    format!("{:016x}", rand::random::<u64>())
}

/// This process's claim session: 16 random hex, made once at start.
pub fn new_session() -> String {
    format!("{:016x}", rand::random::<u64>())
}

/// What every call carries so a restarted process that loses to its own
/// earlier session hears "this device": the first 16 hex of the SHA-256 of
/// the device id.
pub fn device_hash(device_id: &str) -> String {
    hash16(device_id)
}

/// A platform's id as a key part: a Twitch id stays bare (so P1's keys do not
/// change), every other platform's carries its name, so a Kick id equal to a
/// Twitch id never shares a class or a rail (D278, D289).
pub fn pfx(provider: &str, id: &str) -> String {
    match provider.trim().to_ascii_lowercase().as_str() {
        "" | "twitch" => id.to_string(),
        other => format!("{other}:{id}"),
    }
}

/// An account as a key part: `pfx` of its provider and id.
pub fn account_key(account: &AccountRef) -> String {
    pfx(&account.provider, &account.id)
}

/// The chatter on the rule's platform, as a key part.
fn person_of(rule: &Rule, person: &str) -> String {
    pfx(&rule.send_as.provider, person)
}

/// The room of the sending account's own channel: the owner's room id on
/// Twitch, `kick:<the Kick account's id>` on Kick (a Kick channel's broadcaster
/// id is the account's id).
fn own_room(rule: &Rule) -> String {
    match rule.send_as.provider.trim().to_ascii_lowercase().as_str() {
        "" | "twitch" => rule.owner.id.clone(),
        _ => account_key(&rule.send_as),
    }
}

// ---------------------------------------------------------------------------
// Event keys
// ---------------------------------------------------------------------------

/// A chat message's key: `chat:<source-id, else id>`. Cheers use it too.
pub fn chat_key(id: &str) -> String {
    format!("chat:{id}")
}

/// A Twitch notice's key: `notice:<source-id, else id>`.
pub fn notice_key(id: &str) -> String {
    format!("notice:{id}")
}

/// One badge drop's key for one phase: `badge:<feed id>:<announced or earnable>`.
pub fn badge_key(feed_id: &str, phase: BadgePhase) -> String {
    format!("badge:{feed_id}:{}", phase.as_str())
}

/// A follow: `follow:<user id>:<followed_at epoch s>`. The time is epoch
/// seconds (`facts::started_epoch`), so every spelling of it gives one key.
pub fn follow_key(user_id: &str, followed_at_s: i64) -> String {
    format!("follow:{user_id}:{followed_at_s}")
}

/// A redemption: `reward:<redemption id>` (a UUID Twitch gives every device).
pub fn reward_key(redemption_id: &str) -> String {
    format!("reward:{redemption_id}")
}

/// A poll: `poll:<id>`.
pub fn poll_key(id: &str) -> String {
    format!("poll:{id}")
}

/// A prediction: `prediction:<id>`.
pub fn prediction_key(id: &str) -> String {
    format!("prediction:{id}")
}

/// A channel going live: `live:<login>:<started_at epoch s>`.
pub fn live_key(login: &str, started_at_s: i64) -> String {
    format!("live:{}:{started_at_s}", login.trim().to_ascii_lowercase())
}

/// A channel going offline: `offline:<login>:<started_at epoch s of the
/// broadcast that ended>`.
pub fn offline_key(login: &str, started_at_s: i64) -> String {
    format!("offline:{}:{started_at_s}", login.trim().to_ascii_lowercase())
}

/// A reward trigger's part of the family: the first 16 hex of the SHA-256 of
/// the reward id (a UUID never enters a family as text), or `any`.
pub fn reward_part(reward_id: Option<&str>) -> String {
    match reward_id.map(str::trim).filter(|id| !id.is_empty()) {
        Some(id) => hash16(id),
        None => "any".to_string(),
    }
}

/// A whisper's key: `whisper:<whisper id>` (exact, per whisper).
pub fn whisper_key(whisper_id: &str) -> String {
    format!("whisper:{whisper_id}")
}

/// A Timer fire in one chat: `timer:<chat key>` (PLAN § 4.8), claimed per
/// window (`timed::timed_window_s`: `ceil(0.9 x every)`, the reminders' own).
pub fn timer_key(chat_key: &str) -> String {
    format!("timer:{chat_key}")
}

/// A Clock fire: `clock:<chat key>:<local date>T<HH:MM><UTC offset>` of the
/// scheduled time (not the minute a late tick ran), the offset in force at that
/// time (`+HH:MM`), so devices in different time zones give two events.
pub fn clock_key(chat_key: &str, date: &str, hhmm: &str, offset_s: i32) -> String {
    format!("clock:{chat_key}:{date}T{hhmm}{}", offset_words(offset_s))
}

/// `+HH:MM` or `-HH:MM` (the reminders' `offset_text`, one spelling for both).
pub fn offset_words(offset_s: i32) -> String {
    let sign = if offset_s < 0 { '-' } else { '+' };
    let a = offset_s.unsigned_abs();
    format!("{sign}{:02}:{:02}", a / 3600, (a % 3600) / 60)
}

/// An Uptime fire: `uptime:<chat key>:<started_at epoch s>:<seconds>`, once per
/// broadcast per threshold (the threshold in seconds everywhere).
pub fn uptime_key(chat_key: &str, started_epoch_s: i64, seconds: u32) -> String {
    format!("uptime:{chat_key}:{started_epoch_s}:{seconds}")
}

/// A Delay fire in one chat: `delay:<chat key>`, claimed per window
/// (`max(seconds, 600)`, the reminders' own).
pub fn delay_key(chat_key: &str) -> String {
    format!("delay:{chat_key}")
}

/// A timed rule's family: its kind and 16 hex of the SHA-256 over the
/// normalized texts of its actions, so two devices with the same timed message
/// collide whatever else differs and two different messages never starve each
/// other (wording the family names; filters, accounts and thresholds never).
pub fn timed_family(kind: &str, rule: &Rule) -> String {
    let mut texts = String::new();
    for action in &rule.actions {
        match action {
            Action::Reply { texts: t, .. } | Action::Send { texts: t, .. } | Action::Whisper { texts: t, .. } => {
                for line in t {
                    texts.push_str(&fold(&line.text));
                    texts.push('\u{1f}');
                }
            }
            Action::Announce { text, color, .. } => texts.push_str(&format!("a:{}:{color}\u{1f}", fold(text))),
            Action::Command { line, .. } => texts.push_str(&format!("c:{}\u{1f}", fold(line))),
            _ => {}
        }
    }
    format!("{kind}:{}", hash16(&texts))
}

/// The trigger family of a rule for an event: the P1 families, and for a timed
/// rule `timed_family`. `claim_class` calls this in place of `trigger_family`.
pub fn rule_trigger_family(rule: &Rule, facts: &Facts) -> Option<String> {
    match &rule.when {
        Trigger::Timer { .. } => Some(timed_family("timer", rule)),
        Trigger::Clock { .. } => Some(timed_family("clock", rule)),
        Trigger::Uptime { .. } => Some(timed_family("uptime", rule)),
        Trigger::Delay { .. } => Some(timed_family("delay", rule)),
        other => trigger_family(other, facts),
    }
}

// ---------------------------------------------------------------------------
// Action classes
// ---------------------------------------------------------------------------

/// Where an outward action answers. One claim per class per event.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ActionClass {
    /// A message in one chat, named by its room id: Reply, Send to the
    /// event's chat or to another chat, and Announce.
    Post { room_id: String },
    /// A whisper to one person (the full D273 subset, no chat slot).
    Whisper { to: String },
    /// One of Timeout, Delete, Pin and Shoutout in one room, by kind (PLAN § 4.8:
    /// `timeout:<room>`, `delete:<room>`, `pin:<room>`, `shoutout:<room>`).
    Mod { kind: &'static str, room_id: String },
    /// A Command in one room, by command word (`command:<word>:<room>`).
    Command { word: String, room_id: String },
}

impl ActionClass {
    pub fn key(&self) -> String {
        match self {
            ActionClass::Post { room_id } => format!("post:{room_id}"),
            // PLAN § 4.8: one `whisper` class (the recipient rides in the variant for the cap element, not in the key).
            ActionClass::Whisper { .. } => "whisper".to_string(),
            ActionClass::Mod { kind, room_id } => format!("{kind}:{room_id}"),
            ActionClass::Command { word, room_id } => format!("command:{word}:{room_id}"),
        }
    }

    /// The chat the class acts in (none for a whisper).
    pub fn room_id(&self) -> Option<&str> {
        match self {
            ActionClass::Post { room_id } | ActionClass::Mod { room_id, .. } | ActionClass::Command { room_id, .. } => Some(room_id),
            ActionClass::Whisper { .. } => None,
        }
    }

    /// Whether the class books a chat slot and runs steps 3, 6 and 8 (D273).
    pub fn books_slot(&self) -> bool {
        matches!(self, ActionClass::Post { .. })
    }
}

/// One action class of a job.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassPlan {
    pub class: ActionClass,
    /// The class's actions, as indexes into the rule's actions, in rule order.
    pub actions: Vec<usize>,
    /// An action of the class answers the person: a Reply, or a Send in the
    /// event's chat whose text names them.
    pub to_person: bool,
}

/// The room an event belongs to: your own room (your user id) for a badge
/// drop, else the event's room (a Shared Chat copy's source room), so a row
/// in its own room and its partner copy are one class.
pub fn event_room<'a>(rule: &'a Rule, facts: &'a Facts) -> &'a str {
    // A live change names a channel that is not the owner's: the rule posts
    // in the owner's own room, like a badge drop.
    let live = facts.own.as_ref().is_some_and(|o| matches!(o.kind, super::facts::OwnKind::Live { .. }));
    if matches!(facts.kind, RowKind::Badge | RowKind::Whisper) || live {
        &rule.owner.id
    } else {
        &facts.event_room_id
    }
}

/// The rule's outward actions grouped by class, in rule order, and the
/// actions that cannot run (a Send whose chat has no room id yet), with the
/// Activity words for each. Notify is local and belongs to no class.
pub fn classes_for(rule: &Rule, event_room: &str) -> (Vec<ClassPlan>, Vec<(usize, &'static str)>) {
    let mut plans: Vec<ClassPlan> = Vec::new();
    let mut skipped = Vec::new();
    for (index, action) in rule.actions.iter().enumerate() {
        let (room, to_person) = match action {
            Action::Reply { .. } if rule.is_whisper_rule() => {
                // The sender of the whisper: the class keys on "sender" and counts as
                // answering the person; the executor routes it to the door path.
                push_whisper_plan(&mut plans, "sender".to_string(), index);
                continue;
            }
            Action::Reply { .. } => (event_room.to_string(), true),
            Action::Send { to: None, texts, .. } => (event_room.to_string(), names_person(texts)),
            Action::Send { to: Some(chat), texts, .. } => {
                let room = chat.room_id.trim();
                if room.is_empty() {
                    skipped.push((index, NO_ROOM));
                    continue;
                }
                let room = pfx(provider_of_key(&chat.key), room);
                let same = room == event_room;
                (room, same && names_person(texts))
            }
            Action::Notify { .. }
            | Action::Log { .. }
            | Action::JoinChat { .. }
            | Action::LeaveChat { .. }
            | Action::Wait { .. }
            | Action::Speak { .. }
            | Action::Highlight { .. } => continue,
            Action::Announce { .. } => (event_room.to_string(), false),
            Action::Command { line, .. } => {
                let word = super::command::word_of(line);
                push_plan(&mut plans, ActionClass::Command { word, room_id: event_room.to_string() }, index, false);
                continue;
            }
            Action::Timeout { .. } | Action::Delete { .. } | Action::Pin { .. } | Action::Shoutout { .. } => {
                let kind = match action {
                    Action::Timeout { .. } => "timeout",
                    Action::Delete { .. } => "delete",
                    Action::Pin { .. } => "pin",
                    _ => "shoutout",
                };
                push_plan(&mut plans, ActionClass::Mod { kind, room_id: event_room.to_string() }, index, false);
                continue;
            }
            Action::Whisper { to, .. } => {
                // On a Whisper rule an empty `to` is the sender; the executor
                // resolves it, so the class keys on the stored id or "sender".
                let who = to.as_ref().map(|t| t.user_id.trim()).filter(|s| !s.is_empty()).unwrap_or("sender").to_string();
                push_whisper_plan(&mut plans, who, index);
                continue;
            }
        };
        let class = ActionClass::Post { room_id: room };
        match plans.iter_mut().find(|p| p.class == class) {
            Some(plan) => {
                plan.actions.push(index);
                plan.to_person |= to_person;
            }
            None => plans.push(ClassPlan { class, actions: vec![index], to_person }),
        }
    }
    (plans, skipped)
}

fn push_plan(plans: &mut Vec<ClassPlan>, class: ActionClass, index: usize, to_person: bool) {
    match plans.iter_mut().find(|p| p.class == class) {
        Some(plan) => {
            plan.actions.push(index);
            plan.to_person |= to_person;
        }
        None => plans.push(ClassPlan { class, actions: vec![index], to_person }),
    }
}

/// Every Whisper (and a Whisper rule's Reply) of a job is one `whisper` class
/// (PLAN § 4.8): the first action's recipient rides in the variant for the cap element.
fn push_whisper_plan(plans: &mut Vec<ClassPlan>, to: String, index: usize) {
    match plans.iter_mut().find(|p| matches!(p.class, ActionClass::Whisper { .. })) {
        Some(plan) => plan.actions.push(index),
        None => plans.push(ClassPlan { class: ActionClass::Whisper { to }, actions: vec![index], to_person: true }),
    }
}

fn names_person(lines: &[Line]) -> bool {
    lines.iter().any(|line| {
        placeholders(&line.text)
            .iter()
            .any(|name| matches!(name.to_ascii_lowercase().as_str(), "user" | "user.name" | "user.login"))
    })
}

// ---------------------------------------------------------------------------
// Families
// ---------------------------------------------------------------------------

/// A match mode as it appears in a message family.
pub fn mode_word(mode: MatchMode) -> &'static str {
    match mode {
        MatchMode::AnyWord => "any_word",
        MatchMode::AllWords => "all_words",
        MatchMode::Phrase => "phrase",
        MatchMode::Contains => "contains",
        MatchMode::StartsWith => "starts_with",
        MatchMode::Exact => "exact",
        MatchMode::Regex => "regex",
    }
}

/// An event kind as it appears in a channel-event family.
pub fn event_word(kind: EventKind) -> &'static str {
    match kind {
        EventKind::Sub => "sub",
        EventKind::Resub => "resub",
        EventKind::Gift => "gift",
        EventKind::GiftBomb => "gift_bomb",
        EventKind::Raid => "raid",
        EventKind::Cheer => "cheer",
        EventKind::Announcement => "announcement",
        EventKind::SuperChat => "super_chat",
        EventKind::SuperSticker => "super_sticker",
        EventKind::Membership => "membership",
        EventKind::MemberGift => "member_gift",
        EventKind::TikTokGift => "tiktok_gift",
        EventKind::TikTokFollow => "tiktok_follow",
        EventKind::TikTokShare => "tiktok_share",
    }
}

/// A Message trigger's part of the family: `<mode>|<case flag>|<len>:<pattern>`.
/// A Regex pattern is kept as typed; every other mode is NFC-normalized and,
/// unless case-sensitive, lowercased, the same way messages are read. The
/// length (in characters) keeps a `|` inside a pattern from ever making two
/// families collide. A control character (never in a real pattern, and
/// refused by the site) is replaced so the family keeps the plain shape.
pub fn message_part(text: &TextMatch) -> String {
    let pattern = match (text.mode, text.case_sensitive) {
        (MatchMode::Regex, _) => text.pattern.clone(),
        (_, true) => nfc(&text.pattern).into_owned(),
        (_, false) => fold(&text.pattern),
    };
    let len = pattern.chars().count();
    let shown: String = pattern.chars().map(|c| if c.is_control() { '\u{fffd}' } else { c }).collect();
    format!("{}|{}|{len}:{shown}", mode_word(text.mode), u8::from(text.case_sensitive))
}

/// The trigger family and trigger part, before the class: `message:<part>`,
/// `mention:`, `first_message:`, `channel_event:<the event's kind>` or
/// `badge:`. None for a channel-event rule on a row that carries no event.
pub fn trigger_family(when: &Trigger, facts: &Facts) -> Option<String> {
    Some(match when {
        Trigger::Message { text } => format!("message:{}", message_part(text)),
        Trigger::Mention { .. } => "mention:".to_string(),
        Trigger::FirstMessage {} => "first_message:".to_string(),
        Trigger::ChannelEvent { .. } => {
            let word = event_word(facts.event.as_ref()?.kind);
            if facts.event_key.starts_with("kick_evt:") {
                format!("kick_evt:{word}")
            } else {
                format!("channel_event:{word}")
            }
        }
        Trigger::BadgeDrop { .. } => "badge:".to_string(),
        Trigger::Whisper { .. } => "whisper:".to_string(),
        Trigger::Timer { .. } => "timer:".to_string(),
        Trigger::Clock { .. } => "clock:".to_string(),
        Trigger::Uptime { .. } => "uptime:".to_string(),
        Trigger::Delay { .. } => "delay:".to_string(),
        Trigger::Follow {} => "follow:".to_string(),
        Trigger::RewardRedeemed { reward_id } => format!("reward:{}", reward_part(reward_id.as_deref())),
        Trigger::PollStarted {} => "poll:".to_string(),
        Trigger::PredictionStarted {} => "prediction:".to_string(),
        Trigger::LiveChange { online } => format!("live:{}", if *online { "on" } else { "off" }),
    })
}

/// An event element's family: `<trigger family>:<trigger part>|<class>`.
pub fn event_family(trigger_family: &str, class: &ActionClass) -> String {
    format!("{trigger_family}|{}", class.key())
}

// ---------------------------------------------------------------------------
// Rails
// ---------------------------------------------------------------------------

/// A rail's family: `rail:<kind>`.
pub fn rail_family(kind: &str) -> String {
    format!("rail:{kind}")
}

/// One automatic message per other chat per gap, across devices.
pub fn gap_key(room_id: &str) -> String {
    format!("gap:{room_id}")
}

/// The rule cooldown of one class family in one chat. The family enters as
/// its hash, so a key never carries a pattern.
pub fn rulecd_key(family: &str, room_id: &str) -> String {
    format!("rulecd:f:{}:{room_id}", hash16(family))
}

/// The person cooldown of one class family, per sending account and person.
pub fn person_key(family: &str, send_as_id: &str, user_id: &str) -> String {
    format!("person:f:{}:{send_as_id}:{user_id}", hash16(family))
}

/// The conversation limit, per sending account and person.
pub fn conv_key(send_as_id: &str, user_id: &str) -> String {
    format!("conv:{send_as_id}:{user_id}")
}

/// The pace limit, per owner.
pub fn pace_key(owner_id: &str) -> String {
    format!("pace:{owner_id}")
}

/// One keyword reminder's cooldown in one chat (the rule cooldown's
/// pattern): `reminder_kw:f:<16 hex of the reminder family>:<chat key>`,
/// where `family` is the reminder's family text. Two different keyword
/// reminders in one chat have two rails and both post, as they do today.
pub fn reminder_kw_key(family: &str, chat_key: &str) -> String {
    format!("reminder_kw:f:{}:{chat_key}", hash16(family))
}

// ---------------------------------------------------------------------------
// Elements
// ---------------------------------------------------------------------------

/// How an event element wins.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum KeyMode {
    /// Once, for good.
    Exact,
    /// Once per window.
    Windowed,
}

/// Which sends a counted rail counts.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CountWho {
    All,
    /// Only the sends that answer the person.
    Person,
}

/// One element of a claim call, serialized as the endpoint reads it.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Element {
    Event {
        key: String,
        family: String,
        mode: KeyMode,
        #[serde(skip_serializing_if = "Option::is_none")]
        window_s: Option<u32>,
        /// The class's sends this event counts on the counted rails (0 for a
        /// badge drop: its batch counts once).
        sends: u32,
        to_person: bool,
        /// The holder's longest time to send, answered to a device that loses.
        settle_ms: u32,
    },
    /// A badge batch: counted rails count this once when any drop won.
    Batch { sends: u32 },
    Windowed {
        key: String,
        family: String,
        window_s: u32,
    },
    Counted {
        key: String,
        family: String,
        limit: u32,
        period_s: u32,
        counts: CountWho,
    },
    /// The whisper cap, evaluated last in the same transaction (the site
    /// HMACs `recipient`): at most one per person per hour and `cap` people
    /// in a rolling 24 hours, per owner.
    WhisperCap { recipient: String, cap: u32 },
}

impl Element {
    pub fn is_event(&self) -> bool {
        matches!(self, Element::Event { .. })
    }

    /// Every key and family the element carries has the plain shape.
    pub fn plain_ok(&self) -> bool {
        match self {
            Element::Event { key, family, .. } | Element::Windowed { key, family, .. } | Element::Counted { key, family, .. } => {
                plain_ok(key) && plain_ok(family)
            }
            Element::Batch { .. } => true,
            Element::WhisperCap { recipient, cap } => !recipient.is_empty() && recipient.chars().count() <= 600 && (1..=30).contains(cap),
        }
    }
}

/// The rule's longest time to send: its wait plus 5 s, in milliseconds.
pub fn settle_ms(rule: &Rule) -> u32 {
    ((rule.limits.max_wait_seconds.min(130) + 5) * 1000).min(MAX_SETTLE_MS)
}

fn account_of(rule: &Rule) -> String {
    if rule.send_as.id.trim().is_empty() {
        account_key(&rule.owner)
    } else {
        account_key(&rule.send_as)
    }
}

/// What one class's claim call is built from.
pub struct CallInput<'a> {
    pub rule: &'a Rule,
    /// This device's preferences (the gap and pace numbers it judges by).
    pub prefs: &'a AutomationPrefs,
    pub plan: &'a ClassPlan,
    /// The class's event family ([`event_family`]).
    pub family: &'a str,
    /// The event's key, or one per drop of a badge batch.
    pub event_keys: &'a [String],
    /// The room the rule fired in ([`event_room`]); the rule cooldown's chat.
    pub event_room: &'a str,
    /// The person the event came from; None for a badge drop.
    pub person: Option<&'a str>,
}

/// How a trigger's event element wins: a Timer and a Delay once per window,
/// everything else exactly once.
pub fn event_mode(when: &Trigger) -> (KeyMode, Option<u32>) {
    match when {
        Trigger::Timer { every_seconds, .. } => (KeyMode::Windowed, Some(super::reminder_claim::interval_window_s(u64::from(*every_seconds)))),
        Trigger::Delay { seconds } => (KeyMode::Windowed, Some(super::reminder_claim::delay_window_s(u64::from(*seconds)))),
        _ => (KeyMode::Exact, None),
    }
}

/// The elements of one class's claim call: its event element (one per drop
/// for a badge batch, plus the batch element), then its rails: the other-chat
/// gap, the rule and person cooldowns, the conversation limit and the pace.
/// A badge rule has no rule cooldown and no person. Err when an element would
/// not have the plain shape; nothing is sent then.
pub fn class_elements(input: &CallInput) -> Result<Vec<Element>, &'static str> {
    let rule = input.rule;
    let badge = rule.is_badge_rule();
    let sends = u32::try_from(input.plan.actions.len()).unwrap_or(MAX_SENDS).min(MAX_SENDS);
    if input.event_keys.is_empty() || sends == 0 {
        return Err(BUILD_FAILED);
    }
    let prefs = input.prefs.clamped();
    let settle = settle_ms(rule);
    let rail_person = match &input.plan.class {
        ActionClass::Whisper { to } if to != "sender" => Some(to.as_str()),
        _ => input.person,
    };
    let person = rail_person.map(str::trim).filter(|p| !p.is_empty() && !badge);
    let to_person = input.plan.to_person && person.is_some();
    let mut out = Vec::new();
    let (mode, window_s) = event_mode(&input.rule.when);
    for key in input.event_keys {
        // A Kick event has no id another device shares: its key names the
        // event's own words and is claimed once a window.
        let windowed = key.starts_with("kick_evt:");
        out.push(Element::Event {
            key: key.clone(),
            family: input.family.to_string(),
            mode: if windowed { KeyMode::Windowed } else { mode },
            window_s: if windowed { Some(KICK_EVENT_WINDOW_S) } else { window_s },
            sends: if badge { 0 } else { sends },
            to_person,
            settle_ms: settle,
        });
    }
    if badge {
        out.push(Element::Batch { sends: 1 });
    }
    if let Some(room) = input.plan.class.room_id() {
        if room != own_room(rule).as_str() {
            out.push(Element::Windowed {
                key: gap_key(room),
                family: rail_family("gap"),
                window_s: prefs.other_channel_gap_seconds,
            });
        }
    }
    let rule_cd = rule.limits.rule_cooldown_seconds.min(MAX_WINDOW_S);
    if rule_cd > 0 && !badge {
        out.push(Element::Windowed {
            key: rulecd_key(input.family, input.event_room),
            family: rail_family("rulecd"),
            window_s: rule_cd,
        });
    }
    let user_cd = rule.limits.user_cooldown_seconds.min(MAX_WINDOW_S);
    if let Some(person) = person {
        if user_cd > 0 {
            out.push(Element::Windowed {
                key: person_key(input.family, &account_of(rule), &person_of(rule, person)),
                family: rail_family("person"),
                window_s: user_cd,
            });
        }
        if to_person {
            out.push(Element::Counted {
                key: conv_key(&account_of(rule), &person_of(rule, person)),
                family: rail_family("conv"),
                limit: CONV_LIMIT,
                period_s: CONV_PERIOD_S,
                counts: CountWho::Person,
            });
        }
    }
    out.push(Element::Counted {
        key: pace_key(&rule.owner.id),
        family: rail_family("pace"),
        limit: prefs.sends_per_minute,
        period_s: PACE_PERIOD_S,
        counts: CountWho::All,
    });
    // A Whisper class's cap element names the recipient: the stored id, or the
    // sender (`to == "sender"`, the event's person) for the Reply form.
    if let ActionClass::Whisper { to } = &input.plan.class {
        let recipient = if to == "sender" { person } else { Some(to.as_str()) };
        if let Some(recipient) = recipient {
            out.push(Element::WhisperCap { recipient: recipient.to_string(), cap: prefs.whisper_people_per_day });
        }
    }
    if out.iter().all(Element::plain_ok) {
        Ok(out)
    } else {
        Err(BUILD_FAILED)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::automations::{validate, BadgeLeg, BadgeLink, RunState, ValidateCtx};
    use crate::services::account_automations::badges::{BadgeInfo, BadgeSeen, Presence};
    use crate::services::account_automations::compile::fixtures::{reply, rule, send, send_to, notify, ME};
    use crate::services::account_automations::facts::{from_badge, from_chat, rows};

    const UUID: &str = "9b4c2f1e-0d3a-4c55-8f6e-2b7a1c9d0e11";
    const TWENTY: &str = "12345678901234567890";

    fn chat_facts() -> Facts {
        from_chat(&rows::privmsg(UUID, "me", ME, "viewer", "300", "hello @me")).unwrap()
    }

    fn seen(feed_id: &str, phase: BadgePhase, announced: bool, earnable: bool) -> BadgeSeen {
        BadgeSeen {
            badge: BadgeInfo { set_id: feed_id.into(), version: "1".into(), name: "Test".into(), ..BadgeInfo::default() },
            feed_id: feed_id.into(),
            phase,
            announced,
            earnable,
            ts_ms: 0,
            arrived_ms: 0,
            push: true,
            in_gallery: Presence::No,
        }
    }

    fn mention() -> Trigger {
        Trigger::Mention { replies_too: true }
    }

    fn badge_rule() -> Rule {
        rule(
            "r-badge",
            Trigger::BadgeDrop { on: BadgeLeg::Both, link: BadgeLink::default(), batch_text: "{badge.count} new: {badges}".into() },
            vec![send("New badge: {badge}")],
        )
    }

    fn assert_plain(value: &str) {
        assert!(plain_ok(value), "{value}");
        let (_, rest) = value.split_once(':').unwrap();
        assert!(rest.chars().count() <= 600, "{value}");
    }

    #[test]
    fn call_ids_sessions_and_device_hashes_are_16_hex() {
        let hex16 = |s: &str| s.len() == 16 && s.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase());
        assert!(hex16(&new_call_id()) && hex16(&new_session()));
        assert_ne!(new_call_id(), new_call_id());
        assert!(hex16(&device_hash("0123456789abcdef01234567")));
        assert_eq!(device_hash("a"), device_hash("a"));
        assert_ne!(device_hash("a"), device_hash("b"));
    }

    #[test]
    fn the_shape_is_the_committed_fixtures() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../../tests/fixtures/automation_claim_patterns.json")).expect("the fixture is JSON");
        assert_eq!(fixture["plain"].as_str(), Some(PLAIN_SHAPE));
    }

    #[test]
    fn every_builder_matches_the_plain_shape() {
        let feed = "twitchcon-rotterdam-2026-community-v1234";
        assert_eq!(feed.len(), 40);
        let family = format!("message:{}|post:{TWENTY}", message_part(&TextMatch::default()));
        for value in [
            chat_key(UUID),
            notice_key(UUID),
            badge_key(feed, BadgePhase::Announced),
            badge_key(feed, BadgePhase::Earnable),
            gap_key(TWENTY),
            rulecd_key(&family, TWENTY),
            person_key(&family, TWENTY, TWENTY),
            conv_key(TWENTY, TWENTY),
            pace_key(TWENTY),
            reminder_kw_key("reminder:0123456789abcdef", "twitch:a_very_long_login_name"),
        ] {
            assert_plain(&value);
        }
        for kind in RAIL_KINDS {
            let family = rail_family(kind);
            assert_plain(&family);
            assert!(family.len() > "rail:".len());
        }
        // A 200-character pattern of `|` and multibyte characters, in every mode.
        let pattern = "|\u{e9}".repeat(100);
        assert_eq!(pattern.chars().count(), 200);
        let class = ActionClass::Post { room_id: TWENTY.into() };
        for mode in [
            MatchMode::AnyWord,
            MatchMode::AllWords,
            MatchMode::Phrase,
            MatchMode::Contains,
            MatchMode::StartsWith,
            MatchMode::Exact,
            MatchMode::Regex,
        ] {
            for case_sensitive in [false, true] {
                let part = message_part(&TextMatch { mode, pattern: pattern.clone(), case_sensitive });
                assert_plain(&event_family(&format!("message:{part}"), &class));
            }
        }
        // Every P1 trigger's family, for chat, notice and badge facts.
        let facts = chat_facts();
        let notice =
            from_chat(&rows::usernotice(UUID, "me", ME, "fan", "300", "resub", &[("msg-param-cumulative-months", "3")])).unwrap();
        let badge = from_badge(&seen(feed, BadgePhase::Earnable, true, true));
        let events = Trigger::ChannelEvent {
            events: vec![EventKind::Resub],
            min_months: 0,
            min_gifts: 0,
            min_viewers: 0,
            min_bits: 0,
        };
        for (when, facts) in [
            (mention(), &facts),
            (Trigger::FirstMessage {}, &facts),
            (Trigger::Message { text: TextMatch::default() }, &facts),
            (events, &notice),
            (badge_rule().when, &badge),
        ] {
            let trigger = trigger_family(&when, facts).unwrap();
            assert_plain(&event_family(&trigger, &class));
            assert_plain(&facts.event_key);
        }
    }

    fn platform_rule(provider: &str, send_as_id: &str) -> Rule {
        Rule::from_value(&serde_json::json!({
            "id": "r-000000000001", "name": "Kick", "enabled": true, "live": true,
            "owner": { "provider": "twitch", "id": "100", "login": "me", "name": "Me" },
            "send_as": { "provider": provider, "id": send_as_id, "login": "mykick", "name": "MyKick" },
            "when": { "kind": "message", "text": { "mode": "contains", "pattern": "hello", "case_sensitive": false } },
            "only_if": { "platforms": [provider], "channels": { "kind": "any_open" } },
            "actions": [{ "kind": "reply", "id": "a-1", "texts": [{ "id": "l-1", "text": "hi" }], "pick": "random", "thread": false }],
            "limits": { "rule_cooldown_seconds": 30, "user_cooldown_seconds": 300, "max_per_hour": 30, "max_wait_seconds": 10 }
        }))
        .unwrap()
    }

    fn windowed_keys(els: &[Element]) -> Vec<String> {
        els.iter()
            .filter_map(|e| match e {
                Element::Windowed { key, .. } | Element::Counted { key, .. } => Some(key.clone()),
                _ => None,
            })
            .collect()
    }

    fn call_for(rule: &Rule, event_room: &str, event_key: &str, person: &str) -> Vec<Element> {
        let (plans, skipped) = classes_for(rule, event_room);
        assert!(skipped.is_empty());
        let prefs = AutomationPrefs::default();
        let family = event_family("message:contains|0|5:hello", &plans[0].class);
        let keys = vec![event_key.to_string()];
        class_elements(&CallInput {
            rule,
            prefs: &prefs,
            plan: &plans[0],
            family: &family,
            event_keys: &keys,
            event_room,
            person: Some(person),
        })
        .unwrap()
    }

    #[test]
    fn a_kick_rule_prefixes_every_non_twitch_id_in_every_class_and_rail_key() {
        let rule = platform_rule("kick", "5001");
        assert_eq!(classes_for(&rule, "kick:5002").0[0].class.key(), "post:kick:5002");
        let els = call_for(&rule, "kick:5002", "chat:kick:alpha:9b4c2f1e", "77");
        let fh = hash16(&event_family("message:contains|0|5:hello", &ActionClass::Post { room_id: "kick:5002".into() }));
        let keys = windowed_keys(&els);
        assert!(keys.contains(&"gap:kick:5002".to_string()), "another channel's gap rail: {keys:?}");
        assert!(keys.contains(&format!("rulecd:f:{fh}:kick:5002")), "{keys:?}");
        assert!(keys.contains(&format!("person:f:{fh}:kick:5001:kick:77")), "{keys:?}");
        assert!(keys.contains(&"conv:kick:5001:kick:77".to_string()), "{keys:?}");
        assert!(keys.contains(&"pace:100".to_string()), "the pace is the owner's, a Twitch id");
        assert!(els.iter().all(Element::plain_ok));
        // The account's own channel (broadcaster id = account id) has no gap rail.
        let own = call_for(&rule, "kick:5001", "chat:kick:mykick:9b4c2f1e", "77");
        assert!(!windowed_keys(&own).iter().any(|k| k.starts_with("gap:")));
    }

    #[test]
    fn a_kick_id_equal_to_a_twitch_id_shares_no_class_and_no_rail() {
        let twitch = platform_rule("twitch", "5001");
        let kick = platform_rule("kick", "5001");
        assert_ne!(classes_for(&twitch, "5001").0[0].class.key(), classes_for(&kick, "kick:5001").0[0].class.key());
        let a = windowed_keys(&call_for(&twitch, "5002", "chat:9b4c2f1e", "77"));
        let b = windowed_keys(&call_for(&kick, "kick:5002", "chat:kick:alpha:9b4c2f1e", "77"));
        for key in &a {
            if !key.starts_with("pace:") {
                assert!(!b.contains(key), "shared key {key}");
            }
        }
        // Twitch's keys are what P1 made: bare ids.
        assert!(a.iter().any(|k| k.ends_with(":5001:77")), "{a:?}");
        assert!(a.contains(&"conv:5001:77".to_string()), "{a:?}");
    }

    #[test]
    fn a_send_to_another_kick_chat_classes_by_that_chats_prefixed_room() {
        let mut rule = platform_rule("kick", "5001");
        rule.actions = vec![Action::Send {
            id: "a-1".into(),
            to: Some(crate::models::automations::ChannelRef {
                id: "c-1".into(),
                key: "kick:other".into(),
                room_id: "6006".into(),
                name: "other".into(),
            }),
            texts: vec![Line { id: "l-1".into(), text: "hi".into() }],
            pick: crate::models::automations::Pick::Random,
        }];
        let (plans, skipped) = classes_for(&rule, "kick:5002");
        assert!(skipped.is_empty());
        assert_eq!(plans[0].class.key(), "post:kick:6006");
    }

    #[test]
    fn a_kick_event_claims_a_windowed_key_and_a_twitch_event_stays_exact() {
        let rule = platform_rule("kick", "5001");
        let els = call_for(&rule, "kick:5002", "kick_evt:kick:alpha:sub:77:0123456789abcdef", "77");
        assert!(matches!(
            &els[0],
            Element::Event { mode: KeyMode::Windowed, window_s: Some(KICK_EVENT_WINDOW_S), .. }
        ));
        assert!(els[0].plain_ok());
        let chat = call_for(&rule, "kick:5002", "chat:kick:alpha:9b4c2f1e", "77");
        assert!(matches!(&chat[0], Element::Event { mode: KeyMode::Exact, window_s: None, .. }));
        assert_eq!(KICK_EVENT_WINDOW_S, 60);
    }

    #[test]
    fn the_new_event_kinds_have_family_words_and_pfx_keeps_twitch_bare() {
        for (kind, word) in [
            (EventKind::SuperChat, "super_chat"),
            (EventKind::SuperSticker, "super_sticker"),
            (EventKind::Membership, "membership"),
            (EventKind::MemberGift, "member_gift"),
            (EventKind::TikTokGift, "tiktok_gift"),
            (EventKind::TikTokFollow, "tiktok_follow"),
            (EventKind::TikTokShare, "tiktok_share"),
        ] {
            assert_eq!(event_word(kind), word);
            assert!(plain_ok(&format!("channel_event:{word}")));
        }
        assert_eq!(pfx("twitch", "100"), "100");
        assert_eq!(pfx("", "100"), "100");
        assert_eq!(pfx("Kick", "100"), "kick:100");
        assert_eq!(pfx("youtube", "UCabc"), "youtube:UCabc");
    }

    #[test]
    fn two_keyword_reminders_in_one_chat_have_two_rails() {
        let chat = "twitch:somebody";
        let hello = reminder_kw_key("reminder:1111111111111111", chat);
        let rules = reminder_kw_key("reminder:2222222222222222", chat);
        assert_ne!(hello, rules);
        assert_eq!(hello, format!("reminder_kw:f:{}:{chat}", hash16("reminder:1111111111111111")));
        // The same reminder in another chat is another rail too.
        assert_ne!(hello, reminder_kw_key("reminder:1111111111111111", "twitch:other"));
        assert!(plain_ok(&hello) && plain_ok(&rules));
    }

    #[test]
    fn families_tell_patterns_apart_and_fold_like_messages() {
        let regex = |p: &str, cs: bool| message_part(&TextMatch { mode: MatchMode::Regex, pattern: p.into(), case_sensitive: cs });
        assert_ne!(regex("a|b", false), regex("a", false));
        assert_ne!(regex("a|b", false), regex("b|", false));
        assert!(regex("a|b", false).starts_with("regex|0|3:"));
        assert_ne!(regex("a|b", false), regex("a|b", true));
        // A Regex pattern is kept as typed; the other modes fold.
        assert_eq!(regex("Hi", false), "regex|0|2:Hi");
        let contains =
            |p: &str, cs: bool| message_part(&TextMatch { mode: MatchMode::Contains, pattern: p.into(), case_sensitive: cs });
        assert_eq!(contains("Hi", false), "contains|0|2:hi");
        assert_eq!(contains("Hi", true), "contains|1|2:Hi");
        // One pattern typed composed or decomposed is one family.
        assert_eq!(contains("caf\u{e9}", false), contains("CAFE\u{301}", false));
        // A control character never reaches the site.
        assert!(plain_ok(&format!("message:{}", contains("a\tb", false))));
        // Two Message rules with different patterns are different families.
        assert_ne!(contains("hello", false), contains("hi", false));
        // A 201-character pattern is a field error, so no family grows past the shape.
        let mut long = rule(
            "r-long",
            Trigger::Message {
                text: TextMatch { mode: MatchMode::Regex, pattern: "a".repeat(201), case_sensitive: false },
            },
            vec![reply("x")],
        );
        assert!(validate(&mut long, &ValidateCtx::default()).iter().any(|e| e.path == "when.text.pattern"));
    }

    #[test]
    fn one_message_gives_one_key_on_every_device() {
        let row = rows::privmsg(UUID, "me", ME, "viewer", "300", "hi");
        let a = from_chat(&row).unwrap();
        let b = from_chat(&row.clone()).unwrap();
        assert_eq!(a.event_key, format!("chat:{UUID}"));
        assert_eq!(a.event_key, b.event_key);
        // Two compiles of the same rule give the same family.
        let first = crate::services::account_automations::compile::fixtures::compiled_with(
            &[rule("r", mention(), vec![reply("hi")])],
            RunState::On,
        );
        let second = crate::services::account_automations::compile::fixtures::compiled_with(
            &[rule("r", mention(), vec![reply("hi")])],
            RunState::On,
        );
        assert_eq!(
            trigger_family(&first.rules[0].rule.when, &a),
            trigger_family(&second.rules[0].rule.when, &b)
        );
    }

    #[test]
    fn a_shared_chat_row_and_its_partner_copy_are_one_event_and_one_class() {
        let origin = "11111111-2222-3333-4444-555555555555";
        // The row in its own room (500) carries its own id as source-id.
        let native = from_chat(&rows::shared(rows::privmsg(origin, "host", "500", "viewer", "300", "hi @me"), origin, "500")).unwrap();
        // The copy in the partner's room (600) has its own id and names the origin.
        let copy = from_chat(&rows::shared(
            rows::privmsg("66666666-7777-8888-9999-000000000000", "partner", "600", "viewer", "300", "hi @me"),
            origin,
            "500",
        ))
        .unwrap();
        assert_eq!(native.event_key, format!("chat:{origin}"));
        assert_eq!(native.event_key, copy.event_key);
        assert!(!native.shared_partner);
        assert!(copy.shared_partner);
        let r = rule("r", mention(), vec![reply("hi")]);
        let (a, _) = classes_for(&r, event_room(&r, &native));
        let (b, _) = classes_for(&r, event_room(&r, &copy));
        assert_eq!(a[0].class, ActionClass::Post { room_id: "500".into() });
        assert_eq!(a, b);
    }

    #[test]
    fn overlapping_rules_share_one_family_and_different_triggers_do_not() {
        let sub = from_chat(&rows::usernotice(UUID, "me", ME, "fan", "300", "sub", &[])).unwrap();
        let both = Trigger::ChannelEvent {
            events: vec![EventKind::Sub, EventKind::Resub],
            min_months: 0,
            min_gifts: 0,
            min_viewers: 0,
            min_bits: 0,
        };
        let one = Trigger::ChannelEvent { events: vec![EventKind::Sub], min_months: 0, min_gifts: 0, min_viewers: 0, min_bits: 0 };
        assert_eq!(trigger_family(&both, &sub), trigger_family(&one, &sub));
        assert_eq!(trigger_family(&both, &sub).as_deref(), Some("channel_event:sub"));
        // An event rule on a row with no event has no family.
        assert_eq!(trigger_family(&both, &chat_facts()), None);

        // Badge news for Both and for Announced: one key and one family for one sighting.
        let facts = from_badge(&seen("x-v1", BadgePhase::Announced, true, false));
        let announced = Trigger::BadgeDrop { on: BadgeLeg::Announced, link: BadgeLink::default(), batch_text: String::new() };
        assert_eq!(trigger_family(&badge_rule().when, &facts), trigger_family(&announced, &facts));
        assert_eq!(facts.event_key, "badge:x-v1:announced");

        // An away reply (mention) and a welcome (first message) never collide,
        // on the event or on a person rail.
        let class = ActionClass::Post { room_id: ME.into() };
        let facts = chat_facts();
        let away = event_family(&trigger_family(&mention(), &facts).unwrap(), &class);
        let welcome = event_family(&trigger_family(&Trigger::FirstMessage {}, &facts).unwrap(), &class);
        assert_ne!(away, welcome);
        assert_ne!(person_key(&away, ME, "300"), person_key(&welcome, ME, "300"));
        assert_eq!(away, format!("mention:|post:{ME}"));
    }

    #[test]
    fn actions_group_into_classes_by_where_they_post() {
        let facts = chat_facts();
        let r = rule(
            "r",
            mention(),
            vec![reply("hi {user}"), send("again"), send_to("other", "700", "there"), notify("x")],
        );
        let (plans, skipped) = classes_for(&r, event_room(&r, &facts));
        assert!(skipped.is_empty());
        assert_eq!(plans.len(), 2);
        assert_eq!(plans[0].class.key(), format!("post:{ME}"));
        assert_eq!(plans[0].actions, vec![0, 1]);
        assert!(plans[0].to_person);
        assert_eq!(plans[1].class.key(), "post:700");
        assert_eq!(plans[1].actions, vec![2]);
        assert!(!plans[1].to_person);

        // A Send naming the event's own chat joins the event's class.
        let same = rule("s", mention(), vec![send_to("me", ME, "hello {user}")]);
        let (plans, _) = classes_for(&same, ME);
        assert_eq!(plans[0].class.key(), format!("post:{ME}"));
        assert!(plans[0].to_person);

        // A Send that names nobody does not answer the person.
        let (plans, _) = classes_for(&rule("q", mention(), vec![send("hello all")]), ME);
        assert!(!plans[0].to_person);

        // A Send whose chat has no room id yet is skipped, never guessed.
        let (plans, skipped) = classes_for(&rule("n", mention(), vec![send_to("nobody", "", "x")]), ME);
        assert!(plans.is_empty());
        assert_eq!(skipped, vec![(0, NO_ROOM)]);

        // Notify alone has no class.
        assert!(classes_for(&rule("l", mention(), vec![notify("x")]), ME).0.is_empty());
    }

    fn elements_for(r: &Rule, facts: &Facts, prefs: &AutomationPrefs, plan: usize) -> Vec<Element> {
        let (plans, _) = classes_for(r, event_room(r, facts));
        let family = event_family(&trigger_family(&r.when, facts).unwrap(), &plans[plan].class);
        let keys = vec![facts.event_key.clone()];
        let person = (facts.kind != RowKind::Badge).then_some(facts.user_id.as_str());
        class_elements(&CallInput {
            rule: r,
            prefs,
            plan: &plans[plan],
            family: &family,
            event_keys: &keys,
            event_room: event_room(r, facts),
            person,
        })
        .unwrap()
    }

    fn rail<'a>(elements: &'a [Element], kind: &str) -> Option<&'a Element> {
        let family = rail_family(kind);
        elements.iter().find(|e| match e {
            Element::Windowed { family: f, .. } | Element::Counted { family: f, .. } => *f == family,
            _ => false,
        })
    }

    #[test]
    fn a_class_call_carries_its_event_and_its_rails() {
        let facts = chat_facts();
        let prefs = AutomationPrefs::default();
        let r = rule("r", mention(), vec![reply("hi"), reply("again")]);
        let els = elements_for(&r, &facts, &prefs, 0);
        let family = format!("mention:|post:{ME}");
        assert_eq!(
            els[0],
            Element::Event {
                key: format!("chat:{UUID}"),
                family: family.clone(),
                mode: KeyMode::Exact,
                window_s: None,
                sends: 2,
                to_person: true,
                settle_ms: 15_000,
            }
        );
        assert_eq!(els.iter().filter(|e| e.is_event()).count(), 1);
        let fh = hash16(&family);
        assert_eq!(
            rail(&els, "rulecd"),
            Some(&Element::Windowed { key: format!("rulecd:f:{fh}:{ME}"), family: "rail:rulecd".into(), window_s: 30 })
        );
        assert_eq!(
            rail(&els, "person"),
            Some(&Element::Windowed { key: format!("person:f:{fh}:{ME}:300"), family: "rail:person".into(), window_s: 300 })
        );
        assert_eq!(
            rail(&els, "conv"),
            Some(&Element::Counted {
                key: format!("conv:{ME}:300"),
                family: "rail:conv".into(),
                limit: 3,
                period_s: 300,
                counts: CountWho::Person,
            })
        );
        assert_eq!(
            rail(&els, "pace"),
            Some(&Element::Counted { key: format!("pace:{ME}"), family: "rail:pace".into(), limit: 10, period_s: 60, counts: CountWho::All })
        );
        // The owner's own chat has no gap rail.
        assert_eq!(rail(&els, "gap"), None);

        // Another chat's post carries the gap, judged by this device's own setting.
        let mut wide = AutomationPrefs::default();
        wide.other_channel_gap_seconds = 600;
        let other = rule("o", mention(), vec![send_to("other", "700", "hi")]);
        let els = elements_for(&other, &facts, &wide, 0);
        assert_eq!(rail(&els, "gap"), Some(&Element::Windowed { key: "gap:700".into(), family: "rail:gap".into(), window_s: 600 }));
        // That Send names nobody: no conversation rail, and its event does not answer the person.
        assert_eq!(rail(&els, "conv"), None);
        assert!(matches!(els[0], Element::Event { to_person: false, .. }));
        // The rule cooldown's chat is where the rule fired, not where it posts.
        let fh = hash16(&format!("mention:|post:700"));
        assert_eq!(rail(&els, "rulecd"), Some(&Element::Windowed { key: format!("rulecd:f:{fh}:{ME}"), family: "rail:rulecd".into(), window_s: 30 }));

        // Cooldowns of 0 send no rail; the pace limit follows the preference.
        let mut quiet = r.clone();
        quiet.limits.rule_cooldown_seconds = 0;
        quiet.limits.user_cooldown_seconds = 0;
        let mut fast = AutomationPrefs::default();
        fast.sends_per_minute = 20;
        let els = elements_for(&quiet, &facts, &fast, 0);
        assert_eq!(rail(&els, "rulecd"), None);
        assert_eq!(rail(&els, "person"), None);
        assert!(matches!(rail(&els, "pace"), Some(Element::Counted { limit: 20, .. })));
        // A longer wait lets the holder settle longer.
        let mut slow = r.clone();
        slow.limits.max_wait_seconds = 130;
        assert!(matches!(elements_for(&slow, &facts, &fast, 0)[0], Element::Event { settle_ms: 135_000, .. }));
    }

    #[test]
    fn a_badge_batch_counts_once_on_pace_and_has_no_rule_cooldown() {
        let r = badge_rule();
        let drops: Vec<BadgeSeen> = (0..30).map(|i| seen(&format!("b{i}-v1"), BadgePhase::Announced, true, false)).collect();
        let keys: Vec<String> = drops.iter().map(|d| badge_key(&d.feed_id, d.phase)).collect();
        let facts = from_badge(&drops[0]);
        let (plans, _) = classes_for(&r, event_room(&r, &facts));
        assert_eq!(plans[0].class.key(), format!("post:{ME}"));
        let family = event_family(&trigger_family(&r.when, &facts).unwrap(), &plans[0].class);
        assert_eq!(family, format!("badge:|post:{ME}"));
        let prefs = AutomationPrefs::default();
        let els = class_elements(&CallInput {
            rule: &r,
            prefs: &prefs,
            plan: &plans[0],
            family: &family,
            event_keys: &keys,
            event_room: ME,
            person: None,
        })
        .unwrap();
        assert_eq!(els.iter().filter(|e| matches!(e, Element::Event { sends: 0, to_person: false, .. })).count(), 30);
        assert_eq!(els.iter().filter(|e| **e == Element::Batch { sends: 1 }).count(), 1);
        assert_eq!(rail(&els, "rulecd"), None);
        assert_eq!(rail(&els, "person"), None);
        assert_eq!(rail(&els, "conv"), None);
        assert!(rail(&els, "pace").is_some());
        assert!(els.len() <= 60);
    }

    #[test]
    fn elements_serialize_with_the_fields_the_endpoint_reads() {
        use serde_json::json;
        let event = Element::Event {
            key: "chat:x".into(),
            family: "mention:|post:1".into(),
            mode: KeyMode::Exact,
            window_s: None,
            sends: 1,
            to_person: true,
            settle_ms: 15_000,
        };
        assert_eq!(
            serde_json::to_value(&event).unwrap(),
            json!({"kind": "event", "key": "chat:x", "family": "mention:|post:1", "mode": "exact", "sends": 1, "to_person": true, "settle_ms": 15000})
        );
        assert_eq!(serde_json::to_value(Element::Batch { sends: 1 }).unwrap(), json!({"kind": "batch", "sends": 1}));
        assert_eq!(
            serde_json::to_value(Element::Windowed { key: "gap:1".into(), family: "rail:gap".into(), window_s: 30 }).unwrap(),
            json!({"kind": "windowed", "key": "gap:1", "family": "rail:gap", "window_s": 30})
        );
        assert_eq!(
            serde_json::to_value(Element::Counted {
                key: "pace:1".into(),
                family: "rail:pace".into(),
                limit: 10,
                period_s: 60,
                counts: CountWho::All,
            })
            .unwrap(),
            json!({"kind": "counted", "key": "pace:1", "family": "rail:pace", "limit": 10, "period_s": 60, "counts": "all"})
        );
    }

    #[test]
    fn a_call_with_a_key_outside_the_shape_is_never_built() {
        let r = rule("r", mention(), vec![reply("hi")]);
        let (plans, _) = classes_for(&r, ME);
        let prefs = AutomationPrefs::default();
        let empty_key = vec!["chat:".to_string()];
        let input = CallInput {
            rule: &r,
            prefs: &prefs,
            plan: &plans[0],
            family: "mention:|post:100",
            event_keys: &empty_key,
            event_room: ME,
            person: Some("300"),
        };
        assert_eq!(class_elements(&input), Err(BUILD_FAILED));
        let no_keys: Vec<String> = Vec::new();
        assert_eq!(class_elements(&CallInput { event_keys: &no_keys, ..input }), Err(BUILD_FAILED));
    }

    #[test]
    fn p3_keys_are_the_plain_shape_and_spell_a_time_one_way() {
        for key in [
            follow_key("42", 1_794_546_000),
            reward_key("2c3b8a64-5f0e-4a39-9a62-0d3a8a5d1f11"),
            poll_key("p-1"),
            prediction_key("pr-1"),
            live_key("SomeStreamer", 1_794_546_000),
            offline_key("somestreamer", 1_794_546_000),
        ] {
            assert!(plain_ok(&key), "{key}");
        }
        assert_eq!(live_key("SomeStreamer", 7), live_key("somestreamer", 7), "a login is lowercased");
        assert_ne!(live_key("a", 7), offline_key("a", 7));
        assert_eq!(follow_key("42", 9), "follow:42:9");
        let uuid = "2c3b8a64-5f0e-4a39-9a62-0d3a8a5d1f11";
        let part = reward_part(Some(uuid));
        assert_eq!(part.len(), 16);
        assert!(!part.contains(uuid), "a UUID never enters a family as text");
        assert_eq!(reward_part(None), "any");
        assert_eq!(reward_part(Some("  ")), "any");
    }

    #[test]
    fn p3_families_name_their_trigger_and_class() {
        let facts = Facts::default();
        let class = ActionClass::Post { room_id: "100".into() };
        let fam = |t: &Trigger| event_family(&trigger_family(t, &facts).unwrap(), &class);
        assert_eq!(fam(&Trigger::Follow {}), "follow:|post:100");
        assert_eq!(fam(&Trigger::PollStarted {}), "poll:|post:100");
        assert_eq!(fam(&Trigger::PredictionStarted {}), "prediction:|post:100");
        assert_eq!(fam(&Trigger::LiveChange { online: true }), "live:on|post:100");
        assert_eq!(fam(&Trigger::LiveChange { online: false }), "live:off|post:100");
        assert_eq!(fam(&Trigger::RewardRedeemed { reward_id: None }), "reward:any|post:100");
        let one = fam(&Trigger::RewardRedeemed { reward_id: Some("r1".into()) });
        let other = fam(&Trigger::RewardRedeemed { reward_id: Some("r2".into()) });
        assert_ne!(one, other, "two rewards are two families");
        assert!(plain_ok(&one));
    }
}
