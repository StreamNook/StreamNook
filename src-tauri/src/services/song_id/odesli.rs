//! Odesli (song.link) enrichment.
//!
//! Shazam names the track and gives us one platform link (usually Apple Music),
//! but its in-band "open in" links are spotty (it returns an `unsupported.shazam`
//! placeholder for tracks it can't deep-link). Feeding any one real platform URL
//! to Odesli returns a clean set of links for every service plus the song.link
//! aggregator page, which is what the chat card shows. Keyless, rate-limited per
//! IP (fine for on-demand use); any failure just leaves the Shazam-only links.

use std::sync::LazyLock;
use std::time::Duration;

use serde_json::Value;

use super::services;
use super::shazam::Provider;

pub struct OdesliResult {
    pub page_url: Option<String>,
    pub providers: Vec<Provider>,
    pub thumbnail: Option<String>,
}

// Platforms we surface, in display order. Keys match Odesli's `linksByPlatform`.
const PLATFORMS: &[(&str, &str)] = &[
    ("spotify", "Spotify"),
    ("appleMusic", "Apple Music"),
    ("youtubeMusic", "YouTube Music"),
    ("youtube", "YouTube"),
    ("soundcloud", "SoundCloud"),
    ("tidal", "Tidal"),
    ("deezer", "Deezer"),
    ("amazonMusic", "Amazon Music"),
    ("pandora", "Pandora"),
];

static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent("StreamNook")
        .build()
        .expect("Failed to build Odesli HTTP client")
});

pub async fn enrich(seed_url: &str) -> Option<OdesliResult> {
    let api = format!(
        "https://api.song.link/v1-alpha.1/links?url={}&userCountry=US&songIfSingle=true",
        urlencoding::encode(seed_url)
    );

    let resp = CLIENT.get(&api).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let json: Value = resp.json().await.ok()?;
    parse(&json)
}

/// Odesli's answer as the song keeps it. A link is kept only when it goes to
/// its platform's own host (services.rs), and the page only when it is
/// song.link's, so a wrong or odd answer never puts a logo on a link that
/// goes somewhere else.
fn parse(json: &Value) -> Option<OdesliResult> {
    let page_url = json
        .get("pageUrl")
        .and_then(Value::as_str)
        .filter(|url| services::is_link_to("song.link", url))
        .map(str::to_string);

    let mut providers: Vec<Provider> = Vec::new();
    for (key, label) in PLATFORMS {
        if let Some(url) = json
            .pointer(&format!("/linksByPlatform/{}/url", key))
            .and_then(Value::as_str)
            .filter(|url| services::is_link_to(label, url))
        {
            providers.push(Provider {
                name: label.to_string(),
                url: url.to_string(),
            });
        }
    }

    // First entity with a thumbnail (used only if Shazam gave us no cover art).
    let thumbnail = json
        .get("entitiesByUniqueId")
        .and_then(Value::as_object)
        .and_then(|entities| {
            entities
                .values()
                .find_map(|e| e.get("thumbnailUrl").and_then(Value::as_str))
        })
        .map(str::to_string);

    if page_url.is_none() && providers.is_empty() {
        return None;
    }

    Some(OdesliResult {
        page_url,
        providers,
        thumbnail,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn every_platform_listed_is_a_service() {
        for (_, label) in PLATFORMS {
            assert!(services::is_service_name(label), "{label}");
        }
    }

    #[test]
    fn links_are_kept_only_on_their_platforms_host() {
        // The shape of a real answer, with two links that go elsewhere.
        let answer = json!({
            "pageUrl": "https://song.link/us/i/1787022572",
            "linksByPlatform": {
                "spotify": { "url": "https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT" },
                "appleMusic": { "url": "https://geo.music.apple.com/us/album/_/1787022393?i=1787022572&mt=1&app=music" },
                "youtubeMusic": { "url": "https://www.youtube.com/watch?v=6U4n4bkOGs4" },
                "youtube": { "url": "https://www.youtube.com/watch?v=6U4n4bkOGs4" },
                "soundcloud": { "url": "https://soundcloud.com/badbunnypr/nuevayol" },
                "tidal": { "url": "https://listen.tidal.com/track/412345678" },
                "deezer": { "url": "https://www.deezer.com/track/3112345678" },
                "amazonMusic": { "url": "https://x.example/?u=music.amazon.com" },
                "pandora": { "url": "https://www.pandora.com/artist/bad-bunny/nuevayol/TRabc" },
                "napster": { "url": "https://play.napster.com/track/1" }
            },
            "entitiesByUniqueId": { "A": { "thumbnailUrl": "https://i.scdn.co/image/ab67" } }
        });
        let found = parse(&answer).unwrap();
        let names: Vec<&str> = found.providers.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "Spotify",
                "Apple Music",
                "YouTube",
                "SoundCloud",
                "Tidal",
                "Deezer",
                "Pandora"
            ]
        );
        for p in &found.providers {
            assert!(
                services::is_link_to(&p.name, &p.url),
                "{} -> {}",
                p.name,
                p.url
            );
        }
        assert_eq!(
            found.page_url.as_deref(),
            Some("https://song.link/us/i/1787022572")
        );
        assert_eq!(
            found.thumbnail.as_deref(),
            Some("https://i.scdn.co/image/ab67")
        );
    }

    #[test]
    fn a_page_that_is_not_song_links_is_dropped() {
        let found = parse(&json!({
            "pageUrl": "https://song.link.x.example/s/1",
            "linksByPlatform": { "spotify": { "url": "https://open.spotify.com/track/1" } }
        }))
        .unwrap();
        assert_eq!(found.page_url, None);
        assert_eq!(found.providers.len(), 1);
        assert!(parse(&json!({ "pageUrl": "https://x.example/" })).is_none());
    }
}
