//! Channels the viewer hid. A hidden channel never shows in browsing lists
//! (Discover, the Sidebar's suggestions, a category's streams, another
//! platform's directory), and, per channel, nowhere else either: their chat
//! messages, search results, and follows and favourites (the Following lists
//! and live notifications). Stream overlays have their own blocklist and are
//! not touched.
//!
//! The list lives in settings (`hidden_channels`), edited only through the
//! `set_channel_hidden*` commands. Entries are keyed the way favourites are
//! (`providers::key::favorite_id`): a Twitch user id, or `provider:channel`
//! elsewhere. The keys are held here per realm, refreshed on every settings
//! change, so every list can be filtered without a lock on settings. Chat
//! reads its set through `chat_accounts` into the compiled chat rules.

use std::collections::HashSet;
use std::sync::{OnceLock, RwLock};

use crate::models::provider_stream::ProviderStream;
use crate::models::settings::{FavoriteChannel, HiddenChannel, HiddenRealms, Settings};
use crate::models::stream::TwitchStream;
use crate::services::providers::key::favorite_id;
use crate::services::unified_discover::DiscoverRow;

/// Where a list is shown, for which hidden channels leave it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Realm {
    /// Discover, suggestions, categories, directories: every hidden channel.
    Browse,
    Search,
    Follows,
}

#[derive(Default)]
struct Held {
    browse: HashSet<String>,
    search: HashSet<String>,
    follows: HashSet<String>,
    /// Live notifications name a Twitch channel by login, others by key.
    notify: HashSet<String>,
}

static HIDDEN: OnceLock<RwLock<Held>> = OnceLock::new();

fn held() -> &'static RwLock<Held> {
    HIDDEN.get_or_init(|| RwLock::new(Held::default()))
}

/// Take the hidden list from freshly loaded or saved settings.
pub fn refresh(settings: &Settings) {
    let mut next = Held::default();
    for c in &settings.hidden_channels {
        let id = c.channel.id.clone();
        next.browse.insert(id.clone());
        if c.realms.search {
            next.search.insert(id.clone());
        }
        if c.realms.follows {
            next.follows.insert(id.clone());
            next.notify.insert(if c.channel.provider == "twitch" { c.channel.channel.to_lowercase() } else { id });
        }
    }
    if let Ok(mut h) = held().write() {
        *h = next;
    }
}

fn set_of(h: &Held, realm: Realm) -> &HashSet<String> {
    match realm {
        Realm::Browse => &h.browse,
        Realm::Search => &h.search,
        Realm::Follows => &h.follows,
    }
}

/// Run `f` with the realm's keys, or skip it while that realm hides nobody.
fn with_keys(realm: Realm, f: impl FnOnce(&HashSet<String>)) {
    if let Ok(h) = held().read() {
        let keys = set_of(&h, realm);
        if !keys.is_empty() {
            f(keys);
        }
    }
}

fn hit(keys: &HashSet<String>, provider: &str, user_id: &str, user_login: &str) -> bool {
    favorite_id(provider, user_id, user_login).is_some_and(|k| keys.contains(&k))
}

/// Drop hidden channels from a list of Twitch rows shown in `realm`.
pub fn retain_twitch(rows: &mut Vec<TwitchStream>, realm: Realm) {
    with_keys(realm, |keys| rows.retain(|s| !hit(keys, "twitch", &s.user_id, &s.user_login)));
}

/// Drop hidden channels from another platform's rows shown in `realm`.
pub fn retain_provider(rows: &mut Vec<ProviderStream>, realm: Realm) {
    with_keys(realm, |keys| rows.retain(|s| !hit(keys, &s.provider, &s.user_id, &s.user_login)));
}

/// Drop hidden channels from a mixed list shown in `realm`.
pub fn retain_discover(rows: &mut Vec<DiscoverRow>, realm: Realm) {
    with_keys(realm, |keys| rows.retain(|r| !hit(keys, r.provider(), r.user_id(), r.user_login())));
}

/// Whether a go-live notice for `streamer_login` (a Twitch login, or another
/// platform's composite key) is for a channel hidden from follows.
pub fn hides_notification(streamer_login: &str) -> bool {
    held().read().map(|h| h.notify.contains(&streamer_login.to_lowercase())).unwrap_or(false)
}

/// The accounts whose chat messages are hidden, as the chat rules match them:
/// `twitch:<user id>` for Twitch, the favourite key (`kick:<slug>`, ...)
/// elsewhere. With each, the login, for clearing rows already on screen.
pub fn chat_accounts(settings: &Settings) -> Vec<(String, String)> {
    settings
        .hidden_channels
        .iter()
        .filter(|c| c.realms.chat)
        .map(|c| {
            let key = if c.channel.provider == "twitch" {
                format!("twitch:{}", c.channel.id)
            } else {
                c.channel.id.to_lowercase()
            };
            (key, c.channel.channel.to_lowercase())
        })
        .collect()
}

/// Hide (`hidden`) or show `channel` again, in place. Returns false when
/// nothing changed. A channel hidden again refreshes its saved name and face
/// and keeps its block.
pub fn apply(list: &mut Vec<HiddenChannel>, channel: FavoriteChannel, hide: bool, realms: HiddenRealms) -> bool {
    if channel.id.trim().is_empty() {
        return false;
    }
    let at = list.iter().position(|c| c.channel.id == channel.id);
    match (hide, at) {
        (true, Some(i)) => {
            if list[i].channel == channel {
                return false;
            }
            list[i].channel = channel;
            true
        }
        (true, None) => {
            list.push(HiddenChannel { channel, blocked: false, realms });
            true
        }
        (false, Some(i)) => {
            list.remove(i);
            true
        }
        (false, None) => false,
    }
}

/// The hidden Twitch channel `id` StreamNook may block or unblock: its user id.
/// Only a hidden Twitch channel qualifies; other platforms have no block here.
pub fn twitch_target(list: &[HiddenChannel], id: &str) -> Option<String> {
    list.iter()
        .find(|c| c.channel.id == id && c.channel.provider == "twitch")
        .map(|c| c.channel.id.clone())
}

/// Change where hidden channel `id` is hidden besides browsing.
pub fn set_realms(list: &mut [HiddenChannel], id: &str, realms: HiddenRealms) -> bool {
    match list.iter_mut().find(|c| c.channel.id == id) {
        Some(c) if c.realms != realms => {
            c.realms = realms;
            true
        }
        _ => false,
    }
}

/// Record that hidden channel `id` is (or is no longer) blocked on Twitch.
pub fn mark_blocked(list: &mut [HiddenChannel], id: &str, blocked: bool) -> bool {
    match list.iter_mut().find(|c| c.channel.id == id) {
        Some(c) if c.blocked != blocked => {
            c.blocked = blocked;
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: HiddenRealms = HiddenRealms { chat: true, search: true, follows: true };
    const BROWSE_ONLY: HiddenRealms = HiddenRealms { chat: false, search: false, follows: false };

    fn row(id: &str, provider: &str, channel: &str) -> FavoriteChannel {
        FavoriteChannel {
            id: id.into(),
            provider: provider.into(),
            channel: channel.into(),
            display_name: None,
            avatar: None,
            added_at: String::new(),
        }
    }

    fn twitch(id: &str, login: &str) -> TwitchStream {
        serde_json::from_value(serde_json::json!({
            "id": "s", "user_id": id, "user_name": login, "user_login": login, "title": "",
            "viewer_count": 1, "game_name": "", "thumbnail_url": "", "started_at": ""
        }))
        .unwrap()
    }

    #[test]
    fn hiding_and_showing_edit_the_list_in_place() {
        let mut list = vec![];
        assert!(apply(&mut list, row("123", "twitch", "bob"), true, ALL));
        assert!(!apply(&mut list, row("123", "twitch", "bob"), true, ALL), "already hidden");
        assert!(apply(&mut list, row("kick:alice", "kick", "alice"), true, BROWSE_ONLY));
        assert_eq!(list[1].realms, BROWSE_ONLY, "the defaults at hide time are kept");
        assert!(apply(&mut list, row("123", "twitch", ""), false, ALL));
        assert!(!apply(&mut list, row("123", "twitch", ""), false, ALL));
        assert_eq!(list.len(), 1);
        assert!(!apply(&mut list, row(" ", "twitch", "x"), true, ALL), "no key, no entry");
        assert!(set_realms(&mut list, "kick:alice", ALL));
        assert!(!set_realms(&mut list, "kick:alice", ALL));
    }

    #[test]
    fn only_a_hidden_twitch_channel_can_be_blocked_and_hiding_again_keeps_the_block() {
        let mut list = vec![];
        apply(&mut list, row("123", "twitch", "bob"), true, ALL);
        apply(&mut list, row("kick:alice", "kick", "alice"), true, ALL);
        assert_eq!(twitch_target(&list, "123").as_deref(), Some("123"));
        assert_eq!(twitch_target(&list, "kick:alice"), None, "Kick has no block here");
        assert_eq!(twitch_target(&list, "999"), None, "not hidden");
        assert!(mark_blocked(&mut list, "123", true));
        assert!(!mark_blocked(&mut list, "123", true));
        let mut renamed = row("123", "twitch", "bob");
        renamed.display_name = Some("Bob".into());
        assert!(apply(&mut list, renamed, true, BROWSE_ONLY));
        assert!(list[0].blocked, "a refreshed name keeps the block");
        assert_eq!(list[0].realms, ALL, "and its realms");
        let json = serde_json::to_value(&list[0]).unwrap();
        assert_eq!(json["id"], "123", "flattened into the favourites' shape");
        assert_eq!(json["blocked"], true);
        assert_eq!(json["chat"], true);
        let old: HiddenChannel = serde_json::from_value(serde_json::json!({
            "id": "1", "provider": "twitch", "channel": "x", "added_at": ""
        }))
        .unwrap();
        assert_eq!(old.realms, ALL, "an entry saved before realms is hidden everywhere");
    }

    #[test]
    fn each_realm_leaves_out_only_the_channels_hidden_from_it() {
        let mut settings = Settings::default();
        settings.hidden_channels = vec![
            HiddenChannel { channel: row("123", "twitch", "Bob"), blocked: false, realms: ALL },
            HiddenChannel { channel: row("456", "twitch", "carol"), blocked: false, realms: BROWSE_ONLY },
        ];
        refresh(&settings);
        let lists = || vec![twitch("123", "bob"), twitch("456", "carol"), twitch("789", "dave")];
        let left = |realm| {
            let mut rows = lists();
            retain_twitch(&mut rows, realm);
            rows.into_iter().map(|r| r.user_login).collect::<Vec<_>>()
        };
        assert_eq!(left(Realm::Browse), vec!["dave"]);
        assert_eq!(left(Realm::Search), vec!["carol", "dave"]);
        assert_eq!(left(Realm::Follows), vec!["carol", "dave"]);
        assert!(hides_notification("bob"), "Twitch notices name the login");
        assert!(!hides_notification("carol"));
        assert_eq!(chat_accounts(&settings), vec![("twitch:123".to_string(), "bob".to_string())]);
        refresh(&Settings::default());
        assert_eq!(left(Realm::Browse).len(), 3, "an empty list hides nothing");
    }
}
