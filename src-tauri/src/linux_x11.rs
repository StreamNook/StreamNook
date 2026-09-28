//! X11 window hints the CEF runtime does not expose.
//!
//! The sign-in overlays and their popups are owned by the window they float
//! over: `WebviewWindowBuilder::parent` on Windows and macOS, and under
//! WebKitGTK `gtk_window_set_transient_for`. The CEF runtime's Linux windows
//! are winit X11 windows and it answers `parent()` with an error, so the
//! ownership is written straight onto the X window instead: `WM_TRANSIENT_FOR`
//! is the property GTK set underneath, and the window manager reads it the
//! same way (the overlay stays above its owner, minimises with it, and is
//! not offered as a separate task).

//!
//! The aspect-ratio lock is the other hint: GTK's `set_geometry_hints` wrote
//! `WM_NORMAL_HINTS`, and `set_normal_hints` writes the same property with
//! the same fields (minimum, base size and aspect), so the window manager
//! rubber-bands a drag exactly as before.

use crate::rt::WebviewWindow;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// The X window id behind a Tauri window.
pub fn xid(window: &WebviewWindow) -> Result<u64, String> {
    match window.window_handle().map_err(|e| e.to_string())?.as_raw() {
        RawWindowHandle::Xlib(h) => Ok(h.window),
        RawWindowHandle::Xcb(h) => Ok(u64::from(h.window.get())),
        other => Err(format!("not an X11 window: {other:?}")),
    }
}

/// Make `window` transient for `owner`.
///
/// Call after `build()`. The hint is written on a connection of its own and
/// flushed before it closes, so it lands regardless of what the runtime's
/// event loop is doing.
pub fn make_transient_for(window: &WebviewWindow, owner: &WebviewWindow) -> Result<(), String> {
    let (child, parent) = (xid(window)?, xid(owner)?);
    let xlib = x11_dl::xlib::Xlib::open().map_err(|e| e.to_string())?;
    // SAFETY: Xlib calls on a display this function opens and closes itself;
    // the ids came from live windows the runtime owns.
    unsafe {
        let display = (xlib.XOpenDisplay)(std::ptr::null());
        if display.is_null() {
            return Err("XOpenDisplay failed".into());
        }
        (xlib.XSetTransientForHint)(display, child, parent);
        (xlib.XFlush)(display);
        (xlib.XCloseDisplay)(display);
    }
    log::debug!("[X11] window 0x{child:x} is transient for 0x{parent:x}");
    Ok(())
}

/// The size hints the aspect lock writes, in device pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SizeHints {
    pub min_width: i32,
    pub min_height: i32,
    /// The chrome around the video box: the ratio applies to the window
    /// minus this. `None` clears the base size.
    pub base: Option<(i32, i32)>,
    /// Width over height to pin the window's (base-less) size to. `None`
    /// clears the aspect hint.
    pub aspect: Option<f64>,
}

/// Write `WM_NORMAL_HINTS` on `window_xid`: the minimum always, base size and
/// aspect when given. Read-modify-write, so the flags the window library set
/// on the same property (gravity, program size) survive.
pub fn set_normal_hints(window_xid: u64, hints: SizeHints) -> Result<(), String> {
    use x11_dl::xlib::{PAspect, PBaseSize, PMinSize, XSizeHints};

    let xlib = x11_dl::xlib::Xlib::open().map_err(|e| e.to_string())?;
    // SAFETY: a display this function opens and closes; `XSizeHints` is plain
    // data the X calls fill in and read back.
    unsafe {
        let display = (xlib.XOpenDisplay)(std::ptr::null());
        if display.is_null() {
            return Err("XOpenDisplay failed".into());
        }
        let mut size: XSizeHints = std::mem::zeroed();
        let mut supplied: std::os::raw::c_long = 0;
        (xlib.XGetWMNormalHints)(display, window_xid, &mut size, &mut supplied);

        size.flags |= PMinSize;
        size.min_width = hints.min_width;
        size.min_height = hints.min_height;
        match hints.base {
            Some((w, h)) => {
                size.flags |= PBaseSize;
                size.base_width = w;
                size.base_height = h;
            }
            None => {
                size.flags &= !PBaseSize;
                size.base_width = 0;
                size.base_height = 0;
            }
        }
        match hints.aspect.filter(|r| r.is_finite() && *r > 0.0) {
            Some(ratio) => {
                // Width over height as an integer fraction; 1/10000 is far
                // finer than a pixel at any window size.
                let (x, y) = ((ratio * 10_000.0).round() as i32, 10_000);
                size.flags |= PAspect;
                size.min_aspect.x = x;
                size.min_aspect.y = y;
                size.max_aspect.x = x;
                size.max_aspect.y = y;
            }
            None => {
                size.flags &= !PAspect;
            }
        }
        (xlib.XSetWMNormalHints)(display, window_xid, &mut size);
        (xlib.XFlush)(display);
        (xlib.XCloseDisplay)(display);
    }
    Ok(())
}

/// The lowest opacity `set_window_opacity` writes. The whole window fades,
/// text included, and a window at zero could not be seen or found again.
pub const MIN_WINDOW_OPACITY: f64 = 0.2;

/// `_NET_WM_WINDOW_OPACITY` for `opacity` (0 to 1, raised to
/// `MIN_WINDOW_OPACITY`): a CARDINAL where `0xffffffff` is opaque.
pub fn opacity_cardinal(opacity: f64) -> u32 {
    let opacity = if opacity.is_finite() { opacity.clamp(MIN_WINDOW_OPACITY, 1.0) } else { 1.0 };
    (opacity * f64::from(u32::MAX)).round() as u32
}

/// Fade the calling window as a whole through the compositor. A transparent
/// window cannot be painted by this runtime, so the chat overlay is opaque on
/// Linux and its glass slider drives `_NET_WM_WINDOW_OPACITY` instead, which
/// every compositing window manager applies to the whole window. Without a
/// compositor the window simply stays opaque.
#[tauri::command]
pub fn set_window_opacity(window: WebviewWindow, opacity: f64) -> Result<(), String> {
    use x11_dl::xlib::{PropModeReplace, XA_CARDINAL};

    let window_xid = xid(&window)?;
    let value = std::os::raw::c_ulong::from(opacity_cardinal(opacity));
    let xlib = x11_dl::xlib::Xlib::open().map_err(|e| e.to_string())?;
    // SAFETY: a display this function opens and closes; the property data is
    // one 32-bit item, passed as a C long as Xlib requires for format 32.
    unsafe {
        let display = (xlib.XOpenDisplay)(std::ptr::null());
        if display.is_null() {
            return Err("XOpenDisplay failed".into());
        }
        let atom = (xlib.XInternAtom)(display, c"_NET_WM_WINDOW_OPACITY".as_ptr(), 0);
        (xlib.XChangeProperty)(
            display,
            window_xid,
            atom,
            XA_CARDINAL,
            32,
            PropModeReplace,
            (&value as *const std::os::raw::c_ulong).cast(),
            1,
        );
        (xlib.XFlush)(display);
        (xlib.XCloseDisplay)(display);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opacity_maps_to_the_cardinal_with_a_floor() {
        assert_eq!(opacity_cardinal(1.0), u32::MAX);
        assert_eq!(opacity_cardinal(2.0), u32::MAX);
        assert_eq!(opacity_cardinal(f64::NAN), u32::MAX);
        assert_eq!(opacity_cardinal(0.0), opacity_cardinal(MIN_WINDOW_OPACITY));
        assert_eq!(opacity_cardinal(-1.0), opacity_cardinal(MIN_WINDOW_OPACITY));
        assert_eq!(opacity_cardinal(0.5), (f64::from(u32::MAX) * 0.5).round() as u32);
    }
}
