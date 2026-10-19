//! The mentions inbox, for every app's Mentions screen. All of it lives in
//! `services::mention_inbox`; these only hand it to the page or the shell.

use serde::Serialize;

use crate::services::mention_inbox::{self, MentionEntry};

const PAGE_DEFAULT: usize = 50;
const PAGE_MAX: usize = 200;

#[derive(Serialize)]
pub struct MentionPage {
    /// Newest first.
    pub entries: Vec<MentionEntry>,
    pub has_more: bool,
    pub unread_total: u32,
}

/// One page of the signed-in account's mentions, newest first, older than
/// `before_ms` when given. Empty when nobody is signed in to Twitch.
#[tauri::command]
pub async fn get_mention_inbox(before_ms: Option<i64>, limit: Option<u32>) -> MentionPage {
    let limit = limit.map(|l| l as usize).unwrap_or(PAGE_DEFAULT).clamp(1, PAGE_MAX);
    match mention_inbox::page(before_ms, limit) {
        Some((entries, has_more, unread_total)) => MentionPage { entries, has_more, unread_total },
        None => MentionPage { entries: Vec::new(), has_more: false, unread_total: 0 },
    }
}

/// Mark these entries read, or all of them when `ids` is absent.
#[tauri::command]
pub async fn mark_mentions_read(ids: Option<Vec<String>>) {
    mention_inbox::mark_read(ids);
}

#[tauri::command]
pub async fn remove_mention(id: String) {
    mention_inbox::remove(&id);
}

#[tauri::command]
pub async fn clear_mention_inbox() {
    mention_inbox::clear();
}

/// Opening an entry (a row tap or a notification click): marks it read and
/// returns it, so the page can open its chat and look for the message.
#[tauri::command]
pub async fn open_mention(id: String) -> Option<MentionEntry> {
    mention_inbox::open(&id)
}

/// The chats on screen right now (room keys). Their mentions are read, and
/// new ones there arrive already read.
#[tauri::command]
pub async fn set_visible_mention_rooms(rooms: Vec<String>) {
    mention_inbox::set_visible_rooms(rooms);
}
