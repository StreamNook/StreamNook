//! A channel's recent chat from Twitch itself, for the join backfill.
//!
//! twitch.tv fills a chat it opens with `MessageBufferChatHistory`, a
//! persisted query that answers the channel's recent messages for a signed-in
//! viewer (anonymously it answers `Unauthenticated`). Read with the viewer's
//! twitch.tv web session, it covers every channel, where the recent-messages
//! mirror only holds channels it already follows. Signed out, or when Twitch
//! does not answer, the backfill keeps using the mirror.
//!
//! Each message becomes the IRC line the mirror would have sent (tags, emote
//! ranges, reply parent, `historical=1`), so the one history parser turns both
//! into rows, and chat rules, hidden channels and emote stamping apply alike.

use std::fmt::Write as _;
use std::time::Duration;

use serde_json::{json, Value};

use crate::services::auth_proxy::TWITCH_WEB_CLIENT_ID;
use crate::services::twitch_auth_service::TwitchAuthService;
use crate::services::twitch_service::gql_device_id;

const GQL_URL: &str = "https://gql.twitch.tv/gql";
const HISTORY_HASH: &str = "5c638a13b9084d9bdcbe9c06cdd35b3ffdeeeea497bb16abd327c783c44d4398";
/// The session read must not hold up a join; the mirror answers instead.
const TOKEN_BUDGET: Duration = Duration::from_secs(2);
const REQUEST_BUDGET: Duration = Duration::from_secs(6);

/// The channel's recent messages as history IRC lines, oldest first. `None`
/// when there is no web session or Twitch did not answer with a list, so the
/// caller falls back to the mirror.
pub async fn fetch(channel: &str, auth: &TwitchAuthService) -> Option<Vec<String>> {
    let token = match tokio::time::timeout(TOKEN_BUDGET, auth.get_token()).await {
        Ok(Ok(t)) if !t.is_empty() => t,
        _ => return None,
    };
    let body = json!({
        "operationName": "MessageBufferChatHistory",
        "variables": { "channelLogin": channel },
        "extensions": { "persistedQuery": { "version": 1, "sha256Hash": HISTORY_HASH } }
    });
    let send = crate::services::http::client()
        .post(GQL_URL)
        .header("Client-ID", TWITCH_WEB_CLIENT_ID)
        .header("Authorization", format!("OAuth {token}"))
        .header("X-Device-ID", gql_device_id())
        .json(&body)
        .send();
    let answer: Value = match tokio::time::timeout(REQUEST_BUDGET, send).await {
        Ok(Ok(resp)) if resp.status().is_success() => resp.json().await.ok()?,
        Ok(Ok(resp)) => {
            log::debug!("[ChatHistory] {channel}: Twitch answered {}", resp.status());
            return None;
        }
        Ok(Err(e)) => {
            log::debug!("[ChatHistory] {channel}: Twitch request failed: {e}");
            return None;
        }
        Err(_) => {
            log::debug!("[ChatHistory] {channel}: Twitch history timed out");
            return None;
        }
    };
    let lines = lines_from(&answer, channel);
    if lines.is_none() {
        log::debug!("[ChatHistory] {channel}: Twitch history unusable: {}", answer.get("errors").cloned().unwrap_or_default());
    }
    lines
}

/// IRCv3 tag-value escaping.
fn escape_tag(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    for c in v.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            ';' => out.push_str("\\:"),
            ' ' => out.push_str("\\s"),
            '\r' => out.push_str("\\r"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out
}

fn text_at<'a>(v: &'a Value, path: &str) -> &'a str {
    v.pointer(path).and_then(Value::as_str).unwrap_or("")
}

/// The `emotes` tag for a message's fragments: each emote fragment's
/// codepoint range in the joined text, grouped by emote id in first-seen
/// order, as IRC writes it.
fn emotes_tag(fragments: &[Value]) -> String {
    let mut order: Vec<String> = Vec::new();
    let mut ranges: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    let mut pos = 0usize;
    for f in fragments {
        let text = f.get("text").and_then(Value::as_str).unwrap_or("");
        let len = text.chars().count();
        if let Some(id) = f.pointer("/content/emoteID").and_then(Value::as_str).filter(|s| !s.is_empty()) {
            if len > 0 {
                if !ranges.contains_key(id) {
                    order.push(id.to_string());
                }
                ranges.entry(id.to_string()).or_default().push(format!("{}-{}", pos, pos + len - 1));
            }
        }
        pos += len;
    }
    order
        .iter()
        .map(|id| format!("{}:{}", id, ranges[id].join(",")))
        .collect::<Vec<_>>()
        .join("/")
}

/// One history line for a `recentChatMessages` entry, or `None` for one with
/// no id, sender or text, or one a moderator deleted.
fn line_for(m: &Value, channel: &str, room_id: &str) -> Option<String> {
    let id = text_at(m, "/id");
    let login = text_at(m, "/sender/login").to_lowercase();
    let user_id = text_at(m, "/sender/id");
    let fragments = m.pointer("/content/fragments").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut text: String = if fragments.is_empty() {
        text_at(m, "/content/text").to_string()
    } else {
        fragments.iter().map(|f| f.get("text").and_then(Value::as_str).unwrap_or("")).collect()
    };
    if id.is_empty() || login.is_empty() || text.trim().is_empty() || !m.get("deletedAt").is_none_or(Value::is_null) {
        return None;
    }
    text = text.replace(['\r', '\n'], " ");
    let sent_ms = chrono::DateTime::parse_from_rfc3339(text_at(m, "/sentAt"))
        .map(|t| t.timestamp_millis())
        .unwrap_or(0);
    let badges: Vec<String> = m
        .get("senderBadges")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|b| {
                    let set = b.get("setID").and_then(Value::as_str)?;
                    let version = b.get("version").and_then(Value::as_str).unwrap_or("1");
                    Some(format!("{set}/{version}"))
                })
                .collect()
        })
        .unwrap_or_default();
    let display = text_at(m, "/sender/displayName");

    let mut tags = String::from("@historical=1");
    let _ = write!(tags, ";badges={}", badges.join(","));
    let _ = write!(tags, ";color={}", escape_tag(text_at(m, "/senderChatColor")));
    let _ = write!(tags, ";display-name={}", escape_tag(if display.is_empty() { &login } else { display }));
    let _ = write!(tags, ";emotes={}", emotes_tag(&fragments));
    let _ = write!(tags, ";id={}", escape_tag(id));
    if !room_id.is_empty() {
        let _ = write!(tags, ";room-id={}", escape_tag(room_id));
    }
    if let Some(source) = m.pointer("/sourceChannel/id").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        let _ = write!(tags, ";source-room-id={}", escape_tag(source));
    }
    if sent_ms > 0 {
        let _ = write!(tags, ";tmi-sent-ts={sent_ms}");
    }
    let _ = write!(tags, ";user-id={}", escape_tag(user_id));
    if let Some(parent) = m.get("parentMessage").filter(|p| !p.is_null()) {
        let pid = text_at(parent, "/id");
        if !pid.is_empty() {
            let _ = write!(tags, ";reply-parent-msg-id={}", escape_tag(pid));
            let _ = write!(tags, ";reply-parent-user-id={}", escape_tag(text_at(parent, "/sender/id")));
            let _ = write!(tags, ";reply-parent-user-login={}", escape_tag(&text_at(parent, "/sender/login").to_lowercase()));
            let _ = write!(tags, ";reply-parent-display-name={}", escape_tag(text_at(parent, "/sender/displayName")));
            let _ = write!(tags, ";reply-parent-msg-body={}", escape_tag(text_at(parent, "/content/text")));
        }
    }
    Some(format!("{tags} :{login}!{login}@{login}.tmi.twitch.tv PRIVMSG #{channel} :{text}"))
}

/// History lines from a `MessageBufferChatHistory` answer, oldest first.
/// `None` when the answer carries no message list (an error, a signed-out
/// session), which is different from a channel with nothing recent.
pub fn lines_from(answer: &Value, channel: &str) -> Option<Vec<String>> {
    let ch = answer.pointer("/data/channel")?;
    let messages = ch.get("recentChatMessages")?.as_array()?;
    let room_id = ch.get("id").and_then(Value::as_str).unwrap_or("");
    let mut lines: Vec<(i64, String)> = messages
        .iter()
        .filter_map(|m| {
            let at = chrono::DateTime::parse_from_rfc3339(text_at(m, "/sentAt"))
                .map(|t| t.timestamp_millis())
                .unwrap_or(0);
            line_for(m, channel, room_id).map(|l| (at, l))
        })
        .collect();
    lines.sort_by_key(|(at, _)| *at);
    Some(lines.into_iter().map(|(_, l)| l).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape twitch.tv got back for kathi's chat.
    fn answer() -> Value {
        json!({ "data": { "channel": { "id": "418422047", "recentChatMessages": [
            {
                "id": "cdb30a03", "deletedAt": null, "sentAt": "2026-10-13T17:45:15.80728995Z",
                "content": { "text": "$fish", "fragments": [ { "text": "$fish", "content": null } ] },
                "parentMessage": null,
                "sender": { "id": "1384497273", "login": "orro_lol", "displayName": "orro_lol" },
                "senderBadges": [ { "setID": "vip", "version": "1" } ],
                "senderChatColor": null, "sourceChannel": null
            },
            {
                "id": "8d37bcd5", "deletedAt": null, "sentAt": "2026-10-13T17:45:08.239564406Z",
                "content": { "text": "I wonder scrajj Kappa hi Kappa", "fragments": [
                    { "text": "I wonder scrajj ", "content": null },
                    { "text": "Kappa", "content": { "emoteID": "25" } },
                    { "text": " hi ", "content": null },
                    { "text": "Kappa", "content": { "emoteID": "25" } }
                ] },
                "parentMessage": { "id": "p1", "content": { "text": "hey; you" }, "sender": { "id": "9", "login": "Bob", "displayName": "Bob" } },
                "sender": { "id": "67534991", "login": "robits", "displayName": "Robits" },
                "senderBadges": [ { "setID": "vip", "version": "1" }, { "setID": "subscriber", "version": "3" } ],
                "senderChatColor": "#FF4500", "sourceChannel": null
            },
            {
                "id": "gone", "deletedAt": "2026-10-13T17:46:00Z", "sentAt": "2026-10-13T17:45:30Z",
                "content": { "text": "spam", "fragments": [] },
                "sender": { "id": "1", "login": "x", "displayName": "x" }
            }
        ] } } })
    }

    #[test]
    fn messages_become_history_lines_oldest_first_without_deleted_ones() {
        let lines = lines_from(&answer(), "kathi").unwrap();
        assert_eq!(lines.len(), 2, "the deleted one is left out");
        assert!(lines[0].contains(";id=8d37bcd5"), "sorted by sentAt: {}", lines[0]);
        assert!(lines[1].contains(";id=cdb30a03"));
        let first = &lines[0];
        assert!(first.starts_with("@historical=1;badges=vip/1,subscriber/3;color=#FF4500;display-name=Robits;"));
        assert!(first.contains(";emotes=25:16-20,25-29;"), "{first}");
        assert!(first.contains(";room-id=418422047;"));
        assert!(first.contains(";tmi-sent-ts=1791913508239;"));
        assert!(first.contains(";reply-parent-msg-body=hey\\:\\syou"), "tag escaping: {first}");
        assert!(first.contains(";reply-parent-user-login=bob"));
        assert!(first.ends_with(" :robits!robits@robits.tmi.twitch.tv PRIVMSG #kathi :I wonder scrajj Kappa hi Kappa"));
    }

    #[test]
    fn an_answer_without_a_list_is_none_and_an_empty_list_is_some() {
        let refused = json!({ "errors": [ { "message": "unauthenticated" } ], "data": { "channel": { "id": "1", "recentChatMessages": null } } });
        assert_eq!(lines_from(&refused, "kathi"), None);
        let quiet = json!({ "data": { "channel": { "id": "1", "recentChatMessages": [] } } });
        assert_eq!(lines_from(&quiet, "kathi"), Some(vec![]));
    }

    #[tokio::test]
    async fn the_lines_parse_into_rows_with_emotes_and_reply_parents() {
        let lines = lines_from(&answer(), "kathi").unwrap();
        let rows = crate::services::irc_service::IrcService::parse_historical_messages(lines).await;
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].username, "robits");
        assert_eq!(rows[0].tags.get("historical").map(String::as_str), Some("1"));
        assert!(rows[0].metadata.reply_info.is_some());
        assert_eq!(rows[1].content, "$fish");
    }
}
