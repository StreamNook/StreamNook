//! Kick OAuth (Authorization Code + PKCE) — login so we can SEND Kick chat.
//!
//! Like the Twitch credentials, the app's client id/secret are compile-time env
//! vars baked from `.env` (via build.rs), read here with `option_env!` so a build
//! without them still compiles — `connect()` just reports "not configured".
//!
//! Flow: open the system browser to id.kick.com consent, catch the redirect on a
//! localhost:3000 loopback (the app's registered redirect URI), exchange the code
//! at id.kick.com/oauth/token (id + secret + PKCE verifier), cache the token.
//!
//! First slice keeps the token IN MEMORY (per session); keyring persistence like
//! the Twitch tokens is an easy follow-up.

use crate::services::twitch_service::get_app_data_dir;
use anyhow::{anyhow, Result};
use base64::Engine;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::timeout;

const CLIENT_ID: Option<&str> = option_env!("KICK_APP_CLIENT_ID");
const CLIENT_SECRET: Option<&str> = option_env!("KICK_APP_CLIENT_SECRET");
const REDIRECT_URI: &str = "http://localhost:3000/callback";
const AUTHORIZE_URL: &str = "https://id.kick.com/oauth/authorize";
const TOKEN_URL: &str = "https://id.kick.com/oauth/token";
// Exactly the scopes the throwaway probe confirmed work for token exchange + send.
// (events:subscribe is for the future Activity-feed work; its scope string is
// unverified, and an unknown scope makes the whole authorize page bounce.)
const SCOPES: &str = "user:read channel:read chat:write moderation:ban moderation:chat_message:manage";
// Persisted so a Kick login survives app restarts (the token was in-memory only
// before). Keyring is primary; an obfuscated file is the fallback for machines
// where the OS keyring is unavailable.
const KEYRING_SERVICE: &str = "streamnook_kick_token";
const KEYRING_USER: &str = "default";
const OBF_KEY: &[u8] = b"StreamNookKickKey2026";

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct KickToken {
    access_token: String,
    refresh_token: String,
    expires_at: u64, // unix seconds
    #[serde(default)]
    username: Option<String>,
    /// The connected account's own picture, so Accounts can show WHO is signed
    /// in rather than just which platform. Backfilled beside the username.
    #[serde(default)]
    avatar_url: Option<String>,
}

static TOKEN: OnceLock<Mutex<Option<KickToken>>> = OnceLock::new();

fn token_cell() -> &'static Mutex<Option<KickToken>> {
    // Seed from persisted storage on first access, so a prior login is restored.
    TOKEN.get_or_init(|| Mutex::new(load_persisted()))
}

fn token_path() -> Option<PathBuf> {
    get_app_data_dir().ok().map(|d| d.join(".kick_token"))
}

fn obfuscate(data: &[u8]) -> Vec<u8> {
    data.iter()
        .enumerate()
        .map(|(i, b)| b ^ OBF_KEY[i % OBF_KEY.len()])
        .collect()
}

fn persist(tok: &KickToken) {
    let Ok(json) = serde_json::to_string(tok) else {
        return;
    };
    if let Ok(entry) = crate::services::secure_store::Entry::new(KEYRING_SERVICE, KEYRING_USER) {
        let _ = entry.set_password(&json);
    }
    if let Some(p) = token_path() {
        let _ = std::fs::write(p, obfuscate(json.as_bytes()));
    }
}

fn load_persisted() -> Option<KickToken> {
    if let Ok(entry) = crate::services::secure_store::Entry::new(KEYRING_SERVICE, KEYRING_USER) {
        if let Ok(json) = entry.get_password() {
            if let Ok(t) = serde_json::from_str::<KickToken>(&json) {
                return Some(t);
            }
        }
    }
    let p = token_path()?;
    let raw = std::fs::read(p).ok()?;
    let json = String::from_utf8(obfuscate(&raw)).ok()?;
    serde_json::from_str(&json).ok()
}

fn clear_persisted() {
    if let Ok(entry) = crate::services::secure_store::Entry::new(KEYRING_SERVICE, KEYRING_USER) {
        let _ = entry.delete_credential();
    }
    if let Some(p) = token_path() {
        let _ = std::fs::remove_file(p);
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn b64url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn rand_b64(len: usize) -> String {
    // rand 0.10 dropped the root RngCore re-export; fill_bytes now lives on Rng.
    use rand::Rng;
    let mut buf = vec![0u8; len];
    rand::rng().fill_bytes(&mut buf);
    b64url(&buf)
}

fn client_id() -> Option<&'static str> {
    CLIENT_ID.filter(|s| !s.is_empty())
}
fn client_secret() -> Option<&'static str> {
    CLIENT_SECRET.filter(|s| !s.is_empty())
}

pub fn is_connected() -> bool {
    token_cell().lock().map(|t| t.is_some()).unwrap_or(false)
}

/// The connected Kick account's username (for the Connections UI). Returns the
/// cached name; if it's missing (e.g. a token stored before we captured names)
/// but we're connected, it fetches + backfills it so no reconnect is needed.
pub async fn account_name() -> Option<String> {
    account_identity().await.0
}

/// The connected account's picture, if we have fetched it.
pub fn account_avatar() -> Option<String> {
    token_cell()
        .lock()
        .ok()
        .and_then(|t| t.as_ref().and_then(|k| k.avatar_url.clone()))
}

/// Name + picture for the connected account, fetching and caching both on the
/// first call. One request covers them, so they are backfilled together rather
/// than making the avatar a second round trip.
pub async fn account_identity() -> (Option<String>, Option<String>) {
    let cached = token_cell()
        .lock()
        .ok()
        .and_then(|t| t.as_ref().map(|k| (k.username.clone(), k.avatar_url.clone())));
    // Both, or fetch. Returning early on a cached NAME alone would mean an
    // account connected before the avatar was captured could never backfill it —
    // the picture would stay missing for the life of that token.
    if let Some((Some(name), Some(avatar))) = cached.clone() {
        return (Some(name), Some(avatar));
    }
    let Some(access) = access_token().await else {
        return (None, None);
    };
    let Some((name, avatar)) = fetch_identity(&access).await else {
        return (None, None);
    };
    let mut updated: Option<KickToken> = None;
    if let Ok(mut t) = token_cell().lock() {
        if let Some(tok) = t.as_mut() {
            tok.username = Some(name.clone());
            tok.avatar_url = avatar.clone();
            updated = Some(tok.clone());
        }
    }
    if let Some(tok) = updated {
        persist(&tok);
    }
    (Some(name), avatar)
}

/// Fetch the authenticated Kick user's username via the official API (user:read);
/// no query params returns the token owner.
async fn fetch_identity(access_token: &str) -> Option<(String, Option<String>)> {
    let client = reqwest::Client::new();
    let resp = match client
        .get("https://api.kick.com/public/v1/users")
        .bearer_auth(access_token)
        .timeout(Duration::from_secs(8))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            log::warn!("[Kick] fetch_identity request failed: {e}");
            return None;
        }
    };
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        log::warn!("[Kick] fetch_identity HTTP {status}: {body}");
        return None;
    }
    let v: serde_json::Value = resp.json().await.ok()?;
    let name = v
        .pointer("/data/0/name")
        .and_then(|x| x.as_str())
        .map(String::from);
    if name.is_none() {
        log::warn!("[Kick] fetch_identity: no /data/0/name in response: {v}");
    }
    // Kick has spelled this both ways across API versions, so try both rather
    // than losing the avatar to a rename.
    let avatar = ["/data/0/profile_picture", "/data/0/profile_pic"]
        .iter()
        .find_map(|p| v.pointer(p).and_then(|x| x.as_str()))
        .map(String::from);
    Some((name?, avatar))
}

/// Ask Kick whether the stored USER token is still accepted.
///
/// - `Some(true)`  — verified good.
/// - `Some(false)` — Kick rejected it; the token has been cleared.
/// - `None`        — could not tell (offline, timeout, edge block). Nothing changed.
///
/// Two things this must NOT do:
///
/// 1. Use `read_token()`. That falls back to the client-credentials APP token,
///    which is always valid, so a signed-out user would validate as connected.
///    `access_token()` is the user token specifically.
/// 2. Treat 403 as revoked. Kick's public API returns 403 "Request blocked by
///    security policy" to server-side callers holding a perfectly good token
///    (KickDevDocs #281), so only a 401 is real evidence. Signing someone out on
///    a 403 would log them out at random.
pub async fn validate_session() -> Option<bool> {
    let token = access_token().await?;
    let resp = reqwest::Client::new()
        .get("https://api.kick.com/public/v1/users")
        .bearer_auth(&token)
        .timeout(Duration::from_secs(8))
        .send()
        .await
        .ok()?;
    if resp.status().is_success() {
        return Some(true);
    }
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        // A 401 near expiry can just mean the access token aged out between
        // refreshes. Spend one forced refresh before declaring the session dead;
        // only a rejected REFRESH token is proof of real revocation.
        match refresh_now().await {
            RefreshOutcome::Refreshed(_) => return Some(true),
            RefreshOutcome::TryLater => {
                log::debug!("[Kick] 401 but refresh inconclusive; leaving session alone");
                return None;
            }
            RefreshOutcome::Dead => {
                expire_session();
                return Some(false);
            }
        }
    }
    log::debug!("[Kick] session check inconclusive: HTTP {}", resp.status());
    None
}

pub fn disconnect() {
    clear_persisted();
    if let Ok(mut t) = token_cell().lock() {
        *t = None;
    }
    crate::services::providers::emit_platform_account_changed(&["kick"]);
}

#[derive(serde::Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
}

/// Run the full Authorization-Code + PKCE flow and cache the resulting token.
/// An authorization in flight: the bound loopback plus the PKCE material the
/// token exchange will need. Split out from `connect` so the consent page can be
/// opened somewhere other than the system browser — an in-app webview, say —
/// without duplicating any of the protocol.
pub struct PendingAuth {
    listener: TcpListener,
    verifier: String,
    state: String,
}

/// Bind the loopback and build the consent URL. The caller decides where to open
/// it, then hands the `PendingAuth` back to `finish_auth`.
pub async fn begin_auth() -> Result<(String, PendingAuth)> {
    let cid = client_id().ok_or_else(|| {
        anyhow!("Kick app not configured — KICK_APP_CLIENT_ID missing from .env at build time")
    })?;

    // PKCE: verifier (random) + S256 challenge.
    let verifier = rand_b64(48);
    let challenge = b64url(&Sha256::digest(verifier.as_bytes()));
    let state = rand_b64(16);

    // Bind BEFORE the consent page opens so the redirect can't race us.
    let listener = TcpListener::bind("127.0.0.1:3000")
        .await
        .map_err(|e| anyhow!("couldn't bind localhost:3000 for the Kick login redirect: {}", e))?;

    let auth_url = format!(
        "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&code_challenge={}&code_challenge_method=S256&state={}",
        AUTHORIZE_URL,
        urlencoding::encode(cid),
        urlencoding::encode(REDIRECT_URI),
        urlencoding::encode(SCOPES),
        challenge,
        urlencoding::encode(&state),
    );
    Ok((
        auth_url,
        PendingAuth {
            listener,
            verifier,
            state,
        },
    ))
}

/// Wait for the redirect, exchange the code, and store the token.
pub async fn finish_auth(pending: PendingAuth) -> Result<()> {
    let cid = client_id().ok_or_else(|| anyhow!("Kick app not configured"))?;
    let secret = client_secret().ok_or_else(|| anyhow!("Kick app not configured"))?;

    let (code, got_state) = timeout(
        Duration::from_secs(180),
        accept_redirect(pending.listener),
    )
    .await
    .map_err(|_| anyhow!("Kick login timed out (no redirect received)"))??;
    if got_state != pending.state {
        return Err(anyhow!("Kick login state mismatch — aborting"));
    }
    exchange_code(cid, secret, &pending.verifier, &code).await
}

pub async fn connect() -> Result<()> {
    let (auth_url, pending) = begin_auth().await?;
    open_in_browser(&auth_url)?;
    finish_auth(pending).await
}


/// Swap an authorization code for a token and store it.
async fn exchange_code(cid: &str, secret: &str, verifier: &str, code: &str) -> Result<()> {
    let client = reqwest::Client::new();
    let resp = client
        .post(TOKEN_URL)
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", cid),
            ("client_secret", secret),
            ("redirect_uri", REDIRECT_URI),
            ("code_verifier", verifier),
            ("code", code),
        ])
        .send()
        .await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!("Kick token exchange failed (HTTP {}): {}", status, body));
    }
    let tr: TokenResponse = resp.json().await?;
    let identity = fetch_identity(&tr.access_token).await;
    store(KickToken {
        access_token: tr.access_token,
        refresh_token: tr.refresh_token.unwrap_or_default(),
        expires_at: now() + tr.expires_in.unwrap_or(3600),
        username: identity.as_ref().map(|(n, _)| n.clone()),
        avatar_url: identity.and_then(|(_, a)| a),
    });
    Ok(())
}

fn store(tok: KickToken) {
    persist(&tok);
    let was_connected = token_cell().lock().map(|t| t.is_some()).unwrap_or(false);
    if let Ok(mut t) = token_cell().lock() {
        *t = Some(tok);
    }
    // Only a TRANSITION is news. `store` also runs on every silent token refresh,
    // and emitting there would wake every window to learn nothing changed.
    if !was_connected {
        crate::services::providers::emit_platform_account_changed(&["kick"]);
    }
}

/// Accept connections on the loopback until the `/callback` redirect arrives;
/// reply with a friendly page and return the (code, state).
async fn accept_redirect(listener: TcpListener) -> Result<(String, String)> {
    loop {
        let (mut stream, _) = listener.accept().await?;
        let mut buf = vec![0u8; 8192];
        let n = stream.read(&mut buf).await.unwrap_or(0);
        let req = String::from_utf8_lossy(&buf[..n]);
        let first_line = req.lines().next().unwrap_or("");
        if !first_line.contains("/callback") {
            // favicon / preflight / stray request — ack and keep waiting.
            let _ = stream
                .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await;
            continue;
        }
        let query = first_line
            .split('?')
            .nth(1)
            .and_then(|rest| rest.split_whitespace().next())
            .unwrap_or("");
        let mut code = String::new();
        let mut state = String::new();
        for pair in query.split('&') {
            let mut kv = pair.splitn(2, '=');
            match (kv.next(), kv.next()) {
                (Some("code"), Some(v)) => {
                    code = urlencoding::decode(v).map(|c| c.into_owned()).unwrap_or_else(|_| v.to_string())
                }
                (Some("state"), Some(v)) => {
                    state = urlencoding::decode(v).map(|c| c.into_owned()).unwrap_or_else(|_| v.to_string())
                }
                _ => {}
            }
        }
        let html = "<!doctype html><html><body style=\"font-family:system-ui,sans-serif;background:#0e0e10;color:#efeff1;text-align:center;padding-top:80px\"><h2>Kick connected to StreamNook ✓</h2><p>You can close this tab and return to the app.</p></body></html>";
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            html.len(),
            html
        );
        let _ = stream.write_all(resp.as_bytes()).await;
        let _ = stream.flush().await;
        if code.is_empty() {
            return Err(anyhow!("Kick redirect carried no authorization code"));
        }
        return Ok((code, state));
    }
}

fn open_in_browser(url: &str) -> Result<()> {
    // Was a Windows-only `rundll32` call with a `not(windows)` arm that just
    // returned "Kick login is only wired for Windows right now" — which meant the
    // consent leg could never even START off Windows. The quoting hazard that
    // forced rundll32 (and the reason `cmd /C start` is wrong) is documented on
    // `platform::browser::open_in_browser`.
    crate::platform::browser::open_in_browser(url)
        .map_err(|e| anyhow!("couldn't open the browser for Kick login: {}", e))
}

// --- App access token (client credentials) ---------------------------------
//
// Browse, search and who's-live read PUBLIC data from api.kick.com, so they must
// work whether or not the user has connected their Kick account. The
// client-credentials grant mints a server-to-server token from the same app
// credentials the login flow uses (verified 2026-08-20: 200, ~60-day expiry).
// The user token is still preferred when present — same endpoints, and it keeps
// per-user rate limiting rather than pooling every user onto the app token.

struct AppToken {
    access_token: String,
    expires_at: u64,
}

static APP_TOKEN: OnceLock<Mutex<Option<AppToken>>> = OnceLock::new();

fn app_token_cell() -> &'static Mutex<Option<AppToken>> {
    APP_TOKEN.get_or_init(|| Mutex::new(None))
}

/// A token for PUBLIC api.kick.com reads: the connected user's token when there
/// is one, otherwise an app token minted (and cached until near expiry) on
/// demand. `None` only when the app has no baked credentials.
pub async fn read_token() -> Option<String> {
    if let Some(t) = access_token().await {
        return Some(t);
    }
    app_access_token().await
}

/// The cached client-credentials app token, minting a fresh one when absent or
/// within a minute of expiry.
pub async fn app_access_token() -> Option<String> {
    if let Ok(guard) = app_token_cell().lock() {
        if let Some(t) = guard.as_ref() {
            if t.expires_at > now() + 60 {
                return Some(t.access_token.clone());
            }
        }
    }
    let (cid, secret) = (client_id()?, client_secret()?);
    let resp = reqwest::Client::new()
        .post(TOKEN_URL)
        .form(&[
            ("grant_type", "client_credentials"),
            ("client_id", cid),
            ("client_secret", secret),
        ])
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        log::warn!("[Kick] app token request failed: {}", resp.status());
        return None;
    }
    let tr: TokenResponse = resp.json().await.ok()?;
    let access = tr.access_token.clone();
    if let Ok(mut guard) = app_token_cell().lock() {
        *guard = Some(AppToken {
            access_token: tr.access_token,
            expires_at: now() + tr.expires_in.unwrap_or(3600),
        });
    }
    Some(access)
}

/// Outcome of one refresh attempt, so callers can tell "renewed" from "try
/// again later" from "this session is gone".
pub enum RefreshOutcome {
    /// A fresh access token is stored (or another caller just stored one).
    Refreshed(String),
    /// Transient failure (network, 5xx, no credentials). Keep the pair.
    TryLater,
    /// id.kick.com rejected the refresh token itself. The pair is dead: with the
    /// single-flight lock below a rejection can no longer be our own race, so
    /// this means real revocation. The caller decides how loudly to react.
    Dead,
}

/// The single refresh in flight. Kick ROTATES refresh tokens: each one is
/// single-use, and spending it invalidates it. Without this lock, two callers
/// hitting near-expiry together (the 60s live sweep, a chat send, the focus
/// watchdog) both read the SAME refresh token and both spend it; whichever
/// loses gets `invalid_grant`, and out-of-order responses could persist the
/// already-consumed pair. That was a self-inflicted sign-out.
fn refresh_flight() -> &'static tokio::sync::Mutex<()> {
    static FLIGHT: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    FLIGHT.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// Refresh the user token pair, serialized across the whole app.
pub async fn refresh_now() -> RefreshOutcome {
    let _guard = refresh_flight().lock().await;

    // Re-read AFTER acquiring: a caller that queued behind an in-flight refresh
    // finds a fresh token here and must not spend the new refresh token too.
    let Some(cur) = token_cell().lock().ok().and_then(|t| t.clone()) else {
        return RefreshOutcome::Dead;
    };
    if cur.expires_at > now() + 120 {
        return RefreshOutcome::Refreshed(cur.access_token);
    }
    let (cid, secret) = match (client_id(), client_secret()) {
        (Some(a), Some(b)) => (a, b),
        _ => return RefreshOutcome::TryLater,
    };
    if cur.refresh_token.is_empty() {
        return RefreshOutcome::TryLater;
    }
    let client = reqwest::Client::new();
    let resp = match client
        .post(TOKEN_URL)
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", cid),
            ("client_secret", secret),
            ("refresh_token", cur.refresh_token.as_str()),
        ])
        .timeout(Duration::from_secs(10))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            log::debug!("[Kick] token refresh unreachable: {}", e);
            return RefreshOutcome::TryLater;
        }
    };
    let status = resp.status();
    if status == reqwest::StatusCode::BAD_REQUEST || status == reqwest::StatusCode::UNAUTHORIZED {
        log::warn!("[Kick] refresh token rejected (HTTP {}); session is gone", status);
        return RefreshOutcome::Dead;
    }
    if !status.is_success() {
        log::debug!("[Kick] token refresh inconclusive: HTTP {}", status);
        return RefreshOutcome::TryLater;
    }
    let Ok(tr) = resp.json::<TokenResponse>().await else {
        return RefreshOutcome::TryLater;
    };
    let access = tr.access_token.clone();
    store(KickToken {
        access_token: tr.access_token,
        refresh_token: if tr.refresh_token.as_deref().unwrap_or("").is_empty() {
            cur.refresh_token
        } else {
            tr.refresh_token.unwrap()
        },
        expires_at: now() + tr.expires_in.unwrap_or(3600),
        // A refresh renews the token, not the identity — carry both across so a
        // silent refresh can't blank the name and picture in Accounts.
        username: cur.username,
        avatar_url: cur.avatar_url,
    });
    log::debug!("[Kick] user token refreshed");
    RefreshOutcome::Refreshed(access)
}

/// The current access token for the send path, refreshing if it's near expiry.
pub async fn access_token() -> Option<String> {
    let cur = token_cell().lock().ok().and_then(|t| t.clone())?;
    if cur.expires_at > now() + 60 {
        return Some(cur.access_token);
    }
    match refresh_now().await {
        RefreshOutcome::Refreshed(t) => Some(t),
        // Keep handing out the stale token on a transient failure: some Kick
        // endpoints keep accepting it briefly, and the daemon retries shortly.
        RefreshOutcome::TryLater => Some(cur.access_token),
        RefreshOutcome::Dead => {
            expire_session();
            None
        }
    }
}

/// A genuinely dead session: clear it and say so, loudly, in one place.
///
/// This is deliberately distinct from a user-initiated `disconnect()`: the
/// dedicated event lets the UI tell "your session expired, reconnect" apart
/// from "you signed out", so a silently dying token can never leave the app
/// looking connected while showing nobody online.
fn expire_session() {
    log::info!("[Kick] session expired; signing out and notifying the UI");
    disconnect();
    crate::services::providers::emit_platform_session_expired("kick");
}

/// Keep the pair perpetually fresh, independent of traffic.
///
/// Refresh-on-demand alone means the pair only renews when something happens to
/// ask for a token near expiry; this renews it on a clock (the same reason the
/// Twitch login feels immortal: its token is exercised constantly). With the
/// single-flight lock a daemon tick and an on-demand refresh can never race.
pub fn start_refresh_daemon() {
    tauri::async_runtime::spawn(async {
        // Kick access tokens run about two hours; a 5-minute tick with a
        // 20-minute headroom renews well before expiry without hammering.
        let mut tick = tokio::time::interval(Duration::from_secs(300));
        loop {
            tick.tick().await;
            let due = token_cell()
                .lock()
                .ok()
                .and_then(|t| t.clone())
                .map(|t| t.expires_at < now() + 20 * 60);
            match due {
                Some(true) => {
                    if let RefreshOutcome::Dead = refresh_now().await {
                        expire_session();
                    }
                }
                _ => {}
            }
        }
    });
}
