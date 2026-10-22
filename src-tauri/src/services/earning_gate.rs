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
//!
//! On the phones the main player's pause also parks the stream: 30 s into a
//! pause the low-latency relay stops downloading (ll_origin::park), and a play
//! or a loading player's playlist request starts it again at the live edge.

use crate::models::settings::AppState;
use crate::rt::AppHandle;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;
use tauri::Manager;

const PAUSE_GRACE: Duration = Duration::from_secs(6);
/// A phone's main player paused this long stops downloading (the shells keep the
/// same mark for jumping back to live: pausePolicy.ts, SNKit `PausePark`).
pub const PARK_AFTER: Duration = Duration::from_secs(30);

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
    /// The phones' main player only.
    park: ParkGate,
}

/// Where the main player's stream stands on parking: downloading, a pause
/// counting down to the park (its generation), or parked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Park {
    #[default]
    Active,
    Pending(u64),
    Parked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParkStep {
    Arm(u64),
    Park,
    Unpark,
    Rewake(u64),
    Clear,
    /// A new stream or a stop: forget the park (the origin is being stopped anyway).
    Forget,
    Nothing,
}

#[derive(Debug, Default)]
struct ParkGate {
    generation: u64,
    state: Park,
}

impl ParkGate {
    fn arm(&mut self) -> u64 {
        self.generation += 1;
        self.state = Park::Pending(self.generation);
        self.generation
    }

    fn step(&mut self, s: PlayerState) -> ParkStep {
        match (s, self.state) {
            (PlayerState::Playing, Park::Parked) => {
                self.state = Park::Active;
                ParkStep::Unpark
            }
            (PlayerState::Playing, _) => {
                self.state = Park::Active;
                ParkStep::Nothing
            }
            (PlayerState::Paused | PlayerState::PausedNow, Park::Active) => ParkStep::Arm(self.arm()),
            (PlayerState::Teardown, Park::Parked) => {
                self.state = Park::Active;
                ParkStep::Clear
            }
            (PlayerState::Teardown, _) => {
                self.state = Park::Active;
                ParkStep::Nothing
            }
            // Hidden; a repeat pause keeps its first deadline.
            _ => ParkStep::Nothing,
        }
    }

    fn expire(&mut self, generation: u64) -> ParkStep {
        if self.state == Park::Pending(generation) {
            self.state = Park::Parked;
            ParkStep::Park
        } else {
            ParkStep::Nothing
        }
    }

    fn relay_wake(&mut self) -> ParkStep {
        if self.state == Park::Parked { ParkStep::Rewake(self.arm()) } else { ParkStep::Nothing }
    }

    fn reset(&mut self) -> ParkStep {
        let was = self.state == Park::Parked;
        self.generation += 1;
        self.state = Park::Active;
        if was { ParkStep::Forget } else { ParkStep::Nothing }
    }
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
    let (step, park) = with_gate(|g| {
        let step = g.step(&player, state);
        // Phones, main player only: desktop playback is unchanged.
        let park = if cfg!(mobile) && slot.is_none() { g.park.step(state) } else { ParkStep::Nothing };
        (step, park)
    });
    apply_park(park);
    match step {
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

/// A player asked the relay for a parked origin's playlist (stream_server).
pub fn relay_wake() {
    apply_park(with_gate(|g| g.park.relay_wake()));
}

/// A new upstream or a stop (stream_server::reset_ad_state): no park carries over.
pub fn reset_main_park() {
    apply_park(with_gate(|g| g.park.reset()));
}

fn apply_park(step: ParkStep) {
    use crate::services::{app_presence, ll_origin};
    match step {
        ParkStep::Arm(generation) => arm_park_timer(generation),
        // The flag says what really happened: a VOD, a non-LL relay or an
        // inactive origin parks nothing, and the shells must not jump to live.
        ParkStep::Park => app_presence::set_main_parked(ll_origin::park()),
        ParkStep::Unpark => {
            ll_origin::unpark();
            app_presence::set_main_parked(false);
        }
        ParkStep::Rewake(generation) => {
            ll_origin::unpark();
            app_presence::set_main_parked(false);
            arm_park_timer(generation);
        }
        // Teardown while parked: a player rebuilt on the same origin (a swap
        // that skips stop_stream) must find it downloading, never stuck parked.
        ParkStep::Clear => {
            ll_origin::unpark();
            app_presence::set_main_parked(false);
        }
        // reset_ad_state stops the origin right after; restarting its reader
        // first would only start a download to abort.
        ParkStep::Forget => app_presence::set_main_parked(false),
        ParkStep::Nothing => {}
    }
}

fn arm_park_timer(generation: u64) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(PARK_AFTER).await;
        let step = with_gate(|g| g.park.expire(generation));
        if step == ParkStep::Park {
            log::info!("[EarningGate] paused {}s; the stream stops downloading", PARK_AFTER.as_secs());
        }
        apply_park(step);
    });
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
    fn a_pause_parks_after_the_window_and_play_unparks() {
        let mut p = ParkGate::default();
        let ParkStep::Arm(generation) = p.step(PlayerState::Paused) else { panic!("a pause arms the park") };
        assert_eq!(p.expire(generation), ParkStep::Park);
        assert_eq!(p.step(PlayerState::Playing), ParkStep::Unpark);
    }

    #[test]
    fn a_play_inside_the_window_cancels_the_park() {
        let mut p = ParkGate::default();
        let ParkStep::Arm(generation) = p.step(PlayerState::Paused) else { panic!() };
        assert_eq!(p.step(PlayerState::Playing), ParkStep::Nothing);
        assert_eq!(p.expire(generation), ParkStep::Nothing);
    }

    #[test]
    fn a_repeat_pause_keeps_the_first_deadline() {
        let mut p = ParkGate::default();
        let ParkStep::Arm(generation) = p.step(PlayerState::Paused) else { panic!() };
        assert_eq!(p.step(PlayerState::PausedNow), ParkStep::Nothing);
        assert_eq!(p.expire(generation), ParkStep::Park);
    }

    #[test]
    fn a_relay_wake_downloads_again_and_rearms() {
        let mut p = ParkGate::default();
        let ParkStep::Arm(generation) = p.step(PlayerState::Paused) else { panic!() };
        assert_eq!(p.expire(generation), ParkStep::Park);
        let ParkStep::Rewake(again) = p.relay_wake() else { panic!("a parked wake rearms") };
        assert_eq!(p.expire(again), ParkStep::Park);
        assert_eq!(p.relay_wake(), ParkStep::Rewake(again + 1));
    }

    #[test]
    fn teardown_clears_and_reset_forgets_the_park() {
        let mut p = ParkGate::default();
        let ParkStep::Arm(generation) = p.step(PlayerState::Paused) else { panic!() };
        assert_eq!(p.expire(generation), ParkStep::Park);
        assert_eq!(p.step(PlayerState::Teardown), ParkStep::Clear);
        assert_eq!(p.relay_wake(), ParkStep::Nothing);
        let ParkStep::Arm(generation) = p.step(PlayerState::Paused) else { panic!() };
        assert_eq!(p.expire(generation), ParkStep::Park);
        assert_eq!(p.reset(), ParkStep::Forget);
        assert_eq!(p.reset(), ParkStep::Nothing);
    }

    #[test]
    fn hidden_never_parks_or_unparks() {
        let mut p = ParkGate::default();
        assert_eq!(p.step(PlayerState::Hidden), ParkStep::Nothing);
        let ParkStep::Arm(generation) = p.step(PlayerState::Paused) else { panic!() };
        assert_eq!(p.step(PlayerState::Hidden), ParkStep::Nothing);
        assert_eq!(p.expire(generation), ParkStep::Park);
        assert_eq!(p.step(PlayerState::Hidden), ParkStep::Nothing);
    }

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
