//! Keeps Continue Watching in step across a viewer's devices, so a VOD left
//! on one StreamNook install shows up and resumes on another.
//!
//! `vod_progress_service` stays the canonical copy (instant, offline, per
//! account). Two remote copies are kept in step with it:
//!
//! - **StreamNook's account copy** (`POST /api/v1/vod/sync`) carries the CARD
//!   LIST between StreamNook installs. It holds only what the app sends, and
//!   the app sends only `may_sync` entries (deliberately opened VODs), so a
//!   pulled row is always a real card. One request both sends this device's
//!   changes since its cursor (positions and dismissals) and returns every row
//!   another device changed. Newest watch time wins per video.
//! - **Twitch's watch history** (`updateUserViewedVideo`, `viewingHistory`)
//!   carries POSITIONS to Twitch's own apps and back. It cannot carry cards:
//!   Twitch records every broadcast watched LIVE in the same list, with nothing
//!   to tell it from an opened VOD (the first build imported that list and
//!   filled the row with every stream ever watched). So Twitch is written to,
//!   and read only for a VOD being opened here (`pull_one`).
//!
//! When it runs:
//!
//! - **Stopping.** `note_recorded` is called only by `report_vod_position`,
//!   which the player's deliberate-VOD reporter drives. One worker task per app
//!   saves `QUIET` after the last report, so a pause, a close or a switch lands
//!   once. A VOD that keeps playing saves only every `MAX_GAP`, as a net for a
//!   crash: the stop and the exit are what carry the position.
//! - **Exit.** `start_exit_sync` sends whatever is unsent while the app shuts
//!   down, bounded, so quitting mid-VOD on one device still lands it for the
//!   next. Nothing unsent, nothing sent.
//! - **Home.** `pull` syncs the account copy on mount, at most once per
//!   `PULL_GAP`, then pushes to Twitch whatever it has not seen (watched while
//!   sync was off, offline, or the last seconds before the app quit).
//! - **Open.** `sync_on_open` reads the account copy and the VOD's Twitch
//!   position while the VOD resolves, so a VOD paused elsewhere a minute ago
//!   opens at that spot.
//! - **Dismiss.** `note_dismissed` sends a dismissed card shortly after, so it
//!   leaves the viewer's other devices too.
//!
//! ## Which token
//!
//! StreamNook's account copy: the app's own Twitch sign-in as a bearer
//! (`streamnook_api::post_json`); the server derives the account from it. Twitch:
//! the twitch.tv web session the sign-in created (the Recap pattern,
//! `TwitchAuthService`), since Twitch's GQL refuses our client ids for this. No
//! new scope, no sign-in step. For Twitch, the session's account must be the
//! account the store belongs to, or nothing moves.
//!
//! ## Cost
//!
//! Idle and tray: nothing runs (no loop; the worker exits once nothing is
//! pending). A VOD session: one request to each copy when it stops, plus one
//! per `MAX_GAP` only while it keeps playing. A Home mount: one request when
//! the last sync is older than `PULL_GAP`. Quitting: one, only if anything is
//! unsent.
//! Phones have no VOD playback or Continue Watching row yet, so the account
//! sync is desktop-only until they do.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use std::sync::atomic::{AtomicBool, Ordering};

use log::{debug, warn};
use serde_json::{json, Value};
use tauri::Manager;

use crate::models::settings::AppState;
use crate::rt::AppHandle;
use crate::services::auth_proxy;
use crate::services::twitch_service::{vod_refresh_from_node, VOD_CARD_FIELDS};
use crate::services::vod_progress_service::{self, RemoteOrigin, RemoteProgress};

/// A report this long ago with none since means playback stopped: save now.
const QUIET: Duration = Duration::from_secs(10);
/// While a VOD keeps playing, save at least this often. Only a net for a crash
/// or a kill: the stop and the exit are what carry the position, so this is
/// long on purpose (each save is a request to StreamNook and to Twitch).
const MAX_GAP: Duration = Duration::from_secs(10 * 60);
/// How often the worker looks at what is pending.
const TICK: Duration = Duration::from_secs(5);
/// Floor between two account syncs a Home mount starts. The first mount after
/// launch always syncs.
const PULL_GAP: Duration = Duration::from_secs(5 * 60);
/// StreamNook's account-copy endpoint (allowlisted in `streamnook_api`).
const CLOUD_PATH: &str = "/api/v1/vod/sync";
/// Most changes one sync sends (the endpoint's own limit).
const CLOUD_BATCH: usize = 50;
/// Bound on reading the web session (it can wait on the UI thread).
const TOKEN_BUDGET: Duration = Duration::from_secs(10);
/// After Twitch refuses a request, leave it alone this long.
const FAILURE_BACKOFF: Duration = Duration::from_secs(10 * 60);
/// Most catch-up saves one pull sends.
const CATCH_UP_LIMIT: usize = 24;

static APP: std::sync::OnceLock<AppHandle> = std::sync::OnceLock::new();

#[derive(Default)]
struct PushState {
    /// Video id -> newest position not yet saved.
    pending: HashMap<String, Pending>,
    /// Video id -> the last position saved this session, so an unchanged
    /// playhead (paused, then the stop report) is not saved twice.
    last_saved: HashMap<String, i64>,
    worker: bool,
}

struct Pending {
    position: i64,
    /// Store `updated_at` of the report, for `mark_synced`.
    recorded_at: i64,
    noted: Instant,
    /// When this video was last saved, or first noted since then.
    since_save: Instant,
}

static PUSH: Mutex<Option<PushState>> = Mutex::new(None);

/// When Twitch last refused us.
static FAILED_AT: Mutex<Option<Instant>> = Mutex::new(None);
/// The last account sync that landed, and when the account copy last refused.
static PULLED_AT: Mutex<Option<Instant>> = Mutex::new(None);
static CLOUD_FAILED_AT: Mutex<Option<Instant>> = Mutex::new(None);
/// One account sync at a time. A trigger meeting one in flight sets
/// `CLOUD_RERUN`, and the running sync makes one more pass instead of a second
/// task starting.
static CLOUD_IN_FLIGHT: AtomicBool = AtomicBool::new(false);
static CLOUD_RERUN: AtomicBool = AtomicBool::new(false);
/// A dismissal sync is already scheduled; it will carry any dismissal made
/// before it runs, so a run of dismissals costs one request.
static DISMISS_PENDING: AtomicBool = AtomicBool::new(false);
/// The account behind the last web-session token, so a save needs no extra
/// round trip for `userID`.
static SESSION_USER: Mutex<Option<(String, String)>> = Mutex::new(None);

pub fn init(app: AppHandle) {
    let _ = APP.set(app);
}

/// Sync is on: "Resume VODs" and its "Device sync" sub-setting both.
fn enabled() -> bool {
    let Some(state) = APP.get().and_then(|a| a.try_state::<AppState>()) else {
        return false;
    };
    let settings = state.settings.lock().unwrap();
    settings.video_player.resume_vod_playback && settings.video_player.sync_vod_progress
}

fn backing_off() -> bool {
    FAILED_AT
        .lock()
        .ok()
        .and_then(|g| *g)
        .is_some_and(|at| at.elapsed() < FAILURE_BACKOFF)
}

fn note_failure(what: &str, detail: &str) {
    let first = FAILED_AT.lock().map(|mut g| g.replace(Instant::now()).is_none()).unwrap_or(false);
    // The first refusal of a run is worth seeing in a log; repeats are not.
    if first {
        warn!("[VodSync] {what} refused, pausing sync for 10 min: {detail}");
    } else {
        debug!("[VodSync] {what} refused: {detail}");
    }
}

/// The web-session token and its account, when that account owns the store.
async fn session() -> Option<(String, String)> {
    let owner = vod_progress_service::owner_id()?;
    let auth = APP.get()?.try_state::<AppState>().map(|s| s.twitch_auth.clone())?;
    let token = match tokio::time::timeout(TOKEN_BUDGET, auth.get_token()).await {
        Ok(Ok(t)) => t,
        Ok(Err(e)) => {
            debug!("[VodSync] no web session: {e}");
            return None;
        }
        Err(_) => {
            debug!("[VodSync] web session read timed out");
            return None;
        }
    };
    let cached = SESSION_USER
        .lock()
        .ok()
        .and_then(|g| g.clone())
        .filter(|(t, _)| *t == token)
        .map(|(_, id)| id);
    let user_id = match cached {
        Some(id) => id,
        None => {
            let resp = auth_proxy::gql_query(&token, json!({ "query": "query { currentUser { id } }" }))
                .await
                .ok()?;
            let id = resp.pointer("/data/currentUser/id").and_then(Value::as_str)?.to_string();
            if let Ok(mut g) = SESSION_USER.lock() {
                *g = Some((token.clone(), id.clone()));
            }
            id
        }
    };
    if user_id != owner {
        debug!("[VodSync] web session is a different account than the store; not syncing");
        return None;
    }
    Some((token, user_id))
}

/// A deliberate VOD position was just recorded. Queue its save.
pub fn note_recorded(video_id: &str, position_secs: f64, recorded_at: i64) {
    if !position_secs.is_finite() || !enabled() || vod_progress_service::owner_id().is_none() {
        return;
    }
    let Ok(mut guard) = PUSH.lock() else { return };
    let state = guard.get_or_insert_with(PushState::default);
    let now = Instant::now();
    let since_save = state
        .pending
        .get(video_id)
        .map(|p| p.since_save)
        .unwrap_or(now);
    state.pending.insert(
        video_id.to_string(),
        Pending {
            position: position_secs.floor() as i64,
            recorded_at,
            noted: now,
            since_save,
        },
    );
    if !state.worker {
        state.worker = true;
        tauri::async_runtime::spawn(push_worker());
    }
}

/// What is due to be saved now. Pure over the state so the cadence is testable.
fn take_due(state: &mut PushState, now: Instant) -> Vec<(String, i64, i64)> {
    let due: Vec<String> = state
        .pending
        .iter()
        .filter(|(_, p)| now.duration_since(p.noted) >= QUIET || now.duration_since(p.since_save) >= MAX_GAP)
        .map(|(id, _)| id.clone())
        .collect();
    let mut out = Vec::new();
    for id in due {
        let Some(p) = state.pending.remove(&id) else { continue };
        if state.last_saved.get(&id) == Some(&p.position) {
            continue;
        }
        out.push((id, p.position, p.recorded_at));
    }
    out
}

async fn push_worker() {
    loop {
        tokio::time::sleep(TICK).await;
        let due = {
            let Ok(mut guard) = PUSH.lock() else { return };
            let Some(state) = guard.as_mut() else { return };
            let due = take_due(state, Instant::now());
            if due.is_empty() && state.pending.is_empty() {
                // Nothing playing: the worker leaves, and the next report
                // starts a fresh one.
                state.worker = false;
                state.last_saved.clear();
                return;
            }
            due
        };
        if due.is_empty() || !enabled() {
            continue;
        }
        if cloud_sync(true).await {
            crate::services::home_snapshot::note_progress_changed();
        }
        if backing_off() {
            continue;
        }
        let Some((token, user_id)) = session().await else { continue };
        for (video_id, position, recorded_at) in due {
            // Dismissed while the save waited, or not a position that may
            // leave the device: leave Twitch alone.
            let id = video_id.clone();
            let allowed = tokio::task::spawn_blocking(move || {
                vod_progress_service::lookup(&id).is_some_and(|e| vod_progress_service::may_sync(&e))
            })
            .await
            .unwrap_or(false);
            if !allowed {
                continue;
            }
            match save(&token, &user_id, &video_id, position).await {
                Ok(()) => {
                    if let Ok(mut guard) = PUSH.lock() {
                        if let Some(state) = guard.as_mut() {
                            state.last_saved.insert(video_id.clone(), position);
                        }
                    }
                    let id = video_id.clone();
                    let _ = tokio::task::spawn_blocking(move || {
                        vod_progress_service::mark_synced(&id, recorded_at)
                    })
                    .await;
                }
                Err(e) => {
                    note_failure("save", &e);
                    break;
                }
            }
        }
    }
}

/// One `updateUserViewedVideo`. Every archive, highlight and upload is `VOD`
/// to Twitch (its `VideoType` enum is only `VOD` and `LIVE`), and StreamNook
/// never saves `LIVE`.
async fn save(token: &str, user_id: &str, video_id: &str, position: i64) -> Result<(), String> {
    let body = json!({
        "operationName": "StreamNookSaveVodPosition",
        "query": "mutation StreamNookSaveVodPosition($input: UpdateUserViewedVideoInput!) { \
                  updateUserViewedVideo(input: $input) { video { id } } }",
        "variables": { "input": {
            "userID": user_id,
            "videoID": video_id,
            "position": position.max(0),
            "videoType": "VOD",
        } },
    });
    let resp = auth_proxy::gql_query(token, body)
        .await
        .map_err(|e| format!("{e:#}"))?;
    if let Some(errors) = resp.get("errors").filter(|e| !e.is_null()) {
        return Err(errors.to_string());
    }
    debug!("[VodSync] saved {video_id} at {position}s");
    Ok(())
}

/// Twitch's `updatedAt` as Unix seconds.
fn parse_time(v: Option<&Value>) -> Option<i64> {
    let s = v?.as_str()?;
    chrono::DateTime::parse_from_rfc3339(s).ok().map(|t| t.timestamp())
}

/// One viewed VOD from either read, or None when it carries no position.
fn remote_row(node: &Value, history: &Value) -> Option<RemoteProgress> {
    let id = node.get("id").and_then(Value::as_str)?;
    let position = history.get("position").and_then(Value::as_f64)?;
    let updated_at = parse_time(history.get("updatedAt"))?;
    Some(RemoteProgress {
        position_secs: position,
        updated_at,
        channel_login: node
            .pointer("/owner/login")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        meta: vod_refresh_from_node(id, node),
    })
}

/// Home's sync: the account copy (at most once per `PULL_GAP`), then whatever
/// Twitch has not seen. Returns true when the store changed.
pub async fn pull() -> bool {
    if cfg!(mobile) || !enabled() {
        return false;
    }
    let changed = cloud_sync(false).await;
    if !backing_off() {
        if let Some((token, user_id)) = session().await {
            catch_up(&token, &user_id).await;
        }
    }
    changed
}

/// A VOD is opening: fold in where the viewer got to on their other devices
/// (account copy) and in Twitch's apps (its Twitch position). The caller
/// bounds the wait; a slow answer only means the local position wins.
pub async fn sync_on_open(video_id: &str) {
    // Its own task, so the caller's timeout never cuts the account sync off
    // halfway: a late answer still lands for the next open and for Home.
    let cloud = tauri::async_runtime::spawn(cloud_sync(true));
    pull_one(video_id).await;
    let _ = cloud.await;
}

/// A card was dismissed or started over: send it soon, so it leaves the
/// viewer's other devices too.
pub fn note_dismissed() {
    if cfg!(mobile) || !enabled() {
        return;
    }
    if DISMISS_PENDING.swap(true, Ordering::AcqRel) {
        return;
    }
    tauri::async_runtime::spawn(async {
        tokio::time::sleep(Duration::from_secs(2)).await;
        // Cleared BEFORE the sync, so a dismissal arriving during it schedules
        // the next one instead of being swallowed.
        DISMISS_PENDING.store(false, Ordering::Release);
        cloud_sync(true).await;
    });
}

fn cloud_backing_off() -> bool {
    CLOUD_FAILED_AT
        .lock()
        .ok()
        .and_then(|g| *g)
        .is_some_and(|at| at.elapsed() < FAILURE_BACKOFF)
}

/// One change as the endpoint takes it.
fn change_json(c: &vod_progress_service::CloudChange) -> Value {
    let opt = |s: &str| if s.is_empty() { Value::Null } else { Value::String(s.to_string()) };
    json!({
        "video_id": c.video_id,
        "position_secs": c.position_secs.max(0.0),
        "duration_secs": c.duration_secs.max(0.0),
        "watched_at": chrono::DateTime::from_timestamp(c.at, 0)
            .map(|t| t.to_rfc3339())
            .unwrap_or_default(),
        "dismissed": c.dismissed,
        "channel_login": opt(&c.channel_login),
        "channel_name": opt(&c.channel_name),
        "title": opt(&c.title),
        "thumbnail_url": opt(&c.thumbnail_url),
    })
}

/// An account-copy answer: the positions to merge, the dismissals to apply,
/// and the server time to use as the next cursor.
fn cloud_rows(resp: &Value) -> (Vec<RemoteProgress>, Vec<(String, i64)>, Option<String>) {
    let now = resp.get("now").and_then(Value::as_str).map(str::to_string);
    let mut positions = Vec::new();
    let mut dismissals = Vec::new();
    for row in resp.get("rows").and_then(Value::as_array).into_iter().flatten() {
        let Some(id) = row.get("video_id").and_then(Value::as_str) else { continue };
        let Some(at) = parse_time(row.get("watched_at")) else { continue };
        if row.get("dismissed").and_then(Value::as_bool).unwrap_or(false) {
            dismissals.push((id.to_string(), at));
            continue;
        }
        let Some(position) = row.get("position_secs").and_then(Value::as_f64) else { continue };
        let text = |k: &str| row.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        positions.push(RemoteProgress {
            position_secs: position,
            updated_at: at,
            channel_login: text("channel_login"),
            meta: vod_progress_service::VodRefresh {
                video_id: id.to_string(),
                title: text("title"),
                channel_name: text("channel_name"),
                thumbnail_url: text("thumbnail_url"),
                length_secs: row.get("duration_secs").and_then(Value::as_f64).unwrap_or(0.0),
                // Unknown from here; the Home hydrate settles it.
                finished: false,
                profile_image_url: String::new(),
                partner: None,
                game_name: String::new(),
                channel_id: String::new(),
            },
        });
    }
    (positions, dismissals, now)
}

/// Send this device's changes to StreamNook's account copy and fold in what the
/// viewer's other devices changed. `force` skips the `PULL_GAP` floor (a VOD
/// stopping or opening). Returns true when the store changed.
async fn cloud_sync(force: bool) -> bool {
    if cfg!(mobile) || !enabled() || cloud_backing_off() || vod_progress_service::owner_id().is_none() {
        return false;
    }
    let recent = PULLED_AT
        .lock()
        .ok()
        .and_then(|g| *g)
        .is_some_and(|at| at.elapsed() < PULL_GAP);
    if !force && recent {
        return false;
    }
    if CLOUD_IN_FLIGHT.swap(true, Ordering::AcqRel) {
        CLOUD_RERUN.store(true, Ordering::Release);
        return false;
    }
    // Cleared on drop, so a cancelled sync can never wedge every later one.
    struct InFlight;
    impl Drop for InFlight {
        fn drop(&mut self) {
            CLOUD_IN_FLIGHT.store(false, Ordering::Release);
        }
    }
    let mut changed = false;
    let mut passes = 0;
    loop {
        {
            let _guard = InFlight;
            changed |= cloud_round_trip().await;
        }
        passes += 1;
        // A trigger met this sync while it ran: one more pass carries what it
        // recorded. Capped, so a burst of triggers is still a few requests.
        if passes >= 3 || !CLOUD_RERUN.swap(false, Ordering::AcqRel) {
            break;
        }
        if CLOUD_IN_FLIGHT.swap(true, Ordering::AcqRel) {
            // Another trigger started its own sync in the gap; it covers this.
            break;
        }
    }
    changed
}

async fn cloud_round_trip() -> bool {
    let Some(outbox) = tokio::task::spawn_blocking(|| vod_progress_service::cloud_outbox(CLOUD_BATCH))
        .await
        .ok()
        .flatten()
    else {
        return false;
    };
    let body = json!({
        "since": outbox.since,
        "entries": outbox.changes.iter().map(change_json).collect::<Vec<_>>(),
    });
    let resp = match crate::commands::streamnook_api::post_json(CLOUD_PATH, &body, None).await {
        Ok(r) => r,
        Err(e) => {
            debug!("[VodSync] account sync did not complete: {e}");
            return false;
        }
    };
    if !resp.ok {
        // 401 / 403 / 409 will not fix themselves on the next tick.
        if let Ok(mut g) = CLOUD_FAILED_AT.lock() {
            if g.replace(Instant::now()).is_none() {
                warn!("[VodSync] account sync refused ({}), pausing it for 10 min: {}", resp.status, resp.body);
            }
        }
        return false;
    }
    let Ok(answer) = serde_json::from_str::<Value>(&resp.body) else { return false };
    let (positions, dismissals, now) = cloud_rows(&answer);
    let Some(now) = now else { return false };
    if let Ok(mut g) = PULLED_AT.lock() {
        *g = Some(Instant::now());
    }
    let cutoff = outbox.cutoff;
    let sent = outbox.changes.len();
    tokio::task::spawn_blocking(move || {
        debug!(
            "[VodSync] account sync: sent {sent}, got {} positions + {} dismissals",
            positions.len(),
            dismissals.len()
        );
        // Every row here is a deliberate watch on one of the viewer's devices.
        let merged = vod_progress_service::merge_remote(&positions, RemoteOrigin::Opened);
        let dismissed = vod_progress_service::apply_remote_dismissals(&dismissals);
        vod_progress_service::cloud_commit(cutoff, now);
        merged || dismissed
    })
    .await
    .unwrap_or(false)
}

/// The app is quitting: send whatever this device has not sent yet. Returns a
/// receiver that fires when the sync finished (or was skipped), so the exit
/// path can wait on it next to its other shutdown work, bounded. None when
/// nothing is unsent, which is the usual case and costs no request.
pub fn start_exit_sync() -> Option<std::sync::mpsc::Receiver<()>> {
    if cfg!(mobile) || !enabled() || cloud_backing_off() || vod_progress_service::owner_id().is_none() {
        return None;
    }
    let unsent = vod_progress_service::cloud_outbox(1).is_some_and(|o| !o.changes.is_empty());
    if !unsent {
        return None;
    }
    let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
    tauri::async_runtime::spawn(async move {
        cloud_sync(true).await;
        // The stores were flushed before this landed; write the moved cursor
        // too, or the next launch resends the same batch (harmless, wasted).
        let _ = vod_progress_service::flush_now();
        let _ = done_tx.send(());
    });
    Some(done_rx)
}

/// Push row entries holding a position Twitch has not seen.
async fn catch_up(token: &str, user_id: &str) {
    let rows = tokio::task::spawn_blocking(|| vod_progress_service::unsynced(CATCH_UP_LIMIT))
        .await
        .unwrap_or_default();
    for p in rows.into_iter().filter(vod_progress_service::may_sync) {
        // A save the worker is about to send covers this one.
        let queued = PUSH
            .lock()
            .ok()
            .and_then(|g| g.as_ref().map(|s| s.pending.contains_key(&p.video_id)))
            .unwrap_or(false);
        if queued {
            continue;
        }
        match save(token, user_id, &p.video_id, p.position_secs.floor() as i64).await {
            Ok(()) => {
                let (id, at) = (p.video_id.clone(), p.updated_at);
                let _ = tokio::task::spawn_blocking(move || vod_progress_service::mark_synced(&id, at)).await;
            }
            Err(e) => {
                note_failure("catch-up save", &e);
                return;
            }
        }
    }
}

/// Read one VOD's Twitch position while it opens and fold it into the store,
/// so the resume decision that follows sees a position saved elsewhere. The
/// caller bounds the wait; a slow answer just means the local position wins.
pub async fn pull_one(video_id: &str) {
    if !enabled() || backing_off() {
        return;
    }
    let Some((token, user_id)) = session().await else { return };
    let query = format!(
        "query StreamNookViewedVideo($id: ID!) {{ currentUser {{ id }} \
         video(id: $id) {{ {VOD_CARD_FIELDS} self {{ viewingHistory {{ position updatedAt }} }} }} }}"
    );
    let body = json!({
        "operationName": "StreamNookViewedVideo",
        "query": query,
        "variables": { "id": video_id },
    });
    let resp = match auth_proxy::gql_query(&token, body).await {
        Ok(r) => r,
        Err(e) => {
            debug!("[VodSync] position read failed: {e:#}");
            return;
        }
    };
    if resp.pointer("/data/currentUser/id").and_then(Value::as_str) != Some(user_id.as_str()) {
        return;
    }
    let row = resp.pointer("/data/video").filter(|n| !n.is_null()).and_then(|node| {
        let history = node.pointer("/self/viewingHistory").filter(|h| !h.is_null())?;
        remote_row(node, history)
    });
    if let Some(row) = row {
        // The viewer opened this VOD here, so the read is deliberate.
        let _ = tokio::task::spawn_blocking(move || vod_progress_service::merge_remote(&[row], RemoteOrigin::Opened)).await;
    }
}

/// The account changed or signed out: forget whose session we cached and let
/// the next Home mount pull for the new account at once.
pub fn reset() {
    if let Ok(mut g) = SESSION_USER.lock() {
        *g = None;
    }
    if let Ok(mut g) = PULLED_AT.lock() {
        *g = None;
    }
    if let Ok(mut g) = FAILED_AT.lock() {
        *g = None;
    }
    if let Ok(mut g) = CLOUD_FAILED_AT.lock() {
        *g = None;
    }
    if let Ok(mut g) = PUSH.lock() {
        if let Some(state) = g.as_mut() {
            state.pending.clear();
            state.last_saved.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_with(id: &str, position: i64, noted_ago: Duration, saved_ago: Duration, now: Instant) -> PushState {
        let mut s = PushState::default();
        s.pending.insert(
            id.to_string(),
            Pending {
                position,
                recorded_at: 1,
                noted: now - noted_ago,
                since_save: now - saved_ago,
            },
        );
        s
    }

    #[test]
    fn a_report_still_arriving_waits_for_the_quiet_window() {
        let now = Instant::now();
        let mut s = state_with("1", 100, Duration::from_secs(3), Duration::from_secs(20), now);
        assert!(take_due(&mut s, now).is_empty());
        assert_eq!(s.pending.len(), 1);
    }

    #[test]
    fn a_stopped_vod_saves_after_the_quiet_window() {
        let now = Instant::now();
        let mut s = state_with("1", 100, Duration::from_secs(11), Duration::from_secs(11), now);
        assert_eq!(take_due(&mut s, now), vec![("1".to_string(), 100, 1)]);
        assert!(s.pending.is_empty());
    }

    #[test]
    fn a_vod_that_keeps_playing_saves_every_max_gap() {
        let now = Instant::now();
        let mut s = state_with("1", 100, Duration::from_secs(2), MAX_GAP + Duration::from_secs(1), now);
        assert_eq!(take_due(&mut s, now).len(), 1);
    }

    #[test]
    fn a_vod_that_keeps_playing_does_not_save_before_max_gap() {
        let now = Instant::now();
        let mut s = state_with("1", 100, Duration::from_secs(2), Duration::from_secs(120), now);
        assert!(take_due(&mut s, now).is_empty());
    }

    #[test]
    fn an_unchanged_position_is_not_saved_twice() {
        let now = Instant::now();
        let mut s = state_with("1", 100, Duration::from_secs(11), Duration::from_secs(11), now);
        s.last_saved.insert("1".to_string(), 100);
        assert!(take_due(&mut s, now).is_empty());
    }

    #[test]
    fn a_twitch_read_takes_the_position_and_card_fields() {
        let node = json!({
            "id": "10", "title": "t", "lengthSeconds": 7200, "status": "RECORDED",
            "owner": { "id": "9", "login": "Caedrel", "displayName": "Caedrel" }
        });
        let row = remote_row(&node, &json!({ "position": 900, "updatedAt": "2026-10-15T12:00:00Z" })).unwrap();
        assert_eq!(row.meta.video_id, "10");
        assert_eq!(row.position_secs, 900.0);
        assert_eq!(row.channel_login, "Caedrel");
        assert_eq!(row.updated_at, 1_792_065_600);
        assert!(row.meta.finished);
        assert!(remote_row(&node, &json!({ "position": null, "updatedAt": "2026-10-15T12:00:00Z" })).is_none());
    }

    #[test]
    fn an_account_answer_splits_positions_from_dismissals() {
        let answer = json!({
            "now": "2026-10-15T12:05:00.123+00:00",
            "rows": [
                { "video_id": "10", "position_secs": 900.5, "duration_secs": 7200, "watched_at": "2026-10-15T12:00:00+00:00",
                  "dismissed": false, "channel_login": "caedrel", "channel_name": "Caedrel", "title": "t",
                  "thumbnail_url": null, "synced_at": "2026-10-15T12:05:00+00:00" },
                { "video_id": "11", "position_secs": 0, "duration_secs": 0, "watched_at": "2026-10-15T12:01:00+00:00",
                  "dismissed": true, "synced_at": "2026-10-15T12:05:00+00:00" },
                { "video_id": "12", "dismissed": false }
            ]
        });
        let (positions, dismissals, now) = cloud_rows(&answer);
        assert_eq!(now.as_deref(), Some("2026-10-15T12:05:00.123+00:00"));
        assert_eq!(positions.len(), 1);
        assert_eq!(positions[0].meta.video_id, "10");
        assert_eq!(positions[0].position_secs, 900.5);
        assert_eq!(positions[0].updated_at, 1_792_065_600);
        assert_eq!(positions[0].meta.length_secs, 7200.0);
        assert_eq!(positions[0].meta.thumbnail_url, "");
        assert_eq!(dismissals, vec![("11".to_string(), 1_792_065_660)]);
    }

    #[test]
    fn a_change_goes_out_in_the_shape_the_endpoint_takes() {
        let c = vod_progress_service::CloudChange {
            video_id: "10".into(),
            position_secs: 900.0,
            duration_secs: 7200.0,
            at: 1_792_065_600,
            dismissed: false,
            channel_login: "caedrel".into(),
            channel_name: String::new(),
            title: "t".into(),
            thumbnail_url: String::new(),
        };
        let v = change_json(&c);
        assert_eq!(v["watched_at"], "2026-10-15T12:00:00+00:00");
        assert_eq!(v["channel_name"], Value::Null);
        assert_eq!(v["channel_login"], "caedrel");
        assert_eq!(v["dismissed"], false);
    }
}
