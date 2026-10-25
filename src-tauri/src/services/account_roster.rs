//! Every account StreamNook is signed into, as one ready-to-render list.
//!
//! The credentials themselves stay where they always lived (the Twitch, drops,
//! 7TV, Kick, YouTube and TikTok auth services); this is a VIEW over them, so
//! the title bar, Settings and every popout read the same answer instead of each
//! asking six services for themselves.
//!
//! Pushed, never polled: each auth service calls `notify()` when its credential
//! is stored or cleared, the roster is rebuilt from memory, and
//! `account-roster-changed` goes out to every window only when it actually
//! differs from the last one sent.
//!
//! The one piece of state this module owns is which platforms EXPIRED: a session
//! the platform revoked reads differently from one that was never connected, so
//! the title bar can say "reconnect" rather than "connect". It lives in memory
//! only and clears the moment that platform connects again.

use std::collections::HashSet;
use std::sync::Mutex;

use once_cell::sync::Lazy;
use serde::Serialize;

use crate::models::user::UserInfo;
use crate::services::twitch_signin::{self, SignInReason};

/// What a row says about its account.
#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum AccountStatus {
    Connected,
    /// Was signed in until the platform revoked or expired the session, or
    /// until StreamNook needed it granted again (`RosterAccount::reason`).
    Expired,
    NotConnected,
}

#[derive(Serialize, Clone, PartialEq, Eq, Debug)]
pub struct RosterAccount {
    /// `twitch` / `youtube` / `kick` / `tiktok` for platform accounts,
    /// `twitch_drops` / `seventv` for the extras.
    pub id: &'static str,
    pub status: AccountStatus,
    pub name: Option<String>,
    pub handle: Option<String>,
    pub avatar_url: Option<String>,
    /// The id that platform's chat stamps on this account's messages (Twitch
    /// user id, Kick account id, YouTube `UC…` channel id, TikTok user id).
    /// None is normal: a YouTube account can have no channel.
    pub account_id: Option<String>,
    /// Why this account has to sign in again, when StreamNook signed it out
    /// itself. Only Twitch carries one today (`twitch_signin`).
    pub reason: Option<SignInReason>,
    /// This build can sign in to it here. YouTube and TikTok sign in through a desktop webview
    /// only, so the phones offer them nowhere until their own sign-in lands.
    pub can_sign_in: bool,
    /// This build can open that platform's streams and chat. The phones list,
    /// follow and play Twitch and Kick only, so YouTube and TikTok rows, the
    /// settings that only touch them, and their share links stay off there.
    pub can_watch: bool,
}

/// Whether this build can run `id`'s sign-in.
pub fn can_sign_in(id: &str) -> bool {
    match id {
        "youtube" | "tiktok" => cfg!(desktop),
        _ => true,
    }
}

/// Whether this build can watch `id`'s streams and chat.
pub fn can_watch(id: &str) -> bool {
    match id {
        "youtube" | "tiktok" => cfg!(desktop),
        _ => true,
    }
}

#[derive(Serialize, Clone, PartialEq, Eq, Debug)]
pub struct AccountRoster {
    /// The platform accounts you watch and chat as, in display order.
    pub main: Vec<RosterAccount>,
    /// Credentials that serve a feature rather than a platform: the drops and
    /// channel points sign-in, and 7TV.
    pub extras: Vec<RosterAccount>,
}

impl AccountRoster {
    /// The roster entry for a platform account, by id.
    pub fn main_account(&self, id: &str) -> Option<&RosterAccount> {
        self.main.iter().find(|a| a.id == id)
    }
}

/// The primary Twitch identity, cached from the last successful
/// `get_user_info`. Twitch's token proves nothing about WHO it belongs to
/// without a Helix call, and the roster must never make one.
static TWITCH_IDENTITY: Lazy<Mutex<Option<UserInfo>>> = Lazy::new(|| Mutex::new(None));

/// Platforms whose session died on its own since they were last connected.
static EXPIRED: Lazy<Mutex<HashSet<&'static str>>> = Lazy::new(|| Mutex::new(HashSet::new()));

/// The last roster sent, so a credential refresh that changed nothing visible
/// does not wake every window. Async so concurrent rebuilds run one at a time
/// and the last one emitted is always the newest.
static LAST_SENT: Lazy<tokio::sync::Mutex<Option<AccountRoster>>> =
    Lazy::new(|| tokio::sync::Mutex::new(None));

/// Identity retries run since the roster last fully resolved (see `retry_if_unresolved`).
static RETRIES: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

fn static_id(provider: &str) -> Option<&'static str> {
    match provider {
        "twitch" => Some("twitch"),
        "kick" => Some("kick"),
        "youtube" => Some("youtube"),
        "tiktok" => Some("tiktok"),
        _ => None,
    }
}

/// The primary Twitch account's user id, when it is known.
pub fn twitch_user_id() -> Option<String> {
    TWITCH_IDENTITY.lock().ok().and_then(|c| c.as_ref().map(|u| u.id.clone()))
}

/// Record (or clear) who the primary Twitch account is.
pub fn set_twitch_identity(info: Option<UserInfo>) {
    let changed = match TWITCH_IDENTITY.lock() {
        Ok(mut cell) => {
            // Another member (or none): their linked-account claims start over.
            if cell.as_ref().map(|u| u.id.as_str()) != info.as_ref().map(|u| u.id.as_str()) {
                crate::services::linked_accounts::reset();
            }
            let same = match (cell.as_ref(), info.as_ref()) {
                (Some(a), Some(b)) => {
                    a.id == b.id
                        && a.display_name == b.display_name
                        && a.profile_image_url == b.profile_image_url
                }
                (None, None) => true,
                _ => false,
            };
            *cell = info;
            !same
        }
        Err(_) => false,
    };
    if changed {
        notify();
    }
}

/// A platform's session was revoked or expired on the platform's side.
pub fn mark_expired(provider: &str) {
    if let (Some(id), Ok(mut set)) = (static_id(provider), EXPIRED.lock()) {
        set.insert(id);
    }
    notify();
}

fn status_for(id: &'static str, connected: bool) -> AccountStatus {
    if connected {
        if let Ok(mut set) = EXPIRED.lock() {
            set.remove(id);
        }
        return AccountStatus::Connected;
    }
    let expired = EXPIRED.lock().map(|s| s.contains(id)).unwrap_or(false);
    if expired {
        AccountStatus::Expired
    } else {
        AccountStatus::NotConnected
    }
}

fn empty(id: &'static str, status: AccountStatus) -> RosterAccount {
    RosterAccount {
        id,
        status,
        name: None,
        handle: None,
        avatar_url: None,
        account_id: None,
        reason: None,
        can_sign_in: can_sign_in(id),
        can_watch: can_watch(id),
    }
}

async fn platform_row(id: &'static str) -> RosterAccount {
    use crate::services::{kick_auth_service, tiktok_auth_service, youtube_auth_service};
    let connected = match id {
        "kick" => kick_auth_service::is_connected(),
        "youtube" => youtube_auth_service::is_connected(),
        "tiktok" => tiktok_auth_service::is_connected(),
        _ => false,
    };
    let status = status_for(id, connected);
    if !connected {
        return empty(id, status);
    }
    // Served from each service's cache once the identity has resolved; the
    // first read after a connect is the only one that can reach the network.
    // Each id is read AFTER account_identity, which is what fills it from the
    // same upstream response.
    let ((name, avatar_url), handle, account_id) = match id {
        "kick" => {
            let identity = kick_auth_service::account_identity().await;
            (identity, None, kick_auth_service::account_id())
        }
        "youtube" => {
            let identity = youtube_auth_service::account_identity().await;
            (identity, None, youtube_auth_service::account_channel_id())
        }
        "tiktok" => {
            let identity = tiktok_auth_service::account_identity().await;
            (
                identity,
                tiktok_auth_service::account_handle(),
                tiktok_auth_service::account_id(),
            )
        }
        _ => ((None, None), None, None),
    };
    RosterAccount {
        id,
        status,
        name,
        handle,
        avatar_url,
        account_id,
        reason: None,
        can_sign_in: can_sign_in(id),
        can_watch: can_watch(id),
    }
}

fn twitch_row() -> RosterAccount {
    let identity = TWITCH_IDENTITY.lock().ok().and_then(|c| c.clone());
    twitch_row_from(identity, twitch_signin::current())
}

/// A pending sign-in reason wins over a cached identity: the reason is only
/// set once the token is gone, and a profile read that was already in flight
/// can still land after that.
fn twitch_row_from(identity: Option<UserInfo>, reason: Option<SignInReason>) -> RosterAccount {
    if let Some(reason) = reason {
        return RosterAccount {
            reason: Some(reason),
            ..empty("twitch", AccountStatus::Expired)
        };
    }
    match identity {
        Some(user) => RosterAccount {
            id: "twitch",
            status: AccountStatus::Connected,
            name: Some(user.display_name),
            handle: Some(user.login),
            avatar_url: user.profile_image_url,
            account_id: Some(user.id),
            reason: None,
            can_sign_in: true,
            can_watch: true,
        },
        None => empty("twitch", AccountStatus::NotConnected),
    }
}

/// Build the roster from what the auth services hold right now.
pub async fn build() -> AccountRoster {
    use crate::services::drops_auth_service::DropsAuthService;
    use crate::services::seventv_auth_service::SevenTVAuthService;

    let (youtube, kick, tiktok, drops_signed_in, seventv) = tokio::join!(
        platform_row("youtube"),
        platform_row("kick"),
        platform_row("tiktok"),
        DropsAuthService::is_authenticated(),
        SevenTVAuthService::get_auth_status(),
    );

    let drops = empty(
        "twitch_drops",
        if drops_signed_in {
            AccountStatus::Connected
        } else {
            AccountStatus::NotConnected
        },
    );
    let seventv = empty(
        "seventv",
        if seventv.is_authenticated {
            AccountStatus::Connected
        } else {
            AccountStatus::NotConnected
        },
    );

    AccountRoster {
        main: vec![twitch_row(), youtube, kick, tiktok],
        extras: vec![drops, seventv],
    }
}

/// The Twitch account whose whispers to listen for: the connected one, else
/// nobody (signed out, or signed out by StreamNook until it signs in again).
fn whisper_account(roster: &AccountRoster) -> Option<String> {
    roster
        .main_account("twitch")
        .filter(|a| a.status == AccountStatus::Connected)
        .and_then(|a| a.account_id.clone())
        .filter(|id| !id.is_empty())
}

/// Rebuild the roster and tell every window, if it changed.
///
/// Cheap and safe to call from any credential write: it spawns, reads memory,
/// and emits nothing when the visible answer is the same.
pub fn notify() {
    let Some(app) = crate::services::providers::app_handle() else {
        return;
    };
    tauri::async_runtime::spawn(async move {
        use tauri::Emitter;
        let mut last = LAST_SENT.lock().await;
        let roster = build().await;
        follow_chat_identities(&roster);
        retry_if_unresolved(&roster);
        // Claims are reported from every build (they dedupe on their own), so a
        // retry after a cooldown goes out even when the roster did not change.
        crate::services::linked_accounts::on_roster(&roster);
        // The whisper socket follows the Twitch account on every platform:
        // listening while it is connected, closed on sign-out or once Twitch
        // needs the sign-in again.
        crate::services::whisper_service::follow_account(&app, whisper_account(&roster));
        if last.as_ref() == Some(&roster) {
            return;
        }
        // A new account (or the first one this run) opts in and pulls its own
        // mentions while you're away.
        crate::services::mention_watch::client::on_account_changed();
        let _ = app.emit("account-roster-changed", &roster);
        *last = Some(roster);
    });
}

/// Build the roster and follow its chat identities, one build at a time
/// with `notify`, so an older build can never overwrite a newer one's
/// identities (a sign-out landing while a slow build is in flight).
pub async fn build_followed() -> AccountRoster {
    let _turn = LAST_SENT.lock().await;
    let roster = build().await;
    follow_chat_identities(&roster);
    retry_if_unresolved(&roster);
    roster
}

/// The chat identity each other platform's row stands for, as
/// (platform, chat id, login, display name). A row that is not connected
/// comes back empty, which clears that platform.
fn identities_for(
    roster: &AccountRoster,
    youtube_handle: Option<String>,
) -> Vec<(&'static str, String, String, String)> {
    ["kick", "youtube", "tiktok"]
        .into_iter()
        .map(|id| {
            let Some(account) = roster.main_account(id).filter(|a| a.status == AccountStatus::Connected) else {
                return (id, String::new(), String::new(), String::new());
            };
            let name = account.name.clone().unwrap_or_default();
            let handle = (if id == "youtube" { youtube_handle.clone() } else { account.handle.clone() })
                .filter(|h| !h.trim().is_empty());
            // YouTube chat names people by @handle only; a channel title
            // like "Gaming" would match ordinary words, so it is never a
            // display name there.
            let (login, display) = match (id, handle) {
                ("youtube", Some(h)) => (h, String::new()),
                ("youtube", None) => (name, String::new()),
                (_, Some(h)) => (h, name),
                (_, None) => (name.clone(), name),
            };
            (id, account.account_id.clone().unwrap_or_default(), login, display)
        })
        .collect()
}

/// Tell the chat rule engine who you are on each other platform, so a
/// reply to your Kick message or an `@` of your YouTube handle counts as
/// yours.
pub fn follow_chat_identities(roster: &AccountRoster) {
    let youtube_handle = crate::services::youtube_auth_service::account_handle();
    for (id, user_id, login, display) in identities_for(roster, youtube_handle) {
        crate::services::chat_rules::ChatRules::set_platform_identity(id, &user_id, &login, &display);
    }
}

/// A connected platform whose name, or Kick's chat id, has not come back
/// yet (offline at boot, a failed fetch).
fn unresolved(roster: &AccountRoster) -> bool {
    roster.main.iter().any(|a| {
        a.id != "twitch"
            && a.status == AccountStatus::Connected
            && (a.name.is_none() || (a.id == "kick" && a.account_id.is_none()))
    })
}

/// When to ask again after the `n`th retry: 30 s, 2 min, 10 min, then never.
fn retry_delay(n: u8) -> Option<std::time::Duration> {
    match n {
        0 => Some(30),
        1 => Some(120),
        2 => Some(600),
        _ => None,
    }
    .map(std::time::Duration::from_secs)
}

/// Nothing else rebuilds the roster after a lookup failed, so ask again a
/// few times; a fully resolved roster resets the count.
fn retry_if_unresolved(roster: &AccountRoster) {
    use std::sync::atomic::Ordering;
    if !unresolved(roster) {
        RETRIES.store(0, Ordering::Relaxed);
        return;
    }
    let n = RETRIES.fetch_add(1, Ordering::Relaxed);
    if let Some(delay) = retry_delay(n) {
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(delay).await;
            notify();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn youtube_and_tiktok_sign_in_only_where_the_build_can() {
        assert!(can_sign_in("twitch") && can_sign_in("kick") && can_sign_in("twitch_drops"));
        assert_eq!(can_sign_in("youtube"), cfg!(desktop));
        assert_eq!(can_sign_in("tiktok"), cfg!(desktop));
    }

    #[test]
    fn youtube_and_tiktok_watch_only_where_the_build_can() {
        assert!(can_watch("twitch") && can_watch("kick"));
        assert_eq!(can_watch("youtube"), cfg!(desktop));
        assert_eq!(can_watch("tiktok"), cfg!(desktop));
        let row = empty("youtube", AccountStatus::NotConnected);
        assert_eq!(row.can_watch, cfg!(desktop));
        assert_eq!(serde_json::to_value(&row).unwrap()["can_watch"], cfg!(desktop));
    }

    #[test]
    fn expired_clears_on_reconnect() {
        if let Ok(mut set) = EXPIRED.lock() {
            set.insert("kick");
        }
        assert_eq!(status_for("kick", false), AccountStatus::Expired);
        assert_eq!(status_for("kick", true), AccountStatus::Connected);
        assert_eq!(status_for("kick", false), AccountStatus::NotConnected);
    }

    #[test]
    fn status_serializes_snake_case() {
        let json = serde_json::to_string(&AccountStatus::NotConnected).unwrap();
        assert_eq!(json, "\"not_connected\"");
    }

    fn user() -> UserInfo {
        UserInfo {
            id: "1".into(),
            login: "viewer".into(),
            display_name: "Viewer".into(),
            email: None,
            profile_image_url: Some("https://example.invalid/a.png".into()),
            broadcaster_type: None,
        }
    }

    #[test]
    fn twitch_row_says_why_a_sign_in_is_owed() {
        let owed = twitch_row_from(None, Some(SignInReason::NewPermissions));
        assert_eq!(owed.status, AccountStatus::Expired);
        assert_eq!(owed.reason, Some(SignInReason::NewPermissions));
        assert_eq!(owed.name, None);

        let json = serde_json::to_value(&owed).unwrap();
        assert_eq!(json["status"], "expired");
        assert_eq!(json["reason"], "new_permissions");
    }

    #[test]
    fn a_stale_identity_never_hides_the_reason() {
        let row = twitch_row_from(Some(user()), Some(SignInReason::NewPermissions));
        assert_eq!(row.status, AccountStatus::Expired);
        assert_eq!(row.reason, Some(SignInReason::NewPermissions));
        assert_eq!(row.avatar_url, None);
    }

    #[test]
    fn twitch_row_without_a_reason_is_as_before() {
        let signed_in = twitch_row_from(Some(user()), None);
        assert_eq!(signed_in.status, AccountStatus::Connected);
        assert_eq!(signed_in.reason, None);
        assert_eq!(signed_in.handle.as_deref(), Some("viewer"));

        let signed_out = twitch_row_from(None, None);
        assert_eq!(signed_out.status, AccountStatus::NotConnected);
        assert_eq!(signed_out.reason, None);
        assert_eq!(serde_json::to_value(&signed_out).unwrap()["reason"], serde_json::Value::Null);
    }

    fn connected(id: &'static str, name: Option<&str>, account_id: Option<&str>, handle: Option<&str>) -> RosterAccount {
        RosterAccount {
            name: name.map(String::from),
            account_id: account_id.map(String::from),
            handle: handle.map(String::from),
            ..empty(id, AccountStatus::Connected)
        }
    }

    fn roster_of(main: Vec<RosterAccount>) -> AccountRoster {
        AccountRoster { main, extras: vec![] }
    }

    #[test]
    fn identities_pick_the_name_each_chat_uses() {
        let roster = roster_of(vec![
            twitch_row_from(Some(user()), None),
            connected("youtube", Some("Gaming"), Some("UCme"), None),
            connected("kick", Some("SnKickTester"), Some("5550001"), None),
            connected("tiktok", Some("Nick"), Some("70010001"), Some("sntiktoker")),
        ]);
        let ids = identities_for(&roster, Some("@sntester".into()));
        assert_eq!(ids[0], ("kick", "5550001".into(), "SnKickTester".into(), "SnKickTester".into()));
        // YouTube: the handle, and never the channel title as a display name.
        assert_eq!(ids[1], ("youtube", "UCme".into(), "@sntester".into(), String::new()));
        assert_eq!(ids[2], ("tiktok", "70010001".into(), "sntiktoker".into(), "Nick".into()));
        // No handle yet: the title is the login, still no display name.
        let ids = identities_for(&roster, None);
        assert_eq!(ids[1], ("youtube", "UCme".into(), "Gaming".into(), String::new()));
    }

    #[test]
    fn rows_that_are_not_connected_clear_their_platform() {
        let roster = roster_of(vec![empty("kick", AccountStatus::Expired)]);
        for (_, id, login, display) in identities_for(&roster, None) {
            assert!(id.is_empty() && login.is_empty() && display.is_empty());
        }
    }

    #[test]
    fn following_sets_and_clears_the_chat_identities() {
        let _own = crate::services::chat_rules::ChatRules::own_identity_test_lock();
        let _reset = crate::services::chat_rules::PlatformReset::new();
        follow_chat_identities(&roster_of(vec![connected("kick", Some("SnKickTester"), Some("5550001"), None)]));
        assert_eq!(
            crate::services::chat_rules::ChatRules::platform_owners().first().map(String::as_str),
            Some("kick-5550001")
        );
        follow_chat_identities(&roster_of(vec![empty("kick", AccountStatus::NotConnected)]));
        assert!(crate::services::chat_rules::ChatRules::platform_owners().is_empty());
    }

    #[test]
    fn unresolved_rows_and_the_retry_schedule() {
        assert!(!unresolved(&roster_of(vec![twitch_row_from(None, None)])));
        assert!(unresolved(&roster_of(vec![connected("kick", Some("A"), None, None)])));
        assert!(unresolved(&roster_of(vec![connected("youtube", None, Some("UCme"), None)])));
        assert!(!unresolved(&roster_of(vec![connected("youtube", Some("A"), None, None)])));
        assert!(!unresolved(&roster_of(vec![empty("kick", AccountStatus::Expired)])));
        assert_eq!(retry_delay(0), Some(std::time::Duration::from_secs(30)));
        assert_eq!(retry_delay(2), Some(std::time::Duration::from_secs(600)));
        assert_eq!(retry_delay(3), None);
    }
}
