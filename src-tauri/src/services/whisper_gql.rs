//! Twitch's private GQL for whispers: the conversation list, one conversation's
//! history, who the web session belongs to, mark read and archive. Twitch has
//! no official endpoint for any of these; sending stays on Helix and live
//! receiving on EventSub.
//!
//! Every document is sent inline under the web client id with the twitch.tv
//! web session. The first page of either list needs nothing more; any page
//! after it (a request with a cursor) needs a Client-Integrity token, or Twitch
//! answers "failed integrity check". `first` must be 1 to 100.
//!
//! Errors are typed and never read as an empty page: a page that came back with
//! any GraphQL error is an error, so a caller can never mistake a refusal for
//! "this conversation has no more messages".

use crate::services::mention_inbox::MentionPart;
use crate::services::twitch_integrity::Integrity;
use crate::services::whisper_store::{Marker, TwitchBadge};
use serde_json::{json, Value};
use std::collections::HashMap;

/// Every conversation, newest first, with each one's newest messages nested.
pub const THREADS_DOC: &str = r#"query SnWhisperThreads($first: Int!, $cursor: Cursor, $messages: Int!) {
  currentUser {
    id
    whisperThreads(first: $first, after: $cursor) {
      pageInfo { hasNextPage }
      edges {
        cursor
        node {
          id
          unreadMessagesCount
          isArchived
          isMuted
          userLastMessageRead { id sentAt }
          participants { id login displayName chatColor profileImageURL(width: 70) displayBadges { setID version title imageURL } }
          messages(first: $messages) {
            pageInfo { hasNextPage }
            edges { cursor node { id sentAt editedAt deletedAt from { id } content { content emotes { emoteID from to } } } }
          }
        }
      }
    }
  }
}"#;

/// One conversation's messages, newest first, with its meta.
pub const MESSAGES_DOC: &str = r#"query SnWhisperMessages($id: ID!, $first: Int!, $cursor: Cursor) {
  currentUser { id }
  whisperThread(id: $id) {
    id
    unreadMessagesCount
    isArchived
    isMuted
    userLastMessageRead { id sentAt }
    participants { id login displayName chatColor profileImageURL(width: 70) displayBadges { setID version title imageURL } }
    messages(first: $first, after: $cursor) {
      pageInfo { hasNextPage }
      edges { cursor node { id sentAt editedAt deletedAt from { id } content { content emotes { emoteID from to } } } }
    }
  }
}"#;

/// Whose session a token is, checked before any write (a mutation cannot
/// select `currentUser` itself).
pub const WHO_DOC: &str = r#"query SnWhisperWho { currentUser { id } }"#;

pub const MARK_READ_DOC: &str = r#"mutation SnWhisperMarkRead($input: UpdateWhisperThreadInput!) { updateWhisperThread(input: $input) { thread { id unreadMessagesCount isArchived } } }"#;

pub const ARCHIVE_DOC: &str = r#"mutation SnWhisperArchive($input: UpdateWhisperThreadInput!) { updateWhisperThread(input: $input) { thread { id unreadMessagesCount isArchived } } }"#;

#[derive(Debug, Clone, PartialEq)]
pub enum GqlError {
    RateLimited,
    Integrity,
    Unauthorized,
    Network(String),
    Shape(String),
    Other(String),
}

impl GqlError {
    pub fn words(&self) -> String {
        match self {
            GqlError::RateLimited => "Twitch asked StreamNook to slow down.".into(),
            GqlError::Integrity => "Twitch's security check refused the request.".into(),
            GqlError::Unauthorized => "Your twitch.tv session was refused.".into(),
            GqlError::Network(e) => format!("Could not reach Twitch: {e}"),
            GqlError::Shape(e) => format!("Twitch answered in an unexpected shape: {e}"),
            GqlError::Other(e) => format!("Twitch refused the request: {e}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct GqlMessage {
    pub id: String,
    pub sent_at_ms: i64,
    pub edited_ms: Option<i64>,
    pub deleted: bool,
    pub from_id: String,
    pub text: String,
    pub parts: Vec<MentionPart>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GqlParticipant {
    pub id: String,
    pub login: String,
    pub display_name: String,
    pub color: Option<String>,
    pub avatar: Option<String>,
    pub badges: Vec<TwitchBadge>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MessagesPage {
    pub viewer_id: String,
    /// Newest first, as Twitch sends them.
    pub messages: Vec<GqlMessage>,
    pub has_next: bool,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GqlThread {
    pub thread_id: String,
    pub other: Option<GqlParticipant>,
    pub me: Option<GqlParticipant>,
    pub unread: i32,
    pub twitch_read: Option<Marker>,
    pub archived: bool,
    pub muted: bool,
    pub messages: MessagesPage,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ThreadsPage {
    pub viewer_id: String,
    pub threads: Vec<GqlThread>,
    pub has_next: bool,
    pub next_cursor: Option<String>,
}

/// How requests go out. Production is `WebGql`; tests answer from a fake.
#[async_trait::async_trait]
pub trait Gql: Send + Sync {
    async fn post(&self, body: Value, integrity: Option<&Integrity>) -> Result<Value, GqlError>;
}

/// The twitch.tv web session captured for one run.
pub struct WebGql {
    pub token: String,
}

#[async_trait::async_trait]
impl Gql for WebGql {
    async fn post(&self, body: Value, integrity: Option<&Integrity>) -> Result<Value, GqlError> {
        match crate::services::twitch_web_gql::post_status(Some(&self.token), body, "WhisperGql", integrity).await {
            Ok(v) => Ok(v),
            Err((Some(429), _)) => Err(GqlError::RateLimited),
            Err((Some(401), _)) | Err((Some(403), _)) => Err(GqlError::Unauthorized),
            Err((Some(status), e)) => Err(GqlError::Other(format!("{e} ({status})"))),
            Err((None, e)) => Err(GqlError::Network(e)),
        }
    }
}

/// Twitch's own thread id for two users: the smaller id first, compared as
/// numbers. Falls back to string order only when an id is not a number.
pub fn thread_id(a: &str, b: &str) -> String {
    let (x, y) = match (a.parse::<u64>(), b.parse::<u64>()) {
        (Ok(na), Ok(nb)) => {
            if na <= nb {
                (a, b)
            } else {
                (b, a)
            }
        }
        _ => {
            if a <= b {
                (a, b)
            } else {
                (b, a)
            }
        }
    };
    format!("{x}_{y}")
}

/// The image of a Twitch emote, the form chat uses.
pub fn twitch_emote_url(id: &str) -> String {
    format!("https://static-cdn.jtvnw.net/emoticons/v2/{id}/default/dark/2.0")
}

/// Byte offset of each unit index (one extra entry for the end). Code points:
/// every char is one unit. UTF-16: a char outside the basic plane is two units
/// and its second unit has no byte offset of its own (None).
fn offsets(text: &str, utf16: bool) -> Vec<Option<usize>> {
    let mut out = Vec::new();
    for (b, c) in text.char_indices() {
        out.push(Some(b));
        if utf16 && c.len_utf16() == 2 {
            out.push(None);
        }
    }
    out.push(Some(text.len()));
    out
}

/// The emote spans under one counting rule, as (start byte, end byte, id), or
/// None when any range is not a whole word under that rule, two ranges of the
/// same emote read different words, or ranges overlap.
fn spans(text: &str, ranges: &[(String, usize, usize)], utf16: bool) -> Option<Vec<(usize, usize, String)>> {
    let offs = offsets(text, utf16);
    let mut words: HashMap<&str, &str> = HashMap::new();
    let mut out = Vec::new();
    for (id, from, to) in ranges {
        if from > to || *to + 1 >= offs.len() {
            return None;
        }
        let start = offs[*from]?;
        let end = offs[*to + 1]?;
        let word = &text[start..end];
        if word.is_empty() || word.chars().any(char::is_whitespace) {
            return None;
        }
        let before_ok = start == 0 || text[..start].chars().last().is_some_and(char::is_whitespace);
        let after_ok = end == text.len() || text[end..].chars().next().is_some_and(char::is_whitespace);
        if !before_ok || !after_ok {
            return None;
        }
        match words.get(id.as_str()) {
            Some(seen) if *seen != word => return None,
            _ => {
                words.insert(id.as_str(), word);
            }
        }
        out.push((start, end, id.clone()));
    }
    out.sort_by_key(|s| s.0);
    if out.windows(2).any(|w| w[1].0 < w[0].1) {
        return None;
    }
    Some(out)
}

/// A message's Twitch emotes as parts. One counting rule for the whole
/// message: code points if every range is a whole word under them, else
/// UTF-16 under the same test, else no emote parts at all (the text alone).
/// Ranges are inclusive (`to` is the emote's last unit). Empty when the
/// message has no emote, since its text says everything.
pub fn emote_parts(text: &str, ranges: &[(String, usize, usize)]) -> Vec<MentionPart> {
    if ranges.is_empty() {
        return Vec::new();
    }
    let Some(found) = spans(text, ranges, false).or_else(|| spans(text, ranges, true)) else {
        return Vec::new();
    };
    let mut parts = Vec::new();
    let mut at = 0;
    for (start, end, id) in found {
        if start > at {
            parts.push(MentionPart::Text { text: text[at..start].to_string() });
        }
        parts.push(MentionPart::Emote { name: text[start..end].to_string(), url: twitch_emote_url(&id) });
        at = end;
    }
    if at < text.len() {
        parts.push(MentionPart::Text { text: text[at..].to_string() });
    }
    parts
}

fn rank(e: &GqlError) -> u8 {
    match e {
        GqlError::Integrity => 0,
        GqlError::Unauthorized => 1,
        GqlError::RateLimited => 2,
        _ => 3,
    }
}

/// The most telling error in an answer's `errors`, or None when there are none.
/// Twitch says the same thing in a code or in the message, so both are read.
pub(crate) fn errors_of(v: &Value) -> Option<GqlError> {
    let errors = v.get("errors")?.as_array()?;
    let mut worst: Option<GqlError> = None;
    for e in errors {
        let message = e.get("message").and_then(Value::as_str).unwrap_or("").to_string();
        let code = e.pointer("/extensions/code").and_then(Value::as_str).unwrap_or("");
        let low = message.to_ascii_lowercase();
        let kind = if code.eq_ignore_ascii_case("IntegrityCheckFailed") || low.contains("integrity") {
            GqlError::Integrity
        } else if code.eq_ignore_ascii_case("UNAUTHENTICATED")
            || low.contains("unauthenticated")
            || low.contains("unauthorized")
            || low.contains("authorization token")
        {
            GqlError::Unauthorized
        } else if code.eq_ignore_ascii_case("RATE_LIMITED") || low.contains("rate limit") || low.contains("too many requests") {
            GqlError::RateLimited
        } else {
            GqlError::Other(message)
        };
        worst = Some(match worst {
            Some(w) if rank(&w) <= rank(&kind) => w,
            _ => kind,
        });
    }
    worst
}

fn ms(raw: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(raw).ok().map(|d| d.timestamp_millis())
}

fn s(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

fn viewer_of(data: &Value) -> Result<String, GqlError> {
    data.pointer("/currentUser/id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or(GqlError::Unauthorized)
}

fn parse_message(node: &Value) -> Result<GqlMessage, GqlError> {
    let id = s(node, "id").ok_or_else(|| GqlError::Shape("message without id".into()))?;
    let sent_at_ms = s(node, "sentAt")
        .as_deref()
        .and_then(ms)
        .ok_or_else(|| GqlError::Shape("message without sentAt".into()))?;
    let from_id = node
        .pointer("/from/id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| GqlError::Shape("message without sender".into()))?;
    let text = node.pointer("/content/content").and_then(Value::as_str).unwrap_or("").to_string();
    let ranges: Vec<(String, usize, usize)> = node
        .pointer("/content/emotes")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|e| {
                    let id = s(e, "emoteID")?;
                    let from = e.get("from")?.as_u64()? as usize;
                    let to = e.get("to")?.as_u64()? as usize;
                    Some((id, from, to))
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(GqlMessage {
        parts: emote_parts(&text, &ranges),
        id,
        sent_at_ms,
        edited_ms: s(node, "editedAt").as_deref().and_then(ms),
        deleted: node.get("deletedAt").is_some_and(|d| d.is_string()),
        from_id,
        text,
    })
}

fn parse_participant(p: &Value) -> Option<GqlParticipant> {
    let id = s(p, "id")?;
    let badges = p
        .get("displayBadges")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .map(|b| TwitchBadge {
                    set_id: s(b, "setID").unwrap_or_default(),
                    version: s(b, "version").unwrap_or_default(),
                    title: s(b, "title").unwrap_or_default(),
                    image_url: s(b, "imageURL").unwrap_or_default(),
                })
                .filter(|b| !b.image_url.is_empty())
                .collect()
        })
        .unwrap_or_default();
    Some(GqlParticipant {
        login: s(p, "login").unwrap_or_default(),
        display_name: s(p, "displayName").unwrap_or_default(),
        color: s(p, "chatColor").filter(|c| !c.is_empty()),
        avatar: s(p, "profileImageURL").filter(|u| !u.is_empty()),
        badges,
        id,
    })
}

fn parse_messages(conn: &Value, viewer_id: &str) -> Result<MessagesPage, GqlError> {
    let edges = conn
        .get("edges")
        .and_then(Value::as_array)
        .ok_or_else(|| GqlError::Shape("messages without edges".into()))?;
    let mut messages = Vec::with_capacity(edges.len());
    for edge in edges {
        let node = edge.get("node").ok_or_else(|| GqlError::Shape("edge without node".into()))?;
        messages.push(parse_message(node)?);
    }
    let next_cursor = edges
        .last()
        .and_then(|e| s(e, "cursor"))
        .filter(|c| !c.is_empty());
    Ok(MessagesPage {
        viewer_id: viewer_id.to_string(),
        messages,
        has_next: conn.pointer("/pageInfo/hasNextPage").and_then(Value::as_bool).unwrap_or(false),
        next_cursor,
    })
}

fn parse_thread(node: &Value, viewer_id: &str) -> Result<GqlThread, GqlError> {
    let thread_id = s(node, "id").ok_or_else(|| GqlError::Shape("thread without id".into()))?;
    let people: Vec<GqlParticipant> = node
        .get("participants")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(parse_participant).collect())
        .unwrap_or_default();
    let me = people.iter().find(|p| p.id == viewer_id).cloned();
    let other = people.into_iter().find(|p| p.id != viewer_id);
    let twitch_read = node.get("userLastMessageRead").and_then(|r| {
        let id = s(r, "id")?;
        let at_ms = s(r, "sentAt").as_deref().and_then(ms)?;
        Some(Marker { id, at_ms })
    });
    let messages = match node.get("messages") {
        Some(conn) if !conn.is_null() => parse_messages(conn, viewer_id)?,
        _ => return Err(GqlError::Shape("thread without messages".into())),
    };
    Ok(GqlThread {
        thread_id,
        other,
        me,
        unread: node.get("unreadMessagesCount").and_then(Value::as_i64).unwrap_or(0) as i32,
        twitch_read,
        archived: node.get("isArchived").and_then(Value::as_bool).unwrap_or(false),
        muted: node.get("isMuted").and_then(Value::as_bool).unwrap_or(false),
        messages,
    })
}

async fn send(gql: &dyn Gql, body: Value, integrity: Option<&Integrity>) -> Result<Value, GqlError> {
    let answer = gql.post(body, integrity).await?;
    if let Some(e) = errors_of(&answer) {
        return Err(e);
    }
    answer.get("data").filter(|d| !d.is_null()).cloned().ok_or_else(|| GqlError::Shape("no data".into()))
}

/// One page of the conversation list. `nested` is how many of each
/// conversation's newest messages come along (1 to 100).
pub async fn threads(
    gql: &dyn Gql,
    first: u32,
    nested: u32,
    cursor: Option<&str>,
    integrity: Option<&Integrity>,
) -> Result<ThreadsPage, GqlError> {
    let mut variables = json!({ "first": first.clamp(1, 100), "messages": nested.clamp(1, 100) });
    if let Some(c) = cursor {
        variables["cursor"] = json!(c);
    }
    let body = json!({ "operationName": "SnWhisperThreads", "query": THREADS_DOC, "variables": variables });
    let data = send(gql, body, integrity).await?;
    let viewer_id = viewer_of(&data)?;
    let conn = data
        .pointer("/currentUser/whisperThreads")
        .filter(|c| !c.is_null())
        .ok_or_else(|| GqlError::Shape("no whisperThreads".into()))?;
    let edges = conn
        .get("edges")
        .and_then(Value::as_array)
        .ok_or_else(|| GqlError::Shape("whisperThreads without edges".into()))?;
    let mut threads = Vec::with_capacity(edges.len());
    for edge in edges {
        let node = edge.get("node").ok_or_else(|| GqlError::Shape("thread edge without node".into()))?;
        threads.push(parse_thread(node, &viewer_id)?);
    }
    let next_cursor = edges.last().and_then(|e| s(e, "cursor")).filter(|c| !c.is_empty());
    Ok(ThreadsPage {
        has_next: conn.pointer("/pageInfo/hasNextPage").and_then(Value::as_bool).unwrap_or(false),
        viewer_id,
        threads,
        next_cursor,
    })
}

/// One page of a conversation's messages, with its meta. None when Twitch has
/// no such conversation (nobody has written in it yet).
pub async fn messages(
    gql: &dyn Gql,
    thread_id: &str,
    first: u32,
    cursor: Option<&str>,
    integrity: Option<&Integrity>,
) -> Result<Option<GqlThread>, GqlError> {
    let mut variables = json!({ "id": thread_id, "first": first.clamp(1, 100) });
    if let Some(c) = cursor {
        variables["cursor"] = json!(c);
    }
    let body = json!({ "operationName": "SnWhisperMessages", "query": MESSAGES_DOC, "variables": variables });
    let data = send(gql, body, integrity).await?;
    let viewer_id = viewer_of(&data)?;
    match data.get("whisperThread") {
        Some(node) if !node.is_null() => Ok(Some(parse_thread(node, &viewer_id)?)),
        _ => Ok(None),
    }
}

/// The user id the session belongs to.
pub async fn who(gql: &dyn Gql) -> Result<String, GqlError> {
    let body = json!({ "operationName": "SnWhisperWho", "query": WHO_DOC, "variables": {} });
    let data = send(gql, body, None).await?;
    viewer_of(&data)
}

pub async fn mark_read(
    gql: &dyn Gql,
    thread_id: &str,
    message_id: &str,
    integrity: Option<&Integrity>,
) -> Result<(), GqlError> {
    let body = json!({
        "operationName": "SnWhisperMarkRead",
        "query": MARK_READ_DOC,
        "variables": { "input": { "threadID": thread_id, "lastReadMessageID": message_id } },
    });
    let data = send(gql, body, integrity).await?;
    data.pointer("/updateWhisperThread/thread/id")
        .map(|_| ())
        .ok_or_else(|| GqlError::Shape("mark read answered no thread".into()))
}

pub async fn archive(gql: &dyn Gql, thread_id: &str, integrity: Option<&Integrity>) -> Result<(), GqlError> {
    let body = json!({
        "operationName": "SnWhisperArchive",
        "query": ARCHIVE_DOC,
        "variables": { "input": { "threadID": thread_id, "isArchived": true } },
    });
    let data = send(gql, body, integrity).await?;
    data.pointer("/updateWhisperThread/thread/id")
        .map(|_| ())
        .ok_or_else(|| GqlError::Shape("archive answered no thread".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct Canned {
        answers: Mutex<Vec<Result<Value, GqlError>>>,
        bodies: Mutex<Vec<(Value, bool)>>,
    }

    impl Canned {
        fn new(answers: Vec<Result<Value, GqlError>>) -> Self {
            Canned { answers: Mutex::new(answers), bodies: Mutex::new(Vec::new()) }
        }
    }

    #[async_trait::async_trait]
    impl Gql for Canned {
        async fn post(&self, body: Value, integrity: Option<&Integrity>) -> Result<Value, GqlError> {
            self.bodies.lock().unwrap().push((body, integrity.is_some()));
            self.answers.lock().unwrap().remove(0)
        }
    }

    fn message(id: &str, from: &str, text: &str, at: &str) -> Value {
        json!({ "cursor": "1", "node": { "id": id, "sentAt": at, "editedAt": null, "deletedAt": null,
            "from": { "id": from }, "content": { "content": text, "emotes": [] } } })
    }

    fn thread_node(id: &str, other: &str, msgs: Vec<Value>, has_next: bool) -> Value {
        json!({ "id": id, "unreadMessagesCount": 2, "isArchived": false, "isMuted": true,
            "userLastMessageRead": { "id": "m1", "sentAt": "2030-01-01T00:00:00Z" },
            "participants": [
                { "id": "99999999", "login": "me_login", "displayName": "Me", "chatColor": null, "profileImageURL": null, "displayBadges": [] },
                { "id": other, "login": "pal", "displayName": "Pal", "chatColor": "#FF0000", "profileImageURL": "https://img/pal.png",
                  "displayBadges": [ { "setID": "turbo", "version": "1", "title": "Turbo", "imageURL": "https://img/turbo.png" } ] }
            ],
            "messages": { "pageInfo": { "hasNextPage": has_next }, "edges": msgs } })
    }

    #[test]
    fn thread_id_orders_ids_numerically() {
        assert_eq!(thread_id("99999999", "1000000000"), "99999999_1000000000");
        assert_eq!(thread_id("1000000000", "99999999"), "99999999_1000000000");
        assert_eq!(thread_id("abc", "abd"), "abc_abd");
    }

    #[test]
    fn emote_parts_inclusive_to() {
        let parts = emote_parts("hi Kappa there", &[("25".into(), 3, 7)]);
        assert_eq!(
            parts,
            vec![
                MentionPart::Text { text: "hi ".into() },
                MentionPart::Emote { name: "Kappa".into(), url: twitch_emote_url("25") },
                MentionPart::Text { text: " there".into() },
            ]
        );
        assert!(emote_parts("plain words", &[]).is_empty());
    }

    #[test]
    fn emote_parts_multi_emoji_shift() {
        // Two emoji outside the basic plane before the emote. In code points
        // "Kappa" starts at 5; in UTF-16 at 7. A range given in UTF-16 units
        // reads a non-word under code points, so the UTF-16 rule is taken and
        // the emote lands on the right word, never on a neighbour.
        let text = "\u{1F600}\u{1F600} a Kappa";
        let utf16 = emote_parts(text, &[("25".into(), 7, 11)]);
        assert!(utf16.iter().any(|p| matches!(p, MentionPart::Emote { name, .. } if name == "Kappa")));
        let cp = emote_parts(text, &[("25".into(), 5, 9)]);
        assert!(cp.iter().any(|p| matches!(p, MentionPart::Emote { name, .. } if name == "Kappa")));
    }

    #[test]
    fn emote_parts_mixed_rules_drop() {
        // One range is a word only by code points, the other only by UTF-16:
        // no single rule fits both, so no emote parts at all.
        let text = "\u{1F600} Kappa \u{1F600} Pog";
        let ranges = vec![("25".into(), 2, 6), ("88".into(), 11, 13)];
        assert!(emote_parts(text, &ranges).is_empty());
        // The same emote id reading two different words is refused too.
        assert!(emote_parts("Kappa Pog", &[("25".into(), 0, 4), ("25".into(), 6, 8)]).is_empty());
    }

    #[tokio::test]
    async fn parses_a_threads_page() {
        let answer = json!({ "data": { "currentUser": { "id": "99999999", "whisperThreads": {
            "pageInfo": { "hasNextPage": true },
            "edges": [ { "cursor": "", "node": thread_node("99999999_1000000000", "1000000000",
                vec![message("m2", "1000000000", "hello", "2030-01-01T00:01:00Z"), message("m1", "99999999", "hey", "2030-01-01T00:00:00Z")], false) },
                { "cursor": "page-2", "node": thread_node("99999999_1000000001", "1000000001", vec![], false) } ] } } } });
        let gql = Canned::new(vec![Ok(answer)]);
        let page = threads(&gql, 100, 20, None, None).await.unwrap();
        assert_eq!(page.viewer_id, "99999999");
        assert!(page.has_next);
        assert_eq!(page.next_cursor.as_deref(), Some("page-2"));
        let t = &page.threads[0];
        assert_eq!(t.other.as_ref().unwrap().id, "1000000000");
        assert_eq!(t.other.as_ref().unwrap().badges[0].set_id, "turbo");
        assert_eq!(t.me.as_ref().unwrap().login, "me_login");
        assert!(t.muted);
        assert_eq!(t.twitch_read.as_ref().unwrap().id, "m1");
        assert_eq!(t.messages.messages[0].id, "m2");
        let (body, integ) = gql.bodies.lock().unwrap()[0].clone();
        assert_eq!(body["operationName"], "SnWhisperThreads");
        assert!(body["query"].as_str().unwrap().starts_with("query SnWhisperThreads("));
        assert!(body["variables"].get("cursor").is_none());
        assert!(!integ);
    }

    #[tokio::test]
    async fn error_is_not_an_empty_page() {
        let answer = json!({ "errors": [ { "message": "failed integrity check", "path": ["whisperThread", "messages"],
            "extensions": { "code": "IntegrityCheckFailed" } } ], "data": { "currentUser": { "id": "99999999" }, "whisperThread": { "messages": null } } });
        let gql = Canned::new(vec![Ok(answer)]);
        assert_eq!(messages(&gql, "99999999_1000000000", 100, Some("c"), None).await, Err(GqlError::Integrity));
        let answer = json!({ "errors": [ { "message": "service error" } ], "data": null });
        let gql = Canned::new(vec![Ok(answer)]);
        assert!(matches!(threads(&gql, 100, 20, None, None).await, Err(GqlError::Other(_))));
    }

    #[test]
    fn errors_are_typed_from_code_or_words() {
        assert_eq!(errors_of(&json!({ "errors": [ { "message": "x", "extensions": { "code": "IntegrityCheckFailed" } } ] })), Some(GqlError::Integrity));
        assert_eq!(errors_of(&json!({ "errors": [ { "message": "Unauthenticated" } ] })), Some(GqlError::Unauthorized));
        assert_eq!(errors_of(&json!({ "errors": [ { "message": "rate limit exceeded" } ] })), Some(GqlError::RateLimited));
        assert_eq!(
            errors_of(&json!({ "errors": [ { "message": "something" }, { "message": "failed integrity check" } ] })),
            Some(GqlError::Integrity)
        );
        assert_eq!(errors_of(&json!({ "data": {} })), None);
    }

    #[tokio::test]
    async fn mutations_name_their_document_and_input() {
        let ok = json!({ "data": { "updateWhisperThread": { "thread": { "id": "t", "unreadMessagesCount": 0, "isArchived": false } } } });
        let gql = Canned::new(vec![Ok(ok.clone()), Ok(ok)]);
        mark_read(&gql, "99999999_1000000000", "m9", None).await.unwrap();
        archive(&gql, "99999999_1000000000", None).await.unwrap();
        let bodies = gql.bodies.lock().unwrap();
        assert_eq!(bodies[0].0["operationName"], "SnWhisperMarkRead");
        assert!(bodies[0].0["query"].as_str().unwrap().starts_with("mutation SnWhisperMarkRead("));
        assert_eq!(bodies[0].0["variables"]["input"], json!({ "threadID": "99999999_1000000000", "lastReadMessageID": "m9" }));
        assert_eq!(bodies[1].0["variables"]["input"], json!({ "threadID": "99999999_1000000000", "isArchived": true }));
    }

    #[tokio::test]
    async fn who_reads_the_viewer() {
        let gql = Canned::new(vec![Ok(json!({ "data": { "currentUser": { "id": "99999999" } } })), Ok(json!({ "data": { "currentUser": null } }))]);
        assert_eq!(who(&gql).await, Ok("99999999".into()));
        assert_eq!(who(&gql).await, Err(GqlError::Unauthorized));
    }
}
