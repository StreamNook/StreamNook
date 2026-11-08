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
/// A reply's parent message kept per entry, in characters.
const MAX_PARENT: usize = 200;
/// The replied-to line a notification shows, in characters as people count
/// them (an emoji built of several code points is one).
const QUOTE_GRAPHEMES: usize = 60;
/// How long a notification waits for a face that is not cached yet.
const FACES_WAIT: Duration = Duration::from_millis(1500);
pub const CHANGED_EVENT: &str = "mention-inbox-changed";
/// An entry to show as a notification now (the payload is a `MentionNotice`:
/// the entry's own fields, its words and its faces): the phones'
/// notification and the desktop island's preview.
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

/// A mention notification's words, the same on every app. The mention
/// server's pushes follow `crates/sn-mention-rule/tests/notice_fixtures.json`
/// too, so a pushed notification reads like the app's own.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct NoticeText {
    /// The sender's name.
    pub title: String,
    /// What happened and where: "Replied to you in bigstreamer".
    pub subtitle: String,
    /// The whole thing on one line, for a notification with no subtitle:
    /// "Viewer replied to you in bigstreamer".
    pub headline: String,
    /// The message they replied to (yours), quoted on one line. Replies only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quote: Option<String>,
    /// Their message.
    pub body: String,
    /// The chat as a notification names it: "bigstreamer", "SomeSlug on Kick".
    pub chat_title: String,
}

/// The `mention-notify` payload. The entry's fields stay at the top level, so
/// everything that reads the entry keeps working; the words sit under
/// `notice` (never beside the entry's own `text`).
#[derive(Serialize, Clone, Debug)]
pub struct MentionNotice {
    #[serde(flatten)]
    pub entry: MentionEntry,
    pub notice: NoticeText,
    /// The sender's picture, when cached or found within `FACES_WAIT`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sender_avatar: Option<String>,
    /// The channel's picture, the same way.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_avatar: Option<String>,
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
    /// The message a reply answers (yours), its words on one line, for the
    /// notification to quote. Replies only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_parent_text: Option<String>,
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
    /// It arrived while its chat was on screen: the viewer saw it happen, so
    /// the desktop island leaves it out. The inboxes still list it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub seen_live: bool,
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
            entry.seen_live = true;
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

    /// Unread entries the desktop island lists (it leaves out what arrived
    /// in a chat on screen).
    pub fn unread_unseen(&self) -> u32 {
        self.entries.iter().filter(|e| !e.read && !e.removed && !e.seen_live).count() as u32
    }

    /// Newest first, older than `before_ms` when given. `unseen_only` leaves
    /// out what arrived in a chat on screen (the desktop island).
    pub fn page(&self, before_ms: Option<i64>, limit: usize, unseen_only: bool) -> (Vec<MentionEntry>, bool) {
        let mut out: Vec<MentionEntry> = self
            .entries
            .iter()
            .rev()
            .filter(|e| before_ms.is_none_or(|b| e.at_ms < b) && !(unseen_only && e.seen_live))
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
        self.view_on(filter, before_ms, limit, false)
    }

    /// `view`, leaving out what arrived in a chat on screen when
    /// `skip_seen_live` (the desktop island).
    pub fn view_on(
        &self,
        filter: InboxFilter,
        before_ms: Option<i64>,
        limit: usize,
        skip_seen_live: bool,
    ) -> (Vec<InboxRow>, bool, Option<i64>) {
        let mut rows: Vec<InboxRow> = Vec::new();
        let mut cursor = None;
        let mut iter = self
            .entries
            .iter()
            .rev()
            .filter(|e| {
                filter.admits(e.kind)
                    && before_ms.is_none_or(|b| e.at_ms < b)
                    && !(skip_seen_live && e.seen_live)
            })
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

/// The inbox chips: which rows a view shows. `All` is every mention row and
/// every whisper conversation, merged newest first.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum InboxFilter {
    #[default]
    All,
    /// Mentions and replies.
    Mentions,
    /// Highlight phrases.
    Highlights,
    /// Whisper conversations only.
    Whispers,
}

impl InboxFilter {
    pub const CHIPS: [InboxFilter; 4] =
        [InboxFilter::All, InboxFilter::Mentions, InboxFilter::Highlights, InboxFilter::Whispers];

    /// Whether a mention entry of this kind shows under this chip. Whispers
    /// are not mention entries, so the Whispers chip admits none.
    pub fn admits(self, kind: MentionKind) -> bool {
        match self {
            InboxFilter::All => true,
            InboxFilter::Mentions => matches!(kind, MentionKind::Mention | MentionKind::Reply),
            InboxFilter::Highlights => kind == MentionKind::Word,
            InboxFilter::Whispers => false,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            InboxFilter::All => "all",
            InboxFilter::Mentions => "mentions",
            InboxFilter::Highlights => "highlights",
            InboxFilter::Whispers => "whispers",
        }
    }

    /// Whether this chip shows mention rows at all.
    fn shows_mentions(self) -> bool {
        self != InboxFilter::Whispers
    }

    /// Whether this chip shows whisper conversations.
    fn shows_whispers(self) -> bool {
        matches!(self, InboxFilter::All | InboxFilter::Whispers)
    }
}

/// Which app asks for the view. Defined in `inbox_counts`, so `whisper_inbox`
/// can name it too; re-exported here so `mention_inbox::Surface` keeps working.
pub use crate::services::inbox_counts::Surface;

/// One row of any inbox view: a mention row (with the identical messages
/// folded under it), or one whisper conversation at its newest message.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InboxItem {
    Mention {
        row_id: String,
        entry: MentionEntry,
        folded: Vec<FoldedEntry>,
        unread: bool,
        avatar: Option<String>,
    },
    Whisper {
        row_id: String,
        whisper: crate::services::whisper_inbox::RowMeta,
    },
}

impl InboxItem {
    /// The time the row sorts by: a mention row's newest entry, a
    /// conversation's newest message.
    fn at_ms(&self) -> i64 {
        match self {
            InboxItem::Mention { entry, .. } => entry.at_ms,
            InboxItem::Whisper { whisper, .. } => whisper.last_message_timestamp,
        }
    }
}

/// Where the next page of a merged view starts, on each side. Travels to
/// the shells as an opaque JSON string.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
struct ViewCursor {
    /// Mention side: read entries strictly older than this.
    #[serde(default)]
    m: Option<i64>,
    #[serde(default)]
    m_done: bool,
    /// Whisper side: start strictly after this (time, key) position.
    #[serde(default)]
    w: Option<(i64, String)>,
    #[serde(default)]
    w_done: bool,
}

/// One page of a merged view, as the command returns it.
pub struct ViewPage {
    pub rows: Vec<InboxItem>,
    pub has_more: bool,
    pub cursor: Option<String>,
    pub chips: Vec<(InboxFilter, u32)>,
}

/// The oldest entry a mention row consumed: its last folded entry, else
/// its own. The next mention page reads strictly older than this, so a
/// fold is never split across pages.
fn row_floor(row: &InboxRow) -> i64 {
    row.folded.last().map(|f| f.at_ms).unwrap_or(row.entry.at_ms)
}

/// Merge one page from the two sides, newest first.
///
/// `mentions` holds up to `limit` rows read from the mention side's cursor,
/// plus whether that side has more. `whispers` holds up to `limit + 1`
/// conversations read from the whisper side's cursor. A side that the chip
/// does not show, or that is already done, comes in as `None`. Ties go to
/// the mention row. Each side's cursor moves past the last row of that side
/// taken, so no row is skipped and none shows twice.
fn merge_view(
    mentions: Option<(Vec<InboxRow>, bool)>,
    whispers: Option<Vec<crate::services::whisper_inbox::RowMeta>>,
    cursor: &ViewCursor,
    limit: usize,
) -> (Vec<InboxItem>, bool, Option<ViewCursor>) {
    let (m_rows, m_more) = mentions.unwrap_or((Vec::new(), false));
    let mut w_rows = whispers.unwrap_or_default();
    let w_more = w_rows.len() > limit;
    w_rows.truncate(limit);

    let mut out: Vec<InboxItem> = Vec::new();
    let (mut mi, mut wi) = (0usize, 0usize);
    let mut last_floor: Option<i64> = None;
    let mut last_w: Option<(i64, String)> = None;
    while out.len() < limit && (mi < m_rows.len() || wi < w_rows.len()) {
        let take_mention = match (m_rows.get(mi), w_rows.get(wi)) {
            (Some(m), Some(w)) => m.entry.at_ms >= w.last_message_timestamp,
            (Some(_), None) => true,
            _ => false,
        };
        if take_mention {
            let row = m_rows[mi].clone();
            mi += 1;
            last_floor = Some(row_floor(&row));
            out.push(InboxItem::Mention {
                row_id: row.entry.id.clone(),
                entry: row.entry,
                folded: row.folded,
                unread: row.unread,
                avatar: row.avatar,
            });
        } else {
            let w = w_rows[wi].clone();
            wi += 1;
            last_w = Some((w.last_message_timestamp, w.key.clone()));
            out.push(InboxItem::Whisper { row_id: format!("w:{}", w.key), whisper: w });
        }
    }

    let next = ViewCursor {
        m: last_floor.or(cursor.m),
        m_done: cursor.m_done || (mi == m_rows.len() && !m_more),
        w: last_w.or_else(|| cursor.w.clone()),
        w_done: cursor.w_done || (wi == w_rows.len() && !w_more),
    };
    let has_more = !(next.m_done && next.w_done);
    (out, has_more, has_more.then_some(next))
}

/// The chips the inbox shows, with their unread counts. The Whispers chip
/// exists only while a Twitch account is followed (whispers are Twitch's);
/// All counts both sides.
fn chips_for(twitch: bool, mention_unread: [u32; 3], whisper_unread: u32) -> Vec<(InboxFilter, u32)> {
    let [all, mentions, highlights] = mention_unread;
    let mut chips = vec![
        (InboxFilter::All, all + if twitch { whisper_unread } else { 0 }),
        (InboxFilter::Mentions, mentions),
        (InboxFilter::Highlights, highlights),
    ];
    if twitch {
        chips.push((InboxFilter::Whispers, whisper_unread));
    }
    chips
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

/// Whether this verdict raises a ping under the current settings: an @
/// mention, a reply, or a highlight phrase while phrases count.
pub fn pinged(verdict: &Verdict) -> bool {
    pinged_with(verdict, config().highlight_words)
}

fn pinged_with(verdict: &Verdict, highlight_words: bool) -> bool {
    kind_for(verdict, highlight_words).is_some()
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
        reply_parent_text: if kind == MentionKind::Reply { reply_parent_of(msg) } else { None },
        at_ms: sent_at_ms(msg).unwrap_or(now_ms),
        read: false,
        removed: false,
        seen_live: false,
    }
}

/// A reply's parent words: from the parsed reply (Twitch, its history
/// replay and Kick fill it), else Twitch's raw tag.
fn reply_parent_of(msg: &ChatMessage) -> Option<String> {
    msg.metadata
        .reply_info
        .as_ref()
        .and_then(|r| parent_text(&r.parent_msg_body))
        .or_else(|| {
            msg.tags
                .get("reply-parent-msg-body")
                .and_then(|raw| parent_text(&crate::services::irc_service::unescape_irc_tag(raw)))
        })
}

/// A parent message as kept: its words on one line, at most `MAX_PARENT`
/// characters. None when it has none.
pub fn parent_text(raw: &str) -> Option<String> {
    raw_line(raw).map(|line| line.chars().take(MAX_PARENT).collect())
}

/// The replied-to line: the parent's words in quotes, cut at
/// `QUOTE_GRAPHEMES` (never through an emoji) with an ellipsis.
fn quote_line(parent: &str) -> Option<String> {
    use unicode_segmentation::UnicodeSegmentation;
    let line = raw_line(parent)?;
    let graphemes: Vec<&str> = line.graphemes(true).collect();
    let words = if graphemes.len() > QUOTE_GRAPHEMES {
        format!("{}\u{2026}", graphemes[..QUOTE_GRAPHEMES].concat().trim_end())
    } else {
        line
    };
    Some(format!("\u{201C}{words}\u{201D}"))
}

/// The words on one line: whitespace runs and line breaks become one space.
fn raw_line(raw: &str) -> Option<String> {
    let line = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    (!line.is_empty()).then_some(line)
}

/// A platform's name where a notification names it; none for Twitch.
fn platform_label(platform: &str) -> Option<String> {
    match platform {
        "" | "twitch" => None,
        "kick" => Some("Kick".into()),
        "youtube" => Some("YouTube".into()),
        "tiktok" => Some("TikTok".into()),
        other => {
            let mut chars = other.chars();
            chars.next().map(|first| first.to_uppercase().chain(chars).collect())
        }
    }
}

/// A notification's words for an entry.
pub fn notice_text(entry: &MentionEntry) -> NoticeText {
    let name = if entry.sender_name.trim().is_empty() { entry.sender_login.clone() } else { entry.sender_name.clone() };
    let chat_title = match platform_label(&entry.platform) {
        Some(label) => format!("{} on {label}", entry.channel_login),
        None => entry.channel_login.clone(),
    };
    let (subtitle, what) = match entry.kind {
        MentionKind::Reply => ("Replied to you in", "replied to you in"),
        MentionKind::Mention => ("Mentioned you in", "mentioned you in"),
        MentionKind::Word => ("In", "in"),
    };
    NoticeText {
        subtitle: format!("{subtitle} {chat_title}"),
        headline: format!("{name} {what} {chat_title}"),
        quote: match entry.kind {
            MentionKind::Reply => entry.reply_parent_text.as_deref().and_then(quote_line),
            _ => None,
        },
        body: entry.text.clone(),
        title: name,
        chat_title,
    }
}

/// The face lookup for an entry's channel: Twitch by channel id, Kick by
/// slug, YouTube by channel id (what `channel_avatars` takes).
pub fn channel_avatar_request(entry: &MentionEntry) -> Option<crate::services::channel_avatars::AvatarRequest> {
    let id = match entry.platform.as_str() {
        "twitch" => entry.channel_id.clone().unwrap_or_default(),
        "kick" | "youtube" => entry.channel_login.clone(),
        _ => return None,
    };
    (!id.trim().is_empty()).then(|| crate::services::channel_avatars::AvatarRequest { provider: entry.platform.clone(), id })
}

/// The notice for an entry the core decided to announce: its words, and the
/// sender's and channel's faces when cached or found within `FACES_WAIT`.
pub async fn notice_for(entry: MentionEntry) -> MentionNotice {
    let requests: Vec<_> = [avatar_request(&entry), channel_avatar_request(&entry)].into_iter().flatten().collect();
    let faces = match crate::services::providers::app_handle() {
        Some(app) if !requests.is_empty() => {
            crate::services::channel_avatars::wait_for(&app, requests, FACES_WAIT).await
        }
        _ => HashMap::new(),
    };
    let sender_avatar = avatar_of(&entry, &faces);
    let channel_avatar = channel_avatar_request(&entry).and_then(|r| faces.get(&avatar_key(&r)).cloned());
    MentionNotice { notice: notice_text(&entry), entry, sender_avatar, channel_avatar }
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
    /// Other accounts' inbox keys already folded into this one since it loaded.
    merged: HashSet<String>,
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

/// The Twitch owner of the inbox: the signed-in Twitch account, known from
/// launch (and the account the mention server's pull reads for), or the
/// chat's own identity before the account is. Without one, `with_inbox`
/// keeps the first signed-in Kick, YouTube or TikTok account's inbox.
fn owner_id(account: Option<String>, chat: Option<(String, String)>) -> Option<String> {
    account
        .filter(|id| !id.is_empty())
        .or_else(|| chat.map(|(_, id)| id).filter(|id| !id.is_empty()))
}

fn twitch_owner() -> Option<String> {
    owner_id(crate::services::account_roster::twitch_user_id(), ChatRules::own_identity())
}

/// The state for a newly loaded owner: its stored inbox, with the rooms on
/// screen carried over (they did not change with the account).
fn next_state(old_visible: HashSet<String>, owner: String, mut inbox: Inbox) -> State {
    inbox.visible = old_visible;
    State { owner, inbox, dirty: false, merged: HashSet::new() }
}

/// Which other accounts' inboxes still have to be folded into `owner`'s.
fn pending_sources(owner: &str, merged: &HashSet<String>, sources: Vec<String>) -> Vec<String> {
    sources.into_iter().filter(|k| k != owner && !merged.contains(k)).collect()
}

/// File every source inbox's entries into `owner` (dedupe and tombstones
/// apply) and keep its tombstones. Answers the sources that held anything,
/// which the caller empties only once `owner` is safely written.
fn merge_sources(owner: &mut Inbox, sources: Vec<(String, Inbox)>, now_ms: i64) -> Vec<String> {
    let mut held = Vec::new();
    for (key, source) in sources {
        if source.entries.is_empty() && source.tombstones.is_empty() {
            continue;
        }
        for t in &source.tombstones {
            owner.tombstone(t);
        }
        for e in source.entries {
            owner.file(e, now_ms);
        }
        held.push(key);
    }
    held
}

/// Run `f` on the signed-in account's inbox: the Twitch account's, else the
/// first signed-in Kick, YouTube or TikTok account's. `f` returns whether it
/// changed anything. None when no account is known yet.
fn with_inbox<R>(f: impl FnOnce(&mut Inbox) -> (R, bool)) -> Option<R> {
    let sources = ChatRules::platform_owners();
    let owner = twitch_owner().or_else(|| sources.first().cloned())?;
    with_inbox_of(owner, sources, f)
}

/// The Twitch account's inbox only. The mention server pulls and answers
/// for Twitch accounts, so a session without one has nothing to say there.
fn with_twitch_inbox<R>(f: impl FnOnce(&mut Inbox) -> (R, bool)) -> Option<R> {
    let owner = twitch_owner()?;
    with_inbox_of(owner, ChatRules::platform_owners(), f)
}

/// `with_inbox` for a known owner, loading its inbox (and flushing the
/// previous owner's) when the owner changed.
fn with_inbox_of<R>(owner: String, sources: Vec<String>, f: impl FnOnce(&mut Inbox) -> (R, bool)) -> Option<R> {
    let (out, dirty, counts) = {
        let mut guard = state().lock().ok()?;
        let mut loaded = false;
        if guard.as_ref().map(|s| s.owner != owner).unwrap_or(true) {
            let mut visible = HashSet::new();
            if let Some(old) = guard.take() {
                if old.dirty {
                    let _ = write(&old.owner, &old.inbox.to_file());
                }
                visible = old.inbox.visible;
            }
            *guard = Some(next_state(visible, owner.clone(), load(&owner)));
            loaded = true;
        }
        let st = guard.as_mut()?;
        // Accounts signed in together are one person: mentions kept under a
        // Kick or YouTube account (before Twitch was known, or on another
        // platform) follow into this inbox, each key once per load.
        let pending = pending_sources(&st.owner, &st.merged, sources);
        let mut adopted = false;
        if !pending.is_empty() {
            let loaded_sources: Vec<(String, Inbox)> = pending.iter().map(|k| (k.clone(), load(k))).collect();
            let held = merge_sources(&mut st.inbox, loaded_sources, now_ms());
            if !held.is_empty() {
                adopted = true;
                // The new home is written before any source is emptied, so a
                // crash in between leaves a copy (refiling it adds nothing twice).
                match write(&st.owner, &st.inbox.to_file()) {
                    Ok(()) => {
                        for k in &held {
                            let _ = write(k, &InboxFile { version: 2, ..Default::default() });
                        }
                    }
                    Err(_) => st.dirty = true,
                }
            }
            st.merged.extend(pending);
        }
        let (out, changed) = f(&mut st.inbox);
        if changed {
            st.dirty = true;
        }
        // A newly loaded account is announced like a change: its counts replace
        // the old account's, and a list read before the account was known
        // (empty) reads again.
        let counts = (changed || loaded || adopted).then(|| (st.inbox.unread_by_room(), st.inbox.unread_total()));
        (out, st.dirty, counts)
    };
    if dirty {
        ensure_flush_task();
    }
    if let Some((by_room, total)) = counts {
        unread_tx().send_replace(Arc::new(by_room));
        if let Some(app) = crate::services::providers::app_handle() {
            let _ = app.emit(CHANGED_EVENT, serde_json::json!({ "unread_total": total }));
        }
        crate::services::inbox_counts::wake();
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
    // One task, off the chat lane: the faces get a moment to arrive (most are
    // cached), then every app's notice goes out (the phones' notification,
    // the desktop island's preview) and, on desktop, the system notification
    // (or MultiChat's card) for when the main window is not in use.
    #[cfg(desktop)]
    let msg = msg.clone();
    tauri::async_runtime::spawn(async move {
        let notice = notice_for(entry).await;
        if let Some(app) = crate::services::providers::app_handle() {
            let _ = app.emit(NOTIFY_EVENT, &notice);
        }
        #[cfg(desktop)]
        crate::services::mention_notify::deliver(&msg, &notice).await;
    });
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

pub fn page(before_ms: Option<i64>, limit: usize, unseen_only: bool) -> Option<(Vec<MentionEntry>, bool, u32)> {
    with_inbox(|inbox| {
        let (entries, more) = inbox.page(before_ms, limit, unseen_only);
        ((entries, more, inbox.unread_total()), false)
    })
}

/// One page of an inbox view: mention rows and whisper conversations merged
/// newest first (see `merge_view`), with each chip's unread count. The mention
/// side is read under the mention inbox's lock and the whisper side under the
/// whisper store's own, never both at once. A cursor that does not parse
/// answers an empty last page rather than page 1 again.
pub fn view_page(filter: InboxFilter, surface: Surface, cursor: Option<&str>, limit: usize) -> ViewPage {
    let cursor: ViewCursor = match cursor {
        None => ViewCursor::default(),
        Some(raw) => match serde_json::from_str(raw) {
            Ok(c) => c,
            Err(e) => {
                log::warn!("[MentionInbox] inbox view cursor did not parse: {e}");
                return ViewPage { rows: Vec::new(), has_more: false, cursor: None, chips: Vec::new() };
            }
        },
    };
    let twitch = crate::services::whisper_sync::current_scope().is_some();
    let read_mentions = filter.shows_mentions() && !cursor.m_done;
    let read_whispers = twitch && filter.shows_whispers() && !cursor.w_done;

    let mention_side = with_inbox(|inbox| {
        let rows = read_mentions.then(|| {
            let (rows, more, _) = inbox.view_on(filter, cursor.m, limit, surface.skips_seen_live());
            (rows, more)
        });
        let counts = [
            inbox.unread_for(InboxFilter::All),
            inbox.unread_for(InboxFilter::Mentions),
            inbox.unread_for(InboxFilter::Highlights),
        ];
        ((rows, counts), false)
    });
    let (mention_rows, mention_counts) = mention_side.unwrap_or((None, [0, 0, 0]));

    let whisper_rows = read_whispers
        .then(|| crate::services::whisper_inbox::rows_for_view(surface, cursor.w.clone(), limit + 1));
    let whisper_unread = if twitch { crate::services::whisper_inbox::unread_counts().0 } else { 0 };

    // A side the chip does not show is done from the start.
    let cursor = ViewCursor {
        m_done: cursor.m_done || !filter.shows_mentions(),
        w_done: cursor.w_done || !(twitch && filter.shows_whispers()),
        ..cursor
    };
    let (rows, has_more, next) = merge_view(mention_rows, whisper_rows, &cursor, limit);
    ViewPage {
        rows,
        has_more,
        cursor: next.and_then(|c| serde_json::to_string(&c).ok()),
        chips: chips_for(twitch, mention_counts, whisper_unread),
    }
}

/// The mention side of the inbox counts: every unread entry, and the ones
/// the desktop island lists. (0, 0) when no account is known.
pub fn badge_counts() -> (u32, u32) {
    with_inbox(|inbox| ((inbox.unread_total(), inbox.unread_unseen()), false)).unwrap_or((0, 0))
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
    with_twitch_inbox(|inbox| {
        let filed = inbox.merge_remote(remote, now_ms());
        if next > inbox.remote_cursor_ms {
            inbox.remote_cursor_ms = next;
        }
        (filed, filed > 0)
    })
}

/// How far the signed-in account has pulled from the mention server.
pub fn remote_cursor() -> Option<i64> {
    with_twitch_inbox(|inbox| (inbox.remote_cursor_ms, false))
}

/// Whether this app already holds the message (it caught it itself), or its
/// chat is on screen now: a push for it would only repeat what is in front of
/// the viewer. False when no Twitch account is signed in in this process.
pub fn caught_here(id: &str, room: &str) -> bool {
    let room = room.to_lowercase();
    with_twitch_inbox(|inbox| (inbox.get(id).is_some() || inbox.visible.contains(&room), false)).unwrap_or(false)
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
            reply_parent_text: None,
            at_ms,
            read: false,
            removed: false,
            parts: Vec::new(),
            seen_live: false,
        }
    }

    const NOW: i64 = 1_800_000_000_000;

    /// One case of the wording the app and the mention server share.
    #[derive(Deserialize)]
    struct NoticeCase {
        name: String,
        input: NoticeInput,
        want: NoticeWant,
    }

    #[derive(Deserialize)]
    struct NoticeInput {
        kind: MentionKind,
        platform: String,
        channel: String,
        sender_name: String,
        sender_login: String,
        text: String,
        parent: Option<String>,
    }

    #[derive(Deserialize, Debug, PartialEq)]
    struct NoticeWant {
        title: String,
        subtitle: String,
        headline: String,
        quote: Option<String>,
        body: String,
        chat_title: String,
    }

    #[test]
    fn notice_words_match_the_shared_fixtures() {
        let cases: Vec<NoticeCase> =
            serde_json::from_str(include_str!("../../crates/sn-mention-rule/tests/notice_fixtures.json")).unwrap();
        assert!(cases.len() >= 9);
        for c in cases {
            let mut e = entry("m1", &c.input.channel.to_lowercase(), NOW);
            e.platform = c.input.platform;
            e.channel_login = c.input.channel;
            e.sender_name = c.input.sender_name;
            e.sender_login = c.input.sender_login;
            e.text = c.input.text;
            e.kind = c.input.kind;
            e.reply_parent_text = c.input.parent.as_deref().and_then(parent_text);
            let got = notice_text(&e);
            let got = NoticeWant {
                title: got.title,
                subtitle: got.subtitle,
                headline: got.headline,
                quote: got.quote,
                body: got.body,
                chat_title: got.chat_title,
            };
            assert_eq!(got, c.want, "{}", c.name);
        }
    }

    #[test]
    fn the_notice_keeps_the_entry_text_beside_its_words() {
        let mut e = entry("m1", "bigstreamer", NOW);
        e.kind = MentionKind::Reply;
        e.reply_parent_text = Some("did you see that".into());
        let notice = MentionNotice {
            notice: notice_text(&e),
            entry: e,
            sender_avatar: Some("https://static-cdn.jtvnw.net/a.png".into()),
            channel_avatar: None,
        };
        let json = serde_json::to_value(&notice).unwrap();
        assert_eq!(json["text"], "hi @brandon", "the entry's own text stays a string");
        assert_eq!(json["id"], "m1");
        assert_eq!(json["notice"]["quote"], "\u{201C}did you see that\u{201D}");
        assert_eq!(json["notice"]["body"], "hi @brandon");
        assert_eq!(json["sender_avatar"], "https://static-cdn.jtvnw.net/a.png");
        assert!(json.get("channel_avatar").is_none());
        // Every field once: a duplicate key would read as the last one in JS.
        let raw = serde_json::to_string(&notice).unwrap();
        assert_eq!(raw.matches("\"text\":").count(), 1);
    }

    #[test]
    fn parent_text_is_one_bounded_line() {
        assert_eq!(parent_text("  a\n\nb  c "), Some("a b c".into()));
        assert_eq!(parent_text(" \n "), None);
        assert_eq!(parent_text(&"x".repeat(500)).map(|t| t.chars().count()), Some(MAX_PARENT));
    }

    #[test]
    fn a_reply_keeps_its_parent_words_and_a_mention_does_not() {
        let mut msg = ChatMessage {
            id: "r1".into(),
            user_id: "2".into(),
            username: "fan".into(),
            display_name: "Fan".into(),
            color: None,
            badges: vec![],
            timestamp: "0".into(),
            content: "agreed".into(),
            provider: "twitch".into(),
            channel: "#chan".into(),
            emotes: vec![],
            tags: HashMap::new(),
            layout: Default::default(),
            segments: vec![],
            metadata: Default::default(),
            received_at_ms: None,
        };
        msg.tags.insert("reply-parent-msg-body".into(), "hello\\schat".into());
        assert_eq!(entry_for(&msg, MentionKind::Reply, NOW).reply_parent_text.as_deref(), Some("hello chat"));
        assert_eq!(entry_for(&msg, MentionKind::Mention, NOW).reply_parent_text, None);
        // The parsed reply wins over the raw tag (it is where Kick puts it).
        msg.metadata.reply_info = Some(crate::models::chat_layout::ReplyInfo {
            parent_msg_id: "p".into(),
            parent_display_name: "Brandon".into(),
            parent_msg_body: "from the parse".into(),
            parent_user_id: "1".into(),
            parent_user_login: "brandon".into(),
            ..Default::default()
        });
        assert_eq!(entry_for(&msg, MentionKind::Reply, NOW).reply_parent_text.as_deref(), Some("from the parse"));
    }

    #[test]
    fn the_account_owns_the_inbox_before_any_chat() {
        let chat = Some(("brandon".to_string(), "999".to_string()));
        assert_eq!(owner_id(Some("42".into()), chat.clone()), Some("42".into()));
        assert_eq!(owner_id(None, chat.clone()), Some("999".into()));
        assert_eq!(owner_id(Some(String::new()), chat), Some("999".into()));
        assert_eq!(owner_id(None, Some(("brandon".into(), String::new()))), None);
        assert_eq!(owner_id(None, None), None);
    }

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
                local_path: None,
            },
            S::Emote {
                content: "cvHazmat".into(),
                emote_id: None,
                emote_url: "https://e/z".into(),
                is_zero_width: Some(true),
                modifier_flags: None,
                is_personal: None,
                local_path: None,
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
        let (first, more) = inbox.page(None, 2, false);
        assert_eq!(first.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["m4", "m3"]);
        assert!(more);
        let (rest, more) = inbox.page(Some(first[1].at_ms), 10, false);
        assert_eq!(rest.len(), 3);
        assert!(!more);
    }

    #[test]
    fn the_island_page_leaves_out_mentions_seen_as_they_happened() {
        let mut inbox = Inbox::default();
        inbox.file(entry("away", "xqc", NOW), NOW);
        inbox.set_visible(vec!["xqc".into()]);
        inbox.file(entry("watching", "xqc", NOW + 1), NOW);
        inbox.file(entry("docked", "kick:xqc", NOW + 2), NOW);
        // Opening the chat reads the one that came in while away; it stays.
        assert!(inbox.get("away").is_some_and(|e| e.read && !e.seen_live));
        let ids = |v: Vec<MentionEntry>| v.into_iter().map(|e| e.id).collect::<Vec<_>>();
        assert_eq!(ids(inbox.page(None, 10, true).0), ["docked", "away"]);
        assert_eq!(ids(inbox.page(None, 10, false).0), ["docked", "watching", "away"]);
    }

    #[test]
    fn the_island_view_leaves_out_mentions_seen_as_they_happened() {
        let mut inbox = Inbox::default();
        inbox.file(entry("away", "xqc", NOW), NOW);
        inbox.set_visible(vec!["xqc".into()]);
        // A different message, so the phones' view does not fold it into "away".
        let mut watching = entry("watching", "xqc", NOW + 1);
        watching.text = "back again @brandon".into();
        inbox.file(watching, NOW);
        inbox.file(entry("docked", "kick:xqc", NOW + 2), NOW);
        let ids = |rows: Vec<InboxRow>| rows.into_iter().map(|r| r.entry.id).collect::<Vec<_>>();
        let island = inbox.view_on(InboxFilter::All, None, 10, Surface::Island.skips_seen_live()).0;
        assert_eq!(ids(island), ["docked", "away"]);
        for surface in [Surface::Phone, Surface::Multichat] {
            let rows = inbox.view_on(InboxFilter::All, None, 10, surface.skips_seen_live()).0;
            assert_eq!(ids(rows), ["docked", "watching", "away"], "{surface:?}");
        }
        assert_eq!(inbox.unread_unseen(), 1);
    }

    fn wrow(key: &str, at_ms: i64) -> crate::services::whisper_inbox::RowMeta {
        crate::services::whisper_inbox::RowMeta {
            key: key.into(),
            user_id: key.into(),
            user_login: format!("user{key}"),
            user_name: format!("User{key}"),
            profile_image_url: None,
            color: None,
            badges: Vec::new(),
            last_message_timestamp: at_ms,
            last_text: "hello".into(),
            last_parts: Vec::new(),
            last_is_sent: false,
            unread_count: 0,
            archived: false,
            muted: false,
            sender_view: None,
            live: None,
        }
    }

    /// The whisper side as `rows_for_view` serves it: newest first by
    /// (time desc, key asc), strictly after `after`, at most `n`.
    fn whisper_side(
        all: &[crate::services::whisper_inbox::RowMeta],
        after: Option<(i64, String)>,
        n: usize,
    ) -> Vec<crate::services::whisper_inbox::RowMeta> {
        let mut rows = all.to_vec();
        rows.sort_by(|a, b| b.last_message_timestamp.cmp(&a.last_message_timestamp).then_with(|| a.key.cmp(&b.key)));
        rows.into_iter()
            .filter(|r| {
                after.as_ref().is_none_or(|(ts, key)| {
                    r.last_message_timestamp < *ts || (r.last_message_timestamp == *ts && r.key > *key)
                })
            })
            .take(n)
            .collect()
    }

    /// Page an All view to the end, the way `view_page` does, and answer the
    /// row ids in order.
    fn page_all(inbox: &Inbox, whispers: &[crate::services::whisper_inbox::RowMeta], limit: usize) -> Vec<String> {
        let mut cursor = ViewCursor::default();
        let mut seen = Vec::new();
        for _ in 0..50 {
            let m = (!cursor.m_done).then(|| {
                let (rows, more, _) = inbox.view_on(InboxFilter::All, cursor.m, limit, false);
                (rows, more)
            });
            let w = (!cursor.w_done).then(|| whisper_side(whispers, cursor.w.clone(), limit + 1));
            let (rows, more, next) = merge_view(m, w, &cursor, limit);
            seen.extend(rows.iter().map(|r| match r {
                InboxItem::Mention { row_id, .. } | InboxItem::Whisper { row_id, .. } => row_id.clone(),
            }));
            match (more, next) {
                (true, Some(n)) => cursor = n,
                _ => return seen,
            }
        }
        panic!("paging never ended");
    }

    #[test]
    fn view_all_never_splits_a_fold_or_skips_a_whisper() {
        // A mention row at 100 folding entries at 90 and 80, and a whisper
        // conversation at 85 that sorts between the fold's head and its tail.
        let mut inbox = Inbox::default();
        inbox.file(said("m80", "1", "spam", NOW + 80), NOW);
        inbox.file(said("m90", "1", "spam", NOW + 90), NOW);
        inbox.file(said("m100", "1", "spam", NOW + 100), NOW);
        inbox.file(said("old", "2", "older", NOW + 10), NOW);
        let whispers = vec![wrow("85", NOW + 85), wrow("50", NOW + 50), wrow("5", NOW + 5)];
        for limit in 1..=4 {
            assert_eq!(
                page_all(&inbox, &whispers, limit),
                ["m100", "w:85", "w:50", "old", "w:5"],
                "limit {limit}"
            );
        }
    }

    #[test]
    fn view_ties_keep_every_row_once() {
        let mut inbox = Inbox::default();
        inbox.file(said("m", "1", "same time", NOW + 50), NOW);
        let whispers = vec![wrow("b", NOW + 50), wrow("a", NOW + 50), wrow("c", NOW + 40)];
        for limit in 1..=3 {
            assert_eq!(page_all(&inbox, &whispers, limit), ["m", "w:a", "w:b", "w:c"], "limit {limit}");
        }
    }

    #[test]
    fn a_chip_that_shows_one_side_never_reads_the_other() {
        let mut inbox = Inbox::default();
        inbox.file(said("m", "1", "hi", NOW + 9), NOW);
        let cursor = ViewCursor { w_done: true, ..ViewCursor::default() };
        let (rows, more, _) = merge_view(Some((inbox.view(InboxFilter::Mentions, None, 5).0, false)), None, &cursor, 5);
        assert_eq!(rows.len(), 1);
        assert!(!more);
        let cursor = ViewCursor { m_done: true, ..ViewCursor::default() };
        let (rows, more, _) = merge_view(None, Some(vec![wrow("1", NOW)]), &cursor, 5);
        assert!(matches!(&rows[..], [InboxItem::Whisper { row_id, .. }] if row_id == "w:1"));
        assert!(!more);
    }

    #[test]
    fn a_cursor_survives_the_round_trip_as_a_string() {
        let c = ViewCursor { m: Some(NOW), m_done: false, w: Some((NOW - 1, "42".into())), w_done: true };
        let raw = serde_json::to_string(&c).unwrap();
        assert_eq!(serde_json::from_str::<ViewCursor>(&raw).unwrap(), c);
        assert!(serde_json::from_str::<ViewCursor>("not json").is_err());
    }

    #[test]
    fn chips_omit_whispers_without_twitch() {
        assert_eq!(
            chips_for(false, [3, 2, 1], 7),
            vec![(InboxFilter::All, 3), (InboxFilter::Mentions, 2), (InboxFilter::Highlights, 1)]
        );
        assert_eq!(
            chips_for(true, [3, 2, 1], 7),
            vec![
                (InboxFilter::All, 10),
                (InboxFilter::Mentions, 2),
                (InboxFilter::Highlights, 1),
                (InboxFilter::Whispers, 7)
            ]
        );
    }

    #[test]
    fn surfaces_and_filters_serialize_as_the_shells_send_them() {
        assert_eq!(serde_json::to_string(&Surface::Island).unwrap(), "\"island\"");
        assert_eq!(serde_json::from_str::<Surface>("\"multichat\"").unwrap(), Surface::Multichat);
        assert_eq!(serde_json::from_str::<InboxFilter>("\"whispers\"").unwrap(), InboxFilter::Whispers);
        assert_eq!(InboxFilter::Whispers.id(), "whispers");
        assert!(!InboxFilter::Whispers.admits(MentionKind::Mention));
        let item = InboxItem::Whisper { row_id: "w:9".into(), whisper: wrow("9", NOW) };
        let json = serde_json::to_value(&item).unwrap();
        assert_eq!(json["kind"], "whisper");
        assert_eq!(json["whisper"]["key"], "9");
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
    fn a_ping_is_a_mention_a_reply_or_a_phrase_while_phrases_count() {
        assert!(pinged_with(&verdict(false, false, true), true));
        assert!(!pinged_with(&verdict(false, false, true), false));
        assert!(pinged_with(&verdict(true, false, false), false));
        assert!(pinged_with(&verdict(false, true, false), false));
        assert!(!pinged_with(&verdict(false, false, false), true));
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

    #[test]
    fn merging_moves_entries_once_and_keeps_tombstones() {
        let mut kick = Inbox::default();
        let mut e = entry("k1", "kick:xqc", NOW);
        e.platform = "kick".into();
        assert!(kick.file(e.clone(), NOW));
        kick.tombstone("gone");
        let mut twitch = Inbox::default();
        let held = merge_sources(&mut twitch, vec![("kick-5550001".into(), kick), ("kick-name-x".into(), Inbox::default())], NOW);
        assert_eq!(held, vec!["kick-5550001".to_string()]);
        assert_eq!(twitch.unread_total(), 1);
        assert!(twitch.tombstones.iter().any(|t| t == "gone"));
        // The same entry again (a source not yet emptied) files nothing twice.
        let again = Inbox { entries: vec![e], ..Default::default() };
        merge_sources(&mut twitch, vec![("kick-5550001".into(), again)], NOW);
        assert_eq!(twitch.unread_total(), 1);
    }

    #[test]
    fn a_removed_mention_stays_removed_after_a_merge() {
        let mut source = Inbox::default();
        let mut e = entry("k2", "kick:xqc", NOW);
        e.platform = "kick".into();
        source.file(e, NOW);
        let mut owner = Inbox::default();
        owner.tombstone("k2");
        merge_sources(&mut owner, vec![("kick-5550001".into(), source)], NOW);
        assert_eq!(owner.unread_total(), 0);
    }

    #[test]
    fn every_other_signed_in_account_is_folded_in_once() {
        let mut merged = HashSet::new();
        // Kick kept mentions before Twitch was known; a later boot knows Twitch first.
        let sources = vec!["kick-5550001".to_string(), "kick-name-x".to_string(), "youtube-UCme".to_string()];
        assert_eq!(pending_sources("12345", &merged, sources.clone()), sources);
        merged.extend(sources.clone());
        assert!(pending_sources("12345", &merged, sources.clone()).is_empty());
        // A YouTube-only account that signs in to Kick: the owner becomes Kick,
        // and the YouTube inbox is folded in, not hidden.
        let pending = pending_sources("kick-5550001", &HashSet::new(), sources);
        assert_eq!(pending, vec!["kick-name-x".to_string(), "youtube-UCme".to_string()]);
    }

    #[test]
    fn the_rooms_on_screen_survive_an_owner_switch() {
        let visible: HashSet<String> = ["kick:xqc".to_string()].into_iter().collect();
        let st = next_state(visible, "kick-5550001".into(), Inbox::default());
        assert!(st.inbox.visible.contains("kick:xqc"));
        assert!(!st.dirty && st.merged.is_empty());
    }
}
