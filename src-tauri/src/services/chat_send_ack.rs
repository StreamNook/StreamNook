//! Twitch's answer to a chat line sent over IRC.
//!
//! Helix answers a send in its own response (the message id, or why it was
//! dropped). IRC does not: Twitch answers a PRIVMSG with a USERSTATE whose `id`
//! tag is the new message's id, or with a NOTICE whose `msg-id` says why the
//! line was refused (followers-only, slow mode, a duplicate, AutoMod held it,
//! ...). Sends over IRC register here and wait briefly for that answer, so a
//! refused line is never reported as sent and a sent one gets its real id
//! (delete, reply and mod actions need it).
//!
//! Answers are matched per channel in send order: Twitch answers each PRIVMSG
//! in turn on the one connection.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::Duration;

use tokio::sync::oneshot;

/// How long a send waits for Twitch's answer before calling it unconfirmed.
pub const ANSWER_WAIT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// Twitch posted it, under this id.
    Sent(String),
    /// Twitch refused it: the NOTICE's msg-id and its words.
    Refused { code: String, words: String },
}

fn pending() -> &'static Mutex<HashMap<String, VecDeque<oneshot::Sender<Answer>>>> {
    static PENDING: std::sync::OnceLock<Mutex<HashMap<String, VecDeque<oneshot::Sender<Answer>>>>> =
        std::sync::OnceLock::new();
    PENDING.get_or_init(Default::default)
}

/// Registers a send to `channel` before it goes out; the receiver gets
/// Twitch's answer.
pub fn expect(channel: &str) -> oneshot::Receiver<Answer> {
    let (tx, rx) = oneshot::channel();
    let mut map = pending().lock().unwrap_or_else(|e| e.into_inner());
    map.entry(channel.to_lowercase()).or_default().push_back(tx);
    rx
}

/// Hands `answer` to the oldest send still waiting in `channel`. False when
/// nothing was waiting (a USERSTATE on join, a notice about something else).
fn answer(channel: &str, answer: Answer) -> bool {
    let mut map = pending().lock().unwrap_or_else(|e| e.into_inner());
    let Some(queue) = map.get_mut(&channel.to_lowercase()) else { return false };
    while let Some(tx) = queue.pop_front() {
        // A send that stopped waiting (timed out) dropped its receiver.
        if !tx.is_closed() {
            let _ = tx.send(answer);
            return true;
        }
    }
    false
}

/// A USERSTATE: with an `id` tag it answers the oldest send in the channel.
pub fn on_userstate(channel: &str, id: Option<&str>) -> bool {
    match id.filter(|id| !id.is_empty()) {
        Some(id) => answer(channel, Answer::Sent(id.to_string())),
        None => false,
    }
}

/// A NOTICE: one that refuses a line answers the oldest send in the channel.
pub fn on_notice(channel: &str, msg_id: &str, words: &str) -> bool {
    if !refuses_a_line(msg_id) {
        return false;
    }
    answer(channel, Answer::Refused { code: msg_id.to_string(), words: words.to_string() })
}

/// NOTICE msg-ids that mean the line just sent was not posted.
pub fn refuses_a_line(msg_id: &str) -> bool {
    matches!(
        msg_id,
        "msg_banned"
            | "msg_bad_characters"
            | "msg_channel_blocked"
            | "msg_channel_suspended"
            | "msg_duplicate"
            | "msg_emoteonly"
            | "msg_facebook"
            | "msg_followersonly"
            | "msg_followersonly_followed"
            | "msg_followersonly_zero"
            | "msg_r9k"
            | "msg_ratelimit"
            | "msg_rejected"
            | "msg_rejected_mandatory"
            | "msg_requires_verified_phone_number"
            | "msg_slowmode"
            | "msg_subsonly"
            | "msg_suspended"
            | "msg_timedout"
            | "msg_verified_email"
            | "msg_warned"
    )
}

/// AutoMod is holding the message for the room's moderators: not posted, and
/// not refused either. Matches the IRC msg-id or Helix's drop code or words.
pub fn is_held(reason: &str) -> bool {
    let lower = reason.to_lowercase();
    lower.starts_with("msg_rejected") || lower.contains("automod") || lower.contains("checked by mods")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_userstate_with_an_id_answers_the_oldest_send() {
        let first = expect("RoomA");
        let second = expect("rooma");
        assert!(!on_userstate("rooma", None), "a join USERSTATE answers nothing");
        assert!(on_userstate("rooma", Some("id-1")));
        assert!(on_userstate("rooma", Some("id-2")));
        assert_eq!(first.await.unwrap(), Answer::Sent("id-1".into()));
        assert_eq!(second.await.unwrap(), Answer::Sent("id-2".into()));
    }

    #[tokio::test]
    async fn a_refusal_notice_answers_and_others_do_not() {
        let sent = expect("roomb");
        assert!(!on_notice("roomb", "host_on", "Now hosting"), "not about a line");
        assert!(on_notice("roomb", "msg_followersonly", "This room is in followers-only mode."));
        assert_eq!(
            sent.await.unwrap(),
            Answer::Refused { code: "msg_followersonly".into(), words: "This room is in followers-only mode.".into() }
        );
    }

    #[tokio::test]
    async fn a_send_that_stopped_waiting_is_skipped() {
        let gone = expect("roomc");
        drop(gone);
        let waiting = expect("roomc");
        assert!(on_userstate("roomc", Some("id-9")));
        assert_eq!(waiting.await.unwrap(), Answer::Sent("id-9".into()));
        assert!(!on_userstate("roomd", Some("id-x")), "nothing waiting in another room");
    }

    #[test]
    fn automod_holds_read_as_held() {
        assert!(is_held("msg_rejected"));
        assert!(is_held("msg_rejected_mandatory"));
        assert!(is_held("Your message is being checked by mods and has not been sent."));
        assert!(is_held("Message held by AutoMod"));
        assert!(!is_held("This room is in followers-only mode."));
    }
}
