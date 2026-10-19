//! The phone chats: the tab that follows the watched stream, the rooms you
//! opened yourself, and which one is on screen. The same tabs show under the
//! player and on the Chats screen, where you read chat with no stream at all.
//!
//! Both phone shells (the Android page and the iPhone app) open several rooms
//! at once and switch between them without reconnecting. The rules for that
//! list live here so the two shells cannot drift: the stream tab is first and
//! cannot be closed, a room already open is never opened twice, closing the
//! room on screen falls back to the first one left, and the stream tab follows
//! the player unless the viewer is sitting on another room.
//!
//! A room you opened yourself is "kept": it is one of your chats, saved per
//! account and restored at launch, and it stays when the player closes even if
//! you watched it in between. The stream's own chat closes with the player
//! unless you keep it. Restored chats connect only when you first open the
//! Chats screen or start a stream, so a launch that never looks at them joins
//! nothing; Android parks them (leaves every room) after a while in the
//! background with no player, and connects them again on return.
//!
//! Connections stay with each shell. Every change answers which rooms the
//! shell must now join (`acquire`) and leave (`release`); the Android page
//! hands those to its reference-counted chat store, the iPhone app to the core's
//! join and leave commands.
//!
//! Twitch rooms are keyed by the bare lowercased login and carry the numeric
//! Twitch id; every other platform is keyed `provider:slug` and carries no id,
//! because a Kick id is numeric too and, handed to Helix, names an unrelated
//! Twitch account. Twitch `xqc` and Kick `xqc` are two tabs.

use crate::services::providers::key::{make_key, normalize_channel};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

/// Emitted with a [`ChatTabsView`] whenever the list or the room on screen changes.
pub const EVENT: &str = "chat-tabs";

/// How long Android waits in the background with no player before it leaves
/// every room. The iPhone needs none: the system pauses it.
pub const PARK_AFTER_SECS: u64 = 120;

/// The Chats screen keeps the phone awake until this long passes with no touch.
pub const KEEP_AWAKE_IDLE_SECS: u64 = 30 * 60;

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
    /// One of your chats: opened by hand, or the stream's chat you chose to
    /// keep. Saved, and kept when the player closes.
    #[serde(default)]
    pub kept: bool,
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

/// Whether the rooms are joined.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Connection {
    /// Restored at launch and not joined yet.
    #[default]
    Idle,
    /// Every tab is joined.
    Connected,
    /// Left after a while in the background; joined again on return.
    Parked,
}

/// What a room's live line says, from `channel_state` (Twitch) or the live
/// poller (other platforms).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
struct LiveLine {
    live: bool,
    viewer_count: Option<u64>,
}

/// A tab as a shell draws it.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ChatTabView {
    #[serde(flatten)]
    pub tab: ChatTab,
    /// Live now; `None` until the room's live line answers.
    pub live: Option<bool>,
    pub viewer_count: Option<u64>,
    /// Mentions of you in this room not read yet.
    pub mentions: u32,
}

/// The tabs as a shell draws them.
#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct ChatTabsView {
    pub tabs: Vec<ChatTabView>,
    pub active_channel: Option<String>,
    /// The open rooms span platforms, so non-Twitch tabs wear a provider mark.
    pub mixed_platforms: bool,
    pub connection: Connection,
    /// Unread mentions across every tab: the Chats tab's badge.
    pub mention_total: u32,
    pub park_after_secs: u64,
    pub keep_awake_idle_secs: u64,
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

/// One account's chats as saved: the kept tabs and the one on screen.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct SavedChats {
    pub tabs: Vec<ChatTab>,
    pub active: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChatTabs {
    tabs: Vec<ChatTab>,
    active: Option<String>,
    connection: Connection,
    live: HashMap<String, LiveLine>,
    mentions: HashMap<String, u32>,
    /// Whose chats these are: the signed-in Twitch id, or "anon".
    owner: Option<String>,
    revision: u64,
}

#[derive(Default)]
pub struct Effects {
    pub acquire: Vec<ChatTab>,
    pub release: Vec<ChatTab>,
}

impl Effects {
    fn and(mut self, more: Effects) -> Effects {
        self.acquire.extend(more.acquire);
        self.release.extend(more.release);
        self
    }
}

impl ChatTabs {
    pub fn view(&self) -> ChatTabsView {
        let first = self.tabs.first().map(|t| t.provider.as_str());
        let tabs: Vec<ChatTabView> = self
            .tabs
            .iter()
            .map(|t| {
                let line = self.live.get(&t.channel);
                ChatTabView {
                    tab: t.clone(),
                    live: line.map(|l| l.live),
                    viewer_count: line.and_then(|l| l.viewer_count),
                    mentions: self.mentions.get(&t.channel).copied().unwrap_or(0),
                }
            })
            .collect();
        ChatTabsView {
            mention_total: tabs.iter().map(|t| t.mentions).sum(),
            tabs,
            active_channel: self.active.clone(),
            mixed_platforms: self.tabs.iter().any(|t| Some(t.provider.as_str()) != first),
            connection: self.connection,
            park_after_secs: PARK_AFTER_SECS,
            keep_awake_idle_secs: KEEP_AWAKE_IDLE_SECS,
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
        Some(ChatTab { channel, provider, login, channel_id, label, avatar, pinned_to_stream, kept: !pinned_to_stream })
    }

    /// Joins every tab not joined yet: the first look at the Chats screen, a
    /// stream starting, a chat opened, or the return from a park.
    pub fn connect(&mut self) -> Effects {
        let mut fx = Effects::default();
        if self.connection != Connection::Connected {
            self.connection = Connection::Connected;
            fx.acquire = self.tabs.clone();
        }
        fx
    }

    /// Leaves every room after a while in the background, unless a stream is
    /// on (the player keeps playing, and its chat with it).
    pub fn park(&mut self) -> Effects {
        let mut fx = Effects::default();
        if self.connection != Connection::Connected || self.tabs.iter().any(|t| t.pinned_to_stream) {
            return fx;
        }
        self.connection = Connection::Parked;
        fx.release = self.tabs.clone();
        fx
    }

    /// Takes over another account's chats (or the first ones of the session):
    /// anything joined is left, and the new list waits for its first look.
    pub fn restore(&mut self, owner: String, saved: SavedChats) -> Effects {
        let mut fx = Effects::default();
        if self.connection == Connection::Connected {
            fx.release = std::mem::take(&mut self.tabs);
        }
        let mut tabs: Vec<ChatTab> = Vec::new();
        for mut tab in saved.tabs {
            if tab.channel.is_empty() || tabs.iter().any(|t| t.channel == tab.channel) {
                continue;
            }
            tab.pinned_to_stream = false;
            tab.kept = true;
            tabs.push(tab);
        }
        self.active = saved
            .active
            .filter(|a| tabs.iter().any(|t| &t.channel == a))
            .or_else(|| tabs.first().map(|t| t.channel.clone()));
        self.tabs = tabs;
        self.connection = Connection::Idle;
        self.live.clear();
        self.mentions.clear();
        self.owner = Some(owner);
        fx
    }

    /// What is saved: your chats, with the stream's own chat left out unless
    /// you kept it.
    pub fn saved(&self) -> SavedChats {
        let tabs: Vec<ChatTab> = self
            .tabs
            .iter()
            .filter(|t| t.kept)
            .map(|t| ChatTab { pinned_to_stream: false, ..t.clone() })
            .collect();
        let active = self.active.clone().filter(|a| tabs.iter().any(|t| &t.channel == a));
        SavedChats { tabs, active }
    }

    /// The stream tab steps down when the player closes or moves to another
    /// stream: a kept one stays as one of your chats, any other one is left.
    fn retire_stream_tab(&mut self, fx: &mut Effects) -> Option<String> {
        let i = self.tabs.iter().position(|t| t.pinned_to_stream)?;
        if self.tabs[i].kept {
            self.tabs[i].pinned_to_stream = false;
            return Some(self.tabs[i].channel.clone());
        }
        let old = self.tabs.remove(i);
        let key = old.channel.clone();
        fx.release.push(old);
        Some(key)
    }

    /// Points the stream tab at the watched stream; `None` when the player closed.
    ///
    /// Closing the stream drops its tab unless it is kept, and keeps every
    /// room opened beside it. A room already open by hand is promoted to the
    /// stream tab rather than opened twice, and stays kept. The room on screen
    /// follows the stream unless the viewer sits on another room.
    pub fn sync_stream(&mut self, room: Option<&ChatRoomInput>) -> Effects {
        let mut fx = Effects::default();
        let current = self.tabs.iter().position(|t| t.pinned_to_stream);
        let Some(next) = room.and_then(|r| Self::tab_for(r, true)) else {
            if let Some(gone) = self.retire_stream_tab(&mut fx) {
                if self.active.as_deref() == Some(gone.as_str()) && self.tab(&gone).is_none() {
                    self.active = self.tabs.first().map(|t| t.channel.clone());
                }
            }
            return fx;
        };
        if let Some(i) = current.filter(|&i| self.tabs[i].channel == next.channel) {
            // Same room: only what it is called and how it looks can change.
            let tab = &mut self.tabs[i];
            tab.label = kept_label(next.label, &tab.label);
            if next.avatar.is_some() {
                tab.avatar = next.avatar;
            }
            if next.channel_id.is_some() {
                tab.channel_id = next.channel_id;
            }
            return fx;
        }
        // A stream starting joins your restored chats too.
        fx = fx.and(self.connect());
        let old_channel = self.retire_stream_tab(&mut fx);
        let mut next = next;
        match self.tabs.iter().position(|t| t.channel == next.channel) {
            Some(i) => {
                let open = self.tabs.remove(i);
                next.kept = open.kept;
                next.label = kept_label(next.label, &open.label);
                if next.avatar.is_none() {
                    next.avatar = open.avatar;
                }
                if next.channel_id.is_none() {
                    next.channel_id = open.channel_id;
                }
                // Joined already, unless `connect` just asked for it.
            }
            None => fx.acquire.push(next.clone()),
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

    /// Opens a room as one of your chats and puts it on screen, joining your
    /// restored chats along with it. A room already open is only put on screen
    /// (and kept, when it is the stream's). Answers whether the tab knows only
    /// its slug (a non-Twitch room with no picture, or labelled with its slug),
    /// so the caller can ask the platform for the real name and avatar.
    pub fn add(&mut self, room: &ChatRoomInput) -> (Effects, bool) {
        let Some(tab) = Self::tab_for(room, false) else { return (Effects::default(), false) };
        let mut fx = self.connect();
        if let Some(open) = self.tabs.iter_mut().find(|t| t.channel == tab.channel) {
            open.kept = true;
            self.active = Some(tab.channel);
            return (fx, false);
        }
        let needs_meta = tab.provider != "twitch" && (tab.avatar.is_none() || tab.label == tab.login);
        self.active = Some(tab.channel.clone());
        self.tabs.push(tab.clone());
        fx.acquire.push(tab);
        (fx, needs_meta)
    }

    /// Keeps the stream's chat after the player closes, or lets it close with
    /// the player again. Only the stream tab takes this: every other tab is
    /// kept by being open.
    pub fn keep(&mut self, channel: &str, keep: bool) -> bool {
        let key = channel.trim().to_lowercase();
        match self.tabs.iter_mut().find(|t| t.channel == key && t.pinned_to_stream) {
            Some(tab) if tab.kept != keep => {
                tab.kept = keep;
                true
            }
            _ => false,
        }
    }

    /// Closes one of your chats. The stream tab stays (closing it there only
    /// stops keeping it); closing the room on screen puts the first room left
    /// on screen.
    pub fn remove(&mut self, channel: &str) -> Effects {
        let mut fx = Effects::default();
        let key = channel.trim().to_lowercase();
        let Some(i) = self.tabs.iter().position(|t| t.channel == key) else {
            return fx;
        };
        if self.tabs[i].pinned_to_stream {
            self.tabs[i].kept = false;
            return fx;
        }
        let tab = self.tabs.remove(i);
        self.live.remove(&key);
        self.mentions.remove(&key);
        if self.active.as_deref() == Some(key.as_str()) {
            self.active = self.tabs.first().map(|t| t.channel.clone());
        }
        // A room never joined (restored, not looked at yet) has nothing to leave.
        if self.connection == Connection::Connected {
            fx.release.push(tab);
        }
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

    /// A room's live line moved. Answers whether an open tab shows it.
    pub fn note_live(&mut self, channel: &str, live: bool, viewer_count: Option<u64>) -> bool {
        let key = channel.trim().to_lowercase();
        if !self.tabs.iter().any(|t| t.channel == key) {
            return false;
        }
        let line = LiveLine { live, viewer_count: if live { viewer_count } else { None } };
        self.live.insert(key, line) != Some(line)
    }

    /// Unread mentions in a room, as the mentions hub counts them. Answers
    /// whether an open tab shows it.
    pub fn note_mentions(&mut self, channel: &str, unread: u32) -> bool {
        let key = channel.trim().to_lowercase();
        if !self.tabs.iter().any(|t| t.channel == key) {
            return false;
        }
        let before = self.mentions.get(&key).copied().unwrap_or(0);
        if unread == 0 {
            self.mentions.remove(&key);
        } else {
            self.mentions.insert(key, unread);
        }
        before != unread
    }

    pub fn tab(&self, channel: &str) -> Option<&ChatTab> {
        let key = channel.trim().to_lowercase();
        self.tabs.iter().find(|t| t.channel == key)
    }

    pub fn tabs(&self) -> &[ChatTab] {
        &self.tabs
    }

    pub fn connection(&self) -> Connection {
        self.connection
    }

    /// What a shell sees, minus the revision: a change moves the revision on.
    fn shown(&self) -> ChatTabsView {
        ChatTabsView { revision: 0, ..self.view() }
    }
}

static TABS: Mutex<Option<ChatTabs>> = Mutex::new(None);

/// The file every account's chats are saved in, keyed by owner.
const FILE: &str = "chat_tabs.json";

fn owner_now() -> String {
    crate::services::vod_progress_service::owner_id().unwrap_or_else(|| "anon".to_string())
}

fn load_all() -> HashMap<String, SavedChats> {
    let Ok(dir) = crate::services::cache_service::get_app_data_dir() else { return HashMap::new() };
    std::fs::read_to_string(dir.join(FILE))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save(owner: &str, chats: &SavedChats) {
    let Ok(dir) = crate::services::cache_service::get_app_data_dir() else { return };
    let mut all = load_all();
    if all.get(owner) == Some(chats) {
        return;
    }
    if chats.tabs.is_empty() {
        all.remove(owner);
    } else {
        all.insert(owner.to_string(), chats.clone());
    }
    let path = dir.join(FILE);
    let tmp = path.with_extension("json.tmp");
    let written = serde_json::to_string(&all)
        .map_err(|e| e.to_string())
        .and_then(|json| std::fs::write(&tmp, json).map_err(|e| e.to_string()))
        .and_then(|()| std::fs::rename(&tmp, &path).map_err(|e| e.to_string()));
    if let Err(e) = written {
        log::warn!("[ChatTabs] save failed: {e}");
    }
}

/// The joined chats, for the live poller; nothing while they wait or are parked.
pub fn joined() -> Vec<ChatTab> {
    let guard = TABS.lock().unwrap_or_else(|e| e.into_inner());
    match guard.as_ref() {
        Some(t) if t.connection == Connection::Connected => t.tabs.clone(),
        _ => Vec::new(),
    }
}

/// The session's tabs as they stand.
pub fn view() -> ChatTabsView {
    apply(|_| (Effects::default(), ())).0.view
}

/// Runs a change on the session's tabs and answers what it left, the revision
/// moving on when anything changed. The signed-in account's saved chats are
/// restored first (at the first call, and after an account switch), and the
/// chats are saved again whenever what is saved changed.
pub fn apply<X>(f: impl FnOnce(&mut ChatTabs) -> (Effects, X)) -> (ChatTabsChange, X) {
    let owner = owner_now();
    let mut guard = TABS.lock().unwrap_or_else(|e| e.into_inner());
    let tabs = guard.get_or_insert_with(ChatTabs::default);
    let mut first = Effects::default();
    if tabs.owner.as_deref() != Some(owner.as_str()) {
        let saved = load_all().remove(&owner).unwrap_or_default();
        first = tabs.restore(owner.clone(), saved);
        tabs.revision += 1;
    }
    let before = (tabs.shown(), tabs.saved());
    let (fx, out) = f(tabs);
    let fx = first.and(fx);
    let saved = tabs.saved();
    if tabs.shown() != before.0 {
        tabs.revision += 1;
    }
    if saved != before.1 {
        save(&owner, &saved);
    }
    (tabs.change(fx), out)
}

/// A name that is only the login's lowercase ("xqc", what a start from a login alone carries)
/// never replaces the display name the room already has ("xQc"); any other name does.
fn kept_label(next: String, current: &str) -> String {
    if next != current && next.eq_ignore_ascii_case(current) && next == next.to_lowercase() {
        current.to_string()
    } else {
        next
    }
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
    fn a_login_only_start_keeps_the_rooms_display_name() {
        let mut t = ChatTabs::default();
        t.add(&room("twitch", "xqc", Some("1"), "xQc"));
        t.sync_stream(Some(&room("twitch", "xqc", Some("1"), "xqc")));
        assert_eq!(t.tabs[0].label, "xQc", "the open room's name survives the stream claiming it");
        t.sync_stream(Some(&room("twitch", "xqc", Some("1"), "xqc")));
        assert_eq!(t.tabs[0].label, "xQc", "and the same stream again");
        t.sync_stream(Some(&room("twitch", "xqc", Some("1"), "XQC")));
        assert_eq!(t.tabs[0].label, "XQC", "a real new name still lands");
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

    #[test]
    fn a_room_opened_by_hand_and_then_watched_stays_when_the_player_closes() {
        let mut t = ChatTabs::default();
        t.add(&room("twitch", "lunar", Some("1"), "Lunar"));
        t.sync_stream(Some(&room("twitch", "lunar", Some("1"), "Lunar")));
        assert!(t.tabs[0].pinned_to_stream && t.tabs[0].kept);
        let fx = t.sync_stream(None);
        assert!(fx.release.is_empty(), "still one of your chats");
        assert_eq!(keys(&t), ["lunar"]);
        assert!(!t.tabs[0].pinned_to_stream);
        assert_eq!(t.active.as_deref(), Some("lunar"));
    }

    #[test]
    fn the_streams_chat_closes_with_the_player_unless_kept() {
        let mut t = ChatTabs::default();
        t.add(&room("twitch", "mine", Some("1"), "Mine"));
        t.sync_stream(Some(&room("twitch", "s", Some("2"), "S")));
        assert!(!t.tabs[0].kept);
        assert!(t.keep("s", true));
        let fx = t.sync_stream(Some(&room("twitch", "next", Some("3"), "Next")));
        assert!(fx.release.is_empty(), "kept, so it steps down instead of closing");
        assert_eq!(keys(&t), ["next", "s", "mine"]);
        assert!(t.remove("next").release.is_empty(), "closing the stream tab only stops keeping it");
        assert_eq!(names(&t.sync_stream(None).release), ["twitch:next"]);
        assert_eq!(keys(&t), ["s", "mine"]);
    }

    #[test]
    fn only_the_stream_tab_takes_keep() {
        let mut t = ChatTabs::default();
        t.add(&room("twitch", "a", Some("1"), "A"));
        assert!(!t.keep("a", false));
        assert!(t.tabs[0].kept);
    }

    #[test]
    fn restored_chats_wait_for_their_first_look() {
        let mut t = ChatTabs::default();
        let saved = SavedChats {
            tabs: vec![
                ChatTabs::tab_for(&room("twitch", "a", Some("1"), "A"), false).unwrap(),
                ChatTabs::tab_for(&room("kick", "b", None, "B"), false).unwrap(),
            ],
            active: Some("kick:b".into()),
        };
        assert!(t.restore("42".into(), saved).release.is_empty());
        assert_eq!(t.connection, Connection::Idle);
        assert_eq!(t.active.as_deref(), Some("kick:b"));
        assert!(t.remove("a").release.is_empty(), "never joined, nothing to leave");
        assert_eq!(names(&t.connect().acquire), ["kick:b"]);
        assert!(t.connect().acquire.is_empty(), "joined once");
    }

    #[test]
    fn a_stream_or_a_new_chat_joins_the_restored_ones() {
        let saved = SavedChats { tabs: vec![ChatTabs::tab_for(&room("twitch", "a", Some("1"), "A"), false).unwrap()], active: None };
        let mut t = ChatTabs::default();
        t.restore("42".into(), saved.clone());
        assert_eq!(names(&t.sync_stream(Some(&room("twitch", "s", Some("2"), "S"))).acquire), ["twitch:a", "twitch:s"]);
        let mut t = ChatTabs::default();
        t.restore("42".into(), saved.clone());
        assert_eq!(names(&t.add(&room("twitch", "b", Some("3"), "B")).0.acquire), ["twitch:a", "twitch:b"]);
        let mut t = ChatTabs::default();
        t.restore("42".into(), saved);
        let fx = t.sync_stream(Some(&room("twitch", "a", Some("1"), "A")));
        assert_eq!(names(&fx.acquire), ["twitch:a"], "a restored chat watched is joined once");
        assert!(t.tabs[0].kept);
    }

    #[test]
    fn parking_leaves_everything_but_never_under_a_stream() {
        let mut t = ChatTabs::default();
        t.add(&room("twitch", "a", Some("1"), "A"));
        t.sync_stream(Some(&room("twitch", "s", Some("2"), "S")));
        assert!(t.park().release.is_empty());
        t.sync_stream(None);
        assert_eq!(names(&t.park().release), ["twitch:a"]);
        assert_eq!(t.connection, Connection::Parked);
        assert!(t.park().release.is_empty());
        assert_eq!(names(&t.connect().acquire), ["twitch:a"]);
    }

    #[test]
    fn only_your_chats_are_saved() {
        let mut t = ChatTabs::default();
        t.add(&room("twitch", "a", Some("1"), "A"));
        t.sync_stream(Some(&room("twitch", "s", Some("2"), "S")));
        t.set_active("s");
        let saved = t.saved();
        assert_eq!(saved.tabs.iter().map(|t| t.channel.as_str()).collect::<Vec<_>>(), ["a"]);
        assert_eq!(saved.active, None, "the stream tab on screen is not saved");
        t.keep("s", true);
        let saved = t.saved();
        assert_eq!(saved.tabs.iter().map(|t| t.channel.as_str()).collect::<Vec<_>>(), ["s", "a"]);
        assert!(saved.tabs.iter().all(|t| !t.pinned_to_stream && t.kept));
    }

    #[test]
    fn switching_accounts_leaves_the_old_chats() {
        let mut t = ChatTabs::default();
        t.add(&room("twitch", "a", Some("1"), "A"));
        let fx = t.restore("other".into(), SavedChats::default());
        assert_eq!(names(&fx.release), ["twitch:a"]);
        assert!(t.tabs.is_empty() && t.active.is_none());
    }

    #[test]
    fn live_lines_and_mentions_show_on_open_tabs_only() {
        let mut t = ChatTabs::default();
        t.add(&room("twitch", "a", Some("1"), "A"));
        assert!(!t.note_live("gone", true, Some(5)));
        assert!(t.note_live("a", true, Some(120)));
        assert!(!t.note_live("a", true, Some(120)));
        assert!(t.note_mentions("A", 2));
        let v = t.view();
        assert_eq!((v.tabs[0].live, v.tabs[0].viewer_count, v.tabs[0].mentions, v.mention_total), (Some(true), Some(120), 2, 2));
        t.note_live("a", false, Some(3));
        assert_eq!(t.view().tabs[0].viewer_count, None);
        t.remove("a");
        t.add(&room("twitch", "a", Some("1"), "A"));
        assert_eq!((t.view().tabs[0].live, t.view().mention_total), (None, 0), "a closed tab forgets both");
    }
}
