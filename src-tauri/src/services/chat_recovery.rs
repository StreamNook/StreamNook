//! Recovery for a Twitch chat connection that has died for good.
//!
//! The IRC task can end permanently while the local bridge stays up: a failed
//! connect (a phone thawing before the network is back, a Wi-Fi to mobile
//! handoff) ends it outright, and an expired token ends it too, because the
//! token is read once when the task starts. Afterwards `start_chat` sees a
//! handle that looks present and every soft reconnect reports success while
//! nothing arrives; a PART and re-JOIN has no connection to ride. Only a stop
//! clears that state, so a rebuild stops the Twitch side and brings it back
//! with every room the window held, reading the token afresh.
//!
//! Automatic triggers overlap (resuming the app and the stale watchdog fire
//! for the same dead connection, and every open room sees it), so one cycle
//! settles them all: never two at once, and automatic ones at most once a
//! minute. A manual reload is never throttled.

use crate::models::settings::AppState;
use crate::services::chat_service::ChatService;
use crate::services::irc_service::IrcService;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Emitted with [`Rebuilt`] after a cycle, so a shell reading the bridge
/// reconnects to its port and backfills the gap.
pub const EVENT: &str = "chat-rebuilt";

const COOLDOWN: Duration = Duration::from_secs(60);

static CYCLING: AtomicBool = AtomicBool::new(false);
static LAST_CYCLE: Mutex<Option<Instant>> = Mutex::new(None);

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

/// Releases the single-flight flag however the cycle ends.
struct Flight;

impl Drop for Flight {
    fn drop(&mut self) {
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
    let channels = IrcService::channels_held_by(window).await;
    if channels.is_empty() {
        return Ok(None);
    }
    if CYCLING.swap(true, Ordering::SeqCst) {
        return Ok(None);
    }
    let _flight = Flight;
    *LAST_CYCLE.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
    // Warn, not debug: debug logging is off on a phone and this is what a
    // support log needs to show.
    log::warn!("[ChatRecovery] cycling {} room(s) for {}: {}", channels.len(), window, reason);
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
}
