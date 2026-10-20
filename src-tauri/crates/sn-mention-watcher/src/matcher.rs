//! One chat message against everyone watching its channel, with the app's
//! own rule (`sn_mention_rule`).

use serde::Serialize;
use sn_mention_rule::judge::{judge, Input};
use sn_mention_rule::normalize::normalize;

use crate::parse::Privmsg;
use crate::targets::LiveSet;

/// The longest text handed on, in characters.
pub const TEXT_MAX: usize = 500;

/// One match, as `POST /watcher/mentions` takes it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Mention {
    pub user_id: String,
    pub msg_id: String,
    pub channel_id: String,
    pub channel_login: String,
    pub sender_id: String,
    pub sender_login: String,
    pub sender_name: String,
    pub color: String,
    pub text: String,
    pub kind: &'static str,
    pub reply_parent_id: Option<String>,
    pub at_ms: i64,
}

/// Every watcher of the message's channel it names or replies to.
pub fn matches(msg: &Privmsg, live: &LiveSet) -> Vec<Mention> {
    let Some(channel) = live.channels.get(&msg.room_id) else {
        return Vec::new();
    };
    if msg.id.is_empty() {
        return Vec::new();
    }
    let text = normalize(&msg.text, msg.reply_parent_display.as_deref(), msg.reply_parent_login.as_deref());
    let input = Input {
        sender_id: &msg.sender_id,
        sender_login: &msg.sender_login,
        sender_display: &msg.sender_display,
        content: &text,
        reply_parent_user_id: msg.reply_parent_user_id.as_deref(),
    };
    let mut out = Vec::new();
    for user_id in &channel.users {
        let Some(rules) = live.users.get(user_id) else { continue };
        if rules.muted.contains(&msg.room_id) {
            continue;
        }
        let Some(kind) = judge(&input, &rules.identity, rules.opts, &rules.hidden) else { continue };
        out.push(Mention {
            user_id: user_id.clone(),
            msg_id: msg.id.clone(),
            channel_id: msg.room_id.clone(),
            channel_login: channel.login.clone(),
            sender_id: msg.sender_id.clone(),
            sender_login: msg.sender_login.clone(),
            sender_name: if msg.sender_display.is_empty() { msg.sender_login.clone() } else { msg.sender_display.clone() },
            color: msg.color.clone(),
            text: text.chars().take(TEXT_MAX).collect(),
            kind: kind.as_str(),
            reply_parent_id: msg.reply_parent_msg_id.clone(),
            at_ms: msg.sent_ms,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::{parse, Line};
    use crate::targets::Answer;

    fn live() -> LiveSet {
        let raw = r##"{
            "channels": [{"id": "1", "login": "forsen", "users": ["100", "200"]}],
            "users": [
                {"id": "100", "login": "brandon", "display": "Brandon"},
                {"id": "200", "login": "alice", "display": "Alice", "hidden_user_ids": ["bob"]}
            ]
        }"##;
        LiveSet::from_answer(serde_json::from_str::<Answer>(raw).unwrap())
    }

    fn privmsg(line: &str) -> Privmsg {
        match parse(line) {
            Line::Privmsg(m) => *m,
            other => panic!("not a PRIVMSG: {other:?}"),
        }
    }

    #[test]
    fn one_message_can_name_two_watchers() {
        let m = privmsg("@display-name=Carl;id=m1;room-id=1;tmi-sent-ts=42;user-id=7;color=#FF0000 :carl!carl@carl.tmi.twitch.tv PRIVMSG #forsen :brandon and alice gg");
        let got = matches(&m, &live());
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].user_id, "100");
        assert_eq!(got[0].kind, "mention");
        assert_eq!(got[0].sender_name, "Carl");
        assert_eq!(got[0].at_ms, 42);
        assert_eq!(got[1].user_id, "200");
    }

    #[test]
    fn a_reply_is_judged_on_the_text_without_the_parent() {
        let m = privmsg("@display-name=Bob;id=m2;reply-parent-display-name=Brandon;reply-parent-msg-id=p9;reply-parent-user-id=100;reply-parent-user-login=brandon;room-id=1;tmi-sent-ts=1;user-id=8 :bob!bob@bob.tmi.twitch.tv PRIVMSG #forsen :@Brandon alice said hi");
        let got = matches(&m, &live());
        // Brandon gets the reply; Alice hid Bob, so her name in it does not count.
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].user_id, "100");
        assert_eq!(got[0].kind, "reply");
        assert_eq!(got[0].text, "alice said hi");
        assert_eq!(got[0].reply_parent_id.as_deref(), Some("p9"));
    }

    #[test]
    fn other_channels_and_idless_lines_are_ignored() {
        let elsewhere = privmsg("@id=m3;room-id=999;user-id=7 :carl!carl@carl.tmi.twitch.tv PRIVMSG #other :@brandon");
        assert!(matches(&elsewhere, &live()).is_empty());
        let no_id = privmsg("@room-id=1;user-id=7 :carl!carl@carl.tmi.twitch.tv PRIVMSG #forsen :@brandon");
        assert!(matches(&no_id, &live()).is_empty());
    }

    #[test]
    fn long_text_is_cut_by_characters() {
        let body = "브".repeat(600);
        let line = format!("@id=m4;room-id=1;user-id=7 :carl!carl@carl.tmi.twitch.tv PRIVMSG #forsen :@brandon {body}");
        let got = matches(&privmsg(&line), &live());
        assert_eq!(got[0].text.chars().count(), TEXT_MAX);
    }
}
