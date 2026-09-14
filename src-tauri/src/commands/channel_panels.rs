use log::debug;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT};
use reqwest::Client;
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
/// The allowlist is deliberately narrower than ammonia's default (no images, no
/// tables, no class/style) because a panel only ever needs text plus links.
fn panel_sanitizer() -> &'static ammonia::Builder<'static> {
    static SANITIZER: once_cell::sync::Lazy<ammonia::Builder<'static>> =
        once_cell::sync::Lazy::new(|| {
            let mut b = ammonia::Builder::empty();
            b.tags(std::collections::HashSet::from([
                "a", "b", "strong", "i", "em", "u", "s", "br", "p", "span", "ul", "ol", "li",
            ]))
            .link_rel(Some("noopener noreferrer"))
            // href only, and url_schemes below restricts it to http/https, so
            // javascript: and data: URLs cannot survive.
            .tag_attributes(std::collections::HashMap::from([(
                "a",
                std::collections::HashSet::from(["href"]),
            )]))
            .url_schemes(std::collections::HashSet::from(["http", "https"]));
            b
        });
    &SANITIZER
}

fn sanitize_panel_html(raw: Option<String>) -> Option<String> {
    raw.map(|html| panel_sanitizer().clean(&html).to_string())
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
}

#[derive(Debug, Deserialize)]
struct GqlPanelsData {
    user: Option<GqlPanelsUser>,
}

#[derive(Debug, Deserialize)]
struct GqlPanelsUser {
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
    pub name: String,
    pub title: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelAboutData {
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
            let title = s.title.unwrap_or_else(|| name.clone());
            Some(SocialMediaLink { name, title, url })
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

    let client = Client::new();
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
