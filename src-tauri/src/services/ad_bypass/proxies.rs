//! The bundled TTV-LOL relay pool.
//!
//! These are public community playlist relays that answer with an anonymous,
//! region-shifted master playlist for a channel. Carried forward from
//! StreamNook 7.8.6 (`services/proxy_health.rs` at `2bf9720`), trimmed to what
//! ad-free playback actually needs: the base URLs and their region labels. The
//! health-check/optimizer surface stayed behind, because the resolver races the
//! whole pool per stream and the first valid answer wins anyway.

/// Bundled relay bases, in the order 7.8.6 prioritized them. Order is only a
/// tiebreaker for display: the resolver races every base at once.
const BUNDLED: &[(&str, &str)] = &[
    ("https://lb-na.cdn-perfprod.com", "NA"),
    ("https://lb-eu.cdn-perfprod.com", "EU"),
    ("https://lb-eu2.cdn-perfprod.com", "EU"),
    ("https://lb-eu3.cdn-perfprod.com", "RU"),
    ("https://lb-eu4.cdn-perfprod.com", "EU"),
    ("https://lb-eu5.cdn-perfprod.com", "EU"),
    ("https://lb-as.cdn-perfprod.com", "AS"),
    ("https://lb-sa.cdn-perfprod.com", "SA"),
    ("https://eu.luminous.dev", "EU"),
    ("https://eu2.luminous.dev", "EU"),
    ("https://as.luminous.dev", "AS"),
    ("https://twitch.nadeko.net", "RU"),
];

/// Every bundled base URL, without a trailing slash.
pub fn bundled() -> Vec<String> {
    BUNDLED.iter().map(|(url, _)| (*url).to_string()).collect()
}

/// Region label for a base (`https://lb-eu.cdn-perfprod.com` → `EU`).
/// Best-effort: a user-supplied base that isn't bundled has no label.
pub fn region_for_base(base: &str) -> Option<String> {
    let norm = base.trim_end_matches('/');
    BUNDLED
        .iter()
        .find(|(url, _)| *url == norm)
        .map(|(_, region)| (*region).to_string())
}

/// Parse the user's relay override into base URLs. Accepts commas, whitespace
/// or newlines between entries, and tolerates the legacy
/// `--twitch-proxy-playlist=a,b` form that 7.8.6 stored. An empty result means
/// "use the bundled pool".
pub fn parse_bases(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    for tok in raw.split([',', ' ', '\t', '\r', '\n']) {
        let tok = tok.trim();
        let url = tok
            .strip_prefix("--twitch-proxy-playlist=")
            .unwrap_or(tok)
            .trim_end_matches('/');
        if url.starts_with("http://") || url.starts_with("https://") {
            let url = url.to_string();
            if !out.contains(&url) {
                out.push(url);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_pool_is_intact() {
        let pool = bundled();
        assert_eq!(pool.len(), 12);
        assert!(pool.iter().all(|u| u.starts_with("https://")));
        assert!(pool.iter().all(|u| !u.ends_with('/')));
    }

    #[test]
    fn regions_resolve_with_or_without_a_trailing_slash() {
        assert_eq!(
            region_for_base("https://lb-eu.cdn-perfprod.com"),
            Some("EU".into())
        );
        assert_eq!(
            region_for_base("https://twitch.nadeko.net/"),
            Some("RU".into())
        );
        assert_eq!(region_for_base("https://example.invalid"), None);
    }

    #[test]
    fn parses_plain_lists_and_the_legacy_streamlink_form() {
        assert_eq!(
            parse_bases("https://a.example.com, https://b.example.com/"),
            vec!["https://a.example.com", "https://b.example.com"]
        );
        assert_eq!(
            parse_bases("--twitch-proxy-playlist=https://a.example.com,https://b.example.com"),
            vec!["https://a.example.com", "https://b.example.com"]
        );
        assert!(parse_bases("  ").is_empty());
        assert!(parse_bases("not-a-url").is_empty());
    }

    #[test]
    fn duplicate_entries_collapse() {
        assert_eq!(
            parse_bases("https://a.example.com\nhttps://a.example.com/"),
            vec!["https://a.example.com"]
        );
    }
}
