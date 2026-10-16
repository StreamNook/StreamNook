//! The phone chat pane's open rooms: the tab that follows the watched stream,
//! the rooms opened beside it, and which one is on screen.
//!
//! Both phone shells (the Android page and the iPhone app) open several rooms
//! at once and switch between them without reconnecting. The rules for that
//! list live here so the two shells cannot drift: the stream tab is first and
//! cannot be closed, a room already open is never opened twice, closing the
//! room on screen falls back to the first one left, and the stream tab follows
//! the player unless the viewer is sitting on another room.
//!
//! Connections stay with each shell. Every change answers which rooms the
//! shell must now join (`acquire`) and leave (`release`); the Android page
//! hands those to its reference-counted chat store, the iPhone app to the core's
//! join and leave commands. Tabs are not saved: they live for the app session.
//!
//! Twitch rooms are keyed by the bare lowercased login and carry the numeric
//! Twitch id; every other platform is keyed `provider:slug` and carries no id,
//! because a Kick id is numeric too and, handed to Helix, names an unrelated
//! Twitch account. Twitch `xqc` and Kick `xqc` are two tabs.

use crate::services::providers::key::{make_key, normalize_channel};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

/// Emitted with a [`ChatTabsView`] whenever the list or the room on screen changes.
pub const EVENT: &str = "chat-tabs";

/// One open room.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ChatTab {
    /// The room's key: the bare lowercased login on Twitch, `provider:slug`
    /// otherwise.
    pub channel: String,
    pub provider: String,
    /// The name the platform addresses the room by (login or slug).
    pub login: String,
    /// The numeric Twitch id; always `None` on other platforms.
    pub channel_id: Option<String>,
    pub label: String,
    pub avatar: Option<String>,
    /// The tab that follows the watched stream: first, and not closable.
    pub pinned_to_stream: bool,
}

/// A room a shell asks to open or point the stream tab at.
#[derive(Deserialize, Clone, Debug, Default)]
pub struct ChatRoomInput {
    /// "twitch" when absent.
    #[serde(default)]
    pub provider: Option<String>,
    pub channel: String,
    #[serde(default)]
    pub channel_id: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub avatar: Option<String>,
}

/// The tabs as a shell draws them.
#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct ChatTabsView {
    pub tabs: Vec<ChatTab>,
    pub active_channel: Option<String>,
    /// The open rooms span platforms, so non-Twitch tabs wear a provider mark.
    pub mixed_platforms: bool,
    /// Grows with every change, so a shell never paints an older answer over
    /// a newer one (a command's reply and its event can arrive in either order).
    pub revision: u64,
}

/// What a change left on screen, and the connections it asks the shell for.
#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct ChatTabsChange {
    #[serde(flatten)]
    pub view: ChatTabsView,
    /// Rooms to join now.
    pub acquire: Vec<ChatTab>,
    /// Rooms to leave now.
    pub release: Vec<ChatTab>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChatTabs {
    tabs: Vec<ChatTab>,
    active: Option<String>,
    revision: u64,
}

#[derive(Default)]
pub struct Effects {
    pub acquire: Vec<ChatTab>,
    pub release: Vec<ChatTab>,
}

impl ChatTabs {
    pub fn view(&self) -> ChatTabsView {
        let first = self.tabs.first().map(|t| t.provider.as_str());
        ChatTabsView {
            tabs: self.tabs.clone(),
            active_channel: self.active.clone(),
            mixed_platforms: self.tabs.iter().any(|t| Some(t.provider.as_str()) != first),
            revision: self.revision,
        }
    }

    pub fn change(&self, effects: Effects) -> ChatTabsChange {
        ChatTabsChange { view: self.view(), acquire: effects.acquire, release: effects.release }
    }

    /// The tab a room would get, before it is opened.
    pub fn tab_for(room: &ChatRoomInput, pinned_to_stream: bool) -> Option<ChatTab> {
        let provider = room.provider.as_deref().map(str::trim).filter(|p| !p.is_empty()).unwrap_or("twitch");
        let provider = provider.to_lowercase();
        let login = normalize_channel(&provider, room.channel.trim().trim_start_matches('#'));
        if login.is_empty() {
            return None;
        }
        let twitch = provider == "twitch";
        let channel = if twitch { login.clone() } else { make_key(&provider, &login).to_lowercase() };
        let channel_id = if twitch {
            room.channel_id.as_deref().map(str::trim).filter(|id| !id.is_empty()).map(str::to_string)
        } else {
            None
        };
        let label = room
            .label
            .as_deref()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| login.clone());
        let avatar = room.avatar.as_deref().map(str::trim).filter(|a| !a.is_empty()).map(str::to_string);
        Some(ChatTab { channel, provider, login, channel_id, label, avatar, pinned_to_stream })
    }

    /// Points the stream tab at the watched stream; `None` when the player closed.
    ///
    /// Closing the stream drops its tab and keeps every room opened beside it,
    /// so moderating several rooms survives closing the player. A room already
    /// open by hand is promoted to the stream tab rather than opened twice. The
    /// room on screen follows the stream unless the viewer sits on another room.
    pub fn sync_stream(&mut self, room: Option<&ChatRoomInput>) -> Effects {
        let mut fx = Effects::default();
        let current = self.tabs.iter().position(|t| t.pinned_to_stream);
        let Some(next) = room.and_then(|r| Self::tab_for(r, true)) else {
            if let Some(i) = current {
                let old = self.tabs.remove(i);
                if self.active.as_deref() == Some(old.channel.as_str()) {
                    self.active = self.tabs.first().map(|t| t.channel.clone());
                }
                fx.release.push(old);
            }
            return fx;
        };
        if let Some(i) = current.filter(|&i| self.tabs[i].channel == next.channel) {
            // Same room: only what it is called and how it looks can change.
            let tab = &mut self.tabs[i];
            tab.label = next.label;
            if next.avatar.is_some() {
                tab.avatar = next.avatar;
            }
            if next.channel_id.is_some() {
                tab.channel_id = next.channel_id;
            }
            return fx;
        }
        let old = current.map(|i| self.tabs.remove(i));
        let already_open = self.tabs.iter().position(|t| t.channel == next.channel);
        match already_open {
            Some(i) => {
                self.tabs.remove(i);
            }
            None => fx.acquire.push(next.clone()),
        }
        let old_channel = old.as_ref().map(|t| t.channel.clone());
        if let Some(old) = old {
            fx.release.push(old);
        }
        let key = next.channel.clone();
        self.tabs.insert(0, next);
        let sitting_elsewhere = self
            .active
            .as_deref()
            .is_some_and(|a| Some(a) != old_channel.as_deref() && a != key && self.tabs.iter().any(|t| t.channel == a));
        if !sitting_elsewhere {
            self.active = Some(key);
        }
        fx
    }

    /// Opens a room beside the stream and puts it on screen. A room already open
    /// is only put on screen. Answers whether the tab knows only its slug (a
    /// non-Twitch room with no picture, or labelled with its slug), so the
    /// caller can ask the platform for the real name and avatar.
    pub fn add(&mut self, room: &ChatRoomInput) -> (Effects, bool) {
        let mut fx = Effects::default();
        let Some(tab) = Self::tab_for(room, false) else { return (fx, false) };
        if self.tabs.iter().any(|t| t.channel == tab.channel) {
            self.active = Some(tab.channel);
            return (fx, false);
        }
        let needs_meta = tab.provider != "twitch" && (tab.avatar.is_none() || tab.label == tab.login);
        self.active = Some(tab.channel.clone());
        self.tabs.push(tab.clone());
        fx.acquire.push(tab);
        (fx, needs_meta)
    }

    /// Closes a room opened beside the stream. The stream tab stays; closing
    /// the room on screen puts the first room left on screen.
    pub fn remove(&mut self, channel: &str) -> Effects {
        let mut fx = Effects::default();
        let key = channel.trim().to_lowercase();
        let Some(i) = self.tabs.iter().position(|t| t.channel == key && !t.pinned_to_stream) else {
            return fx;
        };
        let tab = self.tabs.remove(i);
        if self.active.as_deref() == Some(key.as_str()) {
            self.active = self.tabs.first().map(|t| t.channel.clone());
        }
        fx.release.push(tab);
        fx
    }

    /// Puts an open room on screen; a room that is not open changes nothing.
    pub fn set_active(&mut self, channel: &str) -> bool {
        let key = channel.trim().to_lowercase();
        if self.active.as_deref() == Some(key.as_str()) || !self.tabs.iter().any(|t| t.channel == key) {
            return false;
        }
        self.active = Some(key);
        true
    }

    /// Fills in a room's real name and picture once the platform answers.
    pub fn patch(&mut self, channel: &str, label: Option<String>, avatar: Option<String>) -> bool {
        let key = channel.trim().to_lowercase();
        let Some(tab) = self.tabs.iter_mut().find(|t| t.channel == key) else { return false };
        let before = tab.clone();
        if let Some(label) = label.filter(|l| !l.trim().is_empty()) {
            tab.label = label;
        }
        if let Some(avatar) = avatar.filter(|a| !a.trim().is_empty()) {
            tab.avatar = Some(avatar);
        }
        *tab != before
    }

    pub fn tab(&self, channel: &str) -> Option<&ChatTab> {
        let key = channel.trim().to_lowercase();
        self.tabs.iter().find(|t| t.channel == key)
    }
}

static TABS: Mutex<ChatTabs> = Mutex::new(ChatTabs { tabs: Vec::new(), active: None, revision: 0 });

/// The session's tabs as they stand.
pub fn view() -> ChatTabsView {
    TABS.lock().unwrap_or_else(|e| e.into_inner()).view()
}

/// Runs a change on the session's tabs and answers what it left, the revision
/// moving on when anything changed.
pub fn apply<X>(f: impl FnOnce(&mut ChatTabs) -> (Effects, X)) -> (ChatTabsChange, X) {
    let mut guard = TABS.lock().unwrap_or_else(|e| e.into_inner());
    let before = (guard.tabs.clone(), guard.active.clone());
    let (fx, out) = f(&mut guard);
    if guard.tabs != before.0 || guard.active != before.1 {
        guard.revision += 1;
    }
    (guard.change(fx), out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn room(provider: &str, channel: &str, id: Option<&str>, label: &str) -> ChatRoomInput {
        ChatRoomInput {
            provider: Some(provider.into()),
            channel: channel.into(),
            channel_id: id.map(str::to_string),
            label: Some(label.into()),
            avatar: None,
        }
    }

    fn keys(t: &ChatTabs) -> Vec<String> {
        t.tabs.iter().map(|t| t.channel.clone()).collect()
    }

    fn names(list: &[ChatTab]) -> Vec<String> {
        list.iter().map(|t| format!("{}:{}", t.provider, t.login)).collect()
    }

    #[test]
    fn a_kick_tab_is_keyed_by_slug_and_never_carries_an_id() {
        let mut t = ChatTabs::default();
        let (fx, _) = t.add(&room("kick", "XQC", Some("676"), "xQc"));
        assert_eq!(t.tabs[0].channel, "kick:xqc");
        assert_eq!(t.tabs[0].login, "xqc");
        assert_eq!(t.tabs[0].channel_id, None);
        assert_eq!(names(&fx.acquire), ["kick:xqc"]);
    }

    #[test]
    fn twitch_tabs_stay_bare_and_keep_their_id() {
        let mut t = ChatTabs::default();
        t.add(&room("twitch", "Foo", Some("9"), "Foo"));
        assert_eq!(t.tabs[0].channel, "foo");
        assert_eq!(t.tabs[0].channel_id.as_deref(), Some("9"));
    }

    #[test]
    fn twitch_and_kick_rooms_of_one_name_are_two_tabs() {
        let mut t = ChatTabs::default();
        t.add(&room("twitch", "xqc", Some("1"), "xqc"));
        t.add(&room("kick", "xqc", None, "xqc"));
        assert_eq!(keys(&t), ["xqc", "kick:xqc"]);
    }

    #[test]
    fn adding_an_open_room_only_switches_to_it() {
        let mut t = ChatTabs::default();
        t.add(&room("twitch", "a", Some("1"), "A"));
        t.add(&room("twitch", "b", Some("2"), "B"));
        let (fx, _) = t.add(&room("twitch", "A", Some("1"), "A"));
        assert!(fx.acquire.is_empty());
        assert_eq!(keys(&t), ["a", "b"]);
        assert_eq!(t.active.as_deref(), Some("a"));
    }

    #[test]
    fn a_slug_only_kick_tab_asks_for_its_name() {
        let mut t = ChatTabs::default();
        assert!(t.add(&room("kick", "joewo", None, "joewo")).1);
        let with_face = ChatRoomInput { avatar: Some("https://x/joe.png".into()), ..room("kick", "jo", None, "Jo") };
        assert!(!t.add(&with_face).1);
        assert!(!t.add(&room("twitch", "t", Some("3"), "t")).1);
        assert!(t.patch("kick:joewo", Some("JoeWo".into()), Some("https://x/joe.png".into())));
        assert_eq!(t.tab("kick:joewo").unwrap().label, "JoeWo");
    }

    #[test]
    fn the_stream_tab_is_first_and_cannot_close() {
        let mut t = ChatTabs::default();
        t.add(&room("twitch", "manual", Some("5"), "Manual"));
        let fx = t.sync_stream(Some(&room("twitch", "xqc", Some("71092938"), "xQc")));
        assert_eq!(names(&fx.acquire), ["twitch:xqc"]);
        assert_eq!(keys(&t), ["xqc", "manual"]);
        assert!(t.remove("xqc").release.is_empty());
        assert_eq!(keys(&t), ["xqc", "manual"]);
    }

    #[test]
    fn closing_the_room_on_screen_falls_back_to_the_first() {
        let mut t = ChatTabs::default();
        t.sync_stream(Some(&room("twitch", "s", Some("1"), "S")));
        t.add(&room("twitch", "b", Some("2"), "B"));
        assert_eq!(t.active.as_deref(), Some("b"));
        let fx = t.remove("B");
        assert_eq!(names(&fx.release), ["twitch:b"]);
        assert_eq!(t.active.as_deref(), Some("s"));
    }

    #[test]
    fn re_pointing_the_stream_releases_the_old_room_across_platforms() {
        let mut t = ChatTabs::default();
        t.sync_stream(Some(&room("kick", "xqc", Some("676"), "xqc")));
        let fx = t.sync_stream(Some(&room("twitch", "xqc", Some("71092938"), "xQc")));
        assert_eq!(names(&fx.release), ["kick:xqc"]);
        assert_eq!(names(&fx.acquire), ["twitch:xqc"]);
        assert_eq!(keys(&t), ["xqc"]);
    }

    #[test]
    fn a_room_open_by_hand_is_promoted_not_duplicated() {
        let mut t = ChatTabs::default();
        t.sync_stream(Some(&room("twitch", "s", Some("1"), "S")));
        t.add(&room("twitch", "b", Some("2"), "B"));
        let fx = t.sync_stream(Some(&room("twitch", "b", Some("2"), "B")));
        assert!(fx.acquire.is_empty(), "already joined");
        assert_eq!(names(&fx.release), ["twitch:s"]);
        assert_eq!(keys(&t), ["b"]);
        assert!(t.tabs[0].pinned_to_stream);
    }

    #[test]
    fn the_room_on_screen_follows_the_stream_unless_the_viewer_sits_elsewhere() {
        let mut t = ChatTabs::default();
        t.sync_stream(Some(&room("twitch", "s1", Some("1"), "S1")));
        t.sync_stream(Some(&room("twitch", "s2", Some("2"), "S2")));
        assert_eq!(t.active.as_deref(), Some("s2"));
        t.add(&room("twitch", "other", Some("3"), "Other"));
        t.sync_stream(Some(&room("twitch", "s3", Some("4"), "S3")));
        assert_eq!(t.active.as_deref(), Some("other"));
    }

    #[test]
    fn closing_the_stream_keeps_the_rooms_opened_beside_it() {
        let mut t = ChatTabs::default();
        t.sync_stream(Some(&room("twitch", "s", Some("1"), "S")));
        t.add(&room("twitch", "m", Some("2"), "M"));
        t.set_active("s");
        let fx = t.sync_stream(None);
        assert_eq!(names(&fx.release), ["twitch:s"]);
        assert_eq!(keys(&t), ["m"]);
        assert_eq!(t.active.as_deref(), Some("m"));
        assert!(t.sync_stream(None).release.is_empty());
    }

    #[test]
    fn the_same_stream_again_only_refreshes_its_name_and_face() {
        let mut t = ChatTabs::default();
        t.sync_stream(Some(&room("twitch", "s", None, "s")));
        let again = ChatRoomInput { avatar: Some("a.png".into()), ..room("twitch", "S", Some("1"), "Streamer") };
        let fx = t.sync_stream(Some(&again));
        assert!(fx.acquire.is_empty() && fx.release.is_empty());
        let tab = &t.tabs[0];
        assert_eq!((tab.label.as_str(), tab.avatar.as_deref(), tab.channel_id.as_deref()), ("Streamer", Some("a.png"), Some("1")));
    }

    #[test]
    fn marks_show_only_when_platforms_mix() {
        let mut t = ChatTabs::default();
        t.add(&room("twitch", "a", Some("1"), "A"));
        assert!(!t.view().mixed_platforms);
        t.add(&room("kick", "a", None, "A"));
        assert!(t.view().mixed_platforms);
    }

    #[test]
    fn switching_to_a_room_that_is_not_open_changes_nothing() {
        let mut t = ChatTabs::default();
        t.add(&room("twitch", "a", Some("1"), "A"));
        assert!(!t.set_active("nope"));
        assert_eq!(t.active.as_deref(), Some("a"));
    }

    #[test]
    fn the_change_carries_the_view_flat() {
        let mut t = ChatTabs::default();
        let (fx, _) = t.add(&room("twitch", "a", Some("1"), "A"));
        let json = serde_json::to_value(t.change(fx)).unwrap();
        assert_eq!(json["active_channel"], "a");
        assert_eq!(json["tabs"][0]["channel"], "a");
        assert_eq!(json["acquire"][0]["login"], "a");
    }
}
