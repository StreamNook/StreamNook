//! Memberships somebody gave this member, announced once each.
//!
//! streamnook.app's `POST /api/v1/membership/gifts` lists the caller's gifts,
//! identified by the bearer alone. It carries no gifter identity (the server
//! never stores one where this can reach), so the notice never names anyone.
//!
//! This replaced watching the subscriber badge arrive over the cosmetics
//! channel. Cosmetics are permanent, so anyone who had EVER held the badge
//! (every current and past paying member) was never told about a gift at all.
//! A gift row exists for every gift, stacked or not.
//!
//! Seeding: the server hands out a numeric gift id per member. The first poll
//! for an account records the newest id silently, so a first launch never
//! announces old gifts; after that, every gift with a higher id is announced,
//! including ones that landed while the app was closed. The cursor is kept per
//! account, so switching accounts neither replays nor swallows anything.

use anyhow::{anyhow, Result};
use chrono::{DateTime, TimeZone, Utc};
use log::{debug, info, warn};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tauri::async_runtime::JoinHandle;
use tauri::Emitter;

use crate::models::settings::Settings;
use crate::rt::AppHandle;
use crate::services::account_store::AccountStore;
use crate::services::cache_service;

const PATH: &str = "/api/v1/membership/gifts";

/// Gifts are rare and nothing about them is urgent; this keeps an idle app
/// to one small request every ten minutes.
const POLL_INTERVAL: Duration = Duration::from_secs(600);

const STATE_FILE: &str = "membership_gifts_state.json";

/// The event the notification centre listens for.
pub const EVENT: &str = "membership-gift-received";

static APP: OnceLock<AppHandle> = OnceLock::new();
/// A std mutex, only ever held across a spawn or an abort, never an await.
static POLL: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);

/// One gift as the server lists it.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct GiftRow {
    pub id: u64,
    /// None for a permanent grant.
    pub term_months: Option<u32>,
    /// When the gifted membership runs out, counting any gift it stacked onto.
    pub membership_until: Option<String>,
    /// Set only when the member's own billing is paused until then.
    pub billing_resumes_at: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
pub struct GiftsResponse {
    pub user_id: String,
    pub latest_id: u64,
    #[serde(default)]
    pub gifts: Vec<GiftRow>,
}

/// What the notification centre is handed. The sentence is built here so the
/// row, the toast and the OS notification all say the same thing.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MembershipGiftNotification {
    pub id: u64,
    pub granted_at: String,
    pub permanent: bool,
    pub term_months: Option<u32>,
    pub billing_resumes_at: Option<String>,
    pub body_plain: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct State {
    /// Twitch user id -> newest gift id already handled.
    #[serde(default)]
    cursors: BTreeMap<String, u64>,
}

fn state_path() -> Result<PathBuf> {
    Ok(cache_service::get_app_data_dir()?.join(STATE_FILE))
}

fn read_state() -> State {
    state_path()
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn write_state(state: &State) {
    let Ok(path) = state_path() else { return };
    match serde_json::to_string(state) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, json) {
                warn!("[MembershipGifts] could not persist state: {e}");
            }
        }
        Err(e) => warn!("[MembershipGifts] could not serialize state: {e}"),
    }
}

/// Which gifts to announce, and the cursor to keep afterwards.
///
/// No cursor means this account is seen for the first time: record the newest
/// id and announce nothing. With a cursor, every listed gift above it is new.
/// The server leaves refunded gifts out of the list, so an empty list with a
/// higher `latest_id` moves the cursor past them; a non-empty list moves it
/// only as far as the last gift listed, so a long backlog pages through.
pub fn advance(cursor: Option<u64>, resp: &GiftsResponse) -> (u64, Vec<GiftRow>) {
    let Some(cursor) = cursor else {
        return (resp.latest_id, Vec::new());
    };
    let fresh: Vec<GiftRow> = resp.gifts.iter().filter(|g| g.id > cursor).cloned().collect();
    let next = match fresh.iter().map(|g| g.id).max() {
        Some(last) => last.max(cursor),
        None => resp.latest_id.max(cursor),
    };
    (next, fresh)
}

fn parse_time(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw).ok().map(|d| d.with_timezone(&Utc))
}

fn long_date<Tz: TimeZone>(at: &DateTime<Utc>, tz: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    at.with_timezone(tz).format("%B %-d, %Y").to_string()
}

/// The sentence for one gift, with dates in `tz`.
pub fn gift_body<Tz: TimeZone>(gift: &GiftRow, tz: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let Some(months) = gift.term_months else {
        return "Somebody gave you a StreamNook membership. It does not expire.".to_string();
    };
    let mut body = format!(
        "Somebody gave you {months} month{} of StreamNook membership.",
        if months == 1 { "" } else { "s" }
    );
    if let Some(resumes) = gift.billing_resumes_at.as_deref().and_then(parse_time) {
        body.push_str(&format!(
            " Your billing is paused until {}, then resumes on its own.",
            long_date(&resumes, tz)
        ));
    } else if let Some(until) = gift.membership_until.as_deref().and_then(parse_time) {
        body.push_str(&format!(" Your membership now runs through {}.", long_date(&until, tz)));
    }
    body
}

fn to_notification(gift: &GiftRow) -> MembershipGiftNotification {
    MembershipGiftNotification {
        id: gift.id,
        granted_at: gift.created_at.clone(),
        permanent: gift.term_months.is_none(),
        term_months: gift.term_months,
        billing_resumes_at: gift.billing_resumes_at.clone(),
        body_plain: gift_body(gift, &chrono::Local),
    }
}

/// One poll: everything new for the signed-in account since the last one.
pub async fn collect_new() -> Result<Vec<MembershipGiftNotification>> {
    let Some(primary) = AccountStore::primary() else {
        return Ok(Vec::new());
    };
    let mut state = read_state();
    let cursor = state.cursors.get(&primary.user_id).copied();

    let resp = crate::commands::streamnook_api::post_json(
        PATH,
        &serde_json::json!({ "after": cursor }),
        None,
    )
    .await
    .map_err(|e| anyhow!(e))?;
    if !resp.ok {
        return Err(anyhow!("gifts request failed: {} {}", resp.status, resp.body));
    }
    let parsed: GiftsResponse = serde_json::from_str(&resp.body)?;

    // The bearer belongs to whoever is primary NOW; if that changed between
    // reading the cursor and the answer, the cursor sent was someone else's.
    let cursor = if parsed.user_id == primary.user_id {
        cursor
    } else {
        state.cursors.get(&parsed.user_id).copied()
    };
    let seeding = cursor.is_none();
    let (next, fresh) = advance(cursor, &parsed);
    let fresh = if parsed.user_id == primary.user_id { fresh } else { Vec::new() };

    if cursor != Some(next) {
        state.cursors.insert(parsed.user_id.clone(), next);
        write_state(&state);
    }
    if seeding {
        info!("[MembershipGifts] first run for this account, seeded at {next}");
    }
    Ok(fresh.iter().map(to_notification).collect())
}

pub fn init(app: AppHandle) {
    let _ = APP.set(app);
}

/// Spawn or abort the poll to match the settings. Rides the notification
/// master switch alone, like the notice it feeds. Desktop only: the phone has
/// no notification centre for it.
pub fn refresh(settings: &Settings) {
    let want = cfg!(desktop) && settings.live_notifications.enabled;
    let mut guard = POLL.lock().unwrap_or_else(|p| p.into_inner());
    match (want, guard.is_some()) {
        (true, false) => {
            debug!("[MembershipGifts] enabling the gift poll");
            *guard = Some(spawn_poll());
        }
        (false, true) => {
            debug!("[MembershipGifts] disabling the gift poll");
            if let Some(handle) = guard.take() {
                handle.abort();
            }
        }
        _ => {}
    }
}

/// Spawned through Tauri's runtime handle: the first call comes from `setup()`,
/// where no Tokio runtime is entered.
fn spawn_poll() -> JoinHandle<()> {
    tauri::async_runtime::spawn(async move {
        loop {
            match collect_new().await {
                Ok(found) => {
                    if !found.is_empty() {
                        info!("[MembershipGifts] {} new gift(s)", found.len());
                        // A gift changes the profile's status line too.
                        crate::services::membership_status::refresh_soon();
                    }
                    if let Some(app) = APP.get() {
                        for item in found {
                            let _ = app.emit(EVENT, &item);
                        }
                    }
                }
                // Offline, signed out or a server hiccup: try again next tick.
                Err(e) => debug!("[MembershipGifts] poll failed: {e}"),
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gift(id: u64) -> GiftRow {
        GiftRow {
            id,
            term_months: Some(3),
            membership_until: Some("2027-01-28T08:30:00Z".to_string()),
            billing_resumes_at: None,
            created_at: "2026-10-10T12:00:00Z".to_string(),
        }
    }

    fn resp(latest_id: u64, gifts: Vec<GiftRow>) -> GiftsResponse {
        GiftsResponse { user_id: "1".to_string(), latest_id, gifts }
    }

    #[test]
    fn first_poll_seeds_silently() {
        let (next, fresh) = advance(None, &resp(7, Vec::new()));
        assert_eq!(next, 7);
        assert!(fresh.is_empty());
    }

    #[test]
    fn first_poll_for_a_member_with_no_gifts_seeds_at_zero() {
        let (next, fresh) = advance(None, &resp(0, Vec::new()));
        assert_eq!(next, 0);
        assert!(fresh.is_empty());
        // ...and their very first gift is then announced.
        let (next, fresh) = advance(Some(0), &resp(4, vec![gift(4)]));
        assert_eq!(next, 4);
        assert_eq!(fresh, vec![gift(4)]);
    }

    #[test]
    fn announces_only_gifts_past_the_cursor() {
        let (next, fresh) = advance(Some(5), &resp(9, vec![gift(5), gift(8), gift(9)]));
        assert_eq!(next, 9);
        assert_eq!(fresh.iter().map(|g| g.id).collect::<Vec<_>>(), vec![8, 9]);
    }

    #[test]
    fn nothing_new_keeps_the_cursor() {
        let (next, fresh) = advance(Some(9), &resp(9, Vec::new()));
        assert_eq!(next, 9);
        assert!(fresh.is_empty());
    }

    #[test]
    fn refunded_gifts_are_stepped_over() {
        // The server lists nothing (gift 10 was refunded) but knows of id 10.
        let (next, fresh) = advance(Some(9), &resp(10, Vec::new()));
        assert_eq!(next, 10);
        assert!(fresh.is_empty());
    }

    #[test]
    fn a_long_backlog_pages_from_the_last_gift_listed() {
        let page: Vec<GiftRow> = (11..=30).map(gift).collect();
        let (next, fresh) = advance(Some(10), &resp(45, page));
        assert_eq!(next, 30);
        assert_eq!(fresh.len(), 20);
    }

    #[test]
    fn body_names_months_and_never_a_gifter() {
        let body = gift_body(&gift(1), &Utc);
        assert_eq!(
            body,
            "Somebody gave you 3 months of StreamNook membership. Your membership now runs through January 28, 2027."
        );
    }

    #[test]
    fn body_says_when_billing_resumes() {
        let mut g = gift(1);
        g.term_months = Some(1);
        g.billing_resumes_at = Some("2026-11-28T08:30:00Z".to_string());
        assert_eq!(
            gift_body(&g, &Utc),
            "Somebody gave you 1 month of StreamNook membership. Your billing is paused until November 28, 2026, then resumes on its own."
        );
    }

    #[test]
    fn body_for_a_permanent_grant() {
        let mut g = gift(1);
        g.term_months = None;
        g.membership_until = None;
        assert_eq!(
            gift_body(&g, &Utc),
            "Somebody gave you a StreamNook membership. It does not expire."
        );
    }

    #[test]
    fn server_answer_parses() {
        let raw = r#"{"user_id":"42","latest_id":3,"gifts":[{"id":3,"term_months":6,"membership_until":null,"billing_resumes_at":"2027-04-01T00:00:00.000Z","created_at":"2026-10-10T12:00:00.123+00:00"}]}"#;
        let parsed: GiftsResponse = serde_json::from_str(raw).expect("parses");
        assert_eq!(parsed.gifts[0].term_months, Some(6));
        let body = gift_body(&parsed.gifts[0], &Utc);
        assert!(body.contains("6 months"));
        assert!(body.contains("April 1, 2027"));
    }

    #[test]
    fn notification_serializes_for_the_frontend() {
        let mut g = gift(5);
        g.term_months = None;
        let json = serde_json::to_value(to_notification(&g)).expect("serializes");
        assert_eq!(json["id"], 5);
        assert_eq!(json["permanent"], true);
        assert_eq!(json["grantedAt"], "2026-10-10T12:00:00Z");
        assert!(json["bodyPlain"].as_str().unwrap().contains("does not expire"));
    }
}
