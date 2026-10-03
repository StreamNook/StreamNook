//! The bonus chest on channels the viewer is watching.
//!
//! Twitch offers a chest every few minutes on a channel you watch. Two things
//! notice one: the channel-points socket pushes `claim-available` the moment
//! it appears, and the channel-state poll reads `availableClaim` once a minute
//! as a fallback. Both hand it to `offer`, which collects it when the viewer
//! has "Auto-claim bonus chests" on and the channel is one they are actively
//! watching (the solo stream or a MultiNook tile, registered through
//! `register_active_channel`). Collecting chests on channels nobody watches is
//! the Autopilot plugin's job, never this.
//!
//! This lives here rather than in a chat component because whether a chest is
//! collected must not depend on which chat happens to be on screen.
//!
//! Every claim, automatic or a click on the chest button, goes through `claim`,
//! which reports it twice: `channel-points-earned` (deduped against the
//! socket's own push) for the balance surfaces and notifications, and
//! `watched-chest-claimed` for the "+N" on the points button and the profile
//! stat.

use std::collections::{HashSet, VecDeque};
use std::sync::Mutex;

use log::{debug, warn};
use once_cell::sync::Lazy;
use serde::Serialize;
use tauri::{Emitter, Manager};

use crate::models::drops::BonusClaimResult;
use crate::models::settings::AppState;
use crate::rt::AppHandle;
use crate::services::channel_points_websocket_service::claim_emit_is_first;

pub const CLAIMED_EVENT: &str = "watched-chest-claimed";

/// Claim ids already taken or in flight. Both detection paths report the same
/// chest, so the second must not claim it again. Bounded: a chest id is never
/// offered again once the next one exists.
const REMEMBERED: usize = 64;
static TAKEN: Lazy<Mutex<(HashSet<String>, VecDeque<String>)>> =
    Lazy::new(|| Mutex::new((HashSet::new(), VecDeque::new())));
/// Claims on their way to Twitch. The phone's points poll and this module can
/// reach one chest at once; the second caller is turned away.
static IN_FLIGHT: Lazy<Mutex<HashSet<String>>> = Lazy::new(|| Mutex::new(HashSet::new()));

#[derive(Serialize, Clone)]
struct Claimed {
    channel_id: String,
    points_earned: i32,
    new_balance: i32,
}

/// First sight of `claim_id`: records it and returns true. False when it was
/// already taken.
fn take(claim_id: &str) -> bool {
    let mut guard = TAKEN.lock().unwrap_or_else(|e| e.into_inner());
    let (seen, order) = &mut *guard;
    take_in(seen, order, claim_id)
}

fn take_in(seen: &mut HashSet<String>, order: &mut VecDeque<String>, claim_id: &str) -> bool {
    if !seen.insert(claim_id.to_string()) {
        return false;
    }
    order.push_back(claim_id.to_string());
    while order.len() > REMEMBERED {
        if let Some(old) = order.pop_front() {
            seen.remove(&old);
        }
    }
    true
}

/// A chest is available on `channel_id`. Collects it when the viewer has
/// auto-claim on and is watching that channel; otherwise the chest button in
/// chat offers it.
pub fn offer(app: &AppHandle, channel_id: String, claim_id: String) {
    if channel_id.is_empty() || claim_id.is_empty() {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        let auto = state
            .settings
            .lock()
            .ok()
            .and_then(|s| s.extra.get("auto_claim_points_watching").and_then(|v| v.as_bool()))
            .unwrap_or(true);
        if !auto {
            return;
        }
        let ws = state.background_service.lock().await.websocket_service.clone();
        let (watched, login) = {
            let ws = ws.lock().await;
            (ws.is_active_channel(&channel_id).await, ws.get_channel_login(&channel_id).await)
        };
        if !watched || !take(&claim_id) {
            return;
        }
        let name = login.unwrap_or_default();
        match claim(&app, &channel_id, &name, &claim_id).await {
            Ok(r) => debug!("[WatchedChest] collected {} on {channel_id}", r.points_earned),
            Err(e) => warn!("[WatchedChest] claim on {channel_id} failed: {e}"),
        }
    });
}

/// Claim a chest and report it. The one path for automatic claims and a click
/// on the chest button.
pub async fn claim(
    app: &AppHandle,
    channel_id: &str,
    channel_name: &str,
    claim_id: &str,
) -> Result<BonusClaimResult, String> {
    // Recorded so a later report of this chest is not offered again.
    take(claim_id);
    if !IN_FLIGHT.lock().unwrap_or_else(|e| e.into_inner()).insert(claim_id.to_string()) {
        return Err("this chest is already being claimed".into());
    }
    let state = app.state::<AppState>();
    let result = {
        let drops_service = state.drops_service.lock().await;
        drops_service
            .claim_channel_points(channel_id, channel_name, claim_id)
            .await
            .map_err(|e| e.to_string())
    };
    IN_FLIGHT.lock().unwrap_or_else(|e| e.into_inner()).remove(claim_id);
    let result = result?;

    // Twitch also pushes this claim as a points-earned (reason CLAIM) on the
    // socket and either can land first; whichever is second is dropped. This
    // emit keeps the claim reported while the socket is down.
    if result.points_earned > 0 && claim_emit_is_first(channel_id) {
        let _ = app.emit(
            "channel-points-earned",
            serde_json::json!({
                "channel_id": channel_id,
                "channel_login": channel_name,
                "channel_display_name": channel_name,
                "points": result.points_earned,
                "reason": "claim",
                "balance": result.new_balance,
            }),
        );
    }
    let _ = app.emit(
        CLAIMED_EVENT,
        Claimed {
            channel_id: channel_id.to_string(),
            points_earned: result.points_earned,
            new_balance: result.new_balance,
        },
    );
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chest_is_taken_once() {
        let (mut seen, mut order) = (HashSet::new(), VecDeque::new());
        assert!(take_in(&mut seen, &mut order, "c1"));
        assert!(!take_in(&mut seen, &mut order, "c1"), "the poll's report of the same chest is skipped");
        assert!(take_in(&mut seen, &mut order, "c2"));
    }

    #[test]
    fn the_memory_stays_bounded() {
        let (mut seen, mut order) = (HashSet::new(), VecDeque::new());
        for i in 0..(REMEMBERED + 10) {
            assert!(take_in(&mut seen, &mut order, &format!("c{i}")));
        }
        assert_eq!(seen.len(), REMEMBERED);
        assert_eq!(order.len(), REMEMBERED);
        assert!(take_in(&mut seen, &mut order, "c0"), "the oldest id was forgotten");
    }
}
