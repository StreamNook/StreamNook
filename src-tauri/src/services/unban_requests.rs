//! Unban requests waiting on a channel's moderators.
//!
//! A banned chatter can ask to be let back in. Twitch keeps the pending ones
//! behind Helix `moderation/unban_requests`; the moderation socket also hears
//! each new one live (`channel.unban_request.create`). Rust keeps one pending
//! list per channel, merged from both, and pushes `unban-requests`
//! ({channel, requests}) on every change so the desktop strip and the phones'
//! Moderation sheet draw the same rows.
//!
//! A request leaves the list when a moderator here approves or denies it, or
//! when `channel.moderate` reports someone else did (approve_unban_request /
//! deny_unban_request), or the chatter was simply unbanned. Resolution goes
//! through Helix PATCH (scope `moderator:manage:unban_requests`).
//!
//! In memory only: the list is the room's live picture while the app runs.

use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::services::twitch_service::TwitchService;

const HELIX_URL: &str = "https://api.twitch.tv/helix/moderation/unban_requests";
/// Pages of 100 read per refresh; past this the rest wait for the next one.
const PAGE_CAP: usize = 10;
/// Twitch's limit on the reply the chatter sees.
const REPLY_MAX: usize = 500;
/// How often a pane's read goes back to Helix; EventSub keeps it live between.
const REFRESH_MS: i64 = 5 * 60 * 1000;
/// How long a resolved chatter stays out of a Helix read that was already in
/// flight when the resolution landed.
const GONE_MS: i64 = 2 * 60 * 1000;
const PER_CHANNEL_CAP: usize = 200;

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct UnbanRequest {
    pub id: String,
    /// Lowercase broadcaster login (the chat slice key).
    pub channel: String,
    pub broadcaster_id: String,
    pub user_id: String,
    pub user_login: String,
    pub user_name: String,
    /// What the chatter wrote.
    pub text: String,
    pub created_at_ms: i64,
    /// How long ago it was sent, in words: "4 minutes ago".
    pub age: String,
}

#[derive(Default)]
struct ChannelState {
    /// Pending, oldest first.
    rows: Vec<UnbanRequest>,
    /// When Helix was last read for this channel (0 = never).
    loaded_at_ms: i64,
    /// Chatters whose request was just resolved, with when.
    gone: HashMap<String, i64>,
}

static STATE: OnceLock<Mutex<HashMap<String, ChannelState>>> = OnceLock::new();

fn state() -> std::sync::MutexGuard<'static, HashMap<String, ChannelState>> {
    STATE.get_or_init(|| Mutex::new(HashMap::new())).lock().unwrap_or_else(|e| e.into_inner())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn s(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").trim().to_string()
}

fn key_of(channel: &str) -> String {
    channel.trim().trim_start_matches('#').to_lowercase()
}

fn iso_ms(v: &Value, key: &str) -> Option<i64> {
    v.get(key)
        .and_then(Value::as_str)
        .and_then(|iso| chrono::DateTime::parse_from_rfc3339(iso).ok())
        .map(|d| d.timestamp_millis())
}

/// One request from either source. Helix names the channel `broadcaster_login`,
/// EventSub `broadcaster_user_login`; both name the chatter `user_*`.
fn request_from(v: &Value, now: i64) -> Option<UnbanRequest> {
    let id = s(v, "id");
    let user_id = s(v, "user_id");
    if id.is_empty() || user_id.is_empty() {
        return None;
    }
    let channel = {
        let login = s(v, "broadcaster_user_login");
        if login.is_empty() { s(v, "broadcaster_login") } else { login }
    }
    .to_lowercase();
    let broadcaster_id = {
        let bid = s(v, "broadcaster_user_id");
        if bid.is_empty() { s(v, "broadcaster_id") } else { bid }
    };
    let created_at_ms = iso_ms(v, "created_at").unwrap_or(now);
    Some(UnbanRequest {
        id,
        channel,
        broadcaster_id,
        user_id,
        user_login: s(v, "user_login"),
        user_name: s(v, "user_name"),
        text: v.get("text").and_then(Value::as_str).unwrap_or("").trim().to_string(),
        created_at_ms,
        age: String::new(),
    })
}

/// A `channel.unban_request.create` event as a row.
pub fn from_create_event(event: &Value, now: i64) -> Option<UnbanRequest> {
    request_from(event, now)
}

/// One Helix page: its pending rows and the cursor to the next page.
pub fn parse_page(json: &Value, now: i64) -> (Vec<UnbanRequest>, Option<String>) {
    let rows: Vec<UnbanRequest> = json
        .get("data")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter(|r| r.get("status").and_then(Value::as_str).map_or(true, |st| st == "pending"))
                .filter_map(|r| request_from(r, now))
                .collect()
        })
        .unwrap_or_default();
    let cursor = json
        .pointer("/pagination/cursor")
        .and_then(Value::as_str)
        .filter(|c| !c.is_empty())
        .map(String::from);
    (rows, cursor)
}

/// The list after a Helix read. A complete read is the truth, plus whatever
/// EventSub delivered after the read began; a read cut short by the page cap
/// only adds. Chatters resolved meanwhile stay out either way.
fn merge(
    helix: Vec<UnbanRequest>,
    memory: &[UnbanRequest],
    complete: bool,
    started_ms: i64,
    gone: &HashMap<String, i64>,
) -> Vec<UnbanRequest> {
    let mut out: Vec<UnbanRequest> = helix;
    for row in memory {
        let keep = !complete || row.created_at_ms >= started_ms;
        if keep && !out.iter().any(|r| r.id == row.id) {
            out.push(row.clone());
        }
    }
    out.retain(|r| gone.get(&r.user_id).map_or(true, |at| r.created_at_ms > *at));
    out.sort_by_key(|r| r.created_at_ms);
    if out.len() > PER_CHANNEL_CAP {
        let extra = out.len() - PER_CHANNEL_CAP;
        out.drain(..extra);
    }
    out
}

/// The reply as Twitch takes it: trimmed, empty means none, at most 500
/// characters.
pub fn reply_text(text: Option<String>) -> Option<String> {
    let t = text?.trim().to_string();
    if t.is_empty() {
        return None;
    }
    Some(t.chars().take(REPLY_MAX).collect::<String>().trim_end().to_string())
}

/// What a moderator reads when a resolution is refused.
#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    /// 400: the channel turned unban requests off. The list empties.
    NotReceiving,
    /// 404: already approved, denied or withdrawn. The row drops.
    AlreadyHandled,
    NotAllowed,
    Other,
}

pub fn refusal(status: u16) -> Refusal {
    match status {
        400 => Refusal::NotReceiving,
        404 => Refusal::AlreadyHandled,
        401 | 403 => Refusal::NotAllowed,
        _ => Refusal::Other,
    }
}

pub fn refusal_words(r: &Refusal) -> &'static str {
    match r {
        Refusal::NotReceiving => "This channel doesn't take unban requests",
        Refusal::AlreadyHandled => "Someone already handled this request",
        Refusal::NotAllowed => "Only this channel's moderators can answer unban requests",
        Refusal::Other => "Twitch didn't take that, try again",
    }
}

/// The channel's rows with their ages worded for now.
fn snapshot(channel: &str, now: i64) -> Vec<UnbanRequest> {
    let map = state();
    map.get(channel)
        .map(|st| {
            st.rows
                .iter()
                .cloned()
                .map(|mut r| {
                    r.age = crate::services::mod_history::ago(r.created_at_ms, now);
                    r
                })
                .collect()
        })
        .unwrap_or_default()
}

fn emit(channel: &str) {
    let requests = snapshot(channel, now_ms());
    if let Some(app) = crate::services::providers::app_handle() {
        use tauri::Emitter;
        let _ = app.emit(
            "unban-requests",
            serde_json::json!({ "channel": channel, "requests": requests }),
        );
    }
}

/// Keep a new request. Returns whether the list changed.
fn add(row: UnbanRequest) -> bool {
    if row.channel.is_empty() {
        return false;
    }
    let mut map = state();
    let st = map.entry(row.channel.clone()).or_default();
    if st.rows.iter().any(|r| r.id == row.id) {
        return false;
    }
    // A chatter has one request at a time; a new one replaces the old.
    st.rows.retain(|r| r.user_id != row.user_id);
    st.gone.remove(&row.user_id);
    st.rows.push(row);
    st.rows.sort_by_key(|r| r.created_at_ms);
    if st.rows.len() > PER_CHANNEL_CAP {
        let extra = st.rows.len() - PER_CHANNEL_CAP;
        st.rows.drain(..extra);
    }
    true
}

/// Drop a chatter's request in a channel. Returns whether one was there.
fn remove_user(channel: &str, user_id: &str, now: i64) -> bool {
    let mut map = state();
    // A channel never read has no requests and no read in flight to guard.
    let Some(st) = map.get_mut(channel) else { return false };
    st.gone.retain(|_, at| now - *at < GONE_MS);
    st.gone.insert(user_id.to_string(), now);
    let before = st.rows.len();
    st.rows.retain(|r| r.user_id != user_id);
    st.rows.len() != before
}

/// Drop a request by id, wherever it is. Returns its channel.
fn remove_id(request_id: &str, now: i64) -> Option<String> {
    let mut map = state();
    for (channel, st) in map.iter_mut() {
        if let Some(pos) = st.rows.iter().position(|r| r.id == request_id) {
            let row = st.rows.remove(pos);
            st.gone.retain(|_, at| now - *at < GONE_MS);
            st.gone.insert(row.user_id, now);
            return Some(channel.clone());
        }
    }
    None
}

/// `channel.unban_request.create` arrived on the moderation socket.
pub fn on_create(event: &Value) {
    let Some(row) = from_create_event(event, now_ms()) else { return };
    let channel = row.channel.clone();
    if add(row) {
        emit(&channel);
    }
}

/// The chatter a `channel.moderate` event settles a request for, if any:
/// an approve or deny (by any moderator, here or on Twitch) or a plain unban.
pub fn settled_by(event: &Value) -> Option<(String, String)> {
    let action = event.get("action").and_then(Value::as_str)?;
    if !matches!(action, "approve_unban_request" | "deny_unban_request" | "unban") {
        return None;
    }
    let channel = key_of(&s(event, "broadcaster_user_login"));
    let user_id = event.get(action).map(|d| s(d, "user_id")).unwrap_or_default();
    if channel.is_empty() || user_id.is_empty() {
        return None;
    }
    Some((channel, user_id))
}

/// A `channel.moderate` event: an approve, deny or unban takes the chatter's
/// request off the list.
pub fn on_moderate(event: &Value) {
    let Some((channel, user_id)) = settled_by(event) else { return };
    if remove_user(&channel, &user_id, now_ms()) {
        emit(&channel);
    }
}

async fn moderator_id() -> Result<(String, String), String> {
    let token = TwitchService::get_token().await.map_err(|_| "Sign in to Twitch first".to_string())?;
    let me = TwitchService::get_user_info().await.map_err(|_| "Sign in to Twitch first".to_string())?;
    Ok((token, me.id))
}

/// Read every pending request from Helix. `Ok((rows, complete))`; an error
/// is the HTTP status (0 for a network failure).
async fn fetch_pending(broadcaster_id: &str) -> Result<(Vec<UnbanRequest>, bool), u16> {
    // No token yet is a passing state (offline, refreshing), not a refusal.
    let (token, me) = moderator_id().await.map_err(|_| 0u16)?;
    let client = crate::services::http::client().clone();
    let mut rows = Vec::new();
    let mut after: Option<String> = None;
    for _ in 0..PAGE_CAP {
        let mut query: Vec<(&str, String)> = vec![
            ("broadcaster_id", broadcaster_id.to_string()),
            ("moderator_id", me.clone()),
            ("status", "pending".to_string()),
            ("first", "100".to_string()),
        ];
        if let Some(cursor) = &after {
            query.push(("after", cursor.clone()));
        }
        let resp = client
            .get(HELIX_URL)
            .query(&query)
            .header("Client-Id", env!("TWITCH_APP_CLIENT_ID"))
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
            .map_err(|_| 0u16)?;
        let status = resp.status().as_u16();
        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            log::debug!("[UnbanRequests] read for {} refused ({}): {}", broadcaster_id, status, body);
            return Err(status);
        }
        let json: Value = resp.json().await.map_err(|_| 0u16)?;
        let (page, cursor) = parse_page(&json, now_ms());
        rows.extend(page);
        match cursor {
            Some(c) => after = Some(c),
            None => return Ok((rows, true)),
        }
    }
    Ok((rows, false))
}

/// The channel's pending unban requests, oldest first. Reads Helix when the
/// list is older than a few minutes (or never read), merged with what
/// EventSub delivered; otherwise answers from memory with fresh ages.
#[tauri::command]
pub async fn get_unban_requests(channel_id: String, channel: String) -> Vec<UnbanRequest> {
    let key = key_of(&channel);
    if key.is_empty() {
        return Vec::new();
    }
    let started = now_ms();
    let stale = state().get(&key).map_or(true, |st| started - st.loaded_at_ms >= REFRESH_MS);
    if stale && !channel_id.trim().is_empty() {
        // Mark it read up front so panes opening together share one read.
        state().entry(key.clone()).or_default().loaded_at_ms = started;
        match fetch_pending(channel_id.trim()).await {
            Ok((mut helix, complete)) => {
                for r in helix.iter_mut() {
                    if r.channel.is_empty() {
                        r.channel = key.clone();
                    }
                }
                let changed = {
                    let mut map = state();
                    let st = map.entry(key.clone()).or_default();
                    let merged = merge(helix, &st.rows, complete, started, &st.gone);
                    let changed = merged != st.rows;
                    st.rows = merged;
                    changed
                };
                if changed {
                    emit(&key);
                }
            }
            // The channel turned requests off: nothing is waiting.
            Err(400) => {
                let had = state().get_mut(&key).map(|st| !std::mem::take(&mut st.rows).is_empty()).unwrap_or(false);
                if had {
                    emit(&key);
                }
            }
            // Not a moderator here (or the sign-in lacks the scope): asking
            // again every minute would only be refused again.
            Err(401) | Err(403) => {}
            // Offline or Twitch hiccuped: let the next pane try again.
            Err(_) => {
                if let Some(st) = state().get_mut(&key) {
                    st.loaded_at_ms = 0;
                }
            }
        }
    }
    snapshot(&key, now_ms())
}

/// Approve or deny one request, with an optional reply the chatter sees.
#[tauri::command]
pub async fn resolve_unban_request(
    channel_id: String,
    request_id: String,
    approve: bool,
    text: Option<String>,
) -> Result<(), String> {
    let (token, me) = moderator_id().await?;
    let mut query: Vec<(&str, String)> = vec![
        ("broadcaster_id", channel_id.trim().to_string()),
        ("moderator_id", me),
        ("unban_request_id", request_id.clone()),
        ("status", if approve { "approved" } else { "denied" }.to_string()),
    ];
    if let Some(reply) = reply_text(text) {
        query.push(("resolution_text", reply));
    }
    let resp = crate::services::http::client()
        .clone()
        .patch(HELIX_URL)
        .query(&query)
        .header("Client-Id", env!("TWITCH_APP_CLIENT_ID"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .map_err(|_| refusal_words(&Refusal::Other).to_string())?;
    let status = resp.status().as_u16();
    if resp.status().is_success() {
        if let Some(channel) = remove_id(&request_id, now_ms()) {
            emit(&channel);
        }
        return Ok(());
    }
    let body = resp.text().await.unwrap_or_default();
    log::warn!("[UnbanRequests] resolve {} refused ({}): {}", request_id, status, body);
    let why = refusal(status);
    match why {
        Refusal::AlreadyHandled => {
            if let Some(channel) = remove_id(&request_id, now_ms()) {
                emit(&channel);
            }
        }
        Refusal::NotReceiving => {
            let channel = state()
                .iter()
                .find(|(_, st)| st.rows.iter().any(|r| r.id == request_id))
                .map(|(k, _)| k.clone());
            if let Some(channel) = channel {
                if let Some(st) = state().get_mut(&channel) {
                    st.rows.clear();
                }
                emit(&channel);
            }
        }
        _ => {}
    }
    Err(refusal_words(&why).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NOW: i64 = 1_760_000_000_000;

    fn iso(ms: i64) -> String {
        chrono::DateTime::from_timestamp_millis(ms).unwrap().to_rfc3339()
    }

    fn row(id: &str, user: &str, at: i64) -> UnbanRequest {
        UnbanRequest {
            id: id.into(),
            channel: "c".into(),
            broadcaster_id: "1".into(),
            user_id: user.into(),
            user_login: user.into(),
            user_name: user.into(),
            text: "sorry".into(),
            created_at_ms: at,
            age: String::new(),
        }
    }

    #[test]
    fn a_create_event_reads_as_a_row() {
        let ev = json!({
            "id": "r1", "broadcaster_user_id": "1", "broadcaster_user_login": "ZackRawrr",
            "broadcaster_user_name": "ZackRawrr", "user_id": "42", "user_login": "gus", "user_name": "Gus",
            "text": "  I was joking  ", "created_at": iso(NOW - 240_000)
        });
        let r = from_create_event(&ev, NOW).unwrap();
        assert_eq!(r.channel, "zackrawrr");
        assert_eq!(r.broadcaster_id, "1");
        assert_eq!(r.user_name, "Gus");
        assert_eq!(r.text, "I was joking");
        assert_eq!(r.created_at_ms, NOW - 240_000);
        assert_eq!(crate::services::mod_history::ago(r.created_at_ms, NOW), "4 minutes ago");
        assert!(from_create_event(&json!({ "user_id": "42" }), NOW).is_none(), "no id, no row");
    }

    #[test]
    fn a_helix_page_keeps_pending_rows_and_its_cursor() {
        let page = json!({
            "data": [
                { "id": "a", "broadcaster_id": "1", "broadcaster_login": "chan", "user_id": "2",
                  "user_login": "bob", "user_name": "Bob", "text": "pls", "status": "pending",
                  "created_at": iso(NOW - 60_000), "resolved_at": null, "resolution_text": null },
                { "id": "b", "broadcaster_id": "1", "broadcaster_login": "chan", "user_id": "3",
                  "user_login": "amy", "user_name": "Amy", "text": "hi", "status": "denied",
                  "created_at": iso(NOW - 90_000) }
            ],
            "pagination": { "cursor": "abc" }
        });
        let (rows, cursor) = parse_page(&page, NOW);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "a");
        assert_eq!(rows[0].channel, "chan");
        assert_eq!(rows[0].broadcaster_id, "1");
        assert_eq!(cursor.as_deref(), Some("abc"));

        let (rows, cursor) = parse_page(&json!({ "data": [], "pagination": {} }), NOW);
        assert!(rows.is_empty());
        assert!(cursor.is_none());
    }

    #[test]
    fn a_complete_read_is_the_truth_plus_what_arrived_since() {
        let memory = vec![row("old", "1", NOW - 600_000), row("live", "2", NOW + 1_000)];
        let helix = vec![row("h", "3", NOW - 300_000)];
        let gone = HashMap::new();
        let merged = merge(helix.clone(), &memory, true, NOW, &gone);
        let ids: Vec<&str> = merged.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["h", "live"], "a stale row Helix no longer lists leaves");

        let partial = merge(helix, &memory, false, NOW, &gone);
        assert_eq!(partial.len(), 3, "a capped read only adds");

        let mut gone = HashMap::new();
        gone.insert("3".to_string(), NOW - 1_000);
        let merged = merge(vec![row("h", "3", NOW - 300_000)], &[], true, NOW, &gone);
        assert!(merged.is_empty(), "a chatter resolved during the read stays out");
    }

    #[test]
    fn approve_or_deny_by_anyone_removes_the_request() {
        let channel = "settle-test";
        let mut r = row("r9", "77", NOW);
        r.channel = channel.into();
        assert!(add(r.clone()));
        assert!(!add(r), "the same request twice is one row");
        let ev = json!({
            "broadcaster_user_login": "Settle-Test", "moderator_user_name": "Kay",
            "action": "approve_unban_request",
            "approve_unban_request": { "is_approved": true, "user_id": "77", "user_login": "gus",
                                       "user_name": "Gus", "moderator_message": "ok" }
        });
        assert_eq!(settled_by(&ev), Some((channel.to_string(), "77".to_string())));
        on_moderate(&ev);
        assert!(snapshot(channel, NOW).is_empty());

        let deny = json!({ "broadcaster_user_login": "c", "action": "deny_unban_request",
                           "deny_unban_request": { "user_id": "5" } });
        assert_eq!(settled_by(&deny), Some(("c".to_string(), "5".to_string())));
        let ban = json!({ "broadcaster_user_login": "c", "action": "ban", "ban": { "user_id": "5" } });
        assert!(settled_by(&ban).is_none());
        state().remove(channel);
    }

    #[test]
    fn resolving_by_id_drops_the_row_and_its_chatter() {
        let channel = "byid-test";
        let mut r = row("q1", "88", NOW);
        r.channel = channel.into();
        add(r);
        assert_eq!(remove_id("q1", NOW).as_deref(), Some(channel));
        assert!(remove_id("q1", NOW).is_none());
        assert!(state().get(channel).unwrap().gone.contains_key("88"));
        state().remove(channel);
    }

    #[test]
    fn the_reply_is_trimmed_to_twitchs_limit() {
        assert_eq!(reply_text(None), None);
        assert_eq!(reply_text(Some("   ".into())), None);
        assert_eq!(reply_text(Some("  welcome back ".into())).as_deref(), Some("welcome back"));
        let long: String = "é".repeat(600);
        assert_eq!(reply_text(Some(long)).unwrap().chars().count(), 500);
    }

    #[test]
    fn refusals_in_words() {
        assert_eq!(refusal(400), Refusal::NotReceiving);
        assert_eq!(refusal_words(&refusal(400)), "This channel doesn't take unban requests");
        assert_eq!(refusal(404), Refusal::AlreadyHandled);
        assert_eq!(refusal_words(&refusal(404)), "Someone already handled this request");
        assert_eq!(refusal(403), Refusal::NotAllowed);
        assert_eq!(refusal(500), Refusal::Other);
    }
}
