//! Periodic "this client is alive, and this is what it is" report.
//!
//! ## Why this exists
//!
//! Before this service, `users.last_seen` and `users.app_version` were written
//! by exactly one thing: the frontend's `upsertUser`, whose only caller is gated
//! on the login TRANSITION (`AppStore.checkAuthStatus`, `if (!wasAuthenticated)`).
//! Nothing refreshed them afterwards. The consequences were all live in
//! production on 2026-09-19:
//!
//! - Closing to tray hides the window instead of exiting, so a client left
//!   running reported the timestamp of its last cold boot for as long as it
//!   stayed up. `last_seen` measured LOGINS, not usage, and DAU/WAU/MAU are
//!   computed straight off it.
//! - A member who switches accounts, or signs out of the active one and gets a
//!   linked account promoted, never re-enters that branch at all: their row was
//!   absent rather than stale.
//! - A client whose single write failed had no second chance for the rest of the
//!   session, and nothing anywhere recorded that it had failed. That cohort went
//!   INVISIBLE rather than looking broken, which is the worst possible failure
//!   for a measurement that decisions get made on.
//!
//! ## Why Rust
//!
//! It outlives every window (the main window is destroyed on close-to-tray and
//! recreated on demand), it must not depend on any page being mounted or any
//! React effect surviving, and the update-check outcome it carries is already
//! Rust-side state. Same reasoning as `home_snapshot` and the Rust-owned chat
//! state: if a fact belongs to the running client, the client's own process owns
//! reporting it.
//!
//! ## Two deliberate choices
//!
//! **It does not consult `client_config.write_via_api`.** That flag has never
//! once been true in production: `clientConfig.ts` reads it from the update
//! manifest, and the release pipeline rebuilds that manifest every ship without
//! ever emitting the key. A report that can be switched off by a field nobody
//! sets is a report that never runs. `/api/v1/user/sync` is server-authoritative
//! (it takes the user id from the bearer token, never the body), so there is
//! nothing to gate.
//!
//! **A failure is counted and reported on recovery.** `report_failures` rides
//! the next successful send, so a client that could not report for an hour says
//! so once it can. Without that, a broken cohort is silently missing and the
//! dashboard reads 100% healthy.

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use log::{debug, warn};
use serde_json::json;
use tauri::AppHandle;

use crate::services::client_identity;

/// How often a healthy client reports. Long enough to be free (four writes an
/// hour per user), short enough that "last seen" is a useful word.
const INTERVAL: Duration = Duration::from_secs(15 * 60);

/// Delay before the first send. Auth resolves during boot and a report with no
/// token is wasted, so this sits clear of startup rather than racing it.
const FIRST_DELAY: Duration = Duration::from_secs(90);

/// Backoff after a failed send. Deliberately shorter than `INTERVAL`: a client
/// that just failed is the one we most want to hear from.
const RETRY_DELAY: Duration = Duration::from_secs(2 * 60);

/// Consecutive failures before we stop retrying faster than the normal cadence.
/// Past this the client is probably offline, and hammering a dead network is
/// pointless.
const MAX_FAST_RETRIES: u32 = 3;

/// Failures since the last successful send. Reported on recovery and then reset,
/// so the dashboard can tell "reported reliably" from "reported once after
/// twenty failures".
static FAILURES: AtomicU32 = AtomicU32::new(0);

/// Start the heartbeat. Call once, from setup.
pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        // `sleep` on tokio's timer is monotonic, so a laptop suspended for eight
        // hours resumes and waits out the remainder rather than firing a burst
        // of catch-up sends the way a wall-clock schedule would.
        tokio::time::sleep(FIRST_DELAY).await;

        loop {
            let delay = match send_once(&app).await {
                Ok(()) => {
                    FAILURES.store(0, Ordering::Relaxed);
                    INTERVAL
                }
                Err(SendError::NoToken) => {
                    // Not a failure: nobody is signed in, so there is no member
                    // to attribute anything to. Do not count it, and do not
                    // retry fast; the next normal tick will find a token if one
                    // appears.
                    debug!("[VersionReport] no token yet, deferring heartbeat");
                    INTERVAL
                }
                Err(SendError::Failed(detail)) => {
                    let n = FAILURES.fetch_add(1, Ordering::Relaxed) + 1;
                    // `warn`, so it survives diagnostics being off. A user who
                    // reports "the dashboard says I have not used this in weeks"
                    // should leave a trace in their own log.
                    warn!("[VersionReport] heartbeat failed ({n} in a row): {detail}");
                    if n <= MAX_FAST_RETRIES {
                        RETRY_DELAY
                    } else {
                        INTERVAL
                    }
                }
            };
            tokio::time::sleep(delay).await;
        }
    });
}

enum SendError {
    /// Nobody signed in. Distinct from a failure on purpose: see the loop.
    NoToken,
    Failed(String),
}

async fn send_once(app: &AppHandle) -> Result<(), SendError> {
    let id = client_identity::current(app);
    let last = crate::commands::components::update_check::last();

    let mut body = json!({
        "app_version": id.app_version,
        "platform": id.platform,
        "arch": id.arch,
        "target": id.target,
        "channel": id.channel,
        "report_failures": FAILURES.load(Ordering::Relaxed),
    });

    // Only sent once a check has completed this session. Absent means "no check
    // has run yet", which the server stores as-is: that is a different fact from
    // "a check ran and failed", and conflating them would hide the case where
    // the timer never fires at all.
    if let Some(check) = last {
        let map = body.as_object_mut().expect("json! built an object");
        map.insert("update_last_check_at".into(), json!(check.checked_at));
        map.insert("update_last_check_ok".into(), json!(check.ok));
        map.insert(
            "update_artifact_for_platform".into(),
            json!(check.artifact_for_platform),
        );
        map.insert("update_available".into(), json!(check.update_available));
        map.insert("update_offered_version".into(), json!(check.offered_version));
        map.insert("update_last_error".into(), json!(check.error));
    }

    match crate::commands::streamnook_api::post_json("/api/v1/user/sync", &body, None).await {
        Ok(resp) if resp.ok => {
            debug!(
                "[VersionReport] heartbeat sent: {} {} ({})",
                id.app_version, id.target, id.channel
            );
            Ok(())
        }
        // A 4xx/5xx is a real answer from the server, and it is still a failure
        // to report: the row did not move.
        Ok(resp) => Err(SendError::Failed(format!(
            "HTTP {} {}",
            resp.status,
            resp.body.chars().take(200).collect::<String>()
        ))),
        Err(e) if e.starts_with("no_token") => Err(SendError::NoToken),
        Err(e) => Err(SendError::Failed(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_retry_is_faster_than_the_normal_cadence() {
        // The point of the backoff is to hear from a just-failed client sooner,
        // not later. Inverting these would silently make failures rarer in the
        // data than successes, which is backwards.
        assert!(RETRY_DELAY < INTERVAL);
    }

    #[test]
    fn the_first_send_waits_clear_of_boot_but_not_a_whole_interval() {
        // Long enough for auth to resolve, short enough that a user who opens
        // the app for ten minutes is still counted.
        assert!(FIRST_DELAY < INTERVAL);
        assert!(FIRST_DELAY >= Duration::from_secs(30));
    }
}
