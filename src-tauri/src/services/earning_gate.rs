//! The earning gate: whether a player counts as watching, for the Twitch
//! minute-watched heartbeat (drops and channel points) and StreamNook watch
//! time. Every player (desktop, the phones' main player, a MultiNook tile)
//! reports what its video does and this decides what that means.
//!
//! The heartbeat samples the flag once a minute, so a short false costs a
//! whole minute. The rules:
//! - `playing` counts at once and cancels a pending pause.
//! - `paused` counts only once it outlives a 6 s grace: a restart, a quality
//!   change and an ad pivot reload the source with a pause right before the
//!   next play, and those must not cost earning.
//! - A rebuffer is not a pause, so players never report one.
//! - `paused_now`: a pause the player cannot wait out (the page is already
//!   hidden and its timers stop) counts at once.
//! - `hidden`: the app went to the background; a pending pause counts at once.
//! - `teardown`: the player went away; a pending pause is dropped and nothing
//!   is reported, since a late pause would gate the NEXT stream off. A real
//!   stop clears the heartbeat with drops monitoring.

use crate::models::settings::AppState;
use crate::rt::AppHandle;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;
use tauri::Manager;

const PAUSE_GRACE: Duration = Duration::from_secs(6);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerState {
    Playing,
    Paused,
    PausedNow,
    Hidden,
    Teardown,
}

impl PlayerState {
    pub fn parse(state: &str) -> Option<Self> {
        Some(match state {
            "playing" => Self::Playing,
            "paused" => Self::Paused,
            "paused_now" => Self::PausedNow,
            "hidden" => Self::Hidden,
            "teardown" => Self::Teardown,
            _ => return None,
        })
    }
}

/// What the gate does now for a report: count playing, count paused, or
/// wait out the grace (the pending pause carries a generation so a newer
/// report cancels it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Apply(bool),
    ArmPause(u64),
    Nothing,
}

#[derive(Default)]
struct Gate {
    generation: u64,
    /// The generation of the pause waiting out its grace, per player.
    pending: HashMap<String, u64>,
}

impl Gate {
    fn step(&mut self, player: &str, state: PlayerState) -> Step {
        self.generation += 1;
        let pending = self.pending.remove(player).is_some();
        match state {
            PlayerState::Playing => Step::Apply(true),
            PlayerState::Paused => {
                self.pending.insert(player.to_string(), self.generation);
                Step::ArmPause(self.generation)
            }
            PlayerState::PausedNow => Step::Apply(false),
            PlayerState::Hidden if pending => Step::Apply(false),
            PlayerState::Hidden | PlayerState::Teardown => Step::Nothing,
        }
    }

    /// The grace ran out: the pause counts if nothing newer came in.
    fn expire(&mut self, player: &str, generation: u64) -> bool {
        if self.pending.get(player) == Some(&generation) {
            self.pending.remove(player);
            true
        } else {
            false
        }
    }
}

static GATE: Mutex<Option<Gate>> = Mutex::new(None);

fn with_gate<R>(f: impl FnOnce(&mut Gate) -> R) -> R {
    let mut guard = GATE.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(Gate::default))
}

fn key(slot: &Option<String>) -> String {
    slot.clone().unwrap_or_default()
}

/// A player's state. `slot` names a MultiNook tile; none is the main player.
pub fn report(app: &AppHandle, slot: Option<String>, state: PlayerState) {
    let player = key(&slot);
    match with_gate(|g| g.step(&player, state)) {
        Step::Apply(playing) => apply(app, &slot, playing),
        Step::ArmPause(generation) => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(PAUSE_GRACE).await;
                if with_gate(|g| g.expire(&player, generation)) {
                    log::info!("[EarningGate] pause outlived the grace ({})", if player.is_empty() { "main" } else { &player });
                    apply(&app, &slot, false);
                }
            });
        }
        Step::Nothing => {}
    }
}

/// The main player feeds the Twitch heartbeat and StreamNook watch time; a
/// tile counts toward watch time only.
fn apply(app: &AppHandle, slot: &Option<String>, playing: bool) {
    let state = app.state::<AppState>();
    match slot {
        Some(slot) => {
            crate::services::watch_minutes::set_playing(&crate::services::watch_minutes::tile_key(slot), playing);
            // MultiNook (and its tiles) exists on desktop only.
            #[cfg(desktop)]
            if playing {
                crate::commands::multi_nook::refresh_tile_watch(&state, slot);
            }
        }
        None => {
            state.watch_heartbeat.set_playing(playing);
            crate::services::watch_minutes::set_playing(crate::services::watch_minutes::MAIN, playing);
            crate::services::live_bookmark::set_playing(playing);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playing_counts_at_once_and_a_pause_waits_out_the_grace() {
        let mut g = Gate::default();
        assert_eq!(g.step("", PlayerState::Playing), Step::Apply(true));
        let Step::ArmPause(generation) = g.step("", PlayerState::Paused) else { panic!("pause must arm") };
        assert!(g.expire("", generation));
    }

    #[test]
    fn a_play_inside_the_grace_cancels_the_pause() {
        let mut g = Gate::default();
        let Step::ArmPause(generation) = g.step("", PlayerState::Paused) else { panic!() };
        assert_eq!(g.step("", PlayerState::Playing), Step::Apply(true));
        assert!(!g.expire("", generation), "a source reload must not cost earning");
    }

    #[test]
    fn hiding_flushes_a_pending_pause_and_nothing_else() {
        let mut g = Gate::default();
        assert_eq!(g.step("", PlayerState::Hidden), Step::Nothing);
        let Step::ArmPause(generation) = g.step("", PlayerState::Paused) else { panic!() };
        assert_eq!(g.step("", PlayerState::Hidden), Step::Apply(false));
        assert!(!g.expire("", generation), "the flushed pause is not counted twice");
    }

    #[test]
    fn teardown_drops_a_pending_pause_without_reporting() {
        let mut g = Gate::default();
        let Step::ArmPause(generation) = g.step("", PlayerState::Paused) else { panic!() };
        assert_eq!(g.step("", PlayerState::Teardown), Step::Nothing);
        assert!(!g.expire("", generation), "a late pause would gate the next stream off");
    }

    #[test]
    fn tiles_and_the_main_player_keep_their_own_pauses() {
        let mut g = Gate::default();
        let Step::ArmPause(main) = g.step("", PlayerState::Paused) else { panic!() };
        assert_eq!(g.step("tile-1", PlayerState::Playing), Step::Apply(true));
        assert!(g.expire("", main));
    }

    #[test]
    fn states_parse_from_their_wire_names() {
        assert_eq!(PlayerState::parse("paused_now"), Some(PlayerState::PausedNow));
        assert_eq!(PlayerState::parse("waiting"), None);
    }
}
