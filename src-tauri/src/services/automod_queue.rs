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
    /// Why it is held, in words: "Held by AutoMod · Swearing, level 2".
    pub label: String,
}

/// An AutoMod category as a person reads it. Twitch's ids vary between
/// versions, so an unknown one is shown with its underscores as spaces.
fn category_words(category: &str) -> String {
    let words = match category.to_ascii_lowercase().as_str() {
        "swearing" | "profanity" => "Swearing",
        "sexwords" | "sex_based_terms" | "sexuality" | "sexual" => "Sex-based terms",
        "aggressive" | "aggression" => "Aggression",
        "disability" | "ableism" => "Disability",
        "racism" | "race_ethnicity_or_religion" | "race" => "Race, religion or ethnicity",
        "misogyny" => "Misogyny",
        "homophobia" | "sexuality_sex_or_gender" => "Sexuality, sex or gender",
        "bullying" | "hostility" => "Bullying",
        other => return other.replace('_', " "),
    };
    words.to_string()
}

/// "Held by AutoMod · Swearing, level 2", or "Held: blocked term “bad”".
pub fn held_label(reason: &str, category: Option<&str>, level: Option<u64>, terms: &[String]) -> String {
    match reason {
        "blocked_term" if !terms.is_empty() => {
            let quoted: Vec<String> = terms.iter().map(|t| format!("\u{201C}{t}\u{201D}")).collect();
            let noun = if terms.len() == 1 { "blocked term" } else { "blocked terms" };
            format!("Held: {noun} {}", quoted.join(", "))
        }
        "blocked_term" => "Held: blocked term".to_string(),
        "blocked_link" => "Held: blocked link".to_string(),
        _ => match (category.filter(|c| !c.is_empty()), level) {
            (Some(c), Some(l)) => format!("Held by AutoMod \u{00B7} {}, level {l}", category_words(c)),
            (Some(c), None) => format!("Held by AutoMod \u{00B7} {}", category_words(c)),
            _ => "Held by AutoMod".to_string(),
        },
    }
}

/// The quiet line a resolved held row folds into.
pub fn resolved_line(row: &HeldMessage) -> String {
    let who = if row.user_name.is_empty() { &row.user_login } else { &row.user_name };
    match row.status.as_str() {
        "approved" => format!("Allowed {who}\u{2019}s held message"),
        "denied" => format!("Denied {who}\u{2019}s held message"),
        "expired" => format!("{who}\u{2019}s held message expired"),
        _ => row.label.clone(),
    }
}

/// The chat-stream frame for a held message, held or resolved, so the phones
/// draw it inline where it arrived and fold it in place once it is decided.
/// `by` is the moderator who decided it.
pub fn row_frame(row: &HeldMessage, by: Option<&str>) -> Value {
    let resolved = row.status != "held";
    serde_json::json!({
        "type": "AUTOMOD_HELD",
        "channel": row.channel,
        "held": row,
        "line": resolved.then(|| resolved_line(row)),
        "by": by.filter(|b| !b.is_empty()),
    })
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
        let category = event.pointer("/automod/category").and_then(|x| x.as_str()).map(String::from);
        let level = event.pointer("/automod/level").and_then(|x| x.as_u64());
        let label = held_label(&reason, category.as_deref(), level, &terms);
        Some(HeldMessage {
            message_id,
            channel: s(event, "/broadcaster_user_login").to_lowercase(),
            broadcaster_id: s(event, "/broadcaster_user_id"),
            user_id: s(event, "/user_id"),
            user_login: s(event, "/user_login"),
            user_name: s(event, "/user_name"),
            text: s(event, "/message/text"),
            reason,
            category,
            level,
            terms,
            held_at_ms,
            status: "held".into(),
            label,
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
        Self::update_row(event).map(|row| (row.channel, row.message_id, row.status))
    }

    /// `update`, returning the resolved row with its new status, for the
    /// chat stream's in-place fold.
    pub fn update_row(event: &Value) -> Option<HeldMessage> {
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
        let mut row = ring.remove(pos)?;
        row.status = status;
        Some(row)
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
    fn held_words_and_the_fold() {
        assert_eq!(held_label("automod", Some("swearing"), Some(2), &[]), "Held by AutoMod \u{00B7} Swearing, level 2");
        assert_eq!(held_label("automod", Some("new_thing"), None, &[]), "Held by AutoMod \u{00B7} new thing");
        assert_eq!(held_label("automod", None, None, &[]), "Held by AutoMod");
        assert_eq!(held_label("blocked_term", None, None, &["bad".into()]), "Held: blocked term \u{201C}bad\u{201D}");
        assert_eq!(held_label("blocked_link", None, None, &[]), "Held: blocked link");

        let ev = serde_json::json!({
            "broadcaster_user_login": "fold", "message_id": "m9", "user_login": "gus", "user_name": "Gus",
            "message": { "text": "rude" }, "reason": "automod", "automod": { "category": "swearing", "level": 1 }
        });
        let row = AutomodQueue::from_hold_event(&ev).unwrap();
        let held = row_frame(&row, None);
        assert_eq!(held["type"], "AUTOMOD_HELD");
        assert_eq!(held["held"]["label"], "Held by AutoMod \u{00B7} Swearing, level 1");
        assert!(held["line"].is_null());
        AutomodQueue::hold(row);
        let upd = serde_json::json!({ "broadcaster_user_login": "fold", "message_id": "m9", "status": "Denied" });
        let done = AutomodQueue::update_row(&upd).unwrap();
        let folded = row_frame(&done, Some("kay"));
        assert_eq!(folded["held"]["status"], "denied");
        assert_eq!(folded["line"], "Denied Gus\u{2019}s held message");
        assert_eq!(folded["by"], "kay");
        AutomodQueue::clear_channel("fold");
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
