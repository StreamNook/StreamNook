use crate::services::badge_service::BadgeService;
use crate::services::twitch_service::TwitchService;
use log::debug;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::sync::RwLock;

// Cache structures
lazy_static::lazy_static! {
    static ref PROFILE_CACHE: Arc<RwLock<HashMap<String, CachedProfile>>> = Arc::new(RwLock::new(HashMap::new()));
}

const CACHE_DURATION: Duration = Duration::from_secs(300); // 5 minutes

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedProfile {
    profile: UserProfileComplete,
    timestamp: SystemTime,
}

// Main response structure containing all profile data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserProfileComplete {
    // Twitch profile data
    pub twitch_profile: Option<TwitchUserProfile>,

    // Badge data (unified from badge service)
    pub badges: BadgeData,

    // 7TV cosmetics
    pub seventv_cosmetics: Option<SevenTVCosmetics>,

    // IVR data
    pub ivr_data: IVRData,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TwitchUserProfile {
    pub id: String,
    pub login: String,
    pub display_name: String,
    #[serde(rename = "type")]
    pub user_type: String,
    pub broadcaster_type: String,
    pub description: String,
    pub profile_image_url: String,
    pub offline_image_url: String,
    pub view_count: i64,
    pub created_at: String,
    /// Channel header banner URL from twitch.tv profile page (GQL `User.bannerImageURL`).
    /// Distinct from `offline_image_url` (the video player's offline placeholder).
    #[serde(default)]
    pub banner_image_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BadgeData {
    pub display_badges: Vec<Badge>,
    pub earned_badges: Vec<Badge>,
    pub third_party_badges: Vec<ThirdPartyBadge>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Badge {
    pub id: String,
    #[serde(rename = "setID")]
    pub set_id: String,
    pub version: String,
    pub title: String,
    pub description: String,
    pub image1x: String,
    pub image2x: String,
    pub image4x: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThirdPartyBadge {
    pub id: String,
    pub provider: String,
    pub title: String,
    #[serde(rename = "imageUrl")]
    pub image_url: String,
    pub image1x: Option<String>,
    pub image2x: Option<String>,
    pub image4x: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SevenTVCosmetics {
    pub paints: Vec<SevenTVPaint>,
    pub badges: Vec<SevenTVBadge>,
    /// 7TV animated profile picture URL (`style.activeProfilePicture`), if the
    /// user has set a custom one. `None` when they have no 7TV avatar — callers
    /// then fall back to the Twitch profile image. 7TV has no profile *banner*,
    /// only this avatar, so this is the only 7TV image surface for the card.
    #[serde(default)]
    pub avatar_url: Option<String>,
}

// v4 API Paint structure with layers
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SevenTVPaint {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub selected: bool,
    pub data: SevenTVPaintData,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SevenTVPaintData {
    pub layers: Vec<SevenTVPaintLayer>,
    pub shadows: Vec<SevenTVPaintShadow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SevenTVPaintLayer {
    pub id: String,
    pub ty: SevenTVPaintLayerType,
    pub opacity: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "__typename")]
#[allow(clippy::enum_variant_names)] // Variant names must match 7TV API __typename values
pub enum SevenTVPaintLayerType {
    PaintLayerTypeLinearGradient {
        angle: Option<i32>,
        repeating: Option<bool>,
        stops: Option<Vec<SevenTVGradientStop>>,
    },
    PaintLayerTypeRadialGradient {
        shape: Option<String>,
        repeating: Option<bool>,
        stops: Option<Vec<SevenTVGradientStop>>,
    },
    PaintLayerTypeSingleColor {
        color: Option<SevenTVColor>,
    },
    PaintLayerTypeImage {
        images: Option<Vec<SevenTVImage>>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SevenTVGradientStop {
    pub at: f64,
    pub color: SevenTVColor,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SevenTVColor {
    pub hex: String,
    pub r: i32,
    pub g: i32,
    pub b: i32,
    pub a: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SevenTVImage {
    pub url: String,
    pub mime: Option<String>,
    pub size: Option<i64>,
    pub scale: Option<i32>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    #[serde(rename = "frameCount")]
    pub frame_count: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SevenTVPaintShadow {
    #[serde(rename = "offsetX")]
    pub offset_x: f64,
    #[serde(rename = "offsetY")]
    pub offset_y: f64,
    pub blur: f64,
    pub color: SevenTVColor,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SevenTVBadge {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub selected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IVRData {
    pub created_at: Option<String>,
    pub following_since: Option<String>,
    pub status_hidden: bool,
    pub is_subscribed: bool,
    pub sub_streak: Option<i32>,
    pub sub_cumulative: Option<i32>,
    pub is_founder: bool,
    pub is_mod: bool,
    pub mod_since: Option<String>,
    pub is_vip: bool,
    pub vip_since: Option<String>,
    // ISO timestamp of the user's most recent broadcast start, if any. Lets
    // the UI surface "Last live 3d ago" for active streamers and skip the row
    // for users who've never gone live.
    pub last_broadcast_at: Option<String>,
    pub last_broadcast_title: Option<String>,
    // Number of channels this user follows. Universally interesting little
    // stat for the profile card. Note: this is NOT a count of messages or
    // chat activity — Twitch / IVR don't expose lifetime message counts.
    pub follows_count: Option<i32>,
    // True when the account is suspended by Twitch. Worth surfacing on the card
    // because a suspended user's messages still sit in chat history.
    pub banned: bool,
    // How many chatters are currently in THIS user's own channel. Present
    // whether or not they're live, and unrelated to the channel you're viewing.
    pub chatter_count: Option<i32>,
    // Subscription tier ("1" / "2" / "3" / "Prime"), type ("paid" / "prime" /
    // "gift"), and the gifter's login + display name when the sub is a gift.
    // All optional — only present when the user is currently subscribed and
    // IVR's subage `meta` block returned the data.
    pub sub_tier: Option<String>,
    pub sub_type: Option<String>,
    pub sub_gifter_login: Option<String>,
    pub sub_gifter_display_name: Option<String>,
    pub error: Option<String>,
}

/// Fetch complete user profile with all data sources aggregated in parallel
#[tauri::command]
pub async fn get_user_profile_complete(
    user_id: String,
    username: String,
    channel_id: String,
    channel_name: String,
) -> Result<UserProfileComplete, String> {
    // Check cache first
    let cache_key = format!("{}:{}:{}", user_id, username, channel_id);
    {
        let cache = PROFILE_CACHE.read().await;
        if let Some(cached) = cache.get(&cache_key) {
            if cached.timestamp.elapsed().unwrap_or(CACHE_DURATION) < CACHE_DURATION {
                debug!("[UserProfile] Cache hit for: {}", username);
                return Ok(cached.profile.clone());
            }
        }
    }

    debug!(
        "[UserProfile] Fetching complete profile for: {} in channel {}",
        username, channel_name
    );

    // Phase 1: the lookups that key off the Twitch user-id (always authoritative).
    let (twitch_result, badges_result, seventv_result) = tokio::join!(
        fetch_twitch_profile(&user_id),
        fetch_badge_data(&user_id, &username, &channel_id, &channel_name),
        fetch_seventv_cosmetics(&user_id)
    );

    // The banner + IVR endpoints are keyed by LOGIN, but the `username` passed in
    // from chat can be a display name (localized names aren't valid logins), which
    // is exactly what made those queries come back empty. Use the canonical login
    // from Helix; fall back to the passed username only if the Helix fetch failed.
    let lookup_login = twitch_result
        .as_ref()
        .ok()
        .map(|p| p.login.clone())
        .filter(|l| !l.is_empty())
        .unwrap_or_else(|| username.clone());

    // Phase 2: login-keyed lookups, now driven by the canonical login.
    let (banner_result, ivr_result) = tokio::join!(
        fetch_twitch_banner(&lookup_login),
        fetch_ivr_data(&lookup_login, &channel_name)
    );

    let twitch_profile = twitch_result.ok().map(|mut p| {
        if p.banner_image_url.is_none() {
            p.banner_image_url = banner_result.ok().flatten();
        }
        p
    });

    let profile = UserProfileComplete {
        twitch_profile,
        badges: badges_result.unwrap_or_else(|_| BadgeData {
            display_badges: vec![],
            earned_badges: vec![],
            third_party_badges: vec![],
        }),
        seventv_cosmetics: seventv_result.ok(),
        ivr_data: ivr_result.unwrap_or_else(|e| IVRData {
            created_at: None,
            following_since: None,
            status_hidden: false,
            is_subscribed: false,
            sub_streak: None,
            sub_cumulative: None,
            is_founder: false,
            is_mod: false,
            mod_since: None,
            is_vip: false,
            vip_since: None,
            last_broadcast_at: None,
            last_broadcast_title: None,
            follows_count: None,
            banned: false,
            chatter_count: None,
            sub_tier: None,
            sub_type: None,
            sub_gifter_login: None,
            sub_gifter_display_name: None,
            error: Some(e),
        }),
    };

    // Cache the result
    {
        let mut cache = PROFILE_CACHE.write().await;
        cache.insert(
            cache_key,
            CachedProfile {
                profile: profile.clone(),
                timestamp: SystemTime::now(),
            },
        );
    }

    Ok(profile)
}

/// Clear profile cache
#[tauri::command]
pub async fn clear_user_profile_cache() -> Result<(), String> {
    let mut cache = PROFILE_CACHE.write().await;
    cache.clear();
    debug!("[UserProfile] Cache cleared");
    Ok(())
}

/// Clear specific user's profile from cache
#[tauri::command]
pub async fn clear_user_profile_cache_for_user(
    user_id: String,
    username: String,
    channel_id: String,
) -> Result<(), String> {
    let cache_key = format!("{}:{}:{}", user_id, username, channel_id);
    let mut cache = PROFILE_CACHE.write().await;
    cache.remove(&cache_key);
    debug!("[UserProfile] Cache cleared for: {}", username);
    Ok(())
}

// Helper functions for fetching individual data sources

async fn fetch_twitch_profile(user_id: &str) -> Result<TwitchUserProfile, String> {
    let token = TwitchService::get_token()
        .await
        .map_err(|e| format!("Failed to get token: {}", e))?;

    let client_id = env!("TWITCH_APP_CLIENT_ID");

    let client = crate::services::http::client().clone();
    let response = client
        .get(format!("https://api.twitch.tv/helix/users?id={}", user_id))
        .header("Client-ID", client_id)
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .map_err(|e| format!("Twitch API request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("Twitch API error: {}", response.status()));
    }

    #[derive(Deserialize)]
    struct TwitchResponse {
        data: Vec<TwitchUserProfile>,
    }

    let twitch_data: TwitchResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse Twitch response: {}", e))?;

    twitch_data
        .data
        .into_iter()
        .next()
        .ok_or_else(|| "User not found".to_string())
}

/// Fetch the channel page banner URL via anonymous Twitch GQL.
/// Helix's offline_image_url is a different thing (offline video-player placeholder);
/// the actual profile-page header banner is only exposed through GQL.
async fn fetch_twitch_banner(login: &str) -> Result<Option<String>, String> {
    let client = crate::services::http::client().clone();

    let query = r#"
        query StreamNookBanner($login: String!) {
            user(login: $login) {
                bannerImageURL
            }
        }
    "#;

    let body = serde_json::json!({
        "operationName": "StreamNookBanner",
        "query": query,
        "variables": { "login": login.to_lowercase() }
    });

    let response = client
        .post("https://gql.twitch.tv/gql")
        .header("Client-ID", env!("TWITCH_WEB_CLIENT_ID"))
        .header("Accept", "*/*")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Banner GQL request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("Banner GQL error: {}", response.status()));
    }

    let json: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse banner GQL response: {}", e))?;

    Ok(json
        .get("data")
        .and_then(|d| d.get("user"))
        .and_then(|u| u.get("bannerImageURL"))
        .and_then(|b| b.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from))
}

async fn fetch_badge_data(
    user_id: &str,
    username: &str,
    channel_id: &str,
    channel_name: &str,
) -> Result<BadgeData, String> {
    // Use the existing badge service
    let badge_service_lock = crate::commands::badge_service::get_service().await?;

    // Auto-initialize if needed
    {
        let service_guard = badge_service_lock.read().await;
        if service_guard.is_none() {
            drop(service_guard);
            crate::commands::badge_service::initialize_badge_service().await;
        }
    }

    let service_guard = badge_service_lock.read().await;
    let service = service_guard
        .as_ref()
        .ok_or_else(|| "Badge service not initialized".to_string())?;

    let token = TwitchService::get_token()
        .await
        .map_err(|e| format!("Failed to get token: {}", e))?;

    // Use get_user_badges_with_earned to fetch ALL earned badges (not just displayed ones)
    // This includes the global badge collection (all achievements) for profile view
    let badge_response = service
        .get_user_badges_with_earned(user_id, username, channel_id, channel_name, &token)
        .await
        .map_err(|e| format!("Failed to get badges: {}", e))?;

    Ok(BadgeData {
        display_badges: badge_response
            .display_badges
            .into_iter()
            .map(|b| Badge {
                id: b.badge_info.id,
                set_id: b.badge_info.set_id,
                version: b.badge_info.version,
                title: b.badge_info.title,
                description: b.badge_info.description,
                image1x: b.badge_info.image_1x,
                image2x: b.badge_info.image_2x,
                image4x: b.badge_info.image_4x,
            })
            .collect(),
        earned_badges: badge_response
            .earned_badges
            .into_iter()
            .map(|b| Badge {
                id: b.badge_info.id,
                set_id: b.badge_info.set_id,
                version: b.badge_info.version,
                title: b.badge_info.title,
                description: b.badge_info.description,
                image1x: b.badge_info.image_1x,
                image2x: b.badge_info.image_2x,
                image4x: b.badge_info.image_4x,
            })
            .collect(),
        third_party_badges: badge_response
            .third_party_badges
            .into_iter()
            .map(|b| ThirdPartyBadge {
                id: b.badge_info.id,
                // The canonical lowercase id, NOT the Debug form: the page compares
                // this against loadout keys and provider groups. See as_key.
                provider: b.provider.as_key().to_string(),
                title: b.badge_info.title,
                image_url: b.badge_info.image_4x.clone(),
                image1x: Some(b.badge_info.image_1x),
                image2x: Some(b.badge_info.image_2x),
                image4x: Some(b.badge_info.image_4x),
            })
            .collect(),
    })
}

async fn fetch_seventv_cosmetics(user_id: &str) -> Result<SevenTVCosmetics, String> {
    let client = crate::services::http::client().clone();

    // Use the v4 GraphQL API with userByConnection query
    let query = format!(
        r#"{{ 
            users {{
                userByConnection(platform: TWITCH, platformId: "{}") {{
                    id
                    style {{
                        activePaint {{ id }}
                        activeBadge {{ id }}
                        activeProfilePicture {{
                            images {{
                                url
                                mime
                                scale
                                frameCount
                            }}
                        }}
                    }}
                    inventory {{
                        paints {{
                            to {{
                                paint {{
                                    id
                                    name
                                    description
                                    data {{
                                        layers {{
                                            id
                                            ty {{
                                                ... on PaintLayerTypeImage {{
                                                    __typename
                                                    images {{
                                                        url
                                                        mime
                                                        size
                                                        scale
                                                        width
                                                        height
                                                        frameCount
                                                    }}
                                                }}
                                                ... on PaintLayerTypeRadialGradient {{
                                                    __typename
                                                    repeating
                                                    shape
                                                    stops {{
                                                        at
                                                        color {{
                                                            hex
                                                            r
                                                            g
                                                            b
                                                            a
                                                        }}
                                                    }}
                                                }}
                                                ... on PaintLayerTypeLinearGradient {{
                                                    __typename
                                                    angle
                                                    repeating
                                                    stops {{
                                                        at
                                                        color {{
                                                            hex
                                                            r
                                                            g
                                                            b
                                                            a
                                                        }}
                                                    }}
                                                }}
                                                ... on PaintLayerTypeSingleColor {{
                                                    __typename
                                                    color {{
                                                        hex
                                                        r
                                                        g
                                                        b
                                                        a
                                                    }}
                                                }}
                                            }}
                                            opacity
                                        }}
                                        shadows {{
                                            offsetX
                                            offsetY
                                            blur
                                            color {{
                                                hex
                                                r
                                                g
                                                b
                                                a
                                            }}
                                        }}
                                    }}
                                }}
                            }}
                        }}
                        badges {{
                            to {{
                                badge {{
                                    id
                                    name
                                    description
                                }}
                            }}
                        }}
                    }}
                }}
            }}
        }}"#,
        user_id
    );

    // Remove newlines and extra spaces for cleaner query
    let clean_query: String = query
        .replace('\n', "")
        .split_whitespace()
        .collect::<Vec<&str>>()
        .join(" ");

    let response = client
        .post("https://7tv.io/v4/gql")
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({ "query": clean_query }))
        .send()
        .await
        .map_err(|e| format!("7TV API request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("7TV API error: {}", response.status()));
    }

    let json: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse 7TV response: {}", e))?;

    // Check for errors
    if let Some(errors) = json.get("errors") {
        if let Some(arr) = errors.as_array() {
            if !arr.is_empty() {
                return Err(format!("7TV API error: {:?}", errors));
            }
        }
    }

    // Parse the v4 response structure
    let user_data = json
        .get("data")
        .and_then(|d| d.get("users"))
        .and_then(|u| u.get("userByConnection"))
        .ok_or_else(|| "7TV user not found".to_string())?;

    // Get selected paint and badge IDs from style
    let selected_paint_id = user_data
        .get("style")
        .and_then(|s| s.get("activePaint"))
        .and_then(|p| p.get("id"))
        .and_then(|id| id.as_str())
        .unwrap_or("");

    let selected_badge_id = user_data
        .get("style")
        .and_then(|s| s.get("activeBadge"))
        .and_then(|b| b.get("id"))
        .and_then(|id| id.as_str())
        .unwrap_or("");

    // Parse paints from inventory
    let paints: Vec<SevenTVPaint> = user_data
        .get("inventory")
        .and_then(|inv| inv.get("paints"))
        .and_then(|p| p.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|paint_wrapper| {
                    let paint = paint_wrapper.get("to")?.get("paint")?;
                    let id = paint.get("id")?.as_str()?;

                    // Parse layers
                    let layers: Vec<SevenTVPaintLayer> = paint
                        .get("data")
                        .and_then(|d| d.get("layers"))
                        .and_then(|l| l.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|layer| parse_paint_layer(layer))
                                .collect()
                        })
                        .unwrap_or_default();

                    // Parse shadows
                    let shadows: Vec<SevenTVPaintShadow> = paint
                        .get("data")
                        .and_then(|d| d.get("shadows"))
                        .and_then(|s| s.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|shadow| {
                                    Some(SevenTVPaintShadow {
                                        offset_x: shadow.get("offsetX")?.as_f64()?,
                                        offset_y: shadow.get("offsetY")?.as_f64()?,
                                        blur: shadow.get("blur")?.as_f64()?,
                                        color: parse_color(shadow.get("color")?)?,
                                    })
                                })
                                .collect()
                        })
                        .unwrap_or_default();

                    Some(SevenTVPaint {
                        id: id.to_string(),
                        name: paint
                            .get("name")
                            .and_then(|n| n.as_str())
                            .unwrap_or("")
                            .to_string(),
                        description: paint
                            .get("description")
                            .and_then(|d| d.as_str())
                            .map(String::from),
                        selected: id == selected_paint_id,
                        data: SevenTVPaintData { layers, shadows },
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    // Parse badges from inventory
    let badges: Vec<SevenTVBadge> = user_data
        .get("inventory")
        .and_then(|inv| inv.get("badges"))
        .and_then(|b| b.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|badge_wrapper| {
                    let badge = badge_wrapper.get("to")?.get("badge")?;
                    let id = badge.get("id")?.as_str()?;
                    Some(SevenTVBadge {
                        id: id.to_string(),
                        name: badge
                            .get("name")
                            .and_then(|n| n.as_str())
                            .unwrap_or("")
                            .to_string(),
                        description: badge
                            .get("description")
                            .and_then(|d| d.as_str())
                            .map(String::from),
                        selected: id == selected_badge_id,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    // 7TV animated avatar (only present if the user set one). Pick the
    // best-quality image; falls back to None so the card uses the Twitch pfp.
    let avatar_url = user_data
        .get("style")
        .and_then(|s| s.get("activeProfilePicture"))
        .and_then(|pp| pp.get("images"))
        .and_then(|imgs| imgs.as_array())
        .and_then(|arr| pick_best_seventv_image(arr));

    Ok(SevenTVCosmetics {
        paints,
        badges,
        avatar_url,
    })
}

/// Pick the highest-quality usable image URL from a 7TV `images[]` array.
/// Prefers webp (broadly supported in WebView2) at the largest scale, falling
/// back to the largest image of any format. Normalizes protocol-relative URLs.
fn pick_best_seventv_image(images: &[serde_json::Value]) -> Option<String> {
    let score = |img: &serde_json::Value| -> i64 {
        let scale = img.get("scale").and_then(|s| s.as_i64()).unwrap_or(0);
        let is_webp = img
            .get("mime")
            .and_then(|m| m.as_str())
            .map(|m| m.contains("webp"))
            .unwrap_or(false);
        scale * 10 + if is_webp { 5 } else { 0 }
    };
    images
        .iter()
        .max_by_key(|img| score(img))
        .and_then(|img| img.get("url"))
        .and_then(|u| u.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| match s.strip_prefix("//") {
            Some(rest) => format!("https://{}", rest),
            None => s.to_string(),
        })
}

// Helper function to parse a color object
fn parse_color(color: &serde_json::Value) -> Option<SevenTVColor> {
    Some(SevenTVColor {
        hex: color.get("hex")?.as_str()?.to_string(),
        r: color.get("r")?.as_i64()? as i32,
        g: color.get("g")?.as_i64()? as i32,
        b: color.get("b")?.as_i64()? as i32,
        a: color.get("a")?.as_i64()? as i32,
    })
}

// Helper function to parse a paint layer
fn parse_paint_layer(layer: &serde_json::Value) -> Option<SevenTVPaintLayer> {
    let id = layer.get("id")?.as_str()?.to_string();
    let opacity = layer.get("opacity")?.as_f64()?;
    let ty = layer.get("ty")?;
    let typename = ty.get("__typename")?.as_str()?;

    let layer_type = match typename {
        "PaintLayerTypeLinearGradient" => SevenTVPaintLayerType::PaintLayerTypeLinearGradient {
            angle: ty.get("angle").and_then(|a| a.as_i64()).map(|a| a as i32),
            repeating: ty.get("repeating").and_then(|r| r.as_bool()),
            stops: ty.get("stops").and_then(|s| s.as_array()).map(|arr| {
                arr.iter()
                    .filter_map(|stop| {
                        Some(SevenTVGradientStop {
                            at: stop.get("at")?.as_f64()?,
                            color: parse_color(stop.get("color")?)?,
                        })
                    })
                    .collect()
            }),
        },
        "PaintLayerTypeRadialGradient" => SevenTVPaintLayerType::PaintLayerTypeRadialGradient {
            shape: ty.get("shape").and_then(|s| s.as_str()).map(String::from),
            repeating: ty.get("repeating").and_then(|r| r.as_bool()),
            stops: ty.get("stops").and_then(|s| s.as_array()).map(|arr| {
                arr.iter()
                    .filter_map(|stop| {
                        Some(SevenTVGradientStop {
                            at: stop.get("at")?.as_f64()?,
                            color: parse_color(stop.get("color")?)?,
                        })
                    })
                    .collect()
            }),
        },
        "PaintLayerTypeSingleColor" => SevenTVPaintLayerType::PaintLayerTypeSingleColor {
            color: ty.get("color").and_then(|c| parse_color(c)),
        },
        "PaintLayerTypeImage" => SevenTVPaintLayerType::PaintLayerTypeImage {
            images: ty.get("images").and_then(|i| i.as_array()).map(|arr| {
                arr.iter()
                    .filter_map(|img| {
                        Some(SevenTVImage {
                            url: img.get("url")?.as_str()?.to_string(),
                            mime: img.get("mime").and_then(|m| m.as_str()).map(String::from),
                            size: img.get("size").and_then(|s| s.as_i64()),
                            scale: img.get("scale").and_then(|s| s.as_i64()).map(|s| s as i32),
                            width: img.get("width").and_then(|w| w.as_i64()).map(|w| w as i32),
                            height: img.get("height").and_then(|h| h.as_i64()).map(|h| h as i32),
                            frame_count: img
                                .get("frameCount")
                                .and_then(|f| f.as_i64())
                                .map(|f| f as i32),
                        })
                    })
                    .collect()
            }),
        },
        _ => return None,
    };

    Some(SevenTVPaintLayer {
        id,
        ty: layer_type,
        opacity,
    })
}

async fn fetch_ivr_data(username: &str, channel_name: &str) -> Result<IVRData, String> {
    let client = crate::services::http::client().clone();

    // Fetch all three IVR endpoints in parallel
    let (user_result, subage_result, modvip_result) = tokio::join!(
        fetch_ivr_user(&client, username),
        fetch_ivr_subage(&client, username, channel_name),
        fetch_ivr_modvip(&client, username, channel_name)
    );

    let mut ivr_data = IVRData {
        created_at: None,
        following_since: None,
        status_hidden: false,
        is_subscribed: false,
        sub_streak: None,
        sub_cumulative: None,
        is_founder: false,
        is_mod: false,
        mod_since: None,
        is_vip: false,
        vip_since: None,
        last_broadcast_at: None,
        last_broadcast_title: None,
        follows_count: None,
        banned: false,
        chatter_count: None,
        sub_tier: None,
        sub_type: None,
        sub_gifter_login: None,
        sub_gifter_display_name: None,
        error: None,
    };

    // Process user data
    if let Ok(user) = user_result {
        ivr_data.created_at = user
            .get("createdAt")
            .and_then(|v| v.as_str())
            .map(String::from);

        if let Some(last_broadcast) = user.get("lastBroadcast") {
            ivr_data.last_broadcast_at = last_broadcast
                .get("startedAt")
                .and_then(|v| v.as_str())
                .map(String::from);
            ivr_data.last_broadcast_title = last_broadcast
                .get("title")
                .and_then(|v| v.as_str())
                .map(String::from);
        }

        ivr_data.follows_count = user
            .get("follows")
            .and_then(|v| v.as_i64())
            .map(|v| v as i32);

        ivr_data.banned = user
            .get("banned")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        ivr_data.chatter_count = user
            .get("chatterCount")
            .and_then(|v| v.as_i64())
            .map(|v| v as i32);
    }

    // Process subage data
    if let Ok(subage) = subage_result {
        ivr_data.status_hidden = subage
            .get("statusHidden")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        ivr_data.following_since = subage
            .get("followedAt")
            .and_then(|v| v.as_str())
            .map(String::from);
        ivr_data.is_subscribed = subage
            .get("subscriber")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        ivr_data.is_founder = subage
            .get("founder")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if let Some(cumulative) = subage.get("cumulative") {
            ivr_data.sub_cumulative = cumulative
                .get("months")
                .and_then(|v| v.as_i64())
                .map(|v| v as i32);
        }

        if let Some(streak) = subage.get("streak") {
            ivr_data.sub_streak = streak
                .get("months")
                .and_then(|v| v.as_i64())
                .map(|v| v as i32);
        }

        // `meta` block carries the current-period sub details (tier, type,
        // gifter). Only present while the user is actively subscribed.
        if let Some(meta) = subage.get("meta") {
            ivr_data.sub_tier = meta.get("tier").and_then(|v| v.as_str()).map(String::from);
            ivr_data.sub_type = meta.get("type").and_then(|v| v.as_str()).map(String::from);
            if let Some(giver) = meta.get("giver") {
                ivr_data.sub_gifter_login = giver
                    .get("login")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                ivr_data.sub_gifter_display_name = giver
                    .get("displayName")
                    .and_then(|v| v.as_str())
                    .map(String::from);
            }
        }
    }

    // Process mod/vip data
    if let Ok(modvip) = modvip_result {
        ivr_data.is_mod = modvip
            .get("isMod")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        ivr_data.is_vip = modvip
            .get("isVip")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if ivr_data.is_mod {
            ivr_data.mod_since = modvip
                .get("modGrantedAt")
                .and_then(|v| v.as_str())
                .map(String::from);
        }

        if ivr_data.is_vip {
            ivr_data.vip_since = modvip
                .get("vipGrantedAt")
                .and_then(|v| v.as_str())
                .map(String::from);
        }
    }

    Ok(ivr_data)
}

async fn fetch_ivr_user(
    client: &reqwest::Client,
    username: &str,
) -> Result<serde_json::Value, String> {
    let response = client
        .get(format!(
            "https://api.ivr.fi/v2/twitch/user?login={}",
            username
        ))
        .send()
        .await
        .map_err(|e| format!("IVR user request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("IVR user API error: {}", response.status()));
    }

    let data: Vec<serde_json::Value> = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse IVR user response: {}", e))?;

    data.into_iter()
        .next()
        .ok_or_else(|| "No user data found".to_string())
}

async fn fetch_ivr_subage(
    client: &reqwest::Client,
    username: &str,
    channel_name: &str,
) -> Result<serde_json::Value, String> {
    let response = client
        .get(format!(
            "https://api.ivr.fi/v2/twitch/subage/{}/{}",
            username, channel_name
        ))
        .send()
        .await
        .map_err(|e| format!("IVR subage request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("IVR subage API error: {}", response.status()));
    }

    response
        .json()
        .await
        .map_err(|e| format!("Failed to parse IVR subage response: {}", e))
}

async fn fetch_ivr_modvip(
    client: &reqwest::Client,
    username: &str,
    channel_name: &str,
) -> Result<serde_json::Value, String> {
    let response = client
        .get(format!(
            "https://api.ivr.fi/v2/twitch/modvip/{}?login={}",
            channel_name, username
        ))
        .send()
        .await
        .map_err(|e| format!("IVR modvip request failed: {}", e))?;

    // 404 is normal if user is not a mod/vip
    if response.status() == 404 {
        return Ok(serde_json::json!({
            "isMod": false,
            "isVip": false,
            "modGrantedAt": null,
            "vipGrantedAt": null
        }));
    }

    if !response.status().is_success() {
        return Err(format!("IVR modvip API error: {}", response.status()));
    }

    let data: Vec<serde_json::Value> = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse IVR modvip response: {}", e))?;

    // Find the user in the list
    for item in data {
        if let Some(login) = item.get("login").and_then(|v| v.as_str()) {
            if login.eq_ignore_ascii_case(username) {
                return Ok(item);
            }
        }
    }

    // User not in list means not a mod/vip
    Ok(serde_json::json!({
        "isMod": false,
        "isVip": false,
        "modGrantedAt": null,
        "vipGrantedAt": null
    }))
}
