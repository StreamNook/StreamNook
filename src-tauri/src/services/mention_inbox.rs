//! The mentions inbox: every time someone named you, replied to you or used
//! one of your highlight phrases, in any chat any window had open, per
//! signed-in Twitch account.
//!
//! Every live message passes `on_verdict` once, from the Twitch IRC lane and
//! from the provider lane, right after the rule engine judged it. That one
//! call files the entry here and drives the mention sound and the system
//! notification, so Kick and YouTube get all three the way Twitch does.
//!
//! Storage follows the whisper archive: `<app_data>/mentions/<owner>.json`,
//! held in memory, flushed every 5 s when dirty and on exit. Capped at
//! `MAX_ENTRIES`, and anything older than `MAX_AGE_MS` is pruned.
//!
//! Rooms are keyed the way chat tabs are: a bare lowercased login for Twitch,
//! `provider:slug` otherwise.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager};

use crate::models::chat_layout::ChatMessage;
use crate::models::settings::Settings;
use crate::services::chat_rules::{channel_filter_key, ChatRules, Verdict};

const DIR: &str = "mentions";
pub const MAX_ENTRIES: usize = 500;
pub const MAX_AGE_MS: i64 = 30 * 24 * 60 * 60 * 1000;
/// Message text kept per entry, in characters.
const MAX_TEXT: usize = 500;
pub const CHANGED_EVENT: &str = "mention-inbox-changed";

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MentionKind {
    /// Your login or display name.
    Mention,
    /// A reply to one of your messages.
    Reply,
    /// One of your highlight phrases.
    Word,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MentionEntry {
    /// The platform's message id.
    pub id: String,
    pub platform: String,
    /// Chat tab key: bare Twitch login, `provider:slug` otherwise.
    pub room: String,
    /// The channel as chat addresses it (login or slug).
    pub channel_login: String,
    #[serde(default)]
    pub channel_id: Option<String>,
    pub sender_id: String,
    pub sender_login: String,
    pub sender_name: String,
    #[serde(default)]
    pub sender_color: Option<String>,
    pub text: String,
    pub kind: MentionKind,
    #[serde(default)]
    pub reply_parent_id: Option<String>,
    pub at_ms: i64,
    pub read: bool,
    /// A moderator deleted the message after it was filed.
    #[serde(default)]
    pub removed: bool,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
struct InboxFile {
    version: u32,
    /// Oldest first.
    entries: Vec<MentionEntry>,
}

/// One owner's inbox in memory. Pure apart from the clock it is handed, so the
/// rules are unit-tested here without files or an app.
#[derive(Default, Debug)]
pub struct Inbox {
    /// Oldest first.
    entries: Vec<MentionEntry>,
    /// Rooms on screen right now: mentions there arrive already read.
    visible: HashSet<String>,
}

impl Inbox {
    fn from_entries(mut entries: Vec<MentionEntry>, now_ms: i64) -> Self {
        entries.sort_by_key(|e| e.at_ms);
        let mut inbox = Inbox {
            entries,
            visible: HashSet::new(),
        };
        inbox.prune(now_ms);
        inbox
    }

    /// File one entry. False when it was already there (same message id).
    pub fn file(&mut self, mut entry: MentionEntry, now_ms: i64) -> bool {
        if self.entries.iter().any(|e| e.id == entry.id && e.platform == entry.platform) {
            return false;
        }
        if self.visible.contains(&entry.room) {
            entry.read = true;
        }
        let at = entry.at_ms;
        let pos = self.entries.partition_point(|e| e.at_ms <= at);
        self.entries.insert(pos, entry);
        self.prune(now_ms);
        true
    }

    fn prune(&mut self, now_ms: i64) {
        self.entries.retain(|e| now_ms - e.at_ms <= MAX_AGE_MS);
        if self.entries.len() > MAX_ENTRIES {
            let extra = self.entries.len() - MAX_ENTRIES;
            self.entries.drain(..extra);
        }
    }

    pub fn unread_by_room(&self) -> HashMap<String, u32> {
        let mut out = HashMap::new();
        for e in self.entries.iter().filter(|e| !e.read && !e.removed) {
            *out.entry(e.room.clone()).or_insert(0) += 1;
        }
        out
    }

    pub fn unread_total(&self) -> u32 {
        self.entries.iter().filter(|e| !e.read && !e.removed).count() as u32
    }

    /// Newest first, older than `before_ms` when given.
    pub fn page(&self, before_ms: Option<i64>, limit: usize) -> (Vec<MentionEntry>, bool) {
        let mut out: Vec<MentionEntry> = self
            .entries
            .iter()
            .rev()
            .filter(|e| before_ms.is_none_or(|b| e.at_ms < b))
            .take(limit + 1)
            .cloned()
            .collect();
        let more = out.len() > limit;
        out.truncate(limit);
        (out, more)
    }

    /// Mark these ids read, or every entry when `ids` is None. True when
    /// anything changed.
    pub fn mark_read(&mut self, ids: Option<&[String]>) -> bool {
        let mut changed = false;
        for e in self.entries.iter_mut().filter(|e| !e.read) {
            if ids.is_none_or(|ids| ids.iter().any(|id| *id == e.id)) {
                e.read = true;
                changed = true;
            }
        }
        changed
    }

    pub fn mark_room_read(&mut self, room: &str) -> bool {
        let mut changed = false;
        for e in self.entries.iter_mut().filter(|e| !e.read && e.room == room) {
            e.read = true;
            changed = true;
        }
        changed
    }

    pub fn set_visible(&mut self, rooms: Vec<String>) -> bool {
        self.visible = rooms.into_iter().map(|r| r.to_lowercase()).collect();
        let rooms: Vec<String> = self.visible.iter().cloned().collect();
        let mut changed = false;
        for room in rooms {
            changed |= self.mark_room_read(&room);
        }
        changed
    }

    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| e.id != id);
        self.entries.len() != before
    }

    pub fn mark_removed(&mut self, id: &str) -> bool {
        match self.entries.iter_mut().find(|e| e.id == id && !e.removed) {
            Some(e) => {
                e.removed = true;
                true
            }
            None => false,
        }
    }

    pub fn clear(&mut self) -> bool {
        let had = !self.entries.is_empty();
        self.entries.clear();
        had
    }

    pub fn get(&self, id: &str) -> Option<&MentionEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut MentionEntry> {
        self.entries.iter_mut().find(|e| e.id == id)
    }
}

/// Chat tab key for a message's channel.
pub fn room_key(provider: &str, channel: &str) -> String {
    let key = channel_filter_key(provider, channel);
    match key.strip_prefix("twitch:") {
        Some(login) => login.to_string(),
        None => key,
    }
}

/// What a live message files as, or None. `highlight_words` is the user's
/// choice to count their phrases.
pub fn kind_for(verdict: &Verdict, highlight_words: bool) -> Option<MentionKind> {
    if verdict.drop {
        None
    } else if verdict.reply_to_me {
        Some(MentionKind::Reply)
    } else if verdict.mentioned {
        Some(MentionKind::Mention)
    } else if highlight_words && verdict.phrase_hit {
        Some(MentionKind::Word)
    } else {
        None
    }
}

fn sent_at_ms(msg: &ChatMessage) -> Option<i64> {
    msg.tags.get("tmi-sent-ts").and_then(|t| t.parse::<i64>().ok())
}

pub fn entry_for(msg: &ChatMessage, kind: MentionKind, now_ms: i64) -> MentionEntry {
    let channel_login = match msg.channel.trim_start_matches('#').split_once(':') {
        Some((_, slug)) => slug.to_lowercase(),
        None => msg.channel.trim_start_matches('#').to_lowercase(),
    };
    let platform = if msg.provider.is_empty() { "twitch".to_string() } else { msg.provider.to_lowercase() };
    MentionEntry {
        id: msg.id.clone(),
        room: room_key(&msg.provider, &msg.channel),
        platform,
        channel_login,
        channel_id: msg.tags.get("room-id").cloned(),
        sender_id: msg.user_id.clone(),
        sender_login: msg.username.to_lowercase(),
        sender_name: if msg.display_name.is_empty() { msg.username.clone() } else { msg.display_name.clone() },
        sender_color: msg.color.clone(),
        text: msg.content.chars().take(MAX_TEXT).collect(),
        kind,
        reply_parent_id: msg.tags.get("reply-parent-msg-id").cloned(),
        at_ms: sent_at_ms(msg).unwrap_or(now_ms),
        read: false,
        removed: false,
    }
}

// ---------------------------------------------------------------------------
// Process-wide state for the signed-in account
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
struct Config {
    highlight_words: bool,
    muted: HashSet<String>,
}

struct State {
    owner: String,
    inbox: Inbox,
    dirty: bool,
}

static STATE: OnceLock<Mutex<Option<State>>> = OnceLock::new();
static CONFIG: OnceLock<RwLock<Config>> = OnceLock::new();
static UNREAD: OnceLock<tokio::sync::watch::Sender<Arc<HashMap<String, u32>>>> = OnceLock::new();
static FLUSH_TASK_STARTED: AtomicBool = AtomicBool::new(false);

fn state() -> &'static Mutex<Option<State>> {
    STATE.get_or_init(|| Mutex::new(None))
}

fn config() -> Config {
    CONFIG
        .get_or_init(|| RwLock::new(Config { highlight_words: true, muted: HashSet::new() }))
        .read()
        .map(|c| c.clone())
        .unwrap_or_default()
}

fn unread_tx() -> &'static tokio::sync::watch::Sender<Arc<HashMap<String, u32>>> {
    UNREAD.get_or_init(|| tokio::sync::watch::channel(Arc::new(HashMap::new())).0)
}

/// Settings changed: which phrases count and which chats stay quiet.
pub fn refresh(settings: &Settings) {
    let next = Config {
        highlight_words: settings.mentions.highlight_words,
        muted: settings.mentions.muted_channels.iter().map(|c| c.trim().to_lowercase()).collect(),
    };
    if let Ok(mut g) = CONFIG.get_or_init(|| RwLock::new(Config::default())).write() {
        *g = next;
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn file_path(owner: &str) -> Option<PathBuf> {
    let app = crate::services::providers::app_handle()?;
    let dir = app.path().app_data_dir().ok()?.join(DIR);
    if !dir.exists() {
        std::fs::create_dir_all(&dir).ok()?;
    }
    let safe: String = owner.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-').collect();
    (!safe.is_empty()).then(|| dir.join(format!("{safe}.json")))
}

fn load(owner: &str) -> Inbox {
    let entries = file_path(owner)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<InboxFile>(&s).ok())
        .map(|f| f.entries)
        .unwrap_or_default();
    Inbox::from_entries(entries, now_ms())
}

fn write(owner: &str, inbox: &Inbox) -> Result<(), String> {
    let path = file_path(owner).ok_or("no mentions folder")?;
    let body = serde_json::to_string(&InboxFile { version: 1, entries: inbox.entries.clone() })
        .map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, body).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

/// Run `f` on the signed-in account's inbox, loading it (and flushing the
/// previous account's) when the account changed. None when nobody is signed
/// in to Twitch. `f` returns whether it changed anything.
fn with_inbox<R>(f: impl FnOnce(&mut Inbox) -> (R, bool)) -> Option<R> {
    let owner = ChatRules::own_identity().map(|(_, id)| id).filter(|id| !id.is_empty())?;
    let (out, changed, counts) = {
        let mut guard = state().lock().ok()?;
        if guard.as_ref().map(|s| s.owner != owner).unwrap_or(true) {
            if let Some(old) = guard.take() {
                if old.dirty {
                    let _ = write(&old.owner, &old.inbox);
                }
            }
            *guard = Some(State { owner: owner.clone(), inbox: load(&owner), dirty: false });
            // A new account's counts replace the old one's.
            let counts = guard.as_ref().map(|s| s.inbox.unread_by_room()).unwrap_or_default();
            unread_tx().send_replace(Arc::new(counts));
        }
        let st = guard.as_mut()?;
        let (out, changed) = f(&mut st.inbox);
        if changed {
            st.dirty = true;
        }
        let counts = changed.then(|| (st.inbox.unread_by_room(), st.inbox.unread_total()));
        (out, changed, counts)
    };
    if changed {
        ensure_flush_task();
        if let Some((by_room, total)) = counts {
            unread_tx().send_replace(Arc::new(by_room));
            if let Some(app) = crate::services::providers::app_handle() {
                let _ = app.emit(CHANGED_EVENT, serde_json::json!({ "unread_total": total }));
            }
        }
    }
    Some(out)
}

fn ensure_flush_task() {
    if FLUSH_TASK_STARTED.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
        return;
    }
    if tokio::runtime::Handle::try_current().is_err() {
        FLUSH_TASK_STARTED.store(false, Ordering::SeqCst);
        return;
    }
    tokio::spawn(async {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            let _ = tokio::task::spawn_blocking(flush_now).await;
        }
    });
}

/// Write the inbox when it changed. Cheap when nothing did.
pub fn flush_now() -> Result<(), String> {
    let snapshot = {
        let Ok(mut guard) = state().lock() else { return Ok(()) };
        match guard.as_mut() {
            Some(st) if st.dirty => {
                st.dirty = false;
                Some((st.owner.clone(), Inbox::from_entries(st.inbox.entries.clone(), now_ms())))
            }
            _ => None,
        }
    };
    match snapshot {
        Some((owner, inbox)) => write(&owner, &inbox).inspect_err(|_| {
            if let Ok(mut g) = state().lock() {
                if let Some(st) = g.as_mut() {
                    st.dirty = true;
                }
            }
        }),
        None => Ok(()),
    }
}

// ---------------------------------------------------------------------------
// The one entry point for live messages
// ---------------------------------------------------------------------------

/// Called for each live message after the rule engine judged it, from the
/// Twitch IRC lane and the provider lane.
pub fn on_verdict(msg: &ChatMessage, verdict: &Verdict) {
    if msg.id.is_empty() {
        return;
    }
    let cfg = config();
    let Some(kind) = kind_for(verdict, cfg.highlight_words) else { return };
    let entry = entry_for(msg, kind, now_ms());
    if cfg.muted.contains(&entry.room) {
        return;
    }
    let visible = with_inbox(|inbox| {
        let visible = inbox.visible.contains(&entry.room);
        let filed = inbox.file(entry.clone(), now_ms());
        ((filed, visible), filed)
    });
    // Already filed (a second pane, a replayed backlog): no second ping.
    let Some((true, visible)) = visible else { return };
    let mentioned = kind == MentionKind::Mention;
    let reply = kind == MentionKind::Reply;
    // A highlight phrase already plays its own rule's sound.
    if mentioned || reply {
        crate::services::mention_ping::on_live_message(msg, mentioned, reply);
    }
    if !visible {
        #[cfg(desktop)]
        crate::services::mention_notify::on_live_message(msg, kind);
    }
}

/// A message from a chat's history backlog (what was said before you joined):
/// filed like a live one, without the sound or the notification.
pub fn on_backfill(msg: &ChatMessage, verdict: &Verdict) {
    if msg.id.is_empty() {
        return;
    }
    let cfg = config();
    let Some(kind) = kind_for(verdict, cfg.highlight_words) else { return };
    let entry = entry_for(msg, kind, now_ms());
    if cfg.muted.contains(&entry.room) {
        return;
    }
    let _ = with_inbox(|inbox| {
        let filed = inbox.file(entry, now_ms());
        ((), filed)
    });
}

/// A moderator deleted a message: an inbox copy says so instead of quoting it.
pub fn on_message_deleted(message_id: &str) {
    let _ = with_inbox(|inbox| ((), inbox.mark_removed(message_id)));
}

// ---------------------------------------------------------------------------
// What other services and the commands call
// ---------------------------------------------------------------------------

pub fn unread_by_room() -> HashMap<String, u32> {
    with_inbox(|inbox| (inbox.unread_by_room(), false)).unwrap_or_default()
}

/// Per-room unread counts, current value included, for the chat tabs.
pub fn watch_unread() -> tokio::sync::watch::Receiver<Arc<HashMap<String, u32>>> {
    // Seed from disk when nothing touched the inbox yet this run.
    let _ = with_inbox(|_| ((), false));
    unread_tx().subscribe()
}

pub fn set_visible_rooms(rooms: Vec<String>) {
    let _ = with_inbox(|inbox| {
        let changed = inbox.set_visible(rooms);
        ((), changed)
    });
}

pub fn mark_room_read(room: &str) {
    let room = room.to_lowercase();
    let _ = with_inbox(|inbox| ((), inbox.mark_room_read(&room)));
}

pub fn page(before_ms: Option<i64>, limit: usize) -> Option<(Vec<MentionEntry>, bool, u32)> {
    with_inbox(|inbox| {
        let (entries, more) = inbox.page(before_ms, limit);
        ((entries, more, inbox.unread_total()), false)
    })
}

pub fn mark_read(ids: Option<Vec<String>>) {
    let _ = with_inbox(|inbox| ((), inbox.mark_read(ids.as_deref())));
}

pub fn remove(id: &str) {
    let _ = with_inbox(|inbox| ((), inbox.remove(id)));
}

pub fn clear() {
    let _ = with_inbox(|inbox| ((), inbox.clear()));
}

/// Opening an entry marks it read and returns it.
pub fn open(id: &str) -> Option<MentionEntry> {
    with_inbox(|inbox| match inbox.get_mut(id) {
        Some(e) => {
            let changed = !e.read;
            e.read = true;
            (Some(e.clone()), changed)
        }
        None => (None, false),
    })
    .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, room: &str, at_ms: i64) -> MentionEntry {
        MentionEntry {
            id: id.into(),
            platform: "twitch".into(),
            room: room.into(),
            channel_login: room.into(),
            channel_id: None,
            sender_id: "1".into(),
            sender_login: "someone".into(),
            sender_name: "Someone".into(),
            sender_color: None,
            text: "hi @brandon".into(),
            kind: MentionKind::Mention,
            reply_parent_id: None,
            at_ms,
            read: false,
            removed: false,
        }
    }

    const NOW: i64 = 1_800_000_000_000;

    #[test]
    fn a_message_is_filed_once() {
        let mut inbox = Inbox::default();
        assert!(inbox.file(entry("a", "xqc", NOW), NOW));
        assert!(!inbox.file(entry("a", "xqc", NOW), NOW));
        assert_eq!(inbox.unread_total(), 1);
    }

    #[test]
    fn the_cap_and_the_age_limit_drop_the_oldest() {
        let mut inbox = Inbox::default();
        for i in 0..(MAX_ENTRIES + 5) {
            inbox.file(entry(&format!("m{i}"), "xqc", NOW - 1000 + i as i64), NOW);
        }
        assert_eq!(inbox.entries.len(), MAX_ENTRIES);
        assert!(inbox.get("m0").is_none());
        assert!(inbox.get(&format!("m{}", MAX_ENTRIES + 4)).is_some());
        inbox.file(entry("old", "xqc", NOW - MAX_AGE_MS - 1), NOW);
        assert!(inbox.get("old").is_none());
    }

    #[test]
    fn a_visible_room_files_read_and_showing_a_room_reads_it() {
        let mut inbox = Inbox::default();
        inbox.file(entry("a", "xqc", NOW), NOW);
        inbox.file(entry("b", "kick:xqc", NOW), NOW);
        assert_eq!(inbox.unread_by_room().get("xqc"), Some(&1));
        assert!(inbox.set_visible(vec!["XQC".into()]));
        assert_eq!(inbox.unread_by_room().get("xqc"), None);
        inbox.file(entry("c", "xqc", NOW + 1), NOW);
        assert_eq!(inbox.unread_by_room().get("xqc"), None);
        assert_eq!(inbox.unread_by_room().get("kick:xqc"), Some(&1));
    }

    #[test]
    fn pages_run_newest_first() {
        let mut inbox = Inbox::default();
        for i in 0..5 {
            inbox.file(entry(&format!("m{i}"), "xqc", NOW + i), NOW);
        }
        let (first, more) = inbox.page(None, 2);
        assert_eq!(first.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["m4", "m3"]);
        assert!(more);
        let (rest, more) = inbox.page(Some(first[1].at_ms), 10);
        assert_eq!(rest.len(), 3);
        assert!(!more);
    }

    #[test]
    fn removed_messages_stop_counting() {
        let mut inbox = Inbox::default();
        inbox.file(entry("a", "xqc", NOW), NOW);
        assert!(inbox.mark_removed("a"));
        assert_eq!(inbox.unread_total(), 0);
        assert!(!inbox.mark_removed("a"));
    }

    #[test]
    fn room_keys_match_chat_tabs() {
        assert_eq!(room_key("twitch", "#XQC"), "xqc");
        assert_eq!(room_key("kick", "XQC"), "kick:xqc");
        assert_eq!(room_key("", "kick:Foo"), "kick:foo");
    }

    fn verdict(mentioned: bool, reply: bool, phrase: bool) -> Verdict {
        Verdict {
            drop: false,
            matched_highlight: phrase,
            mentioned,
            reply_to_me: reply,
            name_hit: None,
            phrase_hit: phrase,
        }
    }

    #[test]
    fn replies_win_then_names_then_opted_in_phrases() {
        assert_eq!(kind_for(&verdict(true, true, false), true), Some(MentionKind::Reply));
        assert_eq!(kind_for(&verdict(true, false, false), true), Some(MentionKind::Mention));
        assert_eq!(kind_for(&verdict(false, false, true), true), Some(MentionKind::Word));
        assert_eq!(kind_for(&verdict(false, false, true), false), None);
        assert_eq!(kind_for(&verdict(false, false, false), true), None);
    }
}
