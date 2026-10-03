//! Opening the chat popouts, MultiChat windows and the floating chat overlay,
//! and remembering where each one was.
//!
//! Rust builds both kinds and places them in physical pixels from start to
//! finish. The page used to build them itself, handing the window API a spot
//! saved in physical pixels, which the API reads as logical ones: on a scaled
//! display every popout opened further right and larger than it was left, and
//! a new overlay could land as a sliver on the screen's edge.
//!
//! Their geometry lives here rather than in the window-state plugin (lib.rs
//! filters these labels out of it). The plugin restores a saved rect when any
//! one corner is on a monitor, and a window it has never seen can only be
//! given a logical spot, which lands on the wrong monitor when monitors are
//! scaled differently. Here every rect is physical, a window with nothing
//! saved opens beside the one that asked for it, and every open is fitted
//! wholly inside one monitor's work area while the window is still hidden.
//!
//! The overlay's click-through lives here too. A window that ignores the mouse
//! can never be clicked to turn itself back, so its control buttons stay live:
//! while click-through is on, a poll watches the cursor and lets the overlay
//! take the mouse only while the cursor is over those buttons. Everywhere else
//! clicks fall through to the app underneath.

use crate::rt::{AppHandle, WebviewWindow, WebviewWindowBuilder, Window};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WindowEvent};

/// Chat overlays open at once. Each is its own renderer (about 150 MB).
const MAX_CHAT_OVERLAYS: usize = 4;
/// Saved rects kept. Past this the least recently opened is dropped, so a
/// viewer who floats many channels over time does not grow the file forever.
const MAX_SAVED: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    ChatOverlay,
    MultiChat,
    Plugin,
}

impl Kind {
    /// Logical size with nothing saved. The MultiChat width is
    /// `MULTICHAT_BASE_WIDTH` in utils/multichatWindow.ts.
    fn default_size(self) -> (f64, f64) {
        match self {
            Kind::ChatOverlay => (380.0, 520.0),
            Kind::MultiChat => (402.0, 620.0),
            Kind::Plugin => (420.0, 560.0),
        }
    }

    /// Logical distance each further window of this kind steps off the last.
    fn step(self) -> f64 {
        match self {
            Kind::ChatOverlay => 32.0,
            Kind::MultiChat | Kind::Plugin => 36.0,
        }
    }

    fn prefix(self) -> &'static str {
        match self {
            Kind::ChatOverlay => "overlay-",
            Kind::MultiChat => "multichat-",
            Kind::Plugin => "plugin-",
        }
    }
}

/// Whether this module places and remembers the window with this label.
pub fn is_placed_label(label: &str) -> bool {
    [Kind::ChatOverlay, Kind::MultiChat, Kind::Plugin]
        .iter()
        .any(|k| label.starts_with(k.prefix()))
}

/// What an open request did.
#[derive(Serialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PopoutOutcome {
    Opened,
    /// Already open, and brought forward.
    Focused,
    /// Every overlay slot is taken.
    LimitReached,
}

/// One channel seeded into a new MultiChat window, in the page's key names.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PopoutChannel {
    pub channel: String,
    #[serde(default)]
    pub channel_id: Option<String>,
    #[serde(default)]
    pub channel_name: Option<String>,
}

/// A popout's outer position and inner size in physical pixels, the way the
/// window events report them.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
struct Saved {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    #[serde(default)]
    maximized: bool,
    /// Unix seconds of the last open: eviction order, and which window of a
    /// kind the viewer placed last.
    #[serde(default)]
    used: u64,
}

static SAVED: Lazy<Mutex<HashMap<String, Saved>>> = Lazy::new(|| Mutex::new(load()));
static DIRTY: AtomicBool = AtomicBool::new(false);
static FLUSH_TASK: AtomicBool = AtomicBool::new(false);

/// Float one channel's chat over other apps. The overlay's label is the
/// channel, so a second request for it brings the open one forward.
#[tauri::command]
pub async fn open_chat_overlay(
    app: AppHandle,
    window: Window,
    channel: String,
    channel_id: Option<String>,
    channel_name: Option<String>,
) -> Result<PopoutOutcome, String> {
    let login = channel.trim().trim_start_matches('@').to_ascii_lowercase();
    if !is_twitch_login(&login) {
        return Err(format!("not a Twitch channel name: {channel}"));
    }
    let label = format!("overlay-{login}");
    if let Some(open) = app.get_webview_window(&label) {
        bring_forward(&open);
        return Ok(PopoutOutcome::Focused);
    }
    let open_overlays = count_open(&app, Kind::ChatOverlay);
    if open_overlays >= MAX_CHAT_OVERLAYS {
        return Ok(PopoutOutcome::LimitReached);
    }

    let name = channel_name
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(&login)
        .to_string();
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    query.append_pair("channel", &login);
    if let Some(id) = channel_id.as_deref().filter(|s| !s.is_empty()) {
        query.append_pair("channelId", id);
    }
    query.append_pair("channelName", &name);

    let placement = placement(
        &app,
        &window,
        &label,
        Kind::ChatOverlay,
        open_overlays as u32,
        Kind::ChatOverlay.default_size(),
    );
    let url = app_route(&app, &format!("/chat-overlay?{}", query.finish()));
    let builder = WebviewWindowBuilder::new(&app, &label, url)
        .title(format!("StreamNook chat overlay: {name}"))
        .decorations(false)
        .resizable(true)
        .always_on_top(true)
        .skip_taskbar(true)
        // Windows draws a 1px frame line as part of the window shadow, even on
        // an undecorated window; the overlay's glass paints its own hairline.
        .shadow(false)
        .minimizable(false)
        .maximizable(false)
        .disable_drag_drop_handler();
    // Linux cannot paint a transparent window, so the overlay is opaque there
    // and its slider fades the whole window. macOS would need the private API.
    #[cfg(target_os = "linux")]
    let builder = builder.background_color(tauri::window::Color(0x0c, 0x0c, 0x0d, 0xff));
    #[cfg(windows)]
    let builder = builder.transparent(true);

    let (w, h) = placement.initial;
    let win = builder
        .inner_size(w, h)
        // Hidden until `settle` has put it where it belongs.
        .visible(false)
        .build()
        .map_err(|e| e.to_string())?;
    settle(win, placement);
    Ok(PopoutOutcome::Opened)
}

/// Open a MultiChat window. The page picks the id because each window's tabs
/// live in the page's storage under it.
#[tauri::command]
pub async fn open_multichat_window(
    app: AppHandle,
    window: Window,
    id: String,
    channels: Vec<PopoutChannel>,
    replace: bool,
    title: String,
) -> Result<PopoutOutcome, String> {
    if !is_multichat_id(&id) {
        return Err(format!("not a MultiChat window id: {id}"));
    }
    let label = format!("multichat-{id}");
    if let Some(open) = app.get_webview_window(&label) {
        bring_forward(&open);
        return Ok(PopoutOutcome::Focused);
    }

    let mut query = url::form_urlencoded::Serializer::new(String::new());
    query.append_pair("id", &id);
    if replace {
        query.append_pair("replace", "1");
    }
    match channels.as_slice() {
        [] => {}
        [one] => {
            query.append_pair("channel", &one.channel);
            if let Some(cid) = one.channel_id.as_deref().filter(|s| !s.is_empty()) {
                query.append_pair("channelId", cid);
            }
            if let Some(name) = one.channel_name.as_deref().filter(|s| !s.is_empty()) {
                query.append_pair("channelName", name);
            }
        }
        // Several channels ride one JSON param, so a new window never races a
        // listener that is not up yet.
        many => {
            let json = serde_json::to_string(many).map_err(|e| e.to_string())?;
            query.append_pair("channels", &json);
        }
    }

    let index = if id == "default" { 0 } else { count_open(&app, Kind::MultiChat) as u32 };
    let placement = placement(&app, &window, &label, Kind::MultiChat, index, Kind::MultiChat.default_size());
    let url = app_route(&app, &format!("/multichat?{}", query.finish()));
    let (w, h) = placement.initial;
    let win = WebviewWindowBuilder::new(&app, &label, url)
        .title(title)
        .decorations(false)
        .resizable(true)
        .minimizable(true)
        .maximizable(true)
        // The OS drag-and-drop handler swallows HTML5 dragstart, which is how
        // the tab strip reorders.
        .disable_drag_drop_handler()
        .inner_size(w, h)
        .visible(false)
        .build()
        .map_err(|e| e.to_string())?;
    settle(win, placement);
    Ok(PopoutOutcome::Opened)
}

/// A ui plugin's request for a window of its own, in the page's key names.
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PluginWindowRequest {
    pub plugin_id: String,
    /// Names the window's content; the plugin's module renders it.
    pub surface: String,
    pub title: String,
    #[serde(default)]
    pub width: Option<f64>,
    #[serde(default)]
    pub height: Option<f64>,
    #[serde(default)]
    pub min_width: Option<f64>,
    #[serde(default)]
    pub min_height: Option<f64>,
}

/// Open a ui plugin's window, routed to `#/plugin/<id>/<surface>` where the
/// host draws the chrome and mounts the plugin's component. One window per
/// plugin and surface: a second request brings the open one forward.
#[tauri::command]
pub async fn open_plugin_window(
    app: AppHandle,
    window: Window,
    request: PluginWindowRequest,
) -> Result<PopoutOutcome, String> {
    let label = plugin_window_label(&request.plugin_id, &request.surface);
    if let Some(open) = app.get_webview_window(&label) {
        bring_forward(&open);
        return Ok(PopoutOutcome::Focused);
    }

    let mut query = url::form_urlencoded::Serializer::new(String::new());
    query.append_pair("title", &request.title);
    let route = format!(
        "/plugin/{}/{}?{}",
        urlencoding::encode(&request.plugin_id),
        urlencoding::encode(&request.surface),
        query.finish()
    );
    let (dw, dh) = Kind::Plugin.default_size();
    let size = (request.width.unwrap_or(dw), request.height.unwrap_or(dh));
    let index = count_open(&app, Kind::Plugin) as u32;
    let placement = placement(&app, &window, &label, Kind::Plugin, index, size);
    let (w, h) = placement.initial;
    let win = WebviewWindowBuilder::new(&app, &label, app_route(&app, &route))
        .title(&request.title)
        .decorations(false)
        .resizable(true)
        .minimizable(true)
        .maximizable(false)
        .min_inner_size(request.min_width.unwrap_or(280.0), request.min_height.unwrap_or(360.0))
        .inner_size(w, h)
        .visible(false)
        .build()
        .map_err(|e| e.to_string())?;
    settle(win, placement);
    Ok(PopoutOutcome::Opened)
}

/// `plugin-<id>-<surface>`, with everything a window label cannot hold
/// (plugin ids carry dots) turned into dashes.
fn plugin_window_label(plugin_id: &str, surface: &str) -> String {
    let clean = |s: &str| -> String {
        s.chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
            .collect()
    };
    format!("plugin-{}-{}", clean(plugin_id), clean(surface))
}

/// Widen the calling window to `width` logical pixels, never past its
/// monitor's work area less a margin, then keep it on screen. Grows only: a
/// window the viewer narrowed, or a maximized one, is left alone. MultiChat
/// sizes itself to its columns this way.
#[tauri::command]
pub async fn grow_popout_width(window: Window, width: f64) -> Result<(), String> {
    grow_width(&window, width).map_err(|e| e.to_string())
}

/// Keep the calling window wholly inside the work area it overlaps most.
/// MultiChat asks after the viewer moves it, so a window dragged onto a smaller
/// monitor does not leave its composer off the edge.
#[tauri::command]
pub async fn fit_popout_on_screen(window: Window) -> Result<(), String> {
    fit_to_screen(&window, None).map_err(|e| e.to_string())
}

/// Keep a popout's saved rect current as the viewer moves and resizes it.
/// Called from the window event handler in lib.rs for popout labels.
pub fn note_geometry(window: &Window, event: &WindowEvent) {
    let (moved, resized) = match event {
        WindowEvent::Moved(p) => (Some(*p), None),
        WindowEvent::Resized(s) => (None, Some(*s)),
        _ => return,
    };
    // A minimized window reports a far-off parking spot, not a place to reopen.
    if window.is_minimized().unwrap_or(true) {
        return;
    }
    let maximized = window.is_maximized().unwrap_or(false);
    let Ok(mut map) = SAVED.lock() else { return };
    // Only windows opened through here have an entry; `settle` writes it.
    let Some(entry) = map.get_mut(window.label()) else { return };
    entry.maximized = maximized;
    if !maximized {
        if let Some(p) = moved {
            entry.x = p.x;
            entry.y = p.y;
        }
        if let Some(s) = resized.filter(|s| s.width > 0 && s.height > 0) {
            entry.width = s.width;
            entry.height = s.height;
        }
    }
    drop(map);
    mark_dirty();
}

/// Write the saved rects if anything changed. The debounced task and the exit
/// path both call this.
pub fn flush_now() -> Result<(), String> {
    if !DIRTY.swap(false, Ordering::AcqRel) {
        return Ok(());
    }
    let json = {
        let map = SAVED.lock().map_err(|e| e.to_string())?;
        serde_json::to_string(&*map).map_err(|e| e.to_string())?
    };
    let result = store_path()
        .ok_or_else(|| "no app data dir".to_string())
        .and_then(|path| {
            let tmp = path.with_extension("json.tmp");
            std::fs::write(&tmp, json)
                .and_then(|_| std::fs::rename(&tmp, &path))
                .map_err(|e| e.to_string())
        });
    if result.is_err() {
        DIRTY.store(true, Ordering::Release);
    }
    result
}

fn mark_dirty() {
    DIRTY.store(true, Ordering::Release);
    if FLUSH_TASK
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
    {
        tauri::async_runtime::spawn(async {
            loop {
                tokio::time::sleep(Duration::from_secs(2)).await;
                if !DIRTY.load(Ordering::Acquire) {
                    continue;
                }
                if let Ok(Err(e)) = tauri::async_runtime::spawn_blocking(flush_now).await {
                    log::debug!("[Popout] geometry flush failed (will retry): {e}");
                }
            }
        });
    }
}

fn store_path() -> Option<PathBuf> {
    crate::services::cache_service::get_app_data_dir()
        .ok()
        .map(|d| d.join("popout_geometry.json"))
}

fn load() -> HashMap<String, Saved> {
    store_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn remember(label: &str, saved: Saved) {
    if let Ok(mut map) = SAVED.lock() {
        map.insert(label.to_string(), saved);
        evict(&mut map);
    }
    mark_dirty();
}

fn evict(map: &mut HashMap<String, Saved>) {
    while map.len() > MAX_SAVED {
        let Some(oldest) = map.iter().min_by_key(|(_, s)| s.used).map(|(k, _)| k.clone()) else {
            break;
        };
        map.remove(&oldest);
    }
}

fn count_open(app: &AppHandle, kind: Kind) -> usize {
    app.webview_windows()
        .keys()
        .filter(|l| l.starts_with(kind.prefix()))
        .count()
}

/// Everything the main thread needs to place a new popout, worked out before
/// it is built.
struct Placement {
    saved: Option<Saved>,
    /// Position and inner size for a window with nothing saved.
    fresh: Rect,
    /// Work area of the monitor the request came from, for a rect on no
    /// monitor at all.
    home: Rect,
    /// Logical size to build at: the intended size in the primary monitor's
    /// scale, which is the one a window built without a position is sized in,
    /// so the resize in `settle` is at most a rescale.
    initial: (f64, f64),
}

fn placement(
    app: &AppHandle,
    from: &Window,
    label: &str,
    kind: Kind,
    index: u32,
    size: (f64, f64),
) -> Placement {
    // Overlay labels changed form when they moved here; the other two kept
    // theirs, so their plugin-saved rects still apply.
    let saved = saved_rect(label).or_else(|| match kind {
        Kind::MultiChat | Kind::Plugin => from_window_state_file(app, label),
        Kind::ChatOverlay => None,
    });
    let scale = from.scale_factor().unwrap_or(1.0);
    let home = from
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| from.primary_monitor().ok().flatten())
        .map(|m| Rect::of_work_area(&m))
        .unwrap_or(Rect { x: 0, y: 0, w: 1280, h: 720 });
    // A minimized or tray-hidden window reports a far-off parking spot.
    let parked = from.is_minimized().unwrap_or(false) || !from.is_visible().unwrap_or(true);
    let caller = if parked {
        None
    } else {
        match (from.outer_position(), from.outer_size()) {
            (Ok(p), Ok(s)) => Some(Rect { x: p.x, y: p.y, w: s.width, h: s.height }),
            _ => None,
        }
    };
    let fresh = fresh_rect(kind, size, caller, scale, index, latest_of_kind(kind, label), home);
    let primary_scale = from
        .primary_monitor()
        .ok()
        .flatten()
        .map(|m| m.scale_factor())
        .unwrap_or(scale);
    let (pw, ph) = saved.map(|s| (s.width, s.height)).unwrap_or((fresh.w, fresh.h));
    Placement {
        saved,
        fresh,
        home,
        initial: (f64::from(pw) / primary_scale, f64::from(ph) / primary_scale),
    }
}

fn saved_rect(label: &str) -> Option<Saved> {
    SAVED.lock().ok()?.get(label).copied()
}

/// The saved rect of the window of this kind the viewer opened most recently,
/// other than `label` itself.
fn latest_of_kind(kind: Kind, label: &str) -> Option<Saved> {
    let map = SAVED.lock().ok()?;
    map.iter()
        .filter(|(k, _)| k.starts_with(kind.prefix()) && k.as_str() != label)
        .max_by_key(|(_, s)| s.used)
        .map(|(_, s)| *s)
}

/// Where a popout with nothing saved opens: stepped off the last one of its
/// kind the viewer placed; else an overlay just inside the right edge of the
/// window that asked and any other window just outside it; else centred on
/// that window's monitor. `size` is the logical size to open at.
fn fresh_rect(
    kind: Kind,
    size: (f64, f64),
    caller: Option<Rect>,
    scale: f64,
    index: u32,
    anchor: Option<Saved>,
    home: Rect,
) -> Rect {
    let px = |v: f64| (v * scale).round() as i32;
    let step = px(kind.step() * f64::from(index));
    if let Some(a) = anchor {
        return Rect { x: a.x + step, y: a.y + step, w: a.width, h: a.height };
    }
    let (w, h) = (px(size.0), px(size.1));
    let (x, y) = match caller {
        Some(c) => match kind {
            Kind::ChatOverlay => (c.x + c.w as i32 - w - px(24.0), c.y + px(64.0)),
            Kind::MultiChat | Kind::Plugin => (c.x + c.w as i32 + px(10.0), c.y),
        },
        None => (home.x + (home.w as i32 - w) / 2, home.y + (home.h as i32 - h) / 2),
    };
    Rect { x: x + step, y: y + step, w: w as u32, h: h as u32 }
}

/// A window's last rect as the window-state plugin saved it, before popouts
/// were placed here, so an existing window reopens where it was.
fn from_window_state_file(app: &AppHandle, label: &str) -> Option<Saved> {
    let path = app
        .path()
        .app_config_dir()
        .ok()?
        .join(tauri_plugin_window_state::DEFAULT_FILENAME);
    let all: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    plugin_entry(all.get(label)?)
}

fn plugin_entry(v: &serde_json::Value) -> Option<Saved> {
    let int = |k: &str| v.get(k)?.as_i64();
    let maximized = v.get("maximized").and_then(|m| m.as_bool()).unwrap_or(false);
    // A maximized window's own x/y is the monitor corner; prev_* is the
    // restored spot.
    let (x, y) = if maximized {
        (int("prev_x")?, int("prev_y")?)
    } else {
        (int("x")?, int("y")?)
    };
    let (w, h) = (int("width")?, int("height")?);
    if w <= 0 || h <= 0 {
        return None;
    }
    Some(Saved { x: x as i32, y: y as i32, width: w as u32, height: h as u32, maximized, used: 0 })
}

/// Put a new popout in place and show it. Runs on the main thread, where the
/// window calls below are handled inline.
fn settle(win: WebviewWindow, placement: Placement) {
    let target = win.clone();
    let queued = win.run_on_main_thread(move || {
        let maximize = match apply(&target.as_ref().window(), &placement) {
            Ok(m) => m,
            Err(e) => {
                log::debug!("[Popout] could not place {}: {e}", target.label());
                false
            }
        };
        let _ = target.show();
        if maximize {
            let _ = target.maximize();
        }
        let _ = target.set_focus();
    });
    if queued.is_err() {
        bring_forward(&win);
    }
}

fn apply(window: &Window, placement: &Placement) -> tauri::Result<bool> {
    let outer = window.outer_size()?;
    let inner = window.inner_size()?;
    let frame_w = outer.width.saturating_sub(inner.width);
    let frame_h = outer.height.saturating_sub(inner.height);
    // Saved and fresh sizes are both inner sizes (a saved one comes from the
    // Resized event; a fresh one is the size the page asked for). An
    // undecorated window with a shadow still has an invisible frame around
    // that, so the outer rect is the inner size plus the frame.
    let want = match placement.saved {
        Some(s) => Rect { x: s.x, y: s.y, w: s.width + frame_w, h: s.height + frame_h },
        None => Rect {
            w: placement.fresh.w + frame_w,
            h: placement.fresh.h + frame_h,
            ..placement.fresh
        },
    };
    let fitted = fit_rect(want, &work_areas(window)?, placement.home);
    // Position first: moving onto a monitor with another scale makes Windows
    // rescale the window, so the size has to be set after the move.
    window.set_position(PhysicalPosition::new(fitted.x, fitted.y))?;
    let (w, h) = (fitted.w.saturating_sub(frame_w), fitted.h.saturating_sub(frame_h));
    window.set_size(PhysicalSize::new(w, h))?;
    let maximized = placement.saved.is_some_and(|s| s.maximized);
    remember(
        window.label(),
        Saved { x: fitted.x, y: fitted.y, width: w, height: h, maximized, used: now() },
    );
    if fitted != want {
        log::debug!("[Popout] {} kept on screen: {want:?} -> {fitted:?}", window.label());
    }
    Ok(maximized)
}

fn grow_width(window: &Window, width: f64) -> tauri::Result<()> {
    if window.is_maximized()? {
        return Ok(());
    }
    let scale = window.scale_factor()?;
    let inner = window.inner_size()?;
    let outer = window.outer_size()?;
    let mut target = (width * scale).round() as u32;
    if let Some(monitor) = window.current_monitor()? {
        // A margin, so the window never sits flush against the screen edge.
        let room = monitor
            .work_area()
            .size
            .width
            .saturating_sub((40.0 * scale).round() as u32);
        let floor = (Kind::MultiChat.default_size().0 * scale).round() as u32;
        target = target.min(room.max(floor));
    }
    if inner.width + (2.0 * scale).round() as u32 >= target {
        return Ok(());
    }
    let frame_w = outer.width.saturating_sub(inner.width);
    fit_to_screen(window, Some(target + frame_w))
}

/// Move the window, and shrink it if it is bigger, so all of it sits inside
/// the work area it overlaps most, optionally at a new outer width first.
/// Leaves a maximized window alone, and makes no call when nothing changes,
/// so a caller that runs this after every move does not loop.
fn fit_to_screen(window: &Window, outer_width: Option<u32>) -> tauri::Result<()> {
    if window.is_maximized()? || window.is_minimized()? {
        return Ok(());
    }
    let pos = window.outer_position()?;
    let outer = window.outer_size()?;
    let inner = window.inner_size()?;
    let now = Rect { x: pos.x, y: pos.y, w: outer.width, h: outer.height };
    let want = Rect { w: outer_width.unwrap_or(outer.width), ..now };
    let home = window
        .current_monitor()?
        .or(window.primary_monitor()?)
        .map(|m| Rect::of_work_area(&m))
        .unwrap_or(want);
    let fitted = fit_rect(want, &work_areas(window)?, home);
    if fitted == now {
        return Ok(());
    }
    let frame_w = outer.width.saturating_sub(inner.width);
    let frame_h = outer.height.saturating_sub(inner.height);
    // Position first: moving onto a monitor with another scale makes Windows
    // rescale the window, so the size has to be set after the move.
    window.set_position(PhysicalPosition::new(fitted.x, fitted.y))?;
    window.set_size(PhysicalSize::new(
        fitted.w.saturating_sub(frame_w),
        fitted.h.saturating_sub(frame_h),
    ))?;
    Ok(())
}

fn work_areas(window: &Window) -> tauri::Result<Vec<Rect>> {
    Ok(window.available_monitors()?.iter().map(Rect::of_work_area).collect())
}

fn bring_forward(win: &WebviewWindow) {
    let _ = win.unminimize();
    let _ = win.show();
    let _ = win.set_focus();
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rect {
    x: i32,
    y: i32,
    w: u32,
    h: u32,
}

impl Rect {
    fn of_work_area(m: &tauri::window::Monitor) -> Rect {
        let a = m.work_area();
        Rect { x: a.position.x, y: a.position.y, w: a.size.width, h: a.size.height }
    }

    fn overlap(&self, o: &Rect) -> i64 {
        let w = (self.x as i64 + self.w as i64).min(o.x as i64 + o.w as i64)
            - (self.x as i64).max(o.x as i64);
        let h = (self.y as i64 + self.h as i64).min(o.y as i64 + o.h as i64)
            - (self.y as i64).max(o.y as i64);
        if w <= 0 || h <= 0 {
            0
        } else {
            w * h
        }
    }
}

/// `rect` moved, and shrunk if it is bigger, so it sits wholly inside the work
/// area it overlaps most, or inside `home` when it overlaps none (a window
/// saved on a monitor that has since been unplugged).
fn fit_rect(rect: Rect, works: &[Rect], home: Rect) -> Rect {
    let area = works
        .iter()
        .copied()
        .filter(|w| rect.overlap(w) > 0)
        .max_by_key(|w| rect.overlap(w))
        .unwrap_or(home);
    let w = rect.w.min(area.w);
    let h = rect.h.min(area.h);
    Rect {
        x: rect.x.clamp(area.x, area.x + (area.w - w) as i32),
        y: rect.y.clamp(area.y, area.y + (area.h - h) as i32),
        w,
        h,
    }
}

/// This app's page at a hash route. A debug build points at the dev server,
/// since a window made at runtime is not sent there on its own (main is
/// recreated the same way); a release build loads the bundled page.
fn app_route(app: &AppHandle, route: &str) -> WebviewUrl {
    if cfg!(debug_assertions) {
        if let Some(mut dev) = app.config().build.dev_url.clone() {
            dev.set_fragment(Some(route));
            return WebviewUrl::External(dev);
        }
    }
    WebviewUrl::App(format!("index.html#{route}").into())
}

/// A Twitch login: lowercase letters, digits and underscores, 25 at most.
fn is_twitch_login(s: &str) -> bool {
    (1..=25).contains(&s.len())
        && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// `default`, or `w2` to `w9` for the extra windows.
fn is_multichat_id(id: &str) -> bool {
    id == "default"
        || matches!(id.strip_prefix('w').and_then(|n| n.parse::<u8>().ok()), Some(2..=9))
}

// ---------------------------------------------------------------------------
// Overlay click-through
// ---------------------------------------------------------------------------

/// How often the cursor is checked while any overlay is click-through.
const CLICK_THROUGH_POLL: Duration = Duration::from_millis(60);
/// Sent to one overlay whenever its click-through state changes.
const CLICK_THROUGH_EVENT: &str = "chat-overlay-click-through";
/// Logical pixels of slack around the buttons, so they are easy to land on.
const CONTROLS_SLACK: f64 = 6.0;

/// The overlay's control buttons, in logical pixels, measured from the
/// window's top right corner so a resize does not move them.
#[derive(Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ControlsZone {
    right: f64,
    top: f64,
    width: f64,
    height: f64,
}

struct ClickThrough {
    controls: ControlsZone,
    /// The cursor is over the buttons and the window is taking the mouse.
    over_controls: bool,
}

#[derive(Serialize, Clone, Copy)]
#[serde(rename_all = "camelCase")]
struct ClickThroughState {
    on: bool,
    over_controls: bool,
}

/// Overlays with click-through on, by label. The window setters below run
/// while this is held, so the poll can never re-ignore a window that was just
/// turned back. They only queue a message, and nothing that waits on the main
/// thread runs under it, so the tray may take it on the main thread.
static CLICK_THROUGH: Lazy<Mutex<HashMap<String, ClickThrough>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
static CLICK_THROUGH_POLLING: AtomicBool = AtomicBool::new(false);

/// Turn the calling overlay's click-through on or off. On needs the place of
/// its control buttons, which stay clickable.
#[tauri::command]
pub async fn set_chat_overlay_click_through(
    window: Window,
    on: bool,
    controls: Option<ControlsZone>,
) -> Result<(), String> {
    let app = window.app_handle().clone();
    let label = window.label().to_string();
    if !label.starts_with(Kind::ChatOverlay.prefix()) {
        return Err(format!("not a chat overlay: {label}"));
    }
    if on {
        let controls = controls.ok_or("click-through needs the controls' place")?;
        let mut map = CLICK_THROUGH.lock().map_err(|e| e.to_string())?;
        window.set_ignore_cursor_events(true).map_err(|e| e.to_string())?;
        map.insert(label.clone(), ClickThrough { controls, over_controls: false });
        drop(map);
        start_click_through_poll(&app);
    } else {
        let mut map = CLICK_THROUGH.lock().map_err(|e| e.to_string())?;
        map.remove(&label);
        window.set_ignore_cursor_events(false).map_err(|e| e.to_string())?;
    }
    send_click_through_state(&app, &label, ClickThroughState { on, over_controls: false });
    Ok(())
}

/// Make every overlay clickable again. The tray's way back.
pub fn make_chat_overlays_clickable(app: &AppHandle) {
    let labels: Vec<String> = match CLICK_THROUGH.lock() {
        Ok(mut map) => {
            let labels: Vec<String> = map.keys().cloned().collect();
            for label in &labels {
                if let Some(win) = app.get_webview_window(label) {
                    let _ = win.set_ignore_cursor_events(false);
                }
            }
            map.clear();
            labels
        }
        Err(_) => return,
    };
    for label in labels {
        send_click_through_state(app, &label, ClickThroughState { on: false, over_controls: false });
    }
}

fn send_click_through_state(app: &AppHandle, label: &str, state: ClickThroughState) {
    let _ = app.emit_to(label, CLICK_THROUGH_EVENT, state);
}

fn start_click_through_poll(app: &AppHandle) {
    if CLICK_THROUGH_POLLING.swap(true, Ordering::AcqRel) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(CLICK_THROUGH_POLL).await;
            let watched: Vec<(String, ControlsZone)> = match CLICK_THROUGH.lock() {
                Ok(map) => map.iter().map(|(l, c)| (l.clone(), c.controls)).collect(),
                Err(_) => break,
            };
            if watched.is_empty() {
                CLICK_THROUGH_POLLING.store(false, Ordering::Release);
                // An overlay turned on between the check and the store would
                // otherwise go unwatched.
                let again = CLICK_THROUGH.lock().map(|m| !m.is_empty()).unwrap_or(false);
                if again && !CLICK_THROUGH_POLLING.swap(true, Ordering::AcqRel) {
                    continue;
                }
                break;
            }
            // The cursor and window geometry wait on the main thread, so they
            // are read before the map is locked.
            let Ok(cursor) = app.cursor_position() else { continue };
            for (label, controls) in watched {
                let Some(win) = app.get_webview_window(&label) else {
                    if let Ok(mut map) = CLICK_THROUGH.lock() {
                        map.remove(&label);
                    }
                    continue;
                };
                let over = match (win.inner_position(), win.inner_size(), win.scale_factor()) {
                    (Ok(pos), Ok(size), Ok(scale)) => {
                        let r = controls_rect(pos.x, pos.y, size.width, controls, scale);
                        r.contains(cursor.x, cursor.y)
                    }
                    _ => continue,
                };
                let Ok(mut map) = CLICK_THROUGH.lock() else { break };
                let Some(entry) = map.get_mut(&label) else { continue };
                if entry.over_controls == over {
                    continue;
                }
                if win.set_ignore_cursor_events(!over).is_ok() {
                    entry.over_controls = over;
                    drop(map);
                    send_click_through_state(&app, &label, ClickThroughState { on: true, over_controls: over });
                }
            }
        }
    });
}

/// The physical rect of an overlay's control buttons, with slack.
#[derive(Clone, Copy, Debug, PartialEq)]
struct FRect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

impl FRect {
    fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

fn controls_rect(win_x: i32, win_y: i32, win_w: u32, c: ControlsZone, scale: f64) -> FRect {
    let right_edge = f64::from(win_x) + f64::from(win_w);
    let x = right_edge - (c.right + c.width + CONTROLS_SLACK) * scale;
    let y = f64::from(win_y) + (c.top - CONTROLS_SLACK) * scale;
    FRect {
        x,
        y,
        w: (c.width + 2.0 * CONTROLS_SLACK) * scale,
        h: (c.height + 2.0 * CONTROLS_SLACK) * scale,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Rect = Rect { x: 0, y: 0, w: 1920, h: 1040 };
    const RIGHT: Rect = Rect { x: 1920, y: 0, w: 2560, h: 1400 };

    fn saved(x: i32, y: i32, used: u64) -> Saved {
        Saved { x, y, width: 400, height: 500, maximized: false, used }
    }

    #[test]
    fn a_window_hanging_off_the_right_edge_moves_fully_on() {
        let r = Rect { x: 1895, y: 80, w: 475, h: 650 };
        assert_eq!(fit_rect(r, &[SCREEN], SCREEN), Rect { x: 1445, y: 80, w: 475, h: 650 });
    }

    #[test]
    fn a_window_bigger_than_the_work_area_shrinks_to_it() {
        let r = Rect { x: -50, y: -50, w: 3000, h: 2000 };
        assert_eq!(fit_rect(r, &[SCREEN], SCREEN), SCREEN);
    }

    #[test]
    fn a_window_on_the_second_monitor_stays_put() {
        let r = Rect { x: 2200, y: 100, w: 400, h: 600 };
        assert_eq!(fit_rect(r, &[SCREEN, RIGHT], SCREEN), r);
    }

    #[test]
    fn a_window_across_two_monitors_lands_on_the_one_it_covers_most() {
        let r = Rect { x: 1800, y: 100, w: 400, h: 600 };
        assert_eq!(fit_rect(r, &[SCREEN, RIGHT], SCREEN), Rect { x: 1920, y: 100, w: 400, h: 600 });
    }

    #[test]
    fn a_window_saved_on_an_unplugged_monitor_comes_home() {
        let r = Rect { x: -2000, y: 100, w: 400, h: 600 };
        assert_eq!(fit_rect(r, &[SCREEN], SCREEN), Rect { x: 0, y: 100, w: 400, h: 600 });
    }

    #[test]
    fn a_new_overlay_opens_inside_the_right_edge_of_the_window_that_asked() {
        let r = fresh_rect(Kind::ChatOverlay, (380.0, 520.0), Some(SCREEN), 1.25, 0, None, SCREEN);
        assert_eq!(r, Rect { x: 1920 - 475 - 30, y: 80, w: 475, h: 650 });
    }

    #[test]
    fn a_new_overlay_steps_off_the_one_placed_last() {
        let r = fresh_rect(
            Kind::ChatOverlay,
            (380.0, 520.0),
            Some(SCREEN),
            1.0,
            1,
            Some(saved(100, 200, 9)),
            SCREEN,
        );
        assert_eq!(r, Rect { x: 132, y: 232, w: 400, h: 500 });
    }

    #[test]
    fn a_parked_caller_centres_the_popout_on_its_monitor() {
        let r = fresh_rect(Kind::MultiChat, (402.0, 620.0), None, 1.0, 0, None, SCREEN);
        assert_eq!(r, Rect { x: 759, y: 210, w: 402, h: 620 });
    }

    #[test]
    fn a_multichat_rect_saved_by_the_window_state_plugin_carries_over() {
        let v = serde_json::json!({"width":402,"height":620,"x":558,"y":158,"prev_x":1,"prev_y":2,"maximized":false});
        assert_eq!(
            plugin_entry(&v),
            Some(Saved { x: 558, y: 158, width: 402, height: 620, maximized: false, used: 0 })
        );
        let m = serde_json::json!({"width":402,"height":620,"x":-8,"y":-8,"prev_x":300,"prev_y":120,"maximized":true});
        assert_eq!(plugin_entry(&m).map(|s| (s.x, s.y, s.maximized)), Some((300, 120, true)));
        assert_eq!(plugin_entry(&serde_json::json!({"width":0,"height":620,"x":1,"y":1})), None);
    }

    #[test]
    fn the_least_recently_opened_rects_go_first() {
        let mut map: HashMap<String, Saved> = (0..(MAX_SAVED as u64 + 3))
            .map(|i| (format!("overlay-c{i}"), saved(0, 0, i)))
            .collect();
        evict(&mut map);
        assert_eq!(map.len(), MAX_SAVED);
        assert!(!map.contains_key("overlay-c0"));
        assert!(!map.contains_key("overlay-c2"));
        assert!(map.contains_key("overlay-c3"));
    }

    #[test]
    fn a_plugin_label_keeps_only_label_safe_characters() {
        assert_eq!(plugin_window_label("drops.farmer", "main view"), "plugin-drops-farmer-main-view");
        assert!(is_placed_label("plugin-drops-farmer-main"));
        assert!(is_placed_label("overlay-rainyyay"));
        assert!(!is_placed_label("main"));
    }

    #[test]
    fn only_twitch_logins_name_an_overlay() {
        assert!(is_twitch_login("rainyyay"));
        assert!(is_twitch_login("bite_siz2"));
        assert!(!is_twitch_login(""));
        assert!(!is_twitch_login("Rainy"));
        assert!(!is_twitch_login("a/b"));
        assert!(!is_twitch_login(&"a".repeat(26)));
    }

    #[test]
    fn the_overlay_controls_follow_the_right_edge_in_physical_pixels() {
        let c = ControlsZone { right: 10.0, top: 4.0, width: 120.0, height: 22.0 };
        // A 400 px wide window at (1000, 200) on a 1.5x display.
        let r = controls_rect(1000, 200, 600, c, 1.5);
        assert_eq!(r, FRect { x: 1600.0 - 136.0 * 1.5, y: 200.0 - 2.0 * 1.5, w: 132.0 * 1.5, h: 34.0 * 1.5 });
        assert!(r.contains(1590.0, 210.0));
        assert!(!r.contains(1590.0, 300.0), "the chat below the strip stays click-through");
        assert!(!r.contains(1100.0, 210.0), "the channel name stays click-through");
    }

    #[test]
    fn multichat_ids_are_default_or_numbered_extras() {
        assert!(is_multichat_id("default"));
        assert!(is_multichat_id("w2"));
        assert!(is_multichat_id("w9"));
        assert!(!is_multichat_id("w1"));
        assert!(!is_multichat_id("w10"));
        assert!(!is_multichat_id("main"));
    }
}
