//! Whisper conversations as a domain: what happens when a whisper arrives, is
//! read, archived, listed or paged. Storage is `whisper_store`; Twitch reads
//! and the sync are `whisper_sync`; sending is `whisper_send`; who is looking
//! at which conversation is `whisper_claims`.
//!
//! Receiving is owned here, not by a window: the whisper socket files every
//! incoming whisper itself, so history keeps growing while the main window is
//! closed to the tray, and a window only ever gets the one conversation that
//! changed (`whisper-conversation-updated`).
//!
//! Conversations are keyed by the other person's numeric Twitch id. An old
//! import could only name a login, so a conversation may sit under its login
//! until the id is seen; the first sighting moves it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use chrono::TimeZone;
use serde::Serialize;
use tauri::{Emitter, Manager};

use crate::rt::AppHandle;
use crate::services::inbox_counts::Surface; // (reconciled: Surface lives in inbox_counts)
use crate::services::mention_inbox::MentionPart;
use crate::services::twitch_service::TwitchService;
use crate::services::whisper_service::WhisperEvent;
use crate::services::whisper_store::{
    self, AccountIndex, Conversation, ConversationMeta, Deep, Marker, Origin, Scope, StoredWhisper, TwitchBadge,
};
use crate::services::whisper_sync::{self, Reason};

pub const UPDATED_EVENT: &str = "whisper-conversation-updated";
/// Many conversations changed at once (a sync), or a sync started or ended.
pub const ARCHIVE_CHANGED_EVENT: &str = "whisper-archive-changed";
/// An incoming whisper worth a notification now (the payload is its
/// `WhisperUpdate`). Decided here for every platform.
pub const NOTIFY_EVENT: &str = "whisper-notify";
/// An incoming whisper for the phones' arrival card. Independent of the
/// notification setting, like `mention-arrived`.
pub const ARRIVED_EVENT: &str = "whisper-arrived";

/// Notifications on, and the Whispers alert type on.
static NOTIFY: AtomicBool = AtomicBool::new(false);

/// Settings changed: whether an incoming whisper may notify.
pub fn refresh_settings(settings: &crate::models::settings::Settings) {
    let n = &settings.live_notifications;
    NOTIFY.store(n.enabled && n.show_whisper_notifications, Ordering::Relaxed);
}

/// Whether a whisper just filed is announced: notifications and the Whispers
/// type on, streamer mode off, Twitch has not muted the conversation, and it
/// did not land in a conversation someone is reading.
fn announces(notify: bool, streamer_mode: bool, muted: bool, reading: bool, incoming: bool) -> bool {
    notify && !streamer_mode && !muted && !reading && incoming
}

/// Whether a whisper just filed shows the phones' arrival card: the same as a
/// mention's card, so the notification setting does not decide it.
fn arrives(streamer_mode: bool, muted: bool, reading: bool, incoming: bool) -> bool {
    !streamer_mode && !muted && !reading && incoming
}

/// Who whispered last since the app started, for `/r`: (login, display
/// name, user id).
static LAST_FROM: Mutex<Option<(String, String, String)>> = Mutex::new(None);

/// The sender of the newest whisper received, for `/r`.
pub fn last_from() -> Option<(String, String, String)> {
    LAST_FROM.lock().ok().and_then(|l| l.clone())
}

/// One conversation's change, as a window applies it: move `replaced_key` to
/// `key` if set, take `meta`, add or replace `message` by id, and drop
/// `removed_message_id`.
#[derive(Serialize, Clone, Debug)]
pub struct WhisperUpdate {
    pub owner_id: String,
    pub key: String,
    pub replaced_key: Option<String>,
    pub meta: RowMeta,
    pub message: Option<StoredWhisper>,
    pub removed_message_id: Option<String>,
    pub loading: Option<bool>,
    pub error: Option<String>,
}

/// What every surface draws for a conversation (list row, inbox row,
/// notification).
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct RowMeta {
    pub key: String,
    pub user_id: String,
    pub user_login: String,
    pub user_name: String,
    pub profile_image_url: Option<String>,
    pub color: Option<String>,
    pub badges: Vec<TwitchBadge>,
    pub last_message_timestamp: i64,
    pub last_text: String,
    pub last_parts: Vec<MentionPart>,
    pub last_is_sent: bool,
    pub unread_count: i32,
    pub archived: bool,
    pub muted: bool,
    /// (reconciled, CA-13.1) The sender as the iPhone's pinned-sender look draws
    /// them (`{name_color, badges, role_badge, paint}`, the shape
    /// `chat_row_view::stamp_pins` writes). Built at read time from cosmetics
    /// already known; desktop and Android draw names through `useChatterLook`
    /// and ignore it.
    pub sender_view: Option<serde_json::Value>,
    /// Whether they are live right now, from what StreamNook already knows
    /// (`whisper_live`). None while offline or not known.
    pub live: Option<crate::services::whisper_live::LiveNow>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct WhisperList {
    pub owner_id: String,
    pub conversations: Vec<RowMeta>,
    pub unread_total: i64,
    pub syncing: bool,
    pub signed_in: bool,
    pub session_problem: Option<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct WhisperPage {
    pub meta: Option<RowMeta>,
    /// Oldest first.
    pub messages: Vec<StoredWhisper>,
    /// What to pass back for the page before this one; None at the start.
    pub cursor: Option<String>,
    pub has_more_local: bool,
    pub older_on_twitch: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct ArchiveChanged {
    pub owner_id: String,
    pub syncing: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct WhisperArrived {
    pub update: WhisperUpdate,
}

fn app() -> Option<AppHandle> {
    crate::services::providers::app_handle()
}

fn current() -> Option<Scope> {
    whisper_sync::current_scope()
}

/// Now on Twitch's clock, as well as it is known.
fn now_twitch_ms() -> i64 {
    chrono::Utc::now().timestamp_millis() + whisper_sync::twitch_offset_ms()
}

/// A fresh index entry for a conversation that does not exist yet.
pub fn new_meta(key: &str, user_id: &str, login: &str, name: &str, avatar: Option<String>) -> ConversationMeta {
    // (reconciled) built on the store's own empty meta, so no field is missed.
    ConversationMeta {
        user_id: user_id.to_string(),
        user_login: login.to_string(),
        user_name: if name.is_empty() { login.to_string() } else { name.to_string() },
        profile_image_url: avatar,
        ..ConversationMeta::new(key)
    }
}

/// Add `m` in (timestamp, id) order. False when a message with its id is
/// already there.
pub(crate) fn insert_sorted(messages: &mut Vec<StoredWhisper>, m: StoredWhisper) -> bool {
    if messages.iter().any(|x| x.id == m.id) {
        return false;
    }
    let pos = messages.partition_point(|x| (x.timestamp, x.id.as_str()) <= (m.timestamp, m.id.as_str()));
    messages.insert(pos, m);
    true
}

/// The newest message from the other person that still shows.
fn newest_incoming(messages: &[StoredWhisper]) -> Option<&StoredWhisper> {
    messages.iter().rev().find(|m| !m.is_sent && !m.deleted)
}

/// Read the conversation through its newest incoming message. Owes Twitch
/// that id when it is Twitch's own. Returns whether anything changed.
fn read_through_now(c: &mut Conversation) -> bool {
    let Some(newest) = newest_incoming(&c.messages).cloned() else {
        return false;
    };
    if c.meta.read_through.as_ref().is_some_and(|r| r.id == newest.id) {
        return false;
    }
    c.meta.read_through = Some(Marker { id: newest.id.clone(), at_ms: newest.timestamp });
    c.meta.read_seq = whisper_store::next_seq();
    if newest.origin == Origin::Twitch {
        c.meta.twitch_read_owed = Some(newest.id);
    }
    true
}

/// The conversation with this person, under their id or, for an old import,
/// their login.
pub fn key_for(scope: &Scope, user_id: &str, login: &str) -> Option<String> {
    whisper_store::read_index(scope, |idx| key_in(idx, user_id, login)).ok().flatten()
}

fn key_in(idx: &AccountIndex, user_id: &str, login: &str) -> Option<String> {
    if !user_id.is_empty() {
        if let Some(c) = idx.conversations.iter().find(|c| c.key == user_id) {
            return Some(c.key.clone());
        }
    }
    let login = login.to_lowercase();
    if login.is_empty() {
        return None;
    }
    idx.conversations
        .iter()
        .find(|c| c.key == login || c.user_login.to_lowercase() == login)
        .map(|c| c.key.clone())
}

fn meta_of(scope: &Scope, key: &str) -> Option<ConversationMeta> {
    whisper_store::read_index(scope, |idx| idx.conversations.iter().find(|c| c.key == key).cloned())
        .ok()
        .flatten()
}

/// Send one conversation's change to every window, and wake the counts.
pub fn emit_update(
    scope: &Scope,
    key: &str,
    message: Option<StoredWhisper>,
    replaced_key: Option<String>,
) -> Option<WhisperUpdate> {
    let meta = meta_of(scope, key)?;
    let update = WhisperUpdate {
        owner_id: scope.owner.clone(),
        key: key.to_string(),
        replaced_key,
        meta: row_meta(&meta),
        message: message.map(with_globals_msg),
        removed_message_id: None,
        loading: None,
        error: None,
    };
    if let Some(app) = app() {
        let _ = app.emit(UPDATED_EVENT, &update);
    }
    crate::services::inbox_counts::wake();
    Some(update)
}

/// Tell windows a conversation changed with no single new message (a sync
/// merge, a read marker, a load of older history), so an open one re-reads.
/// (reconciled: the name whisper_sync calls.)
pub fn emit_conversation(scope: &Scope, key: &str) {
    let _ = emit_update(scope, key, None, None);
}

/// A message left a conversation (a discarded send).
pub fn emit_removed(scope: &Scope, key: &str, message_id: &str) {
    let Some(meta) = meta_of(scope, key) else { return };
    let update = WhisperUpdate {
        owner_id: scope.owner.clone(),
        key: key.to_string(),
        replaced_key: None,
        meta: row_meta(&meta),
        message: None,
        removed_message_id: Some(message_id.to_string()),
        loading: None,
        error: None,
    };
    if let Some(app) = app() {
        let _ = app.emit(UPDATED_EVENT, &update);
    }
    crate::services::inbox_counts::wake();
}

/// Many conversations may have changed, or a sync started or ended.
pub fn emit_archive_changed(owner: &str, syncing: bool) {
    if let Some(app) = app() {
        let _ = app.emit(ARCHIVE_CHANGED_EVENT, &ArchiveChanged { owner_id: owner.to_string(), syncing });
    }
    crate::services::inbox_counts::wake();
}

/// The conversation someone is reading: read it through what just merged in.
pub fn read_through_if_reading(scope: &Scope, key: &str) {
    if crate::services::whisper_claims::reading(&scope.owner, key) {
        let _ = mark_read_in(scope, key);
    }
}

/// A claim on this conversation started counting (focus came back, the sheet
/// opened, the app came back): what is on screen is read.
pub fn on_claim_counting(owner: &str, key: &str) {
    let Some(scope) = current() else { return };
    if scope.owner == owner {
        let _ = mark_read_in(&scope, key);
    }
}

/// File a whisper the socket just received and tell the windows. `twitch_ms`
/// is EventSub's own timestamp, so every time in a conversation is Twitch's.
pub async fn record_incoming(app: AppHandle, owner_id: String, ev: WhisperEvent, twitch_ms: i64) {
    let _ = app;
    if !ev.from_user_login.is_empty() {
        if let Ok(mut last) = LAST_FROM.lock() {
            let name = if ev.from_user_name.is_empty() { ev.from_user_login.clone() } else { ev.from_user_name.clone() };
            *last = Some((ev.from_user_login.clone(), name, ev.from_user_id.clone()));
        }
    }
    let Some(scope) = current() else {
        log::debug!("[WhisperInbox] whisper arrived with no account followed; not filed");
        return;
    };
    if scope.owner != owner_id {
        log::debug!("[WhisperInbox] whisper for an account no longer followed; not filed");
        return;
    }
    whisper_sync::note_twitch_time(twitch_ms); // (reconciled: one argument, 7.4's form)

    // An old login-keyed conversation moves under the id now that it is known.
    let found = key_for(&scope, &ev.from_user_id, &ev.from_user_login);
    let mut replaced = None;
    if let Some(k) = found.as_deref() {
        if k != ev.from_user_id && !ev.from_user_id.is_empty() {
            if whisper_store::rekey(&scope, k, &ev.from_user_id).is_ok() {
                replaced = Some(k.to_string());
            }
        }
    }
    let key = if ev.from_user_id.is_empty() { found.clone().unwrap_or_default() } else { ev.from_user_id.clone() };
    if key.is_empty() {
        return;
    }
    let needs_avatar = meta_of(&scope, &key).map(|m| m.profile_image_url.is_none()).unwrap_or(true);
    let avatar = if needs_avatar && !ev.from_user_id.is_empty() {
        TwitchService::get_user_by_id(&ev.from_user_id).await.ok().and_then(|u| u.profile_image_url)
    } else {
        None
    };

    let message = StoredWhisper {
        id: ev.whisper_id.clone(),
        from_user_id: ev.from_user_id.clone(),
        from_user_login: ev.from_user_login.clone(),
        from_user_name: ev.from_user_name.clone(),
        to_user_id: ev.to_user_id.clone(),
        to_user_login: ev.to_user_login.clone(),
        to_user_name: ev.to_user_name.clone(),
        message: ev.text.clone(),
        timestamp: twitch_ms,
        is_sent: false,
        parts: Vec::new(),
        deleted: false,
        edited_ms: None,
        origin: Origin::Twitch,
        delivery: None,
        sent_local_ms: None,
        got_204: false,
        checks: 0,
        failure: None,
        cut_to: None,
        migrated: false,
    };
    let reading = crate::services::whisper_claims::reading(&scope.owner, &key);
    let create = new_meta(&key, &ev.from_user_id, &ev.from_user_login, &ev.from_user_name, avatar.clone());
    let filed = whisper_store::update(&scope, &key, Some(create), |c| {
        if !insert_sorted(&mut c.messages, message.clone()) {
            return (None, false);
        }
        if c.meta.profile_image_url.is_none() {
            c.meta.profile_image_url = avatar.clone();
        }
        if c.meta.archived {
            // Twitch brings an archived conversation back on a new message;
            // so does StreamNook, and an archive still owed is dropped.
            c.meta.archived = false;
            c.meta.archive_pending = false;
            c.meta.archive_requested_ms = None;
            c.meta.archived_seq = whisper_store::next_seq();
        }
        if reading {
            read_through_now(c);
        }
        (Some(c.meta.muted), true)
    });
    let muted = match filed {
        Ok(Some(Some(muted))) => muted,
        Ok(_) => return,
        Err(e) => {
            log::warn!("[WhisperInbox] could not file a whisper: {e}");
            return;
        }
    };
    if reading {
        whisper_sync::send_owed(key.clone());
    }
    let Some(update) = emit_update(&scope, &key, Some(message), replaced) else { return };
    let notify = NOTIFY.load(Ordering::Relaxed);
    let streamer = crate::services::streamer_mode::StreamerMode::is_active();
    if let Some(app) = crate::services::providers::app_handle() {
        if announces(notify, streamer, muted, reading, true) {
            log::info!("[WhisperInbox] announcing an incoming whisper");
            let _ = app.emit(NOTIFY_EVENT, &update);
        } else {
            log::debug!(
                "[WhisperInbox] whisper filed without a notification (notify {notify}, streamer mode {streamer}, muted {muted}, reading {reading})"
            );
        }
        if arrives(streamer, muted, reading, true) {
            let _ = app.emit(ARRIVED_EVENT, &WhisperArrived { update: update.clone() });
        }
    }
    // Twitch's copy carries the emote ranges EventSub leaves out.
    whisper_sync::trigger(Reason::AfterTraffic(key));
}

fn mark_read_in(scope: &Scope, key: &str) -> Result<(), String> {
    let changed = whisper_store::update(scope, key, None, |c| {
        let changed = read_through_now(c);
        (changed, changed)
    })?;
    if changed == Some(true) {
        whisper_sync::send_owed(key.to_string());
        emit_update(scope, key, None, None);
    }
    Ok(())
}

/// Read a conversation through its newest message, here and on Twitch.
pub fn mark_read(key: &str) -> Result<(), String> {
    let Some(scope) = current() else { return Ok(()) };
    mark_read_in(&scope, key)
}

/// The conversations Mark all read reads: every one with something unread,
/// muted and archived ones included (reading them changes no badge and keeps
/// Twitch in step). (reconciled: factored out so it is tested)
fn unread_keys(idx: &AccountIndex) -> Vec<String> {
    idx.conversations.iter().filter(|c| c.unread_count > 0).map(|c| c.key.clone()).collect()
}

/// Read every conversation with something unread.
pub fn mark_all_read() -> Result<(), String> {
    let Some(scope) = current() else { return Ok(()) };
    let keys: Vec<String> = whisper_store::read_index(&scope, unread_keys)?;
    for key in keys {
        mark_read_in(&scope, &key)?;
    }
    Ok(())
}

/// Archive a conversation here at once and on Twitch as soon as it can.
pub async fn archive(key: &str) -> Result<(), String> {
    let Some(scope) = current() else { return Ok(()) };
    let changed = whisper_store::update(&scope, key, None, |c| {
        if c.meta.archived && !c.meta.archive_pending {
            return ((), false);
        }
        c.meta.archived = true;
        c.meta.archive_pending = true;
        c.meta.archive_requested_ms = Some(now_twitch_ms());
        c.meta.archived_seq = whisper_store::next_seq();
        ((), true)
    })?;
    if changed.is_some() {
        whisper_sync::send_owed(key.to_string());
        emit_update(&scope, key, None, None);
    }
    Ok(())
}

/// Leave every whisper row up to now out of the island and MultiChat.
pub fn island_clear() -> Result<(), String> {
    let Some(scope) = current() else { return Ok(()) };
    whisper_store::update_index(&scope, |idx| {
        idx.island_cleared_through = Some(now_twitch_ms());
        ((), true)
    })?;
    emit_archive_changed(&scope.owner, whisper_sync::syncing());
    Ok(())
}

pub fn row_meta(meta: &ConversationMeta) -> RowMeta {
    RowMeta {
        key: meta.key.clone(),
        user_id: meta.user_id.clone(),
        user_login: meta.user_login.clone(),
        user_name: meta.user_name.clone(),
        profile_image_url: meta.profile_image_url.clone(),
        color: meta.color.clone(),
        badges: meta.badges.clone(),
        last_message_timestamp: meta.last_message_timestamp,
        last_text: meta.last_text.clone(),
        last_parts: with_globals(&meta.last_parts, &meta.last_text),
        last_is_sent: meta.last_is_sent,
        unread_count: meta.unread_count,
        archived: meta.archived,
        muted: meta.muted,
        // (reconciled, CA-13.1) Known cosmetics only: a list never starts lookups.
        sender_view: Some(crate::services::chat_row_view::whisper_sender_view(
            &meta.user_id,
            meta.color.as_deref(),
            &meta.badges,
        )),
        // What is already known; a read never asks Twitch.
        live: crate::services::whisper_live::cached(&meta.user_login),
    }
}

/// Rows a surface shows, newest first (ties by key), after `after` when given.
/// Archived conversations are left out; the island and MultiChat also leave
/// out rows at or before their Clear.
fn rows_of(idx: &AccountIndex, surface: Surface, after: Option<(i64, String)>, limit: usize) -> Vec<ConversationMeta> {
    let cleared = match surface {
        Surface::Island | Surface::Multichat => idx.island_cleared_through,
        Surface::Phone => None,
    };
    let mut rows: Vec<&ConversationMeta> = idx
        .conversations
        .iter()
        .filter(|c| !c.archived)
        .filter(|c| c.last_message_timestamp > 0)
        .filter(|c| cleared.is_none_or(|t| c.last_message_timestamp > t))
        .collect();
    rows.sort_by(|a, b| b.last_message_timestamp.cmp(&a.last_message_timestamp).then_with(|| a.key.cmp(&b.key)));
    rows.into_iter()
        .filter(|c| match &after {
            Some((ts, key)) => c.last_message_timestamp < *ts || (c.last_message_timestamp == *ts && c.key > *key),
            None => true,
        })
        .take(limit)
        .cloned()
        .collect()
}

/// (every unread, unread the island shows): archived and Twitch-muted
/// conversations count toward neither.
fn unread_of(idx: &AccountIndex) -> (u32, u32) {
    let mut all = 0u32;
    let mut island = 0u32;
    for c in idx.conversations.iter().filter(|c| !c.archived && !c.muted) {
        let n = c.unread_count.max(0) as u32;
        all += n;
        if idx.island_cleared_through.is_none_or(|t| c.last_message_timestamp > t) {
            island += n;
        }
    }
    (all, island)
}

/// The inbox view's whisper rows (see `rows_of`).
pub fn rows_for_view(surface: Surface, after: Option<(i64, String)>, limit: usize) -> Vec<RowMeta> {
    let Some(scope) = current() else { return Vec::new() };
    refresh_globals_soon();
    whisper_store::read_index(&scope, |idx| rows_of(idx, surface, after, limit))
        .map(|rows| rows.iter().map(row_meta).collect())
        .unwrap_or_default()
}

/// (all, island) unread for the counts emitter.
pub fn unread_counts() -> (u32, u32) {
    let Some(scope) = current() else { return (0, 0) };
    whisper_store::read_index(&scope, unread_of).unwrap_or((0, 0))
}

/// The signed-in account's conversations, newest first.
pub fn list() -> Result<WhisperList, String> {
    let Some(scope) = current() else {
        return Ok(WhisperList {
            owner_id: String::new(),
            conversations: Vec::new(),
            unread_total: 0,
            syncing: false,
            signed_in: false,
            session_problem: None,
        });
    };
    refresh_globals_soon();
    let (rows, (unread, _)) =
        whisper_store::read_index(&scope, |idx| (rows_of(idx, Surface::Phone, None, usize::MAX), unread_of(idx)))?;
    Ok(WhisperList {
        owner_id: scope.owner.clone(),
        conversations: rows.iter().map(row_meta).collect(),
        unread_total: i64::from(unread),
        syncing: whisper_sync::syncing(),
        signed_in: true,
        session_problem: whisper_sync::session_problem(),
    })
}

fn encode_cursor(ts: i64, id: &str) -> String {
    format!("{ts}:{id}")
}

fn decode_cursor(raw: &str) -> Option<(i64, String)> {
    let (ts, id) = raw.split_once(':')?;
    Some((ts.parse().ok()?, id.to_string()))
}

fn local_day(ms: i64) -> Option<chrono::NaiveDate> {
    chrono::Local.timestamp_millis_opt(ms).single().map(|d| d.date_naive())
}

/// The messages a conversation shows. While older history is still on
/// Twitch, old scraped copies dated on or after the oldest Twitch message's
/// day are hidden (they stay on disk until the history is complete).
fn visible_messages(c: &Conversation) -> Vec<&StoredWhisper> {
    let hide_from = if matches!(c.meta.deep, Deep::Partial { .. }) {
        c.messages.iter().filter(|m| m.origin == Origin::Twitch).filter_map(|m| local_day(m.timestamp)).min()
    } else {
        None
    };
    c.messages
        .iter()
        .filter(|m| match (hide_from, m.origin) {
            (Some(day), Origin::Scraped) => local_day(m.timestamp).is_none_or(|d| d < day),
            _ => true,
        })
        .collect()
}

fn page_of(c: &Conversation, cursor: Option<&str>, limit: usize) -> WhisperPage {
    let visible = visible_messages(c);
    let end = match cursor.and_then(decode_cursor) {
        Some((ts, id)) => visible.partition_point(|m| (m.timestamp, m.id.as_str()) < (ts, id.as_str())),
        None => visible.len(),
    };
    let start = end.saturating_sub(limit);
    let slice = &visible[start..end];
    WhisperPage {
        meta: Some(row_meta(&c.meta)),
        messages: slice.iter().map(|m| with_globals_msg((*m).clone())).collect(),
        cursor: slice.first().map(|m| encode_cursor(m.timestamp, &m.id)),
        has_more_local: start > 0,
        older_on_twitch: matches!(c.meta.deep, Deep::Partial { .. }),
    }
}

fn empty_page() -> WhisperPage {
    WhisperPage { meta: None, messages: Vec::new(), cursor: None, has_more_local: false, older_on_twitch: false }
}

/// One page of a conversation, oldest first, before `cursor` when given.
pub fn page(key: &str, cursor: Option<String>, limit: usize) -> Result<WhisperPage, String> {
    let Some(scope) = current() else { return Ok(empty_page()) };
    refresh_globals_soon();
    let out = whisper_store::read(&scope, key, |c| page_of(c, cursor.as_deref(), limit.clamp(1, 200)))?;
    // (reconciled, CA-13.1) An open conversation looks its sender's 7TV
    // cosmetics up once, so the next read draws their paint and badge.
    if let Some(user_id) = out.as_ref().and_then(|p| p.meta.as_ref()).map(|m| m.user_id.clone()) {
        crate::services::chat_row_view::warm_whisper_sender(&user_id);
    }
    // An open conversation asks whether its person is live, at most
    // once every two minutes; a changed answer lands as an update.
    #[cfg(not(test))]
    if let Some(m) = out.as_ref().and_then(|p| p.meta.as_ref()) {
        crate::services::whisper_live::refresh_soon(scope.clone(), key.to_string(), m.user_login.clone(), m.user_id.clone());
    }
    Ok(out.unwrap_or_else(empty_page))
}

/// Why Twitch did not deliver a whisper, in the words every platform shows
/// under the message: Twitch's own reason where it gives one. `status` is None
/// when the request never got an answer.
pub fn send_failure_reason(status: Option<u16>, body: &str) -> String {
    let message = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("message").and_then(|m| m.as_str()).map(str::trim).map(str::to_string))
        .filter(|m| !m.is_empty());
    let sentence = |m: &str| {
        let mut s = m.to_string();
        if let Some(first) = s.get(..1) {
            s = first.to_uppercase() + &s[1..];
        }
        if !s.ends_with(['.', '!', '?']) {
            s.push('.');
        }
        s
    };
    let lower = message.as_deref().unwrap_or(body).to_ascii_lowercase();
    if lower.contains("settings prevent") {
        return "Cannot send: This user's privacy settings prevent them from receiving whispers from you.".into();
    }
    if lower.contains("verified phone") {
        return "Cannot send: Twitch needs a verified phone number on your account to send whispers.".into();
    }
    match status {
        None => "Could not reach Twitch. Check your connection and try again.".into(),
        Some(401) => "Cannot send: Your session has expired. Please sign in again.".into(),
        Some(429) => "Sending too fast. Please wait a moment before trying again.".into(),
        Some(404) => "Cannot send: User not found.".into(),
        Some(_) => match message {
            Some(m) => format!("Cannot send: {}", sentence(&m)),
            None => "Failed to send message. Please try again.".into(),
        },
    }
}

// ---- Third-party global emotes, added when a message is read ---------------

static GLOBALS: RwLock<Option<Arc<crate::services::irc_service::EmoteLookup>>> = RwLock::new(None);
static GLOBALS_ASKED: Mutex<Option<Instant>> = Mutex::new(None);
const GLOBALS_RETRY: Duration = Duration::from_secs(60);

/// Load the 7TV, BTTV and FFZ global emotes once they are cached (at most
/// once a minute). The first table that lands asks the views to read again.
fn refresh_globals_soon() {
    {
        let mut asked = GLOBALS_ASKED.lock().unwrap_or_else(|e| e.into_inner());
        if asked.is_some_and(|t| t.elapsed() < GLOBALS_RETRY) {
            return;
        }
        *asked = Some(Instant::now());
    }
    tauri::async_runtime::spawn(async {
        let Some(app) = app() else { return };
        let Some(state) = app.try_state::<crate::models::settings::AppState>() else { return };
        let service = state.emote_service.clone();
        let lookup = {
            let svc = service.read().await;
            svc.with_cached_set("global", |set| crate::services::irc_service::EmoteLookup::build(set)).await
        };
        let Some(lookup) = lookup else { return };
        if lookup.len() == 0 {
            return;
        }
        let first = {
            let mut g = GLOBALS.write().unwrap_or_else(|e| e.into_inner());
            let first = g.is_none();
            *g = Some(lookup);
            first
        };
        if first {
            if let Some(scope) = current() {
                emit_archive_changed(&scope.owner, whisper_sync::syncing());
            }
        }
    });
}

/// `parts` (or `text` when there are none) with words naming a global emote
/// turned into emotes. Empty when nothing became an emote and there were no
/// parts, so a shell keeps drawing plain `text`.
fn split_globals(parts: &[MentionPart], text: &str, get: impl Fn(&str) -> Option<String>) -> Vec<MentionPart> {
    let source: Vec<MentionPart> =
        if parts.is_empty() { vec![MentionPart::Text { text: text.to_string() }] } else { parts.to_vec() };
    let mut out: Vec<MentionPart> = Vec::new();
    let mut any = false;
    let mut buffer = String::new();
    for part in source {
        match part {
            MentionPart::Text { text } => {
                for (i, word) in text.split(' ').enumerate() {
                    if i > 0 {
                        buffer.push(' ');
                    }
                    match (!word.is_empty()).then(|| get(word)).flatten() {
                        Some(url) => {
                            if !buffer.is_empty() {
                                out.push(MentionPart::Text { text: std::mem::take(&mut buffer) });
                            }
                            out.push(MentionPart::Emote { name: word.to_string(), url });
                            any = true;
                        }
                        None => buffer.push_str(word),
                    }
                }
            }
            emote => {
                if !buffer.is_empty() {
                    out.push(MentionPart::Text { text: std::mem::take(&mut buffer) });
                }
                out.push(emote);
            }
        }
    }
    if !buffer.is_empty() {
        out.push(MentionPart::Text { text: buffer });
    }
    if !any && parts.is_empty() {
        return Vec::new();
    }
    out
}

fn with_globals(parts: &[MentionPart], text: &str) -> Vec<MentionPart> {
    let lookup = GLOBALS.read().unwrap_or_else(|e| e.into_inner()).clone();
    match lookup {
        Some(l) => split_globals(parts, text, |w| l.get(w).map(|e| e.url.clone())),
        None => parts.to_vec(),
    }
}

fn with_globals_msg(mut m: StoredWhisper) -> StoredWhisper {
    if !m.deleted {
        m.parts = with_globals(&m.parts, &m.message);
    }
    m
}

// ---- The old commands, until each one's last caller is gone ---------------

/// What the not-yet-retired commands answer, in their old shapes, so the old
/// store can go first. Each one leaves with its command.
pub mod legacy {
    use super::*;

    fn conversation_json(c: &Conversation) -> serde_json::Value {
        serde_json::json!({
            "user_id": c.meta.user_id,
            "user_login": c.meta.user_login,
            "user_name": c.meta.user_name,
            "profile_image_url": c.meta.profile_image_url,
            "messages": c.messages,
            "last_message_timestamp": c.meta.last_message_timestamp,
            "unread_count": c.meta.unread_count,
        })
    }

    /// `load_whisper_storage`: every conversation with its messages.
    pub fn load_storage() -> Result<serde_json::Value, String> {
        let Some(scope) = current() else {
            return Ok(serde_json::json!({ "conversations": {}, "version": 1 }));
        };
        let keys: Vec<String> =
            whisper_store::read_index(&scope, |idx| idx.conversations.iter().map(|c| c.key.clone()).collect())?;
        let mut out = serde_json::Map::new();
        for key in keys {
            if let Some(v) = whisper_store::read(&scope, &key, conversation_json)? {
                out.insert(key, v);
            }
        }
        Ok(serde_json::json!({ "conversations": out, "version": 1 }))
    }

    /// `whisper_thread`: one conversation, oldest first.
    pub fn thread(key: &str) -> Result<Option<serde_json::Value>, String> {
        let Some(scope) = current() else { return Ok(None) };
        whisper_store::read(&scope, key, conversation_json)
    }

    /// `whisper_refresh`: a sync, answering how many messages it added.
    pub async fn refresh() -> Result<usize, String> {
        Ok(match whisper_sync::sync_now().await {
            whisper_sync::SyncOutcome::Done { added } | whisper_sync::SyncOutcome::Partial { added } => added as usize,
            whisper_sync::SyncOutcome::Failed { reason } => return Err(reason),
            _ => 0,
        })
    }

    /// `whisper_import`: files are not imported any more.
    pub fn import() -> Result<serde_json::Value, String> {
        Err("Whispers now load from Twitch on their own.".into())
    }

    /// `get_whisper_storage_path`.
    pub fn storage_path() -> Result<String, String> {
        whisper_store::root_dir().map(|p| p.to_string_lossy().into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(id: &str, ts: i64, is_sent: bool, origin: Origin) -> StoredWhisper {
        StoredWhisper {
            id: id.into(),
            from_user_id: if is_sent { "99999999".into() } else { "1000000000".into() },
            from_user_login: String::new(),
            from_user_name: String::new(),
            to_user_id: String::new(),
            to_user_login: String::new(),
            to_user_name: String::new(),
            message: format!("text {id}"),
            timestamp: ts,
            is_sent,
            parts: Vec::new(),
            deleted: false,
            edited_ms: None,
            origin,
            delivery: None,
            sent_local_ms: None,
            got_204: false,
            checks: 0,
            failure: None,
            cut_to: None,
            migrated: false,
        }
    }

    fn conv(messages: Vec<StoredWhisper>) -> Conversation {
        Conversation { meta: new_meta("1000000000", "1000000000", "pebble", "Pebble", None), messages }
    }

    fn meta(key: &str, ts: i64, unread: i32) -> ConversationMeta {
        let mut m = new_meta(key, key, "someone", "Someone", None);
        m.last_message_timestamp = ts;
        m.unread_count = unread;
        m
    }

    #[test]
    fn a_whisper_notifies_only_when_nothing_keeps_it_quiet() {
        assert!(announces(true, false, false, false, true));
        assert!(!announces(false, false, false, false, true));
        assert!(!announces(true, true, false, false, true));
        assert!(!announces(true, false, false, true, true));
        assert!(!announces(true, false, false, false, false));
    }

    #[test]
    fn muted_never_announces() {
        assert!(!announces(true, false, true, false, true));
        assert!(!arrives(false, true, false, true));
    }

    #[test]
    fn whisper_arrival_ignores_notify_setting() {
        // The notification switch is not an input to the arrival card at all.
        assert!(arrives(false, false, false, true));
        assert!(!arrives(true, false, false, true));
        assert!(!arrives(false, false, true, true));
    }

    #[test]
    fn open_conversation_reads_through_on_merge() {
        let mut c = conv(vec![msg("a", 10, false, Origin::Twitch), msg("b", 20, true, Origin::LocalSent), msg("c", 30, false, Origin::Twitch)]);
        assert!(read_through_now(&mut c));
        assert_eq!(c.meta.read_through, Some(Marker { id: "c".into(), at_ms: 30 }));
        assert_eq!(c.meta.twitch_read_owed.as_deref(), Some("c"));
        // Reading again changes nothing.
        assert!(!read_through_now(&mut c));
    }

    #[test]
    fn a_scraped_newest_message_is_read_but_never_owed_to_twitch() {
        let mut c = conv(vec![msg("s", 10, false, Origin::Scraped)]);
        assert!(read_through_now(&mut c));
        assert_eq!(c.meta.twitch_read_owed, None);
    }

    #[test]
    fn insert_keeps_order_and_ignores_a_known_id() {
        let mut v = vec![msg("a", 10, false, Origin::Twitch), msg("c", 30, false, Origin::Twitch)];
        assert!(insert_sorted(&mut v, msg("b", 20, false, Origin::Twitch)));
        assert!(!insert_sorted(&mut v, msg("b", 25, false, Origin::Twitch)));
        let ids: Vec<_> = v.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["a", "b", "c"]);
    }

    /// (reconciled, A34 "mark all read") Mark all read reads every conversation
    /// with something unread, muted and archived ones too, and skips read ones.
    #[test]
    fn mark_all_read_picks_every_unread_conversation() {
        let mut muted = meta("1000000002", 20, 1);
        muted.muted = true;
        let mut archived = meta("1000000003", 30, 4);
        archived.archived = true;
        let idx = AccountIndex {
            conversations: vec![meta("1000000001", 10, 2), muted, archived, meta("1000000004", 40, 0)],
            ..Default::default()
        };
        assert_eq!(unread_keys(&idx), ["1000000001", "1000000002", "1000000003"]);
    }

    #[test]
    fn page_keeps_same_ms_messages() {
        let c = conv((0..5).map(|i| msg(&format!("m{i}"), 100, false, Origin::Twitch)).collect());
        let mut seen = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let p = page_of(&c, cursor.as_deref(), 2);
            for m in p.messages.iter().rev() {
                seen.push(m.id.clone());
            }
            if !p.has_more_local {
                break;
            }
            cursor = p.cursor;
        }
        seen.reverse();
        assert_eq!(seen, ["m0", "m1", "m2", "m3", "m4"]);
    }

    #[test]
    fn partial_hides_scraped_from_oldest_twitch_day() {
        let day = |d: u32, h: u32| chrono::Local.with_ymd_and_hms(2025, 3, d, h, 0, 0).earliest().unwrap().timestamp_millis();
        let mut c = conv(vec![
            msg("old", day(1, 0), false, Origin::Scraped),
            msg("same", day(5, 0), false, Origin::Scraped),
            msg("tw", day(5, 15), false, Origin::Twitch),
        ]);
        c.meta.deep = Deep::Partial { cursor: None };
        let ids: Vec<_> = page_of(&c, None, 50).messages.into_iter().map(|m| m.id).collect();
        assert_eq!(ids, ["old", "tw"]);
        c.meta.deep = Deep::Complete;
        assert_eq!(page_of(&c, None, 50).messages.len(), 3);
    }

    #[test]
    fn rows_leave_out_archived_and_cleared() {
        let mut idx = AccountIndex::default();
        idx.conversations = vec![meta("1000000001", 300, 1), meta("1000000002", 200, 0), meta("1000000003", 100, 2)];
        idx.conversations[1].archived = true;
        let keys = |rows: Vec<ConversationMeta>| rows.into_iter().map(|r| r.key).collect::<Vec<_>>();
        assert_eq!(keys(rows_of(&idx, Surface::Phone, None, 10)), ["1000000001", "1000000003"]);
        idx.island_cleared_through = Some(150);
        assert_eq!(keys(rows_of(&idx, Surface::Island, None, 10)), ["1000000001"]);
        assert_eq!(keys(rows_of(&idx, Surface::Multichat, None, 10)), ["1000000001"]);
        assert_eq!(keys(rows_of(&idx, Surface::Phone, None, 10)), ["1000000001", "1000000003"]);
        assert_eq!(keys(rows_of(&idx, Surface::Phone, Some((300, "1000000001".into())), 10)), ["1000000003"]);
    }

    #[test]
    fn rows_with_one_time_page_by_key() {
        let mut idx = AccountIndex::default();
        idx.conversations = vec![meta("1000000002", 100, 0), meta("1000000001", 100, 0), meta("1000000003", 100, 0)];
        let first = rows_of(&idx, Surface::Phone, None, 2);
        assert_eq!(first.iter().map(|r| r.key.as_str()).collect::<Vec<_>>(), ["1000000001", "1000000002"]);
        let last = first.last().unwrap();
        let rest = rows_of(&idx, Surface::Phone, Some((last.last_message_timestamp, last.key.clone())), 2);
        assert_eq!(rest.iter().map(|r| r.key.as_str()).collect::<Vec<_>>(), ["1000000003"]);
    }

    #[test]
    fn muted_and_archived_count_nowhere() {
        let mut idx = AccountIndex::default();
        idx.conversations = vec![meta("1000000001", 300, 2), meta("1000000002", 200, 3), meta("1000000003", 100, 4)];
        idx.conversations[1].muted = true;
        idx.conversations[2].archived = true;
        assert_eq!(unread_of(&idx), (2, 2));
        idx.island_cleared_through = Some(400);
        assert_eq!(unread_of(&idx), (2, 0));
    }

    #[test]
    fn globals_turn_known_words_into_emotes() {
        let get = |w: &str| (w == "EZ").then(|| "u".to_string());
        let parts = split_globals(&[], "that was EZ ok", get);
        assert_eq!(
            parts,
            vec![
                MentionPart::Text { text: "that was ".into() },
                MentionPart::Emote { name: "EZ".into(), url: "u".into() },
                MentionPart::Text { text: " ok".into() },
            ]
        );
        // Twitch emote parts stay where they are; only text words change.
        let twitch = vec![MentionPart::Emote { name: "Kappa".into(), url: "k".into() }, MentionPart::Text { text: " EZ".into() }];
        let parts = split_globals(&twitch, "Kappa EZ", get);
        assert_eq!(parts[0], MentionPart::Emote { name: "Kappa".into(), url: "k".into() });
        assert_eq!(parts[2], MentionPart::Emote { name: "EZ".into(), url: "u".into() });
    }

    #[test]
    fn plain_text_without_globals_stays_plain() {
        assert!(split_globals(&[], "nothing here", |_| None).is_empty());
    }

    #[test]
    fn a_failed_send_says_why_in_twitchs_words() {
        let body = |m: &str| format!(r#"{{"error":"Bad Request","status":400,"message":"{m}"}}"#);
        assert_eq!(
            send_failure_reason(Some(403), &body("the sender does not have a verified phone number")),
            "Cannot send: Twitch needs a verified phone number on your account to send whispers."
        );
        assert_eq!(
            send_failure_reason(Some(400), &body("The recipient's settings prevent this sender from whispering them")),
            "Cannot send: This user's privacy settings prevent them from receiving whispers from you."
        );
        assert_eq!(send_failure_reason(Some(429), &body("slow")), "Sending too fast. Please wait a moment before trying again.");
        assert_eq!(send_failure_reason(Some(404), ""), "Cannot send: User not found.");
        assert_eq!(send_failure_reason(Some(401), ""), "Cannot send: Your session has expired. Please sign in again.");
        assert_eq!(send_failure_reason(Some(500), "<html>"), "Failed to send message. Please try again.");
        assert_eq!(send_failure_reason(None, ""), "Could not reach Twitch. Check your connection and try again.");
    }

    #[test]
    fn cursors_round_trip_ids_with_colons() {
        assert_eq!(decode_cursor(&encode_cursor(42, "a:b")), Some((42, "a:b".to_string())));
        assert_eq!(decode_cursor("x"), None);
    }
}
