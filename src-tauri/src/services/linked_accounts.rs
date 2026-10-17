//! Tells StreamNook's account database which Kick and YouTube accounts a member
//! has connected, so their cosmetics follow them into those chats.
//!
//! Every StreamNook cosmetic is filed under a Twitch user id; chat on other
//! platforms names people by that platform's id. A member's claim on their Kick
//! or YouTube account is what lets other viewers' apps (`member_alias`) find the
//! member behind a Kick or YouTube chatter.
//!
//! It reports CHANGES, never a heartbeat: the roster is rebuilt on every
//! credential write and a "still connected" is not news. The claim is keyed on
//! the signed-in Twitch account, so connecting Kick while signed out of Twitch
//! records nothing until a Twitch sign-in, which backfills it. A claim another
//! member took moments ago is waited out (the server's cooldown), and a server
//! that could not be reached is retried a few minutes later, never on every
//! rebuild.
//!
//! Only the authenticated endpoint is used. It is the one way to carry the
//! platform account id, which decides whose cosmetics render on a Kick message,
//! so it must come from a caller the server has authenticated.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::services::account_roster::{AccountRoster, AccountStatus};

const LINK_PATH: &str = "/api/v1/accounts/link";
/// The platforms a claim is recorded for. TikTok's sign-in unlocks age-gated
/// LIVEs and says nothing about who you are in its chat.
const PROVIDERS: [&str; 2] = ["kick", "youtube"];
/// When the server did not say how long its cooldown is.
const CONFLICT_RETRY: Duration = Duration::from_secs(15 * 60);
/// A claim that never reached the server.
const UNREACHABLE_RETRY: Duration = Duration::from_secs(5 * 60);

/// What the server made of a claim.
#[derive(Debug, Clone, PartialEq)]
pub enum LinkResult {
    Ok,
    /// Someone else holds this account and their claim is too fresh to take.
    Conflict { retry_after_minutes: Option<u64> },
    /// The endpoint is not there, or could not be reached.
    Unavailable,
    Failed,
}

/// Classify one answer. Only JSON carrying the endpoint's own `ok` counts as
/// the endpoint's: an undeployed POST answers 405, and an unknown path can be
/// served the website's HTML with a 200, neither of which is a claim accepted.
pub fn classify(status: u16, ok: bool, body: &str) -> LinkResult {
    let json = serde_json::from_str::<serde_json::Value>(body).ok().filter(|v| v.is_object());
    let Some(json) = json else { return LinkResult::Unavailable };
    if status == 404 || status == 405 {
        return LinkResult::Unavailable;
    }
    if ok && json.get("ok").and_then(|v| v.as_bool()) == Some(true) {
        return LinkResult::Ok;
    }
    if status == 409 {
        let retry = json
            .get("retry_after_minutes")
            .and_then(|v| v.as_f64())
            .filter(|m| m.is_finite() && *m > 0.0)
            .map(|m| m.ceil() as u64);
        return LinkResult::Conflict { retry_after_minutes: retry };
    }
    LinkResult::Failed
}

/// What to tell the server about one platform, if anything.
#[derive(Debug, Clone, PartialEq)]
pub enum Report {
    Claim { account_id: Option<String> },
    Release,
}

/// What was last reported for a platform (`on:<id>` or `off`), and the
/// earliest a claim may be asked again.
#[derive(Debug, Clone, Default)]
pub struct Reported {
    pub key: Option<String>,
    pub not_before: Option<Instant>,
}

/// The report this roster row calls for, or none.
pub fn plan(connected: bool, account_id: Option<&str>, reported: &Reported, now: Instant) -> Option<(Report, String)> {
    let account_id = account_id.filter(|id| !id.is_empty());
    let key = if connected { format!("on:{}", account_id.unwrap_or("")) } else { "off".to_string() };
    if reported.key.as_deref() == Some(key.as_str()) {
        return None;
    }
    // A connected row that lost its id must never overwrite a claim that had
    // one: the id resolves lazily, and an account with no channel never has
    // one. Only an explicit disconnect releases the claim.
    if connected && account_id.is_none() && reported.key.as_deref().is_some_and(|k| k.starts_with("on:")) {
        return None;
    }
    if reported.not_before.is_some_and(|t| now < t) {
        return None;
    }
    let report = if connected {
        Report::Claim { account_id: account_id.map(str::to_string) }
    } else {
        Report::Release
    };
    Some((report, key))
}

static STATE: Mutex<Option<HashMap<&'static str, Reported>>> = Mutex::new(None);

fn with_state<R>(f: impl FnOnce(&mut HashMap<&'static str, Reported>) -> R) -> R {
    let mut guard = STATE.lock().unwrap_or_else(|p| p.into_inner());
    f(guard.get_or_insert_with(HashMap::new))
}

/// Forget what was reported, for a Twitch account switch: the next roster
/// reports everything again under the new member.
pub fn reset() {
    with_state(|s| s.clear());
}

/// Report what this roster says changed. Called with every roster the core
/// builds; cheap when nothing changed.
pub fn on_roster(roster: &AccountRoster) {
    if crate::services::account_roster::twitch_user_id().is_none() {
        // No member to attach a claim to; a Twitch sign-in rebuilds the roster.
        return;
    }
    let now = Instant::now();
    for provider in PROVIDERS {
        let Some(row) = roster.main_account(provider) else { continue };
        let connected = row.status == AccountStatus::Connected;
        let planned = with_state(|s| {
            let reported = s.entry(provider).or_default();
            let planned = plan(connected, row.account_id.as_deref(), reported, now);
            if let Some((_, key)) = &planned {
                // Taken now, so a second roster arriving mid-request sends nothing.
                reported.key = Some(key.clone());
            }
            planned
        });
        let Some((report, _)) = planned else { continue };
        let (name, avatar) = (row.name.clone(), row.avatar_url.clone());
        tauri::async_runtime::spawn(async move { send(provider, report, name, avatar).await });
    }
}

async fn send(provider: &'static str, report: Report, name: Option<String>, avatar: Option<String>) {
    let body = match &report {
        Report::Claim { account_id } => serde_json::json!({
            "provider": provider,
            "platform_user_id": account_id,
            "account_name": name,
            "avatar_url": avatar,
            "connected": true,
        }),
        Report::Release => serde_json::json!({ "provider": provider, "connected": false }),
    };
    let result = match crate::commands::streamnook_api::post_json(LINK_PATH, &body, None).await {
        Ok(res) => classify(res.status, res.ok, &res.body),
        Err(e) => {
            log::debug!("[LinkedAccounts] {provider}: the link endpoint was not reached: {e}");
            LinkResult::Unavailable
        }
    };
    let retry = match result {
        LinkResult::Ok => {
            log::info!(
                "[LinkedAccounts] {provider} {}",
                if matches!(report, Report::Release) { "released" } else { "claimed" }
            );
            // Our own claim moved: every cached answer about it is stale,
            // including the "nobody" cached for our own account before.
            crate::services::member_alias::invalidate();
            if let Some(app) = crate::services::providers::app_handle() {
                use tauri::Emitter;
                let _ = app.emit("member-aliases-invalidated", provider);
            }
            None
        }
        LinkResult::Conflict { retry_after_minutes } => {
            let wait = retry_after_minutes.map(|m| Duration::from_secs(m * 60)).unwrap_or(CONFLICT_RETRY);
            log::info!("[LinkedAccounts] {provider} claim is held by someone else; retrying in {} min", wait.as_secs() / 60);
            Some(wait)
        }
        LinkResult::Unavailable | LinkResult::Failed => Some(UNREACHABLE_RETRY),
    };
    if let Some(wait) = retry {
        // Nothing was recorded: forget the report so a roster after the wait
        // sends it again, and rebuild the roster then.
        with_state(|s| {
            let reported = s.entry(provider).or_default();
            reported.key = None;
            reported.not_before = Some(Instant::now() + wait);
        });
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(wait + Duration::from_secs(1)).await;
            crate::services::account_roster::notify();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reported(key: Option<&str>) -> Reported {
        Reported { key: key.map(str::to_string), not_before: None }
    }

    #[test]
    fn a_new_connection_is_claimed_with_its_account_id() {
        let now = Instant::now();
        let planned = plan(true, Some("676"), &reported(None), now);
        assert_eq!(planned, Some((Report::Claim { account_id: Some("676".into()) }, "on:676".into())));
    }

    #[test]
    fn nothing_changed_is_not_news() {
        let now = Instant::now();
        assert_eq!(plan(true, Some("676"), &reported(Some("on:676")), now), None);
        assert_eq!(plan(false, None, &reported(Some("off")), now), None);
    }

    #[test]
    fn a_new_account_id_is_news_and_a_lost_one_is_not() {
        let now = Instant::now();
        assert!(plan(true, Some("999"), &reported(Some("on:676")), now).is_some(), "a switched account");
        assert_eq!(plan(true, None, &reported(Some("on:676")), now), None, "an id that has not resolved yet");
        assert_eq!(plan(true, Some(""), &reported(Some("on:676")), now), None);
    }

    #[test]
    fn a_disconnect_releases_the_claim() {
        let now = Instant::now();
        assert_eq!(plan(false, Some("676"), &reported(Some("on:676")), now), Some((Report::Release, "off".into())));
    }

    #[test]
    fn a_cooldown_is_waited_out() {
        let now = Instant::now();
        let waiting = Reported { key: None, not_before: Some(now + Duration::from_secs(60)) };
        assert_eq!(plan(true, Some("676"), &waiting, now), None);
        assert!(plan(true, Some("676"), &waiting, now + Duration::from_secs(61)).is_some());
    }

    #[test]
    fn only_the_endpoints_own_json_is_an_answer() {
        assert_eq!(classify(200, true, r#"{"ok":true}"#), LinkResult::Ok);
        assert_eq!(classify(200, true, "<!doctype html><html>"), LinkResult::Unavailable, "the SPA fallback");
        assert_eq!(classify(405, false, r#"{"error":"method"}"#), LinkResult::Unavailable);
        assert_eq!(classify(404, false, r#"{"error":"nope"}"#), LinkResult::Unavailable);
        assert_eq!(classify(200, true, r#"{"ok":false}"#), LinkResult::Failed);
        assert_eq!(classify(500, false, r#"{"error":"boom"}"#), LinkResult::Failed);
        assert_eq!(classify(200, true, "[1]"), LinkResult::Unavailable, "an array is not the endpoint's");
    }

    #[test]
    fn a_conflict_carries_the_servers_wait() {
        assert_eq!(
            classify(409, false, r#"{"ok":false,"retry_after_minutes":12}"#),
            LinkResult::Conflict { retry_after_minutes: Some(12) }
        );
        assert_eq!(classify(409, false, r#"{"ok":false}"#), LinkResult::Conflict { retry_after_minutes: None });
        assert_eq!(
            classify(409, false, r#"{"ok":false,"retry_after_minutes":-3}"#),
            LinkResult::Conflict { retry_after_minutes: None }
        );
    }
}
