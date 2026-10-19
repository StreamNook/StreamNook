//! Share links for a stream, and reading them back.
//!
//! One shape per platform, built and parsed only here so the share buttons, the
//! deep-link handler and the streamnook.app landing page agree:
//!
//! | Platform | Web link                                      | App link                                 |
//! |----------|-----------------------------------------------|------------------------------------------|
//! | Twitch   | `https://streamnook.app/w/<login>?d=MMDD`      | `streamnook://watch/<login>`             |
//! | others   | `https://streamnook.app/w/<provider>/<channel>?d=MMDD` | `streamnook://watch/<provider>/<channel>` |
//!
//! The web page hands off to the app link when StreamNook is installed and
//! falls back to the platform's own page when it is not. Twitch keeps the
//! one-segment form every link shared so far uses.
//!
//! The day tag (`?d=MMDD`) is there for Discord, which caches a link's preview
//! per exact URL for about a day: a link shared on a new day is a new URL and
//! gets a fresh card. The page ignores it for routing.

use super::providers::key;

pub const SHARE_ORIGIN: &str = "https://streamnook.app";

/// Platforms a share link can name.
const SHAREABLE: [&str; 4] = ["twitch", "kick", "youtube", "tiktok"];

/// A stream a link points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchLink {
    pub provider: &'static str,
    pub channel: String,
}

impl WatchLink {
    /// What `startStream` takes: the bare login for Twitch, `provider:channel`
    /// for everything else (the persisted `stream_key` space).
    pub fn stream_key(&self) -> String {
        key::stream_key(self.provider, &self.channel)
    }
}

fn shareable(provider: &str) -> Option<&'static str> {
    SHAREABLE.iter().copied().find(|p| *p == provider)
}

fn is_video_id(s: &str) -> bool {
    s.len() == 11 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn is_channel_id(s: &str) -> bool {
    s.len() == 24
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        && s[..2].eq_ignore_ascii_case("uc")
}

fn keep(raw: &str, max: usize, ok: impl Fn(char) -> bool) -> String {
    raw.chars().filter(|c| ok(*c)).take(max).collect()
}

/// The channel as the platform addresses it, or None when nothing usable is
/// left. Links are untrusted input, so this is also the parse-side filter.
///
/// - Twitch: lowercase `[a-z0-9_]`, at most 25 characters.
/// - Kick: lowercase slug, `[a-z0-9_-]`.
/// - TikTok: lowercase handle without the `@`, `[a-z0-9_.]`.
/// - YouTube: an 11-character video id or a `UC` channel id as given (both are
///   case-sensitive), otherwise an `@handle`.
pub fn clean_channel(provider: &str, raw: &str) -> Option<String> {
    let raw = raw.trim();
    let clean = match provider {
        "twitch" => keep(raw, 25, |c| c.is_ascii_alphanumeric() || c == '_').to_ascii_lowercase(),
        "kick" => keep(raw, 64, |c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            .to_ascii_lowercase(),
        "tiktok" => keep(raw.trim_start_matches('@'), 64, |c| {
            c.is_ascii_alphanumeric() || c == '_' || c == '.'
        })
        .to_ascii_lowercase(),
        "youtube" => {
            if is_video_id(raw) || is_channel_id(raw) {
                raw.to_string()
            } else {
                let handle = keep(raw.trim_start_matches('@'), 100, |c| {
                    c.is_alphanumeric() || c == '_' || c == '-' || c == '.'
                });
                if handle.is_empty() {
                    String::new()
                } else {
                    format!("@{handle}")
                }
            }
        }
        _ => return None,
    };
    (!clean.is_empty()).then_some(clean)
}

/// The path after `/w/` (web) or `watch/` (app) for a stream.
fn link_path(provider: &'static str, channel: &str) -> String {
    let encoded = match channel.strip_prefix('@') {
        Some(handle) => format!("@{}", urlencoding::encode(handle)),
        None => urlencoding::encode(channel).into_owned(),
    };
    if provider == "twitch" {
        encoded
    } else {
        format!("{provider}/{encoded}")
    }
}

/// The web link to copy or share. `day` is the MMDD tag; see the module notes.
pub fn share_url(provider: &str, channel: &str, day: &str) -> Option<String> {
    let p = shareable(provider)?;
    let channel = clean_channel(p, channel)?;
    Some(format!("{SHARE_ORIGIN}/w/{}?d={day}", link_path(p, &channel)))
}

/// The app link the landing page hands off to.
pub fn deep_link(provider: &str, channel: &str) -> Option<String> {
    let p = shareable(provider)?;
    let channel = clean_channel(p, channel)?;
    Some(format!("streamnook://watch/{}", link_path(p, &channel)))
}

/// Today's day tag in local time, as the share buttons always used.
pub fn day_tag() -> String {
    chrono::Local::now().format("%m%d").to_string()
}

/// Read a share link back into the stream it names.
///
/// Accepts every shape `share_url` and `deep_link` produce, plus the older ones
/// still in circulation: `streamnook://w/<login>` and a bare
/// `streamnook://<login>`. A one-segment link is always Twitch, so a Twitch
/// channel called "kick" still opens on Twitch.
pub fn parse(url: &url::Url) -> Option<WatchLink> {
    let path: Vec<String> = url
        .path_segments()
        .map(|segments| {
            segments
                .filter(|s| !s.is_empty())
                .map(|s| urlencoding::decode(s).map(|d| d.into_owned()).unwrap_or_default())
                .collect()
        })
        .unwrap_or_default();
    let segments: Vec<String> = match url.scheme() {
        "streamnook" => {
            let action = url.host_str().unwrap_or("");
            match action {
                "watch" | "w" => path,
                "" => return None,
                _ => std::iter::once(action.to_string()).chain(path).collect(),
            }
        }
        "https" | "http" => {
            let host = url.host_str().unwrap_or("");
            if host != "streamnook.app" && host != "www.streamnook.app" {
                return None;
            }
            let mut path = path.into_iter();
            if path.next().as_deref() != Some("w") {
                return None;
            }
            path.collect()
        }
        _ => return None,
    };
    let first = segments.first()?;
    let (provider, raw) = match (shareable(&first.to_ascii_lowercase()), segments.get(1)) {
        (Some(p), Some(channel)) => (p, channel.as_str()),
        _ => ("twitch", first.as_str()),
    };
    Some(WatchLink {
        provider,
        channel: clean_channel(provider, raw)?,
    })
}

/// The stream a link should open on this build: `parse`, then nothing for a
/// platform this build cannot watch (`account_roster::can_watch`). A phone
/// handed a YouTube or TikTok link simply opens nothing, the same as a link
/// it cannot read.
pub fn openable(url: &url::Url) -> Option<WatchLink> {
    openable_with(url, crate::services::account_roster::can_watch)
}

fn openable_with(url: &url::Url, can_watch: impl Fn(&str) -> bool) -> Option<WatchLink> {
    parse(url).filter(|link| can_watch(link.provider))
}

/// `parse` for a link held as text.
pub fn parse_str(link: &str) -> Option<WatchLink> {
    url::Url::parse(link.trim()).ok().as_ref().and_then(parse)
}

/// The channel a share should name.
///
/// A YouTube stream is shared by its live VIDEO id where one is known, since
/// that is the page a viewer without StreamNook should land on. The playing
/// row's login is a video id when it came from browse or search; a stream
/// started from a channel (`@handle`, `UC` id) carries the video id on the row
/// (`stream_id`) or in the meta cache its chat filled. With none of those, the
/// channel itself is shared, which opens its live page.
pub fn share_channel(provider: &str, channel: &str, stream_id: Option<&str>) -> String {
    let channel = channel.trim();
    if provider != "youtube" || is_video_id(channel) {
        return channel.to_string();
    }
    if let Some(id) = stream_id.map(str::trim).filter(|id| is_video_id(id)) {
        return id.to_string();
    }
    super::providers::youtube::channel_meta(channel)
        .filter(|meta| meta.is_live)
        .and_then(|meta| meta.video_id)
        .filter(|id| is_video_id(id))
        .unwrap_or_else(|| channel.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(provider: &'static str, channel: &str) -> Option<WatchLink> {
        Some(WatchLink {
            provider,
            channel: channel.to_string(),
        })
    }

    #[test]
    fn twitch_share_links_are_unchanged() {
        assert_eq!(
            share_url("twitch", "Shroud", "1014").as_deref(),
            Some("https://streamnook.app/w/shroud?d=1014")
        );
        assert_eq!(deep_link("twitch", "shroud").as_deref(), Some("streamnook://watch/shroud"));
    }

    #[test]
    fn other_platforms_name_their_provider() {
        let cases = [
            (
                "youtube",
                "m4Nrq3auEUI",
                "https://streamnook.app/w/youtube/m4Nrq3auEUI?d=1014",
                "streamnook://watch/youtube/m4Nrq3auEUI",
            ),
            (
                "youtube",
                "@shachitoo",
                "https://streamnook.app/w/youtube/@shachitoo?d=1014",
                "streamnook://watch/youtube/@shachitoo",
            ),
            (
                "youtube",
                "UCX6OQ3DkcsbYNE6H8uQQuVA",
                "https://streamnook.app/w/youtube/UCX6OQ3DkcsbYNE6H8uQQuVA?d=1014",
                "streamnook://watch/youtube/UCX6OQ3DkcsbYNE6H8uQQuVA",
            ),
            (
                "kick",
                "Some-Streamer_1",
                "https://streamnook.app/w/kick/some-streamer_1?d=1014",
                "streamnook://watch/kick/some-streamer_1",
            ),
            (
                "tiktok",
                "@Some.User_9",
                "https://streamnook.app/w/tiktok/some.user_9?d=1014",
                "streamnook://watch/tiktok/some.user_9",
            ),
        ];
        for (provider, channel, web, app) in cases {
            assert_eq!(share_url(provider, channel, "1014").as_deref(), Some(web), "{provider} {channel}");
            assert_eq!(deep_link(provider, channel).as_deref(), Some(app), "{provider} {channel}");
        }
    }

    #[test]
    fn every_built_link_reads_back_to_its_stream() {
        for (provider, channel) in [
            ("twitch", "shroud"),
            ("youtube", "m4Nrq3auEUI"),
            ("youtube", "@shachitoo"),
            ("youtube", "UCX6OQ3DkcsbYNE6H8uQQuVA"),
            ("kick", "some-streamer_1"),
            ("tiktok", "some.user_9"),
            ("twitch", "kick"),
        ] {
            let want = link(shareable(provider).unwrap(), channel);
            let web = share_url(provider, channel, "1014").unwrap();
            let app = deep_link(provider, channel).unwrap();
            assert_eq!(parse_str(&web), want, "{web}");
            assert_eq!(parse_str(&app), want, "{app}");
        }
    }

    #[test]
    fn older_link_shapes_still_open_on_twitch() {
        assert_eq!(parse_str("streamnook://w/xQc"), link("twitch", "xqc"));
        assert_eq!(parse_str("streamnook://xqc"), link("twitch", "xqc"));
        assert_eq!(parse_str("streamnook://watch/xqc/"), link("twitch", "xqc"));
        assert_eq!(parse_str("https://streamnook.app/w/xqc?d=0101"), link("twitch", "xqc"));
        // A one-segment link names a Twitch channel even when it is a platform name.
        assert_eq!(parse_str("streamnook://watch/youtube"), link("twitch", "youtube"));
    }

    #[test]
    fn junk_links_open_nothing() {
        for junk in [
            "streamnook://",
            "streamnook://watch",
            "streamnook://watch/youtube/!!!",
            "https://example.com/w/xqc",
            "https://streamnook.app/u/xqc",
            "https://streamnook.app/",
            "not a url",
        ] {
            assert_eq!(parse_str(junk), None, "{junk}");
        }
    }

    #[test]
    fn a_link_becomes_the_start_stream_key() {
        assert_eq!(parse_str("streamnook://watch/xqc").unwrap().stream_key(), "xqc");
        assert_eq!(
            parse_str("streamnook://watch/youtube/m4Nrq3auEUI").unwrap().stream_key(),
            "youtube:m4Nrq3auEUI"
        );
        assert_eq!(parse_str("streamnook://watch/kick/xqc").unwrap().stream_key(), "kick:xqc");
        assert_eq!(
            parse_str("streamnook://watch/tiktok/some.user").unwrap().stream_key(),
            "tiktok:some.user"
        );
    }

    #[test]
    fn a_non_ascii_youtube_handle_round_trips() {
        let web = share_url("youtube", "@しゃち", "1014").unwrap();
        assert!(web.is_ascii(), "{web}");
        assert_eq!(parse_str(&web), link("youtube", "@しゃち"));
    }

    #[test]
    fn youtube_shares_the_video_over_the_channel() {
        assert_eq!(share_channel("youtube", "m4Nrq3auEUI", None), "m4Nrq3auEUI");
        assert_eq!(share_channel("youtube", "@shachitoo", Some("m4Nrq3auEUI")), "m4Nrq3auEUI");
        // An unknown channel with no row id shares the channel's live page.
        assert_eq!(share_channel("youtube", "@nobody-known-here", Some("")), "@nobody-known-here");
        assert_eq!(share_channel("kick", "xqc", Some("123")), "xqc");
    }

    #[test]
    fn a_build_that_cannot_watch_a_platform_opens_none_of_its_links() {
        let url = |s: &str| url::Url::parse(s).unwrap();
        let phone = |p: &str| !matches!(p, "youtube" | "tiktok");
        assert!(openable_with(&url("streamnook://watch/youtube/dQw4w9WgXcQ"), phone).is_none());
        assert!(openable_with(&url("streamnook://watch/tiktok/someone"), phone).is_none());
        assert!(openable_with(&url("https://streamnook.app/w/youtube/@handle?d=0101"), phone).is_none());
        assert_eq!(openable_with(&url("streamnook://watch/kick/xqc"), phone).map(|l| l.stream_key()).as_deref(), Some("kick:xqc"));
        assert_eq!(openable_with(&url("streamnook://watch/xqc"), phone).map(|l| l.stream_key()).as_deref(), Some("xqc"));
        // Every platform opens where the build can watch it.
        assert!(openable_with(&url("streamnook://watch/youtube/dQw4w9WgXcQ"), |_| true).is_some());
        // This build answers from the roster.
        assert_eq!(openable(&url("streamnook://watch/tiktok/someone")).is_some(), cfg!(desktop));
    }

    #[test]
    fn platforms_without_a_share_link_build_none() {
        assert_eq!(share_url("rumble", "someone", "1014"), None);
        assert_eq!(deep_link("x", "someone"), None);
    }
}
