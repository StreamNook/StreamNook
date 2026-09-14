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

use crate::models::chat_layout::{ChatMessage, MessageSegment};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

static MAP: OnceLock<RwLock<HashMap<String, HashMap<String, String>>>> = OnceLock::new();

fn map() -> &'static RwLock<HashMap<String, HashMap<String, String>>> {
    MAP.get_or_init(|| RwLock::new(HashMap::new()))
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
        let key = channel.trim_start_matches('#').to_lowercase();
        map()
            .read()
            .ok()
            .and_then(|m| m.get(&key).and_then(|u| u.get(user_id)).cloned())
    }

    /// Record the status carried by a `.message` or `.update` event.
    /// Returns the normalized status when the user is (still) flagged.
    pub fn note(event: &Value) -> Option<String> {
        let channel = s(event, "/broadcaster_user_login").to_lowercase();
        let user_id = s(event, "/user_id");
        if channel.is_empty() || user_id.is_empty() {
            return None;
        }
        let status = normalize(&s(event, "/low_trust_status"));
        let Ok(mut m) = map().write() else { return None };
        let users = m.entry(channel).or_default();
        match status {
            Some(st) => {
                users.insert(user_id, st.to_string());
                Some(st.to_string())
            }
            None => {
                users.remove(&user_id);
                None
            }
        }
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
