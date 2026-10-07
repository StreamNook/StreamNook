//! The emote menu popped out into a window of its own, so it can stay open
//! beside the chat for quick reactions.
//!
//! The page is the same emote menu the chat box opens; this module owns which
//! chat box it types into. A click in the palette is an intent sent here, and
//! Rust hands the text to that one chat box (`emit_to` its window; the page
//! listens on its own window and checks the composer id), then gives that
//! window focus back so Enter sends. The handback is not optional: a WebView2
//! page always activates its window when clicked. `focusable(false)`
//! (WS_EX_NOACTIVATE) and answering WM_MOUSEACTIVATE with MA_NOACTIVATE were
//! both measured not to stop it, because the input windows belong to the
//! WebView2 browser process.
//!
//! Only chat windows may claim the palette and only the palette may type, so
//! no other page (a plugin window, an overlay) can put text in a chat box.

use super::popout_window::{self, PopoutOutcome, EMOTE_PALETTE_LABEL};
use crate::rt::{AppHandle, Window};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::{Emitter, Manager};

/// Sent to the palette when the chat it serves changes.
const TARGET_EVENT: &str = "emote-palette://target";
/// Sent to the chat box's window: text to put at its cursor.
const INSERT_EVENT: &str = "emote-palette://insert";
/// Longer than any emote name or emoji sequence; a chat message's own limit.
const MAX_INSERT_CHARS: usize = 500;

/// The chat the palette serves, in the page's key names.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PaletteChannel {
    /// Twitch login, Kick slug or YouTube video id: what the emote set is
    /// fetched by.
    pub login: String,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    pub provider: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Target {
    window: String,
    composer: String,
    channel: PaletteChannel,
}

static TARGET: Mutex<Option<Target>> = Mutex::new(None);

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Insert {
    composer: String,
    text: String,
}

/// Windows that hold chat boxes.
fn is_composer_window(label: &str) -> bool {
    label == "main" || label.starts_with("multichat-")
}

fn valid_composer_id(id: &str) -> bool {
    (1..=64).contains(&id.chars().count()) && !id.chars().any(char::is_control)
}

fn valid_channel(c: &PaletteChannel) -> bool {
    let plain = |s: &str, max: usize| (1..=max).contains(&s.len()) && !s.chars().any(char::is_control);
    matches!(c.provider.as_str(), "twitch" | "kick" | "youtube")
        && plain(&c.login, 128)
        && c.id.as_deref().is_none_or(|s| s.is_empty() || plain(s, 128))
        && c.name.as_deref().is_none_or(|s| s.is_empty() || plain(s, 128))
}

/// What a palette click may put in a chat box: control characters dropped (a
/// newline would send the message), at most `MAX_INSERT_CHARS`, never empty.
fn clean_insert(text: &str) -> Option<String> {
    let kept: String = text.chars().filter(|c| !c.is_control()).take(MAX_INSERT_CHARS).collect();
    let kept = kept.trim();
    (!kept.is_empty()).then(|| kept.to_string())
}

fn target() -> Option<Target> {
    TARGET.lock().ok()?.clone()
}

fn set_target(next: Option<Target>) -> Option<Target> {
    match TARGET.lock() {
        Ok(mut guard) => std::mem::replace(&mut *guard, next),
        Err(_) => None,
    }
}

fn close_palette(app: &AppHandle) {
    set_target(None);
    if let Some(win) = app.get_webview_window(EMOTE_PALETTE_LABEL) {
        let _ = win.close();
    }
}

/// Pop the emote menu out for the chat box `composer` in the calling window.
/// Already open: it switches to this chat (and, on Windows, this window owns it).
#[tauri::command]
pub async fn open_emote_palette(
    app: AppHandle,
    window: Window,
    composer: String,
    channel: PaletteChannel,
) -> Result<PopoutOutcome, String> {
    let caller = window.label().to_string();
    if !is_composer_window(&caller) {
        return Err(format!("{caller} holds no chat box"));
    }
    if !valid_composer_id(&composer) || !valid_channel(&channel) {
        return Err("not a chat box the emote menu can serve".into());
    }
    set_target(Some(Target { window: caller, composer, channel: channel.clone() }));

    if let Some(open) = app.get_webview_window(EMOTE_PALETTE_LABEL) {
        popout_window::set_emote_palette_owner(&open, &window);
        let _ = app.emit_to(EMOTE_PALETTE_LABEL, TARGET_EVENT, &channel);
        popout_window::bring_forward(&open);
        return Ok(PopoutOutcome::Focused);
    }
    popout_window::build_emote_palette(&app, &window)?;
    Ok(PopoutOutcome::Opened)
}

/// The chat the palette serves, asked by the palette as it boots.
#[tauri::command]
pub fn get_emote_palette_target(window: Window) -> Option<PaletteChannel> {
    if window.label() != EMOTE_PALETTE_LABEL {
        return None;
    }
    target().map(|t| t.channel)
}

/// A palette click: put `text` in the chat box it serves, then give that
/// window focus back so the next key goes to the chat box.
#[tauri::command]
pub fn emote_palette_insert(app: AppHandle, window: Window, text: String) -> Result<(), String> {
    if window.label() != EMOTE_PALETTE_LABEL {
        return Err("only the emote menu can type into chat".into());
    }
    let text = clean_insert(&text).ok_or("nothing to insert")?;
    let target = target().ok_or("the chat this menu served is gone")?;
    let Some(chat) = app.get_webview_window(&target.window) else {
        close_palette(&app);
        return Err("the chat this menu served is gone".into());
    };
    app.emit_to(target.window.as_str(), INSERT_EVENT, Insert { composer: target.composer, text })
        .map_err(|e| e.to_string())?;
    // A minimized chat stays where it is; bringing it back is the viewer's call.
    if !chat.is_minimized().unwrap_or(false) {
        let _ = chat.set_focus();
    }
    Ok(())
}

/// The chat box the palette serves switched channels (or was retargeted by
/// the page), so the palette shows that channel's emotes. Ignored unless the
/// calling chat box is the one served.
#[tauri::command]
pub fn update_emote_palette_channel(
    app: AppHandle,
    window: Window,
    composer: String,
    channel: PaletteChannel,
) -> Result<(), String> {
    if !valid_channel(&channel) {
        return Err("not a channel the emote menu can serve".into());
    }
    let Ok(mut guard) = TARGET.lock() else { return Ok(()) };
    let Some(t) = guard.as_mut() else { return Ok(()) };
    if t.window != window.label() || t.composer != composer || t.channel == channel {
        return Ok(());
    }
    t.channel = channel.clone();
    drop(guard);
    let _ = app.emit_to(EMOTE_PALETTE_LABEL, TARGET_EVENT, &channel);
    Ok(())
}

/// The chat box went away (its view or pane closed): its palette closes too.
#[tauri::command]
pub fn release_emote_palette(app: AppHandle, window: Window, composer: String) {
    let served = target().is_some_and(|t| t.window == window.label() && t.composer == composer);
    if served {
        close_palette(&app);
    }
}

/// A window was destroyed or hidden to the tray. The palette closes with the
/// chat it serves; a closed palette forgets its chat. Owned windows go with
/// their owner on Windows already, but not on hide, and other platforms have
/// no owner at all.
pub fn on_window_gone(app: &AppHandle, label: &str) {
    if label == EMOTE_PALETTE_LABEL {
        // Only when no palette remains: a palette is never rebuilt, but a
        // stale event must not wipe the chat a live one serves.
        if app.get_webview_window(EMOTE_PALETTE_LABEL).is_none() {
            set_target(None);
        }
        return;
    }
    if target().is_some_and(|t| t.window == label) {
        close_palette(app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel(provider: &str) -> PaletteChannel {
        PaletteChannel { login: "xqc".into(), id: Some("71092938".into()), name: Some("xQc".into()), provider: provider.into() }
    }

    #[test]
    fn only_chat_windows_claim_the_palette() {
        assert!(is_composer_window("main"));
        assert!(is_composer_window("multichat-default"));
        assert!(is_composer_window("multichat-w2"));
        assert!(!is_composer_window("emote-palette"));
        assert!(!is_composer_window("overlay-xqc"));
        assert!(!is_composer_window("plugin-some.plugin-page"));
        assert!(!is_composer_window("profile-xqc-1"));
    }

    #[test]
    fn insert_text_cannot_send_or_flood() {
        assert_eq!(clean_insert("KEKW").as_deref(), Some("KEKW"));
        // A newline would submit the message in the chat box.
        assert_eq!(clean_insert("KEKW\n").as_deref(), Some("KEKW"));
        assert_eq!(clean_insert("a\r\nb").as_deref(), Some("ab"));
        assert_eq!(clean_insert(" \t\n"), None);
        assert_eq!(clean_insert(""), None);
        assert_eq!(clean_insert(&"x".repeat(5000)).map(|s| s.chars().count()), Some(MAX_INSERT_CHARS));
        // Emoji sequences survive whole: ZWJ and variation selectors are not controls.
        let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
        assert_eq!(clean_insert(family).as_deref(), Some(family));
        assert_eq!(clean_insert("\u{2764}\u{FE0F}").as_deref(), Some("\u{2764}\u{FE0F}"));
    }

    #[test]
    fn channels_and_composer_ids_are_checked() {
        assert!(valid_channel(&channel("twitch")));
        assert!(valid_channel(&channel("kick")));
        assert!(valid_channel(&channel("youtube")));
        assert!(!valid_channel(&channel("tiktok")));
        assert!(!valid_channel(&PaletteChannel { login: String::new(), ..channel("twitch") }));
        assert!(!valid_channel(&PaletteChannel { login: "a\nb".into(), ..channel("twitch") }));
        assert!(valid_channel(&PaletteChannel { id: None, name: None, ..channel("kick") }));
        assert!(valid_composer_id("\u{ab}r1\u{bb}"));
        assert!(valid_composer_id(":r1f:"));
        assert!(!valid_composer_id(""));
        assert!(!valid_composer_id(&"x".repeat(65)));
        assert!(!valid_composer_id("a\u{0}"));
    }
}
