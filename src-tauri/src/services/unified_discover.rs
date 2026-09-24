//! The "who else is live" lists, built here and handed over finished: Home's
//! Discover tab on the unified view, and the Sidebar's second section
//! (Recommended, or a platform's Top live). Both surfaces render them as-is.
//!
//! Before this, `Home.tsx` and `Sidebar.tsx` each fetched the other platforms'
//! directories themselves and, on every render, concatenated them with Twitch's
//! picks, filtered, sorted and deduplicated the result. Everything the lists
//! need already lived in Rust: the recommendations and live Twitch follows
//! (`home_snapshot`), every other platform's live follows
//! (`provider_live_service`), the live favourites (`favorite_live_service`) and
//! the favourite list (settings). One cache of directories now serves both
//! surfaces; each takes its own `View` of it.
//!
//! A build leaves out:
//!
//! - channels the Following tab already shows, matched by CHANNEL
//!   (`key::channel_ids`). Not by card key: a YouTube directory row is keyed by
//!   video id while a follow row for the same channel is often keyed by UC id,
//!   and a card-key compare lets both through;
//! - live favourites, which the Favourites section shows above the grid,
//!   matched by favourite id (`key::favorite_id`) for the same reason;
//! - repeated cards, by the key the grid renders with (`key::stream_key`),
//!   keeping the most watched.
//!
//! Across every platform it ranks by viewers. The sort is stable, so equal
//! counts keep source order: Twitch first, then each platform in `PROVIDER_IDS`
//! order. A single platform keeps its own order, as its directory ranks it.
//!
//! This module is pure state, with no clock, network or window of its own.
//! `home_snapshot` fetches each directory on its own task, lands it here and
//! emits whatever `rebuild` hands back, which is what lets the tests below land
//! platforms one at a time.

use std::collections::{HashMap, HashSet};

use serde::Serialize;

use crate::models::provider_stream::ProviderStream;
use crate::models::stream::TwitchStream;
use crate::services::providers::key::{channel_ids, favorite_id, stream_key, PROVIDER_IDS};

/// Live rows asked of each platform's directory. One YouTube search page is
/// about 20 rows, which made Discover look nearly empty beside Twitch's picks,
/// so its adapter follows continuations up to this: a few requests, not one per
/// row. Kick's sorted endpoint has no cursor and caps at 100 anyway.
pub const PER_PROVIDER: u32 = 100;

/// A Home arriving on the unified view, or its window coming back to the front,
/// refetches a directory older than this.
pub const MOUNT_STALE_SECS: u64 = 120;

/// While a unified Home stays on screen, a directory older than this is
/// refetched.
pub const PERIOD_SECS: u64 = 300;

/// A platform whose fetches keep failing leaves the list once a failure lands
/// this long after its last success, so a platform that is down stops
/// advertising streams that have long ended.
///
/// Time alone never drops rows. Nothing is fetched while the window is
/// minimized or in the tray, so after hours away there is no evidence the
/// platform is down, only that the rows are old: the window comes back to its
/// last list, refetched as it returns, rather than to a Discover that has lost
/// every platform but Twitch.
pub const MAX_AGE_SECS: u64 = 900;

/// Once a fetch starts, the same platform is not asked again for this long,
/// whatever the outcome, so remounting Home in a loop never becomes a request
/// loop.
pub const RETRY_FLOOR_SECS: u64 = 30;

/// A surface that shows one of these lists. Each is compared against its own
/// last list, so a change one of them cannot see costs the other nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Surface {
    /// Home's Discover grid on the unified view.
    Home,
    /// The Sidebar's second section.
    Sidebar,
}

/// How a surface reads the cache: which platforms, and how much of each.
#[derive(Clone, Copy, Debug)]
pub struct View<'a> {
    /// `"all"`, or one provider id (`"twitch"` is Twitch's picks alone).
    pub scope: &'a str,
    /// Rows taken from the top of each platform's directory.
    pub per_provider: usize,
}

impl<'a> View<'a> {
    /// Home's grid: every platform, the whole directory.
    pub fn home() -> View<'static> {
        View {
            scope: "all",
            per_provider: PER_PROVIDER as usize,
        }
    }

    /// The Sidebar's second section. Merged with Twitch's picks it takes the
    /// top 25 of each directory, since those picks carry most of the section;
    /// one platform on its own shows its top 50. The sizes it used to request.
    pub fn sidebar(scope: &'a str) -> View<'a> {
        View {
            scope,
            per_provider: if scope == "all" { 25 } else { 50 },
        }
    }

    fn all(&self) -> bool {
        self.scope == "all"
    }

    /// Whether this view reads `provider`'s directory.
    pub fn reads(&self, provider: &str) -> bool {
        provider != "twitch" && (self.all() || self.scope == provider)
    }
}

/// One row of the list, in the shape its source produced: a Twitch card keeps
/// every Helix field it renders, a platform card its `provider`, `key` and
/// `watch_url`. Untagged, so the page reads both as the `TwitchStream` rows it
/// used to merge itself.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(untagged)]
pub enum DiscoverRow {
    Twitch(TwitchStream),
    Provider(ProviderStream),
}

impl DiscoverRow {
    pub(crate) fn provider(&self) -> &str {
        match self {
            DiscoverRow::Twitch(_) => "twitch",
            DiscoverRow::Provider(p) => &p.provider,
        }
    }

    pub(crate) fn user_id(&self) -> &str {
        match self {
            DiscoverRow::Twitch(t) => &t.user_id,
            DiscoverRow::Provider(p) => &p.user_id,
        }
    }

    pub(crate) fn user_login(&self) -> &str {
        match self {
            DiscoverRow::Twitch(t) => &t.user_login,
            DiscoverRow::Provider(p) => &p.user_login,
        }
    }

    pub(crate) fn viewers(&self) -> u32 {
        match self {
            DiscoverRow::Twitch(t) => t.viewer_count,
            DiscoverRow::Provider(p) => p.viewer_count,
        }
    }

    pub(crate) fn card_key(&self) -> String {
        stream_key(self.provider(), self.user_login())
    }
}

/// One platform's directory as last fetched, and where its next fetch stands.
#[derive(Default)]
struct Directory {
    rows: Vec<ProviderStream>,
    /// Unix seconds of the last successful fetch; `None` until one lands.
    fetched_at: Option<u64>,
    /// Unix seconds the last fetch started, for `RETRY_FLOOR_SECS`.
    started_at: Option<u64>,
    /// Unix seconds of the last failed fetch since the last success, for
    /// `MAX_AGE_SECS`.
    failed_at: Option<u64>,
    in_flight: bool,
}

impl Directory {
    /// Rows worth showing: some have landed, and the platform has not kept
    /// failing for `MAX_AGE_SECS` since.
    fn showable(&self) -> bool {
        match (self.fetched_at, self.failed_at) {
            (None, _) => false,
            (Some(ok), Some(failed)) => failed.saturating_sub(ok) <= MAX_AGE_SECS,
            (Some(_), None) => true,
        }
    }
}

/// What a build ranks and subtracts that this module does not own, borrowed
/// from its owners for the length of one build.
pub struct Inputs<'a> {
    /// Twitch's picks, every page loaded so far.
    pub recommended: &'a [TwitchStream],
    /// Twitch follows that are live.
    pub followed_live: &'a [TwitchStream],
    /// Every other platform's live follows.
    pub provider_followed_live: &'a [ProviderStream],
    /// Live favourites, including channels followed nowhere.
    pub favorites_live: &'a [ProviderStream],
    /// `Settings.favorite_streamers`.
    pub favorite_ids: &'a [String],
}

/// Each platform's directory, and each surface's list as last handed to the
/// windows.
#[derive(Default)]
pub struct UnifiedDiscover {
    directories: HashMap<String, Directory>,
    /// Compared against on every rebuild, so an input change that leaves a
    /// list as it was costs no IPC. Kept with the scope it was built for: the
    /// page shows a list only under that scope, so a new scope is always sent.
    emitted: HashMap<Surface, (String, Vec<DiscoverRow>)>,
}

impl UnifiedDiscover {
    /// Claim a fetch of `provider`'s directory. `true`, with the fetch marked
    /// in flight, when it is due: none running, the last success at least
    /// `max_age` seconds old, and the last start at least `RETRY_FLOOR_SECS`.
    pub fn begin_fetch(&mut self, provider: &str, now: u64, max_age: u64) -> bool {
        let dir = self.directories.entry(provider.to_string()).or_default();
        let fresh = dir.fetched_at.is_some_and(|t| now.saturating_sub(t) < max_age);
        let too_soon = dir
            .started_at
            .is_some_and(|t| now.saturating_sub(t) < RETRY_FLOOR_SECS);
        if dir.in_flight || fresh || too_soon {
            return false;
        }
        dir.in_flight = true;
        dir.started_at = Some(now);
        true
    }

    /// A claimed fetch finished. A failure (`None`) keeps the rows it had: "we
    /// could not look" is not "nothing is live", and `MAX_AGE_SECS` retires
    /// them if the platform stays down.
    pub fn finish_fetch(&mut self, provider: &str, rows: Option<Vec<ProviderStream>>, now: u64) {
        let dir = self.directories.entry(provider.to_string()).or_default();
        dir.in_flight = false;
        match rows {
            Some(rows) => {
                dir.rows = rows;
                dir.fetched_at = Some(now);
                dir.failed_at = None;
            }
            None => dir.failed_at = Some(now),
        }
    }

    /// A view's list as it stands, without recording it.
    pub fn build(&self, inputs: &Inputs, view: View) -> Vec<DiscoverRow> {
        // Channels the Following tab lists: Twitch follows and every platform's.
        let following: HashSet<String> = inputs
            .followed_live
            .iter()
            .flat_map(|s| channel_ids("twitch", &s.user_id, &s.user_login))
            .chain(
                inputs
                    .provider_followed_live
                    .iter()
                    .filter(|s| s.is_live)
                    .flat_map(|s| channel_ids(&s.provider, &s.user_id, &s.user_login)),
            )
            .collect();

        // Favourites the Favourites section shows: a favourite with a live row
        // in any of the three places that section reads from.
        let favorites: HashSet<&str> = inputs.favorite_ids.iter().map(String::as_str).collect();
        let live_favorites: HashSet<String> = inputs
            .followed_live
            .iter()
            .filter_map(|s| favorite_id("twitch", &s.user_id, &s.user_login))
            .chain(
                inputs
                    .provider_followed_live
                    .iter()
                    .chain(inputs.favorites_live)
                    .filter(|s| s.is_live)
                    .filter_map(|s| favorite_id(&s.provider, &s.user_id, &s.user_login)),
            )
            .filter(|id| favorites.contains(id.as_str()))
            .collect();

        let picks: &[TwitchStream] = if view.all() || view.scope == "twitch" {
            inputs.recommended
        } else {
            &[]
        };
        let directories = PROVIDER_IDS
            .iter()
            .filter(|provider| view.reads(provider))
            .filter_map(|provider| self.directories.get(*provider))
            .filter(|dir| dir.showable())
            .flat_map(|dir| {
                dir.rows
                    .iter()
                    .take(view.per_provider)
                    .cloned()
                    .map(DiscoverRow::Provider)
            });

        let mut rows: Vec<DiscoverRow> = picks
            .iter()
            .cloned()
            .map(DiscoverRow::Twitch)
            .chain(directories)
            .filter(|row| {
                !channel_ids(row.provider(), row.user_id(), row.user_login())
                    .iter()
                    .any(|id| following.contains(id))
            })
            .filter(|row| {
                favorite_id(row.provider(), row.user_id(), row.user_login())
                    .map_or(true, |id| !live_favorites.contains(&id))
            })
            .collect();
        if view.all() {
            rows.sort_by(|a, b| b.viewers().cmp(&a.viewers()));
        }
        let mut seen = HashSet::new();
        rows.retain(|row| seen.insert(row.card_key()));
        rows
    }

    /// Build `surface`'s list and record it as handed out. `Some` when it
    /// differs from the last one that surface was sent, including a new scope
    /// and a surface never sent one, which is the caller's cue to emit.
    pub fn rebuild(&mut self, surface: Surface, view: View, inputs: &Inputs) -> Option<Vec<DiscoverRow>> {
        let list = self.build(inputs, view);
        if let Some((scope, last)) = self.emitted.get(&surface) {
            if scope == view.scope && *last == list {
                return None;
            }
        }
        self.emitted.insert(surface, (view.scope.to_string(), list.clone()));
        Some(list)
    }

    /// Forget what `surface` was last sent, so its next rebuild emits whatever
    /// it holds: a new scope, or a page that has not received any list yet.
    pub fn forget(&mut self, surface: Surface) {
        self.emitted.remove(&surface);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::providers::key::make_key;

    const T0: u64 = 1_000_000;
    const UC: &str = "UCMNEVbszv8ZyvSXoTn3yhpQ";

    fn twitch(user_id: &str, login: &str, viewers: u32) -> TwitchStream {
        TwitchStream {
            id: format!("s{user_id}"),
            user_id: user_id.to_string(),
            user_name: login.to_string(),
            user_login: login.to_string(),
            title: String::new(),
            viewer_count: viewers,
            game_id: String::new(),
            game_name: String::new(),
            thumbnail_url: String::new(),
            started_at: String::new(),
            broadcaster_type: None,
            has_shared_chat: None,
            profile_image_url: None,
            is_live: Some(true),
            tags: None,
            language: None,
        }
    }

    fn row(provider: &str, user_id: &str, login: &str, viewers: u32) -> ProviderStream {
        ProviderStream {
            provider: provider.to_string(),
            key: make_key(provider, login),
            id: String::new(),
            user_id: user_id.to_string(),
            user_login: login.to_string(),
            user_name: login.to_string(),
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

    /// Everything `Inputs` borrows, owned, so a test can change one thing and
    /// rebuild.
    #[derive(Default)]
    struct World {
        recommended: Vec<TwitchStream>,
        followed_live: Vec<TwitchStream>,
        provider_followed_live: Vec<ProviderStream>,
        favorites_live: Vec<ProviderStream>,
        favorite_ids: Vec<String>,
    }

    impl World {
        fn inputs(&self) -> Inputs<'_> {
            Inputs {
                recommended: &self.recommended,
                followed_live: &self.followed_live,
                provider_followed_live: &self.provider_followed_live,
                favorites_live: &self.favorites_live,
                favorite_ids: &self.favorite_ids,
            }
        }
    }

    /// Land `rows` as `provider`'s directory, through the same claim the
    /// service makes.
    fn land(d: &mut UnifiedDiscover, provider: &str, rows: Vec<ProviderStream>, now: u64) {
        assert!(d.begin_fetch(provider, now, MOUNT_STALE_SECS));
        d.finish_fetch(provider, Some(rows), now);
    }

    fn keys(list: &[DiscoverRow]) -> Vec<String> {
        list.iter().map(DiscoverRow::card_key).collect()
    }

    #[test]
    fn every_platform_is_ranked_together_by_viewers() {
        let mut d = UnifiedDiscover::default();
        let mut w = World::default();
        w.recommended = vec![twitch("1", "a", 50), twitch("2", "b", 5)];
        land(&mut d, "kick", vec![row("kick", "10", "k", 30)], T0);
        land(&mut d, "youtube", vec![row("youtube", UC, "vid00000001", 70)], T0);
        assert_eq!(keys(&d.build(&w.inputs(), View::home())), ["youtube:vid00000001", "a", "kick:k", "b"]);
    }

    #[test]
    fn equal_counts_keep_source_order_not_landing_order() {
        let mut d = UnifiedDiscover::default();
        let mut w = World::default();
        w.recommended = vec![twitch("1", "a", 10)];
        land(&mut d, "tiktok", vec![row("tiktok", "7", "t", 10)], T0);
        land(&mut d, "youtube", vec![row("youtube", UC, "vid00000001", 10)], T0);
        land(&mut d, "kick", vec![row("kick", "10", "k", 10)], T0);
        assert_eq!(
            keys(&d.build(&w.inputs(), View::home())),
            ["a", "kick:k", "youtube:vid00000001", "tiktok:t"]
        );
    }

    #[test]
    fn channels_the_following_tab_shows_are_left_out() {
        let mut d = UnifiedDiscover::default();
        let mut w = World::default();
        w.recommended = vec![twitch("1", "followed", 90), twitch("2", "stranger", 10)];
        w.followed_live = vec![twitch("1", "followed", 90), twitch("676", "bob", 1)];
        w.provider_followed_live = vec![
            // Kick follows are swept by slug; the directory spells it differently
            // and carries a different numeric id.
            row("kick", "999", "slugger", 1),
            // A YouTube follow from a live check: keyed by UC id, not by video.
            row("youtube", UC, UC, 1),
        ];
        land(
            &mut d,
            "kick",
            vec![
                row("kick", "10", "Slugger", 80),
                // Twitch user 676 is followed. Kick user 676 is someone else.
                row("kick", "676", "bob", 20),
            ],
            T0,
        );
        // The same YouTube channel, keyed by the broadcast's VIDEO id. The
        // page's card-key compare missed exactly this.
        land(&mut d, "youtube", vec![row("youtube", UC, "vid00000001", 70)], T0);
        assert_eq!(keys(&d.build(&w.inputs(), View::home())), ["kick:bob", "stranger"]);
    }

    #[test]
    fn a_youtube_follow_whose_row_carries_no_channel_id_still_matches() {
        let mut d = UnifiedDiscover::default();
        let mut w = World::default();
        // The live check found the stream but not the id: the login still
        // names the channel it was asked about.
        w.provider_followed_live = vec![row("youtube", "", UC, 1)];
        land(&mut d, "youtube", vec![row("youtube", UC, "vid00000001", 70)], T0);
        assert!(d.build(&w.inputs(), View::home()).is_empty());
    }

    #[test]
    fn live_favourites_are_left_out_by_favourite_id() {
        let mut d = UnifiedDiscover::default();
        let mut w = World::default();
        w.favorite_ids = vec![
            format!("youtube:{UC}"),
            "42".to_string(),
            "kick:resting".to_string(),
        ];
        w.recommended = vec![twitch("42", "fortytwo", 80), twitch("5", "plain", 5)];
        w.favorites_live = vec![
            // Swept live by the favourites watcher, keyed by UC id.
            row("youtube", UC, UC, 3),
            // The watcher's Twitch rows: provider "twitch", the Helix user id.
            row("twitch", "42", "fortytwo", 80),
            // Live but no longer a favourite (unhearted since the last sweep).
            row("kick", "11", "unhearted", 4),
        ];
        land(
            &mut d,
            "kick",
            vec![row("kick", "11", "unhearted", 40), row("kick", "12", "resting", 30)],
            T0,
        );
        land(&mut d, "youtube", vec![row("youtube", UC, "vid00000001", 70)], T0);
        // "resting" is a favourite nothing reports live, so the Favourites
        // section does not show it and Discover must keep it.
        assert_eq!(keys(&d.build(&w.inputs(), View::home())), ["kick:unhearted", "kick:resting", "plain"]);

        // Unhearting the YouTube channel brings its broadcast straight back.
        w.favorite_ids.retain(|id| id != &format!("youtube:{UC}"));
        assert_eq!(keys(&d.rebuild(Surface::Home, View::home(), &w.inputs()).unwrap())[0], "youtube:vid00000001");
    }

    #[test]
    fn one_card_per_card_key_keeping_the_most_watched() {
        let mut d = UnifiedDiscover::default();
        let w = World::default();
        land(
            &mut d,
            "kick",
            vec![row("kick", "10", "dup", 5), row("kick", "10", "DUP", 50)],
            T0,
        );
        // Two broadcasts from one YouTube channel are two cards, as before.
        land(
            &mut d,
            "youtube",
            vec![row("youtube", UC, "vid00000001", 7), row("youtube", UC, "vid00000002", 6)],
            T0,
        );
        let list = d.build(&w.inputs(), View::home());
        assert_eq!(keys(&list), ["kick:dup", "youtube:vid00000001", "youtube:vid00000002"]);
        let DiscoverRow::Provider(kept) = &list[0] else {
            panic!("expected the Kick row")
        };
        assert_eq!(kept.viewer_count, 50);
    }

    #[test]
    fn platforms_land_one_at_a_time_and_each_landing_is_emitted() {
        let mut d = UnifiedDiscover::default();
        let mut w = World::default();
        w.recommended = vec![twitch("1", "a", 50)];
        for provider in ["kick", "youtube", "tiktok"] {
            assert!(d.begin_fetch(provider, T0, MOUNT_STALE_SECS));
        }
        // Nothing has answered: the Twitch picks go out on their own.
        assert_eq!(keys(&d.rebuild(Surface::Home, View::home(), &w.inputs()).unwrap()), ["a"]);

        // Kick answers first and is emitted without waiting for the others.
        d.finish_fetch("kick", Some(vec![row("kick", "10", "k", 30)]), T0 + 1);
        assert_eq!(keys(&d.rebuild(Surface::Home, View::home(), &w.inputs()).unwrap()), ["a", "kick:k"]);
        assert_eq!(d.rebuild(Surface::Home, View::home(), &w.inputs()), None, "an unchanged list is not re-sent");

        // TikTok fails: nothing it had, nothing changes, nothing is sent.
        d.finish_fetch("tiktok", None, T0 + 2);
        assert_eq!(d.rebuild(Surface::Home, View::home(), &w.inputs()), None);

        // YouTube lands last and slots in by viewers.
        d.finish_fetch("youtube", Some(vec![row("youtube", UC, "vid00000001", 90)]), T0 + 3);
        assert_eq!(
            keys(&d.rebuild(Surface::Home, View::home(), &w.inputs()).unwrap()),
            ["youtube:vid00000001", "a", "kick:k"]
        );

        // A follow's viewer count ticking leaves the list alone.
        w.followed_live = vec![twitch("3", "c", 1)];
        assert_eq!(d.rebuild(Surface::Home, View::home(), &w.inputs()), None);
        w.followed_live[0].viewer_count = 2;
        assert_eq!(d.rebuild(Surface::Home, View::home(), &w.inputs()), None);
    }

    #[test]
    fn fetches_are_claimed_once_and_never_retried_hot() {
        let mut d = UnifiedDiscover::default();
        assert!(d.begin_fetch("kick", T0, MOUNT_STALE_SECS));
        assert!(!d.begin_fetch("kick", T0 + 1, MOUNT_STALE_SECS), "one fetch at a time");

        d.finish_fetch("kick", None, T0 + 2);
        assert!(!d.begin_fetch("kick", T0 + RETRY_FLOOR_SECS - 1, MOUNT_STALE_SECS));
        let t = T0 + RETRY_FLOOR_SECS;
        assert!(d.begin_fetch("kick", t, MOUNT_STALE_SECS));

        d.finish_fetch("kick", Some(Vec::new()), t);
        assert!(!d.begin_fetch("kick", t + MOUNT_STALE_SECS - 1, MOUNT_STALE_SECS));
        assert!(!d.begin_fetch("kick", t + PERIOD_SECS - 1, PERIOD_SECS));
        assert!(d.begin_fetch("kick", t + MOUNT_STALE_SECS, MOUNT_STALE_SECS));
    }

    #[test]
    fn a_platform_that_keeps_failing_drops_out_and_comes_back() {
        let mut d = UnifiedDiscover::default();
        let w = World::default();
        land(&mut d, "kick", vec![row("kick", "10", "k", 30)], T0);

        // One failed refresh keeps the last rows up: "we could not look".
        assert!(d.begin_fetch("kick", T0 + PERIOD_SECS, PERIOD_SECS));
        d.finish_fetch("kick", None, T0 + PERIOD_SECS);
        assert_eq!(keys(&d.build(&w.inputs(), View::home())), ["kick:k"]);

        // Still failing once the last success is MAX_AGE old: the rows go.
        let late = T0 + MAX_AGE_SECS + 1;
        assert!(d.begin_fetch("kick", late, PERIOD_SECS));
        d.finish_fetch("kick", None, late);
        assert!(d.build(&w.inputs(), View::home()).is_empty());

        // It answers again and is back.
        let back = late + RETRY_FLOOR_SECS;
        assert!(d.begin_fetch("kick", back, PERIOD_SECS));
        d.finish_fetch("kick", Some(vec![row("kick", "10", "k", 31)]), back);
        assert_eq!(keys(&d.build(&w.inputs(), View::home())), ["kick:k"]);
    }

    #[test]
    fn a_fetch_in_flight_never_hides_the_rows_it_will_replace() {
        // Back from hours in the tray: the refresh starts long after the last
        // success, and the old rows stay up until it lands or fails.
        let mut d = UnifiedDiscover::default();
        let w = World::default();
        land(&mut d, "kick", vec![row("kick", "10", "k", 30)], T0);
        assert!(d.begin_fetch("kick", T0 + 10 * MAX_AGE_SECS, MOUNT_STALE_SECS));
        assert_eq!(keys(&d.build(&w.inputs(), View::home())), ["kick:k"]);
    }

    #[test]
    fn the_sidebar_takes_the_top_of_each_directory_across_every_platform() {
        let mut d = UnifiedDiscover::default();
        let mut w = World::default();
        w.recommended = vec![twitch("1", "a", 5)];
        let kick = (0..30u32)
            .map(|i| row("kick", &i.to_string(), &format!("k{i:02}"), 100 - i))
            .collect();
        land(&mut d, "kick", kick, T0);
        let list = d.build(&w.inputs(), View::sidebar("all"));
        // The top 25 of Kick's 30, ranked in with Twitch's pick.
        assert_eq!(list.len(), 26);
        assert_eq!(keys(&list)[0], "kick:k00");
        assert!(!keys(&list).contains(&"kick:k25".to_string()));
        // Home's grid reads the whole directory from the same cache.
        assert_eq!(d.build(&w.inputs(), View::home()).len(), 31);
    }

    #[test]
    fn one_platform_keeps_its_own_order_and_no_twitch_picks() {
        let mut d = UnifiedDiscover::default();
        let mut w = World::default();
        w.recommended = vec![twitch("1", "a", 500)];
        // YouTube ranks by its own relevance, not viewers; that order stands.
        land(
            &mut d,
            "youtube",
            vec![row("youtube", UC, "vid00000001", 5), row("youtube", UC, "vid00000002", 50)],
            T0,
        );
        land(&mut d, "kick", vec![row("kick", "10", "k", 9)], T0);
        assert_eq!(
            keys(&d.build(&w.inputs(), View::sidebar("youtube"))),
            ["youtube:vid00000001", "youtube:vid00000002"]
        );
        // Twitch on its own is its picks, still minus what is shown above.
        assert_eq!(keys(&d.build(&w.inputs(), View::sidebar("twitch"))), ["a"]);
        w.followed_live = vec![twitch("1", "a", 500)];
        assert!(d.build(&w.inputs(), View::sidebar("twitch")).is_empty());
    }

    #[test]
    fn each_surface_is_compared_against_its_own_last_list() {
        let mut d = UnifiedDiscover::default();
        let mut w = World::default();
        w.recommended = vec![twitch("1", "a", 5)];
        assert!(d.rebuild(Surface::Home, View::home(), &w.inputs()).is_some());
        // The Sidebar has never been sent a list, so it gets one.
        assert!(d.rebuild(Surface::Sidebar, View::sidebar("all"), &w.inputs()).is_some());
        assert_eq!(d.rebuild(Surface::Sidebar, View::sidebar("all"), &w.inputs()), None);
        // A new scope is always sent, even when its rows happen to match.
        assert!(d.rebuild(Surface::Sidebar, View::sidebar("twitch"), &w.inputs()).is_some());
        // A page that has not received anything yet is sent it again.
        d.forget(Surface::Sidebar);
        assert!(d.rebuild(Surface::Sidebar, View::sidebar("twitch"), &w.inputs()).is_some());
        // None of that touched Home's list.
        assert_eq!(d.rebuild(Surface::Home, View::home(), &w.inputs()), None);
    }

    #[test]
    fn rows_reach_the_page_in_the_shape_it_already_reads() {
        let t = serde_json::to_value(DiscoverRow::Twitch(twitch("1", "a", 5))).unwrap();
        assert_eq!(t["user_login"], "a");
        assert!(t.get("provider").is_none(), "a Twitch row is read as Twitch by having none");
        let p = serde_json::to_value(DiscoverRow::Provider(row("kick", "10", "k", 5))).unwrap();
        assert_eq!(p["provider"], "kick");
        assert_eq!(p["key"], "kick:k");
    }
}
