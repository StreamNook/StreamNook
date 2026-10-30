//! Instant go-live alerts on both phones: this phone's registration with the
//! push server (`notify.streamnook.app`).
//!
//! The server holds, per device, the ids of the channels whose go-lives it
//! should push, and pushes `stream.online` to every device holding that id.
//! Which channels those are is decided here, in one place for both phones:
//! the channels the viewer follows, minus the ones muted for live alerts, and
//! none at all while the master switch or the Live channels switch is off. An
//! empty list keeps the device registered with nothing to send, so turning a
//! switch back on needs no new token.
//!
//! The device differs per phone. Android hands over its FCM token through the
//! `fcm_token` file Kotlin writes when Play services answers; the iPhone hands
//! over its APNs token, environment and topic with `push_set_apns_device`.
//! Android also keeps its own Instant alerts switch (`push_notifications`): off
//! drops the device from the server and leaves the scheduled poll to deliver.
//! The iPhone's poll (`services::live_poll`) is a background refresh iOS runs
//! when it chooses, a fallback only, so push stays its main lane and that
//! switch is not read there.
//!
//! Every change that alters the list (a follow, an unfollow, a mute, the
//! switches) re-registers on a short debounce, so a run of quick changes costs
//! one request.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// The push lane's server. Registration is authed by the viewer's own Twitch
/// token; the server validates it against Twitch and stores only the user id,
/// the device token and the channel ids it was handed.
#[cfg(mobile)]
const NOTIFY_SERVER: &str = "https://notify.streamnook.app";

/// Where this phone's pushes go.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum PushDevice {
    /// Firebase Cloud Messaging (Android).
    Fcm { token: String },
    /// Apple Push Notification service (iPhone): `env` is `sandbox` for
    /// development-signed builds and `production` for TestFlight and the App
    /// Store; `topic` is the app's own bundle id.
    Apns { token: String, env: String, topic: String },
}

impl PushDevice {
    /// An APNs device from what the iPhone reports, or why it cannot be one.
    pub fn apns(token: &str, env: &str, topic: &str) -> Result<Self, String> {
        let token = token.trim().to_ascii_lowercase();
        if token.is_empty() || token.len() % 2 != 0 || !token.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err("The device token is not hex.".to_string());
        }
        if !matches!(env, "sandbox" | "production") {
            return Err(format!("Unknown push environment {env}."));
        }
        let topic = topic.trim();
        if topic.is_empty() {
            return Err("The push topic is empty.".to_string());
        }
        Ok(Self::Apns { token, env: env.to_string(), topic: topic.to_string() })
    }

    /// The `/register` body. An FCM device also carries the original
    /// `fcm_token` key, so the body reads the same to a server that predates
    /// `kind` and to one that knows it.
    pub fn register_body(&self, follows: &[String]) -> Value {
        let mut body = self.identity();
        body["follows"] = json!(follows);
        body
    }

    /// The `/unregister` body.
    pub fn unregister_body(&self) -> Value {
        self.identity()
    }

    fn identity(&self) -> Value {
        match self {
            Self::Fcm { token } => json!({ "kind": "fcm", "token": token, "fcm_token": token }),
            Self::Apns { token, env, topic } => json!({ "kind": "apns", "token": token, "env": env, "topic": topic }),
        }
    }
}

/// The settings that decide what the server holds for this phone. Muted logins
/// are kept lowercased and sorted, so two reads of the same list compare equal.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AlertInputs {
    /// The master notifications switch.
    pub enabled: bool,
    /// Notify me about > Live channels.
    pub live: bool,
    /// Android's Instant alerts switch.
    pub push: bool,
    pub muted: Vec<String>,
}

impl AlertInputs {
    /// From a `live_notifications` object as saved. A switch that was never
    /// saved reads as on, matching the serde defaults on the settings struct.
    pub fn from_json(notif: Option<&Value>) -> Self {
        let flag = |name: &str| notif.and_then(|n| n.get(name)).and_then(Value::as_bool).unwrap_or(true);
        let mut muted: Vec<String> = notif
            .and_then(|n| n.get("muted_live_channels"))
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()).collect())
            .unwrap_or_default();
        muted.sort();
        muted.dedup();
        Self { enabled: flag("enabled"), live: flag("show_live_notifications"), push: flag("push_notifications"), muted }
    }

    pub fn from_settings(settings: &crate::models::settings::Settings) -> Self {
        Self::from_json(serde_json::to_value(&settings.live_notifications).ok().as_ref())
    }
}

/// What kind of change `next` is after the inputs the last save left in
/// `cell`, keeping `next` there. `None` when nothing the server holds moved;
/// `MuteOnly` when only the muted channels did.
pub fn note_change(cell: &mut Option<AlertInputs>, next: AlertInputs) -> Option<crate::services::push_plan::Kind> {
    use crate::services::push_plan::Kind;
    let kind = match cell.as_ref() {
        Some(prev) if *prev == next => None,
        Some(prev) if prev.enabled == next.enabled && prev.live == next.live && prev.push == next.push => {
            Some(Kind::MuteOnly)
        }
        _ => Some(Kind::Switch),
    };
    *cell = Some(next);
    kind
}

/// What the server should hold for this phone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plan {
    /// Not registered at all (Android with Instant alerts off).
    Drop,
    /// Registered; with the followed channels when `alerting`, else with none.
    Hold { alerting: bool },
}

pub fn plan(platform: &str, inputs: &AlertInputs) -> Plan {
    if platform == "android" && !inputs.push {
        return Plan::Drop;
    }
    let alerting = inputs.enabled && inputs.live && crate::services::alert_types::works_on("live", platform);
    Plan::Hold { alerting }
}

/// The channel ids to hand the server, from the followed channels as
/// `(id, login)`: the first `push_plan::MAX_IDS` followed channels (each
/// once, in follow order), then minus the muted ones. Capping before the mute
/// filter keeps every mute a removal of what was sent, which the server
/// never charges for. `None` drops the device instead.
pub fn follow_list(platform: &str, inputs: &AlertInputs, follows: &[(String, String)]) -> Option<Vec<String>> {
    match plan(platform, inputs) {
        Plan::Drop => None,
        Plan::Hold { alerting: false } => Some(Vec::new()),
        Plan::Hold { alerting: true } => {
            let mut seen = std::collections::HashSet::new();
            Some(
                follows
                    .iter()
                    .filter(|(id, _)| !id.is_empty())
                    .filter(|(id, _)| seen.insert(id.clone()))
                    .take(crate::services::push_plan::MAX_IDS)
                    .filter(|(_, login)| !inputs.muted.iter().any(|m| m.eq_ignore_ascii_case(login.trim())))
                    .map(|(id, _)| id.clone())
                    .collect(),
            )
        }
    }
}

/// The phone side: the device, the runner and the commands. Phones only. The
/// rules are `services::push_plan`; this module does the I/O around them and
/// keeps their state in `push_state.json`.
///
/// Lock order everywhere: the runner (`RUNNER`), then the primary token slot
/// (`TwitchService::primary_slot`), then the state cell (`STATE`), then the
/// settings lock. Nothing takes the runner while holding the slot, and the
/// state cell is never held across an await. Device tokens are never logged.
#[cfg(mobile)]
pub mod server {
    use super::*;
    use crate::services::cache_service;
    use crate::services::push_plan::{self, Answer, Kind, Outcome, PushState, SentDecision, Snapshot, Step};
    use crate::services::twitch_service::TwitchService;
    use std::sync::Mutex;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    /// Followed channels are paged 100 at a time; the server keeps at most
    /// `push_plan::MAX_IDS` of them, so this many pages cover it.
    const FOLLOW_PAGES: usize = 10;
    const APNS_FILE: &str = "apns_device.json";
    const STATE_FILE: &str = "push_state.json";
    /// How long a sign-out spends taking the device off the server itself.
    const SIGN_OUT_BUDGET: Duration = Duration::from_secs(5);
    /// The most one flush spends in total (runner wait, follows, posts).
    const FLUSH_BUDGET: Duration = Duration::from_secs(25);
    /// Below this much time left, a background refresh does not start a send.
    #[cfg(target_os = "ios")]
    const POLL_FLUSH_MIN: Duration = Duration::from_secs(5);

    static STATE: Mutex<Option<PushState>> = Mutex::new(None);
    static RUNNER: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    /// The settings the last save left, only to tell what kind of change the
    /// next one is. Sends read the live settings.
    static INPUTS: Mutex<Option<AlertInputs>> = Mutex::new(None);
    /// The Twitch user each access token belongs to (by fingerprint).
    static USER_FOR_TOKEN: Mutex<Option<(String, String)>> = Mutex::new(None);

    /// Held while a pass talks to the server; a sign-out holds it across
    /// taking the device off and deleting the token.
    pub struct RunnerGuard {
        _held: tokio::sync::MutexGuard<'static, ()>,
    }

    fn now_ms() -> u64 {
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
    }

    fn app_version() -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    fn state_path() -> Option<std::path::PathBuf> {
        cache_service::get_app_data_dir().ok().map(|d| d.join(STATE_FILE))
    }

    fn load_state() -> PushState {
        let text = state_path().and_then(|p| std::fs::read_to_string(p).ok());
        push_plan::load(text.as_deref())
    }

    fn persist_state(state: &PushState) {
        let Some(path) = state_path() else { return };
        let Ok(text) = serde_json::to_string(state) else { return };
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }

    /// Read-modify-persist on the state cell. Never held across an await.
    /// Before the data folder resolves (a phone's first moments) it works on a
    /// throwaway state and does not fill the cell, so defaults never latch and
    /// later overwrite the real file.
    fn with_state<R>(f: impl FnOnce(&mut PushState) -> R) -> R {
        let mut cell = STATE.lock().unwrap_or_else(|p| p.into_inner());
        if cell.is_none() {
            if state_path().is_none() {
                let mut throwaway = push_plan::load(None);
                return f(&mut throwaway);
            }
            *cell = Some(load_state());
        }
        let Some(state) = cell.as_mut() else { unreachable!("filled above") };
        let before = state.clone();
        let out = f(state);
        if *state != before {
            persist_state(state);
        }
        out
    }

    /// At launch, from the settings as loaded: remember them for change
    /// detection, give every park one fresh try, and run a pass now if
    /// something is still owed.
    pub fn seed(settings: &crate::models::settings::Settings) {
        if let Ok(mut inputs) = INPUTS.lock() {
            *inputs = Some(AlertInputs::from_settings(settings));
        }
        let device = device();
        let now = now_ms();
        let owed = with_state(|s| {
            push_plan::on_launch(s);
            push_plan::is_pending(s, now, app_version(), device.as_ref())
        });
        if owed {
            arm_now();
        }
    }

    /// After every settings save: a change of the switches or the mute list is
    /// owed to the server. Only the mute list moving settles longer.
    pub fn refresh(settings: &crate::models::settings::Settings) {
        let kind = match INPUTS.lock() {
            // `after_settings_change` runs the save hooks one at a time on the
            // settings as they are now, so two saves finishing out of order
            // never leave the older list here.
            Ok(mut inputs) => note_change(&mut inputs, AlertInputs::from_settings(settings)),
            Err(_) => None,
        };
        if let Some(kind) = kind {
            schedule_kind(kind);
        }
    }

    /// A change the server should hear about (a follow, an unfollow, an
    /// account change): settles 3 s.
    pub fn schedule() {
        schedule_kind(Kind::Switch);
    }

    pub fn schedule_kind(kind: Kind) {
        let due = with_state(|s| push_plan::schedule_change(s, kind, now_ms()));
        arm(due);
    }

    /// The primary token changed (`token_hash` of the new one, `None` when it
    /// was deleted). Called by the token slot's only writers while they hold
    /// the slot, so it never waits: it releases a park that waited for this
    /// and arms a pass, nothing else. It never counts as a change.
    pub fn token_changed(token_hash: Option<&str>) {
        let device = device();
        let now = now_ms();
        let owed = with_state(|s| {
            push_plan::release_parks(s, token_hash) && push_plan::is_pending(s, now, app_version(), device.as_ref())
        });
        if owed {
            arm_now();
        }
    }

    fn arm_now() {
        let due = with_state(|s| {
            let due = now_ms();
            s.due_ms = Some(due);
            due
        });
        arm(due);
    }

    /// Run a pass at `due` (epoch ms) unless a later change moved the due
    /// time. Only sets up a timer; it never waits on the runner here.
    fn arm(due: u64) {
        tauri::async_runtime::spawn(async move {
            let wait = due.saturating_sub(now_ms());
            if wait > 0 {
                tokio::time::sleep(Duration::from_millis(wait)).await;
            }
            if with_state(|s| s.due_ms) != Some(due) {
                return;
            }
            if let Err(e) = flush(FLUSH_BUDGET, push_plan::PostReason::Owed).await {
                log::warn!("[PushAlerts] pass did not finish: {e}");
            }
        });
    }

    fn owed_now() -> bool {
        let device = device();
        let now = now_ms();
        with_state(|s| push_plan::is_pending(s, now, app_version(), device.as_ref()))
    }

    /// Up to `MAX_RUNS` passes under the runner, all within `budget`.
    /// `reason` says whether the first pass posts even with nothing owed (a
    /// launch, a worker run, a device report). A pass cut off by the budget
    /// leaves its intent on disk and is tried again shortly.
    async fn flush(budget: Duration, reason: push_plan::PostReason) -> Result<String, String> {
        let run = async {
            let runner = RunnerGuard { _held: RUNNER.lock().await };
            // Decided under the runner: a pass that ran while this one waited
            // (the launch pass before `push_register`) may have made it moot.
            let force = {
                let device = device();
                let now = now_ms();
                with_state(|s| push_plan::first_pass_posts(s, reason, now, app_version(), device.as_ref()))
            };
            run_passes(&runner, force).await
        };
        match tokio::time::timeout(budget, run).await {
            Ok(summary) => Ok(summary),
            Err(_) => {
                let due = with_state(|s| {
                    let due = now_ms() + push_plan::RUN_AGAIN_MS;
                    s.due_ms = Some(due);
                    due
                });
                arm(due);
                Err("timed out".to_string())
            }
        }
    }

    async fn run_passes(runner: &RunnerGuard, force: bool) -> String {
        let mut runs = 1;
        let (mut summary, mut step) = pass(runner, force).await;
        while let Step::RunAgain(_) = step {
            if !push_plan::runs_again(runs) {
                break;
            }
            runs += 1;
            (summary, step) = pass(runner, false).await;
        }
        match step {
            Step::RunAgain(due) | Step::Wait(due) => arm(due),
            Step::Done | Step::Parked => {}
        }
        log::info!("[PushAlerts] {summary}");
        summary
    }

    fn failed(snap: &Snapshot, outcome: Outcome, now: u64, device: Option<&PushDevice>) -> (String, Step) {
        let label = format!("{outcome:?}");
        let step = with_state(|s| push_plan::apply_failure(s, snap, outcome, now, app_version(), device));
        (label, step)
    }

    /// `get_token`, run detached: an outer timeout (a flush budget, the
    /// sign-out budget) can stop waiting for it, but never cancel a refresh
    /// between Twitch's answer and the persist of the rotated token.
    async fn read_token() -> anyhow::Result<String> {
        tauri::async_runtime::spawn(TwitchService::get_token()).await.map_err(anyhow::Error::from).and_then(|r| r)
    }

    /// `get_token_after_reject`, run detached for the same reason.
    async fn read_token_after_reject(rejected: &str) -> anyhow::Result<String> {
        let rejected = rejected.to_string();
        tauri::async_runtime::spawn(async move { TwitchService::get_token_after_reject(&rejected).await })
            .await
            .map_err(anyhow::Error::from)
            .and_then(|r| r)
    }

    /// One pass: a leftover device first, then what the settings say the
    /// server should hold for this device.
    async fn pass(_runner: &RunnerGuard, force: bool) -> (String, Step) {
        let now = now_ms();
        let device = device();
        let (snap, leftover, holds, owed, paid_open) = with_state(|s| {
            (
                push_plan::snapshot(s),
                s.pending_unregister.clone(),
                push_plan::park_holds(s, now, app_version(), device.as_ref()),
                s.wanted > s.sent,
                now >= s.not_before_ms,
            )
        });
        let inputs = current_inputs();
        let target = plan(crate::services::alert_types::PLATFORM, &inputs);
        let token = read_token().await;

        // A device a sign-out could not take off the server.
        let mut leftover_failed = false;
        let mut leftover_action = None;
        if let Some(pending) = leftover {
            let action = push_plan::leftover_action(
                &pending.device,
                device.as_ref(),
                matches!(target, Plan::Hold { .. }),
                token.is_ok(),
                holds,
                paid_open,
            );
            leftover_action = Some(action);
            match action {
                push_plan::LeftoverAction::LeaveToRegister => {}
                push_plan::LeftoverAction::Forget => {
                    // The server refuses a device-only delete for so short a
                    // token; never send it, and stop owing it.
                    log::warn!("[PushAlerts] a signed-out device's token is too short to delete by itself");
                    with_state(|s| push_plan::unregister_done(s, pending.set_at));
                }
                push_plan::LeftoverAction::Delete => {
                    let answer = match post("/unregister", &pending.device.unregister_body(), None).await {
                        Ok((status, retry_after)) => push_plan::classify_status(status, retry_after, true),
                        Err(_) => Answer::Transient,
                    };
                    if answer == Answer::Ok {
                        with_state(|s| push_plan::unregister_done(s, pending.set_at));
                    } else {
                        log::warn!("[PushAlerts] a signed-out device is not off the server yet: {answer:?}");
                        leftover_failed = true;
                    }
                }
            }
        }

        // A leftover left to this pass's register makes that register post.
        let force = push_plan::pass_force(force, leftover_action);
        match push_plan::pass_gate(holds, owed, force, leftover_failed) {
            push_plan::Gate::Go => {}
            push_plan::Gate::RetryLeftover => return failed(&snap, Outcome::Transient, now, device.as_ref()),
            push_plan::Gate::Parked => return ("parked".to_string(), Step::Parked),
            push_plan::Gate::Current => return ("current".to_string(), Step::Done),
        }

        let Some(device) = device else {
            return failed(&snap, Outcome::NoDevice, now, None);
        };

        let (label, step) = match target {
            Plan::Drop => drop_device(&snap, now, device, token).await,
            Plan::Hold { alerting } => register(&snap, now, device, &inputs, alerting, token).await,
        };
        if push_plan::leftover_retry(step, leftover_failed) {
            // Whatever the send did, the leftover delete still needs its backoff.
            let (_, retry) = failed(&snap, Outcome::Transient, now, None);
            return (label, retry);
        }
        (label, step)
    }

    /// Android with Instant alerts off: this device leaves the server. A 401
    /// with a user token gets one fresh token, then parks on the refused one;
    /// a 401 on a device-only delete (the server before device-only deletes)
    /// is only a wait.
    async fn drop_device(snap: &Snapshot, now: u64, device: PushDevice, token: anyhow::Result<String>) -> (String, Step) {
        let mut used = token.ok();
        if used.is_none() && !push_plan::device_only_allowed(&device) {
            return failed(snap, Outcome::SignedOut, now, Some(&device));
        }
        let mut answer = post("/unregister", &device.unregister_body(), used.as_deref()).await;
        if matches!(answer, Ok((401, _))) {
            if let Some(old) = used.clone() {
                match read_token_after_reject(&old).await {
                    Ok(fresh) if fresh != old => {
                        answer = post("/unregister", &device.unregister_body(), Some(&fresh)).await;
                        used = Some(fresh);
                    }
                    Ok(_) => {}
                    Err(e) => return failed(snap, push_plan::classify_token_error(&e), now, Some(&device)),
                }
            }
        }
        let answer = match answer {
            Ok((status, retry_after)) => push_plan::classify_status(status, retry_after, true),
            Err(_) => Answer::Transient,
        };
        if answer != Answer::Ok {
            let outcome = match (answer, used.as_deref()) {
                (Answer::Unauthorized, Some(t)) => {
                    push_plan::outcome_of(answer, Some(crate::services::twitch_service::token_hash(t)), false)
                }
                _ => push_plan::outcome_of(answer, None, true),
            };
            return failed(snap, outcome, now, Some(&device));
        }
        #[cfg(target_os = "ios")]
        crate::services::live_poll::poll::note_registered(None);
        let sent = push_plan::Sent { decision: SentDecision::Unregister, ids: Vec::new(), user_id: None, device };
        let step = with_state(|s| push_plan::apply_sent(s, snap, sent, now));
        ("unregistered".to_string(), step)
    }

    async fn register(
        snap: &Snapshot,
        now: u64,
        device: PushDevice,
        inputs: &AlertInputs,
        alerting: bool,
        token: anyhow::Result<String>,
    ) -> (String, Step) {
        let mut token = match token {
            Ok(token) => token,
            Err(e) => return failed(snap, push_plan::classify_token_error(&e), now, Some(&device)),
        };
        // Who the token belongs to and what it follows. A Helix 401 (a token
        // revoked before it expired) gets one fresh token, like a server 401;
        // refused again, the pass parks on that token.
        let mut refreshed = false;
        let (user_id, follows) = loop {
            let read = async {
                let user_id = user_for(&token).await?;
                let follows = if alerting {
                    // Detached: `followed()` reads the token itself and may
                    // refresh it.
                    tauri::async_runtime::spawn(followed()).await.map_err(anyhow::Error::from).and_then(|r| r)?
                } else {
                    Vec::new()
                };
                anyhow::Ok((user_id, follows))
            }
            .await;
            match read {
                Ok(read) => break read,
                Err(e) => {
                    let outcome = push_plan::classify_token_error(&e);
                    let refused = matches!(outcome, Outcome::TokenRejected { .. });
                    if refused && !refreshed {
                        refreshed = true;
                        match read_token_after_reject(&token).await {
                            Ok(fresh) => {
                                token = fresh;
                                continue;
                            }
                            Err(e) => return failed(snap, push_plan::classify_token_error(&e), now, Some(&device)),
                        }
                    }
                    let outcome = if refused {
                        Outcome::TokenRejected { token_hash: Some(crate::services::twitch_service::token_hash(&token)) }
                    } else {
                        outcome
                    };
                    return failed(snap, outcome, now, Some(&device));
                }
            }
        };
        let ids = follow_list(crate::services::alert_types::PLATFORM, inputs, &follows).unwrap_or_default();
        let may = with_state(|s| {
            let subset = push_plan::is_subset(s, &user_id, &device, &ids);
            push_plan::may_post(s, now, subset)
        });
        if !may {
            return failed(snap, Outcome::Waiting, now, Some(&device));
        }
        let body = device.register_body(&ids);
        let mut used = token;
        let mut answer = post("/register", &body, Some(&used)).await;
        if matches!(answer, Ok((401, _))) {
            // Once: a fresh token if the stored one is the one refused.
            match read_token_after_reject(&used).await {
                Ok(fresh) if fresh != used => {
                    used = fresh;
                    answer = post("/register", &body, Some(&used)).await;
                }
                Ok(_) => {}
                Err(e) => return failed(snap, push_plan::classify_token_error(&e), now, Some(&device)),
            }
        }
        let answer = match answer {
            Ok((status, retry_after)) => push_plan::classify_status(status, retry_after, false),
            Err(_) => Answer::Transient,
        };
        if answer != Answer::Ok {
            let hash = crate::services::twitch_service::token_hash(&used);
            return failed(snap, push_plan::outcome_of(answer, Some(hash), false), now, Some(&device));
        }
        // The iPhone's poll leaves these to the push once pushes arrive.
        #[cfg(target_os = "ios")]
        crate::services::live_poll::poll::note_registered(Some(&ids));
        let count = ids.len();
        let sent = push_plan::Sent { decision: SentDecision::Register, ids, user_id: Some(user_id), device };
        let step = with_state(|s| push_plan::apply_sent(s, snap, sent, now));
        (format!("registered:{count}"), step)
    }

    /// The live settings when this process holds them, else the saved file
    /// (the Android worker, an iPhone background refresh before load).
    fn current_inputs() -> AlertInputs {
        if let Some(inputs) = live_inputs() {
            return inputs;
        }
        let saved = cache_service::get_app_data_dir()
            .ok()
            .and_then(|d| std::fs::read_to_string(d.join("settings.json")).ok())
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .unwrap_or(Value::Null);
        AlertInputs::from_json(saved.get("live_notifications"))
    }

    /// The settings this process holds now, when it holds them.
    fn live_inputs() -> Option<AlertInputs> {
        use tauri::Manager;
        let app = crate::services::providers::app_handle()?;
        let state = app.try_state::<crate::models::settings::AppState>()?;
        let settings = state.settings.lock().ok()?;
        Some(AlertInputs::from_settings(&settings))
    }

    /// A restored install (a backup, a phone transfer) carries another
    /// phone's push state, a device left to delete included: start over,
    /// before `seed` reads it.
    pub fn forget_restored_state() {
        if let Some(path) = state_path() {
            let _ = std::fs::remove_file(path);
        }
        // The iPhone's device file comes back from a backup too (Application
        // Support is backed up): until this phone reports its own token, a
        // pass would register the old phone's. Android's FCM token is not in
        // its backup, and its shell writes it once; it stays.
        #[cfg(target_os = "ios")]
        if let Ok(dir) = cache_service::get_app_data_dir() {
            let _ = std::fs::remove_file(dir.join(APNS_FILE));
        }
        if let Ok(mut cell) = STATE.lock() {
            *cell = None;
        }
    }

    /// This phone's device, when it has reported one.
    fn device() -> Option<PushDevice> {
        let dir = cache_service::get_app_data_dir().ok()?;
        if cfg!(target_os = "ios") {
            let text = std::fs::read_to_string(dir.join(APNS_FILE)).ok()?;
            serde_json::from_str::<PushDevice>(&text).ok()
        } else {
            let token = std::fs::read_to_string(dir.join("fcm_token")).ok()?;
            let token = token.trim();
            (!token.is_empty()).then(|| PushDevice::Fcm { token: token.to_string() })
        }
    }

    /// The first followed Twitch channels as `(id, login)`, paged.
    async fn followed() -> anyhow::Result<Vec<(String, String)>> {
        let mut follows = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..FOLLOW_PAGES {
            let (page, next) = TwitchService::get_all_followed_channels(100, cursor.take()).await?;
            let empty = page.is_empty();
            follows.extend(page.into_iter().map(|c| (c.user_id, c.user_login)));
            match next {
                Some(next) if !empty => cursor = Some(next),
                _ => break,
            }
        }
        Ok(follows)
    }

    /// The Twitch user `token` belongs to, read once per token.
    async fn user_for(token: &str) -> anyhow::Result<String> {
        let hash = crate::services::twitch_service::token_hash(token);
        let cached = USER_FOR_TOKEN
            .lock()
            .ok()
            .and_then(|c| c.as_ref().filter(|(h, _)| *h == hash).map(|(_, id)| id.clone()));
        if let Some(id) = cached {
            return Ok(id);
        }
        let info = TwitchService::get_user_info_with_token(token).await?;
        if let Ok(mut c) = USER_FOR_TOKEN.lock() {
            *c = Some((hash, info.id.clone()));
        }
        Ok(info.id)
    }

    /// One post to the notify server: the status and any `Retry-After`. A
    /// device-only unregister passes no user token.
    async fn post(path: &str, body: &Value, user_token: Option<&str>) -> anyhow::Result<(u16, Option<u64>)> {
        let mut request = crate::services::http::client().clone().post(format!("{NOTIFY_SERVER}{path}")).json(body);
        if let Some(token) = user_token {
            request = request.header(reqwest::header::AUTHORIZATION, format!("OAuth {token}"));
        }
        let resp = request.send().await?;
        let status = resp.status().as_u16();
        let retry_after =
            push_plan::parse_retry_after(resp.headers().get(reqwest::header::RETRY_AFTER).and_then(|v| v.to_str().ok()));
        Ok((status, retry_after))
    }

    /// Before a sign-out deletes the token: take this device off the server
    /// with the leaving account's token, holding the runner so no queued pass
    /// can register that account again before the token is gone. Within
    /// `SIGN_OUT_BUDGET`; when that is not enough, the device is left to be
    /// deleted later by its own token. The caller holds the guard it returns
    /// until the token is deleted.
    pub async fn unregister_before_sign_out() -> Option<RunnerGuard> {
        let device = device()?;
        let deadline = tokio::time::Instant::now() + SIGN_OUT_BUDGET;
        let leave_for_later = |device: PushDevice| {
            let due = with_state(|s| {
                push_plan::begin_unregister(s, device);
                let due = now_ms() + push_plan::BACKOFF_MS[0];
                s.due_ms = Some(due);
                due
            });
            arm(due);
        };
        let runner = match tokio::time::timeout_at(deadline, RUNNER.lock()).await {
            Ok(held) => RunnerGuard { _held: held },
            Err(_) => {
                leave_for_later(device);
                return None;
            }
        };
        let posted = tokio::time::timeout_at(deadline, async {
            let token = read_token().await.ok();
            if token.is_none() && !push_plan::device_only_allowed(&device) {
                return None;
            }
            Some(post("/unregister", &device.unregister_body(), token.as_deref()).await)
        })
        .await;
        let answer = match posted {
            Ok(Some(Ok((status, retry_after)))) => push_plan::classify_status(status, retry_after, true),
            _ => Answer::Transient,
        };
        if answer == Answer::Ok {
            let now = now_ms();
            with_state(|s| push_plan::note_unregistered(s, &device, now));
            #[cfg(target_os = "ios")]
            crate::services::live_poll::poll::note_registered(None);
        } else {
            leave_for_later(device);
        }
        Some(runner)
    }

    /// Back in the foreground: a change made just before the app went away
    /// may still be owed.
    #[cfg(target_os = "ios")]
    pub async fn wake() {
        if owed_now() {
            let _ = flush(FLUSH_BUDGET, push_plan::PostReason::Owed).await;
        }
    }

    /// After an iPhone background refresh's go-live check: what is owed, in
    /// the time the refresh has `left`. Under `POLL_FLUSH_MIN` it starts
    /// nothing (the change stays owed for the next wake or launch).
    #[cfg(target_os = "ios")]
    pub async fn flush_if_owed(left: Duration) {
        if left < POLL_FLUSH_MIN || !owed_now() {
            return;
        }
        let _ = flush(left.min(FLUSH_BUDGET), push_plan::PostReason::Owed).await;
    }

    /// After the Android worker's check (this process may hold no app at all).
    #[cfg(target_os = "android")]
    pub async fn worker_pass() {
        let device = device();
        let now = now_ms();
        let post = with_state(|s| push_plan::should_post(s, now, app_version(), device.as_ref()));
        if post {
            let _ = flush(FLUSH_BUDGET, push_plan::PostReason::Due).await;
        }
    }

    /// Bring the server in step now when it is owed, the device changed, or
    /// the last send is six hours old (Android calls it at every launch).
    #[tauri::command]
    pub async fn push_register() -> Result<String, String> {
        let device = device();
        let now = now_ms();
        let post = with_state(|s| push_plan::should_post(s, now, app_version(), device.as_ref()));
        if !post {
            return Ok("current".to_string());
        }
        flush(FLUSH_BUDGET, push_plan::PostReason::Due).await
    }

    /// Send what is owed, answering by `deadline_ms` (an absolute Unix time in
    /// milliseconds; never more than 25 s away). The iPhone calls it from a
    /// background task when the app leaves the screen, with what iOS left it
    /// minus a margin. A deadline already past answers at once.
    #[tauri::command]
    pub async fn push_flush(deadline_ms: u64) -> Result<String, String> {
        let left = deadline_ms.saturating_sub(now_ms());
        if left == 0 {
            return Ok("no time".to_string());
        }
        if !owed_now() {
            return Ok("current".to_string());
        }
        flush(Duration::from_millis(left).min(FLUSH_BUDGET), push_plan::PostReason::Owed).await
    }

    /// The iPhone's APNs device, as the system reported it: the token in hex,
    /// `sandbox` or `production`, and the app's bundle id as the topic. Kept
    /// for every later send, then sent at once unless the same device went
    /// out in the last ten minutes with nothing owed.
    #[cfg(target_os = "ios")]
    #[tauri::command]
    pub async fn push_set_apns_device(token: String, env: String, topic: String) -> Result<String, String> {
        let device = PushDevice::apns(&token, &env, &topic)?;
        let path = cache_service::get_app_data_dir().map_err(|e| e.to_string())?.join(APNS_FILE);
        let stored = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<PushDevice>(&s).ok());
        if stored.as_ref() != Some(&device) {
            let json = serde_json::to_string(&device).map_err(|e| e.to_string())?;
            std::fs::write(&path, json).map_err(|e| e.to_string())?;
        }
        let now = now_ms();
        if with_state(|s| push_plan::apns_repeat_skip(s, &device, now, app_version())) {
            return Ok("unchanged".to_string());
        }
        flush(FLUSH_BUDGET, push_plan::PostReason::DeviceReport).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn follows() -> Vec<(String, String)> {
        [("1", "alpha"), ("2", "Bravo"), ("3", "charlie"), ("2", "bravo"), ("", "ghost")]
            .iter()
            .map(|(id, login)| (id.to_string(), login.to_string()))
            .collect()
    }

    fn inputs(enabled: bool, live: bool, push: bool, muted: &[&str]) -> AlertInputs {
        let notif = json!({
            "enabled": enabled,
            "show_live_notifications": live,
            "push_notifications": push,
            "muted_live_channels": muted,
        });
        AlertInputs::from_json(Some(&notif))
    }

    #[test]
    fn a_change_is_a_switch_a_mute_or_nothing() {
        use crate::services::push_plan::Kind;
        let mut cell = None;
        assert_eq!(note_change(&mut cell, inputs(true, true, true, &[])), Some(Kind::Switch), "first save");
        assert_eq!(note_change(&mut cell, inputs(true, true, true, &[])), None);
        assert_eq!(note_change(&mut cell, inputs(true, true, true, &["a"])), Some(Kind::MuteOnly));
        assert_eq!(note_change(&mut cell, inputs(true, false, true, &["a"])), Some(Kind::Switch));
        assert_eq!(note_change(&mut cell, inputs(true, false, true, &["A"])), None, "case is not a change");
    }

    /// The body of the first function in this file whose signature starts
    /// with `sig` (the phone module's functions, indented four spaces).
    fn phone_fn_body(sig: &str) -> String {
        let src = include_str!("push_alerts.rs").replace("\r\n", "\n");
        let start = src.find(sig).unwrap_or_else(|| panic!("{sig} not found"));
        let rest = &src[start..];
        rest[..rest.find("\n    }\n").expect("function end")].to_string()
    }

    #[test]
    fn each_door_into_the_runner_names_why_it_may_post() {
        assert!(phone_fn_body("pub async fn push_set_apns_device(").contains("PostReason::DeviceReport"), "the device report heals after ten minutes");
        assert!(phone_fn_body("pub async fn push_register(").contains("PostReason::Due"));
        assert!(phone_fn_body("pub async fn worker_pass(").contains("PostReason::Due"));
        assert!(phone_fn_body("pub async fn push_flush(").contains("PostReason::Owed"));
        assert!(phone_fn_body("async fn pass(").contains("push_plan::pass_force(force, leftover_action)"), "a leftover left to the register forces it");
        assert!(phone_fn_body("pub fn refresh(").contains("note_change("));
    }

    #[test]
    fn a_restored_install_forgets_the_old_phones_push_state_and_device() {
        let body = phone_fn_body("pub fn forget_restored_state(");
        assert!(body.contains("remove_file(path)") && body.contains("APNS_FILE"));
        let lib = include_str!("../lib.rs").replace("\r\n", "\n");
        let reconcile = lib.find("install_marker::reconcile(&data, &mut loaded)").expect("restore check");
        let forget = lib.find("push_alerts::server::forget_restored_state()").expect("restore forgets push state");
        let seed = lib.find("push_alerts::server::seed(&loaded)").expect("seed");
        assert!(reconcile < forget && forget < seed, "forgotten after the restore check, before seed reads it");
    }

    #[test]
    fn two_saves_whose_hooks_run_out_of_order_never_hide_the_next_mute() {
        use crate::services::push_plan::Kind;
        // Muted [a]; an unrelated save (T1) copied the settings, then an
        // unmute of a (T2) saved []. T2's hook runs first, T1's second. Both
        // hooks get the live settings ([]), never T1's stale copy ([a]):
        // `after_settings_change` reads them (pinned in commands::settings).
        let mut cell = Some(inputs(true, true, true, &["a"]));
        assert_eq!(note_change(&mut cell, inputs(true, true, true, &[])), Some(Kind::MuteOnly), "T2: the unmute");
        assert_eq!(note_change(&mut cell, inputs(true, true, true, &[])), None, "T1, late, reads the live []");
        assert_eq!(note_change(&mut cell, inputs(true, true, true, &["a"])), Some(Kind::MuteOnly), "muting a again is seen");
    }

    #[test]
    fn every_followed_channel_alerts_once_in_follow_order() {
        let on = inputs(true, true, true, &[]);
        for platform in ["android", "ios"] {
            assert_eq!(follow_list(platform, &on, &follows()), Some(vec!["1".into(), "2".into(), "3".into()]), "{platform}");
        }
    }

    #[test]
    fn the_list_is_the_first_thousand_follows_minus_the_muted_so_a_mute_is_always_a_removal() {
        let many: Vec<(String, String)> = (0..1500).map(|i| (i.to_string(), format!("c{i}"))).collect();
        let all = follow_list("ios", &inputs(true, true, true, &[]), &many).unwrap();
        assert_eq!(all.len(), 1000);
        assert_eq!(all.last().map(String::as_str), Some("999"));
        let less = follow_list("ios", &inputs(true, true, true, &["c500"]), &many).unwrap();
        assert_eq!(less.len(), 999);
        assert!(less.iter().all(|id| all.contains(id)), "muting never pulls follow 1001 in");
    }

    #[test]
    fn muted_channels_are_left_out_whatever_their_case() {
        let some = inputs(true, true, true, &["BRAVO", " charlie "]);
        assert_eq!(follow_list("ios", &some, &follows()), Some(vec!["1".into()]));
        assert_eq!(follow_list("android", &some, &follows()), Some(vec!["1".into()]));
    }

    #[test]
    fn master_or_live_off_keeps_the_device_with_nothing_to_send() {
        for (enabled, live) in [(false, true), (true, false), (false, false)] {
            let off = inputs(enabled, live, true, &[]);
            assert_eq!(follow_list("ios", &off, &follows()), Some(vec![]));
            assert_eq!(follow_list("android", &off, &follows()), Some(vec![]));
            assert_eq!(plan("ios", &off), Plan::Hold { alerting: false });
        }
    }

    #[test]
    fn instant_alerts_off_drops_an_android_device_and_is_not_read_on_iphone() {
        let no_push = inputs(true, true, false, &[]);
        assert_eq!(plan("android", &no_push), Plan::Drop);
        assert_eq!(follow_list("android", &no_push, &follows()), None);
        assert_eq!(follow_list("ios", &no_push, &follows()), Some(vec!["1".into(), "2".into(), "3".into()]));
    }

    #[test]
    fn both_phones_send_go_live_alerts() {
        assert!(crate::services::alert_types::works_on("live", "ios"));
        assert!(crate::services::alert_types::works_on("live", "android"));
    }

    #[test]
    fn unsaved_switches_read_as_on() {
        let fresh = AlertInputs::from_json(None);
        assert_eq!(fresh, AlertInputs { enabled: true, live: true, push: true, muted: vec![] });
        let defaults = AlertInputs::from_settings(&crate::models::settings::Settings::default());
        assert_eq!(defaults, fresh);
    }

    #[test]
    fn muted_lists_compare_by_content() {
        assert_eq!(inputs(true, true, true, &["b", "A"]), inputs(true, true, true, &["a", "B", "a"]));
        assert_ne!(inputs(true, true, true, &["a"]), inputs(true, true, true, &[]));
    }

    #[test]
    fn apns_body_is_the_contract() {
        let device = PushDevice::apns("ABCDEF0123", "sandbox", "XTL-C84UQRPAQ2.app.streamnook").unwrap();
        assert_eq!(
            device.register_body(&["123".into(), "456".into()]),
            json!({
                "kind": "apns",
                "token": "abcdef0123",
                "env": "sandbox",
                "topic": "XTL-C84UQRPAQ2.app.streamnook",
                "follows": ["123", "456"],
            })
        );
        assert_eq!(device.unregister_body(), json!({ "kind": "apns", "token": "abcdef0123", "env": "sandbox", "topic": "XTL-C84UQRPAQ2.app.streamnook" }));
        assert_eq!(device.register_body(&[])["follows"], json!([]));
    }

    #[test]
    fn fcm_body_still_reads_as_the_old_android_body() {
        let device = PushDevice::Fcm { token: "fcm-abc".into() };
        let body = device.register_body(&["9".into()]);
        assert_eq!(body["kind"], "fcm");
        assert_eq!(body["token"], "fcm-abc");
        // What the server before `kind` reads: {fcm_token, follows}.
        assert_eq!(body["fcm_token"], "fcm-abc");
        assert_eq!(body["follows"], json!(["9"]));
        assert!(body.get("env").is_none() && body.get("topic").is_none());
        let gone = device.unregister_body();
        assert_eq!((gone["kind"].as_str(), gone["token"].as_str(), gone["fcm_token"].as_str()), (Some("fcm"), Some("fcm-abc"), Some("fcm-abc")));
        assert!(gone.get("follows").is_none());
    }

    #[test]
    fn apns_devices_are_checked() {
        assert!(PushDevice::apns("", "sandbox", "app.streamnook").is_err());
        assert!(PushDevice::apns("xyz1", "sandbox", "app.streamnook").is_err());
        assert!(PushDevice::apns("abc", "sandbox", "app.streamnook").is_err());
        assert!(PushDevice::apns("ab12", "development", "app.streamnook").is_err());
        assert!(PushDevice::apns("ab12", "production", " ").is_err());
        assert!(PushDevice::apns("ab12", "production", "app.streamnook").is_ok());
    }

    #[test]
    fn a_stored_device_reads_back() {
        let device = PushDevice::apns("ab12", "production", "app.streamnook").unwrap();
        let text = serde_json::to_string(&device).unwrap();
        assert_eq!(serde_json::from_str::<PushDevice>(&text).unwrap(), device);
    }
}
