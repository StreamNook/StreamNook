use log::{debug, warn};
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// Use Twitch web client ID - works without authentication for read operations
const WEB_CLIENT_ID: &str = env!("TWITCH_WEB_CLIENT_ID");
const GQL_URL: &str = "https://gql.twitch.tv/gql";

/// Sanitizer for panel descriptions.
///
/// A panel description is HTML the STREAMER wrote, and the About drawer renders
/// it with `dangerouslySetInnerHTML`. Anything that survives here executes in the
/// main webview with every app command available to it, so an unsanitized
/// `<img src=x onerror=...>` in any channel's panel would run as the viewer
/// (Helix reads through `helix_get`, chat actions, settings; until 2026-09-07
/// the raw Twitch token too). Sanitizing HERE rather than in the component keeps one
/// choke point: every consumer of this command gets the cleaned string.
///
/// The allowlist is narrower than ammonia's default (no tables, no class/style):
/// text formatting, headings, quotes, code, rules, links and images, which is
/// everything the panel editor's markdown can produce.
fn panel_sanitizer() -> &'static ammonia::Builder<'static> {
    static SANITIZER: once_cell::sync::Lazy<ammonia::Builder<'static>> =
        once_cell::sync::Lazy::new(|| {
            let mut b = ammonia::Builder::empty();
            b.tags(std::collections::HashSet::from([
                "a", "b", "strong", "i", "em", "u", "s", "del", "br", "p", "span", "ul", "ol",
                "li", "h1", "h2", "h3", "h4", "h5", "h6", "blockquote", "code", "pre", "hr",
                "img", "center", "sub", "sup", "small",
            ]))
            .link_rel(Some("noopener noreferrer"))
            // href/src only, and url_schemes below restricts them to http/https,
            // so javascript: and data: URLs cannot survive; relative URLs would
            // resolve against the app's own origin, so they go too.
            .tag_attributes(std::collections::HashMap::from([
                ("a", std::collections::HashSet::from(["href"])),
                ("img", std::collections::HashSet::from(["src", "alt"])),
            ]))
            .url_schemes(std::collections::HashSet::from(["http", "https"]))
            .url_relative(ammonia::UrlRelative::Deny);
            b
        });
    &SANITIZER
}

/// Panel descriptions are markdown that may carry inline HTML, as twitch.tv
/// renders them. A single newline is a line break there, not a soft wrap.
fn panel_markdown_to_html(src: &str) -> String {
    use pulldown_cmark::{html, Event, Options, Parser};
    let options = Options::ENABLE_STRIKETHROUGH;
    let events = Parser::new_ext(src, options).map(|e| match e {
        Event::SoftBreak => Event::HardBreak,
        other => other,
    });
    let mut out = String::with_capacity(src.len() * 3 / 2);
    html::push_html(&mut out, events);
    out
}

fn sanitize_panel_html(raw: Option<String>) -> Option<String> {
    raw.map(|src| {
        panel_sanitizer()
            .clean(&panel_markdown_to_html(&src))
            .to_string()
    })
}

#[cfg(test)]
mod panel_sanitizer_tests {
    use super::sanitize_panel_html;

    fn clean(s: &str) -> String {
        sanitize_panel_html(Some(s.to_string())).unwrap()
    }

    #[test]
    fn strips_script_and_event_handlers() {
        let out = clean(r#"<img src=x onerror="alert(1)"><script>alert(2)</script>hi"#);
        assert!(!out.contains("onerror"), "event handler survived: {out}");
        assert!(!out.contains("script"), "script tag survived: {out}");
        assert!(!out.contains("alert"), "script body survived: {out}");
        assert!(out.contains("hi"), "text was dropped: {out}");
    }

    #[test]
    fn rejects_non_http_url_schemes() {
        let out = clean(r#"<a href="javascript:alert(1)">x</a>"#);
        assert!(!out.contains("javascript"), "javascript: survived: {out}");
        let out = clean(r#"<a href="data:text/html;base64,PHNjcmlwdD4=">x</a>"#);
        assert!(!out.contains("data:"), "data: URL survived: {out}");
    }

    #[test]
    fn keeps_ordinary_formatting_and_links() {
        let out = clean(r#"<p><b>Bold</b> and <a href="https://example.com">a link</a></p>"#);
        assert!(out.contains("<b>Bold</b>"), "lost bold: {out}");
        assert!(out.contains(r#"href="https://example.com""#), "lost href: {out}");
        assert!(out.contains("noopener"), "missing rel hardening: {out}");
    }

    #[test]
    fn escapes_rather_than_drops_unknown_markup() {
        // A stray angle bracket must not be able to open a tag downstream.
        let out = clean("5 < 6 and 7 > 2");
        assert!(!out.contains("< 6"), "raw angle bracket survived: {out}");
    }

    #[test]
    fn renders_markdown() {
        let out = clean("# Rules\n**Be kind** and *chill*\n[Discord](https://discord.gg/x)\n- one\n- two");
        assert!(out.contains("<h1>Rules</h1>"), "lost heading: {out}");
        assert!(out.contains("<strong>Be kind</strong>"), "lost bold: {out}");
        assert!(out.contains("<em>chill</em>"), "lost italic: {out}");
        assert!(out.contains(r#"href="https://discord.gg/x""#), "lost link: {out}");
        assert!(out.contains("<li>one</li>"), "lost list: {out}");
        assert!(!out.contains("**"), "markdown left raw: {out}");
    }

    #[test]
    fn single_newlines_break_lines() {
        let out = clean("line one\nline two");
        assert!(out.contains("<br>"), "newline lost: {out}");
    }

    #[test]
    fn inline_html_inside_markdown_survives_the_allowlist() {
        let out = clean("Hello <b>there</b>\n\n<center>Schedule</center>");
        assert!(out.contains("<b>there</b>"), "lost inline html: {out}");
        assert!(out.contains("<center>Schedule</center>"), "lost block html: {out}");
    }

    #[test]
    fn images_keep_only_https_sources() {
        let out = clean("![logo](https://example.com/a.png) ![x](x.png) <img src=\"javascript:alert(1)\">");
        assert!(out.contains(r#"src="https://example.com/a.png""#), "lost https image: {out}");
        assert!(!out.contains("x.png"), "relative src survived: {out}");
        assert!(!out.contains("javascript"), "javascript src survived: {out}");
    }
}

/// Create headers for GQL requests (no auth required for read operations)
fn create_gql_headers() -> HeaderMap {
    // Stable per-install device id: Twitch keys anonymous recommendation
    // context and rate-limit buckets off X-Device-Id.
    let device_id = crate::services::twitch_service::gql_device_id();
    let session_id = Uuid::new_v4().to_string().replace("-", "");

    let mut headers = HeaderMap::new();
    headers.insert("Client-ID", HeaderValue::from_static(WEB_CLIENT_ID));
    headers.insert(ACCEPT, HeaderValue::from_static("*/*"));
    headers.insert("X-Device-Id", HeaderValue::from_str(&device_id).unwrap());
    headers.insert(
        "Client-Session-Id",
        HeaderValue::from_str(&session_id).unwrap(),
    );
    headers
}

// ============================================================================
// GQL RESPONSE STRUCTS
// ============================================================================

#[derive(Debug, Deserialize)]
struct GqlPanelsResponse {
    data: Option<GqlPanelsData>,
    /// GQL answers partially on a resolver failure: `data` still arrives with the
    /// failed field nulled (social links vanish without a trace), so these are
    /// logged rather than ignored.
    #[serde(default)]
    errors: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct GqlPanelsData {
    user: Option<GqlPanelsUser>,
}

#[derive(Debug, Deserialize)]
struct GqlPanelsUser {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    description: Option<String>,
    panels: Option<Vec<GqlPanel>>,
    #[serde(default)]
    followers: Option<GqlFollowers>,
    #[serde(rename = "profileImageURL", default)]
    profile_image_url: Option<String>,
    #[serde(rename = "displayName", default)]
    display_name: Option<String>,
    #[serde(rename = "broadcastSettings", default)]
    broadcast_settings: Option<GqlBroadcastSettings>,
    /// Social media data lives on user.channel, not user directly
    #[serde(default)]
    channel: Option<GqlChannel>,
}

#[derive(Debug, Deserialize)]
struct GqlPanel {
    id: Option<String>,
    #[serde(rename = "type")]
    panel_type: Option<String>,
    title: Option<String>,
    description: Option<String>,
    #[serde(rename = "imageURL")]
    image_url: Option<String>,
    #[serde(rename = "linkURL")]
    link_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GqlChannel {
    #[serde(rename = "socialMedias", default)]
    social_medias: Option<Vec<GqlSocialMedia>>,
}

#[derive(Debug, Deserialize)]
struct GqlSocialMedia {
    name: Option<String>,
    title: Option<String>,
    url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GqlFollowers {
    #[serde(rename = "totalCount")]
    total_count: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct GqlBroadcastSettings {
    title: Option<String>,
    game: Option<GqlGame>,
}

#[derive(Debug, Deserialize)]
struct GqlGame {
    #[serde(rename = "displayName")]
    display_name: Option<String>,
}

// ============================================================================
// PUBLIC API STRUCTS (sent to frontend)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelPanel {
    pub id: String,
    pub panel_type: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub image_url: Option<String>,
    pub link_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SocialMediaLink {
    /// Twitch's own key, which is just the link's domain label (`linktr`,
    /// `steamcommunity`, `sodapoppin`), so it cannot pick a logo.
    pub name: String,
    /// The streamer's label, with a bare platform name ("twitter", "REDDIT")
    /// replaced by the platform's proper name.
    pub title: String,
    pub url: String,
    /// Which logo to draw, from [`social_platform`]: a brand key, or one of the
    /// generic kinds `merch`, `tip`, `links`, `website`.
    #[serde(default)]
    pub platform: String,
    /// The link preview has a dedicated card for this URL (a Discord invite, a
    /// YouTube channel, an X or Instagram profile, ...), so it is worth showing
    /// as one. Links without one stay a compact logo pill.
    #[serde(default)]
    pub card: bool,
}

/// Brand key, proper name, the hosts that belong to it, and the bare titles
/// streamers type for it. Hosts match exactly or as a parent domain.
const SOCIAL_BRANDS: &[(&str, &str, &[&str], &[&str])] = &[
    ("x", "X", &["x.com", "twitter.com"], &["twitter", "x (twitter)", "twitter / x", "twitter/x", "x/twitter"]),
    ("instagram", "Instagram", &["instagram.com", "instagr.am"], &["ig", "insta"]),
    ("youtube", "YouTube", &["youtube.com", "youtu.be"], &["yt"]),
    ("tiktok", "TikTok", &["tiktok.com"], &["tik tok"]),
    ("discord", "Discord", &["discord.gg", "discord.com", "discordapp.com"], &[]),
    ("reddit", "Reddit", &["reddit.com", "redd.it"], &["subreddit"]),
    ("facebook", "Facebook", &["facebook.com", "fb.com", "fb.me"], &["fb"]),
    ("twitch", "Twitch", &["twitch.tv"], &[]),
    ("kick", "Kick", &["kick.com"], &[]),
    ("snapchat", "Snapchat", &["snapchat.com"], &["snap"]),
    ("steam", "Steam", &["steamcommunity.com", "steampowered.com"], &[]),
    ("spotify", "Spotify", &["spotify.com", "spotify.link"], &[]),
    ("linktree", "Linktree", &["linktr.ee"], &[]),
    ("patreon", "Patreon", &["patreon.com"], &[]),
    ("kofi", "Ko-fi", &["ko-fi.com"], &["kofi"]),
    ("bluesky", "Bluesky", &["bsky.app"], &["bsky"]),
    ("threads", "Threads", &["threads.net", "threads.com"], &[]),
    ("github", "GitHub", &["github.com"], &[]),
];

/// Tip pages and link-in-bio pages carry no brand worth showing; they get a
/// generic icon that says what the link is for.
const TIP_HOSTS: &[&str] = &["streamlabs.com", "streamelements.com", "tipeeestream.com"];
const LINK_HUB_HOSTS: &[&str] = &[
    "beacons.ai", "snipfeed.co", "carrd.co", "bio.link", "allmylinks.com", "solo.to", "lnk.bio",
];

fn host_of(url: &str) -> Option<String> {
    let host = reqwest::Url::parse(url.trim()).ok()?.host_str()?.to_ascii_lowercase();
    Some(host.strip_prefix("www.").map(str::to_string).unwrap_or(host))
}

fn host_matches(host: &str, domains: &[&str]) -> bool {
    domains
        .iter()
        .any(|d| host == *d || host.ends_with(&format!(".{d}")))
}

/// The logo key for a social link: its brand from the URL host, else a generic
/// kind read from the host or the streamer's title.
fn social_platform(url: &str, title: &str) -> &'static str {
    if let Some(host) = host_of(url) {
        if let Some((key, ..)) = SOCIAL_BRANDS.iter().find(|(_, _, hosts, _)| host_matches(&host, hosts)) {
            return key;
        }
        if host_matches(&host, TIP_HOSTS) {
            return "tip";
        }
        if host_matches(&host, LINK_HUB_HOSTS) {
            return "links";
        }
    }
    let title = title.to_lowercase();
    if ["merch", "shop", "store"].iter().any(|w| title.contains(w)) {
        "merch"
    } else if ["tip", "donat"].iter().any(|w| title.contains(w)) {
        "tip"
    } else {
        "website"
    }
}

/// The label to show: the streamer's own title, unless it is only the
/// platform's name in some spelling, which becomes the proper name.
fn social_label(title: &str, platform: &str, url: &str) -> String {
    let trimmed = title.trim();
    let brand = SOCIAL_BRANDS.iter().find(|(key, ..)| *key == platform);
    if let Some((key, proper, _, aliases)) = brand {
        let lower = trimmed.to_lowercase();
        if lower.is_empty() || lower == *key || lower == proper.to_lowercase() || aliases.contains(&lower.as_str()) {
            return proper.to_string();
        }
    }
    if trimmed.is_empty() {
        return host_of(url).unwrap_or_else(|| url.to_string());
    }
    trimmed.to_string()
}

#[cfg(test)]
mod social_link_tests {
    use super::{social_label, social_platform};

    #[test]
    fn platform_comes_from_the_host() {
        assert_eq!(social_platform("https://twitter.com/KaiCenat", "twitter"), "x");
        assert_eq!(social_platform("https://x.com/loltyler1?mx=2", "twitter"), "x");
        assert_eq!(social_platform("https://Youtube.com/CohhCarnage", "Cohh on YouTube"), "youtube");
        assert_eq!(social_platform("https://youtube.com/@kaicenatlive?si=x", "Youtube Live"), "youtube");
        assert_eq!(social_platform("https://www.instagram.com/kaicenat/?hl=en", "Instagram"), "instagram");
        assert_eq!(social_platform("https://steamcommunity.com/groups/x", "on Steam"), "steam");
        assert_eq!(social_platform("https://linktr.ee/nmplol", "All links"), "linktree");
        assert_eq!(social_platform("https://www.discord.gg/abc", "Discord"), "discord");
    }

    #[test]
    fn unbranded_links_get_a_kind() {
        assert_eq!(social_platform("https://streamlabs.com/fanumtv/tip#/", "TTS"), "tip");
        assert_eq!(social_platform("https://snipfeed.co/amouranthtoo", "The BEST Links"), "links");
        assert_eq!(social_platform("https://sodapoppin.shop", "Merch store"), "merch");
        assert_eq!(social_platform("https://www.cohhilition.com", "The Cohhilition Website"), "website");
        assert_eq!(social_platform("not a url", "Donate here"), "tip");
    }

    #[test]
    fn bare_platform_titles_become_proper_names() {
        assert_eq!(social_label("twitter", "x", ""), "X");
        assert_eq!(social_label("REDDIT", "reddit", ""), "Reddit");
        assert_eq!(social_label(" youtube ", "youtube", ""), "YouTube");
        assert_eq!(social_label("", "kofi", ""), "Ko-fi");
    }

    #[test]
    fn profile_links_with_a_card_are_flagged() {
        use crate::commands::link_preview::has_dedicated_card;
        assert!(has_dedicated_card("https://www.instagram.com/kaicenat/?hl=en"));
        assert!(has_dedicated_card("https://twitter.com/KaiCenat"));
        assert!(has_dedicated_card("https://www.youtube.com/c/KaiCenat"));
        assert!(has_dedicated_card("https://youtube.com/@kaicenatlive?si=auACMqFfcaTcLJXl"));
        assert!(has_dedicated_card("https://discord.gg/kaicenat"));
        assert!(!has_dedicated_card("https://www.reddit.com/r/xqcow/"));
        assert!(!has_dedicated_card("https://linktr.ee/nmplol"));
    }

    #[test]
    fn streamer_titles_are_kept() {
        assert_eq!(social_label("Youtube Live", "youtube", ""), "Youtube Live");
        assert_eq!(social_label("🧡 Reddit", "reddit", ""), "🧡 Reddit");
        assert_eq!(social_label("", "website", "https://www.tarik.gg/x"), "tarik.gg");
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelAboutData {
    /// The channel owner's Twitch user id (their 7TV cosmetics id).
    #[serde(default)]
    pub user_id: Option<String>,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub profile_image_url: Option<String>,
    pub follower_count: Option<i64>,
    pub panels: Vec<ChannelPanel>,
    pub social_links: Vec<SocialMediaLink>,
    pub stream_title: Option<String>,
    pub game_name: Option<String>,
}

// ============================================================================
// TAURI COMMAND
// ============================================================================

/// Fetch channel about data (panels, social links, description) via GQL
/// Uses inline query — no persisted query hash required
#[tauri::command]
pub async fn get_channel_about_data(channel_login: String) -> Result<ChannelAboutData, String> {
    debug!(
        "[ChannelPanels] Fetching about data for channel: {}",
        channel_login
    );

    let client = crate::services::http::client().clone();

    // Inline GQL query for channel panels + social media
    // - Panel is a union type — use inline fragment for DefaultPanel fields
    // - Social media data lives on user.channel.socialMedias, not user.socialMedias
    let query = r#"
        query ChannelPanels($login: String!) {
            user(login: $login) {
                id
                displayName
                description
                profileImageURL(width: 300)
                followers {
                    totalCount
                }
                panels {
                    id
                    type
                    ... on DefaultPanel {
                        title
                        description
                        imageURL
                        linkURL
                    }
                }
                channel {
                    socialMedias {
                        name
                        title
                        url
                    }
                }
                broadcastSettings {
                    title
                    game {
                        displayName
                    }
                }
            }
        }
    "#;

    let request_body = serde_json::json!({
        "operationName": "ChannelPanels",
        "query": query,
        "variables": {
            "login": channel_login.to_lowercase()
        }
    });

    let response = client
        .post(GQL_URL)
        .headers(create_gql_headers())
        .json(&request_body)
        .send()
        .await
        .map_err(|e| format!("Failed to send GQL request: {}", e))?;

    if !response.status().is_success() {
        return Err(format!(
            "GQL request failed with status: {}",
            response.status()
        ));
    }

    let response_text = response
        .text()
        .await
        .map_err(|e| format!("Failed to read GQL response: {}", e))?;

    let gql_response: GqlPanelsResponse = serde_json::from_str(&response_text).map_err(|e| {
        format!(
            "Failed to parse GQL response: {} - Raw: {}",
            e,
            &response_text[..500.min(response_text.len())]
        )
    })?;

    if let Some(errors) = &gql_response.errors {
        warn!("[ChannelPanels] GQL errors for {}: {}", channel_login, errors);
    }

    let user = gql_response
        .data
        .and_then(|d| d.user)
        .ok_or_else(|| format!("No user data found for channel: {}", channel_login))?;

    // Transform panels
    let panels: Vec<ChannelPanel> = user
        .panels
        .unwrap_or_default()
        .into_iter()
        .filter_map(|p| {
            let id = p.id?;
            let panel_type = p.panel_type.unwrap_or_else(|| "DEFAULT".to_string());
            Some(ChannelPanel {
                id,
                panel_type,
                title: p.title,
                description: sanitize_panel_html(p.description),
                image_url: p.image_url,
                link_url: p.link_url,
            })
        })
        .collect();

    // Transform social links (from user.channel.socialMedias)
    let social_links: Vec<SocialMediaLink> = user
        .channel
        .and_then(|c| c.social_medias)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|s| {
            let name = s.name?;
            let url = s.url?;
            let raw_title = s.title.unwrap_or_default();
            let platform = social_platform(&url, &raw_title);
            let title = social_label(&raw_title, platform, &url);
            let card = crate::commands::link_preview::has_dedicated_card(&url);
            Some(SocialMediaLink {
                name,
                title,
                url,
                platform: platform.to_string(),
                card,
            })
        })
        .collect();

    // Extract follower count
    let follower_count = user.followers.and_then(|f| f.total_count);

    // Extract stream info
    let (stream_title, game_name) = if let Some(bs) = user.broadcast_settings {
        (bs.title, bs.game.and_then(|g| g.display_name))
    } else {
        (None, None)
    };

    debug!(
        "[ChannelPanels] Fetched {} panels, {} social links for {}",
        panels.len(),
        social_links.len(),
        channel_login
    );

    Ok(ChannelAboutData {
        user_id: user.id,
        display_name: user.display_name,
        description: user.description,
        profile_image_url: user.profile_image_url,
        follower_count,
        panels,
        social_links,
        stream_title,
        game_name,
    })
}

// ============================================================================
// VIEWERS ALSO WATCH (similar channels)
// ============================================================================

/// One live channel from Twitch's "Viewers of X also watch" recommendation
/// set (GQL personalSections SIMILAR_SECTION).
#[derive(Debug, Clone, Serialize)]
pub struct SimilarChannel {
    pub user_id: String,
    pub user_login: String,
    pub display_name: String,
    pub profile_image_url: Option<String>,
    pub viewer_count: u32,
    pub game_name: Option<String>,
    /// Templated preview URL ("...{width}x{height}.jpg"), same shape the GQL
    /// browse paths return; the frontend fills the template.
    pub thumbnail_url: Option<String>,
}

/// Fetch the live channels Twitch shows under "Viewers of X also watch".
/// Anonymous, best-effort: every failure mode returns an empty list so the
/// UI can simply hide the section.
#[tauri::command]
pub async fn get_similar_channels(channel_login: String) -> Result<Vec<SimilarChannel>, String> {
    let login = channel_login.trim().to_lowercase();
    // Twitch login charset; also makes inlining into the query injection-proof.
    if login.is_empty()
        || login.len() > 25
        || !login
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        return Ok(Vec::new());
    }

    // Selecting the section title tokens can server-error, so the label is
    // hardcoded client-side and only the items are selected here.
    let query = format!(
        r#"query {{ personalSections(input: {{
            sectionInputs: [SIMILAR_SECTION],
            recommendationContext: {{ platform: "web", clientApp: "twilight", location: "channel_home" }},
            contextChannelName: "{login}" }}) {{
          type
          items {{ ... on PersonalSectionChannel {{
            user {{ id login displayName profileImageURL(width: 70) }}
            content {{ __typename ... on Stream {{ id viewersCount previewImageURL game {{ name displayName }} }} }}
          }} }}
        }} }}"#
    );

    let client = crate::services::http::client_unbounded();
    let response = client
        .post(GQL_URL)
        .headers(create_gql_headers())
        .json(&serde_json::json!({ "query": query }))
        .send()
        .await;

    let json: serde_json::Value = match response {
        Ok(resp) => match resp.json().await {
            Ok(j) => j,
            Err(e) => {
                debug!("[SimilarChannels] parse failed for {}: {}", login, e);
                return Ok(Vec::new());
            }
        },
        Err(e) => {
            debug!("[SimilarChannels] request failed for {}: {}", login, e);
            return Ok(Vec::new());
        }
    };

    if let Some(errors) = json.get("errors") {
        debug!("[SimilarChannels] GQL errors for {}: {}", login, errors);
        return Ok(Vec::new());
    }

    let mut channels = Vec::new();
    let sections = json
        .pointer("/data/personalSections")
        .and_then(|s| s.as_array())
        .cloned()
        .unwrap_or_default();
    for section in sections {
        let items = section
            .get("items")
            .and_then(|i| i.as_array())
            .cloned()
            .unwrap_or_default();
        for item in items {
            let content = item.get("content").filter(|c| !c.is_null());
            let is_live = content
                .and_then(|c| c.get("__typename"))
                .and_then(|t| t.as_str())
                .map(|t| t == "Stream")
                .unwrap_or(false);
            if !is_live {
                continue;
            }
            let user = match item.get("user").filter(|u| !u.is_null()) {
                Some(u) => u,
                None => continue,
            };
            let user_login = user
                .get("login")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if user_login.is_empty() {
                continue;
            }
            let display_name = user
                .get("displayName")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or(&user_login)
                .to_string();
            channels.push(SimilarChannel {
                user_id: user
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                user_login,
                display_name,
                profile_image_url: user
                    .get("profileImageURL")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                viewer_count: content
                    .and_then(|c| c.get("viewersCount"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32,
                game_name: content
                    .and_then(|c| c.get("game"))
                    .filter(|g| !g.is_null())
                    .and_then(|g| {
                        g.get("displayName")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                            .or_else(|| g.get("name").and_then(|v| v.as_str()))
                    })
                    .map(|s| s.to_string()),
                thumbnail_url: content
                    .and_then(|c| c.get("previewImageURL"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
            });
        }
    }

    debug!(
        "[SimilarChannels] {} live similar channels for {}",
        channels.len(),
        login
    );
    Ok(channels)
}
