//! The text the mention rule reads, as the chat row shows it.
//!
//! Twitch sends a `/me` wrapped as `\x01ACTION text\x01`, and a reply starts
//! with an `@parent` that Twitch's composer inserts (the parent's DISPLAY name,
//! which can be localized). The row shows the reply context instead, so the
//! leading name comes off. Without that, replying to someone whose name is
//! yours would count as naming you.

/// The text inside an ACTION wrapper, and whether there was one.
pub fn unwrap_action(payload: &str) -> (&str, bool) {
    // Minimum valid: "\x01ACTION X\x01" = 10 bytes (8 for the header, 1 of
    // text, 1 closing).
    if payload.len() >= 10 && payload.starts_with("\x01ACTION ") && payload.ends_with('\x01') {
        (&payload[8..payload.len() - 1], true)
    } else {
        (payload, false)
    }
}

/// Byte index in `content` right after a leading "@<name>" mention plus its
/// following whitespace run, or 0 when no alternative matches. The boundary
/// is REQUIRED (whitespace or end of message): with multiple alternatives, a
/// short name that prefixes a longer one must never partially strip
/// ("@foobarbaz" with login "foobar"). Case-folded per char; alternatives
/// are tried in order (login before display name).
pub fn reply_mention_end(content: &str, alts: &[&str]) -> usize {
    let Some(rest) = content.strip_prefix('@') else {
        return 0;
    };
    for alt in alts {
        if alt.is_empty() {
            continue;
        }
        let mut rest_chars = rest.char_indices();
        let mut alt_chars = alt.chars();
        let matched_end = loop {
            match alt_chars.next() {
                // Name fully matched: the end is the next char's byte index
                // (offset by the leading '@'), or end of message.
                None => {
                    break Some(
                        rest_chars
                            .next()
                            .map(|(i, _)| 1 + i)
                            .unwrap_or(content.len()),
                    )
                }
                Some(ac) => match rest_chars.next() {
                    Some((_, rc)) if rc.to_lowercase().eq(ac.to_lowercase()) => {}
                    _ => break None,
                },
            }
        };
        let Some(end) = matched_end else {
            continue;
        };
        let tail = &content[end..];
        if tail.is_empty() {
            return content.len();
        }
        if tail.starts_with(char::is_whitespace) {
            // Consume the whole whitespace run (the old pattern's greedy \s+).
            return end + (tail.len() - tail.trim_start().len());
        }
        // Boundary violated - the name only prefixes a longer word; try the
        // next alternative.
    }
    0
}

/// A reply's text without its leading `@parent` (login tried before display
/// name), and how many codepoints came off the front. Emote positions are
/// codepoint indices, so the app shifts them by that count. Whitespace is
/// trimmed on a reply whether or not a name matched; a message that is not a
/// reply (no parent names) comes back as it is.
pub fn strip_reply_parent(content: &str, parent_login: Option<&str>, parent_display: Option<&str>) -> (String, usize) {
    let alts: Vec<&str> = [parent_login, parent_display]
        .into_iter()
        .flatten()
        .filter(|s| !s.is_empty())
        .collect();
    if alts.is_empty() {
        return (content.to_string(), 0);
    }
    let mention_end = reply_mention_end(content, &alts);
    let rest_no_lead = content[mention_end..].trim_start();
    let front_bytes = content.len() - rest_no_lead.len();
    let front_codepoints = content[..front_bytes].chars().count();
    (rest_no_lead.trim_end().to_string(), front_codepoints)
}

/// The text the rule reads from a PRIVMSG's trailing parameter: trailing
/// whitespace off, a `/me` unwrapped, a reply's `@parent` taken off.
pub fn normalize(payload: &str, reply_parent_display: Option<&str>, reply_parent_login: Option<&str>) -> String {
    let (text, _) = unwrap_action(payload.trim_end());
    strip_reply_parent(text, reply_parent_login, reply_parent_display).0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reply_mention_strip_boundaries() {
        // Exact name + space: mention and the whole whitespace run consumed.
        assert_eq!(reply_mention_end("@foo  hi", &["foo"]), 6);
        // Name at end of message: everything consumed.
        assert_eq!(reply_mention_end("@foo", &["foo"]), 4);
        // Boundary REQUIRED: a name prefixing a longer word must not strip.
        assert_eq!(reply_mention_end("@foobarbaz hi", &["foobar"]), 0);
        // Case-insensitive match.
        assert_eq!(reply_mention_end("@FoO hi", &["foo"]), 5);
        // Second alternative (display name) matches when login does not.
        assert_eq!(reply_mention_end("@ふー hi", &["foo", "ふー"]), 8);
        // No leading @: untouched.
        assert_eq!(reply_mention_end("foo hi", &["foo"]), 0);
    }

    #[test]
    fn an_action_is_unwrapped_and_anything_else_is_left_alone() {
        assert_eq!(unwrap_action("\u{1}ACTION waves\u{1}"), ("waves", true));
        assert_eq!(unwrap_action("\u{1}ACTION \u{1}"), ("\u{1}ACTION \u{1}", false));
        assert_eq!(unwrap_action("ACTION waves"), ("ACTION waves", false));
    }

    #[test]
    fn a_reply_loses_its_parent_and_counts_the_codepoints() {
        assert_eq!(strip_reply_parent("@ふー  hi there ", Some("foo"), Some("ふー")), ("hi there".to_string(), 5));
        assert_eq!(strip_reply_parent("  hi", Some("foo"), None), ("hi".to_string(), 2));
        assert_eq!(strip_reply_parent("  hi ", None, None), ("  hi ".to_string(), 0));
    }

    #[test]
    fn normalize_runs_both_steps_in_order() {
        assert_eq!(normalize("\u{1}ACTION @Foo hi\u{1}", Some("Foo"), Some("foo")), "hi");
        assert_eq!(normalize("hello @foo  ", None, None), "hello @foo");
    }
}
