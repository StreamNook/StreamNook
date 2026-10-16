//! Whether a user follows a channel.
//!
//! Twitch no longer shares other people's follows publicly: api.ivr.fi's
//! `followedAt` comes back null for every pair, and GQL's `follow` /
//! `relationship` fields fail their integrity check without a browser. The
//! answer comes from the places that still have it: the signed-in account's
//! own follows, and a channel's follower list, which Helix opens to the
//! channel's broadcaster and moderators. Anywhere else the honest answer is
//! "unknown", never "not following".

use crate::services::slash_commands::{helix, me};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FollowAnswer {
    /// Following since this RFC 3339 time.
    Following(String),
    NotFollowing,
    Unknown,
}

/// Whether the asking account moderates (or owns) the channel, which decides
/// whether an empty follower page means "not following" or "not shown to you".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewerRole {
    Moderates,
    DoesNot,
    /// Not known to the caller: only a found follow counts as an answer.
    Unknown,
}

/// Reads a Helix `channels/followers` or `channels/followed` page filtered to
/// one user. `authoritative` says an empty page really means no follow.
pub fn read_follow_page(page: &Value, authoritative: bool) -> FollowAnswer {
    let at = page["data"]
        .as_array()
        .and_then(|rows| rows.first())
        .and_then(|row| row["followed_at"].as_str())
        .filter(|s| !s.is_empty());
    match at {
        Some(at) => FollowAnswer::Following(at.to_string()),
        None if authoritative => FollowAnswer::NotFollowing,
        None => FollowAnswer::Unknown,
    }
}

/// IVR first (free, and right if it ever answers again), then Helix.
pub fn merge(ivr_followed_at: Option<&str>, helix: FollowAnswer) -> FollowAnswer {
    match ivr_followed_at.filter(|s| !s.is_empty()) {
        Some(at) => FollowAnswer::Following(at.to_string()),
        None => helix,
    }
}

/// The signed-in account's own follow of `channel_id`. Helix answers this one
/// for every channel, so an empty page means not following.
pub async fn own_follow(channel_id: &str) -> FollowAnswer {
    let Ok((my_id, _)) = me().await else {
        return FollowAnswer::Unknown;
    };
    helix_follow(&my_id, channel_id, ViewerRole::Unknown).await
}

/// The Helix half of the answer for `user_id` following `channel_id`.
pub async fn helix_follow(user_id: &str, channel_id: &str, role: ViewerRole) -> FollowAnswer {
    if user_id.is_empty() || channel_id.is_empty() {
        return FollowAnswer::Unknown;
    }
    let Ok((my_id, _)) = me().await else {
        return FollowAnswer::Unknown;
    };
    let (path, authoritative) = if user_id == my_id {
        (format!("channels/followed?user_id={my_id}&broadcaster_id={channel_id}"), true)
    } else {
        let moderates = my_id == channel_id || role == ViewerRole::Moderates;
        if !moderates && role == ViewerRole::DoesNot {
            // Helix hands a non-moderator an empty page for anyone: no answer.
            return FollowAnswer::Unknown;
        }
        (
            format!("channels/followers?broadcaster_id={channel_id}&user_id={user_id}"),
            moderates,
        )
    };
    match helix(Method::GET, &path, None).await {
        Ok(page) => read_follow_page(&page, authoritative),
        Err(e) => {
            log::debug!("[follow] Helix follow lookup failed: {e}");
            FollowAnswer::Unknown
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A Helix `channels/followers?user_id=` page as a moderator gets it.
    const FOLLOWER_PAGE: &str = r#"{"total":120411,"data":[{"user_id":"773500211","user_login":"emerald_org","user_name":"Emerald_org","followed_at":"2023-03-04T05:06:07Z"}],"pagination":{}}"#;
    /// The same filter for someone who does not follow.
    const EMPTY_PAGE: &str = r#"{"total":120411,"data":[],"pagination":{}}"#;

    fn page(raw: &str) -> Value {
        serde_json::from_str(raw).expect("fixture parses")
    }

    #[test]
    fn a_follower_reads_with_their_date() {
        assert_eq!(
            read_follow_page(&page(FOLLOWER_PAGE), true),
            FollowAnswer::Following("2023-03-04T05:06:07Z".into())
        );
        // Found is found, whoever asked.
        assert_eq!(
            read_follow_page(&page(FOLLOWER_PAGE), false),
            FollowAnswer::Following("2023-03-04T05:06:07Z".into())
        );
    }

    #[test]
    fn an_empty_page_is_not_following_only_when_it_is_authoritative() {
        assert_eq!(read_follow_page(&page(EMPTY_PAGE), true), FollowAnswer::NotFollowing);
        assert_eq!(read_follow_page(&page(EMPTY_PAGE), false), FollowAnswer::Unknown);
    }

    #[test]
    fn ivr_null_never_reads_as_not_following() {
        // What api.ivr.fi answers for every pair today.
        let ivr = json!({ "statusHidden": false, "followedAt": null });
        let at = ivr["followedAt"].as_str();
        assert_eq!(merge(at, FollowAnswer::Unknown), FollowAnswer::Unknown);
        assert_eq!(merge(at, FollowAnswer::NotFollowing), FollowAnswer::NotFollowing);
        assert_eq!(
            merge(Some("2021-01-01T00:00:00Z"), FollowAnswer::Unknown),
            FollowAnswer::Following("2021-01-01T00:00:00Z".into())
        );
    }
}
