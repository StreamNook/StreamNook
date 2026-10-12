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

/// Where a Twitch badge sits in a chat row, lowest first, in the order Twitch
/// draws them: the chatter's role in the channel (broadcaster, staff, moderator,
/// VIP, artist), then their subscription, then what they earned in the channel
/// (bits, gifting, predictions), then global badges (Prime, Turbo, events).
/// Twitch's Chat Bot badge is one of the global ones: it says what the account
/// is, not its role here, so a bot that mods reads moderator, sub, then bot.
/// The IRC `badges` tag carries no order of its own; USERSTATE in particular
/// puts `subscriber` ahead of `moderator`. Kick rows keep Kick's own
/// `sort_order` and do not use this.
pub fn twitch_badge_rank(name: &str) -> u8 {
    match name {
        "broadcaster" => 0,
        "staff" | "admin" | "global_mod" => 1,
        "lead_moderator" | "moderator" => 2,
        "vip" => 3,
        "artist-badge" => 4,
        "founder" | "subscriber" => 5,
        "bits" | "bits-leader" | "bits-charity" | "sub-gifter" | "sub-gift-leader" | "predictions"
        | "hype-train" | "moments" | "clip-champ" => 6,
        _ => 7,
    }
}

/// Put Twitch badges in chat order. Stable, so badges of one rank keep the
/// order Twitch sent them in.
pub fn order_twitch_badges(badges: &mut [Badge]) {
    badges.sort_by_key(|b| twitch_badge_rank(&b.name));
}

/// The same order for a raw `name/version,...` tag (USERSTATE), so the string
/// the page repaints own rows from is already ordered.
pub fn order_twitch_badge_tag(tag: &str) -> String {
    let mut parts: Vec<&str> = tag.split(',').filter(|s| !s.is_empty()).collect();
    parts.sort_by_key(|p| twitch_badge_rank(p.split('/').next().unwrap_or("")));
    parts.join(",")
}

#[cfg(test)]
mod badge_order_tests {
    use super::*;

    #[test]
    fn roles_then_subscription_then_earned_then_global() {
        assert_eq!(
            order_twitch_badge_tag("subscriber/3012,premium/1,moderator/1,bits/1000,vip/1"),
            "moderator/1,vip/1,subscriber/3012,bits/1000,premium/1"
        );
        assert_eq!(order_twitch_badge_tag("glhf-pledge/1,broadcaster/1,founder/0"), "broadcaster/1,founder/0,glhf-pledge/1");
        assert_eq!(order_twitch_badge_tag(""), "");
    }

    #[test]
    fn a_bot_badge_follows_the_role_and_the_subscription() {
        assert_eq!(
            order_twitch_badge_tag("bot-badge/1,subscriber/12,moderator/1"),
            "moderator/1,subscriber/12,bot-badge/1"
        );
    }

    #[test]
    fn badges_of_one_rank_keep_twitchs_order() {
        assert_eq!(order_twitch_badge_tag("turbo/1,premium/1,glhf-pledge/1"), "turbo/1,premium/1,glhf-pledge/1");
    }
}

/// Twitch badge SET ids grouped into the categories a streamer can hide from
/// the OBS overlay, by category id. A set listed here belongs to exactly one
/// category. The overlay renderer carries the same table in
/// `src/components/overlay/overlayConfig.ts` (`TWITCH_BADGE_CATEGORIES`),
/// because the hosted overlay page runs without this backend; a vitest gate
/// reads this table and fails when the two differ, so edit both together.
pub const TWITCH_BADGE_CATEGORY_SETS: &[(&str, &[&str])] = &[
    ("subscriber", &["subscriber", "founder"]),
    ("predictions", &["predictions"]),
    ("gifted-subs", &["sub-gifter", "sub-gift-leader"]),
    ("bits", &["bits", "bits-leader", "bits-charity", "anonymous-cheerer"]),
    ("blossom", &["blossom-badge"]),
    ("bloom", &["bloom-badge"]),
    ("roles", &["lead_moderator", "moderator", "vip"]),
];

/// The catch-all category: every other Twitch set (Prime, Turbo, event and
/// game badges, hype train, clips leader, and the rest).
pub const TWITCH_BADGE_CATEGORY_OTHER: &str = "other";

/// Sets that never fall in a category, so no category switch can hide them:
/// the channel owner and Twitch's own staff. They follow only the overlay's
/// Badges switch.
const TWITCH_BADGE_SETS_NEVER_CATEGORISED: &[&str] = &["broadcaster", "staff", "admin", "global_mod"];

/// The hide category of one Twitch badge set id, or `None` for a set no
/// category covers (the broadcaster and staff badges, or an empty id).
pub fn twitch_badge_category(set_id: &str) -> Option<&'static str> {
    if set_id.is_empty() || TWITCH_BADGE_SETS_NEVER_CATEGORISED.contains(&set_id) {
        return None;
    }
    for (category, sets) in TWITCH_BADGE_CATEGORY_SETS {
        if sets.contains(&set_id) {
            return Some(category);
        }
    }
    Some(TWITCH_BADGE_CATEGORY_OTHER)
}

#[cfg(test)]
mod badge_category_tests {
    use super::*;

    #[test]
    fn set_ids_map_to_their_category() {
        let cases = [
            ("subscriber", Some("subscriber")),
            ("founder", Some("subscriber")),
            ("predictions", Some("predictions")),
            ("sub-gifter", Some("gifted-subs")),
            ("sub-gift-leader", Some("gifted-subs")),
            ("bits", Some("bits")),
            ("bits-leader", Some("bits")),
            ("bits-charity", Some("bits")),
            ("anonymous-cheerer", Some("bits")),
            ("blossom-badge", Some("blossom")),
            ("bloom-badge", Some("bloom")),
            ("lead_moderator", Some("roles")),
            ("moderator", Some("roles")),
            ("vip", Some("roles")),
            ("premium", Some("other")),
            ("turbo", Some("other")),
            ("hype-train", Some("other")),
            ("clips-leader", Some("other")),
            ("glhf-pledge", Some("other")),
            ("broadcaster", None),
            ("staff", None),
            ("admin", None),
            ("global_mod", None),
            ("", None),
        ];
        for (set_id, want) in cases {
            assert_eq!(twitch_badge_category(set_id), want, "set id {set_id:?}");
        }
    }

    #[test]
    fn every_set_sits_in_one_category() {
        let mut seen = std::collections::HashSet::new();
        for (category, sets) in TWITCH_BADGE_CATEGORY_SETS {
            assert_ne!(*category, TWITCH_BADGE_CATEGORY_OTHER);
            for set in *sets {
                assert!(seen.insert(*set), "{set} is listed twice");
                assert!(!TWITCH_BADGE_SETS_NEVER_CATEGORISED.contains(set));
            }
        }
    }
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
    /// The parent author's name as rows show it (`services::chatter_name`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub parent_shown_name: String,
    /// The quoted parent tokenized the way its own row would be: with the
    /// channel and platform it was said in. The page has only the home
    /// channel's sets, so in combined chat it could not do this for a reply
    /// on another platform's row. Absent on rows built before this existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_segments: Option<Vec<MessageSegment>>,
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
    /// The author's name as every row type shows it, before the viewer's own
    /// nickname override (`services::chatter_name`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub shown_name: String,
    /// The tick this row puts on the chat scrollbar, so a viewer scrolled away
    /// can see where the pings, subs and announcements are. Stamped by the
    /// rule engine with the rest of the highlight verdict; the page only draws.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scrollbar_mark: Option<ScrollbarMark>,
}

/// One row's scrollbar tick, stamped by the rule engine.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct ScrollbarMark {
    /// "mention" | "reply" | "highlight" | "announcement" | "sub" | "redeem"
    /// | "raid" | "first_time". Pinned rows are marked by the page from the
    /// channel's pinned-message ids, which arrive after the row does.
    pub kind: String,
    /// The colour the row itself is tinted with, where the user or the
    /// message chose one. Absent means the kind's theme colour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
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
