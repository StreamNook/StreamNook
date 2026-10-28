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

use super::signature_format::DecodedSignature;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Provider {
    pub name: String,
    pub url: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct SongMatch {
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub album_art: Option<String>,
    pub shazam_url: Option<String>,
    /// song.link (Odesli) aggregator page, when enrichment succeeds.
    pub song_link: Option<String>,
    /// Clickable per-service links (from Odesli, or Shazam's hub as a fallback).
    pub providers: Vec<Provider>,
    /// A real platform URL used to seed Odesli enrichment. Not sent to the UI.
    #[serde(skip)]
    pub seed_url: Option<String>,
    /// Its key in Your songs once it is filed there (history.rs).
    pub key: Option<String>,
    /// `providers` are search pages (`search_links`), not the track's own
    /// links. Not sent to the UI.
    #[serde(skip)]
    pub links_are_searches: bool,
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
        .filter(|u| is_real_link(u));

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

    let (providers, seed_url) = extract_links(track);

    Some(SongMatch {
        title,
        artist,
        album,
        album_art,
        shazam_url,
        song_link: None,
        providers,
        seed_url,
        key: None,
        links_are_searches: false,
    })
}

// Pull clickable per-service links out of the track hub. Shazam mixes real
// streaming deep links in with store/subscribe/install junk (e.g. a Google Play
// "install Apple Music" link, or a music.apple.com/subscribe page), so we keep
// only genuine track URLs on known platforms. Returns the deduped link list plus
// the best one to seed Odesli with.
fn extract_links(track: &Value) -> (Vec<Provider>, Option<String>) {
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
        let name = platform_label(&url).to_string();
        if providers.iter().any(|p| p.name == name) {
            continue;
        }
        providers.push(Provider { name, url });
    }

    let seed = pick_seed(&providers);
    (providers, seed)
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

// Accept only real on-platform track links; reject store pages, app-install
// links, and subscribe/upsell pages that Shazam also lists in the hub.
fn is_streaming_track_url(url: &str) -> bool {
    let u = url.to_ascii_lowercase();
    if !u.starts_with("http") {
        return false;
    }
    if u.contains("play.google.com")
        || u.contains("itunes.apple.com")
        || u.contains("/apps/details")
        || u.contains("/subscribe")
        || u.contains("unsupported.shazam")
    {
        return false;
    }
    const HOSTS: &[&str] = &[
        "music.apple.com",
        "open.spotify.com",
        "music.youtube.com",
        "youtube.com",
        "youtu.be",
        "soundcloud.com",
        "tidal.com",
        "deezer.com",
        "music.amazon.com",
        "pandora.com",
    ];
    HOSTS.iter().any(|host| u.contains(host))
}

fn platform_label(url: &str) -> &'static str {
    let u = url.to_ascii_lowercase();
    if u.contains("music.apple.com") {
        "Apple Music"
    } else if u.contains("spotify.com") {
        "Spotify"
    } else if u.contains("music.youtube.com") {
        "YouTube Music"
    } else if u.contains("youtube.com") || u.contains("youtu.be") {
        "YouTube"
    } else if u.contains("soundcloud.com") {
        "SoundCloud"
    } else if u.contains("tidal.com") {
        "Tidal"
    } else if u.contains("deezer.com") {
        "Deezer"
    } else if u.contains("amazon.com") {
        "Amazon Music"
    } else if u.contains("pandora.com") {
        "Pandora"
    } else {
        "Listen"
    }
}

// Odesli resolves every platform from one seed; prefer the links Shazam carries
// most reliably as real http track URLs.
fn pick_seed(providers: &[Provider]) -> Option<String> {
    for preferred in ["Apple Music", "Spotify", "YouTube Music"] {
        if let Some(p) = providers.iter().find(|p| p.name == preferred) {
            return Some(p.url.clone());
        }
    }
    providers.first().map(|p| p.url.clone())
}

// Last-resort links when Shazam exposed no real platform URL and Odesli had
// nothing to resolve: per-service search pages, which land on the track for
// anything findable. Direct links from Odesli always take precedence.
pub fn search_links(artist: &str, title: &str) -> Vec<Provider> {
    let query = format!("{} {}", artist, title);
    let q = urlencoding::encode(&query);
    vec![
        Provider {
            name: "Spotify".to_string(),
            url: format!("https://open.spotify.com/search/{}", q),
        },
        Provider {
            name: "YouTube Music".to_string(),
            url: format!("https://music.youtube.com/search?q={}", q),
        },
        Provider {
            name: "Apple Music".to_string(),
            url: format!("https://music.apple.com/us/search?term={}", q),
        },
    ]
}

fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

// Shazam returns an `unsupported.shazam.com` placeholder for tracks it can't
// deep-link; treat only real off-Shazam http(s) links as usable.
fn is_real_link(url: &str) -> bool {
    url.starts_with("http") && !url.contains("unsupported.shazam")
}
