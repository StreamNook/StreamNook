//! The typed Command action (D282, D290, D291). At save a line becomes a
//! [`CommandAction`] through the same pure parsers the slash dispatcher uses;
//! at fire the line is expanded (variables), parsed again by the same
//! function with no await, and a failed parse skips before any door.
//!
//! Only the thirteen forms below exist. A built-in outside them, a saved
//! custom command, `/me`, any other word and any `--channel` token are field
//! errors. Pure: no slash dispatcher, no `stream_manager`, no network.

use crate::models::automations::{is_chatter_text, placeholders};
use crate::services::slash_dispatch::{
    followers_setting, marker_description, resolve_alias, slow_seconds, title_or_query,
};

/// Twitch's title limit. The engine may not name `stream_manager` (source pin);
/// `slash_dispatch/tests.rs` asserts this equals `stream_manager::TITLE_MAX`.
pub const TITLE_MAX: usize = 140;
/// A marker description's limit.
pub const MARKER_MAX: usize = 140;
/// Twitch's slow mode range, in seconds.
pub const SLOW_RANGE: std::ops::RangeInclusive<i64> = 3..=120;
/// Twitch's followers-only range, in minutes (3 months).
pub const FOLLOWERS_MAX_MINUTES: i64 = 129_600;

/// One typed command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandAction {
    Slow(i64),
    SlowOff,
    /// Minutes; 0 lets anyone who follows talk.
    Followers(i64),
    FollowersOff,
    Subscribers,
    SubscribersOff,
    UniqueChat,
    UniqueChatOff,
    EmoteOnly,
    EmoteOnlyOff,
    /// 0 to 140 characters; None sends no description.
    Marker(Option<String>),
    SetTitle(String),
    /// The category's name as Twitch spells it (resolved at save).
    SetGame(String),
}

/// What a form takes after its word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArgKind {
    None,
    /// Seconds, optional (`/slow`).
    Seconds,
    /// A duration, optional (`/followers 10m`).
    Duration,
    /// Free text, optional (`/marker`).
    Text,
    /// Free text, required (`/settitle`, `/setgame`).
    RequiredText,
}

/// One allowed form, in the order the picker lists them. Mirrored by
/// `tests/fixtures/automation_command_forms.json`, the TS list and Swift's.
pub struct Form {
    pub word: &'static str,
    pub label: &'static str,
    pub arg: ArgKind,
    /// Timer and Clock rules with Own and an owner equal to `send_as` only.
    pub own_timed_only: bool,
}

pub const COMMAND_FORMS: &[Form] = &[
    Form {
        word: "slow",
        label: "Slow mode on",
        arg: ArgKind::Seconds,
        own_timed_only: false,
    },
    Form {
        word: "slowoff",
        label: "Slow mode off",
        arg: ArgKind::None,
        own_timed_only: false,
    },
    Form {
        word: "followers",
        label: "Followers-only on",
        arg: ArgKind::Duration,
        own_timed_only: false,
    },
    Form {
        word: "followersoff",
        label: "Followers-only off",
        arg: ArgKind::None,
        own_timed_only: false,
    },
    Form {
        word: "subscribers",
        label: "Subscribers-only on",
        arg: ArgKind::None,
        own_timed_only: false,
    },
    Form {
        word: "subscribersoff",
        label: "Subscribers-only off",
        arg: ArgKind::None,
        own_timed_only: false,
    },
    Form {
        word: "uniquechat",
        label: "Unique chat on",
        arg: ArgKind::None,
        own_timed_only: false,
    },
    Form {
        word: "uniquechatoff",
        label: "Unique chat off",
        arg: ArgKind::None,
        own_timed_only: false,
    },
    Form {
        word: "emoteonly",
        label: "Emote-only on",
        arg: ArgKind::None,
        own_timed_only: false,
    },
    Form {
        word: "emoteonlyoff",
        label: "Emote-only off",
        arg: ArgKind::None,
        own_timed_only: false,
    },
    Form {
        word: "marker",
        label: "Add a stream marker",
        arg: ArgKind::Text,
        own_timed_only: false,
    },
    Form {
        word: "settitle",
        label: "Set the stream title",
        arg: ArgKind::RequiredText,
        own_timed_only: true,
    },
    Form {
        word: "setgame",
        label: "Set the category",
        arg: ArgKind::RequiredText,
        own_timed_only: true,
    },
];

pub fn form_of(word: &str) -> Option<&'static Form> {
    COMMAND_FORMS.iter().find(|f| f.word == word)
}

pub const NOT_ALLOWED: &str = "Automations can run only the commands in the list";
pub const NO_CHANNEL: &str = "A command can't name another channel";
pub const NEEDS_SLASH: &str = "Start the command with /";
const NEEDS_TEXT: &str = "Write the text after the command";

/// What `check_line` needs from the rule.
pub struct SaveCtx {
    /// The rule is a Timer or Clock with Own.
    pub timed_own: bool,
    /// The rule's owner is the account that acts.
    pub owner_is_send_as: bool,
    /// The chatter's words may appear (the chatter-text rule).
    pub chatter_text_ok: bool,
}

/// The line split into (word, rest): `/Slow  60 ` is ("slow", "60").
fn split(line: &str) -> Result<(String, String), String> {
    let line = line.trim();
    let body = line
        .strip_prefix('/')
        .ok_or_else(|| NEEDS_SLASH.to_string())?;
    let (word, rest) = body.split_once(char::is_whitespace).unwrap_or((body, ""));
    Ok((resolve_alias(word.trim()), rest.trim().to_string()))
}

fn names_channel(rest: &str) -> bool {
    rest.split_whitespace()
        .any(|t| t == "--channel" || t.starts_with("--channel="))
}

/// The command word of a stored line (aliases resolved), "" when it has none.
pub fn word_of(line: &str) -> String {
    split(line).map(|(word, _)| word).unwrap_or_default()
}

/// The line with its alias resolved and spacing settled; an `Err` is the
/// field error. Arguments are kept as typed (they may hold a variable).
pub fn canonical_line(line: &str) -> Result<String, String> {
    let (word, rest) = split(line)?;
    form_of(&word).ok_or_else(|| NOT_ALLOWED.to_string())?;
    if names_channel(&rest) {
        return Err(NO_CHANNEL.into());
    }
    Ok(if rest.is_empty() {
        format!("/{word}")
    } else {
        format!("/{word} {rest}")
    })
}

/// Parse a line with no variable left in it. Pure and synchronous: the same
/// function runs at save (for a line with no variable) and at fire.
pub fn parse_command(line: &str) -> Result<CommandAction, String> {
    let (word, rest) = split(line)?;
    let form = form_of(&word).ok_or_else(|| NOT_ALLOWED.to_string())?;
    if names_channel(&rest) {
        return Err(NO_CHANNEL.into());
    }
    let first = rest.split_whitespace().next().unwrap_or("");
    Ok(match word.as_str() {
        "slow" => {
            let secs = slow_seconds(if first.is_empty() { None } else { Some(first) });
            if !SLOW_RANGE.contains(&secs) {
                return Err("Slow mode takes 3 to 120 seconds".into());
            }
            CommandAction::Slow(secs)
        }
        "slowoff" => CommandAction::SlowOff,
        "followers" => match followers_setting(first) {
            Some(m) if (0..=FOLLOWERS_MAX_MINUTES).contains(&m) => CommandAction::Followers(m),
            Some(_) => return Err("Followers-only takes up to 3 months".into()),
            None => return Err("Use a duration like 10m, 2h or 1d".into()),
        },
        "followersoff" => CommandAction::FollowersOff,
        "subscribers" => CommandAction::Subscribers,
        "subscribersoff" => CommandAction::SubscribersOff,
        "uniquechat" => CommandAction::UniqueChat,
        "uniquechatoff" => CommandAction::UniqueChatOff,
        "emoteonly" => CommandAction::EmoteOnly,
        "emoteonlyoff" => CommandAction::EmoteOnlyOff,
        "marker" => {
            let text = marker_description(&rest);
            if text
                .as_ref()
                .is_some_and(|t| t.chars().count() > MARKER_MAX)
            {
                return Err("Keep a marker to 140 characters".into());
            }
            CommandAction::Marker(text)
        }
        "settitle" | "setgame" => {
            let text = title_or_query(&rest);
            if text.is_empty() {
                return Err(NEEDS_TEXT.into());
            }
            if word == "settitle" {
                if text.chars().count() > TITLE_MAX {
                    return Err("Titles can be at most 140 characters".into());
                }
                CommandAction::SetTitle(text)
            } else {
                CommandAction::SetGame(text)
            }
        }
        _ => {
            let _ = form;
            return Err(NOT_ALLOWED.into());
        }
    })
}

/// The save check: the word and arguments, the gating of `/settitle` and
/// `/setgame`, and the chatter-text rule for any variable.
pub fn check_line(line: &str, ctx: &SaveCtx) -> Result<(), String> {
    let (word, rest) = split(line)?;
    let form = form_of(&word).ok_or_else(|| NOT_ALLOWED.to_string())?;
    if names_channel(&rest) {
        return Err(NO_CHANNEL.into());
    }
    if form.own_timed_only && !(ctx.timed_own && ctx.owner_is_send_as) {
        return Err(
            "This works on a Timer or Clock rule in your own channel, set to act as the owner"
                .into(),
        );
    }
    let names = placeholders(line);
    if names.iter().any(|n| is_chatter_text(n)) && !ctx.chatter_text_ok {
        return Err("The chatter's words can't steer a command".into());
    }
    if names.is_empty() {
        parse_command(line).map(|_| ())
    } else {
        // Variables fill at fire; the word and the shape are checked now.
        match form.arg {
            ArgKind::RequiredText if rest.is_empty() => Err(NEEDS_TEXT.into()),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::command_catalog::BUILTINS;

    fn ctx() -> SaveCtx {
        SaveCtx {
            timed_own: false,
            owner_is_send_as: true,
            chatter_text_ok: false,
        }
    }

    #[test]
    fn each_form_parses_to_its_action() {
        assert_eq!(parse_command("/slow"), Ok(CommandAction::Slow(30)));
        assert_eq!(parse_command("/slow 60"), Ok(CommandAction::Slow(60)));
        assert_eq!(parse_command("/slowoff"), Ok(CommandAction::SlowOff));
        assert_eq!(parse_command("/followers"), Ok(CommandAction::Followers(0)));
        assert_eq!(
            parse_command("/followers 10m"),
            Ok(CommandAction::Followers(10))
        );
        assert_eq!(
            parse_command("/followersoff"),
            Ok(CommandAction::FollowersOff)
        );
        assert_eq!(
            parse_command("/subscribers"),
            Ok(CommandAction::Subscribers)
        );
        assert_eq!(
            parse_command("/subscribersoff"),
            Ok(CommandAction::SubscribersOff)
        );
        assert_eq!(parse_command("/uniquechat"), Ok(CommandAction::UniqueChat));
        assert_eq!(
            parse_command("/uniquechatoff"),
            Ok(CommandAction::UniqueChatOff)
        );
        assert_eq!(parse_command("/emoteonly"), Ok(CommandAction::EmoteOnly));
        assert_eq!(
            parse_command("/emoteonlyoff"),
            Ok(CommandAction::EmoteOnlyOff)
        );
        assert_eq!(
            parse_command("/marker a b"),
            Ok(CommandAction::Marker(Some("a b".into())))
        );
        assert_eq!(parse_command("/marker"), Ok(CommandAction::Marker(None)));
        assert_eq!(
            parse_command("/settitle x y"),
            Ok(CommandAction::SetTitle("x y".into()))
        );
        assert_eq!(
            parse_command("/setgame y"),
            Ok(CommandAction::SetGame("y".into()))
        );
        assert_eq!(COMMAND_FORMS.len(), 13);
    }

    #[test]
    fn aliases_resolve_and_every_other_builtin_is_a_field_error() {
        for b in BUILTINS {
            let allowed = COMMAND_FORMS.iter().any(|f| f.word == b.name);
            assert_eq!(
                canonical_line(&format!("/{}", b.name)).is_ok(),
                allowed,
                "{}",
                b.name
            );
            for alias in b.aliases {
                assert_eq!(
                    canonical_line(&format!("/{alias}")).is_ok(),
                    allowed,
                    "alias {alias}"
                );
                if allowed {
                    assert_eq!(
                        canonical_line(&format!("/{alias}")).unwrap(),
                        format!("/{}", b.name)
                    );
                }
            }
        }
        for banned in [
            "/ban x",
            "/timeout x 5",
            "/w x hi",
            "/announce hi",
            "/shoutout x",
            "/me hi",
            "/spam 5 x",
            "/sub",
        ] {
            assert!(canonical_line(banned).is_err(), "{banned}");
        }
    }

    #[test]
    fn a_custom_command_alias_and_channel_flag_are_refused() {
        assert_eq!(
            canonical_line("/sub"),
            Err(NOT_ALLOWED.into()),
            "a saved custom `sub` is not a built-in"
        );
        assert_eq!(
            canonical_line("/slow 5 --channel x"),
            Err(NO_CHANNEL.into())
        );
        assert_eq!(
            canonical_line("/slow 5 --channel=x"),
            Err(NO_CHANNEL.into())
        );
        assert_eq!(parse_command("/slow --channel x"), Err(NO_CHANNEL.into()));
    }

    #[test]
    fn limits_hold_at_save_and_after_expansion() {
        let title = |n: usize| format!("/settitle {}", "t".repeat(n));
        assert!(parse_command(&title(140)).is_ok());
        assert!(parse_command(&title(141)).is_err());
        assert!(parse_command(&format!("/marker {}", "m".repeat(140))).is_ok());
        assert!(parse_command(&format!("/marker {}", "m".repeat(141))).is_err());
        assert_eq!(
            parse_command("/followers m"),
            Err("Use a duration like 10m, 2h or 1d".into())
        );
        assert!(parse_command("/slow 2").is_err() && parse_command("/slow 121").is_err());
        assert!(parse_command("/settitle").is_err());
    }

    #[test]
    fn settitle_and_setgame_are_timed_own_only() {
        let mut c = ctx();
        assert!(check_line("/settitle x", &c).is_err());
        c.timed_own = true;
        assert!(check_line("/settitle x", &c).is_ok());
        c.owner_is_send_as = false;
        assert!(check_line("/setgame y", &c).is_err());
        assert!(
            check_line("/slow 30", &ctx()).is_ok(),
            "chat settings keep their trigger scope"
        );
    }

    #[test]
    fn chatter_text_cannot_steer_a_command() {
        assert!(check_line("/slow {1}", &ctx()).is_err());
        let ok = SaveCtx {
            chatter_text_ok: true,
            ..ctx()
        };
        assert!(check_line("/slow {1}", &ok).is_ok());
        assert!(check_line(
            "/settitle {game} night",
            &SaveCtx {
                timed_own: true,
                ..ctx()
            }
        )
        .is_ok());
    }

    #[test]
    fn the_forms_equal_the_committed_fixture() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/automation_command_forms.json"
        ))
        .expect("the fixture is JSON");
        let rows: Vec<(String, bool)> = fixture
            .as_array()
            .unwrap()
            .iter()
            .map(|r| {
                (
                    r["word"].as_str().unwrap().to_string(),
                    r["own_timed_only"].as_bool().unwrap(),
                )
            })
            .collect();
        let ours: Vec<(String, bool)> = COMMAND_FORMS
            .iter()
            .map(|f| (f.word.to_string(), f.own_timed_only))
            .collect();
        assert_eq!(rows, ours);
    }
}
