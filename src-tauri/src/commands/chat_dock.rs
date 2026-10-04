//! The chats docked in the main window.
//!
//! A docked chat stays open beside whatever stream is playing: the viewer
//! watches one stream and keeps any number of other channels' chats a click
//! away, without MultiNook. The main chat shows either the watched stream's
//! chat (`active: None`) or one docked chat. This replaced the single pinned
//! chat; a saved pin becomes the first docked chat the first time this loads.
//!
//! The dock lives here, not in the page, because Go Live and close-to-tray
//! destroy the main window and rebuild it, and the dock has to come back with
//! it. It is saved, so it survives a restart. The page mirrors it from
//! `get_chat_dock` and the `chat-dock` event.
//!
//! Not the same thing as combined chat (a streamer's own chats on other
//! platforms, `chat_blend`), which is untouched by docking.

use crate::rt::AppHandle;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::Emitter;

pub const EVENT: &str = "chat-dock";

const PROVIDERS: [&str; 4] = ["twitch", "kick", "youtube", "tiktok"];

fn yes() -> bool {
    true
}

/// One docked chat, as the page knew the channel when docking it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct DockedChat {
    pub provider: String,
    pub login: String,
    #[serde(default)]
    pub channel_id: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub avatar_url: Option<String>,
    /// Mark the chat when new messages arrive while it is hidden. Off keeps
    /// only mentions; for a busy chat that is always moving.
    #[serde(default = "yes")]
    pub light_on_new: bool,
}

impl DockedChat {
    /// The runtime key (`makeKey`): `provider:login`.
    pub fn key(&self) -> String {
        format!("{}:{}", self.provider, self.login)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct ChatDockState {
    /// In the viewer's order.
    #[serde(default)]
    pub chats: Vec<DockedChat>,
    /// The docked chat on screen, by key; `None` shows the watched stream's.
    #[serde(default)]
    pub active: Option<String>,
}

static STATE: Lazy<Mutex<ChatDockState>> = Lazy::new(|| Mutex::new(load()));

/// The dock and the chat on screen, for a window that is starting up.
#[tauri::command]
pub fn get_chat_dock() -> ChatDockState {
    STATE.lock().map(|s| s.clone()).unwrap_or_default()
}

/// Dock a channel's chat. Already docked keeps its place. `show` puts it on
/// screen.
#[tauri::command]
pub fn dock_chat(app: AppHandle, chat: DockedChat, show: bool) -> Result<ChatDockState, String> {
    let chat = clean(chat)?;
    update(&app, |s| apply_dock(s, chat, show))
}

/// Close a docked chat. If it was on screen, the watched stream's chat shows.
#[tauri::command]
pub fn undock_chat(app: AppHandle, key: String) -> Result<ChatDockState, String> {
    update(&app, |s| apply_undock(s, &key))
}

/// Show a docked chat, or the watched stream's with `None`. A key that is not
/// docked is ignored.
#[tauri::command]
pub fn show_docked_chat(app: AppHandle, key: Option<String>) -> Result<ChatDockState, String> {
    update(&app, |s| apply_show(s, key))
}

/// Put the docked chats in this order. Keys not docked are ignored; docked
/// chats the list leaves out keep their relative order after it.
#[tauri::command]
pub fn reorder_chat_dock(app: AppHandle, keys: Vec<String>) -> Result<ChatDockState, String> {
    update(&app, |s| apply_reorder(s, &keys))
}

/// Mark a docked chat on new messages, or on mentions only.
#[tauri::command]
pub fn set_docked_chat_light(app: AppHandle, key: String, on: bool) -> Result<ChatDockState, String> {
    update(&app, |s| {
        if let Some(c) = s.chats.iter_mut().find(|c| c.key() == key) {
            c.light_on_new = on;
        }
    })
}

fn apply_dock(s: &mut ChatDockState, chat: DockedChat, show: bool) {
    let key = chat.key();
    match s.chats.iter_mut().find(|c| c.key() == key) {
        // Fresher name and picture, same place and same mark setting.
        Some(existing) => {
            existing.channel_id = if chat.channel_id.is_empty() { existing.channel_id.clone() } else { chat.channel_id };
            existing.display_name = chat.display_name;
            if chat.avatar_url.is_some() {
                existing.avatar_url = chat.avatar_url;
            }
        }
        None => s.chats.push(chat),
    }
    if show {
        s.active = Some(key);
    }
}

fn apply_undock(s: &mut ChatDockState, key: &str) {
    s.chats.retain(|c| c.key() != key);
    if s.active.as_deref() == Some(key) {
        s.active = None;
    }
}

fn apply_show(s: &mut ChatDockState, key: Option<String>) {
    match key {
        Some(k) if s.chats.iter().any(|c| c.key() == k) => s.active = Some(k),
        Some(_) => {}
        None => s.active = None,
    }
}

fn apply_reorder(s: &mut ChatDockState, keys: &[String]) {
    let mut rest = std::mem::take(&mut s.chats);
    for k in keys {
        if let Some(i) = rest.iter().position(|c| &c.key() == k) {
            s.chats.push(rest.remove(i));
        }
    }
    s.chats.extend(rest);
}

fn update(app: &AppHandle, change: impl FnOnce(&mut ChatDockState)) -> Result<ChatDockState, String> {
    let next = {
        let mut state = STATE.lock().map_err(|e| e.to_string())?;
        let before = state.clone();
        change(&mut state);
        if *state == before {
            return Ok(before);
        }
        state.clone()
    };
    if let Err(e) = save(&next) {
        log::warn!("[ChatDock] could not save the dock: {e}");
    }
    let _ = app.emit_to("main", EVENT, &next);
    // A newly docked Kick / YouTube / TikTok chat gets its live line now.
    crate::services::chat_dock_live::wake();
    Ok(next)
}

/// A chat the page sent, checked and normalised the way `makeKey` normalises:
/// lowercase, except YouTube, whose channel ids are case-sensitive.
fn clean(mut chat: DockedChat) -> Result<DockedChat, String> {
    chat.provider = chat.provider.trim().to_ascii_lowercase();
    if !PROVIDERS.contains(&chat.provider.as_str()) {
        return Err(format!("unknown platform: {}", chat.provider));
    }
    let login = chat.login.trim().trim_start_matches('@');
    chat.login = if chat.provider == "youtube" { login.to_string() } else { login.to_lowercase() };
    if chat.login.is_empty() || chat.login.len() > 128 {
        return Err("a docked chat needs a channel".into());
    }
    if chat.display_name.trim().is_empty() {
        chat.display_name = chat.login.clone();
    }
    Ok(chat)
}

fn data_dir() -> Option<PathBuf> {
    crate::services::cache_service::get_app_data_dir().ok()
}

fn load() -> ChatDockState {
    let Some(dir) = data_dir() else { return ChatDockState::default() };
    if let Some(state) = std::fs::read_to_string(dir.join("chat_dock.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<ChatDockState>(&s).ok())
    {
        return sanitize(state);
    }
    // First run after the dock replaced the pin: carry the pin over once.
    let migrated = std::fs::read_to_string(dir.join("chat_pin.json"))
        .ok()
        .and_then(|s| from_pin_file(&s))
        .unwrap_or_default();
    if !migrated.chats.is_empty() {
        if let Err(e) = save(&migrated) {
            log::warn!("[ChatDock] could not save the migrated pin: {e}");
        }
    }
    migrated
}

/// The old `chat_pin.json` as a dock: the pin becomes the one docked chat, on
/// screen if the pinned side was.
fn from_pin_file(json: &str) -> Option<ChatDockState> {
    #[derive(Deserialize)]
    struct OldPin {
        provider: String,
        login: String,
        #[serde(default)]
        channel_id: String,
        #[serde(default)]
        display_name: String,
        #[serde(default)]
        avatar_url: Option<String>,
    }
    #[derive(Deserialize)]
    struct OldState {
        pin: Option<OldPin>,
        #[serde(default)]
        view: Option<String>,
    }
    let old: OldState = serde_json::from_str(json).ok()?;
    let pin = old.pin?;
    let chat = clean(DockedChat {
        provider: pin.provider,
        login: pin.login,
        channel_id: pin.channel_id,
        display_name: pin.display_name,
        avatar_url: pin.avatar_url,
        light_on_new: true,
    })
    .ok()?;
    let active = (old.view.as_deref() == Some("pinned")).then(|| chat.key());
    Some(ChatDockState { chats: vec![chat], active })
}

/// A saved file is trusted only as far as `clean` would trust the page:
/// unknown platforms and duplicates go, and an active key must be docked.
fn sanitize(state: ChatDockState) -> ChatDockState {
    let mut out = ChatDockState::default();
    for chat in state.chats.into_iter().filter_map(|c| clean(c).ok()) {
        if !out.chats.iter().any(|c| c.key() == chat.key()) {
            out.chats.push(chat);
        }
    }
    out.active = state.active.filter(|k| out.chats.iter().any(|c| &c.key() == k));
    out
}

fn save(state: &ChatDockState) -> Result<(), String> {
    let path = data_dir().ok_or("no app data dir")?.join("chat_dock.json");
    let json = serde_json::to_string(state).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)
        .and_then(|_| std::fs::rename(&tmp, &path))
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chat(provider: &str, login: &str) -> DockedChat {
        DockedChat {
            provider: provider.into(),
            login: login.into(),
            channel_id: "1".into(),
            display_name: String::new(),
            avatar_url: None,
            light_on_new: true,
        }
    }

    #[test]
    fn a_chat_is_normalised_like_make_key() {
        let c = clean(chat(" Twitch ", "@Rainyyay ")).unwrap();
        assert_eq!(c.key(), "twitch:rainyyay");
        assert_eq!(c.display_name, "rainyyay");
        let yt = clean(chat("youtube", "UCabcDEF")).unwrap();
        assert_eq!(yt.key(), "youtube:UCabcDEF", "YouTube ids keep their case");
        assert!(clean(chat("myspace", "tom")).is_err());
        assert!(clean(chat("kick", "  ")).is_err());
    }

    #[test]
    fn docking_twice_keeps_one_entry_and_its_place() {
        let mut s = ChatDockState::default();
        apply_dock(&mut s, clean(chat("twitch", "a")).unwrap(), false);
        apply_dock(&mut s, clean(chat("twitch", "b")).unwrap(), false);
        let mut again = clean(chat("twitch", "a")).unwrap();
        again.display_name = "A".into();
        apply_dock(&mut s, again, true);
        assert_eq!(s.chats.len(), 2);
        assert_eq!(s.chats[0].display_name, "A");
        assert_eq!(s.active.as_deref(), Some("twitch:a"));
    }

    #[test]
    fn undocking_the_shown_chat_goes_back_to_the_stream() {
        let mut s = ChatDockState::default();
        apply_dock(&mut s, clean(chat("kick", "x")).unwrap(), true);
        apply_undock(&mut s, "kick:x");
        assert!(s.chats.is_empty());
        assert_eq!(s.active, None);
    }

    #[test]
    fn showing_needs_a_docked_chat() {
        let mut s = ChatDockState::default();
        apply_dock(&mut s, clean(chat("twitch", "a")).unwrap(), false);
        apply_show(&mut s, Some("twitch:nope".into()));
        assert_eq!(s.active, None);
        apply_show(&mut s, Some("twitch:a".into()));
        assert_eq!(s.active.as_deref(), Some("twitch:a"));
        apply_show(&mut s, None);
        assert_eq!(s.active, None);
    }

    #[test]
    fn reorder_follows_the_list_and_keeps_the_rest() {
        let mut s = ChatDockState::default();
        for l in ["a", "b", "c", "d"] {
            apply_dock(&mut s, clean(chat("twitch", l)).unwrap(), false);
        }
        apply_reorder(&mut s, &["twitch:c".into(), "twitch:zzz".into(), "twitch:a".into()]);
        let order: Vec<_> = s.chats.iter().map(|c| c.login.as_str()).collect();
        assert_eq!(order, ["c", "a", "b", "d"]);
    }

    #[test]
    fn a_saved_pin_becomes_the_first_docked_chat() {
        let json = r#"{"pin":{"provider":"kick","login":"XQC","channel_id":"","display_name":"xQc","avatar_url":null},"view":"pinned"}"#;
        let s = from_pin_file(json).unwrap();
        assert_eq!(s.chats.len(), 1);
        assert_eq!(s.chats[0].key(), "kick:xqc");
        assert_eq!(s.active.as_deref(), Some("kick:xqc"));
        let live = from_pin_file(r#"{"pin":{"provider":"twitch","login":"a"},"view":"live"}"#).unwrap();
        assert_eq!(live.active, None);
        assert!(from_pin_file(r#"{"pin":null}"#).is_none());
    }

    #[test]
    fn a_saved_file_is_cleaned() {
        let s = sanitize(ChatDockState {
            chats: vec![chat("twitch", "A"), chat("twitch", "a"), chat("myspace", "x")],
            active: Some("twitch:gone".into()),
        });
        assert_eq!(s.chats.len(), 1);
        assert_eq!(s.active, None);
        // A file from before the mark setting existed reads as marking.
        let old: DockedChat = serde_json::from_str(r#"{"provider":"twitch","login":"a"}"#).unwrap();
        assert!(old.light_on_new);
    }
}
