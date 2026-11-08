//! Settings > Chat's chat preview: sample chat as real Twitch IRC lines, run
//! through the path a live line takes (parse, your rules, the row view), so
//! every row is exactly the row chat would draw for that line: cheers, events,
//! emotes, badges, replies and all. Nothing is recorded (no history, no inbox,
//! no sound); a row your rules hide is left out, as chat leaves it out.
//!
//! The lines are made up but shaped like Twitch's: real tag sets, real global
//! emote ids, `bits`, `first-msg`, `reply-parent-*`, USERNOTICE events. Words
//! that are 7TV/BTTV/FFZ emotes become emotes when the channel the preview is
//! parsed for has its emote table loaded, as they would in that chat.

use crate::models::chat_layout::{ChatMessage, LayoutResult};
use crate::services::chat_rules::ChatRules;
use crate::services::irc_service::IrcService;
use serde::{Deserialize, Serialize};

/// An old send time: nothing treats these rows as live arrivals (sounds,
/// title flashes). `fresh` rows get the current time so their arrival effects
/// (a mention's flash) play.
const OLD_TS: i64 = 1_700_000_000_000;

/// What the page knows that the lines need: who you are, the chat the rows are
/// parsed for, and the highlights to show off.
#[derive(Debug, Default, Deserialize)]
pub struct PreviewAsk {
    pub scene: String,
    /// The chat whose emotes and badges the rows use (the open one, else yours).
    #[serde(default)]
    pub channel: String,
    #[serde(default)]
    pub room_id: String,
    /// Who you are, as the page knows it (your rules may not have heard yet).
    #[serde(default)]
    pub you_login: String,
    #[serde(default)]
    pub you_id: String,
    #[serde(default)]
    pub you_name: String,
    #[serde(default)]
    pub you_color: String,
    /// Replay arrival effects (a look setting just changed).
    #[serde(default)]
    pub fresh: bool,
    /// Your first plain highlight phrase, user and badge (`set/version`).
    #[serde(default)]
    pub phrase: Option<String>,
    #[serde(default)]
    pub highlight_user: Option<String>,
    #[serde(default)]
    pub highlight_badge: Option<String>,
}

fn escape_tag(v: &str) -> String {
    v.replace('\\', "\\\\").replace(';', "\\:").replace(' ', "\\s")
}

/// `emotes=` for the Twitch emote words in `text`, by character index as
/// Twitch sends it.
fn emotes_tag(text: &str, emotes: &[(&str, &str)]) -> String {
    let words: Vec<(usize, &str)> = {
        let mut out = Vec::new();
        let mut idx = 0usize;
        for w in text.split(' ') {
            out.push((idx, w));
            idx += w.chars().count() + 1;
        }
        out
    };
    emotes
        .iter()
        .filter_map(|(name, id)| {
            let spans: Vec<String> = words
                .iter()
                .filter(|(_, w)| w == name)
                .map(|(at, w)| format!("{}-{}", at, at + w.chars().count() - 1))
                .collect();
            (!spans.is_empty()).then(|| format!("{}:{}", id, spans.join(",")))
        })
        .collect::<Vec<_>>()
        .join("/")
}

struct Line<'a> {
    id: &'a str,
    login: &'a str,
    name: &'a str,
    color: &'a str,
    user_id: &'a str,
    badges: &'a str,
    badge_info: &'a str,
    text: &'a str,
    emotes: &'a [(&'a str, &'a str)],
    tags: Vec<(&'a str, String)>,
    ts: i64,
}

impl<'a> Line<'a> {
    fn new(id: &'a str, login: &'a str, color: &'a str, text: &'a str) -> Self {
        Line {
            id,
            login,
            name: login,
            color,
            user_id: "",
            badges: "",
            badge_info: "",
            text,
            emotes: &[],
            tags: Vec::new(),
            ts: OLD_TS,
        }
    }
    fn badges(mut self, badges: &'a str, info: &'a str) -> Self {
        self.badges = badges;
        self.badge_info = info;
        self
    }
    fn emotes(mut self, emotes: &'a [(&'a str, &'a str)]) -> Self {
        self.emotes = emotes;
        self
    }
    fn tag(mut self, k: &'a str, v: impl Into<String>) -> Self {
        self.tags.push((k, v.into()));
        self
    }

    fn base_tags(&self, ask: &PreviewAsk) -> Vec<(String, String)> {
        let user_id = if self.user_id.is_empty() {
            format!("uid-{}", self.login)
        } else {
            self.user_id.to_string()
        };
        let mut tags = vec![
            ("badge-info".to_string(), self.badge_info.to_string()),
            ("badges".to_string(), self.badges.to_string()),
            ("color".to_string(), self.color.to_string()),
            ("display-name".to_string(), self.name.to_string()),
            ("emotes".to_string(), emotes_tag(self.text, self.emotes)),
            ("first-msg".to_string(), "0".to_string()),
            ("id".to_string(), self.id.to_string()),
            ("mod".to_string(), if self.badges.contains("moderator") { "1" } else { "0" }.to_string()),
            ("returning-chatter".to_string(), "0".to_string()),
            ("room-id".to_string(), ask.room_id.clone()),
            ("subscriber".to_string(), if self.badges.contains("subscriber") { "1" } else { "0" }.to_string()),
            ("tmi-sent-ts".to_string(), self.ts.to_string()),
            ("turbo".to_string(), "0".to_string()),
            ("user-id".to_string(), user_id),
            ("user-type".to_string(), String::new()),
        ];
        for (k, v) in &self.tags {
            match tags.iter_mut().find(|(key, _)| key == k) {
                Some(slot) => slot.1 = v.clone(),
                None => tags.push((k.to_string(), v.clone())),
            }
        }
        tags
    }

    fn privmsg(&self, ask: &PreviewAsk) -> String {
        let tags = self.base_tags(ask);
        format!(
            "@{} :{l}!{l}@{l}.tmi.twitch.tv PRIVMSG #{} :{}",
            join_tags(&tags),
            ask.channel,
            self.text,
            l = self.login
        )
    }

    /// A USERNOTICE (`msg-id` and `system-msg` in `tags`), with the
    /// chatter's own words when there are any.
    fn usernotice(&self, ask: &PreviewAsk) -> String {
        let mut tags = self.base_tags(ask);
        tags.push(("login".to_string(), self.login.to_string()));
        let body = if self.text.is_empty() { String::new() } else { format!(" :{}", self.text) };
        format!("@{} :tmi.twitch.tv USERNOTICE #{}{}", join_tags(&tags), ask.channel, body)
    }
}

fn join_tags(tags: &[(String, String)]) -> String {
    tags.iter()
        .map(|(k, v)| format!("{}={}", k, escape_tag(v)))
        .collect::<Vec<_>>()
        .join(";")
}

const KAPPA: (&str, &str) = ("Kappa", "25");
const LUL: (&str, &str) = ("LUL", "425618");
const SEEMS_GOOD: (&str, &str) = ("SeemsGood", "64138");
const HEY_GUYS: (&str, &str) = ("HeyGuys", "30259");
const NOT_LIKE_THIS: (&str, &str) = ("NotLikeThis", "58765");
const COOL_CAT: (&str, &str) = ("CoolCat", "58127");

/// The lines for one scene, oldest first.
fn lines(ask: &PreviewAsk, you_login: &str, you_id: &str) -> Vec<String> {
    let now = chrono::Utc::now().timestamp_millis();
    let you_name = if ask.you_name.is_empty() { you_login } else { ask.you_name.as_str() };
    let you_color = if ask.you_color.is_empty() { "#9147FF" } else { ask.you_color.as_str() };
    let mention_text = format!("@{you_name} what are your settings ICANT");
    // A greeting to the streamer of the chat being shown (a real name, so the
    // @mention resolves like any other).
    let streamer_hello = format!("@{} hydrate", ask.channel);
    let mention_ts = if ask.fresh { now } else { OLD_TS };
    let you = |text: &'static str| {
        let mut l = Line::new("lc-you", you_login, you_color, text);
        l.name = you_name;
        l.user_id = you_id;
        l.privmsg(ask)
    };
    let mention = {
        let mut l = Line::new("lc-mention", "ratio_rhea", "#B22222", &mention_text).badges("vip/1", "");
        l.ts = mention_ts;
        l.privmsg(ask)
    };
    let reply_text = format!("@{you_name} clean, love it");
    let reply = Line::new("lc-reply", "night_owl_nia", "#DA70D6", &reply_text)
        .badges("subscriber/3", "subscriber/3")
        .tag("reply-parent-msg-id", "lc-you")
        .tag("reply-parent-user-id", you_id)
        .tag("reply-parent-user-login", you_login)
        .tag("reply-parent-display-name", you_name)
        .tag("reply-parent-msg-body", "this is how i look in chat")
        .tag("reply-thread-parent-msg-id", "lc-you")
        .tag("reply-thread-parent-user-login", you_login)
        .privmsg(ask);
    let first = Line::new("lc-first", "saltwater_sam", "#DAA520", "hi! first stream i caught live HeyGuys")
        .emotes(&[HEY_GUYS])
        .tag("first-msg", "1")
        .privmsg(ask);
    let resub = Line::new("lc-resub", "bram_jam", "#FF4500", "still skips every chest NotLikeThis")
        .badges("subscriber/12", "subscriber/14")
        .emotes(&[NOT_LIKE_THIS])
        .tag("msg-id", "resub")
        .tag("msg-param-cumulative-months", "14")
        .tag("msg-param-sub-plan", "1000")
        .tag("msg-param-sub-plan-name", "Channel Subscription")
        .tag("system-msg", "bram_jam subscribed at Tier 1. They've subscribed for 14 months!")
        .usernotice(ask);
    let cheer = Line::new("lc-cheer", "bits_and_bobs", "#DAA520", "Cheer100 clutch tax")
        .badges("bits/1000,subscriber/6", "subscriber/7")
        .tag("bits", "100")
        .privmsg(ask);

    match ask.scene.as_str() {
        "events" => vec![
            resub,
            Line::new("lc-raid", "cozykettle", "#FF6347", "")
                .badges("partner/1", "")
                .tag("msg-id", "raid")
                .tag("msg-param-displayName", "CozyKettle")
                .tag("msg-param-login", "cozykettle")
                .tag("msg-param-viewerCount", "38")
                .tag("system-msg", "38 raiders from CozyKettle have joined!")
                .usernotice(ask),
            Line::new("lc-mystery", "tiny_tornado", "#FF7F50", "")
                .badges("sub-gifter/50,subscriber/6", "subscriber/6")
                .tag("msg-id", "submysterygift")
                .tag("msg-param-mass-gift-count", "5")
                .tag("msg-param-sub-plan", "1000")
                .tag("system-msg", "tiny_tornado is gifting 5 Tier 1 Subs to the community!")
                .usernotice(ask),
            Line::new("lc-gift", "tiny_tornado", "#FF7F50", "")
                .badges("sub-gifter/50,subscriber/6", "subscriber/6")
                .tag("msg-id", "subgift")
                .tag("msg-param-recipient-display-name", "quiet_quill")
                .tag("msg-param-recipient-user-name", "quiet_quill")
                .tag("msg-param-sub-plan", "1000")
                .tag("system-msg", "tiny_tornado gifted a Tier 1 sub to quiet_quill!")
                .usernotice(ask),
            cheer,
            Line::new("lc-highlighted", "fernlight", "#4FC3F7", "please notice me this time")
                .tag("msg-id", "highlighted-message")
                .privmsg(ask),
            Line::new("lc-announce", "marlowe", "#9ACD32", "giveaway at 500 followers, stay tuned")
                .badges("moderator/1", "")
                .tag("msg-id", "announcement")
                .tag("msg-param-color", "PRIMARY")
                .usernotice(ask),
            Line::new("lc-streak", "quietcomet", "#B39DDB", "here every night lol")
                .tag("msg-id", "viewermilestone")
                .tag("msg-param-category", "watch-streak")
                .tag("msg-param-value", "12")
                .tag("msg-param-copoReward", "450")
                .tag("system-msg", "quietcomet watched 12 consecutive streams this month and sparked a watch streak!")
                .usernotice(ask),
        ],
        "names" => vec![
            // The broadcaster of the chat the rows are parsed for: you, when it
            // is your own chat.
            {
                let mut l = Line::new("lc-streamer", ask.channel.as_str(), "#E0457B", "ok this is the one, i can feel it")
                    .badges("broadcaster/1,partner/1", "");
                if ask.channel.eq_ignore_ascii_case(you_login) {
                    l.name = you_name;
                    l.color = you_color;
                    l.user_id = you_id;
                }
                l.privmsg(ask)
            },
            Line::new("lc-mod", "marlowe", "#1E90FF", "keep it kind in here CoolCat")
                .badges("moderator/1,subscriber/24,premium/1", "subscriber/26")
                .emotes(&[COOL_CAT])
                .privmsg(ask),
            Line::new("lc-vip", "nightglass", "#A98BFF", &streamer_hello)
                .badges("vip/1,founder/0", "founder/30")
                .privmsg(ask),
            Line::new("lc-pebble", "pebble_pusher", "#2E8B57", "chat is this the final boss")
                .badges("subscriber/6,sub-gifter/10", "subscriber/8")
                .privmsg(ask),
            cheer,
            Line::new("lc-dark", "rainy_ren", "#1A237E", "he earned this SeemsGood")
                .badges("predictions/blue-1", "")
                .emotes(&[SEEMS_GOOD])
                .privmsg(ask),
            Line::new("lc-action", "popcorn_pete", "#FF8C00", "\u{1}ACTION grabs more popcorn\u{1}").privmsg(ask),
            you("this is how i look in chat"),
        ],
        "mentions" => vec![
            Line::new("lc-dan", "driftwood_dan", "#5F9EA0", "he is so locked in").privmsg(ask),
            mention,
            you("this is how i look in chat"),
            reply,
            Line::new("lc-lol", "popcorn_pete", "#FF8C00", "LUL LUL LUL").emotes(&[LUL]).privmsg(ask),
        ],
        "links" => vec![
            Line::new("lc-link", "marlowe", "#1E90FF", "the app is here streamnook.app")
                .badges("moderator/1", "")
                .privmsg(ask),
            Line::new("lc-yt", "tea_time_tom", "#4FC3F7", "this edit https://www.youtube.com/watch?v=dQw4w9WgXcQ").privmsg(ask),
        ],
        "emotes" => vec![
            Line::new("lc-kappa", "tea_time_tom", "#4FC3F7", "no way Kappa").emotes(&[KAPPA]).privmsg(ask),
            Line::new("lc-words", "popcorn_pete", "#FF8C00", "that ending was so LUL honestly the best run yet")
                .emotes(&[LUL])
                .privmsg(ask),
            Line::new("lc-7tv", "pepe_patrol", "#3CB371", "my heart cant take this Sadge").privmsg(ask),
            Line::new("lc-emoji", "lofi_lena", "#FF8A8A", "hi from the other side 👋🔥").privmsg(ask),
            Line::new("lc-giant", "loud_lettuce", "#9ACD32", "HE DID IT CoolCat")
                .emotes(&[COOL_CAT])
                .tag("msg-id", "gigantified-emote-message")
                .privmsg(ask),
            you("this is how i look in chat"),
        ],
        "hidden" => vec![
            Line::new("lc-pebble", "pebble_pusher", "#2E8B57", "chat is this the final boss")
                .badges("subscriber/6", "subscriber/8")
                .privmsg(ask),
            Line::new("lc-bot", "streamelements", "#5B99FF", "Drops are live, type !drops to see them")
                .badges("moderator/1", "")
                .privmsg(ask),
            Line::new("lc-cmd", "bagel_baron", "#7BFF3C", "!drops").privmsg(ask),
            Line::new("lc-shared", "driftwood_dan", "#5F9EA0", "hi from the other channel HeyGuys")
                .emotes(&[HEY_GUYS])
                .tag("source-room-id", "9001")
                .tag("source-id", "lc-shared-src")
                .privmsg(ask),
            Line::new("lc-deleted", "bram_jam", "#FF4500", "first").privmsg(ask),
            Line::new("lc-timeout", "loud_lettuce", "#9ACD32", "SPAM SPAM SPAM").privmsg(ask),
            you("this is how i look in chat"),
        ],
        "highlights" => {
            let mut out = vec![first];
            if let Some(p) = ask.phrase.as_deref().filter(|p| !p.trim().is_empty()) {
                let text = format!("wait did someone say {}", p.trim());
                out.push(Line::new("lc-phrase", "quiet_quill", "#20B2AA", &text).privmsg(ask));
            }
            if let Some(u) = ask.highlight_user.as_deref().filter(|u| !u.trim().is_empty()) {
                let login = u.trim().to_lowercase();
                out.push(Line::new("lc-hl-user", &login, "#FF69B4", "back again HeyGuys").emotes(&[HEY_GUYS]).privmsg(ask));
            }
            if let Some(b) = ask.highlight_badge.as_deref().filter(|b| b.contains('/')) {
                out.push(Line::new("lc-hl-badge", "kettle_crew_kai", "#87CEEB", "gg everyone").badges(b, "").privmsg(ask));
            }
            out.push(mention);
            out.push(resub);
            out.push(you("this is how i look in chat"));
            out
        }
        _ => vec![
            Line::new("lc-pebble", "pebble_pusher", "#2E8B57", "chat is this the final boss")
                .badges("subscriber/6,sub-gifter/10", "subscriber/8")
                .privmsg(ask),
            Line::new("lc-link", "marlowe", "#1E90FF", "the app is here streamnook.app")
                .badges("moderator/1", "")
                .privmsg(ask),
            first,
            mention,
            you("this is how i look in chat"),
            reply,
            Line::new("lc-deleted", "bram_jam", "#FF4500", "first").privmsg(ask),
            resub,
        ],
    }
}

/// What the preview draws: the rows, and for the Polls & Predictions section
/// a poll and a prediction card built by the same Rust as live ones.
#[derive(Debug, Serialize)]
pub struct PreviewChat {
    pub rows: Vec<ChatMessage>,
    pub poll: Option<crate::services::poll_view::PollView>,
    pub prediction: Option<crate::services::predictions::PredictionView>,
    /// Rust's answer for a 10 point bet on the sample prediction's first side.
    pub prediction_bet: Option<crate::services::predictions::BetView>,
    /// For the Pinned Messages section: a moderator's pin, in the shape the
    /// live pinned read sends (text split by `text_spans::pin_segments`).
    pub pins: Vec<serde_json::Value>,
    /// A room with followers-only (10 minutes) and slow mode on, worded by the
    /// same Rust as a live room, so the room-mode chip or prompt shows.
    pub room_modes: crate::services::room_modes::RoomModeView,
}

pub fn preview(ask: &PreviewAsk) -> PreviewChat {
    let cards = ask.scene == "polls";
    let now = chrono::Utc::now().timestamp_millis();
    let channel = if ask.room_id.is_empty() { "preview" } else { ask.room_id.as_str() };
    let (prediction, prediction_bet) = if cards {
        let (view, bet) = crate::services::predictions::sample_view(channel, now);
        (Some(view), Some(bet))
    } else {
        (None, None)
    };
    PreviewChat {
        rows: rows(ask),
        poll: if cards { sample_poll(channel, now) } else { None },
        prediction,
        prediction_bet,
        pins: if ask.scene == "pinned" { sample_pins(ask) } else { Vec::new() },
        room_modes: {
            use crate::services::room_modes::{describe_with, ModesIn, OwnFollow, Standing};
            let modes = ModesIn { slow_seconds: 30, followers_only_minutes: 10, ..Default::default() };
            describe_with(&modes, Standing::Viewer { is_sub: false }, OwnFollow::ForSecs(6 * 60))
        },
    }
}

/// A moderator's pin in the chat being shown, as the live pinned read builds
/// one (`twitch_service`'s pinned messages).
fn sample_pins(ask: &PreviewAsk) -> Vec<serde_json::Value> {
    let text = "Giveaway at 500 followers! Rules and times: streamnook.app";
    // In your own chat you pinned it; elsewhere a moderator did.
    let yours = !ask.you_login.is_empty() && ask.channel.eq_ignore_ascii_case(&ask.you_login);
    let (by, by_id) = if yours { (ask.you_name.as_str(), ask.you_id.as_str()) } else { ("marlowe", "uid-marlowe") };
    vec![serde_json::json!({
        "id": "preview-pin",
        "message_id": "lc-pinned",
        "type": "MOD",
        "message_text": text,
        "segments": crate::services::text_spans::pin_segments(text),
        "sender_id": "uid-marlowe",
        "sender_name": "marlowe",
        "sender_login": "marlowe",
        "sender_shown_name": crate::services::chatter_name::shown_name("twitch", "marlowe", "marlowe"),
        "sender_color": "#1E90FF",
        "sender_avatar": "",
        "sender_badges": [{ "set_id": "moderator", "version": "1" }],
        "pinned_by": by,
        "pinned_by_id": by_id,
        "pinned_by_avatar": "",
        "started_at": "",
    })]
}

/// An open poll's card, from the payload shape a live poll arrives in.
fn sample_poll(channel_id: &str, now_ms: i64) -> Option<crate::services::poll_view::PollView> {
    let poll = serde_json::json!({
        "channel_id": channel_id, "poll_id": "preview-poll", "title": "Which boss next?", "status": "ACTIVE",
        "remaining_ms": 84_000, "total_voters": 1204, "total_votes": 1204,
        "channel_points_voting": false,
        "choices": [
            { "id": "a", "title": "Malenia", "total_votes": 698 },
            { "id": "b", "title": "Radahn", "total_votes": 373 },
            { "id": "c", "title": "Mohg", "total_votes": 133 },
        ],
        "my_choice_id": null,
    });
    crate::services::poll_view::view_of(&poll, now_ms)
}

/// The scene's rows, as chat would draw them for those lines.
pub fn rows(ask: &PreviewAsk) -> Vec<ChatMessage> {
    let (you_login, you_id) = if !ask.you_login.is_empty() && !ask.you_id.is_empty() {
        // Your rules learn who you are when chat connects; Settings can open
        // before that, and a mention of you must still read as one.
        if ChatRules::own_identity().is_none() {
            ChatRules::set_own_identity(&ask.you_login.to_lowercase(), &ask.you_id);
            if !ask.you_name.is_empty() {
                ChatRules::set_own_display_name(&ask.you_name);
            }
        }
        (ask.you_login.to_lowercase(), ask.you_id.clone())
    } else {
        ChatRules::own_identity().unwrap_or_else(|| ("you".into(), "you".into()))
    };
    let rules = ChatRules::snapshot();
    lines(ask, &you_login, &you_id)
        .into_iter()
        .filter_map(|raw| {
            let mut msg = IrcService::parse_live_line(&raw)?;
            msg.layout = LayoutResult {
                height: 0.0,
                width: 0.0,
                has_reply: msg.metadata.reply_info.is_some(),
                is_first_message: msg.metadata.is_first_message,
            };
            if ChatRules::evaluate_unrecorded(&mut msg, &rules).drop {
                return None;
            }
            crate::services::chat_row_view::stamp(&mut msg);
            Some(msg)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{emotes_tag, rows, PreviewAsk};
    use crate::services::chat_rules::ChatRules;

    #[test]
    fn the_polls_section_gets_a_live_built_poll_and_prediction() {
        let polls = super::preview(&PreviewAsk { scene: "polls".into(), channel: "harborlight".into(), ..Default::default() });
        assert!(polls.poll.is_some_and(|p| p.choices.len() == 3));
        assert!(polls.prediction.is_some());
        let other = super::preview(&PreviewAsk { scene: "names".into(), channel: "harborlight".into(), ..Default::default() });
        assert!(other.poll.is_none() && other.prediction.is_none());
    }

    #[test]
    fn twitch_emote_spans_are_character_indexes() {
        assert_eq!(emotes_tag("no way Kappa", &[("Kappa", "25")]), "25:7-11");
        assert_eq!(emotes_tag("LUL LUL", &[("LUL", "425618")]), "425618:0-2,4-6");
        assert_eq!(emotes_tag("hi", &[("Kappa", "25")]), "");
    }

    #[test]
    fn every_scene_parses_into_rows_chat_would_draw() {
        let _own = ChatRules::own_identity_test_lock();
        ChatRules::set_own_identity("brandon", "999");
        for scene in ["conversation", "events", "names", "mentions", "links", "emotes", "hidden", "highlights"] {
            let ask = PreviewAsk {
                scene: scene.into(),
                channel: "harborlight".into(),
                room_id: "1".into(),
                you_name: "Brandon".into(),
                phrase: Some("hydrate".into()),
                highlight_user: Some("someone".into()),
                highlight_badge: Some("vip/1".into()),
                ..Default::default()
            };
            let got = rows(&ask);
            assert!(got.len() >= 2, "{scene}: {} rows", got.len());
            assert!(got.iter().all(|r| r.metadata.rules_evaluated), "{scene}: unstamped row");
        }
        let convo = rows(&PreviewAsk { scene: "conversation".into(), channel: "harborlight".into(), you_name: "Brandon".into(), ..Default::default() });
        let you = convo.iter().find(|r| r.id == "lc-you").expect("your line");
        assert!(you.metadata.is_own);
        let cheer = rows(&PreviewAsk { scene: "events".into(), channel: "harborlight".into(), ..Default::default() })
            .into_iter()
            .find(|r| r.id == "lc-cheer")
            .expect("the cheer");
        assert_eq!(cheer.metadata.bits_amount, Some(100));
        let resub = convo.iter().find(|r| r.id == "lc-resub").expect("the resub");
        assert_eq!(resub.metadata.msg_type.as_deref(), Some("resub"));
    }
}
