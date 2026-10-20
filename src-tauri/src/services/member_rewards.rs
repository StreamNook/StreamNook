//! StreamNook's own rewards: the event and milestone rows in the `rewards`
//! table, claimed for the signed-in member on every client.
//!
//! These used to be claimed by the desktop page (`watchRewards.ts`), so a
//! member who only used the iPhone never received them. The server decides
//! everything (`claim_reward` checks the window, the category and channel
//! lists, and the real tenure months); this only asks:
//!
//! * at sign-in, every active event reward with no context (login-window
//!   events) and every active milestone reward (tenure badges);
//! * while a Twitch stream is on screen, every active event reward with the
//!   watched channel and its category, at the start, once a minute, and the
//!   moment the category changes.
//!
//! A definitive answer for a reward ends asking about it this session, and a
//! reward whose window the server reports closed is not asked about again. A
//! fresh grant re-reads the entitlement registries and is announced as
//! `reward-granted` so each shell can say so.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use log::{debug, info, warn};
use once_cell::sync::Lazy;
use serde::Serialize;
use serde_json::{json, Value};
use tauri::Emitter;

use crate::services::streamnook_backend as backend;

pub const GRANTED_EVENT: &str = "reward-granted";
/// The rewards list changes rarely; it is read again after this long.
const LIST_TTL: Duration = Duration::from_secs(5 * 60);
/// How often a watched stream is checked against the event rewards.
const WATCH_TICK: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Gate {
    /// Watch-to-earn and login-window events.
    Event,
    /// Subscriber-tenure badges.
    Milestone,
}

impl Gate {
    fn name(self) -> &'static str {
        match self {
            Gate::Event => "event",
            Gate::Milestone => "milestone",
        }
    }
}

#[derive(Clone, Debug)]
struct Reward {
    id: String,
    title: Option<String>,
}

#[derive(Serialize, Clone)]
struct Granted {
    reward_id: String,
    title: Option<String>,
}

/// One claim's answer, as `claim_reward` gives it.
#[derive(Debug, PartialEq)]
struct Answer {
    ok: bool,
    granted: bool,
    reason: Option<String>,
}

fn answer_of(v: &Value) -> Answer {
    Answer {
        ok: v.get("ok").and_then(Value::as_bool).unwrap_or(false),
        granted: v.get("granted").and_then(Value::as_bool).unwrap_or(false),
        reason: v.get("reason").and_then(Value::as_str).map(str::to_string),
    }
}

fn rewards_of(rows: &[Value]) -> Vec<Reward> {
    rows.iter()
        .filter_map(|r| {
            Some(Reward {
                id: r.get("id")?.as_str()?.to_string(),
                title: r.get("title").and_then(Value::as_str).map(str::to_string),
            })
        })
        .collect()
}

/// The last list read per gate, with when it was read.
static LISTS: Lazy<Mutex<[(Option<Instant>, Vec<Reward>); 2]>> = Lazy::new(Default::default);
/// `member:reward` pairs with a definitive answer this session.
static ANSWERED: Lazy<Mutex<HashSet<String>>> = Lazy::new(Default::default);
/// Rewards whose window the server reported closed.
static CLOSED: Lazy<Mutex<HashSet<String>>> = Lazy::new(Default::default);

fn slot(gate: Gate) -> usize {
    match gate {
        Gate::Event => 0,
        Gate::Milestone => 1,
    }
}

/// The active rewards of one kind. A failed read keeps the last good list.
async fn active(gate: Gate) -> Vec<Reward> {
    {
        let lists = LISTS.lock().unwrap_or_else(|e| e.into_inner());
        let (at, rows) = &lists[slot(gate)];
        if at.is_some_and(|t| t.elapsed() < LIST_TTL) {
            return rows.clone();
        }
    }
    let query = format!("select=id,title,reward_kind,reward_id&active=eq.true&gate_kind=eq.{}", gate.name());
    match backend::select("rewards", &query).await {
        Ok(rows) => {
            let rewards = rewards_of(&rows);
            LISTS.lock().unwrap_or_else(|e| e.into_inner())[slot(gate)] = (Some(Instant::now()), rewards.clone());
            rewards
        }
        Err(e) => {
            warn!("[Rewards] {} rewards list: {e}", gate.name());
            LISTS.lock().unwrap_or_else(|e| e.into_inner())[slot(gate)].1.clone()
        }
    }
}

/// Asks the server for every active reward of `gate` with `context`.
pub async fn claim(member_id: &str, gate: Gate, context: Value) {
    if member_id.is_empty() || !backend::configured() {
        return;
    }
    let mut fresh = false;
    for reward in active(gate).await {
        let key = format!("{member_id}:{}", reward.id);
        if CLOSED.lock().unwrap_or_else(|e| e.into_inner()).contains(&reward.id)
            || ANSWERED.lock().unwrap_or_else(|e| e.into_inner()).contains(&key)
        {
            continue;
        }
        let args = json!({ "p_twitch_user_id": member_id, "p_reward_id": reward.id, "p_context": context });
        let answer = match backend::rpc("claim_reward", args).await {
            Ok(v) => answer_of(&v),
            Err(e) => {
                warn!("[Rewards] claim {} failed: {e}", reward.id);
                continue;
            }
        };
        if answer.reason.as_deref() == Some("window_closed") {
            CLOSED.lock().unwrap_or_else(|e| e.into_inner()).insert(reward.id.clone());
            continue;
        }
        if !answer.ok {
            // not_eligible and below_threshold are the ordinary "not for you
            // right now"; anything else is worth a line.
            match answer.reason.as_deref() {
                Some("not_eligible") | Some("below_threshold") => {}
                other => warn!("[Rewards] claim {} refused: {}", reward.id, other.unwrap_or("unknown")),
            }
            continue;
        }
        ANSWERED.lock().unwrap_or_else(|e| e.into_inner()).insert(key);
        if answer.granted {
            info!("[Rewards] granted {} to {member_id}", reward.id);
            fresh = true;
            if let Some(app) = crate::services::providers::app_handle() {
                let _ = app.emit(GRANTED_EVENT, Granted { reward_id: reward.id.clone(), title: reward.title.clone() });
            }
        }
    }
    // The grant just wrote the entitlement: read it now rather than wait for
    // the realtime feed, which may not be subscribed yet at sign-in.
    if fresh {
        crate::services::cosmetics_registry::refresh().await;
    }
}

/// The signed-in Twitch member, from the stored primary account.
fn member_id() -> Option<String> {
    crate::services::account_store::AccountStore::primary().map(|a| a.user_id).filter(|id| !id.is_empty())
}

/// The watched Twitch stream as reward context: its channel and category. Some
/// ways into a stream (a raid, a direct open) carry no category, so the
/// channel's own state supplies it then.
async fn watch_context() -> Option<Value> {
    let target = crate::services::watch_session::current_target().filter(|t| t.provider == "twitch")?;
    let login = target.login.to_lowercase();
    let mut category = target.game_name.clone();
    if category.is_empty() {
        if let Some(state) = crate::services::channel_state::get(&login).await {
            category = state.game_name.unwrap_or_default();
        }
    }
    Some(json!({ "category": category, "channel": login }))
}

static TICKING: AtomicBool = AtomicBool::new(false);

/// The watched stream changed (a start, or its category): ask at once, and keep
/// asking once a minute while a Twitch stream stays on screen.
pub fn watching_changed() {
    tauri::async_runtime::spawn(async {
        if let (Some(member), Some(context)) = (member_id(), watch_context().await) {
            claim(&member, Gate::Event, context).await;
        }
    });
    if TICKING.swap(true, Ordering::AcqRel) {
        return;
    }
    tauri::async_runtime::spawn(async {
        loop {
            tokio::time::sleep(WATCH_TICK).await;
            let Some(context) = watch_context().await else { break };
            if let Some(member) = member_id() {
                debug!("[Rewards] watch tick {context}");
                claim(&member, Gate::Event, context).await;
            }
        }
        TICKING.store(false, Ordering::Release);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_claim_answer() {
        let a = answer_of(&json!({ "ok": true, "granted": true, "reward_id": "r1" }));
        assert_eq!(a, Answer { ok: true, granted: true, reason: None });
        let b = answer_of(&json!({ "ok": false, "reason": "window_closed" }));
        assert_eq!(b.reason.as_deref(), Some("window_closed"));
        assert!(!b.ok && !b.granted);
        // A null or odd answer is a refusal, never a grant.
        assert_eq!(answer_of(&Value::Null), Answer { ok: false, granted: false, reason: None });
    }

    #[test]
    fn reads_the_rewards_rows() {
        let rows = vec![
            json!({ "id": "halloween", "title": "Hallowtide", "reward_kind": "badge", "reward_id": "b1" }),
            json!({ "id": "untitled", "title": null }),
            json!({ "title": "no id" }),
        ];
        let r = rewards_of(&rows);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].id, "halloween");
        assert_eq!(r[0].title.as_deref(), Some("Hallowtide"));
        assert_eq!(r[1].title, None);
    }
}
