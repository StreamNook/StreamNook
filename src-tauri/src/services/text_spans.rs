//! Mentions and links cut out of a chat row's text as their own segments, so
//! every surface (desktop rows, the phones' row view) draws and resolves them
//! the same way and no page runs its own pattern over the text.
//!
//! A mention is an `@` and 1 to 25 word characters followed by the end,
//! whitespace or closing punctuation, on every platform. Twitch also has a few
//! display names with spaces ("Call of Duty", login `callofduty`): on a Twitch
//! row `@Call of Duty` is one mention once that name has been seen in chat
//! (`chatter_name` keeps them). The Twitch parser cuts text at spaces, so such
//! a mention spans several text segments and is joined here.
//!
//! Links: the Twitch parser finds them word by word (`link_detect`). Kick,
//! YouTube and TikTok rows arrive as plain text, so the same rule runs over
//! their text here, and a link reads alike whichever platform it came from.
//! A word that is a link keeps its `@` ("youtube.com/@name").

use crate::models::chat_layout::{ChatMessage, MessageSegment};
use crate::services::link_detect;

const MAX_NAME: usize = 25;
const CLOSERS: &str = ".,!?:;'\")]}>-";
/// Text segments a spaced name may run across: its words and the spaces between.
const MAX_JOIN: usize = 16;

/// One piece of a run of text.
#[derive(Debug, Clone, PartialEq)]
pub enum Piece {
    Text(String),
    Mention { text: String, login: String },
}

/// Whether a mention may end before `next` (`None` is the end of the text).
pub fn ends_mention(next: Option<char>) -> bool {
    next.is_none_or(|c| c.is_whitespace() || CLOSERS.contains(c))
}

fn is_link_word(word: &str) -> bool {
    link_detect::split_link(word.trim_end()).is_some()
}

/// Plain `@login` mentions in one run of text.
pub fn split_text(text: &str) -> Vec<Piece> {
    let mut out = Vec::new();
    let mut plain = String::new();
    for word in text.split_inclusive(char::is_whitespace) {
        if word.contains('@') && !is_link_word(word) {
            split_word(word, &mut plain, &mut out);
        } else {
            plain.push_str(word);
        }
    }
    if !plain.is_empty() {
        out.push(Piece::Text(plain));
    }
    out
}

fn split_word(word: &str, plain: &mut String, out: &mut Vec<Piece>) {
    let chars: Vec<char> = word.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '@' {
            let mut j = i + 1;
            while j < chars.len() && j - i <= MAX_NAME && (chars[j].is_ascii_alphanumeric() || chars[j] == '_') {
                j += 1;
            }
            let len = j - i - 1;
            if (1..=MAX_NAME).contains(&len) && ends_mention(chars.get(j).copied()) {
                if !plain.is_empty() {
                    out.push(Piece::Text(std::mem::take(plain)));
                }
                let name: String = chars[i + 1..j].iter().collect();
                out.push(Piece::Mention { text: format!("@{name}"), login: name.to_lowercase() });
                i = j;
                continue;
            }
        }
        plain.push(chars[i]);
        i += 1;
    }
}

/// A link segment for one link's visible text, as the Twitch parser builds it.
pub fn link_segment(link: &str) -> MessageSegment {
    let url = link_detect::link_url(link);
    let card = crate::commands::link_preview::expected_card(&url).map(String::from);
    MessageSegment::Link {
        content: link.to_string(),
        label: crate::services::chat_row_view::link_label(link),
        url,
        card,
    }
}

/// Cut the mentions (and, off Twitch, the links) out of a row's body and its
/// quoted reply parent. Runs where every row's names are settled
/// (`chatter_name::stamp`). Mention and link segments are never re-read, so
/// running it twice changes nothing.
pub fn apply_row(msg: &mut ChatMessage) {
    let twitch = msg.provider.is_empty() || msg.provider == "twitch";
    if !twitch {
        apply_links(&mut msg.segments);
    }
    apply(&mut msg.segments, twitch);
    if let Some(parent) = msg.metadata.reply_info.as_mut().and_then(|r| r.parent_segments.as_mut()) {
        if !twitch {
            apply_links(parent);
        }
        apply(parent, twitch);
    }
}

/// A pinned message's text as text and link segments, found as a chat row's
/// are. Mentions stay text: the pin card draws its body as plain words.
pub fn pin_segments(text: &str) -> Vec<MessageSegment> {
    let mut segments = vec![MessageSegment::Text { content: text.to_string() }];
    apply_links(&mut segments);
    segments
}

/// Link segments in place of the links inside text segments.
pub fn apply_links(segments: &mut Vec<MessageSegment>) {
    let may_link = |s: &MessageSegment| matches!(s, MessageSegment::Text { content } if content.contains('.'));
    if !segments.iter().any(may_link) {
        return;
    }
    let input = std::mem::take(segments);
    let mut out = Vec::with_capacity(input.len() + 2);
    for seg in input {
        match seg {
            MessageSegment::Text { content } if content.contains('.') => split_links(&content, &mut out),
            other => out.push(other),
        }
    }
    *segments = out;
}

fn split_links(text: &str, out: &mut Vec<MessageSegment>) {
    let mut plain = String::new();
    for word in text.split_inclusive(char::is_whitespace) {
        let bare = word.trim_end();
        let gap = &word[bare.len()..];
        match link_detect::split_link(bare) {
            Some((link, trailing)) => {
                if !plain.is_empty() {
                    out.push(MessageSegment::Text { content: std::mem::take(&mut plain) });
                }
                out.push(link_segment(link));
                plain.push_str(trailing);
                plain.push_str(gap);
            }
            None => plain.push_str(word),
        }
    }
    if !plain.is_empty() {
        out.push(MessageSegment::Text { content: plain });
    }
}

/// Mention segments in place of the mentions inside text segments; `spaced`
/// also joins the spaced Twitch names seen so far.
pub fn apply(segments: &mut Vec<MessageSegment>, spaced: bool) {
    if !segments.iter().any(|s| matches!(s, MessageSegment::Text { content } if content.contains('@'))) {
        return;
    }
    let input = std::mem::take(segments);
    let mut out = Vec::with_capacity(input.len() + 2);
    // What is left of the segment a spaced name ended inside ("!" after "Duty!").
    let mut carry: Option<String> = None;
    let mut i = 0;
    loop {
        let text = if let Some(t) = carry.take() {
            t
        } else if i < input.len() {
            i += 1;
            match &input[i - 1] {
                MessageSegment::Text { content } => content.clone(),
                other => {
                    out.push(other.clone());
                    continue;
                }
            }
        } else {
            break;
        };
        if !text.contains('@') {
            out.push(MessageSegment::Text { content: text });
            continue;
        }
        if spaced {
            if let Some(hit) = spaced_mention(&text, &input[i..]) {
                push_pieces(&mut out, &text[..hit.at]);
                out.push(MessageSegment::Mention { content: hit.text, login: hit.login });
                i += hit.used;
                if !hit.rest.is_empty() {
                    carry = Some(hit.rest);
                }
                continue;
            }
        }
        push_pieces(&mut out, &text);
    }
    *segments = out;
}

fn push_pieces(out: &mut Vec<MessageSegment>, text: &str) {
    for piece in split_text(text) {
        out.push(match piece {
            Piece::Text(content) => MessageSegment::Text { content },
            Piece::Mention { text, login } => MessageSegment::Mention { content: text, login },
        });
    }
}

struct Hit {
    /// Byte offset of the `@` in its segment.
    at: usize,
    text: String,
    login: String,
    /// Following segments the name used up.
    used: usize,
    rest: String,
}

/// A known spaced Twitch name typed after the `@` that starts the last word of
/// `text`, running on into the text segments after it.
fn spaced_mention(text: &str, after: &[MessageSegment]) -> Option<Hit> {
    let at = text.rfind('@')?;
    let word = &text[at + 1..];
    if word.is_empty() || word.contains(char::is_whitespace) || is_link_word(&text[at..]) {
        return None;
    }
    let mut joined = word.to_string();
    let mut ends = Vec::new();
    for seg in after.iter().take(MAX_JOIN) {
        let MessageSegment::Text { content } = seg else { break };
        joined.push_str(content);
        ends.push(joined.len());
    }
    if ends.is_empty() {
        return None;
    }
    let (len, login) = crate::services::chatter_name::spaced_name_at(&joined)?;
    // A name that ends inside the first word is a plain mention.
    if len <= word.len() {
        return None;
    }
    let k = ends.iter().position(|&end| end >= len)?;
    let start = if k == 0 { word.len() } else { ends[k - 1] };
    let MessageSegment::Text { content } = &after[k] else { return None };
    Some(Hit {
        at,
        text: format!("@{}", &joined[..len]),
        login,
        used: k + 1,
        rest: content[len - start..].to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::chatter_name::note_spaced;

    fn t(s: &str) -> MessageSegment {
        MessageSegment::Text { content: s.to_string() }
    }

    /// The parser's shape: one segment per word, one per space between.
    fn words(s: &str) -> Vec<MessageSegment> {
        let parts: Vec<&str> = s.split(' ').collect();
        let mut out = Vec::new();
        for (i, w) in parts.iter().enumerate() {
            out.push(t(w));
            if i + 1 < parts.len() {
                out.push(t(" "));
            }
        }
        out
    }

    fn kinds(segs: &[MessageSegment]) -> Vec<String> {
        segs.iter()
            .map(|s| match s {
                MessageSegment::Text { content } => format!("t:{content}"),
                MessageSegment::Mention { content, login } => format!("m:{content}>{login}"),
                MessageSegment::Link { content, url, .. } => format!("l:{content}>{url}"),
                _ => "?".to_string(),
            })
            .collect()
    }

    #[test]
    fn plain_mentions_keep_their_boundaries() {
        let mut segs = vec![t("hi @Nook_Fan, and a@b c @toolongnameeeeeeeeeeeeeeeeeeee")];
        apply(&mut segs, false);
        assert_eq!(
            kinds(&segs),
            vec!["t:hi ", "m:@Nook_Fan>nook_fan", "t:, and a", "m:@b>b", "t: c @toolongnameeeeeeeeeeeeeeeeeeee"]
        );
    }

    #[test]
    fn a_spaced_name_seen_in_chat_is_one_mention() {
        note_spaced("spacedtestone", "Spaced Test One");
        let mut segs = words("hey @Spaced Test One! gg");
        apply(&mut segs, true);
        assert_eq!(
            kinds(&segs),
            vec!["t:hey", "t: ", "m:@Spaced Test One>spacedtestone", "t:!", "t: ", "t:gg"]
        );
    }

    #[test]
    fn a_spaced_name_matches_whatever_the_case() {
        note_spaced("spacedtesttwo", "Spaced Test Two");
        let mut segs = words("@spaced test two");
        apply(&mut segs, true);
        assert_eq!(kinds(&segs), vec!["m:@spaced test two>spacedtesttwo"]);
    }

    #[test]
    fn the_longest_known_name_wins() {
        note_spaced("longa", "Long Test");
        note_spaced("longb", "Long Test Name");
        let mut segs = words("@Long Test Name hi");
        apply(&mut segs, true);
        assert_eq!(kinds(&segs), vec!["m:@Long Test Name>longb", "t: ", "t:hi"]);
    }

    #[test]
    fn a_spaced_name_needs_its_boundary() {
        note_spaced("boundtest", "Bound Test");
        let mut segs = words("@Bound Testy");
        apply(&mut segs, true);
        assert_eq!(kinds(&segs), vec!["m:@Bound>bound", "t: ", "t:Testy"]);
    }

    #[test]
    fn an_unknown_spaced_name_stays_a_plain_mention() {
        let mut segs = words("@Never Seen Here");
        apply(&mut segs, true);
        assert_eq!(kinds(&segs)[0], "m:@Never>never");
    }

    #[test]
    fn off_twitch_names_are_not_joined() {
        note_spaced("offtwitch", "Off Twitch");
        let mut segs = words("@Off Twitch");
        apply(&mut segs, false);
        assert_eq!(kinds(&segs)[0], "m:@Off>off");
    }

    #[test]
    fn an_emote_between_words_stops_the_join() {
        note_spaced("emotestop", "Emote Stop");
        let mut segs = vec![
            t("@Emote"),
            t(" "),
            MessageSegment::Emote {
                content: "Stop".into(),
                emote_id: None,
                emote_url: "u".into(),
                is_zero_width: None,
                modifier_flags: None,
                is_personal: None,
            },
        ];
        apply(&mut segs, true);
        assert_eq!(kinds(&segs)[..2], ["m:@Emote>emote".to_string(), "t: ".to_string()]);
    }

    #[test]
    fn running_twice_changes_nothing() {
        note_spaced("twicetest", "Twice Test");
        let mut segs = words("yo @Twice Test and @bob");
        apply(&mut segs, true);
        let once = kinds(&segs);
        apply(&mut segs, true);
        assert_eq!(kinds(&segs), once);
    }

    #[test]
    fn a_link_keeps_its_at() {
        let mut segs = vec![t("see https://youtube.com/@MrBeast and www.x.com/@a ok")];
        apply(&mut segs, false);
        assert_eq!(kinds(&segs), vec!["t:see https://youtube.com/@MrBeast and www.x.com/@a ok"]);
    }

    #[test]
    fn links_are_found_in_plain_platform_text() {
        let mut segs = vec![t("watch youtube.com/@MrBeast, then test.fr. @bob 3.5 no")];
        apply_links(&mut segs);
        apply(&mut segs, false);
        assert_eq!(
            kinds(&segs),
            vec![
                "t:watch ",
                "l:youtube.com/@MrBeast>https://youtube.com/@MrBeast",
                "t:, then ",
                "l:test.fr>https://test.fr",
                "t:. ",
                "m:@bob>bob",
                "t: 3.5 no",
            ]
        );
    }

    #[test]
    fn a_link_segment_carries_its_short_label() {
        let MessageSegment::Link { label, url, .. } = link_segment("https://www.example.com/") else { panic!() };
        assert_eq!(label, "example.com");
        assert_eq!(url, "https://www.example.com/");
    }
}
