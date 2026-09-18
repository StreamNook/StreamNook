//! Plugin process supervision: spawn, handshake, request routing, health
//! pings, graceful shutdown, and crash restarts with backoff.
//! Lifecycle rules are frozen in docs/plugins/PROTOCOL.md section 2.

use anyhow::{anyhow, Result};
use log::{debug, error, warn};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::Emitter;
use tokio::io::{BufReader};
use tokio::process::{ChildStdin};
use tokio::sync::{mpsc, oneshot, Mutex as TokioMutex};
use tokio::time::timeout;

use super::registry::InstalledPlugin;
use super::transport::{read_frame, write_frame};
use super::HostInner;

/// Commands the host sends a running plugin's supervisor.
pub enum SupCmd {
    /// Forward an event notification (already filtered by hooks).
    Event { method: String, params: Value },
    /// Send a request to the plugin and return its response (used to invoke a
    /// hooked action the plugin handles). The reply carries the plugin's
    /// result, or an error string.
    Request {
        method: String,
        params: Value,
        reply: oneshot::Sender<std::result::Result<Value, String>>,
    },
    /// Graceful shutdown (disable, uninstall, or app exit).
    Shutdown,
}

/// JSON-RPC error shape for plugin-facing failures (PROTOCOL.md section 5).
pub struct RpcErr {
    pub code: i64,
    pub message: String,
    pub name: &'static str,
    pub retry_after_ms: Option<u64>,
}

impl RpcErr {
    pub fn to_value(&self) -> Value {
        json!({
            "code": self.code,
            "message": self.message,
            "data": { "name": self.name, "retry_after_ms": self.retry_after_ms }
        })
    }
    pub fn capability_denied(detail: &str) -> Self {
        Self { code: -32000, message: format!("capability denied: {detail}"), name: "capability_denied", retry_after_ms: None }
    }
    pub fn consent_denied() -> Self {
        Self { code: -32001, message: "the user declined or revoked consent".into(), name: "consent_denied", retry_after_ms: None }
    }
    pub fn unknown_stream(id: &str) -> Self {
        Self { code: -32002, message: format!("no active relay session for stream '{id}'"), name: "unknown_stream", retry_after_ms: None }
    }
    pub fn rate_limited(retry_after_ms: u64) -> Self {
        Self { code: -32003, message: "rate limited".into(), name: "rate_limited", retry_after_ms: Some(retry_after_ms) }
    }
    pub fn credential_unavailable(detail: &str) -> Self {
        Self { code: -32005, message: format!("credential unavailable: {detail}"), name: "credential_unavailable", retry_after_ms: None }
    }
    pub fn invalid_params(detail: &str) -> Self {
        Self { code: -32602, message: format!("invalid params: {detail}"), name: "invalid_params", retry_after_ms: None }
    }
    pub fn method_not_found(method: &str) -> Self {
        Self { code: -32601, message: format!("method not found: {method}"), name: "method_not_found", retry_after_ms: None }
    }
    pub fn internal(detail: &str) -> Self {
        Self { code: -32603, message: format!("internal error: {detail}"), name: "internal", retry_after_ms: None }
    }
}

enum RunOutcome {
    /// Deliberate stop (disable, uninstall, host shutdown).
    CleanShutdown,
    /// Process died, became unresponsive, or violated the protocol.
    Crashed(String),
}

type Pending = Arc<TokioMutex<HashMap<u64, oneshot::Sender<Value>>>>;
type Writer = Arc<TokioMutex<ChildStdin>>;

/// Outer supervision loop for one plugin: runs the process, restarts with
/// backoff (1s, 5s, 25s; more than 3 restarts in 10 minutes disables the
/// plugin), and exits when the plugin is disabled or the host shuts down.
pub async fn run_supervisor(host: Arc<HostInner>, plugin_id: String) {
    let mut restarts: Vec<Instant> = Vec::new();
    loop {
        if host.shutting_down.load(Ordering::SeqCst) {
            break;
        }
        let record = {
            let registry = host.registry.lock().await;
            registry.plugins.iter().find(|p| p.id == plugin_id).cloned()
        };
        let record = match record {
            Some(r) if r.enabled => r,
            _ => break,
        };

        let outcome = run_once(&host, &record).await;
        host.running.write().await.remove(&plugin_id);

        match outcome {
            RunOutcome::CleanShutdown => break,
            RunOutcome::Crashed(reason) => {
                warn!("[PluginHost] {} crashed: {}", plugin_id, reason);
                restarts.retain(|t| t.elapsed() < Duration::from_secs(600));
                restarts.push(Instant::now());
                if restarts.len() > 3 {
                    error!(
                        "[PluginHost] {} exceeded the restart budget; disabling",
                        plugin_id
                    );
                    let _ = host.set_enabled_in_registry(&plugin_id, false).await;
                    let _ = host.app.emit(
                        "plugin://disabled-after-failures",
                        json!({ "plugin_id": plugin_id, "name": record.name, "reason": reason }),
                    );
                    break;
                }
                let delay = [1u64, 5, 25][restarts.len().saturating_sub(1).min(2)];
                tokio::time::sleep(Duration::from_secs(delay)).await;
            }
        }
    }
    host.running.write().await.remove(&plugin_id);
    debug!("[PluginHost] supervisor for {} ended", plugin_id);
}

/// One full process lifetime: spawn, handshake, serve, shut down.
async fn run_once(host: &Arc<HostInner>, record: &InstalledPlugin) -> RunOutcome {
    let entry_path = Path::new(&record.dir).join(&record.entry);
    let mut command = tokio::process::Command::new(&entry_path);
    command
        .args(&record.args)
        .current_dir(&record.dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        // CREATE_NO_WINDOW: plugins are background processes, never consoles.
        command.creation_flags(0x0800_0000);
    }

    let mut child = match command.spawn() {
        Ok(c) => c,
        Err(e) => return RunOutcome::Crashed(format!("spawn failed: {e}")),
    };
    let stdin = match child.stdin.take() {
        Some(s) => s,
        None => return RunOutcome::Crashed("child stdin unavailable".into()),
    };
    let stdout = match child.stdout.take() {
        Some(s) => s,
        None => return RunOutcome::Crashed("child stdout unavailable".into()),
    };
    if let Some(stderr) = child.stderr.take() {
        spawn_stderr_logger(record.id.clone(), stderr);
    }

    let writer: Writer = Arc::new(TokioMutex::new(stdin));
    let pending: Pending = Arc::new(TokioMutex::new(HashMap::new()));
    let next_id = Arc::new(AtomicU64::new(1));
    let (exit_tx, mut exit_rx) = mpsc::channel::<String>(1);

    // Reader task: routes every incoming frame for this process lifetime.
    {
        let host = host.clone();
        let record = record.clone();
        let writer = writer.clone();
        let pending = pending.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout);
            loop {
                match read_frame(&mut reader).await {
                    Ok(Some(frame)) => {
                        route_frame(&host, &record, &writer, &pending, frame).await;
                    }
                    Ok(None) => {
                        let _ = exit_tx.send("stdout closed".into()).await;
                        break;
                    }
                    Err(e) => {
                        let _ = exit_tx.send(format!("protocol violation: {e}")).await;
                        break;
                    }
                }
            }
        });
    }

    // Handshake: initialize -> result -> initialized.
    let init_params = json!({
        "protocol_version": super::manifest::PROTOCOL_VERSION,
        "host_version": env!("CARGO_PKG_VERSION"),
        "plugin_id": record.id,
        "granted": {
            "events": record.granted.events,
            "host_methods": record.granted.host_methods,
            "credentials": record.granted.credentials,
            "ui": record.granted.ui,
        },
        "data_dir": super::registry::plugin_data_dir(&record.id)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
        "log_dir": super::registry::plugin_state_dir(&record.id)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
    });
    let init_result = match request(&writer, &pending, &next_id, "initialize", init_params, 10).await
    {
        Ok(v) => v,
        Err(e) => {
            let _ = child.kill().await;
            return RunOutcome::Crashed(format!("initialize failed: {e}"));
        }
    };
    let hooks: Vec<String> = init_result
        .get("hooks")
        .and_then(|h| h.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .filter(|h| record.granted.events.contains(h))
                .collect()
        })
        .unwrap_or_default();
    let plugin_version = init_result
        .get("plugin_version")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    if notify_frame(&writer, "initialized", json!({})).await.is_err() {
        let _ = child.kill().await;
        return RunOutcome::Crashed("failed to send initialized".into());
    }

    // Register as running.
    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<SupCmd>();
    {
        let mut running = host.running.write().await;
        running.insert(
            record.id.clone(),
            super::RunningHandle {
                hooks: hooks.iter().cloned().collect(),
                actions: record.granted.actions.iter().cloned().collect(),
                provides: record.granted.provides.iter().cloned().collect(),
                cmd_tx,
                plugin_version: plugin_version.clone(),
            },
        );
    }
    let _ = host.app.emit(
        "plugin://state-changed",
        json!({ "plugin_id": record.id, "running": true }),
    );
    debug!(
        "[PluginHost] {} v{} initialized (hooks: {:?})",
        record.id, plugin_version, hooks
    );

    // Serve until shutdown, crash, or unresponsiveness.
    let ping_misses = Arc::new(AtomicU32::new(0));
    let mut ping_interval = tokio::time::interval(Duration::from_secs(30));
    ping_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ping_interval.tick().await; // consume the immediate first tick

    let outcome = loop {
        tokio::select! {
            cmd = cmd_rx.recv() => match cmd {
                Some(SupCmd::Event { method, params }) => {
                    if notify_frame(&writer, &method, params).await.is_err() {
                        break RunOutcome::Crashed("write failed while sending an event".into());
                    }
                }
                Some(SupCmd::Request { method, params, reply }) => {
                    // Run the request off the select loop so it keeps serving.
                    let writer = writer.clone();
                    let pending = pending.clone();
                    let next_id = next_id.clone();
                    tokio::spawn(async move {
                        let result = request(&writer, &pending, &next_id, &method, params, 30)
                            .await
                            .map_err(|e| e.to_string());
                        let _ = reply.send(result);
                    });
                }
                Some(SupCmd::Shutdown) | None => {
                    let _ = request(&writer, &pending, &next_id, "shutdown", json!(null), 3).await;
                    let _ = notify_frame(&writer, "exit", json!(null)).await;
                    match timeout(Duration::from_secs(5), child.wait()).await {
                        Ok(_) => {}
                        Err(_) => { let _ = child.kill().await; }
                    }
                    break RunOutcome::CleanShutdown;
                }
            },
            reason = exit_rx.recv() => {
                let _ = child.kill().await;
                break RunOutcome::Crashed(reason.unwrap_or_else(|| "process exited".into()));
            }
            _ = ping_interval.tick() => {
                if ping_misses.load(Ordering::SeqCst) >= 3 {
                    let _ = child.kill().await;
                    break RunOutcome::Crashed("unresponsive (3 missed pings)".into());
                }
                let misses = ping_misses.clone();
                let writer = writer.clone();
                let pending = pending.clone();
                let next_id = next_id.clone();
                tokio::spawn(async move {
                    match request(&writer, &pending, &next_id, "ping", json!({}), 25).await {
                        Ok(_) => { misses.store(0, Ordering::SeqCst); }
                        Err(_) => { misses.fetch_add(1, Ordering::SeqCst); }
                    }
                });
            }
        }
    };

    let _ = host.app.emit(
        "plugin://state-changed",
        json!({ "plugin_id": record.id, "running": false }),
    );
    outcome
}

/// Sends a host-to-plugin request and awaits its response.
async fn request(
    writer: &Writer,
    pending: &Pending,
    next_id: &Arc<AtomicU64>,
    method: &str,
    params: Value,
    timeout_secs: u64,
) -> Result<Value> {
    let id = next_id.fetch_add(1, Ordering::SeqCst);
    let (tx, rx) = oneshot::channel::<Value>();
    pending.lock().await.insert(id, tx);
    let frame = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
    {
        let mut w = writer.lock().await;
        if let Err(e) = write_frame(&mut *w, &frame).await {
            pending.lock().await.remove(&id);
            return Err(anyhow!("write failed: {e}"));
        }
    }
    match timeout(Duration::from_secs(timeout_secs), rx).await {
        Ok(Ok(response)) => {
            if let Some(err) = response.get("error") {
                Err(anyhow!("plugin returned an error: {err}"))
            } else {
                Ok(response.get("result").cloned().unwrap_or(Value::Null))
            }
        }
        Ok(Err(_)) => Err(anyhow!("response channel dropped")),
        Err(_) => {
            pending.lock().await.remove(&id);
            Err(anyhow!("timed out after {timeout_secs}s"))
        }
    }
}

/// Sends a host-to-plugin notification.
async fn notify_frame(writer: &Writer, method: &str, params: Value) -> Result<()> {
    let frame = json!({ "jsonrpc": "2.0", "method": method, "params": params });
    let mut w = writer.lock().await;
    write_frame(&mut *w, &frame).await
}

/// Routes one incoming frame from the plugin.
async fn route_frame(
    host: &Arc<HostInner>,
    record: &InstalledPlugin,
    writer: &Writer,
    pending: &Pending,
    frame: Value,
) {
    let has_id = frame.get("id").is_some();
    let method = frame.get("method").and_then(|m| m.as_str()).map(|s| s.to_string());

    match (has_id, method) {
        // Response to one of our requests.
        (true, None) => {
            if let Some(id) = frame.get("id").and_then(|i| i.as_u64()) {
                if let Some(tx) = pending.lock().await.remove(&id) {
                    let _ = tx.send(frame);
                }
            }
        }
        // Host-method request from the plugin.
        (true, Some(method)) => {
            let id = frame.get("id").cloned().unwrap_or(Value::Null);
            let params = frame.get("params").cloned().unwrap_or(Value::Null);
            let host = host.clone();
            let record = record.clone();
            let writer = writer.clone();
            tokio::spawn(async move {
                let response = match super::broker::handle_host_method(
                    &host, &record, &method, params,
                )
                .await
                {
                    Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
                    Err(err) => json!({ "jsonrpc": "2.0", "id": id, "error": err.to_value() }),
                };
                let mut w = writer.lock().await;
                let _ = write_frame(&mut *w, &response).await;
            });
        }
        // Notification from the plugin. `log` and `set_status` are handled;
        // everything else is dropped (notifications cannot receive errors).
        (false, Some(method)) => {
            let params = frame.get("params").cloned().unwrap_or(Value::Null);
            if method == "log" {
                super::broker::handle_log_notification(record, &params);
            } else if method == "set_status" {
                super::broker::handle_set_status(host, record, &params);
            } else {
                debug!(
                    "[PluginHost] {} sent unsupported notification '{}'",
                    record.id, method
                );
            }
        }
        (false, None) => {
            debug!("[PluginHost] {} sent a frame with no method or id", record.id);
        }
    }
}

/// Forwards the plugin's stderr lines into its log file.
fn spawn_stderr_logger(plugin_id: String, stderr: tokio::process::ChildStderr) {
    tokio::spawn(async move {
        use tokio::io::AsyncBufReadExt;
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            super::broker::append_plugin_log(&plugin_id, "stderr", &line);
        }
    });
}
