//! api.ivr.fi: public Twitch account facts Helix does not expose (account
//! creation date, follower count, roles, sub tenure in a channel, mod/VIP).
//!
//! The raw fetchers feed the profile card's combined lookup; the summaries are
//! what settings pages and the chat header ask for, cached here once for every
//! window instead of in each window's own map.

use lru::LruCache;
use serde::{Deserialize, Serialize};
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

/// How someone holds a sub.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubKind {
    Paid,
    Gift,
    Prime,
}

/// IVR's answer about one user's sub in one channel.
///
/// IVR's subage answer has no "subscribed" flag. A current sub is a non-null
/// `meta` block (or a period with days still remaining); a lapsed one keeps
/// only `cumulative`, with no `meta` and no `streak`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SubFacts {
    /// The user hid their standing, so nothing below can be trusted as "none".
    pub hidden: bool,
    pub active: bool,
    pub kind: Option<SubKind>,
    /// 1, 2 or 3.
    pub tier: Option<u8>,
    pub total_months: Option<i64>,
    pub streak_months: Option<i64>,
    pub gifter_login: Option<String>,
    pub gifter_display_name: Option<String>,
    pub ends_at: Option<String>,
    /// A paid sub that will renew on its own.
    pub renews: bool,
    pub user_display_name: Option<String>,
    pub channel_display_name: Option<String>,
}

pub fn sub_facts_from(subage: &serde_json::Value) -> SubFacts {
    let text = |path: &str| {
        subage
            .pointer(path)
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from)
    };
    let count = |path: &str| subage.pointer(path).and_then(|v| v.as_i64()).filter(|n| *n > 0);
    let has_meta = subage.get("meta").is_some_and(|m| m.is_object());
    let active = has_meta
        || count("/cumulative/daysRemaining").is_some()
        || count("/streak/daysRemaining").is_some();
    let raw_tier = text("/meta/tier");
    let kind = match text("/meta/type").as_deref() {
        Some("paid") => Some(SubKind::Paid),
        Some("gift") => Some(SubKind::Gift),
        Some("prime") => Some(SubKind::Prime),
        _ if raw_tier.as_deref().is_some_and(|t| t.eq_ignore_ascii_case("prime")) => {
            Some(SubKind::Prime)
        }
        _ => None,
    };
    let tier = raw_tier
        .as_deref()
        .and_then(|t| t.parse::<u8>().ok())
        .filter(|t| (1..=3).contains(t))
        .or((kind == Some(SubKind::Prime)).then_some(1));
    let streak_months = if active { count("/streak/months") } else { None };
    let total_months = [count("/cumulative/months"), count("/meta/subMonths"), streak_months]
        .into_iter()
        .flatten()
        .max();
    let gifted = kind == Some(SubKind::Gift);
    SubFacts {
        hidden: subage
            .get("statusHidden")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        active,
        kind,
        tier,
        total_months,
        streak_months,
        gifter_login: if gifted { text("/meta/giftMeta/gifter/login") } else { None },
        gifter_display_name: if gifted {
            text("/meta/giftMeta/gifter/displayName")
        } else {
            None
        },
        ends_at: if active {
            text("/meta/endsAt").or_else(|| text("/cumulative/end"))
        } else {
            None
        },
        renews: text("/meta/renewsAt").is_some(),
        user_display_name: text("/user/displayName"),
        channel_display_name: text("/channel/displayName"),
    }
}

type Cache<T> = Mutex<Option<LruCache<String, (Instant, Option<T>)>>>;

static USER_SUMMARIES: Cache<IvrUserSummary> = Mutex::new(None);
static SUBAGE_SUMMARIES: Cache<IvrSubageSummary> = Mutex::new(None);
static SUB_FACTS: Cache<SubFacts> = Mutex::new(None);

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

/// One user's sub in one channel, cached like the summaries. `Ok(None)` is IVR
/// answering with nothing (an unknown user or channel).
pub async fn sub_facts(login: &str, channel: &str) -> Result<Option<SubFacts>, String> {
    let key = format!("{}:{}", login.to_lowercase(), channel.to_lowercase());
    if let Some(hit) = cached(&SUB_FACTS, &key) {
        return Ok(hit);
    }
    let value = match subage(login, channel).await {
        Ok(subage) => Some(sub_facts_from(&subage)),
        Err(e) if is_unreachable(&e) => return Err(e),
        Err(_) => None,
    };
    remember(&SUB_FACTS, key, value.clone());
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

    fn facts(raw: &str) -> SubFacts {
        sub_facts_from(&serde_json::from_str(raw).expect("fixture parses"))
    }

    #[test]
    fn a_gifted_sub_is_current_with_its_gifter() {
        let f = facts(fixtures::CURRENT_GIFT);
        assert!(f.active && !f.hidden);
        assert_eq!(f.kind, Some(SubKind::Gift));
        assert_eq!(f.tier, Some(1));
        assert_eq!((f.total_months, f.streak_months), (Some(4), Some(1)));
        assert_eq!(f.gifter_display_name.as_deref(), Some("JustThunderr"));
        assert_eq!(f.gifter_login.as_deref(), Some("justthunderr"));
        assert!(!f.renews);
        assert_eq!(f.channel_display_name.as_deref(), Some("FeelsSunnyMan"));
    }

    #[test]
    fn a_paid_sub_is_current_and_renews() {
        let f = facts(fixtures::CURRENT_PAID);
        assert!(f.active && f.renews);
        assert_eq!(f.kind, Some(SubKind::Paid));
        assert_eq!((f.total_months, f.streak_months), (Some(7), Some(3)));
        assert_eq!(f.gifter_display_name, None);
    }

    #[test]
    fn a_prime_sub_is_current_at_tier_one() {
        let f = facts(fixtures::CURRENT_PRIME);
        assert!(f.active);
        assert_eq!((f.kind, f.tier), (Some(SubKind::Prime), Some(1)));
        assert_eq!(f.total_months, Some(12));
    }

    #[test]
    fn a_lapsed_sub_keeps_its_months_but_is_not_active() {
        let f = facts(fixtures::PAST);
        assert!(!f.active);
        assert_eq!((f.kind, f.tier), (None, None));
        assert_eq!((f.total_months, f.streak_months), (Some(9), None));
        assert_eq!(f.ends_at, None);
    }

    #[test]
    fn someone_who_never_subscribed_has_no_months() {
        let f = facts(fixtures::NEVER);
        assert!(!f.active);
        assert_eq!(f.total_months, None);
    }

    #[test]
    fn days_left_in_a_period_count_as_current_without_meta() {
        let f = facts(
            r#"{"statusHidden":false,"streak":null,"meta":null,
                "cumulative":{"daysRemaining":5,"months":2}}"#,
        );
        assert!(f.active);
        assert_eq!(f.total_months, Some(2));
    }
}

/// Real api.ivr.fi subage answers (the Prime one follows the same shape), shared
/// with the sub standing tests.
#[cfg(test)]
pub(crate) mod fixtures {
    pub const CURRENT_GIFT: &str = r#"{"user":{"id":"773500211","login":"emerald_org","displayName":"Emerald_org"},"channel":{"id":"134522528","login":"feelssunnyman","displayName":"FeelsSunnyMan"},"statusHidden":false,"followedAt":null,"streak":{"elapsedDays":16,"daysRemaining":13,"months":1,"end":"2026-10-26T16:33:35Z","start":"2026-09-26T16:33:36Z"},"cumulative":{"elapsedDays":16,"daysRemaining":13,"months":4,"end":"2026-10-26T16:33:35Z","start":"2026-09-26T16:33:36Z"},"meta":{"type":"gift","tier":"1","endsAt":"2026-10-26T16:33:35Z","renewsAt":null,"giftMeta":{"giftDate":"2026-09-26T16:33:36.213862398Z","gifter":{"id":"425652082","login":"justthunderr","displayName":"JustThunderr"}}}}"#;
    pub const CURRENT_PAID: &str = r#"{"user":{"id":"425652082","login":"justthunderr","displayName":"JustThunderr"},"channel":{"id":"134522528","login":"feelssunnyman","displayName":"FeelsSunnyMan"},"statusHidden":false,"followedAt":null,"streak":{"elapsedDays":27,"daysRemaining":2,"months":3,"end":"2026-10-15T00:00:00Z","start":"2026-09-15T17:09:58Z"},"cumulative":{"elapsedDays":27,"daysRemaining":2,"months":7,"end":"2026-10-15T00:00:00Z","start":"2026-09-15T17:09:58Z"},"meta":{"type":"paid","tier":"1","endsAt":"2026-10-18T17:09:54Z","renewsAt":"2026-10-15T17:09:55Z","giftMeta":null}}"#;
    pub const CURRENT_PRIME: &str = r#"{"user":{"id":"1","login":"primefan","displayName":"PrimeFan"},"channel":{"id":"134522528","login":"feelssunnyman","displayName":"FeelsSunnyMan"},"statusHidden":false,"followedAt":null,"streak":{"elapsedDays":3,"daysRemaining":27,"months":12,"end":"2026-11-09T00:00:00Z","start":"2026-10-09T00:00:00Z"},"cumulative":{"elapsedDays":3,"daysRemaining":27,"months":12,"end":"2026-11-09T00:00:00Z","start":"2026-10-09T00:00:00Z"},"meta":{"type":"prime","tier":"1","endsAt":"2026-11-09T00:00:00Z","renewsAt":null,"giftMeta":null}}"#;
    pub const PAST: &str = r#"{"user":{"id":"71092938","login":"xqc","displayName":"xQc"},"channel":{"id":"134522528","login":"feelssunnyman","displayName":"FeelsSunnyMan"},"statusHidden":false,"followedAt":null,"streak":null,"cumulative":{"elapsedDays":30,"daysRemaining":0,"months":9,"end":"2026-10-01T21:49:19Z","start":"2026-09-01T21:49:19Z"},"meta":null}"#;
    pub const NEVER: &str = r#"{"user":{"id":"249031143","login":"br_winters","displayName":"br_winters"},"channel":{"id":"134522528","login":"feelssunnyman","displayName":"FeelsSunnyMan"},"statusHidden":false,"followedAt":null,"streak":null,"cumulative":null,"meta":null}"#;
}
