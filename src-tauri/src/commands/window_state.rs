//! The window flags a page asks for together, in one round trip.
//!
//! The Linux frame code wants to know three things about the main window at
//! once: whether it can be resized (to draw its own resize edges, since an
//! undecorated X11 window has none) and whether it is maximized or full screen
//! (both square, so the rounded frame comes off). Each is one plugin call, and
//! at boot every round trip waits behind whatever the page's main thread is
//! doing, so three in a row cost three waits. One command answers all of them.

use crate::rt::Window;
use serde::Serialize;

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
