//! Native Shazam recognition over HTTP.
//!
//! The fingerprint is built locally (algorithm.rs / signature_format.rs); here
//! we send only that signature to Shazam's public discovery endpoint and parse
//! the matched track. No API key or account: the request carries a fingerprint,
//! not audio. Shazam rate-limits per IP (HTTP 429), a non-issue for on-demand
//! use from one machine.

use std::sync::LazyLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rand::seq::IndexedRandom;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use super::links;
use super::services;
use super::signature_format::DecodedSignature;

/// One link of a song: the service it goes to and where. `search` marks a
/// search page for the song's words rather than the track's own page.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct Provider {
    pub name: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub search: bool,
}

pub fn is_false(b: &bool) -> bool {
    !*b
}

/// Where a song's ISRC came from: Shazam's own answer for the audio, or a
/// text search that named one track (links.rs). Shazam's outranks a text one.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IsrcFrom {
    Shazam,
    Text,
}

#[derive(Debug, Serialize, Clone)]
pub struct SongMatch {
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub album_art: Option<String>,
    pub shazam_url: Option<String>,
    /// A song.link page. Only songs kept before its API closed have one.
    pub song_link: Option<String>,
    /// Clickable per-service links: the track's own first, then search pages.
    pub providers: Vec<Provider>,
    /// Its key in Your songs once it is filed there (history.rs).
    pub key: Option<String>,
    /// Every provider is a search page. Not sent to the UI; each provider
    /// carries its own `search` flag.
    #[serde(skip)]
    pub links_are_searches: bool,
    /// The recording's ISRC, validated (links.rs `valid_isrc`).
    #[serde(skip)]
    pub isrc: Option<String>,
    #[serde(skip)]
    pub isrc_from: Option<IsrcFrom>,
    /// Apple Music's id for the song, digits only.
    #[serde(skip)]
    pub apple_id: Option<String>,
    /// Every link lookup answered cleanly, so the links need no follow-up.
    #[serde(skip)]
    pub links_complete: bool,
}

// A handful of real Android device user-agents; one is picked per request. The
// android tag endpoint expects a device-shaped client, and varying the string
// avoids looking like one connection hammering the service.
static USER_AGENTS: &[&str] = &[
    "Dalvik/2.1.0 (Linux; U; Android 13; Pixel 7 Build/TQ3A.230805.001)",
    "Dalvik/2.1.0 (Linux; U; Android 12; SM-G991B Build/SP1A.210812.016)",
    "Dalvik/2.1.0 (Linux; U; Android 11; Pixel 5 Build/RQ3A.210805.001.A1)",
    "Dalvik/2.1.0 (Linux; U; Android 13; SM-S918B Build/TP1A.220624.014)",
];

static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .expect("Failed to build Shazam HTTP client")
});

pub async fn recognize(signature: &DecodedSignature) -> Result<Option<SongMatch>, String> {
    let timestamp_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_millis() as u64;

    let samplems =
        (signature.number_samples as f32 / signature.sample_rate_hz as f32 * 1000.0) as u32;
    let uri = signature.encode_to_uri().map_err(|e| e.to_string())?;

    let body = serde_json::json!({
        "geolocation": { "altitude": 300, "latitude": 45, "longitude": 2 },
        "signature": { "samplems": samplems, "timestamp": timestamp_ms, "uri": uri },
        "timestamp": timestamp_ms,
        "timezone": "Europe/Paris",
    });

    // Each tag is addressed by a fresh request/device id pair.
    let uuid_upper = Uuid::new_v4().hyphenated().to_string().to_uppercase();
    let uuid_lower = Uuid::new_v4().hyphenated().to_string();
    let url = format!(
        "https://amp.shazam.com/discovery/v5/en/US/android/-/tag/{}/{}\
?sync=true&webv3=true&sampling=true&connected=&shazamapiversion=v3&sharehub=true&video=v3",
        uuid_upper, uuid_lower
    );

    let ua = USER_AGENTS
        .choose(&mut rand::rng())
        .copied()
        .unwrap_or(USER_AGENTS[0]);

    let resp = CLIENT
        .post(&url)
        .header("Content-Language", "en_US")
        .header(reqwest::header::USER_AGENT, ua)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Shazam request failed: {}", e))?;

    if resp.status().as_u16() == 429 {
        return Err("Shazam is rate-limiting this connection. Try again in a minute.".to_string());
    }
    if !resp.status().is_success() {
        return Err(format!("Shazam returned status {}", resp.status()));
    }

    let json: Value = resp
        .json()
        .await
        .map_err(|e| format!("Shazam response parse failed: {}", e))?;

    Ok(parse_track(&json))
}

// A no-match response has an empty `matches` array and no `track`, so the missing
// title naturally falls through to None.
fn parse_track(json: &Value) -> Option<SongMatch> {
    let track = json.get("track")?;

    let title = str_field(track, "title")?;
    let artist = str_field(track, "subtitle").unwrap_or_default();

    let album_art = track
        .pointer("/images/coverarthq")
        .and_then(Value::as_str)
        .or_else(|| track.pointer("/images/coverart").and_then(Value::as_str))
        .map(str::to_string);

    let shazam_url = str_field(track, "url")
        .or_else(|| {
            track
                .pointer("/share/href")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .filter(|u| is_real_link(u) && services::is_link_to("Shazam", u));

    // Album lives in the SONG section's metadata rows.
    let album = track
        .get("sections")
        .and_then(Value::as_array)
        .and_then(|sections| {
            sections
                .iter()
                .find(|s| s.get("type").and_then(Value::as_str) == Some("SONG"))
        })
        .and_then(|song| song.get("metadata"))
        .and_then(Value::as_array)
        .and_then(|md| {
            md.iter()
                .find(|m| m.get("title").and_then(Value::as_str) == Some("Album"))
        })
        .and_then(|m| m.get("text").and_then(Value::as_str))
        .map(str::to_string);

    let providers = extract_links(track);

    // The ISRC sits at the track's root; the Apple Music song id on the hub's
    // play action, which carries no link of its own.
    let isrc = str_field(track, "isrc").and_then(|code| links::valid_isrc(&code));
    let apple_id = track
        .pointer("/hub/actions")
        .and_then(Value::as_array)
        .and_then(|actions| {
            actions
                .iter()
                .find(|a| a.get("type").and_then(Value::as_str) == Some("applemusicplay"))
        })
        .and_then(|a| a.get("id").and_then(Value::as_str))
        .and_then(links::valid_apple_id)
        .or_else(|| {
            providers
                .iter()
                .find(|p| p.name == "Apple Music")
                .and_then(|p| links::apple_id_of(&p.url))
        });

    Some(SongMatch {
        title,
        artist,
        album,
        album_art,
        shazam_url,
        song_link: None,
        providers,
        key: None,
        links_are_searches: false,
        isrc_from: isrc.as_ref().map(|_| IsrcFrom::Shazam),
        isrc,
        apple_id,
        links_complete: false,
    })
}

// Pull clickable per-service links out of the track hub. Shazam mixes real
// streaming deep links in with store/subscribe/install junk (e.g. a Google Play
// "install Apple Music" link, or a music.apple.com/subscribe page) and search
// pages (YouTube Music's), so we keep only genuine track URLs on known
// platforms, one per platform. The link finder adds its own search pages.
fn extract_links(track: &Value) -> Vec<Provider> {
    let mut urls: Vec<String> = Vec::new();
    push_action_urls(track.pointer("/hub/actions"), &mut urls);
    if let Some(options) = track.pointer("/hub/options").and_then(Value::as_array) {
        for opt in options {
            push_action_urls(opt.get("actions"), &mut urls);
        }
    }
    if let Some(providers) = track.pointer("/hub/providers").and_then(Value::as_array) {
        for prov in providers {
            push_action_urls(prov.get("actions"), &mut urls);
        }
    }

    // One link per platform, first occurrence wins.
    let mut providers: Vec<Provider> = Vec::new();
    for url in urls {
        let Some(service) = services::service_of(&url) else {
            continue;
        };
        if providers.iter().any(|p| p.name == service.name) {
            continue;
        }
        providers.push(Provider {
            name: service.name.to_string(),
            url,
            search: false,
        });
    }
    providers
}

// Append the http(s) streaming-track URLs from a hub `actions` array.
fn push_action_urls(actions: Option<&Value>, out: &mut Vec<String>) {
    let Some(arr) = actions.and_then(Value::as_array) else {
        return;
    };
    for action in arr {
        if let Some(url) = action.get("uri").and_then(Value::as_str) {
            if is_streaming_track_url(url) {
                out.push(url.to_string());
            }
        }
    }
}

// Accept only real on-platform track links: a streaming service's own host
// (services.rs), and not the subscribe/upsell or app pages or the search
// pages Shazam also lists in the hub (store and install links are on other
// hosts and fall out there).
fn is_streaming_track_url(url: &str) -> bool {
    if !services::service_of(url).is_some_and(|s| s.streams) || services::is_search_page(url) {
        return false;
    }
    let path = url::Url::parse(url)
        .map(|u| u.path().to_ascii_lowercase())
        .unwrap_or_default();
    !path.contains("/subscribe") && !path.contains("/apps/")
}

fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

// Shazam returns an `unsupported.shazam.com` placeholder for tracks it can't
// deep-link; treat only real off-Shazam http(s) links as usable.
fn is_real_link(url: &str) -> bool {
    url.starts_with("http") && !url.contains("unsupported.shazam")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn names(providers: &[Provider]) -> Vec<&str> {
        providers.iter().map(|p| p.name.as_str()).collect()
    }

    #[test]
    fn the_hub_keeps_each_services_own_track_link_once() {
        let track = json!({
            "hub": {
                "actions": [
                    { "uri": "https://music.apple.com/us/album/nuevayol/1787022393?i=1787022572" },
                    { "uri": "https://music.apple.com/us/subscribe?itscg=1" },
                    { "uri": "https://itunes.apple.com/us/album/1787022393" }
                ],
                "options": [
                    { "actions": [
                        { "uri": "https://play.google.com/store/apps/details?id=com.apple.android.music" },
                        { "uri": "intent://music.apple.com/us/album/1" }
                    ] }
                ],
                "providers": [
                    { "actions": [
                        { "uri": "spotify:search:nuevayol" },
                        { "uri": "https://x.example/?u=https://open.spotify.com/track/1" },
                        { "uri": "https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT" },
                        { "uri": "https://open.spotify.com/track/second" }
                    ] },
                    { "actions": [
                        { "uri": "https://www.youtube.com/watch?v=6U4n4bkOGs4" },
                        { "uri": "https://music.youtube.com/watch?v=6U4n4bkOGs4" },
                        { "uri": "https://www.amazon.com/dp/B0DQ8XYZ" },
                        { "uri": "https://www.shazam.com/track/1" }
                    ] }
                ]
            }
        });
        let providers = extract_links(&track);
        assert_eq!(
            names(&providers),
            vec!["Apple Music", "Spotify", "YouTube", "YouTube Music"]
        );
        for p in &providers {
            assert!(
                services::is_link_to(&p.name, &p.url),
                "{} -> {}",
                p.name,
                p.url
            );
            assert!(!p.search, "{}", p.url);
        }
        assert_eq!(
            providers[1].url,
            "https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT"
        );
    }

    #[test]
    fn search_links_go_to_each_services_own_search_with_the_words_encoded() {
        let found: Vec<Provider> = links::DISPLAY
            .iter()
            .filter_map(|name| links::search_link(name, "AC/DC & Queen", "Who's #1?"))
            .collect();
        assert_eq!(
            names(&found),
            vec![
                "Spotify",
                "Apple Music",
                "YouTube Music",
                "YouTube",
                "Deezer",
                "Tidal",
                "SoundCloud"
            ],
            "every service on the list but Beatport, in its order"
        );
        for p in &found {
            assert!(
                services::is_link_to(&p.name, &p.url),
                "{} -> {}",
                p.name,
                p.url
            );
            assert!(p.search && services::is_search_page(&p.url), "{}", p.url);
        }
        let q = "AC%2FDC%20%26%20Queen%20Who%27s%20%231%3F";
        let url_of = |name: &str| found.iter().find(|p| p.name == name).unwrap().url.clone();
        assert_eq!(url_of("Spotify"), format!("https://open.spotify.com/search/{q}"));
        assert_eq!(
            url_of("YouTube Music"),
            format!("https://music.youtube.com/search?q={q}")
        );
        assert_eq!(
            url_of("Apple Music"),
            format!("https://music.apple.com/us/search?term={q}")
        );
        assert_eq!(
            url_of("YouTube"),
            format!("https://www.youtube.com/results?search_query={q}")
        );
        assert_eq!(url_of("Deezer"), format!("https://www.deezer.com/search/{q}"));
        assert_eq!(url_of("Tidal"), format!("https://tidal.com/search?q={q}"));
        assert_eq!(url_of("SoundCloud"), format!("https://soundcloud.com/search?q={q}"));
        let spotify = url::Url::parse(&url_of("Spotify")).unwrap();
        assert_eq!(
            spotify.path_segments().unwrap().count(),
            2,
            "the words stay one path part"
        );
        let youtube = url::Url::parse(&url_of("YouTube Music")).unwrap();
        assert_eq!(
            youtube.query_pairs().collect::<Vec<_>>(),
            vec![("q".into(), "AC/DC & Queen Who's #1?".into())]
        );
        assert!(links::search_link("Beatport", "A", "B").is_none());
    }

    #[test]
    fn a_tag_answer_carries_its_isrc_and_apple_song_id() {
        let answer: Value = serde_json::from_str(include_str!("fixtures/shazam-track.json")).unwrap();
        let song = parse_track(&answer).unwrap();
        assert_eq!(song.title, "Tu Nombre");
        assert_eq!(song.artist, "Antonio Machín");
        assert_eq!(song.album.as_deref(), Some("Antonio Machin"));
        assert_eq!(song.isrc.as_deref(), Some("QM6MZ1422245"));
        assert_eq!(song.isrc_from, Some(IsrcFrom::Shazam));
        assert_eq!(song.apple_id.as_deref(), Some("950742158"));
        assert!(
            song.providers.is_empty(),
            "the hub's only http link is a YouTube Music search page: {:?}",
            song.providers
        );
        assert!(!song.links_complete);

        // A tag answer without an ISRC, or with a malformed one, has none.
        let mut bare = answer.clone();
        bare["track"]["isrc"] = json!("not-an-isrc");
        let song = parse_track(&bare).unwrap();
        assert_eq!((song.isrc, song.isrc_from), (None, None));
        bare["track"].as_object_mut().unwrap().remove("isrc");
        assert_eq!(parse_track(&bare).unwrap().isrc, None);
    }

    #[test]
    fn a_link_without_a_search_flag_reads_as_the_tracks_own() {
        let p: Provider =
            serde_json::from_str(r#"{"name":"Spotify","url":"https://open.spotify.com/track/1"}"#)
                .unwrap();
        assert!(!p.search);
        assert_eq!(
            serde_json::to_string(&p).unwrap(),
            r#"{"name":"Spotify","url":"https://open.spotify.com/track/1"}"#,
            "a direct link is written as before"
        );
        let s = Provider {
            search: true,
            ..p
        };
        assert!(serde_json::to_string(&s).unwrap().ends_with(r#","search":true}"#));
    }

    #[test]
    fn the_shazam_link_is_kept_only_when_it_is_shazams_own_page() {
        let parsed = |url: &str| {
            parse_track(&json!({ "track": { "title": "One", "subtitle": "A", "url": url } }))
                .unwrap()
                .shazam_url
        };
        assert_eq!(
            parsed("https://www.shazam.com/track/726543210/nuevayol").as_deref(),
            Some("https://www.shazam.com/track/726543210/nuevayol")
        );
        assert_eq!(parsed("https://unsupported.shazam.com/x"), None);
        assert_eq!(parsed("https://x.example/?shazam.com"), None);
        assert_eq!(parsed("shazam://track/1"), None);
    }
}
