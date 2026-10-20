//! The verdict for one message and one person: does it name them, reply to
//! them, or neither.
//!
//! The app stamps its rows with the same pieces (`is_known_bot`,
//! `is_reply_to`, `name::find_mention`), and the mention watcher calls `judge`
//! for every person watching the channel a message arrived in.

use std::collections::HashSet;

use crate::name::{find_mention, MatchOptions, MentionTarget};

/// Chat bots, by login. Same list as src/utils/knownBots.ts. Keep the two in step.
pub const KNOWN_BOTS: &[&str] = &[
    "nightbot",
    "streamelements",
    "streamlabs",
    "moobot",
    "fossabot",
    "wizebot",
    "sery_bot",
    "commanderroot",
    "soundtrackbot",
    "streamlootsbot",
    "pretzelrocks",
    "tangiabot",
    "blerp",
    "kofistreambot",
    "own3d",
    "botrixoficial",
    "coebot",
    "phantombot",
    "thepositivebot",
    "streamstickers",
    "lattemotte",
    "restreambot",
    "supibot",
    "anotherttvviewer",
    "streamdatabase",
    "streamdbbot",
    "potatbotat",
    "pajbot",
    "titlechange_bot",
    "buttsbot",
    "snusbot",
    "deepbot",
    "ankhbot",
    "vivbot",
    "revlobot",
    "dixperbro",
    "botisimo",
    "mikuia",
    "wzbot",
    "own3dpro_bot",
    "playwithviewersbot",
    "thepixelbot",
    "cloudbot",
    "9gag",
];

/// The sender is a known chat bot, by login or display name (both lowercase).
pub fn is_known_bot(login: &str, display_lower: &str) -> bool {
    KNOWN_BOTS.contains(&login) || KNOWN_BOTS.contains(&display_lower)
}

/// The message replies to the person with this Twitch user id.
pub fn is_reply_to(user_id: &str, reply_parent_user_id: Option<&str>) -> bool {
    !user_id.is_empty() && reply_parent_user_id == Some(user_id)
}

/// Who is being looked for: the Twitch account and the names that count.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Identity {
    pub user_id: String,
    pub target: MentionTarget,
}

impl Identity {
    pub fn new(user_id: &str, login: &str, display_name: &str) -> Self {
        Identity {
            user_id: user_id.to_string(),
            target: MentionTarget::new(login, display_name),
        }
    }
}

/// One chat message, its text already normalized (`normalize::normalize`).
#[derive(Clone, Copy, Debug, Default)]
pub struct Input<'a> {
    pub sender_id: &'a str,
    pub sender_login: &'a str,
    pub sender_display: &'a str,
    pub content: &'a str,
    pub reply_parent_user_id: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Mention,
    Reply,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Mention => "mention",
            Kind::Reply => "reply",
        }
    }
}

/// Whether `input` names or replies to `id`. Never for their own message, a
/// known bot's, or a sender in `hidden` (user ids or lowercase logins). A reply
/// to them wins over a name in the text.
pub fn judge(input: &Input<'_>, id: &Identity, opts: MatchOptions, hidden: &HashSet<String>) -> Option<Kind> {
    let login = input.sender_login.to_lowercase();
    let own = (!id.user_id.is_empty() && input.sender_id == id.user_id)
        || (!id.target.login.is_empty() && login == id.target.login);
    if own {
        return None;
    }
    let display_lower = input.sender_display.to_lowercase();
    if is_known_bot(&login, &display_lower) {
        return None;
    }
    if hidden.contains(input.sender_id) || hidden.contains(&login) || (!display_lower.is_empty() && hidden.contains(&display_lower)) {
        return None;
    }
    if is_reply_to(&id.user_id, input.reply_parent_user_id) {
        return Some(Kind::Reply);
    }
    find_mention(input.content, &id.target, &opts).map(|_| Kind::Mention)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn me() -> Identity {
        Identity::new("100", "brandon", "Brandon")
    }

    fn msg<'a>(sender_id: &'a str, login: &'a str, content: &'a str, parent: Option<&'a str>) -> Input<'a> {
        Input {
            sender_id,
            sender_login: login,
            sender_display: login,
            content,
            reply_parent_user_id: parent,
        }
    }

    #[test]
    fn a_name_is_a_mention_and_a_reply_wins() {
        let none = HashSet::new();
        assert_eq!(judge(&msg("2", "alice", "hi @brandon", None), &me(), MatchOptions::default(), &none), Some(Kind::Mention));
        assert_eq!(judge(&msg("2", "alice", "hi @brandon", Some("100")), &me(), MatchOptions::default(), &none), Some(Kind::Reply));
        assert_eq!(judge(&msg("2", "alice", "hello", Some("7")), &me(), MatchOptions::default(), &none), None);
    }

    #[test]
    fn own_bot_and_hidden_senders_never_count() {
        let hidden: HashSet<String> = ["carl".to_string(), "55".to_string()].into_iter().collect();
        assert_eq!(judge(&msg("100", "brandon", "@brandon", None), &me(), MatchOptions::default(), &hidden), None);
        assert_eq!(judge(&msg("3", "Nightbot", "@brandon", None), &me(), MatchOptions::default(), &hidden), None);
        assert_eq!(judge(&msg("4", "Carl", "@brandon", None), &me(), MatchOptions::default(), &hidden), None);
        assert_eq!(judge(&msg("55", "dave", "@brandon", Some("100")), &me(), MatchOptions::default(), &hidden), None);
    }

    #[test]
    fn a_reply_needs_a_user_id() {
        let none = HashSet::new();
        let nobody = Identity::new("", "brandon", "");
        assert_eq!(judge(&msg("2", "alice", "hello", Some("")), &nobody, MatchOptions::default(), &none), None);
    }
}
