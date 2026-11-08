//! The whisper archive on disk. Card 7.2 writes the store itself; this file
//! holds its types so the commands compile.

use crate::services::mention_inbox::MentionPart;
use serde::{Deserialize, Serialize};

pub const INDEX_VERSION: u32 = 2;
pub const LRU_CAP: usize = 8;

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
    pub timestamp: i64,
    pub is_sent: bool,
    #[serde(default)]
    pub parts: Vec<MentionPart>,
    #[serde(default)]
    pub deleted: bool,
    #[serde(default)]
    pub edited_ms: Option<i64>,
    #[serde(default)]
    pub origin: Origin,
    #[serde(default)]
    pub delivery: Option<Delivery>,
    #[serde(default)]
    pub sent_local_ms: Option<i64>,
    #[serde(default)]
    pub got_204: bool,
    #[serde(default)]
    pub checks: u8,
    #[serde(default)]
    pub failure: Option<String>,
    #[serde(default)]
    pub cut_to: Option<u32>,
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

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct FileStamp {
    pub len: u64,
    pub modified_ns: u64,
}

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

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct ConversationFile {
    pub version: u32,
    pub key: String,
    #[serde(default)]
    pub meta: Option<ConversationMeta>,
    pub messages: Vec<StoredWhisper>,
}

pub struct Conversation {
    pub meta: ConversationMeta,
    pub messages: Vec<StoredWhisper>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scope {
    pub owner: String,
    pub generation: u64,
}

pub struct Closing(pub(crate) Scope);

impl Closing {
    pub fn scope(&self) -> &Scope {
        &self.0
    }
}

pub fn flush_now() -> Result<(), String> {
    Ok(())
}
