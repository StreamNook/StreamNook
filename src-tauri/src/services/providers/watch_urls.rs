//! Classify a watch URL into its platform + channel so `start_stream` can
//! dispatch without any new command surface. Twitch URLs fall through to the
//! existing path untouched; provider URLs route to `StreamSource` resolution.
//!
//! The accepted shapes mirror what `ProviderStream.watch_url` emits plus what a
//! user might paste:
//!   kick.com/<slug>                       (NOT /video/, /categories/, /clips/)
//!   youtube.com/watch?v=<id>, youtu.be/<id>, youtube.com/live/<id>,
//!   youtube.com/@handle[/live], youtube.com/channel/UC…[/live]
//!   tiktok.com/@handle[/live]

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchTarget {
    /// Anything that isn't a recognized provider URL, including all of today's
    /// `https://twitch.tv/<ch>` traffic. The caller falls through to the
    /// existing Twitch path, preserving its error behavior for junk input.
    Twitch,
    Provider {
        provider: &'static str,
        /// What the platform's resolver addresses: slug (Kick), watch id /
        /// @handle / UC id (YouTube), @-less handle (TikTok).
        channel: String,
    },
}

pub fn classify(url: &str) -> WatchTarget {
    let stripped = url
        .trim()
        .strip_prefix("https://")
        .or_else(|| url.trim().strip_prefix("http://"))
        .unwrap_or(url.trim());
    let stripped = stripped.strip_prefix("www.").unwrap_or(stripped);

    if let Some(rest) = host_path(stripped, "kick.com") {
        // First path segment is the slug; reject Kick's non-channel surfaces.
        let slug = rest.split(['/', '?', '#']).next().unwrap_or("");
        let reserved = [
            "video",
            "categories",
            "category",
            "clips",
            "browse",
            "following",
            "search",
        ];
        if !slug.is_empty() && !reserved.contains(&slug) {
            return WatchTarget::Provider {
                provider: "kick",
                channel: slug.to_ascii_lowercase(),
            };
        }
        return WatchTarget::Twitch;
    }

    if let Some(id) = stripped.strip_prefix("youtu.be/") {
        let id = id.split(['/', '?', '#']).next().unwrap_or("");
        if !id.is_empty() {
            return WatchTarget::Provider {
                provider: "youtube",
                channel: id.to_string(),
            };
        }
    }

    if let Some(rest) = host_path(stripped, "youtube.com") {
        // watch?v=<id>
        if let Some(q) = rest.strip_prefix("watch?").or_else(|| {
            rest.strip_prefix("watch/")
                .and_then(|r| r.split_once('?').map(|(_, q)| q))
        }) {
            if let Some(v) = q
                .split('&')
                .find_map(|kv| kv.strip_prefix("v="))
                .map(|v| v.split(['#', '&']).next().unwrap_or(""))
            {
                if !v.is_empty() {
                    return WatchTarget::Provider {
                        provider: "youtube",
                        channel: v.to_string(),
                    };
                }
            }
        }
        // live/<id>, @handle[/live], channel/UC…[/live]
        for prefix in ["live/", "channel/"] {
            if let Some(tail) = rest.strip_prefix(prefix) {
                let id = tail.split(['/', '?', '#']).next().unwrap_or("");
                if !id.is_empty() {
                    return WatchTarget::Provider {
                        provider: "youtube",
                        channel: id.to_string(),
                    };
                }
            }
        }
        if rest.starts_with('@') {
            let handle = rest.split(['/', '?', '#']).next().unwrap_or("");
            if handle.len() > 1 {
                return WatchTarget::Provider {
                    provider: "youtube",
                    channel: handle.to_string(),
                };
            }
        }
        return WatchTarget::Twitch;
    }

    if let Some(rest) = host_path(stripped, "tiktok.com") {
        if let Some(handle) = rest.strip_prefix('@') {
            let handle = handle.split(['/', '?', '#']).next().unwrap_or("");
            if !handle.is_empty() {
                return WatchTarget::Provider {
                    provider: "tiktok",
                    channel: handle.to_string(),
                };
            }
        }
        return WatchTarget::Twitch;
    }

    WatchTarget::Twitch
}

/// If `stripped` (scheme/www removed) is on `host`, return the path after the
/// first '/', with any leading slash removed. `kick.com` alone yields "".
fn host_path<'a>(stripped: &'a str, host: &str) -> Option<&'a str> {
    let rest = stripped.strip_prefix(host)?;
    if rest.is_empty() {
        return Some("");
    }
    rest.strip_prefix('/').or(match rest.as_bytes().first() {
        Some(b'?') | Some(b'#') => Some(""),
        _ => None, // e.g. "kick.community" must not match "kick.com"
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(p: &'static str, c: &str) -> WatchTarget {
        WatchTarget::Provider {
            provider: p,
            channel: c.to_string(),
        }
    }

    #[test]
    fn classify_table() {
        let cases: Vec<(&str, WatchTarget)> = vec![
            // Twitch and junk fall through
            ("https://twitch.tv/xqc", WatchTarget::Twitch),
            ("https://www.twitch.tv/xqc", WatchTarget::Twitch),
            ("", WatchTarget::Twitch),
            ("not a url", WatchTarget::Twitch),
            // Kick
            ("https://kick.com/xqc", provider("kick", "xqc")),
            ("https://www.kick.com/Trainwreckstv", provider("kick", "trainwreckstv")),
            ("http://kick.com/xqc?clip=x", provider("kick", "xqc")),
            ("https://kick.com/video/abc-123", WatchTarget::Twitch),
            ("https://kick.com/categories/slots", WatchTarget::Twitch),
            ("https://kick.com/", WatchTarget::Twitch),
            ("https://kick.community/xqc", WatchTarget::Twitch),
            // YouTube
            (
                "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
                provider("youtube", "dQw4w9WgXcQ"),
            ),
            (
                "https://youtube.com/watch?t=1&v=dQw4w9WgXcQ",
                provider("youtube", "dQw4w9WgXcQ"),
            ),
            ("https://youtu.be/dQw4w9WgXcQ", provider("youtube", "dQw4w9WgXcQ")),
            (
                "https://www.youtube.com/live/jfKfPfyJRdk?feature=shared",
                provider("youtube", "jfKfPfyJRdk"),
            ),
            ("https://youtube.com/@LinusTechTips/live", provider("youtube", "@LinusTechTips")),
            ("https://youtube.com/@LinusTechTips", provider("youtube", "@LinusTechTips")),
            (
                "https://www.youtube.com/channel/UCXuqSBlHAE6Xw-yeJA0Tunw/live",
                provider("youtube", "UCXuqSBlHAE6Xw-yeJA0Tunw"),
            ),
            ("https://youtube.com/feed/subscriptions", WatchTarget::Twitch),
            // TikTok
            ("https://www.tiktok.com/@pokimane/live", provider("tiktok", "pokimane")),
            ("https://tiktok.com/@pokimane", provider("tiktok", "pokimane")),
            ("https://tiktok.com/foryou", WatchTarget::Twitch),
        ];
        for (input, want) in cases {
            assert_eq!(classify(input), want, "input: {input}");
        }
    }
}
