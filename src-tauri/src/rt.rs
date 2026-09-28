//! The Tauri runtime this app runs on, named once.
//!
//! Every runtime-generic Tauri type (`AppHandle<R>`, `Window<R>`, ...) is
//! written through the aliases below instead of Tauri's `R = Wry` default, so
//! the runtime is one decision made here rather than an assumption repeated in
//! every command signature. A target that needs a different runtime changes
//! `Rt` alone. Code that must stay generic over the runtime (plugins, helpers
//! shared with other runtimes) keeps naming `R: tauri::Runtime` as before.

/// The runtime behind every app handle, window and webview in this crate.
///
/// Linux runs on the Chromium Embedded Framework: Tauri's own Linux runtime
/// is WebKitGTK, which cannot decode H.264 without host GStreamer plugins and
/// re-walks its compositing tree on every animated frame. Windows, macOS and
/// Android keep wry (WebView2, WKWebView, the Android System WebView). The
/// `Rt` alias is the only place the choice is made; `Cargo.toml` mirrors it
/// with per-target `tauri` tables (wry off on Linux).
#[cfg(target_os = "linux")]
pub type Rt = tauri_runtime_cef::CefRuntime<tauri::EventLoopMessage>;
#[cfg(not(target_os = "linux"))]
pub type Rt = tauri::Wry;

pub type App = tauri::App<Rt>;
pub type AppHandle = tauri::AppHandle<Rt>;
pub type Builder = tauri::Builder<Rt>;
pub type Window = tauri::Window<Rt>;
pub type Webview = tauri::Webview<Rt>;
pub type WebviewWindow = tauri::WebviewWindow<Rt>;
/// `M` is whatever manager the builder was started from (an app handle, a
/// window); call sites leave it to inference, exactly as with the Tauri type.
pub type WebviewWindowBuilder<'a, M> = tauri::WebviewWindowBuilder<'a, Rt, M>;
pub type NewWindowResponse = tauri::webview::NewWindowResponse<Rt>;
pub type TauriPlugin = tauri::plugin::TauriPlugin<Rt>;
