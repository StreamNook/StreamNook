//! The one rule for a link that comes from outside the app and is drawn as a
//! picture: it has to be a public web address. Channel panels, link previews
//! and the profile snapshot all pass their image links through here, so a
//! remote value can never point the page at this machine (`asset.localhost`,
//! `localhost`, a loopback or LAN address) or carry a login in the link.
//!
//! The site keeps the same rule in `functions/utils/webLink.ts`; both sides
//! share one table of cases.

use url::{Host, Url};

/// Twitch's CDN, the only place a member's avatar may come from.
const TWITCH_AVATAR_HOST: &str = "static-cdn.jtvnw.net";

/// Names that never leave this machine or its network.
fn is_local_name(host: &str) -> bool {
    host == "localhost" || host.ends_with(".localhost") || host.ends_with(".local")
}

/// The link parsed, when it is a public web address (see [`public_web_url`]).
fn parse_public(link: &str, https_only: bool) -> Option<Url> {
    // A browser rewrites these before it fetches (a backslash becomes a slash,
    // a tab or newline vanishes), so the host it reaches can differ from the
    // host read here. The rest of the app also puts links inside HTML and CSS.
    // None of them belong in a real image link.
    let unsafe_char =
        |c: char| c.is_control() || c.is_whitespace() || matches!(c, '"' | '<' | '>' | '`' | '\\');
    if link.is_empty() || link.chars().any(unsafe_char) {
        return None;
    }
    let url = Url::parse(link).ok()?;
    match url.scheme() {
        "https" => {}
        "http" if !https_only => {}
        _ => return None,
    }
    if !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    // `url` turns `0x7f000001`, `2130706433`, `127.1` and the like into an IPv4
    // host, so a numeric address in any spelling is not a domain here.
    let Some(Host::Domain(host)) = url.host() else {
        return None;
    };
    // A trailing dot is an empty last label, so `example.com.` is refused too.
    if !host.contains('.') || host.split('.').any(str::is_empty) || is_local_name(host) {
        return None;
    }
    Some(url)
}

/// The link, trimmed, when it is a public web address: scheme https (or http
/// when `https_only` is false), no userinfo, a DNS host containing a dot, not
/// `localhost`, `*.localhost` or `*.local`, never an IP literal in any spelling,
/// and none of the characters a browser would rewrite.
pub fn public_web_url(s: &str, https_only: bool) -> Option<String> {
    let link = s.trim();
    parse_public(link, https_only).map(|_| link.to_string())
}

/// An https avatar on Twitch's CDN (`static-cdn.jtvnw.net`), else None.
pub fn twitch_avatar_url(s: &str) -> Option<String> {
    let link = s.trim();
    let url = parse_public(link, true)?;
    (url.host_str() == Some(TWITCH_AVATAR_HOST) && url.port().is_none()).then(|| link.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_and_private_hosts_are_refused() {
        for link in [
            "http://asset.localhost/C%3A%5CUsers%5Cx%5Ca.png",
            "https://asset.localhost/C%3A%5CUsers%5Cx%5Ca.png",
            "http://localhost/x.png",
            "http://localhost:1420/x.png",
            "http://localhost./x.png",
            "https://LOCALHOST/x.png",
            "http://app.localhost/x.png",
            "http://printer.local/x.png",
            "http://127.0.0.1/x.png",
            "http://127.0.0.1:23456/cache/token/emotes/x.png",
            "http://[::1]/x.png",
            "http://[::ffff:127.0.0.1]/x.png",
            "http://0x7f000001/x.png",
            "http://2130706433/x.png",
            "http://017700000001/x.png",
            "http://127.1/x.png",
            "http://0/x.png",
            "http://192.168.1.1/x.png",
            "http://10.0.0.1/x.png",
            "https://intranet/x.png",
            "https://.example.com/x.png",
            "https://a..example.com/x.png",
            "https://example.com./x.png",
            "https://cdn.7tv.app./x.png",
        ] {
            assert_eq!(public_web_url(link, false), None, "{link}");
            assert_eq!(public_web_url(link, true), None, "{link} (https only)");
        }
    }

    #[test]
    fn numeric_host_spellings_are_read_as_addresses_by_the_url_crate() {
        for link in [
            "http://0x7f000001/",
            "http://2130706433/",
            "http://017700000001/",
            "http://127.1/",
        ] {
            let url = Url::parse(link).expect("parses");
            assert!(
                matches!(url.host(), Some(Host::Ipv4(_))),
                "{link} is not an IPv4 host"
            );
        }
    }

    #[test]
    fn userinfo_and_other_schemes_are_refused() {
        for link in [
            "https://user:pw@example.com/x.png",
            "https://user@example.com/x.png",
            "https://:pw@example.com/x.png",
            "file:///C:/Users/x/a.png",
            "data:image/png;base64,AAAA",
            "javascript:alert(1)",
            "asset://localhost/C%3A%5Cx.png",
            "ftp://example.com/x.png",
            "blob:https://example.com/9f1c",
            "//cdn.7tv.app/x.webp",
            "/x.png",
            "x.png",
            "",
            "   ",
        ] {
            assert_eq!(public_web_url(link, false), None, "{link}");
        }
    }

    #[test]
    fn characters_a_browser_would_rewrite_are_refused() {
        for link in [
            "https://example.com\\@127.0.0.1/x.png",
            "https://example.com/a\\b.png",
            "https://example.com/a\"b.png",
            "https://example.com/a<b>.png",
            "https://example.com/a`b.png",
            "https://example.com/a b.png",
            "https://exa\tmple.com/x.png",
            "https://example.com/x.png\nhttp://127.0.0.1/",
            "https://example.com/\u{7f}x.png",
            "https://example.com/\u{85}x.png",
        ] {
            assert_eq!(public_web_url(link, false), None, "{link:?}");
        }
    }

    #[test]
    fn public_links_are_kept() {
        for link in [
            "https://cdn.7tv.app/emote/01F6MQ33FG000FFJ97ZB8MWV52/4x.webp",
            "https://static-cdn.jtvnw.net/jtv_user_pictures/x-profile_image-300x300.png",
            "https://example.com/x.png",
            "https://example.com:8443/x.png",
            "https://img.example.co.uk/a%20b.png?w=64&h=64",
            "https://xn--e1afmkfd.xn--p1ai/x.png",
            "https://static-cdn.jtvnw.net/previews-ttv/live_user_x-{width}x{height}.jpg",
            "https://upload.wikimedia.org/wikipedia/commons/a/ab/Foo_(bar).png",
            // The badge and paint links a profile snapshot carries.
            "https://static-cdn.jtvnw.net/badges/v1/0d2d0ac5-5ec3-4c3e-a2b4-3eaa5d6f7a3b/3",
            "https://cdn.frankerfacez.com/badge/3/4/rounded",
            "https://cdn.betterttv.net/badges/pro/97c7dfb8-d2a5-4ab0-9e42-e9e14630da5b.webp",
            "https://cdn.7tv.app/paint/01GOLDENPAINT0000000000000/layer/01GOLDENLAYERIMAGE00000000/1x_static.webp",
        ] {
            assert_eq!(public_web_url(link, true).as_deref(), Some(link), "{link}");
            assert_eq!(public_web_url(link, false).as_deref(), Some(link), "{link} (http allowed)");
        }
    }

    #[test]
    fn http_is_kept_only_when_https_is_not_required() {
        let link = "http://example.com/x.png";
        assert_eq!(public_web_url(link, false).as_deref(), Some(link));
        assert_eq!(public_web_url(link, true), None);
    }

    #[test]
    fn the_link_comes_back_trimmed() {
        assert_eq!(
            public_web_url("  https://cdn.7tv.app/a.webp \n", true).as_deref(),
            Some("https://cdn.7tv.app/a.webp")
        );
    }

    #[test]
    fn an_avatar_comes_only_from_twitchs_cdn() {
        let ok = "https://static-cdn.jtvnw.net/jtv_user_pictures/golden-profile_image-300x300.png";
        assert_eq!(twitch_avatar_url(ok).as_deref(), Some(ok));
        assert_eq!(twitch_avatar_url(&format!("  {ok}  ")).as_deref(), Some(ok));
        for link in [
            "http://static-cdn.jtvnw.net/jtv_user_pictures/x.png",
            "https://static-cdn.jtvnw.net.evil.com/x.png",
            "https://evil.com/static-cdn.jtvnw.net/x.png",
            "https://static-cdn.jtvnw.net:8443/x.png",
            "https://static-cdn.jtvnw.net./x.png",
            "https://user@static-cdn.jtvnw.net/x.png",
            "https://cdn.7tv.app/a.webp",
            "https://example.com/x.png",
            "http://asset.localhost/C%3A%5Cx.png",
            "http://127.0.0.1/x.png",
            "",
        ] {
            assert_eq!(twitch_avatar_url(link), None, "{link}");
        }
    }
}
