//! The window flags a page asks for together, in one round trip, and the
//! window's full screen, which keeps the maximize across the trip.
//!
//! The Linux frame code wants to know three things about the main window at
//! once: whether it can be resized (to draw its own resize edges, since an
//! undecorated X11 window has none) and whether it is maximized or full screen
//! (both square, so the rounded frame comes off). Each is one plugin call, and
//! at boot every round trip waits behind whatever the page's main thread is
//! doing, so three in a row cost three waits. One command answers all of them.

use crate::rt::Window;
use serde::Serialize;
#[cfg(desktop)]
use std::collections::HashSet;
#[cfg(desktop)]
use std::sync::{Mutex, MutexGuard, OnceLock};

#[derive(Serialize)]
pub struct WindowFlags {
    pub resizable: bool,
    pub maximized: bool,
    pub fullscreen: bool,
}

/// Resizable, maximized and full screen, read together.
#[tauri::command]
pub fn get_window_flags(window: Window) -> Result<WindowFlags, String> {
    // The phone has no resizable, maximized or full-screen window to ask about.
    #[cfg(mobile)]
    {
        let _ = window;
        Err("window flags are a desktop question".to_string())
    }
    #[cfg(desktop)]
    {
        Ok(WindowFlags {
            resizable: window.is_resizable().map_err(|e| e.to_string())?,
            maximized: window.is_maximized().map_err(|e| e.to_string())?,
            fullscreen: window.is_fullscreen().map_err(|e| e.to_string())?,
        })
    }
}

/// Puts the calling window into full screen or takes it out of it, keeping its
/// maximize across the trip. `fullscreen: None` toggles. Answers whether the
/// window is full screen afterwards.
///
/// Both of the app's full screens come through here: the window's own (F11,
/// the title bar button) and the player's (Plyr's CSS full screen, which the
/// page mirrors onto the window). A window already where it was asked to be is
/// left alone, so one full screen taking the window over from the other keeps
/// the maximize the first one remembered.
///
/// Windows and Linux: a borderless window that is maximized keeps its maximized
/// placement, taskbar showing, even after it goes full screen. It is restored
/// first and remembered, and maximized again on the way out. The restore
/// placement Win32 keeps can drift over repeated trips, which sends a maximize
/// to the wrong rectangle, so the window moves to its monitor's origin first.
///
/// macOS: full screen is an animated transition AppKit owns, and zooming the
/// window just before or after it makes AppKit refuse it, so the maximize is
/// never touched there.
#[tauri::command]
pub async fn set_window_fullscreen(
    window: Window,
    fullscreen: Option<bool>,
) -> Result<bool, String> {
    #[cfg(mobile)]
    {
        let _ = (window, fullscreen);
        Err("full screen is a desktop window question".to_string())
    }
    #[cfg(desktop)]
    {
        let current = window.is_fullscreen().map_err(|e| e.to_string())?;
        let next = fullscreen.unwrap_or(!current);
        if next == current {
            return Ok(current);
        }
        if next {
            let was_maximized = cfg!(not(target_os = "macos"))
                && window.is_maximized().map_err(|e| e.to_string())?;
            remember_maximized(window.label(), was_maximized);
            if was_maximized {
                window.unmaximize().map_err(|e| e.to_string())?;
            }
            window.set_fullscreen(true).map_err(|e| e.to_string())?;
        } else {
            window.set_fullscreen(false).map_err(|e| e.to_string())?;
            if take_maximized(window.label()) {
                if let Some(monitor) = window.current_monitor().map_err(|e| e.to_string())? {
                    window
                        .set_position(*monitor.position())
                        .map_err(|e| e.to_string())?;
                }
                window.maximize().map_err(|e| e.to_string())?;
            }
        }
        Ok(next)
    }
}

/// Labels of the windows that were maximized when they went full screen.
#[cfg(desktop)]
fn maximized_before_fullscreen() -> MutexGuard<'static, HashSet<String>> {
    static LABELS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    LABELS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Records whether `label` was maximized as it went full screen. A window that
/// was not forgets any older answer, so a stale one can never maximize it.
#[cfg(desktop)]
fn remember_maximized(label: &str, maximized: bool) {
    let mut labels = maximized_before_fullscreen();
    if maximized {
        labels.insert(label.to_owned());
    } else {
        labels.remove(label);
    }
}

/// Whether `label` should be maximized again on leaving full screen. Answers
/// once per trip.
#[cfg(desktop)]
fn take_maximized(label: &str) -> bool {
    maximized_before_fullscreen().remove(label)
}

#[cfg(all(test, desktop))]
mod tests {
    use super::*;

    #[test]
    fn a_maximized_window_is_maximized_again_once() {
        remember_maximized("test-maximized", true);
        assert!(take_maximized("test-maximized"));
        assert!(!take_maximized("test-maximized"));
    }

    #[test]
    fn a_restored_window_clears_an_older_answer() {
        remember_maximized("test-restored", true);
        remember_maximized("test-restored", false);
        assert!(!take_maximized("test-restored"));
    }

    #[test]
    fn windows_are_remembered_apart() {
        remember_maximized("test-main", true);
        remember_maximized("test-popout", false);
        assert!(!take_maximized("test-popout"));
        assert!(take_maximized("test-main"));
    }
}
