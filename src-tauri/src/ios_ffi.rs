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
            crate::flush_debounced_stores();
        }
        "terminating" => {
            info!("[iOS] terminating");
            sn_tauri_headless::host_run_event(sn_tauri_headless::RunEvent::Exit);
        }
        "foreground" | "memory_warning" => info!("[iOS] {state}"),
        other => warn!("[iOS] unknown lifecycle state {other}"),
    }
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
