//! The iPhone app's way into the core, through UniFFI.
//!
//! The Swift app is the shell on iOS. It starts the core once with an event
//! sink and its sandbox directories, then calls commands by name with JSON
//! arguments: the same names and argument objects the desktop frontend sends
//! through Tauri's `invoke`, dispatched by the `generate_handler!` list in
//! `run()`. Everything the core emits arrives at [`HostSink::event`].
//! `tauri` here is sn-tauri-headless (see the crate root).

use log::{debug, info, warn};
use std::sync::{Arc, OnceLock};

/// Implemented in Swift: where the core's events and shell requests go.
#[uniffi::export(foreign)]
pub trait HostSink: Send + Sync {
    /// An event the core emitted. `target` is the window label an `emit_to`
    /// named, or `None` for a broadcast.
    fn event(&self, name: String, payload_json: String, target: Option<String>);
    /// Something only the shell can do (`exit`, `restart`, ...).
    fn host_request(&self, kind: String, payload_json: String);
}

/// The app's sandbox directories as file system paths.
#[derive(uniffi::Record)]
pub struct HostDirs {
    pub data: String,
    pub cache: String,
    pub logs: String,
}

#[derive(Debug, uniffi::Error)]
pub enum CoreError {
    /// A command failed; `json` is its error value, as the desktop frontend
    /// would receive it from a rejected `invoke`.
    Command { json: String },
    /// The core could not start.
    Start { message: String },
}

impl std::fmt::Display for CoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CoreError::Command { json } => write!(f, "command failed: {json}"),
            CoreError::Start { message } => write!(f, "core did not start: {message}"),
        }
    }
}

impl std::error::Error for CoreError {}

struct SinkBridge(Arc<dyn HostSink>);

impl sn_tauri_headless::HostSink for SinkBridge {
    fn event(&self, name: &str, payload_json: String, target: Option<String>) {
        self.0.event(name.to_string(), payload_json, target);
    }

    fn host_request(&self, kind: &str, payload_json: String) {
        self.0.host_request(kind.to_string(), payload_json);
    }
}

/// What the shell hands over at start for later use: the Keychain-held key
/// for the token vault and the codecs the phone decodes in hardware.
struct Launch {
    vault_key: Vec<u8>,
    codecs: Vec<String>,
}

static LAUNCH: OnceLock<Launch> = OnceLock::new();

/// The token vault key the shell keeps in the Keychain, when it passed a
/// 256-bit one.
pub(crate) fn vault_key() -> Option<[u8; 32]> {
    LAUNCH.get().and_then(|l| <[u8; 32]>::try_from(l.vault_key.as_slice()).ok())
}

/// The codecs the shell reported the phone decodes in hardware.
pub(crate) fn launch_codecs() -> Vec<String> {
    LAUNCH.get().map(|l| l.codecs.clone()).unwrap_or_default()
}

/// Starts the core: installs the sink and directories, then runs the same
/// `run()` the desktop and Android apps run, which returns once setup is done.
#[uniffi::export]
pub fn start(
    sink: Arc<dyn HostSink>,
    dirs: HostDirs,
    version: String,
    vault_key: Vec<u8>,
    codecs: Vec<String>,
) -> Result<(), CoreError> {
    let dirs = sn_tauri_headless::HostDirs {
        data: dirs.data.into(),
        cache: dirs.cache.into(),
        logs: dirs.logs.into(),
    };
    for dir in [&dirs.data, &dirs.cache, &dirs.logs] {
        if let Err(e) = std::fs::create_dir_all(dir) {
            return Err(CoreError::Start {
                message: format!("cannot create {}: {e}", dir.display()),
            });
        }
    }
    if !sn_tauri_headless::host_install(Arc::new(SinkBridge(sink)), dirs, version) {
        return Err(CoreError::Start {
            message: "the core is already running".to_string(),
        });
    }
    let _ = LAUNCH.set(Launch { vault_key, codecs });
    // The shell draws chat natively, so rows carry the worked-out view.
    crate::services::chat_row_view::set_enabled(true);
    std::panic::catch_unwind(crate::run).map_err(|panic| CoreError::Start {
        message: panic
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| panic.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "run() panicked".to_string()),
    })
}

/// Runs a command by name. `args_json` is the arguments object (`{}` or empty
/// for none); the result is the command's return value as JSON.
#[uniffi::export]
pub async fn invoke(cmd: String, args_json: String) -> Result<String, CoreError> {
    let call = sn_tauri_headless::host_dispatch(&cmd, &args_json);
    match sn_tauri_headless::async_runtime::spawn(call).await {
        Ok(Ok(json)) => Ok(json),
        Ok(Err(json)) => Err(CoreError::Command { json }),
        Err(e) => Err(CoreError::Command {
            json: serde_json::Value::String(e.to_string()).to_string(),
        }),
    }
}

/// Hands over the cookies the shell's sign-in web view holds for `origin`: a
/// JSON list of `{name, value, domain, path, expires, secure, httpOnly}`,
/// the whole of that origin's cookies each time (see `platform::shell_cookies`).
#[uniffi::export]
pub fn set_web_cookies(origin: String, cookies_json: String) {
    match crate::platform::shell_cookies::replace(&origin, &cookies_json) {
        Ok(held) => debug!("[iOS] {origin}: {held} web cookie(s) held"),
        Err(e) => warn!("[iOS] {origin}: web cookies not taken: {e}"),
    }
}

/// The app's lifecycle as the shell sees it: `foreground`, `background`,
/// `terminating` or `memory_warning`.
#[uniffi::export]
pub fn lifecycle(state: String) {
    match state.as_str() {
        "background" => {
            info!("[iOS] moved to the background; flushing stores");
            // Also flushes the debounced stores.
            crate::services::app_presence::set_foreground(false);
        }
        "terminating" => {
            info!("[iOS] terminating");
            sn_tauri_headless::host_run_event(sn_tauri_headless::RunEvent::Exit);
        }
        "foreground" => {
            info!("[iOS] back in the foreground");
            crate::services::app_presence::set_foreground(true);
            sn_tauri_headless::async_runtime::spawn(foregrounded());
            // A grant (or revocation) may have landed while suspended, with the
            // realtime socket down: re-read the StreamNook registries.
            sn_tauri_headless::async_runtime::spawn(crate::services::cosmetics_registry::refresh());
        }
        "memory_warning" => {
            info!("[iOS] memory warning; dropping the in-memory caches");
            sn_tauri_headless::async_runtime::spawn(purge_memory_caches());
        }
        other => warn!("[iOS] unknown lifecycle state {other}"),
    }
}

/// What the core found on the way back to the foreground, emitted as
/// `app-foregrounded`: whether the relay's listener had to be re-bound (same
/// port, same player URL), and the chat bridge's port and whether it answers,
/// so the shell reconnects its chat socket there.
#[derive(Clone, serde::Serialize)]
struct Foregrounded {
    relay_rebound: bool,
    chat_port: Option<u16>,
    chat_alive: bool,
}

/// iOS may reclaim a suspended app's listening sockets. Re-bind the relay if
/// its listener died, and bring the chat bridge back if its task ended or its
/// listener stopped answering.
async fn foregrounded() {
    use tauri::Emitter;

    let relay_rebound = match crate::services::stream_server::StreamServer::rebind_if_dead().await {
        Ok(rebound) => rebound,
        Err(e) => {
            warn!("[iOS] relay re-bind failed: {e}");
            false
        }
    };
    let mut chat_port = crate::services::irc_service::IrcService::ensure_local_ws_bridge().await.ok();
    let mut chat_alive = match chat_port {
        Some(port) => loopback_answers(port).await,
        None => false,
    };
    if chat_port.is_some() && !chat_alive {
        warn!("[iOS] the chat bridge's listener does not answer after the resume; re-binding it");
        match crate::services::irc_service::IrcService::rebind_local_ws_bridge().await {
            Ok(port) => {
                chat_alive = loopback_answers(port).await;
                chat_port = Some(port);
            }
            Err(e) => warn!("[iOS] chat bridge re-bind failed: {e}"),
        }
    }
    info!("[iOS] foreground: relay re-bound {relay_rebound}, chat port {chat_port:?} answers {chat_alive}");
    if let Some(app) = crate::services::providers::app_handle() {
        let _ = app.emit("app-foregrounded", Foregrounded { relay_rebound, chat_port, chat_alive });
    }
}

async fn loopback_answers(port: u16) -> bool {
    tokio::time::timeout(
        std::time::Duration::from_millis(500),
        tokio::net::TcpStream::connect(("127.0.0.1", port)),
    )
    .await
    .map(|r| r.is_ok())
    .unwrap_or(false)
}

/// The in-memory caches that refill on their own: channel emote sets, badge
/// and profile caches, 7TV cosmetics and pictures, BTTV Pro badges and emoji
/// images. Live chat state (the joined channels' emotes, personal emotes) is
/// kept, so nothing on screen loses what it shows.
async fn purge_memory_caches() {
    use tauri::Manager;

    if let Some(app) = crate::services::providers::app_handle() {
        if let Some(emotes) = app.try_state::<crate::commands::emotes::EmoteServiceState>() {
            emotes.0.read().await.clear_cache().await;
        }
    }
    if let Err(e) = crate::commands::badge_service::clear_badge_cache_unified().await {
        debug!("[iOS] badge caches not cleared: {e}");
    }
    let _ = crate::commands::profile_cache::clear_profile_cache().await;
    crate::services::seventv_cosmetics_resolver::clear();
    crate::services::seventv_cosmetics_resolver::clear_pictures();
    crate::services::bttv_pro_service::clear_cache().await;
    crate::commands::app::clear_emoji_cache();
    info!("[iOS] in-memory caches dropped");
}

/// Asks the shell to open a web link outside the app (`host_request` kind
/// `open_url`, payload `{url}`). Only http and https links are sent.
pub(crate) fn open_url(url: &str) -> Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|e| format!("not a link: {e}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(format!("{} links are not opened", parsed.scheme()));
    }
    sn_tauri_headless::host_request("open_url", &serde_json::json!({ "url": url }));
    Ok(())
}

/// Asks the shell to put text on the clipboard (`host_request` kind
/// `copy_text`, payload `{text}`).
pub(crate) fn copy_text(text: &str) {
    sn_tauri_headless::host_request("copy_text", &serde_json::json!({ "text": text }));
}

/// Asks the shell to offer a file in the share sheet, where Save to Files is
/// one of the choices (`host_request` kind `share`, payload
/// `{path, mime, name}`). The file stays where it is until the next share
/// replaces it.
pub(crate) fn share_file(path: &std::path::Path, mime: &str, name: &str) {
    sn_tauri_headless::host_request(
        "share",
        &serde_json::json!({ "path": path.to_string_lossy(), "mime": mime, "name": name }),
    );
}

/// The error a feature the iPhone app does not offer yet answers with.
pub(crate) fn not_wired(what: &str) -> String {
    warn!("[iOS] {what} is not wired to the iPhone app yet");
    format!("{what} is not available on iPhone yet")
}
