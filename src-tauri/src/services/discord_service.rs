use crate::models::settings::AppState;
use anyhow::Result;
use discord_rich_presence::{activity::*, DiscordIpc};
use lazy_static::lazy_static;
use rand::prelude::IndexedRandom;
// rand 0.10 moved random_bool / random_range onto RngExt.
use rand::RngExt;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

// Which IPC transport carries the presence. Windows and Linux use the crate's
// own client; macOS needs a wider socket search than the crate performs (see
// services/discord_ipc_macos.rs). Both implement `DiscordIpc`, so every call
// below is platform-agnostic.
#[cfg(target_os = "macos")]
use crate::services::discord_ipc_macos::MacDiscordIpcClient as IpcClient;
#[cfg(not(target_os = "macos"))]
use discord_rich_presence::DiscordIpcClient as IpcClient;

// --- New Imports for matching logic ---
use regex::Regex;
use std::collections::HashSet;
// --- End New Imports ---

// Discord asset keys - these must match the asset names uploaded to Discord Developer Portal
const DISCORD_LARGE_IMAGE: &str = "streamnook_logo";
const DISCORD_SMALL_IMAGE_TWITCH: &str = "twitch";
// Discord application (client) ID for StreamNook's rich presence. Dedicated
// StreamNook-only app, separate from the Penrose bot's application.
const DISCORD_CLIENT_ID: &str = "1524098616648663110";
// Where the rich-presence call-to-action button points.
const DOWNLOAD_URL: &str = "https://streamnook.app";

// Every Discord IPC call is synchronous and blocks until the pipe answers. A
// Discord client whose pipe accepts but never replies would otherwise park a
// runtime worker forever, so every call runs on the blocking pool under one of
// these deadlines.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const IPC_CALL_TIMEOUT: Duration = Duration::from_secs(3);
// After Discord fails to answer in time, no new connection is attempted for
// this long, so presence updates do not each park another thread on a dead pipe.
const UNANSWERED_BACKOFF: Duration = Duration::from_secs(60);

pub struct DiscordService;

struct DiscordState {
    client: Option<IpcClient>,
    start_time: i64,
    // Identity of the currently-displayed activity (the "Watching X" line). When
    // this changes (a raid or auto-switch to a different streamer), the elapsed
    // timer is reset. It stays put when only the title/category changes for the
    // same streamer, so the "watching for" counter doesn't restart on every
    // channel-update.
    current_key: Option<String>,
    // The logged-in Discord user's display name, captured from the IPC handshake
    // READY frame. Used to personalize a few idle phrases. None until we connect
    // (or if Discord doesn't return one).
    discord_username: Option<String>,
    // Bumped by `disconnect`. A connect that began under an older epoch closes
    // its client instead of storing it, so a connect racing a disconnect can
    // never resurrect the presence.
    epoch: u64,
    // Set when Discord accepted the pipe but did not answer in time. No connect
    // is attempted before this instant.
    connect_backoff_until: Option<Instant>,
}

lazy_static! {
    static ref DISCORD_STATE: Arc<Mutex<DiscordState>> = Arc::new(Mutex::new(DiscordState {
        client: None,
        start_time: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64,
        current_key: None,
        discord_username: None,
        epoch: 0,
        connect_backoff_until: None,
    }));
    // Serializes connection attempts without holding DISCORD_STATE, so two
    // callers never both open a pipe and nobody waits longer than one
    // CONNECT_TIMEOUT for an attempt already in flight.
    static ref CONNECT_LOCK: Mutex<()> = Mutex::new(());
}

/// Why a connection could not be made.
#[derive(Debug)]
enum ConnectError {
    /// Discord recently failed to answer; no attempt was made.
    BackingOff,
    /// The pipe was found but Discord did not finish the handshake in time.
    Unanswered,
    /// `disconnect` ran while the connection was being opened.
    Superseded,
    /// Discord is not running, or the pipe refused the connection.
    Failed(String),
}

impl std::fmt::Display for ConnectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConnectError::BackingOff => {
                write!(f, "Discord did not answer recently, waiting before retrying")
            }
            ConnectError::Unanswered => write!(
                f,
                "Discord did not answer within {} s",
                CONNECT_TIMEOUT.as_secs()
            ),
            ConnectError::Superseded => write!(f, "Discord was disconnected while connecting"),
            ConnectError::Failed(e) => write!(f, "{e}"),
        }
    }
}

/// Why one IPC call on an open client failed. In both cases the client has
/// left the state, so the next call reconnects.
#[derive(Debug)]
enum CallError {
    Unanswered,
    Failed(String),
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CallError::Unanswered => write!(
                f,
                "Discord did not answer within {} s",
                IPC_CALL_TIMEOUT.as_secs()
            ),
            CallError::Failed(e) => write!(f, "{e}"),
        }
    }
}

/// Whether a connection attempt may start now, given the current backoff.
fn connect_allowed(backoff_until: Option<Instant>, now: Instant) -> bool {
    backoff_until.is_none_or(|until| now >= until)
}

/// The backoff to store after an attempt. Only an unanswered pipe backs off: a
/// missing pipe fails instantly and costs nothing to retry, so Discord starting
/// after StreamNook is still picked up on the next presence update.
fn backoff_after(unanswered: bool, now: Instant) -> Option<Instant> {
    unanswered.then(|| now + UNANSWERED_BACKOFF)
}

/// Run a blocking closure on the blocking pool, giving up after `limit`. On
/// timeout the closure keeps its thread until the pipe returns, but its result
/// (and anything it owns, such as a client) is simply dropped there and never
/// reaches the shared state.
async fn run_bounded<T, F>(limit: Duration, f: F) -> Option<std::result::Result<T, String>>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let handle = tauri::async_runtime::spawn_blocking(f);
    match tokio::time::timeout(limit, handle).await {
        Ok(joined) => Some(joined.map_err(|e| e.to_string())),
        Err(_) => None,
    }
}

/// Close a client on the blocking pool without waiting for it.
fn close_detached(mut client: IpcClient) {
    drop(tauri::async_runtime::spawn_blocking(move || {
        let _ = client.close();
    }));
}

// Regex patterns for normalizing game names before matching.
lazy_static! {
    static ref CLEAN_PATTERNS: Vec<Regex> = vec![
        Regex::new(r"[®™©]").unwrap(), // Remove trademark symbols
        Regex::new(r"\s*\(.*?\)").unwrap(), // (demo), (beta), etc.
        Regex::new(r"\s*\[.*?\]").unwrap(), // [Open Beta], etc.
        Regex::new(r":\s*[Tt]he\s+").unwrap(), // : The Game
        Regex::new(r"\s*-\s*[Ss]eason\s+\d+").unwrap(), // - Season 1
        Regex::new(r"\s*[Ee]dition$").unwrap(), // Edition
        Regex::new(r"\s*[Rr]emastered$").unwrap(), // Remastered
        Regex::new(r"\s*[Dd]efinitive\s+[Ee]dition$").unwrap(), // Definitive Edition
    ];

    static ref STOP_WORDS: HashSet<&'static str> =
        ["the", "a", "an", "of", "in", "on", "at", "to", "for"]
        .iter().cloned().collect();
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DiscordApp {
    id: String,
    name: String,
    #[serde(default)]
    icon_hash: Option<String>,
    #[serde(default)]
    cover_image: Option<String>,
    #[serde(default)]
    aliases: Vec<String>,
}

/// Cached copy of Discord's detectable-games list (see fetch_discord_detectables).
struct DetectablesCache {
    apps: Vec<DiscordApp>,
    fetched_at: Instant,
}

lazy_static! {
    static ref DETECTABLES_CACHE: Mutex<Option<DetectablesCache>> = Mutex::new(None);
}

// One pool of self-contained one-liners for the idle/browsing presence. A
// single phrase is shown (as `details`) with no second `state` line, so two
// unrelated sayings can't end up stacked on top of each other.
const IDLE_PHRASES: &[&str] = &[
    "Channel surfing for poggers",
    "Lost in the Twitch jungle",
    "Hunting for the next hype train",
    "Just chatting... with myself",
    "Searching for the legendary Kappa",
    "Dodging spoilers like a pro gamer",
    "Vibing in the VODs",
    "Exploring the emote-verse",
    "Where's the 'unfollow' button for reality?",
    "AFK, but my eyes are still watching",
    "In a staring contest with my screen",
    "My brain is in emote-only mode",
    "Currently respawning...",
    "Thinking about what to raid next",
    "Powered by caffeine and good vibes",
    "Waiting for the next 'clip that!' moment",
    "Procrastinating, but make it 1080p60",
    "Avoiding responsibilities in 4K",
    "Touching grass... in a loading screen",
    "Mentally subscribed, financially undecided",
    "Emotionally invested in strangers' gameplay",
    "Living vicariously through better gamers",
    "Lurking like it's a competitive sport",
    "Watching someone else be productive",
    "My personality is just other people's streams",
    "Here for the chat, staying for the chaos",
    "One more stream, then bed (a lie)",
    "Letting the algorithm raise me",
    "Donating my watch time, not my wallet",
    "Refreshing the followed page like it's a job",
    "Professional spectator, unpaid",
    "Chat is my coworking space now",
];

// Idle phrases that roast the logged-in Discord user by name. `{}` is replaced
// with their display name at runtime (only used when we managed to capture it).
const PERSONALIZED_PHRASES: &[&str] = &[
    "Hi, I'm {} and I have no off button",
    "{} said 'one more stream' four streams ago",
    "{} calls this 'research'",
    "Somebody please tell {} to go outside",
    "{} is touching grass tomorrow. Allegedly.",
    "{} is rotting, but in glorious HD",
    "{} mistook watching streams for a personality",
    "{}'s watchlist is a cry for help",
    "{} is here instead of doing literally anything else",
    "{} types in chat like it pays the rent",
    "Loading {}'s entire personality... 99%",
    "{} is emotionally supported by strangers online",
    "{} would rather be here than be productive",
    "{} has strong opinions and zero invitations",
    "{} clicked 'just one video' and lost the whole day",
    "{} is why the 'are you still watching?' prompt exists",
];

// Random labels for the call-to-action button that points at streamnook.app.
// The destination never changes, only the wording. Keep each under Discord's
// 32-character button-label limit.
const DOWNLOAD_BUTTON_LABELS: &[&str] = &[
    "Download StreamNook",
    "Join them?",
    "Get StreamNook",
    "Become one of us",
    "Ditch the browser",
    "Steal my setup",
    "yes, it's free",
    "Watch like this too",
    "Upgrade your lurking",
    "You know you want it",
    "Join the nook",
    "Lurk in style",
    "Resistance is futile",
    "One of us. One of us.",
    "Free, no catch",
    "Do it. Do it now.",
];

impl DiscordService {
    /// Connect to Discord and set idle presence
    pub async fn connect(app_state: &AppState) -> Result<()> {
        let discord_enabled = {
            let settings = app_state.settings.lock().unwrap();
            settings.discord_rpc_enabled
        };

        if !discord_enabled {
            return Ok(());
        }

        if let Err(e) = Self::ensure_connected().await {
            // The frontend logs this too, but only to the JS console. This
            // is the copy that reaches streamnook.log, which is the file a
            // user can actually send back.
            match e {
                ConnectError::BackingOff => log::debug!("[Discord] initial connect skipped: {e}"),
                _ => log::warn!("[Discord] initial connect failed: {e}"),
            }
            return Err(anyhow::anyhow!("{e}"));
        }

        Self::set_idle_presence_internal().await
    }

    /// Make sure a client is stored, opening one if needed.
    ///
    /// The handshake runs off the async thread under CONNECT_TIMEOUT and
    /// without DISCORD_STATE held, so a Discord that never answers can neither
    /// park a runtime worker nor lock out every other presence call.
    async fn ensure_connected() -> std::result::Result<(), ConnectError> {
        let _connecting = CONNECT_LOCK.lock().await;

        let epoch = {
            let guard = DISCORD_STATE.lock().await;
            if guard.client.is_some() {
                return Ok(());
            }
            if !connect_allowed(guard.connect_backoff_until, Instant::now()) {
                return Err(ConnectError::BackingOff);
            }
            guard.epoch
        };

        let outcome = run_bounded(CONNECT_TIMEOUT, Self::connect_client).await;

        let mut guard = DISCORD_STATE.lock().await;
        guard.connect_backoff_until = backoff_after(outcome.is_none(), Instant::now());
        let (client, username) = match outcome {
            None => return Err(ConnectError::Unanswered),
            Some(Err(e)) => return Err(ConnectError::Failed(e)),
            Some(Ok(Err(e))) => return Err(ConnectError::Failed(e.to_string())),
            Some(Ok(Ok(pair))) => pair,
        };

        if guard.epoch != epoch {
            drop(guard);
            close_detached(client);
            return Err(ConnectError::Superseded);
        }
        if guard.client.is_some() {
            // Another path stored a client meanwhile; keep that one.
            drop(guard);
            close_detached(client);
            return Ok(());
        }

        guard.client = Some(client);
        if username.is_some() {
            guard.discord_username = username;
        }
        Ok(())
    }

    /// Run one IPC call on the stored client, off the async thread and bounded
    /// by IPC_CALL_TIMEOUT. The caller holds DISCORD_STATE, so presence writes
    /// stay ordered, but never for longer than the deadline.
    ///
    /// The client goes back into the state only when the call succeeds. A
    /// failed call closes it; an unanswered call leaves it with the parked
    /// thread and starts the backoff. Either way the next call reconnects.
    async fn call_client<F, E>(
        guard: &mut tokio::sync::MutexGuard<'_, DiscordState>,
        call: F,
    ) -> std::result::Result<(), CallError>
    where
        F: FnOnce(&mut IpcClient) -> std::result::Result<(), E> + Send + 'static,
        E: std::fmt::Display,
    {
        let Some(mut client) = guard.client.take() else {
            return Err(CallError::Failed("not connected".to_string()));
        };

        let outcome = run_bounded(IPC_CALL_TIMEOUT, move || {
            let result = call(&mut client).map_err(|e| e.to_string());
            (client, result)
        })
        .await;

        match outcome {
            None => {
                guard.connect_backoff_until = backoff_after(true, Instant::now());
                Err(CallError::Unanswered)
            }
            Some(Err(e)) => Err(CallError::Failed(e)),
            Some(Ok((client, Ok(())))) => {
                guard.client = Some(client);
                Ok(())
            }
            Some(Ok((client, Err(e)))) => {
                close_detached(client);
                Err(CallError::Failed(e))
            }
        }
    }

    /// Open an IPC connection and capture the logged-in user's display name.
    ///
    /// The crate's default `connect()` performs the handshake but throws away
    /// the READY frame, which is exactly where Discord hands back the local
    /// user. We run the handshake by hand so we can keep the name for the
    /// personalized idle phrases. No OAuth or extra scopes are involved. The
    /// basic RPC handshake returns the current user on its own.
    ///
    /// Blocking: only ever called through `run_bounded`.
    fn connect_client() -> Result<(IpcClient, Option<String>)> {
        let mut client = IpcClient::new(DISCORD_CLIENT_ID);
        client.connect_ipc()?;
        client.send(json!({ "v": 1, "client_id": DISCORD_CLIENT_ID }), 0)?;

        // READY frame: { "evt": "READY", "data": { "user": { "global_name", "username", ... } } }
        let username = match client.recv() {
            Ok((_, value)) => {
                let user = value.get("data").and_then(|d| d.get("user"));
                user.and_then(|u| u.get("global_name").and_then(|v| v.as_str()))
                    .filter(|s| !s.is_empty())
                    .or_else(|| user.and_then(|u| u.get("username").and_then(|v| v.as_str())))
                    .map(|s| s.to_string())
            }
            Err(_) => None,
        };

        Ok((client, username))
    }

    /// A call-to-action button with a randomly chosen, often cheeky label. The
    /// destination is always streamnook.app; only the wording rotates.
    fn download_button() -> Button<'static> {
        let mut rng = rand::rng();
        let label = DOWNLOAD_BUTTON_LABELS
            .choose(&mut rng)
            .copied()
            .unwrap_or("Download StreamNook");
        Button::new(label, DOWNLOAD_URL)
    }

    /// Disconnect from Discord
    pub async fn disconnect() -> Result<()> {
        let client = {
            let mut guard = DISCORD_STATE.lock().await;
            guard.epoch = guard.epoch.wrapping_add(1);
            // Turning the feature off and on again is a deliberate retry.
            guard.connect_backoff_until = None;
            guard.client.take()
        };
        if let Some(mut client) = client {
            let goodbye = run_bounded(IPC_CALL_TIMEOUT, move || {
                let _ = client.clear_activity();
                let _ = client.close();
            })
            .await;
            if goodbye.is_none() {
                log::warn!(
                    "[Discord] disconnect: Discord did not answer within {} s",
                    IPC_CALL_TIMEOUT.as_secs()
                );
            }
        }
        Ok(())
    }

    /// Set idle/browsing presence
    pub async fn set_idle_presence(app_state: &AppState) -> Result<()> {
        let discord_enabled = {
            let settings = app_state.settings.lock().unwrap();
            settings.discord_rpc_enabled
        };

        if !discord_enabled {
            return Ok(());
        }

        // Discord may have started AFTER StreamNook, in which case the
        // boot-time connect() found no socket. `update_presence` retries
        // lazily; without the same retry here the idle line - the first
        // presence a browsing user would ever see - never appears until a
        // stream is opened and closed again. A missing Discord stays silent;
        // an unanswered pipe is worth one line.
        if let Err(e) = Self::ensure_connected().await {
            match e {
                ConnectError::Unanswered => log::warn!("[Discord] idle presence skipped: {e}"),
                _ => log::debug!("[Discord] idle presence skipped: {e}"),
            }
        }

        Self::set_idle_presence_internal().await
    }

    /// Show the idle line on the stored client, if there is one. Connecting is
    /// the caller's job.
    async fn set_idle_presence_internal() -> Result<()> {
        let mut guard = DISCORD_STATE.lock().await;

        // Dropping back to idle clears the per-stream identity and restarts the
        // timer, so the next stream we open begins its "watching for" counter
        // from zero instead of inheriting a stale elapsed time.
        guard.current_key = None;
        guard.start_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let timestamp = guard.start_time;

        if guard.client.is_none() {
            return Ok(());
        }

        // Single line only. Set `details` and leave `state` unset so we never
        // show two unrelated random sayings stacked together. When we know
        // the user's Discord name, half the time roast them by name instead.
        // The RNG lives only in this block: it is not Send.
        let phrase: String = {
            let mut rng = rand::rng();
            match &guard.discord_username {
                Some(name) if rng.random_bool(0.5) => PERSONALIZED_PHRASES
                    .choose(&mut rng)
                    .copied()
                    .unwrap_or("{} is watching ironically")
                    .replace("{}", name),
                _ => IDLE_PHRASES
                    .choose(&mut rng)
                    .copied()
                    .unwrap_or("Browsing Twitch")
                    .to_string(),
            }
        };

        let activity = Activity::new()
            .details(phrase)
            .assets(
                Assets::new()
                    .large_image(DISCORD_LARGE_IMAGE)
                    .large_text("StreamNook"),
            )
            .timestamps(Timestamps::new().start(timestamp))
            .buttons(vec![Self::download_button()]);

        Self::call_client(&mut guard, move |client| client.set_activity(activity))
            .await
            .map_err(|e| {
                if matches!(e, CallError::Unanswered) {
                    log::warn!("[Discord] idle presence: {e}");
                }
                anyhow::anyhow!("Failed to set idle presence: {}", e)
            })
    }

    /// Update presence when watching a stream
    pub async fn update_presence(
        details: &str,
        state: &str,
        _large_image: &str,
        _small_image: &str,
        _start_time: u64,
        game_name: &str,
        stream_url: &str,
        app_state: &AppState,
    ) -> Result<()> {
        let discord_enabled = {
            let settings = app_state.settings.lock().unwrap();
            settings.discord_rpc_enabled
        };

        if !discord_enabled {
            return Ok(());
        }

        // Resolve the category image BEFORE taking the Discord lock so a slow
        // network fetch can never delay or block a streamer swap. The detectables
        // list is cached, so this is normally instant.
        let game_image_url = if !game_name.is_empty() {
            Self::resolve_game_image(game_name).await
        } else {
            None
        };

        // Ensure we're connected
        if let Err(e) = Self::ensure_connected().await {
            // Discord not running. Never block the stream on it, but do not
            // swallow the reason either: this path failing without a trace is
            // what made the macOS gap undiagnosable. A backoff already logged
            // its cause when it started.
            match e {
                ConnectError::BackingOff => {
                    log::debug!("[Discord] presence update skipped: {e}")
                }
                _ => log::warn!("[Discord] presence update skipped, no IPC connection: {e}"),
            }
            return Ok(());
        }

        let mut guard = DISCORD_STATE.lock().await;
        if guard.client.is_none() {
            // Disconnected between connecting and here.
            return Ok(());
        }

        // Reset the elapsed timer only when the activity identity changes (a
        // raid or auto-switch to a different streamer). Keep it steady when only
        // the title or category updates for the same streamer, so the
        // "watching for HH:MM" counter doesn't restart on every channel-update.
        let identity = if !details.is_empty() {
            details.to_string()
        } else {
            stream_url.to_string()
        };
        if guard.current_key.as_deref() != Some(identity.as_str()) {
            guard.start_time = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64;
            guard.current_key = Some(identity);
        }
        let timestamp = guard.start_time;

        // Build the activity. Every string is owned so the activity can move
        // onto the blocking pool.
        let mut assets = Assets::new();

        if let Some(url) = game_image_url {
            assets = assets.large_image(url);
            assets = assets.large_text(game_name.to_string());
        } else {
            // Fallback to the StreamNook icon only when there's no category at all.
            assets = assets.large_image(DISCORD_LARGE_IMAGE);
            assets = assets.large_text("StreamNook");
        }

        // Add Twitch logo as small image
        assets = assets.small_image("https://raw.githubusercontent.com/StreamNook/StreamNook/refs/heads/main/src-tauri/images/logo_1704751143960.JPG");
        assets = assets.small_text("Twitch");

        let mut activity: Activity<'static> = Activity::new()
            .details(details.to_string())
            .state(state.to_string())
            .assets(assets)
            .timestamps(Timestamps::new().start(timestamp))
            .activity_type(ActivityType::Watching); // Set activity type to "Watching"

        // Add buttons - always include both if stream_url is available
        if !stream_url.is_empty() {
            activity = activity.buttons(vec![
                Button::new("Watch Stream", stream_url.to_string()),
                Self::download_button(),
            ]);
        }

        // Try to set activity, reconnect if IPC socket fails
        let first = activity.clone();
        match Self::call_client(&mut guard, move |client| client.set_activity(first)).await {
            Ok(()) => return Ok(()),
            Err(CallError::Unanswered) => {
                // The pipe is wedged and the backoff has started; reconnecting
                // now would only wedge again.
                log::warn!("[Discord] presence update: {}", CallError::Unanswered);
                return Ok(());
            }
            Err(e) => {
                // Connection lost - the broken client is already gone.
                log::debug!("[Discord] set_activity failed, reconnecting: {e}");
            }
        }
        drop(guard);

        // Attempt reconnection once
        if let Err(e) = Self::ensure_connected().await {
            match e {
                ConnectError::BackingOff => log::debug!("[Discord] reconnect skipped: {e}"),
                _ => log::warn!("[Discord] reconnect failed: {e}"),
            }
            return Ok(()); // Don't propagate error - this is non-critical
        }

        let mut guard = DISCORD_STATE.lock().await;
        if guard.client.is_none() {
            return Ok(());
        }
        if let Err(e) =
            Self::call_client(&mut guard, move |client| client.set_activity(activity)).await
        {
            // Reconnected and STILL refused. Distinct from "Discord isn't
            // running", and the only place it can be seen.
            log::warn!("[Discord] reconnected but set_activity failed: {e}");
        }
        Ok(()) // Don't block the stream on a cosmetic feature
    }

    /// Clear presence
    pub async fn clear_presence(app_state: &AppState) -> Result<()> {
        let discord_enabled = {
            let settings = app_state.settings.lock().unwrap();
            settings.discord_rpc_enabled
        };

        if !discord_enabled {
            return Ok(());
        }

        let mut guard = DISCORD_STATE.lock().await;
        if guard.client.is_none() {
            return Ok(());
        }
        Self::call_client(&mut guard, |client| client.clear_activity())
            .await
            .map_err(|e| {
                if matches!(e, CallError::Unanswered) {
                    log::warn!("[Discord] clear presence: {e}");
                }
                anyhow::anyhow!("Failed to clear Discord activity: {}", e)
            })
    }

    /// Resolve a Twitch category to a large-image URL for Discord.
    ///
    /// Prefer Discord's detectable-games art (wide capsule covers) when the
    /// category is a game Discord knows about. For anything Discord doesn't list
    /// (Just Chatting, IRL, Music, ASMR, and the like) fall back to Twitch's
    /// own category box art so the activity shows the real category instead of
    /// the StreamNook logo. `game_name` is always a real Twitch category here, so
    /// the box-art URL resolves.
    async fn resolve_game_image(game_name: &str) -> Option<String> {
        if let Ok(apps) = Self::fetch_discord_detectables().await {
            // Try to match the game, passing a similarity threshold.
            if let Some(matched_app) = Self::match_game(&apps, game_name, 0.6) {
                // Prefer cover_image, fallback to icon_hash
                if let Some(cover) = &matched_app.cover_image {
                    return Some(format!(
                        "https://cdn.discordapp.com/app-assets/{}/{}.png?size=512",
                        matched_app.id, cover
                    ));
                }
                if let Some(icon_hash) = &matched_app.icon_hash {
                    return Some(format!(
                        "https://cdn.discordapp.com/app-icons/{}/{}.png?size=512",
                        matched_app.id, icon_hash
                    ));
                }
            }
        }

        Some(Self::twitch_boxart_url(game_name))
    }

    /// Build a Twitch box-art URL for a category by name. Twitch serves box art
    /// keyed by the exact category name, so non-game categories resolve too.
    fn twitch_boxart_url(game_name: &str) -> String {
        format!(
            "https://static-cdn.jtvnw.net/ttv-boxart/{}-288x384.jpg",
            urlencoding::encode(game_name)
        )
    }

    /// Fetch Discord's detectable games list, cached for an hour. The list is
    /// several MB, so re-downloading it on every presence update would add
    /// latency to streamer swaps for no benefit.
    async fn fetch_discord_detectables() -> Result<Vec<DiscordApp>> {
        {
            let cache = DETECTABLES_CACHE.lock().await;
            if let Some(cached) = cache.as_ref() {
                if cached.fetched_at.elapsed() < Duration::from_secs(3600) {
                    return Ok(cached.apps.clone());
                }
            }
        }

        // Shared client; the user agent and the 6 s deadline ride on the request.
        let response = crate::services::http::client()
            .get("https://discord.com/api/v9/applications/detectable")
            .header(reqwest::header::USER_AGENT, "StreamNook/1.0")
            .timeout(Duration::from_secs(6))
            .send()
            .await?;

        let apps: Vec<DiscordApp> = response.json().await?;

        {
            let mut cache = DETECTABLES_CACHE.lock().await;
            *cache = Some(DetectablesCache {
                apps: apps.clone(),
                fetched_at: Instant::now(),
            });
        }

        Ok(apps)
    }

    /// Normalize string for comparison (trim + lowercase).
    fn norm(s: &str) -> String {
        s.trim().to_lowercase()
    }

    /// Clean game name by removing common suffixes/prefixes
    fn clean_game_name(name: &str) -> String {
        if name.is_empty() {
            return String::new();
        }

        let mut cleaned = name.to_string();

        // Remove trademark symbols first (they aren't in the patterns)
        cleaned = CLEAN_PATTERNS[0].replace_all(&cleaned, "").to_string();

        // Remove other patterns
        for pattern in CLEAN_PATTERNS.iter().skip(1) {
            cleaned = pattern.replace_all(&cleaned, " ").to_string();
        }

        // Re-join whitespace and normalize
        Self::norm(&cleaned.split_whitespace().collect::<Vec<_>>().join(" "))
    }

    /// Calculate similarity score between two strings.
    fn similarity_score(s1: &str, s2: &str) -> f64 {
        let mut score = 0.0;
        score
    }

    /// Extract the core game name (first part before colon or dash)
    fn extract_core_name(name: &str) -> String {
        for sep in [':', '–', '—', '-'] {
            if let Some((before, _)) = name.split_once(sep) {
                return before.trim().to_string();
            }
        }
        name.trim().to_string()
    }

    /// Create a set of tokens from a game name
    fn tokenize(name: &str) -> HashSet<String> {
        Self::norm(name)
            .split_whitespace()
            .map(String::from)
            .filter(|token| !STOP_WORDS.contains(token.as_str()))
            .collect()
    }

    /// Match a game name to the closest detectable app above the threshold.
    fn match_game(apps: &[DiscordApp], game_name: &str, threshold: f64) -> Option<DiscordApp> {
        if game_name.is_empty() {
            return None;
        }

        // Pre-calculate for the input game_name
        let g_norm = Self::norm(game_name);
        let g_clean = Self::clean_game_name(game_name);
        let g_core = Self::norm(&Self::extract_core_name(game_name));
        let g_tokens = Self::tokenize(game_name);

        let mut best_match: Option<DiscordApp> = None;
        let mut best_score = 0.0;

        for app in apps {
            let app_name = &app.name;
            if app_name.is_empty() {
                continue;
            }

            let mut names_to_check = vec![app_name.clone()];
            names_to_check.extend(app.aliases.clone());

            for check_name in names_to_check {
                let mut score = 0.0;

                let cn_norm = Self::norm(&check_name);
                let cn_clean = Self::clean_game_name(&check_name);
                let cn_core = Self::norm(&Self::extract_core_name(&check_name));
                let cn_tokens = Self::tokenize(&check_name);

                // Start scoring cascade.
                if g_norm == cn_norm {
                    score = 1.0;
                } else if g_clean == cn_clean {
                    score = 0.95;
                } else if !g_core.is_empty() && !cn_core.is_empty() && g_core == cn_core {
                    score = 0.9;
                } else if g_norm.contains(&cn_norm) || cn_norm.contains(&g_norm) {
                    let overlap_len = g_norm.len().min(cn_norm.len()) as f64;
                    let max_len = g_norm.len().max(cn_norm.len()) as f64;
                    score = 0.8 + (0.1 * overlap_len / max_len);
                } else {
                    let sim_norm = Self::similarity_score(&g_norm, &cn_norm);
                    let sim_clean = Self::similarity_score(&g_clean, &cn_clean);
                    let sim_core = if !g_core.is_empty() && !cn_core.is_empty() {
                        Self::similarity_score(&g_core, &cn_core)
                    } else {
                        0.0
                    };

                    score = sim_norm.max(sim_clean).max(sim_core);
                }

                // Token-based score boost
                if !g_tokens.is_empty() && !cn_tokens.is_empty() {
                    let common_tokens = g_tokens.intersection(&cn_tokens).count() as f64;
                    if common_tokens > 0.0 {
                        let token_score =
                            common_tokens / (g_tokens.len().max(cn_tokens.len()) as f64);
                        score = score.max(token_score * 0.85);
                    }
                }

                if score > best_score && score >= threshold {
                    best_score = score;
                    best_match = Some(app.clone());
                }

                // Perfect match, stop early
                if best_score == 1.0 {
                    return best_match;
                }
            }
        }

        best_match
    }

    // --- End: New Matching Logic ---
} // End impl DiscordService

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discord_connect_backoff_gates_attempts() {
        let now = Instant::now();
        // No backoff stored: connect freely.
        assert!(connect_allowed(None, now));
        // An unanswered pipe blocks attempts until the backoff ends.
        let until = backoff_after(true, now).expect("unanswered pipe backs off");
        assert_eq!(until, now + UNANSWERED_BACKOFF);
        assert!(!connect_allowed(Some(until), now));
        assert!(!connect_allowed(Some(until), now + UNANSWERED_BACKOFF - Duration::from_secs(1)));
        assert!(connect_allowed(Some(until), now + UNANSWERED_BACKOFF));
        // A fast failure (Discord not running) or a success clears the backoff.
        assert_eq!(backoff_after(false, now), None);
    }
}
