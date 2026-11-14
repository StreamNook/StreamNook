//! Badge deep links: `streamnook:///badge/<set>[/<version>]`.
//!
//! The streamnook.app badge page's "Open in StreamNook" button hands off to the
//! app with this link. The host-less form is the one the site writes: released
//! apps read any `streamnook://<host>/...` link as a channel named `<host>`
//! (`share_link::parse`), so `streamnook://badge/clover` opens channel `badge`
//! on them, while an empty host reads as nothing. This build also reads the
//! host form `streamnook://badge/<set>[/<version>]` (with no path it stays a
//! channel link, as before).
//!
//! The set id and version are cleaned the way the site's `parseBadgePath`
//! cleans them, so a link names the same badge on both sides.

/// A badge a link points at.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BadgeLink {
    pub set_id: String,
    pub version: String,
}

/// The decoded, non-empty path segments of a URL.
fn path_segments(url: &url::Url) -> Vec<String> {
    url.path_segments()
        .map(|segments| {
            segments
                .filter(|s| !s.is_empty())
                .map(|s| urlencoding::decode(s).map(|d| d.into_owned()).unwrap_or_default())
                .collect()
        })
        .unwrap_or_default()
}

/// Set id: lowercased, kept to `[a-z0-9_-]`, at most 80; None when nothing is left.
fn clean_set_id(raw: &str) -> Option<String> {
    let id: String = raw
        .to_ascii_lowercase()
        .chars()
        .filter(|c| matches!(c, 'a'..='z' | '0'..='9' | '_' | '-'))
        .take(80)
        .collect();
    (!id.is_empty()).then_some(id)
}

/// Version: kept to `[A-Za-z0-9_-]`, at most 20, `1` when nothing is left.
fn clean_version(raw: &str) -> String {
    let version: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
        .take(20)
        .collect();
    if version.is_empty() {
        "1".to_string()
    } else {
        version
    }
}

/// Read a badge link back into the badge it names.
pub fn parse(url: &url::Url) -> Option<BadgeLink> {
    if url.scheme() != "streamnook" {
        return None;
    }
    let mut segments = path_segments(url);
    match url.host_str().unwrap_or("") {
        "" => {
            if segments.is_empty() || !segments[0].eq_ignore_ascii_case("badge") {
                return None;
            }
            segments.remove(0);
        }
        host if host.eq_ignore_ascii_case("badge") => {}
        _ => return None,
    }
    if segments.is_empty() || segments.len() > 2 {
        return None;
    }
    Some(BadgeLink {
        set_id: clean_set_id(&segments[0])?,
        version: clean_version(segments.get(1).map(String::as_str).unwrap_or("1")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(link: &str) -> Option<BadgeLink> {
        parse(&url::Url::parse(link).expect("test url parses"))
    }

    fn badge(set_id: &str, version: &str) -> Option<BadgeLink> {
        Some(BadgeLink {
            set_id: set_id.to_string(),
            version: version.to_string(),
        })
    }

    #[test]
    fn the_hostless_link_the_site_writes_opens_a_badge() {
        assert_eq!(read("streamnook:///badge/clover"), badge("clover", "1"));
        assert_eq!(read("streamnook:///badge/clover/3"), badge("clover", "3"));
        assert_eq!(read("streamnook:/badge/x"), badge("x", "1"));
    }

    #[test]
    fn set_ids_lowercase_and_versions_keep_their_case() {
        assert_eq!(read("streamnook:///badge/Clover/2"), badge("clover", "2"));
        assert_eq!(read("streamnook:///badge/clover/V2_b"), badge("clover", "V2_b"));
    }

    #[test]
    fn the_host_form_opens_a_badge_too() {
        assert_eq!(read("streamnook://badge/clover"), badge("clover", "1"));
        assert_eq!(read("streamnook://BADGE/clover/2"), badge("clover", "2"));
    }

    #[test]
    fn nothing_usable_is_nothing() {
        assert_eq!(read("streamnook://badge"), None);
        assert_eq!(read("streamnook://badge/"), None);
        assert_eq!(read("streamnook:///badge"), None);
        assert_eq!(read("streamnook:///badge/a/b/c"), None);
        assert_eq!(read("streamnook://badge/a/b/c"), None);
        assert_eq!(read("streamnook:///"), None);
        assert_eq!(read("streamnook:///badge/!!!"), None);
        assert_eq!(read("streamnook:///other/clover"), None);
        assert_eq!(read("streamnook://notify/badge/clover"), None);
        assert_eq!(read("streamnook://watch/clover"), None);
        assert_eq!(read("https://streamnook.app/badge/clover"), None);
        assert_eq!(read("other:///badge/clover"), None);
    }

    #[test]
    fn odd_characters_are_dropped_and_lengths_capped() {
        assert_eq!(read("streamnook:///badge/cl.o%20ver/%21"), badge("clover", "1"));
        let long_set = "a".repeat(120);
        let long_version = "9".repeat(40);
        let parsed = read(&format!("streamnook:///badge/{long_set}/{long_version}")).unwrap();
        assert_eq!(parsed.set_id.len(), 80);
        assert_eq!(parsed.version.len(), 20);
    }
}
