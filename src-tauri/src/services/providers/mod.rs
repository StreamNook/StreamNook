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
pub mod kick_door;
pub mod kick_emotes;
pub mod kick_media;
pub mod kick_profile;
pub mod source;
pub mod table_repair;
pub mod tiktok;
pub mod tiktok_feed;
pub mod tiktok_following;
pub mod tiktok_media;
pub mod tiktok_send;
pub mod watch_urls;
pub mod youtube;
pub mod youtube_account;
pub mod youtube_emotes;
pub mod youtube_media;
pub mod youtube_send;
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
static APP_HANDLE: OnceLock<crate::rt::AppHandle> = OnceLock::new();

pub fn set_app_handle(handle: crate::rt::AppHandle) {
    let _ = APP_HANDLE.set(handle);
    // Who you are on each platform is known before any window asks, so a
    // Kick-only account's replies count from the first message. Follow
    // only: the roster event and its Twitch-side effects wait for `notify`.
    tauri::async_runtime::spawn(async {
        let _ = crate::services::account_roster::build_followed().await;
    });
}

pub fn app_handle() -> Option<crate::rt::AppHandle> {
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
    /// How many channels this adapter holds open right now, for the resource
    /// log. `None` when its table is locked this instant: the sampler never
    /// waits on the chat path.
    fn open_channel_count(&self) -> Option<usize>;
    /// Send `text` to `channel` as the connected account, if any. `reply_to` is the
    /// platform message id being replied to (None for a normal message).
    async fn send(&self, channel: &str, text: &str, reply_to: Option<&str>)
        -> Result<SendOutcome>;
    /// Whether sending to `channel` is currently possible.
    async fn send_capability(&self, channel: &str) -> SendCapability;
    /// The channels `window` holds a claim on, as this adapter stores them
    /// (lowercase), sorted. The automation engine adopts the chats a closing
    /// window held, per platform.
    async fn channels_held_by(&self, window: &str) -> Vec<String>;
    /// Whether a window other than `label` holds `channel`.
    async fn held_by_other_than(&self, channel: &str, label: &str) -> bool;
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

/// The channels in an adapter's table (channel to consumer labels) that
/// `label` holds, sorted. The three adapters share it so they agree.
pub(crate) fn held_by_label<'a>(
    held: impl Iterator<Item = (&'a String, &'a std::collections::HashSet<String>)>,
    label: &str,
) -> Vec<String> {
    let mut out: Vec<String> =
        held.filter(|(_, labels)| labels.contains(label)).map(|(channel, _)| channel.clone()).collect();
    out.sort();
    out
}

/// Whether a consumer other than `label` is in `labels`.
pub(crate) fn held_by_another(labels: Option<&std::collections::HashSet<String>>, label: &str) -> bool {
    labels.is_some_and(|labels| labels.iter().any(|l| l != label))
}

/// Date a live row by the wall clock when it arrives, as the IRC lane dates
/// Twitch's (the engine answers only rows read in the last 30 s). A row that
/// already has a time, or that its adapter marked as backlog, is left alone.
pub(crate) fn stamp_arrival(msg: &mut ChatMessage, now_ms: i64) {
    if msg.received_at_ms.is_none() && !msg.metadata.from_backfill {
        msg.received_at_ms = Some(now_ms);
    }
}

static REGISTRY: OnceCell<ProviderRegistry> = OnceCell::const_new();

/// Open chat channels per adapter, sorted by provider id, without waiting on
/// any of them. Empty until the registry exists, since nothing can be open
/// before it does.
pub fn open_channel_counts() -> Vec<(&'static str, Option<usize>)> {
    let Some(reg) = REGISTRY.get() else {
        return Vec::new();
    };
    let mut counts: Vec<_> = reg
        .providers
        .values()
        .map(|p| (p.id(), p.open_channel_count()))
        .collect();
    counts.sort_by_key(|(id, _)| *id);
    counts
}

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
            reg.register_source(Arc::new(tiktok_media::TikTokSource::new()));
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
    crate::services::account_roster::notify();
    crate::services::account_automations::request_refresh();
}

/// Tell every window that a platform SESSION died on its own (expired or revoked),
/// as opposed to the user signing out. The UI uses this to say "reconnect" out
/// loud instead of quietly showing a connected account with nobody live.
pub fn emit_platform_session_expired(provider: &str) {
    use tauri::Emitter;
    crate::services::account_roster::mark_expired(provider);
    if let Some(app) = app_handle() {
        let _ = app.emit("platform-session-expired", provider.to_string());
    }
    crate::services::account_automations::request_refresh();
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
    stamp_arrival(&mut evaluated, chrono::Utc::now().timestamp_millis());
    let rules = crate::services::chat_rules::ChatRules::snapshot();
    let verdict = crate::services::chat_rules::ChatRules::evaluate(&mut evaluated, &rules);
    if verdict.drop {
        crate::services::irc_service::run_message_side_effects(evaluated, verdict.drop);
        return;
    }
    // Inbox, mention sound and notification, as on the Twitch lane.
    crate::services::mention_inbox::on_verdict(&evaluated, &verdict);
    // A native shell draws the row Rust laid out, as on the Twitch lane.
    crate::services::chat_row_view::stamp(&mut evaluated);
    let msg = &evaluated;
    // A row parsed before its channel's whole 7TV table landed is kept so its
    // 7TV words can be patched in once it does.
    match msg.provider.as_str() {
        "kick" => kick_emotes::hold_row(msg),
        "youtube" => youtube_emotes::hold_row(msg),
        _ => {}
    }
    if let Ok(json) = serde_json::to_string(msg) {
        let frame = crate::services::chat_bridge_route::BridgeFrame::row(
            &msg.channel,
            json,
            crate::services::mention_inbox::pinged(&verdict),
        );
        match IrcService::broadcaster().await {
            Some(tx) => {
                // `broadcast::send` fails when NOTHING is subscribed, and the
                // message is gone. That is silent data loss, and it is invisible
                // from the frontend (nothing arrives to be logged as dropped).
                // A failed send means NOTHING is listening yet, which for a join
                // backlog is the normal case: it is published within ~100ms of
                // resolving, before the frontend's WS client attaches. Hold it for
                // the handshake to replay instead of discarding it.
                if let Err(e) = tx.send(frame) {
                    crate::services::irc_service::hold_undelivered_message(e.0).await;
                }
            }
            // The bus itself is not up yet (the bridge opens alongside the first
            // channel). Same remedy: hold, then replay on attach.
            None => crate::services::irc_service::hold_undelivered_message(frame).await,
        }
    }
    // The same per-message side effects Twitch messages get: chat-log file write
    // and persisted user history. Without this a provider's chat was live-only —
    // absent from the log files and from the profile card's message history after
    // a restart. Ordered lane, so this never blocks the adapter's read loop.
    crate::services::irc_service::run_message_side_effects(msg.clone(), verdict.drop);
}

/// Publish a pre-built JSON control frame (e.g. a `CLEARCHAT`/`CLEARMSG`
/// moderation frame) onto the same bus. The frontend already routes these by
/// their `channel` field, so a provider can drive the existing deletion display
/// + mod log by emitting the same frame shape Twitch's IRC path does.
pub async fn publish_frame(json: String) {
    // A provider's CLEARMSG / CLEARCHAT reaches the mention inbox as Twitch's do.
    crate::services::mention_inbox::on_provider_frame(&json);
    if let Some(tx) = IrcService::broadcaster().await {
        // Every window: the frame's channel is inside the JSON, and the rule is
        // to deliver when the route cannot tell.
        let _ = tx.send(crate::services::chat_bridge_route::BridgeFrame::all(json));
    }
}

#[cfg(test)]
mod automation_hook_tests {
    use super::*;
    use std::collections::HashSet;

    fn set(items: &[&str]) -> HashSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn row() -> ChatMessage {
        serde_json::from_value(serde_json::json!({
            "id": "m1", "user_id": "7", "username": "fan", "display_name": "fan", "color": null,
            "badges": [], "timestamp": "", "content": "hi", "provider": "kick", "channel": "kick:alpha",
            "tags": {}, "metadata": {},
        }))
        .expect("a test row")
    }

    #[test]
    fn held_by_label_lists_the_channels_a_label_holds_sorted() {
        let table: std::collections::HashMap<String, HashSet<String>> = [
            ("zed".to_string(), set(&["automations", "main"])),
            ("alpha".to_string(), set(&["automations"])),
            ("beta".to_string(), set(&["main"])),
        ]
        .into_iter()
        .collect();
        assert_eq!(held_by_label(table.iter(), "automations"), vec!["alpha".to_string(), "zed".to_string()]);
        assert_eq!(held_by_label(table.iter(), "main"), vec!["beta".to_string(), "zed".to_string()]);
        assert!(held_by_label(table.iter(), "nobody").is_empty());
    }

    #[test]
    fn held_by_another_is_false_for_nobody_and_for_only_the_label() {
        assert!(!held_by_another(None, "automations"));
        assert!(!held_by_another(Some(&set(&["automations"])), "automations"));
        assert!(held_by_another(Some(&set(&["automations", "main"])), "automations"));
        assert!(held_by_another(Some(&set(&["main"])), "automations"));
    }

    #[test]
    fn the_provider_lane_passes_the_verdict_and_stamps_arrival() {
        // The verdict reaches the side-effect lane (S7.1 E28), so an ignored Kick row arrives at the engine as dropped.
        let src = include_str!("mod.rs");
        let publish = src.split("pub async fn publish_chat_message").nth(1).and_then(|t| t.split("pub async fn publish_frame").next()).unwrap();
        assert!(publish.contains("run_message_side_effects(evaluated, verdict.drop)"));
        assert!(publish.contains("run_message_side_effects(msg.clone(), verdict.drop)"));
        assert!(publish.contains("stamp_arrival(&mut evaluated"));
    }

    #[test]
    fn a_live_row_is_dated_on_arrival_and_backlog_is_not() {
        let mut live = row();
        stamp_arrival(&mut live, 1_700_000_000_123);
        assert_eq!(live.received_at_ms, Some(1_700_000_000_123));
        stamp_arrival(&mut live, 1_800_000_000_000);
        assert_eq!(live.received_at_ms, Some(1_700_000_000_123), "never restamped");
        let mut backlog = row();
        backlog.metadata.from_backfill = true;
        stamp_arrival(&mut backlog, 1_700_000_000_123);
        assert_eq!(backlog.received_at_ms, None);
    }
}
