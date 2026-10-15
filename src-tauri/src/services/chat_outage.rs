//! A native shell's view of a Twitch chat outage. The web shells print the
//! outage from their own chat store; a native shell runs none of it, so while
//! one asks for rows drawn in Rust (`chat_row_view::enabled`), the core does it:
//!
//! - When the IRC session drops, a grace of 10 s passes first, so a fast
//!   rebuild (the supervisor usually needs a second or two) stays silent. An
//!   outage that outlives it posts one "Chat connection lost, reconnecting..."
//!   system row to each Twitch room.
//! - When the session is back, `chat-gap` names the rooms and the window to
//!   backfill: from 30 s before the outage began to 30 s after now, so the
//!   missed rows land in time order and overlaps are harmless.

use crate::models::chat_layout::{ChatMessage, MessageMetadata, MessageSegment};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

pub const EVENT: &str = "chat-gap";

const GRACE: Duration = Duration::from_secs(10);
const MARGIN_MS: i64 = 30_000;
pub const LOST_TEXT: &str = "Chat connection lost, reconnecting...";

/// Grows with every drop and every return, so a grace timer from an outage that
/// already ended does nothing.
static SEQ: AtomicU64 = AtomicU64::new(0);
/// When the current outage began, in milliseconds since the epoch.
static DOWN_SINCE: Mutex<Option<i64>> = Mutex::new(None);

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Gap {
    pub channels: Vec<String>,
    pub after_ms: i64,
    pub before_ms: i64,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// The window a backfill reads for an outage that began at `since`.
pub fn gap_window(since: i64, now: i64) -> (i64, i64) {
    (since - MARGIN_MS, now + MARGIN_MS)
}

/// The inline row an outage posts to a room.
pub fn lost_row(channel: &str, at_ms: i64) -> ChatMessage {
    let timestamp = at_ms.to_string();
    let id = format!("system-lost-{channel}-{at_ms}");
    let tags: HashMap<String, String> = [("id", id.as_str()), ("tmi-sent-ts", timestamp.as_str())]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    ChatMessage {
        id,
        user_id: String::new(),
        username: "System".to_string(),
        display_name: "System".to_string(),
        color: None,
        badges: Vec::new(),
        timestamp,
        content: LOST_TEXT.to_string(),
        provider: "twitch".to_string(),
        channel: channel.to_string(),
        emotes: Vec::new(),
        tags,
        layout: Default::default(),
        segments: vec![MessageSegment::Text { content: LOST_TEXT.to_string() }],
        metadata: MessageMetadata::default(),
    }
}

/// The IRC session dropped after it was established.
pub fn lost() {
    if !crate::services::chat_row_view::enabled() {
        return;
    }
    {
        let mut since = DOWN_SINCE.lock().unwrap_or_else(|e| e.into_inner());
        if since.is_none() {
            *since = Some(now_ms());
        }
    }
    let seq = SEQ.fetch_add(1, Ordering::SeqCst) + 1;
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(GRACE).await;
        if SEQ.load(Ordering::SeqCst) != seq {
            return;
        }
        let at = now_ms();
        for channel in crate::services::irc_service::IrcService::current_channel_list().await {
            let Ok(frame) = serde_json::to_string(&lost_row(&channel, at)) else { continue };
            crate::services::irc_service::post_channel_frame(&channel, frame).await;
        }
    });
}

/// The IRC session is up. After an outage, every room is told what to backfill.
pub async fn restored() {
    SEQ.fetch_add(1, Ordering::SeqCst);
    let Some(since) = DOWN_SINCE.lock().unwrap_or_else(|e| e.into_inner()).take() else { return };
    let channels = crate::services::irc_service::IrcService::current_channel_list().await;
    if channels.is_empty() {
        return;
    }
    let (after_ms, before_ms) = gap_window(since, now_ms());
    if let Some(app) = crate::services::providers::app_handle() {
        use tauri::Emitter;
        if let Err(e) = app.emit(EVENT, Gap { channels, after_ms, before_ms }) {
            log::debug!("[ChatOutage] emit failed: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_backfill_window_overlaps_both_ends_by_thirty_seconds() {
        assert_eq!(gap_window(100_000, 200_000), (70_000, 230_000));
    }

    #[test]
    fn the_lost_row_is_a_system_row_in_its_room() {
        let row = lost_row("xqc", 1_700_000_000_000);
        assert_eq!(row.username, "System");
        assert_eq!(row.channel, "xqc");
        assert_eq!(row.content, LOST_TEXT);
        assert_eq!(row.tags.get("tmi-sent-ts").map(String::as_str), Some("1700000000000"));
        assert!(row.id.starts_with("system-lost-xqc-"));
    }
}
