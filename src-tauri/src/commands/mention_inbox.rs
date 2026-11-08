//! The mentions inbox, for every app's Mentions screen. All of it lives in
//! `services::mention_inbox`; these only hand it to the page or the shell.

use serde::Serialize;

use crate::services::mention_inbox::{self, InboxFilter, InboxItem, MentionEntry, Surface};

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
/// `before_ms` when given. `unseen_only` (the desktop island) leaves out the
/// ones that arrived in a chat on screen. Empty when nobody is signed in to
/// Twitch.
#[tauri::command]
pub async fn get_mention_inbox(before_ms: Option<i64>, limit: Option<u32>, unseen_only: Option<bool>) -> MentionPage {
    let limit = limit.map(|l| l as usize).unwrap_or(PAGE_DEFAULT).clamp(1, PAGE_MAX);
    match mention_inbox::page(before_ms, limit, unseen_only.unwrap_or(false)) {
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
    /// Newest first: mention rows (identical messages in a row folded into
    /// one) and whisper conversations, as the chip asks.
    pub rows: Vec<InboxItem>,
    pub has_more: bool,
    /// What `cursor` asks for to get the next page; opaque to the shells.
    pub cursor: Option<String>,
    /// The inbox counts' last published total (see `inbox_counts`).
    pub unread_total: u32,
    /// The chips to show, in order, with their unread counts.
    pub chips: Vec<InboxChip>,
    /// The revision of `unread_total`; a shell keeps the highest it has seen.
    pub counts_rev: u64,
    /// The island's count from the same snapshot (`Counts::island`), so the
    /// island has its count at mount before any `inbox-changed` arrives.
    pub island_unread: u32,
    /// The whole snapshot the fields above come from, so a shell can seed every
    /// count it shows (the Whispers window reads `whispers`) from one answer.
    pub counts: crate::services::inbox_counts::Counts,
}

/// One page of an inbox for a chip (`all`, `mentions`, `highlights`,
/// `whispers`) on a surface (`phone`, `island`, `multichat`), with the
/// senders' pictures that are cached. The rest are looked up and land as
/// `channel-avatars`; read again then.
#[tauri::command]
pub async fn get_inbox_view(
    filter: Option<InboxFilter>,
    cursor: Option<String>,
    limit: Option<u32>,
    surface: Option<Surface>,
) -> InboxView {
    let limit = limit.map(|l| l as usize).unwrap_or(PAGE_DEFAULT).clamp(1, PAGE_MAX);
    let page = mention_inbox::view_page(filter.unwrap_or_default(), surface.unwrap_or_default(), cursor.as_deref(), limit);
    let mut rows = page.rows;
    // Whisper rows carry whether their person is live, from the Following
    // list (no network).
    let mut whispers: Vec<crate::services::whisper_inbox::RowMeta> = rows
        .iter()
        .filter_map(|r| match r {
            InboxItem::Whisper { whisper, .. } => Some(whisper.clone()),
            InboxItem::Mention { .. } => None,
        })
        .collect();
    if !whispers.is_empty() {
        crate::services::whisper_live::fill(&mut whispers).await;
        let mut filled = whispers.into_iter();
        for row in &mut rows {
            if let InboxItem::Whisper { whisper, .. } = row {
                if let Some(f) = filled.next() {
                    whisper.live = f.live;
                }
            }
        }
    }
    let pictures = mention_inbox::avatars_for(rows.iter().filter_map(|r| match r {
        InboxItem::Mention { entry, .. } => Some(entry),
        InboxItem::Whisper { .. } => None,
    }));
    for row in &mut rows {
        if let InboxItem::Mention { entry, avatar, .. } = row {
            *avatar = mention_inbox::avatar_of(entry, &pictures);
        }
    }
    let counts = crate::services::inbox_counts::snapshot();
    InboxView {
        rows,
        has_more: page.has_more,
        cursor: page.cursor,
        unread_total: counts.unread_total,
        chips: page.chips.into_iter().map(|(f, unread)| InboxChip { id: f.id(), unread }).collect(),
        counts_rev: counts.rev,
        island_unread: counts.island,
        counts: counts.clone(),
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
    // A pull already running is waited for (its page may hold this very
    // mention), but a tap never hangs on a slow network.
    match tokio::time::timeout(std::time::Duration::from_secs(15), crate::services::mention_watch::client::pull()).await {
        Ok(Ok(_)) => mention_inbox::open(&id),
        Ok(Err(e)) => {
            log::warn!("[MentionWatch] pull for an opened mention failed: {e}");
            None
        }
        Err(_) => {
            log::warn!("[MentionWatch] pull for an opened mention timed out");
            mention_inbox::open(&id)
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
