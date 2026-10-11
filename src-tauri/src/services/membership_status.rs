//! The member's own membership status line: paid or gifted, when a gift ends,
//! and when their own billing starts again.
//!
//! streamnook.app computes this once, server-side, from the subscription, the
//! gift and the billing pause a gift put in place, and serves the same value to
//! the account page and to `POST /api/v1/membership/status`. The app keeps the
//! answer and hands it to the profile as is; no billing rule is re-derived here.
//!
//! Fetched when the profile asks (a repeat within `REUSE_FOR` is served from
//! memory), and again whenever the gift poll sees a new gift, which emits
//! `EVENT` so an open profile updates in place. Nothing polls on a timer.

use anyhow::{anyhow, Result};
use log::debug;
use serde::{Deserialize, Serialize};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::Emitter;

use crate::rt::AppHandle;
use crate::services::account_store::AccountStore;

const PATH: &str = "/api/v1/membership/status";

/// Reopening the profile within this window reuses the last answer.
const REUSE_FOR: Duration = Duration::from_secs(60);

/// Emitted with the new status whenever a fetch changes it.
pub const EVENT: &str = "membership-status-changed";

static APP: OnceLock<AppHandle> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MembershipKind {
    Paid,
    Gifted,
    GiftedOverPaid,
    Boost,
    #[serde(other)]
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BillingNote {
    Paused,
    Continues,
    ContinuesPaypal,
    NoneSetUp,
    EndsAtPeriodEnd,
    Renews,
    PastDue,
    /// A note a newer server knows and this build does not.
    #[serde(other)]
    Unknown,
}

/// The server's answer, field for field. Read in the server's snake_case,
/// handed to the frontend in camelCase.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all(serialize = "camelCase", deserialize = "snake_case"))]
pub struct MembershipStatus {
    pub kind: MembershipKind,
    #[serde(default)]
    pub processor: Option<String>,
    #[serde(default)]
    pub gift_permanent: bool,
    #[serde(default)]
    pub gift_starts_at: Option<String>,
    #[serde(default)]
    pub gift_ends_at: Option<String>,
    #[serde(default)]
    pub billing_note: Option<BillingNote>,
    #[serde(default)]
    pub billing_resumes_at: Option<String>,
    #[serde(default)]
    pub renews_at: Option<String>,
    #[serde(default)]
    pub paid_ends_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StatusResponse {
    user_id: String,
    status: MembershipStatus,
}

struct Cached {
    user_id: String,
    at: Instant,
    status: MembershipStatus,
}

/// A std mutex, never held across an await.
static CACHE: Mutex<Option<Cached>> = Mutex::new(None);

pub fn init(app: AppHandle) {
    let _ = APP.set(app);
}

fn cached_for(user_id: &str, max_age: Duration) -> Option<MembershipStatus> {
    let guard = CACHE.lock().unwrap_or_else(|p| p.into_inner());
    guard
        .as_ref()
        .filter(|c| c.user_id == user_id && c.at.elapsed() < max_age)
        .map(|c| c.status.clone())
}

/// Stores a fresh answer; true when it differs from what was held for that
/// account (a first answer counts as a change).
fn store(user_id: &str, status: &MembershipStatus) -> bool {
    let mut guard = CACHE.lock().unwrap_or_else(|p| p.into_inner());
    let changed = guard
        .as_ref()
        .map_or(true, |c| c.user_id != user_id || &c.status != status);
    *guard = Some(Cached {
        user_id: user_id.to_string(),
        at: Instant::now(),
        status: status.clone(),
    });
    changed
}

/// The signed-in member's status. `force` skips the reuse window. None when
/// nobody is signed in.
pub async fn get(force: bool) -> Result<Option<MembershipStatus>> {
    let Some(primary) = AccountStore::primary() else {
        return Ok(None);
    };
    if !force {
        if let Some(hit) = cached_for(&primary.user_id, REUSE_FOR) {
            return Ok(Some(hit));
        }
    }

    let resp = crate::commands::streamnook_api::post_json(PATH, &serde_json::json!({}), None)
        .await
        .map_err(|e| anyhow!(e))?;
    if !resp.ok {
        return Err(anyhow!("status request failed: {} {}", resp.status, resp.body));
    }
    let parsed: StatusResponse = serde_json::from_str(&resp.body)?;

    let changed = store(&parsed.user_id, &parsed.status);
    // The bearer is whoever is primary now; an answer for another account is
    // kept for that account but not handed to this caller.
    if parsed.user_id != primary.user_id {
        return Ok(None);
    }
    if changed {
        if let Some(app) = APP.get() {
            let _ = app.emit(EVENT, &parsed.status);
        }
    }
    Ok(Some(parsed.status))
}

/// Fetch again in the background, after something changed server-side (the
/// gift poll saw a new gift). An open profile updates through `EVENT`.
pub fn refresh_soon() {
    tauri::async_runtime::spawn(async {
        if let Err(e) = get(true).await {
            debug!("[MembershipStatus] refresh failed: {e}");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_answer_parses_and_reaches_the_frontend_in_camel_case() {
        let raw = r#"{"user_id":"42","status":{"kind":"gifted_over_paid","processor":"stripe","gift_permanent":false,"gift_starts_at":null,"gift_ends_at":"2027-02-01T00:00:00.000Z","billing_note":"paused","billing_resumes_at":"2027-02-01T00:00:00.000Z","renews_at":null,"paid_ends_at":null}}"#;
        let parsed: StatusResponse = serde_json::from_str(raw).expect("parses");
        assert_eq!(parsed.user_id, "42");
        assert_eq!(parsed.status.kind, MembershipKind::GiftedOverPaid);
        assert_eq!(parsed.status.billing_note, Some(BillingNote::Paused));

        let json = serde_json::to_value(&parsed.status).expect("serializes");
        assert_eq!(json["kind"], "gifted_over_paid");
        assert_eq!(json["billingNote"], "paused");
        assert_eq!(json["billingResumesAt"], "2027-02-01T00:00:00.000Z");
        assert_eq!(json["giftPermanent"], false);
    }

    #[test]
    fn every_note_round_trips() {
        for (raw, note) in [
            ("paused", BillingNote::Paused),
            ("continues", BillingNote::Continues),
            ("continues_paypal", BillingNote::ContinuesPaypal),
            ("none_set_up", BillingNote::NoneSetUp),
            ("ends_at_period_end", BillingNote::EndsAtPeriodEnd),
            ("renews", BillingNote::Renews),
            ("past_due", BillingNote::PastDue),
        ] {
            let parsed: BillingNote = serde_json::from_str(&format!("\"{raw}\"")).expect("parses");
            assert_eq!(parsed, note);
            assert_eq!(serde_json::to_value(note).unwrap(), raw);
        }
    }

    #[test]
    fn unknown_values_from_a_newer_server_do_not_break_parsing() {
        let raw = r#"{"kind":"something_new","billing_note":"also_new"}"#;
        let parsed: MembershipStatus = serde_json::from_str(raw).expect("parses");
        assert_eq!(parsed.kind, MembershipKind::None);
        assert_eq!(parsed.billing_note, Some(BillingNote::Unknown));
        assert!(!parsed.gift_permanent);
    }

    #[test]
    fn the_cache_reports_changes_per_account() {
        let a = MembershipStatus {
            kind: MembershipKind::Paid,
            processor: Some("stripe".into()),
            gift_permanent: false,
            gift_starts_at: None,
            gift_ends_at: None,
            billing_note: Some(BillingNote::Renews),
            billing_resumes_at: None,
            renews_at: Some("2026-11-01T00:00:00Z".into()),
            paid_ends_at: None,
        };
        let mut b = a.clone();
        b.kind = MembershipKind::GiftedOverPaid;
        b.billing_note = Some(BillingNote::Paused);

        assert!(store("test-cache-1", &a));
        assert!(!store("test-cache-1", &a));
        assert_eq!(cached_for("test-cache-1", REUSE_FOR), Some(a.clone()));
        assert_eq!(cached_for("someone-else", REUSE_FOR), None);
        assert_eq!(cached_for("test-cache-1", Duration::ZERO), None);
        assert!(store("test-cache-1", &b));
        assert!(store("test-cache-2", &b));
    }
}
