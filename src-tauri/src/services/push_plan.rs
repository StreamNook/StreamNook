//! The phone's go-live push registration as a pure state machine.
//!
//! `push_alerts::server` does the I/O: it reads the device, the settings and
//! the follows, posts to the notify server and keeps this state on disk
//! (`push_state.json`). Everything that decides (is a send owed, may it go
//! now, what each answer means, when to try again) lives here, as functions
//! of the state and a clock reading in milliseconds, so every rule is tested
//! with synthetic instants on desktop.
//!
//! The intent is two counters: `wanted` moves on every change that alters
//! what the server should hold, and `sent` is the `wanted` a successful send
//! read when it started. A send is owed while `wanted > sent`, so a send that
//! started before a newer change can never mark that change done.

use crate::services::push_alerts::PushDevice;
use serde::{Deserialize, Serialize};

/// The server keeps at most this many channel ids per device.
pub const MAX_IDS: usize = 1000;
/// A switch change settles this long before it is sent.
pub const SWITCH_DEBOUNCE_MS: u64 = 3_000;
/// A mute-list change settles longer, so a run of bell taps is one send.
pub const MUTE_DEBOUNCE_MS: u64 = 20_000;
/// However the taps keep coming, a change goes this long after the first.
pub const DEBOUNCE_CEILING_MS: u64 = 60_000;
/// A change made while a send ran goes this soon after it.
pub const RUN_AGAIN_MS: u64 = 3_000;
/// A launch or a worker run re-sends at least this often with nothing new,
/// which heals a device row the server evicted or pruned.
pub const RESEND_AFTER_MS: u64 = 6 * 60 * 60 * 1000;
/// The iPhone reports its device on every launch and return; the same device
/// sent this recently, with nothing owed, is not sent again.
pub const APNS_REPEAT_MS: u64 = 10 * 60 * 1000;
/// Waits after a network failure, a 5xx or an unexpected 4xx, by attempt.
pub const BACKOFF_MS: [u64; 4] = [30_000, 120_000, 600_000, 1_800_000];
/// A 429 without `Retry-After` waits this long.
pub const RATE_LIMIT_DEFAULT_MS: u64 = 600_000;
/// A 429 never waits longer than this, whatever it says.
pub const RATE_LIMIT_CAP_MS: u64 = 3_600_000;
/// A 400 parks the phone this long, or until the app or the device changes.
pub const REJECTED_WAIT_MS: u64 = 24 * 60 * 60 * 1000;
/// One flush sends at most this many times in a row.
pub const MAX_RUNS: u32 = 3;

/// What kind of change a settings save made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A switch, a follow, a sign-in: settles 3 s.
    Switch,
    /// Only the muted channels moved: settles 20 s.
    MuteOnly,
}

/// What the last accepted send told the server.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SentDecision {
    Register,
    Unregister,
}

/// Why sending stopped until something changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParkKind {
    /// No Twitch token at all: waits for a sign-in.
    SignedOut,
    /// The server (or Twitch's refresh) refused this token: waits for a
    /// different token.
    TokenRejected,
    /// The server answered 400: waits a day, a new app version or a new device.
    Rejected,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Park {
    pub kind: ParkKind,
    /// The refused token's fingerprint (`TokenRejected`), never the token.
    #[serde(default)]
    pub token_hash: Option<String>,
    pub at_ms: u64,
    #[serde(default)]
    pub app_version: String,
    #[serde(default)]
    pub device: Option<PushDevice>,
}

/// A device a sign-out could not take off the server, to delete later by its
/// own device token.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingUnregister {
    pub device: PushDevice,
    /// The `unregister_seq` it was set at: only a register whose pass started
    /// at or after it may clear it.
    pub set_at: u64,
}

/// Everything the phone remembers about its registration, on disk.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PushState {
    pub wanted: u64,
    pub sent: u64,
    /// The ids the server accepted last (at most `MAX_IDS`, in send order).
    pub sent_ids: Vec<String>,
    pub sent_user_id: Option<String>,
    pub sent_device: Option<PushDevice>,
    pub sent_decision: Option<SentDecision>,
    pub sent_at_ms: u64,
    /// When the oldest change still owed was made (the 60 s ceiling's start).
    pub first_pending_ms: Option<u64>,
    /// When the next pass is due; a timer whose due time moved stands down.
    pub due_ms: Option<u64>,
    /// The server asked for no paid send before this (a 429).
    pub not_before_ms: u64,
    pub park: Option<Park>,
    pub unregister_seq: u64,
    pub pending_unregister: Option<PendingUnregister>,
    /// Failed sends in a row, for the backoff.
    pub attempt: u32,
}

impl Default for PushState {
    /// A phone with no state on record (a first run, a damaged file) owes one
    /// send, so it registers rather than trusting nothing.
    fn default() -> Self {
        Self {
            wanted: 1,
            sent: 0,
            sent_ids: Vec::new(),
            sent_user_id: None,
            sent_device: None,
            sent_decision: None,
            sent_at_ms: 0,
            first_pending_ms: None,
            due_ms: None,
            not_before_ms: 0,
            park: None,
            unregister_seq: 0,
            pending_unregister: None,
            attempt: 0,
        }
    }
}

/// The state from its file's text: missing or unreadable owes one send.
pub fn load(text: Option<&str>) -> PushState {
    text.and_then(|t| serde_json::from_str(t).ok()).unwrap_or_default()
}

/// What a pass read when it started.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub wanted: u64,
    pub unregister_seq: u64,
}

pub fn snapshot(state: &PushState) -> Snapshot {
    Snapshot { wanted: state.wanted, unregister_seq: state.unregister_seq }
}

/// What a pass leads to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Nothing owed.
    Done,
    /// Sent, and a change came in meanwhile: send again at this time.
    RunAgain(u64),
    /// Try again at this time.
    Wait(u64),
    /// Stopped until something changes.
    Parked,
}

/// Record a change and answer when it is due.
pub fn schedule_change(state: &mut PushState, kind: Kind, now: u64) -> u64 {
    state.wanted += 1;
    let first = *state.first_pending_ms.get_or_insert(now);
    let settle = match kind {
        Kind::Switch => SWITCH_DEBOUNCE_MS,
        Kind::MuteOnly => MUTE_DEBOUNCE_MS,
    };
    let due = (now + settle).min(first + DEBOUNCE_CEILING_MS).max(now);
    state.due_ms = Some(due);
    due
}

/// Whether the park, if any, still holds. Token parks are released by
/// `release_parks` when the token changes; a 400 park by time, a new app
/// version or a new device.
pub fn park_holds(state: &PushState, now: u64, app_version: &str, device: Option<&PushDevice>) -> bool {
    match &state.park {
        None => false,
        Some(p) => match p.kind {
            ParkKind::SignedOut | ParkKind::TokenRejected => true,
            ParkKind::Rejected => {
                now < p.at_ms + REJECTED_WAIT_MS && p.app_version == app_version && p.device.as_ref() == device
            }
        },
    }
}

/// Whether a pass has something to do.
pub fn is_pending(state: &PushState, now: u64, app_version: &str, device: Option<&PushDevice>) -> bool {
    state.pending_unregister.is_some() || (state.wanted > state.sent && !park_holds(state, now, app_version, device))
}

/// Whether a launch, `push_register` or the Android worker posts even with
/// nothing owed: the device changed (an FCM rotation) or the last send is
/// older than `RESEND_AFTER_MS`. Never while a park holds (a pending delete
/// still goes).
pub fn should_post(state: &PushState, now: u64, app_version: &str, device: Option<&PushDevice>) -> bool {
    if park_holds(state, now, app_version, device) {
        return state.pending_unregister.is_some();
    }
    if is_pending(state, now, app_version, device) {
        return true;
    }
    let Some(device) = device else { return false };
    state.sent_device.as_ref() != Some(device) || now.saturating_sub(state.sent_at_ms) >= RESEND_AFTER_MS
}

/// The iPhone's repeat device report: skipped while that device was sent in
/// the last `APNS_REPEAT_MS` and nothing is owed.
pub fn apns_repeat_skip(state: &PushState, device: &PushDevice, now: u64, app_version: &str) -> bool {
    !is_pending(state, now, app_version, Some(device))
        && state.sent_device.as_ref() == Some(device)
        && now.saturating_sub(state.sent_at_ms) < APNS_REPEAT_MS
}

/// Why a flush may post its first pass with nothing owed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PostReason {
    /// Only what is owed (a debounce, a background flush, a wake).
    Owed,
    /// Also when the device changed or the last send is six hours old (a
    /// launch, the Android worker).
    Due,
    /// The iPhone's device report: also once the same device went out more
    /// than `APNS_REPEAT_MS` ago.
    DeviceReport,
}

/// Whether the first pass posts even with nothing owed, decided under the
/// runner (a pass that ran while this one waited may have made it moot).
pub fn first_pass_posts(
    state: &PushState,
    reason: PostReason,
    now: u64,
    app_version: &str,
    device: Option<&PushDevice>,
) -> bool {
    match reason {
        PostReason::Owed => false,
        PostReason::Due => should_post(state, now, app_version, device),
        PostReason::DeviceReport => device.is_some_and(|d| !apns_repeat_skip(state, d, now, app_version)),
    }
}

/// A launch: every park gets one fresh try, and the backoff starts over.
pub fn on_launch(state: &mut PushState) {
    state.park = None;
    state.attempt = 0;
}

/// The primary token changed (`token_hash` is the new one, `None` when it was
/// deleted). Releases a park that waited for exactly this; answers whether it
/// did. A `TokenRejected` park with no hash (a refused refresh) is released by
/// any token write or deletion.
pub fn release_parks(state: &mut PushState, token_hash: Option<&str>) -> bool {
    let release = match &state.park {
        Some(p) if p.kind == ParkKind::SignedOut => token_hash.is_some(),
        Some(p) if p.kind == ParkKind::TokenRejected => {
            p.token_hash.is_none() || token_hash != p.token_hash.as_deref()
        }
        _ => false,
    };
    if release {
        state.park = None;
        state.attempt = 0;
    }
    release
}

/// Leave `device` to be deleted later by its own token. Answers its `set_at`.
pub fn begin_unregister(state: &mut PushState, device: PushDevice) -> u64 {
    state.unregister_seq += 1;
    state.pending_unregister = Some(PendingUnregister { device, set_at: state.unregister_seq });
    state.unregister_seq
}

/// The leftover device was deleted: clear it if it is still the one set at
/// `set_at`.
pub fn unregister_done(state: &mut PushState, set_at: u64) {
    if state.pending_unregister.as_ref().is_some_and(|p| p.set_at == set_at) {
        state.pending_unregister = None;
    }
}

/// A sign-out took `device` off the server with the leaving account's token.
/// Whatever was owed belonged to that account.
pub fn note_unregistered(state: &mut PushState, device: &PushDevice, now: u64) {
    state.sent = state.wanted;
    state.sent_ids.clear();
    state.sent_user_id = None;
    state.sent_device = Some(device.clone());
    state.sent_decision = Some(SentDecision::Unregister);
    state.sent_at_ms = now;
    state.first_pending_ms = None;
    state.due_ms = None;
    state.attempt = 0;
    state.pending_unregister = None;
}

/// Whether a register of `ids` only removes channels from what the server
/// holds for this same user and device (free on the server, and allowed
/// inside a 429 window).
pub fn is_subset(state: &PushState, user_id: &str, device: &PushDevice, ids: &[String]) -> bool {
    state.sent_decision == Some(SentDecision::Register)
        && state.sent_user_id.as_deref() == Some(user_id)
        && state.sent_device.as_ref() == Some(device)
        && ids.iter().all(|id| state.sent_ids.iter().any(|s| s == id))
}

/// Whether a send may go now: a subset always, a paid one after the 429 window.
pub fn may_post(state: &PushState, now: u64, subset: bool) -> bool {
    subset || now >= state.not_before_ms
}

/// Whether a flush that just sent (with a change made meanwhile) sends again.
pub fn runs_again(runs_so_far: u32) -> bool {
    runs_so_far < MAX_RUNS
}

/// What a pass does with a device a sign-out could not take off the server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeftoverAction {
    /// This pass registers that same device for a signed-in account: the
    /// server's register replaces the row, and only its success clears it.
    LeaveToRegister,
    /// The token is too short for a device-only delete (the server refuses
    /// it): never sent, and no longer owed.
    Forget,
    /// Post the device-only delete now.
    Delete,
}

/// The leftover's action. `registers`: the settings say the server should hold
/// this device; `signed_in`: a token was read; `holds`: a park holds;
/// `paid_open`: a paid send may go now (outside a 429 window). Inside a 429
/// window the new account's register cannot go, so the old account's row is
/// deleted by its own token instead of pushing to this phone until it can.
pub fn leftover_action(
    pending_device: &PushDevice,
    device: Option<&PushDevice>,
    registers: bool,
    signed_in: bool,
    holds: bool,
    paid_open: bool,
) -> LeftoverAction {
    let replaced_here = registers && signed_in && !holds && paid_open && device == Some(pending_device);
    if replaced_here {
        LeftoverAction::LeaveToRegister
    } else if !device_only_allowed(pending_device) {
        LeftoverAction::Forget
    } else {
        LeftoverAction::Delete
    }
}

/// Whether a pass goes on to its register or drop after the leftover step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    /// Send what the settings say.
    Go,
    /// A park holds: stop here.
    Parked,
    /// Nothing owed and nothing forced: stop here.
    Current,
    /// Stopping here, but the leftover delete failed: back off and try it again.
    RetryLeftover,
}

pub fn pass_gate(holds: bool, owed: bool, force: bool, leftover_failed: bool) -> Gate {
    if !holds && (owed || force) {
        return Gate::Go;
    }
    if leftover_failed {
        return Gate::RetryLeftover;
    }
    if holds {
        Gate::Parked
    } else {
        Gate::Current
    }
}

/// After the pass's send: a failed leftover delete behind a step that would
/// leave nothing armed (parked or done) still needs its backoff.
pub fn leftover_retry(step: Step, leftover_failed: bool) -> bool {
    leftover_failed && matches!(step, Step::Parked | Step::Done)
}

/// What one accepted send was.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sent {
    pub decision: SentDecision,
    pub ids: Vec<String>,
    pub user_id: Option<String>,
    pub device: PushDevice,
}

/// The server accepted `sent`, which the pass read as `snap`.
pub fn apply_sent(state: &mut PushState, snap: &Snapshot, sent: Sent, now: u64) -> Step {
    state.sent = state.sent.max(snap.wanted);
    let same_device_pending = state
        .pending_unregister
        .as_ref()
        .is_some_and(|p| p.device == sent.device && snap.unregister_seq >= p.set_at);
    if same_device_pending {
        // The server's row for this device now holds what this send said.
        state.pending_unregister = None;
    }
    let mut ids = sent.ids;
    ids.truncate(MAX_IDS);
    state.sent_ids = ids;
    state.sent_user_id = sent.user_id;
    state.sent_device = Some(sent.device);
    state.sent_decision = Some(sent.decision);
    state.sent_at_ms = now;
    state.attempt = 0;
    state.not_before_ms = 0;
    state.park = None;
    if state.wanted > state.sent {
        let due = now + RUN_AGAIN_MS;
        state.due_ms = Some(due);
        return Step::RunAgain(due);
    }
    if state.pending_unregister.is_some() {
        // A leftover device (an older token) still to delete.
        let due = now + BACKOFF_MS[0];
        state.due_ms = Some(due);
        return Step::Wait(due);
    }
    state.first_pending_ms = None;
    state.due_ms = None;
    Step::Done
}

/// Why a pass did not send.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The phone has not reported a device yet; it registers when it does.
    NoDevice,
    SignedOut,
    TokenRejected { token_hash: Option<String> },
    Rejected,
    RateLimited { retry_after_ms: Option<u64> },
    Transient,
    /// A paid send inside the server's 429 window.
    Waiting,
}

/// Apply a pass that did not send.
pub fn apply_failure(
    state: &mut PushState,
    snap: &Snapshot,
    outcome: Outcome,
    now: u64,
    app_version: &str,
    device: Option<&PushDevice>,
) -> Step {
    match outcome {
        Outcome::NoDevice => {
            state.sent = state.sent.max(snap.wanted);
            if state.wanted > state.sent {
                let due = now + RUN_AGAIN_MS;
                state.due_ms = Some(due);
                return Step::Wait(due);
            }
            state.first_pending_ms = None;
            state.due_ms = None;
            Step::Done
        }
        Outcome::SignedOut | Outcome::TokenRejected { .. } | Outcome::Rejected => {
            if state.wanted != snap.wanted {
                // A change came in while this pass ran: it gets its own try.
                let due = now + RUN_AGAIN_MS;
                state.due_ms = Some(due);
                return Step::Wait(due);
            }
            let (kind, token_hash) = match outcome {
                Outcome::SignedOut => (ParkKind::SignedOut, None),
                Outcome::TokenRejected { token_hash } => (ParkKind::TokenRejected, token_hash),
                _ => (ParkKind::Rejected, None),
            };
            state.park = Some(Park {
                kind,
                token_hash,
                at_ms: now,
                app_version: app_version.to_string(),
                device: device.cloned(),
            });
            state.due_ms = None;
            Step::Parked
        }
        Outcome::RateLimited { retry_after_ms } => {
            let wait = retry_after_ms.unwrap_or(RATE_LIMIT_DEFAULT_MS).min(RATE_LIMIT_CAP_MS);
            state.not_before_ms = now + wait;
            state.attempt += 1;
            state.due_ms = Some(now + wait);
            Step::Wait(now + wait)
        }
        Outcome::Transient => {
            let wait = BACKOFF_MS[(state.attempt as usize).min(BACKOFF_MS.len() - 1)];
            state.attempt += 1;
            state.due_ms = Some(now + wait);
            Step::Wait(now + wait)
        }
        Outcome::Waiting => {
            let due = state.not_before_ms.max(now);
            state.due_ms = Some(due);
            Step::Wait(due)
        }
    }
}

/// The notify server's answer, by status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Ok,
    Unauthorized,
    Rejected,
    RateLimited(Option<u64>),
    Transient,
}

/// A status as an answer. An unregister that finds no device (403 not yours,
/// 404 gone) is done. Every other 4xx, 5xx and network error is `Transient`;
/// only 400 is `Rejected`.
pub fn classify_status(status: u16, retry_after_ms: Option<u64>, unregister: bool) -> Answer {
    match status {
        200..=299 => Answer::Ok,
        403 | 404 if unregister => Answer::Ok,
        401 => Answer::Unauthorized,
        400 => Answer::Rejected,
        429 => Answer::RateLimited(retry_after_ms),
        _ => Answer::Transient,
    }
}

/// A non-OK answer as an outcome. A 401 on an unregister is `Transient` (the
/// server before device-only unregister answers 401 without a user token);
/// on a register it is the refused token, after the one refresh.
pub fn outcome_of(answer: Answer, rejected_token_hash: Option<String>, unregister: bool) -> Outcome {
    match answer {
        Answer::Unauthorized if unregister => Outcome::Transient,
        Answer::Unauthorized => Outcome::TokenRejected { token_hash: rejected_token_hash },
        Answer::Rejected => Outcome::Rejected,
        Answer::RateLimited(retry_after_ms) => Outcome::RateLimited { retry_after_ms },
        Answer::Ok | Answer::Transient => Outcome::Transient,
    }
}

/// Why a token or Helix read failed, from `TwitchService`'s typed reason.
/// Anything without one is `Transient`.
pub fn classify_token_error(e: &anyhow::Error) -> Outcome {
    use crate::services::twitch_service::TokenError;
    for cause in e.chain() {
        if let Some(kind) = cause.downcast_ref::<TokenError>() {
            return match kind {
                TokenError::NoToken => Outcome::SignedOut,
                TokenError::Refused => Outcome::TokenRejected { token_hash: None },
                TokenError::VaultUnavailable | TokenError::Unreachable => Outcome::Transient,
            };
        }
    }
    Outcome::Transient
}

/// The server deletes a device by its own token only when that token is at
/// least this long (a shorter one is refused there, so it is never sent).
pub const DEVICE_ONLY_MIN_TOKEN: usize = 64;

/// Whether `device` may be deleted by its own token alone.
pub fn device_only_allowed(device: &PushDevice) -> bool {
    let token = match device {
        PushDevice::Fcm { token } | PushDevice::Apns { token, .. } => token,
    };
    token.trim().len() >= DEVICE_ONLY_MIN_TOKEN
}

/// `Retry-After` in whole seconds, as milliseconds.
pub fn parse_retry_after(value: Option<&str>) -> Option<u64> {
    value?.trim().parse::<u64>().ok().map(|s| s.saturating_mul(1000))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::twitch_service::TokenError;

    fn fcm(t: &str) -> PushDevice {
        PushDevice::Fcm { token: t.to_string() }
    }

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    /// A phone that registered `["1","2","3"]` for user "u" on device "a" at 0.
    fn registered() -> PushState {
        let mut s = PushState::default();
        let snap = snapshot(&s);
        apply_sent(
            &mut s,
            &snap,
            Sent { decision: SentDecision::Register, ids: ids(&["1", "2", "3"]), user_id: Some("u".into()), device: fcm("a") },
            0,
        );
        s
    }

    const V: &str = "8.8.20";

    #[test]
    fn a_damaged_or_missing_state_owes_one_send() {
        assert!(load(None).wanted > load(None).sent);
        assert_eq!(load(Some("{not json")), PushState::default());
        assert!(is_pending(&load(Some("")), 0, V, None));
        // An old file missing new fields keeps what it has.
        let old = load(Some(r#"{"wanted":4,"sent":4}"#));
        assert_eq!((old.wanted, old.sent), (4, 4));
        assert!(!is_pending(&old, 0, V, None));
    }

    #[test]
    fn a_switch_settles_3s_mutes_20s_and_nothing_waits_past_60s() {
        let mut s = registered();
        assert_eq!(schedule_change(&mut s, Kind::Switch, 1_000), 4_000);
        let mut m = registered();
        assert_eq!(schedule_change(&mut m, Kind::MuteOnly, 1_000), 21_000);
        // Taps every 15 s keep restarting the wait, up to the ceiling.
        assert_eq!(schedule_change(&mut m, Kind::MuteOnly, 16_000), 36_000);
        assert_eq!(schedule_change(&mut m, Kind::MuteOnly, 46_000), 61_000);
        assert_eq!(m.due_ms, Some(61_000));
        assert_eq!(m.wanted, 4);
    }

    #[test]
    fn a_send_that_started_before_a_change_never_marks_it_done() {
        let mut s = registered();
        schedule_change(&mut s, Kind::MuteOnly, 10);
        let snap = snapshot(&s);
        schedule_change(&mut s, Kind::MuteOnly, 20);
        let step = apply_sent(
            &mut s,
            &snap,
            Sent { decision: SentDecision::Register, ids: ids(&["1"]), user_id: Some("u".into()), device: fcm("a") },
            30,
        );
        assert_eq!(step, Step::RunAgain(30 + RUN_AGAIN_MS));
        assert_eq!(s.sent, snap.wanted);
        assert!(s.wanted > s.sent);
        assert!(is_pending(&s, 30, V, Some(&fcm("a"))));
    }

    #[test]
    fn a_park_is_never_written_over_a_newer_change() {
        let mut s = registered();
        schedule_change(&mut s, Kind::Switch, 0);
        let snap = snapshot(&s);
        schedule_change(&mut s, Kind::MuteOnly, 1);
        let step = apply_failure(&mut s, &snap, Outcome::SignedOut, 2, V, Some(&fcm("a")));
        assert_eq!(step, Step::Wait(2 + RUN_AGAIN_MS));
        assert!(s.park.is_none());
        let snap = snapshot(&s);
        assert_eq!(apply_failure(&mut s, &snap, Outcome::SignedOut, 3, V, Some(&fcm("a"))), Step::Parked);
        assert!(!is_pending(&s, 4, V, Some(&fcm("a"))));
    }

    #[test]
    fn token_parks_wait_for_a_different_token_and_a_refresh_with_the_same_one_keeps_them() {
        let mut s = registered();
        schedule_change(&mut s, Kind::Switch, 0);
        let snap = snapshot(&s);
        apply_failure(&mut s, &snap, Outcome::TokenRejected { token_hash: Some("h1".into()) }, 1, V, Some(&fcm("a")));
        assert!(!release_parks(&mut s, Some("h1")), "the same token still refused");
        assert!(park_holds(&s, 2, V, Some(&fcm("a"))));
        assert!(release_parks(&mut s, Some("h2")));
        assert!(s.park.is_none());
        assert!(is_pending(&s, 3, V, Some(&fcm("a"))));

        let snap = snapshot(&s);
        apply_failure(&mut s, &snap, Outcome::SignedOut, 4, V, Some(&fcm("a")));
        assert!(!release_parks(&mut s, None), "still no token");
        assert!(release_parks(&mut s, Some("h3")), "a sign-in releases it");
    }

    #[test]
    fn a_rejected_send_waits_a_day_or_a_new_app_or_device() {
        let mut s = registered();
        schedule_change(&mut s, Kind::Switch, 0);
        let snap = snapshot(&s);
        apply_failure(&mut s, &snap, Outcome::Rejected, 100, V, Some(&fcm("a")));
        assert!(park_holds(&s, 100 + REJECTED_WAIT_MS - 1, V, Some(&fcm("a"))));
        assert!(!park_holds(&s, 100 + REJECTED_WAIT_MS, V, Some(&fcm("a"))));
        assert!(!park_holds(&s, 200, "8.8.21", Some(&fcm("a"))));
        assert!(!park_holds(&s, 200, V, Some(&fcm("b"))));
    }

    #[test]
    fn a_launch_clears_every_park() {
        for kind in [Outcome::SignedOut, Outcome::TokenRejected { token_hash: Some("h".into()) }, Outcome::Rejected] {
            let mut s = registered();
            schedule_change(&mut s, Kind::Switch, 0);
            let snap = snapshot(&s);
            apply_failure(&mut s, &snap, kind, 1, V, Some(&fcm("a")));
            assert!(!is_pending(&s, 2, V, Some(&fcm("a"))));
            on_launch(&mut s);
            assert!(is_pending(&s, 2, V, Some(&fcm("a"))));
        }
    }

    #[test]
    fn failures_back_off_and_a_429_waits_what_it_says_within_an_hour() {
        let mut s = registered();
        schedule_change(&mut s, Kind::Switch, 0);
        let snap = snapshot(&s);
        let waits: Vec<Step> =
            (0..5).map(|_| apply_failure(&mut s, &snap, Outcome::Transient, 0, V, None)).collect();
        assert_eq!(
            waits,
            vec![Step::Wait(30_000), Step::Wait(120_000), Step::Wait(600_000), Step::Wait(1_800_000), Step::Wait(1_800_000)]
        );
        let mut r = registered();
        let snap = snapshot(&r);
        assert_eq!(apply_failure(&mut r, &snap, Outcome::RateLimited { retry_after_ms: None }, 0, V, None), Step::Wait(600_000));
        assert_eq!(r.not_before_ms, 600_000);
        assert_eq!(
            apply_failure(&mut r, &snap, Outcome::RateLimited { retry_after_ms: Some(120_000) }, 0, V, None),
            Step::Wait(120_000)
        );
        assert_eq!(
            apply_failure(&mut r, &snap, Outcome::RateLimited { retry_after_ms: Some(9_000_000) }, 0, V, None),
            Step::Wait(RATE_LIMIT_CAP_MS)
        );
    }

    #[test]
    fn a_removal_goes_inside_the_429_window_and_a_paid_send_waits() {
        let mut s = registered();
        s.not_before_ms = 1_000_000;
        assert!(is_subset(&s, "u", &fcm("a"), &ids(&["1", "3"])));
        assert!(may_post(&s, 10, true));
        assert!(!is_subset(&s, "u", &fcm("a"), &ids(&["1", "4"])), "an added channel is paid");
        assert!(!may_post(&s, 10, false));
        assert!(may_post(&s, 1_000_000, false));
        assert!(!is_subset(&s, "other", &fcm("a"), &ids(&["1"])), "another user is not a removal");
        assert!(!is_subset(&s, "u", &fcm("b"), &ids(&["1"])), "another device is not a removal");
        let snap = snapshot(&s);
        assert_eq!(apply_failure(&mut s, &snap, Outcome::Waiting, 10, V, None), Step::Wait(1_000_000));
    }

    #[test]
    fn a_pending_unregister_survives_a_relaunch_and_only_a_later_register_clears_it() {
        let mut s = registered();
        let before = snapshot(&s);
        let set_at = begin_unregister(&mut s, fcm("a"));
        // Relaunch: the state comes back from its file.
        let mut s = load(Some(&serde_json::to_string(&s).unwrap()));
        assert_eq!(s.pending_unregister.as_ref().map(|p| p.set_at), Some(set_at));
        assert!(is_pending(&s, 0, V, Some(&fcm("a"))));
        let sent = |d: &str| Sent { decision: SentDecision::Register, ids: ids(&["9"]), user_id: Some("b".into()), device: fcm(d) };
        // A register that started before the sign-out does not clear it.
        apply_sent(&mut s, &before, sent("a"), 5);
        assert!(s.pending_unregister.is_some());
        // Another device's register does not either.
        let after = snapshot(&s);
        apply_sent(&mut s, &after, sent("z"), 6);
        assert!(s.pending_unregister.is_some());
        // The same device registered by a pass that started after it does.
        apply_sent(&mut s, &after, sent("a"), 7);
        assert!(s.pending_unregister.is_none());
    }

    #[test]
    fn a_failed_register_keeps_the_pending_unregister() {
        let mut s = registered();
        begin_unregister(&mut s, fcm("a"));
        let snap = snapshot(&s);
        apply_failure(&mut s, &snap, Outcome::Transient, 1, V, Some(&fcm("a")));
        apply_failure(&mut s, &snap, Outcome::TokenRejected { token_hash: None }, 2, V, Some(&fcm("a")));
        assert!(s.pending_unregister.is_some());
        assert!(is_pending(&s, 3, V, Some(&fcm("a"))), "a parked phone still deletes the leftover");
        let set_at = s.pending_unregister.as_ref().unwrap().set_at;
        unregister_done(&mut s, set_at + 1);
        assert!(s.pending_unregister.is_some(), "a newer leftover is not cleared by an older delete");
        unregister_done(&mut s, set_at);
        assert!(s.pending_unregister.is_none());
    }

    #[test]
    fn a_launch_or_worker_posts_when_owed_when_the_device_changed_or_after_six_hours() {
        let s = registered();
        assert!(!should_post(&s, 60_000, V, Some(&fcm("a"))));
        assert!(should_post(&s, 60_000, V, Some(&fcm("rotated"))));
        assert!(should_post(&s, RESEND_AFTER_MS, V, Some(&fcm("a"))));
        assert!(!should_post(&s, RESEND_AFTER_MS, V, None), "no device, nothing owed");
        let mut owed = s.clone();
        schedule_change(&mut owed, Kind::MuteOnly, 1);
        assert!(should_post(&owed, 2, V, Some(&fcm("a"))));
        let mut parked = owed.clone();
        let snap = snapshot(&parked);
        apply_failure(&mut parked, &snap, Outcome::SignedOut, 3, V, Some(&fcm("a")));
        assert!(!should_post(&parked, RESEND_AFTER_MS * 2, V, Some(&fcm("a"))), "no post while parked");
        begin_unregister(&mut parked, fcm("a"));
        assert!(should_post(&parked, 4, V, Some(&fcm("a"))), "a leftover delete still goes");
    }

    #[test]
    fn the_iphone_repeat_report_is_skipped_for_ten_minutes_with_nothing_owed() {
        let s = registered();
        assert!(apns_repeat_skip(&s, &fcm("a"), APNS_REPEAT_MS - 1, V));
        assert!(!apns_repeat_skip(&s, &fcm("a"), APNS_REPEAT_MS, V));
        assert!(!apns_repeat_skip(&s, &fcm("b"), 1, V));
        let mut owed = s;
        schedule_change(&mut owed, Kind::Switch, 1);
        assert!(!apns_repeat_skip(&owed, &fcm("a"), 2, V));
    }

    #[test]
    fn a_repeat_device_report_posts_after_ten_minutes_though_the_six_hour_rule_would_not() {
        let s = registered();
        let a = fcm("a");
        let later = APNS_REPEAT_MS + 1;
        assert!(!should_post(&s, later, V, Some(&a)), "the launch rule waits six hours");
        assert!(first_pass_posts(&s, PostReason::DeviceReport, later, V, Some(&a)), "the device report does not");
        assert!(!first_pass_posts(&s, PostReason::DeviceReport, APNS_REPEAT_MS - 1, V, Some(&a)));
        assert!(!first_pass_posts(&s, PostReason::DeviceReport, later, V, None));
        assert!(!first_pass_posts(&s, PostReason::Due, later, V, Some(&a)));
        assert!(first_pass_posts(&s, PostReason::Due, RESEND_AFTER_MS, V, Some(&a)));
        assert!(!first_pass_posts(&s, PostReason::Owed, RESEND_AFTER_MS, V, Some(&a)), "owed alone never forces");
    }

    #[test]
    fn a_sign_out_settles_everything_owed_for_the_leaving_account() {
        let mut s = registered();
        schedule_change(&mut s, Kind::MuteOnly, 1);
        begin_unregister(&mut s, fcm("a"));
        note_unregistered(&mut s, &fcm("a"), 2);
        assert_eq!(s.sent, s.wanted);
        assert_eq!(s.sent_decision, Some(SentDecision::Unregister));
        assert!(s.pending_unregister.is_none());
        assert!(!is_pending(&s, 3, V, Some(&fcm("a"))));
        assert!(!is_subset(&s, "u", &fcm("a"), &ids(&[])), "after an unregister nothing is a removal");
    }

    #[test]
    fn one_flush_sends_at_most_three_times() {
        assert!(runs_again(1));
        assert!(runs_again(2));
        assert!(!runs_again(3));
    }

    #[test]
    fn answers_by_status() {
        assert_eq!(classify_status(200, None, false), Answer::Ok);
        assert_eq!(classify_status(204, None, true), Answer::Ok);
        assert_eq!(classify_status(403, None, true), Answer::Ok, "not yours: nothing of ours left");
        assert_eq!(classify_status(404, None, true), Answer::Ok);
        assert_eq!(classify_status(403, None, false), Answer::Transient, "a proxy or WAF 403 is not a verdict");
        assert_eq!(classify_status(400, None, false), Answer::Rejected);
        assert_eq!(classify_status(401, None, false), Answer::Unauthorized);
        assert_eq!(classify_status(408, None, false), Answer::Transient);
        assert_eq!(classify_status(409, None, false), Answer::Transient);
        assert_eq!(classify_status(429, Some(5_000), false), Answer::RateLimited(Some(5_000)));
        assert_eq!(classify_status(503, None, false), Answer::Transient);
        assert_eq!(outcome_of(Answer::Unauthorized, Some("h".into()), true), Outcome::Transient);
        assert_eq!(outcome_of(Answer::Unauthorized, Some("h".into()), false), Outcome::TokenRejected { token_hash: Some("h".into()) });
        assert_eq!(parse_retry_after(Some(" 120 ")), Some(120_000));
        assert_eq!(parse_retry_after(Some("Wed, 21 Oct 2026 07:28:00 GMT")), None);
        assert_eq!(parse_retry_after(None), None);
    }

    #[test]
    fn a_device_is_deleted_by_its_own_token_only_when_the_token_is_long_enough() {
        assert!(!device_only_allowed(&fcm("short")));
        assert!(device_only_allowed(&fcm(&"f".repeat(64))));
        let apns = PushDevice::Apns { token: "ab".repeat(32), env: "production".into(), topic: "app.streamnook".into() };
        assert!(device_only_allowed(&apns));
    }

    #[test]
    fn token_errors_keep_their_words_and_map_by_reason() {
        let text = "Not authenticated. Please log in to Twitch first.";
        let e = TokenError::NoToken.into_error(text);
        assert_eq!(format!("{e}"), text, "the pages read these words");
        assert_eq!(classify_token_error(&e), Outcome::SignedOut);
        assert_eq!(classify_token_error(&TokenError::Refused.into_error("x")), Outcome::TokenRejected { token_hash: None });
        assert_eq!(classify_token_error(&TokenError::VaultUnavailable.into_error("x")), Outcome::Transient);
        assert_eq!(classify_token_error(&TokenError::Unreachable.into_error("x")), Outcome::Transient);
        // Wrapped again by a caller, the reason is still found.
        let wrapped = TokenError::NoToken.into_error("x").context("followed channels");
        assert_eq!(classify_token_error(&wrapped), Outcome::SignedOut);
        assert_eq!(classify_token_error(&anyhow::anyhow!("helix 500")), Outcome::Transient);
    }

    #[test]
    fn a_refused_refresh_park_is_released_by_any_token_change() {
        let mut s = registered();
        schedule_change(&mut s, Kind::Switch, 0);
        let snap = snapshot(&s);
        apply_failure(&mut s, &snap, Outcome::TokenRejected { token_hash: None }, 1, V, Some(&fcm("a")));
        assert!(release_parks(&mut s, None), "a deletion releases a hash-less park");
        apply_failure(&mut s, &snap, Outcome::TokenRejected { token_hash: None }, 2, V, Some(&fcm("a")));
        assert!(release_parks(&mut s, Some("h")), "a token write releases it too");
    }

    #[test]
    fn a_leftover_is_left_to_the_register_only_when_that_register_can_go() {
        let long = fcm(&"f".repeat(64));
        let other = fcm(&"e".repeat(64));
        assert_eq!(leftover_action(&long, Some(&long), true, true, false, true), LeftoverAction::LeaveToRegister);
        assert_eq!(leftover_action(&long, Some(&long), true, true, false, false), LeftoverAction::Delete, "inside a 429 window");
        assert_eq!(leftover_action(&long, Some(&long), true, false, false, true), LeftoverAction::Delete, "signed out");
        assert_eq!(leftover_action(&long, Some(&long), true, true, true, true), LeftoverAction::Delete, "parked");
        assert_eq!(leftover_action(&long, Some(&long), false, true, false, true), LeftoverAction::Delete, "a drop");
        assert_eq!(leftover_action(&long, Some(&other), true, true, false, true), LeftoverAction::Delete, "another device");
        assert_eq!(leftover_action(&fcm("short"), Some(&fcm("x")), false, false, false, true), LeftoverAction::Forget);
    }

    #[test]
    fn the_gate_goes_when_owed_or_forced_and_never_through_a_park() {
        assert_eq!(pass_gate(false, true, false, false), Gate::Go);
        assert_eq!(pass_gate(false, false, true, false), Gate::Go);
        assert_eq!(pass_gate(false, false, false, false), Gate::Current);
        assert_eq!(pass_gate(true, true, true, false), Gate::Parked);
        assert_eq!(pass_gate(true, true, true, true), Gate::RetryLeftover);
        assert_eq!(pass_gate(false, false, false, true), Gate::RetryLeftover);
    }

    #[test]
    fn a_failed_leftover_behind_a_park_or_done_still_backs_off() {
        assert!(leftover_retry(Step::Parked, true));
        assert!(leftover_retry(Step::Done, true));
        assert!(!leftover_retry(Step::Wait(5), true), "already armed");
        assert!(!leftover_retry(Step::Done, false));
    }
}
