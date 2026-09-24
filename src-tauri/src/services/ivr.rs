//! api.ivr.fi: public Twitch account facts Helix does not expose (account
//! creation date, follower count, roles, sub tenure in a channel, mod/VIP).
//!
//! The raw fetchers feed the profile card's combined lookup; the summaries are
//! what settings pages and the chat header ask for, cached here once for every
//! window instead of in each window's own map.

use lru::LruCache;
use serde::Serialize;
use std::num::NonZeroUsize;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const SUMMARY_TTL: Duration = Duration::from_secs(5 * 60);
const SUMMARY_CAPACITY: usize = 256;

pub(crate) async fn user(username: &str) -> Result<serde_json::Value, String> {
    let response = crate::services::http::client()
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

pub(crate) async fn subage(username: &str, channel_name: &str) -> Result<serde_json::Value, String> {
    let response = crate::services::http::client()
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

pub(crate) async fn modvip(username: &str, channel_name: &str) -> Result<serde_json::Value, String> {
    let response = crate::services::http::client()
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

/// What the profile overview shows about an account.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct IvrUserSummary {
    pub followers: Option<i64>,
    pub created_at: Option<String>,
    pub is_affiliate: bool,
    pub is_partner: bool,
    pub is_staff: bool,
}

/// One user's subscription standing in one channel.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct IvrSubageSummary {
    /// The active sub's type ("paid", "gift", "prime"), None when not subscribed.
    pub active_sub_type: Option<String>,
    /// Total months subscribed, as IVR reports it.
    pub cumulative_months: Option<i64>,
    /// Best available lifetime figure: cumulative, else the current period's
    /// month count, else the streak.
    pub lifetime_months: i64,
}

type Cache<T> = Mutex<Option<LruCache<String, (Instant, Option<T>)>>>;

static USER_SUMMARIES: Cache<IvrUserSummary> = Mutex::new(None);
static SUBAGE_SUMMARIES: Cache<IvrSubageSummary> = Mutex::new(None);

fn cached<T: Clone>(cache: &Cache<T>, key: &str) -> Option<Option<T>> {
    let mut guard = cache.lock().ok()?;
    let lru = guard.as_mut()?;
    match lru.get(key) {
        Some((at, value)) if at.elapsed() < SUMMARY_TTL => Some(value.clone()),
        _ => None,
    }
}

fn remember<T>(cache: &Cache<T>, key: String, value: Option<T>) {
    if let Ok(mut guard) = cache.lock() {
        guard
            .get_or_insert_with(|| {
                LruCache::new(NonZeroUsize::new(SUMMARY_CAPACITY).expect("non-zero capacity"))
            })
            .put(key, (Instant::now(), value));
    }
}

/// `Err` from a fetcher is either IVR answering "nothing here" (an error
/// status or an empty result, cached like any answer) or IVR not being
/// reachable at all (never cached, so the next ask retries).
fn is_unreachable(error: &str) -> bool {
    error.contains("request failed")
}

fn user_summary_from(user: &serde_json::Value) -> IvrUserSummary {
    let role = |name: &str| {
        user.pointer(&format!("/roles/{name}"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    };
    IvrUserSummary {
        followers: user.get("followers").and_then(|v| v.as_i64()),
        created_at: user
            .get("createdAt")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from),
        is_affiliate: role("isAffiliate"),
        is_partner: role("isPartner"),
        is_staff: role("isStaff"),
    }
}

fn subage_summary_from(subage: &serde_json::Value) -> IvrSubageSummary {
    let months = |path: &str| subage.pointer(path).and_then(|v| v.as_i64());
    let cumulative_months = months("/cumulative/months");
    IvrSubageSummary {
        active_sub_type: subage
            .pointer("/meta/type")
            .and_then(|v| v.as_str())
            .map(String::from),
        cumulative_months,
        lifetime_months: cumulative_months
            .or_else(|| months("/meta/subMonths"))
            .or_else(|| months("/streak/months"))
            .unwrap_or(0),
    }
}

pub async fn user_summary(login: &str) -> Result<Option<IvrUserSummary>, String> {
    let key = login.to_lowercase();
    if let Some(hit) = cached(&USER_SUMMARIES, &key) {
        return Ok(hit);
    }
    let value = match user(&key).await {
        Ok(user) => Some(user_summary_from(&user)),
        Err(e) if is_unreachable(&e) => return Err(e),
        Err(_) => None,
    };
    remember(&USER_SUMMARIES, key, value.clone());
    Ok(value)
}

pub async fn subage_summary(
    login: &str,
    channel: &str,
) -> Result<Option<IvrSubageSummary>, String> {
    let key = format!("{}:{}", login.to_lowercase(), channel.to_lowercase());
    if let Some(hit) = cached(&SUBAGE_SUMMARIES, &key) {
        return Ok(hit);
    }
    let value = match subage(login, channel).await {
        Ok(subage) => Some(subage_summary_from(&subage)),
        Err(e) if is_unreachable(&e) => return Err(e),
        Err(_) => None,
    };
    remember(&SUBAGE_SUMMARIES, key, value.clone());
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_user_summary_reads_roles_and_counts() {
        let s = user_summary_from(&json!({
            "followers": 1200, "createdAt": "2015-01-02T03:04:05Z",
            "roles": { "isAffiliate": true, "isPartner": false, "isStaff": null }
        }));
        assert_eq!(s.followers, Some(1200));
        assert_eq!(s.created_at.as_deref(), Some("2015-01-02T03:04:05Z"));
        assert!(s.is_affiliate && !s.is_partner && !s.is_staff);
    }

    #[test]
    fn lifetime_months_falls_back_in_order() {
        let full = subage_summary_from(&json!({
            "cumulative": { "months": 30 }, "streak": { "months": 4 },
            "meta": { "type": "paid", "subMonths": 29 }
        }));
        assert_eq!((full.cumulative_months, full.lifetime_months), (Some(30), 30));
        assert_eq!(full.active_sub_type.as_deref(), Some("paid"));

        let meta_only = subage_summary_from(&json!({ "meta": { "type": "gift", "subMonths": 7 } }));
        assert_eq!((meta_only.cumulative_months, meta_only.lifetime_months), (None, 7));

        let lapsed =
            subage_summary_from(&json!({ "cumulative": null, "streak": { "months": 2 }, "meta": null }));
        assert_eq!((lapsed.active_sub_type, lapsed.lifetime_months), (None, 2));

        let never = subage_summary_from(&json!({ "subscriber": false }));
        assert_eq!(never.lifetime_months, 0);
    }
}
