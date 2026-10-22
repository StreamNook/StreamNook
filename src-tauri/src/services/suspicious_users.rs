//! Suspicious (low-trust) user state per channel, fed by EventSub
//! `channel.suspicious_user.message` / `.update` on the moderation socket.
//! The rule engine stamps every message from a tracked user with
//! `metadata.suspicious` ("monitored" | "restricted"), which the row shows
//! and filters can test (`flags.monitored`, `flags.restricted`).
//!
//! Restricted users' messages never reach IRC; the event is their only
//! source, so `synthesize` turns the event into a ChatMessage that the
//! ordinary publish path stamps and broadcasts (only moderators hold the
//! subscription, so only they see it).
//!
//! Beside the status, the `.message` event says why Twitch flagged someone
//! (ban evasion, bans in channels that share their ban list, a moderator
//! adding them). Twitch has no call to read any of it back, so the user card
//! can only show what this session saw; `card_view` puts it into words and
//! offers the treatments the viewer can switch to.

use crate::models::chat_layout::{ChatMessage, MessageSegment};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

/// What this session saw Twitch say about one chatter in one channel.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Seen {
    /// "monitored" or "restricted"; `None` once the treatment was lifted.
    pub status: Option<&'static str>,
    /// "unknown", "possible" or "likely".
    pub evaluation: Option<String>,
    /// Why Twitch flagged them, in the EventSub spelling (`normalize_type`).
    pub types: Vec<&'static str>,
    /// Channels that share their ban list with this one and banned them.
    pub shared_ban_channel_ids: Vec<String>,
}

static MAP: OnceLock<RwLock<HashMap<String, HashMap<String, Seen>>>> = OnceLock::new();

/// Chatters remembered per channel.
const MAX_SEEN_PER_CHANNEL: usize = 500;

fn map() -> &'static RwLock<HashMap<String, HashMap<String, Seen>>> {
    MAP.get_or_init(|| RwLock::new(HashMap::new()))
}

/// A detection type in one spelling: EventSub's lower case, or Helix's
/// upper case with its own names. Unknown ones drop.
pub fn normalize_type(t: &str) -> Option<&'static str> {
    match t.to_ascii_lowercase().as_str() {
        "manually_added" => Some("manually_added"),
        "ban_evader" | "detected_ban_evader" => Some("ban_evader"),
        "banned_in_shared_channel" => Some("banned_in_shared_channel"),
        "detected_sus_chatter" | "suspicious" => Some("suspicious"),
        _ => None,
    }
}

/// `prev` updated by one `.message` or `.update` event. Both carry the
/// status; only `.message` carries the reasons, so an `.update` keeps the
/// reasons last seen. Pure, for tests.
pub fn merge(prev: Option<Seen>, event: &Value) -> Seen {
    let mut seen = prev.unwrap_or_default();
    seen.status = normalize(&s(event, "/low_trust_status"));
    let carries_reasons = ["types", "ban_evasion_evaluation", "shared_ban_channel_ids"]
        .iter()
        .any(|k| event.get(*k).is_some());
    if carries_reasons {
        seen.evaluation = Some(s(event, "/ban_evasion_evaluation").to_ascii_lowercase()).filter(|e| !e.is_empty());
        seen.types = event["types"]
            .as_array()
            .map(|a| {
                let mut out: Vec<&'static str> = Vec::new();
                for t in a.iter().filter_map(Value::as_str).filter_map(normalize_type) {
                    if !out.contains(&t) {
                        out.push(t);
                    }
                }
                out
            })
            .unwrap_or_default();
        seen.shared_ban_channel_ids = event["shared_ban_channel_ids"]
            .as_array()
            .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
            .unwrap_or_default();
    }
    seen
}

/// A treatment the viewer can switch the chatter to from their card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LowTrustAction {
    pub id: &'static str,
    /// "Monitor", "Restrict", "Clear".
    pub label: &'static str,
    /// What `update_suspicious_user_status` takes as `status`.
    pub status: &'static str,
    /// For screen readers and tooltips.
    pub spoken: &'static str,
    /// What the card says once Twitch took it.
    pub done: &'static str,
}

/// The card's low-trust line and buttons.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LowTrust {
    /// "monitored", "restricted" or "none".
    pub status: &'static str,
    /// "Restricted · Likely ban evader · Banned in 2 other channels"; `None`
    /// when this session saw nothing about them.
    pub line: Option<String>,
    pub actions: Vec<LowTrustAction>,
}

const MONITOR: LowTrustAction = LowTrustAction {
    id: "monitor",
    label: "Monitor",
    status: "ACTIVE_MONITORING",
    spoken: "Monitor: their messages show a low-trust mark to moderators",
    done: "Now monitored",
};
const RESTRICT: LowTrustAction = LowTrustAction {
    id: "restrict",
    label: "Restrict",
    status: "RESTRICTED",
    spoken: "Restrict: only moderators see their messages",
    done: "Now restricted",
};
const CLEAR: LowTrustAction = LowTrustAction {
    id: "clear",
    label: "Clear",
    status: "NO_TREATMENT",
    spoken: "Clear: treat them as any other chatter",
    done: "No longer monitored or restricted",
};

/// The reasons Twitch gave, in words. Pure, for tests.
pub fn reason_words(seen: &Seen) -> Vec<String> {
    let mut out = Vec::new();
    match seen.evaluation.as_deref() {
        Some("likely") => out.push("Likely ban evader".to_string()),
        Some("possible") => out.push("Possible ban evader".to_string()),
        _ if seen.types.contains(&"ban_evader") => out.push("Flagged as a ban evader".to_string()),
        _ => {}
    }
    let shared = seen.shared_ban_channel_ids.len();
    if shared == 1 {
        out.push("Banned in 1 other channel".to_string());
    } else if shared > 1 {
        out.push(format!("Banned in {shared} other channels"));
    } else if seen.types.contains(&"banned_in_shared_channel") {
        out.push("Banned in a channel that shares bans with this one".to_string());
    }
    if seen.types.contains(&"suspicious") {
        out.push("Flagged as suspicious by Twitch".to_string());
    }
    if seen.types.contains(&"manually_added") {
        out.push("Added by a moderator".to_string());
    }
    out
}

/// The card's view of what was seen: the status word first, then the
/// reasons, and the treatments other than the one in force. Pure, for tests.
pub fn view_of(seen: Option<&Seen>) -> LowTrust {
    let status = seen.and_then(|s| s.status);
    let mut words: Vec<String> = Vec::new();
    match status {
        Some("restricted") => words.push("Restricted".into()),
        Some("monitored") => words.push("Monitored".into()),
        _ => {}
    }
    if let Some(seen) = seen {
        words.extend(reason_words(seen));
    }
    let actions = match status {
        Some("restricted") => vec![MONITOR, CLEAR],
        Some("monitored") => vec![RESTRICT, CLEAR],
        _ => vec![MONITOR, RESTRICT],
    };
    LowTrust {
        status: status.unwrap_or("none"),
        line: if words.is_empty() { None } else { Some(words.join(" · ")) },
        actions,
    }
}

fn s(v: &Value, ptr: &str) -> String {
    v.pointer(ptr).and_then(|x| x.as_str()).unwrap_or("").to_string()
}

/// "active_monitoring" -> "monitored", "restricted" -> "restricted", else None.
fn normalize(status: &str) -> Option<&'static str> {
    match status.to_ascii_lowercase().as_str() {
        "active_monitoring" | "monitored" => Some("monitored"),
        "restricted" => Some("restricted"),
        _ => None,
    }
}

pub struct SuspiciousUsers;

impl SuspiciousUsers {
    pub fn status(channel: &str, user_id: &str) -> Option<String> {
        if user_id.is_empty() {
            return None;
        }
        Self::seen(channel, user_id).and_then(|s| s.status).map(str::to_string)
    }

    /// Everything this session saw about `user_id` in `channel`.
    pub fn seen(channel: &str, user_id: &str) -> Option<Seen> {
        if user_id.is_empty() {
            return None;
        }
        let key = channel.trim_start_matches('#').to_lowercase();
        map()
            .read()
            .ok()
            .and_then(|m| m.get(&key).and_then(|u| u.get(user_id)).cloned())
    }

    /// The user card's low-trust line and buttons for `user_id` in `channel`.
    pub fn card_view(channel: &str, user_id: &str) -> LowTrust {
        view_of(Self::seen(channel, user_id).as_ref())
    }

    /// Record the status carried by a `.message` or `.update` event.
    /// Returns the normalized status when the user is (still) flagged. A
    /// lifted treatment keeps the reasons seen, which the card still shows.
    pub fn note(event: &Value) -> Option<String> {
        let channel = s(event, "/broadcaster_user_login").to_lowercase();
        let user_id = s(event, "/user_id");
        if channel.is_empty() || user_id.is_empty() {
            return None;
        }
        let Ok(mut m) = map().write() else { return None };
        let users = m.entry(channel).or_default();
        let seen = merge(users.remove(&user_id), event);
        let status = seen.status.map(str::to_string);
        if seen != Seen::default() {
            users.insert(user_id, seen);
        }
        // A lifted treatment only explains the past; past the cap one goes so
        // a long session's map stops growing.
        if users.len() > MAX_SEEN_PER_CHANNEL {
            if let Some(k) = users.iter().find(|(_, s)| s.status.is_none()).map(|(k, _)| k.clone()) {
                users.remove(&k);
            }
        }
        status
    }

    /// Every flagged user in `channel`: (user_id, "monitored" | "restricted").
    pub fn list(channel: &str) -> Vec<(String, String)> {
        let key = channel.trim_start_matches('#').to_lowercase();
        let mut out: Vec<(String, String)> = map()
            .read()
            .ok()
            .and_then(|m| {
                m.get(&key).map(|u| {
                    u.iter()
                        .filter_map(|(k, v)| v.status.map(|st| (k.clone(), st.to_string())))
                        .collect()
                })
            })
            .unwrap_or_default();
        out.sort();
        out
    }

    pub fn clear_channel(channel: &str) {
        let key = channel.trim_start_matches('#').to_lowercase();
        if let Ok(mut m) = map().write() {
            m.remove(&key);
        }
    }

    /// Build a ChatMessage from a `channel.suspicious_user.message` event so a
    /// restricted user's message (absent from IRC) can enter the normal chat
    /// pipeline. `None` for anything without a message id.
    pub fn synthesize(event: &Value) -> Option<ChatMessage> {
        let id = s(event, "/message/message_id");
        if id.is_empty() {
            return None;
        }
        let text = s(event, "/message/text");
        let login = s(event, "/user_login");
        let display = {
            let d = s(event, "/user_name");
            if d.is_empty() { login.clone() } else { d }
        };
        let channel = s(event, "/broadcaster_user_login").to_lowercase();
        let ts_ms = event
            .pointer("/message/sent_at")
            .and_then(|x| x.as_str())
            .and_then(|iso| chrono::DateTime::parse_from_rfc3339(iso).ok())
            .map(|d| d.timestamp_millis())
            .unwrap_or_else(|| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0)
            });
        let mut tags = HashMap::new();
        tags.insert("id".into(), id.clone());
        tags.insert("user-id".into(), s(event, "/user_id"));
        tags.insert("display-name".into(), display.clone());
        tags.insert("login".into(), login.clone());
        tags.insert("room-id".into(), s(event, "/broadcaster_user_id"));
        tags.insert("tmi-sent-ts".into(), ts_ms.to_string());
        // Fragments: keep text; emotes render as their text (no CDN id here
        // worth a lookup for a message only moderators see).
        let segments = vec![MessageSegment::Text { content: text.clone() }];
        Some(ChatMessage {
            id,
            user_id: s(event, "/user_id"),
            username: login,
            display_name: display,
            color: None,
            badges: Vec::new(),
            timestamp: ts_ms.to_string(),
            content: text,
            provider: "twitch".into(),
            channel,
            emotes: Vec::new(),
            tags,
            layout: Default::default(),
            segments,
            metadata: Default::default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_status_and_clear() {
        let ev = serde_json::json!({
            "broadcaster_user_login": "Forsen", "user_id": "9", "low_trust_status": "active_monitoring"
        });
        assert_eq!(SuspiciousUsers::note(&ev).as_deref(), Some("monitored"));
        assert_eq!(SuspiciousUsers::status("#forsen", "9").as_deref(), Some("monitored"));
        let upd = serde_json::json!({
            "broadcaster_user_login": "forsen", "user_id": "9", "low_trust_status": "no_treatment"
        });
        assert!(SuspiciousUsers::note(&upd).is_none());
        assert!(SuspiciousUsers::status("forsen", "9").is_none());
        SuspiciousUsers::clear_channel("forsen");
    }

    #[test]
    fn an_update_keeps_the_reasons_a_message_gave() {
        let msg = serde_json::json!({
            "broadcaster_user_login": "chan", "user_id": "5", "low_trust_status": "restricted",
            "types": ["ban_evader", "banned_in_shared_channel", "ban_evader", "something_new"],
            "ban_evasion_evaluation": "LIKELY",
            "shared_ban_channel_ids": ["100", "200"]
        });
        let seen = merge(None, &msg);
        assert_eq!(seen.status, Some("restricted"));
        assert_eq!(seen.types, vec!["ban_evader", "banned_in_shared_channel"], "repeats and unknown ones drop");
        assert_eq!(seen.evaluation.as_deref(), Some("likely"));
        assert_eq!(seen.shared_ban_channel_ids, vec!["100", "200"]);

        let upd = serde_json::json!({ "broadcaster_user_login": "chan", "user_id": "5", "low_trust_status": "active_monitoring" });
        let after = merge(Some(seen.clone()), &upd);
        assert_eq!(after.status, Some("monitored"));
        assert_eq!(after.types, seen.types);
        assert_eq!(after.shared_ban_channel_ids, seen.shared_ban_channel_ids);

        let none = serde_json::json!({ "low_trust_status": "none", "shared_ban_channel_ids": null, "types": [] });
        let cleared = merge(Some(after), &none);
        assert_eq!(cleared, Seen::default(), "a message with no reasons replaces the old ones");
    }

    #[test]
    fn helix_spellings_normalize() {
        assert_eq!(normalize_type("MANUALLY_ADDED"), Some("manually_added"));
        assert_eq!(normalize_type("DETECTED_BAN_EVADER"), Some("ban_evader"));
        assert_eq!(normalize_type("DETECTED_SUS_CHATTER"), Some("suspicious"));
        assert_eq!(normalize_type("BANNED_IN_SHARED_CHANNEL"), Some("banned_in_shared_channel"));
        assert_eq!(normalize_type("nope"), None);
        assert_eq!(normalize("ACTIVE_MONITORING"), Some("monitored"));
        assert_eq!(normalize("NO_TREATMENT"), None);
    }

    #[test]
    fn the_cards_words_and_buttons() {
        let nothing = view_of(None);
        assert_eq!(nothing.status, "none");
        assert_eq!(nothing.line, None);
        assert_eq!(nothing.actions.iter().map(|a| a.id).collect::<Vec<_>>(), ["monitor", "restrict"]);

        let seen = Seen {
            status: Some("restricted"),
            evaluation: Some("likely".into()),
            types: vec!["ban_evader", "banned_in_shared_channel"],
            shared_ban_channel_ids: vec!["1".into(), "2".into()],
        };
        let v = view_of(Some(&seen));
        assert_eq!(v.line.as_deref(), Some("Restricted · Likely ban evader · Banned in 2 other channels"));
        assert_eq!(v.actions.iter().map(|a| a.status).collect::<Vec<_>>(), ["ACTIVE_MONITORING", "NO_TREATMENT"]);

        let monitored = Seen { status: Some("monitored"), ..Seen::default() };
        let m = view_of(Some(&monitored));
        assert_eq!(m.line.as_deref(), Some("Monitored"));
        assert_eq!(m.actions.iter().map(|a| a.label).collect::<Vec<_>>(), ["Restrict", "Clear"]);

        let lifted = Seen {
            status: None,
            evaluation: Some("unknown".into()),
            types: vec!["ban_evader", "banned_in_shared_channel", "manually_added"],
            shared_ban_channel_ids: vec!["9".into()],
        };
        let l = view_of(Some(&lifted));
        assert_eq!(l.status, "none");
        assert_eq!(l.line.as_deref(), Some("Flagged as a ban evader · Banned in 1 other channel · Added by a moderator"));

        let shared_only = Seen { types: vec!["banned_in_shared_channel"], ..Seen::default() };
        assert_eq!(reason_words(&shared_only), ["Banned in a channel that shares bans with this one"]);
        let possible = Seen { evaluation: Some("possible".into()), ..Seen::default() };
        assert_eq!(reason_words(&possible), ["Possible ban evader"]);
    }

    #[test]
    fn a_lifted_flag_leaves_the_list_but_keeps_its_reasons() {
        let msg = serde_json::json!({
            "broadcaster_user_login": "lowtrustcard", "user_id": "77", "low_trust_status": "active_monitoring",
            "types": ["manually_added"], "ban_evasion_evaluation": "unknown", "shared_ban_channel_ids": []
        });
        assert_eq!(SuspiciousUsers::note(&msg).as_deref(), Some("monitored"));
        assert_eq!(SuspiciousUsers::list("lowtrustcard"), vec![("77".to_string(), "monitored".to_string())]);
        let upd = serde_json::json!({ "broadcaster_user_login": "lowtrustcard", "user_id": "77", "low_trust_status": "no_treatment" });
        assert!(SuspiciousUsers::note(&upd).is_none());
        assert!(SuspiciousUsers::status("lowtrustcard", "77").is_none());
        assert!(SuspiciousUsers::list("lowtrustcard").is_empty());
        assert_eq!(SuspiciousUsers::card_view("#LowTrustCard", "77").line.as_deref(), Some("Added by a moderator"));
        SuspiciousUsers::clear_channel("lowtrustcard");
    }

    #[test]
    fn synthesizes_a_restricted_message() {
        let ev = serde_json::json!({
            "broadcaster_user_id": "1", "broadcaster_user_login": "Forsen",
            "user_id": "9", "user_login": "evader", "user_name": "Evader",
            "low_trust_status": "restricted",
            "message": { "message_id": "abc", "text": "hello mods", "sent_at": "2026-09-07T16:00:00Z" }
        });
        let m = SuspiciousUsers::synthesize(&ev).unwrap();
        assert_eq!(m.id, "abc");
        assert_eq!(m.channel, "forsen");
        assert_eq!(m.tags.get("tmi-sent-ts").map(|s| s.as_str()), Some("1788796800000"));
        assert_eq!(m.content, "hello mods");
    }
}
