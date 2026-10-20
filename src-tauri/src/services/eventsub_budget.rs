//! The account's EventSub socket budget, shared by the watch, moderation and
//! whisper sockets.
//!
//! Twitch allows three WebSocket transports with subscriptions per client id
//! and user, and every StreamNook app (desktop, Android, iPhone) signed in as
//! one person shares them. Once they are taken, every subscribe on a fourth
//! socket answers 429 "number of websocket transports limit exceeded", and
//! the socket, left with no subscription, is closed by Twitch 10 s later.
//! Retrying that on a short timer only spends requests: a slot frees when
//! another socket closes, which is minutes away or never. So a socket that
//! meets the limit waits 60 s, doubling to 300 s, says so once per episode,
//! and tries again early when something may have freed a slot: this app's
//! own subscribed socket closing, the app coming back to the front, or (for
//! the watch socket) a channel switch, which starts a fresh attempt anyway.

use log::{info, warn};
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::Notify;

use crate::services::eventsub_service::SocketOwner;

/// The first wait after the socket limit refused a subscribe.
pub const OVER_BUDGET_MIN_SECS: u64 = 60;
/// The longest wait it doubles to.
pub const OVER_BUDGET_MAX_SECS: u64 = 300;

/// Wakes every socket waiting out the limit.
static WAKE: Notify = Notify::const_new();

/// The sockets whose last attempt met the limit (one episode each, until a
/// subscribe goes through).
static OVER: Mutex<Vec<SocketOwner>> = Mutex::new(Vec::new());

/// Whether a refused subscribe is the account's socket limit: 429 with
/// "transport" in Twitch's answer. Any other 429 is a plain rate limit.
pub fn is_transport_limit(status: u16, body: &str) -> bool {
    status == 429 && body.to_ascii_lowercase().contains("transport")
}

/// The wait before the next attempt after meeting the limit, and the wait to
/// carry after that.
pub fn over_budget_wait(retry_secs: u64) -> (u64, u64) {
    let wait = retry_secs.clamp(OVER_BUDGET_MIN_SECS, OVER_BUDGET_MAX_SECS);
    (wait, (wait * 2).min(OVER_BUDGET_MAX_SECS))
}

fn over() -> std::sync::MutexGuard<'static, Vec<SocketOwner>> {
    OVER.lock().unwrap_or_else(|p| p.into_inner())
}

/// `owner` met the limit. True when this starts an episode (the one time it
/// is logged).
pub fn note_over_budget(owner: SocketOwner) -> bool {
    let mut over = over();
    if over.contains(&owner) {
        return false;
    }
    over.push(owner);
    true
}

/// `owner` subscribed: its episode is over. True when one was running.
pub fn note_room(owner: SocketOwner) -> bool {
    let mut over = over();
    let before = over.len();
    over.retain(|o| *o != owner);
    over.len() != before
}

/// Whether `owner`'s last attempt met the limit.
pub fn over_budget(owner: SocketOwner) -> bool {
    over().contains(&owner)
}

/// Log the start of an episode once.
pub fn log_episode(owner_label: &str, wait_secs: u64) {
    warn!(
        "[EventSub] {owner_label} socket: Twitch's socket limit for this account is reached (3 per account, shared with every device signed in as it); retrying in {wait_secs}s, then up to every {OVER_BUDGET_MAX_SECS}s"
    );
}

/// Log the end of an episode once.
pub fn log_room(owner_label: &str) {
    info!("[EventSub] {owner_label} socket: subscribed again, the socket limit has room");
}

/// Something may have freed a slot (a subscribed socket of this app closed,
/// the app came back to the front): sockets waiting out the limit try now.
pub fn wake() {
    WAKE.notify_waiters();
}

/// Wait `secs`, or less when `wake` is called meanwhile. True when woken.
pub async fn wait(secs: u64) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(Duration::from_secs(secs)) => false,
        _ = WAKE.notified() => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_transport_limit_reads_as_over_budget() {
        let body = r#"{"error":"Too Many Requests","status":429,"message":"number of websocket transports limit exceeded"}"#;
        assert!(is_transport_limit(429, body));
        assert!(is_transport_limit(429, "Number of WebSocket Transports limit exceeded"));
        assert!(!is_transport_limit(429, r#"{"message":"rate limit exceeded"}"#), "a plain rate limit");
        assert!(!is_transport_limit(409, "transport"), "a duplicate subscription");
        assert!(!is_transport_limit(403, "transport"));
    }

    #[test]
    fn the_limit_waits_a_minute_doubling_to_five() {
        assert_eq!(over_budget_wait(5), (60, 120));
        assert_eq!(over_budget_wait(120), (120, 240));
        assert_eq!(over_budget_wait(240), (240, 300));
        assert_eq!(over_budget_wait(300), (300, 300));
        assert_eq!(over_budget_wait(9_000), (300, 300));
    }

    #[test]
    fn an_episode_is_logged_once_and_ends_with_a_subscribe() {
        let owner = SocketOwner::Moderation;
        note_room(owner);
        assert!(note_over_budget(owner), "the first refusal starts an episode");
        assert!(!note_over_budget(owner), "the next ones do not log again");
        assert!(over_budget(owner));
        assert!(note_room(owner), "a subscribe ends it");
        assert!(!over_budget(owner));
        assert!(!note_room(owner));
    }

    #[test]
    fn a_wake_cuts_the_wait_short() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let waiting = tokio::spawn(wait(300));
            tokio::task::yield_now().await;
            wake();
            let woken = tokio::time::timeout(Duration::from_secs(2), waiting).await;
            assert_eq!(woken.expect("woke within the wait").unwrap(), true);
            // Without a wake it runs its course.
            assert_eq!(tokio::time::timeout(Duration::from_secs(2), wait(0)).await.unwrap(), false);
        });
    }
}
