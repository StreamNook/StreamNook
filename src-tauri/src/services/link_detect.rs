//! Where a chat message stops being text and starts being a link.
//!
//! This is the canonical implementation. `streamnook.app`'s
//! `src/overlay/twitchClient.ts` re-implements it for the hosted overlay page
//! (which has no Rust) and MUST be kept in step — the two run over the same
//! messages and a viewer should not see a link in one and plain text in the
//! other. The TLD table below is duplicated there verbatim.
//!
//! Three shapes count as a link:
//!
//! 1. An explicit scheme: `https://example.com/x`.
//! 2. A `www.` host: `www.example.com`.
//! 3. A bare domain: `test.fr`, `example.co.uk/path`.
//!
//! Shape 3 is the reason this file exists. Matching it on punctuation alone
//! turns ordinary chat into links — "3.5" and "wait...what" and "home.Then" all
//! contain a dot between two runs of characters — so the last label is checked
//! against a real TLD table instead. That still linkifies a few words that are
//! not URLs (`main.rs` and `that.it` are both real TLDs) which is the same
//! behaviour every other chat client lands on; the alternative, dropping bare
//! domains entirely, is what users reported as broken.

use std::collections::HashSet;
use std::sync::OnceLock;

/// Punctuation that belongs to the sentence, not to the link: "see test.fr."
/// and "(example.com)". Stripped from the end and re-emitted as text.
const TRAILING: &[char] = &['.', ',', '!', '?', ':', ';', '\'', '"', ')', ']', '}', '>', '*', '_'];

/// Every ISO 3166-1 alpha-2 country code, plus the generic TLDs that actually
/// turn up in chat. Not exhaustive for new gTLDs on purpose: an unknown label
/// renders as plain text, which is a far cheaper mistake than linkifying the
/// second half of every sentence that runs two words together.
const CC_TLDS: &[&str] = &[
    "ac", "ad", "ae", "af", "ag", "ai", "al", "am", "ao", "aq", "ar", "as", "at", "au", "aw", "ax",
    "az", "ba", "bb", "bd", "be", "bf", "bg", "bh", "bi", "bj", "bm", "bn", "bo", "br", "bs", "bt",
    "bw", "by", "bz", "ca", "cc", "cd", "cf", "cg", "ch", "ci", "ck", "cl", "cm", "cn", "co", "cr",
    "cu", "cv", "cw", "cx", "cy", "cz", "de", "dj", "dk", "dm", "do", "dz", "ec", "ee", "eg", "er",
    "es", "et", "eu", "fi", "fj", "fk", "fm", "fo", "fr", "ga", "gd", "ge", "gf", "gg", "gh", "gi",
    "gl", "gm", "gn", "gp", "gq", "gr", "gs", "gt", "gu", "gw", "gy", "hk", "hm", "hn", "hr", "ht",
    "hu", "id", "ie", "il", "im", "in", "io", "iq", "ir", "is", "it", "je", "jm", "jo", "jp", "ke",
    "kg", "kh", "ki", "km", "kn", "kp", "kr", "kw", "ky", "kz", "la", "lb", "lc", "li", "lk", "lr",
    "ls", "lt", "lu", "lv", "ly", "ma", "mc", "md", "me", "mg", "mh", "mk", "ml", "mm", "mn", "mo",
    "mp", "mq", "mr", "ms", "mt", "mu", "mv", "mw", "mx", "my", "mz", "na", "nc", "ne", "nf", "ng",
    "ni", "nl", "no", "np", "nr", "nu", "nz", "om", "pa", "pe", "pf", "pg", "ph", "pk", "pl", "pm",
    "pn", "pr", "ps", "pt", "pw", "py", "qa", "re", "ro", "rs", "ru", "rw", "sa", "sb", "sc", "sd",
    "se", "sg", "sh", "si", "sk", "sl", "sm", "sn", "so", "sr", "ss", "st", "su", "sv", "sx", "sy",
    "sz", "tc", "td", "tf", "tg", "th", "tj", "tk", "tl", "tm", "tn", "to", "tr", "tt", "tv", "tw",
    "tz", "ua", "ug", "uk", "us", "uy", "uz", "va", "vc", "ve", "vg", "vi", "vn", "vu", "wf", "ws",
    "ye", "yt", "za", "zm", "zw",
];

const G_TLDS: &[&str] = &[
    "com", "net", "org", "edu", "gov", "mil", "int", "info", "biz", "name", "pro", "mobi", "asia",
    "jobs", "travel", "cat", "tel", "xxx", "aero", "coop", "museum", "post", "app", "dev", "page",
    "live", "tech", "space", "world", "today", "news", "media", "click", "link", "fun", "art",
    "life", "love", "moe", "gay", "lol", "wtf", "ninja", "pizza", "cloud", "digital", "agency",
    "studio", "design", "games", "group", "plus", "team", "tools", "video", "watch", "zone",
    "stream", "chat", "social", "community", "network", "systems", "solutions", "services", "top",
    "vip", "win", "one", "run", "cool", "fyi", "gift", "help", "host", "icu", "ink", "online",
    "party", "press", "pub", "red", "review", "rip", "rocks", "sale", "science", "sexy", "shop",
    "site", "store", "study", "tokyo", "trade", "uno", "website", "wiki", "work", "xyz", "yoga",
    "zip", "blog", "bar", "buzz", "date", "download", "email", "fit",
];

fn tlds() -> &'static HashSet<&'static str> {
    static TLDS: OnceLock<HashSet<&'static str>> = OnceLock::new();
    TLDS.get_or_init(|| CC_TLDS.iter().chain(G_TLDS.iter()).copied().collect())
}

fn has_scheme(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

/// Does `host` read as a registrable domain (`example.com`, `a.b.co.uk`)?
fn is_domain(host: &str) -> bool {
    // An `@` means a handle or an email address, neither of which we linkify;
    // credentials in a bare (schemeless) host are not a shape worth honouring.
    if host.is_empty() || host.contains('@') || !host.is_ascii() {
        return false;
    }
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() < 2 {
        return false;
    }
    // Every label: non-empty, alphanumeric or hyphen, and not hyphen-edged.
    for label in &labels {
        if label.is_empty()
            || label.starts_with('-')
            || label.ends_with('-')
            || !label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return false;
        }
    }
    // The TLD carries the decision. Alphabetic only, so "3.5" and "1.2.3" are
    // numbers rather than hosts.
    let tld = labels[labels.len() - 1];
    if !tld.bytes().all(|b| b.is_ascii_alphabetic()) {
        return false;
    }
    tlds().contains(tld.to_ascii_lowercase().as_str())
}

/// Split one whitespace-delimited word into its link and the sentence
/// punctuation trailing it. `None` when the word is not a link.
///
/// Returned as borrows of `word`, so the caller decides what to allocate.
pub fn split_link(word: &str) -> Option<(&str, &str)> {
    if word.is_empty() {
        return None;
    }
    let candidate = word.trim_end_matches(TRAILING);
    if candidate.is_empty() {
        return None;
    }
    let trailing = &word[candidate.len()..];

    let looks_like_link = if has_scheme(candidate) {
        true
    } else if candidate.to_ascii_lowercase().starts_with("www.") {
        true
    } else {
        // Bare domain: everything before the first path/query/fragment marker
        // has to read as a host on its own.
        let host_end = candidate
            .find(['/', '?', '#'])
            .unwrap_or(candidate.len());
        is_domain(&candidate[..host_end])
    };

    looks_like_link.then_some((candidate, trailing))
}

/// The href for a link's visible text. A schemeless link is assumed https,
/// which is what every browser address bar does now.
pub fn link_url(link: &str) -> String {
    if has_scheme(link) {
        link.to_string()
    } else {
        format!("https://{}", link)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link_of(word: &str) -> Option<(&str, &str)> {
        split_link(word)
    }

    #[test]
    fn explicit_schemes_and_www_still_match() {
        assert_eq!(link_of("https://example.com"), Some(("https://example.com", "")));
        assert_eq!(link_of("http://example.com/a?b=1"), Some(("http://example.com/a?b=1", "")));
        assert_eq!(link_of("www.example.com"), Some(("www.example.com", "")));
        assert_eq!(link_url("www.example.com"), "https://www.example.com");
        assert_eq!(link_url("https://example.com"), "https://example.com");
    }

    #[test]
    fn bare_domains_are_links() {
        // The reported case: a bare ccTLD domain with no scheme.
        assert_eq!(link_of("test.fr"), Some(("test.fr", "")));
        assert_eq!(link_of("example.com"), Some(("example.com", "")));
        assert_eq!(link_of("example.co.uk/path"), Some(("example.co.uk/path", "")));
        assert_eq!(link_of("streamnook.app/overlay?x=1"), Some(("streamnook.app/overlay?x=1", "")));
        assert_eq!(link_url("test.fr"), "https://test.fr");
    }

    #[test]
    fn sentence_punctuation_stays_out_of_the_link() {
        assert_eq!(link_of("test.fr."), Some(("test.fr", ".")));
        assert_eq!(link_of("(example.com)"), None); // leading paren is part of the host check
        assert_eq!(link_of("example.com),"), Some(("example.com", "),")));
        assert_eq!(link_of("https://example.com!"), Some(("https://example.com", "!")));
    }

    #[test]
    fn ordinary_chat_is_not_linkified() {
        // Numbers, versions and ellipses.
        assert_eq!(link_of("3.5"), None);
        assert_eq!(link_of("1.2.3"), None);
        assert_eq!(link_of("wait...what"), None);
        assert_eq!(link_of("..."), None);
        // A missing space after a full stop must not become a link.
        assert_eq!(link_of("home.Then"), None);
        assert_eq!(link_of("yeah.ok"), None);
        // Not a TLD.
        assert_eq!(link_of("some.thing"), None);
        // Handles and emails.
        assert_eq!(link_of("someone@example.com"), None);
        // Plain words.
        assert_eq!(link_of("hello"), None);
        assert_eq!(link_of(""), None);
    }

    #[test]
    fn malformed_hosts_are_rejected() {
        assert_eq!(link_of("-example.com"), None);
        assert_eq!(link_of("example-.com"), None);
        assert_eq!(link_of("example..com"), None);
        assert_eq!(link_of(".com"), None);
        assert_eq!(link_of("exam ple.com"), None); // words never contain spaces, but be explicit
    }

    #[test]
    fn tld_matching_ignores_case() {
        assert_eq!(link_of("Example.COM"), Some(("Example.COM", "")));
        assert_eq!(link_of("HTTPS://example.com"), Some(("HTTPS://example.com", "")));
    }

    #[test]
    fn the_tld_table_has_no_duplicates() {
        // A duplicate is harmless at runtime but signals a bad merge.
        let all: Vec<&str> = CC_TLDS.iter().chain(G_TLDS.iter()).copied().collect();
        let unique: HashSet<&str> = all.iter().copied().collect();
        assert_eq!(all.len(), unique.len(), "duplicate TLD in the table");
    }
}
