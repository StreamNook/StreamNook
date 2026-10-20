//! Text that reaches the page as plain words with no emote positions (a pinned
//! message, a prediction outcome, the profile card's recent messages, a mod
//! room message) as segments, cut by the same parser and channel table as a
//! chat row. One word then shows the same emote on every surface.

use std::collections::HashMap;
use std::time::Duration;

use crate::models::chat_layout::MessageSegment;
use crate::services::emote_service::Emote;
use crate::services::irc_service::{self, IrcService};

/// How long a call waits for a channel table that has not landed yet. The page
/// asks once its own copy of the channel's emotes arrives, which is usually
/// after the table.
const TABLE_WAIT: Duration = Duration::from_secs(5);

/// Login the made-up line is sent from. It never reaches chat; an empty
/// `user-id` keeps any viewer's personal 7TV emotes out of the parse.
const NICK: &str = "streamnook";

/// Segments for each of `texts` as said in Twitch `channel`, in order. Twitch
/// emotes match by name (the text carries no positions), the channel's 7TV,
/// BTTV and FFZ emotes as in a chat row, and no cheermotes (nobody cheered).
/// A channel with no table yet gives the texts back as plain text.
#[tauri::command]
pub async fn parse_text_segments(channel: String, texts: Vec<String>) -> Vec<Vec<MessageSegment>> {
    let Some(key) = channel_key(&channel) else {
        return texts.iter().map(|t| plain(t)).collect();
    };
    irc_service::wait_for_channel_emotes(&key, TABLE_WAIT).await;
    let tags = irc_service::with_channel_emotes(&key, |set| {
        texts.iter().map(|t| emotes_tag(t, &set.twitch)).collect::<Vec<_>>()
    })
    .await
    .unwrap_or_else(|| vec![String::new(); texts.len()]);
    texts.iter().zip(tags).map(|(text, tag)| segments_for(&key, text, &tag)).collect()
}

/// The channel as the parse table keys it: a bare lowercase login. Anything
/// else could not sit in an IRC line's channel slot.
fn channel_key(channel: &str) -> Option<String> {
    let key = channel.trim().trim_start_matches('#').to_lowercase();
    (!key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')).then_some(key)
}

fn plain(text: &str) -> Vec<MessageSegment> {
    if text.is_empty() {
        Vec::new()
    } else {
        vec![MessageSegment::Text { content: text.to_string() }]
    }
}

/// `text` parsed as a PRIVMSG to `key` carrying `tag` as its `emotes` tag. The
/// line parser drops a message's trailing whitespace, so that goes back on as
/// text and the segments cover the whole string.
fn segments_for(key: &str, text: &str, tag: &str) -> Vec<MessageSegment> {
    let body = text.trim_end();
    if body.is_empty() || body.contains(['\r', '\n', '\u{1}']) {
        return plain(text);
    }
    let line = format!("@emotes={tag};user-id= :{NICK}!{NICK}@{NICK}.tmi.twitch.tv PRIVMSG #{key} :{body}");
    let Some(msg) = IrcService::parse_live_line(&line) else {
        return plain(text);
    };
    let mut segments = msg.segments;
    let tail = &text[body.len()..];
    if !tail.is_empty() {
        segments.push(MessageSegment::Text { content: tail.to_string() });
    }
    segments
}

/// The `emotes` tag Twitch would attach to `text`: each word naming one of the
/// channel's Twitch emotes, by inclusive codepoint range, grouped by emote id
/// in first-seen order. Words split on single spaces, as the parser splits.
fn emotes_tag(text: &str, twitch: &[Emote]) -> String {
    let by_name: HashMap<&str, &str> = twitch.iter().map(|e| (e.name.as_str(), e.id.as_str())).collect();
    let mut order: Vec<&str> = Vec::new();
    let mut ranges: HashMap<&str, Vec<String>> = HashMap::new();
    let mut pos = 0usize;
    for word in text.trim_end().split(' ') {
        let len = word.chars().count();
        if let Some(id) = by_name.get(word).copied().filter(|_| len > 0) {
            if !ranges.contains_key(id) {
                order.push(id);
            }
            ranges.entry(id).or_default().push(format!("{}-{}", pos, pos + len - 1));
        }
        pos += len + 1;
    }
    order.iter().map(|id| format!("{}:{}", id, ranges[id].join(","))).collect::<Vec<_>>().join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn twitch(id: &str, name: &str) -> Emote {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "name": name,
            "url": format!("https://static-cdn.jtvnw.net/emoticons/v2/{id}/default/dark/2.0"),
            "provider": "twitch",
        }))
        .unwrap()
    }

    #[test]
    fn tag_marks_each_native_word_by_codepoint() {
        let set = [twitch("25", "Kappa"), twitch("1902", "Keepo")];
        assert_eq!(emotes_tag("é Kappa Keepo Kappa", &set), "25:2-6,14-18/1902:8-12");
        assert_eq!(emotes_tag("Kappa  hi", &set), "25:0-4");
        assert_eq!(emotes_tag("nothing here", &set), "");
    }

    #[test]
    fn channel_key_takes_only_a_login() {
        assert_eq!(channel_key(" #XQC ").as_deref(), Some("xqc"));
        assert_eq!(channel_key("a b"), None);
        assert_eq!(channel_key("kick:slug"), None);
        assert_eq!(channel_key(""), None);
    }

    fn joined(segments: &[MessageSegment]) -> String {
        segments
            .iter()
            .map(|s| match s {
                MessageSegment::Text { content } | MessageSegment::Emote { content, .. } => content.clone(),
                _ => String::new(),
            })
            .collect()
    }

    #[test]
    fn trailing_space_survives_the_line_parser() {
        assert_eq!(joined(&segments_for("textsegs_none", "hello ", "")), "hello ");
    }

    #[test]
    fn words_resolve_through_the_channel_table_and_the_native_tag() {
        let mut set = crate::services::emote_service::EmoteSet::new();
        let mut seven: Emote = twitch("7tv-cat", "catJAM");
        seven.provider = crate::services::emote_service::EmoteProvider::SevenTV;
        set.seven_tv = vec![seven];
        IrcService::install_parse_lookup_for_test("textsegs_wired", &set);
        let text = "Kappa hi catJAM";
        let segs = segments_for("textsegs_wired", text, "25:0-4");
        let ids: Vec<_> = segs
            .iter()
            .filter_map(|s| match s {
                MessageSegment::Emote { emote_id, .. } => emote_id.clone(),
                _ => None,
            })
            .collect();
        assert_eq!(ids, ["25", "7tv-cat"]);
        assert_eq!(joined(&segs), text);
    }
}
