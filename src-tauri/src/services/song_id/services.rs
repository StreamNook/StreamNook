//! The music services a song's links go to, told apart by each link's host.
//!
//! A link is named for a service (and drawn with its logo, in the chat card
//! and Your songs) only when its host is one of that service's own domains or
//! a subdomain of one, never because the name appears somewhere in the
//! address: `https://x.example/?u=open.spotify.com` is no Spotify link, and
//! neither is `https://open.spotify.com.x.example/`. Shazam's hub (shazam.rs),
//! the link finder (links.rs) and the stored history (history.rs) all ask
//! here. The page's logos are keyed by these names
//! (src/components/songs/serviceMarks.ts, held to this table by its test).

use url::Url;

pub struct Service {
    /// The name a link carries, as the page shows it.
    pub name: &'static str,
    /// Its own domains; a subdomain of one counts as the service.
    hosts: &'static [&'static str],
    /// A place to play the track (Shazam's own page and song.link are not).
    pub streams: bool,
}

/// Every service a song's link can name.
pub const SERVICES: &[Service] = &[
    Service {
        name: "Spotify",
        hosts: &["open.spotify.com", "play.spotify.com"],
        streams: true,
    },
    Service {
        name: "Apple Music",
        hosts: &["music.apple.com"],
        streams: true,
    },
    Service {
        name: "YouTube Music",
        hosts: &["music.youtube.com"],
        streams: true,
    },
    Service {
        name: "YouTube",
        hosts: &["youtube.com", "youtu.be"],
        streams: true,
    },
    Service {
        name: "SoundCloud",
        hosts: &["soundcloud.com"],
        streams: true,
    },
    Service {
        name: "Tidal",
        hosts: &["tidal.com"],
        streams: true,
    },
    Service {
        name: "Deezer",
        hosts: &["deezer.com"],
        streams: true,
    },
    Service {
        name: "Amazon Music",
        hosts: &["music.amazon.com"],
        streams: true,
    },
    Service {
        name: "Pandora",
        hosts: &["pandora.com"],
        streams: true,
    },
    Service {
        name: "Beatport",
        hosts: &["beatport.com"],
        streams: true,
    },
    Service {
        name: "Shazam",
        hosts: &["shazam.com"],
        streams: false,
    },
    Service {
        name: "song.link",
        hosts: &["song.link", "album.link", "odesli.co"],
        streams: false,
    },
];

/// The service `url` goes to: an http(s) address with no sign-in part and no
/// port, whose host is one of the service's domains or under one. The most
/// specific domain wins, so music.youtube.com is YouTube Music, not YouTube.
pub fn service_of(url: &str) -> Option<&'static Service> {
    let parsed = Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "https" | "http")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.port().is_some()
    {
        return None;
    }
    let host = parsed.host_str()?;
    SERVICES
        .iter()
        .flat_map(|service| service.hosts.iter().map(move |h| (service, *h)))
        .filter(|(_, h)| host == *h || host.strip_suffix(h).is_some_and(|rest| rest.ends_with('.')))
        .max_by_key(|(_, h)| h.len())
        .map(|(service, _)| service)
}

/// Whether `url` goes to the service called `name`.
pub fn is_link_to(name: &str, url: &str) -> bool {
    service_of(url).is_some_and(|s| s.name == name)
}

/// Whether `url` is one of a service's search pages rather than a track: the
/// shapes the link finder writes (links.rs `search_link`) and the ones
/// Shazam's hub and older builds stored. A search page never counts as the
/// track's own link.
pub fn is_search_page(url: &str) -> bool {
    let Some(service) = service_of(url) else {
        return false;
    };
    let Ok(parsed) = Url::parse(url) else {
        return false;
    };
    let path = parsed.path().to_ascii_lowercase();
    // The first two path parts: a country part comes first on Apple Music
    // (`/us/search`) and sometimes on Deezer (`/us/search/<words>`).
    let mut parts = path.split('/').filter(|p| !p.is_empty());
    let first = parts.next().unwrap_or("");
    let second = parts.next().unwrap_or("");
    match service.name {
        "Apple Music" | "Deezer" => first == "search" || second == "search",
        "YouTube" => first == "results",
        "Tidal" => first == "search" || (first == "browse" && second == "search"),
        "Spotify" | "YouTube Music" | "SoundCloud" | "Beatport" => first == "search",
        _ => false,
    }
}

/// Whether `name` is one of the services (tests hold other tables to this one).
#[cfg(test)]
pub fn is_service_name(name: &str) -> bool {
    SERVICES.iter().any(|s| s.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(url: &str) -> Option<&'static str> {
        service_of(url).map(|s| s.name)
    }

    #[test]
    fn real_links_name_their_service() {
        // The shapes Shazam's hub and Odesli's answer carry.
        for (url, name) in [
            (
                "https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT",
                "Spotify",
            ),
            (
                "https://open.spotify.com/search/Bad%20Bunny%20NUEVAYoL",
                "Spotify",
            ),
            (
                "https://music.apple.com/us/album/nuevayol/1787022393?i=1787022572",
                "Apple Music",
            ),
            (
                "https://geo.music.apple.com/us/album/_/1787022393?i=1787022572&mt=1&app=music",
                "Apple Music",
            ),
            (
                "https://music.youtube.com/watch?v=6U4n4bkOGs4",
                "YouTube Music",
            ),
            (
                "https://music.youtube.com/search?q=Bad%20Bunny",
                "YouTube Music",
            ),
            ("https://www.youtube.com/watch?v=6U4n4bkOGs4", "YouTube"),
            ("https://youtube.com/watch?v=6U4n4bkOGs4", "YouTube"),
            ("https://m.youtube.com/watch?v=6U4n4bkOGs4", "YouTube"),
            ("https://youtu.be/6U4n4bkOGs4", "YouTube"),
            ("https://soundcloud.com/badbunnypr/nuevayol", "SoundCloud"),
            ("https://on.soundcloud.com/abc123", "SoundCloud"),
            ("https://listen.tidal.com/track/412345678", "Tidal"),
            ("https://tidal.com/browse/track/412345678", "Tidal"),
            ("https://www.deezer.com/track/3112345678", "Deezer"),
            ("https://www.beatport.com/track/-/14957156", "Beatport"),
            (
                "https://music.amazon.com/albums/B0DQ8XYZ?trackAsin=B0DQ8ABC",
                "Amazon Music",
            ),
            (
                "https://www.pandora.com/artist/bad-bunny/nuevayol/TRabc",
                "Pandora",
            ),
            ("https://www.shazam.com/track/726543210/nuevayol", "Shazam"),
            ("https://song.link/s/4cOdK2wGLETKBW3PvgPWqT", "song.link"),
            ("https://album.link/i/1787022393", "song.link"),
            ("https://odesli.co/abc", "song.link"),
            ("HTTPS://OPEN.SPOTIFY.COM/track/x", "Spotify"),
            ("http://www.deezer.com/track/1", "Deezer"),
        ] {
            assert_eq!(named(url), Some(name), "{url}");
        }
    }

    #[test]
    fn a_name_anywhere_but_the_host_is_no_service() {
        for url in [
            "https://x.example/?u=https://open.spotify.com/track/1",
            "https://x.example/open.spotify.com/track/1",
            "https://open.spotify.com.x.example/track/1",
            "https://notspotify.com/track/1",
            "https://xmusic.apple.com/us/album/1",
            "https://open.spotify.com@x.example/track/1",
            "https://user:pass@open.spotify.com/track/1",
            "https://open.spotify.com:8443/track/1",
            "https://www.amazon.com/dp/B0DQ8XYZ",
            "https://itunes.apple.com/us/album/1",
            "https://play.google.com/store/apps/details?id=com.apple.android.music",
            "spotify:track:4cOdK2wGLETKBW3PvgPWqT",
            "javascript:alert('open.spotify.com')",
            "ftp://open.spotify.com/track/1",
            "https://open.spotify.com./track/1",
            "https://xn--pple-43d.com/music.apple.com",
            "",
            "open.spotify.com/track/1",
        ] {
            assert_eq!(named(url), None, "{url}");
        }
    }

    #[test]
    fn a_link_counts_only_for_its_own_service() {
        assert!(is_link_to("Spotify", "https://open.spotify.com/track/1"));
        assert!(!is_link_to(
            "Apple Music",
            "https://open.spotify.com/track/1"
        ));
        assert!(!is_link_to(
            "YouTube",
            "https://music.youtube.com/watch?v=1"
        ));
        assert!(is_service_name("Tidal"));
        assert!(!is_service_name("Listen"));
    }

    #[test]
    fn search_pages_are_told_from_track_links() {
        for url in [
            "https://open.spotify.com/search/Sade%20Kiss%20of%20Life",
            "https://music.apple.com/us/search?term=Sade",
            "https://music.youtube.com/search?q=Tu+Nombre&feature=shazam",
            "https://www.youtube.com/results?search_query=Sade",
            "https://www.deezer.com/search/Sade",
            "https://www.deezer.com/us/search/Sade",
            "https://tidal.com/search?q=Sade",
            "https://soundcloud.com/search?q=Sade",
        ] {
            assert!(is_search_page(url), "{url}");
        }
        for url in [
            "https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT",
            "https://music.apple.com/us/song/950742158",
            "https://music.apple.com/us/album/search/1?i=2",
            "https://music.youtube.com/watch?v=4MOzk7-6j-I",
            "https://www.youtube.com/watch?v=4MOzk7-6j-I",
            "https://www.deezer.com/track/1030607572",
            "https://tidal.com/browse/track/1",
            "https://www.beatport.com/track/-/14957156",
            "https://x.example/search?q=1",
        ] {
            assert!(!is_search_page(url), "{url}");
        }
    }

    #[test]
    fn every_service_has_a_name_and_its_own_domains() {
        let mut names = std::collections::HashSet::new();
        for s in SERVICES {
            assert!(names.insert(s.name), "{} twice", s.name);
            assert!(!s.hosts.is_empty(), "{} has no domain", s.name);
            for h in s.hosts {
                assert_eq!(*h, h.to_ascii_lowercase());
                assert!(
                    h.contains('.') && !h.starts_with('.') && !h.ends_with('.'),
                    "{h}"
                );
            }
        }
    }
}
