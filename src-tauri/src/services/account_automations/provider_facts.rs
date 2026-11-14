//! What a row of another platform (Kick, YouTube, TikTok) says, who you are
//! on each platform, and the Kick moderation rail.
//!
//! The provider is read from `facts.channel_key` (`kick:alpha`), so `Facts`
//! has no field for it. A Kick chat row with a payload message id claims
//! `chat:<chat key>:<id>`; a Kick event, which carries no shared id, claims
//! `kick_evt:<chat key>:<kind>:<actor>:<16 hex of the normalized payload>`
//! for 60 s (D278). A row Kick made up on this device (`kick-local-`) keeps
//! its local id, and the executor refuses to run any outward action for it.
//! YouTube and TikTok rules only notify, so their keys are never claimed.

use crate::models::automations::{AccountRef, EventKind, SenderRole};
use crate::models::chat_layout::ChatMessage;
use crate::services::providers::key;
use crate::services::providers::kick::{self, EVENT_PAYLOAD_TAG};

use super::claims::{hash16, pfx};
use super::compile::Compiled;
use super::facts::{nfc, roles_of, EventFacts, Facts, RowKind};

pub const KICK: &str = "kick";
pub const YOUTUBE: &str = "youtube";
pub const TIKTOK: &str = "tiktok";

/// Your id and login on one platform's chat (`ChatRules::platform_identity`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlatformId {
    pub provider: String,
    /// The chat user id; empty when only the name is known.
    pub id: String,
    pub login: String,
}

impl PlatformId {
    /// The chat key of this account's own channel, when it has one: a YouTube
    /// channel is named by its id, the others by their name.
    pub fn own_chat_key(&self) -> Option<String> {
        let channel = if self.provider == YOUTUBE {
            self.id.trim()
        } else {
            self.login.trim()
        };
        (!channel.is_empty()).then(|| key::make_key(&self.provider, channel))
    }
}

/// The row's platform when the engine reads it.
pub fn provider_of(msg: &ChatMessage) -> Option<&'static str> {
    match msg.provider.trim().to_ascii_lowercase().as_str() {
        KICK => Some(KICK),
        YOUTUBE => Some(YOUTUBE),
        TIKTOK => Some(TIKTOK),
        _ => None,
    }
}

/// A Twitch row; a row with no provider is Twitch's.
pub fn is_twitch(msg: &ChatMessage) -> bool {
    let provider = msg.provider.trim();
    provider.is_empty() || provider.eq_ignore_ascii_case("twitch")
}

/// A Twitch account; an account with no provider is Twitch's.
pub fn is_twitch_account(account: &AccountRef) -> bool {
    let provider = account.provider.trim();
    provider.is_empty() || provider.eq_ignore_ascii_case("twitch")
}

fn tag<'a>(msg: &'a ChatMessage, name: &str) -> &'a str {
    msg.tags.get(name).map(String::as_str).unwrap_or("").trim()
}

/// `kick_evt:<chat key>:<kind>:<actor id or login>:<16 hex>`; an empty actor
/// reads `-`.
pub fn kick_event_key(chat_key: &str, kind: &str, actor: &str, payload_hex: &str) -> String {
    let actor = if actor.is_empty() { "-" } else { actor };
    format!("kick_evt:{chat_key}:{kind}:{actor}:{payload_hex}")
}

/// A Kick channel's room id: its broadcaster's user id, once the channel is
/// resolved. YouTube and TikTok rows are never claimed, so the channel stands
/// in for a room.
fn room_of(provider: &str, channel: &str) -> String {
    match provider {
        KICK => kick::channel_meta(channel)
            .and_then(|m| m.user_id)
            .map(|id| id.to_string())
            .unwrap_or_default(),
        _ => channel.to_string(),
    }
}

fn provider_event(kind: &str) -> Option<EventFacts> {
    let kind = match kind {
        "superchat" => EventKind::SuperChat,
        "supersticker" => EventKind::SuperSticker,
        "membership" => EventKind::Membership,
        "membergift" => EventKind::MemberGift,
        "tiktok_gift" => EventKind::TikTokGift,
        "tiktok_follow" => EventKind::TikTokFollow,
        "tiktok_share" => EventKind::TikTokShare,
        _ => return None,
    };
    Some(EventFacts {
        kind,
        months: 0,
        gifts: 0,
        viewers: 0,
        bits: 0,
        tier: String::new(),
    })
}

/// The facts of a Kick, YouTube or TikTok row. None for another provider's
/// row, a row whose channel names another platform, a row with no id, or a
/// Kick row whose channel is not resolved yet (it could not be claimed).
pub fn from_provider(msg: &ChatMessage) -> Option<Facts> {
    let provider = provider_of(msg)?;
    let parsed = key::parse_key(msg.channel.trim());
    let channel = parsed.channel.trim().to_string();
    if parsed.provider != provider || channel.is_empty() {
        return None;
    }
    let channel_key = key::make_key(provider, &channel);
    let own_id = match tag(msg, "id") {
        "" => msg.id.trim(),
        id => id,
    };
    if own_id.is_empty() {
        return None;
    }
    let room_id = match tag(msg, "room-id") {
        "" => room_of(provider, &channel),
        id => id.to_string(),
    };
    if room_id.is_empty() {
        return None;
    }
    let notice = msg.tags.contains_key("system-msg") || msg.metadata.system_message.is_some();
    let user_login = match msg.username.trim() {
        "" => tag(msg, "login"),
        name => name,
    }
    .trim_start_matches('@')
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
    let kind_word = msg
        .metadata
        .msg_type
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| tag(msg, "msg-id"))
        .to_ascii_lowercase();
    let actor = if user_id.is_empty() {
        user_login.clone()
    } else {
        user_id.clone()
    };
    let event_key = if notice && provider == KICK {
        let payload = match tag(msg, EVENT_PAYLOAD_TAG) {
            "" => msg.metadata.system_message.clone().unwrap_or_default(),
            payload => payload.to_string(),
        };
        kick_event_key(&channel_key, &kind_word, &actor, &hash16(&payload))
    } else {
        format!("chat:{channel_key}:{own_id}")
    };
    let event = match (notice, provider) {
        (false, _) => None,
        (true, KICK) => super::facts::notice_event(msg),
        (true, _) => provider_event(&kind_word),
    };
    let text_nfc = nfc(&msg.content).into_owned();
    Some(Facts {
        kind: if notice {
            RowKind::Notice
        } else {
            RowKind::Message
        },
        event_key,
        channel_key,
        channel_login: channel,
        room_id: room_id.clone(),
        event_room_id: pfx(provider, &room_id),
        message_id: own_id.to_string(),
        user_id,
        user_login,
        user_name,
        roles: roles_of(&msg.badges),
        text: msg.content.clone(),
        text_folded: text_nfc.to_lowercase(),
        text_nfc,
        reply_parent_user_id: msg
            .metadata
            .reply_info
            .as_ref()
            .map(|r| r.parent_user_id.trim().to_string())
            .unwrap_or_default(),
        event,
        received_at_ms: msg.received_at_ms,
        from_backfill: msg.metadata.from_backfill,
        ..Facts::default()
    })
}

/// One of your own rows on its platform: its chat id is one of yours (the
/// platform's identity, or an account a rule names), else, when the identity
/// has no id yet, its name is yours (the rule the chat rules use).
pub fn is_own_row(compiled: &Compiled, provider: &str, msg: &ChatMessage, me: &PlatformId) -> bool {
    let id = msg.user_id.trim();
    if !id.is_empty() && compiled.is_mine_on(provider, id) {
        return true;
    }
    if !me.id.is_empty() {
        return id == me.id;
    }
    let login = me.login.trim().trim_start_matches('@').to_lowercase();
    !login.is_empty()
        && [msg.username.as_str(), msg.display_name.as_str()]
            .iter()
            .any(|name| name.trim().trim_start_matches('@').to_lowercase() == login)
}

/// A Kick Timeout or Delete never lands on one of your accounts, the
/// streamer, a moderator or a VIP (the Twitch target rail, with Kick's ids).
pub fn kick_target_rail(compiled: &Compiled, facts: &Facts) -> Result<(), &'static str> {
    if compiled.is_mine_on(KICK, &facts.user_id) {
        return Err("Skipped: that is one of your accounts");
    }
    if facts.roles.iter().any(|r| {
        matches!(
            r,
            SenderRole::Broadcaster | SenderRole::Moderator | SenderRole::Vip
        )
    }) {
        return Err("Skipped: the streamer, moderators and VIPs are never timed out or deleted");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::automations::{AutomationPrefs, Automations, RunState, Trigger};
    use crate::services::account_automations::claims;
    use crate::services::account_automations::compile::{compile_with, CompileInput};
    use crate::services::account_automations::matcher::{self, Ctx, LocalTime};
    use serde_json::{json, Value};

    const MINE: &str = "5001";

    fn row(
        provider: &str,
        channel: &str,
        id: &str,
        user_id: &str,
        login: &str,
        text: &str,
        room: &str,
    ) -> ChatMessage {
        let mut tags = json!({ "id": id });
        if !room.is_empty() {
            tags["room-id"] = json!(room);
        }
        serde_json::from_value(json!({
            "id": id, "user_id": user_id, "username": login, "display_name": login, "color": null,
            "badges": [], "timestamp": "", "content": text, "provider": provider, "channel": channel,
            "tags": tags, "metadata": {}, "received_at_ms": 1,
        }))
        .expect("a test row")
    }

    fn kick_row(id: &str, user_id: &str, text: &str) -> ChatMessage {
        row("kick", "kick:alpha", id, user_id, "fan", text, "5002")
    }

    fn event_row(
        provider: &str,
        msg_id: &str,
        id: &str,
        user_id: &str,
        payload: &str,
        extra: &[(&str, &str)],
    ) -> ChatMessage {
        let mut msg = row(
            provider,
            &format!("{provider}:alpha"),
            id,
            user_id,
            "fan",
            "",
            "5002",
        );
        msg.tags.insert("msg-id".into(), msg_id.into());
        msg.tags
            .insert("system-msg".into(), "fan did a thing".into());
        if !user_id.is_empty() {
            msg.tags.insert("user-id".into(), user_id.into());
        }
        if !payload.is_empty() {
            msg.tags.insert(EVENT_PAYLOAD_TAG.into(), payload.into());
        }
        for (k, v) in extra {
            msg.tags.insert((*k).into(), (*v).into());
        }
        msg.metadata.msg_type = Some(msg_id.into());
        msg.metadata.system_message = Some("fan did a thing".into());
        msg
    }

    fn platform_rule(
        id: &str,
        provider: &str,
        send_as: &str,
        when: Value,
        channels: Value,
        actions: Value,
    ) -> Value {
        json!({
            "id": id, "name": id, "enabled": true, "live": true,
            "owner": { "provider": "twitch", "id": "100", "login": "me", "name": "Me" },
            "send_as": { "provider": provider, "id": send_as, "login": "mykick", "name": "MyKick" },
            "when": when,
            "only_if": { "platforms": [provider], "channels": channels },
            "actions": actions,
            "limits": { "rule_cooldown_seconds": 0, "user_cooldown_seconds": 0, "max_per_hour": 120, "max_wait_seconds": 10 },
        })
    }

    fn reply_action() -> Value {
        json!([{ "kind": "reply", "id": "a-1", "texts": [{ "id": "l-1", "text": "hi {user}" }], "pick": "random", "thread": false }])
    }

    fn notify_action() -> Value {
        json!([{ "kind": "notify", "id": "a-1", "title": "T", "body": "B", "toast": true, "system": false, "sound": null, "volume": 1.0 }])
    }

    fn kick_chats() -> Value {
        json!({ "kind": "these", "list": [{ "id": "c-1", "key": "kick:alpha", "room_id": "5002", "name": "alpha" }] })
    }

    fn mention() -> Value {
        json!({ "kind": "mention", "replies_too": true })
    }

    fn compiled(rules: Vec<Value>, ids: &[PlatformId]) -> Compiled {
        let automations = Automations {
            run: RunState::On,
            rules,
            ..Automations::default()
        };
        let prefs = AutomationPrefs::default();
        let own = vec![("100".to_string(), "me".to_string())];
        compile_with(
            &CompileInput {
                automations: &automations,
                prefs: &prefs,
                own_accounts: &own,
                profile_user_id: None,
                primary_id: None,
            },
            ids,
        )
    }

    fn kick_id() -> PlatformId {
        PlatformId {
            provider: "kick".into(),
            id: MINE.into(),
            login: "mykick".into(),
        }
    }

    fn fired(c: &Compiled, facts: &Facts) -> Vec<bool> {
        let ctx = Ctx {
            away: None,
            local: LocalTime::from_naive(chrono::Local::now().naive_local()),
            trace: false,
        };
        matcher::evaluate(c, facts, &ctx)
            .into_iter()
            .map(|v| v.fired)
            .collect()
    }

    // ---- facts ------------------------------------------------------------------------

    #[test]
    fn a_kick_chat_row_claims_the_chat_key_and_its_room_is_prefixed() {
        let f = from_provider(&kick_row("9b4c2f1e", "77", "hello")).unwrap();
        assert_eq!(f.channel_key, "kick:alpha");
        assert_eq!(f.event_key, "chat:kick:alpha:9b4c2f1e");
        assert_eq!(
            (f.room_id.as_str(), f.event_room_id.as_str()),
            ("5002", "kick:5002")
        );
        assert_eq!(
            (f.user_id.as_str(), f.user_login.as_str(), f.kind),
            ("77", "fan", RowKind::Message)
        );
        assert!(claims::plain_ok(&f.event_key));
        // The row goes through the shared reader too.
        assert_eq!(
            crate::services::account_automations::facts::from_chat(&kick_row(
                "9b4c2f1e", "77", "hello"
            ))
            .unwrap()
            .event_key,
            f.event_key
        );
    }

    #[test]
    fn a_row_with_no_resolved_room_no_id_or_another_platforms_channel_has_no_facts() {
        assert!(
            from_provider(&row("kick", "kick:alpha", "m1", "77", "fan", "hi", "")).is_none(),
            "no room id and no cached meta"
        );
        assert!(from_provider(&row("kick", "kick:alpha", "", "77", "fan", "hi", "5002")).is_none());
        assert!(from_provider(&row(
            "kick",
            "twitch:alpha",
            "m1",
            "77",
            "fan",
            "hi",
            "5002"
        ))
        .is_none());
        assert!(
            from_provider(&row("kick", "alpha", "m1", "77", "fan", "hi", "5002")).is_none(),
            "a bare name reads as Twitch"
        );
        assert!(from_provider(&row(
            "rumble",
            "rumble:alpha",
            "m1",
            "77",
            "fan",
            "hi",
            "5002"
        ))
        .is_none());
        assert!(from_provider(&row(
            "twitch",
            "twitch:alpha",
            "m1",
            "77",
            "fan",
            "hi",
            "5002"
        ))
        .is_none());
    }

    #[test]
    fn a_made_up_kick_id_stays_local_and_never_looks_shared() {
        let f = from_provider(&kick_row("kick-local-3", "77", "hello")).unwrap();
        assert!(f.message_id.starts_with(kick::KICK_LOCAL_ID));
        assert_eq!(f.event_key, "chat:kick:alpha:kick-local-3");
    }

    #[test]
    fn the_same_kick_event_on_two_devices_gives_one_windowed_key() {
        let device_a = from_provider(&event_row(
            "kick",
            "sub",
            "kick-evt-1",
            "77",
            "sub|1",
            &[("msg-param-cumulative-months", "1")],
        ))
        .unwrap();
        let device_b = from_provider(&event_row(
            "kick",
            "sub",
            "kick-evt-9",
            "77",
            "sub|1",
            &[("msg-param-cumulative-months", "1")],
        ))
        .unwrap();
        assert_eq!(
            device_a.event_key, device_b.event_key,
            "local counters never enter the key"
        );
        assert!(
            device_a
                .event_key
                .starts_with("kick_evt:kick:alpha:sub:77:"),
            "{}",
            device_a.event_key
        );
        assert!(claims::plain_ok(&device_a.event_key));
        // A real second gift by the same gifter is another payload: another key.
        let first = from_provider(&event_row(
            "kick",
            "subgift",
            "kick-evt-2",
            "77",
            "subgift|bob",
            &[],
        ))
        .unwrap();
        let second = from_provider(&event_row(
            "kick",
            "subgift",
            "kick-evt-3",
            "77",
            "subgift|carol",
            &[],
        ))
        .unwrap();
        assert_ne!(first.event_key, second.event_key);
        // Another actor or kind is another key; an empty actor reads `-` and still separates payloads.
        let other_actor =
            from_provider(&event_row("kick", "sub", "kick-evt-4", "78", "sub|1", &[])).unwrap();
        assert_ne!(device_a.event_key, other_actor.event_key);
        let nobody_a = from_provider(&event_row(
            "kick",
            "subgift",
            "kick-evt-5",
            "",
            "subgift|bob",
            &[],
        ))
        .unwrap();
        let nobody_b = from_provider(&event_row(
            "kick",
            "subgift",
            "kick-evt-6",
            "",
            "subgift|carol",
            &[],
        ))
        .unwrap();
        assert!(
            nobody_a.event_key.contains(":subgift:fan:")
                || nobody_a.event_key.contains(":subgift:-:"),
            "{}",
            nobody_a.event_key
        );
        assert_ne!(nobody_a.event_key, nobody_b.event_key);
        let mut anonymous = event_row("kick", "subgift", "kick-evt-7", "", "subgift|bob", &[]);
        anonymous.username = String::new();
        anonymous.display_name = String::new();
        anonymous.tags.remove("login");
        assert!(from_provider(&anonymous)
            .unwrap()
            .event_key
            .contains(":subgift:-:"));
    }

    #[test]
    fn kick_events_read_like_twitch_events_and_the_other_platforms_have_their_own() {
        let sub = from_provider(&event_row(
            "kick",
            "resub",
            "kick-evt-1",
            "77",
            "resub|5",
            &[("msg-param-cumulative-months", "5")],
        ))
        .unwrap();
        let e = sub.event.unwrap();
        assert_eq!((e.kind, e.months), (EventKind::Resub, 5));
        let bomb = from_provider(&event_row(
            "kick",
            "submysterygift",
            "kick-evt-2",
            "77",
            "submysterygift|a,b,c",
            &[("msg-param-mass-gift-count", "3")],
        ))
        .unwrap();
        let e = bomb.event.unwrap();
        assert_eq!((e.kind, e.gifts), (EventKind::GiftBomb, 3));
        let gift = from_provider(&event_row(
            "kick",
            "subgift",
            "kick-evt-3",
            "77",
            "subgift|bob",
            &[],
        ))
        .unwrap();
        assert_eq!(gift.event.unwrap().kind, EventKind::Gift);
        for (provider, msg_id, kind) in [
            ("youtube", "superchat", EventKind::SuperChat),
            ("youtube", "supersticker", EventKind::SuperSticker),
            ("youtube", "membership", EventKind::Membership),
            ("youtube", "membergift", EventKind::MemberGift),
            ("tiktok", "tiktok_gift", EventKind::TikTokGift),
            ("tiktok", "tiktok_follow", EventKind::TikTokFollow),
            ("tiktok", "tiktok_share", EventKind::TikTokShare),
        ] {
            let f = from_provider(&event_row(provider, msg_id, "yt-1", "UCabc", "", &[])).unwrap();
            assert_eq!(f.event.map(|e| e.kind), Some(kind), "{provider} {msg_id}");
            assert_eq!(
                f.event_key,
                format!("chat:{provider}:alpha:yt-1"),
                "never a claim key"
            );
        }
        let recipient =
            from_provider(&event_row("youtube", "subgift", "yt-2", "UCabc", "", &[])).unwrap();
        assert!(
            recipient.event.is_none(),
            "a YouTube recipient row maps to nothing"
        );
    }

    #[test]
    fn a_kick_event_trigger_uses_the_kick_family_and_a_chat_one_does_not() {
        let when = Trigger::ChannelEvent {
            events: vec![EventKind::Sub],
            min_months: 0,
            min_gifts: 0,
            min_viewers: 0,
            min_bits: 0,
        };
        let sub = from_provider(&event_row(
            "kick",
            "sub",
            "kick-evt-1",
            "77",
            "sub|1",
            &[("msg-param-cumulative-months", "1")],
        ))
        .unwrap();
        assert_eq!(
            claims::trigger_family(&when, &sub).as_deref(),
            Some("kick_evt:sub")
        );
    }

    // ---- who you are --------------------------------------------------------------------

    #[test]
    fn your_own_kick_row_is_yours_by_id_then_by_name() {
        let c = compiled(
            vec![platform_rule(
                "r1",
                "kick",
                MINE,
                mention(),
                kick_chats(),
                reply_action(),
            )],
            &[kick_id()],
        );
        let mine = row("kick", "kick:alpha", "m1", MINE, "mykick", "hi", "5002");
        assert!(is_own_row(&c, "kick", &mine, &kick_id()));
        assert!(!is_own_row(
            &c,
            "kick",
            &kick_row("m2", "77", "hi"),
            &kick_id()
        ));
        // No chat id known yet: the name has to do.
        let nameless = PlatformId {
            provider: "kick".into(),
            id: String::new(),
            login: "mykick".into(),
        };
        let by_name = row("kick", "kick:alpha", "m3", "999", "MyKick", "hi", "5002");
        assert!(is_own_row(
            &compiled(vec![], &[]),
            "kick",
            &by_name,
            &nameless
        ));
        assert!(!is_own_row(
            &compiled(vec![], &[]),
            "kick",
            &kick_row("m4", "77", "hi"),
            &nameless
        ));
    }

    #[test]
    fn your_own_youtube_row_is_caught_by_the_identity_not_by_a_rule() {
        let me = PlatformId {
            provider: "youtube".into(),
            id: "UCme".into(),
            login: "me".into(),
        };
        let c = compiled(vec![], &[me.clone()]);
        let mine = row(
            "youtube",
            "youtube:alpha",
            "y-1",
            "UCme",
            "me",
            "hi",
            "alpha",
        );
        let theirs = row(
            "youtube",
            "youtube:alpha",
            "y-2",
            "UCfan",
            "fan",
            "hi",
            "alpha",
        );
        assert!(is_own_row(&c, "youtube", &mine, &me));
        assert!(!is_own_row(&c, "youtube", &theirs, &me));
        assert_eq!(
            me.own_chat_key().as_deref(),
            Some("youtube:UCme"),
            "a YouTube channel is named by its id"
        );
        let kick = PlatformId {
            provider: "kick".into(),
            id: "5001".into(),
            login: "MyKick".into(),
        };
        assert_eq!(kick.own_chat_key().as_deref(), Some("kick:mykick"));
        assert_eq!(
            PlatformId {
                provider: "tiktok".into(),
                id: String::new(),
                login: String::new()
            }
            .own_chat_key(),
            None
        );
    }

    #[test]
    fn a_twitch_chatter_with_your_kick_id_is_not_you() {
        let c = compiled(
            vec![platform_rule(
                "r1",
                "kick",
                MINE,
                mention(),
                kick_chats(),
                reply_action(),
            )],
            &[kick_id()],
        );
        assert!(
            c.is_mine_on("kick", MINE),
            "the Kick set holds the rule's account"
        );
        assert!(!c.is_mine(MINE), "the Twitch set does not");
        assert!(!c.twitch_ids().contains(MINE));
        assert!(c.is_mine("100") && !c.is_mine_on("kick", "100"));
    }

    #[test]
    fn the_kick_target_rail_refuses_you_the_streamer_moderators_and_vips() {
        let c = compiled(
            vec![platform_rule(
                "r1",
                "kick",
                MINE,
                mention(),
                kick_chats(),
                reply_action(),
            )],
            &[kick_id()],
        );
        let facts = |user: &str, roles: Vec<SenderRole>| Facts {
            user_id: user.into(),
            roles,
            ..Facts::default()
        };
        assert!(kick_target_rail(&c, &facts("77", vec![SenderRole::Viewer])).is_ok());
        assert!(kick_target_rail(&c, &facts("77", vec![SenderRole::Subscriber])).is_ok());
        assert!(kick_target_rail(&c, &facts(MINE, vec![SenderRole::Viewer])).is_err());
        for role in [
            SenderRole::Broadcaster,
            SenderRole::Moderator,
            SenderRole::Vip,
        ] {
            assert!(
                kick_target_rail(&c, &facts("77", vec![role])).is_err(),
                "{role:?}"
            );
        }
        // A Twitch id that equals your Kick id is a valid Kick target: the sets are per provider.
        assert!(kick_target_rail(&c, &facts("100", vec![SenderRole::Viewer])).is_ok());
    }

    // ---- compile and match ------------------------------------------------------------------

    #[test]
    fn a_kick_rule_runs_with_the_connected_account_and_listens_on_kick() {
        let c = compiled(
            vec![platform_rule(
                "r1",
                "kick",
                MINE,
                mention(),
                kick_chats(),
                reply_action(),
            )],
            &[kick_id()],
        );
        let cr = &c.rules[0];
        assert_eq!(cr.held, None);
        assert!(cr.mode.is_some());
        assert_eq!(cr.own_keys, vec!["kick:mykick".to_string()]);
        assert_eq!(cr.these_keys, vec!["kick:alpha".to_string()]);
        assert_eq!(c.needed_provider_chats, vec!["kick:alpha".to_string()]);
        assert!(
            c.needed_chats.is_empty(),
            "no Twitch room is joined for a Kick rule"
        );
        assert!(c.live_chats.contains(&"kick:alpha".to_string()));
    }

    #[test]
    fn a_kick_rule_with_no_account_or_another_one_is_held_in_words() {
        let none = compiled(
            vec![platform_rule(
                "r1",
                "kick",
                MINE,
                mention(),
                kick_chats(),
                reply_action(),
            )],
            &[],
        );
        assert_eq!(
            none.rules[0].held.as_deref(),
            Some("Connect Kick to run this rule")
        );
        assert!(none.rules[0].mode.is_none());
        let other = PlatformId {
            provider: "kick".into(),
            id: "9999".into(),
            login: "someone".into(),
        };
        let c = compiled(
            vec![platform_rule(
                "r1",
                "kick",
                MINE,
                mention(),
                kick_chats(),
                reply_action(),
            )],
            &[other],
        );
        assert_eq!(
            c.rules[0].held.as_deref(),
            Some("Kick is signed in as another account")
        );
    }

    #[test]
    fn a_notify_rule_on_another_platform_waits_for_that_platform_identity() {
        // A Notify posts nothing, so the rule is sent as the owner's Twitch account and only
        // listens on YouTube: what it waits for is that platform's identity.
        let mut rule = platform_rule(
            "r1",
            "twitch",
            "100",
            json!({ "kind": "channel_event", "events": ["super_chat"] }),
            json!({ "kind": "any_open" }),
            notify_action(),
        );
        rule["only_if"]["platforms"] = json!(["youtube"]);
        let waiting = compiled(vec![rule.clone()], &[]);
        assert_eq!(
            waiting.rules[0].held.as_deref(),
            Some("Waiting for your YouTube account")
        );
        let yt = PlatformId {
            provider: "youtube".into(),
            id: "UCabc".into(),
            login: "me".into(),
        };
        let c = compiled(vec![rule], &[yt]);
        assert_eq!(c.rules[0].held, None);
        assert!(c.rules[0].mode.is_some());
    }

    #[test]
    fn a_rule_stored_before_platforms_existed_runs_as_a_twitch_rule() {
        let mut old = platform_rule(
            "r1",
            "twitch",
            "100",
            mention(),
            json!({ "kind": "own" }),
            reply_action(),
        );
        old["only_if"].as_object_mut().unwrap().remove("platforms");
        let c = compiled(vec![old], &[]);
        assert_eq!(c.rules[0].held, None);
        assert_eq!(c.rules[0].own_key, "twitch:me");
        assert!(c.needed_chats == vec!["me".to_string()] && c.needed_provider_chats.is_empty());
    }

    #[test]
    fn twitch_and_provider_chats_share_the_fifty_chat_cap() {
        let chats = |provider: &str, n: usize| json!({ "kind": "these", "list": (0..n).map(|i| json!({ "id": format!("c-{provider}{i}"), "key": format!("{provider}:chan{i}"), "room_id": "1", "name": format!("chan{i}") })).collect::<Vec<_>>() });
        let twitch = platform_rule(
            "rt",
            "twitch",
            "100",
            mention(),
            chats("twitch", 25),
            reply_action(),
        );
        let kick_a = platform_rule(
            "ra",
            "kick",
            MINE,
            mention(),
            chats("kick", 25),
            reply_action(),
        );
        let mut kick_b = platform_rule(
            "rb",
            "kick",
            MINE,
            mention(),
            chats("kick", 25),
            reply_action(),
        );
        kick_b["only_if"]["channels"]["list"][0]["key"] = json!("kick:other0");
        let c = compiled(vec![twitch, kick_a, kick_b], &[kick_id()]);
        assert_eq!(c.needed_chats.len(), 25);
        assert_eq!(
            c.needed_chats.len() + c.needed_provider_chats.len(),
            50,
            "Twitch first, then the rest up to 50"
        );
    }

    #[test]
    fn a_kick_message_rule_fires_on_kick_rows_only() {
        let when = json!({ "kind": "message", "text": { "mode": "contains", "pattern": "hello", "case_sensitive": false } });
        let c = compiled(
            vec![platform_rule(
                "r1",
                "kick",
                MINE,
                when,
                kick_chats(),
                reply_action(),
            )],
            &[kick_id()],
        );
        let on_kick = from_provider(&kick_row("m1", "77", "hello there")).unwrap();
        assert_eq!(fired(&c, &on_kick), vec![true]);
        // The same channel name on Twitch is another chat key: the rule is not even a candidate.
        let twitch = crate::services::account_automations::facts::from_chat(&row(
            "twitch",
            "alpha",
            "m1",
            "77",
            "fan",
            "hello there",
            "5002",
        ))
        .unwrap();
        assert!(fired(&c, &twitch).is_empty());
    }

    #[test]
    fn an_any_open_rule_answers_only_the_platforms_it_names() {
        let when = json!({ "kind": "message", "text": { "mode": "contains", "pattern": "hello", "case_sensitive": false } });
        let c = compiled(
            vec![platform_rule(
                "r1",
                "kick",
                MINE,
                when,
                json!({ "kind": "any_open" }),
                reply_action(),
            )],
            &[kick_id()],
        );
        let kick = from_provider(&kick_row("m1", "77", "hello")).unwrap();
        assert_eq!(fired(&c, &kick), vec![true]);
        let twitch = crate::services::account_automations::facts::from_chat(&row(
            "twitch", "alpha", "m2", "77", "fan", "hello", "5002",
        ))
        .unwrap();
        assert_eq!(
            fired(&c, &twitch),
            vec![false],
            "a Twitch row is not a platform this rule names"
        );
    }

    #[test]
    fn a_kick_mention_uses_the_kick_identity_not_the_twitch_owner() {
        let c = compiled(
            vec![platform_rule(
                "r1",
                "kick",
                MINE,
                mention(),
                kick_chats(),
                reply_action(),
            )],
            &[kick_id()],
        );
        let yes = from_provider(&kick_row("m1", "77", "hey @mykick are you there")).unwrap();
        assert_eq!(fired(&c, &yes), vec![true]);
        let no = from_provider(&kick_row("m2", "77", "hey @me are you there")).unwrap();
        assert_eq!(
            fired(&c, &no),
            vec![false],
            "the Twitch owner's name is not a Kick mention"
        );
        let mut reply = kick_row("m3", "77", "no name here");
        reply.metadata.reply_info = Some(crate::models::chat_layout::ReplyInfo {
            parent_msg_id: "p".into(),
            parent_display_name: "mykick".into(),
            parent_msg_body: "x".into(),
            parent_segments: None,
            parent_user_id: MINE.into(),
            parent_user_login: "mykick".into(),
            parent_shown_name: String::new(),
        });
        assert_eq!(
            fired(&c, &from_provider(&reply).unwrap()),
            vec![true],
            "a reply to your Kick account"
        );
    }
}
