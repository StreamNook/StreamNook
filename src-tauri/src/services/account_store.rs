//! Multi-account registry. Phase 1 foundation for the "send as" picker and,
//! later, multi-account actions (automation, whispers, mod tools).
//!
//! Design constraints (deliberate, see Brain `projects/StreamNook.md` ->
//! "Multi-account support"):
//!
//!   - The PRIMARY account (the one you watch / stream as) keeps its own slot,
//!     the sealed `.twitch_token` file, and `TwitchService::get_token()` remains
//!     the hot path for it. This module only touches that slot through
//!     `TwitchService`'s primary-slot methods; otherwise it records the
//!     primary's *identity* so the account list has a complete view.
//!
//!   - SECONDARY ("action") accounts keep their OAuth token in their own sealed
//!     `.twitch_account_<user id>` file (see `token_vault`).
//!
//!   - Phase 1 ships no UI. It establishes the data model, persistence, a
//!     refresh-aware per-account token accessor, and a cheap startup reconcile
//!     that records the current login as the primary. Add / remove / set-primary
//!     flows land in later phases.
//!
//!   - Every door that moves the account generation keeps this registry in
//!     step: `set_active` and `sign_out_active` write it themselves, a primary
//!     sign-in records its account (`reconcile_after_sign_in`), and
//!     `TwitchService::logout` drops the departed main (`forget_primary`).

use anyhow::Result;
use crate::services::token_vault;
use log::{debug, warn};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use crate::models::user::UserInfo;
use crate::services::twitch_service::{get_app_data_dir, StorableToken, TwitchService};

/// Plain-JSON registry of every known account (primary + secondaries). The
/// metadata here is not secret (login, display name, avatar); the actual tokens
/// live in sealed per-account files.
const ACCOUNTS_FILE_NAME: &str = "accounts.json";


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

/// Seconds before expiry a token is renewed.
const REFRESH_BUFFER: i64 = 300;

/// The registry file's reader-writer lock (D248): `list()` reads under it
/// shared, `save` writes under it exclusively, so a reader sees the old file
/// or the new one, never a truncated, missing or locked one. A poisoned lock
/// is recovered with `into_inner` (the data is a file, not memory).
static REGISTRY_IO: RwLock<()> = RwLock::new(());

/// Today's registry read, as a seam over a path: an empty list for a missing
/// file, and an empty list (with a warning) for one that does not parse.
/// Callers hold `REGISTRY_IO` shared.
fn read_registry(path: &std::path::Path) -> Vec<StoredAccount> {
    if !path.exists() {
        return Vec::new();
    }
    match fs::read_to_string(path).map(|s| serde_json::from_str::<Vec<StoredAccount>>(&s)) {
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

/// A linked account's file read, typed (D250, D258): the load is sorted
/// through `token_error_for`. No file, or a file that does not parse (a
/// 0-byte file, `{}`), is `NoToken` under today's words; any other failure is
/// `VaultUnavailable`.
fn secondary_load(load: Result<Option<StorableToken>>, user_id: &str) -> Result<StorableToken> {
    use crate::services::twitch_service::TokenError;
    let kind = match TwitchService::token_error_for(&load) {
        None => return load.map(|t| t.expect("token_error_for says a token was read")),
        Some(kind) => kind,
    };
    let words = match (&load, kind) {
        (Err(e), TokenError::VaultUnavailable) => format!("{e:#}"),
        _ => format!("No stored token for secondary account {}", user_id),
    };
    Err(kind.into_error(words))
}

type SlotRead = std::result::Result<Option<StorableToken>, crate::services::twitch_service::TokenError>;

/// Whether an expiring file may be refreshed, given what the slot read
/// answered. Never when the slot cannot be read (the file may hold the slot's
/// own credential) or holds the same refresh token (the slot owns it); yes
/// when the slot is empty or holds a different one.
fn may_refresh_file(file: &StorableToken, slot: &SlotRead) -> bool {
    match slot {
        Err(_) => false,
        Ok(Some(slot)) => file.refresh_token.is_empty() || file.refresh_token != slot.refresh_token,
        Ok(None) => true,
    }
}

/// Held across an account door; lowers its `AltRefresh`'s change counter when
/// dropped.
#[must_use]
pub(crate) struct AccountChangeGuard {
    change: Arc<AtomicUsize>,
}

impl Drop for AccountChangeGuard {
    fn drop(&mut self) {
        self.change.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Everything the alt-account refresh shares, in one value (D256): the
/// per-account single-flight locks (one per linked account, so two refreshes
/// of one single-use refresh token never run at once; entries stay, bounded
/// by accounts ever linked; the std mutex is never held across an await), the
/// and the account-change counter. The app keeps
/// one (`ALT_REFRESH`) behind one-line delegations; every test builds its own.
pub(crate) struct AltRefresh {
    map: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    change: Arc<AtomicUsize>,
}

impl AltRefresh {
    pub(crate) fn new() -> AltRefresh {
        AltRefresh { map: Mutex::new(HashMap::new()), change: Arc::new(AtomicUsize::new(0)) }
    }

    fn guard(&self, id: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.map.lock().unwrap_or_else(|e| e.into_inner()).entry(id.to_string()).or_default().clone()
    }

    /// An account door is about to run. Raises the counter (so no refresh
    /// starts) and waits out the refreshes in flight by taking and dropping
    /// every per-account lock. The guard exists before the first await, so a
    /// dropped caller still lowers the counter.
    pub(crate) async fn begin_account_change(&self) -> AccountChangeGuard {
        self.change.fetch_add(1, Ordering::SeqCst);
        let guard = AccountChangeGuard { change: self.change.clone() };
        let locks: Vec<Arc<tokio::sync::Mutex<()>>> =
            self.map.lock().unwrap_or_else(|e| e.into_inner()).values().cloned().collect();
        for lock in locks {
            drop(lock.lock_owned().await);
        }
        guard
    }

    /// A linked account's token: today's `TwitchService::refresh_token` (the
    /// shared client, unchanged) does the renewing (D264).
    pub(crate) async fn refresh_alt<L, S>(&self, id: &str, load: L, store: S) -> Result<String>
    where
        L: FnOnce() -> Result<StorableToken>,
        S: FnOnce(&StorableToken) -> Result<()> + Send + 'static,
    {
        self.single_flight_refresh(
            id,
            load,
            TwitchService::read_slot_token,
            move |rt| async move { TwitchService::refresh_token(&rt).await },
            store,
        )
        .await
    }

    /// Load `id`'s token, and renew it when it is near expiry, under the
    /// account's own lock. A fresh token is returned whatever else is going
    /// on. Before it spends the refresh token, on the value it loaded under
    /// that lock, it refuses (an `Unreachable` "try again" error under today's
    /// network words) while an account door is running, and when
    /// `may_refresh_file` says the slot holds the same credential or cannot be
    /// read. The renewal and the store run on a spawned task that owns the
    /// guard, so a caller that goes away never cancels a refresh between
    /// Twitch's answer and the store (a dropped Tauri `JoinHandle` detaches,
    /// never aborts). Steps are closures so tests use fakes. Keeps today's
    /// rules: `expires_at == 0` never renews; the errors carry their kind
    /// (D250) under today's words.
    pub(crate) async fn single_flight_refresh<L, SR, SRF, R, RF, S>(
        &self,
        id: &str,
        load: L,
        slot_read: SR,
        refresh: R,
        store: S,
    ) -> Result<String>
    where
        L: FnOnce() -> Result<StorableToken>,
        SR: FnOnce() -> SRF,
        SRF: std::future::Future<Output = SlotRead>,
        R: FnOnce(String) -> RF + Send + 'static,
        RF: std::future::Future<Output = Result<StorableToken>> + Send + 'static,
        S: FnOnce(&StorableToken) -> Result<()> + Send + 'static,
    {
        let guard = self.guard(id).lock_owned().await;
        let token = load()?;
        if !(token.expires_at > 0 && chrono::Utc::now().timestamp() >= token.expires_at - REFRESH_BUFFER) {
            return Ok(token.access_token);
        }
        if token.refresh_token.is_empty() {
            return Err(crate::services::twitch_service::TokenError::Refused
                .into_error(format!("Account {} token expired and has no refresh token; re-link it.", id)));
        }
        // An account door is running: it may be moving this very token, so
        // nothing starts a refresh now.
        if self.change.load(Ordering::SeqCst) > 0 {
            return Err(TwitchService::refresh_unavailable("an account change is in progress"));
        }
        // Under the alt's guard (lock order: alt guard, then the slot), on the
        // value just loaded, before the refresh task is spawned.
        if !may_refresh_file(&token, &slot_read().await) {
            return Err(TwitchService::refresh_unavailable(
                "the sign-in is shared with your main account or could not be checked",
            ));
        }
        let refresh_token = token.refresh_token;
        let task = tauri::async_runtime::spawn(async move {
            let _guard = guard;
            let refreshed = refresh(refresh_token).await?;
            store(&refreshed)?;
            Ok::<String, anyhow::Error>(refreshed.access_token)
        });
        task.await.map_err(|e| TwitchService::refresh_unavailable(&format!("the refresh task failed: {e}")))?
    }
}

/// The app's one alt-refresh state. Named only by `AccountStore::get_token_for`
/// and `AccountStore::begin_account_change` (a pin checks it).
static ALT_REFRESH: std::sync::LazyLock<AltRefresh> = std::sync::LazyLock::new(AltRefresh::new);

/// Whether Twitch reports a different login than the one on file. Logins are
/// lowercase on Twitch, so a case difference alone is not a rename.
fn login_changed(stored: &str, validated: &str) -> bool {
    !validated.is_empty() && !stored.eq_ignore_ascii_case(validated)
}

/// The registry with `info` as the main account: every other account demoted
/// and kept, `info`'s own entry refreshed in place, or added first.
fn with_primary(mut accounts: Vec<StoredAccount>, info: UserInfo) -> Vec<StoredAccount> {
    for a in accounts.iter_mut() {
        a.is_primary = false;
    }
    if let Some(existing) = accounts.iter_mut().find(|a| a.user_id == info.id) {
        existing.is_primary = true;
        existing.login = info.login;
        existing.display_name = info.display_name;
        existing.avatar_url = info.profile_image_url;
    } else {
        accounts.insert(
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
    accounts
}

/// The registry without its main account, and that account's id (None when
/// there was no main, which leaves the registry as it was).
fn without_primary(accounts: Vec<StoredAccount>) -> (Vec<StoredAccount>, Option<String>) {
    let leaving = accounts.iter().find(|a| a.is_primary).map(|a| a.user_id.clone());
    let rest = accounts.into_iter().filter(|a| !a.is_primary).collect();
    (rest, leaving)
}

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
        // Shared with every other reader; a save holds it exclusively (D248).
        let _registry = REGISTRY_IO.read().unwrap_or_else(|e| e.into_inner());
        read_registry(&path)
    }

    /// The registry write: pretty JSON, atomic (temp file, `sync_all`, rename),
    /// and exclusive against every reader for just that write. The lock is
    /// released before any invalidation runs.
    fn save_at(path: &std::path::Path, accounts: &[StoredAccount]) -> Result<()> {
        let json = serde_json::to_string_pretty(accounts)?;
        {
            let _registry = REGISTRY_IO.write().unwrap_or_else(|e| e.into_inner());
            crate::services::token_vault::write_file_atomic(path, json.as_bytes())?;
        }
        Ok(())
    }

    fn save(accounts: &[StoredAccount]) -> Result<()> {
        let path = Self::accounts_file_path()?;
        Self::save_at(&path, accounts)?;
        // Per-account stores cache "who is primary" and would keep writing to
        // the departing account's file for the length of their TTL. This is the
        // one chokepoint every registry mutation passes through (set_active,
        // sign_out_active, reset_all), so invalidating here cannot be missed by
        // a future path the way three separate call sites could.
        crate::services::vod_progress_service::invalidate_owner();
        crate::services::vod_sync::reset();
        // Chat rows leave out the viewer's own StreamNook badge by these ids.
        crate::services::chat_row_view::set_own_accounts(accounts.iter().map(|a| a.user_id.clone()).collect());
        // Remembered failed token fetches are forgotten and the rules recompile
        // (a no-op before the engine starts; only reached after a good write).
        crate::services::account_automations::on_accounts_changed();
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

    // ----- secondary token storage (sealed per-account file) ---------------

    fn secondary_token_file_path(user_id: &str) -> Result<PathBuf> {
        let mut path = get_app_data_dir()?;
        if !path.exists() {
            fs::create_dir_all(&path)?;
        }
        path.push(format!(".twitch_account_{}", user_id));
        Ok(path)
    }

    fn store_secondary_token(user_id: &str, token: &StorableToken) -> Result<()> {
        token_vault::store_json(&Self::secondary_token_file_path(user_id)?, token)
    }

    fn load_secondary_token(user_id: &str) -> Result<StorableToken> {
        secondary_load(token_vault::load_json(&Self::secondary_token_file_path(user_id)?), user_id)
    }

    fn delete_secondary_token(user_id: &str) -> Result<()> {
        token_vault::remove(&Self::secondary_token_file_path(user_id)?)
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

        // One lock per account, a refresh no caller can cancel, no refresh of a
        // token the slot also holds or while an account door runs (D241 to D256).
        let id = user_id.to_string();
        ALT_REFRESH.refresh_alt(user_id, || Self::load_secondary_token(user_id), move |t| Self::store_secondary_token(&id, t)).await
    }

    /// An account door is about to run: no alt refresh starts, and the ones in
    /// flight finish, until the guard drops. A one-line delegation (a pin
    /// checks it).
    pub(crate) async fn begin_account_change() -> AccountChangeGuard {
        ALT_REFRESH.begin_account_change().await
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
    /// startup with the user id and login Twitch's validate endpoint returned.
    ///
    /// Non-destructive: when a DIFFERENT account is found occupying the primary
    /// slot (a re-login as someone else), the previous main is demoted to a
    /// linked account rather than deleted. Always best-effort: any failure is
    /// logged and swallowed so login is never broken.
    pub async fn reconcile_primary(validated_user_id: &str, validated_login: Option<&str>) {
        let accounts = Self::list();
        if accounts.iter().any(|a| a.is_primary && a.user_id == validated_user_id) {
            Self::keep_primary(accounts, validated_user_id, validated_login).await;
            return;
        }

        // A different account now occupies the primary slot.
        let info = match TwitchService::get_user_info().await {
            Ok(i) => i,
            Err(e) => {
                debug!("[accounts] could not record primary identity yet: {}", e);
                return;
            }
        };
        Self::record_primary(accounts, info);
    }

    /// A sign-in door just put `token` in the primary slot and moved the account
    /// generation to `generation`. Records that token's account as the main one,
    /// as the startup reconcile would, so `primary()` names it from now on
    /// instead of from the next launch. Skipped when another door moved the
    /// account while Twitch was being asked who this is. Best-effort, like
    /// `reconcile_primary`: a failure is logged and the sign-in still stands.
    pub async fn reconcile_after_sign_in(token: &StorableToken, generation: u64) {
        let info = match TwitchService::get_user_info_with_token(&token.access_token).await {
            Ok(i) => i,
            Err(e) => {
                warn!("[accounts] could not identify the account that signed in: {}", e);
                return;
            }
        };
        if TwitchService::account_generation() != generation {
            debug!("[accounts] the account moved again during sign-in; not recording it");
            return;
        }
        let accounts = Self::list();
        if accounts.iter().any(|a| a.is_primary && a.user_id == info.id) {
            Self::keep_primary(accounts, &info.id, Some(&info.login)).await;
        } else {
            Self::record_primary(accounts, info);
        }
    }

    /// The registry already names the signed-in account as main. Record a
    /// rename if Twitch reports one, and mirror the live slot token into this
    /// account's own store as a backup, so if a different account later takes
    /// the slot, this one can be demoted to a usable linked account without
    /// losing its token.
    async fn keep_primary(accounts: Vec<StoredAccount>, user_id: &str, validated_login: Option<&str>) {
        let stored_login = accounts.iter().find(|a| a.user_id == user_id).map(|a| a.login.clone());
        if let (Some(stored), Some(login)) = (stored_login, validated_login) {
            if login_changed(&stored, login) {
                Self::record_rename(accounts, user_id, login).await;
            }
        }
        if let Ok(token) = TwitchService::load_primary_token().await {
            let _ = Self::store_secondary_token(user_id, &token);
        }
    }

    /// A different account occupies the primary slot. Record it as the new main
    /// WITHOUT deleting the previous one: the old main is demoted to a linked
    /// account (its token was mirrored on a prior clean startup, so it usually
    /// survives intact; worst case it just needs a re-link).
    fn record_primary(accounts: Vec<StoredAccount>, info: UserInfo) {
        // The new main's token lives in the primary slot now, so a stale
        // secondary copy (if it was previously a linked account) is redundant.
        let _ = Self::delete_secondary_token(&info.id);
        match Self::save(&with_primary(accounts, info)) {
            Ok(_) => debug!("[accounts] primary reconciled (non-destructive)"),
            Err(e) => warn!("[accounts] failed to record primary: {}", e),
        }
    }

    /// The main account signed out of the primary slot: drop it from the
    /// registry along with its backup token, so `primary()` names nobody until
    /// the next sign-in records one, and the next sign-in opens in the staging
    /// web profile instead of the departed account's. Linked accounts stay
    /// linked. `TwitchService::logout`, the one sign-out door, calls this;
    /// best-effort.
    pub fn forget_primary() {
        let (rest, leaving) = without_primary(Self::list());
        let Some(leaving) = leaving else {
            return;
        };
        let _ = Self::delete_secondary_token(&leaving);
        match Self::save(&rest) {
            Ok(_) => debug!("[accounts] main account signed out; registry has no main"),
            Err(e) => warn!("[accounts] failed to drop the signed-out main: {}", e),
        }
    }

    /// The main account was renamed on Twitch: its id stayed, its login did not.
    /// Everything that reads the login from here (the badge collection, `/me`,
    /// the send limiter's own-login check) would keep asking for a name that is
    /// gone. Takes the display name and avatar from Twitch too when it answers;
    /// otherwise the validated login alone is recorded.
    async fn record_rename(mut accounts: Vec<StoredAccount>, user_id: &str, login: &str) {
        let info = TwitchService::get_user_info()
            .await
            .ok()
            .filter(|i| i.id == user_id);
        let Some(account) = accounts.iter_mut().find(|a| a.user_id == user_id) else {
            return;
        };
        let previous = std::mem::replace(&mut account.login, login.to_string());
        match info {
            Some(i) => {
                account.login = i.login;
                account.display_name = i.display_name;
                account.avatar_url = i.profile_image_url;
            }
            None => account.display_name = login.to_string(),
        }
        match Self::save(&accounts) {
            Ok(_) => debug!(
                "[accounts] main account renamed on Twitch: @{} is now @{}",
                previous, login
            ),
            Err(e) => warn!(
                "[accounts] failed to record the main account's new login: {}",
                e
            ),
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

        // Hold the primary slot from reading the outgoing main to writing the
        // new one, so a token refresh can never land in between and be filed
        // under the wrong account.
        {
            let slot = TwitchService::primary_slot().await;
            // Preserve the outgoing main losslessly: copy its slot token into its
            // own store so it becomes a normal linked account.
            if let Some(old) = accounts.iter().find(|a| a.is_primary) {
                if let Ok(old_token) = TwitchService::load_primary_token().await {
                    let _ = Self::store_secondary_token(&old.user_id, &old_token);
                }
            }

            // Move the target's token into the primary slot.
            TwitchService::persist_primary_token(&slot, &target_token).await?;
            TwitchService::bump_account_generation();
        }
        // Then drop its secondary copy so the token has exactly one home.
        let _ = Self::delete_secondary_token(user_id);

        let mut next = accounts;
        for a in next.iter_mut() {
            a.is_primary = a.user_id == user_id;
        }
        Self::save(&next)?;
        // A phone's go-live pushes move to the account now in front.
        #[cfg(mobile)]
        crate::services::push_alerts::server::schedule();
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
            {
                let slot = TwitchService::primary_slot().await;
                TwitchService::persist_primary_token(&slot, &next_token).await?;
                TwitchService::bump_account_generation();
            }
            let _ = Self::delete_secondary_token(&next.user_id);

            let mut rebuilt: Vec<StoredAccount> = Self::list()
                .into_iter()
                .filter(|a| Some(&a.user_id) != leaving_id.as_ref())
                .collect();
            for a in rebuilt.iter_mut() {
                a.is_primary = a.user_id == next.user_id;
            }
            Self::save(&rebuilt)?;
            // `logout` above took this device off the server; it registers again
            // under the promoted account.
            #[cfg(mobile)]
            crate::services::push_alerts::server::schedule();
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

        // Clear the primary token slot.
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
        crate::services::youtube_auth_service::disconnect().await;

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

    if let Ok(root) = crate::services::twitch_service::twitch_web_profiles_root() {
        // Belt-and-suspenders over reset_all's per-account deletes: drop the whole
        // profiles tree so an orphaned profile can't rehydrate a session.
        targets.push(root);
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

#[cfg(test)]
mod tests {
    use super::{
        login_changed, may_refresh_file, read_registry, secondary_load, with_primary, without_primary, AccountStore, AltRefresh,
        SlotRead, StoredAccount, REGISTRY_IO,
    };
    use crate::models::user::UserInfo;
    use crate::services::twitch_service::{StorableToken, TokenError, TwitchService};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    fn stored(access: &str, refresh: &str, expires_in: i64) -> StorableToken {
        StorableToken {
            access_token: access.into(),
            refresh_token: refresh.into(),
            expires_at: if expires_in == 0 { 0 } else { chrono::Utc::now().timestamp() + expires_in },
        }
    }

    async fn no_slot() -> SlotRead {
        Ok(None)
    }

    async fn within<T>(f: impl std::future::Future<Output = T>) -> T {
        tokio::time::timeout(Duration::from_secs(5), f).await.expect("a missing lock must fail, not hang")
    }

    // Every test builds its own `AltRefresh`: no process-wide state is shared.

    #[tokio::test]
    async fn two_callers_spend_one_refresh_and_both_get_the_new_token() {
        let alt = Arc::new(AltRefresh::new());
        let loads = Arc::new(AtomicUsize::new(0));
        let refreshes = Arc::new(AtomicUsize::new(0));
        let gate = Arc::new(tokio::sync::Notify::new());
        let call = |alt: Arc<AltRefresh>, loads: Arc<AtomicUsize>, refreshes: Arc<AtomicUsize>, gate: Arc<tokio::sync::Notify>, first: bool| async move {
            alt.single_flight_refresh(
                "sf-concurrency",
                move || {
                    loads.fetch_add(1, Ordering::SeqCst);
                    // After the first caller refreshed, the file holds the new token.
                    Ok(if first { stored("old", "r1", 10) } else { stored("new", "r2", 3600) })
                },
                no_slot,
                move |_| async move {
                    refreshes.fetch_add(1, Ordering::SeqCst);
                    gate.notified().await;
                    Ok(stored("new", "r2", 3600))
                },
                |_| Ok(()),
            )
            .await
        };
        let a = tokio::spawn(call(alt.clone(), loads.clone(), refreshes.clone(), gate.clone(), true));
        while refreshes.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let mut b = Box::pin(call(alt.clone(), loads.clone(), refreshes.clone(), gate.clone(), false));
        assert!(futures::poll!(b.as_mut()).is_pending(), "B waits on the guard");
        assert_eq!(loads.load(Ordering::SeqCst), 1, "B has not loaded yet");
        gate.notify_one();
        assert_eq!(within(a).await.unwrap().unwrap(), "new");
        assert_eq!(within(b).await.unwrap(), "new");
        assert_eq!(refreshes.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_dropped_caller_never_cancels_the_refresh_before_the_store() {
        let alt = Arc::new(AltRefresh::new());
        let (started_tx, started_rx) = tokio::sync::oneshot::channel::<()>();
        let (go_tx, go_rx) = tokio::sync::oneshot::channel::<()>();
        let (stored_tx, stored_rx) = tokio::sync::oneshot::channel::<String>();
        let caller = tokio::spawn({
            let alt = alt.clone();
            async move {
                alt.single_flight_refresh(
                    "sf-detach",
                    || Ok(stored("old", "r1", 10)),
                    no_slot,
                    move |_| async move {
                        let _ = started_tx.send(());
                        let _ = go_rx.await;
                        Ok(stored("fresh", "r2", 3600))
                    },
                    move |t| {
                        let _ = stored_tx.send(t.access_token.clone());
                        Ok(())
                    },
                )
                .await
            }
        });
        within(started_rx).await.unwrap();
        caller.abort();
        let _ = go_tx.send(());
        assert_eq!(within(stored_rx).await.unwrap(), "fresh");
    }

    #[tokio::test]
    async fn the_helper_keeps_todays_rules() {
        let alt = AltRefresh::new();
        let refreshes = Arc::new(AtomicUsize::new(0));
        let count = |r: Arc<AtomicUsize>| move |_: String| async move {
            r.fetch_add(1, Ordering::SeqCst);
            Ok(stored("x", "y", 3600))
        };
        // Outside the five-minute window: no refresh, and the slot is never read.
        let slot_reads = Arc::new(AtomicUsize::new(0));
        let reads = slot_reads.clone();
        let counting_slot = move || async move {
            reads.fetch_add(1, Ordering::SeqCst);
            let none: SlotRead = Ok(None);
            none
        };
        assert_eq!(within(alt.single_flight_refresh("sf-fresh", || Ok(stored("ok", "r", 3600)), counting_slot, count(refreshes.clone()), |_| Ok(()))).await.unwrap(), "ok");
        assert_eq!(slot_reads.load(Ordering::SeqCst), 0);
        // expires_at == 0 never renews.
        assert_eq!(within(alt.single_flight_refresh("sf-zero", || Ok(stored("zero", "r", 0)), no_slot, count(refreshes.clone()), |_| Ok(()))).await.unwrap(), "zero");
        // An empty refresh token is a refusal, with today's words.
        let err = within(alt.single_flight_refresh("sf-empty", || Ok(stored("old", "", 10)), no_slot, count(refreshes.clone()), |_| Ok(()))).await.unwrap_err();
        assert!(err.to_string().contains("no refresh token"));
        assert_eq!(TwitchService::fetch_failure(&err), TokenError::Refused);
        assert_eq!(refreshes.load(Ordering::SeqCst), 0);
        // Two ids refresh independently, and the map keeps its entry.
        assert!(!Arc::ptr_eq(&alt.guard("sf-a"), &alt.guard("sf-b")));
        assert!(Arc::ptr_eq(&alt.guard("sf-a"), &alt.guard("sf-a")));
    }

    #[test]
    fn a_linked_accounts_file_read_is_typed() {
        let words = |e: &anyhow::Error| e.to_string();
        assert_eq!(secondary_load(Ok(Some(stored("a", "r", 3600))), "7").unwrap().access_token, "a");
        let missing = secondary_load(Ok(None), "7").unwrap_err();
        assert_eq!(TwitchService::fetch_failure(&missing), TokenError::NoToken);
        assert!(words(&missing).contains("No stored token for secondary account 7"), "today's words: {missing}");
        // A 0-byte file and `{}` do not parse: no usable token, never a store outage.
        for text in ["", "{}"] {
            let parse = serde_json::from_str::<StorableToken>(text).unwrap_err();
            let load = Err(anyhow::Error::new(parse).context("does not hold the expected JSON"));
            let e = secondary_load(load, "7").unwrap_err();
            assert_eq!(TwitchService::fetch_failure(&e), TokenError::NoToken, "{text:?}");
            assert!(words(&e).contains("No stored token for secondary account 7"));
        }
        let vault = secondary_load(Err(anyhow::anyhow!("keyring locked")), "7").unwrap_err();
        assert_eq!(TwitchService::fetch_failure(&vault), TokenError::VaultUnavailable);
    }

    #[test]
    fn may_refresh_file_table() {
        let file = stored("a", "r1", 10);
        assert!(may_refresh_file(&file, &Ok(None)), "empty slot");
        assert!(may_refresh_file(&file, &Ok(Some(stored("s", "r2", 3600)))), "a different credential");
        assert!(!may_refresh_file(&file, &Ok(Some(stored("s", "r1", 3600)))), "the slot owns it");
        assert!(!may_refresh_file(&file, &Err(TokenError::VaultUnavailable)), "slot unreadable");
    }

    #[tokio::test]
    async fn an_expiring_token_the_slot_holds_or_cannot_show_is_never_refreshed() {
        let alt = AltRefresh::new();
        for (name, slot) in [
            ("sf-slot-same", Ok(Some(stored("s", "r1", 3600)))),
            ("sf-slot-vault", Err(TokenError::VaultUnavailable)),
        ] {
            let spent = Arc::new(AtomicUsize::new(0));
            let counter = spent.clone();
            let err = within(alt.single_flight_refresh(
                name,
                || Ok(stored("old", "r1", 10)),
                move || async move { slot },
                move |_| async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    Ok(stored("x", "y", 3600))
                },
                |_| Ok(()),
            ))
            .await
            .unwrap_err();
            assert_eq!(TwitchService::fetch_failure(&err), TokenError::Unreachable, "{name}");
            assert!(err.to_string().contains("network error"), "{name}: {err}");
            assert_eq!(spent.load(Ordering::SeqCst), 0, "{name}");
        }
        // A different slot credential refreshes; the slot is read after the load, inside the guard.
        let order = Arc::new(std::sync::Mutex::new(Vec::<&'static str>::new()));
        let (o1, o2) = (order.clone(), order.clone());
        let ok = within(alt.single_flight_refresh(
            "sf-slot-other",
            move || {
                o1.lock().unwrap().push("file");
                Ok(stored("old", "r1", 10))
            },
            move || async move {
                o2.lock().unwrap().push("slot");
                Ok(Some(stored("s", "r2", 3600)))
            },
            |_| async { Ok(stored("renewed", "r3", 3600)) },
            |_| Ok(()),
        ))
        .await;
        assert_eq!(ok.unwrap(), "renewed");
        assert_eq!(*order.lock().unwrap(), vec!["file", "slot"]);
    }

    #[tokio::test]
    async fn during_an_account_change_an_expiring_token_makes_no_refresh_and_a_fresh_one_is_returned() {
        let alt = AltRefresh::new();
        let change = alt.begin_account_change().await;
        assert_eq!(alt.change.load(Ordering::SeqCst), 1);
        let spent = Arc::new(AtomicUsize::new(0));
        let counter = spent.clone();
        let err = within(alt.single_flight_refresh(
            "ac-expiring",
            || Ok(stored("old", "r1", 10)),
            no_slot,
            move |_| async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok(stored("x", "y", 3600))
            },
            |_| Ok(()),
        ))
        .await
        .unwrap_err();
        assert_eq!(TwitchService::fetch_failure(&err), TokenError::Unreachable);
        assert!(err.to_string().contains("network error"), "today's network words: {err}");
        assert_eq!(spent.load(Ordering::SeqCst), 0);
        let fresh = within(alt.single_flight_refresh("ac-fresh", || Ok(stored("fine", "r1", 3600)), no_slot, |_| async { panic!("no refresh") }, |_| Ok(()))).await;
        assert_eq!(fresh.unwrap(), "fine");
        drop(change);
        assert_eq!(alt.change.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn begin_account_change_waits_for_a_refresh_in_flight_and_guards_overlap() {
        let alt = Arc::new(AltRefresh::new());
        let (started_tx, started_rx) = tokio::sync::oneshot::channel::<()>();
        let (go_tx, go_rx) = tokio::sync::oneshot::channel::<()>();
        let (stored_tx, stored_rx) = tokio::sync::oneshot::channel::<()>();
        let refresh = tokio::spawn({
            let alt = alt.clone();
            async move {
                alt.single_flight_refresh(
                    "ac-drain",
                    || Ok(stored("old", "r1", 10)),
                    no_slot,
                    move |_| async move {
                        let _ = started_tx.send(());
                        let _ = go_rx.await;
                        Ok(stored("fresh", "r2", 3600))
                    },
                    move |_| {
                        let _ = stored_tx.send(());
                        Ok(())
                    },
                )
                .await
            }
        });
        within(started_rx).await.unwrap();
        let mut begin = Box::pin(alt.begin_account_change());
        assert!(futures::poll!(begin.as_mut()).is_pending(), "it waits while a refresh holds its account's lock");
        assert_eq!(alt.change.load(Ordering::SeqCst), 1, "the counter is raised before the wait");
        let _ = go_tx.send(());
        within(stored_rx).await.unwrap();
        let first = within(begin).await;
        let second = within(alt.begin_account_change()).await;
        assert_eq!(alt.change.load(Ordering::SeqCst), 2);
        drop(first);
        assert_eq!(alt.change.load(Ordering::SeqCst), 1);
        drop(second);
        assert_eq!(alt.change.load(Ordering::SeqCst), 0);
        let _ = within(refresh).await;
        // A caller dropped mid-wait still lowers the counter.
        let dropped = Box::pin(alt.begin_account_change());
        drop(dropped);
        assert_eq!(alt.change.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_reader_never_sees_an_empty_list_during_200_locked_saves() {
        let dir = std::env::temp_dir().join(format!("sn-registry-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("accounts.json");
        let two = |n: u32| vec![account(&format!("a{n}"), true), account(&format!("b{n}"), false)];
        AccountStore::save_at(&path, &two(0)).unwrap();
        let done = Arc::new(AtomicBool::new(false));
        let reader = {
            let (path, done) = (path.clone(), done.clone());
            std::thread::spawn(move || {
                let (mut empty, mut reads) = (0u32, 0u32);
                while !done.load(Ordering::SeqCst) {
                    // The discipline `list()` has: the shared lock around the read.
                    let _registry = REGISTRY_IO.read().unwrap_or_else(|e| e.into_inner());
                    reads += 1;
                    if read_registry(&path).is_empty() {
                        empty += 1;
                    }
                }
                (empty, reads)
            })
        };
        for n in 1..=200 {
            AccountStore::save_at(&path, &two(n)).unwrap();
        }
        done.store(true, Ordering::SeqCst);
        let (empty, reads) = reader.join().unwrap();
        assert!(reads > 0);
        assert_eq!(empty, 0, "{empty} of {reads} reads came back empty");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The text of the `fn` whose signature starts with `sig`, to its closing brace line.
    fn body_of(src: &str, sig: &str, end: &str) -> String {
        let src = src.replace("\r\n", "\n");
        let start = src.find(sig).unwrap_or_else(|| panic!("{sig}"));
        src[start..start + src[start..].find(end).expect("end")].to_string()
    }

    #[test]
    fn whole_statement_pins_for_the_registry_lock() {
        let src = include_str!("account_store.rs");
        let list = body_of(src, "pub fn list() -> Vec<StoredAccount>", "\n    }\n");
        let lock = list.find("let _registry = REGISTRY_IO.read()").expect("list takes the shared lock");
        assert!(lock < list.find("read_registry(").expect("list reads"), "the lock is held before the read");
        let save_at = body_of(src, "fn save_at(", "\n    }\n");
        let open = save_at.find("{\n            let _registry = REGISTRY_IO.write()").expect("a block opening with the exclusive lock");
        let write = save_at.find("write_file_atomic(").expect("the write");
        let close = save_at.find("\n        }\n").expect("the block closes");
        assert!(open < write && write < close, "the write sits inside the block");
        assert!(!save_at.contains("fs::write("));
        let save = body_of(src, "fn save(accounts: &[StoredAccount])", "\n    }\n");
        assert!(save.find("Self::save_at(&path, accounts)?").unwrap() < save.find("invalidate_owner(").unwrap(), "the lock is gone before any invalidation");
        assert!(!save.contains("fs::write(") && !save.contains("REGISTRY_IO"));
        assert!(save.find("set_own_accounts").unwrap() < save.find("on_accounts_changed()").unwrap());
        let read = body_of(src, "fn read_registry(", "\n}\n");
        assert!(read.contains("path.exists()") && !read.contains("REGISTRY_IO"));
    }

    #[test]
    fn source_pins_for_the_alt_refresh() {
        let full = include_str!("account_store.rs").replace("\r\n", "\n");
        let src = full.clone();
        // ALT_REFRESH is named only by the static, get_token_for and begin_account_change
        // (counted before the test module, whose own lines name it too).
        let before_tests = &full[..full.find("#[cfg(test)]\nmod tests {").expect("the test module")];
        let uses: Vec<&str> = before_tests
            .lines()
            .filter(|l| !l.trim_start().starts_with("//") && l.contains("ALT_REFRESH"))
            .collect();
        assert_eq!(uses.len(), 3, "{uses:?}");
        assert!(body_of(&src, "pub async fn get_token_for(", "\n    }\n").contains("ALT_REFRESH.refresh_alt("));
        assert_eq!(
            body_of(&src, "pub(crate) async fn begin_account_change()", "\n    }\n").matches("ALT_REFRESH.begin_account_change().await").count(),
            1
        );
        // The helper: guard, load, change check, slot check, then the spawn.
        let helper = body_of(&src, "pub(crate) async fn single_flight_refresh<", "\n    }\n");
        assert!(helper.find("lock_owned()").unwrap() < helper.find("load()?").unwrap());
        assert!(helper.find("load()?").unwrap() < helper.find("self.change.load(").unwrap());
        assert!(helper.find("self.change.load(").unwrap() < helper.find("may_refresh_file(&token, &slot_read().await)").unwrap());
        assert!(helper.find("may_refresh_file(").unwrap() < helper.find("async_runtime::spawn").unwrap());
        // The refresh runs on Tauri's runtime (the Android worker drops its own), never tokio::spawn or the engine's spawner.
        assert!(helper.contains("tauri::async_runtime::spawn(") && !helper.contains("tokio::spawn") && !helper.contains("spawner"));
        assert!(helper.contains("TokenError::Refused") && helper.contains("refresh_unavailable("));
        assert!(!helper.contains("TwitchService::get_token"));
        // The state is just the map and the counter; the refresh is today's `refresh_token` on the shared client.
        // (The names are built in two parts so this test does not find itself.)
        assert!(!src.contains(&format!("{}{}", "alt_refresh_", "client")) && !src.contains(&format!("{}{}", "refresh_token_", "with")));
        assert!(body_of(&src, "pub(crate) async fn refresh_alt<", "\n    }\n").contains("TwitchService::refresh_token(&rt)"));
        // The launch-time forced re-auth binds no guard (nothing can be refreshing yet).
        let reauth = body_of(&src, "pub async fn run_force_reauth_if_needed()", "\n    }\n");
        assert!(!reauth.contains("begin_account_change"));
    }

    #[test]
    fn source_pins_for_the_account_store() {
        let src = include_str!("account_store.rs");
        // No account door changed: signatures as today.
        for sig in [
            "pub async fn set_active(user_id: &str) -> Result<StoredAccount>",
            "pub async fn sign_out_active() -> Result<Option<StoredAccount>>",
            "pub fn remove_secondary(user_id: &str) -> Result<()>",
            "pub async fn reset_all()",
            "pub async fn add_secondary(token: StorableToken) -> Result<StoredAccount>",
            "fn record_primary(accounts: Vec<StoredAccount>, info: UserInfo)",
            "async fn keep_primary(accounts: Vec<StoredAccount>, user_id: &str, validated_login: Option<&str>)",
            "pub fn forget_primary()",
        ] {
            assert!(src.contains(sig), "{sig}");
        }
    }

    #[test]
    fn a_new_login_for_the_same_id_is_a_rename() {
        assert!(login_changed("shaiza__", "1shaiza"));
        assert!(!login_changed("BR_Winters", "br_winters"));
        assert!(!login_changed("shaiza__", ""));
    }

    fn account(id: &str, primary: bool) -> StoredAccount {
        StoredAccount {
            user_id: id.into(),
            login: format!("login{id}"),
            display_name: format!("Name{id}"),
            avatar_url: None,
            is_primary: primary,
            added_at: 5,
        }
    }

    fn info(id: &str, login: &str) -> UserInfo {
        UserInfo {
            id: id.into(),
            login: login.into(),
            display_name: login.into(),
            email: None,
            profile_image_url: Some("pic".into()),
            broadcaster_type: None,
        }
    }

    fn primaries(accounts: &[StoredAccount]) -> Vec<&str> {
        accounts.iter().filter(|a| a.is_primary).map(|a| a.user_id.as_str()).collect()
    }

    #[test]
    fn signing_in_as_someone_new_makes_them_main_and_keeps_the_old_main_linked() {
        let next = with_primary(vec![account("1", true), account("2", false)], info("3", "three"));
        assert_eq!(primaries(&next), ["3"]);
        assert_eq!(next.len(), 3);
        assert_eq!(next[0].user_id, "3", "a new main is listed first");
        assert!(next.iter().any(|a| a.user_id == "1" && !a.is_primary));
    }

    #[test]
    fn signing_in_as_a_linked_account_promotes_it_in_place() {
        let next = with_primary(vec![account("1", true), account("2", false)], info("2", "renamed"));
        assert_eq!(primaries(&next), ["2"]);
        assert_eq!(next.len(), 2);
        let two = next.iter().find(|a| a.user_id == "2").unwrap();
        assert_eq!((two.login.as_str(), two.added_at), ("renamed", 5));
    }

    #[test]
    fn signing_out_leaves_no_main_and_keeps_the_linked_accounts() {
        let (rest, leaving) = without_primary(vec![account("1", false), account("2", true)]);
        assert_eq!(leaving.as_deref(), Some("2"));
        assert!(primaries(&rest).is_empty());
        assert_eq!(rest.len(), 1);
        let (same, none) = without_primary(rest.clone());
        assert_eq!((same.len(), none), (1, None));
    }
}
