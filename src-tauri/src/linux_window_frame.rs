//! Who draws the main window's frame on Linux.
//!
//! The window is undecorated on Windows and Linux. Windows 11 still rounds its
//! corners and draws a hairline border, because DWM does that for every
//! top-level window. On Linux nothing does: an undecorated window is a
//! hard-edged rectangle.
//!
//! On a floating desktop (GNOME, KDE, Cinnamon, Xfce, ...) StreamNook draws the
//! frame itself: the window is created transparent, and the page rounds and
//! borders its own body (`html[data-window-frame="rounded"]` in globals.css).
//!
//! A tiling compositor (Hyprland, sway, niri, i3, ...) already rounds and
//! borders every window from the user's own config, so a frame from us would
//! sit inside theirs. There the window stays opaque and square and the
//! compositor's frame is the only one. Opaque matters beyond looks: a window
//! with an alpha channel makes those compositors blend, and often blur, behind
//! it on every frame.
//!
//! The decision is made once per process. The window builders read it for
//! transparency, and the page reads it from `window.__SN_WINDOW_FRAME__`, set
//! by an init script that runs before any page script, so all three ways the
//! main window is created (config, Rust, JS) agree.
//!
//! On the CEF runtime (`rt.rs`) every desktop gets `Compositor`. Chromium
//! embedded as a windowed browser only paints transparently off-screen, which
//! the runtime does not use: a transparent X11 window fails Chromium's
//! `CreateWindow` with `BadMatch` and stays black. So the window is opaque and
//! square everywhere; `decide` keeps the desktop classification for the day a
//! rounded frame is drawn some other way, and the page plumbing stays in place.

/// Who draws the window frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Frame {
    /// StreamNook draws it: transparent window, rounded and bordered page.
    Rounded,
    /// The compositor draws it: opaque window, square page.
    Compositor,
}

impl Frame {
    pub fn as_str(self) -> &'static str {
        match self {
            Frame::Rounded => "rounded",
            Frame::Compositor => "compositor",
        }
    }

    pub fn transparent(self) -> bool {
        self == Frame::Rounded
    }
}

/// Desktops (as named in `XDG_CURRENT_DESKTOP`) whose compositor frames every
/// window itself. Compared case-insensitively.
const TILING_DESKTOPS: &[&str] = &[
    "hyprland",
    "sway",
    "niri",
    "river",
    "i3",
    "bspwm",
    "dwm",
    "awesome",
    "qtile",
    "xmonad",
    "herbstluftwm",
    "leftwm",
    "spectrwm",
];

/// Decide from `XDG_CURRENT_DESKTOP` (a colon-separated list) and whether a
/// tiling compositor's own socket variable is set, which catches sessions that
/// leave `XDG_CURRENT_DESKTOP` unset. Anything else, including an unknown
/// desktop, gets our frame: a floating window manager draws nothing for an
/// undecorated window.
pub fn decide(current_desktop: Option<&str>, tiling_socket_present: bool) -> Frame {
    if tiling_socket_present {
        return Frame::Compositor;
    }
    let tiling = current_desktop
        .unwrap_or("")
        .split(':')
        .map(|d| d.trim().to_ascii_lowercase())
        .any(|d| TILING_DESKTOPS.contains(&d.as_str()));
    if tiling {
        Frame::Compositor
    } else {
        Frame::Rounded
    }
}

/// The script that hands the decision to the page before its first paint.
pub fn init_script(frame: Frame) -> String {
    format!("window.__SN_WINDOW_FRAME__ = \"{}\";", frame.as_str())
}

/// This process's decision: `Compositor` on every desktop, because the CEF
/// runtime cannot paint a transparent window (module doc). The desktop is
/// still classified, for the log.
#[cfg(target_os = "linux")]
pub fn frame() -> Frame {
    static FRAME: std::sync::OnceLock<Frame> = std::sync::OnceLock::new();
    *FRAME.get_or_init(|| {
        let socket = ["HYPRLAND_INSTANCE_SIGNATURE", "SWAYSOCK", "NIRI_SOCKET", "I3SOCK"]
            .iter()
            .any(|v| std::env::var_os(v).is_some_and(|s| !s.is_empty()));
        let desktop = std::env::var("XDG_CURRENT_DESKTOP").ok();
        let wanted = decide(desktop.as_deref(), socket);
        log::info!(
            "[LinuxFrame] desktop={} would draw {}; the CEF runtime cannot paint a transparent window, so the frame is {}",
            desktop.as_deref().unwrap_or("(unset)"),
            wanted.as_str(),
            Frame::Compositor.as_str()
        );
        Frame::Compositor
    })
}

/// A plugin with no commands whose only job is the init script above, which
/// Tauri adds to every webview created after it is registered.
#[cfg(target_os = "linux")]
pub fn plugin<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("sn-window-frame")
        .js_init_script(init_script(frame()))
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floating_desktops_get_our_frame() {
        for d in ["GNOME", "ubuntu:GNOME", "KDE", "X-Cinnamon", "XFCE", "MATE", "Budgie:GNOME", "COSMIC"] {
            assert_eq!(decide(Some(d), false), Frame::Rounded, "{d}");
        }
    }

    #[test]
    fn tiling_compositors_keep_their_own_frame() {
        for d in ["Hyprland", "sway", "niri", "river", "i3", "bspwm", "Hyprland:wlroots"] {
            assert_eq!(decide(Some(d), false), Frame::Compositor, "{d}");
        }
    }

    #[test]
    fn a_tiling_socket_wins_even_without_a_desktop_name() {
        assert_eq!(decide(None, true), Frame::Compositor);
        assert_eq!(decide(Some("GNOME"), true), Frame::Compositor);
    }

    #[test]
    fn an_unknown_or_missing_desktop_gets_our_frame() {
        assert_eq!(decide(None, false), Frame::Rounded);
        assert_eq!(decide(Some("openbox"), false), Frame::Rounded);
        assert_eq!(decide(Some(""), false), Frame::Rounded);
    }

    #[test]
    fn only_our_frame_makes_the_window_transparent() {
        assert!(Frame::Rounded.transparent());
        assert!(!Frame::Compositor.transparent());
    }

    #[test]
    fn the_init_script_names_the_frame() {
        assert_eq!(init_script(Frame::Rounded), "window.__SN_WINDOW_FRAME__ = \"rounded\";");
        assert_eq!(init_script(Frame::Compositor), "window.__SN_WINDOW_FRAME__ = \"compositor\";");
    }
}
