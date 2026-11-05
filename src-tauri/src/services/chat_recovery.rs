//! Recovery for a Twitch chat connection that has died for good.
//!
//! The IRC supervisor retries forever and reads the token on every attempt,
//! but a socket that died while a phone slept sits there until the read
//! deadline notices, and a PART and re-JOIN rides that dead socket. A rebuild
//! stops the Twitch side and brings it back with every room the window held,
//! now rather than after the deadline and the retry backoff.
//!
//! Automatic triggers overlap (resuming the app and the stale watchdog fire
//! for the same dead connection, and every open room sees it), so one cycle
//! settles them all: never two at once, and automatic ones at most once a
//! minute. A manual reload is never throttled.
//!
//! The stop and the fresh start clear every claim, so a room a window joins
//! while the cycle runs is noted here too, and the cycle joins it at the end.

use crate::models::settings::AppState;
use crate::services::chat_service::ChatService;
use crate::services::irc_service::IrcService;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Emitted with [`Rebuilt`] after a cycle, so a shell reading the bridge
/// reconnects to its port and backfills the gap.
pub const EVENT: &str = "chat-rebuilt";

const COOLDOWN: Duration = Duration::from_secs(60);

static CYCLING: AtomicBool = AtomicBool::new(false);
static LAST_CYCLE: Mutex<Option<Instant>> = Mutex::new(None);

/// Window -> Twitch room -> whether its last word was a claim (`true`) or a
/// release (`false`).
type Journal = HashMap<String, HashMap<String, bool>>;

/// What windows claimed and let go of while a cycle runs; `None` outside one.
static DURING: Mutex<Option<Journal>> = Mutex::new(None);

fn during() -> std::sync::MutexGuard<'static, Option<Journal>> {
    DURING.lock().unwrap_or_else(|e| e.into_inner())
}

/// Notes that `window` claimed (`held`) or let go of Twitch `channel`, so a
/// cycle in flight brings back a room its stop swept. A no-op outside one.
pub fn note(channel: &str, window: &str, held: bool) {
    record(&mut during(), channel, window, held);
}

fn record(journal: &mut Option<Journal>, channel: &str, window: &str, held: bool) {
    if let Some(journal) = journal.as_mut() {
        journal.entry(window.to_string()).or_default().insert(channel.to_lowercase(), held);
    }
}

/// The rooms to join after the re-joins: those the window holds now or
/// claimed during the cycle and did not let go of again, less those already
/// brought back. Sorted.
fn late_rooms(brought_back: &[String], held_now: Vec<String>, noted: Option<HashMap<String, bool>>) -> Vec<String> {
    let mut late = held_now;
    late.extend(noted.into_iter().flatten().filter(|(_, held)| *held).map(|(channel, _)| channel));
    late.retain(|c| !brought_back.contains(c));
    late.sort();
    late.dedup();
    late
}

#[derive(serde::Serialize, Clone, Debug, PartialEq)]
pub struct Rebuilt {
    /// The bridge's port after the cycle.
    pub port: u16,
    /// The Twitch rooms brought back.
    pub channels: Vec<String>,
}

/// Whether an automatic cycle may run now: none ran within the cooldown.
fn cooled(last: Option<Instant>, now: Instant) -> bool {
    last.is_none_or(|at| now.duration_since(at) >= COOLDOWN)
}

/// Releases the single-flight flag and closes the journal however the cycle
/// ends.
struct Flight;

impl Drop for Flight {
    fn drop(&mut self) {
        *during() = None;
        CYCLING.store(false, Ordering::SeqCst);
    }
}

/// Tears the Twitch chat down and brings it back with every room `window`
/// held. `None` when no cycle ran: one is already running, an automatic one
/// ran within the minute, or the window holds no Twitch room.
pub async fn rebuild(state: &AppState, window: &str, force: bool, reason: &str) -> anyhow::Result<Option<Rebuilt>> {
    if !force && !cooled(*LAST_CYCLE.lock().unwrap_or_else(|e| e.into_inner()), Instant::now()) {
        return Ok(None);
    }
    if CYCLING.swap(true, Ordering::SeqCst) {
        return Ok(None);
    }
    let _flight = Flight;
    // Opened before the room list is read, so a claim lands in one or the
    // other.
    *during() = Some(Journal::new());
    let mut channels = IrcService::channels_held_by(window).await;
    if channels.is_empty() {
        return Ok(None);
    }
    *LAST_CYCLE.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
    // Warn, not debug: debug logging is off on a phone and this is what a
    // support log needs to show.
    log::warn!("[ChatRecovery] cycling {} room(s) for {}: {}", channels.len(), window, reason);
    // The 7TV live-update socket went through the same absence; a dead one
    // is reconnected now rather than minutes from now.
    crate::services::seventv_eventapi::check_after_resume();
    // Kick, YouTube and TikTok ride the same bridge: with any of them on it,
    // only the Twitch side goes down.
    if crate::services::providers::has_active_bridge_users() {
        IrcService::stop_twitch_only().await;
    } else {
        IrcService::stop().await?;
    }
    let (first, rest) = channels.split_first().expect("not empty");
    let port = ChatService::start(first, state, true, false, window).await?;
    for channel in rest {
        if let Err(e) = ChatService::join_channel(channel, state, window).await {
            log::warn!("[ChatRecovery] re-join {channel} failed: {e}");
        }
    }
    // A room the window joined while this ran was swept by the stop or the
    // fresh start, or joined a connection that was not up yet.
    let noted = during().as_mut().and_then(|journal| journal.remove(window));
    let late = late_rooms(&channels, IrcService::channels_held_by(window).await, noted);
    for channel in &late {
        log::warn!("[ChatRecovery] joining {channel}, claimed during the cycle");
        if let Err(e) = ChatService::join_channel(channel, state, window).await {
            log::warn!("[ChatRecovery] late join {channel} failed: {e}");
        }
    }
    channels.extend(late);
    channels.sort();
    Ok(Some(Rebuilt { port, channels }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_cycles_wait_a_minute() {
        let now = Instant::now();
        assert!(cooled(None, now));
        assert!(!cooled(Some(now), now + Duration::from_secs(59)));
        assert!(cooled(Some(now), now + Duration::from_secs(60)));
    }

    #[test]
    fn a_room_claimed_mid_cycle_is_joined_by_the_rebuild() {
        let mut journal: Option<Journal> = None;
        // Outside a cycle nothing is noted.
        record(&mut journal, "early", "main", true);
        assert!(journal.is_none());

        journal = Some(Journal::new());
        // Swept by the stop: no claim left, only the note.
        record(&mut journal, "xqc", "main", true);
        // Claimed and let go of again within the cycle.
        record(&mut journal, "shroud", "main", true);
        record(&mut journal, "shroud", "main", false);
        // Already brought back by the re-joins.
        record(&mut journal, "Pokimane", "main", true);
        // Another window's claim.
        record(&mut journal, "caseoh_", "overlay-caseoh_", true);

        let noted = journal.as_mut().and_then(|j| j.remove("main"));
        let brought_back = vec!["pokimane".to_string()];
        // Claimed after the fresh start: still held, not yet in the list.
        let held_now = vec!["pokimane".to_string(), "lirik".to_string()];
        assert_eq!(late_rooms(&brought_back, held_now, noted), vec!["lirik".to_string(), "xqc".to_string()]);
        assert!(journal.is_some_and(|j| j.contains_key("overlay-caseoh_")));
    }
}
