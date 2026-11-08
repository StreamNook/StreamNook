//! The whisper archive on disk, one Twitch account at a time.
//!
//! Layout under the app data folder:
//!
//! - `whispers/<owner>/index.json`: every conversation's meta, no messages;
//! - `whispers/<owner>/c/<key>.json`: one conversation's messages, oldest
//!   first, with a copy of its meta;
//! - `whispers/<owner>.json.v1.bak`: the old single file, kept after the move.
//!
//! Memory holds each touched account's index and at most `LRU_CAP` loaded
//! conversations. A change marks files dirty; a debounced task writes the
//! conversation files first and the index last, so the index is the commit
//! point, and each file has its own dirty flag. Every write names the account
//! and the generation it was started under; a closed generation is refused, so
//! nothing is written for an account after it was signed out of or switched
//! away from (teardown's own writes go through `Closing`).

use crate::services::mention_inbox::MentionPart;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

pub const INDEX_VERSION: u32 = 2;
pub const LRU_CAP: usize = 8;
const FILE_VERSION: u32 = 1;
const DIR: &str = "whispers";
const LEGACY_GLOBAL: &str = "whispers.json";
const FLUSH_EVERY: std::time::Duration = std::time::Duration::from_secs(5);
const DELETED_TEXT: &str = "Message deleted";

// ---- Types ------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    #[default]
    Twitch,
    LocalSent,
    Scraped,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Delivery {
    Sending,
    Sent,
    Unknown,
    Delivered,
    NotDelivered,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct StoredWhisper {
    pub id: String,
    pub from_user_id: String,
    pub from_user_login: String,
    pub from_user_name: String,
    pub to_user_id: String,
    pub to_user_login: String,
    pub to_user_name: String,
    pub message: String,
    /// Twitch time (ms) for Twitch messages; local time plus the measured
    /// offset for our own sends.
    pub timestamp: i64,
    pub is_sent: bool,
    /// Twitch emote parts only. Empty: the message is plain `message`.
    #[serde(default)]
    pub parts: Vec<MentionPart>,
    #[serde(default)]
    pub deleted: bool,
    #[serde(default)]
    pub edited_ms: Option<i64>,
    #[serde(default)]
    pub origin: Origin,
    /// Only our own sends made by this version.
    #[serde(default)]
    pub delivery: Option<Delivery>,
    /// The local clock when we sent it.
    #[serde(default)]
    pub sent_local_ms: Option<i64>,
    /// Helix answered 204 for this send.
    #[serde(default)]
    pub got_204: bool,
    /// Real delivery checks run so far.
    #[serde(default)]
    pub checks: u8,
    /// Why it was not delivered, in the words every platform shows.
    #[serde(default)]
    pub failure: Option<String>,
    /// Twitch cut this whisper to this many characters.
    #[serde(default)]
    pub cut_to: Option<u32>,
    /// A copy of one of our sends from the old store, adoptable once.
    #[serde(default)]
    pub migrated: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Marker {
    pub id: String,
    pub at_ms: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Cursor {
    pub cursor: String,
    pub device_id: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Deep {
    #[default]
    Unknown,
    Complete,
    Partial { cursor: Option<Cursor> },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TwitchBadge {
    pub set_id: String,
    pub version: String,
    pub title: String,
    pub image_url: String,
}

/// The size and modified time of a conversation file the store wrote.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct FileStamp {
    pub len: u64,
    pub modified_ns: u64,
}

/// One conversation's meta in the index (no messages).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ConversationMeta {
    pub key: String,
    pub user_id: String,
    pub user_login: String,
    pub user_name: String,
    pub profile_image_url: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub badges: Vec<TwitchBadge>,
    #[serde(default)]
    pub thread_id: Option<String>,
    pub last_message_timestamp: i64,
    #[serde(default)]
    pub last_text: String,
    #[serde(default)]
    pub last_parts: Vec<MentionPart>,
    #[serde(default)]
    pub last_is_sent: bool,
    pub unread_count: i32,
    #[serde(default)]
    pub read_through: Option<Marker>,
    #[serde(default)]
    pub twitch_read: Option<Marker>,
    #[serde(default)]
    pub twitch_read_owed: Option<String>,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub archive_pending: bool,
    #[serde(default)]
    pub archive_requested_ms: Option<i64>,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub deep: Deep,
    #[serde(default)]
    pub gap: Option<Cursor>,
    #[serde(default)]
    pub file_stamp: Option<FileStamp>,
    /// Some message is one of our sends still waiting on Twitch (Sending,
    /// Sent or Unknown); set by `recount` (reconciled).
    #[serde(default)]
    pub delivery_pending: bool,
    /// Sequence numbers of local writes to fields a Twitch page also carries.
    #[serde(skip)]
    pub archived_seq: u64,
    #[serde(skip)]
    pub read_seq: u64,
}

impl ConversationMeta {
    /// An empty conversation with the user `key`.
    pub fn new(key: &str) -> Self {
        ConversationMeta {
            key: key.to_string(),
            user_id: key.to_string(),
            user_login: String::new(),
            user_name: String::new(),
            profile_image_url: None,
            color: None,
            badges: Vec::new(),
            thread_id: None,
            last_message_timestamp: 0,
            last_text: String::new(),
            last_parts: Vec::new(),
            last_is_sent: false,
            unread_count: 0,
            read_through: None,
            twitch_read: None,
            twitch_read_owed: None,
            archived: false,
            archive_pending: false,
            archive_requested_ms: None,
            muted: false,
            deep: Deep::Unknown,
            gap: None,
            file_stamp: None,
            delivery_pending: false,
            archived_seq: 0,
            read_seq: 0,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct AccountIndex {
    pub version: u32,
    pub conversations: Vec<ConversationMeta>,
    #[serde(default)]
    pub imported_at: Option<i64>,
    #[serde(default)]
    pub list_deep: Deep,
    #[serde(default)]
    pub island_cleared_through: Option<i64>,
}

/// Messages of one conversation, oldest first by (timestamp, id), with a copy
/// of its meta for rejoining the index after a crash.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct ConversationFile {
    pub version: u32,
    pub key: String,
    #[serde(default)]
    pub meta: Option<ConversationMeta>,
    pub messages: Vec<StoredWhisper>,
}

/// A loaded conversation: its meta and its messages.
pub struct Conversation {
    pub meta: ConversationMeta,
    pub messages: Vec<StoredWhisper>,
}

/// Which account and generation a write was started under.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scope {
    pub owner: String,
    pub generation: u64,
}

/// Teardown's right to write and flush a closed scope.
pub struct Closing(pub(crate) Scope);

impl Closing {
    pub fn scope(&self) -> &Scope {
        &self.0
    }
}

// ---- State ------------------------------------------------------------------

struct Loaded {
    messages: Vec<StoredWhisper>,
    dirty: bool,
}

struct OwnerEntry {
    dir: PathBuf,
    index: AccountIndex,
    index_dirty: bool,
    /// The index was written by a newer StreamNook: read, never write.
    read_only: bool,
    loaded: HashMap<String, Loaded>,
    /// Least recently used first.
    lru: VecDeque<String>,
    pinned: HashSet<String>,
}

impl OwnerEntry {
    fn new(dir: PathBuf) -> Self {
        OwnerEntry {
            dir,
            index: AccountIndex { version: INDEX_VERSION, ..Default::default() },
            index_dirty: false,
            read_only: false,
            loaded: HashMap::new(),
            lru: VecDeque::new(),
            pinned: HashSet::new(),
        }
    }
}

#[derive(Default)]
struct StoreState {
    owners: HashMap<String, OwnerEntry>,
    /// The open generation of each account.
    open: HashMap<String, u64>,
}

static STATE: Lazy<Mutex<StoreState>> = Lazy::new(|| Mutex::new(StoreState::default()));
static ANY_DIRTY: AtomicBool = AtomicBool::new(false);
static FLUSH_TASK_STARTED: AtomicBool = AtomicBool::new(false);
static SEQ: AtomicU64 = AtomicU64::new(0);
#[cfg(test)]
static TEST_ROOT: Mutex<Option<PathBuf>> = Mutex::new(None);

fn state() -> MutexGuard<'static, StoreState> {
    STATE.lock().unwrap_or_else(|p| p.into_inner())
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Keep ids filesystem-safe: Twitch ids are digits and logins are letters,
/// digits and underscores, so nothing real is lost.
fn safe_name(raw: &str) -> String {
    raw.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-').collect()
}

/// A number that only grows, for ordering local writes against pages that
/// were requested before them.
pub fn next_seq() -> u64 {
    SEQ.fetch_add(1, Ordering::SeqCst) + 1
}

// ---- Where the files live ---------------------------------------------------

/// The app data folder. Works without a window: the core's app handle when
/// there is one, else the phone's data folder recorded at start.
pub fn root_dir() -> Result<PathBuf, String> {
    #[cfg(test)]
    {
        let mut root = TEST_ROOT.lock().unwrap_or_else(|p| p.into_inner());
        let dir = root.get_or_insert_with(|| {
            let dir = std::env::temp_dir().join(format!("sn-whisper-store-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            dir
        });
        Ok(dir.clone())
    }
    #[cfg(not(test))]
    {
        use tauri::Manager;
        if let Some(app) = crate::services::providers::app_handle() {
            if let Ok(dir) = app.path().app_data_dir() {
                return Ok(dir);
            }
        }
        crate::services::app_paths::mobile_base().ok_or_else(|| "no app data folder".to_string())
    }
}

/// Use `dir` as the app data folder in tests (the first call wins).
#[cfg(test)]
pub fn set_root_for_tests(dir: PathBuf) {
    let mut root = TEST_ROOT.lock().unwrap_or_else(|p| p.into_inner());
    if root.is_none() {
        *root = Some(dir);
    }
}

fn conv_path(dir: &Path, key: &str) -> PathBuf {
    dir.join("c").join(format!("{key}.json"))
}

fn stamp_of(path: &Path) -> Option<FileStamp> {
    let meta = fs::metadata(path).ok()?;
    let modified_ns = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos() as u64;
    Some(FileStamp { len: meta.len(), modified_ns })
}

/// Write `body` to `path` through a temporary file, so a crash leaves the old
/// file whole.
fn write_atomic(path: &Path, body: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("could not create the whispers folder: {e}"))?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, body).map_err(|e| format!("could not write a whispers file: {e}"))?;
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("could not save a whispers file: {e}")
    })
}

/// Rename a file that did not parse out of the way, keeping it.
fn set_aside(path: &Path) {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let aside = path.with_file_name(format!("{name}.corrupt-{}", now_ms()));
    let _ = fs::rename(path, aside);
}

/// Move `from` to `to`, adding a time to `to` when that name is taken.
fn retire(from: &Path, to: &Path) {
    let target = if to.exists() {
        let name = to.file_name().and_then(|n| n.to_str()).unwrap_or("file");
        to.with_file_name(format!("{name}-{}", now_ms()))
    } else {
        to.to_path_buf()
    };
    let _ = fs::rename(from, target);
}

/// Temporary files a crash left behind.
fn remove_leftover_tmp(dir: &Path) {
    for folder in [dir.to_path_buf(), dir.join("c")] {
        let Ok(items) = fs::read_dir(&folder) else { continue };
        for item in items.flatten() {
            let path = item.path();
            if path.extension().and_then(|e| e.to_str()) == Some("tmp") {
                let _ = fs::remove_file(path);
            }
        }
    }
}

fn read_conv_file(path: &Path) -> Result<ConversationFile, String> {
    let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

#[derive(Serialize)]
struct ConversationFileOut<'a> {
    version: u32,
    key: &'a str,
    meta: Option<&'a ConversationMeta>,
    messages: &'a [StoredWhisper],
}

// ---- Counting -----------------------------------------------------------------

/// Oldest first by (timestamp, id); a message id appears once.
fn normalize(messages: &mut Vec<StoredWhisper>) {
    messages.sort_by(|a, b| a.timestamp.cmp(&b.timestamp).then_with(|| a.id.cmp(&b.id)));
    let mut seen = HashSet::new();
    messages.retain(|m| seen.insert(m.id.clone()));
}

/// Where a read marker sits: its message, else the newest message at or
/// before its time, else before everything.
fn marker_position(messages: &[StoredWhisper], marker: &Marker) -> Option<usize> {
    if let Some(i) = messages.iter().rposition(|m| m.id == marker.id) {
        return Some(i);
    }
    messages.iter().rposition(|m| m.timestamp <= marker.at_ms)
}

/// Unread and the newest line, from the messages and the later of the two
/// read markers. Unread is every message from the other person, not deleted,
/// after that marker.
pub fn recount(meta: &mut ConversationMeta, messages: &[StoredWhisper]) {
    let boundary = [meta.read_through.as_ref(), meta.twitch_read.as_ref()]
        .into_iter()
        .flatten()
        .filter_map(|m| marker_position(messages, m))
        .max();
    let start = boundary.map_or(0, |b| b + 1).min(messages.len());
    meta.unread_count = messages[start..].iter().filter(|m| !m.is_sent && !m.deleted).count() as i32;
    // (reconciled) a send still waiting on Twitch keeps the conversation on
    // the delivery recheck list (whisper_send::recheck_pending).
    meta.delivery_pending = messages.iter().any(|m| {
        matches!(m.delivery, Some(Delivery::Sending) | Some(Delivery::Sent) | Some(Delivery::Unknown))
    });
    match messages.last() {
        Some(last) => {
            meta.last_message_timestamp = last.timestamp;
            meta.last_is_sent = last.is_sent;
            if last.deleted {
                meta.last_text = DELETED_TEXT.to_string();
                meta.last_parts = Vec::new();
            } else {
                meta.last_text = last.message.clone();
                meta.last_parts = last.parts.clone();
            }
        }
        None => {
            meta.last_text.clear();
            meta.last_parts.clear();
            meta.last_is_sent = false;
        }
    }
}

fn newest_incoming(messages: &[StoredWhisper]) -> Option<Marker> {
    messages
        .iter()
        .rev()
        .find(|m| !m.is_sent)
        .map(|m| Marker { id: m.id.clone(), at_ms: m.timestamp })
}

/// A meta for a conversation known only from its messages.
fn meta_from_messages(key: &str, messages: &[StoredWhisper]) -> ConversationMeta {
    let mut meta = ConversationMeta::new(key);
    if let Some(m) = messages.last() {
        let (id, login, name) = if m.is_sent {
            (&m.to_user_id, &m.to_user_login, &m.to_user_name)
        } else {
            (&m.from_user_id, &m.from_user_login, &m.from_user_name)
        };
        if !id.is_empty() {
            meta.user_id = id.clone();
        }
        meta.user_login = login.clone();
        meta.user_name = name.clone();
    }
    meta
}

// ---- Loading an account -----------------------------------------------------

#[derive(Deserialize)]
struct V1Storage {
    #[serde(default)]
    conversations: HashMap<String, V1Conversation>,
}

#[derive(Deserialize)]
struct V1Conversation {
    #[serde(default)]
    user_id: String,
    #[serde(default)]
    user_login: String,
    #[serde(default)]
    user_name: String,
    #[serde(default)]
    profile_image_url: Option<String>,
    #[serde(default)]
    messages: Vec<StoredWhisper>,
    #[serde(default)]
    last_message_timestamp: i64,
    #[serde(default)]
    unread_count: i32,
}

fn read_v1(path: &Path) -> Result<V1Storage, String> {
    let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

/// Where an old message came from, read from the id the old store gave it.
fn tag_migrated(m: &mut StoredWhisper) {
    if m.id.starts_with("msg-") {
        m.origin = Origin::Scraped;
    } else if m.id.starts_with("sent-") {
        m.origin = Origin::LocalSent;
        m.migrated = true;
        m.delivery = None;
        m.sent_local_ms = Some(m.timestamp);
    } else {
        m.origin = Origin::Twitch;
    }
}

/// The old store kept an unread count; the new one keeps the message read
/// through. The (N+1)-th newest message from the other person keeps exactly N
/// unread, even when stamps tie.
fn migrated_marker(messages: &[StoredWhisper], unread: i32) -> Option<Marker> {
    let incoming: Vec<&StoredWhisper> = messages.iter().filter(|m| !m.is_sent && !m.deleted).collect();
    let n = unread.max(0) as usize;
    if incoming.len() <= n {
        return None;
    }
    let m = incoming[incoming.len() - 1 - n];
    Some(Marker { id: m.id.clone(), at_ms: m.timestamp })
}

fn write_conv_body(meta: Option<&ConversationMeta>, key: &str, messages: &[StoredWhisper]) -> Result<String, String> {
    serde_json::to_string(&ConversationFileOut { version: FILE_VERSION, key, meta, messages })
        .map_err(|e| format!("could not encode a conversation: {e}"))
}

/// Move an old single-file archive into the new layout: every conversation
/// file first, the index last.
fn migrate(entry: &mut OwnerEntry, storage: V1Storage) -> Result<(), String> {
    let mut metas = Vec::new();
    for (raw_key, conv) in storage.conversations {
        let key = safe_name(&raw_key);
        if key.is_empty() {
            continue;
        }
        let mut messages = conv.messages;
        for m in &mut messages {
            tag_migrated(m);
        }
        normalize(&mut messages);
        let mut meta = ConversationMeta::new(&key);
        meta.user_id = if conv.user_id.is_empty() { key.clone() } else { conv.user_id };
        meta.user_login = conv.user_login;
        meta.user_name = conv.user_name;
        meta.profile_image_url = conv.profile_image_url;
        meta.last_message_timestamp = conv.last_message_timestamp;
        meta.read_through = migrated_marker(&messages, conv.unread_count);
        recount(&mut meta, &messages);
        let path = conv_path(&entry.dir, &key);
        write_atomic(&path, &write_conv_body(Some(&meta), &key, &messages)?)?;
        meta.file_stamp = stamp_of(&path);
        metas.push(meta);
    }
    entry.index = AccountIndex { version: INDEX_VERSION, conversations: metas, ..Default::default() };
    write_index(entry)?;
    log::info!("[WhisperStore] moved {} conversations to the new layout", entry.index.conversations.len());
    Ok(())
}

/// Bring conversation files the index does not describe (written after the
/// index, or never in it) back into the index.
fn rejoin_files(entry: &mut OwnerEntry) {
    let Ok(items) = fs::read_dir(entry.dir.join("c")) else { return };
    for item in items.flatten() {
        let path = item.path();
        let Some(key) = path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_suffix(".json"))
            .map(str::to_string)
        else {
            continue;
        };
        let stamp = stamp_of(&path);
        let pos = position(&entry.index, &key);
        if let Some(i) = pos {
            if stamp.is_some() && entry.index.conversations[i].file_stamp == stamp {
                continue;
            }
        }
        let file = match read_conv_file(&path) {
            Ok(file) => file,
            Err(e) => {
                log::warn!("[WhisperStore] a conversation file did not parse ({e}); it is kept aside and fetched again");
                set_aside(&path);
                if let Some(i) = pos {
                    let meta = &mut entry.index.conversations[i];
                    meta.deep = Deep::Partial { cursor: None };
                    meta.file_stamp = None;
                    entry.index_dirty = true;
                }
                continue;
            }
        };
        let mut messages = file.messages;
        normalize(&mut messages);
        match pos {
            Some(i) => {
                let meta = &mut entry.index.conversations[i];
                recount(meta, &messages);
                meta.file_stamp = stamp;
            }
            None => {
                let mut meta = match file.meta {
                    Some(mut snapshot) => {
                        snapshot.key = key.clone();
                        snapshot
                    }
                    None => {
                        let mut meta = meta_from_messages(&key, &messages);
                        meta.read_through = newest_incoming(&messages);
                        meta.deep = Deep::Partial { cursor: None };
                        meta
                    }
                };
                recount(&mut meta, &messages);
                meta.file_stamp = stamp;
                entry.index.conversations.push(meta);
            }
        }
        entry.index_dirty = true;
    }
}

/// Load one account from the folder under `root`: its index, or the old
/// single file (moved to the new layout), or the oldest global file (adopted
/// once), or nothing yet.
fn seed_at(root: &Path, owner: &str) -> Result<OwnerEntry, String> {
    let base = root.join(DIR);
    let dir = base.join(owner);
    let mut entry = OwnerEntry::new(dir.clone());
    remove_leftover_tmp(&dir);
    let index_path = dir.join("index.json");
    if index_path.exists() {
        let parsed = fs::read_to_string(&index_path)
            .ok()
            .and_then(|s| serde_json::from_str::<AccountIndex>(&s).ok());
        match parsed {
            Some(index) if index.version > INDEX_VERSION => {
                log::warn!("[WhisperStore] the whisper index was written by a newer StreamNook; it is opened read-only");
                entry.index = index;
                entry.read_only = true;
            }
            Some(index) => {
                entry.index = index;
                rejoin_files(&mut entry);
            }
            None => {
                log::warn!("[WhisperStore] the whisper index did not parse; rebuilding it from the conversation files");
                set_aside(&index_path);
                entry.index_dirty = true;
                rejoin_files(&mut entry);
            }
        }
        return Ok(entry);
    }
    let v1 = base.join(format!("{owner}.json"));
    if v1.exists() {
        match read_v1(&v1) {
            Ok(storage) => {
                migrate(&mut entry, storage)?;
                retire(&v1, &base.join(format!("{owner}.json.v1.bak")));
            }
            Err(e) => {
                log::warn!("[WhisperStore] the old whispers file did not parse ({e}); it is kept aside");
                set_aside(&v1);
            }
        }
        return Ok(entry);
    }
    let global = root.join(LEGACY_GLOBAL);
    if global.exists() {
        match read_v1(&global) {
            Ok(storage) => {
                migrate(&mut entry, storage)?;
                retire(&global, &root.join("whispers.json.migrated"));
            }
            Err(e) => log::warn!("[WhisperStore] the oldest whispers file did not parse ({e}); left in place"),
        }
        return Ok(entry);
    }
    if dir.join("c").is_dir() {
        entry.index_dirty = true;
        rejoin_files(&mut entry);
    }
    Ok(entry)
}

// ---- Access -------------------------------------------------------------------

enum Access<'a> {
    Open(&'a Scope),
    Closing(&'a Closing),
}

impl Access<'_> {
    fn scope(&self) -> &Scope {
        match self {
            Access::Open(s) => s,
            Access::Closing(c) => &c.0,
        }
    }
}

/// The account's folder name, if this access may touch it.
fn owner_for(st: &StoreState, access: &Access) -> Result<String, String> {
    let owner = safe_name(&access.scope().owner);
    if owner.is_empty() {
        return Err("not signed in".into());
    }
    if let Access::Open(scope) = access {
        if st.open.get(&owner) != Some(&scope.generation) {
            return Err("closed".into());
        }
    }
    Ok(owner)
}

fn entry_mut<'a>(st: &'a mut StoreState, owner: &str) -> Result<&'a mut OwnerEntry, String> {
    if !st.owners.contains_key(owner) {
        let root = root_dir()?;
        let seeded = seed_at(&root, owner)?;
        st.owners.insert(owner.to_string(), seeded);
    }
    Ok(st.owners.get_mut(owner).expect("seeded above"))
}

fn position(index: &AccountIndex, key: &str) -> Option<usize> {
    index.conversations.iter().position(|m| m.key == key)
}

fn is_dirty(entry: &OwnerEntry) -> bool {
    entry.index_dirty || entry.loaded.values().any(|l| l.dirty)
}

/// Open `owner` under `generation`: writes started under any other generation
/// of this account are refused from now on.
pub fn open(owner: &str, generation: u64) -> Scope {
    let name = safe_name(owner);
    if !name.is_empty() {
        state().open.insert(name, generation);
    }
    Scope { owner: owner.to_string(), generation }
}

/// Refuse every future `update` started under `scope`. Teardown keeps writing
/// and flushing through the returned `Closing`.
pub fn close(scope: &Scope) -> Closing {
    let name = safe_name(&scope.owner);
    let mut st = state();
    if st.open.get(&name) == Some(&scope.generation) {
        st.open.remove(&name);
    }
    Closing(scope.clone())
}

pub fn read_index<R>(scope: &Scope, f: impl FnOnce(&AccountIndex) -> R) -> Result<R, String> {
    let mut st = state();
    let owner = owner_for(&st, &Access::Open(scope))?;
    let entry = entry_mut(&mut st, &owner)?;
    let out = f(&entry.index);
    let dirty = is_dirty(entry);
    drop(st);
    if dirty {
        mark_dirty();
    }
    Ok(out)
}

/// Change the account's index. `f` must not add or remove conversations
/// (`update` and `rekey` do that).
pub fn update_index<R>(scope: &Scope, f: impl FnOnce(&mut AccountIndex) -> (R, bool)) -> Result<R, String> {
    let mut st = state();
    let owner = owner_for(&st, &Access::Open(scope))?;
    let entry = entry_mut(&mut st, &owner)?;
    if entry.read_only {
        return Err("read-only".into());
    }
    let (out, changed) = f(&mut entry.index);
    if changed {
        entry.index_dirty = true;
    }
    let dirty = is_dirty(entry);
    drop(st);
    if dirty {
        mark_dirty();
    }
    Ok(out)
}

/// Make room for one more loaded conversation: the least recently used one
/// that is not pinned leaves, written first when dirty.
fn make_room(entry: &mut OwnerEntry, incoming: &str) -> Result<(), String> {
    while entry.loaded.len() >= LRU_CAP {
        let victim = entry
            .lru
            .iter()
            .find(|k| k.as_str() != incoming && !entry.pinned.contains(k.as_str()))
            .cloned();
        let Some(victim) = victim else {
            return Err("too many whisper conversations are in use at once".into());
        };
        if entry.loaded.get(&victim).is_some_and(|l| l.dirty) {
            write_conv(entry, &victim)?;
        }
        entry.loaded.remove(&victim);
        entry.lru.retain(|k| k != &victim);
    }
    Ok(())
}

/// Load a conversation's messages into memory (most recently used now).
/// `check_ahead`: an index entry newer than its file (the file was lost or is
/// short) is recounted from what the file has and marked for a fresh import.
fn load(entry: &mut OwnerEntry, key: &str, check_ahead: bool) -> Result<(), String> {
    if entry.loaded.contains_key(key) {
        entry.lru.retain(|k| k != key);
        entry.lru.push_back(key.to_string());
        return Ok(());
    }
    make_room(entry, key)?;
    let path = conv_path(&entry.dir, key);
    let mut damaged = false;
    let mut messages = if path.exists() {
        match read_conv_file(&path) {
            Ok(file) => file.messages,
            Err(e) => {
                log::warn!("[WhisperStore] a conversation file did not parse ({e}); it is kept aside and fetched again");
                set_aside(&path);
                damaged = true;
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };
    normalize(&mut messages);
    if let Some(i) = position(&entry.index, key) {
        let meta = &mut entry.index.conversations[i];
        let ahead = check_ahead
            && match messages.last() {
                Some(newest) => meta.last_message_timestamp > newest.timestamp,
                None => !meta.last_text.is_empty(),
            };
        if damaged || ahead {
            recount(meta, &messages);
            meta.deep = Deep::Partial { cursor: None };
            meta.file_stamp = None;
            entry.index_dirty = true;
        }
    }
    entry.loaded.insert(key.to_string(), Loaded { messages, dirty: false });
    entry.lru.push_back(key.to_string());
    Ok(())
}

/// Write one loaded conversation's file now and record its stamp.
fn write_conv(entry: &mut OwnerEntry, key: &str) -> Result<(), String> {
    if entry.read_only {
        return Ok(());
    }
    let Some(loaded) = entry.loaded.get(key) else { return Ok(()) };
    let meta = position(&entry.index, key).map(|i| &entry.index.conversations[i]);
    let body = write_conv_body(meta, key, &loaded.messages)?;
    let path = conv_path(&entry.dir, key);
    write_atomic(&path, &body)?;
    let stamp = stamp_of(&path);
    if let Some(l) = entry.loaded.get_mut(key) {
        l.dirty = false;
    }
    if let Some(i) = position(&entry.index, key) {
        entry.index.conversations[i].file_stamp = stamp;
    }
    entry.index_dirty = true;
    Ok(())
}

fn write_index(entry: &mut OwnerEntry) -> Result<(), String> {
    if entry.read_only {
        return Ok(());
    }
    let body = serde_json::to_string(&entry.index).map_err(|e| format!("could not encode the whisper index: {e}"))?;
    write_atomic(&entry.dir.join("index.json"), &body)?;
    entry.index_dirty = false;
    Ok(())
}

fn update_with<R>(
    access: Access,
    key: &str,
    create: Option<ConversationMeta>,
    f: impl FnOnce(&mut Conversation) -> (R, bool),
) -> Result<Option<R>, String> {
    let key = safe_name(key);
    if key.is_empty() {
        return Ok(None);
    }
    let mut st = state();
    let owner = owner_for(&st, &access)?;
    let entry = entry_mut(&mut st, &owner)?;
    if entry.read_only {
        return Err("read-only".into());
    }
    let existed = position(&entry.index, &key).is_some();
    if !existed {
        let Some(mut meta) = create else { return Ok(None) };
        meta.key = key.clone();
        meta.file_stamp = None;
        entry.index.conversations.push(meta);
        entry.index_dirty = true;
    }
    if let Err(e) = load(entry, &key, existed) {
        if !existed {
            entry.index.conversations.retain(|m| m.key != key);
        }
        return Err(e);
    }
    let i = position(&entry.index, &key).expect("present above");
    let loaded = entry.loaded.get_mut(&key).expect("loaded above");
    let mut conv = Conversation {
        meta: entry.index.conversations[i].clone(),
        messages: std::mem::take(&mut loaded.messages),
    };
    let (out, changed) = f(&mut conv);
    let Conversation { mut meta, mut messages } = conv;
    if changed || !existed {
        meta.key = key.clone();
        meta.file_stamp = entry.index.conversations[i].file_stamp.clone();
        normalize(&mut messages);
        recount(&mut meta, &messages);
        entry.index.conversations[i] = meta;
        entry.index_dirty = true;
        loaded.dirty = true;
    }
    loaded.messages = messages;
    let dirty = is_dirty(entry);
    drop(st);
    if dirty {
        mark_dirty();
    }
    Ok(Some(out))
}

/// Change one conversation. Absent: created from `create` (an existing file of
/// that key is loaded first), or `Ok(None)` without one. When `f` reports a
/// change, the messages are put in order, unread and the newest line are
/// recounted, and both files are marked for the next flush.
pub fn update<R>(
    scope: &Scope,
    key: &str,
    create: Option<ConversationMeta>,
    f: impl FnOnce(&mut Conversation) -> (R, bool),
) -> Result<Option<R>, String> {
    update_with(Access::Open(scope), key, create, f)
}

/// `update` for teardown, on an account already closed. Never creates.
pub fn update_closing<R>(
    closing: &Closing,
    key: &str,
    f: impl FnOnce(&mut Conversation) -> (R, bool),
) -> Result<Option<R>, String> {
    update_with(Access::Closing(closing), key, None, f)
}

pub fn read<R>(scope: &Scope, key: &str, f: impl FnOnce(&Conversation) -> R) -> Result<Option<R>, String> {
    let key = safe_name(key);
    let mut st = state();
    let owner = owner_for(&st, &Access::Open(scope))?;
    let entry = entry_mut(&mut st, &owner)?;
    if position(&entry.index, &key).is_none() {
        return Ok(None);
    }
    load(entry, &key, true)?;
    let i = position(&entry.index, &key).expect("present above");
    let loaded = entry.loaded.get_mut(&key).expect("loaded above");
    let conv = Conversation {
        meta: entry.index.conversations[i].clone(),
        messages: std::mem::take(&mut loaded.messages),
    };
    let out = f(&conv);
    loaded.messages = conv.messages;
    let dirty = is_dirty(entry) && !entry.read_only;
    drop(st);
    if dirty {
        mark_dirty();
    }
    Ok(Some(out))
}

/// Move the conversation at `from` to `to`, merging into one already there.
/// Files: the new conversation file first, then the index, then the old file
/// is deleted, so an index rebuilt after a crash never brings the old one back.
pub fn rekey(scope: &Scope, from: &str, to: &str) -> Result<(), String> {
    let (from, to) = (safe_name(from), safe_name(to));
    if from.is_empty() || to.is_empty() || from == to {
        return Ok(());
    }
    let mut st = state();
    let owner = owner_for(&st, &Access::Open(scope))?;
    let entry = entry_mut(&mut st, &owner)?;
    if entry.read_only {
        return Err("read-only".into());
    }
    if position(&entry.index, &from).is_none() {
        return Ok(());
    }
    let to_exists = position(&entry.index, &to).is_some();
    let to_was_pinned = entry.pinned.contains(&to);
    if to_exists {
        load(entry, &to, true)?;
        entry.pinned.insert(to.clone());
    }
    let loaded_from = load(entry, &from, true);
    if !to_was_pinned {
        entry.pinned.remove(&to);
    }
    loaded_from?;
    let fi = position(&entry.index, &from).expect("present above");
    let moving_meta = entry.index.conversations.remove(fi);
    let moving = entry.loaded.remove(&from).map(|l| l.messages).unwrap_or_default();
    entry.lru.retain(|k| k != &from);
    let from_was_pinned = entry.pinned.remove(&from);
    if to_exists {
        let ti = position(&entry.index, &to).expect("present above");
        let target = entry.loaded.get_mut(&to).expect("loaded above");
        let mut messages = std::mem::take(&mut target.messages);
        messages.extend(moving);
        normalize(&mut messages);
        let meta = &mut entry.index.conversations[ti];
        if meta.profile_image_url.is_none() {
            meta.profile_image_url = moving_meta.profile_image_url;
        }
        recount(meta, &messages);
        target.messages = messages;
        target.dirty = true;
    } else {
        let mut meta = moving_meta;
        meta.key = to.clone();
        meta.user_id = to.clone();
        meta.file_stamp = None;
        let mut messages = moving;
        normalize(&mut messages);
        recount(&mut meta, &messages);
        entry.index.conversations.push(meta);
        make_room(entry, &to)?;
        entry.loaded.insert(to.clone(), Loaded { messages, dirty: true });
        entry.lru.push_back(to.clone());
    }
    if from_was_pinned {
        entry.pinned.insert(to.clone());
    }
    write_conv(entry, &to)?;
    write_index(entry)?;
    let _ = fs::remove_file(conv_path(&entry.dir, &from));
    Ok(())
}

/// Drop one message from a conversation. True when it was there.
pub fn remove_message(scope: &Scope, key: &str, message_id: &str) -> Result<bool, String> {
    let removed = update(scope, key, None, |conv| {
        let before = conv.messages.len();
        conv.messages.retain(|m| m.id != message_id);
        let removed = conv.messages.len() != before;
        (removed, removed)
    })?;
    Ok(removed.unwrap_or(false))
}

/// Write one conversation's file now (before a send leaves for Twitch). The
/// index follows at the next flush; a crash in between is repaired at load.
pub fn write_now(scope: &Scope, key: &str) -> Result<(), String> {
    let key = safe_name(key);
    let mut st = state();
    let owner = owner_for(&st, &Access::Open(scope))?;
    let entry = entry_mut(&mut st, &owner)?;
    if entry.read_only {
        return Err("read-only".into());
    }
    if entry.loaded.get(&key).is_some_and(|l| l.dirty) {
        write_conv(entry, &key)?;
    }
    let dirty = is_dirty(entry);
    drop(st);
    if dirty {
        mark_dirty();
    }
    Ok(())
}

/// Keep a conversation in memory while a send of it is queued or in flight.
pub fn pin(scope: &Scope, key: &str, pinned: bool) -> Result<(), String> {
    let key = safe_name(key);
    let mut st = state();
    let owner = owner_for(&st, &Access::Open(scope))?;
    let entry = entry_mut(&mut st, &owner)?;
    if pinned {
        entry.pinned.insert(key);
    } else {
        entry.pinned.remove(&key);
    }
    Ok(())
}

// ---- Flushing -----------------------------------------------------------------

/// Write an account's dirty conversation files, then its index, but the index
/// only when every conversation write succeeded.
fn flush_entry(entry: &mut OwnerEntry) -> Result<(), String> {
    if entry.read_only {
        return Ok(());
    }
    let mut first_err: Option<String> = None;
    let dirty: Vec<String> = entry.loaded.iter().filter(|(_, l)| l.dirty).map(|(k, _)| k.clone()).collect();
    for key in dirty {
        if let Err(e) = write_conv(entry, &key) {
            first_err.get_or_insert(e);
        }
    }
    if first_err.is_none() && entry.index_dirty {
        if let Err(e) = write_index(entry) {
            first_err = Some(e);
        }
    }
    match first_err {
        None => Ok(()),
        Some(e) => Err(e),
    }
}

fn mark_dirty() {
    ANY_DIRTY.store(true, Ordering::Release);
    ensure_flush_task();
}

fn ensure_flush_task() {
    if FLUSH_TASK_STARTED
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
    {
        // No runtime yet: let a later write start the task. ANY_DIRTY stays
        // set, so nothing is lost (the exit flush is the last net).
        if tokio::runtime::Handle::try_current().is_err() {
            FLUSH_TASK_STARTED.store(false, Ordering::SeqCst);
            return;
        }
        tokio::spawn(async {
            loop {
                tokio::time::sleep(FLUSH_EVERY).await;
                if !ANY_DIRTY.load(Ordering::Acquire) {
                    continue;
                }
                match tokio::task::spawn_blocking(flush_now).await {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => log::debug!("[WhisperStore] flush failed (will retry): {e}"),
                    Err(_) => ANY_DIRTY.store(true, Ordering::Release),
                }
            }
        });
    }
}

/// Write everything dirty, for every account in memory (open or closing).
pub fn flush_now() -> Result<(), String> {
    if !ANY_DIRTY.swap(false, Ordering::AcqRel) {
        return Ok(());
    }
    let mut st = state();
    let mut first_err: Option<String> = None;
    for entry in st.owners.values_mut() {
        if let Err(e) = flush_entry(entry) {
            first_err.get_or_insert(e);
        }
    }
    if st.owners.values().any(is_dirty) {
        ANY_DIRTY.store(true, Ordering::Release);
    }
    match first_err {
        None => Ok(()),
        Some(e) => Err(e),
    }
}

/// Write a closed account's dirty files now. Never refused.
pub fn flush_closing(closing: &Closing) -> Result<(), String> {
    let owner = safe_name(&closing.0.owner);
    let mut st = state();
    let Some(entry) = st.owners.get_mut(&owner) else { return Ok(()) };
    let out = flush_entry(entry);
    if is_dirty(entry) {
        ANY_DIRTY.store(true, Ordering::Release);
    }
    out
}

/// Forget a closed account's memory, only after its files are written. A failed
/// write hands the `Closing` back so teardown can try again. An account that
/// was opened again meanwhile keeps its memory.
pub fn drop_owner(closing: Closing) -> Result<(), Closing> {
    let owner = safe_name(&closing.0.owner);
    let mut st = state();
    if st.open.contains_key(&owner) {
        return Ok(());
    }
    let Some(entry) = st.owners.get_mut(&owner) else { return Ok(()) };
    if flush_entry(entry).is_err() || is_dirty(entry) {
        ANY_DIRTY.store(true, Ordering::Release);
        drop(st);
        return Err(closing);
    }
    st.owners.remove(&owner);
    Ok(())
}

// ---- Test helpers -----------------------------------------------------------

#[cfg(test)]
pub(crate) fn flush_owner_for_tests(owner: &str) -> Result<(), String> {
    let mut st = state();
    match st.owners.get_mut(&safe_name(owner)) {
        Some(entry) => flush_entry(entry),
        None => Ok(()),
    }
}

#[cfg(test)]
pub(crate) fn forget_for_tests(owner: &str) {
    state().owners.remove(&safe_name(owner));
}

#[cfg(test)]
pub(crate) fn resident_for_tests(owner: &str) -> usize {
    state().owners.get(&safe_name(owner)).map_or(0, |e| e.loaded.len())
}

#[cfg(test)]
pub(crate) fn dirty_for_tests(owner: &str, key: &str) -> bool {
    state()
        .owners
        .get(&safe_name(owner))
        .and_then(|e| e.loaded.get(key))
        .is_some_and(|l| l.dirty)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    static NEXT: AtomicU64 = AtomicU64::new(0);

    /// A fresh account id per test, so tests sharing the process store never meet.
    fn owner() -> String {
        format!("99999{}", 1000 + NEXT.fetch_add(1, Ordering::SeqCst))
    }

    const THEM: &str = "1000000000";

    fn dir_of(owner: &str) -> PathBuf {
        root_dir().unwrap().join(DIR).join(owner)
    }

    fn whisper(id: &str, at: i64, sent: bool) -> StoredWhisper {
        let (from, from_login, to, to_login) =
            if sent { ("99999999", "me", THEM, "someone") } else { (THEM, "someone", "99999999", "me") };
        StoredWhisper {
            id: id.into(),
            from_user_id: from.into(),
            from_user_login: from_login.into(),
            from_user_name: from_login.to_uppercase(),
            to_user_id: to.into(),
            to_user_login: to_login.into(),
            to_user_name: to_login.to_uppercase(),
            message: format!("text {id}"),
            timestamp: at,
            is_sent: sent,
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
        }
    }

    fn incoming(id: &str, at: i64) -> StoredWhisper {
        whisper(id, at, false)
    }

    fn outgoing(id: &str, at: i64) -> StoredWhisper {
        whisper(id, at, true)
    }

    fn add(scope: &Scope, key: &str, m: StoredWhisper) {
        update(scope, key, Some(ConversationMeta::new(key)), |c| {
            c.messages.push(m);
            ((), true)
        })
        .unwrap();
    }

    fn meta_of(scope: &Scope, key: &str) -> ConversationMeta {
        read(scope, key, |c| c.meta.clone()).unwrap().unwrap()
    }

    fn v1_message(id: &str, at: i64, sent: bool) -> serde_json::Value {
        let m = whisper(id, at, sent);
        json!({
            "id": m.id, "from_user_id": m.from_user_id, "from_user_login": m.from_user_login,
            "from_user_name": m.from_user_name, "to_user_id": m.to_user_id, "to_user_login": m.to_user_login,
            "to_user_name": m.to_user_name, "message": m.message, "timestamp": m.timestamp, "is_sent": m.is_sent
        })
    }

    fn v1_conversation(user_id: &str, messages: Vec<serde_json::Value>, unread: i32) -> serde_json::Value {
        json!({
            "user_id": user_id, "user_login": "someone", "user_name": "Someone", "profile_image_url": null,
            "messages": messages, "last_message_timestamp": 0, "unread_count": unread
        })
    }

    fn write_json(path: &Path, body: &serde_json::Value) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body.to_string()).unwrap();
    }

    fn conv_file(owner: &str, key: &str) -> ConversationFile {
        read_conv_file(&conv_path(&dir_of(owner), key)).unwrap()
    }

    #[test]
    fn migration_splits_files_tags_origins_keeps_backup() {
        let o = owner();
        let base = root_dir().unwrap().join(DIR);
        write_json(
            &base.join(format!("{o}.json")),
            &json!({"version": 1, "conversations": {
                "1000000000": v1_conversation(THEM, vec![
                    v1_message("msg-someone-0", 1000, false),
                    v1_message("sent-2000-0000abcd", 2000, true),
                    v1_message("aaaaaaaa-0000-4000-8000-000000000001", 3000, false),
                ], 0),
                "otherlogin": v1_conversation("otherlogin", vec![v1_message("msg-otherlogin-0", 500, false)], 1),
            }}),
        );
        let s = open(&o, 1);
        let origins = read(&s, THEM, |c| {
            c.messages.iter().map(|m| (m.id.clone(), m.origin, m.migrated, m.delivery)).collect::<Vec<_>>()
        })
        .unwrap()
        .unwrap();
        assert_eq!(
            origins,
            vec![
                ("msg-someone-0".to_string(), Origin::Scraped, false, None),
                ("sent-2000-0000abcd".to_string(), Origin::LocalSent, true, None),
                ("aaaaaaaa-0000-4000-8000-000000000001".to_string(), Origin::Twitch, false, None),
            ]
        );
        assert!(dir_of(&o).join("index.json").is_file());
        assert!(conv_path(&dir_of(&o), THEM).is_file());
        assert!(conv_path(&dir_of(&o), "otherlogin").is_file());
        assert!(base.join(format!("{o}.json.v1.bak")).is_file());
        assert!(!base.join(format!("{o}.json")).exists());
        assert_eq!(meta_of(&s, "otherlogin").unread_count, 1);
        assert_eq!(meta_of(&s, THEM).unread_count, 0);
    }

    #[test]
    fn migration_repeats_safely_after_crash_before_index() {
        let o = owner();
        let base = root_dir().unwrap().join(DIR);
        write_json(
            &base.join(format!("{o}.json")),
            &json!({"version": 1, "conversations": {
                "1000000000": v1_conversation(THEM, vec![v1_message("msg-someone-0", 1000, false), v1_message("msg-someone-1", 2000, false)], 0),
            }}),
        );
        // A crash after a conversation file but before the index: a half file and no index.
        fs::create_dir_all(dir_of(&o).join("c")).unwrap();
        fs::write(conv_path(&dir_of(&o), THEM), "{\"version\":1,\"key\":").unwrap();
        let s = open(&o, 1);
        assert_eq!(read(&s, THEM, |c| c.messages.len()).unwrap(), Some(2));
        assert!(dir_of(&o).join("index.json").is_file());
    }

    #[test]
    fn migration_keeps_unread_with_tied_stamps() {
        let o = owner();
        let base = root_dir().unwrap().join(DIR);
        // Day-only stamps from the old page scraper: every message ties.
        let day = 86_400_000;
        let five = |p: &str| (0..5).map(|n| v1_message(&format!("msg-{p}-{n}"), day, false)).collect::<Vec<_>>();
        write_json(
            &base.join(format!("{o}.json")),
            &json!({"version": 1, "conversations": {
                "1000000001": v1_conversation("1000000001", five("a"), 2),
                "1000000002": v1_conversation("1000000002", five("b"), 0),
                "1000000003": v1_conversation("1000000003", five("c"), 9),
            }}),
        );
        let s = open(&o, 1);
        assert_eq!(meta_of(&s, "1000000001").unread_count, 2);
        assert_eq!(meta_of(&s, "1000000002").unread_count, 0);
        assert_eq!(meta_of(&s, "1000000003").unread_count, 5);
    }

    #[test]
    fn legacy_global_file_is_adopted_once() {
        let root = std::env::temp_dir().join(format!(
            "sn-whisper-global-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&root);
        write_json(
            &root.join(LEGACY_GLOBAL),
            &json!({"version": 1, "conversations": {"1000000000": v1_conversation(THEM, vec![v1_message("msg-someone-0", 1000, false)], 0)}}),
        );
        let first = seed_at(&root, "99999001").unwrap();
        assert_eq!(first.index.conversations.len(), 1);
        assert!(!root.join(LEGACY_GLOBAL).exists());
        assert!(root.join("whispers.json.migrated").is_file());
        let second = seed_at(&root, "99999002").unwrap();
        assert!(second.index.conversations.is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn newer_version_index_opens_read_only() {
        let o = owner();
        write_json(&dir_of(&o).join("index.json"), &json!({"version": 3, "conversations": []}));
        let s = open(&o, 1);
        assert_eq!(
            update(&s, THEM, Some(ConversationMeta::new(THEM)), |_| ((), true)).err(),
            Some("read-only".to_string())
        );
        assert_eq!(read_index(&s, |i| i.version).unwrap(), 3);
    }

    #[test]
    fn corrupt_file_reimports() {
        let o = owner();
        let s = open(&o, 1);
        add(&s, THEM, incoming("a1", 10));
        flush_owner_for_tests(&o).unwrap();
        forget_for_tests(&o);
        fs::write(conv_path(&dir_of(&o), THEM), "not json").unwrap();
        assert_eq!(meta_of(&s, THEM).deep, Deep::Partial { cursor: None });
        assert_eq!(read(&s, THEM, |c| c.messages.len()).unwrap(), Some(0));
        let kept_aside = fs::read_dir(dir_of(&o).join("c"))
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().contains(".json.corrupt-"));
        assert!(kept_aside);
    }

    #[test]
    fn corrupt_index_is_rebuilt_from_files() {
        let o = owner();
        let s = open(&o, 1);
        add(&s, THEM, incoming("a1", 10));
        add(&s, THEM, incoming("a2", 20));
        update(&s, THEM, None, |c| {
            c.meta.read_through = Some(Marker { id: "a1".into(), at_ms: 10 });
            ((), true)
        })
        .unwrap();
        flush_owner_for_tests(&o).unwrap();
        forget_for_tests(&o);
        fs::write(dir_of(&o).join("index.json"), "{ broken").unwrap();
        assert_eq!(meta_of(&s, THEM).unread_count, 1);
        let kept_aside = fs::read_dir(dir_of(&o))
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().starts_with("index.json.corrupt-"));
        assert!(kept_aside);
        flush_owner_for_tests(&o).unwrap();
        let rebuilt: AccountIndex = serde_json::from_str(&fs::read_to_string(dir_of(&o).join("index.json")).unwrap()).unwrap();
        assert_eq!(rebuilt.conversations.len(), 1);
    }

    #[test]
    fn file_ahead_of_index_recovers() {
        let o = owner();
        let s = open(&o, 1);
        add(&s, THEM, incoming("a1", 10));
        flush_owner_for_tests(&o).unwrap();
        forget_for_tests(&o);
        // The conversation file was written again (a second whisper), the index never was.
        let path = conv_path(&dir_of(&o), THEM);
        let mut file = read_conv_file(&path).unwrap();
        file.messages.push(incoming("a2", 20));
        fs::write(&path, serde_json::to_string(&file).unwrap()).unwrap();
        let m = meta_of(&s, THEM);
        assert_eq!(m.unread_count, 2);
        assert_eq!(m.last_text, "text a2");
        assert_eq!(m.last_message_timestamp, 20);
    }

    #[test]
    fn orphan_file_rejoins_index() {
        let o = owner();
        let s = open(&o, 1);
        add(&s, "1000000001", incoming("a1", 10));
        add(&s, "1000000002", incoming("b1", 10));
        update(&s, "1000000002", None, |c| {
            c.messages.push(incoming("b2", 20));
            c.meta.read_through = Some(Marker { id: "b1".into(), at_ms: 10 });
            ((), true)
        })
        .unwrap();
        flush_owner_for_tests(&o).unwrap();
        forget_for_tests(&o);
        // The index on disk was written before the second conversation existed.
        let index_path = dir_of(&o).join("index.json");
        let mut index: AccountIndex = serde_json::from_str(&fs::read_to_string(&index_path).unwrap()).unwrap();
        index.conversations.retain(|m| m.key == "1000000001");
        fs::write(&index_path, serde_json::to_string(&index).unwrap()).unwrap();
        assert_eq!(read_index(&s, |i| i.conversations.len()).unwrap(), 2);
        assert_eq!(meta_of(&s, "1000000002").unread_count, 1);
    }

    #[test]
    fn file_without_meta_rejoins_read_through() {
        let o = owner();
        let s = open(&o, 1);
        let file = ConversationFile {
            version: FILE_VERSION,
            key: THEM.into(),
            meta: None,
            messages: vec![incoming("a1", 10), incoming("a2", 20)],
        };
        write_json(&conv_path(&dir_of(&o), THEM), &serde_json::to_value(&file).unwrap());
        let m = meta_of(&s, THEM);
        assert_eq!(m.unread_count, 0);
        assert_eq!(m.deep, Deep::Partial { cursor: None });
        assert_eq!(m.user_id, THEM);
    }

    #[test]
    fn index_ahead_of_file_recovers() {
        let o = owner();
        let s = open(&o, 1);
        add(&s, THEM, incoming("a1", 10));
        flush_owner_for_tests(&o).unwrap();
        forget_for_tests(&o);
        fs::remove_file(conv_path(&dir_of(&o), THEM)).unwrap();
        let m = meta_of(&s, THEM);
        assert_eq!(m.deep, Deep::Partial { cursor: None });
        assert_eq!(m.unread_count, 0);
    }

    #[test]
    fn flush_keeps_other_files_dirty() {
        let o = owner();
        let s = open(&o, 1);
        add(&s, "1000000001", incoming("a1", 10));
        add(&s, "1000000002", incoming("b1", 10));
        // The first conversation's file cannot be written: a folder sits where it goes.
        let blocked = conv_path(&dir_of(&o), "1000000001");
        fs::create_dir_all(&blocked).unwrap();
        assert!(flush_owner_for_tests(&o).is_err());
        assert!(dirty_for_tests(&o, "1000000001"));
        assert!(!dirty_for_tests(&o, "1000000002"));
        assert!(conv_path(&dir_of(&o), "1000000002").is_file());
        assert!(!dir_of(&o).join("index.json").exists());
        fs::remove_dir(&blocked).unwrap();
        assert!(flush_owner_for_tests(&o).is_ok());
        assert!(dir_of(&o).join("index.json").is_file());
    }

    #[test]
    fn import_never_exceeds_eight_resident() {
        let o = owner();
        let s = open(&o, 1);
        for n in 0..20 {
            add(&s, &format!("10000000{n:02}"), incoming(&format!("m{n}"), n as i64));
            assert!(resident_for_tests(&o) <= LRU_CAP);
        }
        flush_owner_for_tests(&o).unwrap();
        for n in 0..20 {
            assert_eq!(read(&s, &format!("10000000{n:02}"), |c| c.messages.len()).unwrap(), Some(1));
            assert!(resident_for_tests(&o) <= LRU_CAP);
        }
    }

    #[test]
    fn pinned_conversation_is_never_evicted() {
        let o = owner();
        let s = open(&o, 1);
        add(&s, THEM, outgoing("s1", 1));
        pin(&s, THEM, true).unwrap();
        for n in 0..12 {
            add(&s, &format!("20000000{n:02}"), incoming(&format!("m{n}"), n as i64));
        }
        let held = state().owners.get(&o).is_some_and(|e| e.loaded.contains_key(THEM));
        assert!(held);
        pin(&s, THEM, false).unwrap();
    }

    #[test]
    fn closed_owner_refuses_writes() {
        let o = owner();
        let s = open(&o, 1);
        add(&s, THEM, incoming("a1", 1));
        let _closing = close(&s);
        assert_eq!(update(&s, THEM, None, |_| ((), true)).err(), Some("closed".to_string()));
        assert_eq!(read(&s, THEM, |_| ()).err(), Some("closed".to_string()));
    }

    #[test]
    fn sign_out_and_back_in_same_account_writes() {
        let o = owner();
        let first = open(&o, 1);
        add(&first, THEM, incoming("a1", 1));
        let closing = close(&first);
        let second = open(&o, 2);
        add(&second, THEM, incoming("a2", 2));
        assert_eq!(read(&second, THEM, |c| c.messages.len()).unwrap(), Some(2));
        assert_eq!(update(&first, THEM, None, |_| ((), true)).err(), Some("closed".to_string()));
        // The old teardown leaves the reopened account in memory.
        assert!(drop_owner(closing).is_ok());
        assert!(resident_for_tests(&o) > 0);
    }

    #[test]
    fn teardown_writes_and_flushes_after_close() {
        let o = owner();
        let s = open(&o, 1);
        add(&s, THEM, outgoing("s1", 1));
        let closing = close(&s);
        update_closing(&closing, THEM, |c| {
            c.messages[0].failure = Some("You signed out before this was sent.".into());
            ((), true)
        })
        .unwrap();
        flush_closing(&closing).unwrap();
        assert_eq!(
            conv_file(&o, THEM).messages[0].failure.as_deref(),
            Some("You signed out before this was sent.")
        );
        assert!(drop_owner(closing).is_ok());
        assert_eq!(resident_for_tests(&o), 0);
        assert!(dir_of(&o).join("index.json").is_file());
    }

    #[test]
    fn failed_flush_keeps_owner() {
        let o = owner();
        let s = open(&o, 1);
        add(&s, THEM, incoming("a1", 1));
        let blocked = conv_path(&dir_of(&o), THEM);
        fs::create_dir_all(&blocked).unwrap();
        let back = drop_owner(close(&s));
        assert!(back.is_err());
        assert!(resident_for_tests(&o) > 0);
        fs::remove_dir(&blocked).unwrap();
        let closing = back.err().unwrap();
        assert!(drop_owner(closing).is_ok());
        assert_eq!(resident_for_tests(&o), 0);
    }

    #[test]
    fn unread_uses_later_marker() {
        let messages = vec![incoming("i1", 10), incoming("i2", 20), incoming("i3", 30)];
        let mut meta = ConversationMeta::new(THEM);
        meta.read_through = Some(Marker { id: "i1".into(), at_ms: 10 });
        meta.twitch_read = Some(Marker { id: "i2".into(), at_ms: 20 });
        recount(&mut meta, &messages);
        assert_eq!(meta.unread_count, 1);
        std::mem::swap(&mut meta.read_through, &mut meta.twitch_read);
        recount(&mut meta, &messages);
        assert_eq!(meta.unread_count, 1);
        meta.read_through = None;
        meta.twitch_read = None;
        recount(&mut meta, &messages);
        assert_eq!(meta.unread_count, 3);
    }

    #[test]
    fn unread_ignores_clock_skew() {
        let messages = vec![incoming("i1", 10), incoming("i2", 20), incoming("i3", 30)];
        let mut meta = ConversationMeta::new(THEM);
        // The marker's time came from a clock far ahead; its message still decides.
        meta.read_through = Some(Marker { id: "i2".into(), at_ms: 999_999 });
        recount(&mut meta, &messages);
        assert_eq!(meta.unread_count, 1);
        // A marker whose message is gone falls back to its time.
        meta.read_through = Some(Marker { id: "gone".into(), at_ms: 25 });
        recount(&mut meta, &messages);
        assert_eq!(meta.unread_count, 1);
    }

    #[test]
    fn sent_and_deleted_messages_are_never_unread() {
        let mut gone = incoming("i2", 20);
        gone.deleted = true;
        let messages = vec![incoming("i1", 10), gone, outgoing("s1", 30)];
        let mut meta = ConversationMeta::new(THEM);
        recount(&mut meta, &messages);
        assert_eq!(meta.unread_count, 1);
        assert!(meta.last_is_sent);
        let mut newest_gone = incoming("i3", 40);
        newest_gone.deleted = true;
        let messages = vec![incoming("i1", 10), newest_gone];
        recount(&mut meta, &messages);
        assert_eq!(meta.last_text, DELETED_TEXT);
    }

    #[test]
    fn rekey_writes_new_file_then_index_then_deletes_old() {
        let o = owner();
        let s = open(&o, 1);
        add(&s, "someone", incoming("a1", 10));
        flush_owner_for_tests(&o).unwrap();
        rekey(&s, "someone", THEM).unwrap();
        assert!(!conv_path(&dir_of(&o), "someone").exists());
        assert!(conv_path(&dir_of(&o), THEM).is_file());
        let keys = read_index(&s, |i| i.conversations.iter().map(|m| (m.key.clone(), m.user_id.clone())).collect::<Vec<_>>())
            .unwrap();
        assert_eq!(keys, vec![(THEM.to_string(), THEM.to_string())]);
        let on_disk: AccountIndex = serde_json::from_str(&fs::read_to_string(dir_of(&o).join("index.json")).unwrap()).unwrap();
        assert_eq!(on_disk.conversations.len(), 1);
        // Into a conversation that exists: messages join by id.
        add(&s, "someone2", incoming("a1", 10));
        update(&s, "someone2", None, |c| {
            c.messages.push(incoming("b2", 30));
            ((), true)
        })
        .unwrap();
        rekey(&s, "someone2", THEM).unwrap();
        let ids = read(&s, THEM, |c| c.messages.iter().map(|m| m.id.clone()).collect::<Vec<_>>()).unwrap().unwrap();
        assert_eq!(ids, vec!["a1".to_string(), "b2".to_string()]);
    }

    #[test]
    fn leftover_tmp_files_are_removed() {
        let o = owner();
        fs::create_dir_all(dir_of(&o).join("c")).unwrap();
        fs::write(dir_of(&o).join("index.json.tmp"), "x").unwrap();
        fs::write(dir_of(&o).join("c").join("1000000000.json.tmp"), "x").unwrap();
        let s = open(&o, 1);
        read_index(&s, |_| ()).unwrap();
        assert!(!dir_of(&o).join("index.json.tmp").exists());
        assert!(!dir_of(&o).join("c").join("1000000000.json.tmp").exists());
    }

    #[test]
    fn empty_owner_is_refused() {
        let s = open("", 1);
        assert_eq!(
            update(&s, THEM, Some(ConversationMeta::new(THEM)), |_| ((), true)).err(),
            Some("not signed in".to_string())
        );
    }

    #[test]
    fn write_now_persists_before_any_flush() {
        let o = owner();
        let s = open(&o, 1);
        add(&s, THEM, outgoing("s1", 1));
        write_now(&s, THEM).unwrap();
        assert_eq!(conv_file(&o, THEM).messages.len(), 1);
        assert!(!dirty_for_tests(&o, THEM));
    }

    #[test]
    fn remove_message_drops_one() {
        let o = owner();
        let s = open(&o, 1);
        add(&s, THEM, outgoing("s1", 1));
        add(&s, THEM, incoming("a1", 2));
        assert!(remove_message(&s, THEM, "s1").unwrap());
        assert!(!remove_message(&s, THEM, "s1").unwrap());
        assert_eq!(read(&s, THEM, |c| c.messages.len()).unwrap(), Some(1));
    }

    #[test]
    fn next_seq_only_grows() {
        let a = next_seq();
        let b = next_seq();
        assert!(b > a);
    }
}
