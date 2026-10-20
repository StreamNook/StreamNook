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
    /// The row worked out for a native renderer (`services::chat_row_view`):
    /// readable name colour, badge art, paint and the grouped body. Stamped
    /// only while a native shell asked for it; the web renderers derive the
    /// same things themselves and never see it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<RowView>,
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
    /// Keep tinted: the row holds the rule's color while on screen instead
    /// of only flashing.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hold: bool,
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
        /// The card kind this link's shape all but always gets (`expected_card`:
        /// "youtube", "discord", "generic"...); absent when it may have none. A
        /// shell holds exactly that card's room while it loads and nothing for
        /// other links, so none flashes a placeholder and vanishes.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        card: Option<String>,
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

/// A chat row as a native renderer draws it, built once in Rust so the shell
/// never re-derives colours, badge art or emote grouping.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct RowView {
    /// The author colour after the readability pass (`chat_design.name_color_adjustment`).
    pub name_color: String,
    /// Badges in draw order, already filtered by the badge settings.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub badges: Vec<ViewBadge>,
    /// The 7TV paint the author wears, when known at parse time. Later
    /// answers arrive as a `ROW_COSMETICS` frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paint: Option<PaintView>,
    /// The body, grouped: overlays and modifiers ride their target emote.
    #[serde(default)]
    pub spans: Vec<ViewSpan>,
    /// The quoted parent of a reply, grouped the same way.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_spans: Option<Vec<ViewSpan>>,
    /// The StreamNook Atmosphere the author's rows wear (an id in the catalog
    /// `sn_atmospheres` serves), when known at parse time. Later answers and
    /// changes arrive as a `ROW_MEMBER` frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub atmosphere: Option<String>,
    /// The CS2 Major Cologne look the author's rows wear instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cologne: Option<crate::services::member_theme::Cologne>,
    /// A Shared Chat partner's row: the channel it came from (face, its own
    /// color floored for the theme), when already known as the row is stamped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<crate::services::shared_chat_source::SharedSource>,
    /// The viewer's own nickname for the author, shown in place of their
    /// name (`chat_customization.user_overrides`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nickname: Option<String>,
    /// Link preview cards under the body, in link order (`chat_design.link_previews`).
    /// One still being read arrives as a `ROW_PREVIEW` frame.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub previews: Vec<RowPreview>,
    /// The chat event this row is, as `provider:category` (`twitch:subscription`,
    /// `kick:follow`), or None for an ordinary message. The shell hides the row
    /// while that key is in `chat_events.hidden_provider_events`, the same keys
    /// the settings page writes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    /// The art a subscription or gift notice leads with, as the desktop card
    /// draws it: Twitch's own gift (the Sub Gifter badge). None for a Prime sub
    /// (the shell draws Twitch's Prime crown) and for every other row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_art: Option<String>,
}

/// One link's card on a row.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct RowPreview {
    pub url: String,
    /// The card, or None while it is still being read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<crate::commands::link_preview::LinkPreview>,
    /// While pending: this link's shape almost always has a card, so the shell
    /// holds its room now.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reserve: bool,
    /// Keep the link in the text beside its card (`chat_design.link_preview_keep_link`);
    /// otherwise the card stands for it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub keep_link: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct ViewBadge {
    pub url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    /// "twitch" | "7tv" | another provider name.
    pub provider: String,
}

/// One piece of a drawn body.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ViewSpan {
    Text {
        text: String,
    },
    /// `@name`; `color` is the named chatter's readable colour when they have
    /// spoken in a room this app shows.
    Mention {
        text: String,
        login: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        color: Option<String>,
    },
    Link {
        /// What the row prints (shortened per `chat_design.shorten_links`).
        label: String,
        url: String,
    },
    Emoji {
        text: String,
        url: String,
    },
    Emote {
        #[serde(flatten)]
        emote: ViewEmote,
        /// Zero-width emotes stacked over this one, bottom first.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        overlays: Vec<ViewEmote>,
        /// Effect bits the attached modifiers lend the group (the modifier
        /// table in `emote_service`, routing bits removed).
        #[serde(default, skip_serializing_if = "is_zero")]
        effects: u32,
    },
    Cheer {
        text: String,
        bits: u32,
        color: String,
        url: String,
    },
    Gif {
        text: String,
        url: String,
    },
}

fn is_zero(v: &u32) -> bool {
    *v == 0
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct ViewEmote {
    pub code: String,
    pub url: String,
    /// The emote's size at 1x in CSS pixels, when the provider says, so the
    /// row lays out once before the image arrives.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub personal: bool,
    /// A modifier that draws nothing itself and only lends its effect.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
}

/// A 7TV paint reduced to what a renderer fills a name with.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PaintView {
    pub id: String,
    /// "linear" | "radial" | "image" | "solid"
    pub kind: String,
    /// Degrees, CSS convention (0 points up), for linear paints.
    #[serde(default)]
    pub angle: f32,
    #[serde(default)]
    pub repeating: bool,
    /// Gradient stops, `at` from 0 to 1, colours as `#rrggbbaa`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stops: Vec<PaintStop>,
    /// The animated image of an image paint, never the `_static` still.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
    /// Drop shadows, outermost last.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shadows: Vec<PaintShadow>,
    /// The paint's name, for the surfaces that show it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Every gradient and image layer, top first as CSS stacks them, for
    /// renderers that draw the whole paint rather than its first fill.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layers: Vec<PaintLayerView>,
    /// The first single colour layer, drawn under the layers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_color: Option<String>,
    /// The lowest layer opacity under 1, applied to the whole name.
    #[serde(default = "full_opacity", skip_serializing_if = "is_full_opacity")]
    pub opacity: f32,
}

impl Default for PaintView {
    fn default() -> Self {
        Self {
            id: String::new(),
            kind: String::new(),
            angle: 0.0,
            repeating: false,
            stops: Vec::new(),
            image_url: None,
            shadows: Vec::new(),
            name: String::new(),
            layers: Vec::new(),
            base_color: None,
            opacity: 1.0,
        }
    }
}

fn full_opacity() -> f32 {
    1.0
}

fn is_full_opacity(v: &f32) -> bool {
    *v >= 1.0
}

/// One gradient or image layer of a paint. Each spans the whole name.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct PaintLayerView {
    /// "linear" | "radial" | "image"
    pub kind: String,
    #[serde(default)]
    pub angle: f32,
    #[serde(default)]
    pub repeating: bool,
    /// Radial only: true for a circle, else an ellipse.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub circle: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stops: Vec<PaintStop>,
    /// The animated image whenever the layer animates, never the `_static` still.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct PaintStop {
    pub at: f32,
    pub color: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct PaintShadow {
    pub x: f32,
    pub y: f32,
    pub blur: f32,
    pub color: String,
}
