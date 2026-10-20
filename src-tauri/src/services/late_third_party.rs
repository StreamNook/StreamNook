//! Chat-client badges (FFZ, BetterTTV, Chatterino and the rest) for rows a
//! native shell got before the badge feeds were in.
//!
//! The row view reads those badges from the badge service's cache without
//! waiting (`third_party_worn_now`), and the service starts a moment after
//! launch and reads every feed first. Rows stamped before then go out without
//! the badges a desktop page shows, and nothing would stamp them again. Their
//! raw lines are kept here (a bounded few) and, once the service is up, parsed
//! again; the rows that now wear such a badge are patched (`HISTORY_SEGMENTS`
//! carrying the view).

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use serde_json::{json, Value};

use crate::models::chat_layout::ChatMessage;

/// The most rows kept while waiting: a cold start's first seconds of chat.
const MAX_PENDING: usize = 300;

static PENDING: Mutex<VecDeque<(String, String)>> = Mutex::new(VecDeque::new());

/// Keeps a Twitch row's raw line when it was stamped before the badge feeds were in.
pub fn note_if_early(msg: &ChatMessage, raw: &str) {
    if !crate::services::chat_row_view::enabled() || crate::commands::badge_service::third_party_ready() {
        return;
    }
    let twitch = msg.provider.is_empty() || msg.provider == "twitch";
    if !twitch || msg.user_id.is_empty() {
        return;
    }
    let channel = msg.channel.trim_start_matches('#').to_lowercase();
    if let Ok(mut pending) = PENDING.lock() {
        if pending.len() >= MAX_PENDING {
            pending.pop_front();
        }
        pending.push_back((channel, raw.to_string()));
    }
}

/// Whether a stamped row wears a chat-client badge (not the platform's, 7TV's
/// or StreamNook's).
fn wears_chat_client_badge(msg: &ChatMessage) -> bool {
    msg.metadata.view.as_ref().is_some_and(|view| {
        view.badges
            .iter()
            .any(|b| !matches!(b.provider.as_str(), "" | "twitch" | "kick" | "youtube" | "tiktok" | "7tv" | "streamnook"))
    })
}

/// Parses the kept rows again now that the feeds are in and patches the ones
/// that gained a badge. Called once the badge service is ready.
pub async fn flush() {
    let rows: Vec<(String, String)> = match PENDING.lock() {
        Ok(mut pending) => pending.drain(..).collect(),
        Err(_) => return,
    };
    if rows.is_empty() {
        return;
    }
    let mut by_channel: HashMap<String, Vec<Value>> = HashMap::new();
    for (channel, raw) in rows {
        let Some(mut msg) = crate::services::irc_service::IrcService::parse_live_line(&raw) else { continue };
        crate::services::chat_row_view::stamp(&mut msg);
        if !wears_chat_client_badge(&msg) {
            continue;
        }
        let mut row = json!({
            "id": msg.id,
            "segments": msg.segments,
            "parent_segments": msg.metadata.reply_info.as_ref().and_then(|r| r.parent_segments.as_ref()),
        });
        if let Some(view) = &msg.metadata.view {
            row["view"] = json!(view);
        }
        by_channel.entry(channel).or_default().push(row);
    }
    for (channel, rows) in by_channel {
        log::debug!("[ChatRowView] {channel}: {} early rows now wear their chat-client badges", rows.len());
        let frame = json!({ "type": "HISTORY_SEGMENTS", "channel": channel, "rows": rows });
        crate::services::irc_service::post_channel_frame(&channel, frame.to_string()).await;
    }
}
