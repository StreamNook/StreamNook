//! Unicode bidi controls in chatters' text, closed at the edge of each piece.
//!
//! A display name or message can carry an embedding, override or isolate
//! (U+202A..U+202E, U+2066..U+2069) that is never closed. A renderer that sets
//! a row as one paragraph (the iPhone row, an OS notification) carries that
//! control past the piece it was typed in, so a name ending in U+202E reverses
//! the message after it. Each piece is balanced here, once, for every surface:
//! a terminator with nothing to close is dropped and every control left open is
//! closed at the end of its own piece. Right-to-left letters and the marks
//! (U+200E, U+200F) are left alone, so Arabic and Hebrew still read correctly.

use crate::models::chat_layout::{ChatMessage, MessageSegment};

const LRE: char = '\u{202A}';
const RLE: char = '\u{202B}';
const PDF: char = '\u{202C}';
const LRO: char = '\u{202D}';
const RLO: char = '\u{202E}';
const LRI: char = '\u{2066}';
const RLI: char = '\u{2067}';
const FSI: char = '\u{2068}';
const PDI: char = '\u{2069}';

fn is_control(c: char) -> bool {
    matches!(c, LRE..=RLO | LRI..=PDI)
}

#[derive(Clone, Copy, PartialEq)]
enum Open {
    Embedding,
    Isolate,
}

/// `text` with every bidi embedding, override and isolate closed inside it,
/// or None when it is already balanced (the common case, no allocation).
pub fn balanced(text: &str) -> Option<String> {
    if !text.chars().any(is_control) {
        return None;
    }
    let mut out = String::with_capacity(text.len() + 6);
    let mut stack: Vec<Open> = Vec::new();
    for c in text.chars() {
        match c {
            LRE | RLE | LRO | RLO => {
                stack.push(Open::Embedding);
                out.push(c);
            }
            LRI | RLI | FSI => {
                stack.push(Open::Isolate);
                out.push(c);
            }
            // A PDF closes the innermost embedding, and only one opened inside
            // the innermost isolate; any other PDF does nothing, so it goes.
            PDF => {
                if stack.last() == Some(&Open::Embedding) {
                    stack.pop();
                    out.push(c);
                }
            }
            // A PDI closes the innermost isolate and every embedding opened in it.
            PDI => {
                if let Some(at) = stack.iter().rposition(|o| *o == Open::Isolate) {
                    stack.truncate(at);
                    out.push(c);
                }
            }
            _ => out.push(c),
        }
    }
    while let Some(open) = stack.pop() {
        out.push(if open == Open::Isolate { PDI } else { PDF });
    }
    (out != text).then_some(out)
}

fn balance(text: &mut String) {
    if let Some(fixed) = balanced(text) {
        *text = fixed;
    }
}

fn balance_segments(segments: &mut [MessageSegment]) {
    for seg in segments {
        match seg {
            MessageSegment::Text { content, .. }
            | MessageSegment::Mention { content, .. }
            | MessageSegment::Gif { content, .. } => balance(content),
            MessageSegment::Link { content, label, .. } => {
                balance(content);
                balance(label);
            }
            _ => {}
        }
    }
}

/// Balances every piece of chatter text a row draws: the name, the body and
/// its segments, the quoted parent of a reply, the platform's event sentence
/// and the IRC tags the page reads names and text from.
pub fn balance_row(msg: &mut ChatMessage) {
    balance(&mut msg.display_name);
    balance(&mut msg.content);
    balance_segments(&mut msg.segments);
    let meta = &mut msg.metadata;
    balance(&mut meta.shown_name);
    if let Some(text) = meta.system_message.as_mut() {
        balance(text);
    }
    if let Some(reply) = meta.reply_info.as_mut() {
        balance(&mut reply.parent_display_name);
        balance(&mut reply.parent_shown_name);
        balance(&mut reply.parent_msg_body);
        if let Some(segments) = reply.parent_segments.as_mut() {
            balance_segments(segments);
        }
    }
    for value in msg.tags.values_mut() {
        balance(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_and_right_to_left_text_is_untouched() {
        assert_eq!(balanced("kingjabas"), None);
        assert_eq!(balanced("مرحبا بالجميع 123"), None);
        assert_eq!(balanced("שלום world"), None);
        // Marks are not controls that open anything.
        assert_eq!(balanced("\u{200F}(Inherited) 8\u{200E}"), None);
    }

    #[test]
    fn an_open_override_is_closed_at_the_end_of_its_piece() {
        assert_eq!(
            balanced("\u{202E}8 )detirehnI(").as_deref(),
            Some("\u{202E}8 )detirehnI(\u{202C}")
        );
        assert_eq!(
            balanced("a\u{202B}b\u{202D}c").as_deref(),
            Some("a\u{202B}b\u{202D}c\u{202C}\u{202C}")
        );
        assert_eq!(
            balanced("\u{2067}its 5pm").as_deref(),
            Some("\u{2067}its 5pm\u{2069}")
        );
        // Mixed: the isolate closes after the embedding opened inside it.
        assert_eq!(
            balanced("\u{2068}x\u{202E}y").as_deref(),
            Some("\u{2068}x\u{202E}y\u{202C}\u{2069}")
        );
    }

    #[test]
    fn closed_pairs_stay_and_stray_terminators_go() {
        assert_eq!(balanced("\u{202E}abc\u{202C} def"), None);
        assert_eq!(balanced("\u{2067}abc\u{2069}"), None);
        assert_eq!(balanced("abc\u{202C}\u{2069}").as_deref(), Some("abc"));
        // A PDF cannot close an embedding outside the isolate it sits in.
        assert_eq!(
            balanced("\u{202B}a\u{2066}b\u{202C}c\u{2069}d").as_deref(),
            Some("\u{202B}a\u{2066}bc\u{2069}d\u{202C}")
        );
        // A PDI closes the embeddings opened inside its isolate.
        assert_eq!(balanced("\u{2066}a\u{202E}b\u{2069}c"), None);
    }

    #[test]
    fn a_row_is_balanced_in_every_piece() {
        let mut msg: ChatMessage = serde_json::from_value(serde_json::json!({
            "id": "1", "user_id": "7", "username": "a", "display_name": "\u{202E}8 )detirehnI(", "color": null,
            "badges": [], "timestamp": "", "content": "why you in bed\u{202B}", "tags": {},
            "segments": [{ "type": "text", "content": "why you in bed\u{202B}" }]
        }))
        .unwrap();
        msg.tags
            .insert("display-name".into(), "\u{202E}8 )detirehnI(".into());
        msg.tags
            .insert("reply-parent-msg-body".into(), "\u{2067}hi".into());
        balance_row(&mut msg);
        assert_eq!(msg.display_name, "\u{202E}8 )detirehnI(\u{202C}");
        assert_eq!(msg.content, "why you in bed\u{202B}\u{202C}");
        assert!(
            matches!(&msg.segments[0], MessageSegment::Text { content, .. } if content == "why you in bed\u{202B}\u{202C}")
        );
        assert_eq!(msg.tags["display-name"], "\u{202E}8 )detirehnI(\u{202C}");
        assert_eq!(msg.tags["reply-parent-msg-body"], "\u{2067}hi\u{2069}");
    }
}
