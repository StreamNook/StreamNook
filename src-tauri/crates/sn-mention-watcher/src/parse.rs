//! Twitch IRC lines, just enough of them: PRIVMSG with its tags, PING,
//! RECONNECT, NOTICE and a JOIN refusal.

use std::collections::HashMap;

/// One chat message, tags read, text as sent (normalizing is the rule's job).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Privmsg {
    pub id: String,
    pub room_id: String,
    pub channel_login: String,
    pub sender_id: String,
    pub sender_login: String,
    pub sender_display: String,
    pub color: String,
    pub sent_ms: i64,
    pub text: String,
    pub reply_parent_msg_id: Option<String>,
    pub reply_parent_user_id: Option<String>,
    pub reply_parent_login: Option<String>,
    pub reply_parent_display: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Line {
    Privmsg(Box<Privmsg>),
    /// Answer with `PONG :<arg>`.
    Ping(String),
    /// Twitch is about to drop this connection: open another.
    Reconnect,
    /// A NOTICE (`msg-id` tag, channel, text), logged; a join refusal shows up here.
    Notice(String, String, String),
    Other,
}

/// Undo IRCv3 tag escaping.
fn unescape(value: &str) -> String {
    if !value.contains('\\') {
        return value.to_string();
    }
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some(':') => out.push(';'),
            Some('s') => out.push(' '),
            Some('\\') => out.push('\\'),
            Some('r') => out.push('\r'),
            Some('n') => out.push('\n'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

fn tags(raw: &str) -> HashMap<&str, String> {
    raw.split(';')
        .filter_map(|kv| {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            (!k.is_empty()).then(|| (k, unescape(v)))
        })
        .collect()
}

/// One line, without its trailing CRLF.
pub fn parse(line: &str) -> Line {
    let mut rest = line.trim_end_matches(['\r', '\n']);
    let mut tag_map = HashMap::new();
    if let Some(after) = rest.strip_prefix('@') {
        let Some((raw_tags, tail)) = after.split_once(' ') else {
            return Line::Other;
        };
        tag_map = tags(raw_tags);
        rest = tail;
    }
    let mut prefix = "";
    if let Some(after) = rest.strip_prefix(':') {
        let Some((p, tail)) = after.split_once(' ') else {
            return Line::Other;
        };
        prefix = p;
        rest = tail;
    }
    let (command, params) = rest.split_once(' ').unwrap_or((rest, ""));
    let (middle, trailing) = match params.split_once(" :") {
        Some((m, t)) => (m, Some(t)),
        None => match params.strip_prefix(':') {
            Some(t) => ("", Some(t)),
            None => (params, None),
        },
    };
    let tag = |k: &str| tag_map.get(k).cloned().unwrap_or_default();
    let opt = |k: &str| tag_map.get(k).filter(|v| !v.is_empty()).cloned();
    match command {
        "PING" => Line::Ping(trailing.unwrap_or(middle).to_string()),
        "RECONNECT" => Line::Reconnect,
        "NOTICE" => Line::Notice(
            tag("msg-id"),
            middle.trim_start_matches('#').to_string(),
            trailing.unwrap_or("").to_string(),
        ),
        "PRIVMSG" => {
            let channel = middle.trim().trim_start_matches('#').to_lowercase();
            let login = prefix.split('!').next().unwrap_or("").to_lowercase();
            Line::Privmsg(Box::new(Privmsg {
                id: tag("id"),
                room_id: tag("room-id"),
                channel_login: channel,
                sender_id: tag("user-id"),
                sender_login: login,
                sender_display: tag("display-name"),
                color: tag("color"),
                sent_ms: tag("tmi-sent-ts").parse().unwrap_or(0),
                text: trailing.unwrap_or("").to_string(),
                reply_parent_msg_id: opt("reply-parent-msg-id"),
                reply_parent_user_id: opt("reply-parent-user-id"),
                reply_parent_login: opt("reply-parent-user-login"),
                reply_parent_display: opt("reply-parent-display-name"),
            }))
        }
        _ => Line::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Shaped like real lines from irc-ws.chat.twitch.tv (tags trimmed).
    const PLAIN: &str = "@badge-info=;badges=;color=#1E90FF;display-name=Alice;emotes=;first-msg=0;flags=;id=885196de-cb67-427a-baa8-82f9b0fcd05f;mod=0;returning-chatter=0;room-id=22484632;subscriber=0;tmi-sent-ts=1700000000123;turbo=0;user-id=12345;user-type= :alice!alice@alice.tmi.twitch.tv PRIVMSG #forsen :hey @Brandon gg\r\n";
    const REPLY: &str = "@color=;display-name=Bob;id=b1;reply-parent-display-name=Brandon;reply-parent-msg-body=nice\\sone;reply-parent-msg-id=p1;reply-parent-user-id=100;reply-parent-user-login=brandon;reply-thread-parent-msg-id=p1;reply-thread-parent-user-login=brandon;room-id=22484632;tmi-sent-ts=1700000000999;user-id=6789 :bob!bob@bob.tmi.twitch.tv PRIVMSG #forsen :@Brandon agreed";
    const ACTION: &str = "@display-name=Carl;id=c1;room-id=1;tmi-sent-ts=5;user-id=7 :carl!carl@carl.tmi.twitch.tv PRIVMSG #xqc :\u{1}ACTION waves at brandon\u{1}";

    #[test]
    fn reads_a_privmsg_with_its_tags() {
        let Line::Privmsg(m) = parse(PLAIN) else { panic!("not a PRIVMSG") };
        assert_eq!(m.id, "885196de-cb67-427a-baa8-82f9b0fcd05f");
        assert_eq!(m.room_id, "22484632");
        assert_eq!(m.channel_login, "forsen");
        assert_eq!(m.sender_id, "12345");
        assert_eq!(m.sender_login, "alice");
        assert_eq!(m.sender_display, "Alice");
        assert_eq!(m.color, "#1E90FF");
        assert_eq!(m.sent_ms, 1_700_000_000_123);
        assert_eq!(m.text, "hey @Brandon gg");
        assert_eq!(m.reply_parent_user_id, None);
    }

    #[test]
    fn reads_reply_tags_and_unescapes_them() {
        let Line::Privmsg(m) = parse(REPLY) else { panic!("not a PRIVMSG") };
        assert_eq!(m.reply_parent_msg_id.as_deref(), Some("p1"));
        assert_eq!(m.reply_parent_user_id.as_deref(), Some("100"));
        assert_eq!(m.reply_parent_login.as_deref(), Some("brandon"));
        assert_eq!(m.reply_parent_display.as_deref(), Some("Brandon"));
        assert_eq!(m.text, "@Brandon agreed");
        assert_eq!(unescape("nice\\sone\\:\\\\"), "nice one;\\");
    }

    #[test]
    fn keeps_an_action_wrapped_for_the_rule() {
        let Line::Privmsg(m) = parse(ACTION) else { panic!("not a PRIVMSG") };
        assert_eq!(m.text, "\u{1}ACTION waves at brandon\u{1}");
    }

    #[test]
    fn reads_ping_reconnect_and_notice() {
        assert_eq!(parse("PING :tmi.twitch.tv"), Line::Ping("tmi.twitch.tv".into()));
        assert_eq!(parse(":tmi.twitch.tv RECONNECT"), Line::Reconnect);
        assert_eq!(
            parse("@msg-id=msg_channel_suspended :tmi.twitch.tv NOTICE #gone :This channel does not exist or has been suspended."),
            Line::Notice("msg_channel_suspended".into(), "gone".into(), "This channel does not exist or has been suspended.".into())
        );
        assert_eq!(parse(":tmi.twitch.tv 001 justinfan123 :Welcome, GLHF!"), Line::Other);
        assert_eq!(parse(""), Line::Other);
    }
}
