//! Multi-account registry. Phase 1 foundation for the "send as" picker and,
//! later, multi-account actions (automation, whispers, mod tools).
//!
//! Design constraints (deliberate, see Brain `projects/StreamNook.md` ->
//! "Multi-account support"):
//!
//!   - The PRIMARY account (the one you watch / stream as) keeps its exact
//!     existing storage: the obfuscated `.twitch_token` file, the main cookie
//!     jar, and the legacy keyring entry (`streamnook_twitch_token` / `user`).
//!     `TwitchService::get_token()` remains the unchanged hot path for it. This
//!     module never reads or writes the primary's token storage; it only records
//!     the primary's *identity* so the account list has a complete view.
//!
//!   - SECONDARY ("action") accounts store their OAuth token in the OS keyring
//!     keyed by Twitch user id, plus an obfuscated file backup. They NEVER touch
//!     the cookie jar: the cookie jar is the single web session, which belongs
//!     to the primary alone.
//!
//!   - Phase 1 ships no UI. It establishes the data model, persistence, a
//!     refresh-aware per-account token accessor, and a cheap startup reconcile
//!     that records the current login as the primary. Add / remove / set-primary
//!     flows land in later phases.

use anyhow::Result;
use keyring::Entry;
use log::{debug, warn};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use crate::services::twitch_service::{get_app_data_dir, StorableToken, TwitchService};

/// Plain-JSON registry of every known account (primary + secondaries). The
/// metadata here is not secret (login, display name, avatar); the actual tokens
/// live in the keyring / obfuscated per-account files.
const ACCOUNTS_FILE_NAME: &str = "accounts.json";

/// Keyring service name for SECONDARY account tokens. Deliberately distinct from
/// the primary's legacy `streamnook_twitch_token` / `user` entry so the two can
/// never collide. The keyring username is the account's Twitch user id.
const ACCOUNT_KEYRING_SERVICE: &str = "streamnook_twitch_account";

/// Matches the XOR obfuscation key TwitchService uses for the primary token file,
/// so secondary token files are stored with the same (light) at-rest scheme.
const TOKEN_OBFUSCATION_KEY: &[u8] = b"StreamNookTokenKey2024";

/// The forced-logout switch. Bump this string to sign EVERY user out and back in
/// on their next launch. Changing it is the only action required to ship a forced
/// re-auth: on startup the value is compared against the marker file on disk and,
/// if they differ, every credential and web session is wiped exactly once, then
/// the new value is recorded so it never repeats for that value.
///
/// Reach for it when a change to how auth is stored means existing sessions can't
/// satisfy the new layout (a new scope, a new profile scheme, a credential moving
/// stores). A date-plus-tag value keeps successive forced re-auths self-documenting.
const FORCE_REAUTH_TOKEN: &str = "2026-06-18-clean-auth";

/// Records the last `FORCE_REAUTH_TOKEN` already applied. Lives in the app data
/// dir next to the token files, NOT in WebView localStorage, so the session wipe
/// it triggers can never erase its own "already done" record.
const FORCE_REAUTH_MARKER_FILE: &str = ".force_reauth";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredAccount {
    pub user_id: String,
    pub login: String,
    pub display_name: String,
    #[serde(default)]
    pub avatar_url: Option<String>,
    #[serde(default)]
    pub is_primary: bool,
    /// Unix seconds when the account was linked. 0 for the migrated primary.
    #[serde(default)]
    pub added_at: i64,
}

pub struct AccountStore;

impl AccountStore {
    // ----- registry (metadata) persistence -------------------------------

    fn accounts_file_path() -> Result<PathBuf> {
        let mut path = get_app_data_dir()?;
        if !path.exists() {
            fs::create_dir_all(&path)?;
        }
        path.push(ACCOUNTS_FILE_NAME);
        Ok(path)
    }

    /// Every known account, primary first. Empty if nothing has been recorded
    /// yet (e.g. a brand-new install, or before the first startup reconcile).
    pub fn list() -> Vec<StoredAccount> {
        let path = match Self::accounts_file_path() {
            Ok(p) => p,
            Err(_) => return Vec::new(),
        };
        if !path.exists() {
            return Vec::new();
        }
        match fs::read_to_string(&path).map(|s| serde_json::from_str::<Vec<StoredAccount>>(&s)) {
            Ok(Ok(mut accounts)) => {
                accounts.sort_by(|a, b| b.is_primary.cmp(&a.is_primary));
                accounts
            }
            _ => {
                warn!("[accounts] registry unreadable or malformed; treating as empty");
                Vec::new()
            }
        }
    }

    fn save(accounts: &[StoredAccount]) -> Result<()> {
        let path = Self::accounts_file_path()?;
        let json = serde_json::to_string_pretty(accounts)?;
        fs::write(&path, json)?;
        // Per-account stores cache "who is primary" and would keep writing to
        // the departing account's file for the length of their TTL. This is the
        // one chokepoint every registry mutation passes through (set_active,
        // sign_out_active, reset_all), so invalidating here cannot be missed by
        // a future path the way three separate call sites could.
        crate::services::vod_progress_service::invalidate_owner();
        Ok(())
    }

    /// Number of distinct accounts. The frontend uses this to decide whether to
    /// render the "send as" picker (only shown when there are 2 or more).
    pub fn count() -> usize {
        Self::list().len()
    }

    pub fn get(user_id: &str) -> Option<StoredAccount> {
        Self::list().into_iter().find(|a| a.user_id == user_id)
    }

    pub fn primary() -> Option<StoredAccount> {
        Self::list().into_iter().find(|a| a.is_primary)
    }

    // ----- secondary token storage (keyring + obfuscated file) -----------

    fn xor(data: &[u8]) -> Vec<u8> {
        data.iter()
            .enumerate()
            .map(|(i, b)| b ^ TOKEN_OBFUSCATION_KEY[i % TOKEN_OBFUSCATION_KEY.len()])
            .collect()
    }

    fn secondary_token_file_path(user_id: &str) -> Result<PathBuf> {
        let mut path = get_app_data_dir()?;
        if !path.exists() {
            fs::create_dir_all(&path)?;
        }
        path.push(format!(".twitch_account_{}", user_id));
        Ok(path)
    }

    fn store_secondary_token(user_id: &str, token: &StorableToken) -> Result<()> {
        let json = serde_json::to_string(token)?;

        // File (primary store for secondaries), obfuscated to match the primary scheme.
        let path = Self::secondary_token_file_path(user_id)?;
        fs::write(&path, Self::xor(json.as_bytes()))?;

        // Keyring (backup), keyed by user id. Best-effort, like the primary path.
        if let Ok(entry) = Entry::new(ACCOUNT_KEYRING_SERVICE, user_id) {
            let _ = entry.set_password(&json);
        }
        Ok(())
    }

    fn load_secondary_token(user_id: &str) -> Result<StorableToken> {
        // File first.
        if let Ok(path) = Self::secondary_token_file_path(user_id) {
            if path.exists() {
                if let Ok(bytes) = fs::read(&path) {
                    let decoded = Self::xor(&bytes);
                    if let Ok(s) = String::from_utf8(decoded) {
                        if let Ok(token) = serde_json::from_str::<StorableToken>(&s) {
                            return Ok(token);
                        }
                    }
                }
            }
        }
        // Keyring fallback.
        if let Ok(entry) = Entry::new(ACCOUNT_KEYRING_SERVICE, user_id) {
            if let Ok(pwd) = entry.get_password() {
                if let Ok(token) = serde_json::from_str::<StorableToken>(&pwd) {
                    return Ok(token);
                }
            }
        }
        Err(anyhow::anyhow!(
            "No stored token for secondary account {}",
            user_id
        ))
    }

    fn delete_secondary_token(user_id: &str) -> Result<()> {
        if let Ok(path) = Self::secondary_token_file_path(user_id) {
            if path.exists() {
                let _ = fs::remove_file(&path);
            }
        }
        if let Ok(entry) = Entry::new(ACCOUNT_KEYRING_SERVICE, user_id) {
            let _ = entry.delete_credential();
        }
        Ok(())
    }

    // ----- public accessors / mutators -----------------------------------

    /// Resolve a usable access token for ANY account, refreshing if it is within
    /// five minutes of expiry. The primary delegates to the unchanged
    /// `TwitchService::get_token()`; secondaries use their own stored token and
    /// refresh through the shared `TwitchService::refresh_token`.
    pub async fn get_token_for(user_id: &str) -> Result<String> {
        if let Some(primary) = Self::primary() {
            if primary.user_id == user_id {
                return TwitchService::get_token().await;
            }
        }

        let mut token = Self::load_secondary_token(user_id)?;
        let buffer = 300; // 5-minute pre-expiry refresh window, matching the primary.
        if token.expires_at > 0 && chrono::Utc::now().timestamp() >= token.expires_at - buffer {
            if token.refresh_token.is_empty() {
                return Err(anyhow::anyhow!(
                    "Account {} token expired and has no refresh token; re-link it.",
                    user_id
                ));
            }
            let refreshed = TwitchService::refresh_token(&token.refresh_token).await?;
            Self::store_secondary_token(user_id, &refreshed)?;
            token = refreshed;
        }
        Ok(token.access_token)
    }

    /// Link a new secondary account from a freshly obtained token. Identifies the
    /// account from the token itself, stores the token, and upserts it into the
    /// registry as a non-primary account. Returns the stored metadata.
    /// (Phase 2's add-account flow calls this.)
    pub async fn add_secondary(token: StorableToken) -> Result<StoredAccount> {
        let info = TwitchService::get_user_info_with_token(&token.access_token).await?;

        // Don't allow the primary to also be added as a secondary.
        if let Some(primary) = Self::primary() {
            if primary.user_id == info.id {
                return Err(anyhow::anyhow!(
                    "That account is already signed in as your primary."
                ));
            }
        }

        Self::store_secondary_token(&info.id, &token)?;

        let account = StoredAccount {
            user_id: info.id.clone(),
            login: info.login,
            display_name: info.display_name,
            avatar_url: info.profile_image_url,
            is_primary: false,
            added_at: chrono::Utc::now().timestamp(),
        };

        let mut accounts = Self::list();
        accounts.retain(|a| a.user_id != account.user_id);
        accounts.push(account.clone());
        Self::save(&accounts)?;
        debug!("[accounts] linked secondary account @{}", account.login);
        Ok(account)
    }

    /// Remove a secondary account (its token and registry entry). The primary is
    /// not removable here; that belongs to logout (Phase 4).
    pub fn remove_secondary(user_id: &str) -> Result<()> {
        if let Some(primary) = Self::primary() {
            if primary.user_id == user_id {
                return Err(anyhow::anyhow!("Cannot remove the primary account here."));
            }
        }
        Self::delete_secondary_token(user_id)?;
        // Drop the account's isolated Twitch web session too, so an unlinked
        // account leaves no lingering browser profile behind.
        crate::services::twitch_service::delete_twitch_web_profile(user_id);
        let mut accounts = Self::list();
        accounts.retain(|a| a.user_id != user_id);
        Self::save(&accounts)?;
        Ok(())
    }

    /// Record the current login as the main account in the registry, and keep a
    /// backup copy of its token so it can be demoted losslessly later. Called on
    /// startup with the user id Twitch's validate endpoint already returned.
    ///
    /// Non-destructive: when a DIFFERENT account is found occupying the primary
    /// slot (a re-login as someone else), the previous main is demoted to a
    /// linked account rather than deleted. Always best-effort: any failure is
    /// logged and swallowed so login is never broken.
    pub async fn reconcile_primary(validated_user_id: &str) {
        let accounts = Self::list();
        let current_primary = accounts.iter().find(|a| a.is_primary).cloned();

        // Fast path: the registry already agrees with the signed-in account.
        // Mirror the live slot token into this account's own store as a backup,
        // so if a different account later takes the slot, this one can be demoted
        // to a usable linked account without losing its token.
        if let Some(p) = &current_primary {
            if p.user_id == validated_user_id {
                if let Ok(token) = TwitchService::load_primary_token().await {
                    let _ = Self::store_secondary_token(validated_user_id, &token);
                }
                return;
            }
        }

        // A different account now occupies the primary slot. Record it as the new
        // main WITHOUT deleting the previous one: the old main is demoted to a
        // linked account (its token was mirrored on a prior clean startup, so it
        // usually survives intact; worst case it just needs a re-link).
        let info = match TwitchService::get_user_info().await {
            Ok(i) => i,
            Err(e) => {
                debug!("[accounts] could not record primary identity yet: {}", e);
                return;
            }
        };

        let mut next = accounts;
        // Demote everyone; the new main is re-marked below.
        for a in next.iter_mut() {
            a.is_primary = false;
        }
        // The new main's token lives in the primary slot now, so a stale
        // secondary copy (if it was previously a linked account) is redundant.
        let _ = Self::delete_secondary_token(&info.id);

        if let Some(existing) = next.iter_mut().find(|a| a.user_id == info.id) {
            existing.is_primary = true;
            existing.login = info.login;
            existing.display_name = info.display_name;
            existing.avatar_url = info.profile_image_url;
        } else {
            next.insert(
                0,
                StoredAccount {
                    user_id: info.id,
                    login: info.login,
                    display_name: info.display_name,
                    avatar_url: info.profile_image_url,
                    is_primary: true,
                    added_at: 0,
                },
            );
        }

        match Self::save(&next) {
            Ok(_) => debug!("[accounts] primary reconciled (non-destructive)"),
            Err(e) => warn!("[accounts] failed to record primary: {}", e),
        }
    }

    /// Promote a linked account to the active (main) account: the one you watch
    /// and stream as. Lossless and non-destructive:
    ///   1. capture the outgoing main's token from the primary slot and file it
    ///      in that account's own store, so the old main survives as a linked
    ///      account you can switch back to;
    ///   2. move the target account's token into the primary slot;
    ///   3. flip the `is_primary` flags, keeping every account in the registry.
    ///
    /// Errors (without changing anything) if the target is unknown or has no
    /// usable stored token, so a failed switch never strands the current main.
    pub async fn set_active(user_id: &str) -> Result<StoredAccount> {
        let accounts = Self::list();
        let target = accounts
            .iter()
            .find(|a| a.user_id == user_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("That account is not linked."))?;

        if target.is_primary {
            return Ok(target); // already the main
        }

        // Resolve the target's token BEFORE touching the primary slot, so any
        // failure here leaves the current main fully intact.
        let target_token = Self::load_secondary_token(user_id).map_err(|_| {
            anyhow::anyhow!(
                "@{} needs to be signed in again before it can become your main.",
                target.login
            )
        })?;

        // Preserve the outgoing main losslessly: copy its slot token into its own
        // store so it becomes a normal linked account.
        if let Some(old) = accounts.iter().find(|a| a.is_primary) {
            if let Ok(old_token) = TwitchService::load_primary_token().await {
                let _ = Self::store_secondary_token(&old.user_id, &old_token);
            }
        }

        // Move the target's token into the primary slot, then drop its secondary
        // copy so the token has exactly one home.
        TwitchService::persist_primary_token(&target_token).await?;
        let _ = Self::delete_secondary_token(user_id);

        let mut next = accounts;
        for a in next.iter_mut() {
            a.is_primary = a.user_id == user_id;
        }
        Self::save(&next)?;
        debug!("[accounts] switched main account to @{}", target.login);
        Self::get(user_id).ok_or_else(|| anyhow::anyhow!("Account vanished after switch"))
    }

    /// Sign out of the current main. If other accounts are linked, the most
    /// recently added one is promoted to main (so signing out lands you on a
    /// linked account instead of fully logged out) and returned; the signed-out
    /// account is removed entirely. If it was the only account, this is a full
    /// logout and returns `None`.
    pub async fn sign_out_active() -> Result<Option<StoredAccount>> {
        let accounts = Self::list();
        let leaving_id = accounts
            .iter()
            .find(|a| a.is_primary)
            .map(|p| p.user_id.clone());

        // Promotion candidates: every linked account other than the one leaving,
        // most-recently-added first.
        let mut candidates: Vec<StoredAccount> = accounts
            .iter()
            .filter(|a| Some(&a.user_id) != leaving_id.as_ref())
            .cloned()
            .collect();
        candidates.sort_by(|a, b| b.added_at.cmp(&a.added_at));

        // Pick the first candidate that actually has a usable token to promote.
        let promote = candidates.into_iter().find_map(|c| {
            Self::load_secondary_token(&c.user_id)
                .ok()
                .map(|tok| (c, tok))
        });

        // Clear the outgoing main's web session + primary slot, drop any backup
        // copy of its token, and remove its isolated Twitch web profile.
        let _ = TwitchService::logout().await;
        if let Some(lid) = &leaving_id {
            let _ = Self::delete_secondary_token(lid);
            crate::services::twitch_service::delete_twitch_web_profile(lid);
        }

        if let Some((next, next_token)) = promote {
            // Promote the chosen account into the now-empty primary slot.
            TwitchService::persist_primary_token(&next_token).await?;
            let _ = Self::delete_secondary_token(&next.user_id);

            let mut rebuilt: Vec<StoredAccount> = Self::list()
                .into_iter()
                .filter(|a| Some(&a.user_id) != leaving_id.as_ref())
                .collect();
            for a in rebuilt.iter_mut() {
                a.is_primary = a.user_id == next.user_id;
            }
            Self::save(&rebuilt)?;
            debug!("[accounts] signed out main; promoted @{}", next.login);
            return Ok(Some(next));
        }

        // Nothing to promote: a full logout. Drop the lone (now tokenless)
        // primary entry so the registry doesn't keep a ghost.
        if let Some(lid) = &leaving_id {
            let remaining: Vec<StoredAccount> = Self::list()
                .into_iter()
                .filter(|a| &a.user_id != lid)
                .collect();
            let _ = Self::save(&remaining);
        }
        debug!("[accounts] signed out; no other account to promote (full logout)");
        Ok(None)
    }

    /// Full reset for a forced re-auth (e.g. a scopes upgrade invalidates every
    /// stored token at once). Unlike `sign_out_active`, this promotes nothing:
    /// every account's token is equally invalid, so they all need a fresh login.
    /// Clears the primary token stores, drops every secondary token, deletes every
    /// per-account Twitch web profile, and empties the registry.
    ///
    /// Emptying the registry is the load-bearing part: it forces the subsequent
    /// login to open in the DEFAULT WebView2 profile — the only profile
    /// `TwitchAuthService::get_token` reads. If the registry survived a forced
    /// re-auth, the re-login would reopen in the leaving account's per-account
    /// profile and the freshly-set `auth-token` web cookie would land where the
    /// stream resolver can't see it (the "non-proxy mode requires a twitch login"
    /// dead-end even though the user just logged in).
    pub async fn reset_all() {
        let accounts = Self::list();

        // Clear the primary token stores (file + cookies + keyring).
        let _ = TwitchService::logout().await;

        // The drops/points credential is a separate device login; clear it too
        // so a forced re-auth doesn't keep crediting a now-invalid account.
        let _ = crate::services::drops_auth_service::DropsAuthService::logout().await;

        // Drop every account's own token + isolated web profile so nothing
        // lingers to re-hydrate a stale, scope-deficient session.
        for a in &accounts {
            let _ = Self::delete_secondary_token(&a.user_id);
            crate::services::twitch_service::delete_twitch_web_profile(&a.user_id);
        }

        // Empty the registry so `primary()` is None and the next login uses the
        // default profile.
        let _ = Self::save(&[]);

        // The other platforms are accounts too, and a forced re-auth that left
        // them signed in was an odd half-measure: the app would claim to be
        // connected to Kick and YouTube while insisting Twitch start over. Each
        // service owns its own teardown (tokens, cookies, webview profile), and
        // the two live under DIFFERENT roots — Kick's under app-local data,
        // YouTube's under config — so this delegates rather than deleting paths.
        crate::services::kick_auth_service::disconnect();
        crate::services::youtube_auth_service::disconnect();

        debug!(
            "[accounts] reset_all: cleared {} account(s) + platform sessions for forced re-auth",
            accounts.len()
        );
    }

    fn force_reauth_marker_path() -> Result<PathBuf> {
        let mut path = get_app_data_dir()?;
        if !path.exists() {
            fs::create_dir_all(&path)?;
        }
        path.push(FORCE_REAUTH_MARKER_FILE);
        Ok(path)
    }

    /// Run the one-time forced re-auth, gated on `FORCE_REAUTH_TOKEN`. Returns
    /// true when a wipe was performed this launch.
    ///
    /// MUST be called before any twitch webview is created (from `main()`, before
    /// the Tauri builder builds the windows) so the per-account profiles and the
    /// default WebView2 store are unlocked and the deletes actually land. A prior
    /// attempt ran from the live frontend, where the running session held those
    /// files locked, so the wipe silently no-opped and users stayed signed in.
    ///
    /// Best-effort and self-healing: the marker is written ONLY after the wipe, so
    /// a failed or interrupted run just repeats on the next launch instead of
    /// marking itself done.
    pub async fn run_force_reauth_if_needed() -> bool {
        let marker = match Self::force_reauth_marker_path() {
            Ok(p) => p,
            Err(e) => {
                warn!("[accounts] force-reauth: could not resolve marker path: {e}");
                return false;
            }
        };

        if fs::read_to_string(&marker).unwrap_or_default().trim() == FORCE_REAUTH_TOKEN {
            return false;
        }

        debug!("[accounts] force-reauth '{FORCE_REAUTH_TOKEN}': clearing every session");

        // App + drops tokens, every per-account web profile, and the registry.
        Self::reset_all().await;

        // Legacy single-profile sessions and the main window's own web store are
        // not owned by reset_all/logout, so wipe them too while nothing holds
        // them open.
        wipe_default_webview_store();

        match fs::write(&marker, FORCE_REAUTH_TOKEN) {
            Ok(_) => debug!("[accounts] force-reauth applied; marker written"),
            Err(e) => warn!(
                "[accounts] force-reauth: marker write failed; will retry next launch: {e}"
            ),
        }
        true
    }
}

/// Remove the WebView2 stores that `AccountStore::reset_all` does not own: the
/// whole `twitch_web_profiles` tree (every per-account profile plus the pending
/// stage, in case one outlived the registry) and the main window's default
/// `EBWebView` store under each known app-data location. Best-effort; a path
/// that's absent or briefly locked is logged and skipped.
fn wipe_default_webview_store() {
    let mut targets: Vec<PathBuf> = Vec::new();

    if let Ok(base) = get_app_data_dir() {
        // Belt-and-suspenders over reset_all's per-account deletes: drop the whole
        // profiles tree so an orphaned profile can't rehydrate a session.
        targets.push(base.join("twitch_web_profiles"));
    }

    // The main window's own web store. Tauri places it under the bundle id or the
    // product name depending on build, so cover both under every app-data root.
    for root in [dirs::config_dir(), dirs::data_dir(), dirs::data_local_dir()]
        .into_iter()
        .flatten()
    {
        targets.push(root.join("StreamNook").join("EBWebView"));
        targets.push(root.join("com.streamnook.dev").join("EBWebView"));
    }

    for path in targets {
        if path.exists() {
            if let Err(e) = fs::remove_dir_all(&path) {
                warn!("[accounts] force-reauth: could not remove {path:?}: {e}");
            }
        }
    }
}
