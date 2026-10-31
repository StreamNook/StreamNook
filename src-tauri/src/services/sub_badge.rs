//! The subscriber badge art a Subscribe control shows for a channel: the
//! channel's own badge for the months, else the platform's standard badge.
//! Always an answer, so no control falls back to a placeholder.

use std::time::Duration;

use crate::services::channel_relationship::{badge_art, badge_version};
use crate::services::sub_standing::{self, SubBadgeRung};

/// Twitch's standard subscriber badge, for a channel with no badge of its own
/// when no badge set could be read.
pub const TWITCH_STANDARD_SUB_BADGE: &str =
    "https://static-cdn.jtvnw.net/badges/v1/5d9f2208-5dd8-11e7-8513-2ff4adfae661/2";

/// How long a Kick channel resolve may take before the standard art answers.
const KICK_RESOLVE_BUDGET: Duration = Duration::from_secs(5);

/// The tier 1 badge a channel's ladder shows at `months`: the rung held, else
/// the lowest rung (what a new subscriber would wear).
pub fn ladder_art(ladder: &[SubBadgeRung], months: i64) -> Option<String> {
    sub_standing::tenure_badges(ladder, None, months)
        .0
        .or_else(|| {
            ladder
                .iter()
                .filter(|r| r.version < 1000)
                .min_by_key(|r| r.version)
                .map(|r| r.art.clone())
        })
}

/// The first non-empty art, in the order given, else `standard`.
pub fn first_art(candidates: [Option<String>; 3], standard: &str) -> String {
    candidates
        .into_iter()
        .flatten()
        .find(|a| !a.is_empty())
        .unwrap_or_else(|| standard.to_string())
}

/// Twitch: the channel's ladder from public GQL (no sign-in), then its Helix
/// badge set (needs a sign-in), then the cached global set, then the standard
/// badge. A source is asked only when every one before it had nothing.
pub async fn twitch_art(login: &str, channel_id: Option<&str>, months: i64) -> String {
    let login = login.trim().to_lowercase();
    let ladder = if login.is_empty() {
        None
    } else {
        ladder_art(&sub_standing::room_badges(&login).await.ladder, months)
    };
    if ladder.is_some() {
        return first_art([ladder, None, None], TWITCH_STANDARD_SUB_BADGE);
    }
    let version = badge_version(months);
    let helix = match channel_id.map(str::trim).filter(|id| !id.is_empty()) {
        Some(id) => crate::commands::badges::fetch_channel_badges(id.to_string())
            .await
            .ok()
            .and_then(|set| badge_art(&set, "subscriber", version)),
        None => None,
    };
    if helix.is_some() {
        return first_art([None, helix, None], TWITCH_STANDARD_SUB_BADGE);
    }
    let global = match crate::commands::badges::get_cached_global_badges().await {
        Ok(Some(set)) => badge_art(&set, "subscriber", version),
        _ => None,
    };
    first_art([None, None, global], TWITCH_STANDARD_SUB_BADGE)
}

/// Kick: the channel's custom tiers from its resolve (resolved now when it
/// never was, within a budget; a cached channel answers at once), else Kick's
/// standard sparkle.
pub async fn kick_art(slug: &str, months: i64) -> String {
    let slug = slug.trim().to_lowercase();
    let slug = slug.strip_prefix("kick:").unwrap_or(&slug).to_string();
    if !slug.is_empty() {
        let _ = tokio::time::timeout(
            KICK_RESOLVE_BUDGET,
            crate::services::providers::kick::ensure_resolved(&slug, false),
        )
        .await;
    }
    let custom = crate::services::providers::kick::sub_badge_art(&slug, months.max(0) as u32);
    let standard = crate::services::chat_row_view::kick_badge_art("subscriber").unwrap_or_default();
    custom.filter(|a| !a.is_empty()).unwrap_or(standard)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rung(version: u32, art: &str) -> SubBadgeRung {
        SubBadgeRung {
            version,
            art: art.into(),
        }
    }

    #[test]
    fn the_ladder_gives_the_rung_held_else_the_lowest() {
        let ladder = vec![
            rung(3, "m3"),
            rung(6, "m6"),
            rung(12, "m12"),
            rung(2000, "t2"),
        ];
        assert_eq!(ladder_art(&ladder, 7).as_deref(), Some("m6"));
        assert_eq!(ladder_art(&ladder, 12).as_deref(), Some("m12"));
        // Below the first rung: a new subscriber on a ladder that starts at 3.
        assert_eq!(ladder_art(&ladder, 0).as_deref(), Some("m3"));
        assert_eq!(ladder_art(&[], 0), None);
    }

    #[test]
    fn the_first_art_found_wins_else_the_standard() {
        assert_eq!(
            first_art([Some("a".into()), Some("b".into()), None], "std"),
            "a"
        );
        assert_eq!(
            first_art([None, Some("b".into()), Some("c".into())], "std"),
            "b"
        );
        assert_eq!(first_art([None, None, Some(String::new())], "std"), "std");
        assert_eq!(
            first_art([None, None, None], TWITCH_STANDARD_SUB_BADGE),
            TWITCH_STANDARD_SUB_BADGE
        );
    }

    #[test]
    fn a_kick_channel_without_the_tier_shows_its_lowest() {
        use crate::services::providers::kick::sub_badge_or_lowest;
        let subs = vec![(6, "k6".to_string()), (1, "k1".to_string())];
        assert_eq!(sub_badge_or_lowest(&subs, 3).as_deref(), Some("k1"));
        assert_eq!(sub_badge_or_lowest(&subs, 0).as_deref(), Some("k1"));
        assert_eq!(sub_badge_or_lowest(&subs, 9).as_deref(), Some("k6"));
        assert_eq!(sub_badge_or_lowest(&[], 9), None);
    }

    #[test]
    fn kick_s_standard_sparkle_is_bundled() {
        let art =
            crate::services::chat_row_view::kick_badge_art("subscriber").expect("bundled sparkle");
        assert!(art.starts_with("data:image/png;base64,"));
    }

    #[test]
    fn a_signed_out_relationship_still_carries_badge_art() {
        let src = include_str!("channel_relationship.rs").replace("\r\n", "\n");
        let start = src
            .find("let Some(me) = crate::services::account_store::AccountStore::primary() else {")
            .expect("the signed-in check");
        let end = src[start..].find("};").expect("the end of its else");
        assert!(
            src[start..start + end].contains("sub_badge::twitch_art("),
            "signed out, the control still gets art"
        );
        assert!(src.contains("let badge_url = Some(crate::services::sub_badge::twitch_art(channel_login, Some(channel_id), months).await);"));
    }
}
