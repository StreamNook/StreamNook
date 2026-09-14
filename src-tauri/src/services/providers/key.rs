//! Composite source-key codec shared by the chat bus and the provider adapters.
//!
//! A source is identified by `"<provider>:<channel>"` (e.g. `"kick:xqc"`). This
//! mirrors `src/utils/providerKey.ts` on the frontend. A bare key with no
//! recognised provider prefix is treated as a legacy Twitch login, so older
//! persisted state and the existing Twitch code paths keep working unchanged.

pub const PROVIDER_IDS: [&str; 6] = ["twitch", "kick", "youtube", "rumble", "tiktok", "x"];
pub const DEFAULT_PROVIDER: &str = "twitch";

pub fn is_provider_id(s: &str) -> bool {
    PROVIDER_IDS.contains(&s)
}

/// Case policy for a channel identifier.
///
/// A YouTube id addresses one specific video or channel, so `AGr94tpNVkw` and
/// `agr94tpnvkw` are different things and normalising one produces "This video is
/// unavailable". Twitch logins and Kick slugs are case-insensitive.
///
/// Anything that stores or compares a channel identifier must go through this,
/// not its own `.to_lowercase()`. The follow commands each had their own, which
/// is how imported YouTube follows (stored with their real `UC` casing) stopped
/// matching the lowercased ones the follow button wrote.
pub fn normalize_channel(provider: &str, channel: &str) -> String {
    let c = channel.trim();
    if provider == "youtube" {
        c.to_string()
    } else {
        c.to_lowercase()
    }
}

/// Whether two identifiers name the same channel on `provider`.
///
/// Case-insensitive even for YouTube, deliberately: this is a READ, and rows
/// persisted before `normalize_channel` existed were lowercased on the way in.
/// Comparing loosely lets those match while writes stay canonical.
pub fn same_channel(provider: &str, a: &str, b: &str) -> bool {
    if provider == "youtube" {
        a.eq_ignore_ascii_case(b)
    } else {
        normalize_channel(provider, a) == normalize_channel(provider, b)
    }
}

/// Build a composite key.
pub fn make_key(provider: &str, channel: &str) -> String {
    format!("{}:{}", provider, normalize_channel(provider, channel))
}

pub struct ParsedKey {
    pub provider: String,
    pub channel: String,
}

/// Split a composite key. Only splits on a recognised provider prefix; anything
/// else (a bare login, or text that merely contains a colon) is read as Twitch.
pub fn parse_key(key: &str) -> ParsedKey {
    if let Some(idx) = key.find(':') {
        let maybe = &key[..idx];
        if is_provider_id(maybe) {
            return ParsedKey {
                provider: maybe.to_string(),
                channel: key[idx + 1..].to_string(),
            };
        }
    }
    ParsedKey {
        provider: DEFAULT_PROVIDER.to_string(),
        channel: key.to_lowercase(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_provider_keys() {
        let k = make_key("kick", "XQC");
        assert_eq!(k, "kick:xqc");
        let p = parse_key(&k);
        assert_eq!(p.provider, "kick");
        assert_eq!(p.channel, "xqc");
    }

    #[test]
    fn youtube_keeps_its_casing_others_do_not() {
        // A UC id is case-sensitive; lowercasing it yields a channel that doesn't
        // exist. Kick slugs and Twitch logins are case-insensitive.
        assert_eq!(normalize_channel("youtube", "UCabcDEF123"), "UCabcDEF123");
        assert_eq!(normalize_channel("kick", "XQC"), "xqc");
        assert_eq!(normalize_channel("twitch", "XQC"), "xqc");
        assert_eq!(make_key("youtube", "UCabcDEF123"), "youtube:UCabcDEF123");
    }

    #[test]
    fn same_channel_tolerates_legacy_lowercased_youtube_rows() {
        // Rows written before the casing fix are lowercased on disk; a read must
        // still match them against the canonical id.
        assert!(same_channel("youtube", "ucabcdef123", "UCabcDEF123"));
        assert!(same_channel("kick", "XQC", "xqc"));
        assert!(!same_channel("youtube", "UCabc", "UCdef"));
    }

    #[test]
    fn bare_login_reads_as_twitch() {
        let p = parse_key("xqc");
        assert_eq!(p.provider, "twitch");
        assert_eq!(p.channel, "xqc");
    }

    #[test]
    fn unknown_prefix_reads_as_twitch() {
        // A channel literally named like a provider, or stray text with a colon.
        let p = parse_key("notaprovider:thing");
        assert_eq!(p.provider, "twitch");
        assert_eq!(p.channel, "notaprovider:thing");
    }
}
