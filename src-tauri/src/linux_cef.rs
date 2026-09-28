//! Process entry for the Linux build, which runs on the Chromium Embedded
//! Framework (see `rt.rs`).
//!
//! Three things about CEF shape `main` and cannot live in `run()`:
//!
//! 1. **This binary is also every Chromium helper.** CEF starts its
//!    renderer, GPU, utility and zygote processes by re-executing the app's
//!    own executable with a `--type=` argument. Such a child must hand itself
//!    to CEF and exit before ANY app code runs: logging, the single-instance
//!    plugin (which would take a GPU process for a second launch), profile
//!    housekeeping. `enter` is therefore the first call in `main`.
//! 2. **The profile lock precedes the plugins.** The runtime opens Chromium's
//!    profile at `Builder::build`, before any plugin initialises, and a second
//!    copy of the app dies on the profile's `SingletonLock` before
//!    `tauri-plugin-single-instance` can forward its argv to the first. So the
//!    forward happens here, before the builder, over the plugin's own D-Bus
//!    interface; the plugin stays registered as the receiving side.
//! 3. **The runtime is X11-only.** A Wayland session without XWayland cannot
//!    show a window, and the failure inside the builder is a panic. Checking
//!    the display first turns that into one sentence on stderr.
//!
//! The Chromium switches themselves are decided in `linux_graphics`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// `identifier` in `tauri.conf.json`. Needed before the Tauri context exists
/// (the CEF cache path and the single-instance D-Bus name derive from it);
/// `tests` pins it to the config.
pub const APP_IDENTIFIER: &str = "com.streamnook.dev";

/// The `streamnook://` scheme, for Chromium's protocol-handler registration.
const DEEP_LINK_SCHEME: &str = "streamnook";

/// What `enter` found this process to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entry {
    /// A CEF helper process; it has already run and `main` returns.
    Helper,
    /// No X11 display can be opened; `main` explains and exits non-zero.
    NoDisplay,
    /// A second launch, forwarded to the running instance; `main` returns.
    Forwarded,
    /// The browser process: carry on into `run()`.
    Browser,
}

/// Lines decided before the logger existed, logged by `run()` once it does.
static REPORTS: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn report(line: String) {
    if let Ok(mut r) = REPORTS.lock() {
        r.push(line);
    }
}

/// Drain the startup lines for the log.
pub fn take_reports() -> Vec<String> {
    REPORTS.lock().map(|mut r| std::mem::take(&mut *r)).unwrap_or_default()
}

/// Chromium's root cache: every webview profile lives under it.
///
/// CEF requires every profile to be the root cache or a child of it; the
/// runtime relocates a profile from anywhere else to a hashed folder under
/// the root (one warning per webview in the log). Tauri gives every webview
/// created without a `data_directory` the folder `<local data>/<identifier>`
/// on Linux, so with the root one level below it (`.../cef`) the main window
/// and every other plain webview share one relocated default profile, and
/// the named sign-in profiles under `profiles/` are used as they are.
///
/// The root is deliberately NOT that forced folder itself: a webview whose
/// `data_directory` equals the root gets a context on Chromium's primary
/// profile, and the runtime's scheme handlers never reach it (`tauri://`
/// answers `ERR_UNKNOWN_URL_SCHEME` and the app never loads).
pub fn cache_root() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(APP_IDENTIFIER)
        .join("cef")
}

/// Where a named webview profile lives (see `platform::webview_store`).
pub fn profiles_root() -> PathBuf {
    cache_root().join("profiles")
}

/// First call in `main`. See the module doc for why each step is here.
pub fn enter() -> Entry {
    let (graphics, graphics_report) = crate::linux_graphics::configure();
    report(graphics_report);

    tauri_runtime_cef::configure(tauri_runtime_cef::CefConfig {
        identifier: APP_IDENTIFIER.to_string(),
        command_line_args: graphics.switches,
        cache_path: Some(cache_root()),
        deep_link_schemes: vec![DEEP_LINK_SCHEME.to_string()],
        // `document.cookie` on the app's own origin (`tauri://localhost`).
        cookieable_schemes: vec!["tauri".to_string()],
        linux_windowing: tauri_runtime_cef::LinuxWindowing::X11,
        ..Default::default()
    });

    if std::env::args().any(|a| a.starts_with("--type=")) {
        tauri_runtime_cef::run_cef_helper_process();
        return Entry::Helper;
    }

    if !x11_display_reachable() {
        return Entry::NoDisplay;
    }

    if forward_to_running_instance() {
        return Entry::Forwarded;
    }

    init_gtk_for_the_tray();
    install_policies();
    Entry::Browser
}

/// The tray icon's menu is a GTK menu (muda), and building it panics unless
/// GTK has been initialised; under wry Tauri's own window layer did that,
/// under CEF nothing does. Done here, on the main thread while the process
/// is still single-threaded, with GTK pinned to X11: the app's windows are
/// X11 windows (the runtime is X11-only), and a GTK menu opened through
/// Wayland could not be placed next to them. The runtime installs its own X
/// error handlers when it starts, after this, so GTK's fatal one does not
/// take over the process.
fn init_gtk_for_the_tray() {
    if std::env::var_os("GDK_BACKEND").is_some_and(|v| v != "x11") {
        report(format!(
            "[LinuxCef] GDK_BACKEND was {:?}; set to x11 for the tray menu (the app's windows are X11)",
            std::env::var("GDK_BACKEND").unwrap_or_default()
        ));
    }
    std::env::set_var("GDK_BACKEND", "x11");
    if let Err(e) = gtk::init() {
        report(format!("[LinuxCef] gtk::init failed ({e}); the tray icon will have no menu"));
    }
}

/// The message `main` prints for `Entry::NoDisplay`.
pub const NO_DISPLAY_MESSAGE: &str = "StreamNook needs an X11 display and none can be opened (DISPLAY is unset or unreachable). On a Wayland desktop, enable XWayland or run xwayland-satellite, then start StreamNook again.";

fn x11_display_reachable() -> bool {
    if std::env::var_os("DISPLAY").is_none_or(|d| d.is_empty()) {
        return false;
    }
    let Ok(xlib) = x11_dl::xlib::Xlib::open() else {
        // No libX11 at all; let the runtime report the real error.
        return true;
    };
    // SAFETY: open and close a display on this thread, nothing else touches it.
    unsafe {
        let display = (xlib.XOpenDisplay)(std::ptr::null());
        if display.is_null() {
            return false;
        }
        (xlib.XCloseDisplay)(display);
    }
    true
}

/// Whether `enter` reached a session D-Bus.
static SESSION_BUS: AtomicBool = AtomicBool::new(false);

/// Whether this process has a session D-Bus. tauri-plugin-single-instance
/// panics at setup without one (an unparsable `DBUS_SESSION_BUS_ADDRESS`,
/// such as `disabled:`), so `run()` registers it only when this is true.
pub fn session_bus_available() -> bool {
    SESSION_BUS.load(Ordering::Relaxed)
}

/// Hand this launch's argv and cwd to a running instance over
/// tauri-plugin-single-instance's D-Bus interface. True when one answered,
/// in which case this process has nothing left to do: the running instance
/// shows its window and feeds any `streamnook://` link to the deep-link
/// plugin, exactly as it does for a forwarded launch on Windows.
fn forward_to_running_instance() -> bool {
    let argv: Vec<String> = std::env::args().collect();
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let (name, path) = single_instance_dbus_names(APP_IDENTIFIER);
    let connection = match zbus::blocking::Connection::session() {
        Ok(connection) => connection,
        Err(e) => {
            report(format!(
                "[LinuxCef] no session D-Bus ({e}); single-instance is off, so a second launch starts a second copy"
            ));
            return false;
        }
    };
    SESSION_BUS.store(true, Ordering::Relaxed);
    let answered = connection
        .call_method(
            Some(name.as_str()),
            path.as_str(),
            Some("org.SingleInstance.DBus"),
            "ExecuteCallback",
            &(argv, cwd),
        )
        .is_ok();
    if answered {
        eprintln!("StreamNook is already running; handed this launch to it.");
    }
    answered
}

/// The bus name and object path the plugin registers for `identifier`: the
/// name is `<identifier>.SingleInstance`, the path is the name with `.` as
/// `/` and `-` as `_`.
fn single_instance_dbus_names(identifier: &str) -> (String, String) {
    let name = format!("{identifier}.SingleInstance");
    let path = format!("/{}", name.replace('.', "/").replace('-', "_"));
    (name, path)
}

/// Popups and permission prompts, decided process-wide. The Tauri builder's
/// `on_new_window` is never consulted by this runtime (a webview that
/// installs one simply gets every popup denied), so the popup rule that
/// `commands::twitch::sign_in_popup` expresses per window is restated here:
/// a provider popup opened by a sign-in overlay that must keep
/// `window.opener` is allowed, as a native Chromium window on the overlay's
/// profile. A `target="_blank"` link or `window.open` on the app's own pages
/// goes to the system browser, as it does on the other desktops. Nothing
/// else opens a window.
///
/// Permissions (microphone, notifications, clipboard read, ...) go to the
/// app's own pages and to nothing a sign-in page could ask for. Without a
/// policy the runtime denies everything, so this only widens it for the app.
fn install_policies() {
    tauri_runtime_cef::set_popup_policy(|request| {
        let label = request.webview_label;
        match popup_decision(label, request.opener_url, request.url) {
            Popup::Native => {
                log::debug!("[overlay] popup from '{label}' -> {}", request.url);
                true
            }
            Popup::SystemBrowser => {
                log::debug!("[LinuxCef] link from '{label}' opened in the browser: {}", request.url);
                if let Err(e) = crate::platform::browser::open_in_browser(request.url) {
                    log::warn!("[LinuxCef] {e}");
                }
                false
            }
            Popup::Refused => {
                log::warn!("[overlay] refused a popup from '{label}' to {}", request.url);
                false
            }
        }
    });
    tauri_runtime_cef::set_permission_policy(|request, responder| {
        let own_page = request.origin.as_ref().is_some_and(|o| is_app_origin(&o.scheme, &o.host));
        if own_page {
            responder.allow();
        } else {
            responder.deny(tauri_runtime_cef::DenyReason::PolicyDenied);
        }
    });
}

/// Sign-in overlays whose provider opens its account picker in a popup, on
/// top of `OPENER_POPUPS_IN_OVERLAY`. Google's picker for the YouTube
/// sign-in arrives from a cross-origin iframe; Windows navigates the overlay
/// to it (`CONTAIN_POPUPS_IN_OVERLAY`), which this runtime has no hook for,
/// so here it opens as a native popup on the overlay's profile and the
/// session lands in the same cookie jar.
const NATIVE_POPUPS_ON_LINUX: &[&str] = &["youtube-login"];

/// What becomes of a popup request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Popup {
    /// A native Chromium window that keeps `window.opener`.
    Native,
    /// Refused, and the URL handed to the system browser.
    SystemBrowser,
    Refused,
}

/// The rule `sign_in_popup` applies, plus the app's own links: a sign-in
/// overlay that needs popups gets them for a provider's https page or a
/// scripted blank window; an http(s) link from one of the app's pages opens
/// in the system browser; nothing else opens.
fn popup_decision(webview_label: &str, opener_url: &str, url: &str) -> Popup {
    let sign_in_overlay = crate::commands::twitch::OPENER_POPUPS_IN_OVERLAY.contains(&webview_label)
        || NATIVE_POPUPS_ON_LINUX.contains(&webview_label);
    if sign_in_overlay {
        return if url.starts_with("https://") || url.is_empty() || url == "about:blank" {
            Popup::Native
        } else {
            Popup::Refused
        };
    }
    let from_app_page = url::Url::parse(opener_url)
        .is_ok_and(|u| is_app_origin(u.scheme(), u.host_str().unwrap_or_default()));
    let web_link = url.starts_with("https://") || url.starts_with("http://");
    if from_app_page && web_link {
        Popup::SystemBrowser
    } else {
        Popup::Refused
    }
}

/// The app's own pages: `tauri://localhost` (the native custom-scheme form
/// on Linux) and `http://tauri.localhost` (the form the runtime also serves).
fn is_app_origin(scheme: &str, host: &str) -> bool {
    scheme == "tauri" || (scheme == "http" && host == "tauri.localhost")
}

/// The main window's download handler. The app's pages save a file through
/// `<a download>` (the profile share image when the clipboard refuses it);
/// without a handler the runtime drops the download, so the file goes
/// straight to the Downloads folder, next to any file of the same name
/// rather than over it.
pub fn on_download(_webview: crate::rt::Webview, event: tauri::webview::DownloadEvent<'_>) -> bool {
    match event {
        tauri::webview::DownloadEvent::Requested { destination, .. } => {
            let folder = dirs::download_dir()
                .or_else(|| dirs::home_dir().map(|h| h.join("Downloads")))
                .unwrap_or_else(std::env::temp_dir);
            if let Err(e) = std::fs::create_dir_all(&folder) {
                log::warn!("[LinuxCef] download refused, no folder {}: {e}", folder.display());
                return false;
            }
            *destination = download_path(&folder, destination);
            log::info!("[LinuxCef] saving a download to {}", destination.display());
            true
        }
        tauri::webview::DownloadEvent::Finished { path, success, .. } => {
            log::info!("[LinuxCef] download finished ok={success} path={path:?}");
            true
        }
        _ => true,
    }
}

/// `folder` plus the suggested file name (its last component only), with
/// ` (1)`, ` (2)`, ... before the extension while that name is taken.
fn download_path(folder: &Path, suggested: &Path) -> PathBuf {
    let name = suggested
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "download".to_string());
    let candidate = folder.join(&name);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem.to_string(), format!(".{ext}")),
        _ => (name.clone(), String::new()),
    };
    (1..)
        .map(|n| folder.join(format!("{stem} ({n}){ext}")))
        .find(|p| !p.exists())
        .unwrap_or(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_identifier_is_the_one_in_tauri_conf() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).expect("tauri.conf.json parses");
        assert_eq!(conf["identifier"].as_str(), Some(APP_IDENTIFIER));
    }

    #[test]
    fn the_deep_link_scheme_is_the_one_in_tauri_conf() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).expect("tauri.conf.json parses");
        let schemes = &conf["plugins"]["deep-link"]["desktop"]["schemes"];
        assert!(schemes.as_array().is_some_and(|s| s.iter().any(|v| v == DEEP_LINK_SCHEME)));
    }

    #[test]
    fn the_dbus_names_follow_the_plugin() {
        let (name, path) = single_instance_dbus_names("com.streamnook.dev");
        assert_eq!(name, "com.streamnook.dev.SingleInstance");
        assert_eq!(path, "/com/streamnook/dev/SingleInstance");
        let (_, path) = single_instance_dbus_names("com.example.my-app");
        assert_eq!(path, "/com/example/my_app/SingleInstance");
    }

    #[test]
    fn only_a_sign_in_overlay_that_needs_popups_gets_them() {
        let google = "https://accounts.google.com/v3/signin";
        assert_eq!(popup_decision("tiktok-login", google, "https://appleid.apple.com/auth/authorize"), Popup::Native);
        assert_eq!(popup_decision("tiktok-login", google, "about:blank"), Popup::Native);
        assert_eq!(popup_decision("tiktok-login", google, ""), Popup::Native);
        assert_eq!(popup_decision("tiktok-login", google, "http://example.com"), Popup::Refused);
        assert_eq!(popup_decision("youtube-login", google, "https://accounts.google.com/o/oauth2"), Popup::Native);
        assert_eq!(popup_decision("youtube-login", google, "javascript:void(0)"), Popup::Refused);
        for label in ["twitch-login", "kick-login", "drops-login", "subscribe-abc-123"] {
            let decision = popup_decision(label, "https://www.twitch.tv/", "https://www.twitch.tv/p/terms");
            assert_eq!(decision, Popup::Refused, "{label}");
        }
    }

    #[test]
    fn the_apps_own_links_open_in_the_system_browser() {
        let app = "tauri://localhost/#/settings";
        assert_eq!(popup_decision("main", app, "https://twitch.tv/streamnook"), Popup::SystemBrowser);
        assert_eq!(popup_decision("multichat-default", "http://tauri.localhost/", "http://example.com"), Popup::SystemBrowser);
        assert_eq!(popup_decision("main", app, "about:blank"), Popup::Refused);
        assert_eq!(popup_decision("main", app, "file:///etc/passwd"), Popup::Refused);
        assert_eq!(popup_decision("main", app, "streamnook://watch/x"), Popup::Refused);
        // A remote page in an app window, or a hidden harvest page, never
        // reaches the browser.
        assert_eq!(popup_decision("main", "https://www.youtube.com/embed/x", "https://ads.example"), Popup::Refused);
        assert_eq!(popup_decision("tiktok-feed", "https://www.tiktok.com/live", "https://www.tiktok.com/x"), Popup::Refused);
        assert_eq!(popup_decision("main", "", "https://twitch.tv"), Popup::Refused);
    }

    #[test]
    fn a_download_never_overwrites_a_file() {
        let dir = std::env::temp_dir().join(format!("sn-download-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let first = download_path(&dir, Path::new("profile.png"));
        assert_eq!(first, dir.join("profile.png"));
        std::fs::write(&first, b"x").unwrap();
        assert_eq!(download_path(&dir, Path::new("profile.png")), dir.join("profile (1).png"));
        std::fs::write(dir.join("profile (1).png"), b"x").unwrap();
        assert_eq!(download_path(&dir, Path::new("profile.png")), dir.join("profile (2).png"));
        // Only the file name counts: a path in the suggestion cannot leave the folder.
        assert_eq!(download_path(&dir, Path::new("../../evil.sh")), dir.join("evil.sh"));
        assert_eq!(download_path(&dir, Path::new("")), dir.join("download"));
        std::fs::write(dir.join("README"), b"x").unwrap();
        assert_eq!(download_path(&dir, Path::new("README")), dir.join("README (1)"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn permissions_go_to_the_apps_own_origin_only() {
        assert!(is_app_origin("tauri", "localhost"));
        assert!(is_app_origin("http", "tauri.localhost"));
        assert!(!is_app_origin("https", "www.twitch.tv"));
        assert!(!is_app_origin("http", "localhost"));
    }
}
