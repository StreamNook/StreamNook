use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Badge {
    pub name: String,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_url_1x: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_url_2x: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_url_4x: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct EmotePos {
    pub id: String,
    pub start: usize,
    pub end: usize,
    pub url: String,
    /// A Twitch chat GIF from the `gifs` tag rather than an emote from
    /// `emotes`. Same position arithmetic, different segment: the span is a
    /// bracketed description, so it gets no 7TV override and no text parsing.
    #[serde(default)]
    pub gif: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct LayoutResult {
    pub height: f32,
    pub width: f32,
    #[serde(default)]
    pub has_reply: bool,
    #[serde(default)]
    pub is_first_message: bool,
}

/// Reply information parsed from IRC tags
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ReplyInfo {
    pub parent_msg_id: String,
    pub parent_display_name: String,
    pub parent_msg_body: String,
    pub parent_user_id: String,
    pub parent_user_login: String,
}

/// Pre-computed message metadata - THE ENDGAME
/// All these fields are computed in Rust to eliminate frontend processing
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct MessageMetadata {
    /// Whether this is an ACTION message (/me command)
    #[serde(default)]
    pub is_action: bool,
    /// Whether this message mentions the current user (set by frontend context)
    #[serde(default)]
    pub is_mentioned: bool,
    /// Whether this is the user's first message in the channel
    #[serde(default)]
    pub is_first_message: bool,
    /// Pre-formatted timestamp string (e.g., "3:45 PM")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub formatted_timestamp: Option<String>,
    /// Pre-formatted timestamp with seconds (e.g., "3:45:30 PM")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub formatted_timestamp_with_seconds: Option<String>,
    /// Reply information if this is a reply message
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_info: Option<ReplyInfo>,
    /// Source room ID for shared chat messages
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_room_id: Option<String>,
    /// Whether this message is from shared chat (different from current room)
    #[serde(default)]
    pub is_from_shared_chat: bool,
    /// Message type for special messages (sub, resub, subgift, etc.)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub msg_type: Option<String>,
    /// Bits amount if this is a cheer message
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bits_amount: Option<u32>,
    /// System message for subscriptions/donations
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_message: Option<String>,
    /// True when the chat rule engine (services::chat_rules) has evaluated
    /// this message: the row treats the fields below as authoritative and
    /// runs no matcher of its own. False on optimistic local rows.
    #[serde(default)]
    pub rules_evaluated: bool,
    /// The message replies to one of ours.
    #[serde(default)]
    pub is_reply_to_me: bool,
    /// First matching user highlight rule (phrase, user, or badge), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub highlight: Option<HighlightStamp>,
    /// Built-in event tint (raid, returning chatter, first message, self).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub built_in: Option<BuiltInStamp>,
    /// Ids of the user's saved filters this message satisfies. A pane bound
    /// to a filter id shows only rows carrying it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub filter_ids: Vec<String>,
    /// Low-trust status of the sender in this channel, from EventSub
    /// `channel.suspicious_user.*`: "monitored" | "restricted". Moderators only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suspicious: Option<String>,
    /// Row came from a history backfill, not live delivery. Lets the list
    /// dim scrollback (`chat_design.backfill_opacity`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub from_backfill: bool,
}

/// A matched highlight rule, stamped by the Rust rule engine.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct HighlightStamp {
    pub rule_id: String,
    /// "phrase" | "user" | "badge"
    pub kind: String,
    pub color: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sound_id: Option<String>,
    #[serde(default)]
    pub cooldown_ms: u64,
}

/// A built-in event highlight, stamped by the Rust rule engine.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct BuiltInStamp {
    /// "raider" | "returning" | "first_time" | "self"
    pub kind: String,
    pub color: String,
    pub label: String,
}

/// Represents a parsed segment of a chat message
/// These segments are pre-calculated in Rust for instant React rendering
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum MessageSegment {
    Text {
        content: String,
    },
    Emote {
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        emote_id: Option<String>,
        emote_url: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_zero_width: Option<bool>,
        /// FFZ modifier bitmask; present only on FFZ modifier emotes. The
        /// renderer applies these effects to the preceding emote.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        modifier_flags: Option<u32>,
        /// Set on a 7TV PERSONAL emote — one belonging to the sender rather than
        /// the room, which therefore renders in channels that never added it.
        /// Carried so a renderer can offer to suppress them; absent means the
        /// emote came from the channel's own sets.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        is_personal: Option<bool>,
    },
    Emoji {
        content: String,
        emoji_url: String,
    },
    Link {
        content: String,
        url: String,
    },
    /// Cheermote segment for animated bits (e.g., Cheer500)
    Cheermote {
        content: String,       // Original text, e.g., "Cheer500"
        prefix: String,        // e.g., "cheer" (lowercase)
        bits: u32,             // e.g., 500
        tier: String,          // e.g., "100" (tier ID for URL)
        color: String,         // e.g., "#9c3ee8" (tier color)
        cheermote_url: String, // Animated GIF URL
    },
    /// A Twitch chat GIF (GIPHY-backed, sent by Tier 2 and 3 subscribers).
    /// The message text carries a bracketed description at the GIF's span,
    /// e.g. `[Y A Y Yes GIF by Djemilah Birnie]`; that stays in `content` for
    /// search, logs and tooltips while the renderer draws the asset. Twitch
    /// requires the URL be used exactly as sent.
    Gif {
        content: String,
        gif_id: String,
        gif_url: String,
    },
}

/// Default source platform for messages deserialized from pre-multi-platform
/// state. Keeps old cached JSON (which has no `provider`) valid as Twitch.
fn default_provider() -> String {
    "twitch".to_string()
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ChatMessage {
    pub id: String,
    pub user_id: String,
    pub username: String,
    pub display_name: String,
    pub color: Option<String>,
    pub badges: Vec<Badge>,
    pub timestamp: String,
    pub content: String,
    /// Source platform: "twitch" | "kick" | "youtube" | "rumble" | "tiktok" | "x".
    /// Defaults to twitch so existing constructions and cached JSON stay valid.
    #[serde(default = "default_provider")]
    pub provider: String,
    /// Source channel name for multi-stream chat routing
    #[serde(default)]
    pub channel: String,
    /// Native-emote positions, kept internally for segment parsing but never
    /// serialized: the frontend derives its copy from the `emotes` IRC tag and
    /// has no reader of this field, which only duplicated `segments` on the
    /// wire for every message.
    #[serde(default, skip_serializing)]
    pub emotes: Vec<EmotePos>,
    pub tags: HashMap<String, String>,
    /// Placeholder only - the frontend's ResizeObserver measurement is
    /// authoritative and nothing reads this off the wire, so it stays local.
    #[serde(default, skip_serializing)]
    pub layout: LayoutResult,
    /// Pre-parsed message segments ready for rendering
    /// This is the "endgame" - all parsing done in Rust, zero regex on main thread
    #[serde(default)]
    pub segments: Vec<MessageSegment>,
    /// Pre-computed metadata - THE ENDGAME
    /// All message analysis done in Rust, frontend just renders
    #[serde(default)]
    pub metadata: MessageMetadata,
}
