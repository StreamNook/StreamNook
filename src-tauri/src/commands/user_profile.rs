use crate::services::ivr;
use crate::services::twitch_service::TwitchService;
use log::debug;
use serde::{Deserialize, Serialize};
use lru::LruCache;
use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::sync::RwLock;

// Cache structures
/// Profile cards opened recently, newest kept. Keyed by user, name and channel,
/// so a busy chat's worth of clicked names cannot grow it without limit.
const PROFILE_CACHE_CAPACITY: usize = 128;

lazy_static::lazy_static! {
    static ref PROFILE_CACHE: Arc<RwLock<LruCache<String, CachedProfile>>> = Arc::new(RwLock::new(
        LruCache::new(NonZeroUsize::new(PROFILE_CACHE_CAPACITY).expect("non-zero capacity")),
    ));
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

/// A piece of the profile sent ahead of the whole, so a card can paint what has
/// landed (avatar, bio, 7TV name paint) while IVR and the badge lookups finish.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "part", rename_all = "snake_case")]
pub enum ProfilePart {
    Twitch { profile: TwitchUserProfile },
    Seventv { cosmetics: SevenTVCosmetics },
}

/// Fetch complete user profile with all data sources aggregated in parallel.
/// `on_part` receives each early piece as it lands; the return value is still
/// the whole profile.
#[tauri::command]
pub async fn get_user_profile_complete(
    user_id: String,
    username: String,
    channel_id: String,
    channel_name: String,
    on_part: tauri::ipc::Channel<ProfilePart>,
) -> Result<UserProfileComplete, String> {
    let send = |part: ProfilePart| {
        let _ = on_part.send(part);
    };
    // Check cache first
    let cache_key = format!("{}:{}:{}", user_id, username, channel_id);
    {
        let cache = PROFILE_CACHE.read().await;
        if let Some(cached) = cache.peek(&cache_key) {
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

    // The banner + IVR endpoints are keyed by LOGIN, but the `username` passed in
    // can be a display name (localized names aren't valid logins), which made
    // those queries come back empty. What chat passes is almost always the login
    // already, so when it has a login's shape they start NOW, alongside the
    // id-keyed lookups, instead of waiting a round trip for Helix to confirm it.
    // Only when Helix names a different login are they asked again with it.
    let guess = username.to_lowercase();
    let speculative = async {
        if is_login_shaped(&guess) {
            Some(tokio::join!(fetch_twitch_banner(&guess), fetch_ivr_data(&guess, &channel_name)))
        } else {
            None
        }
    };
    let ((twitch_result, badges_result, seventv_result), speculative) = tokio::join!(
        async {
            tokio::join!(
                async {
                    let result = fetch_twitch_profile(&user_id).await;
                    if let Ok(profile) = &result {
                        send(ProfilePart::Twitch { profile: profile.clone() });
                    }
                    result
                },
                fetch_badge_data(&user_id, &username, &channel_id, &channel_name),
                async {
                    let result = fetch_seventv_cosmetics(&user_id).await;
                    if let Ok(cosmetics) = &result {
                        send(ProfilePart::Seventv { cosmetics: cosmetics.clone() });
                    }
                    result
                }
            )
        },
        speculative
    );

    // The canonical login from Helix; the passed username only if Helix failed.
    let lookup_login = twitch_result
        .as_ref()
        .ok()
        .map(|p| p.login.clone())
        .filter(|l| !l.is_empty())
        .unwrap_or_else(|| username.clone());

    let (banner_result, ivr_result) = match speculative {
        Some(answers) if lookup_login.eq_ignore_ascii_case(&guess) => answers,
        _ => tokio::join!(
            fetch_twitch_banner(&lookup_login),
            fetch_ivr_data(&lookup_login, &channel_name)
        ),
    };

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
        cache.put(
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
    cache.pop(&cache_key);
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

/// Could this be a Twitch login as typed: 1 to 25 of `a-z`, `0-9`, `_`.
/// A localized display name never is.
fn is_login_shaped(name: &str) -> bool {
    (1..=25).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// The card's 7TV section, read from the resolver's inventory so the card's two
/// asks for it (this profile and the cosmetics panel) share one request.
async fn fetch_seventv_cosmetics(user_id: &str) -> Result<SevenTVCosmetics, String> {
    let owned = crate::services::seventv_cosmetics_resolver::owned(user_id)
        .await
        .ok_or_else(|| "7TV user not found".to_string())?;
    Ok(cosmetics_from_inventory(&owned))
}

fn cosmetics_from_inventory(owned: &crate::services::seventv_cosmetics_resolver::Inventory) -> SevenTVCosmetics {
    SevenTVCosmetics {
        paints: owned.cosmetics.paints.iter().filter_map(paint_from_definition).collect(),
        badges: owned.cosmetics.badges.iter().filter_map(badge_from_definition).collect(),
        // 7TV animated avatar (only present if the user set one). None lets the
        // card fall back to the Twitch pfp.
        avatar_url: pick_best_seventv_image(&owned.avatar_images),
    }
}

fn is_selected(definition: &serde_json::Value) -> bool {
    definition.get("selected").and_then(|s| s.as_bool()).unwrap_or(false)
}

fn text_of(definition: &serde_json::Value, key: &str) -> Option<String> {
    definition.get(key).and_then(|v| v.as_str()).map(String::from)
}

fn paint_from_definition(paint: &serde_json::Value) -> Option<SevenTVPaint> {
    let id = paint.get("id")?.as_str()?;
    let data = paint.get("data");
    let layers: Vec<SevenTVPaintLayer> = data
        .and_then(|d| d.get("layers"))
        .and_then(|l| l.as_array())
        .map(|arr| arr.iter().filter_map(parse_paint_layer).collect())
        .unwrap_or_default();
    let shadows: Vec<SevenTVPaintShadow> = data
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
        name: text_of(paint, "name").unwrap_or_default(),
        description: text_of(paint, "description"),
        selected: is_selected(paint),
        data: SevenTVPaintData { layers, shadows },
    })
}

fn badge_from_definition(badge: &serde_json::Value) -> Option<SevenTVBadge> {
    Some(SevenTVBadge {
        id: badge.get("id")?.as_str()?.to_string(),
        name: text_of(badge, "name").unwrap_or_default(),
        description: text_of(badge, "description"),
        selected: is_selected(badge),
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
    // Fetch all three IVR endpoints in parallel
    let (user_result, subage_result, modvip_result) = tokio::join!(
        ivr::user(username),
        ivr::subage(username, channel_name),
        ivr::modvip(username, channel_name)
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


/// Account facts IVR knows and Helix does not (followers, creation date, roles).
#[tauri::command]
pub async fn get_ivr_user_summary(login: String) -> Result<Option<ivr::IvrUserSummary>, String> {
    ivr::user_summary(&login).await
}

/// A user's subscription standing in a channel, from IVR.
#[tauri::command]
pub async fn get_ivr_subage_summary(
    login: String,
    channel: String,
) -> Result<Option<ivr::IvrSubageSummary>, String> {
    ivr::subage_summary(&login, &channel).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::seventv_cosmetics_resolver::{Inventory, UserCosmetics};
    use serde_json::json;

    #[test]
    fn only_login_shaped_names_start_the_login_lookups_early() {
        assert!(is_login_shaped("xqc"));
        assert!(is_login_shaped("br_winters"));
        assert!(!is_login_shaped(""));
        assert!(!is_login_shaped("\u{d55c}\u{ad6d}\u{c5b4}"));
        assert!(!is_login_shaped("has space"));
        assert!(!is_login_shaped(&"a".repeat(26)));
    }

    #[test]
    fn the_card_reads_its_typed_cosmetics_from_the_shared_inventory() {
        let owned = Inventory {
            cosmetics: UserCosmetics {
                paints: vec![json!({
                    "id": "p1", "name": "Sunset", "selected": true,
                    "data": {
                        "layers": [{ "id": "l1", "opacity": 1.0, "ty": { "__typename": "PaintLayerTypeSingleColor", "color": { "hex": "#ff0000", "r": 255, "g": 0, "b": 0, "a": 255 } } }],
                        "shadows": [{ "offsetX": 1.0, "offsetY": 2.0, "blur": 3.0, "color": { "hex": "#000000", "r": 0, "g": 0, "b": 0, "a": 255 } }]
                    }
                })],
                badges: vec![json!({ "id": "b1", "name": "Sub", "description": "desc" })],
                seventv_user_id: Some("7tv".into()),
            },
            avatar_images: vec![json!({ "url": "//cdn.7tv.app/a/4x.webp", "mime": "image/webp", "scale": 4 })],
        };
        let c = cosmetics_from_inventory(&owned);
        assert_eq!(c.paints.len(), 1);
        assert!(c.paints[0].selected);
        assert_eq!(c.paints[0].name, "Sunset");
        assert_eq!(c.paints[0].data.shadows.len(), 1);
        assert_eq!(c.badges.len(), 1);
        assert!(!c.badges[0].selected);
        assert_eq!(c.badges[0].description.as_deref(), Some("desc"));
        assert!(c.avatar_url.is_some());
    }
}
