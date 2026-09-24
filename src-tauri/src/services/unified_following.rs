//! Your channels across every platform, built here and handed over finished:
//! the Favourites shelf, the Following list (Home's tab and the Sidebar's
//! Followed section) and the offline roster. The surfaces only choose which
//! platform's rows to show.
//!
//! Before this, `Home.tsx` and `Sidebar.tsx` each merged Twitch's follows, the
//! other platforms' follows and the favourites sweep on every render, and both
//! listed every Twitch channel ahead of every other platform's while their
//! comments said the list ranks by viewers. Every input already lived in Rust:
//! live and offline Twitch follows (`home_snapshot`), the other platforms' live
//! follows (`provider_live_service`), live favourites (`favorite_live_service`),
//! and the follow list, favourite list and favourite identities (settings).
//!
//! A channel is identified by its favourite id (`key::favorite_id`), the same
//! id the Discover lists subtract by: a YouTube channel arrives keyed by video
//! id from one source and by UC id from another, and only the favourite id
//! names it once.
//!
//! Pure state, with no clock, network or window of its own.

use std::collections::{HashMap, HashSet};

use serde::Serialize;

use crate::models::provider_stream::ProviderStream;
use crate::models::settings::{FavoriteChannel, ProviderFollow};
use crate::models::stream::TwitchStream;
use crate::services::providers::key::{favorite_id, make_key};
use crate::services::providers::watch_urls::watch_url;
use crate::services::unified_discover::DiscoverRow;

/// What a build reads, borrowed from its owners for the length of one build.
pub struct Inputs<'a> {
    /// Twitch follows that are live.
    pub followed_live: &'a [TwitchStream],
    /// Twitch follows that are not.
    pub followed_offline: &'a [TwitchStream],
    /// Every other platform's live follows.
    pub provider_followed_live: &'a [ProviderStream],
    /// Live favourites, including channels followed nowhere.
    pub favorites_live: &'a [ProviderStream],
    /// The follow list, for every platform but Twitch.
    pub provider_follows: &'a [ProviderFollow],
    /// `Settings.favorite_streamers`.
    pub favorite_ids: &'a [String],
    /// The name and face kept for each favourite, for one live nowhere.
    pub favorite_channels: &'a [FavoriteChannel],
}

/// Every list, across every platform.
#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct Following {
    /// Live favourites, most watched first.
    pub favorites: Vec<DiscoverRow>,
    /// Live follows that are not favourites, most watched first across every
    /// platform.
    pub live: Vec<DiscoverRow>,
    /// Follows and favourites that are not live: Twitch's roster, then the
    /// other platforms' follows, then favourites followed nowhere.
    pub offline: Vec<DiscoverRow>,
}

fn row_favorite_id(row: &DiscoverRow) -> Option<String> {
    favorite_id(row.provider(), row.user_id(), row.user_login())
}

/// What a row is deduplicated on: its favourite id, else its own card, since a
/// row that names no channel is still worth showing.
fn identity(row: &DiscoverRow) -> String {
    row_favorite_id(row).unwrap_or_else(|| row.card_key())
}

/// A row for a channel with no live row to show: an offline follow, or a
/// favourite live nowhere.
fn offline_row(
    provider: &str,
    channel: &str,
    user_id: &str,
    name: Option<&str>,
    avatar: Option<&str>,
) -> DiscoverRow {
    DiscoverRow::Provider(ProviderStream {
        provider: provider.to_string(),
        key: make_key(provider, channel),
        id: channel.to_string(),
        // The avatar resolver keys off `user_id`, so the channel stands in where
        // there is no platform id.
        user_id: user_id.to_string(),
        user_login: channel.to_string(),
        user_name: name.filter(|n| !n.is_empty()).unwrap_or(channel).to_string(),
        title: String::new(),
        viewer_count: 0,
        game_id: String::new(),
        game_name: String::new(),
        category_thumbnail: None,
        thumbnail_url: String::new(),
        started_at: String::new(),
        profile_image_url: avatar.filter(|a| !a.is_empty()).map(str::to_string),
        is_live: false,
        watch_url: watch_url(provider, channel),
        tags: None,
    })
}

pub fn build(inputs: &Inputs) -> Following {
    let wanted: HashSet<&str> = inputs.favorite_ids.iter().map(String::as_str).collect();
    let twitch_live = || inputs.followed_live.iter().cloned().map(DiscoverRow::Twitch);
    let provider_live = || {
        inputs
            .provider_followed_live
            .iter()
            .filter(|s| s.is_live)
            .cloned()
            .map(DiscoverRow::Provider)
    };

    // Favourites, from every source a live row comes from, first of each
    // channel: a Twitch follow's row carries more than the sweep's.
    let mut seen = HashSet::new();
    let mut favorites: Vec<DiscoverRow> = twitch_live()
        .chain(provider_live())
        .chain(
            inputs
                .favorites_live
                .iter()
                .filter(|s| s.is_live)
                .cloned()
                .map(DiscoverRow::Provider),
        )
        .filter(|row| seen.insert(identity(row)))
        .filter(|row| row_favorite_id(row).is_some_and(|id| wanted.contains(id.as_str())))
        .collect();
    favorites.sort_by(|a, b| b.viewers().cmp(&a.viewers()));
    let live_favorites: HashSet<String> = favorites.iter().filter_map(row_favorite_id).collect();

    // Every other live follow, ranked across platforms. The sort is stable, so
    // equal counts keep Twitch first.
    let mut cards = HashSet::new();
    let mut live: Vec<DiscoverRow> = twitch_live()
        .chain(provider_live())
        .filter(|row| row_favorite_id(row).map_or(true, |id| !live_favorites.contains(&id)))
        .filter(|row| cards.insert(row.card_key()))
        .collect();
    live.sort_by(|a, b| b.viewers().cmp(&a.viewers()));

    // Other platforms' follows that are not live, matched by CHANNEL: a live
    // YouTube row is keyed by its video id while the follow holds the UC id, and
    // only the channel lines up.
    let live_channels: HashSet<String> = inputs
        .provider_followed_live
        .iter()
        .filter(|s| s.is_live)
        .flat_map(|s| [&s.user_id, &s.user_login].map(|id| (s.provider.as_str(), id.to_lowercase())))
        .filter(|(_, id)| !id.is_empty())
        .map(|(provider, id)| format!("{}:{}", provider, id))
        .collect();
    let offline_follows: Vec<DiscoverRow> = inputs
        .provider_follows
        .iter()
        .filter(|f| !live_channels.contains(&format!("{}:{}", f.provider, f.channel.to_lowercase())))
        .map(|f| {
            offline_row(
                &f.provider,
                &f.channel,
                &f.channel,
                f.display_name.as_deref(),
                f.avatar.as_deref(),
            )
        })
        .collect();

    // Favourites live nowhere. A real offline row stands in where there is one
    // (a follow on some platform), since it carries the platform's own ids; the
    // kept identity is the fallback for a favourite followed nowhere.
    // Twitch's roster is refreshed far less often than its live list, so a
    // channel that just went live can still be in it.
    let twitch_live_ids: HashSet<&str> = inputs.followed_live.iter().map(|s| s.user_id.as_str()).collect();
    let twitch_offline = || {
        inputs
            .followed_offline
            .iter()
            .filter(|s| !twitch_live_ids.contains(s.user_id.as_str()))
            .cloned()
            .map(DiscoverRow::Twitch)
    };
    let mut real: HashMap<String, DiscoverRow> = HashMap::new();
    for row in twitch_offline().chain(offline_follows.iter().cloned()) {
        if let Some(id) = row_favorite_id(&row) {
            real.entry(id).or_insert(row);
        }
    }
    let offline_favorites: Vec<DiscoverRow> = inputs
        .favorite_channels
        .iter()
        .filter(|f| wanted.contains(f.id.as_str()) && !live_favorites.contains(&f.id))
        .map(|f| {
            real.get(&f.id).cloned().unwrap_or_else(|| {
                // A Twitch favourite's id IS its user id, which the avatar
                // resolver and the profile card key off.
                let user_id = if f.provider == "twitch" { &f.id } else { &f.channel };
                offline_row(
                    &f.provider,
                    &f.channel,
                    user_id,
                    f.display_name.as_deref(),
                    f.avatar.as_deref(),
                )
            })
        })
        .collect();

    // A favourite you also follow is in two of these three.
    let mut seen = HashSet::new();
    let offline = twitch_offline()
        .chain(offline_follows)
        .chain(offline_favorites)
        .filter(|row| seen.insert(identity(row)))
        .collect();

    Following { favorites, live, offline }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn twitch(login: &str, id: &str, viewers: u32) -> TwitchStream {
        serde_json::from_value(serde_json::json!({
            "id": format!("s{id}"),
            "user_id": id,
            "user_login": login,
            "user_name": login,
            "game_id": "",
            "game_name": "",
            "type": "live",
            "title": "",
            "viewer_count": viewers,
            "started_at": "",
            "language": "en",
            "thumbnail_url": "",
            "tag_ids": [],
            "tags": [],
            "is_mature": false
        }))
        .expect("a TwitchStream")
    }

    fn provider(p: &str, login: &str, user_id: &str, viewers: u32) -> ProviderStream {
        ProviderStream {
            provider: p.into(),
            key: make_key(p, login),
            id: String::new(),
            user_id: user_id.into(),
            user_login: login.into(),
            user_name: login.into(),
            title: String::new(),
            viewer_count: viewers,
            game_id: String::new(),
            game_name: String::new(),
            category_thumbnail: None,
            thumbnail_url: String::new(),
            started_at: String::new(),
            profile_image_url: None,
            is_live: true,
            watch_url: String::new(),
            tags: None,
        }
    }

    fn follow(p: &str, channel: &str) -> ProviderFollow {
        serde_json::from_value(serde_json::json!({
            "provider": p,
            "channel": channel,
            "added_at": "",
        }))
        .expect("a ProviderFollow")
    }

    fn favorite(id: &str, p: &str, channel: &str) -> FavoriteChannel {
        FavoriteChannel {
            id: id.into(),
            provider: p.into(),
            channel: channel.into(),
            display_name: Some(format!("{channel} name")),
            avatar: None,
            added_at: String::new(),
        }
    }

    fn logins(rows: &[DiscoverRow]) -> Vec<&str> {
        rows.iter().map(|r| r.user_login()).collect()
    }

    struct Given {
        followed_live: Vec<TwitchStream>,
        followed_offline: Vec<TwitchStream>,
        provider_followed_live: Vec<ProviderStream>,
        favorites_live: Vec<ProviderStream>,
        provider_follows: Vec<ProviderFollow>,
        favorite_ids: Vec<String>,
        favorite_channels: Vec<FavoriteChannel>,
    }

    impl Given {
        fn new() -> Self {
            Given {
                followed_live: vec![],
                followed_offline: vec![],
                provider_followed_live: vec![],
                favorites_live: vec![],
                provider_follows: vec![],
                favorite_ids: vec![],
                favorite_channels: vec![],
            }
        }

        fn build(&self) -> Following {
            build(&Inputs {
                followed_live: &self.followed_live,
                followed_offline: &self.followed_offline,
                provider_followed_live: &self.provider_followed_live,
                favorites_live: &self.favorites_live,
                provider_follows: &self.provider_follows,
                favorite_ids: &self.favorite_ids,
                favorite_channels: &self.favorite_channels,
            })
        }
    }

    #[test]
    fn live_follows_rank_by_viewers_across_platforms() {
        let mut g = Given::new();
        g.followed_live = vec![twitch("small", "1", 40), twitch("mid", "2", 900)];
        g.provider_followed_live = vec![provider("tiktok", "big", "9", 12_000), provider("kick", "tie", "8", 40)];
        let f = g.build();
        // Not every Twitch channel first: a 12k TikTok LIVE outranks them.
        assert_eq!(logins(&f.live), ["big", "mid", "small", "tie"]);
    }

    #[test]
    fn a_favourite_shows_once_and_leaves_the_live_list() {
        let mut g = Given::new();
        g.favorite_ids = vec!["youtube:UCaaaaaaaaaaaaaaaaaaaaaa".into(), "2".into()];
        g.followed_live = vec![twitch("fav", "2", 10), twitch("other", "3", 5)];
        // The same YouTube channel from the follow poller (by video id as its
        // login) and from the favourites sweep (by UC id).
        g.provider_followed_live = vec![provider("youtube", "vid123abcde", "UCaaaaaaaaaaaaaaaaaaaaaa", 300)];
        g.favorites_live = vec![provider("youtube", "UCaaaaaaaaaaaaaaaaaaaaaa", "UCaaaaaaaaaaaaaaaaaaaaaa", 300)];
        let f = g.build();
        assert_eq!(logins(&f.favorites), ["vid123abcde", "fav"]);
        assert_eq!(logins(&f.live), ["other"]);
    }

    #[test]
    fn a_favourite_followed_nowhere_comes_from_the_sweep() {
        let mut g = Given::new();
        g.favorite_ids = vec!["kick:solo".into()];
        g.favorites_live = vec![provider("kick", "solo", "77", 50)];
        let f = g.build();
        assert_eq!(logins(&f.favorites), ["solo"]);
        assert!(f.live.is_empty(), "a favourite you don't follow is not a follow");
    }

    #[test]
    fn the_offline_roster_covers_every_platform_once() {
        let mut g = Given::new();
        g.followed_offline = vec![twitch("sleepy", "5", 0)];
        g.provider_follows = vec![follow("kick", "awake"), follow("kick", "resting"), follow("youtube", "UCbbbbbbbbbbbbbbbbbbbbbb")];
        // Live by its video id; the follow holds the UC id. Still live.
        g.provider_followed_live = vec![
            provider("kick", "awake", "10", 5),
            provider("youtube", "vidxyz12345", "UCbbbbbbbbbbbbbbbbbbbbbb", 7),
        ];
        // A favourite you also follow (offline), and one followed nowhere.
        g.favorite_ids = vec!["kick:resting".into(), "tiktok:ghost".into()];
        g.favorite_channels = vec![favorite("kick:resting", "kick", "resting"), favorite("tiktok:ghost", "tiktok", "ghost")];
        let f = g.build();
        assert_eq!(logins(&f.offline), ["sleepy", "resting", "ghost"]);
        let ghost = f.offline.iter().find(|r| r.user_login() == "ghost").unwrap();
        match ghost {
            DiscoverRow::Provider(p) => {
                assert_eq!(p.user_name, "ghost name");
                assert_eq!(p.watch_url, "https://www.tiktok.com/@ghost/live");
                assert!(!p.is_live);
            }
            DiscoverRow::Twitch(_) => panic!("a TikTok favourite is a platform row"),
        }
    }

    #[test]
    fn a_channel_that_just_went_live_leaves_the_offline_roster() {
        let mut g = Given::new();
        g.followed_live = vec![twitch("woke", "5", 30)];
        // The roster has not been refreshed since.
        g.followed_offline = vec![twitch("woke", "5", 0), twitch("asleep", "6", 0)];
        let f = g.build();
        assert_eq!(logins(&f.offline), ["asleep"]);
        assert_eq!(logins(&f.live), ["woke"]);
    }

    #[test]
    fn a_live_favourite_is_not_in_the_offline_roster() {
        let mut g = Given::new();
        g.favorite_ids = vec!["2".into()];
        g.favorite_channels = vec![favorite("2", "twitch", "fav")];
        g.followed_live = vec![twitch("fav", "2", 10)];
        let f = g.build();
        assert!(f.offline.is_empty());
        assert_eq!(logins(&f.favorites), ["fav"]);
    }
}
