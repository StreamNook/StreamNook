//! What one chat row or badge sighting says, read once and shared by every
//! rule the row reaches.
//!
//! Pure: a row in, plain values out. The event key is the same on every
//! device that sees the event (Twitch message and notice ids and badge feed
//! ids are the same everywhere), which is what lets one device answer it. A
//! Shared Chat copy carries the origin's id in `source-id`, and the origin row
//! carries its own id there, so the row in its own room and every partner
//! copy give one key with no lookup.

use std::borrow::Cow;

use serde::Deserialize;

use super::badges::BadgeSeen;
use super::claims;
use crate::models::automations::{EventKind, SenderRole};
use crate::models::chat_layout::{Badge, ChatMessage};
use crate::services::chat_rules::channel_filter_key;

/// A row read off the socket longer ago than this never fires a rule.
pub const MAX_ROW_AGE_MS: i64 = 30_000;
/// Characters of the chatter's message `{message}` carries.
pub const MESSAGE_VAR_CHARS: usize = 200;

/// What kind of row the facts come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RowKind {
    /// A chat message; a cheer is one too.
    #[default]
    Message,
    /// A Twitch notice: subs, gifts, raids, announcements.
    Notice,
    /// A new badge from the badge feed.
    Badge,
    /// A whisper to the owner (P2).
    Whisper,
    /// A timer, clock, uptime or delay fire (P2): no person started it.
    Timed,
    /// A P3 event that is not a chat row: your own channel's follow,
    /// redemption, poll or prediction, or a watched channel going live or
    /// offline. `Facts.own` says which.
    Own,
}

/// The numbers of a sub, gift, raid, cheer or announcement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventFacts {
    pub kind: EventKind,
    /// Months subscribed (1 for a new sub).
    pub months: u32,
    /// Subs gifted.
    pub gifts: u32,
    /// Viewers a raid brought.
    pub viewers: u32,
    /// Bits cheered.
    pub bits: u32,
    /// "1", "2", "3" or "Prime"; empty when the event has none.
    pub tier: String,
}

/// One row as every rule reads it.
#[derive(Clone, Debug, Default)]
pub struct Facts {
    pub kind: RowKind,
    /// The event's claim key: `chat:<id>`, `notice:<id>` or `badge:<feed id>:<phase>`.
    pub event_key: String,
    /// `twitch:<login>` of the chat the row is in.
    pub channel_key: String,
    pub channel_login: String,
    /// The room the row is in (where a reply goes).
    pub room_id: String,
    /// The room the event belongs to: a Shared Chat copy's source room, else
    /// the row's own room. Post classes key on it.
    pub event_room_id: String,
    /// The row's own message id (a reply's parent).
    pub message_id: String,
    pub user_id: String,
    pub user_login: String,
    pub user_name: String,
    /// Never empty: Viewer when the chatter has no other role.
    pub roles: Vec<SenderRole>,
    /// The chatter's text as written.
    pub text: String,
    /// The text in NFC (case-sensitive matching).
    pub text_nfc: String,
    /// The text in NFC and lowercased (every other matching).
    pub text_folded: String,
    pub first_message: bool,
    /// A Shared Chat copy from a partner's chat.
    pub shared_partner: bool,
    /// The id of the person a reply answers; empty when it is no reply.
    pub reply_parent_user_id: String,
    pub event: Option<EventFacts>,
    pub badge: Option<BadgeSeen>,
    /// P3: what an own-channel event or a live change says; None for a chat
    /// row and a badge drop.
    pub own: Option<OwnFacts>,
    /// When the row was read off the socket (wall clock, ms); None for a row
    /// that did not arrive live.
    pub received_at_ms: Option<i64>,
    pub from_backfill: bool,
    /// The chat row, kept only while a running rule has an expression
    /// (`Compiled.expressions`); the saved-filter language reads it.
    pub message: Option<std::sync::Arc<ChatMessage>>,
}

/// `text` in Unicode NFC.
pub fn nfc(text: &str) -> Cow<'_, str> {
    icu_normalizer::ComposingNormalizerBorrowed::new_nfc().normalize(text)
}

/// `text` in NFC and lowercased: how patterns and messages compare when a
/// rule is not case-sensitive.
pub fn fold(text: &str) -> String {
    nfc(text).to_lowercase()
}

/// An online edge whose broadcast started longer ago than this (by Twitch's
/// clock) never fires: the answer came late, or the app slept.
pub const MAX_ONLINE_AGE_MS: i64 = 10 * 60 * 1000;

/// A `started_at` or `followed_at` as epoch seconds: RFC 3339, the parse
/// `reminder_service.rs:758` uses, so Helix `started_at`, GQL `createdAt` and
/// the watch seed spell one instant one way (PLAN § 4.3).
pub fn started_epoch(text: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(text.trim()).ok().map(|d| d.timestamp())
}

/// Whether an online edge is fresh: its broadcast started at most 10 minutes
/// ago by Twitch's clock (`twitch_now_ms`).
pub fn online_edge_fresh(started_s: i64, twitch_now_ms: i64) -> bool {
    twitch_now_ms - started_s * 1000 <= MAX_ONLINE_AGE_MS
}

/// Which P3 source an own-event row came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnKind {
    Follow,
    Reward,
    Poll,
    Prediction,
    /// A watched channel went live (`online`) or offline.
    Live { online: bool },
}

/// What a follow, redemption, poll, prediction or live change says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnFacts {
    pub kind: OwnKind,
    /// Whose channel the event happened in (empty for a live change).
    pub owner_id: String,
    pub reward_id: String,
    /// The reward's title.
    pub reward: String,
    /// What the viewer typed on a redemption, at most 200 characters.
    pub input: String,
    pub poll: String,
    pub prediction: String,
    /// A live change: the broadcast's start, epoch seconds.
    pub started_s: i64,
}

fn own_blank(kind: OwnKind, owner_id: &str) -> OwnFacts {
    OwnFacts {
        kind,
        owner_id: owner_id.to_string(),
        reward_id: String::new(),
        reward: String::new(),
        input: String::new(),
        poll: String::new(),
        prediction: String::new(),
        started_s: 0,
    }
}

fn field(event: &serde_json::Value, key: &str) -> String {
    event[key].as_str().unwrap_or("").trim().to_string()
}

/// The facts of one own-channel EventSub notification (`event` is the
/// payload's `event`). None when the event lacks what its claim key needs: a
/// follow with no readable `followed_at`, or any event with no id. The row is
/// in the owner's channel and room.
pub fn from_own(topic: super::own_topics::TopicKind, event: &serde_json::Value, owner_id: &str, owner_login: &str, now_ms: i64) -> Option<Facts> {
    use super::own_topics::TopicKind;
    let login = owner_login.trim().to_ascii_lowercase();
    let mut own;
    let event_key;
    let mut user = (String::new(), String::new(), String::new());
    match topic {
        TopicKind::Follow => {
            let user_id = field(event, "user_id");
            let followed = started_epoch(&field(event, "followed_at"))?;
            if user_id.is_empty() {
                return None;
            }
            event_key = claims::follow_key(&user_id, followed);
            user = (user_id, field(event, "user_login").to_ascii_lowercase(), field(event, "user_name"));
            own = own_blank(OwnKind::Follow, owner_id);
        }
        TopicKind::Redemption => {
            let id = field(event, "id");
            if id.is_empty() {
                return None;
            }
            event_key = claims::reward_key(&id);
            user = (field(event, "user_id"), field(event, "user_login").to_ascii_lowercase(), field(event, "user_name"));
            own = own_blank(OwnKind::Reward, owner_id);
            own.reward_id = field(&event["reward"], "id");
            own.reward = field(&event["reward"], "title");
            own.input = field(event, "user_input").chars().take(MESSAGE_VAR_CHARS).collect();
        }
        TopicKind::PollBegin => {
            let id = field(event, "id");
            if id.is_empty() {
                return None;
            }
            event_key = claims::poll_key(&id);
            own = own_blank(OwnKind::Poll, owner_id);
            own.poll = field(event, "title");
        }
        TopicKind::PredictionBegin => {
            let id = field(event, "id");
            if id.is_empty() {
                return None;
            }
            event_key = claims::prediction_key(&id);
            own = own_blank(OwnKind::Prediction, owner_id);
            own.prediction = field(event, "title");
        }
    }
    let user_name = if user.2.is_empty() { user.1.clone() } else { user.2.clone() };
    Some(Facts {
        kind: RowKind::Own,
        event_key,
        channel_key: channel_filter_key("twitch", &login),
        channel_login: login,
        room_id: owner_id.to_string(),
        event_room_id: owner_id.to_string(),
        user_id: user.0,
        user_login: user.1,
        user_name,
        roles: vec![SenderRole::Viewer],
        own: Some(own),
        received_at_ms: Some(now_ms),
        ..Facts::default()
    })
}

/// The facts of a watched channel going live or offline. `started_s` is the
/// broadcast's start (for an offline edge, the broadcast that ended).
pub fn from_live(login: &str, room_id: &str, online: bool, started_s: i64, now_ms: i64) -> Facts {
    let login = login.trim().to_ascii_lowercase();
    let mut own = own_blank(OwnKind::Live { online }, "");
    own.started_s = started_s;
    Facts {
        kind: RowKind::Own,
        event_key: if online { claims::live_key(&login, started_s) } else { claims::offline_key(&login, started_s) },
        channel_key: channel_filter_key("twitch", &login),
        channel_login: login,
        room_id: room_id.to_string(),
        event_room_id: room_id.to_string(),
        roles: vec![SenderRole::Viewer],
        own: Some(own),
        received_at_ms: Some(now_ms),
        ..Facts::default()
    }
}

fn tag<'a>(msg: &'a ChatMessage, key: &str) -> &'a str {
    msg.tags.get(key).map(String::as_str).unwrap_or("").trim()
}

/// The chatter's roles from their badges: broadcaster, moderator (and lead
/// moderator), VIP, subscriber (and founder); Viewer when none.
pub fn roles_of(badges: &[Badge]) -> Vec<SenderRole> {
    let mut roles = Vec::new();
    for badge in badges {
        let role = match badge.name.to_ascii_lowercase().as_str() {
            "broadcaster" => SenderRole::Broadcaster,
            "moderator" | "lead_moderator" => SenderRole::Moderator,
            "vip" => SenderRole::Vip,
            "subscriber" | "founder" => SenderRole::Subscriber,
            _ => continue,
        };
        if !roles.contains(&role) {
            roles.push(role);
        }
    }
    if roles.is_empty() {
        roles.push(SenderRole::Viewer);
    }
    roles
}

/// The facts of a live Twitch chat row or notice. None for another
/// platform's row, or a row with no room or no id (it could not be claimed).
pub fn from_chat(msg: &ChatMessage) -> Option<Facts> {
    let provider = msg.provider.trim();
    if !(provider.is_empty() || provider.eq_ignore_ascii_case("twitch")) {
        return super::provider_facts::from_provider(msg);
    }
    let room_id = tag(msg, "room-id");
    let own_id = match tag(msg, "id") {
        "" => msg.id.trim(),
        id => id,
    };
    let channel_login = msg.channel.trim().trim_start_matches('#').to_lowercase();
    if room_id.is_empty() || own_id.is_empty() || channel_login.is_empty() {
        return None;
    }
    let source_room = msg
        .metadata
        .source_room_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| tag(msg, "source-room-id"));
    let event_room_id = if source_room.is_empty() { room_id } else { source_room };
    let key_id = match tag(msg, "source-id") {
        "" => own_id,
        id => id,
    };
    // Every notice carries `system-msg`; a chat message never does.
    let notice = msg.tags.contains_key("system-msg") || msg.metadata.system_message.is_some();
    let user_login = match msg.username.trim() {
        "" => tag(msg, "login"),
        name => name,
    }
    .to_lowercase();
    let user_id = match msg.user_id.trim() {
        "" => tag(msg, "user-id"),
        id => id,
    }
    .to_string();
    let user_name = match msg.display_name.trim() {
        "" => user_login.clone(),
        name => name.to_string(),
    };
    let reply_parent_user_id = msg
        .metadata
        .reply_info
        .as_ref()
        .map(|r| r.parent_user_id.trim())
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| tag(msg, "reply-parent-user-id"))
        .to_string();
    let text_nfc = nfc(&msg.content).into_owned();
    Some(Facts {
        kind: if notice { RowKind::Notice } else { RowKind::Message },
        event_key: if notice { claims::notice_key(key_id) } else { claims::chat_key(key_id) },
        channel_key: channel_filter_key("twitch", &channel_login),
        channel_login,
        room_id: room_id.to_string(),
        event_room_id: event_room_id.to_string(),
        message_id: own_id.to_string(),
        user_id,
        user_login,
        user_name,
        roles: roles_of(&msg.badges),
        text: msg.content.clone(),
        text_folded: text_nfc.to_lowercase(),
        text_nfc,
        first_message: msg.metadata.is_first_message || tag(msg, "first-msg") == "1",
        shared_partner: msg.metadata.is_from_shared_chat,
        reply_parent_user_id,
        event: if notice { notice_event(msg) } else { cheer_event(msg) },
        badge: None,
        own: None,
        received_at_ms: msg.received_at_ms,
        from_backfill: msg.metadata.from_backfill,
        message: None,
    })
}

fn number(msg: &ChatMessage, key: &str) -> u32 {
    tag(msg, key).parse().unwrap_or(0)
}

fn tier(msg: &ChatMessage) -> String {
    match tag(msg, "msg-param-sub-plan") {
        "1000" => "1",
        "2000" => "2",
        "3000" => "3",
        plan if plan.eq_ignore_ascii_case("prime") => "Prime",
        _ => "",
    }
    .to_string()
}

/// A notice's event. One gift out of a gift bomb is no event of its own:
/// the bomb is thanked once.
pub(super) fn notice_event(msg: &ChatMessage) -> Option<EventFacts> {
    let kind = msg
        .metadata
        .msg_type
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| tag(msg, "msg-id"));
    let (kind, months, gifts, viewers) = match kind {
        "sub" => (EventKind::Sub, number(msg, "msg-param-cumulative-months").max(1), 0, 0),
        "resub" => (EventKind::Resub, number(msg, "msg-param-cumulative-months").max(1), 0, 0),
        "subgift" | "anonsubgift" => {
            if !tag(msg, "msg-param-community-gift-id").is_empty() {
                return None;
            }
            (EventKind::Gift, 0, 1, 0)
        }
        "submysterygift" | "anonsubmysterygift" => {
            (EventKind::GiftBomb, 0, number(msg, "msg-param-mass-gift-count").max(1), 0)
        }
        "raid" => (EventKind::Raid, 0, 0, number(msg, "msg-param-viewerCount")),
        "announcement" => (EventKind::Announcement, 0, 0, 0),
        _ => return None,
    };
    Some(EventFacts { kind, months, gifts, viewers, bits: 0, tier: tier(msg) })
}

/// A chat message with bits is a cheer.
fn cheer_event(msg: &ChatMessage) -> Option<EventFacts> {
    let bits = msg.metadata.bits_amount.unwrap_or_else(|| tag(msg, "bits").parse().unwrap_or(0));
    (bits > 0).then(|| EventFacts { kind: EventKind::Cheer, months: 0, gifts: 0, viewers: 0, bits, tier: String::new() })
}

/// The facts of one badge sighting.
pub fn from_badge(seen: &BadgeSeen) -> Facts {
    Facts {
        kind: RowKind::Badge,
        event_key: claims::badge_key(&seen.feed_id, seen.phase),
        roles: vec![SenderRole::Viewer],
        badge: Some(seen.clone()),
        received_at_ms: Some(seen.arrived_ms),
        ..Facts::default()
    }
}

/// A made-up row for Try it.
#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct TrySample {
    pub text: String,
    /// The chat's login.
    pub channel: String,
    /// The chat's room id, when the editor knows it.
    pub room_id: String,
    pub user_login: String,
    pub user_name: String,
    /// Empty = a plain viewer.
    pub roles: Vec<SenderRole>,
    pub first_message: bool,
    /// The account id the sample replies to, if any.
    pub reply_to: Option<String>,
    pub event: Option<EventKind>,
    /// Months, gifts, viewers or bits, for the event.
    pub amount: u32,
    /// The sample comes from a Shared Chat partner.
    pub shared_chat: bool,
}

/// The facts of a Try it sample. Its key is never claimed.
pub fn from_sample(sample: &TrySample) -> Facts {
    let channel_login = sample.channel.trim().trim_start_matches('#').to_lowercase();
    let user_login = sample.user_login.trim().trim_start_matches('@').to_lowercase();
    let room_id = match sample.room_id.trim() {
        "" => "try".to_string(),
        id => id.to_string(),
    };
    let event = sample.event.map(|kind| {
        let n = sample.amount;
        EventFacts {
            kind,
            months: if matches!(kind, EventKind::Sub | EventKind::Resub) { n.max(1) } else { 0 },
            gifts: if matches!(kind, EventKind::Gift | EventKind::GiftBomb) { n.max(1) } else { 0 },
            viewers: if kind == EventKind::Raid { n } else { 0 },
            bits: if kind == EventKind::Cheer { n } else { 0 },
            tier: String::new(),
        }
    });
    let kind = match sample.event {
        Some(kind) if kind != EventKind::Cheer => RowKind::Notice,
        _ => RowKind::Message,
    };
    let mut roles = sample.roles.clone();
    if roles.is_empty() {
        roles.push(SenderRole::Viewer);
    }
    let text_nfc = nfc(&sample.text).into_owned();
    Facts {
        kind,
        event_key: "chat:try".to_string(),
        channel_key: channel_filter_key("twitch", &channel_login),
        channel_login,
        room_id: room_id.clone(),
        event_room_id: room_id,
        message_id: "try".to_string(),
        user_id: format!("try-{user_login}"),
        user_name: match sample.user_name.trim() {
            "" => user_login.clone(),
            name => name.to_string(),
        },
        user_login,
        roles,
        text: sample.text.clone(),
        text_folded: text_nfc.to_lowercase(),
        text_nfc,
        first_message: sample.first_message,
        shared_partner: sample.shared_chat,
        reply_parent_user_id: sample.reply_to.clone().unwrap_or_default(),
        event,
        badge: None,
        own: None,
        received_at_ms: None,
        from_backfill: false,
        message: None,
    }
}

/// The facts of a whisper to the owner. `user_*` is the sender; the event key
/// is the whisper's id, the same on every device.
pub fn from_whisper(owner_id: &str, whisper_id: &str, from_id: &str, from_login: &str, from_name: &str, text: &str, now_ms: i64) -> Facts {
    let text_nfc = nfc(text).into_owned();
    Facts {
        kind: RowKind::Whisper,
        event_key: claims::whisper_key(whisper_id),
        channel_key: String::new(),
        channel_login: String::new(),
        room_id: owner_id.to_string(),
        event_room_id: owner_id.to_string(),
        message_id: whisper_id.to_string(),
        user_id: from_id.to_string(),
        user_login: from_login.to_lowercase(),
        user_name: if from_name.is_empty() { from_login.to_lowercase() } else { from_name.to_string() },
        roles: vec![SenderRole::Viewer],
        text: text.to_string(),
        text_folded: text_nfc.to_lowercase(),
        text_nfc,
        received_at_ms: Some(now_ms),
        ..Facts::default()
    }
}

/// The facts of a timed fire in one chat; `event_key` is built by the caller
/// (`claims::timer_key`, `clock_key`, `uptime_key`, `delay_key`).
pub fn from_timed(event_key: String, channel_login: &str, room_id: &str, now_ms: i64) -> Facts {
    Facts {
        kind: RowKind::Timed,
        event_key,
        channel_key: channel_filter_key("twitch", channel_login),
        channel_login: channel_login.to_lowercase(),
        room_id: room_id.to_string(),
        event_room_id: room_id.to_string(),
        roles: vec![SenderRole::Viewer],
        received_at_ms: Some(now_ms),
        ..Facts::default()
    }
}

/// Chat rows for the engine's tests.
#[cfg(test)]
pub(crate) mod rows {
    use std::collections::HashMap;

    use crate::models::chat_layout::{Badge, ChatMessage, MessageMetadata};

    pub(crate) fn badge(name: &str) -> Badge {
        Badge {
            name: name.to_string(),
            version: "1".to_string(),
            image_url_1x: None,
            image_url_2x: None,
            image_url_4x: None,
            title: None,
            description: None,
        }
    }

    /// A live chat row: `login` (user id `user_id`) says `text` in `channel`
    /// (room `room`), read at 1 s on the wall clock.
    pub(crate) fn privmsg(id: &str, channel: &str, room: &str, login: &str, user_id: &str, text: &str) -> ChatMessage {
        let tags: HashMap<String, String> = [("id", id), ("room-id", room), ("user-id", user_id), ("login", login)]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        ChatMessage {
            id: id.to_string(),
            user_id: user_id.to_string(),
            username: login.to_string(),
            display_name: login.to_string(),
            color: None,
            badges: Vec::new(),
            timestamp: "0".to_string(),
            content: text.to_string(),
            provider: "twitch".to_string(),
            channel: channel.to_string(),
            emotes: Vec::new(),
            tags,
            layout: Default::default(),
            segments: Vec::new(),
            metadata: MessageMetadata::default(),
            received_at_ms: Some(1_000),
        }
    }

    /// A notice of type `msg_id` with its `msg-param-*` tags.
    pub(crate) fn usernotice(
        id: &str,
        channel: &str,
        room: &str,
        login: &str,
        user_id: &str,
        msg_id: &str,
        params: &[(&str, &str)],
    ) -> ChatMessage {
        let mut m = privmsg(id, channel, room, login, user_id, "");
        m.metadata.msg_type = Some(msg_id.to_string());
        m.metadata.system_message = Some(String::new());
        m.tags.insert("msg-id".to_string(), msg_id.to_string());
        m.tags.insert("system-msg".to_string(), String::new());
        for (k, v) in params {
            m.tags.insert(k.to_string(), v.to_string());
        }
        m
    }

    /// `m` as a Shared Chat row whose origin is `source_id` in `source_room`.
    pub(crate) fn shared(mut m: ChatMessage, source_id: &str, source_room: &str) -> ChatMessage {
        let room = m.tags.get("room-id").cloned().unwrap_or_default();
        m.tags.insert("source-id".to_string(), source_id.to_string());
        m.tags.insert("source-room-id".to_string(), source_room.to_string());
        m.metadata.source_room_id = Some(source_room.to_string());
        m.metadata.is_from_shared_chat = room != source_room;
        m
    }
}

#[cfg(test)]
mod tests {
    use super::rows::{badge, privmsg, shared, usernotice};
    use super::*;
    use crate::services::account_automations::badges::{BadgeInfo, BadgePhase, Presence};

    const ID: &str = "9b4c2f1e-0d3a-4c55-8f6e-2b7a1c9d0e11";

    #[test]
    fn a_chat_row_reads_its_facts() {
        let mut row = privmsg(ID, "#Me", "100", "Viewer", "300", "Hello @me");
        row.badges = vec![badge("moderator"), badge("subscriber")];
        let f = from_chat(&row).unwrap();
        assert_eq!(f.kind, RowKind::Message);
        assert_eq!(f.event_key, format!("chat:{ID}"));
        assert_eq!(f.channel_key, "twitch:me");
        assert_eq!(f.channel_login, "me");
        assert_eq!((f.room_id.as_str(), f.event_room_id.as_str()), ("100", "100"));
        assert_eq!(f.message_id, ID);
        assert_eq!((f.user_login.as_str(), f.user_name.as_str(), f.user_id.as_str()), ("viewer", "Viewer", "300"));
        assert_eq!(f.roles, vec![SenderRole::Moderator, SenderRole::Subscriber]);
        assert_eq!(f.text, "Hello @me");
        assert_eq!(f.text_folded, "hello @me");
        assert_eq!(f.received_at_ms, Some(1_000));
        assert!(f.event.is_none());
        assert!(!f.first_message && !f.shared_partner && !f.from_backfill);
    }

    #[test]
    fn roles_come_from_badges() {
        let roles = |names: &[&str]| roles_of(&names.iter().map(|n| badge(n)).collect::<Vec<_>>());
        assert_eq!(roles(&[]), vec![SenderRole::Viewer]);
        assert_eq!(roles(&["glhf-pledge"]), vec![SenderRole::Viewer]);
        assert_eq!(roles(&["broadcaster"]), vec![SenderRole::Broadcaster]);
        assert_eq!(roles(&["lead_moderator"]), vec![SenderRole::Moderator]);
        assert_eq!(roles(&["vip", "founder"]), vec![SenderRole::Vip, SenderRole::Subscriber]);
        assert_eq!(roles(&["subscriber", "founder"]), vec![SenderRole::Subscriber]);
    }

    #[test]
    fn first_messages_replies_and_backfill_are_read() {
        let mut row = privmsg(ID, "me", "100", "new", "300", "hi");
        row.tags.insert("first-msg".to_string(), "1".to_string());
        row.tags.insert("reply-parent-user-id".to_string(), "100".to_string());
        row.metadata.from_backfill = true;
        let f = from_chat(&row).unwrap();
        assert!(f.first_message);
        assert_eq!(f.reply_parent_user_id, "100");
        assert!(f.from_backfill);
    }

    #[test]
    fn notices_read_their_numbers() {
        let notice = |msg_id: &str, params: &[(&str, &str)]| from_chat(&usernotice(ID, "me", "100", "fan", "300", msg_id, params)).unwrap();
        let resub = notice("resub", &[("msg-param-cumulative-months", "12"), ("msg-param-sub-plan", "2000")]);
        assert_eq!(resub.kind, RowKind::Notice);
        assert_eq!(resub.event_key, format!("notice:{ID}"));
        assert_eq!(
            resub.event,
            Some(EventFacts { kind: EventKind::Resub, months: 12, gifts: 0, viewers: 0, bits: 0, tier: "2".into() })
        );
        let prime = notice("sub", &[("msg-param-sub-plan", "Prime")]).event.unwrap();
        assert_eq!((prime.kind, prime.months, prime.tier.as_str()), (EventKind::Sub, 1, "Prime"));
        let bomb = notice("submysterygift", &[("msg-param-mass-gift-count", "5"), ("msg-param-sub-plan", "1000")]).event.unwrap();
        assert_eq!((bomb.kind, bomb.gifts, bomb.tier.as_str()), (EventKind::GiftBomb, 5, "1"));
        let gift = notice("subgift", &[]).event.unwrap();
        assert_eq!((gift.kind, gift.gifts), (EventKind::Gift, 1));
        let raid = notice("raid", &[("msg-param-viewerCount", "120")]).event.unwrap();
        assert_eq!((raid.kind, raid.viewers), (EventKind::Raid, 120));
        assert_eq!(notice("announcement", &[]).event.unwrap().kind, EventKind::Announcement);
        assert!(notice("ritual", &[]).event.is_none());
        // One gift out of a gift bomb is thanked by the bomb, never on its own.
        assert!(notice("subgift", &[("msg-param-community-gift-id", "777")]).event.is_none());
    }

    #[test]
    fn a_cheer_is_a_chat_message_with_bits() {
        let mut row = privmsg(ID, "me", "100", "fan", "300", "Cheer500 hi");
        row.metadata.bits_amount = Some(500);
        let f = from_chat(&row).unwrap();
        assert_eq!(f.kind, RowKind::Message);
        assert_eq!(f.event_key, format!("chat:{ID}"));
        assert_eq!(f.event.unwrap().bits, 500);
        let mut tagged = privmsg(ID, "me", "100", "fan", "300", "Cheer100");
        tagged.tags.insert("bits".to_string(), "100".to_string());
        assert_eq!(from_chat(&tagged).unwrap().event.unwrap().kind, EventKind::Cheer);
    }

    #[test]
    fn shared_chat_rows_key_on_the_origin() {
        let origin = "11111111-2222-3333-4444-555555555555";
        let native = from_chat(&shared(privmsg(origin, "host", "500", "fan", "300", "hi"), origin, "500")).unwrap();
        let copy = from_chat(&shared(privmsg("other-copy-id", "partner", "600", "fan", "300", "hi"), origin, "500")).unwrap();
        assert_eq!(native.event_key, copy.event_key);
        assert_eq!(copy.room_id, "600");
        assert_eq!(copy.event_room_id, "500");
        assert_eq!(copy.message_id, "other-copy-id");
        assert!(copy.shared_partner && !native.shared_partner);
        // A shared notice copy keys on the origin too.
        let notice = from_chat(&shared(usernotice("n-copy", "partner", "600", "fan", "300", "raid", &[]), "n-origin", "500")).unwrap();
        assert_eq!(notice.event_key, "notice:n-origin");
    }

    #[test]
    fn rows_that_cannot_be_claimed_have_no_facts() {
        let mut kick = privmsg(ID, "me", "100", "fan", "300", "hi");
        kick.provider = "kick".to_string();
        assert!(from_chat(&kick).is_none());
        let mut no_room = privmsg(ID, "me", "100", "fan", "300", "hi");
        no_room.tags.remove("room-id");
        assert!(from_chat(&no_room).is_none());
        let mut no_id = privmsg("", "me", "100", "fan", "300", "hi");
        no_id.tags.remove("id");
        assert!(from_chat(&no_id).is_none());
    }

    #[test]
    fn folding_is_nfc_and_lowercase() {
        assert_eq!(fold("Caf\u{e9}"), fold("CAFE\u{301}"));
        assert_eq!(fold("Caf\u{e9}"), "caf\u{e9}");
        assert_eq!(nfc("e\u{301}"), "\u{e9}");
    }

    #[test]
    fn badge_and_sample_facts() {
        let seen = BadgeSeen {
            badge: BadgeInfo { set_id: "x".into(), version: "1".into(), ..BadgeInfo::default() },
            feed_id: "x-v1".into(),
            phase: BadgePhase::Earnable,
            announced: true,
            earnable: true,
            ts_ms: 5,
            arrived_ms: 9,
            push: true,
            in_gallery: Presence::Unknown,
        };
        let f = from_badge(&seen);
        assert_eq!(f.kind, RowKind::Badge);
        assert_eq!(f.event_key, "badge:x-v1:earnable");
        assert_eq!(f.received_at_ms, Some(9));

        let sample = TrySample {
            text: "hi @me".into(),
            channel: "#Me".into(),
            user_login: "@Fan".into(),
            event: Some(EventKind::Raid),
            amount: 50,
            ..TrySample::default()
        };
        let f = from_sample(&sample);
        assert_eq!(f.kind, RowKind::Notice);
        assert_eq!(f.channel_key, "twitch:me");
        assert_eq!(f.user_login, "fan");
        assert_eq!(f.roles, vec![SenderRole::Viewer]);
        assert_eq!(f.event.unwrap().viewers, 50);
        let cheer = from_sample(&TrySample { event: Some(EventKind::Cheer), amount: 100, ..TrySample::default() });
        assert_eq!(cheer.kind, RowKind::Message);
    }

    #[test]
    fn started_at_spells_one_instant_one_way() {
        let a = started_epoch("2026-11-13T05:00:00Z").unwrap();
        assert_eq!(started_epoch("2026-11-13T05:00:00.000Z"), Some(a));
        assert_eq!(started_epoch("2026-11-13T07:00:00+02:00"), Some(a));
        assert_eq!(started_epoch(" 2026-11-13T05:00:00Z "), Some(a));
        assert_eq!(started_epoch(""), None);
        assert_eq!(started_epoch("yesterday"), None);
    }

    #[test]
    fn an_online_edge_is_fresh_for_ten_minutes_by_twitch_time() {
        let started = started_epoch("2026-11-13T05:00:00Z").unwrap();
        assert!(online_edge_fresh(started, started * 1000 + MAX_ONLINE_AGE_MS));
        assert!(!online_edge_fresh(started, started * 1000 + MAX_ONLINE_AGE_MS + 1));
        assert!(online_edge_fresh(started, started * 1000 - 5_000), "a start a little ahead of our clock is fine");
    }

    #[test]
    fn own_events_read_their_payloads() {
        use crate::services::account_automations::own_topics::TopicKind;
        let follow = serde_json::json!({ "user_id": "7", "user_login": "Fan", "user_name": "Fan", "followed_at": "2026-11-13T05:00:00Z" });
        let f = from_own(TopicKind::Follow, &follow, "100", "Me", 5).unwrap();
        assert_eq!(f.event_key, format!("follow:7:{}", started_epoch("2026-11-13T05:00:00Z").unwrap()));
        assert_eq!((f.channel_key.as_str(), f.room_id.as_str(), f.user_login.as_str()), ("twitch:me", "100", "fan"));
        assert_eq!(f.kind, RowKind::Own);
        assert!(from_own(TopicKind::Follow, &serde_json::json!({ "user_id": "7" }), "100", "me", 5).is_none(), "no readable followed_at, no key");

        let redeem = serde_json::json!({ "id": "red-1", "user_id": "8", "user_login": "viewer", "user_name": "Viewer",
            "user_input": "hello", "reward": { "id": "rw-1", "title": "Hydrate" } });
        let r = from_own(TopicKind::Redemption, &redeem, "100", "me", 5).unwrap();
        assert_eq!(r.event_key, "reward:red-1");
        let own = r.own.unwrap();
        assert_eq!((own.reward_id.as_str(), own.reward.as_str(), own.input.as_str()), ("rw-1", "Hydrate", "hello"));
        let long = serde_json::json!({ "id": "red-2", "user_input": "x".repeat(500), "reward": {} });
        assert_eq!(from_own(TopicKind::Redemption, &long, "100", "me", 5).unwrap().own.unwrap().input.chars().count(), MESSAGE_VAR_CHARS);

        let poll = serde_json::json!({ "id": "p1", "title": "Pizza?" });
        assert_eq!(from_own(TopicKind::PollBegin, &poll, "100", "me", 5).unwrap().event_key, "poll:p1");
        assert_eq!(from_own(TopicKind::PredictionBegin, &poll, "100", "me", 5).unwrap().event_key, "prediction:p1");
        assert!(from_own(TopicKind::PollBegin, &serde_json::json!({}), "100", "me", 5).is_none());

        let up = from_live("Friend", "555", true, 1_794_546_000, 5);
        assert_eq!((up.event_key.as_str(), up.channel_key.as_str(), up.room_id.as_str()), ("live:friend:1794546000", "twitch:friend", "555"));
        assert_eq!(from_live("Friend", "555", false, 1_794_546_000, 5).event_key, "offline:friend:1794546000");
    }
}
