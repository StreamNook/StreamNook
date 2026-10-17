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
//! What the server confirmed is kept per Twitch member in `linked_accounts.json`,
//! so a relaunch posts nothing new, and a disconnect is sent only for a claim
//! this device made (a platform never connected here is not "released" on
//! every launch). A confirmed claim is asserted again after a week, so a
//! claim the server lost through some other path comes back.
//!
//! Only the authenticated endpoint is used. It is the one way to carry the
//! platform account id, which decides whose cosmetics render on a Kick message,
//! so it must come from a caller the server has authenticated.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::services::account_roster::{AccountRoster, AccountStatus};

const LINK_PATH: &str = "/api/v1/accounts/link";
/// The platforms a claim is recorded for. TikTok's sign-in unlocks age-gated
/// LIVEs and says nothing about who you are in its chat.
const PROVIDERS: [&str; 2] = ["kick", "youtube"];
/// When the server did not say how long its cooldown is.
const CONFLICT_RETRY: Duration = Duration::from_secs(15 * 60);
/// A claim that never reached the server.
const UNREACHABLE_RETRY: Duration = Duration::from_secs(5 * 60);
/// A confirmed claim is asserted again after this long.
const REASSERT_SECS: u64 = 7 * 24 * 60 * 60;
const RECORD_FILE: &str = "linked_accounts.json";

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

/// What this device last told the server about a platform, and when.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Confirmed {
    /// `on:<platform id>` or `off`.
    pub key: String,
    /// Unix seconds of the server's answer.
    pub at: u64,
}

/// One platform's report state: what the server confirmed (kept on disk),
/// what is in flight, and the earliest a claim may be asked again.
#[derive(Debug, Clone, Default)]
pub struct Reported {
    pub confirmed: Option<Confirmed>,
    pub pending: Option<String>,
    pub not_before: Option<Instant>,
}

/// What a roster row calls for.
#[derive(Debug, Clone, PartialEq)]
pub enum Plan {
    Nothing,
    Send(Report, String),
}

/// The report this roster row calls for, if any. `now_secs` is wall-clock
/// time for the weekly re-assert.
pub fn plan(connected: bool, account_id: Option<&str>, reported: &Reported, now: Instant, now_secs: u64) -> Plan {
    let account_id = account_id.filter(|id| !id.is_empty());
    let key = if connected { format!("on:{}", account_id.unwrap_or("")) } else { "off".to_string() };
    if reported.pending.as_deref() == Some(key.as_str()) {
        return Plan::Nothing;
    }
    let confirmed = reported.confirmed.as_ref();
    let claimed = confirmed.is_some_and(|c| c.key.starts_with("on:"));
    if !connected {
        // Only a claim this device made is released; a platform never
        // connected here has nothing of ours to let go of.
        return if claimed { Plan::Send(Report::Release, key) } else { Plan::Nothing };
    }
    // A connected row that lost its id must never overwrite a claim that had
    // one: the id resolves lazily, and an account with no channel never has
    // one. Only an explicit disconnect releases the claim.
    if account_id.is_none() && claimed {
        return Plan::Nothing;
    }
    let stale = confirmed.is_some_and(|c| now_secs.saturating_sub(c.at) >= REASSERT_SECS);
    if confirmed.is_some_and(|c| c.key == key) && !stale {
        return Plan::Nothing;
    }
    if reported.not_before.is_some_and(|t| now < t) {
        return Plan::Nothing;
    }
    Plan::Send(Report::Claim { account_id: account_id.map(str::to_string) }, key)
}

struct State {
    /// The member whose record is loaded.
    member: Option<String>,
    platforms: HashMap<&'static str, Reported>,
}

// HashMap::new is not const, so the state is made on first use.
static STATE: once_cell::sync::Lazy<Mutex<State>> =
    once_cell::sync::Lazy::new(|| Mutex::new(State { member: None, platforms: HashMap::new() }));

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut guard = STATE.lock().unwrap_or_else(|p| p.into_inner());
    f(&mut guard)
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn record_path() -> Option<std::path::PathBuf> {
    crate::services::twitch_service::get_app_data_dir().ok().map(|dir| dir.join(RECORD_FILE))
}

/// Every member's confirmed reports, by Twitch user id, then platform.
fn read_record() -> HashMap<String, HashMap<String, Confirmed>> {
    record_path()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|raw| serde_json::from_slice(&raw).ok())
        .unwrap_or_default()
}

fn write_confirmed(member: &str, provider: &str, confirmed: &Confirmed) {
    let Some(path) = record_path() else { return };
    let mut record = read_record();
    record.entry(member.to_string()).or_default().insert(provider.to_string(), confirmed.clone());
    if let Ok(raw) = serde_json::to_vec_pretty(&record) {
        let _ = std::fs::write(path, raw);
    }
}

/// Load `member`'s confirmed reports when another member's (or none) are in memory.
fn load_member(state: &mut State, member: &str) {
    if state.member.as_deref() == Some(member) {
        return;
    }
    let mine = read_record().remove(member).unwrap_or_default();
    state.platforms.clear();
    for provider in PROVIDERS {
        let confirmed = mine.get(provider).cloned();
        state.platforms.insert(provider, Reported { confirmed, ..Default::default() });
    }
    state.member = Some(member.to_string());
}

/// Forget what is in memory, for a Twitch account switch: the next roster
/// loads the new member's record.
pub fn reset() {
    with_state(|s| {
        s.member = None;
        s.platforms.clear();
    });
}

/// Report what this roster says changed. Called with every roster the core
/// builds or reads; cheap when nothing changed.
pub fn on_roster(roster: &AccountRoster) {
    let Some(member) = crate::services::account_roster::twitch_user_id() else {
        // No member to attach a claim to; a Twitch sign-in rebuilds the roster.
        return;
    };
    let now = Instant::now();
    let secs = now_secs();
    for provider in PROVIDERS {
        let Some(row) = roster.main_account(provider) else { continue };
        let connected = row.status == AccountStatus::Connected;
        let planned = with_state(|s| {
            load_member(s, &member);
            let reported = s.platforms.entry(provider).or_default();
            let planned = plan(connected, row.account_id.as_deref(), reported, now, secs);
            if let Plan::Send(_, key) = &planned {
                // Taken now, so a second roster arriving mid-request sends nothing.
                reported.pending = Some(key.clone());
            }
            planned
        });
        let Plan::Send(report, key) = planned else { continue };
        let (name, avatar, member) = (row.name.clone(), row.avatar_url.clone(), member.clone());
        tauri::async_runtime::spawn(async move { send(member, provider, report, key, name, avatar).await });
    }
}

async fn send(
    member: String,
    provider: &'static str,
    report: Report,
    key: String,
    name: Option<String>,
    avatar: Option<String>,
) {
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
            let confirmed = Confirmed { key, at: now_secs() };
            write_confirmed(&member, provider, &confirmed);
            with_state(|s| {
                if s.member.as_deref() == Some(member.as_str()) {
                    let reported = s.platforms.entry(provider).or_default();
                    reported.confirmed = Some(confirmed);
                    reported.pending = None;
                }
            });
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
            let reported = s.platforms.entry(provider).or_default();
            reported.pending = None;
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

    const DAY: u64 = 24 * 60 * 60;
    const NOW: u64 = 1_800_000_000;

    fn reported(key: Option<&str>, at: u64) -> Reported {
        Reported {
            confirmed: key.map(|k| Confirmed { key: k.to_string(), at }),
            pending: None,
            not_before: None,
        }
    }

    fn claim(id: &str) -> Plan {
        Plan::Send(Report::Claim { account_id: Some(id.to_string()) }, format!("on:{id}"))
    }

    #[test]
    fn a_new_connection_is_claimed_with_its_account_id() {
        assert_eq!(plan(true, Some("676"), &reported(None, 0), Instant::now(), NOW), claim("676"));
    }

    #[test]
    fn a_relaunch_with_the_same_claim_sends_nothing() {
        let now = Instant::now();
        assert_eq!(plan(true, Some("676"), &reported(Some("on:676"), NOW - DAY), now, NOW), Plan::Nothing);
    }

    #[test]
    fn a_claim_is_asserted_again_after_a_week() {
        let now = Instant::now();
        assert_eq!(plan(true, Some("676"), &reported(Some("on:676"), NOW - 8 * DAY), now, NOW), claim("676"));
    }

    #[test]
    fn a_platform_never_claimed_here_is_never_released() {
        let now = Instant::now();
        assert_eq!(plan(false, None, &reported(None, 0), now, NOW), Plan::Nothing);
        assert_eq!(plan(false, None, &reported(Some("off"), NOW), now, NOW), Plan::Nothing);
    }

    #[test]
    fn a_disconnect_releases_our_claim() {
        let now = Instant::now();
        assert_eq!(
            plan(false, Some("676"), &reported(Some("on:676"), NOW), now, NOW),
            Plan::Send(Report::Release, "off".into())
        );
    }

    #[test]
    fn a_new_account_id_is_news_and_a_lost_one_is_not() {
        let now = Instant::now();
        assert_eq!(plan(true, Some("999"), &reported(Some("on:676"), NOW), now, NOW), claim("999"));
        assert_eq!(plan(true, None, &reported(Some("on:676"), NOW), now, NOW), Plan::Nothing);
        assert_eq!(plan(true, Some(""), &reported(Some("on:676"), NOW), now, NOW), Plan::Nothing);
    }

    #[test]
    fn a_report_in_flight_is_not_sent_twice() {
        let now = Instant::now();
        let inflight = Reported { confirmed: None, pending: Some("on:676".into()), not_before: None };
        assert_eq!(plan(true, Some("676"), &inflight, now, NOW), Plan::Nothing);
    }

    #[test]
    fn a_cooldown_is_waited_out() {
        let now = Instant::now();
        let waiting = Reported { confirmed: None, pending: None, not_before: Some(now + Duration::from_secs(60)) };
        assert_eq!(plan(true, Some("676"), &waiting, now, NOW), Plan::Nothing);
        assert_eq!(plan(true, Some("676"), &waiting, now + Duration::from_secs(61), NOW), claim("676"));
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
