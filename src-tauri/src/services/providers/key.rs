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

// --- Row identity ------------------------------------------------------------
//
// The other "which channel is this" questions a mixed list asks. Each has a
// TypeScript twin, and each pair must agree: Rust and the page apply halves of
// the same rule (see `favorite_id`).

/// The key a stream card renders under: the bare lowercase login for Twitch,
/// `provider:channel` for everything else.
///
/// Twin of `streamKey` in `src/utils/streamProvider.ts`, and the same space: the
/// PERSISTED shape, where Twitch stays bare. Never compare it against `make_key`
/// output, which prefixes Twitch too.
pub fn stream_key(provider: &str, user_login: &str) -> String {
    if provider == DEFAULT_PROVIDER {
        user_login.to_lowercase()
    } else {
        make_key(provider, user_login)
    }
}

/// The id a FAVOURITE is stored under, read off a live row, or `None` when the
/// row names no channel.
///
/// Twin of `favoriteIdOf` in `src/utils/favorites.ts`, and the two must agree
/// EXACTLY: the unified Discover list drops a live favourite by this id while
/// the Favourites section above it is still chosen by the TypeScript one, so a
/// disagreement shows a channel twice, or in neither place. It is the
/// identifier each platform's live check accepts:
///
/// - twitch: the bare numeric user id
/// - kick: `kick:<slug>`, never the numeric id Kick rows also carry
/// - youtube: `youtube:<UC id>`, else `youtube:@handle`, else `None`, because a
///   browse row's login is a VIDEO id and names one broadcast
/// - others: `provider:<login>`
pub fn favorite_id(provider: &str, user_id: &str, user_login: &str) -> Option<String> {
    match provider {
        "twitch" => (!user_id.is_empty()).then(|| user_id.to_string()),
        "youtube" => {
            if user_id.starts_with("UC") {
                Some(make_key("youtube", user_id))
            } else if user_login.starts_with('@') {
                Some(make_key("youtube", user_login))
            } else {
                None
            }
        }
        _ => (!user_login.is_empty()).then(|| make_key(provider, user_login)),
    }
}

/// Every channel identity a row carries, in the `favorite_id` space.
///
/// A superset of `favorite_id`, for asking "is this channel already listed
/// somewhere else". A YouTube row fetched BY channel (a live check) names that
/// channel in `user_login` and usually the UC id in `user_id` too, and either
/// one matching a browse row's UC id is the same channel. Never use this to
/// decide what counts as a favourite; that must be `favorite_id` exactly.
pub fn channel_ids(provider: &str, user_id: &str, user_login: &str) -> Vec<String> {
    let mut ids: Vec<String> = favorite_id(provider, user_id, user_login).into_iter().collect();
    if provider == "youtube" && (is_youtube_channel_id(user_login) || user_login.starts_with('@')) {
        let by_login = make_key("youtube", user_login);
        if !ids.contains(&by_login) {
            ids.push(by_login);
        }
    }
    ids
}

/// A YouTube channel id: `UC` and 24 characters. The length is what matters, as
/// an 11-character VIDEO id can start with `UC` too. The YouTube adapter keeps
/// the same test privately (`is_channel_id_str`).
pub fn is_youtube_channel_id(id: &str) -> bool {
    id.len() == 24 && id.starts_with("UC")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_match_the_shared_fixtures() {
        let fixtures: serde_json::Value =
            serde_json::from_str(include_str!("../../../../src/utils/providerTwins.fixtures.json"))
                .expect("fixture file parses");
        let cases = |name: &str| fixtures[name].as_array().expect("cases").clone();
        let s = |v: &serde_json::Value, i: usize| v[i].as_str().expect("string").to_string();
        for c in cases("normalize") {
            assert_eq!(normalize_channel(&s(&c, 0), &s(&c, 1)), s(&c, 2));
        }
        for c in cases("make_key") {
            assert_eq!(make_key(&s(&c, 0), &s(&c, 1)), s(&c, 2));
        }
        for c in cases("parse_key") {
            let parsed = parse_key(&s(&c, 0));
            assert_eq!((parsed.provider, parsed.channel), (s(&c, 1), s(&c, 2)));
        }
    }

    #[test]
    fn stream_key_keeps_twitch_bare_and_prefixes_the_rest() {
        assert_eq!(stream_key("twitch", "XQC"), "xqc");
        assert_eq!(stream_key("kick", "XQC"), "kick:xqc");
        // A YouTube video id is case-sensitive.
        assert_eq!(stream_key("youtube", "AGr94tpNVkw"), "youtube:AGr94tpNVkw");
    }

    /// The table from `src/utils/favorites.test.ts`. The two derivations must
    /// agree, so a case added there belongs here too.
    #[test]
    fn favorite_id_agrees_with_its_typescript_twin() {
        assert_eq!(favorite_id("twitch", "71092938", "xqc").as_deref(), Some("71092938"));
        // Kick's live check queries `slug=`; the numeric id would never resolve.
        assert_eq!(favorite_id("kick", "12345", "XQC").as_deref(), Some("kick:xqc"));
        assert_eq!(
            favorite_id("youtube", "UCabcDEF123", "AGr94tpNVkw").as_deref(),
            Some("youtube:UCabcDEF123")
        );
        assert_eq!(favorite_id("youtube", "", "@somechannel").as_deref(), Some("youtube:@somechannel"));
        assert_eq!(favorite_id("youtube", "", "AGr94tpNVkw"), None);
        assert_eq!(favorite_id("tiktok", "999", "@Someone").as_deref(), Some("tiktok:@someone"));
        assert_eq!(favorite_id("twitch", "", "xqc"), None);
        assert_eq!(favorite_id("kick", "1", ""), None);
    }

    #[test]
    fn channel_ids_read_a_youtube_channel_off_either_field() {
        let uc = "UCMNEVbszv8ZyvSXoTn3yhpQ";
        let by_uc = format!("youtube:{uc}");
        // Live-check shape: asked about by UC id, which comes back in both fields.
        assert_eq!(channel_ids("youtube", uc, uc), vec![by_uc.clone()]);
        // The scrape found no id; the login still names the channel.
        assert_eq!(channel_ids("youtube", "", uc), vec![by_uc.clone()]);
        // Asked about by handle: both identities.
        assert_eq!(
            channel_ids("youtube", uc, "@chan"),
            vec![by_uc.clone(), "youtube:@chan".to_string()]
        );
        // A browse row: the login is a video id, even one that starts with UC.
        assert_eq!(channel_ids("youtube", uc, "UCa4bcdefgh"), vec![by_uc]);
        // Everything else carries exactly its favourite id, so a Kick 676 and
        // a Twitch 676 stay two channels.
        assert_eq!(channel_ids("kick", "676", "Bob"), vec!["kick:bob".to_string()]);
        assert_eq!(channel_ids("twitch", "676", "bob"), vec!["676".to_string()]);
    }

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
