//! The chat pinned in the main window.
//!
//! Pinning keeps one chat in place while the viewer moves between streams. The
//! pinned chat and the current stream's chat are the two sides of one switch,
//! and the viewer flips between them; the side they last chose stays chosen
//! across stream switches. Not the same thing as a pinned MESSAGE, which is
//! channel_state's `pinned`.
//!
//! The pin lives here, not in the page, because Go Live and close-to-tray
//! destroy the main window and rebuild it, and the pin has to come back with
//! it. It is also saved, so it survives a restart until the viewer unpins.
//! The page mirrors it from `get_chat_pin` and the `chat-pin` event.

use crate::rt::AppHandle;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::Emitter;

pub const EVENT: &str = "chat-pin";

const PROVIDERS: [&str; 4] = ["twitch", "kick", "youtube", "tiktok"];

/// The channel whose chat is pinned, as the page knew it when pinning.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ChatPin {
    pub provider: String,
    pub login: String,
    #[serde(default)]
    pub channel_id: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub avatar_url: Option<String>,
}

/// Which side of the switch the main window shows.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ChatPinView {
    Pinned,
    #[default]
    Live,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct ChatPinState {
    pub pin: Option<ChatPin>,
    #[serde(default)]
    pub view: ChatPinView,
}

static STATE: Lazy<Mutex<ChatPinState>> = Lazy::new(|| Mutex::new(load()));

/// The pin and the side shown, for a window that is starting up.
#[tauri::command]
pub fn get_chat_pin() -> ChatPinState {
    STATE.lock().map(|s| s.clone()).unwrap_or_default()
}

/// Pin a channel's chat, or unpin with `None`. Pinning shows the pinned side;
/// unpinning goes back to the live chat.
#[tauri::command]
pub fn set_chat_pin(app: AppHandle, pin: Option<ChatPin>) -> Result<ChatPinState, String> {
    let pin = pin.map(clean).transpose()?;
    update(&app, |s| {
        s.view = if pin.is_some() { ChatPinView::Pinned } else { ChatPinView::Live };
        s.pin = pin;
    })
}

/// Show the pinned chat or the current stream's. Ignored while nothing is
/// pinned, so the live side is the only one there is.
#[tauri::command]
pub fn set_chat_pin_view(app: AppHandle, view: ChatPinView) -> Result<ChatPinState, String> {
    update(&app, |s| {
        if s.pin.is_some() {
            s.view = view;
        }
    })
}

fn update(app: &AppHandle, change: impl FnOnce(&mut ChatPinState)) -> Result<ChatPinState, String> {
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
        log::warn!("[ChatPin] could not save the pin: {e}");
    }
    let _ = app.emit_to("main", EVENT, &next);
    Ok(next)
}

/// A pin the page sent, checked and normalised.
fn clean(mut pin: ChatPin) -> Result<ChatPin, String> {
    pin.provider = pin.provider.trim().to_ascii_lowercase();
    if !PROVIDERS.contains(&pin.provider.as_str()) {
        return Err(format!("unknown platform: {}", pin.provider));
    }
    pin.login = pin.login.trim().trim_start_matches('@').to_lowercase();
    if pin.login.is_empty() || pin.login.len() > 128 {
        return Err("a pinned chat needs a channel".into());
    }
    if pin.display_name.trim().is_empty() {
        pin.display_name = pin.login.clone();
    }
    Ok(pin)
}

fn store_path() -> Option<PathBuf> {
    crate::services::cache_service::get_app_data_dir()
        .ok()
        .map(|d| d.join("chat_pin.json"))
}

fn load() -> ChatPinState {
    store_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<ChatPinState>(&s).ok())
        // A saved view without a pin means nothing; never restore one.
        .map(|s| if s.pin.is_none() { ChatPinState::default() } else { s })
        .unwrap_or_default()
}

fn save(state: &ChatPinState) -> Result<(), String> {
    let path = store_path().ok_or("no app data dir")?;
    let json = serde_json::to_string(state).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)
        .and_then(|_| std::fs::rename(&tmp, &path))
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pin(provider: &str, login: &str) -> ChatPin {
        ChatPin {
            provider: provider.into(),
            login: login.into(),
            channel_id: "1".into(),
            display_name: String::new(),
            avatar_url: None,
        }
    }

    #[test]
    fn a_pin_is_normalised() {
        let p = clean(pin(" Twitch ", "@Rainyyay ")).unwrap();
        assert_eq!(p.provider, "twitch");
        assert_eq!(p.login, "rainyyay");
        assert_eq!(p.display_name, "rainyyay", "a missing name falls back to the login");
    }

    #[test]
    fn a_pin_needs_a_known_platform_and_a_channel() {
        assert!(clean(pin("myspace", "tom")).is_err());
        assert!(clean(pin("kick", "  ")).is_err());
        assert!(clean(pin("kick", &"a".repeat(129))).is_err());
    }

    #[test]
    fn the_saved_shape_reads_back() {
        let s = ChatPinState { pin: Some(clean(pin("kick", "xqc")).unwrap()), view: ChatPinView::Pinned };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"view\":\"pinned\""));
        assert_eq!(serde_json::from_str::<ChatPinState>(&json).unwrap(), s);
        // An older or hand-edited file without a view reads as live.
        let bare: ChatPinState = serde_json::from_str(r#"{"pin":null}"#).unwrap();
        assert_eq!(bare.view, ChatPinView::Live);
    }
}
