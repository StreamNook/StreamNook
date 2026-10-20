//! Does a chat message name you? The one rule for it, shared by every chat
//! (the rule engine stamps rows with it) and kept free of app state so the
//! closed-app alert watcher can link the same file.
//!
//! Three ways to be named, each its own setting except the first:
//! - `@login`, always (the rule the chat row has had all along);
//! - your login as a whole word, without the `@`;
//! - your display name as a whole word, when it differs from your login
//!   (a localized or styled name), with or without the `@`.
//!
//! A whole word means the characters on either side are not ASCII letters,
//! digits or `_`, so `brandon` matches "hi brandon!" but not "brandons" or
//! "xbrandon". Non-ASCII neighbours count as boundaries, so a Korean or
//! Japanese display name still matches inside unspaced text.

/// Who is being looked for. Both names lowercased; `display` only when it is
/// not just the login in another case.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MentionTarget {
    pub login: String,
    pub display: Option<String>,
}

impl MentionTarget {
    pub fn new(login: &str, display_name: &str) -> Self {
        let login = login.trim().trim_start_matches('@').to_lowercase();
        let display = display_name.trim().trim_start_matches('@').to_lowercase();
        let display = (!display.is_empty() && display != login).then_some(display);
        MentionTarget { login, display }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MatchOptions {
    pub bare_login: bool,
    pub display_name: bool,
}

impl Default for MatchOptions {
    fn default() -> Self {
        MatchOptions {
            bare_login: true,
            display_name: true,
        }
    }
}

/// Which name matched, for the inbox's "mentioned you" line and for tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameHit {
    AtLogin,
    BareLogin,
    DisplayName,
}

fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Whether `needle` (already lowercase) appears in `hay` (already lowercase)
/// with a boundary after it and, unless `at_prefixed`, before it too. An
/// `@` right before the needle always counts as a boundary.
fn find_word(hay: &str, needle: &str, at_prefixed: bool) -> bool {
    if needle.is_empty() {
        return false;
    }
    let mut from = 0;
    while let Some(pos) = hay[from..].find(needle) {
        let start = from + pos;
        let end = start + needle.len();
        let before_ok = at_prefixed || hay[..start].chars().next_back().map_or(true, |c| !is_word(c));
        let after_ok = hay[end..].chars().next().map_or(true, |c| !is_word(c));
        if before_ok && after_ok {
            return true;
        }
        // Step past this occurrence's first character, on a char boundary.
        from = start + hay[start..].chars().next().map_or(1, |c| c.len_utf8());
    }
    false
}

/// The strongest way `content` names `target`, if any.
pub fn find_mention(content: &str, target: &MentionTarget, opts: &MatchOptions) -> Option<NameHit> {
    if target.login.is_empty() {
        return None;
    }
    let lower = content.to_lowercase();
    if find_word(&lower, &format!("@{}", target.login), true) {
        return Some(NameHit::AtLogin);
    }
    if opts.bare_login && find_word(&lower, &target.login, false) {
        return Some(NameHit::BareLogin);
    }
    if opts.display_name {
        if let Some(display) = &target.display {
            if find_word(&lower, display, false) {
                return Some(NameHit::DisplayName);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(content: &str, login: &str, display: &str, opts: MatchOptions) -> Option<NameHit> {
        find_mention(content, &MentionTarget::new(login, display), &opts)
    }

    const ALL: MatchOptions = MatchOptions {
        bare_login: true,
        display_name: true,
    };
    const AT_ONLY: MatchOptions = MatchOptions {
        bare_login: false,
        display_name: false,
    };

    #[test]
    fn at_login_always_matches_with_a_word_boundary_after() {
        assert_eq!(hit("hey @Brandon!", "brandon", "", AT_ONLY), Some(NameHit::AtLogin));
        assert_eq!(hit("@BRANDON", "brandon", "", AT_ONLY), Some(NameHit::AtLogin));
        assert_eq!(hit("@brandons", "brandon", "", AT_ONLY), None);
        assert_eq!(hit("@brandon_", "brandon", "", AT_ONLY), None);
        // Same as the row rule before: anything may sit before the @.
        assert_eq!(hit("x@brandon", "brandon", "", AT_ONLY), Some(NameHit::AtLogin));
    }

    #[test]
    fn bare_login_needs_both_boundaries_and_its_setting() {
        assert_eq!(hit("brandon nice play", "brandon", "", ALL), Some(NameHit::BareLogin));
        assert_eq!(hit("gg, Brandon.", "brandon", "", ALL), Some(NameHit::BareLogin));
        assert_eq!(hit("brandons", "brandon", "", ALL), None);
        assert_eq!(hit("xbrandon", "brandon", "", ALL), None);
        assert_eq!(hit("brandon nice play", "brandon", "", AT_ONLY), None);
        // A later whole-word copy still counts after an embedded one.
        assert_eq!(hit("brandons and brandon", "brandon", "", ALL), Some(NameHit::BareLogin));
    }

    #[test]
    fn display_name_matches_only_when_it_differs_from_the_login() {
        assert_eq!(MentionTarget::new("brandon", "Brandon").display, None);
        assert_eq!(hit("hi winters", "br_winters", "Winters", ALL), Some(NameHit::DisplayName));
        assert_eq!(hit("hi @winters", "br_winters", "Winters", ALL), Some(NameHit::DisplayName));
        assert_eq!(
            hit("hi winters", "br_winters", "Winters", MatchOptions { bare_login: true, display_name: false }),
            None
        );
    }

    #[test]
    fn a_non_ascii_display_name_matches_inside_unspaced_text() {
        assert_eq!(hit("안녕브랜든님", "brandon", "브랜든", ALL), Some(NameHit::DisplayName));
        assert_eq!(hit("こんにちは", "brandon", "브랜든", ALL), None);
    }

    #[test]
    fn an_empty_login_never_matches() {
        assert_eq!(hit("anything @", "", "", ALL), None);
        assert_eq!(hit("", "brandon", "", ALL), None);
    }

    #[test]
    fn at_login_wins_over_the_weaker_hits() {
        assert_eq!(hit("brandon @brandon", "brandon", "", ALL), Some(NameHit::AtLogin));
    }
}
