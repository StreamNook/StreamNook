//! The mentions inbox, for every app's Mentions screen. All of it lives in
//! `services::mention_inbox`; these only hand it to the page or the shell.

use serde::Serialize;

use crate::services::mention_inbox::{self, InboxFilter, InboxRow, MentionEntry};

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

#[derive(Serialize)]
pub struct InboxChip {
    pub id: &'static str,
    pub unread: u32,
}

#[derive(Serialize)]
pub struct InboxView {
    /// Newest first; identical messages in a row folded into one.
    pub rows: Vec<InboxRow>,
    pub has_more: bool,
    /// What `before_ms` asks for to get the next page.
    pub before_ms: Option<i64>,
    pub unread_total: u32,
    /// The chips the phones show, in order, with their unread counts.
    pub chips: Vec<InboxChip>,
}

/// The phones' inbox: one page of the timeline for a chip (`all`,
/// `mentions`, `highlights`), with the senders' pictures that are cached.
/// The rest are looked up and land as `channel-avatars`; read again then.
#[tauri::command]
pub async fn get_inbox_view(filter: Option<InboxFilter>, before_ms: Option<i64>, limit: Option<u32>) -> InboxView {
    let limit = limit.map(|l| l as usize).unwrap_or(PAGE_DEFAULT).clamp(1, PAGE_MAX);
    let Some((mut rows, has_more, cursor, unread_total, chips)) =
        mention_inbox::view(filter.unwrap_or_default(), before_ms, limit)
    else {
        return InboxView {
            rows: Vec::new(),
            has_more: false,
            before_ms: None,
            unread_total: 0,
            chips: InboxFilter::CHIPS.iter().map(|f| InboxChip { id: f.id(), unread: 0 }).collect(),
        };
    };
    let pictures = mention_inbox::avatars_for(rows.iter().map(|r| &r.entry));
    for row in &mut rows {
        row.avatar = mention_inbox::avatar_of(&row.entry, &pictures);
    }
    InboxView {
        rows,
        has_more,
        before_ms: cursor,
        unread_total,
        chips: chips.into_iter().map(|(f, unread)| InboxChip { id: f.id(), unread }).collect(),
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
/// returns it, so the page can open its chat and look for the message. A
/// mention pushed while the app was away may not be here yet: it is pulled
/// from the mention server first.
#[tauri::command]
pub async fn open_mention(id: String) -> Option<MentionEntry> {
    if let Some(entry) = mention_inbox::open(&id) {
        return Some(entry);
    }
    match crate::services::mention_watch::client::pull().await {
        Ok(_) => mention_inbox::open(&id),
        Err(e) => {
            log::warn!("[MentionWatch] pull for an opened mention failed: {e}");
            None
        }
    }
}

/// Pulls what the mention server found while the app was away into the
/// inbox now. Answers how many entries were new (0 with the switch off).
#[tauri::command]
pub async fn mention_watch_pull() -> Result<u32, String> {
    crate::services::mention_watch::client::pull().await
}

/// A mention push arrived while the app is in front (the iPhone asks before
/// showing its banner): whether it should show, by the same rules as the
/// app's own notices. Its entry is pulled into the inbox either way.
#[tauri::command]
pub async fn mention_push_arrived(id: String, room: String) -> bool {
    let show = crate::services::mention_watch::claim_push(
        &crate::services::mention_watch::client::saved_settings(),
        &id,
        &room,
    );
    crate::services::mention_watch::client::pull_soon();
    show
}

/// The chats on screen right now (room keys). Their mentions are read, and
/// new ones there arrive already read.
#[tauri::command]
pub async fn set_visible_mention_rooms(rooms: Vec<String>) {
    mention_inbox::set_visible_rooms(rooms);
}
