//! The "add a chat" picker on every app: your follows that are live, then
//! your follows that are offline, each row marked when its chat is open.
//!
//! Built from what the Following snapshot already holds (`home_snapshot`'s
//! unified Following lists and the offline roster's last broadcasts), so
//! opening the picker makes no request. Searching any channel stays
//! `search_chat_rooms`.
//!
//! Rows are keyed like the chat tabs: the bare lowercased login on Twitch,
//! `provider:slug` otherwise.

use crate::services::providers::key::{make_key, normalize_channel};
use crate::services::unified_discover::DiscoverRow;
use crate::services::unified_following::Following;
use serde::Serialize;
use std::collections::{HashMap, HashSet};

/// One channel the picker lists.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct PickerRow {
    pub provider: String,
    /// The key its chat tab has.
    pub key: String,
    pub user_login: String,
    /// The numeric Twitch id; empty on other platforms.
    pub user_id: String,
    pub user_name: String,
    pub profile_image_url: Option<String>,
    pub is_live: bool,
    pub viewer_count: Option<u64>,
    pub game_name: Option<String>,
    /// When an offline channel last went live (RFC 3339), when Twitch said.
    pub last_live: Option<String>,
    /// Its chat is open already: the row switches to it rather than adding.
    pub open: bool,
}

/// The picker's two lists.
#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct ChatPicker {
    /// Live follows in the Following tab's order: favourites, then the rest,
    /// most watched first.
    pub live: Vec<PickerRow>,
    /// Offline follows, most recently live first; never-seen-live last, in
    /// the roster's order.
    pub offline: Vec<PickerRow>,
    /// The lists span platforms, so rows wear a platform mark.
    pub mixed_platforms: bool,
}

fn row(r: &DiscoverRow, live: bool, last_live: Option<String>, open: &HashSet<String>) -> Option<PickerRow> {
    let (provider, login, user_id, name, avatar, viewers, game) = match r {
        DiscoverRow::Twitch(t) => (
            "twitch".to_string(),
            t.user_login.clone(),
            t.user_id.clone(),
            t.user_name.clone(),
            t.profile_image_url.clone(),
            t.viewer_count,
            t.game_name.clone(),
        ),
        DiscoverRow::Provider(p) => (
            p.provider.to_lowercase(),
            p.user_login.clone(),
            String::new(),
            p.user_name.clone(),
            p.profile_image_url.clone(),
            p.viewer_count,
            p.game_name.clone(),
        ),
    };
    let login = normalize_channel(&provider, login.trim());
    if login.is_empty() {
        return None;
    }
    let key = if provider == "twitch" { login.clone() } else { make_key(&provider, &login).to_lowercase() };
    Some(PickerRow {
        open: open.contains(&key),
        user_name: if name.trim().is_empty() { login.clone() } else { name },
        key,
        provider,
        user_login: login,
        user_id,
        profile_image_url: avatar.filter(|a| !a.trim().is_empty()),
        is_live: live,
        viewer_count: live.then_some(u64::from(viewers)),
        game_name: Some(game).filter(|g| live && !g.trim().is_empty()),
        last_live,
    })
}

/// The row's own data says the channel is not live (a Kick row whose stream ended before the
/// lists were rebuilt): it belongs with the offline follows, not in Live with no audience.
fn says_offline(r: &DiscoverRow) -> bool {
    match r {
        DiscoverRow::Twitch(t) => t.is_live == Some(false),
        DiscoverRow::Provider(p) => !p.is_live,
    }
}

/// The picker from the Following lists. `platforms` keeps the ones a shell
/// can chat on (None keeps every one); `open` holds the keys of chats open now.
pub fn build(
    following: &Following,
    last_broadcasts: &HashMap<String, Option<String>>,
    platforms: Option<&[String]>,
    open: &HashSet<String>,
) -> ChatPicker {
    let allowed = |r: &PickerRow| platforms.is_none_or(|p| p.iter().any(|x| x.eq_ignore_ascii_case(&r.provider)));
    let mut seen: HashSet<String> = HashSet::new();
    let mut live = Vec::new();
    let mut ended = Vec::new();
    for r in following.favorites.iter().chain(following.live.iter()) {
        if says_offline(r) {
            ended.push(r);
            continue;
        }
        if let Some(r) = row(r, true, None, open).filter(|r| allowed(r)) {
            if seen.insert(r.key.clone()) {
                live.push(r);
            }
        }
    }
    let mut offline = Vec::new();
    for r in ended.into_iter().chain(following.offline.iter()) {
        let last = match r {
            DiscoverRow::Twitch(t) => last_broadcasts.get(&t.user_id).cloned().flatten(),
            DiscoverRow::Provider(_) => None,
        };
        if let Some(r) = row(r, false, last, open).filter(|r| allowed(r)) {
            if seen.insert(r.key.clone()) {
                offline.push(r);
            }
        }
    }
    // RFC 3339 times from one source sort as text; a stable sort keeps the
    // roster's order among the never-seen-live.
    offline.sort_by(|a, b| match (&a.last_live, &b.last_live) {
        (Some(x), Some(y)) => y.cmp(x),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
    let first = live.first().or(offline.first()).map(|r| r.provider.clone());
    let mixed_platforms = live.iter().chain(offline.iter()).any(|r| Some(&r.provider) != first.as_ref());
    ChatPicker { live, offline, mixed_platforms }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::stream::TwitchStream;

    fn twitch(login: &str, id: &str, viewers: u32) -> DiscoverRow {
        let t: TwitchStream = serde_json::from_value(serde_json::json!({
            "id": id, "user_id": id, "user_name": login.to_uppercase(), "user_login": login, "title": "",
            "viewer_count": viewers, "game_name": "Art", "thumbnail_url": "", "started_at": "",
        }))
        .expect("a minimal Twitch row");
        DiscoverRow::Twitch(t)
    }

    fn kick(slug: &str) -> DiscoverRow {
        let p: crate::models::provider_stream::ProviderStream = serde_json::from_value(serde_json::json!({
            "provider": "kick", "key": format!("kick:{slug}"), "user_login": slug, "user_name": slug,
            "is_live": true, "watch_url": "",
        }))
        .expect("a minimal provider row");
        DiscoverRow::Provider(p)
    }

    fn keys(rows: &[PickerRow]) -> Vec<&str> {
        rows.iter().map(|r| r.key.as_str()).collect()
    }

    #[test]
    fn favourites_lead_then_live_then_offline_by_last_live() {
        let following = Following {
            favorites: vec![twitch("fav", "1", 10)],
            live: vec![twitch("big", "2", 900), kick("salty")],
            offline: vec![twitch("old", "3", 0), twitch("never", "4", 0), twitch("recent", "5", 0)],
            ..Default::default()
        };
        let last = HashMap::from([
            ("3".to_string(), Some("2026-10-01T10:00:00Z".to_string())),
            ("4".to_string(), None),
            ("5".to_string(), Some("2026-10-17T22:00:00Z".to_string())),
        ]);
        let p = build(&following, &last, None, &HashSet::new());
        assert_eq!(keys(&p.live), ["fav", "big", "kick:salty"]);
        assert_eq!(keys(&p.offline), ["recent", "old", "never"]);
        assert!(p.mixed_platforms);
        assert_eq!(p.live[1].viewer_count, Some(900));
        assert_eq!(p.offline[0].viewer_count, None);
        assert_eq!(p.offline[0].game_name, None, "an offline row names no category");
    }

    #[test]
    fn a_live_listed_row_that_says_offline_sorts_as_offline() {
        let mut ended = kick("xqc");
        if let DiscoverRow::Provider(p) = &mut ended {
            p.is_live = false;
        }
        let following = Following { live: vec![twitch("xqc", "1", 30), ended], ..Default::default() };
        let p = build(&following, &HashMap::new(), None, &HashSet::new());
        assert_eq!(keys(&p.live), ["xqc"]);
        assert_eq!(keys(&p.offline), ["kick:xqc"]);
        assert!(!p.offline[0].is_live);
        assert!(p.mixed_platforms, "the Kick row keeps its mark beside the Twitch one");
    }

    #[test]
    fn open_chats_are_marked_and_platforms_filter() {
        let following = Following { live: vec![twitch("a", "1", 5), kick("b")], ..Default::default() };
        let open = HashSet::from(["a".to_string()]);
        let p = build(&following, &HashMap::new(), Some(&["twitch".to_string()]), &open);
        assert_eq!(keys(&p.live), ["a"]);
        assert!(p.live[0].open);
        assert!(!p.mixed_platforms);
    }

    #[test]
    fn a_channel_listed_twice_shows_once() {
        let following = Following {
            favorites: vec![twitch("a", "1", 5)],
            live: vec![twitch("A", "1", 5)],
            offline: vec![twitch("a", "1", 0)],
            ..Default::default()
        };
        let p = build(&following, &HashMap::new(), None, &HashSet::new());
        assert_eq!(keys(&p.live), ["a"]);
        assert!(p.offline.is_empty());
    }
}
