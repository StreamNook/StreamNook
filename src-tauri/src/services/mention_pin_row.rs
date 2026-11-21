//! The row a pinned mention is drawn as (SN-284).
//!
//! LANE R2 OWNS THIS FILE. Lane R1 (the pins service, the inbox pin rule, the
//! commands and the stack) wrote only the contract below and a text-row stub
//! behind it, so the Rust core compiles and tests alone. R2 replaces each stub
//! body (the signatures are what `services::mention_pins` and
//! `services::mention_inbox` call):
//!
//! - `snapshot_of(msg)`: the filed message as raw JSON for `Inbox::pin_rows`
//!   (every tag kept, segments intact, lean path over 8 KB, the raw IRC line
//!   beside it for Twitch).
//! - `cached(entry)`: the built row for a live pin, from a cache keyed by the
//!   entry id; `None` until it is built (the answer leaves the pin out until
//!   then; every row has the 10 s fallback to the text row).
//! - `refresh_rows(runs)`: called by the pins tick every 2 s while a live pin
//!   exists; rebuilds what changed and answers whether any row's hash changed
//!   (the pins revision then bumps).

use std::hash::{Hash, Hasher};

use serde_json::value::RawValue;

use crate::models::chat_layout::ChatMessage;
use crate::services::mention_inbox::{MentionEntry, MentionPart, PinRun};

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

/// R2: the filed message's snapshot. Stub: the message as the chat serialized it.
pub fn snapshot_of(msg: &ChatMessage) -> Option<Box<RawValue>> {
    serde_json::value::to_raw_value(msg).ok()
}

/// R2: the built row for a live pin, or `None` until it exists. Stub: the text
/// row, built on the spot from the entry's `text` and `parts`.
pub fn cached(entry: &MentionEntry) -> Option<PinRow> {
    Some(text_row(entry))
}

/// R2: rebuild the rows of `runs` that changed (new emote tables, cosmetics,
/// badge sets, settings); true when any row's hash changed. Stub: nothing to rebuild.
pub fn refresh_rows(_runs: &[PinRun]) -> bool {
    false
}

/// The fallback row: the entry's own words and emotes, drawn as chat would
/// with nothing decorated. R2 keeps this as the 10 s fallback.
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
