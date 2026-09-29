//! Twitch Recap hours for the signed-in member, published so their StreamNook
//! profile can count the watching they did before they joined.
//!
//! Twitch's annual recap (`Query.annualRecap`) is the only viewer-side watch
//! total Twitch exposes, one closed year at a time. This service fetches every
//! year Twitch offers for the signed-in account, keeps the answers on disk (a
//! closed year never changes), and posts them to `/api/v1/stats/recap`. The
//! server decides which years count, against the member's join date, because
//! every viewer of the profile must see the same number.
//!
//! ## Which token
//!
//! Only what the member already has; this never asks them to sign in to
//! anything. The twitch.tv web session their StreamNook sign-in created
//! (`TwitchAuthService`, the same one the Turbo and subscription checks use).
//! It is the only credential that works: Twitch's GQL refuses our own client
//! ids, and refuses the StreamNook login token under its web client id ("The
//! Authorization token is invalid"). The account is read back from that
//! session (`currentUser.id`) and the write is authenticated as that same
//! account, so a recap can never be filed under a different member.
//!
//! ## Which years
//!
//! Twitch accepts only the years it has published (`YEAR_2023` onwards), and a
//! year it has not published fails validation (`Unknown type ViewerRecap2026`).
//! So the service walks forward from the first year until Twitch rejects one,
//! which picks up each new recap in December with no release.
//!
//! ## Cost
//!
//! One tick a day, and a tick does nothing unless a week has passed since the
//! last probe or an answer is still unpublished. A probe is a handful of small
//! GQL requests. Nothing stays resident beyond the tiny cache file.

use std::collections::BTreeMap;
use std::time::Duration;

use chrono::Datelike;
use log::{debug, warn};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::Manager;

use crate::models::settings::AppState;
use crate::rt::AppHandle;
use crate::services::account_store::AccountStore;
use crate::services::auth_proxy;

/// The first year Twitch publishes a recap for.
const FIRST_YEAR: i32 = 2023;

/// Delay before the first tick, clear of boot and of the version report.
const FIRST_DELAY: Duration = Duration::from_secs(3 * 60);

/// How often the loop wakes. Cheap: a tick with nothing to do reads one file.
const TICK: Duration = Duration::from_secs(24 * 60 * 60);

/// How long a probe's answers are trusted before Twitch is asked again, which
/// is how a newly published year, or a year that had no data, is picked up.
const PROBE_TTL_MS: i64 = 7 * 24 * 60 * 60 * 1000;

/// Bound on reading the web session. On a phone it waits on the UI thread, and a
/// backgrounded webview must not park the loop forever.
const TOKEN_BUDGET: Duration = Duration::from_secs(30);

const CACHE_FILE: &str = "twitch_recap.json";

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub struct YearRecap {
    pub hours: i64,
    pub days: i64,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
struct AccountRecap {
    /// Years with a recap that has hours in it. A year with no data is not
    /// kept, so the next probe asks again.
    years: BTreeMap<i32, YearRecap>,
    last_probe_ms: i64,
    /// Whether the server has the current `years`.
    published: bool,
}

/// Per Twitch user id.
type Cache = BTreeMap<String, AccountRecap>;

/// Start the daily loop. Call once, from setup.
pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_DELAY).await;
        loop {
            if let Err(e) = tick(&app).await {
                warn!("[TwitchRecap] {e}");
            }
            tokio::time::sleep(TICK).await;
        }
    });
}

async fn tick(app: &AppHandle) -> Result<(), String> {
    let mut cache = read_cache();
    let now = chrono::Utc::now().timestamp_millis();

    // Nothing to ask Twitch and nothing to publish for the signed-in account:
    // skip the web session read, which on a desktop opens a short-lived hidden
    // window. The registry is only the cheap gate; the account actually used is
    // the one the web session names below.
    let Some(primary) = AccountStore::primary().map(|a| a.user_id) else {
        return Ok(());
    };
    let due = cache.get(&primary).is_none_or(|a| {
        now - a.last_probe_ms >= PROBE_TTL_MS || (!a.published && !a.years.is_empty())
    });
    if !due {
        return Ok(());
    }

    // Never asks the member for anything: without a twitch.tv session the next
    // tick tries again.
    let Some(token) = web_session_token(app).await else {
        return Ok(());
    };
    let user_id = current_user_id(&token).await?;
    let entry = cache.entry(user_id.clone()).or_default();

    if now - entry.last_probe_ms >= PROBE_TTL_MS {
        let fetched = fetch_all_years(&token, &user_id).await?;
        if fetched != entry.years {
            entry.years = fetched;
            entry.published = false;
        }
        entry.last_probe_ms = now;
        write_cache(&cache);
    }

    let entry = cache.get_mut(&user_id).expect("inserted above");
    if !entry.published && !entry.years.is_empty() {
        publish(&user_id, &entry.years).await?;
        entry.published = true;
        write_cache(&cache);
    }
    Ok(())
}

/// The twitch.tv session the member's StreamNook sign-in created, if it can
/// be read in time.
async fn web_session_token(app: &AppHandle) -> Option<String> {
    let auth = app.try_state::<AppState>().map(|s| s.twitch_auth.clone())?;
    match tokio::time::timeout(TOKEN_BUDGET, auth.get_token()).await {
        Ok(Ok(t)) => Some(t),
        Ok(Err(e)) => {
            debug!("[TwitchRecap] no web session: {e}");
            None
        }
        Err(_) => {
            debug!("[TwitchRecap] web session read timed out");
            None
        }
    }
}

async fn current_user_id(token: &str) -> Result<String, String> {
    let resp = auth_proxy::gql_query(token, json!({ "query": "query { currentUser { id } }" }))
        .await
        .map_err(|e| format!("currentUser request failed: {e:#}"))?;
    resp["data"]["currentUser"]["id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("no currentUser in the answer: {}", resp["errors"]))
}

/// Every published year's recap, from `FIRST_YEAR` until Twitch rejects a year.
/// Any other failure aborts the whole probe, so a transient error never reads
/// as "this member has no recap".
async fn fetch_all_years(token: &str, user_id: &str) -> Result<BTreeMap<i32, YearRecap>, String> {
    let mut years = BTreeMap::new();
    for year in FIRST_YEAR..=chrono::Utc::now().year() {
        let resp = auth_proxy::gql_query(token, json!({ "query": year_query(user_id, year) }))
            .await
            .map_err(|e| format!("recap {year} request failed: {e:#}"))?;
        match classify(&resp, year) {
            YearAnswer::Recap(r) => {
                years.insert(year, r);
            }
            YearAnswer::Empty => {}
            YearAnswer::NotPublished => break,
            YearAnswer::Failed(detail) => return Err(format!("recap {year}: {detail}")),
        }
    }
    Ok(years)
}

/// Inline, because each year's viewer payload is its own union member
/// (`ViewerRecap2025`) and the year is an enum literal.
fn year_query(user_id: &str, year: i32) -> String {
    let id = serde_json::to_string(user_id).expect("a string serializes");
    format!(
        "query {{ annualRecap(channelID: {id}, options: {{year: YEAR_{year}}}) {{ error viewerRecap {{ ... on ViewerRecap{year} {{ totalHoursWatched distinctDaysWatched }} }} }} }}"
    )
}

#[derive(Debug, PartialEq, Eq)]
enum YearAnswer {
    Recap(YearRecap),
    /// Twitch answered, with no watching recorded for that year.
    Empty,
    /// Twitch has not published this year.
    NotPublished,
    Failed(String),
}

fn classify(resp: &Value, year: i32) -> YearAnswer {
    // A validation failure carries no `data` at all. Naming the year's type or
    // enum is how an unpublished year fails; anything else is a real error.
    if resp.get("data").is_none() {
        let errors = resp["errors"].to_string();
        let unpublished = errors.contains(&format!("ViewerRecap{year}"))
            || errors.contains(&format!("YEAR_{year}"))
            || errors.contains("AnnualRecapYear");
        return if unpublished {
            YearAnswer::NotPublished
        } else {
            YearAnswer::Failed(errors)
        };
    }
    let recap = &resp["data"]["annualRecap"];
    if recap.is_null() {
        return YearAnswer::Failed(resp["errors"].to_string());
    }
    let viewer = &recap["viewerRecap"];
    let hours = viewer["totalHoursWatched"].as_i64().unwrap_or(0);
    if hours <= 0 {
        return YearAnswer::Empty;
    }
    let days = viewer["distinctDaysWatched"].as_i64().unwrap_or(0);
    YearAnswer::Recap(YearRecap { hours, days })
}

async fn publish(user_id: &str, years: &BTreeMap<i32, YearRecap>) -> Result<(), String> {
    let rows: Vec<Value> = years
        .iter()
        .map(|(year, r)| json!({ "year": year, "hours": r.hours, "days": r.days }))
        .collect();
    let body = json!({ "years": rows });
    let resp = crate::commands::streamnook_api::post_json("/api/v1/stats/recap", &body, Some(user_id)).await?;
    if resp.ok {
        debug!("[TwitchRecap] published {} year(s)", rows.len());
        Ok(())
    } else {
        Err(format!(
            "publish answered HTTP {} {}",
            resp.status,
            resp.body.chars().take(200).collect::<String>()
        ))
    }
}

fn cache_path() -> Option<std::path::PathBuf> {
    crate::services::twitch_service::get_app_data_dir()
        .ok()
        .map(|dir| dir.join(CACHE_FILE))
}

fn read_cache() -> Cache {
    cache_path()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|raw| serde_json::from_slice(&raw).ok())
        .unwrap_or_default()
}

/// Written to a temp file and renamed, so a crash mid-write never leaves a
/// truncated cache behind.
fn write_cache(cache: &Cache) {
    let Some(path) = cache_path() else {
        return;
    };
    let tmp = path.with_extension("json.tmp");
    let result = serde_json::to_vec(cache)
        .map_err(|e| e.to_string())
        .and_then(|bytes| std::fs::write(&tmp, bytes).map_err(|e| e.to_string()))
        .and_then(|_| std::fs::rename(&tmp, &path).map_err(|e| e.to_string()));
    if let Err(e) = result {
        warn!("[TwitchRecap] could not save the recap cache: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unpublished_year_stops_the_walk() {
        let resp = json!({ "errors": [{ "message": "Unknown type \"ViewerRecap2026\"." }] });
        assert_eq!(classify(&resp, 2026), YearAnswer::NotPublished);
    }

    #[test]
    fn a_signed_out_answer_is_a_failure_not_an_empty_year() {
        let resp = json!({
            "errors": [{ "message": "unauthenticated" }],
            "data": { "annualRecap": null }
        });
        assert!(matches!(classify(&resp, 2025), YearAnswer::Failed(_)));
    }

    #[test]
    fn other_validation_errors_are_failures() {
        let resp = json!({ "errors": [{ "message": "Cannot query field \"x\"" }] });
        assert!(matches!(classify(&resp, 2025), YearAnswer::Failed(_)));
    }

    #[test]
    fn a_year_with_hours_is_kept() {
        let resp = json!({ "data": { "annualRecap": {
            "error": null,
            "viewerRecap": { "totalHoursWatched": 812, "distinctDaysWatched": 240 }
        } } });
        assert_eq!(classify(&resp, 2024), YearAnswer::Recap(YearRecap { hours: 812, days: 240 }));
    }

    #[test]
    fn a_year_without_watching_is_empty() {
        let resp = json!({ "data": { "annualRecap": { "error": "NO_DATA", "viewerRecap": null } } });
        assert_eq!(classify(&resp, 2023), YearAnswer::Empty);
    }

    #[test]
    fn the_query_names_the_year_type_and_quotes_the_id() {
        let q = year_query("12345", 2025);
        assert!(q.contains("channelID: \"12345\""));
        assert!(q.contains("year: YEAR_2025"));
        assert!(q.contains("... on ViewerRecap2025"));
    }
}
