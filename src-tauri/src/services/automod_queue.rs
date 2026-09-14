//! AutoMod held-message queue, fed by EventSub `automod.message.hold` and
//! `automod.message.update` (v2) on the chat-tied moderation socket. Rust
//! owns the queue: bounded per channel, expiry mirrors Twitch's own, and
//! every window reads the same state through `get_automod_queue` plus the
//! `eventsub://automod-hold` / `eventsub://automod-update` events.
//!
//! Resolution goes through Helix `POST moderation/automod/message`
//! (scope `moderator:manage:automod`).

use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};

const PER_CHANNEL_CAP: usize = 100;
/// Twitch expires a held message after a few minutes; drop stale rows locally
/// so a missed update event cannot pin a queue forever.
const EXPIRE_MS: i64 = 6 * 60 * 1000;

#[derive(Serialize, Clone, Debug)]
pub struct HeldMessage {
    pub message_id: String,
    /// Lowercase broadcaster login (the chat slice key).
    pub channel: String,
    pub broadcaster_id: String,
    pub user_id: String,
    pub user_login: String,
    pub user_name: String,
    pub text: String,
    /// "automod" | "blocked_term"
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub terms: Vec<String>,
    pub held_at_ms: i64,
    /// "held" | "approved" | "denied" | "expired"
    pub status: String,
}

static QUEUE: OnceLock<Mutex<HashMap<String, VecDeque<HeldMessage>>>> = OnceLock::new();

fn queue() -> &'static Mutex<HashMap<String, VecDeque<HeldMessage>>> {
    QUEUE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn s(v: &Value, ptr: &str) -> String {
    v.pointer(ptr).and_then(|x| x.as_str()).unwrap_or("").to_string()
}

pub struct AutomodQueue;

impl AutomodQueue {
    /// Parse an `automod.message.hold` event into a queue row.
    pub fn from_hold_event(event: &Value) -> Option<HeldMessage> {
        let message_id = s(event, "/message_id");
        if message_id.is_empty() {
            return None;
        }
        let reason = {
            let r = s(event, "/reason");
            if r.is_empty() {
                if event.get("blocked_term").map(|b| !b.is_null()).unwrap_or(false) {
                    "blocked_term".into()
                } else {
                    "automod".into()
                }
            } else {
                r
            }
        };
        let terms: Vec<String> = event
            .pointer("/blocked_term/terms_found")
            .and_then(|t| t.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|t| t.pointer("/term").and_then(|x| x.as_str()).map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let held_at_ms = event
            .pointer("/held_at")
            .and_then(|x| x.as_str())
            .and_then(|iso| chrono::DateTime::parse_from_rfc3339(iso).ok())
            .map(|d| d.timestamp_millis())
            .unwrap_or_else(now_ms);
        Some(HeldMessage {
            message_id,
            channel: s(event, "/broadcaster_user_login").to_lowercase(),
            broadcaster_id: s(event, "/broadcaster_user_id"),
            user_id: s(event, "/user_id"),
            user_login: s(event, "/user_login"),
            user_name: s(event, "/user_name"),
            text: s(event, "/message/text"),
            reason,
            category: event
                .pointer("/automod/category")
                .and_then(|x| x.as_str())
                .map(String::from),
            level: event.pointer("/automod/level").and_then(|x| x.as_u64()),
            terms,
            held_at_ms,
            status: "held".into(),
        })
    }

    pub fn hold(row: HeldMessage) {
        let Ok(mut map) = queue().lock() else { return };
        let ring = map.entry(row.channel.clone()).or_default();
        if ring.iter().any(|r| r.message_id == row.message_id) {
            return;
        }
        if ring.len() >= PER_CHANNEL_CAP {
            ring.pop_front();
        }
        ring.push_back(row);
    }

    /// Apply an `automod.message.update` event. Returns (channel, message_id,
    /// status) when the row was known, so the caller can emit a targeted event.
    pub fn update(event: &Value) -> Option<(String, String, String)> {
        let message_id = s(event, "/message_id");
        let channel = s(event, "/broadcaster_user_login").to_lowercase();
        let status = s(event, "/status").to_lowercase();
        if message_id.is_empty() {
            return None;
        }
        let Ok(mut map) = queue().lock() else { return None };
        let ring = map.get_mut(&channel)?;
        let pos = ring.iter().position(|r| r.message_id == message_id)?;
        // Resolved rows leave the queue; the frontend removes them on the event.
        ring.remove(pos);
        Some((channel, message_id, status))
    }

    /// Rows still held for a channel, oldest first; expired ones are pruned.
    pub fn list(channel: &str) -> Vec<HeldMessage> {
        let key = channel.trim_start_matches('#').to_lowercase();
        let cutoff = now_ms() - EXPIRE_MS;
        let Ok(mut map) = queue().lock() else { return Vec::new() };
        let Some(ring) = map.get_mut(&key) else { return Vec::new() };
        ring.retain(|r| r.held_at_ms >= cutoff);
        ring.iter().cloned().collect()
    }

    pub fn clear_channel(channel: &str) {
        let key = channel.trim_start_matches('#').to_lowercase();
        if let Ok(mut map) = queue().lock() {
            map.remove(&key);
        }
    }

    pub fn total_held() -> usize {
        queue()
            .lock()
            .map(|m| m.values().map(|r| r.len()).sum())
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hold_update_roundtrip() {
        let ev = serde_json::json!({
            "broadcaster_user_id": "1", "broadcaster_user_login": "Forsen",
            "user_id": "2", "user_login": "bob", "user_name": "Bob",
            "message_id": "m1", "message": { "text": "spicy" },
            "reason": "automod", "automod": { "category": "aggressive", "level": 3 }
        });
        let row = AutomodQueue::from_hold_event(&ev).unwrap();
        assert_eq!(row.channel, "forsen");
        assert_eq!(row.category.as_deref(), Some("aggressive"));
        assert_eq!(row.level, Some(3));
        AutomodQueue::hold(row.clone());
        AutomodQueue::hold(row); // dedupe
        assert_eq!(AutomodQueue::list("forsen").len(), 1);
        let upd = serde_json::json!({
            "broadcaster_user_login": "forsen", "message_id": "m1", "status": "Approved"
        });
        let (ch, id, st) = AutomodQueue::update(&upd).unwrap();
        assert_eq!((ch.as_str(), id.as_str(), st.as_str()), ("forsen", "m1", "approved"));
        assert!(AutomodQueue::list("forsen").is_empty());
        AutomodQueue::clear_channel("forsen");
    }

    #[test]
    fn blocked_term_rows_carry_terms() {
        let ev = serde_json::json!({
            "broadcaster_user_login": "x", "message_id": "m2", "message": { "text": "hi" },
            "blocked_term": { "terms_found": [ { "term": "bad" }, { "term": "worse" } ] }
        });
        let row = AutomodQueue::from_hold_event(&ev).unwrap();
        assert_eq!(row.reason, "blocked_term");
        assert_eq!(row.terms, vec!["bad".to_string(), "worse".to_string()]);
    }
}
