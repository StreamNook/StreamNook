//! Inline link previews for chat.
//!
//! Given a URL pasted in chat, returns a small metadata card payload the UI can
//! render inline. Three shapes:
//!   - YouTube: keyless oEmbed for title/author + the always-present hqdefault
//!     thumbnail derived from the video id.
//!   - Direct image: the URL itself is the image.
//!   - Generic: fetch the page and read its OpenGraph / Twitter-card / <title>
//!     meta tags via `scraper`.
//!
//! The fetch runs here (not in the webview) so it sidesteps CORS, and so the UI
//! never has to touch arbitrary chat-pasted URLs directly. Answers go through
//! `services::link_preview_cache` (bounded, one fetch per URL at a time): many
//! chatters paste the same link, and several chat windows share this backend.
//! Only trusted hosts auto-expand (the allowlist lives there too), so this never
//! auto-fetches the whole firehose.

use crate::services::twitch_service::TwitchService;
use log::debug;
use regex::Regex;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;
use std::time::Duration;

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
pub struct LinkPreview {
    pub url: String,
    /// "youtube" | "image" | "generic" | "tweet" | "discord" | "steam" | "spotify" | "reddit"
    pub kind: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub image: Option<String>,
    pub site_name: Option<String>,
    /// Generic author/byline. Also carries the price label for the "steam" kind
    /// and the `discord.gg/<code>` handle for the "discord" kind.
    pub author: Option<String>,
    /// Author avatar (used by the "tweet" kind).
    pub author_avatar: Option<String>,
    /// Present for the "youtube" and "clip" kinds.
    pub video_id: Option<String>,
    /// View count (the "clip" kind).
    pub view_count: Option<u64>,
    /// Length in seconds (the "clip" kind).
    pub duration: Option<f64>,
    /// Live "online now" count (the "discord" kind).
    pub online_count: Option<u64>,
    /// Total member count (the "discord" kind).
    pub member_count: Option<u64>,
}

// We read only the page <head> (OG / Twitter-card /
// <title> meta all live there), capped so a huge page never downloads in full —
// that full-body download + full-DOM parse is what made slow/large pages take
// many seconds to preview.
const HEAD_SCAN_CAP: usize = 512 * 1024;

// YouTube channel/profile pages bury their OG tags ~635 KB into a ~977 KB <head>
// (everything before them is ytcfg / inline JSON), well past the default cap, so
// the generic scrape stopped short and returned "no preview metadata" for every
// channel link. They need a larger ceiling. Scoped to that one path so the
// chat-firehose default stays tight.
const YT_CHANNEL_SCAN_CAP: usize = 1536 * 1024;

// --- HTTP client -----------------------------------------------------------

/// Dedicated client with a desktop browser UA. Many sites only emit OG tags to
/// "real" browsers, and a short timeout keeps a slow/hostile host from holding
/// a slot. 10s is plenty for a HEAD-of-document meta read.
static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .user_agent(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
             (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36",
        )
        .timeout(Duration::from_secs(10))
        .build()
        .expect("Failed to build link-preview HTTP client")
});

// --- Tauri command ---------------------------------------------------------

#[tauri::command]
pub async fn fetch_link_preview(url: String) -> Result<LinkPreview, String> {
    // Only ever touch http(s). Guards against `javascript:` / `file:` / `data:`
    // and other schemes the regex on the frontend shouldn't pass anyway.
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("Unsupported URL scheme".to_string());
    }

    crate::services::link_preview_cache::get(&url).await.ok_or_else(|| "No preview".to_string())
}

/// Whether `build_preview` has a dedicated card for this URL's shape (a Discord
/// invite, a YouTube channel or video, an X profile or post, an Instagram
/// profile, a Steam app, Spotify, a Twitch clip or VOD) rather than the generic
/// OpenGraph scrape. Mirrors the matcher order in `build_preview`, except Reddit:
/// a profile link there is a subreddit or user, whose card says no more than
/// the pill does.
pub(crate) fn has_dedicated_card(url: &str) -> bool {
    let canonical = canonicalize_x(url);
    let url = canonical.as_str();
    youtube_id(url).is_some()
        || is_youtube_channel(url)
        || tweet_ref(url).is_some()
        || x_profile_handle(url).is_some()
        || twitch_clip_id(url).is_some()
        || twitch_vod_id(url).is_some()
        || discord_invite_code(url).is_some()
        || steam_app_id(url).is_some()
        || spotify_kind(url).is_some()
        || instagram_handle(url).is_some()
}

pub(crate) async fn build_preview(url: &str) -> Result<LinkPreview, String> {
    // twitter.com now redirects to x.com — normalize so both get identical
    // handling (and the card opens the x.com variant).
    let canonical = canonicalize_x(url);
    let url = canonical.as_str();

    if let Some(video_id) = youtube_id(url) {
        return Ok(youtube_preview(url, &video_id).await);
    }
    if is_youtube_channel(url) {
        return youtube_channel_preview(url).await;
    }
    if let Some((handle, id)) = tweet_ref(url) {
        return tweet_preview(url, handle.as_deref(), &id).await;
    }
    if let Some(handle) = x_profile_handle(url) {
        return x_profile_preview(url, &handle).await;
    }
    if is_giphy(url) {
        return giphy_preview(url).await;
    }
    if let Some(clip_id) = twitch_clip_id(url) {
        return twitch_clip_preview(url, &clip_id).await;
    }
    if let Some(vod_id) = twitch_vod_id(url) {
        return twitch_vod_preview(url, &vod_id).await;
    }
    if let Some(code) = discord_invite_code(url) {
        return discord_invite_preview(url, &code).await;
    }
    if let Some(app_id) = steam_app_id(url) {
        return steam_preview(url, &app_id).await;
    }
    if let Some(kind) = spotify_kind(url) {
        return spotify_preview(url, kind).await;
    }
    if let Some(handle) = instagram_handle(url) {
        return instagram_preview(url, &handle).await;
    }
    if is_tenor_page(url) {
        return tenor_preview(url).await;
    }
    // A media viewer link names its image in a query parameter, so the path
    // never ends in an image extension. Preview the image it names; the card
    // still opens the link that was posted.
    if let Some(inner) = unwrap_media_viewer(url) {
        if is_image_url(&inner) {
            return Ok(image_preview(url, &inner));
        }
        let mut preview = fetch_generic(&inner).await?;
        preview.url = url.to_string();
        return Ok(preview);
    }
    if let Some(link) = reddit_link(url) {
        return reddit_preview(url, link).await;
    }
    if is_image_url(url) {
        return Ok(image_preview(url, url));
    }

    let mut preview = fetch_generic(url).await?;
    // Imgur gallery/album/post pages: the og:image IS the real image, but the
    // og:description is Imgur's generic site boilerplate ("Discover the magic of
    // the internet at Imgur..."). Render it image-led (the "media" kind) and
    // drop the boilerplate so it reads like a picture, not an ad for Imgur.
    if is_imgur_page(url) {
        if preview.image.is_some() {
            preview.kind = "media".to_string();
        }
        preview.description = None;
    }
    Ok(preview)
}

// --- YouTube ---------------------------------------------------------------

static YT_ID_RE: LazyLock<Regex> = LazyLock::new(|| {
    // watch?v=, &v=, youtu.be/, shorts/, live/, embed/, v/
    Regex::new(r"(?:youtu\.be/|youtube\.com/(?:shorts/|live/|embed/|v/)|[?&]v=)([A-Za-z0-9_-]{11})")
        .expect("youtube id regex")
});

fn youtube_id(url: &str) -> Option<String> {
    if !(url.contains("youtube.com") || url.contains("youtu.be")) {
        return None;
    }
    YT_ID_RE
        .captures(url)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

async fn youtube_preview(url: &str, video_id: &str) -> LinkPreview {
    // hqdefault always exists for a valid id (maxresdefault can 404).
    let thumbnail = format!("https://i.ytimg.com/vi/{}/hqdefault.jpg", video_id);
    let mut title = None;
    let mut author = None;

    // Keyless oEmbed for title/author. Failure is non-fatal: we still have a
    // perfectly good thumbnail card.
    match CLIENT
        .get("https://www.youtube.com/oembed")
        .query(&[("url", url), ("format", "json")])
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            if let Ok(json) = resp.json::<serde_json::Value>().await {
                title = json
                    .get("title")
                    .and_then(|v| v.as_str())
                    .map(|s| truncate(s, 300));
                author = json
                    .get("author_name")
                    .and_then(|v| v.as_str())
                    .map(|s| truncate(s, 120));
            }
        }
        Ok(resp) => debug!("[LinkPreview] yt oembed status {}", resp.status()),
        Err(e) => debug!("[LinkPreview] yt oembed error: {}", e),
    }

    LinkPreview {
        url: url.to_string(),
        kind: "youtube".to_string(),
        title,
        description: None,
        image: Some(thumbnail),
        site_name: Some("YouTube".to_string()),
        author,
        author_avatar: None,
        video_id: Some(video_id.to_string()),
        view_count: None,
        duration: None,
        online_count: None,
        member_count: None,
    }
}

// --- YouTube channels / profiles -------------------------------------------

static YT_CHANNEL_RE: LazyLock<Regex> = LazyLock::new(|| {
    // @handle, /channel/<id>, /c/<name>, /user/<name>. A bare /@handle is the
    // modern form; the others are legacy. Trailing subpaths (/videos, /about)
    // are fine — we only need to know it's a channel, not a video.
    Regex::new(r"youtube\.com/(?:@[A-Za-z0-9_.\-]+|channel/[A-Za-z0-9_\-]+|c/[^/?#]+|user/[^/?#]+)")
        .expect("youtube channel regex")
});

fn is_youtube_channel(url: &str) -> bool {
    url.contains("youtube.com") && YT_CHANNEL_RE.is_match(url)
}

/// A YouTube channel page serves the channel name (og:title), the square avatar
/// (og:image, e.g. a 900x900 `yt3.googleusercontent.com` image), and the channel
/// description (og:description) to a normal browser UA, so a plain OG scrape is
/// enough. We just reshape it into a profile-style card: the avatar moves to
/// `author_avatar` so the UI renders it as a centered circle (the generic card's
/// square left-thumbnail crops oddly for an avatar), and the kind is tagged so it
/// gets the YouTube glyph.
async fn youtube_channel_preview(url: &str) -> Result<LinkPreview, String> {
    // Google serves a cookie-consent interstitial (no channel OG tags) to clients
    // without a consent cookie, so a plain scrape returns "no preview metadata".
    // Sending an accepted-consent cookie (both the modern SOCS and legacy CONSENT
    // forms) skips it and yields the real channel page.
    let mut preview = fetch_generic_with_cookie(
        url,
        Some("SOCS=CAISNQgDEgk0ODE3Nzk3MjQaAmVuIAEaBgiA_LyaBg; CONSENT=YES+"),
        YT_CHANNEL_SCAN_CAP,
    )
    .await?;
    preview.kind = "youtube_channel".to_string();
    preview.author_avatar = preview.image.take();
    preview.site_name = Some("YouTube".to_string());
    Ok(preview)
}

// --- Twitter / X -----------------------------------------------------------

/// Rewrite a twitter.com (or www/mobile/m subdomain) URL to its x.com
/// equivalent so both variants get identical handling. Anchored at the scheme so
/// a stray "twitter.com" inside a path/query isn't touched.
fn canonicalize_x(url: &str) -> String {
    let lower = url.to_lowercase();
    for host in [
        "twitter.com",
        "www.twitter.com",
        "mobile.twitter.com",
        "m.twitter.com",
    ] {
        for scheme in ["https://", "http://"] {
            let prefix = format!("{}{}", scheme, host);
            if lower.starts_with(&prefix) {
                let rest = &url[prefix.len()..];
                if rest.is_empty() || rest.starts_with(['/', '?', '#']) {
                    return format!("{}x.com{}", scheme, rest);
                }
            }
        }
    }
    url.to_string()
}

static TWEET_RE: LazyLock<Regex> = LazyLock::new(|| {
    // Optional handle, then status/<id>. Covers x.com, twitter.com, and the
    // /i/status/<id> form (no handle).
    Regex::new(r"(?:twitter\.com|x\.com)/(?:([A-Za-z0-9_]{1,15})/)?status(?:es)?/(\d+)")
        .expect("tweet url regex")
});

fn tweet_ref(url: &str) -> Option<(Option<String>, String)> {
    if !(url.contains("x.com") || url.contains("twitter.com")) {
        return None;
    }
    let caps = TWEET_RE.captures(url)?;
    let handle = caps
        .get(1)
        .map(|m| m.as_str().to_string())
        .filter(|h| h != "i" && h != "web"); // /i/status, /web/status are not handles
    let id = caps.get(2)?.as_str().to_string();
    Some((handle, id))
}

async fn tweet_preview(url: &str, handle: Option<&str>, id: &str) -> Result<LinkPreview, String> {
    // FixTweet's keyless JSON unfurler. Fetching through it (rather than x.com
    // directly) is what makes tweets render at all now that X bot-walls its own
    // pages to logged-out clients, and as a side effect X never sees the
    // viewer's IP. Any failure falls back to a plain OG scrape of the URL.
    let handle_seg = handle.unwrap_or("i");
    let api = format!("https://api.fxtwitter.com/{}/status/{}", handle_seg, id);

    let resp = match CLIENT
        .get(&api)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => {
            debug!("[LinkPreview] fxtwitter status {}", r.status());
            return fetch_generic(url).await;
        }
        Err(e) => {
            debug!("[LinkPreview] fxtwitter error: {}", e);
            return fetch_generic(url).await;
        }
    };

    let json: serde_json::Value = match resp.json().await {
        Ok(j) => j,
        Err(e) => {
            debug!("[LinkPreview] fxtwitter json error: {}", e);
            return fetch_generic(url).await;
        }
    };

    let tweet = match json.get("tweet") {
        Some(t) => t,
        None => return fetch_generic(url).await,
    };

    let text = tweet
        .get("text")
        .and_then(|v| v.as_str())
        .map(|s| truncate(s, 600));
    let author = tweet.get("author");
    let name = author
        .and_then(|a| a.get("name"))
        .and_then(|v| v.as_str())
        .map(|s| truncate(s, 120));
    let screen = author
        .and_then(|a| a.get("screen_name"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let avatar = author
        .and_then(|a| a.get("avatar_url"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // First usable media image: a photo's url, otherwise a video/gif thumbnail.
    let image = tweet
        .get("media")
        .and_then(|m| m.get("all"))
        .and_then(|all| all.as_array())
        .and_then(|arr| {
            arr.iter().find_map(|item| {
                let media_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
                if media_type == "photo" {
                    item.get("url").and_then(|v| v.as_str()).map(String::from)
                } else {
                    item.get("thumbnail_url")
                        .and_then(|v| v.as_str())
                        .map(String::from)
                }
            })
        });

    // Nothing usable -> let the generic path have a go at the raw URL.
    if text.is_none() && name.is_none() {
        return fetch_generic(url).await;
    }

    Ok(LinkPreview {
        url: url.to_string(),
        kind: "tweet".to_string(),
        title: name,
        description: text,
        image,
        site_name: Some("X".to_string()),
        author: screen.map(|s| format!("@{}", s)),
        author_avatar: avatar,
        video_id: None,
        view_count: None,
        duration: None,
        online_count: None,
        member_count: None,
    })
}

// X profile links (x.com/<handle>, no /status/). Bare handle only — paths like
// /home, /i/..., /<handle>/status/... are excluded.
static X_PROFILE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://x\.com/([A-Za-z0-9_]{1,15})/?(?:\?|#|$)").expect("x profile regex")
});

// First-path segments on x.com that are routes, not user handles.
const X_RESERVED: &[&str] = &[
    "i",
    "home",
    "explore",
    "search",
    "notifications",
    "messages",
    "settings",
    "compose",
    "intent",
    "hashtag",
    "share",
    "login",
    "signup",
    "about",
    "tos",
    "privacy",
    "help",
    "account",
    "logout",
    "status",
];

fn x_profile_handle(url: &str) -> Option<String> {
    let handle = X_PROFILE_RE.captures(url)?.get(1)?.as_str();
    if X_RESERVED.contains(&handle.to_lowercase().as_str()) {
        return None;
    }
    Some(handle.to_string())
}

async fn x_profile_preview(url: &str, handle: &str) -> Result<LinkPreview, String> {
    // FixTweet's keyless user endpoint — same rationale as the tweet path.
    let api = format!("https://api.fxtwitter.com/{}", handle);
    let resp = match CLIENT
        .get(&api)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => {
            debug!("[LinkPreview] fxtwitter user status {}", r.status());
            return fetch_generic(url).await;
        }
        Err(e) => {
            debug!("[LinkPreview] fxtwitter user error: {}", e);
            return fetch_generic(url).await;
        }
    };

    let json: serde_json::Value = match resp.json().await {
        Ok(j) => j,
        Err(_) => return fetch_generic(url).await,
    };
    let user = match json.get("user") {
        Some(u) => u,
        None => return fetch_generic(url).await,
    };

    let name = user
        .get("name")
        .and_then(|v| v.as_str())
        .map(|s| truncate(s, 120));
    let screen = user
        .get("screen_name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let bio = user
        .get("description")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| truncate(s, 400));
    // Upscale the avatar from the API's _normal (48px) to _400x400.
    let avatar = user
        .get("avatar_url")
        .and_then(|v| v.as_str())
        .map(|s| s.replace("_normal.", "_400x400."));
    let banner = user
        .get("banner_url")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    if name.is_none() && screen.is_none() {
        return fetch_generic(url).await;
    }

    // Reuse the "tweet" card layout: avatar + name + @handle + X glyph, bio as
    // the body text, banner as the media image.
    Ok(LinkPreview {
        url: url.to_string(),
        kind: "tweet".to_string(),
        title: name,
        description: bio,
        image: banner,
        site_name: Some("X".to_string()),
        author: screen.map(|s| format!("@{}", s)),
        author_avatar: avatar,
        video_id: None,
        view_count: None,
        duration: None,
        online_count: None,
        member_count: None,
    })
}

// --- Giphy -----------------------------------------------------------------

static GIPHY_ID_RE: LazyLock<Regex> = LazyLock::new(|| {
    // Trailing alphanumeric id within a single path segment (the slug ends in
    // `-<id>`; embed/media forms have the bare id).
    Regex::new(r"giphy\.com/(?:gifs|clips|embed|stickers|media)/(?:[^/?#]*-)?([A-Za-z0-9]{5,})")
        .expect("giphy id regex")
});

fn is_giphy(url: &str) -> bool {
    url.contains("giphy.com") || url.contains("gph.is")
}

fn giphy_gif_from_url(url: &str) -> Option<String> {
    let id = GIPHY_ID_RE.captures(url)?.get(1)?.as_str();
    Some(format!("https://media.giphy.com/media/{}/giphy.gif", id))
}

async fn giphy_preview(url: &str) -> Result<LinkPreview, String> {
    let make = |gif: String| LinkPreview {
        url: url.to_string(),
        kind: "image".to_string(),
        title: None,
        description: None,
        image: Some(gif),
        site_name: Some("GIPHY".to_string()),
        author: None,
        author_avatar: None,
        video_id: None,
        view_count: None,
        duration: None,
        online_count: None,
        member_count: None,
    };

    // Giphy's keyless oEmbed resolves any giphy page/clip/short-link to the
    // direct gif URL. The gif renders inline animated via a plain <img>.
    match CLIENT
        .get("https://giphy.com/services/oembed")
        .query(&[("url", url)])
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            if let Ok(json) = resp.json::<serde_json::Value>().await {
                if let Some(gif) = json.get("url").and_then(|v| v.as_str()) {
                    return Ok(make(gif.to_string()));
                }
            }
        }
        Ok(resp) => debug!("[LinkPreview] giphy oembed status {}", resp.status()),
        Err(e) => debug!("[LinkPreview] giphy oembed error: {}", e),
    }

    // Fallback: derive the gif URL straight from the id in the page URL.
    if let Some(gif) = giphy_gif_from_url(url) {
        return Ok(make(gif));
    }

    // Last resort (e.g. a gph.is short link the regex can't read): let the
    // generic path follow redirects and read the page's og:image.
    fetch_generic(url).await
}

// --- Twitch clips ----------------------------------------------------------

static TWITCH_CLIP_RE: LazyLock<Regex> = LazyLock::new(|| {
    // clips.twitch.tv/<slug>, clips.twitch.tv/embed?clip=<slug>,
    // (www.|m.)twitch.tv/<channel>/clip/<slug>, twitch.tv/clip/<slug>.
    // Clip slugs are [A-Za-z0-9_-]; the optional channel segment has no hyphens.
    Regex::new(
        r"(?:clips\.twitch\.tv/(?:embed\?clip=)?|twitch\.tv/(?:[A-Za-z0-9_]{1,25}/)?clip/)([A-Za-z0-9_-]+)",
    )
    .expect("twitch clip regex")
});

fn twitch_clip_id(url: &str) -> Option<String> {
    if !url.contains("twitch.tv") {
        return None;
    }
    TWITCH_CLIP_RE
        .captures(url)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

/// Resolve a Twitch clip via Helix `/clips?id=`. A plain OG scrape of a clip page
/// only yields Twitch's generic "world's leading video platform" boilerplate
/// (the page is client-rendered), so we ask the API directly. Reuses the logged-
/// in user's token — they're in chat, so they have one. Any failure (not logged
/// in, network, deleted clip) falls back to the generic scrape, which is exactly
/// the prior behavior, so this never regresses an existing card.
async fn twitch_clip_preview(url: &str, clip_id: &str) -> Result<LinkPreview, String> {
    let token = match TwitchService::get_token().await {
        Ok(t) => t,
        Err(e) => {
            debug!("[LinkPreview] twitch clip: no token ({})", e);
            return fetch_generic(url).await;
        }
    };

    let resp = match CLIENT
        .get("https://api.twitch.tv/helix/clips")
        .query(&[("id", clip_id)])
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", token))
        .header("Client-Id", env!("TWITCH_APP_CLIENT_ID"))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => {
            debug!("[LinkPreview] twitch clips status {}", r.status());
            return fetch_generic(url).await;
        }
        Err(e) => {
            debug!("[LinkPreview] twitch clips error: {}", e);
            return fetch_generic(url).await;
        }
    };

    let json: serde_json::Value = match resp.json().await {
        Ok(j) => j,
        Err(_) => return fetch_generic(url).await,
    };

    // data[] is empty for a deleted/invalid clip — let the generic path try.
    let clip = match json
        .get("data")
        .and_then(|d| d.as_array())
        .and_then(|arr| arr.first())
    {
        Some(c) => c,
        None => return fetch_generic(url).await,
    };

    let str_field = |key: &str| {
        clip.get(key)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
    };

    let title = str_field("title").map(|s| truncate(&s, 300));
    let broadcaster = str_field("broadcaster_name");
    // "Clipped by X" goes in the description line.
    let description = str_field("creator_name").map(|c| format!("Clipped by {}", c));
    let image = str_field("thumbnail_url");
    let view_count = clip.get("view_count").and_then(|v| v.as_u64());
    let duration = clip.get("duration").and_then(|v| v.as_f64());

    Ok(LinkPreview {
        url: url.to_string(),
        kind: "clip".to_string(),
        title,
        description,
        image,
        site_name: Some("Twitch".to_string()),
        author: broadcaster,
        author_avatar: None,
        video_id: Some(clip_id.to_string()),
        view_count,
        duration,
        online_count: None,
        member_count: None,
    })
}

// --- Twitch VODs -----------------------------------------------------------

static TWITCH_VOD_RE: LazyLock<Regex> = LazyLock::new(|| {
    // twitch.tv/videos/<numeric id> (www / m subdomains included by substring).
    Regex::new(r"twitch\.tv/videos/(\d+)").expect("twitch vod regex")
});

static TWITCH_DURATION_RE: LazyLock<Regex> = LazyLock::new(|| {
    // Helix returns VOD length as e.g. "3h8m33s", "27m11s", "58s".
    Regex::new(r"^(?:(\d+)h)?(?:(\d+)m)?(?:(\d+)s)?$").expect("twitch duration regex")
});

fn twitch_vod_id(url: &str) -> Option<String> {
    if !url.contains("twitch.tv") {
        return None;
    }
    TWITCH_VOD_RE
        .captures(url)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

fn parse_twitch_duration(s: &str) -> Option<f64> {
    let caps = TWITCH_DURATION_RE.captures(s.trim())?;
    let part = |i: usize| {
        caps.get(i)
            .and_then(|m| m.as_str().parse::<f64>().ok())
            .unwrap_or(0.0)
    };
    let total = part(1) * 3600.0 + part(2) * 60.0 + part(3);
    if total > 0.0 {
        Some(total)
    } else {
        None
    }
}

/// Resolve a Twitch VOD via Helix `/videos?id=`. Same rationale and auth as the
/// clip path (a plain scrape only yields Twitch boilerplate). Falls back to the
/// generic scrape on any failure.
async fn twitch_vod_preview(url: &str, vod_id: &str) -> Result<LinkPreview, String> {
    let token = match TwitchService::get_token().await {
        Ok(t) => t,
        Err(e) => {
            debug!("[LinkPreview] twitch vod: no token ({})", e);
            return fetch_generic(url).await;
        }
    };

    let resp = match CLIENT
        .get("https://api.twitch.tv/helix/videos")
        .query(&[("id", vod_id)])
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", token))
        .header("Client-Id", env!("TWITCH_APP_CLIENT_ID"))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => {
            debug!("[LinkPreview] twitch videos status {}", r.status());
            return fetch_generic(url).await;
        }
        Err(e) => {
            debug!("[LinkPreview] twitch videos error: {}", e);
            return fetch_generic(url).await;
        }
    };

    let json: serde_json::Value = match resp.json().await {
        Ok(j) => j,
        Err(_) => return fetch_generic(url).await,
    };

    let vod = match json
        .get("data")
        .and_then(|d| d.as_array())
        .and_then(|arr| arr.first())
    {
        Some(v) => v,
        None => return fetch_generic(url).await,
    };

    let str_field = |key: &str| {
        vod.get(key)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
    };

    let title = str_field("title").map(|s| truncate(&s, 300));
    let author = str_field("user_name");
    // VOD thumbnails carry %{width}x%{height} placeholders that must be filled.
    // A still-processing VOD returns a "404_processing" template instead; that
    // resolves to a valid Twitch image, so it's fine to render.
    let image = str_field("thumbnail_url")
        .map(|s| s.replace("%{width}", "480").replace("%{height}", "272"))
        .filter(|s| !s.is_empty());
    let view_count = vod.get("view_count").and_then(|v| v.as_u64());
    let duration = str_field("duration").and_then(|s| parse_twitch_duration(&s));

    Ok(LinkPreview {
        url: url.to_string(),
        kind: "vod".to_string(),
        title,
        description: None,
        image,
        site_name: Some("Twitch".to_string()),
        author,
        author_avatar: None,
        video_id: Some(vod_id.to_string()),
        view_count,
        duration,
        online_count: None,
        member_count: None,
    })
}

// --- Discord invites -------------------------------------------------------

static DISCORD_INVITE_RE: LazyLock<Regex> = LazyLock::new(|| {
    // discord.gg/<code>, discord.com/invite/<code>, discordapp.com/invite/<code>
    // (ptb./canary. subdomains caught by the substring guard). Standard codes are
    // alphanumeric; vanity invites can contain hyphens.
    Regex::new(r"(?i)discord(?:app)?\.(?:gg|com)/(?:invite/)?([A-Za-z0-9-]+)")
        .expect("discord invite regex")
});

fn discord_invite_code(url: &str) -> Option<String> {
    let lower = url.to_lowercase();
    // Only treat discord.gg, or an explicit /invite/ path, as an invite. A plain
    // discord.com link (e.g. /channels/...) is left to the generic path.
    if !(lower.contains("discord.gg/")
        || lower.contains("discord.com/invite/")
        || lower.contains("discordapp.com/invite/"))
    {
        return None;
    }
    let code = DISCORD_INVITE_RE.captures(url)?.get(1)?.as_str();
    if code.is_empty() {
        return None;
    }
    Some(code.to_string())
}

/// Resolve a Discord invite via the public invite API. Keyless, no bot, no
/// widget-enable required (the same endpoint the in-app "Join the community"
/// card uses). Returns a compact server card: icon, name, live online/member
/// counts. Any failure falls back to the generic scrape.
async fn discord_invite_preview(url: &str, code: &str) -> Result<LinkPreview, String> {
    let api = format!(
        "https://discord.com/api/v10/invites/{}?with_counts=true",
        code
    );
    let resp = match CLIENT.get(&api).send().await {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => {
            debug!("[LinkPreview] discord invite status {}", r.status());
            return fetch_generic(url).await;
        }
        Err(e) => {
            debug!("[LinkPreview] discord invite error: {}", e);
            return fetch_generic(url).await;
        }
    };

    let json: serde_json::Value = match resp.json().await {
        Ok(j) => j,
        Err(_) => return fetch_generic(url).await,
    };

    let guild = match json.get("guild") {
        Some(g) => g,
        None => return fetch_generic(url).await,
    };

    let guild_id = guild.get("id").and_then(|v| v.as_str()).unwrap_or("");
    let name = guild
        .get("name")
        .and_then(|v| v.as_str())
        .map(|s| truncate(s, 120));
    let description = guild
        .get("description")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| truncate(s, 300));
    // Animated icons carry an `a_` prefix and resolve as gif; everything else png.
    let image = guild
        .get("icon")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|hash| {
            let ext = if hash.starts_with("a_") { "gif" } else { "png" };
            format!(
                "https://cdn.discordapp.com/icons/{}/{}.{}?size=128",
                guild_id, hash, ext
            )
        });
    let online_count = json
        .get("approximate_presence_count")
        .and_then(|v| v.as_u64());
    let member_count = json
        .get("approximate_member_count")
        .and_then(|v| v.as_u64());

    if name.is_none() {
        return fetch_generic(url).await;
    }

    Ok(LinkPreview {
        url: url.to_string(),
        kind: "discord".to_string(),
        title: name,
        description,
        image,
        site_name: Some("Discord".to_string()),
        author: Some(format!("discord.gg/{}", code)),
        author_avatar: None,
        video_id: None,
        view_count: None,
        duration: None,
        online_count,
        member_count,
    })
}

// --- Steam store -----------------------------------------------------------

static STEAM_APP_RE: LazyLock<Regex> = LazyLock::new(|| {
    // store.steampowered.com/app/<id> and the /agecheck/app/<id> gate variant.
    Regex::new(r"store\.steampowered\.com/(?:agecheck/)?app/(\d+)").expect("steam app regex")
});

fn steam_app_id(url: &str) -> Option<String> {
    if !url.contains("steampowered.com") {
        return None;
    }
    STEAM_APP_RE
        .captures(url)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

/// Resolve a Steam app via the keyless storefront `appdetails` API: capsule art,
/// title, price, and the short description. Falls back to the generic scrape on
/// any failure (the bare store page is age-gated and yields only boilerplate).
async fn steam_preview(url: &str, app_id: &str) -> Result<LinkPreview, String> {
    let api = format!(
        "https://store.steampowered.com/api/appdetails?appids={}&cc=us&l=en",
        app_id
    );
    let resp = match CLIENT.get(&api).send().await {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => {
            debug!("[LinkPreview] steam status {}", r.status());
            return fetch_generic(url).await;
        }
        Err(e) => {
            debug!("[LinkPreview] steam error: {}", e);
            return fetch_generic(url).await;
        }
    };

    let json: serde_json::Value = match resp.json().await {
        Ok(j) => j,
        Err(_) => return fetch_generic(url).await,
    };

    // Response is keyed by the app id: { "<id>": { success, data: {...} } }.
    let entry = json
        .get(app_id)
        .or_else(|| json.as_object().and_then(|o| o.values().next()));
    let data = match entry.and_then(|e| {
        if e.get("success").and_then(|v| v.as_bool()).unwrap_or(false) {
            e.get("data")
        } else {
            None
        }
    }) {
        Some(d) => d,
        None => return fetch_generic(url).await,
    };

    let title = data
        .get("name")
        .and_then(|v| v.as_str())
        .map(|s| truncate(s, 200));
    let image = data
        .get("header_image")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let description = data
        .get("short_description")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| truncate(&strip_html(s), 300));
    // Price label: "Free" when free-to-play, else the storefront-formatted final
    // price (already localized, e.g. "$19.99"). None when unreleased/unpriced.
    let is_free = data
        .get("is_free")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let price = if is_free {
        Some("Free".to_string())
    } else {
        data.get("price_overview")
            .and_then(|p| p.get("final_formatted"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
    };

    if title.is_none() {
        return fetch_generic(url).await;
    }

    Ok(LinkPreview {
        url: url.to_string(),
        kind: "steam".to_string(),
        title,
        description,
        image,
        site_name: Some("Steam".to_string()),
        author: price,
        author_avatar: None,
        video_id: None,
        view_count: None,
        duration: None,
        online_count: None,
        member_count: None,
    })
}

// --- Spotify ---------------------------------------------------------------

static SPOTIFY_RE: LazyLock<Regex> = LazyLock::new(|| {
    // open.spotify.com/[intl-xx/](track|album|playlist|artist|episode|show)/<id>
    Regex::new(
        r"open\.spotify\.com/(?:intl-[a-z]{2}/)?(track|album|playlist|artist|episode|show)/[A-Za-z0-9]+",
    )
    .expect("spotify regex")
});

fn spotify_kind(url: &str) -> Option<String> {
    if !url.contains("open.spotify.com") {
        return None;
    }
    SPOTIFY_RE
        .captures(url)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

/// Resolve a Spotify link via the keyless oEmbed endpoint: title + square cover
/// art. The content type (track/album/…) comes from the URL itself so the card
/// can label it without a second request. Falls back to the generic scrape.
async fn spotify_preview(url: &str, content_kind: String) -> Result<LinkPreview, String> {
    let resp = match CLIENT
        .get("https://open.spotify.com/oembed")
        .query(&[("url", url)])
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => {
            debug!("[LinkPreview] spotify oembed status {}", r.status());
            return fetch_generic(url).await;
        }
        Err(e) => {
            debug!("[LinkPreview] spotify oembed error: {}", e);
            return fetch_generic(url).await;
        }
    };

    let json: serde_json::Value = match resp.json().await {
        Ok(j) => j,
        Err(_) => return fetch_generic(url).await,
    };

    let title = json
        .get("title")
        .and_then(|v| v.as_str())
        .map(|s| truncate(s, 200));
    let image = json
        .get("thumbnail_url")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    if title.is_none() && image.is_none() {
        return fetch_generic(url).await;
    }

    // "track" -> "Track" for the card's type label.
    let label = {
        let mut chars = content_kind.chars();
        match chars.next() {
            Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            None => content_kind.clone(),
        }
    };

    Ok(LinkPreview {
        url: url.to_string(),
        kind: "spotify".to_string(),
        title,
        description: Some(label),
        image,
        site_name: Some("Spotify".to_string()),
        author: None,
        author_avatar: None,
        video_id: None,
        view_count: None,
        duration: None,
        online_count: None,
        member_count: None,
    })
}

// --- Instagram profiles (via SearchAPI) ------------------------------------

static INSTAGRAM_RE: LazyLock<Regex> = LazyLock::new(|| {
    // instagram.com/<username> (www / m subdomains caught by the substring guard).
    // Username chars are letters, digits, dot, underscore.
    Regex::new(r"(?i)instagram\.com/([A-Za-z0-9._]+)/?(?:\?|#|$)").expect("instagram regex")
});

// First-path segments on instagram.com that are routes, not usernames.
const INSTAGRAM_RESERVED: &[&str] = &[
    "p",
    "reel",
    "reels",
    "explore",
    "stories",
    "tv",
    "accounts",
    "about",
    "directory",
    "developer",
    "legal",
    "privacy",
    "session",
];

fn instagram_handle(url: &str) -> Option<String> {
    if !url.to_lowercase().contains("instagram.com") {
        return None;
    }
    let handle = INSTAGRAM_RE.captures(url)?.get(1)?.as_str();
    if handle.is_empty() || INSTAGRAM_RESERVED.contains(&handle.to_lowercase().as_str()) {
        return None;
    }
    Some(handle.to_string())
}

/// Resolve an Instagram profile WITHOUT an API key, login, or signup — via
/// Instagram's own public web-profile JSON endpoint, called with the desktop-web
/// app-id header (`X-IG-App-ID`, the same value the instagram.com site sends;
/// learned from the keyless `granary` library). Returns the public profile
/// (name, avatar, bio, follower count) for public accounts. Instagram can
/// rate-limit/block this from some IPs, so ANY failure (status, parse, private or
/// empty profile) falls back to the generic scrape — the prior minimal chip — so
/// it never regresses.
async fn instagram_preview(url: &str, handle: &str) -> Result<LinkPreview, String> {
    let api = format!(
        "https://www.instagram.com/api/v1/users/web_profile_info/?username={}",
        handle
    );
    let resp = match CLIENT
        .get(&api)
        .header("X-IG-App-ID", "936619743392459")
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => {
            debug!("[LinkPreview] instagram web_profile status {}", r.status());
            return fetch_generic(url).await;
        }
        Err(e) => {
            debug!("[LinkPreview] instagram web_profile error: {}", e);
            return fetch_generic(url).await;
        }
    };

    let json: serde_json::Value = match resp.json().await {
        Ok(j) => j,
        Err(_) => return fetch_generic(url).await,
    };

    let user = match json.pointer("/data/user") {
        Some(u) if u.is_object() => u,
        _ => return fetch_generic(url).await,
    };

    let str_field = |key: &str| {
        user.get(key)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
    };

    let name = str_field("full_name").map(|s| truncate(&s, 120));
    let username = str_field("username").unwrap_or_else(|| handle.to_string());
    let bio = str_field("biography").map(|s| truncate(&s, 400));
    let avatar = str_field("profile_pic_url_hd").or_else(|| str_field("profile_pic_url"));
    let followers = user
        .pointer("/edge_followed_by/count")
        .and_then(|v| v.as_u64());

    if name.is_none() && avatar.is_none() {
        return fetch_generic(url).await;
    }

    Ok(LinkPreview {
        url: url.to_string(),
        kind: "instagram".to_string(),
        title: Some(name.unwrap_or_else(|| username.clone())),
        description: bio,
        image: None,
        site_name: Some("Instagram".to_string()),
        author: Some(format!("@{}", username)),
        author_avatar: avatar,
        video_id: None,
        view_count: None,
        duration: None,
        online_count: None,
        // member_count carries the follower count (both generic u64s).
        member_count: followers,
    })
}

// --- Tenor (animated GIFs) -------------------------------------------------

fn is_tenor_page(url: &str) -> bool {
    // The shareable view page. Direct media*.tenor.com/*.gif links are caught by
    // the image path before we ever reach here.
    url.to_lowercase().contains("tenor.com/view/")
}

/// A Tenor view page exposes the direct animated GIF via
/// `<meta itemprop="contentUrl" content="https://media*.tenor.com/.../x.gif">`.
/// We pull that and render it as an animated image card (the same shape Giphy
/// uses), falling back to og:image / the generic scrape if it's missing.
async fn tenor_preview(url: &str) -> Result<LinkPreview, String> {
    let resp = match CLIENT.get(url).timeout(Duration::from_secs(8)).send().await {
        Ok(r) if r.status().is_success() => r,
        _ => return fetch_generic(url).await,
    };

    let body = read_capped_body(resp, HEAD_SCAN_CAP).await?;
    let html = String::from_utf8_lossy(&body).into_owned();

    // Scope the parsed document so it's dropped before the fallback await below:
    // `scraper::Html` is not `Send`, and a tauri::command future must be `Send`.
    // `meta()` returns owned Strings, so nothing borrowed escapes this block.
    let gif = {
        let document = Html::parse_document(&html);
        meta(&document, "itemprop", "contentUrl")
            .filter(|s| s.to_lowercase().contains(".gif"))
            .or_else(|| meta(&document, "property", "og:image"))
            .or_else(|| meta(&document, "name", "twitter:image"))
    };

    match gif {
        Some(image) => Ok(LinkPreview {
            url: url.to_string(),
            kind: "image".to_string(),
            title: None,
            description: None,
            image: Some(image),
            site_name: Some("Tenor".to_string()),
            author: None,
            author_avatar: None,
            video_id: None,
            view_count: None,
            duration: None,
            online_count: None,
            member_count: None,
        }),
        None => fetch_generic(url).await,
    }
}

/// Minimal HTML-tag stripper for the rare API text field that ships markup
/// (e.g. a Steam short_description with an inline <br>). Entity-decoding is not
/// needed here; these fields are plain or lightly tagged.
fn strip_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

// --- Reddit ----------------------------------------------------------------
//
// reddit.com answers an app's request for any page, `.json` or old.reddit with
// a script challenge and no preview tags, so the generic scrape only ever saw a
// page titled "Reddit". Two public, sign-in-free endpoints still answer:
//   - oEmbed (`/oembed?url=`): title, author and the post's real subreddit for
//     any `/r/<sub>/comments/<id>` link. It finds the post by id alone, so a
//     link that names no subreddit is asked for as `/r/all/comments/<id>`.
//   - the post's Atom feed (`/comments/<id>/.rss`): its first entry carries the
//     preview image Reddit chose, which Reddit itself blurs for NSFW and
//     spoiler posts. Anonymous feed reads are limited to a few a minute, so the
//     image is best effort and the card never depends on it.

const REDDIT_UA: &str = concat!("StreamNook/", env!("CARGO_PKG_VERSION"), " (chat link preview)");

/// What a Reddit link points at, read from the URL alone.
#[derive(Debug, Clone, PartialEq)]
enum RedditLink {
    /// `/r/<sub>/comments/<id>/<slug>` and its comment permalinks. `redd.it/<id>`,
    /// `/comments/<id>` and `/gallery/<id>` name no subreddit.
    Post {
        id: String,
        subreddit: Option<String>,
        slug: Option<String>,
    },
    /// A link that only redirects to a post: `/r/<sub>/s/<token>` share links
    /// and `v.redd.it` videos.
    Redirect { subreddit: Option<String>, video: bool },
    Subreddit(String),
    User(String),
}

fn valid_subreddit(s: &str) -> bool {
    (2..=21).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

fn valid_post_id(s: &str) -> bool {
    (2..=12).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric())
}

fn valid_reddit_user(s: &str) -> bool {
    (3..=20).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn reddit_link(url: &str) -> Option<RedditLink> {
    let parsed = url::Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "https" | "http") {
        return None;
    }
    let host = parsed.host_str()?.to_ascii_lowercase();
    let segs: Vec<&str> = parsed.path_segments()?.filter(|s| !s.is_empty()).collect();
    let post = |id: &str, subreddit: Option<&str>, slug: Option<&&str>| RedditLink::Post {
        id: id.to_ascii_lowercase(),
        subreddit: subreddit.map(String::from),
        slug: slug.map(|s| s.to_string()),
    };

    if host == "redd.it" {
        return match segs.as_slice() {
            [id] if valid_post_id(id) => Some(post(id, None, None)),
            _ => None,
        };
    }
    if host == "v.redd.it" {
        return (!segs.is_empty()).then_some(RedditLink::Redirect { subreddit: None, video: true });
    }
    if !(host == "reddit.com" || host.ends_with(".reddit.com")) {
        return None;
    }
    match segs.as_slice() {
        ["r", sub, "comments", id, rest @ ..] if valid_subreddit(sub) && valid_post_id(id) => {
            Some(post(id, Some(*sub), rest.first()))
        }
        ["r", sub, "s", _token, ..] if valid_subreddit(sub) => Some(RedditLink::Redirect {
            subreddit: Some(sub.to_string()),
            video: false,
        }),
        ["r", sub, ..] if valid_subreddit(sub) => Some(RedditLink::Subreddit(sub.to_string())),
        ["comments", id, rest @ ..] if valid_post_id(id) => Some(post(id, None, rest.first())),
        ["gallery", id] if valid_post_id(id) => Some(post(id, None, None)),
        // Where v.redd.it lands first; it may redirect on to the post.
        ["video", _] => Some(RedditLink::Redirect { subreddit: None, video: true }),
        ["user" | "u", name, ..] if valid_reddit_user(name) => Some(RedditLink::User(name.to_string())),
        _ => None,
    }
}

/// A post's URL slug read as words, for when oEmbed does not answer.
fn reddit_slug_title(slug: &str) -> Option<String> {
    let words = slug.replace('_', " ");
    let words = words.trim();
    (words.chars().any(|c| c.is_alphanumeric())).then(|| words.to_string())
}

#[derive(Debug, Default, PartialEq)]
struct RedditPost {
    title: Option<String>,
    author: Option<String>,
    subreddit: Option<String>,
}

static REDDIT_EMBED_SUB_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"reddit\.com/r/([A-Za-z0-9_]{2,21})/comments/").expect("reddit embed subreddit regex")
});

fn parse_reddit_oembed(json: &serde_json::Value) -> RedditPost {
    let text = |key: &str| {
        json.get(key)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
    };
    RedditPost {
        title: text("title").map(|t| truncate(&t, 300)),
        // "[deleted]" is not a name.
        author: text("author_name").filter(|a| !a.starts_with('[')),
        // The embed's first link is the post under its real subreddit, also
        // when it was asked for as /r/all.
        subreddit: json
            .get("html")
            .and_then(|v| v.as_str())
            .and_then(|html| REDDIT_EMBED_SUB_RE.captures(html))
            .map(|c| c[1].to_string()),
    }
}

async fn reddit_oembed(post_url: &str) -> Option<RedditPost> {
    let resp = CLIENT
        .get("https://www.reddit.com/oembed")
        .query(&[("url", post_url)])
        .header(reqwest::header::USER_AGENT, REDDIT_UA)
        .timeout(Duration::from_secs(6))
        .send()
        .await
        .map_err(|e| debug!("[LinkPreview] reddit oembed error: {}", e))
        .ok()?;
    if !resp.status().is_success() {
        debug!("[LinkPreview] reddit oembed status {}", resp.status());
        return None;
    }
    let json = resp.json::<serde_json::Value>().await.ok()?;
    Some(parse_reddit_oembed(&json))
}

static FEED_THUMBNAIL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<media:thumbnail\s+url="([^"]+)""#).expect("feed thumbnail regex"));

fn xml_unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

/// The post's preview image from its Atom feed: the first entry is the post,
/// every later one a comment. Only Reddit-hosted images, and only previews
/// wide enough to fill the card (list thumbnails are 140 px).
fn parse_reddit_feed_image(xml: &str) -> Option<String> {
    let start = xml.find("<entry")?;
    let end = xml[start..].find("</entry>").map_or(xml.len(), |i| start + i);
    let raw = FEED_THUMBNAIL_RE.captures(&xml[start..end])?.get(1)?.as_str();
    let image = xml_unescape(raw);
    let parsed = url::Url::parse(&image).ok()?;
    if parsed.scheme() != "https" {
        return None;
    }
    let host = parsed.host_str()?.to_ascii_lowercase();
    if !(host == "redd.it" || host.ends_with(".redd.it") || host.ends_with(".redditmedia.com")) {
        return None;
    }
    let width = parsed
        .query_pairs()
        .find(|(key, _)| key == "width")
        .and_then(|(_, v)| v.parse::<u32>().ok());
    if width.is_some_and(|w| w < 320) {
        return None;
    }
    Some(image)
}

async fn reddit_feed_image(id: &str) -> Option<String> {
    let resp = CLIENT
        .get(format!("https://www.reddit.com/comments/{}/.rss", id))
        .header(reqwest::header::USER_AGENT, REDDIT_UA)
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        debug!("[LinkPreview] reddit feed status {}", resp.status());
        return None;
    }
    // A popular post's feed lists hundreds of comments; the post comes first.
    let buf = read_body_until(resp, 256 * 1024, b"</entry>").await.ok()?;
    parse_reddit_feed_image(&String::from_utf8_lossy(&buf))
}

/// Follow a share or video link one hop at a time until it names a post. Only
/// reddit.com and redd.it are ever requested: a hop anywhere else ends it.
async fn follow_reddit_redirects(url: &str) -> Option<RedditLink> {
    let client = crate::services::http::client_no_redirect();
    let mut current = url::Url::parse(url).ok()?;
    for _ in 0..3 {
        let resp = client
            .get(current.clone())
            .header(reqwest::header::USER_AGENT, REDDIT_UA)
            .timeout(Duration::from_secs(6))
            .send()
            .await
            .ok()?;
        if !resp.status().is_redirection() {
            return None;
        }
        let location = resp.headers().get(reqwest::header::LOCATION)?.to_str().ok()?;
        current = current.join(location).ok()?;
        match reddit_link(current.as_str())? {
            post @ RedditLink::Post { .. } => return Some(post),
            RedditLink::Redirect { .. } => continue,
            _ => return None,
        }
    }
    None
}

/// The Reddit card. Rust writes every label; the page only draws them.
fn reddit_card(
    url: &str,
    title: Option<String>,
    subreddit: Option<String>,
    author: Option<String>,
    image: Option<String>,
) -> LinkPreview {
    LinkPreview {
        url: url.to_string(),
        kind: "reddit".to_string(),
        title,
        description: None,
        image,
        site_name: Some(subreddit.map_or_else(|| "Reddit".to_string(), |s| format!("r/{}", s))),
        author: author.map(|a| format!("u/{}", a)),
        author_avatar: None,
        video_id: None,
        view_count: None,
        duration: None,
        online_count: None,
        member_count: None,
    }
}

/// A share or video link whose redirect did not resolve: only what the link
/// itself says, a subreddit or that it is a video. The page draws a title-less
/// Reddit card as a compact chip.
fn unresolved_reddit_card(url: &str, subreddit: Option<String>, video: bool) -> Result<LinkPreview, String> {
    if video {
        let mut card = reddit_card(url, None, None, None, None);
        card.site_name = Some("Reddit video".to_string());
        return Ok(card);
    }
    match subreddit {
        Some(sub) => Ok(reddit_card(url, None, Some(sub), None, None)),
        None => Err("reddit link did not resolve".to_string()),
    }
}

async fn reddit_preview(url: &str, link: RedditLink) -> Result<LinkPreview, String> {
    let link = match link {
        RedditLink::Redirect { subreddit, video } => match follow_reddit_redirects(url).await {
            Some(post) => post,
            None => return unresolved_reddit_card(url, subreddit, video),
        },
        other => other,
    };
    match link {
        RedditLink::Post { id, subreddit, slug } => {
            let asked_as = subreddit.as_deref().unwrap_or("all");
            let post_url = format!("https://www.reddit.com/r/{}/comments/{}/", asked_as, id);
            let (post, image) = tokio::join!(reddit_oembed(&post_url), reddit_feed_image(&id));
            let post = post.unwrap_or_default();
            let title = post.title.or_else(|| slug.as_deref().and_then(reddit_slug_title));
            let subreddit = post
                .subreddit
                .or(subreddit)
                .filter(|s| !s.eq_ignore_ascii_case("all"));
            if title.is_none() && subreddit.is_none() && image.is_none() {
                return Err("no reddit post metadata".to_string());
            }
            Ok(reddit_card(url, title, subreddit, post.author, image))
        }
        RedditLink::Subreddit(name) => Ok(reddit_card(url, Some(format!("r/{}", name)), None, None, None)),
        RedditLink::User(name) => Ok(reddit_card(url, Some(format!("u/{}", name)), None, None, None)),
        RedditLink::Redirect { .. } => Err("reddit link did not resolve".to_string()),
    }
}

// --- Imgur -----------------------------------------------------------------

fn is_imgur_page(url: &str) -> bool {
    // Gallery/album/post HTML pages, NOT i.imgur.com direct images (those are
    // handled by the image path before we ever get here).
    let lower = url.to_lowercase();
    lower.contains("://imgur.com/")
        || lower.contains("://www.imgur.com/")
        || lower.contains("://m.imgur.com/")
}

// --- Direct image ----------------------------------------------------------

fn is_image_url(url: &str) -> bool {
    // Strip query/fragment, then look at the extension.
    let path = url.split(['?', '#']).next().unwrap_or(url).to_lowercase();
    [".jpg", ".jpeg", ".png", ".gif", ".webp", ".avif", ".bmp"]
        .iter()
        .any(|ext| path.ends_with(ext))
}

/// The card for a link that is (or wraps) a picture: `url` is what was posted,
/// `image` is what the card draws.
fn image_preview(url: &str, image: &str) -> LinkPreview {
    LinkPreview {
        url: url.to_string(),
        kind: "image".to_string(),
        title: None,
        description: None,
        image: Some(image.to_string()),
        site_name: None,
        author: None,
        author_avatar: None,
        video_id: None,
        view_count: None,
        duration: None,
        online_count: None,
        member_count: None,
    }
}

/// The image behind a media viewer link, e.g. Reddit's
/// `reddit.com/media?url=https%3A%2F%2Fi.redd.it%2Fx.jpg`, which is what
/// Reddit's "open image" hands out. The page itself is a bot check with no
/// preview tags, so the picture only shows if we read it out of `url`.
///
/// Only Reddit's own image hosts are unwrapped. The chat trusted reddit.com,
/// not whatever host a crafted `url=` names, and loading that host's image
/// would show the viewer's IP to it.
fn unwrap_media_viewer(url: &str) -> Option<String> {
    let outer = url::Url::parse(url).ok()?;
    let host = outer.host_str()?.to_ascii_lowercase();
    if !(host == "reddit.com" || host.ends_with(".reddit.com")) {
        return None;
    }
    if outer.path().trim_end_matches('/') != "/media" {
        return None;
    }
    let inner = outer
        .query_pairs()
        .find(|(key, _)| key == "url")
        .map(|(_, value)| value.into_owned())?;
    let parsed = url::Url::parse(&inner).ok()?;
    if !matches!(parsed.scheme(), "https" | "http") {
        return None;
    }
    let inner_host = parsed.host_str()?.to_ascii_lowercase();
    if !(inner_host == "redd.it" || inner_host.ends_with(".redd.it")) {
        return None;
    }
    Some(inner)
}

// --- Generic OpenGraph scrape ----------------------------------------------

/// Stream a response body and stop the moment we've seen `</head>` (or hit the
/// cap), instead of `resp.text()` pulling the whole page. Meta tags live in
/// `<head>`, so this is all we need — it turns a multi-MB download + full-DOM
/// parse into reading the first few KB of most pages. Shared by the generic
/// scraper and the Tenor extractor.
async fn read_capped_body(resp: reqwest::Response, cap: usize) -> Result<Vec<u8>, String> {
    read_body_until(resp, cap, b"</head").await
}

/// Stream a body until `marker` (ASCII, any case) has arrived or `cap` bytes
/// have been read, whichever comes first.
async fn read_body_until(
    mut resp: reqwest::Response,
    cap: usize,
    marker: &[u8],
) -> Result<Vec<u8>, String> {
    let mut buf: Vec<u8> = Vec::with_capacity(64 * 1024);
    loop {
        match resp.chunk().await {
            Ok(Some(chunk)) => {
                let prev = buf.len();
                buf.extend_from_slice(&chunk);
                // Scan the newly appended bytes (overlapping the previous chunk
                // so a marker split across chunks is still caught).
                let from = prev.saturating_sub(marker.len() - 1);
                if buf[from..]
                    .windows(marker.len())
                    .any(|w| w.eq_ignore_ascii_case(marker))
                {
                    break;
                }
                if buf.len() >= cap {
                    break;
                }
            }
            Ok(None) => break,
            Err(e) => return Err(format!("read body failed: {}", e)),
        }
    }
    Ok(buf)
}

async fn fetch_generic(url: &str) -> Result<LinkPreview, String> {
    fetch_generic_with_cookie(url, None, HEAD_SCAN_CAP).await
}

/// `fetch_generic`, optionally sending a `Cookie` header and using a caller-chosen
/// head-scan ceiling. The cookie skips Google's consent interstitial on YouTube
/// channel pages (which otherwise carries no OG tags); it's None for every other
/// host. `cap` lets the YouTube channel path read deep enough to reach its OG
/// tags while the chat firehose keeps the tight default.
async fn fetch_generic_with_cookie(
    url: &str,
    cookie: Option<&str>,
    cap: usize,
) -> Result<LinkPreview, String> {
    // A tighter per-request timeout than the client default: a generic preview is
    // a "nice to have", so don't let an unresponsive host hold the UI's loading
    // state for the full client budget.
    let mut builder = CLIENT.get(url).timeout(Duration::from_secs(8));
    if let Some(c) = cookie {
        builder = builder.header(reqwest::header::COOKIE, c);
    }
    let resp = builder
        .send()
        .await
        .map_err(|e| format!("fetch failed: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("status {}", resp.status()));
    }

    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_lowercase();

    // The link's content-type can reveal it's actually an image (no extension
    // in the URL, e.g. a CDN redirect). Treat it as an image card.
    if content_type.starts_with("image/") {
        return Ok(image_preview(url, url));
    }

    // Anything that isn't HTML has no OG tags to read.
    if !content_type.is_empty() && !content_type.contains("html") {
        return Err("not an HTML page".to_string());
    }

    // Read only the page <head> (capped), then parse meta tags from it.
    let buf = read_capped_body(resp, cap).await?;
    let html = String::from_utf8_lossy(&buf).into_owned();
    let preview = parse_meta(url, &html);

    // No usable signal -> error so the UI just leaves the plain link alone
    // instead of rendering an empty card.
    if preview.title.is_none() && preview.image.is_none() && preview.description.is_none() {
        return Err("no preview metadata".to_string());
    }

    Ok(preview)
}

fn parse_meta(url: &str, html: &str) -> LinkPreview {
    // html5ever decodes HTML entities in attribute values and text during
    // parsing, so the strings we pull out are already clean.
    let document = Html::parse_document(html);

    let title = meta(&document, "property", "og:title")
        .or_else(|| meta(&document, "name", "twitter:title"))
        .or_else(|| doc_title(&document))
        .map(|s| truncate(&s, 300));

    let description = meta(&document, "property", "og:description")
        .or_else(|| meta(&document, "name", "twitter:description"))
        .or_else(|| meta(&document, "name", "description"))
        .map(|s| truncate(&s, 500));

    let image = meta(&document, "property", "og:image")
        .or_else(|| meta(&document, "property", "og:image:url"))
        .or_else(|| meta(&document, "name", "twitter:image"))
        .or_else(|| meta(&document, "name", "twitter:image:src"))
        .map(|s| resolve_url(url, &s));

    let site_name = meta(&document, "property", "og:site_name").map(|s| truncate(&s, 120));

    LinkPreview {
        url: url.to_string(),
        kind: "generic".to_string(),
        title,
        description,
        image,
        site_name,
        author: None,
        author_avatar: None,
        video_id: None,
        view_count: None,
        duration: None,
        online_count: None,
        member_count: None,
    }
}

/// First non-empty `content` attr for `meta[<attr>="<key>"]`.
fn meta(document: &Html, attr: &str, key: &str) -> Option<String> {
    let selector = Selector::parse(&format!(r#"meta[{}="{}"]"#, attr, key)).ok()?;
    for el in document.select(&selector) {
        if let Some(content) = el.value().attr("content") {
            let trimmed = content.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn doc_title(document: &Html) -> Option<String> {
    let selector = Selector::parse("title").ok()?;
    let el = document.select(&selector).next()?;
    let text = el.text().collect::<String>();
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Resolve a possibly-relative og:image against the page URL. Handles the three
/// common forms (absolute, scheme-relative `//host/..`, root-relative `/..`)
/// without pulling in a URL-parsing crate. Anything else is returned as-is.
fn resolve_url(base: &str, target: &str) -> String {
    let t = target.trim();
    if t.starts_with("http://") || t.starts_with("https://") {
        return t.to_string();
    }
    if let Some(rest) = t.strip_prefix("//") {
        let scheme = if base.starts_with("http://") {
            "http"
        } else {
            "https"
        };
        return format!("{}://{}", scheme, rest);
    }
    // Build the origin (scheme://host[:port]) from the base.
    let origin = origin_of(base);
    if t.starts_with('/') {
        return format!("{}{}", origin, t);
    }
    // Truly relative (rare for og:image). Fall back to origin + "/" + target.
    format!("{}/{}", origin, t)
}

fn origin_of(url: &str) -> String {
    // scheme://authority/...  -> scheme://authority
    if let Some(scheme_end) = url.find("://") {
        let after = scheme_end + 3;
        let authority_end = url[after..]
            .find('/')
            .map(|i| after + i)
            .unwrap_or(url.len());
        return url[..authority_end].to_string();
    }
    url.to_string()
}

fn truncate(s: &str, max_chars: usize) -> String {
    let trimmed = s.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let mut out: String = trimmed.chars().take(max_chars).collect();
    out.push('\u{2026}'); // ellipsis
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The link that showed as plain text: Reddit's viewer around an i.redd.it jpg.
    const REDDIT_VIEWER: &str =
        "https://www.reddit.com/media?url=https%3A%2F%2Fi.redd.it%2Ftdy7wdh3gvod1.jpg";

    #[test]
    fn image_extensions_match_in_any_case_with_or_without_a_query() {
        assert!(is_image_url("https://i.redd.it/tdy7wdh3gvod1.jpg"));
        assert!(is_image_url("https://i.imgur.com/CzXTtJV.jpeg"));
        assert!(is_image_url("https://files.catbox.moe/abc.JPG"));
        assert!(is_image_url("https://i.nuuls.com/AbC.Jpeg"));
        assert!(is_image_url("https://preview.redd.it/abc.jpg?width=640&auto=webp&s=1"));
        assert!(is_image_url("https://cdn.discordapp.com/attachments/1/2/a.jpg?ex=1&is=2&hm=3&"));
        assert!(is_image_url("https://i.imgur.com/a.png#top"));
        assert!(!is_image_url("https://www.reddit.com/r/pics/comments/abc/title/"));
        assert!(!is_image_url("https://example.com/jpg"));
    }

    #[test]
    fn an_extension_inside_the_query_is_not_an_image_link_on_its_own() {
        // Why the viewer link fell through: its path is `/media`.
        assert!(!is_image_url(REDDIT_VIEWER));
    }

    #[test]
    fn reddit_viewer_links_unwrap_to_the_image() {
        assert_eq!(
            unwrap_media_viewer(REDDIT_VIEWER).as_deref(),
            Some("https://i.redd.it/tdy7wdh3gvod1.jpg"),
        );
        // Any reddit.com host, an unencoded parameter, a trailing slash.
        assert_eq!(
            unwrap_media_viewer("https://old.reddit.com/media/?url=https://i.redd.it/x.JPG").as_deref(),
            Some("https://i.redd.it/x.JPG"),
        );
        assert_eq!(
            unwrap_media_viewer("https://reddit.com/media?foo=1&url=https%3A%2F%2Fi.redd.it%2Fy.png").as_deref(),
            Some("https://i.redd.it/y.png"),
        );
        // preview.redd.it keeps its own query, which was encoded inside ours.
        let preview = unwrap_media_viewer(
            "https://www.reddit.com/media?url=https%3A%2F%2Fpreview.redd.it%2Fabc.jpeg%3Fwidth%3D640%26auto%3Dwebp%26s%3Ddeadbeef",
        )
        .expect("preview.redd.it unwraps");
        assert_eq!(preview, "https://preview.redd.it/abc.jpeg?width=640&auto=webp&s=deadbeef");
        assert!(is_image_url(&preview));
    }

    #[test]
    fn viewer_links_never_unwrap_to_a_host_reddit_does_not_run() {
        for url in [
            "https://www.reddit.com/media?url=https%3A%2F%2Fgrabber.example%2Fx.jpg",
            "https://www.reddit.com/media?url=https%3A%2F%2Fi.redd.it.grabber.example%2Fx.jpg",
            "https://www.reddit.com/media?url=https%3A%2F%2Fnotredd.it%2Fx.jpg",
            "https://www.reddit.com/media?url=javascript%3Aalert(1)",
            "https://www.reddit.com/media?url=",
            "https://www.reddit.com/media",
            "https://www.reddit.com/r/pics/?url=https%3A%2F%2Fi.redd.it%2Fx.jpg",
            "https://reddit.com.grabber.example/media?url=https%3A%2F%2Fi.redd.it%2Fx.jpg",
            "https://notreddit.com/media?url=https%3A%2F%2Fi.redd.it%2Fx.jpg",
        ] {
            assert_eq!(unwrap_media_viewer(url), None, "{url}");
        }
    }

    fn post(id: &str, subreddit: Option<&str>, slug: Option<&str>) -> Option<RedditLink> {
        Some(RedditLink::Post {
            id: id.to_string(),
            subreddit: subreddit.map(String::from),
            slug: slug.map(String::from),
        })
    }

    #[test]
    fn reddit_post_links_parse_from_every_host_and_shape() {
        let slug = Some("the_undertaker_immediately_after_being");
        let full = "https://www.reddit.com/r/pics/comments/1x8gy05/the_undertaker_immediately_after_being/";
        assert_eq!(reddit_link(full), post("1x8gy05", Some("pics"), slug));
        assert_eq!(
            reddit_link("https://old.reddit.com/r/pics/comments/1x8gy05/the_undertaker_immediately_after_being/?utm_source=share&utm_medium=web3x"),
            post("1x8gy05", Some("pics"), slug),
        );
        assert_eq!(reddit_link("https://reddit.com/r/IAmA/comments/z1c9z"), post("z1c9z", Some("IAmA"), None));
        assert_eq!(
            reddit_link("https://www.reddit.com/r/IAmA/comments/z1c9z/i_am_barack_obama/c60o0iw/"),
            post("z1c9z", Some("IAmA"), Some("i_am_barack_obama")),
        );
        // Links that name no subreddit.
        assert_eq!(reddit_link("https://redd.it/92dd8"), post("92dd8", None, None));
        assert_eq!(reddit_link("https://www.reddit.com/comments/92dd8"), post("92dd8", None, None));
        assert_eq!(reddit_link("https://www.reddit.com/gallery/92dd8"), post("92dd8", None, None));
    }

    #[test]
    fn reddit_share_video_subreddit_and_user_links_parse() {
        assert_eq!(
            reddit_link("https://www.reddit.com/r/pics/s/1aBcDeFgHi"),
            Some(RedditLink::Redirect { subreddit: Some("pics".into()), video: false }),
        );
        assert_eq!(
            reddit_link("https://v.redd.it/abc123xyz"),
            Some(RedditLink::Redirect { subreddit: None, video: true }),
        );
        assert_eq!(reddit_link("https://www.reddit.com/r/xqcow/"), Some(RedditLink::Subreddit("xqcow".into())));
        assert_eq!(reddit_link("https://www.reddit.com/r/pics/top/?t=week"), Some(RedditLink::Subreddit("pics".into())));
        // A malformed post id still says which subreddit it is under.
        assert_eq!(reddit_link("https://www.reddit.com/r/pics/comments/!!/"), Some(RedditLink::Subreddit("pics".into())));
        assert_eq!(reddit_link("https://www.reddit.com/user/spez/"), Some(RedditLink::User("spez".into())));
        assert_eq!(reddit_link("https://reddit.com/u/some-one_1"), Some(RedditLink::User("some-one_1".into())));
    }

    #[test]
    fn non_reddit_and_non_page_links_are_left_alone() {
        for url in [
            "https://i.redd.it/tdy7wdh3gvod1.jpg",
            "https://preview.redd.it/abc.jpeg?width=640&s=1",
            "https://www.reddit.com/",
            REDDIT_VIEWER,
            "https://www.reddit.com/r/a/",
            "https://redd.it/",
            "https://notreddit.com/r/pics/",
            "https://reddit.com.grabber.example/r/pics/comments/92dd8/",
            "ftp://reddit.com/r/pics/",
        ] {
            assert_eq!(reddit_link(url), None, "{url}");
        }
        // Profile links in a channel's About stay pills (see has_dedicated_card).
        assert!(!has_dedicated_card("https://www.reddit.com/r/pics/comments/1x8gy05/x/"));
    }

    #[test]
    fn a_slug_reads_as_words() {
        assert_eq!(
            reddit_slug_title("the_undertaker_immediately_after_being").as_deref(),
            Some("the undertaker immediately after being"),
        );
        assert_eq!(reddit_slug_title("_"), None);
        assert_eq!(reddit_slug_title(""), None);
    }

    #[test]
    fn oembed_gives_title_author_and_the_real_subreddit() {
        // Trimmed from the live answer for /r/all/comments/92dd8/.
        let json = serde_json::json!({
            "author_name": "qgyh2",
            "html": "<blockquote class=\"reddit-embed-bq\" style=\"height:240px\" >\n<a href=\"https://www.reddit.com/r/pics/comments/92dd8/test_post_please_ignore/\">test post please ignore</a><br> by\n<a href=\"https://www.reddit.com/user/qgyh2/\">u/qgyh2</a> in\n<a href=\"https://www.reddit.com/r/pics/\">pics</a>\n</blockquote>",
            "provider_name": "reddit",
            "title": "test post please ignore",
            "type": "rich",
        });
        assert_eq!(
            parse_reddit_oembed(&json),
            RedditPost {
                title: Some("test post please ignore".into()),
                author: Some("qgyh2".into()),
                subreddit: Some("pics".into()),
            },
        );
        let deleted = serde_json::json!({ "author_name": "[deleted]", "title": "  " });
        assert_eq!(parse_reddit_oembed(&deleted), RedditPost::default());
    }

    /// The head of a real post feed: the post entry, then a comment entry.
    const FEED: &str = r#"<?xml version="1.0" encoding="UTF-8"?><feed xmlns="http://www.w3.org/2005/Atom" xmlns:media="http://search.yahoo.com/mrss/"><category term="pics" label="r/pics"/><logo>https://b.thumbs.redditmedia.com/1zT3.png</logo><title>The Undertaker immediately after being accidentally lit on fire in 2010. : pics</title><entry><author><name>/u/Objectionne</name></author><category term="pics" label="r/pics"/><content type="html">&lt;a href=&quot;https://i.redd.it/2nwb8pw6h2wh1.png&quot;&gt;[link]&lt;/a&gt;</content><id>t3_1x8gy05</id><media:thumbnail url="https://preview.redd.it/2nwb8pw6h2wh1.png?width=640&amp;crop=smart&amp;auto=webp&amp;s=30d4357560e133a60a9234d8e7055fdbd0831233" /><title>The Undertaker immediately after being accidentally lit on fire in 2010.</title></entry><entry><author><name>/u/someone</name></author><media:thumbnail url="https://preview.redd.it/comment.jpg?width=640&amp;s=2" /></entry>"#;

    #[test]
    fn the_feed_gives_the_posts_own_preview_image() {
        assert_eq!(
            parse_reddit_feed_image(FEED).as_deref(),
            Some("https://preview.redd.it/2nwb8pw6h2wh1.png?width=640&crop=smart&auto=webp&s=30d4357560e133a60a9234d8e7055fdbd0831233"),
        );
    }

    #[test]
    fn the_feed_image_is_skipped_when_small_foreign_or_on_a_comment() {
        let swap = |from: &str, to: &str| FEED.replacen(from, to, 1);
        // A 140 px list thumbnail would stretch across the card.
        assert_eq!(parse_reddit_feed_image(&swap("width=640&amp;crop", "width=140&amp;height=105&amp;crop")), None);
        // Never an image from a host Reddit does not run.
        assert_eq!(parse_reddit_feed_image(&swap("https://preview.redd.it/2nwb", "https://grabber.example/2nwb")), None);
        // A text post: its entry has no thumbnail, and a comment's does not count.
        let text_post = swap(r#"<media:thumbnail url="https://preview.redd.it/2nwb"#, r#"<media:other url="https://preview.redd.it/2nwb"#);
        assert_eq!(parse_reddit_feed_image(&text_post), None);
        assert_eq!(parse_reddit_feed_image("<feed></feed>"), None);
    }

    #[test]
    fn reddit_cards_carry_rust_written_labels() {
        let card = reddit_card(
            "https://redd.it/92dd8",
            Some("test post please ignore".into()),
            Some("pics".into()),
            Some("qgyh2".into()),
            None,
        );
        assert_eq!(card.kind, "reddit");
        assert_eq!(card.site_name.as_deref(), Some("r/pics"));
        assert_eq!(card.author.as_deref(), Some("u/qgyh2"));
        let bare = reddit_card("https://www.reddit.com/user/spez", Some("u/spez".into()), None, None, None);
        assert_eq!(bare.site_name.as_deref(), Some("Reddit"));
    }

    #[test]
    fn unresolved_share_and_video_links_say_only_what_the_link_says() {
        let share = unresolved_reddit_card("https://www.reddit.com/r/pics/s/x", Some("pics".into()), false).unwrap();
        assert_eq!((share.title, share.site_name.as_deref()), (None, Some("r/pics")));
        let video = unresolved_reddit_card("https://v.redd.it/x", None, true).unwrap();
        assert_eq!((video.title, video.site_name.as_deref()), (None, Some("Reddit video")));
        assert!(unresolved_reddit_card("https://v.redd.it/x", None, false).is_err());
    }

    #[tokio::test]
    async fn subreddit_and_user_links_build_a_card_without_fetching() {
        let sub = build_preview("https://www.reddit.com/r/xqcow/").await.expect("a card");
        assert_eq!((sub.kind.as_str(), sub.title.as_deref()), ("reddit", Some("r/xqcow")));
        let user = build_preview("https://www.reddit.com/user/spez").await.expect("a card");
        assert_eq!((user.kind.as_str(), user.title.as_deref()), ("reddit", Some("u/spez")));
    }

    /// Live: `cargo test --no-default-features --lib reddit_post_live -- --ignored`.
    #[tokio::test]
    #[ignore = "network"]
    async fn reddit_post_live() {
        let card = build_preview("https://redd.it/1x8gy05").await.expect("a card");
        println!("{card:#?}");
        assert_eq!(card.kind, "reddit");
        assert_eq!(card.site_name.as_deref(), Some("r/pics"));
        assert!(card.title.as_deref().is_some_and(|t| t.starts_with("The Undertaker")), "{card:?}");
        assert_eq!(card.author.as_deref(), Some("u/Objectionne"));
    }

    #[tokio::test]
    async fn the_reported_link_previews_as_its_image_without_fetching() {
        let preview = build_preview(REDDIT_VIEWER).await.expect("an image card");
        assert_eq!(preview.kind, "image");
        assert_eq!(preview.image.as_deref(), Some("https://i.redd.it/tdy7wdh3gvod1.jpg"));
        // The card still opens what was posted.
        assert_eq!(preview.url, REDDIT_VIEWER);
    }
}
