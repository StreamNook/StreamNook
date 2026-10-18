//! A chat room's modes in words, for the message box.
//!
//! This module tracks nothing. The Twitch modes live in `chat_send_limiter`
//! (fed by ROOMSTATE and USERSTATE) and Kick's arrive whole with each settings
//! event; both hand their facts here and ship the result on the ROOMSTATE frame
//! the page already receives, under `modes`. The page only draws it.
//!
//! What it produces: each active mode with a label, a short form for a narrow
//! box and one sentence of detail; whether each mode holds the viewer back when
//! that is known; the two ways the message box can show them (a chip at its
//! edge, or a prompt in place of "Send a message", the viewer's choice); a
//! "you can chat" line when every mode spares the viewer; and the slow-mode
//! wait that applies to them.
//!
//! Followers-only chat is decided by the account's own follow of the channel
//! when the caller knows it (`OwnFollow`): a long enough follow spares the
//! viewer, and a missing or too recent one turns the prompt into what to do
//! ("Follow to chat", "You can chat in 4 minutes").
//!
//! The phone composers also take `blocked` and `reason`: set only when the
//! viewer's standing or follow PROVES a mode keeps them from sending, so the
//! field can be disabled with one line saying why. An unknown standing or
//! follow never blocks.

use serde::Serialize;

/// The signed-in account's follow of the channel, for followers-only chat.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OwnFollow {
    /// Not looked up, or the lookup had no answer.
    #[default]
    Unknown,
    NotFollowing,
    /// Following for this many seconds.
    ForSecs(i64),
}

/// The modes a room has on. Off is `0`, `-1` or `false`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModesIn {
    pub slow_seconds: u64,
    /// -1 = off, 0 = any follower, N = followed at least N minutes.
    pub followers_only_minutes: i64,
    pub subs_only: bool,
    pub emote_only: bool,
    /// Twitch's unique chat (r9k).
    pub unique: bool,
}

/// What the viewer is in the room, as far as the app knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Standing {
    /// Not known yet (no USERSTATE), or a platform that does not say.
    Unknown,
    Broadcaster,
    Moderator,
    Vip,
    Viewer { is_sub: bool },
}

impl Standing {
    fn privileged(self) -> bool {
        matches!(self, Standing::Broadcaster | Standing::Moderator | Standing::Vip)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModeKind {
    SubsOnly,
    EmoteOnly,
    FollowersOnly,
    Slow,
    Unique,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RoomMode {
    pub kind: ModeKind,
    /// "Followers-only chat (10 minutes)".
    pub label: String,
    /// "Followers-only (10m)", for joining several on one line.
    pub compact: String,
    /// "Followers 10m", for a narrow message box.
    pub short: String,
    /// One sentence on what the mode does.
    pub detail: String,
    /// `Some(false)`: this mode does not hold the viewer back. `None`: unknown.
    pub applies_to_you: Option<bool>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct RoomModeView {
    /// Active modes, the most limiting first. Empty when the room is open.
    pub modes: Vec<RoomMode>,
    /// The chip style: the most limiting mode that may hold the viewer back
    /// in a word or two ("Slow 30s"), else the most limiting mode. `None`
    /// when the room is open.
    pub chip: Option<String>,
    /// Every active mode spares the viewer (a moderator, say): the chip is
    /// drawn a shade quieter.
    pub exempt: bool,
    /// The prompt style: replaces "Send a message" while a mode may hold the
    /// viewer back, naming the most limiting such mode.
    pub prompt: Option<String>,
    /// The same in a word or two, for a narrow message box.
    pub prompt_short: Option<String>,
    /// "You can chat as a moderator" when every active mode spares the viewer.
    pub standing: Option<String>,
    /// The slow-mode wait that applies to the viewer; 0 when none does.
    pub slow_seconds: u64,
    /// The mode that certainly keeps the viewer from sending (sub-only, then
    /// emote-only, then followers-only), or `None` when nothing proves it.
    pub blocked: Option<ModeKind>,
    /// What a disabled message box says while `blocked` is set.
    pub reason: Option<String>,
    /// A moderator's warning the viewer has not acknowledged: the box stays
    /// locked behind its Acknowledge button. Filled in by the chat limiter,
    /// which knows the room (`chat_warning`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<crate::services::chat_warning::WarningView>,
}

/// The longest full label the prompt uses; past it the compact form reads
/// better in a chat column ("Followers-only (10m)").
const PROMPT_LABEL_MAX: usize = 22;

fn plural(n: i64, one: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {one}s")
    }
}

/// A follow age in words and short form: (`"10 minutes"`, `"10m"`).
/// Twitch's month is 30 days.
pub fn follow_age(minutes: i64) -> (String, String) {
    const HOUR: i64 = 60;
    const DAY: i64 = 24 * HOUR;
    const WEEK: i64 = 7 * DAY;
    const MONTH: i64 = 30 * DAY;
    let m = minutes.max(0);
    for (size, one, short) in [(MONTH, "month", "mo"), (WEEK, "week", "w"), (DAY, "day", "d"), (HOUR, "hour", "h")] {
        if m >= size && m % size == 0 {
            return (plural(m / size, one), format!("{}{short}", m / size));
        }
    }
    (plural(m, "minute"), format!("{m}m"))
}

/// A slow-mode wait in words and short form: (`"30 seconds"`, `"30s"`).
pub fn slow_wait(seconds: u64) -> (String, String) {
    let s = seconds as i64;
    if s >= 60 && s % 60 == 0 {
        (plural(s / 60, "minute"), format!("{}m", s / 60))
    } else {
        (plural(s, "second"), format!("{s}s"))
    }
}

fn mode(kind: ModeKind, label: String, compact: String, short: String, detail: String, applies: Option<bool>) -> RoomMode {
    RoomMode { kind, label, compact, short, detail, applies_to_you: applies }
}

/// The room's modes in words, and what they mean for a viewer of `standing`
/// whose follow of the channel is not known.
pub fn describe(m: &ModesIn, standing: Standing) -> RoomModeView {
    describe_with(m, standing, OwnFollow::Unknown)
}

/// The room's modes in words, and what they mean for a viewer of `standing`
/// who follows the channel as `follow` says.
pub fn describe_with(m: &ModesIn, standing: Standing, follow: OwnFollow) -> RoomModeView {
    let privileged = standing.privileged();
    // Emote-only, slow mode and unique chat hold back everyone who is not a
    // moderator, VIP or the broadcaster.
    let everyone = match standing {
        Standing::Unknown => None,
        _ => Some(!privileged),
    };
    let mut modes = Vec::new();
    if m.subs_only {
        let applies = match standing {
            Standing::Unknown => None,
            Standing::Viewer { is_sub } => Some(!is_sub),
            _ => Some(false),
        };
        modes.push(mode(
            ModeKind::SubsOnly,
            "Subscriber-only chat".into(),
            "Subscriber-only".into(),
            "Sub-only".into(),
            "Only subscribers can chat.".into(),
            applies,
        ));
    }
    if m.emote_only {
        modes.push(mode(
            ModeKind::EmoteOnly,
            "Emote-only chat".into(),
            "Emote-only".into(),
            "Emote-only".into(),
            "Messages can only contain emotes.".into(),
            everyone,
        ));
    }
    // What a followers-only room asks of this viewer before they can chat:
    // `Some(None)` = follow first, `Some(Some(secs))` = wait this long.
    let mut follow_need: Option<Option<i64>> = None;
    if m.followers_only_minutes >= 0 {
        let need_secs = m.followers_only_minutes * 60;
        // Subscribers are spared followers-only chat like the privileged.
        let subscribed = matches!(standing, Standing::Viewer { is_sub: true });
        let applies = if privileged || subscribed {
            Some(false)
        } else {
            match follow {
                OwnFollow::Unknown => None,
                OwnFollow::NotFollowing => {
                    follow_need = Some(None);
                    Some(true)
                }
                OwnFollow::ForSecs(secs) if secs < need_secs => {
                    follow_need = Some(Some(need_secs - secs));
                    Some(true)
                }
                OwnFollow::ForSecs(_) => Some(false),
            }
        };
        let row = if m.followers_only_minutes == 0 {
            mode(
                ModeKind::FollowersOnly,
                "Followers-only chat".into(),
                "Followers-only".into(),
                "Followers-only".into(),
                "Only followers can chat.".into(),
                applies,
            )
        } else {
            let (long, short) = follow_age(m.followers_only_minutes);
            mode(
                ModeKind::FollowersOnly,
                format!("Followers-only chat ({long})"),
                format!("Followers-only ({short})"),
                format!("Followers {short}"),
                format!("Only accounts that have followed for {long} can chat."),
                applies,
            )
        };
        modes.push(row);
    }
    if m.slow_seconds > 0 {
        let (long, short) = slow_wait(m.slow_seconds);
        modes.push(mode(
            ModeKind::Slow,
            format!("Slow mode ({long})"),
            format!("Slow mode ({short})"),
            format!("Slow {short}"),
            format!("Chatters wait {long} between messages."),
            everyone,
        ));
    }
    if m.unique {
        modes.push(mode(
            ModeKind::Unique,
            "Unique chat".into(),
            "Unique chat".into(),
            "Unique".into(),
            "Messages that repeat an earlier one are not allowed.".into(),
            everyone,
        ));
    }

    // The chip names one mode: the most limiting that may hold the viewer
    // back, or, when none does, the most limiting there is. Hovering it lists
    // the rest.
    let binding: Vec<&RoomMode> = modes.iter().filter(|r| r.applies_to_you != Some(false)).collect();
    let exempt = !modes.is_empty() && binding.is_empty();
    let chip = binding.first().copied().or(modes.first()).map(|top| top.short.clone());
    // The prompt names only a mode that holds the viewer back: its full label
    // when it is the only one and short enough for a chat column, else its
    // compact form. A viewer every mode spares keeps "Send a message".
    let (prompt, prompt_short) = match binding.first() {
        None => (None, None),
        // A followers-only room the viewer cannot chat in yet says what it
        // takes, in place of naming the mode.
        Some(top) if top.kind == ModeKind::FollowersOnly && follow_need.is_some() => match follow_need.flatten() {
            None => (Some("Follow to chat".to_string()), Some("Follow to chat".to_string())),
            Some(secs) => {
                // Whole minutes, rounded up, so it never says 0.
                let (long, short) = follow_age((secs + 59) / 60);
                (Some(format!("You can chat in {long}")), Some(format!("Chat in {short}")))
            }
        },
        Some(top) => {
            let words = if binding.len() == 1 && top.label.chars().count() <= PROMPT_LABEL_MAX {
                top.label.clone()
            } else {
                top.compact.clone()
            };
            (Some(words), Some(top.short.clone()))
        }
    };
    let standing_line = if exempt {
        let as_what = match standing {
            Standing::Broadcaster => "the broadcaster",
            Standing::Moderator => "a moderator",
            Standing::Vip => "a VIP",
            // Only sub-only and followers-only chat can spare a viewer.
            Standing::Viewer { is_sub: true } => "a subscriber",
            Standing::Viewer { is_sub: false } => "a follower",
            Standing::Unknown => "",
        };
        (!as_what.is_empty()).then(|| format!("You can chat as {as_what}"))
    } else {
        None
    };
    let slow_seconds = modes
        .iter()
        .find(|r| r.kind == ModeKind::Slow && r.applies_to_you != Some(false))
        .map(|_| m.slow_seconds)
        .unwrap_or(0);
    // Only the modes that refuse a message outright can block the box; slow
    // mode and unique chat let the viewer type. Proven only, never guessed.
    let proven = |kind: ModeKind| modes.iter().any(|r| r.kind == kind && r.applies_to_you == Some(true));
    let blocked = [ModeKind::SubsOnly, ModeKind::EmoteOnly, ModeKind::FollowersOnly]
        .into_iter()
        .find(|kind| proven(*kind));
    let reason = blocked.map(|kind| match kind {
        ModeKind::SubsOnly => "Subscribers only".to_string(),
        ModeKind::EmoteOnly => "Emote-only: pick an emote".to_string(),
        _ => match follow_need.flatten() {
            Some(secs) => format!("You can chat in {}", follow_age((secs + 59) / 60).0),
            None => "Follow to chat".to_string(),
        },
    });
    RoomModeView { modes, chip, exempt, prompt, prompt_short, standing: standing_line, slow_seconds, blocked, reason, warning: None }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn viewer() -> Standing {
        Standing::Viewer { is_sub: false }
    }

    #[test]
    fn an_open_room_says_nothing() {
        let v = describe(&ModesIn { followers_only_minutes: -1, ..Default::default() }, viewer());
        assert!(v.modes.is_empty());
        assert_eq!(v.chip, None);
        assert!(!v.exempt);
        assert_eq!(v.prompt, None);
        assert_eq!(v.prompt_short, None);
        assert_eq!(v.standing, None);
        assert_eq!(v.slow_seconds, 0);
    }

    #[test]
    fn follow_ages_read_in_the_largest_whole_unit() {
        assert_eq!(follow_age(10), ("10 minutes".into(), "10m".into()));
        assert_eq!(follow_age(1), ("1 minute".into(), "1m".into()));
        assert_eq!(follow_age(30), ("30 minutes".into(), "30m".into()));
        assert_eq!(follow_age(60), ("1 hour".into(), "1h".into()));
        assert_eq!(follow_age(90), ("90 minutes".into(), "90m".into()));
        assert_eq!(follow_age(1440), ("1 day".into(), "1d".into()));
        assert_eq!(follow_age(10080), ("1 week".into(), "1w".into()));
        assert_eq!(follow_age(43200), ("1 month".into(), "1mo".into()));
        assert_eq!(follow_age(129600), ("3 months".into(), "3mo".into()));
    }

    #[test]
    fn slow_waits_read_in_seconds_or_whole_minutes() {
        assert_eq!(slow_wait(30), ("30 seconds".into(), "30s".into()));
        assert_eq!(slow_wait(1), ("1 second".into(), "1s".into()));
        assert_eq!(slow_wait(90), ("90 seconds".into(), "90s".into()));
        assert_eq!(slow_wait(120), ("2 minutes".into(), "2m".into()));
    }

    #[test]
    fn one_mode_names_the_chip() {
        let v = describe(&ModesIn { emote_only: true, followers_only_minutes: -1, ..Default::default() }, viewer());
        assert_eq!(v.modes.len(), 1);
        assert_eq!(v.modes[0].kind, ModeKind::EmoteOnly);
        assert_eq!(v.chip.as_deref(), Some("Emote-only"));
        assert!(!v.exempt);
        assert_eq!(v.prompt.as_deref(), Some("Emote-only chat"));
        assert_eq!(v.prompt_short.as_deref(), Some("Emote-only"));
        assert_eq!(v.modes[0].applies_to_you, Some(true));
    }

    #[test]
    fn several_modes_list_most_limiting_first() {
        let v = describe(
            &ModesIn { slow_seconds: 30, followers_only_minutes: 10, ..Default::default() },
            viewer(),
        );
        let kinds: Vec<_> = v.modes.iter().map(|m| m.kind).collect();
        assert_eq!(kinds, vec![ModeKind::FollowersOnly, ModeKind::Slow]);
        assert_eq!(v.modes[0].label, "Followers-only chat (10 minutes)");
        assert_eq!(v.modes[0].detail, "Only accounts that have followed for 10 minutes can chat.");
        assert_eq!(v.modes[1].label, "Slow mode (30 seconds)");
        assert_eq!(v.chip.as_deref(), Some("Followers 10m"), "the most limiting one; hovering lists the rest");
        assert_eq!(v.prompt.as_deref(), Some("Followers-only (10m)"));
        assert_eq!(v.prompt_short.as_deref(), Some("Followers 10m"));
        assert_eq!(v.slow_seconds, 30);
        // Follow age is the platform's call, never assumed.
        assert_eq!(v.modes[0].applies_to_you, None);
    }

    #[test]
    fn the_chip_uses_the_short_form() {
        let slow = describe(&ModesIn { slow_seconds: 30, followers_only_minutes: -1, ..Default::default() }, viewer());
        assert_eq!(slow.chip.as_deref(), Some("Slow 30s"));
        let any = describe(&ModesIn { followers_only_minutes: 0, ..Default::default() }, viewer());
        assert_eq!(any.chip.as_deref(), Some("Followers-only"));
    }

    #[test]
    fn a_long_single_label_prompts_in_its_compact_form() {
        let slow = describe(&ModesIn { slow_seconds: 30, followers_only_minutes: -1, ..Default::default() }, viewer());
        assert_eq!(slow.prompt.as_deref(), Some("Slow mode (30 seconds)"));
        let follow = describe(&ModesIn { followers_only_minutes: 10, ..Default::default() }, viewer());
        assert_eq!(follow.prompt.as_deref(), Some("Followers-only (10m)"));
        let any = describe(&ModesIn { followers_only_minutes: 0, ..Default::default() }, viewer());
        assert_eq!(any.prompt.as_deref(), Some("Followers-only chat"));
    }

    #[test]
    fn every_mode_has_its_words() {
        let v = describe(
            &ModesIn { slow_seconds: 120, followers_only_minutes: 0, subs_only: true, emote_only: true, unique: true },
            viewer(),
        );
        let labels: Vec<_> = v.modes.iter().map(|m| m.label.as_str()).collect();
        assert_eq!(
            labels,
            vec!["Subscriber-only chat", "Emote-only chat", "Followers-only chat", "Slow mode (2 minutes)", "Unique chat"]
        );
        let shorts: Vec<_> = v.modes.iter().map(|m| m.short.as_str()).collect();
        assert_eq!(shorts, vec!["Sub-only", "Emote-only", "Followers-only", "Slow 2m", "Unique"]);
        assert_eq!(v.chip.as_deref(), Some("Sub-only"));
    }

    #[test]
    fn moderators_vips_and_the_broadcaster_are_told_they_can_chat() {
        let modes = ModesIn { slow_seconds: 30, emote_only: true, followers_only_minutes: 10, ..Default::default() };
        for (standing, line) in [
            (Standing::Moderator, "You can chat as a moderator"),
            (Standing::Vip, "You can chat as a VIP"),
            (Standing::Broadcaster, "You can chat as the broadcaster"),
        ] {
            let v = describe(&modes, standing);
            assert_eq!(v.modes.len(), 3, "the modes still show");
            assert!(v.modes.iter().all(|m| m.applies_to_you == Some(false)));
            assert!(v.exempt);
            assert_eq!(v.chip.as_deref(), Some("Emote-only"), "the chip still shows, quieter");
            assert_eq!(v.prompt, None, "the box keeps its usual prompt");
            assert_eq!(v.standing.as_deref(), Some(line));
            assert_eq!(v.slow_seconds, 0, "no countdown for a privileged sender");
        }
    }

    #[test]
    fn a_subscriber_in_sub_only_chat_can_chat() {
        let modes = ModesIn { subs_only: true, followers_only_minutes: -1, ..Default::default() };
        let sub = describe(&modes, Standing::Viewer { is_sub: true });
        assert_eq!(sub.modes[0].applies_to_you, Some(false));
        assert_eq!(sub.standing.as_deref(), Some("You can chat as a subscriber"));
        assert!(sub.exempt);
        assert_eq!(sub.prompt, None);

        let not_sub = describe(&modes, viewer());
        assert_eq!(not_sub.modes[0].applies_to_you, Some(true));
        assert_eq!(not_sub.chip.as_deref(), Some("Sub-only"));
        assert!(!not_sub.exempt);
        assert_eq!(not_sub.prompt.as_deref(), Some("Subscriber-only chat"));
        assert_eq!(not_sub.standing, None);
    }

    #[test]
    fn an_unknown_standing_claims_nothing() {
        let v = describe(&ModesIn { slow_seconds: 5, emote_only: true, followers_only_minutes: -1, ..Default::default() }, Standing::Unknown);
        assert!(v.modes.iter().all(|m| m.applies_to_you.is_none()));
        assert_eq!(v.chip.as_deref(), Some("Emote-only"));
        assert_eq!(v.prompt.as_deref(), Some("Emote-only"));
        assert_eq!(v.standing, None);
        assert_eq!(v.slow_seconds, 5, "the wait still counts down");
        assert_eq!(v.blocked, None, "an unknown standing never blocks");
    }

    #[test]
    fn a_long_enough_follow_spares_the_viewer() {
        let modes = ModesIn { followers_only_minutes: 10, ..Default::default() };
        let v = describe_with(&modes, viewer(), OwnFollow::ForSecs(3 * 86_400));
        assert_eq!(v.modes[0].applies_to_you, Some(false));
        assert!(v.exempt);
        assert_eq!(v.prompt, None, "the box keeps Send a message");
        assert_eq!(v.standing.as_deref(), Some("You can chat as a follower"));
        assert_eq!(v.blocked, None);
    }

    #[test]
    fn a_viewer_who_does_not_follow_is_told_to() {
        let modes = ModesIn { followers_only_minutes: 10, ..Default::default() };
        let v = describe_with(&modes, viewer(), OwnFollow::NotFollowing);
        assert_eq!(v.modes[0].applies_to_you, Some(true));
        assert_eq!(v.prompt.as_deref(), Some("Follow to chat"));
        assert_eq!(v.prompt_short.as_deref(), Some("Follow to chat"));
        assert!(!v.exempt);
        assert_eq!(v.blocked, Some(ModeKind::FollowersOnly));
        assert_eq!(v.reason.as_deref(), Some("Follow to chat"));
    }

    #[test]
    fn a_recent_follow_counts_down_in_whole_minutes() {
        let modes = ModesIn { followers_only_minutes: 10, ..Default::default() };
        // 6 minutes 30 seconds in: 3.5 minutes left, said as 4.
        let v = describe_with(&modes, viewer(), OwnFollow::ForSecs(390));
        assert_eq!(v.prompt.as_deref(), Some("You can chat in 4 minutes"));
        assert_eq!(v.prompt_short.as_deref(), Some("Chat in 4m"));
        assert_eq!(v.reason.as_deref(), Some("You can chat in 4 minutes"));
        // Seconds from the end it still says a minute, never zero.
        let v = describe_with(&modes, viewer(), OwnFollow::ForSecs(599));
        assert_eq!(v.prompt.as_deref(), Some("You can chat in 1 minute"));
    }

    #[test]
    fn a_follow_is_ignored_where_it_does_not_decide() {
        // Any-follower room: following at all is enough.
        let any = ModesIn { followers_only_minutes: 0, ..Default::default() };
        let v = describe_with(&any, viewer(), OwnFollow::ForSecs(0));
        assert!(v.exempt);
        assert_eq!(v.prompt, None);
        assert_eq!(v.blocked, None);
        // A moderator is spared whatever their follow.
        let ten = ModesIn { followers_only_minutes: 10, ..Default::default() };
        let v = describe_with(&ten, Standing::Moderator, OwnFollow::NotFollowing);
        assert_eq!(v.prompt, None);
        assert_eq!(v.blocked, None);
        assert_eq!(v.standing.as_deref(), Some("You can chat as a moderator"));
        // A followed viewer in slow mode is still told about slow mode, and
        // slow mode never disables the box.
        let slow = ModesIn { followers_only_minutes: 10, slow_seconds: 30, ..Default::default() };
        let v = describe_with(&slow, viewer(), OwnFollow::ForSecs(86_400));
        assert_eq!(v.prompt.as_deref(), Some("Slow mode (30 seconds)"));
        assert_eq!(v.blocked, None);
    }

    #[test]
    fn an_unknown_follow_never_blocks() {
        let modes = ModesIn { followers_only_minutes: 0, ..Default::default() };
        let v = describe_with(&modes, viewer(), OwnFollow::Unknown);
        assert_eq!(v.blocked, None);
        assert_eq!(v.reason, None);
    }

    #[test]
    fn blocks_name_the_most_limiting_proven_mode() {
        // Sub-only beats emote-only beats followers-only.
        let all = ModesIn { subs_only: true, emote_only: true, followers_only_minutes: 0, ..Default::default() };
        let v = describe_with(&all, viewer(), OwnFollow::NotFollowing);
        assert_eq!(v.blocked, Some(ModeKind::SubsOnly));
        assert_eq!(v.reason.as_deref(), Some("Subscribers only"));
        // A subscriber is spared sub-only, so emote-only is next.
        let v = describe_with(&all, Standing::Viewer { is_sub: true }, OwnFollow::NotFollowing);
        assert_eq!(v.blocked, Some(ModeKind::EmoteOnly));
        assert_eq!(v.reason.as_deref(), Some("Emote-only: pick an emote"));
        // Emote-only off: the follow decides, and a subscriber is spared it.
        let follow = ModesIn { followers_only_minutes: 0, ..Default::default() };
        let v = describe_with(&follow, viewer(), OwnFollow::NotFollowing);
        assert_eq!(v.blocked, Some(ModeKind::FollowersOnly));
        let v = describe_with(&follow, Standing::Viewer { is_sub: true }, OwnFollow::NotFollowing);
        assert_eq!(v.blocked, None);
        assert_eq!(v.standing.as_deref(), Some("You can chat as a subscriber"));
        // Privileged viewers are never blocked.
        for standing in [Standing::Broadcaster, Standing::Moderator, Standing::Vip] {
            assert_eq!(describe_with(&all, standing, OwnFollow::NotFollowing).blocked, None);
        }
    }
}
