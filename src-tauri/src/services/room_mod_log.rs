//! A room's recent moderator actions, for the phones' Moderation sheet.
//!
//! Twitch tells a moderator every action in a channel they moderate through
//! EventSub `channel.moderate` (v2), which the moderation socket already
//! receives. Each one is kept here per channel, newest last, in words a
//! person reads ("kay_lee timed out gus for 10m: spoilers"), and pushed as
//! `room-mod-log` so an open sheet follows along. A ban or timeout carries
//! the chatter's id, so the sheet can offer Unban through `moderate_chatter`.
//!
//! In memory only: the log is the room's live picture while the app runs.

use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};

/// How many actions a channel keeps.
const PER_CHANNEL: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LogEntry {
    pub id: String,
    /// Lowercase channel login.
    pub channel: String,
    /// Twitch's action name: "ban", "timeout", "unban", "untimeout", "delete",
    /// "warn", "clear", "slow", "followers", "emoteonly", "subscribers", ...
    pub action: String,
    pub moderator: String,
    /// The chatter acted on, when there is one.
    pub target: Option<String>,
    pub target_id: Option<String>,
    /// The whole line: "kay_lee timed out gus for 10m".
    pub line: String,
    /// The reason or the deleted message, quoted under the line.
    pub quote: Option<String>,
    pub at_ms: i64,
    /// A ban or timeout that Unban would lift.
    pub liftable: bool,
}

static LOG: OnceLock<Mutex<HashMap<String, VecDeque<LogEntry>>>> = OnceLock::new();

fn log() -> std::sync::MutexGuard<'static, HashMap<String, VecDeque<LogEntry>>> {
    LOG.get_or_init(|| Mutex::new(HashMap::new())).lock().unwrap_or_else(|e| e.into_inner())
}

fn s<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty())
}

fn name(v: &Value, prefix: &str) -> Option<String> {
    s(v, &format!("{prefix}_name")).or_else(|| s(v, &format!("{prefix}_login"))).map(str::to_string)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// One `channel.moderate` event in words. Pure, for tests.
pub fn entry_from(event: &Value, at_ms: i64) -> Option<LogEntry> {
    let action = s(event, "action")?.to_string();
    let channel = s(event, "broadcaster_user_login")?.to_lowercase();
    let moderator = name(event, "moderator_user").unwrap_or_else(|| "A moderator".to_string());
    // The action's own object carries the chatter and the details.
    let detail = event.get(action.as_str()).filter(|d| d.is_object());
    let target = detail.and_then(|d| name(d, "user"));
    let target_id = detail.and_then(|d| s(d, "user_id")).map(str::to_string);
    let reason = detail.and_then(|d| s(d, "reason")).map(str::to_string);
    let who = target.clone().unwrap_or_else(|| "someone".to_string());
    let (line, quote, liftable) = match action.as_str() {
        "ban" => (format!("{moderator} banned {who}"), reason, true),
        "timeout" => {
            let length = detail
                .and_then(|d| s(d, "expires_at"))
                .and_then(|e| chrono::DateTime::parse_from_rfc3339(e).ok())
                .map(|e| ((e.timestamp_millis() - at_ms) / 1000).max(1) as u32)
                .map(|secs| {
                    // Round to the length a moderator picked, not the seconds left.
                    let picked = [1, 60, 300, 600, 1_800, 3_600, 86_400, 604_800, 1_209_600]
                        .into_iter()
                        .min_by_key(|p| (*p as i64 - secs as i64).abs())
                        .filter(|p| (*p as i64 - secs as i64).abs() <= (*p as i64 / 20).max(2))
                        .unwrap_or(secs);
                    crate::services::moderation::duration_label(picked)
                });
            let line = match length {
                Some(l) => format!("{moderator} timed out {who} for {l}"),
                None => format!("{moderator} timed out {who}"),
            };
            (line, reason, true)
        }
        "unban" => (format!("{moderator} unbanned {who}"), None, false),
        "untimeout" => (format!("{moderator} lifted {who}\u{2019}s timeout"), None, false),
        "delete" => (
            format!("{moderator} deleted a message from {who}"),
            detail.and_then(|d| s(d, "message_body")).map(str::to_string),
            false,
        ),
        "warn" => (format!("{moderator} warned {who}"), reason, false),
        "clear" => (format!("{moderator} cleared chat"), None, false),
        "slow" => {
            let secs = detail.and_then(|d| d.get("wait_time_seconds")).and_then(Value::as_u64);
            (match secs {
                Some(s) => format!("{moderator} turned on slow mode ({s}s)"),
                None => format!("{moderator} turned on slow mode"),
            }, None, false)
        }
        "slowoff" => (format!("{moderator} turned off slow mode"), None, false),
        "followers" => (format!("{moderator} turned on followers-only chat"), None, false),
        "followersoff" => (format!("{moderator} turned off followers-only chat"), None, false),
        "subscribers" => (format!("{moderator} turned on subscriber-only chat"), None, false),
        "subscribersoff" => (format!("{moderator} turned off subscriber-only chat"), None, false),
        "emoteonly" => (format!("{moderator} turned on emote-only chat"), None, false),
        "emoteonlyoff" => (format!("{moderator} turned off emote-only chat"), None, false),
        "uniquechat" => (format!("{moderator} turned on unique chat"), None, false),
        "uniquechatoff" => (format!("{moderator} turned off unique chat"), None, false),
        "mod" => (format!("{moderator} made {who} a moderator"), None, false),
        "unmod" => (format!("{moderator} removed {who} as a moderator"), None, false),
        "vip" => (format!("{moderator} made {who} a VIP"), None, false),
        "unvip" => (format!("{moderator} removed {who} as a VIP"), None, false),
        "raid" => (format!("{moderator} started a raid"), None, false),
        "unraid" => (format!("{moderator} canceled the raid"), None, false),
        "add_blocked_term" | "add_permitted_term" | "remove_blocked_term" | "remove_permitted_term" => {
            let terms = detail
                .and_then(|d| d.get("terms"))
                .and_then(Value::as_array)
                .map(|t| t.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "))
                .filter(|t| !t.is_empty());
            let verb = match action.as_str() {
                "add_blocked_term" => "blocked",
                "remove_blocked_term" => "unblocked",
                "add_permitted_term" => "permitted",
                _ => "unpermitted",
            };
            (format!("{moderator} {verb} a phrase"), terms, false)
        }
        "approve_unban_request" => (format!("{moderator} approved {who}\u{2019}s unban request"), None, false),
        "deny_unban_request" => (format!("{moderator} denied {who}\u{2019}s unban request"), None, false),
        other => (format!("{moderator}: {}", other.replace('_', " ")), None, false),
    };
    Some(LogEntry {
        id: format!("{channel}-{at_ms}-{action}-{}", target_id.as_deref().unwrap_or("")),
        channel,
        action,
        moderator,
        target,
        target_id,
        line,
        quote,
        at_ms,
        liftable,
    })
}

/// Keep one entry, newest last, and answer it.
pub fn record(entry: LogEntry) -> LogEntry {
    let mut map = log();
    let ring = map.entry(entry.channel.clone()).or_default();
    if ring.len() >= PER_CHANNEL {
        ring.pop_front();
    }
    ring.push_back(entry.clone());
    entry
}

/// A `channel.moderate` event arrived: keep it and tell the shells.
pub fn on_moderate_event(event: &Value) {
    let Some(entry) = entry_from(event, now_ms()) else { return };
    let entry = record(entry);
    if let Some(app) = crate::services::providers::app_handle() {
        use tauri::Emitter;
        let _ = app.emit("room-mod-log", &entry);
    }
}

/// A channel's recent actions, newest first.
pub fn recent(channel: &str) -> Vec<LogEntry> {
    let key = channel.trim_start_matches('#').to_lowercase();
    log().get(&key).map(|r| r.iter().rev().cloned().collect()).unwrap_or_default()
}

pub fn clear_all() {
    log().clear();
}

/// The room's recent moderator actions, newest first.
#[tauri::command]
pub fn get_room_mod_log(channel: String) -> Vec<LogEntry> {
    recent(&channel)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const AT: i64 = 1_760_000_000_000;

    fn iso(ms: i64) -> String {
        chrono::DateTime::from_timestamp_millis(ms).unwrap().to_rfc3339()
    }

    #[test]
    fn a_timeout_reads_with_the_length_picked() {
        let ev = json!({
            "broadcaster_user_login": "ZackRawrr", "moderator_user_name": "Kay_Lee", "action": "timeout",
            "timeout": { "user_id": "42", "user_login": "gus", "user_name": "Gus", "reason": "spoilers",
                         "expires_at": iso(AT + 599_400) }
        });
        let e = entry_from(&ev, AT).unwrap();
        assert_eq!(e.channel, "zackrawrr");
        assert_eq!(e.line, "Kay_Lee timed out Gus for 10m");
        assert_eq!(e.quote.as_deref(), Some("spoilers"));
        assert_eq!(e.target_id.as_deref(), Some("42"));
        assert!(e.liftable);
    }

    #[test]
    fn deletes_modes_and_terms_in_words() {
        let del = json!({ "broadcaster_user_login": "c", "moderator_user_login": "kay", "action": "delete",
                          "delete": { "user_login": "gus", "message_body": "the boss dies" } });
        let e = entry_from(&del, AT).unwrap();
        assert_eq!(e.line, "kay deleted a message from gus");
        assert_eq!(e.quote.as_deref(), Some("the boss dies"));
        assert!(!e.liftable);

        let slow = json!({ "broadcaster_user_login": "c", "moderator_user_name": "Kay", "action": "slow",
                           "slow": { "wait_time_seconds": 30 } });
        assert_eq!(entry_from(&slow, AT).unwrap().line, "Kay turned on slow mode (30s)");

        let term = json!({ "broadcaster_user_login": "c", "moderator_user_name": "Kay", "action": "add_blocked_term",
                           "add_blocked_term": { "terms": ["bad", "worse"] } });
        let t = entry_from(&term, AT).unwrap();
        assert_eq!(t.line, "Kay blocked a phrase");
        assert_eq!(t.quote.as_deref(), Some("bad, worse"));

        let odd = json!({ "broadcaster_user_login": "c", "action": "shared_chat_ban" });
        assert_eq!(entry_from(&odd, AT).unwrap().line, "A moderator: shared chat ban");
        assert!(entry_from(&json!({ "action": "ban" }), AT).is_none(), "no channel, nothing kept");
    }

    #[test]
    fn the_log_keeps_the_newest_hundred_newest_first() {
        let channel = "ring-test";
        for i in 0..105 {
            let ev = json!({ "broadcaster_user_login": channel, "moderator_user_name": "Kay", "action": "ban",
                             "ban": { "user_login": format!("u{i}"), "user_id": format!("{i}") } });
            record(entry_from(&ev, AT + i).unwrap());
        }
        let got = recent(channel);
        assert_eq!(got.len(), 100);
        assert_eq!(got[0].target.as_deref(), Some("u104"));
        assert_eq!(got[99].target.as_deref(), Some("u5"));
    }
}
