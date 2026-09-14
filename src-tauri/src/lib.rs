// Suppress clippy warnings for this release - these are style issues, not bugs!
#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(unused_variables)]
#![allow(deprecated)]
#![allow(clippy::collapsible_if)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::needless_return)]
#![allow(clippy::ptr_arg)]
#![allow(clippy::type_complexity)]
#![allow(clippy::redundant_closure)]
#![allow(clippy::manual_map)]
#![allow(clippy::let_and_return)]
#![allow(clippy::single_match)]
#![allow(clippy::derivable_impls)]
#![allow(clippy::needless_borrow)]
#![allow(clippy::manual_div_ceil)]
#![allow(clippy::unwrap_or_default)]
#![allow(unused_mut)]
#![allow(unused_assignments)]
#![allow(clippy::needless_borrows_for_generic_args)]
#![allow(clippy::manual_flatten)]
#![allow(clippy::collapsible_match)]

use commands::chat_query::{
    get_chat_history_stats, get_chat_rule_errors, search_chat, validate_chat_filter,
    validate_chat_phrase,
};
use commands::moderation_tools::{
    get_automod_queue, get_streamer_mode_state, get_user_note, get_user_pronouns,
    resolve_automod_message, set_user_note, update_channel_info, upload_image,
};
use commands::{
    accounts::*, announcements::*, app::*, automation::*, badge_metadata::*, badge_service::*,
    badges::*, cache::*, channel_panels::*, channel_state::*, chat::*, chat_identity::*, components::*,
    cosmetics_cache::*, diagnostic_logging::*, drops::*, emoji::*, emote_prefetch::*,
    emotes::*, eventsub::*, ffz::*, gifs::*, helix::*, home_snapshot::*, hype_train::*, identity::*, justlog::*, layout::*,
    link_preview::*, logs::*, mod_log_storage::*, modroom::*, plugins::*,
    profile_cache::*, provider_browse::*,
    resub::*, session::*, settings::*, seventv::*, seventv_cosmetics::*,
    seventv_cosmetics_fetch::*, song_id::*, streamnook_api::*, streaming::*, subscriptions::*,
    twitch::*,
    universal_cache::*,
    user_profile::*, vod_progress::*, watch_streak::*, whisper_storage::*,
};
// Desktop-only feature modules, excluded from the phone app (watch/earn/chat
// only): MultiNook tiling, Discord RPC, and profile-card screen capture.
#[cfg(desktop)]
use commands::{discord::*, multi_nook::*, screen_capture::*};
use log::{debug, error, info, warn};
use models::settings::{AppState, CloseToTrayMode, Settings};
use services::background_service::BackgroundService;
use services::cache_service;
use services::drops_service::DropsService;
use services::live_notification_service::LiveNotificationService;
use services::whisper_service::WhisperService;
use std::sync::{Arc, Mutex};
use tauri::{Builder, Emitter, Manager, WindowEvent};
// Tray + menu are desktop-only (no tray paradigm on Android).
#[cfg(desktop)]
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tokio::sync::Mutex as TokioMutex;

/// A `streamnook://` deep link that arrived before the frontend was listening
/// (cold start). The webview drains it via `take_pending_watch_link` on mount.
#[derive(Default)]
struct PendingWatchLink(Mutex<Option<String>>);

/// Parse a `streamnook://` share link into a Twitch channel login to open.
///
/// Accepts `streamnook://watch/<channel>` and `streamnook://w/<channel>` (the
/// format the share button and web landing page produce), plus a bare
/// `streamnook://<channel>` fallback. Returns a sanitized login (lowercase,
/// `[a-z0-9_]`) or None when there's nothing watchable in the URL.
fn parse_watch_link(url: &tauri::Url) -> Option<String> {
    if url.scheme() != "streamnook" {
        return None;
    }
    // The action ("watch"/"w") is the URL authority; the channel is the first
    // path segment. A bare streamnook://<channel> has the channel as the authority.
    let action = url.host_str().unwrap_or("");
    let first_segment = url
        .path_segments()
        .and_then(|mut segments| segments.next())
        .unwrap_or("");
    let raw = match action {
        "watch" | "w" => first_segment,
        _ => action,
    };
    // Twitch logins are at most 25 chars; cap so a padded/junk URL can't emit an
    // oversized string to the frontend.
    let clean: String = raw
        .trim()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .take(25)
        .collect::<String>()
        .to_lowercase();
    if clean.is_empty() {
        None
    } else {
        Some(clean)
    }
}

/// Drain a deep link the app was cold-started with, if any. The frontend calls
/// this once on mount so a link that fired before its listener existed still
/// opens the channel. Returns None when the app wasn't launched from a link.
#[tauri::command]
fn take_pending_watch_link(state: tauri::State<'_, PendingWatchLink>) -> Option<String> {
    state.0.lock().ok().and_then(|mut guard| guard.take())
}

/// First-paint signal from the webview. The main window is created hidden
/// (config `visible: false`) so cold start never shows a blank shell; the
/// frontend invokes this after its first painted frame. Saved geometry is
/// restored here rather than by the window-state plugin's ready-time hook
/// (skip_initial_state below): the plugin's MAXIMIZED restore runs
/// SW_MAXIMIZE, which force-shows a hidden window before anything painted.
/// Restoring while hidden, then showing, keeps the gate intact and a
/// maximized session comes back maximized in a single ShowWindow.
#[cfg(desktop)]
#[tauri::command]
fn reveal_main_window(app: tauri::AppHandle) {
    use tauri_plugin_window_state::{StateFlags, WindowExt};
    if let Some(window) = app.get_webview_window("main") {
        if !window.is_visible().unwrap_or(false) {
            let _ = window.restore_state(
                StateFlags::SIZE | StateFlags::POSITION | StateFlags::MAXIMIZED,
            );
            info!("[Main] reveal: first paint signaled, geometry restored");
        } else {
            // Failsafe or tray already showed it; the signal still proves the
            // frontend->reveal path works (it was silently ACL-blocked once).
            info!("[Main] reveal: first paint signaled (window already shown)");
        }
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Mobile twin of the reveal command. The shared frontend invokes it on first
/// paint whatever the platform; on Android the activity owns visibility (tao's
/// `set_visible` is a no-op there), so there is nothing to reveal, but the
/// command must exist and be allowed or the invoke is denied at the ACL.
#[cfg(mobile)]
#[tauri::command]
fn reveal_main_window(_app: tauri::AppHandle) {}

/// Dead-window net for the hidden-until-ready gate: if the frontend never
/// reaches its reveal invoke (boot crash, failed chunk load, wedged webview),
/// show the window anyway so the app can never run headless. The window paints
/// its configured background color, so a forced early show is dark, not white.
#[cfg(desktop)]
fn arm_reveal_failsafe(handle: tauri::AppHandle) {
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(5));
        if let Some(window) = handle.get_webview_window("main") {
            if !window.is_visible().unwrap_or(false) {
                warn!("[Main] reveal failsafe fired: frontend never signaled first paint");
                let _ = window.show();
            }
        }
    });
}

/// Bring the main StreamNook window forward — used by the tray icon left-click
/// and the "Show StreamNook" menu item. Restores from minimized if needed and
/// re-shows if the window was hidden to the tray on close.
#[cfg(desktop)]
fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }
    // Main was fully closed (the streamer went live, which destroys it to free its
    // ~350MB of webview + player memory). Recreate it from the same options the
    // config defines so the app shell, overlays, and player come back identically.
    // The WebView2 browser args are applied process-wide via env var, so no
    // per-window arg work is needed here.
    //
    // URL: in a DEBUG build (`tauri dev`) runtime-created windows are NOT
    // auto-pointed at the Vite dev server the way config-defined windows are, so an
    // App URL loads the (unbuilt) bundled dist and renders blank white. Use the dev
    // URL explicitly in debug; the shipped release loads the bundled index.html.
    let app_url = if cfg!(debug_assertions) {
        app.config()
            .build
            .dev_url
            .clone()
            .map(tauri::WebviewUrl::External)
            .unwrap_or_else(|| tauri::WebviewUrl::App("index.html".into()))
    } else {
        tauri::WebviewUrl::App("index.html".into())
    };
    match tauri::WebviewWindowBuilder::new(app, "main", app_url)
        .title("StreamNook")
    .inner_size(1600.0, 1000.0)
    .min_inner_size(800.0, 600.0)
    .center()
    .resizable(true)
    .decorations(false)
    // Matches the config window's backgroundColor so the shell paints dark
    // while the webview boots. Recreation is user-initiated (tray click), so
    // the window shows immediately instead of gating on the reveal signal;
    // saved geometry is restored below since skip_initial_state("main")
    // covers every creation of this label, not just the first.
    .background_color(tauri::window::Color(0x0c, 0x0c, 0x0d, 0xff))
    .build()
    {
        Ok(win) => {
            debug!("[Main] Recreated main window on demand");
            {
                use tauri_plugin_window_state::{StateFlags, WindowExt};
                let _ = win.restore_state(
                    StateFlags::SIZE | StateFlags::POSITION | StateFlags::MAXIMIZED,
                );
            }
            // Re-point the UI-hang watchdog at the new HWND. The old watchdog
            // thread self-exits once its HWND is destroyed (see ui_hang_watchdog).
            #[cfg(windows)]
            if let Ok(hwnd) = win.hwnd() {
                services::ui_hang_watchdog::start_for_hwnd(hwnd.0 as isize);
            }
        }
        Err(e) => error!("[Main] Failed to recreate main window: {e}"),
    }
}

/// Pull an oversized saved restore rect back inside the monitor's work area.
///
/// Only the size is corrected, and `showCmd` is untouched, so a window that restored
/// maximized stays maximized with no flash and only its restore geometry changes.
/// Keeping left/top also sidesteps the workspace-vs-screen coordinate ambiguity of
/// `rcNormalPosition`.
#[cfg(windows)]
fn sanitize_restore_rect(window: &tauri::WebviewWindow) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowPlacement, SetWindowPlacement, WINDOWPLACEMENT,
    };

    let Ok(Some(monitor)) = window.current_monitor() else {
        return;
    };
    let work = monitor.work_area();
    let Ok(raw) = window.hwnd() else {
        return;
    };
    let hwnd = HWND(raw.0 as *mut std::ffi::c_void);

    let mut placement = WINDOWPLACEMENT {
        length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
        ..Default::default()
    };
    if unsafe { GetWindowPlacement(hwnd, &mut placement) }.is_err() {
        return;
    }

    let r = placement.rcNormalPosition;
    let (w, h) = (r.right - r.left, r.bottom - r.top);
    let (max_w, max_h) = (work.size.width as i32, work.size.height as i32);
    if w <= max_w && h <= max_h {
        return;
    }

    placement.rcNormalPosition.right = r.left + w.min(max_w);
    placement.rcNormalPosition.bottom = r.top + h.min(max_h);
    let _ = unsafe { SetWindowPlacement(hwnd, &placement) };
    debug!("[Main] Clamped oversized restore rect to the monitor work area");
}

/// Get-or-create the main window. Invoked from a MultiChat popout when an action
/// needs the main app (badge/paint overlay, public profile viewer, whisper,
/// watch-in-main, clip/VOD playback, or the "open main app" button) after Go Live
/// destroyed it. Shows the existing window if it's only hidden.
#[cfg(desktop)]
#[tauri::command]
fn ensure_main_window(app: tauri::AppHandle) {
    show_main_window(&app);
}

/// Destroy the main window (Go Live closes it to free its memory). Done Rust-side
/// so it bypasses BOTH the JS `core:window:allow-destroy` permission and the
/// close-to-tray CloseRequested interception (a forceful destroy fires no
/// CloseRequested). The caller stops the stream + drops first; Rust releases the
/// window's IRC claims on the resulting Destroyed event.
#[cfg(desktop)]
#[tauri::command]
fn close_main_window(app: tauri::AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.destroy();
    }
}

#[cfg(target_os = "android")]
mod android_notify;
mod commands;
mod models;
mod plugin_host;
mod services;
#[cfg(target_os = "android")]
mod twitch_login_plugin;
mod utils;

/// Load settings from the custom location in the same directory as cache
fn load_settings_from_file() -> Result<Settings, Box<dyn std::error::Error>> {
    let app_dir = cache_service::get_app_data_dir()?;
    let settings_path = app_dir.join("settings.json");

    if !settings_path.exists() {
        return Ok(Settings::default());
    }

    let json = std::fs::read_to_string(&settings_path)?;
    let settings: Settings = serde_json::from_str(&json)?;
    Ok(settings)
}

/// Clean up leftover files from previous update attempts
fn cleanup_update_artifacts() {
    if let Ok(current_exe) = std::env::current_exe() {
        if let Some(exe_dir) = current_exe.parent() {
            // Remove leftover StreamNook_new.exe if it exists
            let temp_exe = exe_dir.join("StreamNook_new.exe");
            if temp_exe.exists() {
                debug!("[Main] Cleaning up leftover update file: {:?}", temp_exe);
                let _ = std::fs::remove_file(&temp_exe);
            }

            // Remove leftover update batch script if it exists
            let batch_file = exe_dir.join("update_streamnook.bat");
            if batch_file.exists() {
                debug!("[Main] Cleaning up leftover batch file: {:?}", batch_file);
                let _ = std::fs::remove_file(&batch_file);
            }
        }
    }
}

/// One-time cleanup for users upgrading from a Streamlink-bundled build. Older
/// releases shipped a ~200 MB `streamlink/` folder next to the exe; native
/// resolution no longer needs it, so remove it in the background to reclaim the
/// space. Best-effort and silent: any failure is logged and simply retried on
/// the next launch. Runs off the main thread so a large delete never delays
/// startup.
fn cleanup_legacy_streamlink_bundle() {
    let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
    else {
        return;
    };
    let streamlink_dir = exe_dir.join("streamlink");
    if !streamlink_dir.exists() {
        return;
    }
    std::thread::spawn(move || match std::fs::remove_dir_all(&streamlink_dir) {
        Ok(_) => debug!(
            "[Cleanup] Removed legacy bundled streamlink folder: {:?}",
            streamlink_dir
        ),
        Err(e) => debug!(
            "[Cleanup] Could not remove legacy streamlink folder {:?}: {}",
            streamlink_dir, e
        ),
    });
}

#[tauri::command]
fn read_clipboard_text_native(app: tauri::AppHandle) -> Result<String, String> {
    app.clipboard().read_text().map_err(|e| e.to_string())
}

/// Shared app entry point. Desktop's `main.rs` calls this directly; on mobile
/// Tauri's generated Android/iOS project loads this library and invokes `run()`
/// via the `mobile_entry_point` macro.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Apply WebView2 browser arguments uniformly to every webview in the process.
    // Setting them via this env var (inherited by the msedgewebview2.exe child)
    // instead of on a single window in tauri.conf.json avoids HRESULT 0x8007139F
    // ("group or resource is not in the correct state"): WebView2 rejects any window
    // that requests different browser args than an existing window sharing the same
    // user-data folder. Pinning the args on only the main window broke every
    // Rust-created window on the default profile (7TV login/refresh, automation,
    // chat-identity). One uniform set keeps the main-window tweaks (audio in-process,
    // SmartScreen off) while letting those popups open again.
    //
    // CalculateNativeWinOcclusion is disabled: with it on, the UI thread wedges in a
    // blocking COM call whenever the window is occluded (covered, not minimized) or the
    // shell queries it (taskbar jump list / thumbnail), showing "(Not Responding)" for
    // tens of seconds until the call returns. This is a separate fault from the
    // alt-tab/minimize freeze, which was the composited child-HWND webview and is fixed
    // by not enabling `tauri/unstable`. Disabling the occlusion calc only stops Chromium
    // throttling hidden windows, a negligible cost for one media window.
    let mut webview_args = String::from(
        "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection,AudioServiceOutOfProcess,CalculateNativeWinOcclusion",
    );
    // Headless debugging: SN_CDP_PORT=<port> opens the webview's remote debug
    // port on localhost so tooling can inspect the live page (heap snapshots,
    // DOM counters, the cdp.mjs recipes). Per-launch opt-in through the
    // environment only, never a persisted setting: the port is a local attack
    // surface (any process on this machine can drive the page), the same
    // trust boundary as the DEV-only devtools loader. Until 2026-09-06 this was
    // compiled out of release builds, which left every release-build memory
    // report uninspectable short of a byte-patched exe. The value must parse
    // as a port so nothing else can ride into the browser arguments.
    let cdp_port: Option<u16> = std::env::var("SN_CDP_PORT")
        .ok()
        .and_then(|p| p.trim().parse::<u16>().ok())
        .filter(|p| *p != 0);
    if let Some(port) = cdp_port {
        webview_args.push_str(&format!(" --remote-debugging-port={port}"));
    }
    std::env::set_var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS", webview_args);

    // Initialize the logging system FIRST so all debug!/error! macros work
    services::diagnostic_logger::init_logging();
    if let Some(port) = cdp_port {
        warn!("[Main] SN_CDP_PORT set: WebView2 remote debugging is listening on 127.0.0.1:{port} for this launch");
    }

    // Clean up any leftover files from previous update attempts
    cleanup_update_artifacts();

    // Reclaim the ~200 MB legacy streamlink bundle for users upgrading from a
    // Streamlink-bundled build (native resolution no longer needs it).
    cleanup_legacy_streamlink_bundle();

    // Migrate emote cache if app version changed (handles format changes like webp → avif)
    // This clears stale emote files that may have the old format
    let current_version = env!("CARGO_PKG_VERSION");
    match services::universal_cache_service::migrate_emote_cache_on_version_change(current_version)
    {
        Ok(migrated) => {
            if migrated {
                debug!("[Main] Emote cache migrated for new version");
            }
        }
        Err(e) => {
            error!("[Main] Failed to migrate emote cache: {}", e);
        }
    }

    // One-time, FFZ-only purge so previously cached static FFZ frames re-download
    // as their animated variants. Targeted by source URL, so other providers'
    // emote caches are left intact.
    match services::universal_cache_service::migrate_ffz_animated_cache() {
        Ok(purged) => {
            if purged {
                debug!("[Main] FFZ emote cache purged for animated re-fetch");
            }
        }
        Err(e) => {
            error!("[Main] Failed to purge FFZ emote cache: {}", e);
        }
    }

    // One-time purge of OLD non-7TV emote files (bare-id keys + URL-derived
    // `.0`/`.bin` extensions) so they re-cache under provider-namespaced keys with
    // content-typed extensions. 7TV files (already correct) are kept.
    match services::universal_cache_service::migrate_emote_namespace_cache() {
        Ok(purged) => {
            if purged {
                debug!("[Main] Non-7TV emote cache purged for namespaced re-fetch");
            }
        }
        Err(e) => {
            error!("[Main] Failed to purge non-7TV emote cache: {}", e);
        }
    }

    // Forced-logout switch. Bumping FORCE_REAUTH_TOKEN in account_store signs every
    // user out on their next launch so they re-login into the current auth-storage
    // layout. It runs HERE, before the Tauri builder creates any window, so the
    // per-account profiles and the default WebView2 store are unlocked and the wipe
    // actually lands (a prior frontend attempt failed because the live session held
    // those files locked).
    //
    // DESKTOP ONLY. On mobile the app-data base is not resolved until setup(), so
    // this early call cannot write its marker ("could not resolve marker path:
    // Read-only file system"). The marker is written only after a successful wipe,
    // by design, so the run repeats on EVERY launch — wiping tokens and WebView
    // storage each time. That signs the user out on every start, clears the
    // localStorage keys the one-time frontend migrations rely on, and re-opens the
    // setup wizard forever, which makes the Android build unusable.
    //
    // Skipping it on mobile is also correct on its own terms: this is a migration
    // off a pre-2026-06-18 auth layout, and no Android install has ever had one.
    #[cfg(desktop)]
    if tauri::async_runtime::block_on(
        services::account_store::AccountStore::run_force_reauth_if_needed(),
    ) {
        debug!("[Main] Forced one-time re-auth: all sessions cleared");
    }

    // Load settings from our custom location in the same directory as cache
    let settings = load_settings_from_file().unwrap_or_else(|_| Settings::default());
    // Compile the chat rule engine (highlights, ignores, saved filters) from
    // the loaded settings before any chat connects.
    services::chat_rules::ChatRules::refresh(&settings);
    services::streamer_mode::StreamerMode::refresh(&settings);

    // Apply persisted diagnostic logging setting immediately after loading settings
    services::diagnostic_logger::set_diagnostics_enabled(settings.error_reporting_enabled);

    // Initialize drops service with persisted settings (including priority_games for favorites)
    let drops_service = Arc::new(TokioMutex::new(DropsService::new_with_settings(
        settings.drops.clone(),
    )));

    let settings_arc = Arc::new(Mutex::new(settings));

    // The debounced settings flusher snapshots from this shared state at flush
    // time (same Arc the managed AppState holds below).
    commands::settings::register_settings_source(settings_arc.clone());

    // Initialize live notification service
    let live_notification_service = Arc::new(LiveNotificationService::new());

    // Initialize whisper service
    let whisper_service = Arc::new(TokioMutex::new(WhisperService::new()));

    // Initialize layout service
    let layout_service = Arc::new(services::layout_service::LayoutService::new());

    // Initialize emote service
    let emote_service = Arc::new(tokio::sync::RwLock::new(
        services::emote_service::EmoteService::new(),
    ));
    let emote_service_state = commands::emotes::EmoteServiceState(emote_service.clone());

    // Initialize AFK emote prefetch service (bulk disk-cache of followed-channel emotes)
    let emote_prefetch_state = commands::emote_prefetch::EmotePrefetchServiceState(Arc::new(
        services::emote_prefetch_service::EmotePrefetchService::new(emote_service.clone()),
    ));

    // Initialize EventSub service
    let eventsub_service = Arc::new(tokio::sync::RwLock::new(
        services::eventsub_service::EventSubService::new(),
    ));
    let eventsub_service_state = commands::eventsub::EventSubServiceState(eventsub_service.clone());

    let builder = Builder::default();
    // Single-instance is desktop-only and must be registered first: it forwards a
    // streamnook:// deep link opened while the app is already running to the
    // existing instance (via the deep-link feature) instead of spawning a
    // duplicate window. No multi-process/duplicate-window concept on mobile.
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
        show_main_window(app);
    }));
    let builder = builder
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_opener::init());
    // Window-state (size/position/maximized persistence) is desktop-only; there
    // are no OS windows to persist on mobile.
    #[cfg(desktop)]
    let builder = builder.plugin(
        tauri_plugin_window_state::Builder::default()
            .with_state_flags(
                tauri_plugin_window_state::StateFlags::SIZE
                    | tauri_plugin_window_state::StateFlags::POSITION
                    | tauri_plugin_window_state::StateFlags::MAXIMIZED,
            )
            // The Twitch login/drops/subscribe popups size themselves to the app
            // window every time, so saved geometry must not restore (and shrink)
            // them. Excluding them also stops a stale small size from leaving
            // their content webview mismatched and blank.
            // The generated-label utility webviews (profile-<user>-<ts>,
            // kick-resolve-*, identity-fetch-*, seventv-login-*) accrete one
            // entry per unique label forever - the state file had grown to
            // 176 windows - so they are excluded too. multichat-default and
            // plugin-* keep stable labels and wanted geometry, so they stay.
            .with_filter(|label| {
                !(label == "twitch-login"
                    || label == "drops-login"
                    || label.starts_with("subscribe-")
                    || label.starts_with("profile-")
                    || label.starts_with("kick-resolve-")
                    || label.starts_with("identity-fetch-")
                    || label.starts_with("seventv-login-"))
            })
            // The main window is created hidden and revealed on first paint;
            // the plugin's ready-time MAXIMIZED restore would force-show it
            // early (SW_MAXIMIZE activates), so reveal_main_window owns the
            // restore instead.
            .skip_initial_state("main")
            .build(),
    );
    // Android in-app Twitch login WebView overlay (native Kotlin plugin).
    #[cfg(target_os = "android")]
    let builder = builder.plugin(twitch_login_plugin::init());
    builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_os::init())
        // Native OS notifications. Cross-platform: on Android this is the system
        // notification shade, which is where live alerts belong on a phone rather
        // than the in-app Dynamic Island the desktop uses.
        .plugin(tauri_plugin_notification::init())
        .manage(live_notification_service.clone())
        .manage(whisper_service.clone())
        .manage(layout_service.clone())
        .manage(emote_service_state)
        .manage(emote_prefetch_state)
        .manage(eventsub_service_state)
        .setup(move |app| {
            // Only the primary instance reaches setup (a deep-link secondary
            // exits inside the single-instance plugin), so this is the proof
            // that unlocks the file log and its "==== started ====" banner.
            services::file_log::arm();
            let app_handle = app.handle().clone();
            // Mobile: resolve the app-private data dir once, up front, so every
            // file-based token/cookie/cache/settings store writes to a writable
            // sandbox path. Desktop resolvers keep their existing platform paths
            // (this cell stays unset there). See services::app_paths.
            #[cfg(mobile)]
            if let Ok(dir) = app.path().app_data_dir() {
                services::app_paths::set_base(dir);
            }
            // Mobile: the settings load at the top of run() executed BEFORE the
            // base above was resolvable, so it always fell back to
            // Settings::default() (setup_complete=false among everything else),
            // and the next save wrote those defaults back over the real file,
            // reverting setup_complete on every launch. Re-read the real
            // settings.json now that the path resolves and push the values into
            // the stores that were constructed from the defaults.
            #[cfg(mobile)]
            if let Ok(loaded) = load_settings_from_file() {
                services::diagnostic_logger::set_diagnostics_enabled(
                    loaded.error_reporting_enabled,
                );
                let drops_settings = loaded.drops.clone();
                if let Ok(mut s) = settings_arc.lock() {
                    *s = loaded;
                }
                let drops_service_reload = drops_service.clone();
                tauri::async_runtime::spawn(async move {
                    drops_service_reload
                        .lock()
                        .await
                        .update_settings(drops_settings)
                        .await;
                });
            }
            // Runtime stall detector: measures backend freezes (tokio-blocked vs
            // whole-process) and records them to the capture file. Started here,
            // inside the tokio runtime Tauri set up.
            services::runtime_watchdog::start();
            // Process-tree memory telemetry: one [Resource] line a minute in the
            // file log (rust, WebView2 browser/GPU/renderers/utilities, plugin
            // children), so a "StreamNook is at 800 MB" report can be read off
            // the log without attaching anything to the user's machine.
            services::resource_log::start(app_handle.clone());
            // UI-thread "Not Responding" detector: probes the main window's message
            // pump (the same signal Windows uses for "(Not Responding)") and records
            // hangs the runtime_watchdog can't see, like a wedged WebView2/COM call
            // while the login overlay is up. Needs the HWND, so it starts here.
            #[cfg(windows)]
            {
                if let Some(main) = app.get_webview_window("main") {
                    if let Ok(hwnd) = main.hwnd() {
                        services::ui_hang_watchdog::start_for_hwnd(hwnd.0 as isize);
                    }
                }
            }
            // A build before the logical-units fix could resize the window past the
            // screen, and the window-state plugin persists that rect because it only
            // skips saving while maximized. Correcting rcNormalPosition rather than the
            // live size also covers the case where the user quit while maximized, where
            // the oversized rect stays invisible until the next unmaximize or drag.
            // Config windows are built before this setup hook runs and the plugin
            // restores in its on_window_ready callback, so the state is already applied.
            #[cfg(windows)]
            {
                if let Some(main) = app.get_webview_window("main") {
                    sanitize_restore_rect(&main);
                }
            }
            // Hidden-until-ready gate: the frontend reveals on first paint;
            // this net guarantees a window even if the frontend never boots.
            #[cfg(desktop)]
            arm_reveal_failsafe(app_handle.clone());
            // Hand the stream server an app handle so the ad auto-pivot can emit
            // its `ad-pivot` reload event to the player.
            services::stream_server::set_app_handle(app_handle.clone());
            services::providers::set_app_handle(app_handle.clone());

            // Start the shared 7TV EventAPI WebSocket client (live emote set
            // updates, and later cosmetics). It idle-connects and subscribes
            // per channel as the IRC service JOINs/PARTs them.
            services::seventv_eventapi::init(app_handle.clone(), emote_service.clone());

            // Dedicated EventSub socket for the moderator view (channel.moderate).
            // Tied to chat, not the watched stream: it subscribes per channel the
            // IRC service JOINs and the user moderates, so the mod log enriches
            // with the acting moderator in single / offline / MultiNook / popout.
            services::eventsub_moderation::init(app_handle.clone());
            // Streamer mode detector (sleeps unless the setting is "auto").
            services::streamer_mode::StreamerMode::init(app_handle.clone());

            // Register deep link scheme on Windows
            #[cfg(windows)]
            {
                use tauri_plugin_deep_link::DeepLinkExt;
                let _ = app.deep_link().register_all();
            }

            // Resolve streamnook:// share links to a channel and hand it to the UI.
            //
            // Two arrival paths, both covered here:
            //  - Warm (app running): the single-instance plugin re-feeds the URL, the
            //    deep-link plugin emits, on_open_url fires, we emit to the (mounted)
            //    frontend.
            //  - Cold (app launched by the link): the plugin already emitted before this
            //    listener (and the webview) existed, so we also drain get_current()
            //    into PendingWatchLink, which the frontend pulls on mount.
            {
                use tauri_plugin_deep_link::DeepLinkExt;
                app.manage(PendingWatchLink::default());

                let watch_handle = app_handle.clone();
                app.deep_link().on_open_url(move |event| {
                    for url in event.urls() {
                        if let Some(channel) = parse_watch_link(&url) {
                            #[cfg(desktop)]
                            show_main_window(&watch_handle);
                            let _ = watch_handle.emit("streamnook:watch", channel);
                        }
                    }
                });

                if let Ok(Some(urls)) = app.deep_link().get_current() {
                    for url in urls {
                        if let Some(channel) = parse_watch_link(&url) {
                            if let Some(pending) = app.try_state::<PendingWatchLink>() {
                                if let Ok(mut guard) = pending.0.lock() {
                                    *guard = Some(channel.clone());
                                }
                            }
                            let _ = app_handle.emit("streamnook:watch", channel);
                        }
                    }
                }
            }

            // Create and manage the background service correctly within the setup hook
            let background_service = Arc::new(TokioMutex::new(BackgroundService::new(
                app_handle.clone(),
                drops_service.clone(),
            )));

            let twitch_auth =
                services::twitch_auth_service::TwitchAuthService::new(app_handle.clone());

            // Out-of-process plugin host (docs/plugins/). Ships with zero
            // plugins; it only ever runs what the user installs and enables.
            let plugin_host = Arc::new(plugin_host::PluginHost::new(app_handle.clone()));

            // Core parity heartbeat: reports the on-screen channel while it
            // plays, nothing else. Ticks no-op until a stream is watched.
            let watch_heartbeat = Arc::new(
                services::watch_heartbeat_service::WatchHeartbeatService::new(),
            );
            watch_heartbeat.start();

            // Whether background points automation was left enabled. Used right
            // after the background service starts to bring the user-global points
            // socket up at launch, so the plugin's background earns notify even
            // before any stream is opened (restores pre-extraction behavior).
            let initial_automation = settings_arc
                .lock()
                .map(|s| s.drops.auto_claim_channel_points)
                .unwrap_or(false);

            // Hand the provider adapters a settings handle. Done here rather than
            // from the Twitch chat path, which a YouTube-only session never runs.
            services::providers::init_settings(settings_arc.clone());

            let app_state = AppState {
                settings: settings_arc,
                drops_service,
                background_service: background_service.clone(),
                layout_service: layout_service.clone(),
                emote_service: emote_service.clone(),
                twitch_auth,
                plugin_host: plugin_host.clone(),
                watch_heartbeat,
            };

            // Clone the app_state before managing it (only the desktop live
            // notification spawn below consumes it)
            #[cfg(not(target_os = "android"))]
            let app_state_for_live_notif = app_state.clone();
            let app_state_for_provider_live = app_state.clone();
            let app_state_for_favorite_live = app_state.clone();

            // Manage AppState directly, not wrapped in Arc
            app.manage(app_state);

            // Who's-live polling for followed channels on non-Twitch platforms.
            // Separate from the Twitch live-notification service: it reads the
            // app-local follow list and each platform's own adapter, but emits
            // the SAME `streamer-went-live` event so the notification UI is
            // shared. Started here (not earlier in setup) because it needs the
            // AppState, which only exists at this point.
            services::provider_live_service::start(app_handle.clone(), app_state_for_provider_live);

            // Who's-live polling for FAVOURITED channels, which may not be
            // followed on any platform and so are invisible to both the Twitch
            // follow poller and the provider one above. Emits its own
            // `favorites-live-update` for the lists, and `streamer-went-live`
            // tagged `source: "favorite"` so the notification UI can gate it
            // on its own setting.
            services::favorite_live_service::start(
                app_handle.clone(),
                app_state_for_favorite_live,
            );

            // Keep the Kick OAuth pair perpetually fresh (single-flight refresh
            // on a clock), and the YouTube cookie harvest young. Both are what
            // makes those logins behave like the Twitch one: renewed as a matter
            // of course instead of dying quietly between uses.
            // Both daemons harvest through hidden desktop webviews; the phone
            // has one webview and signs in through its native overlay instead.
            #[cfg(desktop)]
            {
                services::kick_auth_service::start_refresh_daemon();
                services::youtube_auth_service::start_reharvest_daemon();
            }

            // Start the plugin host: loads the registry and starts plugins
            // the user previously enabled. No-op with none installed.
            tauri::async_runtime::spawn(async move {
                plugin_host.startup().await;
            });

            // Start background service, then bring the user-global points socket
            // up if automation was left enabled.
            tauri::async_runtime::spawn(async move {
                background_service.lock().await.start().await;
                background_service
                    .lock()
                    .await
                    .set_automation_active(initial_automation)
                    .await;
            });

            // Start live notification service. Desktop only: on Android the
            // WorkManager worker (android_notify.rs) is the single delivery
            // lane, and this unthrottled 60s poll is exactly the background
            // churn it replaces.
            #[cfg(not(target_os = "android"))]
            {
                let live_notif_service = live_notification_service.clone();
                let live_app_handle = app_handle.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = live_notif_service.start(live_app_handle, app_state_for_live_notif).await {
                        error!("Failed to start live notification service: {}", e);
                    }
                });
            }

            // Rust-owned Home snapshot: one followed-streams poll a minute that
            // feeds the live-notification diff AND the Home/Sidebar grids, plus
            // the offline roster, recommended page and hype trains on their own
            // cadences. A mounting Home paints from get_home_snapshot with no
            // network on its critical path. See services::home_snapshot.
            services::home_snapshot::start(app_handle.clone(), live_notification_service.clone());

            // Per-channel chat state (viewers, points, pinned) for every channel a
            // window has chat open on, and per-user history pushes for open user
            // cards. Replaces three JS timers per mounted chat and a 2.5 s poll
            // per open card. See services::channel_state.
            services::channel_state::start(app_handle.clone());
            // Minimized-window signal: the page cannot see it (native occlusion
            // detection is off), Rust can. See services::window_visibility.
            // Desktop only: on Android the activity pause already drives
            // document.visibilityState, which isWindowHidden() reads first.
            #[cfg(desktop)]
            services::window_visibility::start(app_handle.clone());
            services::user_message_history_service::UserMessageHistoryService::set_app_handle(app_handle.clone());
            // Badge-drop feed socket, owned here so drops still arrive while the
            // main window is destroyed (live mode, tray). See services::badge_feed.
            services::badge_feed::start(app_handle.clone());

            // Badge-drop detection now lives server-side on the Penrose bot and
            // is delivered to the app over the badge WebSocket feed
            // (services::badge_feed, started above). The old cache-polling
            // detector is retired so drops surface within minutes instead of up
            // to a day late, and so a pushed drop and a locally-detected one can
            // never double-notify.

            // Verify token health on startup
            tauri::async_runtime::spawn(async move {
                use services::twitch_service::TwitchService;

                debug!("[Main] Starting token health verification...");

                // Wait a moment to let the app fully initialize
                tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

                match TwitchService::verify_token_health().await {
                    Ok(status) => {
                        if status.is_valid {
                            debug!(
                                "[Main] Token health check passed: {}h {}m remaining",
                                status.hours_remaining, status.minutes_remaining
                            );
                            if status.needs_refresh {
                                debug!("[Main] Token expires soon, but will auto-refresh on next API call");
                            }

                            // Record the current login as the primary account in the
                            // multi-account registry. Cheap once recorded; self-heals if
                            // the user later signs in as a different account. Best-effort.
                            if let Some(uid) = status.user_id.as_deref() {
                                services::account_store::AccountStore::reconcile_primary(uid).await;
                            }
                        } else {
                            debug!(
                                "[Main] Token health check failed: {:?}",
                                status.error.unwrap_or_else(|| "Unknown error".to_string())
                            );
                        }
                    }
                    Err(e) => {
                        debug!("[Main] Token health verification error: {}", e);
                    }
                }
            });

            // No background global-badge prefetch. Global badges are fetched
            // lazily on the first chat badge resolution (get_user_badges), and
            // badge-drop detection is handled centrally by the bot and delivered
            // over the badge feed, so no client hits Helix for badges at startup.

            // Initialize unified badge service
            tauri::async_runtime::spawn(async move {
                use commands::badge_service::initialize_badge_service;

                debug!("[Main] Initializing unified badge service...");

                // Wait a moment for token to be available
                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

                initialize_badge_service().await;
            });

            // System tray (desktop-only — no tray paradigm on Android). Keeps the
            // app running when the user closes the main window while StreamNook
            // MultiChat popouts are still open. Left click brings the main window
            // forward; right click opens a menu with Show / Open MultiChat / Quit.
            #[cfg(desktop)]
            {
            let show_item = MenuItem::with_id(app, "show", "Show StreamNook", true, None::<&str>)?;
            let open_multichat_item = MenuItem::with_id(
                app,
                "open_multichat",
                "Open MultiChat",
                true,
                None::<&str>,
            )?;
            // A click-through chat overlay cannot be clicked to turn itself
            // back; the tray is reachable even with a game in front.
            let overlay_clickable_item = MenuItem::with_id(
                app,
                "overlay_clickable",
                "Make chat overlays clickable",
                true,
                None::<&str>,
            )?;
            let sep = PredefinedMenuItem::separator(app)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit StreamNook", true, None::<&str>)?;
            let tray_menu = Menu::with_items(
                app,
                &[&show_item, &open_multichat_item, &overlay_clickable_item, &sep, &quit_item],
            )?;

            let _tray = TrayIconBuilder::new()
                .menu(&tray_menu)
                .show_menu_on_left_click(false)
                .tooltip("StreamNook")
                .icon(app.default_window_icon().unwrap().clone())
                .on_menu_event(|app_handle, event| match event.id.as_ref() {
                    "show" => show_main_window(app_handle),
                    "overlay_clickable" => {
                        // Every overlay window listens; payload forces interactive.
                        let _ = app_handle.emit(
                            "chat-overlay-toggle-interactive",
                            serde_json::json!({ "interactive": true }),
                        );
                    }
                    "open_multichat" => {
                        // Recreate/show main first (going live may have CLOSED it),
                        // then defer to its JS helper, which owns popout spawning
                        // (URL params, label generation, persistence id). On a cold
                        // recreate the emit can land before the JS listener is up; the
                        // user still gets main back and can open MultiChat from there.
                        show_main_window(app_handle);
                        if let Some(main_win) = app_handle.get_webview_window("main") {
                            let _ = main_win.emit("tray-open-multichat", ());
                        }
                    }
                    "quit" => {
                        app_handle.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        show_main_window(tray.app_handle());
                    }
                })
                .build(app)?;
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // App commands
            get_app_version,
            get_app_name,
            get_app_description,
            get_app_authors,
            fetch_exchange_rates,
            get_window_size,
            take_pending_watch_link,
            #[cfg(target_os = "android")]
            twitch_login_plugin::open_mobile_login,
            #[cfg(target_os = "android")]
            twitch_login_plugin::close_mobile_login,
            #[cfg(target_os = "android")]
            twitch_login_plugin::get_mobile_login_cookies,
            #[cfg(target_os = "android")]
            android_notify::push_register,
            #[cfg(target_os = "android")]
            android_notify::push_unregister,
            #[cfg(mobile)]
            flush_persistent_stores,
            reveal_main_window,
            #[cfg(desktop)]
            ensure_main_window,
            #[cfg(desktop)]
            close_main_window,
            calculate_aspect_ratio_size,
            calculate_aspect_ratio_size_preserve_video,
            start_titlebar_drag,
            get_system_info,
            get_emoji_image,
            read_clipboard_text_native,
            // Twitch commands
            twitch_login,
            create_clip,
            get_live_broadcast,
            create_vod_clip,
            begin_clip_edit,
            finalize_clip,
            get_clip_render_status,
            delete_clip,
            twitch_start_device_login,
            twitch_complete_device_login,
            twitch_logout,
            clear_webview_data,
            // Login/subscribe/overlay popup commands are desktop-only (they create
            // webview windows); mobile logs in via the device-code flow.
            #[cfg(desktop)]
            open_twitch_login_window,
            #[cfg(desktop)]
            open_drops_login_window,
            #[cfg(desktop)]
            open_subscribe_window,
            #[cfg(desktop)]
            report_login_popup_url,
            #[cfg(desktop)]
            close_login_overlay,
            #[cfg(desktop)]
            mount_twitch_overlay,
            #[cfg(desktop)]
            set_twitch_overlay_bounds,
            #[cfg(desktop)]
            set_twitch_overlay_visible,
            has_stored_credentials,
            list_twitch_accounts,
            get_twitch_account_count,
            add_twitch_account,
            remove_twitch_account,
            set_active_twitch_account,
            sign_out_active_twitch_account,
            modroom_status,
            modroom_connect,
            modroom_disconnect,
            modroom_list_moderated,
            modroom_get_room_token,
            get_followed_streams,
            get_channel_info,
            get_user_info,
            get_recommended_streams,
            get_recommended_streams_paginated,
            open_browser_url,
            #[cfg(desktop)]
            focus_window,
            get_top_games,
            get_top_games_paginated,
            get_streams_by_game,
            search_channels,
            search_categories,
            get_categories_by_name,
            get_category_info,
            get_user_by_id,
            get_user_by_login,
            get_channel_moderators,
            get_channel_vips,
            follow_channel,
            unfollow_channel,
            check_following_status,
            get_all_followed_channels,
            get_offline_last_broadcasts,
            get_streams_by_user_ids,
            get_users_by_ids,
            verify_token_health,
            force_refresh_token,
            get_twitch_token,
            check_stream_online,
            check_streams_online,
            get_streams_by_game_name,
            get_streams_by_game_id,
            get_streams_by_game_with_tags,
            get_clips_by_game,
            get_clips_by_broadcaster,
            get_clip_reactions,
            get_games_by_ids,
            get_videos_by_game,
            get_user_videos,
            get_vod_comments,
            send_whisper,
            start_whisper_listener,
            get_whisper_history,
            search_whisper_user,
            import_all_whisper_history,
            refresh_whisper_history,
            // Streaming commands
            start_stream,
            resolve_clip_media,
            stop_stream,
            get_ad_detection,
            get_stream_low_latency,
            set_experimental_low_latency,
            set_codec_preference,
            set_max_video_height,
            start_ll_diag,
            append_ll_diag,
            stop_ll_diag,
            get_stream_qualities,
            change_stream_quality,
            rewind_live_stream,
            get_live_rewind_info,
            // VOD watch position
            report_vod_position,
            get_vod_progress,
            clear_vod_progress,
            // Song recognition
            identify_song,
            // Multi-stream commands (desktop-only — not part of the phone app)
            #[cfg(desktop)]
            start_multi_nook,
            #[cfg(desktop)]
            stop_multi_nook,
            #[cfg(desktop)]
            stop_all_multi_nooks,
            #[cfg(desktop)]
            get_active_multi_nooks,
            register_active_channel,
            unregister_active_channel,
            // Chat commands
            start_chat,
            stop_chat,
            restart_chat_bridge,
            validate_platform_sessions,
            platform_account_info,
            get_youtube_channel_emojis,
            commands::streaming::youtube_sabr_probe,
            get_chat_lifecycle_log,
            debug_break_chat_socket,
            debug_unjoin_channel,
            nudge_chat_channels,
            send_chat_message,
            join_chat_channel,
            leave_chat_channel,
            search_chat,
            validate_chat_filter,
            validate_chat_phrase,
            get_chat_rule_errors,
            get_chat_history_stats,
            get_automod_queue,
            resolve_automod_message,
            get_streamer_mode_state,
            update_channel_info,
            get_user_pronouns,
            get_user_note,
            set_user_note,
            upload_image,
            start_multi_chat,
            provider_chat_connect,
            provider_chat_disconnect,
            provider_send_message,
            provider_send_capability,
            provider_source_caps,
            provider_directory,
            provider_search,
            provider_categories,
            provider_channel_meta,
            provider_live_check,
            get_provider_followed_live,
            get_favorite_live,
            refresh_favorites,
            kick_account_sync,
            youtube_account_sync,
            provider_channel_avatars,
            kick_user_profile,
            youtube_user_profile,
            provider_membership,
            log_frontend_diag,
            kick_account_is_synced,
            report_kick_follows,
            report_kick_resolve_diag,
            get_provider_follows,
            provider_follow,
            provider_unfollow,
            report_kick_chatroom,
            report_kick_emotes,
            get_kick_channel_meta,
            get_youtube_channel_meta,
            get_tiktok_channel_meta,
            youtube_connect,
            youtube_disconnect,
            youtube_is_connected,
            youtube_account_name,
            youtube_delete_message,
            youtube_ban_user,
            youtube_unban_user,
            youtube_can_moderate,
            kick_connect,
            kick_disconnect,
            kick_is_connected,
            kick_account_name,
            kick_ban_user,
            kick_unban_user,
            kick_delete_message,
            kick_can_moderate,
            kick_chat_history,
            kick_viewer_state,
            get_kick_channel_emotes,
            get_youtube_channel_emotes,
            load_mod_logs,
            append_mod_log,
            clear_mod_logs,
            parse_historical_messages,
            load_channel_history,
            get_chat_log_dir,
            update_chat_settings,
            clear_chat,
            delete_chat_message,
            pin_chat_message,
            unpin_chat_message,
            ban_user,
            unban_user,
            add_channel_moderator,
            remove_channel_moderator,
            add_channel_vip,
            remove_channel_vip,
            update_suspicious_user_status,
            update_user_chat_color,
            get_user_chat_colors,
            block_user,
            unblock_user,
            get_channel_moderators,
            get_channel_vips,
            get_chatters_by_role,
            get_channel_chatters,
            send_chat_announcement,
            send_shoutout,
            start_commercial,
            start_raid,
            cancel_raid,
            add_blocked_term,
            remove_blocked_term,
            create_poll,
            get_active_poll,
            end_poll,
            create_prediction,
            end_prediction,
            create_stream_marker,
            warn_chat_user,
            update_shield_mode,
            // Discord commands (desktop-only — Discord Rich Presence)
            #[cfg(desktop)]
            connect_discord,
            #[cfg(desktop)]
            disconnect_discord,
            #[cfg(desktop)]
            set_idle_discord_presence,
            #[cfg(desktop)]
            update_discord_presence,
            #[cfg(desktop)]
            clear_discord_presence,
            // Settings commands
            load_settings,
            save_settings,
            get_settings_dir,
            open_settings_folder,
            export_settings,
            import_settings,
            get_current_app_version,
            get_latest_app_version,
            download_and_install_app_update,
            get_release_notes,
            send_test_notification,
            // Badge commands
            fetch_global_badges,
            get_cached_global_badges,
            prefetch_global_badges,
            force_refresh_global_badges,
            get_badge_cache_age,
            get_badges_missing_metadata,
            debug_list_twitch_badges,
            debug_compare_badge_sources,
            fetch_channel_badges,
            // Rust makes every Helix read the page needs; the page never holds
            // the token (get_twitch_credentials retired 2026-09-07).
            helix_get,
            streamnook_api_request,
            get_user_badges,
            // Unified Badge Service commands
            get_user_badges_unified,
            get_user_badges_with_earned_unified,
            get_third_party_badges_for_user_unified,
            parse_badge_string,
            prefetch_global_badges_unified,
            prefetch_channel_badges_unified,
            prefetch_third_party_badges,
            clear_badge_cache_unified,
            clear_channel_badge_cache_unified,
            get_global_badge_collection,
            get_all_third_party_badges,
            get_bttv_pro_badge,
            get_discovered_bttv_pro_badges,
            ingest_badge_drops,
            // Badge Metadata commands
            fetch_badge_metadata,
            // Link preview commands
            fetch_link_preview,
            // Cache commands
            save_emote_by_id,
            load_emote_by_id,
            save_emotes_to_cache,
            load_emotes_from_cache,
            save_badges_to_cache,
            load_badges_from_cache,
            clear_cache,
            get_cache_statistics,
            save_favorite_emotes_cache,
            load_favorite_emotes_cache,
            add_favorite_emote_cache,
            remove_favorite_emote_cache,
            // Universal Cache commands
            get_universal_cached_item,
            save_universal_cached_item,
            sync_universal_cache_data,
            cleanup_universal_cache,
            clear_all_universal_cache,
            get_universal_cache_statistics,
            open_universal_cache_folder,
            assign_badge_positions,
            export_manifest,
            download_and_cache_file,
            get_cached_file,
            get_cached_files,
            get_all_universal_cached_items,
            get_universal_cached_items_batch,
            auto_sync_universal_cache_if_stale,

            // Cosmetics Cache commands
            cache_user_cosmetics,
            get_cached_user_cosmetics,
            cache_third_party_badges,
            get_cached_third_party_badges,
            prefetch_user_cosmetics,
            // Profile Cache commands
            get_user_profile,
            refresh_user_profile,
            clear_profile_cache,
            preload_badge_databases,
            // User Profile commands (unified aggregation)
            get_user_profile_complete,
            clear_user_profile_cache,
            clear_user_profile_cache_for_user,
            // Drops commands
            get_drops_settings,
            update_drops_settings,
            get_active_drop_campaigns,
            refresh_drops_connection_status,
            get_drops_inventory,
            get_drop_progress,
            get_campaign_eligible_channels,
            claim_drop,
            check_channel_points,
            claim_channel_points,
            get_drops_statistics,
            get_claimed_drops,
            get_channel_points_history,
            get_channel_points_balance,
            get_all_channel_points_balances,
            record_channel_points_balance,
            refresh_followed_channel_points,
            start_drops_monitoring,
            stop_drops_monitoring,
            update_monitoring_channel,
            report_player_playing,
            // Automation commands
            // Drops Authentication commands
            start_drops_device_flow,
            poll_drops_token,
            drops_logout,
            is_drops_authenticated,
            validate_drops_token,
            open_drop_details,
            // Prediction commands
            place_prediction,
            get_active_prediction,
            get_channel_points_for_channel,
            // Poll commands
            vote_on_poll,
            // Watch token allocation commands
            // Channel Points Rewards commands
            get_channel_rewards,
            redeem_channel_reward,
            send_highlighted_message,
            unlock_random_emote,
            get_modifiable_emotes,
            unlock_modified_emote,
            unlock_chosen_emote,
            // Component commands
            check_components_installed,
            get_local_component_versions,
            get_remote_component_versions,
            check_for_bundle_update,
            extract_bundled_components,
            download_and_install_bundle,
            restart_to_apply_update,
            // Session resume
            save_resume_snapshot,
            take_resume_snapshot,
            // Announcements
            fetch_announcements,
            fetch_user_chat_logs,
            fetch_user_deep_logs,
            // Layout commands (message history only - height calculation removed)
            get_user_message_history,
            get_user_message_history_limited,
            clear_user_message_history,
            get_user_history_count,
            // Emoji commands
            convert_emoji_shortcodes,
            // Emote commands
            fetch_channel_emotes,
            get_emote_by_name,
            clear_emote_cache,
            get_gif_picker_status,
            search_gifs,
            send_gif_message,
            ffz_local_user_status,
            // Emote prefetch (AFK bulk cache) commands
            emote_prefetch_plan,
            emote_prefetch_start,
            emote_prefetch_stop,
            emote_prefetch_status,
            // 7TV commands
            seventv_graphql,
            seventv_graphql_authed,
            // 7TV Cosmetics commands (paints/badges apply + auth — kept on mobile)
            get_seventv_auth_status,
            get_seventv_login_url,
            store_seventv_token,
            validate_seventv_token,
            logout_seventv,
            set_seventv_paint,
            set_seventv_badge,
            // Login popup is desktop-only (creates a 7TV login webview window)
            #[cfg(desktop)]
            open_seventv_login_window,
            receive_seventv_token,
            // 7TV per-account (linked secondaries)
            get_seventv_auth_status_for,
            validate_seventv_token_for,
            logout_seventv_for,
            set_seventv_paint_for,
            set_seventv_badge_for,
            #[cfg(desktop)]
            open_seventv_login_window_for_account,
            #[cfg(desktop)]
            refresh_seventv_token_for_account,
            // 7TV Global Cosmetics commands
            get_all_seventv_badges,
            get_all_seventv_paints,
            get_seventv_paint_usage,
            // Automation commands (whisper scraper only — desktop-only webview scrape)
            #[cfg(desktop)]
            scrape_whispers,
            #[cfg(desktop)]
            receive_whisper_export,
            emit_whisper_progress,
            // Whisper Storage commands
            load_whisper_storage,
            save_whisper_storage,
            save_whisper_conversation,
            append_whisper_message,
            delete_whisper_conversation,
            get_whisper_storage_path,
            migrate_whispers_from_localstorage,
            // Log commands
            log_message,
            log_messages_batch,
            track_activity,
            get_recent_logs,
            get_logs_by_level,
            get_recent_activity,
            clear_logs,
            // Spawns xdg-open / explorer; there is no file manager to hand a
            // path to on Android, and the button is behind !IS_MOBILE anyway.
            #[cfg(desktop)]
            open_logs_folder,
            // EventSub commands
            connect_eventsub,
            disconnect_eventsub,
            is_eventsub_connected,
            get_eventsub_session_id,
            add_eventsub_moderation,
            remove_eventsub_moderation,
            // Chat Identity commands (desktop-only — webview-scrape badge-loadout editor)
            #[cfg(desktop)]
            fetch_chat_identity_badges,
            #[cfg(desktop)]
            update_chat_identity,
            #[cfg(desktop)]
            receive_badge_data,
            #[cfg(desktop)]
            receive_update_result,
            // Same feature on Android, GQL only — there is no second webview to
            // fall back to, so the scrape half simply does not exist there.
            #[cfg(target_os = "android")]
            fetch_chat_identity_badges_gql,
            #[cfg(target_os = "android")]
            update_chat_identity_gql,
            // StreamNook Identity (badge loadout) commands
            get_streamnook_identity,
            get_streamnook_identities,
            get_streamnook_identity_resolved,
            set_streamnook_identity,
            // Authenticated writes to StreamNook's own API. One command, path-allowlisted.
            streamnook_api_post,
            // Hype Train commands
            get_hype_train_status,
            get_bulk_hype_train_status,
            // Home snapshot (Rust-owned Home/Sidebar data)
            get_home_snapshot,
            set_home_mounted,
            refresh_home_section,
            set_home_extra_channels,
            load_more_home_recommended,
            // Per-channel chat state + user history watches (Rust-owned polls)
            watch_channel_state,
            unwatch_channel_state,
            get_channel_state,
            refresh_channel_state,
            watch_user_history,
            unwatch_user_history,

            // Resub notification commands
            get_resub_notification,
            use_resub_token,
            get_my_subscriptions,
            get_my_past_subscriptions,
            // Channel Panels commands
            get_channel_about_data,
            get_similar_channels,
            // Pinned Chat commands
            get_pinned_chat_messages,
            // Diagnostic Logging commands
            set_diagnostics_enabled,
            is_diagnostics_enabled,
            // Watch Streak commands
            get_watch_streak,
            get_watch_streaks_batch,
            share_watch_streak,
            // Screen capture (Profile share) — desktop-only
            #[cfg(desktop)]
            capture_screen_region,
            #[cfg(desktop)]
            capture_animated_webp,
            // Plugin host commands
            plugins_list,
            plugins_sources,
            plugins_add_source,
            plugins_remove_source,
            plugins_browse_source,
            plugins_begin_install,
            plugins_commit_install,
            plugins_cancel_install,
            plugins_install_local,
            plugins_uninstall,
            plugins_set_enabled,
            plugins_get_panel,
            plugins_set_panel_values,
            plugins_respond_consent,
            plugins_revoke_credential,
            plugins_reset_credential_consent,
            plugins_audit_log,
            plugins_fetch_readme,
            plugins_invoke_action,
            plugins_provides,
            plugins_report_stream_event,
            plugins_ui_bundle,
        ])
        // Window-event handler. Two behaviors:
        //
        // 1. Main window close: hide to the tray instead of exiting, per the
        //    user's Close button preference. The default only hides while
        //    MultiChat popouts are open (quitting would take them with it);
        //    Always hides every time, Never always exits. When we hide, the
        //    process keeps running and popouts stay alive. The JS side then
        //    destroys the hidden main to free its memory, which under Always
        //    with no popouts leaves ZERO windows; the ExitRequested arm in
        //    `.run()` below keeps the process alive for that case.
        //
        // 2. Popout destroyed: when a popout closes, if it was the last
        //    popout AND the main window is currently hidden (i.e. the user
        //    previously closed main to the tray expecting the popouts to keep
        //    the app alive), exit the process. Otherwise the app keeps
        //    running until the user picks "Quit" from the tray.
        .on_window_event(|window, event| {
            // Window-event handling (close-to-tray, MultiChat popout lifecycle) is
            // desktop-only. Mobile has a single window and no tray, so it's a no-op.
            #[cfg(desktop)]
            {
            let label = window.label().to_string();
            let app_handle = window.app_handle().clone();

            if let WindowEvent::Destroyed = event {
                // A destroyed webview never runs its React cleanup, so any
                // chat consumer claims it still holds would pin those channels
                // JOINed (and their IRC traffic flowing) forever. Sweep them;
                // channels with no remaining consumers PART.
                let gone = label.clone();
                tauri::async_runtime::spawn(async move {
                    services::irc_service::IrcService::release_window_claims(&gone, None).await;
                    // Same sweep for the non-Twitch adapters. Their consumers are
                    // window labels too, so without this a closed popout leaves
                    // every Kick/YouTube/TikTok pane's socket and task running and
                    // its BRIDGE_USERS count incremented for the whole session.
                    services::providers::release_window_claims(&gone).await;
                });
                // Tell popouts the main window is gone so their Go Live control
                // flips to "Live Chat" (standalone). Mirrors `main-ready`, which
                // the main window emits when it boots.
                if label == "main" {
                    use tauri::Emitter;
                    let _ = app_handle.emit("main-closed", ());
                }
            }

            if label == "main" {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    let popouts_open = app_handle
                        .webview_windows()
                        .iter()
                        .any(|(l, _)| l.starts_with("multichat-") || l.starts_with("overlay-"));
                    // Settings can be poisoned or momentarily locked; falling
                    // back to the default keeps close working either way.
                    let mode = app_handle
                        .try_state::<AppState>()
                        .and_then(|s| s.settings.lock().ok().map(|g| g.close_to_tray))
                        .unwrap_or_default();
                    let hide_to_tray = match mode {
                        CloseToTrayMode::Always => true,
                        CloseToTrayMode::Never => false,
                        CloseToTrayMode::WithPopouts => popouts_open,
                    };
                    if hide_to_tray {
                        debug!(
                            "[Main] Close requested (mode {:?}, popouts_open {}) — hiding main to tray",
                            mode, popouts_open
                        );
                        api.prevent_close();
                        if let Some(main_win) = app_handle.get_webview_window("main") {
                            // Tell the JS side it's about to go background so
                            // it can stop the active stream (Streamlink + video
                            // + drops monitoring) before we hide. Chat stays
                            // alive because the popouts still need it — the JS
                            // handler is intentionally NOT a full stopStream
                            // (which would tear down the IRC connection too).
                            let _ = main_win.emit("main-hiding-to-tray", ());
                            let _ = main_win.hide();
                        }
                    }
                }

            } else if label.starts_with("multichat-") || label.starts_with("overlay-") {
                if let WindowEvent::Destroyed = event {
                    // Tell the main window this popout is gone so it can
                    // drop the popout's channel set from its tracking and
                    // re-show its own ChatWidget if it was hiding because of
                    // those channels.
                    if let Some(main_win) = app_handle.get_webview_window("main") {
                        let _ = main_win.emit("multichat-popout-closed", &label);
                    }

                    let still_open = app_handle
                        .webview_windows()
                        .iter()
                        .filter(|(l, _)| (l.starts_with("multichat-") || l.starts_with("overlay-")) && **l != label)
                        .count();
                    if still_open == 0 {
                        // Exit when the last popout closes and main is unavailable —
                        // either hidden to tray OR fully destroyed (going live closes
                        // main to free its memory). A destroyed main returns None
                        // here, so treat None as "gone"; otherwise the process would
                        // linger with no windows.
                        //
                        // Neither is a reason to exit when the user asked to
                        // always live in the tray. Under `Always` they expect to
                        // quit from the tray menu, so closing their last popout
                        // must not take the app with it. That covers a destroyed
                        // main too: closing main under `Always` hides it and then
                        // the JS listener destroys it to free memory, so "main is
                        // None" is the ordinary Always tray state, not a stranded
                        // process. The tray recreates it on demand.
                        let always_tray = app_handle
                            .try_state::<AppState>()
                            .and_then(|s| s.settings.lock().ok().map(|g| g.close_to_tray))
                            .unwrap_or_default()
                            == CloseToTrayMode::Always;
                        let should_exit = match app_handle.get_webview_window("main") {
                            Some(main_win) => !main_win.is_visible().unwrap_or(true) && !always_tray,
                            None => !always_tray,
                        };
                        if should_exit {
                            debug!("[Main] Last MultiChat closed while main hidden/closed — exiting");
                            app_handle.exit(0);
                        }
                    }
                }
            }
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            // Last window gone. The runtime fires this with `code: None` when
            // the final window is destroyed (as opposed to `app.exit()`, which
            // carries `Some(code)`) and exits unless we object. Under the
            // "Always minimize" close mode that is the NORMAL tray state: the
            // CloseRequested interception hides main, then the JS
            // `main-hiding-to-tray` listener destroys it to free its ~350 MB,
            // and with no popouts open nothing else is left. Keep the process
            // (and with it the tray icon) alive; the tray recreates main on
            // demand via `show_main_window`. Tray Quit and the updater still
            // exit because they pass an exit code.
            #[cfg(desktop)]
            if let tauri::RunEvent::ExitRequested { code: None, api, .. } = &event {
                let always_tray = app_handle
                    .try_state::<AppState>()
                    .and_then(|s| s.settings.lock().ok().map(|g| g.close_to_tray))
                    .unwrap_or_default()
                    == CloseToTrayMode::Always;
                if always_tray {
                    debug!("[Main] Last window closed under Always close mode — staying in the tray");
                    api.prevent_exit();
                }
            }
            if let tauri::RunEvent::Exit = event {
                // Flush every debounced store before the process dies; each is
                // a no-op when nothing is dirty.
                let _ = commands::settings::flush_settings_now();
                let _ = services::universal_cache_service::flush_manifest_now();
                let _ = services::mod_log_storage_service::ModLogStorageService::flush_now();
                let _ = services::whisper_storage_service::WhisperStorageService::flush_now();
                let _ = services::vod_progress_service::flush_now();
            let _ = services::chat_logger_service::ChatLoggerService::flush_all();
                // Ask running plugin processes to shut down before the app
                // process dies, waiting briefly so well-behaved plugins exit
                // gracefully (stragglers are killed with the supervisor).
                let state = app_handle.state::<AppState>();
                let host = state.plugin_host.clone();
                tauri::async_runtime::block_on(async move {
                    host.shutdown_all().await;
                    for _ in 0..20 {
                        if !host.has_running().await {
                            break;
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    }
                });
            }
        });
}
