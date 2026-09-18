//! Live aspect-ratio constraint for the main window.
//!
//! # Why this is a native hook and not a resize handler
//!
//! The aspect-ratio lock used to be a correction applied AFTER the fact: the
//! frontend listened to `onResized`, debounced 100 ms, asked Rust for a
//! conforming size and called `setSize()`. Two properties of that design make a
//! drag feel broken rather than locked:
//!
//! 1. **Dragging a window edge runs the OS modal sizing loop.** `onResized`
//!    fires throughout it, so any pause longer than the debounce lands a
//!    `setSize()` *inside* the loop. That corrupts the loop's cached rect
//!    exactly the way it does for the titlebar move loop (see
//!    `start_titlebar_drag`), and the next mouse movement re-applies the rect
//!    the loop still believes in, so the window jumps back to a size the user
//!    never asked for.
//! 2. **The formula locked one axis and derived the other.** With chat on the
//!    right the width was authoritative, so the whole vertical delta of a
//!    bottom-edge or corner drag was discarded; with chat on the bottom the
//!    same was true of the horizontal delta. Half of the gestures a user can
//!    make did nothing but snap back.
//!
//! `WM_SIZING` exists for precisely this: the OS hands the app the rect the
//! pointer is dragging towards BEFORE applying it, the app adjusts it in place,
//! and the OS rubber-bands to the adjusted rect. Constraining there gives the
//! feel of a native aspect-locked window - the dragged edge tracks the pointer
//! every frame, and nothing is ever undone after the fact.
//!
//! # Division of labour
//!
//! The frontend owns the *shape* of the constraint (which video aspect ratio,
//! how much chrome sits outside the video box: title bar, sidebar, chat panel
//! and its separator, MultiNook gaps). It pushes that through
//! `set_window_aspect_constraint` whenever it changes. This module only applies
//! it, and only while a user drag is in flight.
//!
//! Offsets arrive in LOGICAL pixels and are scaled with the window's live DPI
//! here, not by the caller: a drag can cross onto a monitor with different
//! scaling mid-gesture, and a cached scale factor would skew the constraint for
//! the rest of the drag. See `Brain/references/Tauri_Window_Sizing_Units.md`.
//!
//! # Platforms
//!
//! Windows constrains live. Everywhere else `constrains_live()` reports false
//! and the frontend keeps its debounced correction, which is jerky but is what
//! those platforms had before this module existed. The two paths are mutually
//! exclusive by construction: the frontend asks, once, which one is in force.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

/// Whether a user drag should be constrained at all. Cleared for theater mode,
/// for a window with no video in it, and whenever the user turns the lock off.
static ENABLED: AtomicBool = AtomicBool::new(false);

/// Target `video width / video height`, as `f64` bits. 16:9 for a single
/// stream; MultiNook pushes the ratio of its whole tile grid.
static RATIO_BITS: AtomicU64 = AtomicU64::new(0);

/// Chrome outside the video box, in LOGICAL pixels.
static EXTRA_W: AtomicU32 = AtomicU32::new(0);
static EXTRA_H: AtomicU32 = AtomicU32::new(0);

/// `minWidth` / `minHeight` from `tauri.conf.json`, in logical pixels. The
/// constraint runs after the sizing loop has already enforced the minimum via
/// `WM_GETMINMAXINFO`, so it has to re-apply it or a derived axis could push
/// the window under it.
const MIN_LOGICAL_W: f64 = 800.0;
const MIN_LOGICAL_H: f64 = 600.0;

/// Floor on the video box itself, in logical pixels. Covers the degenerate case
/// where the chrome alone is wider than the minimum window (a very wide chat
/// panel), which would otherwise leave a negative video width.
const MIN_VIDEO_LOGICAL_W: f64 = 160.0;

/// Store the constraint the frontend just computed.
///
/// The shape is written before the enable flag so the hook can never read a
/// half-updated constraint, and cleared before it for the same reason.
pub fn set_constraint(enabled: bool, ratio: f64, extra_width: u32, extra_height: u32) {
    let usable = enabled && ratio.is_finite() && ratio > 0.0;
    if !usable {
        ENABLED.store(false, Ordering::Release);
        return;
    }
    RATIO_BITS.store(ratio.to_bits(), Ordering::Relaxed);
    EXTRA_W.store(extra_width, Ordering::Relaxed);
    EXTRA_H.store(extra_height, Ordering::Relaxed);
    ENABLED.store(true, Ordering::Release);
}

/// True when this platform constrains the window DURING the drag, which is the
/// frontend's signal to stand its own debounced correction down. Reports the
/// real installed state rather than the target OS, so a failed hook falls back
/// to the old path instead of silently disabling the lock.
pub fn constrains_live() -> bool {
    #[cfg(windows)]
    {
        imp::is_installed()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Attach the sizing hook to the main window. Called once per window at
/// creation (setup, and again when the tray recreates the window after Go Live
/// destroyed it), mirroring `ui_hang_watchdog::start_for_hwnd`. Inert until a
/// constraint is enabled, so it costs one comparison per `WM_SIZING`.
#[allow(unused_variables)]
pub fn install_for_hwnd(hwnd_raw: isize) {
    #[cfg(windows)]
    imp::install(hwnd_raw);
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicBool, Ordering};

    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
    use windows::Win32::UI::HiDpi::GetDpiForWindow;
    use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetClientRect, GetWindowRect, WM_SIZING, WMSZ_BOTTOM, WMSZ_BOTTOMLEFT, WMSZ_LEFT,
        WMSZ_RIGHT, WMSZ_TOP, WMSZ_TOPLEFT, WMSZ_TOPRIGHT,
    };

    use super::{
        ENABLED, EXTRA_H, EXTRA_W, MIN_LOGICAL_H, MIN_LOGICAL_W, MIN_VIDEO_LOGICAL_W, RATIO_BITS,
    };

    /// Subclass id, unique within this window. Arbitrary; "SNAW" as bytes.
    const SUBCLASS_ID: usize = 0x534e_4157;

    static INSTALLED: AtomicBool = AtomicBool::new(false);

    pub fn is_installed() -> bool {
        INSTALLED.load(Ordering::Acquire)
    }

    pub fn install(hwnd_raw: isize) {
        let hwnd = HWND(hwnd_raw as *mut c_void);
        // Idempotent by contract: re-installing the same proc + id only
        // replaces the reference data, so a window recreated on the same HWND
        // value re-arms cleanly instead of skipping and silently losing the
        // lock.
        let ok = unsafe { SetWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID, 0) }.as_bool();
        INSTALLED.store(ok, Ordering::Release);
        if ok {
            log::debug!("[WindowAspect] sizing hook attached");
        } else {
            log::warn!(
                "[WindowAspect] SetWindowSubclass failed; the aspect lock falls back to the \
                 debounced frontend correction"
            );
        }
    }

    unsafe extern "system" fn subclass_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _id: usize,
        _data: usize,
    ) -> LRESULT {
        if msg == WM_SIZING && ENABLED.load(Ordering::Acquire) && lparam.0 != 0 {
            // SAFETY: for WM_SIZING the OS passes a pointer to the rect it is
            // about to apply, valid for the duration of this message, and this
            // proc runs on the thread that owns the window.
            let rect = unsafe { &mut *(lparam.0 as *mut RECT) };
            constrain(hwnd, wparam.0 as u32, rect);
        }
        // Pass down regardless. tao does not handle WM_SIZING, and DefWindowProc
        // does not touch the rect, so the adjustment above survives intact.
        unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
    }

    /// Current window and client sizes, in physical pixels.
    fn measure(hwnd: HWND) -> Option<((f64, f64), (f64, f64))> {
        let mut window = RECT::default();
        let mut client = RECT::default();
        unsafe { GetWindowRect(hwnd, &mut window) }.ok()?;
        unsafe { GetClientRect(hwnd, &mut client) }.ok()?;
        Some((
            (
                f64::from(window.right - window.left),
                f64::from(window.bottom - window.top),
            ),
            (
                f64::from(client.right - client.left),
                f64::from(client.bottom - client.top),
            ),
        ))
    }

    /// The window size this drag should settle on, in physical pixels: the
    /// point on the constraint line the gesture is actually reaching for.
    ///
    /// Pure, so the two rules it encodes stay testable without a window: which
    /// axis a gesture makes authoritative, and a floor applied to the video box
    /// rather than to one window axis (clamping an axis directly would take the
    /// result off the line and put the black bars back).
    pub(super) fn fit(
        edge: u32,
        proposed: (f64, f64),
        current: (f64, f64),
        extra: (f64, f64),
        ratio: f64,
        video_floor: f64,
    ) -> (f64, f64) {
        // A side edge is unambiguous: the axis the user grabbed is the axis
        // they mean. On a corner, follow whichever axis the pointer is moving
        // in THIS frame, with the vertical delta scaled into horizontal units
        // so the two are comparable. The two candidates coincide exactly at the
        // switch point, so a diagonal drag hands the lead over without a jump.
        let width_leads = match edge {
            WMSZ_LEFT | WMSZ_RIGHT => true,
            WMSZ_TOP | WMSZ_BOTTOM => false,
            _ => (proposed.0 - current.0).abs() >= (proposed.1 - current.1).abs() * ratio,
        };

        let video_w = if width_leads {
            proposed.0 - extra.0
        } else {
            (proposed.1 - extra.1) * ratio
        }
        .max(video_floor);

        (video_w + extra.0, video_w / ratio + extra.1)
    }

    fn constrain(hwnd: HWND, edge: u32, rect: &mut RECT) {
        let ratio = f64::from_bits(RATIO_BITS.load(Ordering::Relaxed));
        if !ratio.is_finite() || ratio <= 0.0 {
            return;
        }

        let dpi = unsafe { GetDpiForWindow(hwnd) };
        let scale = if dpi == 0 { 1.0 } else { f64::from(dpi) / 96.0 };

        let Some(((cur_w, cur_h), (client_w, client_h))) = measure(hwnd) else {
            return;
        };

        // WM_SIZING carries the WINDOW rect; the constraint is about the CLIENT
        // area. Measure the gap rather than deriving it from the window style:
        // tao's borderless window keeps a real resize frame and trims it back in
        // WM_NCCALCSIZE, so the gap is neither zero nor a decorated frame.
        let frame_w = (cur_w - client_w).max(0.0);
        let frame_h = (cur_h - client_h).max(0.0);

        let extra_w = f64::from(EXTRA_W.load(Ordering::Relaxed)) * scale + frame_w;
        let extra_h = f64::from(EXTRA_H.load(Ordering::Relaxed)) * scale + frame_h;

        let proposed_w = f64::from(rect.right - rect.left);
        let proposed_h = f64::from(rect.bottom - rect.top);

        // Re-apply the window minimum as a floor on the VIDEO box, so both axes
        // stay on the constraint line instead of one being clamped off it.
        let floor = (MIN_LOGICAL_W * scale + frame_w - extra_w)
            .max((MIN_LOGICAL_H * scale + frame_h - extra_h) * ratio)
            .max(MIN_VIDEO_LOGICAL_W * scale);

        let (w, h) = fit(
            edge,
            (proposed_w, proposed_h),
            (cur_w, cur_h),
            (extra_w, extra_h),
            ratio,
            floor,
        );
        let new_w = w.round() as i32;
        let new_h = h.round() as i32;

        // Move only the edges the pointer is dragging; the opposite ones are
        // the anchor, exactly as an unconstrained resize would leave them.
        let drags_left = matches!(edge, WMSZ_LEFT | WMSZ_TOPLEFT | WMSZ_BOTTOMLEFT);
        let drags_top = matches!(edge, WMSZ_TOP | WMSZ_TOPLEFT | WMSZ_TOPRIGHT);

        if drags_left {
            rect.left = rect.right - new_w;
        } else {
            rect.right = rect.left + new_w;
        }
        if drags_top {
            rect.top = rect.bottom - new_h;
        } else {
            rect.bottom = rect.top + new_h;
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::imp::fit;
    use windows::Win32::UI::WindowsAndMessaging::{
        WMSZ_BOTTOM, WMSZ_BOTTOMRIGHT, WMSZ_LEFT, WMSZ_RIGHT,
    };

    const R: f64 = 16.0 / 9.0;
    /// 64px sidebar + 402px chat on the right + its 4px separator, 40px title
    /// bar, no DPI scaling and no window frame: the everyday desktop layout.
    const EXTRA: (f64, f64) = (470.0, 40.0);
    /// A window whose video box is exactly 1280x720, i.e. already on the line -
    /// which is where every frame of a live drag starts from.
    const ON_LINE: (f64, f64) = (1750.0, 760.0);

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    fn video(size: (f64, f64)) -> (f64, f64) {
        (size.0 - EXTRA.0, size.1 - EXTRA.1)
    }

    fn on_ratio(size: (f64, f64)) -> bool {
        let (w, h) = video(size);
        close(w / h, R)
    }

    #[test]
    fn a_side_edge_keeps_the_axis_the_user_grabbed() {
        let out = fit(WMSZ_RIGHT, (1850.0, 760.0), ON_LINE, EXTRA, R, 160.0);
        assert!(close(out.0, 1850.0), "width must be exactly what was asked for");
        assert!(on_ratio(out));
    }

    #[test]
    fn the_bottom_edge_is_no_longer_discarded() {
        // The old formula locked width whenever chat sat on the side, so a
        // bottom-edge drag changed nothing and the window snapped back.
        let out = fit(WMSZ_BOTTOM, (1750.0, 860.0), ON_LINE, EXTRA, R, 160.0);
        assert!(close(out.1, 860.0), "height must be exactly what was asked for");
        assert!(out.0 > ON_LINE.0, "width has to follow height out");
        assert!(on_ratio(out));
    }

    #[test]
    fn a_corner_follows_the_axis_the_pointer_is_moving_in() {
        let mostly_down = fit(WMSZ_BOTTOMRIGHT, (1755.0, 860.0), ON_LINE, EXTRA, R, 160.0);
        assert!(close(mostly_down.1, 860.0));
        let mostly_right = fit(WMSZ_BOTTOMRIGHT, (2050.0, 765.0), ON_LINE, EXTRA, R, 160.0);
        assert!(close(mostly_right.0, 2050.0));
    }

    #[test]
    fn the_corner_candidates_meet_where_the_lead_changes_hands() {
        // Continuity at the switch point is what keeps a diagonal drag from
        // jumping when one axis takes over from the other mid-gesture.
        let dh = 60.0;
        let proposed = (ON_LINE.0 + dh * R, ON_LINE.1 + dh);
        let width_led = fit(WMSZ_BOTTOMRIGHT, proposed, ON_LINE, EXTRA, R, 160.0);
        let height_led = fit(WMSZ_BOTTOM, proposed, ON_LINE, EXTRA, R, 160.0);
        assert!(close(width_led.0, height_led.0) && close(width_led.1, height_led.1));
    }

    #[test]
    fn the_floor_keeps_both_axes_on_the_constraint_line() {
        // Dragging far past the minimum: the result is still a legal 16:9 box,
        // not a window with one axis clamped and bars back on the other.
        let out = fit(WMSZ_LEFT, (300.0, 640.0), (900.0, 640.0), EXTRA, R, 400.0);
        assert!(close(video(out).0, 400.0));
        assert!(on_ratio(out));
    }
}
