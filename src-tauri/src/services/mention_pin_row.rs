//! The row a pinned mention is drawn as (SN-284).
//!
//! A pin shows the message as its chat drew it: the same words and emotes, the
//! author's badges, name color and cosmetics, whichever chat it came from and
//! whether or not this window ever joined that chat. This module owns that row.
//!
//! - **The snapshot** (`snapshot_of`): when a pin is filed live, the filed
//!   `ChatMessage` is kept beside the inbox entries (`Inbox::pin_rows`, never on
//!   a wire) with every tag. A Twitch message keeps the IRC line it was parsed
//!   from next to it, so its words are cut again with the emotes of the chat it
//!   was said in; Kick, YouTube and TikTok keep their segments. Over 8 KB the
//!   parent's segments go first (Twitch re-cuts them from the line), and a
//!   snapshot that still does not fit is not kept: that pin shows its text row.
//! - **The row** (`build_row`): the snapshot with its segments cut again from
//!   the line (the joined chat's emote table, else a private per-chat table that
//!   lives only while the chat holds a live pin), its badges carrying Twitch's
//!   art in three sizes and a title (`badges_resolved`), and the row view
//!   stamped with link previews off. Nothing here teaches the live chat
//!   anything (no spaced-name memory, no re-stamp queue, no emote cache write).
//! - **The cache** (`cached`, `refresh_rows`): rows are kept by pin id with
//!   their hash; the answer reads the cache and never builds. A pin is absent
//!   from the answer until its row is cached; after 10 s with no complete row
//!   it shows the text row. While a pin is live and a surface shows it, every
//!   row is built again every 2 s and the pins revision moves only when a row's
//!   hash did.
//! - **Late data**: a chat's badge set (one fetch in flight at a time, an empty
//!   answer final, a failure retried after 30 s, 2 min, then 10 min) and a
//!   private emote table are fetched once and the pins that waited for them are
//!   built again.

use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::models::chat_layout::ChatMessage;
use crate::services::irc_service::{CheermoteSet, EmoteLookup, IrcService, ParseSnapshots};
use crate::services::mention_inbox::{MentionEntry, MentionPart, PinRun};

/// A snapshot past this many bytes is made lean, and one still past it is not kept.
pub const SNAPSHOT_CAP: usize = 8 * 1024;
/// A pin with no complete row this long after it was first asked for shows the text row.
pub const FALLBACK_AFTER: Duration = Duration::from_secs(10);
/// Rows are built again at most this often.
pub const REFRESH_EVERY: Duration = Duration::from_secs(2);
/// A private emote table lingers this long after the last pin of its chat ended.
const TABLE_LINGER: Duration = Duration::from_secs(60);
/// Private tables kept at most (one per chat with a live pin is at most 25).
const MAX_TABLES: usize = 25;
/// Failed badge reads are tried again after these waits; the last repeats.
const BADGE_RETRY: [Duration; 3] = [Duration::from_secs(30), Duration::from_secs(120), Duration::from_secs(600)];

/// The tags a remote snapshot leaves out: neither renderer reads them.
pub const DROPPED_TAGS: [&str; 2] = ["client-nonce", "flags"];

/// One built row: the `ChatMessage`-shaped JSON a shell draws, and its hash
/// (`rev`), which changes exactly when the drawn row would.
#[derive(Clone, Debug, PartialEq)]
pub struct PinRow {
    pub row: serde_json::Value,
    pub rev: u64,
}

/// The hash of a built row.
pub fn row_hash(row: &serde_json::Value) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    row.to_string().hash(&mut h);
    h.finish()
}

// ---------------------------------------------------------------------------
// The snapshot
// ---------------------------------------------------------------------------

/// What `Inbox::pin_rows` holds for a pin.
#[derive(Serialize, Deserialize, Clone, Debug)]
struct Snap {
    v: u8,
    msg: ChatMessage,
    /// The IRC line a Twitch PRIVMSG was parsed from (or, for a remote row,
    /// rebuilt from the server's raw body and tags).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    raw: Option<String>,
    /// The segments were left out: the line cuts them again.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    lean: bool,
}

fn encode(snap: &Snap) -> Option<Box<RawValue>> {
    serde_json::value::to_raw_value(snap).ok()
}

fn is_twitch(msg: &ChatMessage) -> bool {
    msg.provider.is_empty() || msg.provider == "twitch"
}

/// The filed message as raw JSON for `Inbox::pin_rows`: every tag kept,
/// segments intact, `raw` (the IRC line) beside it for a Twitch PRIVMSG.
/// Lean over 8 KB; None when even that does not fit (the pin shows its text row).
pub fn snapshot_of(msg: &ChatMessage, raw: Option<&str>) -> Option<Box<RawValue>> {
    let mut kept = msg.clone();
    kept.layout = Default::default();
    kept.metadata.view = None;
    // Art that is a data URI is rebuilt by name when the row is drawn.
    for badge in &mut kept.badges {
        for url in [&mut badge.image_url_1x, &mut badge.image_url_2x, &mut badge.image_url_4x] {
            if url.as_deref().is_some_and(|u| u.starts_with("data:")) {
                *url = None;
            }
        }
    }
    let line = raw
        .filter(|line| is_twitch(msg) && crate::services::irc_service::irc_command(line) == Some("PRIVMSG"))
        .map(str::to_string);
    let mut snap = Snap { v: 1, msg: kept, raw: line, lean: false };
    let fits = |snap: &Snap| encode(snap).filter(|r| r.get().len() <= SNAPSHOT_CAP);
    if let Some(ok) = fits(&snap) {
        return Some(ok);
    }
    // The parent's words go first (the line cuts them again for Twitch).
    if let Some(reply) = snap.msg.metadata.reply_info.as_mut() {
        reply.parent_segments = None;
    }
    if let Some(ok) = fits(&snap) {
        return Some(ok);
    }
    if snap.raw.is_some() {
        snap.msg.segments.clear();
        snap.lean = true;
        return fits(&snap);
    }
    None
}

// ---------------------------------------------------------------------------
// The owner: cache, tables, badge book
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct Cached {
    row: serde_json::Value,
    hash: u64,
    /// Built from a snapshot; false for the text row.
    built: bool,
    /// Built with nothing missing (a built row stored after the 10 s may not be).
    complete: bool,
}

/// What the cache holds after a build lands: a complete row is never replaced
/// by an incomplete one, anything else is replaced when its hash differs (so a
/// row stored incomplete at the 10 s improves as late data arrives). Answers
/// the entry to store and whether the drawn row changed.
fn merged(existing: Option<&Cached>, row: PinRow, complete: bool) -> (Option<Cached>, bool) {
    match existing {
        Some(c) if c.built && c.complete && !complete => (None, false),
        Some(c) if c.built && c.hash == row.rev => {
            (Some(Cached { row: row.row, hash: row.rev, built: true, complete: complete || c.complete }), false)
        }
        _ => (Some(Cached { row: row.row, hash: row.rev, built: true, complete }), true),
    }
}

impl Cached {
    fn pin_row(&self) -> PinRow {
        PinRow { row: self.row.clone(), rev: self.hash }
    }
}

struct Table {
    lookup: Arc<EmoteLookup>,
    at: Instant,
}

/// How a chat's badge set stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BadgeState {
    /// A read is out; `fails` counts the failed ones before it.
    Fetching { fails: u32 },
    /// Answered, empty or not: final.
    Answered,
    Failed { fails: u32, retry_at: Instant },
}

/// The answered-chats record: one read per chat, an empty answer final, a
/// failure retried on a schedule. Pure over the clock it is handed.
#[derive(Default, Debug)]
pub struct BadgeBook {
    rooms: HashMap<String, BadgeState>,
}

impl BadgeBook {
    /// Whether to start a read of `room` now (`""` is the global set).
    pub fn should_fetch(&self, room: &str, now: Instant) -> bool {
        match self.rooms.get(room) {
            None => true,
            Some(BadgeState::Fetching { .. } | BadgeState::Answered) => false,
            Some(BadgeState::Failed { retry_at, .. }) => now >= *retry_at,
        }
    }

    pub fn start(&mut self, room: &str) {
        let fails = match self.rooms.get(room) {
            Some(BadgeState::Failed { fails, .. } | BadgeState::Fetching { fails }) => *fails,
            _ => 0,
        };
        self.rooms.insert(room.to_string(), BadgeState::Fetching { fails });
    }

    pub fn finish(&mut self, room: &str, ok: bool, now: Instant) {
        if ok {
            self.rooms.insert(room.to_string(), BadgeState::Answered);
            return;
        }
        let fails = match self.rooms.get(room) {
            Some(BadgeState::Failed { fails, .. } | BadgeState::Fetching { fails }) => fails + 1,
            _ => 1,
        };
        let wait = BADGE_RETRY[(fails as usize - 1).min(BADGE_RETRY.len() - 1)];
        self.rooms.insert(room.to_string(), BadgeState::Failed { fails, retry_at: now + wait });
    }

    pub fn answered(&self, room: &str) -> bool {
        matches!(self.rooms.get(room), Some(BadgeState::Answered))
    }

    /// The held answer is gone (a table dropped with its chat): ask again when needed.
    pub fn forget(&mut self, room: &str) {
        if matches!(self.rooms.get(room), Some(BadgeState::Answered)) {
            self.rooms.remove(room);
        }
    }
}

#[derive(Default)]
struct Owner {
    rows: HashMap<String, Cached>,
    /// When each live pin was first asked for (the 10 s fallback).
    seen: HashMap<String, Instant>,
    /// The live pins' entries as last seen, so late data can rebuild them.
    entries: HashMap<String, MentionEntry>,
    building: HashSet<String>,
    tables: HashMap<String, Table>,
    table_inflight: HashSet<String>,
    cheers: HashMap<String, Arc<CheermoteSet>>,
    cheer_inflight: HashSet<String>,
    badges: BadgeBook,
    /// Per chat login, like the badge sets: one read in flight, a failure retried
    /// after 30 s, 2 min, then 10 min, so a failing fetch never loops with the rebuild.
    table_book: BadgeBook,
    cheer_book: BadgeBook,
    badge_queue: VecDeque<String>,
    badge_worker: bool,
    last_refresh: Option<Instant>,
}

static OWNER: once_cell::sync::Lazy<Mutex<Owner>> = once_cell::sync::Lazy::new(|| Mutex::new(Owner::default()));

fn owner() -> std::sync::MutexGuard<'static, Owner> {
    OWNER.lock().unwrap_or_else(|e| e.into_inner())
}

/// What a build found missing: each is fetched once (single in flight) and the
/// pins waiting for it are built again when it lands.
#[derive(Default, Debug, PartialEq, Eq, Clone)]
pub struct Needs {
    /// `(login, room id)` of a chat whose emote table is not held.
    pub table: Option<(String, String)>,
    /// `(login, room id)` of a chat whose cheermotes a bits message needs.
    pub cheermotes: Option<(String, String)>,
    /// Chats whose badge set has not been answered.
    pub badge_rooms: Vec<String>,
    pub global_badges: bool,
}

impl Needs {
    pub fn is_empty(&self) -> bool {
        self.table.is_none() && self.cheermotes.is_none() && self.badge_rooms.is_empty() && !self.global_badges
    }
}

// ---------------------------------------------------------------------------
// The cache the answer reads
// ---------------------------------------------------------------------------

/// The built row for a live pin, or `None` until it exists. A pin with no
/// snapshot shows its text row at once; one with a snapshot starts building and
/// is absent from the answer until the row is cached, or 10 s pass.
pub fn cached(entry: &MentionEntry) -> Option<PinRow> {
    let now = Instant::now();
    let first = {
        let mut o = owner();
        if let Some(c) = o.rows.get(&entry.id) {
            return Some(c.pin_row());
        }
        o.entries.insert(entry.id.clone(), entry.clone());
        *o.seen.entry(entry.id.clone()).or_insert(now)
    };
    // The inbox is read with the owner's lock released (lock order: inbox, then pins).
    let has_snapshot = crate::services::mention_inbox::pin_row_of(&entry.id).is_some();
    if !has_snapshot {
        return Some(store_fallback(entry));
    }
    spawn_build(entry.clone());
    if now.duration_since(first) >= FALLBACK_AFTER {
        return Some(store_fallback(entry));
    }
    None
}

fn store_fallback(entry: &MentionEntry) -> PinRow {
    let row = text_row(entry);
    owner().rows.entry(entry.id.clone()).or_insert(Cached { row: row.row.clone(), hash: row.rev, built: false, complete: false });
    row
}

/// Called by the pins tick (every second while a live pin exists): keeps the
/// cache to the live pins, and at most every 2 s, while a surface shows pins,
/// builds every live row again. Answers false: a row that changed moves the
/// pins revision itself when its build lands (`mention_pins::rows_changed`).
pub fn refresh_rows(runs: &[PinRun]) -> bool {
    retain_live(runs);
    if runs.is_empty() || !crate::services::mention_pins::any_surface_shown() {
        return false;
    }
    let now = Instant::now();
    {
        let mut o = owner();
        if o.last_refresh.is_some_and(|at| now.duration_since(at) < REFRESH_EVERY) {
            return false;
        }
        o.last_refresh = Some(now);
        for run in runs {
            o.entries.insert(run.id.clone(), run.entry.clone());
            o.seen.entry(run.id.clone()).or_insert(now);
        }
    }
    for run in runs {
        spawn_build(run.entry.clone());
    }
    false
}

/// Forget what no live pin needs: rows, first-seen times, entries, and private
/// tables of chats with no live pin (kept a minute past the last).
pub fn retain_live(runs: &[PinRun]) {
    let live: HashSet<&str> = runs.iter().map(|r| r.id.as_str()).collect();
    let rooms: HashSet<String> = runs.iter().map(|r| r.entry.channel_login.to_lowercase()).collect();
    let now = Instant::now();
    let mut o = owner();
    o.rows.retain(|id, _| live.contains(id.as_str()));
    o.seen.retain(|id, _| live.contains(id.as_str()));
    o.entries.retain(|id, _| live.contains(id.as_str()));
    let dropped: Vec<String> =
        o.tables.iter().filter(|(login, t)| !rooms.contains(*login) && now.duration_since(t.at) >= TABLE_LINGER).map(|(l, _)| l.clone()).collect();
    for login in dropped {
        o.tables.remove(&login);
        o.table_book.forget(&login);
    }
    let gone: Vec<String> = o.cheers.keys().filter(|l| !rooms.contains(*l)).cloned().collect();
    for login in gone {
        o.cheers.remove(&login);
        o.cheer_book.forget(&login);
    }
}

// ---------------------------------------------------------------------------
// Building
// ---------------------------------------------------------------------------

fn spawn_build(entry: MentionEntry) {
    {
        let mut o = owner();
        if !o.building.insert(entry.id.clone()) {
            return;
        }
    }
    tauri::async_runtime::spawn(async move {
        build_and_store(&entry).await;
        owner().building.remove(&entry.id);
    });
}

/// Builds one pin's row and stores it when it is complete (or good enough
/// after the 10 s); starts what it found missing.
async fn build_and_store(entry: &MentionEntry) {
    let id = entry.id.clone();
    let raw = {
        let id = id.clone();
        tokio::task::spawn_blocking(move || crate::services::mention_inbox::pin_row_of(&id)).await.ok().flatten()
    };
    let Some(raw) = raw else { return };
    let Ok(snap) = serde_json::from_str::<Snap>(raw.get()) else {
        // A snapshot that no longer decodes costs the pin its decoration only.
        store_built(entry, text_row(entry), false);
        return;
    };
    let view = crate::services::chat_row_view::enabled();
    let (row, needs) = build_row(entry, snap, view);
    start_fetches(&needs);
    let first = owner().seen.get(&id).copied().unwrap_or_else(Instant::now);
    let complete = needs.is_empty();
    let late = first.elapsed() >= FALLBACK_AFTER;
    if complete || late {
        // A complete row replaces whatever is there; an incomplete one (after
        // the 10 s) a fallback or another incomplete row that differs, never a
        // complete one.
        store_built(entry, row, complete);
    }
}

fn store_built(entry: &MentionEntry, row: PinRow, complete: bool) {
    let changed = {
        let mut o = owner();
        let (next, changed) = merged(o.rows.get(&entry.id), row, complete);
        if let Some(next) = next {
            o.rows.insert(entry.id.clone(), next);
        }
        changed
    };
    if changed {
        crate::services::mention_pins::rows_changed();
    }
}

/// The room a Twitch row's badges come from: the Shared Chat partner's when
/// the row was sent there, else the chat's own.
fn badge_room(msg: &ChatMessage) -> Option<String> {
    let tag = |name: &str| msg.tags.get(name).map(String::as_str).filter(|v| !v.is_empty());
    msg.metadata
        .source_room_id
        .as_deref()
        .filter(|v| !v.is_empty())
        .or_else(|| tag("source-room-id"))
        .or_else(|| tag("room-id"))
        .map(str::to_string)
}

fn tag_value<'a>(raw: &'a str, name: &str) -> Option<&'a str> {
    let tags = raw.strip_prefix('@')?.split_once(' ')?.0;
    tags.split(';').find_map(|t| t.strip_prefix(name)?.strip_prefix('='))
}

/// The row for `entry` from its snapshot, and what it found missing. `view`
/// stamps the native row view (the iPhone's); the web draws the row itself.
fn build_row(entry: &MentionEntry, snap: Snap, view: bool) -> (PinRow, Needs) {
    let mut needs = Needs::default();
    let Snap { mut msg, raw, .. } = snap;
    let mut home: Option<Arc<EmoteLookup>> = None;
    msg.layout = Default::default();
    msg.metadata.view = None;
    if msg.color.is_none() {
        msg.color = entry.raw_color.clone();
    }
    if is_twitch(&msg) {
        if let Some(line) = raw.as_deref() {
            home = recut_twitch(&mut msg, line, &mut needs);
        }
        resolve_twitch_badges(&mut msg, &mut needs);
    }
    crate::services::bidi_text::balance_row(&mut msg);
    if view {
        // Emote sizes come from the table the words were cut with (the live
        // row reads the joined chat's, which a pin's chat may not be).
        crate::services::irc_service::with_size_table(home, || {
            crate::services::chat_row_view::stamp_view_with(
                &mut msg,
                crate::services::chat_row_view::StampOverride { link_previews: Some(false), record_seen: Some(false) },
            )
        });
    }
    let row = serde_json::to_value(&msg).unwrap_or_else(|_| text_row(entry).row);
    let rev = row_hash(&row);
    (PinRow { row, rev }, needs)
}

/// The words and emotes of a Twitch row cut again from its line, with the
/// table of the chat it was said in; tags, color, badges and metadata stay the
/// snapshot's.
fn recut_twitch(msg: &mut ChatMessage, line: &str, needs: &mut Needs) -> Option<Arc<EmoteLookup>> {
    let login = crate::services::irc_service::extract_channel_from_irc_line(line)?;
    let room_id = tag_value(line, "room-id").unwrap_or_default().to_string();
    let source = tag_value(line, "source-room-id").filter(|s| !s.is_empty() && *s != room_id);
    let partner = source.and_then(crate::services::shared_chat_emotes::peek_lookup);
    let (own, cheers) = {
        let o = owner();
        (o.tables.get(&login).map(|t| t.lookup.clone()), o.cheers.get(&login).cloned())
    };
    let home = crate::services::irc_service::parse_lookup_for(&login).or(own);
    let channel = partner.or_else(|| home.clone());
    if channel.is_none() && !room_id.is_empty() {
        needs.table = Some((login.clone(), room_id.clone()));
    }
    let bits = tag_value(line, "bits").and_then(|b| b.parse::<u32>().ok()).is_some_and(|b| b > 0);
    let cheermotes = crate::services::irc_service::channel_cheermote_set(&login).or(cheers);
    if bits && cheermotes.is_none() && !room_id.is_empty() {
        needs.cheermotes = Some((login.clone(), room_id.clone()));
    }
    let snaps = ParseSnapshots::for_pin(channel, tag_value(line, "user-id").unwrap_or_default(), cheermotes);
    let fresh = IrcService::parse_privmsg_with(line, Some(&snaps))?;
    msg.segments = fresh.segments;
    if let (Some(mine), Some(theirs)) = (msg.metadata.reply_info.as_mut(), fresh.metadata.reply_info) {
        mine.parent_segments = theirs.parent_segments;
    }
    home
}

/// Twitch badge art in all three sizes and a title, from the chat's own set
/// (or the Shared Chat partner's) and the global one; `badges_resolved` only
/// when both are answered.
fn resolve_twitch_badges(msg: &mut ChatMessage, needs: &mut Needs) {
    use crate::services::chat_row_view as view;
    let room = badge_room(msg);
    for badge in &mut msg.badges {
        // A partner row's badge already wears its room's 2x art in all three slots
        // (the parse did that); the index's real sizes replace it when it has them.
        if let Some(art) = view::twitch_badge_sizes(room.as_deref(), &badge.name, &badge.version) {
            badge.image_url_1x = Some(art.x1);
            badge.image_url_2x = Some(art.x2);
            badge.image_url_4x = Some(art.x4);
            if !art.title.is_empty() {
                badge.title = Some(art.title);
            }
        }
    }
    if let Some(room) = room.filter(|r| !r.is_empty()) {
        if !room_answered(&room) {
            needs.badge_rooms.push(room);
        }
    }
    if !view::has_global_badges() {
        needs.global_badges = true;
    }
    msg.badges_resolved = needs.badge_rooms.is_empty() && !needs.global_badges;
}

fn room_answered(room: &str) -> bool {
    crate::services::chat_row_view::has_room_badges(room) || owner().badges.answered(room)
}

/// The fallback row: the entry's own words and emotes, drawn as chat would
/// with nothing decorated.
pub fn text_row(entry: &MentionEntry) -> PinRow {
    let segments: Vec<serde_json::Value> = if entry.parts.is_empty() {
        vec![serde_json::json!({ "type": "text", "content": entry.text })]
    } else {
        entry
            .parts
            .iter()
            .map(|p| match p {
                MentionPart::Text { text } => serde_json::json!({ "type": "text", "content": text }),
                MentionPart::Emote { name, url } => {
                    serde_json::json!({ "type": "emote", "content": name, "emote_url": url })
                }
            })
            .collect()
    };
    let mut tags = serde_json::Map::new();
    tags.insert("id".into(), entry.id.clone().into());
    tags.insert("user-id".into(), entry.sender_id.clone().into());
    tags.insert("display-name".into(), entry.sender_name.clone().into());
    tags.insert("tmi-sent-ts".into(), entry.at_ms.to_string().into());
    if let Some(room_id) = &entry.channel_id {
        tags.insert("room-id".into(), room_id.clone().into());
    }
    let row = serde_json::json!({
        "id": entry.id,
        "user_id": entry.sender_id,
        "username": entry.sender_login,
        "display_name": entry.sender_name,
        "color": entry.raw_color.clone().or_else(|| entry.sender_color.clone()),
        "badges": [],
        "timestamp": "",
        "content": entry.text,
        "provider": entry.platform,
        "channel": entry.channel_login,
        "tags": tags,
        "segments": segments,
        "metadata": {},
    });
    let rev = row_hash(&row);
    PinRow { row, rev }
}

// ---------------------------------------------------------------------------
// Late data: badge sets, tables, cheermotes (single in flight each)
// ---------------------------------------------------------------------------

fn start_fetches(needs: &Needs) {
    for room in &needs.badge_rooms {
        request_badges(room);
    }
    if needs.global_badges {
        request_badges("");
    }
    if let Some((login, room_id)) = &needs.table {
        request_table(login, room_id);
    }
    if let Some((login, room_id)) = &needs.cheermotes {
        request_cheermotes(login, room_id);
    }
}

/// Everything waiting on late data is built again.
fn rebuild_all() {
    let entries: Vec<MentionEntry> = owner().entries.values().cloned().collect();
    for entry in entries {
        spawn_build(entry);
    }
}

fn request_badges(room: &str) {
    let start_worker = {
        let mut o = owner();
        if !o.badges.should_fetch(room, Instant::now()) {
            return;
        }
        o.badges.start(room);
        o.badge_queue.push_back(room.to_string());
        !std::mem::replace(&mut o.badge_worker, true)
    };
    if start_worker {
        tauri::async_runtime::spawn(badge_worker());
    }
}

/// Badge reads, one at a time.
async fn badge_worker() {
    loop {
        let next = {
            let mut o = owner();
            let next = o.badge_queue.pop_front();
            if next.is_none() {
                o.badge_worker = false;
            }
            next
        };
        let Some(room) = next else { return };
        let ok = if room.is_empty() {
            crate::commands::badges::fetch_global_badges().await.is_ok()
        } else {
            crate::commands::badges::fetch_channel_badges(room.clone()).await.is_ok()
        };
        owner().badges.finish(&room, ok, Instant::now());
        rebuild_all();
    }
}

async fn app_emote_service() -> crate::services::emote_service::EmoteService {
    use tauri::Manager;
    if let Some(app) = crate::services::providers::app_handle() {
        if let Some(state) = app.try_state::<crate::commands::emotes::EmoteServiceState>() {
            return state.0.read().await.clone();
        }
    }
    crate::services::emote_service::EmoteService::new()
}

/// The private table to drop when the cap is reached: the oldest of a chat with
/// no live pin (kept only for the linger), else the oldest of all.
fn eviction_victim(tables: &HashMap<String, Table>, live_rooms: &HashSet<String>) -> Option<String> {
    tables
        .iter()
        .min_by_key(|(login, t)| (live_rooms.contains(*login), t.at))
        .map(|(login, _)| login.clone())
}

/// Whether to start a private-table read of `login` now: none in flight, and
/// the chat's record allows it (a failure waits out its 30 s / 2 min / 10 min).
fn begin_table(login: &str) -> bool {
    let mut o = owner();
    if !o.table_book.should_fetch(login, Instant::now()) || !o.table_inflight.insert(login.to_string()) {
        return false;
    }
    o.table_book.start(login);
    true
}

/// The same gate for a chat's cheermote set.
fn begin_cheermotes(login: &str) -> bool {
    let mut o = owner();
    if !o.cheer_book.should_fetch(login, Instant::now()) || !o.cheer_inflight.insert(login.to_string()) {
        return false;
    }
    o.cheer_book.start(login);
    true
}

fn request_table(login: &str, room_id: &str) {
    if !begin_table(login) {
        return;
    }
    let (login, room_id) = (login.to_string(), room_id.to_string());
    tauri::async_runtime::spawn(async move {
        let table = fetch_table(&login, &room_id).await;
        {
            let mut o = owner();
            o.table_inflight.remove(&login);
            o.table_book.finish(&login, table.is_some(), Instant::now());
            if let Some(lookup) = table {
                if o.tables.len() >= MAX_TABLES {
                    let live: HashSet<String> = o.entries.values().map(|e| e.channel_login.to_lowercase()).collect();
                    if let Some(victim) = eviction_victim(&o.tables, &live) {
                        o.tables.remove(&victim);
                        o.table_book.forget(&victim);
                    }
                }
                o.tables.insert(login.clone(), Table { lookup, at: Instant::now() });
            }
        }
        rebuild_all();
    });
}

/// A chat's emote table for a pin: the saved dictionary, else the providers.
/// Nothing is written to the emote caches.
async fn fetch_table(login: &str, room_id: &str) -> Option<Arc<EmoteLookup>> {
    let service = app_emote_service().await;
    let set = tokio::time::timeout(
        Duration::from_secs(15),
        service.fetch_channel_emotes_uncached(Some(login.to_string()), Some(room_id.to_string())),
    )
    .await
    .ok()
    .flatten()?;
    Some(EmoteLookup::build(&set))
}

fn request_cheermotes(login: &str, room_id: &str) {
    if !begin_cheermotes(login) {
        return;
    }
    let (login, room_id) = (login.to_string(), room_id.to_string());
    tauri::async_runtime::spawn(async move {
        let set = IrcService::fetch_cheermote_set(&login, &room_id).await;
        {
            let mut o = owner();
            o.cheer_inflight.remove(&login);
            o.cheer_book.finish(&login, set.is_some(), Instant::now());
            if let Some(set) = set {
                o.cheers.insert(login.clone(), Arc::new(set));
            }
        }
        rebuild_all();
    });
}

// ---------------------------------------------------------------------------
// Remote rows (a mention the server found)
// ---------------------------------------------------------------------------

/// What the server's watcher sent for a mention beyond its text (slice B).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RemoteRaw {
    /// The PRIVMSG's trailing text exactly as Twitch sent it (a `/me` still
    /// wrapped in its `\x01ACTION` marks).
    pub body_raw: String,
    /// The escaped tag section, with or without its leading `@`.
    pub tags_raw: String,
    /// The body is a `/me` and the wrapper was cut off it.
    pub action: bool,
}

fn escape_tag(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            ';' => out.push_str("\\:"),
            ' ' => out.push_str("\\s"),
            '\\' => out.push_str("\\\\"),
            '\r' => out.push_str("\\r"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out
}

/// The IRC line a remote row is parsed from: the server's tags (without the
/// two no renderer reads) and body around the entry's login and chat. A tag
/// the server did not send but the row needs is filled from the entry.
pub fn remote_line(entry: &MentionEntry, raw: &RemoteRaw) -> String {
    let mut tags: Vec<String> = raw
        .tags_raw
        .trim_start_matches('@')
        .split(';')
        .filter(|t| !t.is_empty())
        .filter(|t| !DROPPED_TAGS.contains(&t.split('=').next().unwrap_or_default()))
        .map(str::to_string)
        .collect();
    let has = |tags: &[String], name: &str| tags.iter().any(|t| t.split('=').next() == Some(name));
    let mut fill = |name: &str, value: String| {
        if !value.is_empty() && !has(&tags, name) {
            tags.push(format!("{name}={value}"));
        }
    };
    fill("id", entry.id.clone());
    fill("user-id", entry.sender_id.clone());
    fill("display-name", escape_tag(&entry.sender_name));
    fill("tmi-sent-ts", entry.at_ms.to_string());
    fill("room-id", entry.channel_id.clone().unwrap_or_default());
    let body = if raw.action && !raw.body_raw.starts_with('\u{1}') {
        format!("\u{1}ACTION {}\u{1}", raw.body_raw)
    } else {
        raw.body_raw.clone()
    };
    let login = &entry.sender_login;
    format!("@{} :{login}!{login}@{login}.tmi.twitch.tv PRIVMSG #{} :{body}", tags.join(";"), entry.channel_login)
}

/// A remote row's snapshot: the server's raw body and tags parsed with the
/// live parser and the chat's emote table (a private one when this app never
/// joined it). None when it cannot be built (the pin then waits out the 10 s
/// and shows its text row).
pub async fn remote_snapshot(entry: &MentionEntry, raw: &RemoteRaw) -> Option<Box<RawValue>> {
    if raw.body_raw.is_empty() || raw.tags_raw.is_empty() {
        return None;
    }
    let line = remote_line(entry, raw);
    let login = entry.channel_login.to_lowercase();
    let room_id = tag_value(&line, "room-id").unwrap_or_default().to_string();
    let mut channel = crate::services::irc_service::parse_lookup_for(&login).or_else(|| owner().tables.get(&login).map(|t| t.lookup.clone()));
    if channel.is_none() && !room_id.is_empty() && begin_table(&login) {
        let table = fetch_table(&login, &room_id).await;
        let mut o = owner();
        o.table_inflight.remove(&login);
        o.table_book.finish(&login, table.is_some(), Instant::now());
        if let Some(lookup) = table {
            o.tables.insert(login.clone(), Table { lookup: lookup.clone(), at: Instant::now() });
            channel = Some(lookup);
        }
    }
    let cheermotes = crate::services::irc_service::channel_cheermote_set(&login).or_else(|| owner().cheers.get(&login).cloned());
    let snaps = ParseSnapshots::for_pin(channel, tag_value(&line, "user-id").unwrap_or_default(), cheermotes);
    let mut msg = IrcService::parse_privmsg_with(&line, Some(&snaps))?;
    msg.id = entry.id.clone();
    snapshot_of(&msg, Some(&line))
}

#[cfg(test)]
pub(crate) fn install_table_for_test(login: &str, lookup: Arc<EmoteLookup>) {
    owner().tables.insert(login.to_string(), Table { lookup, at: Instant::now() });
}

#[cfg(test)]
pub(crate) fn reset_for_test() {
    *owner() = Owner::default();
}

#[cfg(test)]
pub(crate) fn mark_badges_answered_for_test(room: &str) {
    let mut o = owner();
    o.badges.start(room);
    o.badges.finish(room, true, Instant::now());
}

#[cfg(test)]
#[path = "mention_pin_row_tests.rs"]
mod tests;
