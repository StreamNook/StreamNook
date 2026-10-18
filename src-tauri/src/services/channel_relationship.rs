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
    read_twitch(channel_id, channel_login).await.0
}

/// `twitch`, and whether the subscription lookup answered: a failed one reads
/// as "not subscribed", which must not replace an answer already held.
pub async fn read_twitch(channel_id: &str, channel_login: &str) -> (Relationship, bool) {
    let following = match own_follow(channel_id).await {
        FollowAnswer::Following(_) => Some(true),
        FollowAnswer::NotFollowing => Some(false),
        FollowAnswer::Unknown => None,
    };
    let Some(me) = crate::services::account_store::AccountStore::primary() else {
        return (Relationship { following, ..Default::default() }, true);
    };
    let subage_read = crate::services::ivr::subage_summary(&me.login, channel_login).await;
    let subs_answered = subage_read.is_ok();
    let subage = subage_read.ok().flatten();
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
    let relationship = Relationship {
        following,
        subscribed,
        has_sub_history: cumulative > 0 && !subscribed,
        cumulative_months: cumulative,
        badge_url,
    };
    (relationship, subs_answered)
}

/// A fresh read laid over the answer held: a part Twitch did not answer this
/// time (the follow lookup, the subscription lookup) keeps its earlier value,
/// so a lookup that failed never flips a menu to a guess.
pub fn merge(held: Option<&Relationship>, read: Relationship, subs_answered: bool) -> Relationship {
    let Some(held) = held else { return read };
    let following = read.following.or(held.following);
    if subs_answered {
        Relationship { following, ..read }
    } else {
        Relationship { following, ..held.clone() }
    }
}

/// Who a subscription USERNOTICE made a subscriber, and where: `(room id,
/// login)` for a sub, resub or upgrade (the sender) or a gift (its recipient).
/// None for anything else, a mystery gift's announcement included.
pub fn sub_notice(raw: &str) -> Option<(String, String)> {
    let tags = raw.strip_prefix('@')?.split(' ').next()?;
    let tag = |name: &str| {
        tags.split(';')
            .find_map(|t| t.strip_prefix(name)?.strip_prefix('='))
            .filter(|v| !v.is_empty())
    };
    let subscriber = match tag("msg-id")? {
        "sub" | "resub" | "giftpaidupgrade" | "primepaidupgrade" | "anongiftpaidupgrade" => tag("login")?,
        "subgift" => tag("msg-param-recipient-user-name")?,
        _ => return None,
    };
    Some((tag("room-id")?.to_string(), subscriber.to_lowercase()))
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

    fn rel(following: Option<bool>, subscribed: bool, months: i64) -> Relationship {
        Relationship {
            following,
            subscribed,
            has_sub_history: months > 0 && !subscribed,
            cumulative_months: months,
            badge_url: Some(format!("badge{months}")),
        }
    }

    #[test]
    fn a_failed_lookup_keeps_what_was_known() {
        let held = rel(Some(true), true, 14);
        // IVR did not answer: the read says "not subscribed", which is a guess.
        let read = rel(Some(true), false, 0);
        assert_eq!(merge(Some(&held), read, false), held);
        // The follow lookup did not answer: the held follow stays.
        let read = rel(None, true, 15);
        assert_eq!(merge(Some(&held), read, true), rel(Some(true), true, 15));
        // Real answers replace it, an unfollow and a lapse included.
        let read = rel(Some(false), false, 14);
        assert_eq!(merge(Some(&held), read.clone(), true), read);
        // Nothing held yet: the read is the answer.
        assert_eq!(merge(None, rel(None, false, 0), false), rel(None, false, 0));
    }

    #[test]
    fn sub_notices_name_who_became_a_subscriber() {
        let resub = "@badge-info=;login=Viewer;msg-id=resub;room-id=71092938;system-msg=x :tmi.twitch.tv USERNOTICE #xqc :hi";
        assert_eq!(sub_notice(resub), Some(("71092938".into(), "viewer".into())));
        let gift = "@login=gifter;msg-id=subgift;msg-param-recipient-user-name=Lucky;room-id=1 :tmi.twitch.tv USERNOTICE #a";
        assert_eq!(sub_notice(gift), Some(("1".into(), "lucky".into())));
        let mystery = "@login=gifter;msg-id=submysterygift;room-id=1 :tmi.twitch.tv USERNOTICE #a";
        assert_eq!(sub_notice(mystery), None);
        let raid = "@login=r;msg-id=raid;room-id=1 :tmi.twitch.tv USERNOTICE #a";
        assert_eq!(sub_notice(raid), None);
        assert_eq!(sub_notice(":tmi.twitch.tv PRIVMSG #a :sub"), None);
        // A tag whose name only starts with "login" is not the login.
        let odd = "@login-hint=x;msg-id=sub;room-id=1 :tmi.twitch.tv USERNOTICE #a";
        assert_eq!(sub_notice(odd), None);
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
