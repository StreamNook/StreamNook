//! Twitch's text-face emotes: `:)`, `;)`, `<3`, `R)` and the rest of the
//! "smilies" sets (the robot faces, the purple set, the Turbo/Prime monkeys).
//!
//! Helix names them by the pattern Twitch's old chat matched them with
//! (`\:-?\)`, `\&lt\;3`, `[oO](_|\.)[oO]`), so no typed word ever equals the
//! name: a message the user sent drew `:)` as plain text, and the emote menu
//! and Tab completion listed the pattern instead of the face. Every Twitch
//! emote list is tidied here as it arrives (`tidy_names`) so the rest of the
//! app sees the face itself, and `word_matches` accepts the other forms the
//! pattern allowed (`:-)` for `:)`, `:p` for `:P`, `o.O` for `O_o`).
//!
//! Which set's art a face shows is the sender's choice: Twitch tags a received
//! message with the id from the set its sender picked, and Helix lists the
//! signed-in user's own set, so a sent message wears that one.

use once_cell::sync::Lazy;
use regex::Regex;

use crate::services::emote_service::{Emote, EmoteProvider};

/// Helix's pattern names for the faces every set shares, and the face each
/// one is shown as.
const KNOWN: &[(&str, &str)] = &[
    (r"\:-?\)", ":)"),
    (r"\:-?\(", ":("),
    (r"\:-?D", ":D"),
    (r"\&gt\;\(", ">("),
    (r"\:-?[z|Z|\|]", ":|"),
    (r"[oO](_|\.)[oO]", "O_o"),
    (r"B-?\)", "B)"),
    (r"\:-?(o|O)", ":O"),
    (r"\&lt\;3", "<3"),
    (r"\:-?[\\/]", ":/"),
    (r"\;-?\)", ";)"),
    (r"\:-?(p|P)", ":P"),
    (r"\;-?(p|P)", ";P"),
    (r"R-?\)", "R)"),
];

/// Each known face with the pattern, HTML entities decoded, matching the whole
/// word.
static PATTERNS: Lazy<Vec<(&'static str, Regex)>> = Lazy::new(|| {
    KNOWN
        .iter()
        .filter_map(|(pattern, face)| {
            let plain = pattern.replace(r"\&lt\;", "<").replace(r"\&gt\;", ">");
            Regex::new(&format!("^(?:{plain})$")).ok().map(|re| (*face, re))
        })
        .collect()
});

/// Whether a Helix emote name is one of these patterns rather than a name.
/// Ordinary emote names are letters and digits; a pattern always carries an
/// escape, an optional mark, a choice or a character class.
fn is_pattern(name: &str) -> bool {
    name.contains(['\\', '?', '|', '['])
}

/// The face a Helix smiley name stands for, or None for an ordinary name.
pub fn display_code(name: &str) -> Option<String> {
    if !is_pattern(name) {
        return None;
    }
    if let Some((_, face)) = KNOWN.iter().find(|(pattern, _)| *pattern == name) {
        return Some((*face).to_string());
    }
    let face = first_match(name);
    let face = face.replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&");
    (!face.is_empty()).then_some(face)
}

/// The simplest text a pattern in Twitch's small smiley grammar matches:
/// optional parts left out, the first choice of a group or class taken,
/// escapes read as the character itself.
fn first_match(pattern: &str) -> String {
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::new();
    // Byte length of the last unit written, so a following `?` can drop it.
    let mut last_len = 0usize;
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        let before = out.len();
        match c {
            '\\' => {
                if let Some(&next) = chars.get(i + 1) {
                    out.push(next);
                }
                i += 2;
            }
            '?' => {
                out.truncate(out.len() - last_len);
                last_len = 0;
                i += 1;
                continue;
            }
            '[' => {
                let mut j = i + 1;
                let mut members: Vec<char> = Vec::new();
                while j < chars.len() && chars[j] != ']' {
                    if chars[j] == '\\' {
                        j += 1;
                    }
                    if let Some(&ch) = chars.get(j) {
                        members.push(ch);
                    }
                    j += 1;
                }
                // `[\\/]` reads as `/` and `[z|Z|\|]` as `z`: a backslash or a
                // pipe is a face's mouth only when the class offers nothing else.
                let pick = members
                    .iter()
                    .find(|ch| !matches!(ch, '\\' | '|'))
                    .or(members.first());
                if let Some(&ch) = pick {
                    out.push(ch);
                }
                i = j + 1;
            }
            '(' => {
                let mut depth = 1usize;
                let mut j = i + 1;
                let mut first_end: Option<usize> = None;
                while j < chars.len() && depth > 0 {
                    match chars[j] {
                        '\\' => j += 1,
                        '(' => depth += 1,
                        ')' => depth -= 1,
                        '|' if depth == 1 && first_end.is_none() => first_end = Some(j),
                        _ => {}
                    }
                    j += 1;
                }
                let close = j.saturating_sub(1);
                let inner: String = chars[i + 1..first_end.unwrap_or(close).max(i + 1)].iter().collect();
                out.push_str(&first_match(&inner));
                i = j;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
        last_len = out.len() - before;
    }
    out
}

/// Rename every pattern-named Twitch emote in `twitch` to its face, in place.
/// Names that are already faces or ordinary names are left alone, so a list
/// tidied once (or read back from the disk cache) passes through unchanged.
pub fn tidy_names(twitch: &mut [Emote]) {
    for e in twitch.iter_mut() {
        if e.provider != EmoteProvider::Twitch {
            continue;
        }
        if let Some(face) = display_code(&e.name) {
            e.name = face;
        }
    }
}

/// Whether `word` is a typed form of the face `code`: the face itself, or any
/// other form Twitch's pattern for it accepts.
pub fn word_matches(code: &str, word: &str) -> bool {
    word == code
        || PATTERNS
            .iter()
            .any(|(face, re)| *face == code && re.is_match(word))
}

/// The smiley in `twitch` that `word` types, if any. Only Twitch's smilies are
/// considered, and only short words, so an ordinary word costs one length test.
pub fn smiley_for<'a>(word: &str, twitch: &'a [Emote]) -> Option<&'a Emote> {
    if word.is_empty() || word.chars().count() > 4 {
        return None;
    }
    twitch
        .iter()
        .filter(|e| e.emote_type.as_deref() == Some("smilies"))
        .find(|e| word_matches(&e.name, word))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn smiley(id: &str, name: &str) -> Emote {
        Emote {
            id: id.to_string(),
            name: name.to_string(),
            url: String::new(),
            provider: EmoteProvider::Twitch,
            is_zero_width: None,
            local_url: None,
            emote_type: Some("smilies".to_string()),
            owner_id: None,
            owner_name: None,
            original_name: None,
            owner_ref: None,
            width: None,
            height: None,
            modifier_flags: None,
            ffz_sub_only: None,
            animated: None,
        }
    }

    #[test]
    fn every_known_pattern_compiles() {
        assert_eq!(PATTERNS.len(), KNOWN.len());
    }

    #[test]
    fn helix_pattern_names_read_as_faces() {
        // The exact names Helix sent for the robot set (ids 1-14).
        for (name, face) in [
            (r"\:-?\)", ":)"),
            (r"\;-?\)", ";)"),
            (r"\&lt\;3", "<3"),
            (r"\&gt\;\(", ">("),
            (r"R-?\)", "R)"),
            (r"[oO](_|\.)[oO]", "O_o"),
            (r"\:-?[\\/]", ":/"),
            (r"\;-?(p|P)", ";P"),
        ] {
            assert_eq!(display_code(name).as_deref(), Some(face), "{name}");
        }
    }

    #[test]
    fn unknown_patterns_take_their_simplest_text() {
        // Faces only a set beyond the robot one has (the monkeys' extras).
        assert_eq!(display_code(r"\&lt\;\]").as_deref(), Some("<]"));
        assert_eq!(display_code(r"\#-?[\\/]").as_deref(), Some("#/"));
        assert_eq!(display_code(r"\:-?(s|S)").as_deref(), Some(":s"));
        assert_eq!(display_code(r"\:-?7").as_deref(), Some(":7"));
        assert_eq!(display_code(r"\:-?\&gt\;").as_deref(), Some(":>"));
    }

    #[test]
    fn ordinary_names_are_left_alone() {
        assert_eq!(display_code("Kappa"), None);
        assert_eq!(display_code(":)"), None);
        assert_eq!(display_code("<3"), None);
        let mut list = vec![smiley("1", r"\:-?\)"), smiley("25", "Kappa"), smiley("9", "<3")];
        tidy_names(&mut list);
        let names: Vec<&str> = list.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, [":)", "Kappa", "<3"]);
    }

    #[test]
    fn typed_forms_find_the_face() {
        assert!(word_matches(":)", ":)"));
        assert!(word_matches(":)", ":-)"));
        assert!(word_matches(":P", ":p"));
        assert!(word_matches(";P", ";-p"));
        assert!(word_matches("O_o", "o.O"));
        assert!(word_matches(":/", ":\\"));
        assert!(word_matches(":|", ":z"));
        assert!(word_matches("<3", "<3"));
        assert!(!word_matches(":)", ":("));
        assert!(!word_matches(":)", "x:)"));
        assert!(!word_matches(":D", ":d"));
    }

    #[test]
    fn smiley_for_reads_only_the_smilies() {
        let mut list = vec![smiley("1", r"\:-?\)"), smiley("11", r"\;-?\)")];
        tidy_names(&mut list);
        assert_eq!(smiley_for(";)", &list).map(|e| e.id.as_str()), Some("11"));
        assert_eq!(smiley_for(":-)", &list).map(|e| e.id.as_str()), Some("1"));
        assert!(smiley_for("hello", &list).is_none());
        let mut kappa = smiley("25", ":)");
        kappa.emote_type = Some("globals".into());
        assert!(smiley_for(":)", &[kappa]).is_none());
    }
}
