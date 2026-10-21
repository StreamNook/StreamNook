//! The name a chat row shows for its author, decided once in Rust so every row
//! type (plain message, sub, watch streak, raid, redemption, reply header)
//! shows the same person the same way.
//!
//! Twitch carries two names: the login (lowercase ASCII, what `@` mentions and
//! commands resolve) and the display name, which may differ from the login only
//! by case ("Mochi_Panda") or be localized entirely ("もちパンダ"). The rule:
//!
//! - a display name that differs from the login only by case or spaces is shown
//!   as is ("Mochi_Panda", and "Call of Duty" for `callofduty`);
//! - a localized display name is shown as `display (login)`, so the name people
//!   type to reach that chatter is always visible;
//! - a missing display name falls back to the login.
//!
//! The viewer's own nickname override is applied on top by the page, which
//! holds the live override map.
//!
//! Some USERNOTICE rows (watch-streak milestones among them) arrive with the
//! login in `display-name` even for a chatter whose PRIVMSGs carry a localized
//! one. Rows are therefore taught the localized name seen on the chatter's own
//! messages, keyed by user id: an id is the only stable identity, a name never is.
//!
//! A few Twitch accounts carry spaces in their display name ("Call of Duty").
//! Those names are kept too, keyed by the name (what people type), so
//! `@Call of Duty` in chat reads as one mention (`text_spans`).

use crate::models::chat_layout::ChatMessage;
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::sync::Mutex;

/// Upper bound on remembered localized names. Only localized names are kept,
/// so this is far above what a session meets; past it the map starts over.
const MAX_KNOWN: usize = 20_000;

static KNOWN_LOCALIZED: Lazy<Mutex<HashMap<String, String>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn same_ignoring_case(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

/// Same name apart from capital letters and spaces. Twitch logins carry
/// neither, so "Call of Duty" is simply how `callofduty` is written.
fn same_name(display: &str, login: &str) -> bool {
    let squash = |s: &str| -> String { s.chars().filter(|c| !c.is_whitespace()).flat_map(char::to_lowercase).collect() };
    squash(display) == squash(login)
}

/// Upper bound on remembered spaced display names. Only a handful of Twitch
/// accounts carry one; past it the map starts over.
const MAX_SPACED: usize = 2_000;

/// Twitch display names with spaces in them, lowercased, to the login.
static KNOWN_SPACED: Lazy<Mutex<HashMap<String, String>>> = Lazy::new(|| Mutex::new(HashMap::new()));

/// Remember a Twitch display name that carries spaces. Called as rows are
/// parsed, before their text is cut into segments, so a name is known by the
/// time the next row mentions it.
pub(crate) fn note_spaced(login: &str, display: &str) {
    let display = display.trim();
    let login = login.trim();
    if login.is_empty() || !display.contains(char::is_whitespace) {
        return;
    }
    if let Ok(mut known) = KNOWN_SPACED.lock() {
        let key = display.to_lowercase();
        if known.len() >= MAX_SPACED && !known.contains_key(&key) {
            known.clear();
        }
        known.insert(key, login.to_lowercase());
    }
}

/// The longest known spaced name `text` starts with (ignoring case) that ends
/// where a mention may end: its byte length in `text`, and the login.
pub fn spaced_name_at(text: &str) -> Option<(usize, String)> {
    let known = KNOWN_SPACED.lock().ok()?;
    let mut best: Option<(usize, String)> = None;
    for (name, login) in known.iter() {
        let Some(len) = prefix_len_ignoring_case(text, name) else { continue };
        if !crate::services::text_spans::ends_mention(text[len..].chars().next()) {
            continue;
        }
        if best.as_ref().is_none_or(|(b, _)| len > *b) {
            best = Some((len, login.clone()));
        }
    }
    best
}

fn prefix_len_ignoring_case(text: &str, name: &str) -> Option<usize> {
    let mut chars = text.char_indices();
    for nc in name.chars() {
        let (_, tc) = chars.next()?;
        if !tc.to_lowercase().eq(nc.to_lowercase()) {
            return None;
        }
    }
    Some(chars.next().map(|(i, _)| i).unwrap_or(text.len()))
}

/// The shown name for a chatter. `login` and `display` as the platform sent
/// them; the `(login)` suffix applies to Twitch only, where the login is the
/// handle people type. Other platforms show their display name.
pub fn shown_name(provider: &str, login: &str, display: &str) -> String {
    let display = display.trim();
    let login = login.trim();
    if display.is_empty() {
        return login.to_string();
    }
    if provider != "twitch" || login.is_empty() || same_name(display, login) {
        return display.to_string();
    }
    format!("{display} ({login})")
}

fn remember(user_id: &str, display: &str) {
    if let Ok(mut known) = KNOWN_LOCALIZED.lock() {
        if known.len() >= MAX_KNOWN && !known.contains_key(user_id) {
            known.clear();
        }
        known.insert(user_id.to_string(), display.to_string());
    }
}

fn forget(user_id: &str) {
    if let Ok(mut known) = KNOWN_LOCALIZED.lock() {
        known.remove(user_id);
    }
}

fn known(user_id: &str) -> Option<String> {
    KNOWN_LOCALIZED.lock().ok()?.get(user_id).cloned()
}

/// Settle a row's author names: fill a Twitch display name the row lacks from
/// what the chatter's own messages showed, stamp the shown name on the row and
/// on its reply parent, then cut the row's mentions (and links off Twitch) out
/// of its text. Idempotent.
pub fn stamp(msg: &mut ChatMessage) {
    if msg.provider == "twitch" {
        note_spaced(&msg.username, &msg.display_name);
    }
    if msg.provider == "twitch" && !msg.user_id.is_empty() {
        let login = msg.username.as_str();
        let display = msg.display_name.trim();
        let localized = !display.is_empty() && !same_ignoring_case(display, login);
        if localized {
            remember(&msg.user_id, display);
        } else if msg.tags.contains_key("system-msg") {
            // A notice whose display name is missing or just the login: use
            // the localized name the chatter's own messages carry, if any.
            if let Some(name) = known(&msg.user_id) {
                msg.display_name = name;
            }
        } else if !display.is_empty() {
            // A plain message is authoritative: a chatter who dropped their
            // localized name no longer has one.
            forget(&msg.user_id);
        }
    }

    msg.metadata.shown_name = shown_name(&msg.provider, &msg.username, &msg.display_name);

    if let Some(reply) = msg.metadata.reply_info.as_mut() {
        if msg.provider == "twitch" {
            note_spaced(&reply.parent_user_login, &reply.parent_display_name);
        }
        let parent_display = if reply.parent_display_name.trim().is_empty()
            || same_ignoring_case(&reply.parent_display_name, &reply.parent_user_login)
        {
            known(&reply.parent_user_id)
                .filter(|_| !reply.parent_user_id.is_empty() && msg.provider == "twitch")
                .unwrap_or_else(|| reply.parent_display_name.clone())
        } else {
            reply.parent_display_name.clone()
        };
        reply.parent_shown_name =
            shown_name(&msg.provider, &reply.parent_user_login, &parent_display);
    }

    crate::services::text_spans::apply_row(msg);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::chat_layout::{LayoutResult, MessageMetadata, ReplyInfo};

    fn row(user_id: &str, login: &str, display: &str, notice: bool) -> ChatMessage {
        let mut tags = HashMap::new();
        tags.insert("user-id".to_string(), user_id.to_string());
        tags.insert("display-name".to_string(), display.to_string());
        if notice {
            tags.insert("msg-id".to_string(), "viewermilestone".to_string());
            tags.insert("system-msg".to_string(), format!("{login} watched 3 consecutive streams"));
        }
        ChatMessage {
            id: format!("{user_id}-{notice}"),
            user_id: user_id.to_string(),
            username: login.to_string(),
            display_name: display.to_string(),
            color: None,
            badges: Vec::new(),
            timestamp: "0".to_string(),
            content: String::new(),
            provider: "twitch".to_string(),
            channel: "chan".to_string(),
            emotes: Vec::new(),
            tags,
            layout: LayoutResult::default(),
            segments: Vec::new(),
            metadata: MessageMetadata::default(),
        }
    }

    #[test]
    fn case_only_difference_shows_display_name() {
        assert_eq!(shown_name("twitch", "mochi", "Mochi"), "Mochi");
        assert_eq!(shown_name("twitch", "mochi", "mochi"), "mochi");
    }

    #[test]
    fn a_name_that_differs_only_by_spaces_shows_as_is() {
        assert_eq!(shown_name("twitch", "callofduty", "Call of Duty"), "Call of Duty");
        assert_eq!(shown_name("twitch", "callofduty", "Call of Duty "), "Call of Duty");
        assert_eq!(shown_name("twitch", "mochi_panda", "もち パンダ"), "もち パンダ (mochi_panda)");
    }

    #[test]
    fn a_spaced_name_is_found_at_the_start_of_text() {
        note_spaced("spacedlookup", "Spaced Look Up");
        assert_eq!(spaced_name_at("spaced LOOK up! hi"), Some((14, "spacedlookup".to_string())));
        assert_eq!(spaced_name_at("Spaced Look Upper"), None);
    }

    #[test]
    fn localized_display_name_carries_the_login() {
        assert_eq!(
            shown_name("twitch", "mochi_panda", "もちパンダ"),
            "もちパンダ (mochi_panda)"
        );
    }

    #[test]
    fn missing_display_name_falls_back_to_login() {
        assert_eq!(shown_name("twitch", "mochi", ""), "mochi");
    }

    #[test]
    fn other_platforms_show_their_display_name() {
        assert_eq!(shown_name("kick", "stone916", "Stone916"), "Stone916");
        assert_eq!(shown_name("youtube", "UCabc", "Alice"), "Alice");
    }

    #[test]
    fn privmsg_and_watch_streak_resolve_to_the_same_name() {
        let mut privmsg = row("900001", "mochi_panda", "もちパンダ", false);
        stamp(&mut privmsg);
        // The milestone notice names the chatter by login only.
        let mut streak = row("900001", "mochi_panda", "mochi_panda", true);
        stamp(&mut streak);
        assert_eq!(privmsg.metadata.shown_name, "もちパンダ (mochi_panda)");
        assert_eq!(streak.metadata.shown_name, privmsg.metadata.shown_name);
        assert_eq!(streak.display_name, "もちパンダ");
    }

    #[test]
    fn notice_carrying_the_localized_name_resolves_the_same() {
        let mut privmsg = row("900002", "kurobik4ri", "クロビカリ", false);
        stamp(&mut privmsg);
        let mut sub = row("900002", "kurobik4ri", "クロビカリ", true);
        stamp(&mut sub);
        assert_eq!(sub.metadata.shown_name, privmsg.metadata.shown_name);
    }

    #[test]
    fn a_plain_message_with_a_latin_name_drops_the_remembered_one() {
        let mut before = row("900003", "renamed", "改名", false);
        stamp(&mut before);
        let mut after = row("900003", "renamed", "Renamed", false);
        stamp(&mut after);
        let mut notice = row("900003", "renamed", "renamed", true);
        stamp(&mut notice);
        assert_eq!(notice.metadata.shown_name, "renamed");
    }

    #[test]
    fn reply_parent_gets_the_same_rule() {
        let mut parent = row("900004", "parent_login", "親", false);
        stamp(&mut parent);
        let mut reply = row("900005", "child", "Child", false);
        reply.metadata.reply_info = Some(ReplyInfo {
            parent_msg_id: "p".into(),
            parent_display_name: "親".into(),
            parent_user_id: "900004".into(),
            parent_user_login: "parent_login".into(),
            ..Default::default()
        });
        stamp(&mut reply);
        assert_eq!(
            reply.metadata.reply_info.unwrap().parent_shown_name,
            "親 (parent_login)"
        );
    }

    #[test]
    fn stamping_twice_is_stable() {
        let mut m = row("900006", "twice", "二回", false);
        stamp(&mut m);
        stamp(&mut m);
        assert_eq!(m.metadata.shown_name, "二回 (twice)");
    }
}
