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

/// What a row says about its account.
#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum AccountStatus {
    Connected,
    /// Was signed in until the platform revoked or expired the session.
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

fn static_id(provider: &str) -> Option<&'static str> {
    match provider {
        "twitch" => Some("twitch"),
        "kick" => Some("kick"),
        "youtube" => Some("youtube"),
        "tiktok" => Some("tiktok"),
        _ => None,
    }
}

/// Record (or clear) who the primary Twitch account is.
pub fn set_twitch_identity(info: Option<UserInfo>) {
    let changed = match TWITCH_IDENTITY.lock() {
        Ok(mut cell) => {
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
    }
}

fn twitch_row() -> RosterAccount {
    let identity = TWITCH_IDENTITY.lock().ok().and_then(|c| c.clone());
    match identity {
        Some(user) => RosterAccount {
            id: "twitch",
            status: AccountStatus::Connected,
            name: Some(user.display_name),
            handle: Some(user.login),
            avatar_url: user.profile_image_url,
            account_id: Some(user.id),
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
        if last.as_ref() == Some(&roster) {
            return;
        }
        let _ = app.emit("account-roster-changed", &roster);
        *last = Some(roster);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
