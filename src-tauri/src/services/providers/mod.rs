//! Multi-platform chat + activity providers.
//!
//! Each platform is an adapter that normalises into the shared `ChatMessage` /
//! `ActivityEvent` model and publishes onto the same local-WS broadcast the
//! frontend already consumes. This module owns the cross-cutting pieces: the
//! source-key codec, and (added per phase) the provider trait + registry +
//! generic chat webview. Twitch keeps its own dedicated path in `irc_service`.

pub mod hls_master;
pub mod key;
pub mod kick;
pub mod kick_account;
pub mod kick_emotes;
pub mod kick_media;
pub mod kick_profile;
pub mod source;
pub mod tiktok;
pub mod watch_urls;
pub mod youtube;
pub mod youtube_account;
pub mod youtube_emotes;
pub mod youtube_media;
pub mod youtube_subscribe;

use crate::models::chat_layout::ChatMessage;
use crate::models::settings::Settings;
use crate::services::irc_service::IrcService;
use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use tokio::sync::OnceCell;

/// The live settings handle, so an adapter can read a user preference without
/// holding AppState. Wired once at app setup rather than from the Twitch chat
/// path, because a provider chat can start in a session where Twitch never does.
static SETTINGS: OnceLock<Arc<Mutex<Settings>>> = OnceLock::new();

pub fn init_settings(settings: Arc<Mutex<Settings>>) {
    let _ = SETTINGS.set(settings);
}

/// Read one field off the live settings. `None` before the handle is wired or if
/// the lock is poisoned, so every caller has to state its own fallback.
pub(crate) fn setting<T>(read: impl FnOnce(&Settings) -> T) -> Option<T> {
    let guard = SETTINGS.get()?.lock().ok()?;
    Some(read(&guard))
}

/// Count of active non-Twitch provider connections on the shared local-WS
/// bridge. While > 0, the Twitch start path preserves the bridge instead of
/// tearing it down (see `IrcService::start`). It is zero in a Twitch-only
/// session, so that path is byte-identical when no providers are in use.
static BRIDGE_USERS: AtomicUsize = AtomicUsize::new(0);

pub fn has_active_bridge_users() -> bool {
    BRIDGE_USERS.load(Ordering::SeqCst) > 0
}

pub(crate) fn inc_bridge_users() {
    BRIDGE_USERS.fetch_add(1, Ordering::SeqCst);
}

pub(crate) fn dec_bridge_users() {
    // Saturating: never wrap below zero if a disconnect races a failed connect.
    let _ = BRIDGE_USERS.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
        Some(n.saturating_sub(1))
    });
}

/// The app handle, stored at startup so providers can spawn the hidden webviews
/// some platforms need (e.g. Kick's Cloudflare-gated channel lookup).
static APP_HANDLE: OnceLock<tauri::AppHandle> = OnceLock::new();

pub fn set_app_handle(handle: tauri::AppHandle) {
    let _ = APP_HANDLE.set(handle);
}

pub fn app_handle() -> Option<tauri::AppHandle> {
    APP_HANDLE.get().cloned()
}

/// Whether the active connection can send to a source right now. Drives the
/// chat input's read-only vs sendable state on the frontend.
#[derive(Debug, Clone, Copy, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SendCapability {
    /// Reading only; no send path on this platform (or in this build).
    ReadOnly,
    /// A connected account / session can send here.
    Sendable,
    /// Sending is possible but the user must connect / sign in first.
    NeedsLogin,
}

/// Result of a provider send, mirroring the Twitch `SendResult` shape so the
/// frontend can treat both the same way.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SendOutcome {
    pub message_id: Option<String>,
    pub is_sent: bool,
    pub drop_reason: Option<String>,
}

/// A non-Twitch chat platform adapter. Each implementation connects to the
/// platform, normalizes incoming chat into the shared `ChatMessage`, and
/// publishes it onto the same local-WS bus the frontend already consumes via
/// `publish_chat_message`. Twitch keeps its dedicated path in `irc_service`.
#[async_trait]
pub trait ChatProvider: Send + Sync {
    /// Stable provider id ("kick", "youtube", ...). Matches ProviderId on the frontend.
    fn id(&self) -> &'static str;
    /// Begin streaming `channel`'s chat for `window` (a consumer label).
    async fn connect(&self, channel: &str, window: &str) -> Result<()>;
    /// Drop `window`'s claim; disconnect when the last consumer leaves.
    async fn disconnect(&self, channel: &str, window: &str) -> Result<()>;
    /// Drop EVERY claim held by `window`, disconnecting each channel that loses
    /// its last consumer.
    ///
    /// `disconnect` is keyed by channel, so a caller that only knows the window
    /// (the `Destroyed` window event) cannot use it — a destroyed webview never
    /// runs its React cleanup, so without this its claims pin every channel it
    /// held open forever. Adapters sweep under a SINGLE lock rather than looping
    /// `disconnect`, so a concurrent connect can't interleave between channels.
    ///
    /// Default: nothing, for adapters that hold no per-window state.
    async fn release_window(&self, _window: &str) {}
    /// Send `text` to `channel` as the connected account, if any. `reply_to` is the
    /// platform message id being replied to (None for a normal message).
    async fn send(&self, channel: &str, text: &str, reply_to: Option<&str>)
        -> Result<SendOutcome>;
    /// Whether sending to `channel` is currently possible.
    async fn send_capability(&self, channel: &str) -> SendCapability;
}

/// Registry of available platform adapters, keyed by provider id.
///
/// Two independent facets: `providers` (chat) and `sources` (browse + watch). A
/// platform may implement either or both — TikTok reads chat and resolves
/// playback without a login, while a future platform might ship only one side.
#[derive(Default)]
pub struct ProviderRegistry {
    providers: HashMap<&'static str, Arc<dyn ChatProvider>>,
    sources: HashMap<&'static str, Arc<dyn source::StreamSource>>,
}

impl ProviderRegistry {
    pub fn register(&mut self, provider: Arc<dyn ChatProvider>) {
        self.providers.insert(provider.id(), provider);
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn ChatProvider>> {
        self.providers.get(id).cloned()
    }

    pub fn register_source(&mut self, src: Arc<dyn source::StreamSource>) {
        self.sources.insert(src.id(), src);
    }

    pub fn get_source(&self, id: &str) -> Option<Arc<dyn source::StreamSource>> {
        self.sources.get(id).cloned()
    }

    /// Every registered watch/browse adapter, for the capability command.
    pub fn sources(&self) -> impl Iterator<Item = (&'static str, Arc<dyn source::StreamSource>)> + '_ {
        self.sources.iter().map(|(id, s)| (*id, s.clone()))
    }

    /// Every registered chat adapter, for cross-provider sweeps like the
    /// window-destroy release.
    pub fn providers(&self) -> impl Iterator<Item = Arc<dyn ChatProvider>> + '_ {
        self.providers.values().cloned()
    }
}

static REGISTRY: OnceCell<ProviderRegistry> = OnceCell::const_new();

/// The process-wide adapter registry, built once. Per-platform adapters are
/// added here as each ships (Kick first); until then it is empty and the
/// provider commands report the provider as unavailable.
pub async fn registry() -> &'static ProviderRegistry {
    REGISTRY
        .get_or_init(|| async {
            let mut reg = ProviderRegistry::default();
            reg.register(Arc::new(kick::KickProvider::new()));
            reg.register(Arc::new(youtube::YouTubeProvider::new()));
            reg.register(Arc::new(tiktok::TikTokProvider::new()));
            // Watch/browse adapters, added per platform phase.
            reg.register_source(Arc::new(kick_media::KickSource::new()));
            reg.register_source(Arc::new(youtube_media::YouTubeSource::new()));
            reg
        })
        .await
}

/// Tell every window that a platform account connected or disconnected.
///
/// Connection state is a pure in-memory bool, so nothing can observe a change by
/// polling it — which is exactly what several surfaces used to do, once per pane,
/// every five seconds. This is the signal that replaces all of them.
pub fn emit_platform_account_changed(providers: &[&str]) {
    use tauri::Emitter;
    if let Some(app) = app_handle() {
        let payload: Vec<String> = providers.iter().map(|p| p.to_string()).collect();
        let _ = app.emit("platform-account-changed", payload);
    }
}

/// Tell every window that a platform SESSION died on its own (expired or revoked),
/// as opposed to the user signing out. The UI uses this to say "reconnect" out
/// loud instead of quietly showing a connected account with nobody live.
pub fn emit_platform_session_expired(provider: &str) {
    use tauri::Emitter;
    if let Some(app) = app_handle() {
        let _ = app.emit("platform-session-expired", provider.to_string());
    }
}

/// Drop every provider claim held by `window`, across all adapters.
///
/// The Twitch side of this is `IrcService::release_window_claims`; this is its
/// provider counterpart, called from the same `Destroyed` window event. Without
/// it, closing a popout leaves each Kick/YouTube/TikTok pane's socket and task
/// running and its `BRIDGE_USERS` count incremented for the rest of the session.
pub async fn release_window_claims(window: &str) {
    for provider in registry().await.providers() {
        provider.release_window(window).await;
    }
}

/// Serialize a normalized chat message and publish it onto the local-WS bus the
/// frontend already listens to. The bridge is brought up on demand so an adapter
/// can publish whether or not a Twitch chat is open.
pub async fn publish_chat_message(msg: &ChatMessage) {
    // Same rule engine as the Twitch lane: ignores, highlights, mentions,
    // saved filters, history ring. Ignored rows skip the bus but still hit
    // the side-effect lane below (logs are the record).
    let mut evaluated = msg.clone();
    let rules = crate::services::chat_rules::ChatRules::snapshot();
    let verdict = crate::services::chat_rules::ChatRules::evaluate(&mut evaluated, &rules);
    if verdict.drop {
        crate::services::irc_service::run_message_side_effects(evaluated);
        return;
    }
    let msg = &evaluated;
    if let Ok(json) = serde_json::to_string(msg) {
        match IrcService::broadcaster().await {
            Some(tx) => {
                // `broadcast::send` fails when NOTHING is subscribed, and the
                // message is gone. That is silent data loss, and it is invisible
                // from the frontend (nothing arrives to be logged as dropped).
                // A failed send means NOTHING is listening yet, which for a join
                // backlog is the normal case: it is published within ~100ms of
                // resolving, before the frontend's WS client attaches. Hold it for
                // the handshake to replay instead of discarding it.
                if let Err(e) = tx.send(json) {
                    crate::services::irc_service::hold_undelivered_message(e.0).await;
                }
            }
            // The bus itself is not up yet (the bridge opens alongside the first
            // channel). Same remedy: hold, then replay on attach.
            None => crate::services::irc_service::hold_undelivered_message(json).await,
        }
    }
    // The same per-message side effects Twitch messages get: chat-log file write
    // and persisted user history. Without this a provider's chat was live-only —
    // absent from the log files and from the profile card's message history after
    // a restart. Ordered lane, so this never blocks the adapter's read loop.
    crate::services::irc_service::run_message_side_effects(msg.clone());
}

/// Publish a pre-built JSON control frame (e.g. a `CLEARCHAT`/`CLEARMSG`
/// moderation frame) onto the same bus. The frontend already routes these by
/// their `channel` field, so a provider can drive the existing deletion display
/// + mod log by emitting the same frame shape Twitch's IRC path does.
pub async fn publish_frame(json: String) {
    if let Some(tx) = IrcService::broadcaster().await {
        let _ = tx.send(json);
    }
}
