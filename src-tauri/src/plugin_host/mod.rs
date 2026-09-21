//! Out-of-process plugin host. Spawns separate plugin executables, talks
//! JSON-RPC 2.0 over their stdio, enforces the capability model, and brokers
//! credentials with consent. The contract lives in docs/plugins/.

pub mod broker;
pub mod events;
pub mod install;
pub mod manifest;
pub mod process;
pub mod registry;
pub mod signing;
pub mod transport;

use anyhow::{anyhow, bail, Result};
use log::debug;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tauri::{AppHandle, Emitter};
use tokio::sync::{mpsc, oneshot, Mutex as TokioMutex, RwLock as TokioRwLock};

use process::SupCmd;
use registry::{Registry, SourceEntry};

/// One source's catalogue, as the marketplace should present it.
///
/// `entries` is already filtered to what this platform can install.
/// `unavailable_here` is how many were removed by that filter, which is the
/// only thing that lets the UI tell "nothing reachable" apart from "nothing
/// built for you yet". Without it both look identical: an empty list.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceListing {
    pub entries: Vec<install::IndexEntry>,
    pub unavailable_here: usize,
    /// The `<os>-<arch>` key the filter tested against, so the UI can name the
    /// platform it is talking about rather than guessing from the user agent.
    pub platform: String,
}

/// What the user answered on a consent prompt.
#[derive(Debug, Clone, Copy)]
pub enum ConsentDecision {
    Allow,
    Always,
    Deny,
}

#[derive(Debug, Clone)]
pub struct ActiveChannel {
    pub channel_id: String,
    pub login: String,
}

/// A plugin process that completed its handshake.
pub struct RunningHandle {
    pub hooks: HashSet<String>,
    /// Actions this plugin handles and features it provides (the hooks it
    /// fills), used to route action calls and light up UI generically.
    pub actions: HashSet<String>,
    pub provides: HashSet<String>,
    pub cmd_tx: mpsc::UnboundedSender<SupCmd>,
    pub plugin_version: String,
}

pub struct HostInner {
    pub app: AppHandle,
    pub registry: TokioMutex<Registry>,
    pub running: TokioRwLock<HashMap<String, RunningHandle>>,
    pub consent_pending: TokioMutex<HashMap<String, oneshot::Sender<ConsentDecision>>>,
    /// Verified-but-unconsented installs awaiting the consent dialog,
    /// keyed by a one-time token (install step 6 in SIGNING.md).
    pub pending_installs: TokioMutex<HashMap<String, PendingInstall>>,
    pub active_channel: TokioRwLock<Option<ActiveChannel>>,
    pub notify_stamps: TokioMutex<HashMap<String, Vec<Instant>>>,
    pub shutting_down: AtomicBool,
}

pub struct PendingInstall {
    pub source_url: String,
    pub prepared: install::PreparedInstall,
}

impl HostInner {
    /// True when at least one running plugin subscribed to the event.
    pub async fn any_hook(&self, event: &str) -> bool {
        self.running
            .read()
            .await
            .values()
            .any(|h| h.hooks.contains(event))
    }

    /// Fans an event out to every running plugin that subscribed to it.
    pub async fn emit_event(&self, event: &str, params: Value) {
        let running = self.running.read().await;
        for handle in running.values().filter(|h| h.hooks.contains(event)) {
            let _ = handle.cmd_tx.send(SupCmd::Event {
                method: event.to_string(),
                params: params.clone(),
            });
        }
    }

    /// Sends an event to one specific plugin if it subscribed.
    pub async fn emit_event_to(&self, plugin_id: &str, event: &str, params: Value) {
        let running = self.running.read().await;
        if let Some(handle) = running.get(plugin_id) {
            if handle.hooks.contains(event) {
                let _ = handle.cmd_tx.send(SupCmd::Event {
                    method: event.to_string(),
                    params,
                });
            }
        }
    }

    pub async fn set_enabled_in_registry(&self, plugin_id: &str, enabled: bool) -> Result<()> {
        let mut reg = self.registry.lock().await;
        let plugin = reg
            .plugins
            .iter_mut()
            .find(|p| p.id == plugin_id)
            .ok_or_else(|| anyhow!("plugin '{plugin_id}' is not installed"))?;
        plugin.enabled = enabled;
        registry::save(&reg)?;
        Ok(())
    }

    pub async fn set_credential_consent(
        &self,
        plugin_id: &str,
        kind: &str,
        state: &str,
    ) -> Result<()> {
        let mut reg = self.registry.lock().await;
        let plugin = reg
            .plugins
            .iter_mut()
            .find(|p| p.id == plugin_id)
            .ok_or_else(|| anyhow!("plugin '{plugin_id}' is not installed"))?;
        plugin
            .credential_consent
            .insert(kind.to_string(), state.to_string());
        registry::save(&reg)?;
        Ok(())
    }
}

/// Serializable view of an installed plugin for the React UI.
#[derive(Serialize, Clone)]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub tier: String,
    pub description: String,
    pub homepage: Option<String>,
    pub enabled: bool,
    pub running: bool,
    /// Runtime kind: "process" or "ui".
    pub kind: String,
    /// True when the plugin contributes an in-app UI module the frontend
    /// loads: a ui plugin, or a process plugin with a ui_entry (hybrid).
    pub has_ui: bool,
    pub source: String,
    pub granted: registry::GrantedCaps,
    pub credential_consent: HashMap<String, String>,
    pub has_panel: bool,
}

#[derive(Serialize, Clone)]
pub struct SourceInfo {
    pub url: String,
    pub name: String,
    pub operator: String,
    pub fingerprint: String,
    pub official: bool,
}

/// Seeds, or re-pins, the built-in StreamNook source so its curated catalog is
/// available with no manual add. If the user had already added the same URL as
/// a community source, it is upgraded in place (marked built-in, re-pinned to
/// the bundled operator key). Returns true when the registry changed.
fn ensure_official_source(registry: &mut Registry) -> bool {
    let Some((url, pubkey)) = install::OFFICIAL_INDEX else {
        return false;
    };
    if let Some(existing) = registry.sources.iter_mut().find(|s| s.url == url) {
        let changed = !existing.official || existing.operator_pubkey != pubkey;
        existing.official = true;
        existing.operator_pubkey = pubkey.to_string();
        changed
    } else {
        registry.sources.push(SourceEntry {
            url: url.to_string(),
            name: "StreamNook".to_string(),
            operator: "StreamNook".to_string(),
            operator_pubkey: pubkey.to_string(),
            official: true,
        });
        true
    }
}

/// Upgrades any legacy "ask" credential consent (prompt every session) to
/// "always". Enabling a plugin, with the consent shown at install or first
/// enable, is the grant; re-prompting each launch was too much. Returns true
/// when something changed.
fn migrate_credential_consent(registry: &mut Registry) -> bool {
    let mut changed = false;
    for plugin in &mut registry.plugins {
        for state in plugin.credential_consent.values_mut() {
            if state == "ask" {
                *state = "always".to_string();
                changed = true;
            }
        }
    }
    changed
}

pub struct PluginHost {
    inner: Arc<HostInner>,
}

impl PluginHost {
    pub fn new(app: AppHandle) -> Self {
        Self {
            inner: Arc::new(HostInner {
                app,
                registry: TokioMutex::new(Registry::default()),
                running: TokioRwLock::new(HashMap::new()),
                consent_pending: TokioMutex::new(HashMap::new()),
                pending_installs: TokioMutex::new(HashMap::new()),
                active_channel: TokioRwLock::new(None),
                notify_stamps: TokioMutex::new(HashMap::new()),
                shutting_down: AtomicBool::new(false),
            }),
        }
    }

    /// Loads the registry, starts every enabled plugin, starts the emitters.
    pub async fn startup(&self) {
        match registry::load() {
            Ok(mut loaded) => {
                // Seed the built-in StreamNook source so its curated catalog is
                // available immediately, with no manual add; and upgrade any
                // legacy per-session credential consent to the granted-at-enable
                // model so plugins are not re-prompted every launch.
                let mut dirty = ensure_official_source(&mut loaded);
                dirty |= migrate_credential_consent(&mut loaded);
                if dirty {
                    if let Err(e) = registry::save(&loaded) {
                        log::error!("[PluginHost] failed to persist registry on startup: {e}");
                    }
                }
                // Only process plugins get a supervisor; ui plugins are
                // loaded by the frontend, which reads the registry itself.
                let enabled: Vec<String> = loaded
                    .plugins
                    .iter()
                    .filter(|p| p.enabled && p.kind == "process")
                    .map(|p| p.id.clone())
                    .collect();
                *self.inner.registry.lock().await = loaded;
                for id in enabled {
                    self.spawn_supervisor(id);
                }
                // Startup runs concurrently with the webview boot; this ping
                // makes a window that listed plugins before the registry was
                // ready re-sync (the ui-plugin loader listens for it).
                let _ = self.inner.app.emit(
                    "plugin://state-changed",
                    json!({ "plugin_id": Value::Null, "running": false }),
                );
            }
            Err(e) => {
                log::error!("[PluginHost] registry load failed: {e}");
            }
        }
        events::start_background_emitters(self.inner.clone());
        debug!("[PluginHost] started");
    }

    fn spawn_supervisor(&self, plugin_id: String) {
        let inner = self.inner.clone();
        tauri::async_runtime::spawn(async move {
            process::run_supervisor(inner, plugin_id).await;
        });
    }

    pub async fn list_installed(&self) -> Vec<PluginInfo> {
        let registry = self.inner.registry.lock().await;
        let running = self.inner.running.read().await;
        registry
            .plugins
            .iter()
            .map(|p| PluginInfo {
                id: p.id.clone(),
                name: p.name.clone(),
                version: p.version.clone(),
                author: p.author.clone(),
                tier: p.tier.clone(),
                description: p.description.clone(),
                homepage: p.homepage.clone(),
                enabled: p.enabled,
                // A ui plugin has no process; enabled means the frontend
                // loads it, which is its running state.
                running: if p.kind == "ui" {
                    p.enabled
                } else {
                    running.contains_key(&p.id)
                },
                kind: p.kind.clone(),
                has_ui: p.kind == "ui" || p.ui_entry.is_some(),
                source: p.source.clone(),
                granted: p.granted.clone(),
                credential_consent: p.credential_consent.clone(),
                has_panel: registry::panel_schema_path(&p.id)
                    .map(|path| path.exists())
                    .unwrap_or(false),
            })
            .collect()
    }

    pub async fn sources(&self) -> Vec<SourceInfo> {
        let registry = self.inner.registry.lock().await;
        registry
            .sources
            .iter()
            .map(|s| SourceInfo {
                url: s.url.clone(),
                name: s.name.clone(),
                operator: s.operator.clone(),
                fingerprint: signing::key_fingerprint(&s.operator_pubkey),
                official: s.official,
            })
            .collect()
    }

    /// Verifies and pins a new community source. Returns its info (with the
    /// fingerprint the add-source dialog already showed).
    pub async fn add_source(&self, url: &str) -> Result<SourceInfo> {
        if !url.starts_with("https://") {
            bail!("sources must be https URLs");
        }
        {
            let registry = self.inner.registry.lock().await;
            if registry.sources.iter().any(|s| s.url == url) {
                bail!("this source is already added");
            }
        }
        let (entry, fingerprint) = install::probe_source(url).await?;
        let info = SourceInfo {
            url: entry.url.clone(),
            name: entry.name.clone(),
            operator: entry.operator.clone(),
            fingerprint,
            official: false,
        };
        let mut registry = self.inner.registry.lock().await;
        registry.sources.push(entry);
        registry::save(&registry)?;
        Ok(info)
    }

    pub async fn remove_source(&self, url: &str) -> Result<()> {
        let mut registry = self.inner.registry.lock().await;
        if registry.sources.iter().any(|s| s.url == url && s.official) {
            bail!("the built-in StreamNook source cannot be removed");
        }
        let before = registry.sources.len();
        registry.sources.retain(|s| s.url != url);
        if registry.sources.len() == before {
            bail!("source not found");
        }
        registry::save(&registry)?;
        Ok(())
    }

    /// Lists what a source offers (verified fetch, no install).
    ///
    /// Returns the count of entries that exist but have no build for this
    /// platform alongside the ones that do, because the two empty states need
    /// different words. "Your sources are unreachable" and "these plugins have
    /// no build for your platform yet" send a user to debug completely
    /// different things, and the listing alone cannot tell them apart: both
    /// arrive as an empty vector.
    ///
    /// This became load-bearing the moment Linux existed. Every plugin in the
    /// official index declares windows and macos builds and no linux one, so a
    /// Linux user got an empty marketplace whose copy blamed their network.
    pub async fn browse_source(&self, url: &str) -> Result<SourceListing> {
        let pinned = {
            let registry = self.inner.registry.lock().await;
            registry
                .sources
                .iter()
                .find(|s| s.url == url)
                .map(|s| s.operator_pubkey.clone())
                .ok_or_else(|| anyhow!("source not found; add it first"))?
        };
        let (doc, _) = install::fetch_index(url, Some(&pinned)).await?;
        // Filter the LISTING, not just the install step. A plugin with no build
        // for this platform is not "available but broken" - it is not
        // available, and showing it only to fail at the final click is worse
        // than never showing it. Also keeps other platforms' artifacts out of
        // the payload entirely, alongside `skip_serializing` on the map itself.
        //
        // The COUNT of what was filtered still travels, because hiding the
        // entries and hiding the reason are different decisions. The entries
        // themselves do not.
        let platform = install::current_platform_key();
        let total = doc.plugins.len();
        let entries = install::installable_on(doc.plugins, &platform);
        Ok(SourceListing {
            unavailable_here: total - entries.len(),
            entries,
            platform,
        })
    }

    /// Install step 1: download, verify, and unpack from a source (SIGNING.md
    /// steps 1 through 5). Nothing is registered and no plugin code can run;
    /// the returned token and record feed the consent dialog, then the UI
    /// either commits or cancels. The record's `granted` set is the manifest's
    /// requested capabilities, which is exactly what the dialog must render.
    pub async fn begin_install(
        &self,
        source_url: &str,
        plugin_id: &str,
    ) -> Result<(String, registry::InstalledPlugin)> {
        let (source, pinned_author) = {
            let registry = self.inner.registry.lock().await;
            if registry
                .plugins
                .iter()
                .any(|p| p.id == plugin_id && p.enabled)
            {
                bail!("disable the existing version of this plugin first");
            }
            let source = registry
                .sources
                .iter()
                .find(|s| s.url == source_url)
                .cloned()
                .ok_or_else(|| anyhow!("source not found; add it first"))?;
            // Author key is pinned per author handle.
            let pinned = registry
                .plugins
                .iter()
                .find(|p| p.id == plugin_id)
                .map(|p| p.author.clone())
                .and_then(|author| registry.author_keys.get(&author).cloned());
            (source, pinned)
        };
        let prepared =
            install::prepare_install(&source, plugin_id, pinned_author.as_deref()).await?;
        let record = prepared.record.clone();
        let token = uuid::Uuid::new_v4().to_string();
        self.inner.pending_installs.lock().await.insert(
            token.clone(),
            PendingInstall {
                source_url: source_url.to_string(),
                prepared,
            },
        );
        Ok((token, record))
    }

    /// Install step 2 after the user consented: pin keys and register the
    /// plugin (disabled; enabling stays a separate explicit action).
    pub async fn commit_install(&self, token: &str) -> Result<PluginInfo> {
        let pending = self
            .inner
            .pending_installs
            .lock()
            .await
            .remove(token)
            .ok_or_else(|| anyhow!("install session not found (it may have been cancelled)"))?;
        let plugin_id = pending.prepared.record.id.clone();
        install::promote_staging(&pending.prepared.staging, &plugin_id)?;

        let mut registry = self.inner.registry.lock().await;
        if let Some(new_key) = pending.prepared.repin_operator_key {
            if let Some(s) = registry
                .sources
                .iter_mut()
                .find(|s| s.url == pending.source_url)
            {
                s.operator_pubkey = new_key;
            }
        }
        let (author_name, author_key) = pending.prepared.pin_author_key;
        registry.author_keys.insert(author_name, author_key);
        registry.plugins.retain(|p| p.id != plugin_id);
        registry.plugins.push(pending.prepared.record);
        registry::save(&registry)?;
        drop(registry);

        Ok(self
            .find_info(&plugin_id)
            .await
            .expect("freshly installed plugin must list"))
    }

    /// The user declined the consent dialog: discard the staged artifact.
    /// The live pkg dir (an installed older version, if any) is untouched.
    pub async fn cancel_install(&self, token: &str) -> Result<()> {
        let pending = self.inner.pending_installs.lock().await.remove(token);
        if let Some(pending) = pending {
            let _ = std::fs::remove_dir_all(&pending.prepared.staging);
        }
        Ok(())
    }

    /// Development install from a local folder (no signature chain; labeled
    /// local-dev in the UI; consent and capabilities apply unchanged).
    pub async fn install_local(&self, dir: &str) -> Result<PluginInfo> {
        let record = install::prepare_local_install(dir)?;
        let id = record.id.clone();
        let mut registry = self.inner.registry.lock().await;
        if registry.plugins.iter().any(|p| p.id == id && p.enabled) {
            bail!("disable the existing version of this plugin first");
        }
        registry.plugins.retain(|p| p.id != id);
        registry.plugins.push(record);
        registry::save(&registry)?;
        drop(registry);
        Ok(self
            .find_info(&id)
            .await
            .expect("freshly installed plugin must list"))
    }

    pub async fn uninstall(&self, plugin_id: &str) -> Result<()> {
        self.set_enabled(plugin_id, false).await?;
        // Give a running process a moment to begin its graceful shutdown
        // before its files disappear.
        for _ in 0..50 {
            if !self.inner.running.read().await.contains_key(plugin_id) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        let mut registry = self.inner.registry.lock().await;
        registry.plugins.retain(|p| p.id != plugin_id);
        registry::save(&registry)?;
        drop(registry);
        // Delete the plugin's folder from disk: the unpacked pkg (its exe and
        // assets), logs, and panel state. A local-dev source folder lives
        // outside it and is never touched. On Windows the just-stopped process
        // can hold its exe a moment longer, so retry briefly rather than
        // leaving the folder behind.
        if let Ok(dir) = registry::plugin_state_dir(plugin_id) {
            let mut removed = false;
            for _ in 0..20 {
                match std::fs::remove_dir_all(&dir) {
                    Ok(_) => {
                        removed = true;
                        break;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        removed = true;
                        break;
                    }
                    Err(_) => tokio::time::sleep(std::time::Duration::from_millis(100)).await,
                }
            }
            if !removed {
                log::warn!(
                    "[PluginHost] could not fully remove {plugin_id}'s folder; files may still be locked"
                );
            }
        }
        Ok(())
    }

    pub async fn set_enabled(&self, plugin_id: &str, enabled: bool) -> Result<()> {
        let (kind, has_ui) = {
            let mut registry = self.inner.registry.lock().await;
            let plugin = registry
                .plugins
                .iter_mut()
                .find(|p| p.id == plugin_id)
                .ok_or_else(|| anyhow!("plugin '{plugin_id}' is not installed"))?;
            plugin.enabled = enabled;
            let info = (plugin.kind.clone(), plugin.kind == "ui" || plugin.ui_entry.is_some());
            registry::save(&registry)?;
            info
        };
        // The process part: spawn or shut down the sidecar. A pure ui plugin
        // has none. A hybrid (process + ui_entry) plugin runs both.
        if kind == "process" {
            if enabled {
                if !self.inner.running.read().await.contains_key(plugin_id) {
                    self.spawn_supervisor(plugin_id.to_string());
                }
            } else if let Some(handle) = self.inner.running.read().await.get(plugin_id) {
                let _ = handle.cmd_tx.send(SupCmd::Shutdown);
            }
        }
        // The UI part: the frontend loader listens for this signal and loads
        // or unloads the module in each window. Emitted for any plugin that
        // contributes UI, so a pure-ui plugin loads even though no process
        // event would otherwise fire.
        if has_ui {
            let _ = self.inner.app.emit(
                "plugin://state-changed",
                serde_json::json!({ "plugin_id": plugin_id, "running": enabled }),
            );
        }
        Ok(())
    }

    /// Reads the in-app UI module of an enabled plugin for the frontend loader
    /// (UI_PLUGINS.md): a ui plugin's `entry`, or a hybrid process plugin's
    /// `ui_entry`. Fails closed when the plugin has no UI or is not enabled.
    pub async fn ui_bundle(&self, plugin_id: &str) -> Result<String> {
        let (dir, module) = {
            let registry = self.inner.registry.lock().await;
            let plugin = registry
                .plugins
                .iter()
                .find(|p| p.id == plugin_id)
                .ok_or_else(|| anyhow!("plugin '{plugin_id}' is not installed"))?;
            if !plugin.enabled {
                bail!("plugin '{plugin_id}' is not enabled");
            }
            let module = if plugin.kind == "ui" {
                plugin.entry.clone()
            } else {
                plugin
                    .ui_entry
                    .clone()
                    .ok_or_else(|| anyhow!("plugin '{plugin_id}' has no UI module"))?
            };
            (plugin.dir.clone(), module)
        };
        let path = std::path::Path::new(&dir).join(&module);
        std::fs::read_to_string(&path)
            .map_err(|e| anyhow!("failed to read the plugin module {}: {e}", path.display()))
    }

    pub async fn get_panel(&self, plugin_id: &str) -> Option<Value> {
        let schema = registry::panel_schema_path(plugin_id)
            .ok()
            .and_then(|p| registry::read_json_file(&p))?;
        let values = registry::panel_values_path(plugin_id)
            .ok()
            .and_then(|p| registry::read_json_file(&p))
            .unwrap_or_else(|| json!({}));
        Some(json!({ "schema": schema, "values": values }))
    }

    /// Persists panel values changed in the UI and forwards them to the
    /// plugin as on_panel_change.
    pub async fn set_panel_values(&self, plugin_id: &str, values: Value) -> Result<()> {
        if !values.is_object() {
            bail!("panel values must be an object");
        }
        let path = registry::panel_values_path(plugin_id)?;
        registry::write_json_file(&path, &values)?;
        self.inner
            .emit_event_to(plugin_id, "on_panel_change", json!({ "values": values }))
            .await;
        Ok(())
    }

    /// Resolves a pending credential consent prompt.
    pub async fn respond_consent(&self, request_id: &str, decision: &str) -> Result<()> {
        let decision = match decision {
            "allow" => ConsentDecision::Allow,
            "always" => ConsentDecision::Always,
            "deny" => ConsentDecision::Deny,
            other => bail!("unknown consent decision '{other}'"),
        };
        let sender = self.inner.consent_pending.lock().await.remove(request_id);
        match sender {
            Some(tx) => {
                let _ = tx.send(decision);
                Ok(())
            }
            None => bail!("consent request not found (it may have timed out)"),
        }
    }

    pub async fn revoke_credential(&self, plugin_id: &str, kind: &str) -> Result<()> {
        self.inner
            .set_credential_consent(plugin_id, kind, "revoked")
            .await?;
        registry::audit_append(plugin_id, &format!("credential consent revoked kind={kind}"));
        Ok(())
    }

    pub async fn reset_credential_consent(&self, plugin_id: &str, kind: &str) -> Result<()> {
        self.inner
            .set_credential_consent(plugin_id, kind, "always")
            .await
    }

    pub fn audit_log(&self, plugin_id: &str) -> Vec<String> {
        registry::audit_read(plugin_id)
    }

    /// Stream lifecycle reported by the frontend player. Updates the active
    /// channel and forwards the matching events to subscribed plugins.
    pub async fn report_stream_event(
        &self,
        kind: &str,
        channel_id: Option<String>,
        login: Option<String>,
        display_name: Option<String>,
    ) -> Result<()> {
        match kind {
            "start" => {
                let channel_id = channel_id.ok_or_else(|| anyhow!("channel_id required"))?;
                let login = login.unwrap_or_default();
                *self.inner.active_channel.write().await = Some(ActiveChannel {
                    channel_id: channel_id.clone(),
                    login: login.clone(),
                });
                self.inner
                    .emit_event(
                        "on_stream_start",
                        json!({ "channel": {
                            "channel_id": channel_id,
                            "login": login,
                            "display_name": display_name.unwrap_or_default(),
                            "game_id": null, "game_name": null,
                            "started_at": null, "viewer_count": null,
                        }}),
                    )
                    .await;
            }
            "stop" => {
                let channel_id = channel_id.ok_or_else(|| anyhow!("channel_id required"))?;
                {
                    let mut active = self.inner.active_channel.write().await;
                    if active.as_ref().map(|a| a.channel_id == channel_id) == Some(true) {
                        *active = None;
                    }
                }
                self.inner
                    .emit_event("on_stream_stop", json!({ "channel_id": channel_id }))
                    .await;
            }
            "change" => {
                let channel_id = channel_id.ok_or_else(|| anyhow!("channel_id required"))?;
                let login = login.unwrap_or_default();
                *self.inner.active_channel.write().await = Some(ActiveChannel {
                    channel_id: channel_id.clone(),
                    login: login.clone(),
                });
                self.inner
                    .emit_event(
                        "on_channel_change",
                        json!({ "channel_id": channel_id, "login": login }),
                    )
                    .await;
            }
            other => bail!("unknown stream event '{other}'"),
        }
        Ok(())
    }

    /// True when at least one running plugin subscribed to chat messages.
    /// Chat parsing checks this before building the per-message payload, so
    /// the hot IRC path pays nothing while no plugin listens.
    pub async fn wants_chat_messages(&self) -> bool {
        self.inner.any_hook("on_chat_message").await
    }

    /// Fans one parsed chat message out to subscribed plugins. The payload is
    /// the lean wire shape from PROTOCOL.md, built by the chat service.
    pub async fn emit_chat_message(&self, params: Value) {
        self.inner.emit_event("on_chat_message", params).await;
    }


    /// Graceful shutdown of every running plugin (used at app exit).
    pub async fn shutdown_all(&self) {
        self.inner.shutting_down.store(true, Ordering::SeqCst);
        let running = self.inner.running.read().await;
        for handle in running.values() {
            let _ = handle.cmd_tx.send(SupCmd::Shutdown);
        }
    }

    /// True while at least one plugin process is still up (exit waits on this).
    pub async fn has_running(&self) -> bool {
        !self.inner.running.read().await.is_empty()
    }

    /// Invokes a named action on whichever running plugin handles it, and
    /// returns its result. This is how a core UI hands off a control to a
    /// plugin without knowing which plugin (or that any plugin) exists.
    pub async fn invoke_action(&self, action: &str, args: Value) -> Result<Value> {
        let cmd_tx = {
            let running = self.inner.running.read().await;
            running
                .values()
                .find(|h| h.actions.contains(action))
                .map(|h| h.cmd_tx.clone())
        };
        let Some(cmd_tx) = cmd_tx else {
            bail!("no plugin handles the action '{action}'");
        };
        let (reply, rx) = oneshot::channel();
        cmd_tx
            .send(SupCmd::Request {
                method: "invoke_action".into(),
                params: json!({ "action": action, "args": args }),
                reply,
            })
            .map_err(|_| anyhow!("the plugin handling '{action}' is not running"))?;
        match rx.await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(e)) => bail!("the plugin failed to handle '{action}': {e}"),
            Err(_) => bail!("the plugin did not respond to '{action}'"),
        }
    }

    /// The id of a running plugin that provides a feature, if any. Lets a core
    /// UI light up (or gray out) controls based on whether something backs them.
    pub async fn provides(&self, feature: &str) -> Option<String> {
        let running = self.inner.running.read().await;
        running
            .iter()
            .find(|(_, h)| h.provides.contains(feature))
            .map(|(id, _)| id.clone())
    }

    async fn find_info(&self, plugin_id: &str) -> Option<PluginInfo> {
        self.list_installed()
            .await
            .into_iter()
            .find(|p| p.id == plugin_id)
    }
}

#[cfg(test)]
mod source_listing_tests {
    use super::SourceListing;

    /// The field names the marketplace reads are a CONTRACT with
    /// `src/types/plugins.ts`, and breaking it fails silently in the worst
    /// possible way: `unavailableHere` comes back `undefined`, `> 0` is false,
    /// and the Discover tab shows "your sources may be unreachable" to a user
    /// whose sources loaded perfectly. Nothing errors, nothing logs, the words
    /// are simply wrong. So the wire names are pinned here rather than trusted
    /// to the serde attribute staying put.
    #[test]
    fn the_wire_names_match_what_the_ui_reads() {
        let json = serde_json::to_value(SourceListing {
            entries: Vec::new(),
            unavailable_here: 3,
            platform: "linux-x86_64".into(),
        })
        .expect("serializes");

        assert_eq!(
            json.get("unavailableHere").and_then(|v| v.as_u64()),
            Some(3),
            "the UI reads `unavailableHere`; a snake_case field reads as \
             undefined there and silently selects the wrong empty state"
        );
        assert!(json.get("entries").is_some(), "the UI reads `entries`");
        assert_eq!(
            json.get("platform").and_then(|v| v.as_str()),
            Some("linux-x86_64"),
            "the UI turns this into a human platform name"
        );
        assert!(
            json.get("unavailable_here").is_none(),
            "the snake_case spelling must NOT also be present, or a future \
             rename could leave the UI reading a field that no longer updates"
        );
    }
}
