//! The signed-in viewer's relationship to a Twitch channel, for the follow and
//! subscribe controls (the phone's chat menu, the desktop player overlay,
//! MultiNook tiles): whether they follow it, whether they subscribe or used to,
//! and the subscriber badge their next month would wear.
//!
//! Follows come from `follow_lookup::own_follow`, so an unanswerable lookup is
//! "unknown" (the control waits) rather than a guess. Subscription months come
//! from IVR's sub-age record; the badge art from the channel's Helix badge set,
//! then the global one.

use serde::Serialize;

use crate::commands::badges::HelixBadgesResponse;
use crate::services::follow_lookup::{own_follow, FollowAnswer};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Relationship {
    /// `None` while unknown (no sign-in, or Helix did not answer).
    pub following: Option<bool>,
    pub subscribed: bool,
    /// Subscribed before, not now: the control offers to resubscribe.
    pub has_sub_history: bool,
    pub cumulative_months: i64,
    /// The subscriber badge for the months the viewer has, or the next month
    /// a lapsed subscriber would reach.
    pub badge_url: Option<String>,
}

/// The subscriber badge version Twitch shows for `months` (its tiers).
pub fn badge_version(months: i64) -> &'static str {
    match months {
        m if m >= 72 => "72",
        m if m >= 60 => "60",
        m if m >= 48 => "48",
        m if m >= 36 => "36",
        m if m >= 24 => "24",
        m if m >= 18 => "18",
        m if m >= 12 => "12",
        m if m >= 9 => "9",
        m if m >= 6 => "6",
        m if m >= 3 => "3",
        m if m >= 2 => "2",
        _ => "0",
    }
}

/// The months a badge is chosen for: a lapsed subscriber sees the next month.
pub fn badge_months(subscribed: bool, cumulative: i64) -> i64 {
    if !subscribed && cumulative > 0 {
        cumulative + 1
    } else {
        cumulative
    }
}

/// A badge's 2x art from a Helix badge answer, the exact version or the set's
/// first one.
pub fn badge_art(badges: &HelixBadgesResponse, set_id: &str, version: &str) -> Option<String> {
    let set = badges.data.iter().find(|s| s.set_id == set_id)?;
    let v = set.versions.iter().find(|v| v.id == version).or(set.versions.first())?;
    Some(v.image_url_2x.clone()).filter(|u| !u.is_empty())
}

/// The viewer's relationship to the Twitch channel `channel_id` (login
/// `channel_login`).
pub async fn twitch(channel_id: &str, channel_login: &str) -> Relationship {
    let following = match own_follow(channel_id).await {
        FollowAnswer::Following(_) => Some(true),
        FollowAnswer::NotFollowing => Some(false),
        FollowAnswer::Unknown => None,
    };
    let Some(me) = crate::services::account_store::AccountStore::primary() else {
        return Relationship { following, ..Default::default() };
    };
    let subage = crate::services::ivr::subage_summary(&me.login, channel_login).await.ok().flatten();
    let subscribed = subage.as_ref().is_some_and(|s| s.active_sub_type.is_some());
    let cumulative = subage.as_ref().and_then(|s| s.cumulative_months).unwrap_or(0);
    let version = badge_version(badge_months(subscribed, cumulative));
    let channel_set = crate::commands::badges::fetch_channel_badges(channel_id.to_string()).await.ok();
    let mut badge_url = channel_set.as_ref().and_then(|b| badge_art(b, "subscriber", version));
    if badge_url.is_none() {
        if let Ok(Some(global)) = crate::commands::badges::get_cached_global_badges().await {
            badge_url = badge_art(&global, "subscriber", version);
        }
    }
    Relationship {
        following,
        subscribed,
        has_sub_history: cumulative > 0 && !subscribed,
        cumulative_months: cumulative,
        badge_url,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::badges::{HelixBadgeSet, HelixBadgeVersion};

    fn version(id: &str, url: &str) -> HelixBadgeVersion {
        HelixBadgeVersion {
            id: id.into(),
            image_url_1x: String::new(),
            image_url_2x: url.into(),
            image_url_4x: String::new(),
            title: id.into(),
            description: String::new(),
            click_action: None,
            click_url: None,
        }
    }

    #[test]
    fn badge_versions_follow_twitch_tiers() {
        assert_eq!(badge_version(0), "0");
        assert_eq!(badge_version(1), "0");
        assert_eq!(badge_version(2), "2");
        assert_eq!(badge_version(5), "3");
        assert_eq!(badge_version(11), "9");
        assert_eq!(badge_version(13), "12");
        assert_eq!(badge_version(100), "72");
    }

    #[test]
    fn a_lapsed_subscriber_sees_the_next_month() {
        assert_eq!(badge_months(false, 5), 6);
        assert_eq!(badge_months(true, 5), 5);
        assert_eq!(badge_months(false, 0), 0);
    }

    #[test]
    fn badge_art_takes_the_version_or_the_first() {
        let badges = HelixBadgesResponse {
            data: vec![HelixBadgeSet {
                set_id: "subscriber".into(),
                versions: vec![version("0", "u0"), version("6", "u6")],
            }],
        };
        assert_eq!(badge_art(&badges, "subscriber", "6").as_deref(), Some("u6"));
        assert_eq!(badge_art(&badges, "subscriber", "12").as_deref(), Some("u0"));
        assert_eq!(badge_art(&badges, "vip", "1"), None);
    }
}
