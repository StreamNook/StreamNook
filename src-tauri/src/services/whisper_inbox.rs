//! Whisper conversations as a domain: what happens when a whisper arrives, is
//! sent, read, imported or refreshed. Persistence is whisper_storage_service;
//! this module decides what to write.
//!
//! Receiving is owned here, not by a window: the whisper socket records every
//! incoming whisper itself, so history keeps accruing while the main window is
//! closed to the tray, and a window only ever gets the one conversation that
//! changed (`whisper-conversation-updated`) instead of shipping the whole
//! archive back to disk on every change.
//!
//! Conversations are keyed by the other user's numeric Twitch id. Imports can
//! only name a login, so a conversation may sit under its login until the id is
//! known; any later sighting of the id moves it (re-keying).

use crate::services::twitch_service::TwitchService;
use crate::services::whisper_history_service::{WhisperHistoryService, WhisperMessage};
use crate::services::whisper_service::WhisperEvent;
use crate::services::whisper_storage_service::{
    StoredConversation, StoredWhisper, WhisperStorageService,
};
use chrono::TimeZone;
use futures::stream::{self, StreamExt};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter};

pub const UPDATED_EVENT: &str = "whisper-conversation-updated";
/// Many conversations changed at once (an import); reload the archive.
pub const ARCHIVE_CHANGED_EVENT: &str = "whisper-archive-changed";

/// Parallel Helix lookups while resolving an import's logins to ids.
const RESOLVE_CONCURRENCY: usize = 4;

/// The conversation open in the Whispers panel, if any. A whisper landing in
/// it is read as it arrives, so it never counts as unread.
static ACTIVE: Mutex<Option<String>> = Mutex::new(None);

type Conversations = HashMap<String, StoredConversation>;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ConversationMeta {
    pub user_id: String,
    pub user_login: String,
    pub user_name: String,
    pub profile_image_url: Option<String>,
    pub last_message_timestamp: i64,
    pub unread_count: i32,
}

/// One conversation's change, as a window applies it: move `replaced_key` to
/// `key` if set, take `meta`, and append `message` unless it is already there.
#[derive(Debug, Clone, Serialize)]
pub struct WhisperUpdate {
    pub owner_id: String,
    pub key: String,
    pub replaced_key: Option<String>,
    pub meta: ConversationMeta,
    pub message: Option<StoredWhisper>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ImportSummary {
    pub conversations: usize,
    pub messages: usize,
}

/// Who "me" is when an import has to fill in the sender of a message.
#[derive(Debug, Clone, Default)]
pub struct Me {
    pub id: String,
    pub login: String,
    pub display_name: String,
}

fn meta_of(c: &StoredConversation) -> ConversationMeta {
    ConversationMeta {
        user_id: c.user_id.clone(),
        user_login: c.user_login.clone(),
        user_name: c.user_name.clone(),
        profile_image_url: c.profile_image_url.clone(),
        last_message_timestamp: c.last_message_timestamp,
        unread_count: c.unread_count,
    }
}

fn is_numeric_id(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Treat an empty string like a missing value, as the exporter's `a || b` did.
fn or_else(value: Option<&str>, fallback: &str) -> String {
    match value {
        Some(v) if !v.is_empty() => v.to_string(),
        _ => fallback.to_string(),
    }
}

/// Where a conversation with this user lives: under their id, or, for one
/// imported before the id was known, under their login.
fn find_key(convs: &Conversations, user_id: &str, login: &str) -> Option<String> {
    if convs.contains_key(user_id) {
        return Some(user_id.to_string());
    }
    let login = login.to_lowercase();
    if convs.contains_key(&login) {
        return Some(login);
    }
    convs
        .iter()
        .find(|(_, c)| c.user_login.to_lowercase() == login)
        .map(|(k, _)| k.clone())
}

/// Move the conversation at `from` to `to`, merging into one already there.
fn rekey(convs: &mut Conversations, from: &str, to: &str) {
    if from == to {
        return;
    }
    let Some(mut moving) = convs.remove(from) else { return };
    moving.user_id = to.to_string();
    match convs.get_mut(to) {
        Some(existing) => {
            if existing.profile_image_url.is_none() {
                existing.profile_image_url = moving.profile_image_url.take();
            }
            existing.unread_count += moving.unread_count;
            merge_messages(existing, moving.messages);
        }
        None => {
            convs.insert(to.to_string(), moving);
        }
    }
}

/// Add messages not already present (by id), keep time order, and move the
/// conversation's last-message time to its newest message. Returns how many
/// were added.
fn merge_messages(conv: &mut StoredConversation, incoming: Vec<StoredWhisper>) -> usize {
    let mut known: HashSet<String> = conv.messages.iter().map(|m| m.id.clone()).collect();
    let before = conv.messages.len();
    for m in incoming {
        if known.insert(m.id.clone()) {
            conv.messages.push(m);
        }
    }
    let added = conv.messages.len() - before;
    if added > 0 {
        conv.messages.sort_by_key(|m| m.timestamp);
        if let Some(last) = conv.messages.last() {
            conv.last_message_timestamp = last.timestamp;
        }
    }
    added
}

/// Whether recording a whisper from this user needs their avatar looked up:
/// only for a conversation that does not exist yet, or one found under a login
/// that has no picture. Everything else already has what it needs.
fn needs_avatar(convs: &Conversations, ev: &WhisperEvent) -> bool {
    match find_key(convs, &ev.from_user_id, &ev.from_user_login) {
        None => true,
        Some(key) => key != ev.from_user_id && convs[&key].profile_image_url.is_none(),
    }
}

/// Record one incoming whisper. Returns the change, or None when that whisper
/// was already stored.
fn apply_incoming(
    convs: &mut Conversations,
    ev: &WhisperEvent,
    received_ms: i64,
    avatar: Option<String>,
    active: Option<&str>,
) -> Option<(String, Option<String>, StoredWhisper)> {
    let message = StoredWhisper {
        id: ev.whisper_id.clone(),
        from_user_id: ev.from_user_id.clone(),
        from_user_login: ev.from_user_login.clone(),
        from_user_name: ev.from_user_name.clone(),
        to_user_id: ev.to_user_id.clone(),
        to_user_login: ev.to_user_login.clone(),
        to_user_name: ev.to_user_name.clone(),
        message: ev.text.clone(),
        timestamp: received_ms,
        is_sent: false,
    };
    let from_id = ev.from_user_id.as_str();

    let Some(found) = find_key(convs, from_id, &ev.from_user_login) else {
        convs.insert(
            from_id.to_string(),
            StoredConversation {
                user_id: from_id.to_string(),
                user_login: ev.from_user_login.clone(),
                user_name: ev.from_user_name.clone(),
                profile_image_url: avatar,
                messages: vec![message.clone()],
                last_message_timestamp: received_ms,
                unread_count: if active == Some(from_id) { 0 } else { 1 },
            },
        );
        return Some((from_id.to_string(), None, message));
    };

    let conv = convs.get_mut(&found).expect("found above");
    if conv.messages.iter().any(|m| m.id == message.id) {
        return None;
    }
    let is_open = active == Some(found.as_str()) || active == Some(from_id);
    if !is_open {
        conv.unread_count += 1;
    }
    merge_messages(conv, vec![message.clone()]);

    let replaced = if found != from_id {
        if conv.profile_image_url.is_none() {
            conv.profile_image_url = avatar;
        }
        rekey(convs, &found, from_id);
        Some(found)
    } else {
        None
    };
    Some((from_id.to_string(), replaced, message))
}

fn update_for(
    convs: &Conversations,
    owner_id: &str,
    key: String,
    replaced_key: Option<String>,
    message: Option<StoredWhisper>,
) -> Option<WhisperUpdate> {
    let meta = meta_of(convs.get(&key)?);
    Some(WhisperUpdate { owner_id: owner_id.to_string(), key, replaced_key, meta, message })
}

/// Persist a whisper the socket just received and tell the windows. Runs off
/// the socket's read loop, so a slow avatar lookup never delays keepalives.
pub async fn record_incoming(app: AppHandle, owner_id: String, ev: WhisperEvent, received_ms: i64) {
    let lookup = WhisperStorageService::update_conversations(&app, &owner_id, |convs| {
        (needs_avatar(convs, &ev), false)
    });
    let avatar = match lookup {
        Ok(true) => TwitchService::get_user_by_id(&ev.from_user_id)
            .await
            .ok()
            .and_then(|u| u.profile_image_url),
        Ok(false) => None,
        Err(e) => {
            log::warn!("[WhisperInbox] could not open whisper storage: {e}");
            return;
        }
    };
    let active = ACTIVE.lock().ok().and_then(|a| a.clone());
    let result = WhisperStorageService::update_conversations(&app, &owner_id, |convs| {
        match apply_incoming(convs, &ev, received_ms, avatar, active.as_deref()) {
            Some((key, replaced, message)) => {
                (update_for(convs, &owner_id, key, replaced, Some(message)), true)
            }
            None => (None, false),
        }
    });
    match result {
        Ok(Some(update)) => {
            let _ = app.emit(UPDATED_EVENT, &update);
        }
        Ok(None) => {}
        Err(e) => log::warn!("[WhisperInbox] failed to record whisper: {e}"),
    }
}

/// Which conversation the Whispers panel has open (None when closed).
pub fn set_active(key: Option<String>) {
    if let Ok(mut active) = ACTIVE.lock() {
        *active = key;
    }
}

pub fn mark_read(app: &AppHandle, owner_id: &str, key: &str) -> Result<(), String> {
    WhisperStorageService::update_conversations(app, owner_id, |convs| match convs.get_mut(key) {
        Some(c) if c.unread_count > 0 => {
            c.unread_count = 0;
            ((), true)
        }
        _ => ((), false),
    })
}

/// The panel's view of the conversation being written to. A brand-new one
/// exists only in the panel until its first message is sent.
#[derive(Debug, Clone, Deserialize)]
pub struct SendTarget {
    pub key: String,
    pub user_id: String,
    pub user_login: String,
    pub user_name: String,
    pub profile_image_url: Option<String>,
}

/// Send a whisper and record it. The recipient's id is looked up by login when
/// the conversation only knows their login (an import that never resolved).
pub async fn send(
    app: &AppHandle,
    owner_id: &str,
    target: SendTarget,
    text: String,
) -> Result<WhisperUpdate, String> {
    let stored = WhisperStorageService::update_conversations(app, owner_id, |convs| {
        (convs.get(&target.key).map(|c| c.user_id.clone()), false)
    })?;
    let mut recipient = stored.unwrap_or_else(|| target.user_id.clone());
    let mut resolved_avatar = None;
    if !is_numeric_id(&recipient) {
        let user = TwitchService::get_user_by_login(&target.user_login)
            .await
            .map_err(|_| "Cannot send: User not found on Twitch.".to_string())?;
        recipient = user.id;
        resolved_avatar = user.profile_image_url;
    }

    TwitchService::send_whisper(&recipient, &text)
        .await
        .map_err(|e| e.to_string())?;

    let me = TwitchService::get_user_info().await.ok();
    let sent_ms = now_ms();
    let message = StoredWhisper {
        id: format!("sent-{}-{:08x}", sent_ms, rand::random::<u32>()),
        from_user_id: owner_id.to_string(),
        from_user_login: me.as_ref().map(|u| u.login.clone()).unwrap_or_default(),
        from_user_name: me.as_ref().map(|u| u.display_name.clone()).unwrap_or_default(),
        to_user_id: recipient.clone(),
        to_user_login: target.user_login.clone(),
        to_user_name: target.user_name.clone(),
        message: text,
        timestamp: sent_ms,
        is_sent: true,
    };

    let owner = owner_id.to_string();
    WhisperStorageService::update_conversations(app, owner_id, move |convs| {
        let conv = convs.entry(target.key.clone()).or_insert_with(|| StoredConversation {
            user_id: recipient.clone(),
            user_login: target.user_login.clone(),
            user_name: target.user_name.clone(),
            profile_image_url: target.profile_image_url.clone(),
            messages: Vec::new(),
            last_message_timestamp: sent_ms,
            unread_count: 0,
        });
        conv.user_id = recipient;
        if resolved_avatar.is_some() {
            conv.profile_image_url = resolved_avatar;
        }
        merge_messages(conv, vec![message.clone()]);
        (update_for(convs, &owner, target.key, None, Some(message)), true)
    })?
    .ok_or_else(|| "Conversation vanished while sending".to_string())
}

// ---- Imports --------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportedUser {
    id: Option<String>,
    login: String,
    #[serde(default)]
    display_name: String,
    #[serde(rename = "profileImageURL")]
    profile_image_url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportedMessage {
    id: String,
    from_user_id: Option<String>,
    from_user_login: Option<String>,
    from_user_name: Option<String>,
    #[serde(default)]
    content: String,
    #[serde(default)]
    sent_at: String,
    is_sent: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportedConversation {
    user: ExportedUser,
    #[serde(default)]
    messages: Vec<ExportedMessage>,
    last_message_at: Option<String>,
}

/// A whisper export (the scraper's output, or a file saved from it).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WhisperExport {
    pub version: Option<i64>,
    pub my_user_id: Option<String>,
    pub my_username: Option<String>,
    pub conversations: Vec<serde_json::Value>,
}

/// Merge an export into the archive. Returns the keys it touched and a summary.
fn merge_export(
    convs: &mut Conversations,
    export: &WhisperExport,
    me: &Me,
    now: i64,
) -> (Vec<String>, ImportSummary) {
    let mut touched = Vec::new();
    let mut summary = ImportSummary { conversations: 0, messages: 0 };
    for raw in &export.conversations {
        let Ok(conv) = serde_json::from_value::<ExportedConversation>(raw.clone()) else {
            continue;
        };
        let other = &conv.user;
        let key = or_else(other.id.as_deref(), &other.login.to_lowercase());
        let display = if other.display_name.is_empty() { &other.login } else { &other.display_name };
        let messages: Vec<StoredWhisper> = conv
            .messages
            .iter()
            .map(|m| {
                let named_me = m
                    .from_user_name
                    .as_deref()
                    .is_some_and(|n| !me.login.is_empty() && n.eq_ignore_ascii_case(&me.login));
                let is_sent = m.is_sent == Some(true) || named_me;
                let (me_or_them_id, me_or_them_login, me_or_them_name) = if is_sent {
                    (me.id.as_str(), me.login.as_str(), me.display_name.as_str())
                } else {
                    (key.as_str(), other.login.as_str(), display.as_str())
                };
                StoredWhisper {
                    id: m.id.clone(),
                    from_user_id: or_else(m.from_user_id.as_deref(), me_or_them_id),
                    from_user_login: or_else(m.from_user_login.as_deref(), me_or_them_login),
                    from_user_name: or_else(m.from_user_name.as_deref(), me_or_them_name),
                    to_user_id: if is_sent { key.clone() } else { me.id.clone() },
                    to_user_login: if is_sent { other.login.clone() } else { me.login.clone() },
                    to_user_name: if is_sent { display.to_string() } else { me.display_name.clone() },
                    message: m.content.clone(),
                    timestamp: parse_whisper_date(&m.sent_at, now),
                    is_sent,
                }
            })
            .collect();
        summary.conversations += 1;
        summary.messages += messages.len();

        match convs.get_mut(&key) {
            Some(existing) => {
                if existing.profile_image_url.is_none() {
                    existing.profile_image_url = other.profile_image_url.clone().filter(|u| !u.is_empty());
                }
                merge_messages(existing, messages);
            }
            None => {
                let mut fresh = StoredConversation {
                    user_id: key.clone(),
                    user_login: other.login.clone(),
                    user_name: display.to_string(),
                    profile_image_url: other.profile_image_url.clone().filter(|u| !u.is_empty()),
                    messages: Vec::new(),
                    last_message_timestamp: conv
                        .last_message_at
                        .as_deref()
                        .map(|s| parse_whisper_date(s, now))
                        .unwrap_or(now),
                    unread_count: 0,
                };
                let last = fresh.last_message_timestamp;
                merge_messages(&mut fresh, messages);
                // An export's own "last message" time wins over the newest parsed
                // message, as before; parsing a scraped date is best effort.
                if conv.last_message_at.is_some() {
                    fresh.last_message_timestamp = last;
                }
                convs.insert(key.clone(), fresh);
            }
        }
        touched.push(key);
    }
    (touched, summary)
}

/// Import an export, then resolve the logins it could only name into ids (and
/// fetch missing pictures), so every imported conversation can be written to.
pub async fn import_export(
    app: &AppHandle,
    owner_id: &str,
    export: WhisperExport,
) -> Result<ImportSummary, String> {
    let signed_in = TwitchService::get_user_info().await.ok();
    let me = Me {
        id: signed_in.as_ref().map(|u| u.id.clone()).or(export.my_user_id.clone()).unwrap_or_default(),
        login: signed_in
            .as_ref()
            .map(|u| u.login.clone())
            .or(export.my_username.clone())
            .unwrap_or_default(),
        display_name: signed_in
            .as_ref()
            .map(|u| u.display_name.clone())
            .or(export.my_username.clone())
            .unwrap_or_default(),
    };
    let now = now_ms();
    let (to_resolve, summary) = WhisperStorageService::update_conversations(app, owner_id, |convs| {
        let (touched, summary) = merge_export(convs, &export, &me, now);
        let to_resolve: Vec<(String, String)> = touched
            .into_iter()
            .filter_map(|key| {
                let c = convs.get(&key)?;
                (!is_numeric_id(&key) || c.profile_image_url.is_none())
                    .then(|| (key, c.user_login.clone()))
            })
            .collect();
        ((to_resolve, summary), true)
    })?;

    let resolved: Vec<(String, Option<(String, Option<String>)>)> = stream::iter(to_resolve)
        .map(|(key, login)| async move {
            let user = TwitchService::get_user_by_login(&login).await.ok();
            (key, user.map(|u| (u.id, u.profile_image_url)))
        })
        .buffer_unordered(RESOLVE_CONCURRENCY)
        .collect()
        .await;

    WhisperStorageService::update_conversations(app, owner_id, |convs| {
        for (key, found) in resolved {
            let Some((id, avatar)) = found else { continue };
            if let Some(c) = convs.get_mut(&key) {
                if avatar.is_some() {
                    c.profile_image_url = avatar;
                }
                c.user_id = id.clone();
            }
            if !is_numeric_id(&key) {
                rekey(convs, &key, &id);
            }
        }
        ((), true)
    })?;
    let _ = app.emit(ARCHIVE_CHANGED_EVENT, owner_id);
    Ok(summary)
}

/// Merge one thread's recent page into its conversation. Returns how many
/// messages were new.
fn merge_refresh(conv: &mut StoredConversation, me: &Me, page: Vec<WhisperMessage>, now: i64) -> usize {
    let other_id = conv.user_id.clone();
    let fresh = page
        .into_iter()
        .map(|m| {
            let is_sent = m.from_user_id == me.id;
            StoredWhisper {
                id: m.id,
                from_user_login: if is_sent { me.login.clone() } else { conv.user_login.clone() },
                from_user_name: or_else(
                    Some(m.from_user_name.as_str()),
                    if is_sent { &me.display_name } else { &conv.user_name },
                ),
                from_user_id: m.from_user_id,
                to_user_id: if is_sent { other_id.clone() } else { me.id.clone() },
                to_user_login: if is_sent { conv.user_login.clone() } else { me.login.clone() },
                to_user_name: if is_sent { conv.user_name.clone() } else { me.display_name.clone() },
                message: m.content,
                timestamp: parse_whisper_date(&m.sent_at, now),
                is_sent,
            }
        })
        .collect();
    merge_messages(conv, fresh)
}

/// Pull each conversation's most recent page from Twitch and merge in anything
/// StreamNook never saw (replies sent from the site or the phone).
pub async fn refresh(app: &AppHandle, owner_id: &str, token: &str) -> Result<usize, String> {
    let user = TwitchService::get_user_info()
        .await
        .map_err(|e| format!("Failed to get user info: {e}"))?;
    let me = Me { id: user.id, login: user.login, display_name: user.display_name };
    let ids = WhisperStorageService::update_conversations(app, owner_id, |convs| {
        (convs.keys().filter(|k| is_numeric_id(k)).cloned().collect::<Vec<_>>(), false)
    })?;

    let pages: Vec<(String, Vec<WhisperMessage>)> = stream::iter(ids)
        .map(|id| {
            let me_id = me.id.clone();
            async move {
                match WhisperHistoryService::get_whisper_messages(token, &me_id, &id, None).await {
                    Ok((messages, _cursor)) => (id, messages),
                    Err(e) => {
                        log::debug!("[WhisperInbox] refresh of thread {id} failed: {e}");
                        (id, Vec::new())
                    }
                }
            }
        })
        .buffer_unordered(RESOLVE_CONCURRENCY)
        .collect()
        .await;

    let now = now_ms();
    let added = WhisperStorageService::update_conversations(app, owner_id, |convs| {
        let mut added = 0;
        for (id, page) in pages {
            if let Some(conv) = convs.get_mut(&id) {
                added += merge_refresh(conv, &me, page, now);
            }
        }
        (added, added > 0)
    })?;
    if added > 0 {
        let _ = app.emit(ARCHIVE_CHANGED_EVENT, owner_id);
    }
    Ok(added)
}

// ---- Dates ----------------------------------------------------------------

static US_LOCALE_DATE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^(\d{1,2})/(\d{1,2})/(\d{4})(?:,?\s*(\d{1,2}):(\d{2})(?::(\d{2}))?\s*(AM|PM)?)?")
        .expect("valid regex")
});

/// A whisper timestamp as epoch milliseconds. Twitch's API sends RFC 3339; the
/// web scraper reads the page's tooltip, a US-locale string such as
/// "5/31/2021, 8:30:39 PM PDT" (or just "5/31/2021"), read as local time with
/// the zone abbreviation dropped. Anything unreadable becomes `now`.
pub fn parse_whisper_date(raw: &str, now: i64) -> i64 {
    let s = raw.trim();
    if s.is_empty() {
        return now;
    }
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return dt.timestamp_millis();
    }
    if let Ok(dt) = chrono::DateTime::parse_from_rfc2822(s) {
        return dt.timestamp_millis();
    }
    if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f") {
        if let Some(dt) = chrono::Local.from_local_datetime(&naive).earliest() {
            return dt.timestamp_millis();
        }
    }
    if let Some(c) = US_LOCALE_DATE.captures(s) {
        let num = |i: usize| c.get(i).and_then(|m| m.as_str().parse::<u32>().ok());
        let (Some(month), Some(day), Some(year)) = (num(1), num(2), num(3)) else {
            return now;
        };
        let mut hour = num(4).unwrap_or(0);
        match c.get(7).map(|m| m.as_str().to_ascii_uppercase()) {
            Some(ref ap) if ap == "PM" && hour < 12 => hour += 12,
            Some(ref ap) if ap == "AM" && hour == 12 => hour = 0,
            _ => {}
        }
        if let Some(dt) = chrono::Local
            .with_ymd_and_hms(year as i32, month, day, hour, num(5).unwrap_or(0), num(6).unwrap_or(0))
            .earliest()
        {
            return dt.timestamp_millis();
        }
    }
    now
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn event(from_id: &str, login: &str, id: &str) -> WhisperEvent {
        WhisperEvent {
            from_user_id: from_id.into(),
            from_user_login: login.into(),
            from_user_name: login.to_uppercase(),
            to_user_id: "1".into(),
            to_user_login: "me".into(),
            to_user_name: "Me".into(),
            whisper_id: id.into(),
            text: "hi".into(),
        }
    }

    fn conv(user_id: &str, login: &str) -> StoredConversation {
        StoredConversation {
            user_id: user_id.into(),
            user_login: login.into(),
            user_name: login.into(),
            profile_image_url: None,
            messages: Vec::new(),
            last_message_timestamp: 0,
            unread_count: 0,
        }
    }

    fn me() -> Me {
        Me { id: "1".into(), login: "me".into(), display_name: "Me".into() }
    }

    #[test]
    fn a_first_whisper_opens_an_unread_conversation() {
        let mut convs = Conversations::new();
        let (key, replaced, _) =
            apply_incoming(&mut convs, &event("42", "bob", "w1"), 100, Some("pic".into()), None).unwrap();
        assert_eq!((key.as_str(), replaced), ("42", None));
        let c = &convs["42"];
        assert_eq!((c.unread_count, c.last_message_timestamp), (1, 100));
        assert_eq!(c.profile_image_url.as_deref(), Some("pic"));
    }

    #[test]
    fn a_whisper_into_the_open_conversation_is_already_read() {
        let mut convs = Conversations::new();
        apply_incoming(&mut convs, &event("42", "bob", "w1"), 100, None, Some("42"));
        apply_incoming(&mut convs, &event("42", "bob", "w2"), 200, None, Some("42"));
        assert_eq!(convs["42"].unread_count, 0);
        assert_eq!(convs["42"].messages.len(), 2);
    }

    #[test]
    fn a_repeated_whisper_changes_nothing() {
        let mut convs = Conversations::new();
        apply_incoming(&mut convs, &event("42", "bob", "w1"), 100, None, None);
        assert!(apply_incoming(&mut convs, &event("42", "bob", "w1"), 200, None, None).is_none());
        assert_eq!(convs["42"].unread_count, 1);
    }

    #[test]
    fn a_login_keyed_import_moves_to_the_id_on_first_whisper() {
        let mut convs = Conversations::new();
        convs.insert("bob".into(), conv("bob", "Bob"));
        assert!(needs_avatar(&convs, &event("42", "bob", "w1")));
        let (key, replaced, _) =
            apply_incoming(&mut convs, &event("42", "bob", "w1"), 100, Some("pic".into()), None).unwrap();
        assert_eq!((key.as_str(), replaced.as_deref()), ("42", Some("bob")));
        assert!(!convs.contains_key("bob"));
        assert_eq!(convs["42"].user_id, "42");
        assert_eq!(convs["42"].profile_image_url.as_deref(), Some("pic"));
    }

    #[test]
    fn a_known_conversation_needs_no_avatar_lookup() {
        let mut convs = Conversations::new();
        convs.insert("42".into(), conv("42", "bob"));
        assert!(!needs_avatar(&convs, &event("42", "bob", "w1")));
    }

    #[test]
    fn rekeying_onto_an_existing_conversation_merges_instead_of_overwriting() {
        let mut convs = Conversations::new();
        let mut by_id = conv("42", "bob");
        by_id.messages.push(StoredWhisper {
            id: "a".into(), from_user_id: "42".into(), from_user_login: "bob".into(),
            from_user_name: "Bob".into(), to_user_id: "1".into(), to_user_login: "me".into(),
            to_user_name: "Me".into(), message: "x".into(), timestamp: 10, is_sent: false,
        });
        let mut by_login = by_id.clone();
        by_login.messages[0].id = "b".into();
        by_login.messages[0].timestamp = 5;
        convs.insert("42".into(), by_id);
        convs.insert("bob".into(), by_login);
        rekey(&mut convs, "bob", "42");
        let ids: Vec<_> = convs["42"].messages.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["b", "a"]);
        assert_eq!(convs.len(), 1);
    }

    #[test]
    fn an_export_merges_in_time_order_and_fills_the_sender() {
        let mut convs = Conversations::new();
        let export = WhisperExport {
            version: Some(1),
            my_user_id: None,
            my_username: None,
            conversations: vec![json!({
                "user": { "login": "Bob", "displayName": "Bob" },
                "messages": [
                    { "id": "m2", "fromUserName": "me", "content": "yo", "sentAt": "2021-05-31T20:31:00Z" },
                    { "id": "m1", "fromUserName": "Bob", "content": "hey", "sentAt": "2021-05-31T20:30:00Z" }
                ]
            })],
        };
        let (touched, summary) = merge_export(&mut convs, &export, &me(), 0);
        assert_eq!(touched, vec!["bob"]);
        assert_eq!(summary, ImportSummary { conversations: 1, messages: 2 });
        let c = &convs["bob"];
        assert_eq!(c.messages[0].id, "m1");
        assert!(!c.messages[0].is_sent);
        assert!(c.messages[1].is_sent);
        assert_eq!(c.messages[1].from_user_id, "1");
        assert_eq!(c.messages[1].to_user_id, "bob");
        // Re-importing the same file adds nothing.
        let (_, again) = merge_export(&mut convs, &export, &me(), 0);
        assert_eq!(again.messages, 2);
        assert_eq!(convs["bob"].messages.len(), 2);
    }

    #[test]
    fn refresh_adds_only_unseen_messages() {
        let mut c = conv("42", "bob");
        let page = vec![
            WhisperMessage { id: "r1".into(), from_user_id: "1".into(), from_user_name: String::new(), content: "a".into(), sent_at: "2021-01-01T00:00:00Z".into(), cursor: None },
            WhisperMessage { id: "r2".into(), from_user_id: "42".into(), from_user_name: "Bob".into(), content: "b".into(), sent_at: "2021-01-01T00:01:00Z".into(), cursor: None },
        ];
        assert_eq!(merge_refresh(&mut c, &me(), page.clone(), 0), 2);
        assert_eq!(merge_refresh(&mut c, &me(), page, 0), 0);
        assert!(c.messages[0].is_sent);
        assert_eq!(c.messages[0].from_user_name, "Me");
        assert_eq!(c.messages[1].to_user_id, "1");
    }

    #[test]
    fn parses_every_date_shape_whispers_arrive_in() {
        assert_eq!(parse_whisper_date("2021-05-31T20:30:39Z", 7), 1622493039000);
        assert_eq!(parse_whisper_date("", 7), 7);
        assert_eq!(parse_whisper_date("garbage", 7), 7);
        let local = |y, mo, d, h, mi, s| {
            chrono::Local.with_ymd_and_hms(y, mo, d, h, mi, s).earliest().unwrap().timestamp_millis()
        };
        assert_eq!(parse_whisper_date("5/31/2021, 8:30:39 PM PDT", 7), local(2021, 5, 31, 20, 30, 39));
        assert_eq!(parse_whisper_date("5/31/2021, 12:05 AM", 7), local(2021, 5, 31, 0, 5, 0));
        assert_eq!(parse_whisper_date("9/24/2026", 7), local(2026, 9, 24, 0, 0, 0));
    }
}
