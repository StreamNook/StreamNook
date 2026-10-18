//! A chatter's moderation history in one channel, for the user card.
//!
//! Twitch keeps it in `viewerCardModLogs`, the same private GQL root its own
//! viewer card reads, and only a moderator of the channel (or its
//! broadcaster) may read it; anyone else gets an error code back, which is
//! reported as `no_access`, never as an empty history. One query asks for:
//!
//! - `targetedActions`: bans, timeouts, unbans, warnings and acknowledged
//!   warnings. Twitch gives no structured moderator, length or reason, only
//!   a localized line with the moderator as a user token, so the moderator
//!   comes from that token, the length from the text, and Twitch's own line
//!   is kept as the detail.
//! - `messages` with `isDeleted`: the chatter's deleted messages. Twitch does
//!   not record who deleted one or when, only when it was sent.
//! - `comments`: the notes the channel's moderators left on the chatter.
//! - `chatRoomBanStatus`: whether a ban or timeout is in force right now,
//!   which decides between Ban and Unban on the card.
//!
//! The call is made as the viewer with the twitch.tv web session the app's
//! sign-in created, under the web client id.

use serde::Serialize;
use serde_json::{json, Value};

const QUERY: &str = r#"query ChatterModHistory($channelID: ID!, $targetID: ID!) {
  logs: viewerCardModLogs(channelID: $channelID, targetID: $targetID) {
    actions: targetedActions(first: 50) {
      __typename
      ... on ModLogsTargetedActionsConnection {
        count
        edges { node { id timestamp type localizedLabel { fallbackString localizedStringFragments { token { __typename ... on ModActionsLocalizedTextToken { text } ... on User { id login displayName } } } } } }
      }
      ... on ModLogsTargetedActionsError { code }
    }
    messages(first: 100) {
      __typename
      ... on ViewerCardModLogsMessagesConnection { edges { node { __typename ... on ViewerCardModLogsChatMessage { id sentAt isDeleted content { text } } } } }
      ... on ViewerCardModLogsMessagesError { code }
    }
    comments(first: 20) {
      __typename
      ... on ModLogsCommentConnection { edges { node { id text timestamp author { id login displayName } } } }
      ... on ModLogsCommentsError { code }
    }
  }
  banStatus: chatRoomBanStatus(channelID: $channelID, userID: $targetID) { createdAt expiresAt isPermanent reason moderator { id login displayName } }
}"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    Ban,
    Timeout,
    Unban,
    Untimeout,
    Warn,
    Acknowledged,
    Deleted,
    Comment,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HistoryEntry {
    pub id: String,
    pub kind: EntryKind,
    /// "Timed out 10 minutes", "Deleted message", "Mod comment".
    pub title: String,
    /// Who did it, when Twitch says. Deleted messages never say.
    pub by: Option<String>,
    /// Unix ms; 0 when Twitch gave no time.
    pub at_ms: i64,
    /// "3 days ago", or "sent 3 days ago" for a deleted message.
    pub when: String,
    /// The message text, the comment, or Twitch's own line for an action.
    pub quote: Option<String>,
    /// The timeout length in seconds, when the line names one.
    pub seconds: Option<u64>,
}

/// A ban or timeout in force now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CurrentBan {
    pub permanent: bool,
    /// Unix ms the timeout ends; `None` for a ban.
    pub ends_ms: Option<i64>,
    pub by: Option<String>,
    pub reason: Option<String>,
    /// "Banned" or "Timed out, 8 minutes left".
    pub label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryStatus {
    Available,
    /// Not a moderator of this channel.
    NoAccess,
    /// No twitch.tv session in the app.
    SignedOut,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModHistory {
    pub status: HistoryStatus,
    /// Newest first.
    pub entries: Vec<HistoryEntry>,
    pub current: Option<CurrentBan>,
    /// One line for a status other than `available`.
    pub note: Option<String>,
}

impl ModHistory {
    fn empty(status: HistoryStatus) -> Self {
        let note = match status {
            HistoryStatus::Available => None,
            HistoryStatus::NoAccess => Some("Only moderators of this channel can see its history.".to_string()),
            HistoryStatus::SignedOut => Some("Sign in to Twitch to see moderation history.".to_string()),
            HistoryStatus::Failed => Some("Twitch did not answer. Try again in a moment.".to_string()),
        };
        ModHistory { status, entries: Vec::new(), current: None, note }
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn parse_ms(v: &Value) -> i64 {
    v.as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.timestamp_millis())
        .unwrap_or(0)
}

/// "just now", "5 minutes ago", "3 days ago", "last month", "4 months ago".
pub fn ago(at_ms: i64, now_ms: i64) -> String {
    if at_ms <= 0 {
        return String::new();
    }
    let secs = ((now_ms - at_ms) / 1000).max(0);
    let plural = |n: i64, one: &str| if n == 1 { format!("1 {one} ago") } else { format!("{n} {one}s ago") };
    match secs {
        s if s < 60 => "just now".to_string(),
        s if s < 3_600 => plural(s / 60, "minute"),
        s if s < 86_400 => plural(s / 3_600, "hour"),
        s if s < 7 * 86_400 => plural(s / 86_400, "day"),
        s if s < 30 * 86_400 => plural(s / (7 * 86_400), "week"),
        s if s < 60 * 86_400 => "last month".to_string(),
        s if s < 365 * 86_400 => plural(s / (30 * 86_400), "month"),
        s => plural(s / (365 * 86_400), "year"),
    }
}

/// A length in words: 600 -> "10 minutes", 86400 -> "1 day", 1 -> "1 second".
pub fn length_words(seconds: u64) -> String {
    let plural = |n: u64, one: &str| if n == 1 { format!("1 {one}") } else { format!("{n} {one}s") };
    for (size, one) in [(604_800, "week"), (86_400, "day"), (3_600, "hour"), (60, "minute")] {
        if seconds >= size && seconds % size == 0 {
            return plural(seconds / size, one);
        }
    }
    plural(seconds, "second")
}

/// The first length named in a line of text: "10m", "10 minutes", "600s",
/// "1 day". Twitch writes lengths a few ways across languages and versions,
/// so this only claims a number followed by a known unit.
pub fn length_in(text: &str) -> Option<u64> {
    let lower = text.to_lowercase();
    let b = lower.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_digit() && (i == 0 || !b[i - 1].is_ascii_alphanumeric()) {
            let start = i;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            let n: u64 = lower[start..i].parse().ok()?;
            let mut j = i;
            while j < b.len() && b[j] == b' ' {
                j += 1;
            }
            let rest = &lower[j..];
            let word_end = rest.find(|c: char| !c.is_ascii_alphabetic()).unwrap_or(rest.len());
            let unit = &rest[..word_end];
            let mult = match unit {
                "s" | "sec" | "secs" | "second" | "seconds" => 1,
                "m" | "min" | "mins" | "minute" | "minutes" => 60,
                "h" | "hr" | "hrs" | "hour" | "hours" => 3_600,
                "d" | "day" | "days" => 86_400,
                "w" | "wk" | "week" | "weeks" => 604_800,
                _ => 0,
            };
            if mult > 0 {
                return Some(n * mult);
            }
        } else {
            i += 1;
        }
    }
    None
}

fn user_name(u: &Value) -> Option<String> {
    u.get("displayName")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .or_else(|| u.get("login").and_then(Value::as_str))
        .map(str::to_string)
}

fn connection_edges<'a>(node: Option<&'a Value>, ok_type: &str) -> Result<Vec<&'a Value>, Option<String>> {
    let Some(node) = node.filter(|n| !n.is_null()) else {
        return Err(None);
    };
    if node.get("__typename").and_then(Value::as_str) != Some(ok_type) {
        return Err(node.get("code").and_then(Value::as_str).map(str::to_string));
    }
    Ok(node
        .get("edges")
        .and_then(Value::as_array)
        .map(|e| e.iter().filter_map(|edge| edge.get("node")).collect())
        .unwrap_or_default())
}

fn action_entry(node: &Value, target_id: &str, now: i64) -> Option<HistoryEntry> {
    let kind = match node.get("type").and_then(Value::as_str)? {
        "BAN" => EntryKind::Ban,
        "TIMEOUT" => EntryKind::Timeout,
        "UNBAN" => EntryKind::Unban,
        "UNTIMEOUT" => EntryKind::Untimeout,
        "WARN" => EntryKind::Warn,
        "ACKNOWLEDGE_WARNING" => EntryKind::Acknowledged,
        _ => return None,
    };
    let label = node.get("localizedLabel");
    let line = label
        .and_then(|l| l.get("fallbackString"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let tokens: Vec<&Value> = label
        .and_then(|l| l.get("localizedStringFragments"))
        .and_then(Value::as_array)
        .map(|f| f.iter().filter_map(|frag| frag.get("token")).collect())
        .unwrap_or_default();
    // The moderator is the user token that is not the chatter. An
    // acknowledgement is the chatter's own act, so it names nobody else.
    let by = tokens
        .iter()
        .filter(|t| t.get("__typename").and_then(Value::as_str) == Some("User"))
        .find(|t| t.get("id").and_then(Value::as_str) != Some(target_id))
        .and_then(|t| user_name(t));
    let text: String = tokens
        .iter()
        .filter_map(|t| t.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join(" ");
    let seconds = if kind == EntryKind::Timeout {
        length_in(&text).or_else(|| line.as_deref().and_then(length_in))
    } else {
        None
    };
    let title = match (kind, seconds) {
        (EntryKind::Timeout, Some(1)) => "Purged".to_string(),
        (EntryKind::Timeout, Some(s)) => format!("Timed out {}", length_words(s)),
        (EntryKind::Timeout, None) => "Timed out".to_string(),
        (EntryKind::Ban, _) => "Banned".to_string(),
        (EntryKind::Unban, _) => "Unbanned".to_string(),
        (EntryKind::Untimeout, _) => "Timeout lifted".to_string(),
        (EntryKind::Warn, _) => "Warned".to_string(),
        (EntryKind::Acknowledged, _) => "Acknowledged a warning".to_string(),
        _ => return None,
    };
    let at_ms = parse_ms(node.get("timestamp").unwrap_or(&Value::Null));
    Some(HistoryEntry {
        id: node.get("id").and_then(Value::as_str).unwrap_or_default().to_string(),
        kind,
        title,
        by,
        at_ms,
        when: ago(at_ms, now),
        quote: line,
        seconds,
    })
}

fn current_ban(v: Option<&Value>, now: i64) -> Option<CurrentBan> {
    let v = v.filter(|v| !v.is_null())?;
    let permanent = v.get("isPermanent").and_then(Value::as_bool).unwrap_or(false);
    let ends_ms = Some(parse_ms(v.get("expiresAt").unwrap_or(&Value::Null))).filter(|ms| *ms > 0);
    if !permanent && ends_ms.map_or(true, |end| end <= now) {
        return None;
    }
    let label = match ends_ms {
        Some(end) if !permanent => {
            let left = ((end - now) / 1000).max(1) as u64;
            // Round up to the unit a person reads: "8 minutes left".
            let rounded = match left {
                s if s < 60 => s,
                s if s < 3_600 => s.div_ceil(60) * 60,
                s if s < 86_400 => s.div_ceil(3_600) * 3_600,
                s => s.div_ceil(86_400) * 86_400,
            };
            format!("Timed out, {} left", length_words(rounded))
        }
        _ => "Banned".to_string(),
    };
    Some(CurrentBan {
        permanent,
        ends_ms: if permanent { None } else { ends_ms },
        by: v.get("moderator").and_then(user_name),
        reason: v.get("reason").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string),
        label,
    })
}

/// Turn the GQL answer into the card's history. Pure, for tests.
pub fn from_response(json: &Value, target_id: &str, now: i64) -> ModHistory {
    let logs = json.pointer("/data/logs");
    let current = current_ban(json.pointer("/data/banStatus"), now);
    let Some(logs) = logs.filter(|l| !l.is_null()) else {
        let mut h = ModHistory::empty(HistoryStatus::Failed);
        h.current = current;
        return h;
    };
    let actions = connection_edges(logs.get("actions"), "ModLogsTargetedActionsConnection");
    let messages = connection_edges(logs.get("messages"), "ViewerCardModLogsMessagesConnection");
    let comments = connection_edges(logs.get("comments"), "ModLogsCommentConnection");
    if actions.is_err() && messages.is_err() {
        // Twitch answers every part with an error code for a non-moderator.
        let status = match (&actions, &messages) {
            (Err(Some(_)), _) | (_, Err(Some(_))) => HistoryStatus::NoAccess,
            _ => HistoryStatus::Failed,
        };
        let mut h = ModHistory::empty(status);
        h.current = current;
        return h;
    }
    let mut entries: Vec<HistoryEntry> = Vec::new();
    for node in actions.unwrap_or_default() {
        entries.extend(action_entry(node, target_id, now));
    }
    for node in messages.unwrap_or_default() {
        if node.get("__typename").and_then(Value::as_str) != Some("ViewerCardModLogsChatMessage")
            || node.get("isDeleted").and_then(Value::as_bool) != Some(true)
        {
            continue;
        }
        let at_ms = parse_ms(node.get("sentAt").unwrap_or(&Value::Null));
        let when = ago(at_ms, now);
        entries.push(HistoryEntry {
            id: node.get("id").and_then(Value::as_str).unwrap_or_default().to_string(),
            kind: EntryKind::Deleted,
            title: "Deleted message".to_string(),
            by: None,
            at_ms,
            when: if when.is_empty() { when } else { format!("sent {when}") },
            quote: node.pointer("/content/text").and_then(Value::as_str).map(str::to_string),
            seconds: None,
        });
    }
    for node in comments.unwrap_or_default() {
        let at_ms = parse_ms(node.get("timestamp").unwrap_or(&Value::Null));
        entries.push(HistoryEntry {
            id: node.get("id").and_then(Value::as_str).unwrap_or_default().to_string(),
            kind: EntryKind::Comment,
            title: "Mod comment".to_string(),
            by: node.get("author").and_then(user_name),
            at_ms,
            when: ago(at_ms, now),
            quote: node.get("text").and_then(Value::as_str).map(str::to_string),
            seconds: None,
        });
    }
    entries.sort_by(|a, b| b.at_ms.cmp(&a.at_ms));
    ModHistory { status: HistoryStatus::Available, entries, current, note: None }
}

/// The chatter's history in `channel_id`, read as the signed-in viewer.
pub async fn fetch(channel_id: &str, user_id: &str) -> ModHistory {
    let Some(token) = crate::services::twitch_web_gql::app_web_session().await else {
        return ModHistory::empty(HistoryStatus::SignedOut);
    };
    let body = json!({
        "operationName": "ChatterModHistory",
        "query": QUERY,
        "variables": { "channelID": channel_id, "targetID": user_id },
    });
    match crate::services::twitch_web_gql::post(Some(&token), body, "mod-history").await {
        Ok(json) => from_response(&json, user_id, now_ms()),
        Err(e) => {
            log::warn!("[mod-history] {e}");
            ModHistory::empty(HistoryStatus::Failed)
        }
    }
}

/// The chatter's moderation history in a channel, for the user card.
#[tauri::command]
pub async fn get_chatter_mod_history(channel_id: String, user_id: String) -> ModHistory {
    fetch(channel_id.trim(), user_id.trim()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_760_000_000_000;

    fn iso(ms: i64) -> String {
        chrono::DateTime::from_timestamp_millis(ms).unwrap().to_rfc3339()
    }

    #[test]
    fn lengths_read_from_text() {
        assert_eq!(length_in("for 10m"), Some(600));
        assert_eq!(length_in("timed out for 10 minutes: spam"), Some(600));
        assert_eq!(length_in("1 day"), Some(86_400));
        assert_eq!(length_in("600s"), Some(600));
        assert_eq!(length_in("no length here 42"), None);
        assert_eq!(length_in("user2 was timed out"), None);
    }

    #[test]
    fn length_words_pick_the_largest_whole_unit() {
        assert_eq!(length_words(600), "10 minutes");
        assert_eq!(length_words(86_400), "1 day");
        assert_eq!(length_words(90), "90 seconds");
        assert_eq!(length_words(1), "1 second");
    }

    #[test]
    fn ago_words() {
        assert_eq!(ago(NOW - 30_000, NOW), "just now");
        assert_eq!(ago(NOW - 3 * 86_400_000, NOW), "3 days ago");
        assert_eq!(ago(NOW - 14 * 86_400_000, NOW), "2 weeks ago");
        assert_eq!(ago(NOW - 40 * 86_400_000, NOW), "last month");
        assert_eq!(ago(0, NOW), "");
    }

    #[test]
    fn a_non_moderator_reads_no_access() {
        let json = json!({ "data": { "logs": {
            "actions": { "__typename": "ModLogsTargetedActionsError", "code": "UNAUTHORIZED" },
            "messages": { "__typename": "ViewerCardModLogsMessagesError", "code": "UNAUTHORIZED" },
            "comments": { "__typename": "ModLogsCommentsError", "code": "UNAUTHORIZED" }
        }, "banStatus": null } });
        let h = from_response(&json, "2", NOW);
        assert_eq!(h.status, HistoryStatus::NoAccess);
        assert!(h.entries.is_empty());
        assert!(h.note.is_some());
    }

    #[test]
    fn actions_deleted_messages_and_comments_merge_newest_first() {
        let json = json!({ "data": { "logs": {
            "actions": { "__typename": "ModLogsTargetedActionsConnection", "count": 2, "edges": [
                { "node": { "id": "a1", "timestamp": iso(NOW - 3 * 86_400_000), "type": "TIMEOUT", "localizedLabel": {
                    "fallbackString": "mod_kay timed out gus for 10m. Reason: spoilers",
                    "localizedStringFragments": [
                        { "token": { "__typename": "User", "id": "1", "login": "mod_kay", "displayName": "Mod_Kay" } },
                        { "token": { "__typename": "ModActionsLocalizedTextToken", "text": "timed out" } },
                        { "token": { "__typename": "User", "id": "2", "login": "gus", "displayName": "gus" } },
                        { "token": { "__typename": "ModActionsLocalizedTextToken", "text": "for 10m. Reason: spoilers" } }
                    ] } } },
                { "node": { "id": "a2", "timestamp": iso(NOW - 120 * 86_400_000), "type": "BAN", "localizedLabel": {
                    "fallbackString": "mod_kay banned gus",
                    "localizedStringFragments": [
                        { "token": { "__typename": "User", "id": "1", "login": "mod_kay", "displayName": "Mod_Kay" } }
                    ] } } },
                { "node": { "id": "a3", "timestamp": iso(NOW), "type": "SOMETHING_NEW" } }
            ] },
            "messages": { "__typename": "ViewerCardModLogsMessagesConnection", "edges": [
                { "node": { "__typename": "ViewerCardModLogsChatMessage", "id": "m1", "sentAt": iso(NOW - 2 * 86_400_000), "isDeleted": true, "content": { "text": "the boss dies" } } },
                { "node": { "__typename": "ViewerCardModLogsChatMessage", "id": "m2", "sentAt": iso(NOW - 86_400_000), "isDeleted": false, "content": { "text": "hi" } } }
            ] },
            "comments": { "__typename": "ModLogsCommentConnection", "edges": [
                { "node": { "id": "c1", "text": "alt of spam_bot?", "timestamp": iso(NOW - 60 * 60_000), "author": { "id": "1", "login": "mod_kay", "displayName": "Mod_Kay" } } }
            ] }
        }, "banStatus": null } });
        let h = from_response(&json, "2", NOW);
        assert_eq!(h.status, HistoryStatus::Available);
        let ids: Vec<&str> = h.entries.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["c1", "m1", "a1", "a2"]);
        let timeout = &h.entries[2];
        assert_eq!(timeout.title, "Timed out 10 minutes");
        assert_eq!(timeout.seconds, Some(600));
        assert_eq!(timeout.by.as_deref(), Some("Mod_Kay"));
        assert_eq!(timeout.when, "3 days ago");
        assert_eq!(h.entries[1].when, "sent 2 days ago");
        assert_eq!(h.entries[1].by, None);
        assert_eq!(h.entries[0].kind, EntryKind::Comment);
    }

    #[test]
    fn current_ban_and_timeout() {
        let banned = json!({ "data": { "logs": null, "banStatus": {
            "createdAt": iso(NOW - 1000), "expiresAt": null, "isPermanent": true, "reason": " bot links ",
            "moderator": { "id": "1", "login": "mod_kay", "displayName": "Mod_Kay" } } } });
        let c = from_response(&banned, "2", NOW).current.unwrap();
        assert!(c.permanent);
        assert_eq!(c.label, "Banned");
        assert_eq!(c.reason.as_deref(), Some("bot links"));

        let timed = json!({ "data": { "logs": null, "banStatus": {
            "createdAt": iso(NOW - 1000), "expiresAt": iso(NOW + 7 * 60_000 + 5_000), "isPermanent": false, "reason": "",
            "moderator": null } } });
        let c = from_response(&timed, "2", NOW).current.unwrap();
        assert_eq!(c.label, "Timed out, 8 minutes left");
        assert_eq!(c.reason, None);

        let over = json!({ "data": { "logs": null, "banStatus": {
            "expiresAt": iso(NOW - 1000), "isPermanent": false } } });
        assert!(from_response(&over, "2", NOW).current.is_none());
    }
}
