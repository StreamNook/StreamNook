//! Pinned mentions (SN-284), for every app's chat. The rules and the state live
//! in `services::mention_inbox` (what a pin is) and `services::mention_pins`
//! (where it shows and when its clock runs); these only hand them to a page or
//! the shell. Every command is async: none runs on a window's main thread.

use crate::services::mention_pins::{self, MentionPins, SurfaceIn};

/// The most surfaces one window reports, and the most visible ids one reports.
const SURFACES_MAX: usize = 32;
const VISIBLE_MAX: usize = 64;

/// A window's mounted pin stacks, in one call (SN-284). `page` is the document
/// (the page's `performance.timeOrigin`, or a UUID per iPhone process), `seq`
/// orders this page's calls (the page-start call is seq 0 with no surfaces);
/// a call older than the last applied one, or from a retired page, is dropped.
/// Ignored while Pin mentions is off.
#[tauri::command]
pub async fn set_pin_surfaces(window: crate::rt::Window, page: String, seq: u64, surfaces: Vec<SurfaceIn>) -> Result<bool, String> {
    let surfaces: Vec<SurfaceIn> = surfaces
        .into_iter()
        .take(SURFACES_MAX)
        .map(|mut s| {
            s.visible.truncate(VISIBLE_MAX);
            s
        })
        .collect();
    Ok(mention_pins::set_surfaces(window.label(), &page, seq, surfaces))
}

/// The pins one surface draws, from the answer Rust keeps current: oldest
/// first, each with its row, its channel mark when it is not this surface's
/// chat, and its clock. Never writes or emits. `rooms` is v1's argument: a v1
/// page asking by rooms is answered with no pins.
#[tauri::command]
pub async fn get_mention_pins(
    window: crate::rt::Window,
    page: Option<String>,
    surface: Option<String>,
    rooms: Option<Vec<String>>,
) -> Result<MentionPins, String> {
    let _ = rooms;
    Ok(mention_pins::answer_for(window.label(), page.as_deref(), surface.as_deref()))
}

/// The viewer used this window (pointer, key, wheel, touch, composer edits):
/// the pins' clocks count only after input like this. The shell throttles to
/// one per second and sends no time; Rust stamps the receipt. The emote
/// palette's input counts for the window it serves. On desktop it counts only
/// while the window is the OS foreground one.
#[tauri::command]
pub async fn note_viewer_input(window: crate::rt::Window) -> Result<(), String> {
    // Whether this window is the OS foreground one; the palette itself is, and
    // its input is credited to the window it serves.
    let front = mention_pins::window_has_os_focus(&window);
    #[cfg(desktop)]
    let label = if window.label() == crate::commands::popout_window::EMOTE_PALETTE_LABEL {
        crate::commands::emote_palette::served_window().unwrap_or_else(|| window.label().to_string())
    } else {
        window.label().to_string()
    };
    // The iPhone's FFI invokes carry the window label `main`.
    #[cfg(not(desktop))]
    let label = window.label().to_string();
    mention_pins::note_viewer_input(&label, front);
    Ok(())
}

/// Clear all, or the X on one pin: these pins (each with every copy of the
/// same message) go everywhere, saved at once. Answers how many ids it took.
#[tauri::command]
pub async fn dismiss_mention_pins(ids: Vec<String>) -> Result<bool, String> {
    let ids: Vec<String> = ids.into_iter().filter(|i| !i.is_empty() && i.len() <= 128).take(64).collect();
    if ids.is_empty() {
        return Ok(false);
    }
    let changed = crate::services::mention_inbox::dismiss_pins(&ids);
    if changed {
        let _ = tokio::task::spawn_blocking(crate::services::mention_inbox::flush_now).await;
        mention_pins::on_pins_ended();
    }
    Ok(changed)
}
