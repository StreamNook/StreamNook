//! Sending whispers: the one door every send goes through (the Whispers
//! window, the phones, `/w` and `/r`, notification replies). It files the
//! message as Sending before Twitch hears of it, sends in order and spaced
//! (Twitch allows three a second), binds the send to the token it uses, and
//! only calls a whisper Delivered once Twitch's own copy of it is seen:
//! Helix can drop a whisper and still answer 204.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;
use std::time::Duration;

use once_cell::sync::Lazy;
use serde::Deserialize;

use crate::services::twitch_service::TwitchService;
use crate::services::whisper_inbox::{self, send_failure_reason, WhisperUpdate};
use crate::services::whisper_store::{self, Closing, Delivery, Origin, Scope, StoredWhisper};
use crate::services::whisper_sync::{self, Clock, Sleeper, SystemClock, TokioSleeper};

/// The longest whisper, in Unicode scalar values. Helix cuts longer ones at
/// 500 even for people you have whispered before, so this is the one limit.
pub const MAX_LEN: usize = 500;
/// The gap between two sends (Twitch allows three a second).
const SPACING_MS: i64 = 350;
/// When Twitch's copy is looked for after a send.
const FIRST_CHECK_MS: u64 = 1_500;
const SECOND_CHECK_MS: u64 = 10_000;
/// A Twitch copy is ours only inside this window around our send time.
const ADOPT_BEFORE_MS: i64 = 120_000;
const ADOPT_AFTER_MS: i64 = 600_000;
/// Old local copies carry the local clock, so their window is wider.
const MIGRATED_WINDOW_MS: i64 = 300_000;
/// Waits before resending after Twitch said "too many".
const RATE_RETRIES_MS: [u64; 3] = [1_000, 2_000, 4_000];
/// The length Twitch cuts a whisper to.
const TWITCH_CUT: usize = 500;
/// How long a teardown waits for a send already on the wire.
const TEARDOWN_WAIT: Duration = Duration::from_secs(35);

const NOT_DELIVERED_TWITCH: &str = "Twitch did not deliver this whisper.";
const SWITCHED: &str = "You switched accounts before this was sent.";
const SIGNED_OUT_BEFORE: &str = "You signed out before this was sent.";
const NOT_SIGNED_IN: &str = "You are not signed in to Twitch.";
const ONLY_NOT_DELIVERED: &str = "Only a whisper that was not delivered can be sent again.";

#[derive(Deserialize, Clone, Debug)]
pub struct SendTarget {
    pub key: String,
    pub user_id: String,
    pub user_login: String,
    pub user_name: String,
    pub profile_image_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SendFail {
    /// Twitch answered with this status and body.
    Status(u16, String),
    /// Nothing left the device (no connection, no name lookup).
    BeforeConnect(String),
    /// The request may have reached Twitch, but no answer came back.
    NoAnswer(String),
    SignedOut,
}

type Fut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// How sends reach Twitch: production is `HelixWhispers`, tests a fake.
pub trait Helix: Send + Sync {
    fn token(&self) -> Fut<'_, Result<String, SendFail>>;
    fn identity(&self, token: &str) -> Fut<'_, Result<String, SendFail>>;
    fn send(&self, token: &str, from: &str, to: &str, text: &str) -> Fut<'_, Result<(), SendFail>>;
    fn reject(&self, token: &str) -> Fut<'_, Result<String, SendFail>>;
}

/// Which account a token belongs to, for the last token asked about.
static IDENTITY: Mutex<Option<(String, String)>> = Mutex::new(None);

pub struct HelixWhispers;

impl Helix for HelixWhispers {
    fn token(&self) -> Fut<'_, Result<String, SendFail>> {
        Box::pin(async { TwitchService::get_token().await.map_err(|_| SendFail::SignedOut) })
    }

    fn identity(&self, token: &str) -> Fut<'_, Result<String, SendFail>> {
        let token = token.to_string();
        Box::pin(async move {
            if let Some((t, id)) = IDENTITY.lock().unwrap_or_else(|e| e.into_inner()).clone() {
                if t == token {
                    return Ok(id);
                }
            }
            match TwitchService::get_user_info_with_token(&token).await {
                Ok(user) => {
                    *IDENTITY.lock().unwrap_or_else(|e| e.into_inner()) = Some((token, user.id.clone()));
                    Ok(user.id)
                }
                Err(e) => Err(SendFail::NoAnswer(e.to_string())),
            }
        })
    }

    fn send(&self, token: &str, from: &str, to: &str, text: &str) -> Fut<'_, Result<(), SendFail>> {
        let (token, from, to, text) = (token.to_string(), from.to_string(), to.to_string(), text.to_string());
        Box::pin(async move { TwitchService::send_whisper_with_token(&token, &from, &to, &text).await })
    }

    fn reject(&self, token: &str) -> Fut<'_, Result<String, SendFail>> {
        let token = token.to_string();
        Box::pin(async move { TwitchService::get_token_after_reject(&token).await.map_err(|_| SendFail::SignedOut) })
    }
}

pub fn text_len(text: &str) -> usize {
    text.chars().count()
}

fn too_long(len: usize) -> String {
    format!("Whispers can be at most {MAX_LEN} characters ({len} now).")
}

fn other_account(name: Option<&str>) -> String {
    let name = name.filter(|n| !n.is_empty()).unwrap_or("another account");
    format!("This whisper came to {name}. Switch to that account to reply.")
}

/// A notification reply belongs to the account the whisper came to.
pub(crate) fn reply_owner_check(expected: &str, expected_name: Option<&str>, actual: &str) -> Result<(), String> {
    if expected.is_empty() || expected == actual {
        Ok(())
    } else {
        Err(other_account(expected_name))
    }
}

/// How one Helix attempt ended.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Outcome {
    Sent,
    /// It may have reached Twitch; only Twitch's thread can say.
    Unknown,
    RateLimited,
    Refused(String),
}

pub(crate) fn classify(r: &Result<(), SendFail>) -> Outcome {
    match r {
        Ok(()) => Outcome::Sent,
        Err(SendFail::NoAnswer(_)) => Outcome::Unknown,
        Err(SendFail::BeforeConnect(_)) => Outcome::Refused(send_failure_reason(None, "")),
        Err(SendFail::Status(429, _)) => Outcome::RateLimited,
        Err(SendFail::Status(code, body)) => Outcome::Refused(send_failure_reason(Some(*code), body)),
        Err(SendFail::SignedOut) => Outcome::Refused(NOT_SIGNED_IN.into()),
    }
}

fn apply_outcome(m: &mut StoredWhisper, outcome: &Outcome) {
    match outcome {
        Outcome::Sent => {
            m.delivery = Some(Delivery::Sent);
            m.got_204 = true;
            m.failure = None;
        }
        Outcome::Unknown => {
            m.delivery = Some(Delivery::Unknown);
            m.failure = None;
        }
        Outcome::RateLimited => {
            m.delivery = Some(Delivery::NotDelivered);
            m.failure = Some(send_failure_reason(Some(429), ""));
        }
        Outcome::Refused(reason) => {
            m.delivery = Some(Delivery::NotDelivered);
            m.failure = Some(reason.clone());
        }
    }
}

/// The token for the owner, bound to the account it really belongs to.
pub(crate) async fn bound_identity(helix: &dyn Helix, owner: &str) -> Result<(String, String), String> {
    let token = helix.token().await.map_err(|_| NOT_SIGNED_IN.to_string())?;
    let id = helix.identity(&token).await.map_err(|e| match e {
        SendFail::BeforeConnect(_) | SendFail::NoAnswer(_) => send_failure_reason(None, ""),
        _ => NOT_SIGNED_IN.to_string(),
    })?;
    if id != owner {
        return Err(SWITCHED.into());
    }
    Ok((token, id))
}

// ---- Adoption: Twitch's copy of a whisper we sent ---------------------------

fn is_padding(c: char) -> bool {
    matches!(c, '\u{034F}' | '\u{200B}'..='\u{200D}' | '\u{2060}' | '\u{FEFF}' | '\u{E0000}' | '\u{2800}')
}

/// Text as compared for adoption: no duplicate-bypass padding, single spaces,
/// no ends.
pub(crate) fn normalize(text: &str) -> String {
    text.chars().filter(|c| !is_padding(*c)).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ")
}

fn adoptable(m: &StoredWhisper) -> bool {
    m.origin == Origin::LocalSent
        && (m.migrated
            || matches!(m.delivery, Some(Delivery::Sent) | Some(Delivery::Unknown))
            || (m.delivery == Some(Delivery::NotDelivered) && m.got_204))
}

fn in_window(local: &StoredWhisper, twitch_ms: i64) -> bool {
    let d = twitch_ms - local.timestamp;
    if local.migrated {
        d.abs() <= MIGRATED_WINDOW_MS
    } else {
        (-ADOPT_BEFORE_MS..=ADOPT_AFTER_MS).contains(&d)
    }
}

/// Twitch cut our text: its copy is exactly the cut length and the start of ours.
fn twitch_cut(twitch_norm: &str, ours_norm: &str) -> bool {
    text_len(twitch_norm) == TWITCH_CUT && ours_norm.len() > twitch_norm.len() && ours_norm.starts_with(twitch_norm)
}

/// Pair Twitch's copies of our whispers with our local sends. `fresh` holds
/// Twitch messages not yet known by id; each adopted one is written onto its
/// local copy (Twitch's id, time and text win) and taken out of `fresh`.
/// Returns how many were adopted.
pub fn adopt(messages: &mut Vec<StoredWhisper>, fresh: &mut Vec<StoredWhisper>, me: &str) -> usize {
    let mut mine: Vec<usize> = (0..fresh.len()).filter(|&i| fresh[i].is_sent || fresh[i].from_user_id == me).collect();
    mine.sort_by(|&a, &b| (fresh[a].timestamp, &fresh[a].id).cmp(&(fresh[b].timestamp, &fresh[b].id)));
    let mut taken: Vec<usize> = Vec::new();
    let mut floor = i64::MIN;
    for fi in mine {
        let tw = fresh[fi].clone();
        let tw_norm = normalize(&tw.message);
        let pick = messages
            .iter()
            .enumerate()
            .filter(|(_, m)| adoptable(m) && m.timestamp >= floor && in_window(m, tw.timestamp))
            .filter(|(_, m)| {
                let ours = normalize(&m.message);
                ours == tw_norm || twitch_cut(&tw_norm, &ours)
            })
            .min_by(|(_, a), (_, b)| (a.timestamp, &a.id).cmp(&(b.timestamp, &b.id)))
            .map(|(i, _)| i);
        let Some(li) = pick else { continue };
        let cut = normalize(&messages[li].message) != tw_norm;
        let local = &mut messages[li];
        floor = local.timestamp;
        local.id = tw.id;
        local.timestamp = tw.timestamp;
        local.message = tw.message;
        local.parts = tw.parts;
        local.edited_ms = tw.edited_ms;
        local.deleted = tw.deleted;
        local.origin = Origin::Twitch;
        local.delivery = if local.migrated { None } else { Some(Delivery::Delivered) };
        local.failure = None;
        local.migrated = false;
        local.cut_to = cut.then(|| text_len(&local.message) as u32);
        taken.push(fi);
    }
    let adopted = taken.len();
    taken.sort_unstable();
    for i in taken.into_iter().rev() {
        fresh.remove(i);
    }
    if adopted > 0 {
        messages.sort_by(|a, b| (a.timestamp, &a.id).cmp(&(b.timestamp, &b.id)));
    }
    adopted
}

/// After a real read of Twitch's thread that started at `started_local`:
/// every send still waiting that was made at least a second before counts one
/// check; two checks without Twitch's copy make it Not delivered. Returns the
/// ids that changed.
pub(crate) fn apply_check(messages: &mut [StoredWhisper], started_local: i64) -> Vec<String> {
    let mut changed = Vec::new();
    for m in messages.iter_mut() {
        if m.origin != Origin::LocalSent || !matches!(m.delivery, Some(Delivery::Sent) | Some(Delivery::Unknown)) {
            continue;
        }
        let Some(sent) = m.sent_local_ms else { continue };
        if sent > started_local - 1_000 {
            continue;
        }
        m.checks = m.checks.saturating_add(1);
        if m.checks >= 2 {
            m.delivery = Some(Delivery::NotDelivered);
            m.failure = Some(NOT_DELIVERED_TWITCH.into());
        }
        changed.push(m.id.clone());
    }
    changed
}

/// A Not delivered whisper made ready to go again.
pub(crate) fn reset_for_retry(m: &mut StoredWhisper, now_local: i64, offset: i64) {
    m.delivery = Some(Delivery::Sending);
    m.checks = 0;
    m.got_204 = false;
    m.failure = None;
    m.sent_local_ms = Some(now_local);
    m.timestamp = now_local + offset;
}

// ---- The queue -------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
enum Stage {
    Queued,
    InFlight,
    /// Answered after the account closed: teardown files the outcome.
    Answered(Outcome),
}

#[derive(Clone, Debug)]
struct Pending {
    owner: String,
    key: String,
    stage: Stage,
}

static PENDING: Lazy<Mutex<HashMap<String, Pending>>> = Lazy::new(|| Mutex::new(HashMap::new()));
/// One send on the wire at a time, in the order they were asked for (tokio's
/// mutex is fair).
static SEND_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static LAST_START_MS: Mutex<Option<i64>> = Mutex::new(None);

fn set_stage(id: &str, owner: &str, key: &str, stage: Stage) {
    PENDING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(id.to_string(), Pending { owner: owner.to_string(), key: key.to_string(), stage });
}

fn clear_stage(id: &str) {
    PENDING.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
}

/// Whether any send of this conversation is still queued or on the wire.
fn conversation_sending(pending: &HashMap<String, Pending>, owner: &str, key: &str) -> bool {
    pending.values().any(|p| p.owner == owner && p.key == key && matches!(p.stage, Stage::Queued | Stage::InFlight))
}

/// A conversation with a send queued or in flight stays in memory; it may
/// leave once its last send has ended.
fn unpin_if_idle(scope: &Scope, key: &str) {
    let busy = conversation_sending(&PENDING.lock().unwrap_or_else(|e| e.into_inner()), &scope.owner, key);
    if !busy {
        let _ = whisper_store::pin(scope, key, false);
    }
}

/// Sends that were Sending when the app last stopped: no process is sending
/// them any more, and only Twitch's thread can say whether they went out. Each
/// becomes Unknown, which the next read of the thread adopts or checks; none is
/// ever sent again. A send this process still has queued or on the wire is
/// left alone. Returns how many changed.
pub fn settle_stale_sends(scope: &Scope) -> usize {
    let keys: Vec<String> = whisper_store::read_index(scope, |idx| {
        idx.conversations.iter().filter(|c| c.delivery_pending).map(|c| c.key.clone()).collect()
    })
    .unwrap_or_default();
    let live: std::collections::HashSet<String> = PENDING.lock().unwrap_or_else(|e| e.into_inner()).keys().cloned().collect();
    let mut settled = 0;
    for key in keys {
        let changed = whisper_store::update(scope, &key, None, |c| {
            let mut n = 0usize;
            for m in c.messages.iter_mut() {
                if m.origin == Origin::LocalSent && m.delivery == Some(Delivery::Sending) && !live.contains(&m.id) {
                    m.delivery = Some(Delivery::Unknown);
                    m.got_204 = false;
                    n += 1;
                }
            }
            (n, n > 0)
        });
        if let Ok(Some(n)) = changed {
            settled += n;
        }
    }
    settled
}

fn scope_current(scope: &Scope) -> bool {
    whisper_sync::current_scope().as_ref() == Some(scope)
}

fn new_local_id(now: i64) -> String {
    format!("sent-{}-{:08x}", now, rand::random::<u32>())
}

fn is_numeric_id(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

fn find_message(scope: &Scope, key: &str, id: &str) -> Option<StoredWhisper> {
    whisper_store::read(scope, key, |c| c.messages.iter().find(|m| m.id == id).cloned()).ok().flatten().flatten()
}

/// File the message in its conversation (creating it), as Sending.
fn file_send(scope: &Scope, target: &SendTarget, message: StoredWhisper) -> Result<String, String> {
    let key = whisper_inbox::key_for(scope, &target.user_id, &target.user_login).unwrap_or_else(|| target.key.clone());
    let create = whisper_inbox::new_meta(
        &key,
        &target.user_id,
        &target.user_login,
        &target.user_name,
        target.profile_image_url.clone(),
    );
    whisper_store::update(scope, &key, Some(create), |c| {
        if c.meta.archived {
            c.meta.archived = false;
            c.meta.archive_pending = false;
            c.meta.archive_requested_ms = None;
            c.meta.archived_seq = whisper_store::next_seq();
        }
        let added = whisper_inbox::insert_sorted(&mut c.messages, message);
        ((), added)
    })?;
    Ok(key)
}

/// Write how a send ended. When the account closed meanwhile, the outcome
/// waits for teardown instead.
fn finish(scope: &Scope, key: &str, id: &str, outcome: &Outcome) -> Option<WhisperUpdate> {
    let applied = whisper_store::update(scope, key, None, |c| match c.messages.iter_mut().find(|m| m.id == id) {
        Some(m) => {
            apply_outcome(m, outcome);
            (Some(m.clone()), true)
        }
        None => (None, false),
    });
    match applied {
        Ok(Some(Some(m))) => {
            clear_stage(id);
            unpin_if_idle(scope, key);
            whisper_inbox::emit_update(scope, key, Some(m), None)
        }
        Ok(_) => {
            clear_stage(id);
            unpin_if_idle(scope, key);
            None
        }
        Err(_) => {
            set_stage(id, &scope.owner, key, Stage::Answered(outcome.clone()));
            None
        }
    }
}

/// Whether `scope` is still the followed account, asked when a send's turn comes.
type IsCurrent = dyn Fn(&Scope) -> bool + Send + Sync;

/// Send a message already filed as Sending, in its turn.
#[allow(clippy::too_many_arguments)]
async fn run_queued(
    scope: &Scope,
    helix: &dyn Helix,
    clock: &dyn Clock,
    sleeper: &dyn Sleeper,
    current: &IsCurrent,
    key: &str,
    id: &str,
    expect_owner: Option<&str>,
    expect_owner_name: Option<&str>,
    schedule: bool,
) -> Result<WhisperUpdate, String> {
    set_stage(id, &scope.owner, key, Stage::Queued);
    let _ = whisper_store::pin(scope, key, true);
    let _turn = SEND_LOCK.lock().await;
    if !current(scope) {
        // Teardown files the words for it.
        return Err(SIGNED_OUT_BEFORE.into());
    }
    let Some(message) = find_message(scope, key, id) else {
        clear_stage(id);
        unpin_if_idle(scope, key);
        return Err("This whisper was discarded.".into());
    };
    set_stage(id, &scope.owner, key, Stage::InFlight);

    let last = *LAST_START_MS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(last) = last {
        let gap = clock.now_ms() - last;
        if gap < SPACING_MS {
            sleeper.sleep((SPACING_MS - gap) as u64).await;
        }
    }
    *LAST_START_MS.lock().unwrap_or_else(|e| e.into_inner()) = Some(clock.now_ms());

    let (mut token, me) = match bound_identity(helix, &scope.owner).await {
        Ok(v) => v,
        Err(reason) => {
            finish(scope, key, id, &Outcome::Refused(reason.clone()));
            return Err(reason);
        }
    };
    if let Some(expected) = expect_owner {
        if let Err(reason) = reply_owner_check(expected, expect_owner_name, &me) {
            finish(scope, key, id, &Outcome::Refused(reason.clone()));
            return Err(reason);
        }
    }

    let mut attempt = 0usize;
    let mut rejected_once = false;
    let outcome = loop {
        let r = helix.send(&token, &me, &message.to_user_id, &message.message).await;
        if matches!(r, Err(SendFail::Status(401, _))) && !rejected_once {
            rejected_once = true;
            match helix.reject(&token).await {
                Ok(fresh) => match helix.identity(&fresh).await {
                    Ok(who) if who == scope.owner => {
                        token = fresh;
                        continue;
                    }
                    _ => break Outcome::Refused(SWITCHED.into()),
                },
                Err(_) => break classify(&r),
            }
        }
        let o = classify(&r);
        if o == Outcome::RateLimited && attempt < RATE_RETRIES_MS.len() {
            sleeper.sleep(RATE_RETRIES_MS[attempt]).await;
            attempt += 1;
            continue;
        }
        break o;
    };
    let update = finish(scope, key, id, &outcome);
    match outcome {
        Outcome::Sent | Outcome::Unknown => {
            if schedule {
                schedule_checks(scope.clone(), key.to_string());
            }
            update.ok_or_else(|| SIGNED_OUT_BEFORE.to_string())
        }
        Outcome::RateLimited => Err(send_failure_reason(Some(429), "")),
        Outcome::Refused(reason) => Err(reason),
    }
}

/// The whole send, with the seams passed in.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn send_in(
    scope: &Scope,
    helix: &dyn Helix,
    clock: &dyn Clock,
    sleeper: &dyn Sleeper,
    mut target: SendTarget,
    text: String,
    expect_owner: Option<String>,
    expect_owner_name: Option<String>,
    schedule: bool,
) -> Result<WhisperUpdate, String> {
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err("Type a message to send.".into());
    }
    let len = text_len(&text);
    if len > MAX_LEN {
        return Err(too_long(len));
    }
    if let Some(expected) = expect_owner.as_deref() {
        reply_owner_check(expected, expect_owner_name.as_deref(), &scope.owner)?;
    }
    if !is_numeric_id(&target.user_id) {
        let user = TwitchService::get_user_by_login(&target.user_login)
            .await
            .map_err(|_| "Cannot send: User not found on Twitch.".to_string())?;
        target.user_id = user.id;
        if target.profile_image_url.is_none() {
            target.profile_image_url = user.profile_image_url;
        }
    }
    let now = clock.now_ms();
    let message = StoredWhisper {
        id: new_local_id(now),
        from_user_id: scope.owner.clone(),
        from_user_login: String::new(),
        from_user_name: String::new(),
        to_user_id: target.user_id.clone(),
        to_user_login: target.user_login.clone(),
        to_user_name: target.user_name.clone(),
        message: text,
        timestamp: now + whisper_sync::twitch_offset_ms(),
        is_sent: true,
        parts: Vec::new(),
        deleted: false,
        edited_ms: None,
        origin: Origin::LocalSent,
        delivery: Some(Delivery::Sending),
        sent_local_ms: Some(now),
        got_204: false,
        checks: 0,
        failure: None,
        cut_to: None,
        migrated: false,
    };
    let key = file_send(scope, &target, message.clone())?;
    // In memory until the send ends, and on disk before Twitch hears of it, so
    // a crash leaves it to be checked.
    let _ = whisper_store::pin(scope, &key, true);
    whisper_store::write_now(scope, &key)?;
    whisper_inbox::emit_update(scope, &key, Some(message.clone()), None);
    run_queued(
        scope,
        helix,
        clock,
        sleeper,
        &scope_current,
        &key,
        &message.id,
        expect_owner.as_deref(),
        expect_owner_name.as_deref(),
        schedule,
    )
    .await
}

/// Send a whisper from the signed-in account. Returns once Twitch answered
/// (Sent, or why not); delivery arrives later as an update.
pub async fn send(
    target: SendTarget,
    text: String,
    expect_owner: Option<String>,
    expect_owner_name: Option<String>,
) -> Result<WhisperUpdate, String> {
    let scope = whisper_sync::current_scope().ok_or_else(|| NOT_SIGNED_IN.to_string())?;
    send_in(&scope, &HelixWhispers, &SystemClock, &TokioSleeper, target, text, expect_owner, expect_owner_name, true).await
}

/// Look for Twitch's copy twice: shortly after the send, then at ten seconds.
fn schedule_checks(scope: Scope, key: String) {
    tauri::async_runtime::spawn(async move {
        TokioSleeper.sleep(FIRST_CHECK_MS).await;
        check_once(&scope, &key).await;
        TokioSleeper.sleep(SECOND_CHECK_MS - FIRST_CHECK_MS).await;
        check_once(&scope, &key).await;
    });
}

/// One real look at Twitch's thread for this conversation. A read that fails
/// is not a check.
async fn check_once(scope: &Scope, key: &str) {
    if !scope_current(scope) {
        return;
    }
    let started = SystemClock.now_ms();
    if whisper_sync::refresh_thread(key.to_string()).await.is_err() {
        return;
    }
    let changed = whisper_store::update(scope, key, None, |c| {
        let ids = apply_check(&mut c.messages, started);
        let any = !ids.is_empty();
        (ids.into_iter().filter_map(|id| c.messages.iter().find(|m| m.id == id).cloned()).collect::<Vec<_>>(), any)
    });
    if let Ok(Some(messages)) = changed {
        for m in messages {
            whisper_inbox::emit_update(scope, key, Some(m), None);
        }
    }
}

/// Look again for every send still waiting on Twitch's copy (after a resume,
/// or on following an account: a check the suspended app never ran).
pub fn recheck_pending() {
    let Some(scope) = whisper_sync::current_scope() else { return };
    let keys: Vec<String> = whisper_store::read_index(&scope, |idx| {
        idx.conversations.iter().filter(|c| c.delivery_pending).map(|c| c.key.clone()).collect()
    })
    .unwrap_or_default();
    if keys.is_empty() {
        return;
    }
    tauri::async_runtime::spawn(async move {
        for key in keys {
            check_once(&scope, &key).await;
        }
    });
}

/// Send a Not delivered whisper again. Twitch's thread is read first: when it
/// already has the whisper, that copy is adopted and nothing is sent twice.
pub async fn retry(key: String, message_id: String) -> Result<WhisperUpdate, String> {
    let scope = whisper_sync::current_scope().ok_or_else(|| NOT_SIGNED_IN.to_string())?;
    match find_message(&scope, &key, &message_id) {
        Some(m) if m.delivery == Some(Delivery::NotDelivered) => {}
        _ => return Err(ONLY_NOT_DELIVERED.into()),
    }
    let _ = whisper_sync::refresh_thread(key.clone()).await;
    match find_message(&scope, &key, &message_id) {
        // Adopted under Twitch's id: it was delivered after all.
        None => return whisper_inbox::emit_update(&scope, &key, None, None).ok_or_else(|| ONLY_NOT_DELIVERED.into()),
        Some(m) if m.delivery != Some(Delivery::NotDelivered) => {
            return whisper_inbox::emit_update(&scope, &key, Some(m), None).ok_or_else(|| ONLY_NOT_DELIVERED.into());
        }
        Some(_) => {}
    }
    let now = SystemClock.now_ms();
    let offset = whisper_sync::twitch_offset_ms();
    let reset = whisper_store::update(&scope, &key, None, |c| {
        let Some(pos) = c.messages.iter().position(|m| m.id == message_id) else { return (None, false) };
        let mut m = c.messages.remove(pos);
        reset_for_retry(&mut m, now, offset);
        whisper_inbox::insert_sorted(&mut c.messages, m.clone());
        (Some(m), true)
    })?;
    let Some(Some(m)) = reset else { return Err(ONLY_NOT_DELIVERED.into()) };
    let _ = whisper_store::pin(&scope, &key, true);
    whisper_store::write_now(&scope, &key)?;
    whisper_inbox::emit_update(&scope, &key, Some(m), None);
    run_queued(&scope, &HelixWhispers, &SystemClock, &TokioSleeper, &scope_current, &key, &message_id, None, None, true).await
}

/// Drop a Not delivered whisper.
pub fn discard(key: &str, message_id: &str) -> Result<(), String> {
    let Some(scope) = whisper_sync::current_scope() else { return Ok(()) };
    match find_message(&scope, key, message_id) {
        Some(m) if m.delivery == Some(Delivery::NotDelivered) => {}
        _ => return Err("Only a whisper that was not delivered can be discarded.".into()),
    }
    if whisper_store::remove_message(&scope, key, message_id)? {
        clear_stage(message_id);
        whisper_inbox::emit_removed(&scope, key, message_id);
    }
    Ok(())
}

/// A reply typed into a notification (Windows toast, Android with the app
/// running). A notification can start the app, so this waits a little for the
/// account to load. Refused when the whisper came to another account.
pub async fn reply_from_notification(
    owner_id: Option<String>,
    owner_name: Option<String>,
    user_id: &str,
    login: &str,
    name: &str,
    avatar: Option<String>,
    text: String,
) -> Result<(), String> {
    let mut scope = whisper_sync::current_scope();
    for _ in 0..20 {
        if scope.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
        scope = whisper_sync::current_scope();
    }
    let scope = scope.ok_or_else(|| NOT_SIGNED_IN.to_string())?;
    if let Some(expected) = owner_id.as_deref() {
        reply_owner_check(expected, owner_name.as_deref(), &scope.owner)?;
    }
    let key = whisper_inbox::key_for(&scope, user_id, login).unwrap_or_else(|| user_id.to_string());
    let target = SendTarget {
        key,
        user_id: user_id.to_string(),
        user_login: login.to_string(),
        user_name: name.to_string(),
        profile_image_url: avatar,
    };
    send_in(&scope, &HelixWhispers, &SystemClock, &TokioSleeper, target, text, owner_id, owner_name, true)
        .await
        .map(|_| ())
}

/// A notification reply in a process Android started only for it (no app, no
/// followed account). Files the reply as Unknown before sending, on disk, so
/// the next sync adopts or checks it; a refusal marks it Not delivered.
pub async fn cold_reply(
    owner_id: Option<String>,
    owner_name: Option<String>,
    user_id: &str,
    login: &str,
    name: &str,
    text: String,
) -> Result<(), String> {
    let text = text.trim().to_string();
    if text.is_empty() || user_id.is_empty() {
        return Ok(());
    }
    let len = text_len(&text);
    if len > MAX_LEN {
        return Err(too_long(len));
    }
    let helix = HelixWhispers;
    let token = helix.token().await.map_err(|_| NOT_SIGNED_IN.to_string())?;
    let me = helix.identity(&token).await.map_err(|_| send_failure_reason(None, ""))?;
    if let Some(expected) = owner_id.as_deref() {
        reply_owner_check(expected, owner_name.as_deref(), &me)?;
    }
    let scope = whisper_store::open(&me, 0);
    let now = SystemClock.now_ms();
    let id = new_local_id(now);
    let message = StoredWhisper {
        id: id.clone(),
        from_user_id: me.clone(),
        from_user_login: String::new(),
        from_user_name: String::new(),
        to_user_id: user_id.to_string(),
        to_user_login: login.to_string(),
        to_user_name: name.to_string(),
        message: text.clone(),
        timestamp: now,
        is_sent: true,
        parts: Vec::new(),
        deleted: false,
        edited_ms: None,
        origin: Origin::LocalSent,
        delivery: Some(Delivery::Unknown),
        sent_local_ms: Some(now),
        got_204: false,
        checks: 0,
        failure: None,
        cut_to: None,
        migrated: false,
    };
    let target =
        SendTarget { key: user_id.to_string(), user_id: user_id.to_string(), user_login: login.to_string(), user_name: name.to_string(), profile_image_url: None };
    let key = file_send(&scope, &target, message)?;
    whisper_store::write_now(&scope, &key)?;
    whisper_store::flush_now()?;
    let outcome = classify(&helix.send(&token, &me, user_id, &text).await);
    let (stored, result) = match outcome {
        Outcome::Sent => (Outcome::Unknown, Ok(())),
        Outcome::Unknown => (Outcome::Unknown, Ok(())),
        Outcome::RateLimited => {
            let r = send_failure_reason(Some(429), "");
            (Outcome::Refused(r.clone()), Err(r))
        }
        Outcome::Refused(r) => (Outcome::Refused(r.clone()), Err(r)),
    };
    let got_204 = matches!(result, Ok(())) && matches!(stored, Outcome::Unknown);
    let _ = whisper_store::update(&scope, &key, None, |c| match c.messages.iter_mut().find(|m| m.id == id) {
        Some(m) => {
            apply_outcome(m, &stored);
            m.got_204 = got_204;
            ((), true)
        }
        None => ((), false),
    });
    let _ = whisper_store::write_now(&scope, &key);
    let _ = whisper_store::flush_now();
    result
}

/// The account closed (sign-out or a switch): wait for the send already on
/// the wire, then file what every send of that account came to.
pub async fn teardown(closing: &Closing) {
    let owner = closing.0.owner.clone();
    let _turn = tokio::time::timeout(TEARDOWN_WAIT, SEND_LOCK.lock()).await;
    let mine: Vec<(String, Pending)> = {
        let mut pending = PENDING.lock().unwrap_or_else(|e| e.into_inner());
        let ids: Vec<String> = pending.iter().filter(|(_, p)| p.owner == owner).map(|(id, _)| id.clone()).collect();
        ids.into_iter().filter_map(|id| pending.remove(&id).map(|p| (id, p))).collect()
    };
    for (id, p) in mine {
        let (outcome, got_204) = match p.stage {
            Stage::Queued => (Outcome::Refused(SIGNED_OUT_BEFORE.into()), false),
            Stage::InFlight => (Outcome::Unknown, false),
            Stage::Answered(Outcome::Sent) => (Outcome::Unknown, true),
            Stage::Answered(o) => (o, false),
        };
        let _ = whisper_store::update_closing(closing, &p.key, |c| match c.messages.iter_mut().find(|m| m.id == id) {
            Some(m) => {
                apply_outcome(m, &outcome);
                m.got_204 = m.got_204 || got_204;
                ((), true)
            }
            None => ((), false),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    const ME: &str = "99999999";
    const THEM: &str = "1000000000";

    struct FakeHelix {
        id: String,
        sends: AtomicU32,
    }

    impl Helix for FakeHelix {
        fn token(&self) -> Fut<'_, Result<String, SendFail>> {
            Box::pin(async { Ok("token-a".to_string()) })
        }
        fn identity(&self, _token: &str) -> Fut<'_, Result<String, SendFail>> {
            let id = self.id.clone();
            Box::pin(async move { Ok(id) })
        }
        fn send(&self, _t: &str, _f: &str, _to: &str, _x: &str) -> Fut<'_, Result<(), SendFail>> {
            self.sends.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(()) })
        }
        fn reject(&self, _t: &str) -> Fut<'_, Result<String, SendFail>> {
            Box::pin(async { Err(SendFail::SignedOut) })
        }
    }

    struct FixedClock(i64);
    impl Clock for FixedClock {
        fn now_ms(&self) -> i64 {
            self.0
        }
    }

    struct NoSleep;
    impl Sleeper for NoSleep {
        fn sleep(&self, _ms: u64) -> Pin<Box<dyn Future<Output = ()> + Send>> {
            Box::pin(async {})
        }
    }

    fn scope() -> Scope {
        Scope { owner: ME.into(), generation: 1 }
    }

    fn target() -> SendTarget {
        SendTarget { key: THEM.into(), user_id: THEM.into(), user_login: "pebble".into(), user_name: "Pebble".into(), profile_image_url: None }
    }

    fn local(id: &str, text: &str, ts: i64, delivery: Option<Delivery>) -> StoredWhisper {
        StoredWhisper {
            id: id.into(),
            from_user_id: ME.into(),
            from_user_login: String::new(),
            from_user_name: String::new(),
            to_user_id: THEM.into(),
            to_user_login: "pebble".into(),
            to_user_name: "Pebble".into(),
            message: text.into(),
            timestamp: ts,
            is_sent: true,
            parts: Vec::new(),
            deleted: false,
            edited_ms: None,
            origin: Origin::LocalSent,
            delivery,
            sent_local_ms: Some(ts),
            got_204: delivery == Some(Delivery::Sent),
            checks: 0,
            failure: None,
            cut_to: None,
            migrated: false,
        }
    }

    fn twitch(id: &str, text: &str, ts: i64) -> StoredWhisper {
        let mut m = local(id, text, ts, None);
        m.origin = Origin::Twitch;
        m.sent_local_ms = None;
        m.got_204 = false;
        m
    }

    #[tokio::test]
    async fn send_over_500_is_refused() {
        let helix = FakeHelix { id: ME.into(), sends: AtomicU32::new(0) };
        let text = "x".repeat(MAX_LEN + 1);
        let r = send_in(&scope(), &helix, &FixedClock(0), &NoSleep, target(), text, None, None, false).await;
        assert_eq!(r.unwrap_err(), "Whispers can be at most 500 characters (501 now).");
        assert_eq!(helix.sends.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn send_counts_scalars_not_bytes() {
        let helix = FakeHelix { id: ME.into(), sends: AtomicU32::new(0) };
        let text = "\u{1F600}".repeat(MAX_LEN + 1);
        let r = send_in(&scope(), &helix, &FixedClock(0), &NoSleep, target(), text, None, None, false).await;
        assert!(r.unwrap_err().contains("(501 now)"));
        assert_eq!(text_len(&"\u{1F600}".repeat(MAX_LEN)), MAX_LEN);
    }

    #[tokio::test]
    async fn empty_send_is_refused() {
        let helix = FakeHelix { id: ME.into(), sends: AtomicU32::new(0) };
        let r = send_in(&scope(), &helix, &FixedClock(0), &NoSleep, target(), "   ".into(), None, None, false).await;
        assert_eq!(r.unwrap_err(), "Type a message to send.");
    }

    #[tokio::test]
    async fn send_refuses_other_accounts_token() {
        let helix = FakeHelix { id: "1234567".into(), sends: AtomicU32::new(0) };
        assert_eq!(bound_identity(&helix, ME).await.unwrap_err(), SWITCHED);
        let same = FakeHelix { id: ME.into(), sends: AtomicU32::new(0) };
        assert_eq!(bound_identity(&same, ME).await.unwrap(), ("token-a".to_string(), ME.to_string()));
    }

    #[test]
    fn reply_for_other_account_refused() {
        assert_eq!(
            reply_owner_check(ME, Some("Otter"), "1234567").unwrap_err(),
            "This whisper came to Otter. Switch to that account to reply."
        );
        assert_eq!(
            reply_owner_check(ME, None, "1234567").unwrap_err(),
            "This whisper came to another account. Switch to that account to reply."
        );
        assert!(reply_owner_check(ME, Some("Otter"), ME).is_ok());
        assert!(reply_owner_check("", None, "1234567").is_ok());
    }

    #[test]
    fn timeout_after_write_is_unknown() {
        assert_eq!(classify(&Err(SendFail::NoAnswer("timed out".into()))), Outcome::Unknown);
        let mut m = local("s", "hi", 0, Some(Delivery::Sending));
        apply_outcome(&mut m, &Outcome::Unknown);
        assert_eq!(m.delivery, Some(Delivery::Unknown));
        assert!(adoptable(&m));
    }

    #[test]
    fn connect_failure_is_not_delivered_at_once() {
        assert_eq!(
            classify(&Err(SendFail::BeforeConnect("dns".into()))),
            Outcome::Refused("Could not reach Twitch. Check your connection and try again.".into())
        );
        assert_eq!(classify(&Err(SendFail::Status(429, String::new()))), Outcome::RateLimited);
        assert_eq!(classify(&Ok(())), Outcome::Sent);
    }

    #[test]
    fn adoption_pairs_identical_texts() {
        let mut messages = vec![local("a", "ok", 1_000, Some(Delivery::Sent)), local("b", "ok", 2_000, Some(Delivery::Sent))];
        let mut fresh = vec![twitch("t2", "ok", 2_500), twitch("t1", "ok", 1_500)];
        assert_eq!(adopt(&mut messages, &mut fresh, ME), 2);
        assert!(fresh.is_empty());
        let ids: Vec<_> = messages.iter().map(|m| (m.id.as_str(), m.delivery)).collect();
        assert_eq!(ids, [("t1", Some(Delivery::Delivered)), ("t2", Some(Delivery::Delivered))]);
    }

    #[test]
    fn adoption_survives_clock_skew() {
        // Stamps are already offset-corrected; a remaining 90 s lead still pairs.
        let mut messages = vec![local("a", "see you  soon", 100_000, Some(Delivery::Sent))];
        let mut fresh = vec![twitch("t", "see you soon", 10_000)];
        assert_eq!(adopt(&mut messages, &mut fresh, ME), 1);
        assert_eq!(messages[0].id, "t");
        assert_eq!(messages[0].timestamp, 10_000);
    }

    #[test]
    fn late_same_text_never_adopts() {
        let mut messages = vec![local("a", "ok", 0, Some(Delivery::NotDelivered))];
        messages[0].got_204 = true;
        let mut fresh = vec![twitch("t", "ok", 3_600_000)];
        assert_eq!(adopt(&mut messages, &mut fresh, ME), 0);
        assert_eq!(fresh.len(), 1);
    }

    #[test]
    fn queued_copy_never_adopted() {
        let mut messages = vec![local("a", "ok", 0, Some(Delivery::Sending))];
        let mut fresh = vec![twitch("t", "ok", 500)];
        assert_eq!(adopt(&mut messages, &mut fresh, ME), 0);
        // A refusal that never got a 204 is not adoptable either.
        let mut refused = vec![local("b", "ok", 0, Some(Delivery::NotDelivered))];
        assert_eq!(adopt(&mut refused, &mut fresh, ME), 0);
    }

    #[test]
    fn different_text_never_pairs() {
        let mut messages = vec![local("a", "ok see you", 0, Some(Delivery::Sent))];
        let mut fresh = vec![twitch("t", "ok", 500)];
        assert_eq!(adopt(&mut messages, &mut fresh, ME), 0);
    }

    #[test]
    fn prefix_of_unrelated_send_never_pairs() {
        let mut messages = vec![local("a", &"y".repeat(400), 0, Some(Delivery::Sent))];
        let mut fresh = vec![twitch("t", &"y".repeat(300), 500)];
        assert_eq!(adopt(&mut messages, &mut fresh, ME), 0);
    }

    #[test]
    fn truncated_copy_pairs() {
        let mut long = local("a", &"z".repeat(520), 0, None);
        long.migrated = true;
        let mut messages = vec![long];
        let mut fresh = vec![twitch("t", &"z".repeat(500), 1_000)];
        assert_eq!(adopt(&mut messages, &mut fresh, ME), 1);
        assert_eq!(messages[0].cut_to, Some(500));
    }

    #[test]
    fn migrated_sends_are_adopted_once() {
        let mut old = local("sent-1", "hey", 0, None);
        old.migrated = true;
        old.sent_local_ms = None;
        let mut messages = vec![old];
        let mut fresh = vec![twitch("t", "hey", 240_000)];
        assert_eq!(adopt(&mut messages, &mut fresh, ME), 1);
        assert_eq!(messages[0].delivery, None);
        assert!(!messages[0].migrated);
        let mut again = vec![twitch("u", "hey", 241_000)];
        assert_eq!(adopt(&mut messages, &mut again, ME), 0);
    }

    #[test]
    fn crash_after_204_is_adopted_not_resent() {
        // At load a Sending message becomes Unknown (the store does that);
        // Unknown is adoptable and nothing here can resend it.
        let mut messages = vec![local("a", "hello", 0, Some(Delivery::Unknown))];
        let mut fresh = vec![twitch("t", "hello", 1_200)];
        assert_eq!(adopt(&mut messages, &mut fresh, ME), 1);
        assert_eq!(messages[0].delivery, Some(Delivery::Delivered));
    }

    /// Retry first refreshes the thread, so a Not delivered
    /// whisper whose Helix answer was 204 is adopted by Twitch's copy (new id,
    /// Delivered) and `retry` then finds no Not delivered message to resend; one
    /// that never got a 204 is not adoptable and is resent.
    #[test]
    fn retry_adopts_instead_of_resending() {
        let mut dropped = local("a", "see you there", 0, Some(Delivery::NotDelivered));
        dropped.got_204 = true;
        let mut messages = vec![dropped];
        let mut fresh = vec![twitch("t", "see you there", 1_100)];
        assert_eq!(adopt(&mut messages, &mut fresh, ME), 1);
        assert_eq!(messages[0].id, "t", "the retry's message id is gone, so nothing is resent");
        assert_eq!(messages[0].delivery, Some(Delivery::Delivered));

        let refused = local("b", "see you there", 0, Some(Delivery::NotDelivered));
        assert!(!refused.got_204);
        assert!(!adoptable(&refused), "a send Helix refused was never delivered and is resent on Retry");
    }

    /// Signing out with a send
    /// still queued and one on the wire: the queued one is Not delivered with
    /// the sign-out sentence, the one on the wire is Unknown (checked later,
    /// never resent), both written through the closing account.
    #[tokio::test]
    async fn signout_drains_queue() {
        let owner = "99999941";
        let key = THEM;
        let scope = whisper_store::open(owner, 1);
        whisper_store::update(&scope, key, Some(whisper_store::ConversationMeta::new(key)), |c| {
            c.messages.push(local("q1", "first", 1, Some(Delivery::Sending)));
            c.messages.push(local("w1", "second", 2, Some(Delivery::Sending)));
            ((), true)
        })
        .unwrap();
        set_stage("q1", owner, key, Stage::Queued);
        set_stage("w1", owner, key, Stage::InFlight);
        let closing = whisper_store::close(&scope);
        teardown(&closing).await;
        let states = whisper_store::update_closing(&closing, key, |c| {
            let of = |id: &str| c.messages.iter().find(|m| m.id == id).map(|m| (m.delivery, m.failure.clone()));
            ((of("q1"), of("w1")), false)
        })
        .unwrap()
        .unwrap();
        assert_eq!(states.0, Some((Some(Delivery::NotDelivered), Some(SIGNED_OUT_BEFORE.to_string()))));
        assert_eq!(states.1.map(|s| s.0), Some(Some(Delivery::Unknown)));
        let left = PENDING.lock().unwrap_or_else(|e| e.into_inner()).values().filter(|p| p.owner == owner).count();
        assert_eq!(left, 0, "nothing of the signed-out account stays queued");
    }

    #[test]
    fn two_real_checks_before_not_delivered() {
        let mut messages = vec![local("a", "hi", 0, Some(Delivery::Sent))];
        // A read that started before the send is not a check of it.
        assert!(apply_check(&mut messages, 500).is_empty());
        assert_eq!(apply_check(&mut messages, 1_500), vec!["a".to_string()]);
        assert_eq!(messages[0].delivery, Some(Delivery::Sent));
        apply_check(&mut messages, 10_000);
        assert_eq!(messages[0].delivery, Some(Delivery::NotDelivered));
        assert_eq!(messages[0].failure.as_deref(), Some(NOT_DELIVERED_TWITCH));
        // Still adoptable if Twitch's copy turns up after all.
        assert!(adoptable(&messages[0]));
    }

    #[test]
    fn an_unknown_send_needs_two_checks() {
        let mut messages = vec![local("a", "hi", 0, Some(Delivery::Unknown))];
        apply_check(&mut messages, 600_000);
        apply_check(&mut messages, 600_100);
        assert_eq!(messages[0].delivery, Some(Delivery::NotDelivered));
    }

    #[test]
    fn migrated_sends_never_turn_not_delivered() {
        let mut old = local("sent-1", "hey", 0, None);
        old.migrated = true;
        let mut messages = vec![old];
        assert!(apply_check(&mut messages, 900_000).is_empty());
        assert_eq!(messages[0].delivery, None);
    }

    // ---- The queue and the restart ---------------------------------------------------

    /// Helix as a script: each send takes the next answer (Ok when the script
    /// is empty); a refused token is renewed as "token-b", whose owner is
    /// `renewed_id`.
    struct ScriptHelix {
        id: String,
        renewed_id: String,
        sends: AtomicU32,
        rejects: AtomicU32,
        script: Mutex<std::collections::VecDeque<Result<(), SendFail>>>,
        on_send: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
    }

    impl ScriptHelix {
        fn new(id: &str, script: Vec<Result<(), SendFail>>) -> Self {
            ScriptHelix {
                id: id.into(),
                renewed_id: id.into(),
                sends: AtomicU32::new(0),
                rejects: AtomicU32::new(0),
                script: Mutex::new(script.into()),
                on_send: Mutex::new(None),
            }
        }
    }

    impl Helix for ScriptHelix {
        fn token(&self) -> Fut<'_, Result<String, SendFail>> {
            Box::pin(async { Ok("token-a".to_string()) })
        }
        fn identity(&self, token: &str) -> Fut<'_, Result<String, SendFail>> {
            let id = if token == "token-b" { self.renewed_id.clone() } else { self.id.clone() };
            Box::pin(async move { Ok(id) })
        }
        fn send(&self, _t: &str, _f: &str, _to: &str, _x: &str) -> Fut<'_, Result<(), SendFail>> {
            self.sends.fetch_add(1, Ordering::SeqCst);
            if let Some(hook) = self.on_send.lock().unwrap().as_ref() {
                hook();
            }
            let answer = self.script.lock().unwrap().pop_front().unwrap_or(Ok(()));
            Box::pin(async move { answer })
        }
        fn reject(&self, _t: &str) -> Fut<'_, Result<String, SendFail>> {
            self.rejects.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok("token-b".to_string()) })
        }
    }

    #[derive(Default)]
    struct RecordingSleeper(Mutex<Vec<u64>>);
    impl Sleeper for RecordingSleeper {
        fn sleep(&self, ms: u64) -> Pin<Box<dyn Future<Output = ()> + Send>> {
            self.0.lock().unwrap().push(ms);
            Box::pin(async {})
        }
    }

    /// Every queue test reads the same clock, so the shared spacing state
    /// never sees two clocks.
    const QUEUE_NOW: i64 = 1_000_000;

    fn always(_: &Scope) -> bool {
        true
    }

    fn file(scope: &Scope, key: &str, messages: Vec<StoredWhisper>) {
        whisper_store::update(scope, key, Some(whisper_store::ConversationMeta::new(key)), |c| {
            c.messages.extend(messages);
            ((), true)
        })
        .unwrap();
    }

    fn stored(scope: &Scope, key: &str, id: &str) -> Option<StoredWhisper> {
        whisper_store::read(scope, key, |c| c.messages.iter().find(|m| m.id == id).cloned()).unwrap().flatten()
    }

    #[tokio::test]
    async fn queued_sends_are_spaced() {
        let scope = whisper_store::open("99999951", 1);
        file(&scope, THEM, vec![local("q1", "one", QUEUE_NOW, Some(Delivery::Sending)), local("q2", "two", QUEUE_NOW, Some(Delivery::Sending))]);
        let helix = ScriptHelix::new("99999951", Vec::new());
        let clock = FixedClock(QUEUE_NOW);
        run_queued(&scope, &helix, &clock, &NoSleep, &always, THEM, "q1", None, None, false).await.unwrap();
        let second = RecordingSleeper::default();
        run_queued(&scope, &helix, &clock, &second, &always, THEM, "q2", None, None, false).await.unwrap();
        assert_eq!(second.0.lock().unwrap().first().copied(), Some(SPACING_MS as u64), "the next send waits out the gap");
        assert_eq!(stored(&scope, THEM, "q2").unwrap().delivery, Some(Delivery::Sent));
    }

    #[tokio::test]
    async fn too_many_sends_back_off_then_not_delivered() {
        let scope = whisper_store::open("99999952", 1);
        file(&scope, THEM, vec![local("r1", "busy", QUEUE_NOW, Some(Delivery::Sending))]);
        let refusals = (0..4).map(|_| Err(SendFail::Status(429, String::new()))).collect();
        let helix = ScriptHelix::new("99999952", refusals);
        let sleeper = RecordingSleeper::default();
        let err = run_queued(&scope, &helix, &FixedClock(QUEUE_NOW), &sleeper, &always, THEM, "r1", None, None, false).await.unwrap_err();
        assert!(err.starts_with("Sending too fast"), "{err}");
        assert_eq!(helix.sends.load(Ordering::SeqCst), 4, "three retries after the first refusal");
        let waits = sleeper.0.lock().unwrap().clone();
        assert!(RATE_RETRIES_MS.iter().all(|w| waits.contains(w)), "{waits:?}");
        let m = stored(&scope, THEM, "r1").unwrap();
        assert_eq!(m.delivery, Some(Delivery::NotDelivered));
        assert!(m.failure.unwrap().starts_with("Sending too fast"));
    }

    #[tokio::test]
    async fn a_refused_token_is_renewed_once_and_checked_again() {
        let scope = whisper_store::open("99999953", 1);
        file(&scope, THEM, vec![local("t1", "hi", QUEUE_NOW, Some(Delivery::Sending)), local("t2", "hi again", QUEUE_NOW, Some(Delivery::Sending))]);
        let helix = ScriptHelix::new("99999953", vec![Err(SendFail::Status(401, String::new()))]);
        run_queued(&scope, &helix, &FixedClock(QUEUE_NOW), &NoSleep, &always, THEM, "t1", None, None, false).await.unwrap();
        assert_eq!((helix.sends.load(Ordering::SeqCst), helix.rejects.load(Ordering::SeqCst)), (2, 1));
        assert_eq!(stored(&scope, THEM, "t1").unwrap().delivery, Some(Delivery::Sent));

        // A renewed token that belongs to someone else sends nothing more.
        let mut other = ScriptHelix::new("99999953", vec![Err(SendFail::Status(401, String::new()))]);
        other.renewed_id = "1234567".into();
        let err = run_queued(&scope, &other, &FixedClock(QUEUE_NOW), &NoSleep, &always, THEM, "t2", None, None, false).await.unwrap_err();
        assert_eq!(err, SWITCHED);
        assert_eq!(other.sends.load(Ordering::SeqCst), 1);
        assert_eq!(stored(&scope, THEM, "t2").unwrap().failure.as_deref(), Some(SWITCHED));
    }

    #[tokio::test]
    async fn an_answer_after_the_account_closed_waits_for_teardown() {
        let scope = whisper_store::open("99999954", 1);
        file(&scope, THEM, vec![local("c1", "bye", QUEUE_NOW, Some(Delivery::Sending))]);
        let helix = ScriptHelix::new("99999954", Vec::new());
        let closed: std::sync::Arc<Mutex<Option<Closing>>> = std::sync::Arc::new(Mutex::new(None));
        let (keep, sc) = (std::sync::Arc::clone(&closed), scope.clone());
        *helix.on_send.lock().unwrap() = Some(Box::new(move || {
            *keep.lock().unwrap() = Some(whisper_store::close(&sc));
        }));
        let err = run_queued(&scope, &helix, &FixedClock(QUEUE_NOW), &NoSleep, &always, THEM, "c1", None, None, false).await.unwrap_err();
        assert_eq!(err, SIGNED_OUT_BEFORE);
        let stage = PENDING.lock().unwrap().get("c1").map(|p| p.stage.clone());
        assert_eq!(stage, Some(Stage::Answered(Outcome::Sent)), "nothing is written under the closed account");
        let closing = closed.lock().unwrap().take().unwrap();
        teardown(&closing).await;
        let after = whisper_store::update_closing(&closing, THEM, |c| (c.messages.iter().find(|m| m.id == "c1").map(|m| (m.delivery, m.got_204)), false))
            .unwrap()
            .unwrap();
        assert_eq!(after, Some((Some(Delivery::Unknown), true)), "teardown files it as Unknown, 204 kept for adoption");
    }

    #[test]
    fn a_conversation_stays_pinned_while_one_of_its_sends_is_live() {
        let mut p = HashMap::new();
        p.insert("a".to_string(), Pending { owner: ME.into(), key: THEM.into(), stage: Stage::Queued });
        assert!(conversation_sending(&p, ME, THEM));
        p.insert("a".to_string(), Pending { owner: ME.into(), key: THEM.into(), stage: Stage::Answered(Outcome::Sent) });
        assert!(!conversation_sending(&p, ME, THEM), "an answered send no longer holds it");
        p.insert("b".to_string(), Pending { owner: ME.into(), key: "other".into(), stage: Stage::InFlight });
        assert!(!conversation_sending(&p, ME, THEM));
        assert!(conversation_sending(&p, ME, "other"));
    }

    #[test]
    fn stale_sending_turns_unknown_and_live_sends_stay() {
        let scope = whisper_store::open("99999955", 1);
        let mut old = local("old", "from last time", 5_000, Some(Delivery::Sending));
        old.got_204 = true;
        file(&scope, THEM, vec![old, local("live", "going now", 6_000, Some(Delivery::Sending))]);
        set_stage("live", "99999955", THEM, Stage::InFlight);
        assert_eq!(settle_stale_sends(&scope), 1);
        let old = stored(&scope, THEM, "old").unwrap();
        assert_eq!((old.delivery, old.got_204), (Some(Delivery::Unknown), false));
        assert!(adoptable(&old), "Unknown is adopted by Twitch's copy");
        assert_eq!(stored(&scope, THEM, "live").unwrap().delivery, Some(Delivery::Sending));
        clear_stage("live");
    }

    #[test]
    fn restart_never_resends() {
        // What a crash left Sending is settled, never sent: no Helix call can
        // come from it, and nothing stays Sending.
        let scope = whisper_store::open("99999956", 1);
        file(&scope, THEM, vec![local("a", "hi", 1_000, Some(Delivery::Sending)), local("b", "yo", 2_000, Some(Delivery::NotDelivered))]);
        let helix = ScriptHelix::new("99999956", Vec::new());
        settle_stale_sends(&scope);
        let all = whisper_store::read(&scope, THEM, |c| c.messages.clone()).unwrap().unwrap();
        assert!(all.iter().all(|m| m.delivery != Some(Delivery::Sending)), "{:?}", all.iter().map(|m| m.delivery).collect::<Vec<_>>());
        assert_eq!(helix.sends.load(Ordering::SeqCst), 0);
        let mut checked = all.clone();
        apply_check(&mut checked, 50_000);
        apply_check(&mut checked, 60_000);
        assert!(checked.iter().all(|m| m.delivery != Some(Delivery::Sending)));
    }

    #[test]
    fn retry_resets_the_send() {
        let mut m = local("a", "hi", 0, Some(Delivery::NotDelivered));
        m.checks = 2;
        m.got_204 = true;
        m.failure = Some("x".into());
        reset_for_retry(&mut m, 5_000, -200);
        assert_eq!((m.delivery, m.checks, m.got_204, m.failure.clone()), (Some(Delivery::Sending), 0, false, None));
        assert_eq!((m.sent_local_ms, m.timestamp), (Some(5_000), 4_800));
    }

    #[test]
    fn normalize_folds_spacing_and_padding() {
        assert_eq!(normalize("  ok \u{E0000}  then "), "ok then");
    }
}
