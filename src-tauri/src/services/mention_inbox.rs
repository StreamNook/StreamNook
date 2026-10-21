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
//! Mentions the mention server found while the app was away arrive through
//! `merge_remote` (`services::mention_watch` pulls them): filed like a
//! backfill, deduped by id against what this app caught itself. A removed or
//! cleared id is kept as a tombstone, so a later pull never brings it back.
//!
//! Rooms are keyed the way chat tabs are: a bare lowercased login for Twitch,
//! `provider:slug` otherwise.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager};

use crate::models::chat_layout::ChatMessage;
use crate::models::settings::Settings;
use crate::services::chat_rules::{channel_filter_key, ChatRules, Verdict};

const DIR: &str = "mentions";
pub const MAX_ENTRIES: usize = 500;
pub const MAX_AGE_MS: i64 = 30 * 24 * 60 * 60 * 1000;
/// Removed and cleared ids kept so a pull never refiles them; the oldest go first.
pub const MAX_TOMBSTONES: usize = 2000;
/// Message text kept per entry, in characters.
const MAX_TEXT: usize = 500;
pub const CHANGED_EVENT: &str = "mention-inbox-changed";
/// An entry to show as a notification now (the payload is the entry): the
/// phones' notification and the desktop island's preview.
pub const NOTIFY_EVENT: &str = "mention-notify";
/// A live entry just filed unread (its chat is not on screen): the phones'
/// inbox card over the message bar. Payload `{entry, avatar}`. Not sent for
/// replay, nor while streamer mode is on.
pub const ARRIVED_EVENT: &str = "mention-arrived";

/// The `mention-arrived` payload.
#[derive(Serialize, Clone, Debug)]
pub struct Arrival {
    pub entry: MentionEntry,
    /// The sender's picture when it is cached already; otherwise it is being
    /// looked up and lands as `channel-avatars`.
    pub avatar: Option<String>,
}
/// One chat notifies at most this often, so a chat that keeps @-ing you is
/// one notification, not a stack.
const NOTIFY_COOLDOWN: Duration = Duration::from_secs(20);
/// A live message older than this when it arrives is replay, not news.
const NOTIFY_MAX_AGE_MS: i64 = 15_000;

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
    /// The channel as chat addresses it: a Twitch login (lowercased), else
    /// the slug or id with its own case.
    pub channel_login: String,
    /// What opens this chat (`startStream`): the bare Twitch login, else
    /// `provider:channel` with the channel's own case, since a YouTube
    /// channel id is case-sensitive. Older entries lack it; use `room`.
    #[serde(default)]
    pub open_key: String,
    /// The channel's own page on its platform (its live page, or the video):
    /// what a shell that cannot show this platform's chat opens instead.
    #[serde(default)]
    pub watch_url: String,
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
    /// The message as the chat drew it: words and emotes in order, so the
    /// inbox shows the same emotes. Empty on entries filed before it (and
    /// for a message with none): show `text`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<MentionPart>,
}

/// One piece of a filed message.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MentionPart {
    Text { text: String },
    Emote { name: String, url: String },
}

/// The chat's segments as inbox parts: emotes (and cheers) as images, the
/// rest as words. Zero-width emotes are left out; they only decorate the
/// emote before them. Empty when the message has no emote at all, since its
/// `text` already says everything. Capped at `MAX_TEXT` characters, like the
/// text.
fn parts_for(segments: &[crate::models::chat_layout::MessageSegment]) -> Vec<MentionPart> {
    use crate::models::chat_layout::MessageSegment as S;
    let mut out: Vec<MentionPart> = Vec::new();
    let mut budget = MAX_TEXT;
    let mut any_emote = false;
    for seg in segments {
        if budget == 0 {
            break;
        }
        let (content, image) = match seg {
            S::Emote { content, emote_url, is_zero_width, .. } => {
                if is_zero_width.unwrap_or(false) {
                    continue;
                }
                (content, Some(emote_url))
            }
            S::Cheermote { content, cheermote_url, .. } => (content, Some(cheermote_url)),
            S::Text { content }
            | S::Emoji { content, .. }
            | S::Link { content, .. }
            | S::Gif { content, .. }
            | S::Mention { content, .. } => (content, None),
        };
        match image {
            Some(url) if !url.is_empty() => {
                any_emote = true;
                budget = budget.saturating_sub(content.chars().count().max(1));
                out.push(MentionPart::Emote { name: content.clone(), url: url.clone() });
            }
            _ => {
                let piece: String = content.chars().take(budget).collect();
                budget -= piece.chars().count();
                match out.last_mut() {
                    Some(MentionPart::Text { text }) => text.push_str(&piece),
                    _ => out.push(MentionPart::Text { text: piece }),
                }
            }
        }
    }
    if any_emote { out } else { Vec::new() }
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
struct InboxFile {
    version: u32,
    /// Oldest first.
    entries: Vec<MentionEntry>,
    /// Ids removed or cleared here, oldest first (version 2).
    #[serde(default)]
    tombstones: Vec<String>,
    /// How far this account has pulled from the mention server: the last
    /// `next` it answered (version 2).
    #[serde(default)]
    remote_cursor_ms: i64,
}

/// One owner's inbox in memory. Pure apart from the clock it is handed, so the
/// rules are unit-tested here without files or an app.
#[derive(Default, Debug)]
pub struct Inbox {
    /// Oldest first.
    entries: Vec<MentionEntry>,
    /// Rooms on screen right now: mentions there arrive already read.
    visible: HashSet<String>,
    /// Removed and cleared ids, oldest first, at most `MAX_TOMBSTONES`.
    tombstones: Vec<String>,
    remote_cursor_ms: i64,
}

impl Inbox {
    fn from_entries(mut entries: Vec<MentionEntry>, now_ms: i64) -> Self {
        entries.sort_by_key(|e| e.at_ms);
        let mut inbox = Inbox {
            entries,
            visible: HashSet::new(),
            tombstones: Vec::new(),
            remote_cursor_ms: 0,
        };
        inbox.prune(now_ms);
        inbox
    }

    fn from_file(file: InboxFile, now_ms: i64) -> Self {
        let mut inbox = Self::from_entries(file.entries, now_ms);
        inbox.tombstones = file.tombstones;
        inbox.remote_cursor_ms = file.remote_cursor_ms;
        inbox
    }

    fn to_file(&self) -> InboxFile {
        InboxFile {
            version: 2,
            entries: self.entries.clone(),
            tombstones: self.tombstones.clone(),
            remote_cursor_ms: self.remote_cursor_ms,
        }
    }

    fn tombstone(&mut self, id: &str) {
        if self.tombstones.iter().any(|t| t == id) {
            return;
        }
        self.tombstones.push(id.to_string());
        if self.tombstones.len() > MAX_TOMBSTONES {
            let extra = self.tombstones.len() - MAX_TOMBSTONES;
            self.tombstones.drain(..extra);
        }
    }

    /// File one entry. False when it was already there (same message id), or
    /// was removed or cleared here before.
    pub fn file(&mut self, mut entry: MentionEntry, now_ms: i64) -> bool {
        if self.entries.iter().any(|e| e.id == entry.id && e.platform == entry.platform) {
            return false;
        }
        if self.tombstones.iter().any(|t| *t == entry.id) {
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
        let removed = self.entries.len() != before;
        if removed {
            self.tombstone(id);
        }
        removed
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
        let ids: Vec<String> = self.entries.drain(..).map(|e| e.id).collect();
        for id in ids {
            self.tombstone(&id);
        }
        had
    }

    /// Mentions the mention server found, filed like a backfill (read when
    /// their chat is on screen). Answers how many were new: one this app
    /// caught itself, or removed before, is skipped.
    pub fn merge_remote(&mut self, remote: Vec<MentionEntry>, now_ms: i64) -> usize {
        remote.into_iter().filter(|e| self.file(e.clone(), now_ms)).count()
    }

    pub fn get(&self, id: &str) -> Option<&MentionEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut MentionEntry> {
        self.entries.iter_mut().find(|e| e.id == id)
    }

    /// One page of the phones' inbox timeline: newest first, only what
    /// `filter` admits, older than `before_ms` when given, and each run of
    /// identical messages (same chat, same sender, same words, one right after
    /// the other) folded into its newest row. Returns the rows, whether there
    /// is more, and the cursor for the next page (the oldest entry consumed,
    /// so a fold never splits across pages).
    pub fn view(&self, filter: InboxFilter, before_ms: Option<i64>, limit: usize) -> (Vec<InboxRow>, bool, Option<i64>) {
        let mut rows: Vec<InboxRow> = Vec::new();
        let mut cursor = None;
        let mut iter = self
            .entries
            .iter()
            .rev()
            .filter(|e| filter.admits(e.kind) && before_ms.is_none_or(|b| e.at_ms < b))
            .peekable();
        while let Some(head) = iter.next() {
            let mut row = InboxRow {
                entry: head.clone(),
                folded: Vec::new(),
                unread: !head.read && !head.removed,
                avatar: None,
            };
            cursor = Some(head.at_ms);
            while let Some(next) = iter.next_if(|n| same_message(head, n)) {
                row.unread |= !next.read && !next.removed;
                row.folded.push(FoldedEntry { id: next.id.clone(), at_ms: next.at_ms, read: next.read });
                cursor = Some(next.at_ms);
            }
            rows.push(row);
            if rows.len() >= limit {
                return (rows, iter.peek().is_some(), cursor);
            }
        }
        (rows, false, cursor)
    }

    /// Unread entries one chip would show.
    pub fn unread_for(&self, filter: InboxFilter) -> u32 {
        self.entries
            .iter()
            .filter(|e| !e.read && !e.removed && filter.admits(e.kind))
            .count() as u32
    }
}

/// The phones' inbox chips: which entries a view shows. The whispers car adds
/// its own chip and rows to the view (the command's `chips` lists what exists).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum InboxFilter {
    #[default]
    All,
    /// Mentions and replies.
    Mentions,
    /// Highlight phrases.
    Highlights,
}

impl InboxFilter {
    pub const CHIPS: [InboxFilter; 3] = [InboxFilter::All, InboxFilter::Mentions, InboxFilter::Highlights];

    pub fn admits(self, kind: MentionKind) -> bool {
        match self {
            InboxFilter::All => true,
            InboxFilter::Mentions => matches!(kind, MentionKind::Mention | MentionKind::Reply),
            InboxFilter::Highlights => kind == MentionKind::Word,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            InboxFilter::All => "all",
            InboxFilter::Mentions => "mentions",
            InboxFilter::Highlights => "highlights",
        }
    }
}

/// One row of the phones' inbox: the newest entry of a run of identical
/// messages, with the older ones folded under it.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct InboxRow {
    pub entry: MentionEntry,
    /// The older identical messages, newest first. Empty for a lone message.
    pub folded: Vec<FoldedEntry>,
    /// Any message in the row is unread.
    pub unread: bool,
    /// The sender's picture, when it is known yet.
    pub avatar: Option<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct FoldedEntry {
    pub id: String,
    pub at_ms: i64,
    pub read: bool,
}

/// Invisible characters chat clients add to slip a repeated message past
/// Twitch's duplicate check; two messages that differ only by them are the
/// same message to a reader.
fn is_padding(c: char) -> bool {
    matches!(c, '\u{034F}' | '\u{200B}'..='\u{200D}' | '\u{2060}' | '\u{FEFF}' | '\u{E0000}' | '\u{2800}')
}

fn fold_text(text: &str) -> String {
    text.chars()
        .filter(|c| !is_padding(*c))
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn same_message(a: &MentionEntry, b: &MentionEntry) -> bool {
    let sender = |e: &MentionEntry| if e.sender_id.is_empty() { e.sender_login.to_lowercase() } else { e.sender_id.clone() };
    !a.removed
        && !b.removed
        && a.kind == b.kind
        && a.room == b.room
        && sender(a) == sender(b)
        && fold_text(&a.text) == fold_text(&b.text)
}

/// What the picture cache looks a sender up by: the platform's own id space
/// (Twitch and YouTube ids, Kick slugs).
pub fn avatar_request(entry: &MentionEntry) -> Option<crate::services::channel_avatars::AvatarRequest> {
    let id = match entry.platform.as_str() {
        "kick" => entry.sender_login.clone(),
        "twitch" | "youtube" => entry.sender_id.clone(),
        _ => return None,
    };
    (!id.trim().is_empty()).then(|| crate::services::channel_avatars::AvatarRequest { provider: entry.platform.clone(), id })
}

/// The picture cache's key for a request, as `channel_avatars::request` answers it.
pub fn avatar_key(req: &crate::services::channel_avatars::AvatarRequest) -> String {
    format!("{}:{}", req.provider, req.id.to_lowercase())
}

/// Pictures for these entries' senders, keyed by `avatar_key`: whatever is
/// cached, now. The rest are looked up in the background (batched, kept on
/// disk) and broadcast as `channel-avatars`.
pub fn avatars_for<'a>(entries: impl Iterator<Item = &'a MentionEntry>) -> HashMap<String, String> {
    let Some(app) = crate::services::providers::app_handle() else { return HashMap::new() };
    let mut seen = HashSet::new();
    let requests: Vec<_> = entries
        .filter_map(avatar_request)
        .filter(|r| seen.insert(avatar_key(r)))
        .collect();
    if requests.is_empty() {
        return HashMap::new();
    }
    crate::services::channel_avatars::request(&app, requests)
}

/// The picture for one entry's sender out of `avatars_for`'s answer.
pub fn avatar_of(entry: &MentionEntry, known: &HashMap<String, String>) -> Option<String> {
    avatar_request(entry).and_then(|r| known.get(&avatar_key(&r)).cloned())
}

/// The key that opens a message's chat: `room_key` without the lowercasing
/// of anything but the provider and a Twitch login.
pub fn open_key(provider: &str, channel: &str) -> String {
    let ch = channel.trim().trim_start_matches('#');
    let (p, c) = match ch.split_once(':') {
        Some((p, c)) => (p.to_lowercase(), c.to_string()),
        None => ((if provider.is_empty() { "twitch" } else { provider }).to_lowercase(), ch.to_string()),
    };
    if p == "twitch" {
        c.to_lowercase()
    } else {
        format!("{p}:{c}")
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
    let open = open_key(&msg.provider, &msg.channel);
    let channel_login = open.split_once(':').map(|(_, c)| c.to_string()).unwrap_or_else(|| open.clone());
    let platform = if msg.provider.is_empty() { "twitch".to_string() } else { msg.provider.to_lowercase() };
    let watch_url = crate::services::providers::watch_urls::watch_url(&platform, &channel_login);
    MentionEntry {
        id: msg.id.clone(),
        room: room_key(&msg.provider, &msg.channel),
        open_key: open,
        watch_url,
        platform,
        channel_login,
        channel_id: msg.tags.get("room-id").cloned(),
        sender_id: msg.user_id.clone(),
        sender_login: msg.username.to_lowercase(),
        sender_name: if msg.display_name.is_empty() { msg.username.clone() } else { msg.display_name.clone() },
        // The color the chat row shows, so a dark name stays readable.
        sender_color: Some(crate::services::chat_row_view::readable_color(msg.color.as_deref())),
        text: msg.content.chars().take(MAX_TEXT).collect(),
        parts: parts_for(&msg.segments),
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
    /// Notifications on and "Mentions and replies" on.
    notify: bool,
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
        .get_or_init(|| RwLock::new(Config { highlight_words: true, muted: HashSet::new(), notify: false }))
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
        notify: settings.live_notifications.enabled && settings.live_notifications.show_mention_notifications,
    };
    if let Ok(mut g) = CONFIG.get_or_init(|| RwLock::new(Config::default())).write() {
        *g = next;
    }
}

/// Whether a newly filed live entry is announced now. Pure apart from the
/// clocks and the cooldown table it is handed.
fn should_notify(
    room: &str,
    sent_at: Option<i64>,
    now_ms: i64,
    now: Instant,
    last: &mut HashMap<String, Instant>,
) -> bool {
    if sent_at.is_some_and(|t| now_ms - t > NOTIFY_MAX_AGE_MS) {
        return false;
    }
    if last.get(room).is_some_and(|t| now.duration_since(*t) < NOTIFY_COOLDOWN) {
        return false;
    }
    last.retain(|_, t| now.duration_since(*t) < NOTIFY_COOLDOWN);
    last.insert(room.to_string(), now);
    true
}

static NOTIFY_LAST: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();

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
    let file = file_path(owner)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<InboxFile>(&s).ok())
        .unwrap_or_default();
    Inbox::from_file(file, now_ms())
}

fn write(owner: &str, file: &InboxFile) -> Result<(), String> {
    let path = file_path(owner).ok_or("no mentions folder")?;
    let body = serde_json::to_string(file).map_err(|e| e.to_string())?;
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
                    let _ = write(&old.owner, &old.inbox.to_file());
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
                let mut file = st.inbox.to_file();
                file.entries = Inbox::from_entries(file.entries, now_ms()).entries;
                Some((st.owner.clone(), file))
            }
            _ => None,
        }
    };
    match snapshot {
        Some((owner, file)) => write(&owner, &file).inspect_err(|_| {
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
    if visible || crate::services::streamer_mode::StreamerMode::is_active() {
        return;
    }
    let fresh = sent_at_ms(msg).is_none_or(|t| now_ms() - t <= NOTIFY_MAX_AGE_MS);
    if fresh {
        if let Some(app) = crate::services::providers::app_handle() {
            let avatar = avatar_of(&entry, &avatars_for(std::iter::once(&entry)));
            let _ = app.emit(ARRIVED_EVENT, Arrival { entry: entry.clone(), avatar });
        }
    }
    if !cfg.notify {
        return;
    }
    let fire = match NOTIFY_LAST.get_or_init(|| Mutex::new(HashMap::new())).lock() {
        Ok(mut last) => should_notify(&entry.room, sent_at_ms(msg), now_ms(), Instant::now(), &mut last),
        Err(_) => false,
    };
    if !fire {
        return;
    }
    // Desktop: the system notification (or MultiChat's card) for when the
    // main window is not in use. Every app: the entry, for the page to show
    // (the phones' notification, the desktop island's preview).
    #[cfg(desktop)]
    crate::services::mention_notify::deliver(msg, kind);
    if let Some(app) = crate::services::providers::app_handle() {
        let _ = app.emit(NOTIFY_EVENT, &entry);
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
    // Opening a chat keeps it watched while offline (mentions while you're away).
    crate::services::mention_watch::client::lease_rooms(&rooms);
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

/// One page of the phones' inbox timeline (see `Inbox::view`), with the
/// unread total and each chip's unread count. None when nobody is signed in.
pub fn view(
    filter: InboxFilter,
    before_ms: Option<i64>,
    limit: usize,
) -> Option<(Vec<InboxRow>, bool, Option<i64>, u32, Vec<(InboxFilter, u32)>)> {
    with_inbox(|inbox| {
        let (rows, more, cursor) = inbox.view(filter, before_ms, limit);
        let chips = InboxFilter::CHIPS.iter().map(|f| (*f, inbox.unread_for(*f))).collect();
        ((rows, more, cursor, inbox.unread_total(), chips), false)
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

/// One page the mention server sent: its mentions filed without a sound, a
/// notification or an arrival card, and the account's cursor moved to `next`
/// (forward only), in one go, so a page is at most one inbox update. Muted
/// chats are left out. Answers how many were new; None when nobody is signed
/// in to Twitch. A cursor that moved with nothing new is written with the next
/// change: losing it costs only a pull that refiles nothing.
pub fn merge_remote(remote: Vec<MentionEntry>, next: i64) -> Option<usize> {
    let cfg = config();
    let remote: Vec<MentionEntry> = remote.into_iter().filter(|e| !cfg.muted.contains(&e.room)).collect();
    with_inbox(|inbox| {
        let filed = inbox.merge_remote(remote, now_ms());
        if next > inbox.remote_cursor_ms {
            inbox.remote_cursor_ms = next;
        }
        (filed, filed > 0)
    })
}

/// How far the signed-in account has pulled from the mention server.
pub fn remote_cursor() -> Option<i64> {
    with_inbox(|inbox| (inbox.remote_cursor_ms, false))
}

/// Whether this app already holds the message (it caught it itself), or its
/// chat is on screen now: a push for it would only repeat what is in front of
/// the viewer. False when nobody is signed in in this process.
pub fn caught_here(id: &str, room: &str) -> bool {
    let room = room.to_lowercase();
    with_inbox(|inbox| (inbox.get(id).is_some() || inbox.visible.contains(&room), false)).unwrap_or(false)
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
            open_key: room.into(),
            watch_url: String::new(),
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
            parts: Vec::new(),
        }
    }

    const NOW: i64 = 1_800_000_000_000;

    #[test]
    fn parts_keep_emotes_in_place_and_merge_words() {
        use crate::models::chat_layout::MessageSegment as S;
        let segs = vec![
            S::Text { content: "@you ".into() },
            S::Emote {
                content: "Kappa".into(),
                emote_id: Some("25".into()),
                emote_url: "https://e/25".into(),
                is_zero_width: None,
                modifier_flags: None,
                is_personal: None,
            },
            S::Emote {
                content: "cvHazmat".into(),
                emote_id: None,
                emote_url: "https://e/z".into(),
                is_zero_width: Some(true),
                modifier_flags: None,
                is_personal: None,
            },
            S::Text { content: " hi ".into() },
            S::Link { content: "x.com".into(), url: "https://x.com".into(), card: None, label: String::new() },
        ];
        assert_eq!(
            parts_for(&segs),
            vec![
                MentionPart::Text { text: "@you ".into() },
                MentionPart::Emote { name: "Kappa".into(), url: "https://e/25".into() },
                MentionPart::Text { text: " hi x.com".into() },
            ]
        );
        // No emote: the text says it all.
        assert!(parts_for(&[S::Text { content: "plain".into() }]).is_empty());
    }

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

    #[test]
    fn open_keys_keep_a_case_sensitive_channel() {
        assert_eq!(open_key("twitch", "#XQC"), "xqc");
        assert_eq!(open_key("youtube", "UCabcDEF"), "youtube:UCabcDEF");
        assert_eq!(open_key("", "youtube:UCabcDEF"), "youtube:UCabcDEF");
        assert_eq!(room_key("youtube", "UCabcDEF"), "youtube:ucabcdef");
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
    fn one_notification_per_chat_per_cooldown_and_never_for_replay() {
        let mut last = HashMap::new();
        let t0 = Instant::now();
        let now_ms = 1_000_000;
        assert!(should_notify("a", Some(now_ms - 1_000), now_ms, t0, &mut last));
        assert!(!should_notify("a", Some(now_ms), now_ms, t0 + Duration::from_secs(5), &mut last));
        assert!(should_notify("b", None, now_ms, t0 + Duration::from_secs(5), &mut last));
        assert!(should_notify("a", None, now_ms, t0 + NOTIFY_COOLDOWN, &mut last));
        assert!(!should_notify("c", Some(now_ms - NOTIFY_MAX_AGE_MS - 1), now_ms, t0, &mut last));
    }

    fn said(id: &str, sender: &str, text: &str, at_ms: i64) -> MentionEntry {
        let mut e = entry(id, "xqc", at_ms);
        e.sender_id = sender.into();
        e.text = text.into();
        e
    }

    #[test]
    fn identical_messages_in_a_row_fold_into_the_newest() {
        let mut inbox = Inbox::default();
        inbox.file(said("a", "1", "@you hi", NOW), NOW);
        inbox.file(said("b", "1", "@you  hi\u{E0000}", NOW + 1), NOW);
        inbox.file(said("c", "1", "@You hi", NOW + 2), NOW);
        inbox.file(said("d", "2", "@you hi", NOW + 3), NOW);
        inbox.file(said("e", "1", "@you hi", NOW + 4), NOW);
        let (rows, more, _) = inbox.view(InboxFilter::All, None, 10);
        assert!(!more);
        // e alone (d comes between it and the run), d alone (another sender), c folding b and a.
        assert_eq!(rows.iter().map(|r| r.entry.id.as_str()).collect::<Vec<_>>(), ["e", "d", "c"]);
        assert_eq!(rows[2].folded.iter().map(|f| f.id.as_str()).collect::<Vec<_>>(), ["b", "a"]);
        assert!(rows[2].unread);
    }

    #[test]
    fn a_removed_message_or_another_chat_never_folds() {
        let mut inbox = Inbox::default();
        inbox.file(said("a", "1", "hi", NOW), NOW);
        let mut other = said("b", "1", "hi", NOW + 1);
        other.room = "kick:xqc".into();
        inbox.file(other, NOW);
        inbox.file(said("c", "1", "hi", NOW + 2), NOW);
        inbox.mark_removed("c");
        let (rows, _, _) = inbox.view(InboxFilter::All, None, 10);
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn chips_filter_by_kind_and_count_their_unread() {
        let mut inbox = Inbox::default();
        inbox.file(said("a", "1", "one", NOW), NOW);
        let mut reply = said("b", "2", "two", NOW + 1);
        reply.kind = MentionKind::Reply;
        inbox.file(reply, NOW);
        let mut word = said("c", "3", "three", NOW + 2);
        word.kind = MentionKind::Word;
        inbox.file(word, NOW);
        inbox.mark_read(Some(&["a".to_string()]));
        let ids = |f| inbox.view(f, None, 10).0.into_iter().map(|r| r.entry.id).collect::<Vec<_>>();
        assert_eq!(ids(InboxFilter::Mentions), ["b", "a"]);
        assert_eq!(ids(InboxFilter::Highlights), ["c"]);
        assert_eq!(inbox.unread_for(InboxFilter::Mentions), 1);
        assert_eq!(inbox.unread_for(InboxFilter::Highlights), 1);
        assert_eq!(inbox.unread_for(InboxFilter::All), 2);
    }

    #[test]
    fn view_pages_never_split_a_fold() {
        let mut inbox = Inbox::default();
        for i in 0..4 {
            inbox.file(said(&format!("same{i}"), "1", "spam", NOW + i), NOW);
        }
        inbox.file(said("x", "2", "other", NOW + 10), NOW);
        inbox.file(said("y", "3", "older", NOW - 10), NOW);
        let (first, more, cursor) = inbox.view(InboxFilter::All, None, 2);
        assert!(more);
        assert_eq!(first[1].entry.id, "same3");
        assert_eq!(first[1].folded.len(), 3);
        assert_eq!(cursor, Some(NOW));
        let (rest, more, _) = inbox.view(InboxFilter::All, cursor, 2);
        assert_eq!(rest.iter().map(|r| r.entry.id.as_str()).collect::<Vec<_>>(), ["y"]);
        assert!(!more);
    }

    #[test]
    fn pictures_are_looked_up_in_each_platforms_own_id_space() {
        let mut e = entry("a", "xqc", NOW);
        e.sender_id = "123".into();
        e.sender_login = "Viewer".into();
        assert_eq!(avatar_request(&e).map(|r| avatar_key(&r)), Some("twitch:123".into()));
        e.platform = "kick".into();
        assert_eq!(avatar_request(&e).map(|r| avatar_key(&r)), Some("kick:viewer".into()));
        e.platform = "tiktok".into();
        assert!(avatar_request(&e).is_none());
    }

    #[test]
    fn replies_win_then_names_then_opted_in_phrases() {
        assert_eq!(kind_for(&verdict(true, true, false), true), Some(MentionKind::Reply));
        assert_eq!(kind_for(&verdict(true, false, false), true), Some(MentionKind::Mention));
        assert_eq!(kind_for(&verdict(false, false, true), true), Some(MentionKind::Word));
        assert_eq!(kind_for(&verdict(false, false, true), false), None);
        assert_eq!(kind_for(&verdict(false, false, false), true), None);
    }

    #[test]
    fn removed_and_cleared_ids_never_come_back_from_a_pull() {
        let mut inbox = Inbox::default();
        assert!(inbox.file(entry("a", "room", 1_000), 2_000));
        assert!(inbox.file(entry("b", "room", 1_100), 2_000));
        assert!(inbox.remove("a"));
        assert_eq!(inbox.merge_remote(vec![entry("a", "room", 1_000), entry("c", "room", 1_200)], 2_000), 1);
        assert!(inbox.get("a").is_none() && inbox.get("c").is_some());
        assert!(inbox.clear());
        assert_eq!(inbox.merge_remote(vec![entry("b", "room", 1_100), entry("c", "room", 1_200)], 2_000), 0);
        assert_eq!(inbox.merge_remote(vec![entry("d", "room", 1_300)], 2_000), 1);
    }

    #[test]
    fn a_pull_never_files_what_this_app_caught_itself() {
        let mut inbox = Inbox::default();
        assert!(inbox.file(entry("a", "room", 1_000), 2_000));
        assert_eq!(inbox.merge_remote(vec![entry("a", "room", 1_000)], 2_000), 0);
        assert_eq!(inbox.entries.len(), 1);
    }

    #[test]
    fn a_pulled_mention_for_a_chat_on_screen_arrives_read() {
        let mut inbox = Inbox::default();
        inbox.set_visible(vec!["room".into()]);
        assert_eq!(inbox.merge_remote(vec![entry("a", "room", 1_000), entry("b", "other", 1_000)], 2_000), 2);
        assert!(inbox.get("a").unwrap().read);
        assert!(!inbox.get("b").unwrap().read);
    }

    #[test]
    fn tombstones_are_capped_oldest_first() {
        let mut inbox = Inbox::default();
        for i in 0..(MAX_TOMBSTONES + 5) {
            inbox.tombstone(&format!("id{i}"));
        }
        assert_eq!(inbox.tombstones.len(), MAX_TOMBSTONES);
        assert_eq!(inbox.tombstones.first().map(String::as_str), Some("id5"));
        inbox.tombstone("id5");
        assert_eq!(inbox.tombstones.len(), MAX_TOMBSTONES);
    }

    #[test]
    fn the_file_keeps_tombstones_and_the_cursor_and_reads_version_one() {
        let mut inbox = Inbox::default();
        assert!(inbox.file(entry("a", "room", 1_000), 2_000));
        assert!(inbox.remove("a"));
        inbox.remote_cursor_ms = 42;
        let text = serde_json::to_string(&inbox.to_file()).unwrap();
        let back = Inbox::from_file(serde_json::from_str(&text).unwrap(), 2_000);
        assert_eq!(back.tombstones, vec!["a".to_string()]);
        assert_eq!(back.remote_cursor_ms, 42);
        let v1 = r#"{"version":1,"entries":[]}"#;
        let old = Inbox::from_file(serde_json::from_str(v1).unwrap(), 2_000);
        assert!(old.tombstones.is_empty() && old.remote_cursor_ms == 0);
    }
}
