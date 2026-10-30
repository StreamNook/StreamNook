//! The pipe to the Discord client, and the Discord-side lookups presence needs.
//!
//! What the status says is decided in `services/discord_presence.rs`; this
//! file only carries it. Every IPC call is synchronous and blocks until the
//! pipe answers, so each runs on the blocking pool under a deadline, and at
//! most one such call is outstanding at a time: a Discord that accepts the
//! pipe but never answers parks one thread, never one per retry.

use anyhow::Result;
use discord_rich_presence::{activity::Activity, DiscordIpc};
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

// Which IPC transport carries the presence. Windows and Linux use the crate's
// own client; macOS needs a wider socket search than the crate performs (see
// services/discord_ipc_macos.rs). Both implement `DiscordIpc`.
#[cfg(target_os = "macos")]
use crate::services::discord_ipc_macos::MacDiscordIpcClient as IpcClient;
#[cfg(not(target_os = "macos"))]
use discord_rich_presence::DiscordIpcClient as IpcClient;

use regex::Regex;

/// StreamNook's own Discord application (its name is what Discord shows).
pub const DISCORD_CLIENT_ID: &str = "1524098616648663110";

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const IPC_CALL_TIMEOUT: Duration = Duration::from_secs(3);
/// After Discord fails to answer in time, no new connection is attempted for
/// this long.
const UNANSWERED_BACKOFF: Duration = Duration::from_secs(60);

const DETECTABLES_FRESH: Duration = Duration::from_secs(60 * 60);
const DETECTABLES_RETRY: Duration = Duration::from_secs(5 * 60);
const ASSETS_FRESH: Duration = Duration::from_secs(6 * 60 * 60);
const ASSETS_RETRY: Duration = Duration::from_secs(5 * 60);

pub struct DiscordService;

/// The Discord account on the other end of the pipe, from its READY frame.
/// Every URL is built from a checked id and hash only.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DiscordUser {
    /// The account id; empty when READY's was not a Discord id.
    pub id: String,
    pub name: String,
    /// The @handle.
    pub username: String,
    /// The avatar, moving when the account has an animated one.
    pub avatar_url: String,
    /// Its first frame (the same picture when it does not move).
    pub avatar_still: String,
    pub decoration: Option<String>,
    pub decoration_still: Option<String>,
}

struct DiscordState {
    client: Option<IpcClient>,
    user: Option<DiscordUser>,
    // Bumped by `shutdown`. A connect that began under an older epoch closes
    // its client instead of storing it.
    epoch: u64,
    // Set when Discord accepted the pipe but did not answer in time.
    connect_backoff_until: Option<Instant>,
}

lazy_static! {
    static ref DISCORD_STATE: Arc<Mutex<DiscordState>> = Arc::new(Mutex::new(DiscordState {
        client: None,
        user: None,
        epoch: 0,
        connect_backoff_until: None,
    }));
    static ref CONNECT_LOCK: Mutex<()> = Mutex::new(());
}

/// A client is stored (it may still turn out dead on the next write).
static CONNECTED: AtomicBool = AtomicBool::new(false);
/// A blocking IPC call is running, possibly abandoned by its awaiting future.
static IPC_BUSY: AtomicBool = AtomicBool::new(false);

/// Clears `IPC_BUSY` when the blocking closure finishes, however it finishes,
/// and wakes the presence pump, which waits for exactly that.
struct BusyGuard;

impl Drop for BusyGuard {
    fn drop(&mut self) {
        IPC_BUSY.store(false, Ordering::SeqCst);
        crate::services::discord_presence::wake();
    }
}

/// Why a send or connect did not happen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendError {
    /// Discord is not running, refused the pipe, or the pipe broke.
    NoPipe,
    /// Discord accepted the pipe and did not answer in time; backing off.
    Unanswered,
    /// A recent unanswered attempt is still backing off; nothing was tried.
    BackingOff,
    /// An earlier blocking call is still parked; nothing was tried.
    Busy,
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SendError::NoPipe => write!(f, "Discord is not reachable"),
            SendError::Unanswered => write!(f, "Discord did not answer in time"),
            SendError::BackingOff => write!(f, "Discord did not answer recently, waiting before retrying"),
            SendError::Busy => write!(f, "an earlier Discord call has not returned"),
        }
    }
}

/// Whether a connection attempt may start now, given the current backoff.
fn connect_allowed(backoff_until: Option<Instant>, now: Instant) -> bool {
    backoff_until.is_none_or(|until| now >= until)
}

/// The backoff to store after an attempt. Only an unanswered pipe backs off.
fn backoff_after(unanswered: bool, now: Instant) -> Option<Instant> {
    unanswered.then(|| now + UNANSWERED_BACKOFF)
}

/// Run a blocking closure on the blocking pool, giving up after `limit`. On
/// timeout the closure keeps its thread until the pipe returns; `IPC_BUSY`
/// stays set until then, so nothing else is started meanwhile.
async fn run_bounded<T, F>(limit: Duration, f: F) -> Option<std::result::Result<T, String>>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    IPC_BUSY.store(true, Ordering::SeqCst);
    let handle = tauri::async_runtime::spawn_blocking(move || {
        let _busy = BusyGuard;
        f()
    });
    match tokio::time::timeout(limit, handle).await {
        Ok(joined) => Some(joined.map_err(|e| e.to_string())),
        Err(_) => None,
    }
}

/// Close a client on the blocking pool without waiting for it. Not counted as
/// busy: it closes a pipe that already failed a write (or a spare from a race),
/// and counting it would refuse the reconnect that follows at once.
fn close_detached(mut client: IpcClient) {
    drop(tauri::async_runtime::spawn_blocking(move || {
        let _ = client.close();
    }));
}

/// A Discord picture hash: 32 lowercase hex digits, `a_` first when it moves.
fn image_hash(s: &str) -> bool {
    let h = s.strip_prefix("a_").unwrap_or(s);
    h.len() == 32 && h.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// A Discord id: 17 to 20 digits.
fn snowflake(s: &str) -> bool {
    (17..=20).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit())
}

/// The account a READY frame names: its id, names, avatar (moving and still,
/// or the default avatar Discord draws for it) and avatar decoration.
pub(crate) fn parse_ready(value: &Value) -> Option<DiscordUser> {
    let user = value.get("data")?.get("user")?;
    let text = |key: &str| user.get(key).and_then(Value::as_str).filter(|s| !s.is_empty());
    let name = text("global_name").or_else(|| text("username"))?.to_string();
    let username = text("username").unwrap_or("").to_string();
    let raw_id = text("id").unwrap_or("");
    let id = if snowflake(raw_id) { raw_id } else { "" };
    let (avatar_url, avatar_still) = match text("avatar").filter(|h| image_hash(h)) {
        Some(hash) if !id.is_empty() => {
            let still = format!("https://cdn.discordapp.com/avatars/{id}/{hash}.webp?size=128");
            let moving = if hash.starts_with("a_") {
                format!("{still}&animated=true")
            } else {
                still.clone()
            };
            (moving, still)
        }
        _ => {
            let discriminator = text("discriminator").and_then(|d| d.parse::<u64>().ok()).unwrap_or(0);
            let index = if discriminator != 0 {
                discriminator % 5
            } else {
                raw_id.parse::<u64>().map(|n| (n >> 22) % 6).unwrap_or(0)
            };
            let url = format!("https://cdn.discordapp.com/embed/avatars/{index}.png");
            (url.clone(), url)
        }
    };
    let asset = user
        .get("avatar_decoration_data")
        .and_then(|d| d.get("asset"))
        .and_then(Value::as_str)
        .filter(|h| image_hash(h));
    let (decoration, decoration_still) = match asset {
        Some(asset) => (
            Some(format!(
                "https://cdn.discordapp.com/avatar-decoration-presets/{asset}.png?size=160&passthrough=true"
            )),
            Some(format!("https://cdn.discordapp.com/avatar-decoration-presets/{asset}.png?size=160")),
        ),
        None => (None, None),
    };
    Some(DiscordUser {
        id: id.to_string(),
        name,
        username,
        avatar_url,
        avatar_still,
        decoration,
        decoration_still,
    })
}

impl DiscordService {
    /// A client is stored.
    pub fn connected() -> bool {
        CONNECTED.load(Ordering::SeqCst)
    }

    /// A blocking IPC call has not returned yet.
    pub fn busy() -> bool {
        IPC_BUSY.load(Ordering::SeqCst)
    }

    /// When the unanswered-pipe backoff ends, if one is running.
    pub async fn backoff_until() -> Option<Instant> {
        DISCORD_STATE.lock().await.connect_backoff_until
    }

    /// Make sure a client is stored and return the account it reached.
    pub async fn connect() -> std::result::Result<Option<DiscordUser>, SendError> {
        Self::ensure_connected().await?;
        Ok(Self::user().await)
    }

    /// The account Discord is signed in to, for the editor's preview while the
    /// status is off: one handshake, its READY read, the pipe closed again. No
    /// activity is set and no client is kept. A stored client's account is
    /// returned as it is. Waits out the same backoff as a connect.
    pub async fn peek_user() -> std::result::Result<Option<DiscordUser>, SendError> {
        if Self::busy() {
            return Err(SendError::Busy);
        }
        let _connecting = CONNECT_LOCK.lock().await;
        {
            let guard = DISCORD_STATE.lock().await;
            if guard.client.is_some() {
                return Ok(guard.user.clone());
            }
            if !connect_allowed(guard.connect_backoff_until, Instant::now()) {
                return Err(SendError::BackingOff);
            }
        }
        let outcome = run_bounded(CONNECT_TIMEOUT, || -> Result<Option<DiscordUser>> {
            let (mut client, user) = Self::connect_client()?;
            let _ = client.close();
            Ok(user)
        })
        .await;
        let mut guard = DISCORD_STATE.lock().await;
        guard.connect_backoff_until = backoff_after(outcome.is_none(), Instant::now());
        match outcome {
            None => Err(SendError::Unanswered),
            Some(Ok(Ok(user))) => {
                if user.is_some() {
                    guard.user = user.clone();
                }
                Ok(user)
            }
            _ => Err(SendError::NoPipe),
        }
    }

    /// The account the stored client reached (refreshed on every connect,
    /// including the reconnect inside `send`).
    pub async fn user() -> Option<DiscordUser> {
        DISCORD_STATE.lock().await.user.clone()
    }

    /// Set (`Some`) or clear (`None`) the activity, reconnecting once if the
    /// stored pipe turned out dead.
    pub async fn send(activity: Option<Activity<'static>>) -> std::result::Result<(), SendError> {
        Self::ensure_connected().await?;
        let first = activity.clone();
        match Self::call(first).await {
            Ok(()) => return Ok(()),
            Err(SendError::NoPipe) => {}
            Err(other) => return Err(other),
        }
        Self::ensure_connected().await?;
        Self::call(activity).await
    }

    /// One set or clear on the stored client.
    async fn call(activity: Option<Activity<'static>>) -> std::result::Result<(), SendError> {
        if Self::busy() {
            return Err(SendError::Busy);
        }
        let mut guard = DISCORD_STATE.lock().await;
        let Some(mut client) = guard.client.take() else {
            CONNECTED.store(false, Ordering::SeqCst);
            return Err(SendError::NoPipe);
        };
        let outcome = run_bounded(IPC_CALL_TIMEOUT, move || {
            let result = match activity {
                Some(a) => client.set_activity(a),
                None => client.clear_activity(),
            }
            .map_err(|e| e.to_string());
            (client, result)
        })
        .await;
        match outcome {
            None => {
                CONNECTED.store(false, Ordering::SeqCst);
                guard.connect_backoff_until = backoff_after(true, Instant::now());
                Err(SendError::Unanswered)
            }
            Some(Err(_)) => {
                CONNECTED.store(false, Ordering::SeqCst);
                Err(SendError::NoPipe)
            }
            Some(Ok((client, Ok(())))) => {
                guard.client = Some(client);
                Ok(())
            }
            Some(Ok((client, Err(e)))) => {
                log::debug!("[Discord] write failed: {e}");
                CONNECTED.store(false, Ordering::SeqCst);
                drop(guard);
                close_detached(client);
                Err(SendError::NoPipe)
            }
        }
    }

    /// Clear the activity and close the pipe. Never rate limited: turning the
    /// feature off hides the status at once.
    pub async fn shutdown() {
        let client = {
            let mut guard = DISCORD_STATE.lock().await;
            guard.epoch = guard.epoch.wrapping_add(1);
            // Turning the feature off and on again is a deliberate retry.
            guard.connect_backoff_until = None;
            CONNECTED.store(false, Ordering::SeqCst);
            guard.client.take()
        };
        if let Some(mut client) = client {
            let goodbye = run_bounded(IPC_CALL_TIMEOUT, move || {
                let _ = client.clear_activity();
                let _ = client.close();
            })
            .await;
            if goodbye.is_none() {
                log::debug!("[Discord] shutdown: Discord did not answer within {} s", IPC_CALL_TIMEOUT.as_secs());
            }
        }
    }

    /// Store a client, opening one if needed. The handshake runs off the
    /// async thread under CONNECT_TIMEOUT and without DISCORD_STATE held.
    async fn ensure_connected() -> std::result::Result<(), SendError> {
        if Self::busy() {
            return Err(SendError::Busy);
        }
        let _connecting = CONNECT_LOCK.lock().await;

        let epoch = {
            let guard = DISCORD_STATE.lock().await;
            if guard.client.is_some() {
                return Ok(());
            }
            if !connect_allowed(guard.connect_backoff_until, Instant::now()) {
                return Err(SendError::BackingOff);
            }
            guard.epoch
        };

        let outcome = run_bounded(CONNECT_TIMEOUT, Self::connect_client).await;

        let mut guard = DISCORD_STATE.lock().await;
        guard.connect_backoff_until = backoff_after(outcome.is_none(), Instant::now());
        let (client, user) = match outcome {
            None => return Err(SendError::Unanswered),
            Some(Err(_)) => return Err(SendError::NoPipe),
            Some(Ok(Err(e))) => {
                log::debug!("[Discord] connect failed: {e}");
                return Err(SendError::NoPipe);
            }
            Some(Ok(Ok(pair))) => pair,
        };

        if guard.epoch != epoch {
            drop(guard);
            close_detached(client);
            return Err(SendError::NoPipe);
        }
        if guard.client.is_some() {
            drop(guard);
            close_detached(client);
            return Ok(());
        }
        guard.client = Some(client);
        if user.is_some() {
            guard.user = user;
        }
        CONNECTED.store(true, Ordering::SeqCst);
        Ok(())
    }

    /// Open an IPC connection and read the READY frame, which names the
    /// account. The crate's `connect()` discards that frame. A CLOSE frame or
    /// an unreadable answer leaves the account unknown.
    ///
    /// Blocking: only ever called through `run_bounded`.
    fn connect_client() -> Result<(IpcClient, Option<DiscordUser>)> {
        let mut client = IpcClient::new(DISCORD_CLIENT_ID);
        client.connect_ipc()?;
        client.send(json!({ "v": 1, "client_id": DISCORD_CLIENT_ID }), 0)?;
        let user = read_frame(&mut client).ok().and_then(|value| parse_ready(&value));
        Ok((client, user))
    }
}

/// The most a frame from the pipe may claim to hold. READY is a few hundred
/// bytes; the crate's own `recv` allocates whatever length the peer writes.
const MAX_FRAME: usize = 64 * 1024;

/// One frame's JSON body, refusing a length over `MAX_FRAME` before reading it.
fn read_frame(client: &mut IpcClient) -> Result<Value> {
    let mut header = [0u8; 8];
    client.read(&mut header)?;
    let length = frame_length(&header)?;
    let mut body = vec![0u8; length];
    client.read(&mut body)?;
    Ok(serde_json::from_slice(&body)?)
}

/// The body length an 8-byte frame header announces (opcode, then length,
/// both little-endian u32), when it is within `MAX_FRAME`.
fn frame_length(header: &[u8; 8]) -> Result<usize> {
    let length = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
    if length > MAX_FRAME {
        anyhow::bail!("Discord frame of {length} bytes refused");
    }
    Ok(length)
}

impl DiscordService {

    // ---- Category art -------------------------------------------------------------------

    /// The matched Discord game's icon for a category name (D13: equal names
    /// only), `NoMatch` when Discord lists no such game, `Unavailable` when
    /// the list could not be read (worth asking again later).
    pub async fn category_icon(category: &str) -> CategoryIcon {
        let key = category.trim().to_lowercase();
        if key.is_empty() {
            return CategoryIcon::NoMatch;
        }
        if let Some(hit) = MATCHES.lock().await.get(&key) {
            return hit.clone().map_or(CategoryIcon::NoMatch, CategoryIcon::Match);
        }
        let Some(apps) = Self::detectables().await else {
            return CategoryIcon::Unavailable;
        };
        let found = match_game(&apps, category).and_then(|app| {
            app.icon_hash
                .as_ref()
                .filter(|h| !h.is_empty())
                .map(|h| format!("https://cdn.discordapp.com/app-icons/{}/{}.png?size=512", app.id, h))
        });
        MATCHES.lock().await.insert(key, found.clone());
        found.map_or(CategoryIcon::NoMatch, CategoryIcon::Match)
    }

    /// Discord's detectable-games list (about 13 MB), cached an hour (a
    /// failure five minutes). One download at a time: a second caller waits
    /// for the first and reads its result.
    async fn detectables() -> Option<Arc<Vec<DiscordApp>>> {
        let fresh_copy = || async {
            let cache = DETECTABLES.lock().await;
            match cache.as_ref() {
                Some((apps, at)) => {
                    let fresh = if apps.is_some() { DETECTABLES_FRESH } else { DETECTABLES_RETRY };
                    (at.elapsed() < fresh).then(|| apps.clone())
                }
                None => None,
            }
        };
        if let Some(apps) = fresh_copy().await {
            return apps;
        }
        let _downloading = DETECTABLES_FETCH.lock().await;
        if let Some(apps) = fresh_copy().await {
            return apps;
        }
        let fetched = async {
            let response = crate::services::http::client()
                .get("https://discord.com/api/v9/applications/detectable")
                .header(reqwest::header::USER_AGENT, "StreamNook/1.0")
                .timeout(Duration::from_secs(6))
                .send()
                .await
                .ok()?;
            if !response.status().is_success() {
                return None;
            }
            response.json::<Vec<DiscordApp>>().await.ok().map(Arc::new)
        }
        .await;
        if fetched.is_some() {
            MATCHES.lock().await.clear();
        }
        *DETECTABLES.lock().await = Some((fetched.clone(), Instant::now()));
        fetched
    }

    /// Whether an image URL answers 200 without a redirect. Twitch answers a
    /// box-art name it does not know with a redirect to its grey placeholder.
    /// `None` when the probe itself failed (not cached, so it is retried).
    pub async fn image_exists(url: &str) -> Option<bool> {
        if let Some(known) = PROBES.lock().await.get(url) {
            return Some(*known);
        }
        let response = crate::services::http::client_no_redirect()
            .head(url)
            .header(reqwest::header::USER_AGENT, "StreamNook/1.0")
            .timeout(Duration::from_secs(4))
            .send()
            .await
            .ok()?;
        let exists = response.status() == reqwest::StatusCode::OK;
        PROBES.lock().await.insert(url.to_string(), exists);
        Some(exists)
    }

    /// The app's uploaded Rich Presence assets, name to id (`streamnook_logo`,
    /// and the platform badges once uploaded). Cached six hours, a failure
    /// five minutes. Empty when unknown.
    pub async fn asset_ids() -> Arc<HashMap<String, String>> {
        {
            let cache = ASSETS.lock().await;
            if let Some((ids, ok, at)) = cache.as_ref() {
                let fresh = if *ok { ASSETS_FRESH } else { ASSETS_RETRY };
                if at.elapsed() < fresh {
                    return ids.clone();
                }
            }
        }
        let fetched = async {
            let response = crate::services::http::client()
                .get(format!("https://discord.com/api/v9/oauth2/applications/{DISCORD_CLIENT_ID}/assets"))
                .header(reqwest::header::USER_AGENT, "StreamNook/1.0")
                .timeout(Duration::from_secs(6))
                .send()
                .await
                .ok()?;
            if !response.status().is_success() {
                return None;
            }
            let list = response.json::<Vec<AssetRow>>().await.ok()?;
            Some(list.into_iter().map(|a| (a.name, a.id)).collect::<HashMap<_, _>>())
        }
        .await;
        let ok = fetched.is_some();
        let mut cache = ASSETS.lock().await;
        let ids = match fetched {
            Some(map) => Arc::new(map),
            // A failed refresh keeps the last good list, so badges stay on
            // their asset keys instead of flipping to links and back.
            None => cache.as_ref().map(|(ids, _, _)| ids.clone()).unwrap_or_default(),
        };
        *cache = Some((ids.clone(), ok, Instant::now()));
        ids
    }
}

/// What Discord's game list says about a category.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CategoryIcon {
    Match(String),
    NoMatch,
    Unavailable,
}

#[derive(Debug, Clone, Deserialize)]
struct AssetRow {
    id: String,
    name: String,
}

#[derive(Debug, Clone, Deserialize)]
struct DiscordApp {
    id: String,
    name: String,
    #[serde(default)]
    icon_hash: Option<String>,
    #[serde(default)]
    aliases: Vec<String>,
}

lazy_static! {
    static ref DETECTABLES: Mutex<Option<(Option<Arc<Vec<DiscordApp>>>, Instant)>> = Mutex::new(None);
    static ref DETECTABLES_FETCH: Mutex<()> = Mutex::new(());
    static ref MATCHES: Mutex<HashMap<String, Option<String>>> = Mutex::new(HashMap::new());
    static ref PROBES: Mutex<HashMap<String, bool>> = Mutex::new(HashMap::new());
    static ref ASSETS: Mutex<Option<(Arc<HashMap<String, String>>, bool, Instant)>> = Mutex::new(None);
    // Patterns for normalizing game names before matching.
    static ref CLEAN_PATTERNS: Vec<Regex> = vec![
        Regex::new(r"[®™©]").unwrap(),
        Regex::new(r"\s*\(.*?\)").unwrap(),
        Regex::new(r"\s*\[.*?\]").unwrap(),
        Regex::new(r":\s*[Tt]he\s+").unwrap(),
        Regex::new(r"\s*-\s*[Ss]eason\s+\d+").unwrap(),
        Regex::new(r"\s*[Ee]dition$").unwrap(),
        Regex::new(r"\s*[Rr]emastered$").unwrap(),
        Regex::new(r"\s*[Dd]efinitive\s+[Ee]dition$").unwrap(),
    ];
}

/// Trim and lowercase.
fn norm(s: &str) -> String {
    s.trim().to_lowercase()
}

/// A game name without trademark marks, bracketed notes and edition words.
fn clean_game_name(name: &str) -> String {
    if name.is_empty() {
        return String::new();
    }
    let mut cleaned = CLEAN_PATTERNS[0].replace_all(name, "").to_string();
    for pattern in CLEAN_PATTERNS.iter().skip(1) {
        cleaned = pattern.replace_all(&cleaned, " ").to_string();
    }
    norm(&cleaned.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// Letters and digits only, lowercased: "Counter-Strike 2" and "Counter
/// Strike 2" fold to the same word.
fn fold_words(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// The Discord game whose name or alias equals `game` after trimming and
/// lowercasing, after `clean_game_name`, or after `fold_words`, in that order
/// of preference. Never a partial match: Twitch categories like "IRL" or
/// "Music" share letters with unrelated games.
fn match_game<'a>(apps: &'a [DiscordApp], game: &str) -> Option<&'a DiscordApp> {
    let g_norm = norm(game);
    let g_clean = clean_game_name(game);
    let g_fold = fold_words(game);
    if g_fold.is_empty() {
        return None;
    }
    let mut best: Option<(&DiscordApp, u8)> = None;
    for app in apps {
        for name in std::iter::once(&app.name).chain(app.aliases.iter()) {
            if name.is_empty() {
                continue;
            }
            let score = if norm(name) == g_norm {
                3
            } else if !g_clean.is_empty() && clean_game_name(name) == g_clean {
                2
            } else if fold_words(name) == g_fold {
                1
            } else {
                0
            };
            if score == 3 {
                return Some(app);
            }
            if score > best.map(|(_, s)| s).unwrap_or(0) {
                best = Some((app, score));
            }
        }
    }
    best.map(|(app, _)| app)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discord_connect_backoff_gates_attempts() {
        let now = Instant::now();
        assert!(connect_allowed(None, now));
        let until = backoff_after(true, now).expect("unanswered pipe backs off");
        assert_eq!(until, now + UNANSWERED_BACKOFF);
        assert!(!connect_allowed(Some(until), now));
        assert!(!connect_allowed(Some(until), now + UNANSWERED_BACKOFF - Duration::from_secs(1)));
        assert!(connect_allowed(Some(until), now + UNANSWERED_BACKOFF));
        assert_eq!(backoff_after(false, now), None);
    }

    fn app(id: &str, name: &str, aliases: &[&str]) -> DiscordApp {
        DiscordApp {
            id: id.into(),
            name: name.into(),
            icon_hash: Some("h".into()),
            aliases: aliases.iter().map(|a| a.to_string()).collect(),
        }
    }

    #[test]
    fn category_art_matches_only_the_real_game() {
        let apps = vec![
            app("1", "Catgirl", &[]),
            app("2", "MUSICUS!", &[]),
            app("3", "Carto", &[]),
            app("4", "Sports: Renovations", &[]),
            app("5", "Chess2", &[]),
            app("6", "Just Crow Things", &[]),
            app("7", "Minecraft", &["Minecraft Launcher"]),
            app("8", "Grand Theft Auto V Legacy", &["Grand Theft Auto V"]),
            app("9", "Counter-Strike 2", &[]),
            app("10", "Raft", &[]),
        ];
        for category in ["IRL", "Music", "Art", "Sports", "Chess", "Just Chatting", "Makers & Crafting"] {
            assert!(match_game(&apps, category).is_none(), "{category} must find no game");
        }
        assert_eq!(match_game(&apps, "minecraft").map(|a| a.id.as_str()), Some("7"));
        assert_eq!(match_game(&apps, "Grand Theft Auto V").map(|a| a.id.as_str()), Some("8"));
        assert_eq!(match_game(&apps, "Counter Strike 2").map(|a| a.id.as_str()), Some("9"));
    }

    #[test]
    fn ready_frame_gives_the_discord_user() {
        let id = "80351110224678912";
        let hash = "0123456789abcdef0123456789abcdef";
        let deco = "fedcba9876543210fedcba9876543210";
        let moving = json!({ "evt": "READY", "data": { "user": {
            "id": id, "username": "nelly", "global_name": "Nelly", "avatar": format!("a_{hash}"), "discriminator": "0",
            "avatar_decoration_data": { "asset": deco, "sku_id": "1144058522808614923" }
        }}});
        assert_eq!(
            parse_ready(&moving),
            Some(DiscordUser {
                id: id.into(),
                name: "Nelly".into(),
                username: "nelly".into(),
                avatar_url: format!("https://cdn.discordapp.com/avatars/{id}/a_{hash}.webp?size=128&animated=true"),
                avatar_still: format!("https://cdn.discordapp.com/avatars/{id}/a_{hash}.webp?size=128"),
                decoration: Some(format!(
                    "https://cdn.discordapp.com/avatar-decoration-presets/{deco}.png?size=160&passthrough=true"
                )),
                decoration_still: Some(format!("https://cdn.discordapp.com/avatar-decoration-presets/{deco}.png?size=160")),
            })
        );
        let still = json!({ "data": { "user": { "id": id, "username": "nelly", "avatar": hash } } });
        let u = parse_ready(&still).unwrap();
        assert_eq!(u.avatar_url, u.avatar_still, "a still avatar is one picture");
        assert_eq!(u.decoration, None);
        // A hash or id that is not Discord's never reaches a URL.
        let forged = json!({ "data": { "user": {
            "id": id, "username": "x", "avatar": "../../evil?", "avatar_decoration_data": { "asset": "x\"y" }
        }}});
        let u = parse_ready(&forged).unwrap();
        assert!(u.avatar_url.starts_with("https://cdn.discordapp.com/embed/avatars/"));
        assert_eq!((u.decoration, u.decoration_still), (None, None));
        let bad_id = json!({ "data": { "user": { "id": "12/../3", "username": "x", "avatar": hash } } });
        let u = parse_ready(&bad_id).unwrap();
        assert_eq!(u.id, "");
        assert_eq!(u.avatar_url, "https://cdn.discordapp.com/embed/avatars/0.png");

        let new_default = json!({ "data": { "user": { "id": id, "username": "nelly", "global_name": "" } } });
        let expected = (80351110224678912u64 >> 22) % 6;
        assert_eq!(
            parse_ready(&new_default).map(|u| (u.name, u.avatar_url)),
            Some(("nelly".into(), format!("https://cdn.discordapp.com/embed/avatars/{expected}.png")))
        );
        let legacy = json!({ "data": { "user": { "id": "1", "username": "old", "discriminator": "1337" } } });
        assert_eq!(
            parse_ready(&legacy).map(|u| (u.name, u.avatar_url)),
            Some(("old".into(), "https://cdn.discordapp.com/embed/avatars/2.png".into()))
        );
        assert_eq!(parse_ready(&json!({ "evt": "ERROR" })), None);
    }

    #[test]
    fn oversized_frames_are_refused() {
        let mut header = [0u8; 8];
        header[4..8].copy_from_slice(&(512u32).to_le_bytes());
        assert_eq!(frame_length(&header).unwrap(), 512);
        header[4..8].copy_from_slice(&(u32::MAX).to_le_bytes());
        assert!(frame_length(&header).is_err(), "never allocates what the peer claims");
    }
}
