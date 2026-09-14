//! A Kick chatter's profile card, assembled from kick.com's public JSON.
//!
//! Everything the card needs is reachable WITHOUT a login and without a webview:
//! the per-room user endpoint alone carries the card's spine (in-room badges,
//! moderator flag, followage, account age, sub tenure), and two optional calls
//! add the bio, socials and follower count. All three go through
//! `kick::browser_get`, the shared helper that sends Chrome client hints — the
//! same path `kick_emotes` already uses, so there is no new HTTP client here and
//! nothing spawns a browser.
//!
//! Shapes below were probed against the live site rather than inferred; the
//! per-room endpoint is undocumented, so every field is optional and a missing
//! one degrades the card instead of failing it.

use crate::services::providers::kick;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// One badge the user wears in this room.
#[derive(Debug, Clone, Serialize)]
pub struct KickBadge {
    /// Machine name: `moderator`, `subscriber`, `og`, `vip`, `verified`, …
    pub kind: String,
    /// What Kick calls it in a tooltip.
    pub text: String,
    /// Badge art, when the badge has any. Role badges usually do not.
    pub image_url: Option<String>,
    /// Months, for the subscriber badge. Absent on every other kind.
    pub months: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct KickUserProfile {
    pub user_id: Option<i64>,
    /// Cased display name.
    pub username: String,
    /// Lowercase channel slug.
    pub slug: String,
    pub avatar_url: Option<String>,
    pub bio: Option<String>,
    pub banner_url: Option<String>,
    pub verified: bool,
    pub is_staff: bool,
    pub is_channel_owner: bool,
    pub is_moderator: bool,
    /// Banned in THIS room.
    pub is_banned: bool,
    pub followers_count: Option<i64>,
    /// Following this room since (RFC3339).
    pub following_since: Option<String>,
    /// Kick account created (RFC3339).
    pub created_at: Option<String>,
    /// Months subscribed to this room. 0 means not subscribed.
    pub subscribed_for: i64,
    pub badges: Vec<KickBadge>,
    /// Socials the user filled in, as `("twitter", "handle")` pairs. Only
    /// non-empty ones, so the card can render the list without filtering.
    pub socials: Vec<(String, String)>,
}

fn str_at(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Parse the `badges` + `badges_v2` arrays into one list.
///
/// `badges` carries the ROLE badges (moderator, subscriber with a month count,
/// og, vip, verified) with no art; `badges_v2` carries global cosmetic badges
/// WITH art. Both matter to the card, so they merge rather than one winning.
fn parse_badges(user: &Value) -> Vec<KickBadge> {
    let mut out = Vec::new();
    if let Some(arr) = user.get("badges").and_then(|b| b.as_array()) {
        for b in arr {
            // `active: false` is a badge the user could wear but isn't.
            if b.get("active").and_then(|a| a.as_bool()) == Some(false) {
                continue;
            }
            let Some(kind) = str_at(b, "type") else { continue };
            out.push(KickBadge {
                text: str_at(b, "text").unwrap_or_else(|| kind.clone()),
                months: b.get("count").and_then(|c| c.as_i64()),
                image_url: None,
                kind,
            });
        }
    }
    if let Some(arr) = user.get("badges_v2").and_then(|b| b.as_array()) {
        for b in arr {
            let Some(name) = str_at(b, "name") else { continue };
            out.push(KickBadge {
                text: name.replace('_', " "),
                image_url: str_at(b, "image_url"),
                months: None,
                kind: name,
            });
        }
    }
    out
}

/// The socials Kick exposes on a user record, in a stable order.
const SOCIAL_KEYS: [&str; 6] = ["twitter", "instagram", "youtube", "discord", "tiktok", "facebook"];

/// Build a chatter's profile for `room`.
///
/// `room` is the channel whose chat they are in — the badges and followage are
/// scoped to it, which is exactly what the card wants to show.
pub async fn fetch(room: &str, username: &str) -> Result<KickUserProfile, String> {
    let room_lc = room.to_lowercase();
    let user_lc = username.to_lowercase();

    let cache_key = format!("{}/{}", room_lc, user_lc);
    if let Some(hit) = cached(&cache_key) {
        return Ok(hit);
    }

    let started = Instant::now();
    // The one REQUIRED call: without it there is no card worth showing.
    let url = format!(
        "https://kick.com/api/v2/channels/{}/users/{}",
        room_lc, user_lc
    );
    let resp = kick::browser_get(&url, &room_lc)
        .await
        .ok_or_else(|| format!("Kick had nothing for '{}'", username))?;
    let core_ms = started.elapsed().as_millis();
    let in_room: Value = resp
        .json()
        .await
        .map_err(|e| format!("Kick sent an unreadable profile: {}", e))?;

    let mut profile = KickUserProfile {
        user_id: in_room.get("id").and_then(|v| v.as_i64()),
        username: str_at(&in_room, "username").unwrap_or_else(|| username.to_string()),
        slug: str_at(&in_room, "slug").unwrap_or(user_lc.clone()),
        avatar_url: str_at(&in_room, "profile_pic"),
        is_staff: in_room.get("is_staff").and_then(|v| v.as_bool()).unwrap_or(false),
        is_channel_owner: in_room
            .get("is_channel_owner")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        is_moderator: in_room
            .get("is_moderator")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        // `banned` is null when they are not.
        is_banned: in_room.get("banned").map(|b| !b.is_null()).unwrap_or(false),
        following_since: str_at(&in_room, "following_since"),
        created_at: str_at(&in_room, "created_at"),
        subscribed_for: in_room
            .get("subscribed_for")
            .and_then(|v| v.as_i64())
            .unwrap_or(0),
        badges: parse_badges(&in_room),
        ..Default::default()
    };

    // Enrichment. Both are optional: a failure here costs a bio or a follower
    // count, never the card. Run concurrently so the card opens at the speed of
    // the slowest ONE rather than the sum.
    let users_url = format!("https://kick.com/api/v1/users/{}", user_lc);
    let channel_url = format!("https://kick.com/api/v2/channels/{}", user_lc);
    // BOUNDED. These are optional by design, but awaiting them unbounded meant a
    // slow (not failed) enrichment held the whole card up to the client's 12s
    // timeout — the card was only as fast as Kick's slowest secondary endpoint.
    // Past the budget we return what we have; the fields they fill are a bio, a
    // follower count and a banner.
    let enrich_started = Instant::now();
    let enriched = tokio::time::timeout(
        ENRICH_BUDGET,
        async {
            tokio::join!(
                kick::browser_get(&users_url, &user_lc),
                kick::browser_get(&channel_url, &user_lc),
            )
        },
    )
    .await;
    let timed_out = enriched.is_err();
    let (user_rec, channel_rec) = enriched.unwrap_or((None, None));

    if let Some(resp) = user_rec {
        if let Ok(v) = resp.json::<Value>().await {
            profile.bio = str_at(&v, "bio");
            if profile.avatar_url.is_none() {
                profile.avatar_url = str_at(&v, "profilepic");
            }
            for key in SOCIAL_KEYS {
                if let Some(handle) = str_at(&v, key) {
                    profile.socials.push((key.to_string(), handle));
                }
            }
        }
    }

    if let Some(resp) = channel_rec {
        if let Ok(v) = resp.json::<Value>().await {
            profile.verified = v.get("verified").map(|x| !x.is_null()).unwrap_or(false);
            // Kick sends this as a STRING, not a number.
            profile.followers_count = v
                .get("followers_count")
                .and_then(|c| c.as_i64().or_else(|| c.as_str()?.parse().ok()));
            profile.banner_url = v
                .get("banner_image")
                .and_then(|b| b.get("url"))
                .and_then(|u| u.as_str())
                .map(str::to_string);
        }
    }

    log::debug!(
        "[Kick] profile '{}' in {}ms (core {}ms, enrich {}ms{})",
        user_lc,
        started.elapsed().as_millis(),
        core_ms,
        enrich_started.elapsed().as_millis(),
        if timed_out { ", BUDGET EXCEEDED" } else { "" }
    );
    store(&cache_key, &profile);
    Ok(profile)
}

/// How long the optional enrichment calls may hold the card open.
const ENRICH_BUDGET: Duration = Duration::from_millis(2500);
/// Profiles are re-read often (every click on the same chatter), and the values
/// on them move slowly, so a short cache turns a repeat open into an instant one.
const PROFILE_TTL: Duration = Duration::from_secs(300);

static PROFILE_CACHE: OnceLock<Mutex<HashMap<String, (KickUserProfile, Instant)>>> = OnceLock::new();

fn profile_cache() -> &'static Mutex<HashMap<String, (KickUserProfile, Instant)>> {
    PROFILE_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cached(key: &str) -> Option<KickUserProfile> {
    profile_cache()
        .lock()
        .ok()?
        .get(key)
        .filter(|(_, at)| at.elapsed() < PROFILE_TTL)
        .map(|(p, _)| p.clone())
}

fn store(key: &str, profile: &KickUserProfile) {
    if let Ok(mut c) = profile_cache().lock() {
        // Bounded: a long session in a busy channel would otherwise keep every
        // chatter whose card was ever opened.
        if c.len() > 256 {
            c.retain(|_, (_, at)| at.elapsed() < PROFILE_TTL);
            if c.len() > 256 {
                c.clear();
            }
        }
        c.insert(key.to_string(), (profile.clone(), Instant::now()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn merges_role_and_cosmetic_badges() {
        let v = json!({
            "badges": [
                { "type": "moderator", "text": "Moderator", "active": true },
                { "type": "subscriber", "text": "Subscriber", "active": true, "count": 7 },
                { "type": "vip", "text": "VIP", "active": false }
            ],
            "badges_v2": [
                { "name": "kick_whale", "image_url": "https://x/whale.png" }
            ]
        });
        let badges = parse_badges(&v);
        // The inactive VIP is dropped; the other three survive.
        assert_eq!(badges.len(), 3);
        assert_eq!(badges[1].kind, "subscriber");
        assert_eq!(badges[1].months, Some(7));
        assert_eq!(badges[2].kind, "kick_whale");
        assert_eq!(badges[2].image_url.as_deref(), Some("https://x/whale.png"));
    }

    #[test]
    fn blank_strings_are_treated_as_absent() {
        // Kick returns "" rather than null for socials the user left empty, so a
        // naive read would render a row of empty links.
        let v = json!({ "bio": "  ", "twitter": "xqc", "facebook": "" });
        assert_eq!(str_at(&v, "bio"), None);
        assert_eq!(str_at(&v, "facebook"), None);
        assert_eq!(str_at(&v, "twitter").as_deref(), Some("xqc"));
    }
}
