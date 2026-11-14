//! Fakes for the engine's tests: a fake door to the outside (with a fake
//! claim server that follows the claim function's element rules, and a fake
//! token fetch and validate under the real `TokenVerifier`), a fake in-app
//! seam, rule and row builders, and a `Kit` that wires one engine.
//!
//! Every test thread gets its own account ids and chat, because the send
//! limiters are process-wide: two tests never share a limiter or a chat.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::{watch, Notify};

use crate::commands::streamnook_api::ApiResponse;
use crate::models::automations::{AutomationPrefs, Automations, Away, RunState};
use crate::models::chat_layout::ChatMessage;
use crate::services::channel_state::ChannelState;
use crate::services::chat_send_limiter::{self, Ticket};
use crate::services::chat_service::SendOutcome;

use super::activity::{Entry, Filter};
use super::badges::{BadgeInfo, BadgePhase, BadgeSeen, Presence, Probe};
use super::claim_client::ClaimTimings;
use super::claims::{self, Element, KeyMode};
use super::executor::Job;
use super::facts::Facts;
use super::own_topics::{OwnState, TopicKind, TopicRefusal};
use super::speak::SpeakPayload;
use super::guards;
use super::local::{AccountChange, AccountChanges, Local};
use super::notify::{NotifyPayload, NotifyRequest};
use super::outward::{ChatSend, CommandDoors, ModDoors, Outward, WhisperDoor, WhisperTo};
use super::local::WhisperPerson;
use super::timed;
use super::timers;
use crate::services::twitch_service::{DoorOutcome, SearchFail};
use crate::services::whisper_send::Outcome as WhisperOutcome;
use crate::services::whisper_service::WhisperEvent;
use super::store::Device;
pub use super::verify::{BoxFuture, Fetch, Identity, TokenRefusal, TokenVerifier, Validation, VerifiedToken, VerifyIo};
use super::{facts, lock, now_ms, Clock, Deps, Engine, Spawn, StatusSnapshot, Timings};

static NEXT_IDS: AtomicU64 = AtomicU64::new(1);

thread_local! {
    static IDS: (String, String, String) = {
        let n = NEXT_IDS.fetch_add(1, Ordering::SeqCst);
        (format!("9{n:05}01"), format!("9{n:05}02"), format!("testchan{n}"))
    };
}

/// This test's main account id (its login is `me()`, the test's own channel).
pub fn main_id() -> String {
    IDS.with(|ids| ids.0.clone())
}

/// This test's linked account id (login "alt").
pub fn alt_id() -> String {
    IDS.with(|ids| ids.1.clone())
}

/// This test's chat (login), room id 4242.
pub fn chan() -> String {
    IDS.with(|ids| ids.2.clone())
}

/// The login of this test's main account: the test's own chat, so a rule on
/// that chat acts in its owner's own channel (no "other channels" rail).
pub fn me() -> String {
    chan()
}

/// The same login from the main account's id alone (the fakes run off-thread).
fn main_login(main: &str) -> String {
    let n: u64 = main.get(1..6).and_then(|n| n.parse().ok()).unwrap_or(0);
    format!("testchan{n}")
}

/// The fake token of an account.
pub fn token_of(id: &str) -> String {
    format!("tok-{id}")
}

pub fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

/// The waits, in milliseconds.
pub fn fast() -> Timings {
    Timings {
        remove_wait: Duration::from_millis(150),
        notify_confirm: Duration::from_millis(80),
        chat_start_first: Duration::from_millis(200),
        chat_start_cap: Duration::from_secs(2),
        activity_emit: Duration::from_millis(5),
        gallery_refresh: Duration::from_secs(6 * 3600),
        gallery_retry: Duration::from_secs(1),
        job_release_wait: Duration::from_millis(300),
        adopt_wait: Duration::from_secs(1),
        batcher_tick: Duration::from_millis(20),
        counts_flush: Duration::from_millis(50),
        activity_flush: Duration::from_millis(50),
        claims: ClaimTimings {
            reclaim_at: [
                Duration::from_millis(150),
                Duration::from_millis(300),
                Duration::from_millis(600),
                Duration::from_millis(1_200),
            ],
            release_retry: [Duration::from_millis(20), Duration::from_millis(50)],
            unknown_repeat: Duration::from_millis(50),
            reminder_bound: Duration::from_millis(500),
        },
    }
}

/// An engine's parts on fakes, for a test that builds the engine itself.
pub fn test_deps() -> Deps {
    let clock = TestClock::new().as_clock();
    Deps {
        outward: Arc::new(FakeOutward::new(Arc::new(FakeServer::default()), clock.clone())),
        local: Arc::new(FakeLocal::new()),
        app: None,
        files: None,
        device: Device::fresh(),
        timings: fast(),
        clock,
        spawn: test_spawner(),
    }
}

/// The engine's tasks on the test's own runtime, so `start_paused` time moves
/// them; a plain `#[test]` with no runtime falls back to Tauri's.
pub fn test_spawner() -> Spawn {
    Arc::new(|task: futures::future::BoxFuture<'static, ()>| match tokio::runtime::Handle::try_current() {
        Ok(runtime) => {
            runtime.spawn(task);
        }
        Err(_) => {
            tauri::async_runtime::spawn(task);
        }
    })
}

/// A steady clock tests move by hand (milliseconds).
#[derive(Clone)]
pub struct TestClock {
    base: Instant,
    offset_ms: Arc<AtomicI64>,
}

impl TestClock {
    pub fn new() -> TestClock {
        TestClock { base: Instant::now(), offset_ms: Arc::new(AtomicI64::new(0)) }
    }

    pub fn advance(&self, by: Duration) {
        self.offset_ms.fetch_add(by.as_millis() as i64, Ordering::SeqCst);
    }

    pub fn as_clock(&self) -> Clock {
        let me = self.clone();
        Arc::new(move || me.base.elapsed().as_millis() as i64 + me.offset_ms.load(Ordering::SeqCst))
    }
}

// ---- rows and elements --------------------------------------------------------

pub fn chat_row(id: &str, user: &str, text: &str) -> ChatMessage {
    row(json!({ "id": id, "room-id": "4242" }), id, user, text, json!({}))
}

/// A row with no `room-id` tag, so a send must look the room up.
pub fn untagged_row(id: &str, user: &str, text: &str) -> ChatMessage {
    row(json!({ "id": id }), id, user, text, json!({}))
}

/// A Shared Chat row: `room` is this copy's room, `source_room` the origin's.
pub fn shared_row(id: &str, source_id: &str, room: &str, source_room: Option<&str>) -> ChatMessage {
    let mut tags = json!({ "id": id, "source-id": source_id, "room-id": room });
    if let Some(src) = source_room {
        tags["source-room-id"] = json!(src);
    }
    let meta = json!({ "source_room_id": source_room, "is_from_shared_chat": source_room.is_some() });
    row(tags, id, "viewer", "hey @me", meta)
}

fn row(tags: Value, id: &str, user: &str, text: &str, metadata: Value) -> ChatMessage {
    // "me" in a row is this test's main account, whose login is its own chat.
    let user = if user == "me" { me() } else { user.to_string() };
    let user = user.as_str();
    let text = text.replace("@me", &format!("@{}", me()));
    serde_json::from_value(json!({
        "id": id,
        "user_id": format!("u-{user}"),
        "username": user,
        "display_name": user,
        "color": null,
        "badges": [],
        "timestamp": "",
        "content": text,
        "provider": "twitch",
        "channel": chan(),
        "tags": tags,
        "metadata": metadata,
        "received_at_ms": now_ms(),
    }))
    .expect("a test row")
}

pub fn badge_seen(feed_id: &str) -> BadgeSeen {
    BadgeSeen {
        badge: BadgeInfo { set_id: feed_id.into(), version: "1".into(), name: format!("Badge {feed_id}"), ..BadgeInfo::default() },
        feed_id: feed_id.into(),
        phase: BadgePhase::Announced,
        announced: true,
        earnable: false,
        ts_ms: now_ms(),
        arrived_ms: now_ms(),
        push: true,
        in_gallery: Presence::No,
    }
}

pub fn event_el(key: &str) -> Element {
    Element::Event {
        key: key.into(),
        family: "mention:|post:4242".into(),
        mode: KeyMode::Exact,
        window_s: None,
        sends: 1,
        to_person: true,
        settle_ms: 15_000,
    }
}

pub fn windowed_event_el(key: &str, window_s: u32) -> Element {
    Element::Event {
        key: key.into(),
        family: "timer:0123456789abcdef|post:4242".into(),
        mode: KeyMode::Windowed,
        window_s: Some(window_s),
        sends: 1,
        to_person: false,
        settle_ms: 15_000,
    }
}

pub fn rail_el(key: &str, window_s: u32) -> Element {
    let kind = key.split(':').next().unwrap_or("gap");
    Element::Windowed { key: key.into(), family: claims::rail_family(kind), window_s }
}

// ---- rules ----------------------------------------------------------------------

pub fn account(id: &str, login: &str) -> Value {
    json!({ "provider": "twitch", "id": id, "login": login, "name": login })
}

fn chat_ref(login: &str, room: &str) -> Value {
    json!({ "id": format!("c-{login}"), "key": format!("twitch:{login}"), "room_id": room, "name": login })
}

fn reply(id: &str, text: &str) -> Value {
    json!({ "kind": "reply", "id": id, "texts": [{ "id": format!("l-{id}"), "text": text }], "pick": "in_order", "thread": false })
}

fn send(id: &str, to: Option<Value>, text: &str) -> Value {
    json!({ "kind": "send", "id": id, "to": to, "texts": [{ "id": format!("l-{id}"), "text": text }], "pick": "in_order" })
}

fn notify_action() -> Value {
    json!({ "kind": "notify", "id": "a-n", "title": "Mention", "body": "{user} said hi", "toast": true, "system": false, "sound": null, "volume": 1.0 })
}

fn rule(id: &str, live: bool, when: Value, channels: Value, actions: Vec<Value>) -> Value {
    json!({
        "id": id,
        "name": format!("Rule {id}"),
        "enabled": true,
        "live": live,
        "owner": account(&main_id(), &me()),
        "send_as": account(&main_id(), &me()),
        "when": when,
        "only_if": { "channels": channels, "roles": [], "skip_users": [], "only_users": [], "only_live": false,
                     "only_away": false, "shared_chat": false, "hours": null },
        "actions": actions,
        "limits": { "rule_cooldown_seconds": 0, "user_cooldown_seconds": 0, "max_per_hour": 120, "max_wait_seconds": 10 },
        "vary": false,
        "origin": null,
        "created_ms": 1,
        "updated_ms": 1,
    })
}

fn mention() -> Value {
    json!({ "kind": "mention", "replies_too": true })
}

fn this_chat() -> Value {
    json!({ "kind": "these", "list": [chat_ref(&chan(), "4242")] })
}

/// Mention in this test's chat, Reply "@{user} thanks (<id>)".
pub fn mention_rule(id: &str, live: bool) -> Value {
    rule(id, live, mention(), this_chat(), vec![reply("a-1", &format!("@{{user}} thanks ({id})"))])
}

/// A rule of the main account in its own channel on a P3 trigger (`when` is
/// the trigger's JSON), Live or Test, with these actions.
pub fn own_rule(id: &str, live: bool, when: Value, actions: Vec<Value>) -> Value {
    rule(id, live, when, json!({ "kind": "own" }), actions)
}

/// A Live rule that watches one channel for it going live or offline.
pub fn live_change_rule(id: &str, online: bool, friend: &str, room: &str, actions: Vec<Value>) -> Value {
    rule(
        id,
        true,
        json!({ "kind": "live_change", "online": online }),
        json!({ "kind": "these", "list": [chat_ref(friend, room)] }),
        actions,
    )
}

/// A Message rule in your own channel whose only action is a Highlight.
pub fn highlight_rule(id: &str, pattern: &str, color: &str) -> Value {
    rule(
        id,
        true,
        json!({ "kind": "message", "text": { "mode": "contains", "pattern": pattern, "case_sensitive": false } }),
        json!({ "kind": "own" }),
        vec![json!({ "kind": "highlight", "id": "a-h", "color": color })],
    )
}

pub fn notify_titled(title: &str) -> Value {
    json!({ "kind": "notify", "id": "a-n", "title": title, "body": "", "toast": true, "system": false, "sound": null, "volume": 1.0 })
}

pub fn send_action(id: &str, text: &str) -> Value {
    send(id, None, text)
}

pub fn speak_action(text: &str, on_stream: bool) -> Value {
    json!({ "kind": "speak", "id": "a-s", "text": text, "on_stream": on_stream })
}

/// A `channel.follow` event's `event` object.
pub fn follow_event(user_id: &str, name: &str) -> Value {
    json!({ "user_id": user_id, "user_login": name.to_lowercase(), "user_name": name, "followed_at": "2026-11-13T05:00:00Z" })
}

/// The facts of that follow in the main account's channel.
pub fn follow_facts(user_id: &str) -> Facts {
    facts::from_own(TopicKind::Follow, &follow_event(user_id, "Fan"), &main_id(), &me(), now_ms()).expect("follow facts")
}

/// The same rule sending as another account.
pub fn as_sender(mut rule: Value, id: &str, login: &str) -> Value {
    rule["send_as"] = account(id, login);
    rule
}

pub fn mention_send_here_rule(id: &str, live: bool) -> Value {
    rule(id, live, mention(), this_chat(), vec![send("a-1", None, &format!("hi {{user}} ({id})"))])
}

pub fn mention_send_to_rule(id: &str, live: bool, login: &str, room: &str) -> Value {
    rule(id, live, mention(), this_chat(), vec![send("a-1", Some(chat_ref(login, room)), &format!("ping from {{user}} ({id})"))])
}

pub fn reply_send_notify_rule(id: &str, live: bool) -> Value {
    rule(
        id,
        live,
        mention(),
        this_chat(),
        vec![reply("a-1", "@{user} one"), send("a-2", Some(chat_ref("elsewhere", "777")), "two {user}"), notify_action()],
    )
}

pub fn reply_notify_rule(id: &str, live: bool) -> Value {
    rule(id, live, mention(), this_chat(), vec![reply("a-1", &format!("@{{user}} thanks ({id})")), notify_action()])
}

pub fn two_replies_rule(id: &str, live: bool) -> Value {
    rule(id, live, mention(), this_chat(), vec![reply("a-1", "@{user} first"), reply("a-2", "@{user} second")])
}

/// The Kick account id the fake door and the Kick rules use.
pub const KICK_ME: &str = "5001";

fn kick_rule(id: &str, channels: Value) -> Value {
    let when = json!({ "kind": "message", "text": { "mode": "contains", "pattern": "hello", "case_sensitive": false } });
    let mut r = rule(id, true, when, channels, vec![reply("a-1", "hi {user}")]);
    r["send_as"] = json!({ "provider": "kick", "id": KICK_ME, "login": "mykick", "name": "MyKick" });
    r["only_if"]["platforms"] = json!(["kick"]);
    r
}

/// A Kick rule listening in these Kick chats (room id 5002 each).
pub fn kick_these_rule(id: &str, chats: &[&str]) -> Value {
    let list: Vec<Value> = chats
        .iter()
        .map(|c| json!({ "id": format!("c-{c}"), "key": format!("kick:{c}"), "room_id": "5002", "name": c }))
        .collect();
    kick_rule(id, json!({ "kind": "these", "list": list }))
}

/// A Kick rule listening in any open chat.
pub fn kick_any_open_rule(id: &str) -> Value {
    kick_rule(id, json!({ "kind": "any_open" }))
}

/// A Kick chat row from `fan` in `kick:alpha` (room 5002), read now.
pub fn kick_row(id: &str, user_id: &str, text: &str) -> ChatMessage {
    kick_row_from(id, user_id, "fan", text)
}

pub fn kick_row_from(id: &str, user_id: &str, login: &str, text: &str) -> ChatMessage {
    serde_json::from_value(json!({
        "id": id, "user_id": user_id, "username": login, "display_name": login, "color": null,
        "badges": [], "timestamp": "", "content": text, "provider": "kick", "channel": "kick:alpha",
        "tags": { "id": id, "room-id": "5002" }, "metadata": {}, "received_at_ms": now_ms(),
    }))
    .expect("a Kick test row")
}

pub fn these_rule(id: &str, chats: &[&str], only_live: bool) -> Value {
    let list: Vec<Value> = chats.iter().map(|c| chat_ref(c, "4242")).collect();
    let mut r = rule(id, true, mention(), json!({ "kind": "these", "list": list }), vec![reply("a-1", "@{user} hi")]);
    r["only_if"]["only_live"] = json!(only_live);
    r
}

pub fn these_reply_rule(id: &str, chats: &[&str], live: bool) -> Value {
    let list: Vec<Value> = chats.iter().map(|c| chat_ref(c, "777")).collect();
    rule(id, live, mention(), json!({ "kind": "these", "list": list }), vec![reply("a-1", &format!("@{{user}} hi ({id})"))])
}

pub fn any_open_rule(id: &str) -> Value {
    rule(id, true, mention(), json!({ "kind": "any_open" }), vec![reply("a-1", "@{user} hi")])
}

pub fn away_rule(id: &str, live: bool) -> Value {
    let mut r = rule(id, live, mention(), this_chat(), vec![reply("a-1", "@{user} I'm away right now: {away}")]);
    r["only_if"]["only_away"] = json!(true);
    r
}

// ---- small sync helpers ----------------------------------------------------------

/// A gate tests open and close.
struct Gate(watch::Sender<bool>);

impl Gate {
    fn open() -> Gate {
        Gate(watch::channel(true).0)
    }
    fn close(&self) {
        self.0.send_replace(false);
    }
    fn release(&self) {
        self.0.send_replace(true);
    }
    async fn pass(&self) {
        let mut rx = self.0.subscribe();
        let _ = rx.wait_for(|open| *open).await;
    }
}

/// A counter tests can wait on.
#[derive(Default)]
struct Started {
    count: AtomicUsize,
    notify: Notify,
}

impl Started {
    fn hit(&self) {
        self.count.fetch_add(1, Ordering::SeqCst);
        self.notify.notify_waiters();
    }
    async fn reach(&self, n: usize) {
        let wait = async {
            loop {
                let notified = self.notify.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                if self.count.load(Ordering::SeqCst) >= n {
                    return;
                }
                notified.await;
            }
        };
        tokio::time::timeout(Duration::from_secs(5), wait).await.expect("the fake was never reached");
    }
}

// ---- the fake claim server ----------------------------------------------------------

#[derive(Clone, Debug)]
struct Holder {
    session: String,
    device_hash: String,
    name: String,
    calls: Vec<(String, i64, i64)>,
}

#[derive(Clone, Debug)]
struct WinRow {
    session: String,
    device_hash: String,
    name: String,
    last_at: i64,
    calls: Vec<String>,
}

#[derive(Default)]
struct ServerState {
    exact: HashMap<(String, String, String), Holder>,
    windows: HashMap<(String, String, String), WinRow>,
    /// (account, key, call, at).
    counted: Vec<(String, String, String, i64)>,
    /// What each call's wins replaced: (row key, the row before).
    saved: HashMap<String, Vec<((String, String, String), Option<WinRow>)>>,
    held_prefixes: Vec<(String, String)>,
    skew_ms: i64,
}

/// One claim server shared by every "device" of a test.
#[derive(Default)]
pub struct FakeServer {
    state: Mutex<ServerState>,
}

impl FakeServer {
    pub fn skew(&self, ms: i64) {
        lock(&self.state).skew_ms = ms;
    }

    /// `n` counted rows already in the period for keys starting with `prefix`
    /// (the pace of `account`).
    pub fn fill_counted(&self, account: &str, prefix: &str, n: usize) {
        let mut s = lock(&self.state);
        let at = now_ms() + s.skew_ms;
        for i in 0..n {
            s.counted.push((account.into(), format!("{prefix}{account}"), format!("pre{i}"), at));
        }
    }

    /// Every windowed element whose key starts with `prefix` is held by
    /// another device named `name` for a minute.
    pub fn hold_windows(&self, prefix: &str, name: &str) {
        lock(&self.state).held_prefixes.push((prefix.into(), name.into()));
    }

    /// The database's default element: every key present, `null` where its
    /// default object has `null`, and `same_device` false.
    fn answer(index: usize, kind: &str, won: bool) -> Value {
        json!({ "index": index, "kind": kind, "won": won, "holder_name": null, "same_device": false,
                "holder_age_ms": null, "holder_settle_ms": null, "free_at_ms": null, "retry_after_ms": null,
                "reason": if won { Value::Null } else { json!("skipped") } })
    }

    pub fn handle(&self, account: &str, body: &Value) -> Value {
        let mut s = lock(&self.state);
        let now = now_ms() + s.skew_ms;
        let call = body["call_id"].as_str().unwrap_or_default().to_string();
        let session = body["session"].as_str().unwrap_or_default().to_string();
        if body["action"] == "release" {
            return Self::release(&mut s, account, &call, &session);
        }
        let device_hash = body["device_hash"].as_str().unwrap_or_default().to_string();
        let name = body["name"].as_str().unwrap_or_default().to_string();
        let elements = body["elements"].as_array().cloned().unwrap_or_default();
        let before = (s.exact.clone(), s.windows.clone(), s.counted.clone(), s.saved.clone());
        let mut answers: Vec<Value> = elements.iter().enumerate().map(|(i, e)| Self::answer(i, e["kind"].as_str().unwrap_or(""), false)).collect();
        let mut won_sends = 0u64;
        let mut won_person = 0u64;
        let mut any_won = false;
        for (i, e) in elements.iter().enumerate().filter(|(_, e)| e["kind"] == "event") {
            let key = (account.to_string(), e["key"].as_str().unwrap_or("").to_string(), e["family"].as_str().unwrap_or("").to_string());
            let settle = e["settle_ms"].as_i64().unwrap_or(0);
            let windowed = e["mode"] == "windowed";
            let win_ms = e["window_s"].as_i64().unwrap_or(0) * 1000;
            let won = if windowed {
                // Once per window: a row another session wrote inside the window holds it
                // until the window ends (the answer says when).
                let key_text = key.1.clone();
                if let Some((_, holder)) = s.held_prefixes.iter().find(|(p, _)| key_text.starts_with(p.as_str())).cloned() {
                    answers[i] = json!({ "index": i, "kind": "event", "won": false, "holder_name": holder,
                        "same_device": false, "holder_age_ms": 0, "holder_settle_ms": null, "free_at_ms": now + 60_000,
                        "retry_after_ms": null, "reason": "held" });
                    false
                } else {
                    let prior = s.windows.get(&key).cloned();
                    match &prior {
                        Some(row) if row.session != session && row.last_at >= now - win_ms => {
                            answers[i] = json!({ "index": i, "kind": "event", "won": false, "holder_name": row.name,
                                "same_device": row.device_hash == device_hash, "holder_age_ms": now - row.last_at,
                                "holder_settle_ms": null, "free_at_ms": row.last_at + win_ms, "retry_after_ms": null, "reason": "held" });
                            false
                        }
                        _ => {
                            s.saved.entry(call.clone()).or_default().push((key.clone(), prior));
                            s.windows.insert(key, WinRow { session: session.clone(), device_hash: device_hash.clone(), name: name.clone(), last_at: now, calls: vec![call.clone()] });
                            true
                        }
                    }
                }
            } else {
              match s.exact.get_mut(&key) {
                None => {
                    s.exact.insert(key, Holder { session: session.clone(), device_hash: device_hash.clone(), name: name.clone(), calls: vec![(call.clone(), now, settle)] });
                    true
                }
                Some(h) if h.session == session => {
                    h.calls.push((call.clone(), now, settle));
                    true
                }
                Some(h) => {
                    let newest = h.calls.last().cloned().unwrap_or_default();
                    answers[i] = json!({ "index": i, "kind": "event", "won": false, "holder_name": h.name,
                        "same_device": h.device_hash == device_hash, "holder_age_ms": now - newest.1,
                        "holder_settle_ms": newest.2, "free_at_ms": null, "retry_after_ms": null, "reason": "held" });
                    false
                }
              }
            };
            if won {
                any_won = true;
                won_sends += e["sends"].as_u64().unwrap_or(0);
                if e["to_person"] == json!(true) {
                    won_person += e["sends"].as_u64().unwrap_or(0);
                }
                answers[i] = Self::answer(i, "event", true);
            }
        }
        if !any_won {
            return json!({ "elements": answers, "server_now_ms": now });
        }
        let batch = elements.iter().find(|e| e["kind"] == "batch").and_then(|e| e["sends"].as_u64());
        let mut loser: Option<(usize, Value)> = None;
        for (i, e) in elements.iter().enumerate() {
            match e["kind"].as_str() {
                Some("batch") => answers[i] = Self::answer(i, "batch", true),
                Some("windowed") => {
                    let key_text = e["key"].as_str().unwrap_or("").to_string();
                    let kind = key_text.split(':').next().unwrap_or("").to_string();
                    let window_ms = e["window_s"].as_i64().unwrap_or(0) * 1000;
                    if let Some((_, holder)) = s.held_prefixes.iter().find(|(p, _)| key_text.starts_with(p.as_str())) {
                        loser = Some((i, json!({ "index": i, "kind": "windowed", "won": false, "holder_name": holder,
                            "same_device": false, "holder_age_ms": 0, "holder_settle_ms": null, "free_at_ms": now + 60_000,
                            "retry_after_ms": null, "reason": kind })));
                        break;
                    }
                    let key = (account.to_string(), key_text, e["family"].as_str().unwrap_or("").to_string());
                    let prior = s.windows.get(&key).cloned();
                    match &prior {
                        Some(row) if row.session != session && row.last_at >= now - window_ms => {
                            loser = Some((i, json!({ "index": i, "kind": "windowed", "won": false, "holder_name": row.name,
                                "same_device": row.device_hash == device_hash, "holder_age_ms": now - row.last_at,
                                "holder_settle_ms": null, "free_at_ms": row.last_at + window_ms, "retry_after_ms": null, "reason": kind })));
                            break;
                        }
                        _ => {
                            s.saved.entry(call.clone()).or_default().push((key.clone(), prior));
                            s.windows.insert(key, WinRow { session: session.clone(), device_hash: device_hash.clone(), name: name.clone(), last_at: now, calls: vec![call.clone()] });
                            answers[i] = Self::answer(i, "windowed", true);
                        }
                    }
                }
                Some("counted") => {
                    let key_text = e["key"].as_str().unwrap_or("").to_string();
                    let kind = key_text.split(':').next().unwrap_or("").to_string();
                    let period_ms = e["period_s"].as_i64().unwrap_or(60) * 1000;
                    let limit = e["limit"].as_u64().unwrap_or(1);
                    let n = batch.unwrap_or(if e["counts"] == "person" { won_person } else { won_sends });
                    let in_period: Vec<i64> = s
                        .counted
                        .iter()
                        .filter(|(a, k, _, at)| a == account && *k == key_text && *at > now - period_ms)
                        .map(|(_, _, _, at)| *at)
                        .collect();
                    if n > 0 && in_period.len() as u64 + n > limit {
                        // A duration: how long until the oldest counted send leaves the period.
                        let oldest = in_period.iter().copied().min().unwrap_or(now);
                        let retry_after_ms = (oldest + period_ms - now).max(0);
                        loser = Some((i, json!({ "index": i, "kind": "counted", "won": false, "holder_name": null,
                            "same_device": false, "holder_age_ms": null, "holder_settle_ms": null, "free_at_ms": null,
                            "retry_after_ms": retry_after_ms, "reason": kind })));
                        break;
                    }
                    for _ in 0..n {
                        s.counted.push((account.to_string(), key_text.clone(), call.clone(), now));
                    }
                    answers[i] = Self::answer(i, "counted", true);
                }
                _ => {}
            }
        }
        if let Some((index, lost)) = loser {
            (s.exact, s.windows, s.counted, s.saved) = before;
            let mut rolled: Vec<Value> = elements.iter().enumerate().map(|(i, e)| {
                let mut a = Self::answer(i, e["kind"].as_str().unwrap_or(""), false);
                a["reason"] = json!("rail");
                a
            }).collect();
            rolled[index] = lost;
            return json!({ "elements": rolled, "server_now_ms": now });
        }
        json!({ "elements": answers, "server_now_ms": now })
    }

    fn release(s: &mut ServerState, account: &str, call: &str, session: &str) -> Value {
        let mut released = 0;
        s.exact.retain(|(a, _, _), h| {
            if a != account || h.session != session {
                return true;
            }
            let before = h.calls.len();
            h.calls.retain(|(c, _, _)| c != call);
            released += before - h.calls.len();
            !h.calls.is_empty()
        });
        if let Some(saved) = s.saved.remove(call) {
            for (key, prior) in saved {
                released += 1;
                match prior {
                    Some(row) => {
                        s.windows.insert(key, row);
                    }
                    None => {
                        s.windows.remove(&key);
                    }
                }
            }
        }
        let before = s.counted.len();
        s.counted.retain(|(a, _, c, _)| !(a == account && c == call));
        released += before - s.counted.len();
        json!({ "released": released })
    }
}

// ---- the fake door ---------------------------------------------------------------------

/// How the fake answers for one account's token, as a shortcut over the
/// fetch and validate fakes below.
#[derive(Clone, Debug, PartialEq)]
pub enum FakeToken {
    Good,
    NoToken,
    Refused,
    WrongAccount(String),
    Unreachable,
}

/// What a fake Command door answers and records. Every call is also pushed onto
/// the fake outward's `calls()` log (`command:chat_settings`, ...), so a test
/// reads the order search, claim, write.
pub struct FakeCommandDoors {
    state: Arc<Mutex<OutState>>,
    pub log: Mutex<Vec<String>>,
    pub settings: Mutex<Vec<(String, String, String, Value)>>,
    pub markers: Mutex<Vec<(String, String, Option<String>)>>,
    pub infos: Mutex<Vec<(String, String, Option<String>, Option<String>)>>,
    pub searches: Mutex<Vec<(String, String)>>,
    /// Answers in order; Sent when empty.
    pub answers: Mutex<VecDeque<DoorOutcome>>,
    /// The `SetGame` read's answer; an empty list when unset.
    pub search_answer: Mutex<Option<Result<Vec<Value>, SearchFail>>>,
}

impl FakeCommandDoors {
    fn new(state: Arc<Mutex<OutState>>) -> FakeCommandDoors {
        FakeCommandDoors {
            state,
            log: Mutex::new(Vec::new()),
            settings: Mutex::new(Vec::new()),
            markers: Mutex::new(Vec::new()),
            infos: Mutex::new(Vec::new()),
            searches: Mutex::new(Vec::new()),
            answers: Mutex::new(VecDeque::new()),
            search_answer: Mutex::new(None),
        }
    }

    fn note(&self, name: &str) {
        lock(&self.state).calls.push(format!("command:{name}"));
        lock(&self.log).push(name.to_string());
    }

    fn next(&self) -> DoorOutcome {
        lock(&self.answers).pop_front().unwrap_or(DoorOutcome::Sent)
    }
}

#[async_trait]
impl CommandDoors for FakeCommandDoors {
    async fn chat_settings(&self, token: &str, moderator_id: &str, room: &str, settings: &Value) -> DoorOutcome {
        self.note("chat_settings");
        lock(&self.settings).push((token.into(), moderator_id.into(), room.into(), settings.clone()));
        self.next()
    }
    async fn marker(&self, token: &str, broadcaster_id: &str, description: Option<&str>) -> DoorOutcome {
        self.note("marker");
        lock(&self.markers).push((token.into(), broadcaster_id.into(), description.map(str::to_string)));
        self.next()
    }
    async fn channel_info(&self, token: &str, broadcaster_id: &str, title: Option<&str>, game_id: Option<&str>) -> DoorOutcome {
        self.note("channel_info");
        lock(&self.infos).push((token.into(), broadcaster_id.into(), title.map(str::to_string), game_id.map(str::to_string)));
        self.next()
    }
    async fn search_categories(&self, token: &str, query: &str) -> Result<Vec<Value>, SearchFail> {
        self.note("search_categories");
        lock(&self.searches).push((token.into(), query.into()));
        lock(&self.search_answer).clone().unwrap_or_else(|| Ok(Vec::new()))
    }
}

/// The fake moderation, Pin, Announce and Shoutout doors.
pub struct FakeModDoors {
    state: Arc<Mutex<OutState>>,
    pub log: Mutex<Vec<String>>,
    pub timeouts: Mutex<Vec<(String, String, String, String, u32)>>,
    pub deletes: Mutex<Vec<(String, String, String, String)>>,
    pub pins: Mutex<Vec<(String, String, String, String, Option<u32>)>>,
    pub announces: Mutex<Vec<(String, String, String, String, String)>>,
    pub shoutouts: Mutex<Vec<(String, String, String, String)>>,
    announce_at: Mutex<Vec<tokio::time::Instant>>,
    /// Parks `timeout` while closed (a test holds twelve jobs at the door).
    gate: Gate,
    pub answers: Mutex<VecDeque<DoorOutcome>>,
}

impl FakeModDoors {
    fn new(state: Arc<Mutex<OutState>>) -> FakeModDoors {
        FakeModDoors {
            state,
            log: Mutex::new(Vec::new()),
            timeouts: Mutex::new(Vec::new()),
            deletes: Mutex::new(Vec::new()),
            pins: Mutex::new(Vec::new()),
            announces: Mutex::new(Vec::new()),
            shoutouts: Mutex::new(Vec::new()),
            announce_at: Mutex::new(Vec::new()),
            gate: Gate::open(),
            answers: Mutex::new(VecDeque::new()),
        }
    }

    fn note(&self, name: &str) {
        lock(&self.state).calls.push(format!("mod:{name}"));
        lock(&self.log).push(name.to_string());
    }

    fn next(&self) -> DoorOutcome {
        lock(&self.answers).pop_front().unwrap_or(DoorOutcome::Sent)
    }

    /// When each Announce reached the door (the runtime's clock, virtual under `start_paused`).
    pub fn announce_times(&self) -> Vec<tokio::time::Instant> {
        lock(&self.announce_at).clone()
    }

    /// Park every `timeout` at the door until `release_timeouts`.
    pub fn park_timeouts(&self) {
        self.gate.close();
    }

    pub fn release_timeouts(&self) {
        self.gate.release();
    }
}

#[async_trait]
impl ModDoors for FakeModDoors {
    async fn timeout(&self, token: &str, moderator_id: &str, room: &str, target: &str, seconds: u32, _reason: Option<&str>) -> DoorOutcome {
        self.note("timeout");
        self.gate.pass().await;
        lock(&self.timeouts).push((token.into(), moderator_id.into(), room.into(), target.into(), seconds));
        self.next()
    }
    async fn delete(&self, token: &str, moderator_id: &str, room: &str, message_id: &str) -> DoorOutcome {
        self.note("delete");
        lock(&self.deletes).push((token.into(), moderator_id.into(), room.into(), message_id.into()));
        self.next()
    }
    async fn pin(&self, token: &str, moderator_id: &str, room: &str, message_id: &str, seconds: Option<u32>) -> DoorOutcome {
        self.note("pin");
        lock(&self.pins).push((token.into(), moderator_id.into(), room.into(), message_id.into(), seconds));
        self.next()
    }
    async fn announce(&self, token: &str, moderator_id: &str, room: &str, text: &str, color: &str) -> DoorOutcome {
        self.note("announce");
        lock(&self.announce_at).push(tokio::time::Instant::now());
        lock(&self.announces).push((token.into(), moderator_id.into(), room.into(), text.into(), color.into()));
        self.next()
    }
    async fn shoutout(&self, token: &str, moderator_id: &str, room: &str, target: &str) -> DoorOutcome {
        self.note("shoutout");
        lock(&self.shoutouts).push((token.into(), moderator_id.into(), room.into(), target.into()));
        self.next()
    }
}

/// The fake whisper door.
pub struct FakeWhisperDoor {
    state: Arc<Mutex<OutState>>,
    /// The user ids `knows` answers yes to.
    pub known: Mutex<BTreeSet<String>>,
    pub sent: Mutex<Vec<(String, WhisperTo, String)>>,
    /// What the next sends end as; Sent when unset.
    pub outcome: Mutex<Option<(Result<(), String>, WhisperOutcome)>>,
    /// The whole whisper list is read (`history_complete`); false by default.
    pub history: Mutex<bool>,
}

impl FakeWhisperDoor {
    fn new(state: Arc<Mutex<OutState>>) -> FakeWhisperDoor {
        FakeWhisperDoor {
            state,
            known: Mutex::new(BTreeSet::new()),
            sent: Mutex::new(Vec::new()),
            outcome: Mutex::new(None),
            history: Mutex::new(false),
        }
    }
}

#[async_trait]
impl WhisperDoor for FakeWhisperDoor {
    async fn knows(&self, _owner_id: &str, user_id: &str) -> bool {
        lock(&self.known).contains(user_id)
    }
    async fn send(&self, send_as: &str, to: &WhisperTo, text: &str) -> (Result<(), String>, WhisperOutcome) {
        lock(&self.state).calls.push("whisper:send".into());
        lock(&self.sent).push((send_as.into(), to.clone(), text.into()));
        lock(&self.outcome).clone().unwrap_or((Ok(()), WhisperOutcome::Sent))
    }
    async fn history_complete(&self, _owner_id: &str) -> bool {
        *lock(&self.history)
    }
}

/// A scripted HTTP answer for the next claim or release.
#[derive(Clone, Debug)]
pub enum FakeHttp {
    Network,
    Status(u16, String),
}

/// What the fake chat send answers.
#[derive(Clone, Debug, PartialEq)]
pub enum SendResult {
    Sent,
    Refused(String),
    Unknown,
}

/// One send the fake took.
#[derive(Clone, Debug)]
pub struct SentMsg {
    pub text: String,
    pub token: String,
    pub login: String,
    pub send_as: String,
    /// How far after its booked slot it went out.
    pub late_by: Duration,
}

const FETCH_REFUSED: &str = "Twitch refused the token refresh";

/// One message the fake Kick was asked to send.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KickSent {
    /// The Kick account id the rule sends as (what the door checks).
    pub expected: String,
    pub slug: String,
    pub text: String,
    pub reply_to: Option<String>,
}

#[derive(Default)]
struct KickFake {
    /// The connected Kick account's id.
    account: Option<String>,
    /// Once the first claim has been made, the account reads as this one
    /// (the outer `Some` says it is scripted; the inner is the new account).
    switch_after_claim: Option<Option<String>>,
    /// Who the chat rules say you are, by provider: (chat id, login).
    identities: HashMap<String, (String, String)>,
    sends: Vec<KickSent>,
    mods: Vec<crate::services::providers::kick_door::KickModeration>,
    result: Option<crate::services::providers::kick_door::KickOutcome>,
    delay: Duration,
}

#[derive(Default)]
struct OutState {
    /// What an account's fetch answers, in order; the last one repeats.
    scripts: HashMap<String, VecDeque<String>>,
    last_token: HashMap<String, String>,
    /// An account whose fetch fails, and how.
    fetch_fail: HashMap<String, Fetch>,
    /// An account whose fetch is refused after this many calls.
    fail_after: HashMap<String, usize>,
    fetch_calls: HashMap<String, usize>,
    fetch_total: usize,
    /// The nth fetch of the test (1-based) takes this long.
    slow_fetch: Option<(usize, Duration)>,
    verify_calls: usize,
    validations: HashMap<String, Validation>,
    validate_calls: HashMap<String, usize>,
    validate_never: bool,
    own_accounts: Option<Vec<(String, String)>>,
    profile: Option<String>,
    /// The registry's primary; unset means this test's main account.
    primary: Option<Option<String>>,
    calls: Vec<String>,
    sends: Vec<SentMsg>,
    send_attempts: usize,
    send_result: Option<SendResult>,
    send_delay: Duration,
    claim_answers: VecDeque<FakeHttp>,
    release_answers: VecDeque<FakeHttp>,
    claim_delay: Duration,
    claim_calls: usize,
    finished_claims: usize,
    bearers: Vec<String>,
    release_bearers: Vec<String>,
    bodies: Vec<Value>,
    releases: Vec<String>,
    drop_releases: bool,
    room_delay: Duration,
    kick: KickFake,
}

/// The login the fake gives a test account.
fn login_for(id: &str, main: &str, alt: &str) -> String {
    if id == main {
        main_login(main)
    } else if id == alt {
        "alt".into()
    } else {
        format!("user{id}")
    }
}

/// The real verifier over the fake fetch and validate: its cache, failure
/// memory and budget run in every engine test.
fn fake_io(
    state: Arc<Mutex<OutState>>,
    main: String,
    alt: String,
    clock: Clock,
    generation: Arc<AtomicU64>,
) -> VerifyIo {
    let fetch_state = state.clone();
    let validate_state = state;
    VerifyIo {
        fetch: Box::new(move |id: String| -> BoxFuture<Fetch> {
            let state = fetch_state.clone();
            Box::pin(async move {
                let (answer, slow) = {
                    let mut guard = lock(&state);
                    let s = &mut *guard;
                    s.calls.push("verify".into());
                    s.fetch_total += 1;
                    let n = {
                        let calls = s.fetch_calls.entry(id.clone()).or_insert(0);
                        *calls += 1;
                        *calls
                    };
                    let answer = if s.fail_after.get(&id).is_some_and(|limit| n > *limit) {
                        Fetch::Refused(FETCH_REFUSED.into())
                    } else if let Some(fail) = s.fetch_fail.get(&id) {
                        fail.clone()
                    } else if let Some(next) = s.scripts.get_mut(&id).and_then(VecDeque::pop_front) {
                        s.last_token.insert(id.clone(), next.clone());
                        Fetch::Token(next)
                    } else {
                        Fetch::Token(s.last_token.get(&id).cloned().unwrap_or_else(|| token_of(&id)))
                    };
                    (answer, s.slow_fetch.filter(|(n, _)| s.fetch_total == *n).map(|(_, d)| d))
                };
                if let Some(d) = slow {
                    tokio::time::sleep(d).await;
                }
                answer
            })
        }),
        validate: Box::new(move |token: String| -> BoxFuture<Validation> {
            let state = validate_state.clone();
            let (main, alt) = (main.clone(), alt.clone());
            Box::pin(async move {
                let (answer, never) = {
                    let mut s = lock(&state);
                    *s.validate_calls.entry(token.clone()).or_insert(0) += 1;
                    let answer = s.validations.get(&token).cloned().unwrap_or_else(|| match token.strip_prefix("tok-") {
                        Some(id) => Validation::Valid(Identity {
                            user_id: id.into(),
                            login: login_for(id, &main, &alt),
                            expires_in_s: 3600,
                        }),
                        None => Validation::Rejected,
                    });
                    (answer, s.validate_never)
                };
                if never {
                    std::future::pending::<()>().await;
                }
                answer
            })
        }),
        spawn: Box::new(|task: BoxFuture<()>| {
            tokio::spawn(task);
        }),
        token_hash: Box::new(|token: &str| format!("hash-{token}")),
        now_ms: Box::new(move || clock()),
        generation: Box::new(move || generation.load(Ordering::SeqCst)),
    }
}

pub struct FakeOutward {
    server: Arc<FakeServer>,
    /// The test's ids, captured when the fake was built: the engine calls the
    /// fake from runtime threads, where `main_id()` would name other ids.
    main: String,
    alt: String,
    state: Arc<Mutex<OutState>>,
    generation: Arc<AtomicU64>,
    verifier: TokenVerifier,
    pub command_doors: Arc<FakeCommandDoors>,
    pub mod_doors: Arc<FakeModDoors>,
    pub whisper_door: Arc<FakeWhisperDoor>,
    /// Tokens `drop_token` was told to forget, and the accounts `record_send_401` named.
    dropped: Mutex<Vec<VerifiedToken>>,
    send_401s: Mutex<Vec<String>>,
    /// `room_id("me")` answers the main account's id (a Timer in your own channel).
    own_room_is_main: AtomicBool,
    /// The next claim loses on this reason (and `retry_after_ms`).
    loss: Mutex<Option<(String, Option<i64>)>>,
    /// An account's second `verified_token` of the test answers this instead.
    second_verify: Mutex<HashMap<String, FakeToken>>,
    /// The first verify after a claim call (step 9's) answers this instead.
    after_claim: Mutex<HashMap<String, FakeToken>>,
    /// From the first claim call on, the registry's primary reads as this (step 9's read).
    primary_after_claim: Mutex<Option<Option<String>>>,
    verifies: Mutex<HashMap<String, usize>>,
    send_gate: Gate,
    claim_gate: Gate,
    release_gate: Gate,
    room_gate: Gate,
    send_started: Started,
    claim_started: Started,
    release_started: Started,
    room_started: Started,
    kick_pace: Mutex<crate::services::provider_pace::Pace>,
}

impl FakeOutward {
    /// A fake door whose verifier reads `clock` (the test's).
    pub fn new(server: Arc<FakeServer>, clock: Clock) -> FakeOutward {
        let (main, alt) = (main_id(), alt_id());
        let state = Arc::new(Mutex::new(OutState::default()));
        let generation = Arc::new(AtomicU64::new(1));
        let io = fake_io(state.clone(), main.clone(), alt.clone(), clock, generation.clone());
        let command_doors = Arc::new(FakeCommandDoors::new(state.clone()));
        let mod_doors = Arc::new(FakeModDoors::new(state.clone()));
        let whisper_door = Arc::new(FakeWhisperDoor::new(state.clone()));
        FakeOutward {
            server,
            main,
            alt,
            state,
            generation,
            verifier: TokenVerifier::new(io),
            command_doors,
            mod_doors,
            whisper_door,
            dropped: Mutex::new(Vec::new()),
            send_401s: Mutex::new(Vec::new()),
            own_room_is_main: AtomicBool::new(false),
            loss: Mutex::new(None),
            second_verify: Mutex::new(HashMap::new()),
            after_claim: Mutex::new(HashMap::new()),
            primary_after_claim: Mutex::new(None),
            verifies: Mutex::new(HashMap::new()),
            send_gate: Gate::open(),
            claim_gate: Gate::open(),
            release_gate: Gate::open(),
            room_gate: Gate::open(),
            send_started: Started::default(),
            claim_started: Started::default(),
            release_started: Started::default(),
            room_started: Started::default(),
            kick_pace: Mutex::new(crate::services::provider_pace::Pace::default()),
        }
    }

    /// Shortcut: how `id`'s token answers. A missing token and a refused
    /// refresh are both a refused fetch, as the real door classifies them.
    pub fn set_token(&self, id: &str, how: FakeToken) {
        let token = token_of(id);
        let mut s = lock(&self.state);
        match how {
            FakeToken::Good => {
                s.validations.remove(&token);
                s.fetch_fail.remove(id);
                s.scripts.remove(id);
                s.last_token.remove(id);
            }
            FakeToken::NoToken => {
                s.fetch_fail.insert(id.into(), Fetch::Refused("no usable Twitch token is stored".into()));
            }
            FakeToken::Refused => {
                s.fetch_fail.insert(id.into(), Fetch::Refused(FETCH_REFUSED.into()));
            }
            FakeToken::WrongAccount(actual) => {
                s.validations.insert(token, Validation::Valid(Identity { user_id: actual, login: "someone".into(), expires_in_s: 3600 }));
            }
            FakeToken::Unreachable => {
                s.validations.insert(token, Validation::Unreachable("Twitch did not answer".into()));
            }
        }
    }
    /// `id`'s fetch answers these tokens in order; the last one repeats.
    pub fn script_tokens(&self, id: &str, tokens: Vec<&str>) {
        let mut s = lock(&self.state);
        s.scripts.insert(id.into(), tokens.iter().map(|t| t.to_string()).collect());
        s.last_token.remove(id);
    }
    /// `token` validates as `user_id`.
    pub fn validates(&self, token: &str, user_id: &str, login: &str) {
        lock(&self.state).validations.insert(
            token.into(),
            Validation::Valid(Identity { user_id: user_id.into(), login: login.into(), expires_in_s: 3600 }),
        );
    }
    pub fn validate_answer(&self, token: &str, answer: Validation) {
        lock(&self.state).validations.insert(token.into(), answer);
    }
    /// Validation never answers (id.twitch.tv hangs).
    pub fn validate_never_answers(&self) {
        lock(&self.state).validate_never = true;
    }
    pub fn validate_calls(&self, token: &str) -> usize {
        lock(&self.state).validate_calls.get(token).copied().unwrap_or(0)
    }
    /// Token fetches for `id`.
    pub fn fetch_calls(&self, id: &str) -> usize {
        lock(&self.state).fetch_calls.get(id).copied().unwrap_or(0)
    }
    /// `verified_token` calls, answered or not.
    pub fn verify_calls(&self) -> usize {
        lock(&self.state).verify_calls
    }
    /// `id`'s fetch is refused after `n` calls.
    pub fn fail_token_after(&self, id: &str, n: usize) {
        lock(&self.state).fail_after.insert(id.into(), n);
    }
    /// The second fetch of the test (the first send's step 9) takes `d`.
    pub fn slow_verify_after_first(&self, d: Duration) {
        self.slow_verify_at(2, d);
    }
    /// The `n`th token fetch of the test (1-based) takes `d`.
    pub fn slow_verify_at(&self, n: usize, d: Duration) {
        lock(&self.state).slow_fetch = Some((n, d));
    }
    pub fn set_own_accounts(&self, list: Vec<(String, String)>) {
        lock(&self.state).own_accounts = Some(list);
    }
    pub fn set_profile(&self, id: Option<&str>) {
        lock(&self.state).profile = id.map(str::to_string);
    }
    pub fn set_primary(&self, id: Option<&str>) {
        lock(&self.state).primary = Some(id.map(str::to_string));
    }
    pub fn bump_generation(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
    }
    pub fn calls(&self) -> Vec<String> {
        lock(&self.state).calls.clone()
    }
    pub fn sends(&self) -> Vec<SentMsg> {
        lock(&self.state).sends.clone()
    }
    pub fn send_attempts(&self) -> usize {
        lock(&self.state).send_attempts
    }
    pub fn lose_sends_with(&self, result: SendResult) {
        lock(&self.state).send_result = Some(result);
    }
    pub fn delay_sends(&self, d: Duration) {
        lock(&self.state).send_delay = d;
    }
    pub fn park_sends(&self) {
        self.send_gate.close();
    }
    pub fn release_sends(&self) {
        self.send_gate.release();
    }
    pub async fn wait_for_send_started(&self) {
        self.send_started.reach(1).await;
    }
    pub fn claim_calls(&self) -> usize {
        lock(&self.state).claim_calls
    }
    pub fn finished_claims(&self) -> usize {
        lock(&self.state).finished_claims
    }
    pub fn claim_bearers(&self) -> Vec<String> {
        lock(&self.state).bearers.clone()
    }
    pub fn release_bearers(&self) -> Vec<String> {
        lock(&self.state).release_bearers.clone()
    }
    pub fn bodies(&self) -> Vec<Value> {
        lock(&self.state).bodies.clone()
    }
    pub fn releases(&self) -> Vec<String> {
        lock(&self.state).releases.clone()
    }
    pub fn claim_answers(&self, answers: Vec<FakeHttp>) {
        lock(&self.state).claim_answers = answers.into();
    }
    pub fn release_answers(&self, answers: Vec<FakeHttp>) {
        lock(&self.state).release_answers = answers.into();
    }
    pub fn delay_claims(&self, d: Duration) {
        lock(&self.state).claim_delay = d;
    }
    pub fn park_claims(&self) {
        self.claim_gate.close();
    }
    pub fn release_claims(&self) {
        self.claim_gate.release();
    }
    pub async fn wait_for_claim_started(&self) {
        self.claim_started.reach(1).await;
    }
    pub async fn wait_for_claims_started(&self, n: usize) {
        self.claim_started.reach(n).await;
    }
    pub fn park_releases(&self) {
        self.release_gate.close();
    }
    pub fn release_releases(&self) {
        self.release_gate.release();
    }
    pub async fn wait_for_release_started(&self) {
        self.release_started.reach(1).await;
    }
    /// Releases never arrive (a network error every time).
    pub fn drop_releases(&self) {
        lock(&self.state).drop_releases = true;
    }
    pub fn slow_rooms(&self, d: Duration) {
        lock(&self.state).room_delay = d;
    }
    pub fn park_rooms(&self) {
        self.room_gate.close();
    }
    pub fn release_rooms(&self) {
        self.room_gate.release();
    }
    pub async fn wait_for_room_started(&self) {
        self.room_started.reach(1).await;
    }

    /// The account a token validates as, for the claim server's scope.
    fn owner_of(&self, token: &str) -> String {
        match lock(&self.state).validations.get(token) {
            Some(Validation::Valid(identity)) => identity.user_id.clone(),
            _ => token.strip_prefix("tok-").unwrap_or(token).to_string(),
        }
    }
}

impl FakeOutward {
    /// Kick is connected as `id` (`login`): the account the door reads, and
    /// who the chat rules say you are there.
    pub fn connect_kick(&self, id: &str, login: &str) {
        let mut s = lock(&self.state);
        s.kick.account = Some(id.into());
        s.kick.switch_after_claim = None;
        s.kick.identities.insert("kick".into(), (id.into(), login.into()));
    }

    /// The Kick account is gone (its chat identity stays: the chat rules still know the name).
    pub fn disconnect_kick(&self) {
        lock(&self.state).kick.account = None;
    }

    /// Who the chat rules say you are on `provider` (YouTube, TikTok tests).
    pub fn set_platform_identity(&self, provider: &str, id: &str, login: &str) {
        lock(&self.state).kick.identities.insert(provider.into(), (id.into(), login.into()));
    }

    /// From the first claim on, the connected account reads as `other`.
    pub fn switch_kick_after_claim(&self, other: Option<&str>) {
        lock(&self.state).kick.switch_after_claim = Some(other.map(str::to_string));
    }

    /// The next Kick sends end like this (the last repeats).
    pub fn script_kick(&self, outcome: crate::services::providers::kick_door::KickOutcome) {
        lock(&self.state).kick.result = Some(outcome);
    }

    pub fn delay_kick(&self, d: Duration) {
        lock(&self.state).kick.delay = d;
    }

    /// Every Kick send attempt that reached the door.
    pub fn kick_sends(&self) -> Vec<KickSent> {
        lock(&self.state).kick.sends.clone()
    }

    pub fn kick_mods(&self) -> Vec<crate::services::providers::kick_door::KickModeration> {
        lock(&self.state).kick.mods.clone()
    }

    fn kick_current(s: &OutState) -> Option<String> {
        match &s.kick.switch_after_claim {
            Some(other) if s.claim_calls > 0 => other.clone(),
            _ => s.kick.account.clone(),
        }
    }
}

#[async_trait]
impl Outward for FakeOutward {
    async fn verified_token(&self, account_id: &str, budget: Duration) -> Result<VerifiedToken, TokenRefusal> {
        lock(&self.state).verify_calls += 1;
        let nth = {
            let mut seen = lock(&self.verifies);
            let n = seen.entry(account_id.to_string()).or_insert(0);
            *n += 1;
            *n
        };
        // Step 9's re-verify: the first verify of `id` once a claim call has gone out.
        let claimed = lock(&self.state).claim_calls > 0;
        if claimed {
            if let Some(how) = lock(&self.after_claim).remove(account_id) {
                return Err(refusal_of(account_id, how));
            }
        }
        if nth == 2 {
            if let Some(how) = lock(&self.second_verify).remove(account_id) {
                return Err(refusal_of(account_id, how));
            }
        }
        self.verifier.verify(account_id, budget).await
    }

    fn verifier(&self) -> &TokenVerifier {
        &self.verifier
    }

    fn own_accounts(&self) -> Vec<(String, String)> {
        lock(&self.state)
            .own_accounts
            .clone()
            .unwrap_or_else(|| vec![(self.main.clone(), main_login(&self.main)), (self.alt.clone(), "alt".into())])
    }

    fn profile_user_id(&self) -> Option<String> {
        lock(&self.state).profile.clone()
    }

    fn primary_id(&self) -> Option<String> {
        let state = lock(&self.state);
        if state.claim_calls > 0 {
            if let Some(after) = lock(&self.primary_after_claim).clone() {
                return after;
            }
        }
        state.primary.clone().unwrap_or_else(|| Some(self.main.clone()))
    }

    fn account_generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    fn command_doors(&self) -> &dyn CommandDoors {
        &*self.command_doors
    }

    fn mod_doors(&self) -> &dyn ModDoors {
        &*self.mod_doors
    }

    fn whisper_door(&self) -> &dyn WhisperDoor {
        &*self.whisper_door
    }

    fn drop_token(&self, token: &VerifiedToken) {
        lock(&self.dropped).push(token.clone());
        self.verifier.drop_token_of(&token.token);
    }

    fn record_send_401(&self, account_id: &str) {
        lock(&self.send_401s).push(account_id.to_string());
        self.verifier.record_send_401(account_id);
    }

    async fn room_id(&self, login: &str) -> Result<String, String> {
        let delay = {
            let mut s = lock(&self.state);
            s.calls.push("room_id".into());
            s.room_delay
        };
        self.room_started.hit();
        self.room_gate.pass().await;
        tokio::time::sleep(delay).await;
        if login == main_login(&self.main) && self.own_room_is_main.load(Ordering::SeqCst) {
            return Ok(self.main.clone());
        }
        Ok("4242".into())
    }

    async fn chat_send(&self, ticket: &mut Ticket, send: &ChatSend) -> SendOutcome {
        let late_by = Instant::now().saturating_duration_since(ticket.at());
        let (result, delay) = {
            let mut s = lock(&self.state);
            s.calls.push("send".into());
            s.send_attempts += 1;
            (s.send_result.clone().unwrap_or(SendResult::Sent), s.send_delay)
        };
        chat_send_limiter::mark_sending(ticket);
        self.send_started.hit();
        self.send_gate.pass().await;
        tokio::time::sleep(delay).await;
        match result {
            SendResult::Refused(reason) => {
                chat_send_limiter::cancel_booking(ticket);
                SendOutcome::Refused(reason)
            }
            other => {
                chat_send_limiter::record_ticket(ticket);
                if other == SendResult::Sent {
                    lock(&self.state).sends.push(SentMsg {
                        text: send.text.clone(),
                        token: send.token.clone(),
                        login: send.login.clone(),
                        send_as: send.send_as.clone(),
                        late_by,
                    });
                    SendOutcome::Sent { message_id: Some("sent-1".into()) }
                } else {
                    SendOutcome::Unknown
                }
            }
        }
    }

    fn kick_account_id(&self) -> Option<String> {
        let mut s = lock(&self.state);
        s.calls.push("kick_id".into());
        FakeOutward::kick_current(&s)
    }

    fn platform_identity(&self, provider: &str) -> Option<(String, String)> {
        lock(&self.state).kick.identities.get(provider).cloned()
    }

    async fn kick_send(&self, expected_account: &str, slug: &str, text: &str, reply_to: Option<&str>) -> crate::services::providers::kick_door::KickOutcome {
        use crate::services::providers::kick_door::{KickOutcome, OTHER_ACCOUNT_WORDS};
        let (result, delay) = {
            let mut s = lock(&self.state);
            s.calls.push("kick_send".into());
            // The real door checks the account again before it sends.
            if FakeOutward::kick_current(&s).as_deref() != Some(expected_account) {
                return KickOutcome::Refused { reason: OTHER_ACCOUNT_WORDS.into(), status: None };
            }
            s.kick.sends.push(KickSent {
                expected: expected_account.into(),
                slug: slug.into(),
                text: text.into(),
                reply_to: reply_to.map(str::to_string),
            });
            (s.kick.result.clone().unwrap_or(KickOutcome::Sent { message_id: Some("kick-1".into()) }), s.kick.delay)
        };
        tokio::time::sleep(delay).await;
        result
    }

    async fn kick_moderate(&self, _expected_account: &str, op: &crate::services::providers::kick_door::KickModeration) -> crate::services::providers::kick_door::KickOutcome {
        let mut s = lock(&self.state);
        s.calls.push("kick_moderate".into());
        s.kick.mods.push(op.clone());
        s.kick.result.clone().unwrap_or(crate::services::providers::kick_door::KickOutcome::Sent { message_id: None })
    }

    async fn kick_room_id(&self, _slug: &str) -> Result<String, String> {
        Ok("5002".into())
    }

    fn kick_pace(&self) -> &Mutex<crate::services::provider_pace::Pace> {
        &self.kick_pace
    }

    async fn claim_post(&self, bearer: &str, _owner_id: &str, body: &Value) -> Result<ApiResponse, String> {
        // The site scopes a call to the account its bearer validates as.
        let account = self.owner_of(bearer);
        let release = body["action"] == "release";
        let (scripted, delay, dropped) = {
            let mut s = lock(&self.state);
            s.bodies.push(body.clone());
            if release {
                s.calls.push("release".into());
                s.releases.push(body["call_id"].as_str().unwrap_or_default().to_string());
                s.release_bearers.push(bearer.to_string());
                (s.release_answers.pop_front(), Duration::ZERO, s.drop_releases)
            } else {
                s.calls.push("claim".into());
                s.claim_calls += 1;
                s.bearers.push(bearer.to_string());
                (s.claim_answers.pop_front(), s.claim_delay, false)
            }
        };
        if release {
            self.release_started.hit();
            self.release_gate.pass().await;
        } else {
            self.claim_started.hit();
            self.claim_gate.pass().await;
            tokio::time::sleep(delay).await;
        }
        let out = if dropped {
            Err("network: dropped".to_string())
        } else {
            match scripted {
                Some(FakeHttp::Network) => Err("network: refused".to_string()),
                Some(FakeHttp::Status(status, body)) => Ok(ApiResponse { status, ok: (200..300).contains(&status), body }),
                None => {
                    let mut answer = self.server.handle(&account, body);
                    if !release {
                        if let Some((reason, retry)) = lock(&self.loss).take() {
                            lose_claim(&mut answer, &reason, retry);
                        }
                    }
                    Ok(ApiResponse { status: 200, ok: true, body: answer.to_string() })
                }
            }
        };
        if !release {
            lock(&self.state).finished_claims += 1;
        }
        out
    }
}

// ---- the fake in-app seam ------------------------------------------------------------------

#[derive(Default)]
struct ChatState {
    holders: HashMap<String, BTreeSet<String>>,
    connected: bool,
    cycling: bool,
    fail_starts: bool,
    starts: usize,
    last_start_label: Option<String>,
    joins: HashMap<String, usize>,
    leaves: HashMap<String, usize>,
    watched: BTreeSet<String>,
    live: HashMap<String, Option<bool>>,
    watch: Option<String>,
    /// What USERSTATE said an account is in a channel.
    roles: HashMap<(String, String), crate::services::chat_send_limiter::Role>,
    /// The whisper picker's people.
    people: Vec<WhisperPerson>,
}

pub struct FakeLocal {
    settings: Mutex<(Automations, AutomationPrefs)>,
    chat: Mutex<ChatState>,
    notes: Mutex<Vec<(String, String)>>,
    notifications: Mutex<Vec<NotifyPayload>>,
    system: Mutex<Vec<String>>,
    targets: Mutex<Vec<String>>,
    streamer: AtomicBool,
    turned_off: AtomicBool,
    statuses: AtomicUsize,
    change_gate: Gate,
    changes_held: Arc<AtomicUsize>,
    /// P3: the bodies of the system notices (their titles are in `system`).
    system_bodies: Mutex<Vec<String>>,
    speaks: Mutex<Vec<SpeakPayload>>,
    own: Mutex<OwnFake>,
    twitch_offset_ms: AtomicI64,
}

/// What the fake whisper session was asked for, and what it says it holds.
#[derive(Default)]
struct OwnFake {
    holds: Vec<(String, BTreeSet<TopicKind>)>,
    releases: usize,
    state: OwnState,
}

/// The fake account-change guard: counted while held.
struct HeldChange(Arc<AtomicUsize>);

impl Drop for HeldChange {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl FakeLocal {
    pub(crate) fn new() -> FakeLocal {
        FakeLocal {
            settings: Mutex::new((Automations::default(), AutomationPrefs::default())),
            chat: Mutex::new(ChatState { connected: true, ..ChatState::default() }),
            notes: Mutex::new(Vec::new()),
            notifications: Mutex::new(Vec::new()),
            system: Mutex::new(Vec::new()),
            targets: Mutex::new(vec!["main".into()]),
            streamer: AtomicBool::new(false),
            turned_off: AtomicBool::new(false),
            statuses: AtomicUsize::new(0),
            change_gate: Gate::open(),
            changes_held: Arc::new(AtomicUsize::new(0)),
            system_bodies: Mutex::new(Vec::new()),
            speaks: Mutex::new(Vec::new()),
            own: Mutex::new(OwnFake::default()),
            twitch_offset_ms: AtomicI64::new(0),
        }
    }

    pub fn hold(&self, label: &str, room: &str) {
        lock(&self.chat).holders.entry(room.into()).or_default().insert(label.into());
    }
    pub fn holders_now(&self, room: &str) -> Vec<String> {
        lock(&self.chat).holders.get(room).map(|h| h.iter().cloned().collect()).unwrap_or_default()
    }
    /// Rooms the engine's label holds.
    pub fn joined(&self) -> BTreeSet<String> {
        lock(&self.chat)
            .holders
            .iter()
            .filter(|(_, h)| h.contains(super::AUTOMATIONS_LABEL))
            .map(|(room, _)| room.clone())
            .collect()
    }
    pub fn join_calls(&self, room: &str) -> usize {
        lock(&self.chat).joins.get(room).copied().unwrap_or(0)
    }
    pub fn leave_calls(&self, room: &str) -> usize {
        lock(&self.chat).leaves.get(room).copied().unwrap_or(0)
    }
    /// What IrcService does on a fresh start or stop: every claim goes.
    pub fn clear_claims(&self) {
        lock(&self.chat).holders.clear();
    }
    pub fn set_connected(&self, on: bool) {
        lock(&self.chat).connected = on;
    }
    pub fn set_cycling(&self, on: bool) {
        lock(&self.chat).cycling = on;
    }
    pub fn fail_starts(&self, on: bool) {
        lock(&self.chat).fail_starts = on;
    }
    pub fn start_calls(&self) -> usize {
        lock(&self.chat).starts
    }
    pub fn last_start_label(&self) -> Option<String> {
        lock(&self.chat).last_start_label.clone()
    }
    pub fn watched(&self) -> BTreeSet<String> {
        lock(&self.chat).watched.clone()
    }
    pub fn set_live(&self, login: &str, live: Option<bool>) {
        lock(&self.chat).live.insert(login.into(), live);
    }
    pub fn set_watch(&self, login: Option<&str>) {
        lock(&self.chat).watch = login.map(str::to_string);
    }
    pub fn notes(&self) -> Vec<(String, String)> {
        lock(&self.notes).clone()
    }
    pub fn notifications(&self) -> Vec<NotifyPayload> {
        lock(&self.notifications).clone()
    }
    pub fn system_notices(&self) -> Vec<String> {
        lock(&self.system).clone()
    }
    pub fn set_targets(&self, targets: Vec<String>) {
        *lock(&self.targets) = targets;
    }
    pub fn set_streamer(&self, on: bool) {
        self.streamer.store(on, Ordering::SeqCst);
    }
    /// The live-notification switch: this seam never reads it, which is the point.
    pub fn set_live_notifications_off(&self) {}
    pub fn turned_off(&self) -> bool {
        self.turned_off.load(Ordering::SeqCst)
    }
    /// A linked-account refresh is running: the account change waits for it.
    pub fn park_account_change(&self) {
        self.change_gate.close();
    }
    pub fn release_account_change(&self) {
        self.change_gate.release();
    }
    pub fn account_changes_held(&self) -> usize {
        self.changes_held.load(Ordering::SeqCst)
    }
    pub fn speaks(&self) -> Vec<SpeakPayload> {
        lock(&self.speaks).clone()
    }
    pub fn system_bodies(&self) -> Vec<String> {
        lock(&self.system_bodies).clone()
    }
    /// Twitch's clock runs this far ahead of the test's wall clock.
    pub fn set_twitch_offset_ms(&self, ms: i64) {
        self.twitch_offset_ms.store(ms, Ordering::SeqCst);
    }
    /// Every `hold_own_topics` call, in order.
    pub fn own_holds(&self) -> Vec<(String, BTreeSet<TopicKind>)> {
        lock(&self.own).holds.clone()
    }
    pub fn own_releases(&self) -> usize {
        lock(&self.own).releases
    }
    /// What the fake whisper session holds and refused.
    pub fn set_own_state(&self, session_up: bool, held: &[TopicKind], refused: &[(TopicKind, TopicRefusal)]) {
        let mut own = lock(&self.own);
        own.state = OwnState { session_up, held: held.iter().copied().collect(), refused: refused.iter().copied().collect() };
    }
}

#[async_trait]
impl Local for FakeLocal {
    fn settings(&self) -> (Automations, AutomationPrefs) {
        lock(&self.settings).clone()
    }
    fn own_role(&self, as_login: &str, channel: &str) -> Option<crate::services::chat_send_limiter::Role> {
        lock(&self.chat).roles.get(&(as_login.to_string(), channel.to_string())).copied()
    }
    fn whisper_people(&self, _owner_id: &str) -> Vec<WhisperPerson> {
        lock(&self.chat).people.clone()
    }
    async fn chats_held_by(&self, label: &str) -> Vec<String> {
        let mut rooms: Vec<String> =
            lock(&self.chat).holders.iter().filter(|(_, h)| h.contains(label)).map(|(r, _)| r.clone()).collect();
        rooms.sort();
        rooms
    }
    async fn holders(&self, channel: &str) -> Vec<String> {
        self.holders_now(channel)
    }
    async fn chat_connected(&self) -> bool {
        lock(&self.chat).connected
    }
    fn recovery_cycling(&self) -> bool {
        lock(&self.chat).cycling
    }
    async fn chat_start(&self, first: &str) -> Result<(), String> {
        let mut c = lock(&self.chat);
        c.starts += 1;
        c.last_start_label = Some(super::AUTOMATIONS_LABEL.into());
        if c.fail_starts {
            return Err("IRC connection not established".into());
        }
        c.connected = true;
        c.holders.entry(first.into()).or_default().insert(super::AUTOMATIONS_LABEL.into());
        Ok(())
    }
    async fn chat_join(&self, channel: &str) -> Result<(), String> {
        let mut c = lock(&self.chat);
        *c.joins.entry(channel.into()).or_insert(0) += 1;
        if !c.connected {
            return Err("IRC connection not established".into());
        }
        c.holders.entry(channel.into()).or_default().insert(super::AUTOMATIONS_LABEL.into());
        Ok(())
    }
    async fn chat_leave(&self, channel: &str) {
        let mut c = lock(&self.chat);
        *c.leaves.entry(channel.into()).or_insert(0) += 1;
        if let Some(h) = c.holders.get_mut(channel) {
            h.remove(super::AUTOMATIONS_LABEL);
            if h.is_empty() {
                c.holders.remove(channel);
            }
        }
    }
    async fn provider_join(&self, provider: &str, channel: &str) -> Result<(), String> {
        let key = format!("{provider}:{}", channel.to_lowercase());
        let mut c = lock(&self.chat);
        *c.joins.entry(key.clone()).or_insert(0) += 1;
        c.holders.entry(key).or_default().insert(super::AUTOMATIONS_LABEL.into());
        Ok(())
    }
    async fn provider_leave(&self, provider: &str, channel: &str) {
        let key = format!("{provider}:{}", channel.to_lowercase());
        let mut c = lock(&self.chat);
        *c.leaves.entry(key.clone()).or_insert(0) += 1;
        if let Some(h) = c.holders.get_mut(&key) {
            h.remove(super::AUTOMATIONS_LABEL);
            if h.is_empty() {
                c.holders.remove(&key);
            }
        }
    }
    async fn provider_held_by(&self, provider: &str, label: &str) -> Vec<String> {
        let prefix = format!("{provider}:");
        let mut out: Vec<String> = lock(&self.chat)
            .holders
            .iter()
            .filter(|(key, holders)| key.starts_with(&prefix) && holders.contains(label))
            .map(|(key, _)| key[prefix.len()..].to_string())
            .collect();
        out.sort();
        out
    }
    async fn provider_held_by_other_than(&self, provider: &str, channel: &str, label: &str) -> bool {
        let key = format!("{provider}:{}", channel.to_lowercase());
        lock(&self.chat).holders.get(&key).is_some_and(|holders| holders.iter().any(|l| l != label))
    }
    async fn watch_live(&self, login: &str, _room_id: &str) {
        lock(&self.chat).watched.insert(login.into());
    }
    async fn unwatch_live(&self, login: &str) {
        lock(&self.chat).watched.remove(login);
    }
    async fn is_live(&self, login: &str) -> Option<bool> {
        lock(&self.chat).live.get(login).copied().flatten()
    }
    async fn channel_info(&self, _login: &str) -> Option<ChannelState> {
        None
    }
    fn current_watch(&self) -> Option<String> {
        lock(&self.chat).watch.clone()
    }
    async fn room_notice(&self, login: &str, text: &str) {
        lock(&self.notes).push((login.into(), text.into()));
    }
    fn streamer_mode(&self) -> bool {
        self.streamer.load(Ordering::SeqCst)
    }
    fn notify_targets(&self) -> Vec<String> {
        lock(&self.targets).clone()
    }
    fn main_focused_on_screen(&self) -> bool {
        false
    }
    fn emit_notify(&self, _targets: &[String], payload: &NotifyPayload) {
        lock(&self.notifications).push(payload.clone());
    }
    fn system_notice(&self, request: &NotifyRequest, _id: u64) {
        lock(&self.system).push(request.title.clone());
        lock(&self.system_bodies).push(request.body.clone());
    }
    fn emit_activity(&self, _watchers: &[String], _entries: &[Entry]) {}
    fn emit_status(&self, _snapshot: &StatusSnapshot) {
        self.statuses.fetch_add(1, Ordering::SeqCst);
    }
    async fn probe_link(&self, _url: &str) -> Probe {
        Probe::Status(200)
    }
    async fn refresh_gallery(&self) -> Result<(), String> {
        Ok(())
    }
    async fn turn_off(&self) {
        lock(&self.settings).0.run = RunState::Off;
        self.turned_off.store(true, Ordering::SeqCst);
    }
    async fn hold_own_topics(&self, owner_id: &str, kinds: &BTreeSet<TopicKind>) {
        lock(&self.own).holds.push((owner_id.to_string(), kinds.clone()));
    }
    async fn release_own_topics(&self, _owner_id: &str) {
        lock(&self.own).releases += 1;
    }
    fn own_topic_state(&self) -> OwnState {
        lock(&self.own).state.clone()
    }
    fn twitch_now(&self, wall_ms: i64) -> i64 {
        wall_ms + self.twitch_offset_ms.load(Ordering::SeqCst)
    }
    fn emit_speak(&self, _targets: &[String], payload: &SpeakPayload) {
        lock(&self.speaks).push(payload.clone());
    }
}

#[async_trait]
impl AccountChanges for FakeLocal {
    async fn begin_account_change(&self) -> AccountChange {
        self.change_gate.pass().await;
        self.changes_held.fetch_add(1, Ordering::SeqCst);
        Box::new(HeldChange(self.changes_held.clone()))
    }
}

// ---- P2 helpers ---------------------------------------------------------------------------------

/// Turn a claim answer into a loss on `reason`: the element of the kind
/// `counted` (else the last) loses with `reason` and the others roll back with
/// `rail`, as the claim function answers.
fn lose_claim(answer: &mut Value, reason: &str, retry_after_ms: Option<i64>) {
    let Some(list) = answer["elements"].as_array_mut() else { return };
    if list.is_empty() {
        return;
    }
    let at = list.iter().position(|e| e["kind"] == "counted").unwrap_or(list.len() - 1);
    for (i, e) in list.iter_mut().enumerate() {
        e["won"] = json!(false);
        e["reason"] = json!(if i == at { reason } else { "rail" });
    }
    list[at]["retry_after_ms"] = json!(retry_after_ms);
}

impl FakeOutward {
    /// The next claim loses on `reason` (`hour`, `day`, `gap`, ...).
    pub fn script_claim_loss(&self, reason: &str) {
        *lock(&self.loss) = Some((reason.into(), None));
    }
    /// The same, with the server's `retry_after_ms` (a `day` loss).
    pub fn script_claim_loss_with_retry(&self, reason: &str, retry_after_ms: i64) {
        *lock(&self.loss) = Some((reason.into(), Some(retry_after_ms)));
    }
    /// The second `verified_token` of `id` in the test answers `how` (step 9's).
    pub fn script_second_verify(&self, id: &str, how: FakeToken) {
        lock(&self.second_verify).insert(id.into(), how);
    }
    pub fn slow_claims(&self, d: Duration) {
        self.delay_claims(d);
    }
    /// Tokens `drop_token` was told to forget, in order.
    pub fn dropped_tokens(&self) -> Vec<VerifiedToken> {
        lock(&self.dropped).clone()
    }
    /// Accounts `record_send_401` named, in order.
    pub fn send_401s(&self) -> Vec<String> {
        lock(&self.send_401s).clone()
    }
}

impl FakeLocal {
    /// USERSTATE said `login` is `role` in `channel`.
    pub fn set_role(&self, login: &str, channel: &str, role: crate::services::chat_send_limiter::Role) {
        lock(&self.chat).roles.insert((login.to_string(), channel.to_string()), role);
    }
    /// The whisper picker lists these people.
    pub fn set_people(&self, people: Vec<WhisperPerson>) {
        lock(&self.chat).people = people;
    }
}

impl Kit {
    /// A kit whose Timer rules post in the owner's own channel: the fake room
    /// lookup answers the main account's id for "me", which is what `SetTitle`
    /// and `SetGame` need (the room must be the verified user).
    pub fn new_timed_own() -> Kit {
        let kit = Kit::new();
        kit.outward.own_room_is_main.store(true, Ordering::SeqCst);
        kit
    }

    /// A Timer rule in the owner's own channel that runs one Command line (a JSON rule: Live, no cooldowns).
    pub fn timed_rule(&self, id: &str, line: &str) -> Value {
        rule(
            id,
            true,
            json!({ "kind": "timer", "every_seconds": 300, "min_messages": 0 }),
            json!({ "kind": "own" }),
            vec![json!({ "kind": "command", "id": "a-1", "line": line })],
        )
    }

    /// One tick of the timed machine, a second after the last (injected steady and wall times).
    async fn step_timers(&self) {
        // One second on the kit's clock; the engine's own steady time is what the joins and phases use.
        self.clock.advance(Duration::from_secs(1));
        let steady = self.engine.now();
        let wall = self.timed_wall.fetch_add(1000, Ordering::SeqCst) + 1000;
        timed::tick(&self.engine, steady, wall).await;
    }

    /// Step the timed machine `seconds` seconds, then wait for the jobs.
    pub async fn advance_timers(&self, seconds: u32) {
        for _ in 0..seconds {
            self.step_timers().await;
        }
        self.idle().await;
    }

    /// Run a Timer rule's next fire: step second by second until the machine stamps it (it arms on the first tick), then wait for the job.
    pub async fn fire_timed(&self, rule_id: &str) {
        let compiled = self.engine.compiled();
        let cr = compiled.rules.iter().find(|r| r.rule.id == rule_id).expect("rule");
        let crate::models::automations::Trigger::Timer { every_seconds, .. } = cr.rule.when else {
            panic!("fire_timed drives Timer rules")
        };
        let key = if matches!(cr.rule.only_if.channels, crate::models::automations::ChannelScope::Own {}) {
            cr.own_key.clone()
        } else {
            cr.these_keys[0].clone()
        };
        let stamp = |engine: &Engine| lock(&engine.counts).timed(rule_id, &key).last_fire_ms;
        let before = stamp(&self.engine);
        let steps = i64::from(every_seconds) + timers::offset_ms(&key, every_seconds) / 1000 + 3;
        for _ in 0..steps {
            self.step_timers().await;
            if stamp(&self.engine) != before {
                break;
            }
        }
        self.idle().await;
    }

    /// A kit with one Timer rule in this test's chat: every `every_s` seconds, after `min_messages` other lines, it sends "timer line".
    pub async fn new_timer_rule(every_s: u32, min_messages: u32) -> Kit {
        let kit = Kit::new();
        // The rule posts in this test's chat, which is not the owner's: other chats must be allowed.
        kit.prefs(|p| p.other_channels = true).await;
        let mut rules = vec![rule(
            "t1",
            true,
            json!({ "kind": "timer", "every_seconds": every_s, "min_messages": min_messages }),
            this_chat(),
            vec![send("a-1", None, "timer line")],
        )];
        // Varied: the limiter refuses a repeat of the same text inside 30 s, and a fire every
        // 5 minutes of a rule with one fixed line must not be a duplicate in the tests.
        rules[0]["vary"] = json!(true);
        kit.settings(RunState::On, rules).await;
        kit
    }

    /// Two devices on the same claim server, both running that Timer rule.
    pub async fn pair_with_timer(every_s: u32, min_messages: u32) -> (Kit, Kit) {
        let a = Kit::new_timer_rule(every_s, min_messages).await;
        let b = a.second_device();
        b.prefs(|p| p.other_channels = true).await;
        let rules = lock(&a.local.settings).0.rules.clone();
        b.settings(RunState::On, rules).await;
        (a, b)
    }

    /// One live line from someone else in this test's chat (the Timer's counter reads it).
    pub async fn chat_line_from_other(&self) {
        let id = format!("line-{}", NEXT_IDS.fetch_add(1, Ordering::SeqCst));
        self.engine.on_chat(&chat_row(&id, "viewer", "just chatting"), false);
    }

    /// Compile again (an edit or an account change would).
    pub async fn recompile(&self) {
        self.engine.recompile().await;
    }

    /// A kit with one Whisper rule: when someone you have whispered with
    /// whispers, whisper "hi" back to them (the `to: null` Reply form).
    pub async fn new_whisper_rule() -> Kit {
        let kit = Kit::new();
        let rules = vec![rule(
            "w1",
            true,
            json!({ "kind": "whisper", "text": null }),
            json!({ "kind": "own" }),
            vec![json!({ "kind": "whisper", "id": "a-1", "to": null, "texts": [{ "id": "l-1", "text": "hi" }], "pick": "in_order" })],
        )];
        kit.settings(RunState::On, rules).await;
        kit
    }

    /// A whisper from `from` that Twitch stamped `age_ms` ago, as `record_incoming` hands it to the engine.
    pub async fn whisper_in(&self, whisper_id: &str, from: &str, age_ms: i64) {
        let ev = WhisperEvent {
            from_user_id: from.into(),
            from_user_login: format!("u{from}"),
            from_user_name: format!("U{from}"),
            to_user_id: main_id(),
            to_user_login: me(),
            to_user_name: me(),
            whisper_id: whisper_id.into(),
            text: "hello".into(),
        };
        let twitch_ms = crate::services::whisper_sync::twitch_now(now_ms()) - age_ms;
        // Awaited: the gates and the admission finish before the jobs are waited on (no sleep).
        self.engine.on_whisper_now(&main_id(), &ev, twitch_ms).await;
        self.idle().await;
    }

    /// A mention from `viewer` carrying these badges (`moderator`, `vip`, ...).
    pub async fn fire_with_roles(&self, id: &str, roles: &[&str]) {
        let mut row = chat_row(id, "viewer", "hey @me");
        row.badges = roles.iter().map(|r| serde_json::from_value(json!({ "name": r, "version": "1" })).expect("a badge")).collect();
        self.engine.on_chat(&row, false);
        self.idle().await;
    }
}

/// A token refusal for a scripted `FakeToken` (the second verify and the verify after a claim).
fn refusal_of(account_id: &str, how: FakeToken) -> TokenRefusal {
    match how {
        FakeToken::WrongAccount(actual) => TokenRefusal::WrongAccount { expected: account_id.to_string(), actual },
        FakeToken::Unreachable => TokenRefusal::Unreachable("Twitch did not answer".into()),
        FakeToken::NoToken => TokenRefusal::NoToken("no usable Twitch token is stored".into()),
        _ => TokenRefusal::Refused(FETCH_REFUSED.into()),
    }
}

/// A JSON rule with this trigger, scope and actions (Live, no cooldowns): the testkit's private `rule`, public.
pub fn rule_value(id: &str, when: Value, channels: Value, actions: Vec<Value>) -> Value {
    rule(id, true, when, channels, actions)
}

/// `n` chats of this test: (login, room id), each its own lane.
pub fn many_chats(n: usize) -> Vec<(String, String)> {
    (0..n).map(|i| (format!("{}x{i}", chan()), format!("77{i}"))).collect()
}

/// A mention rule in these chats whose one action is a Timeout of 600 s.
pub fn timeout_rule_in(id: &str, chats: &[(String, String)]) -> Value {
    let list: Vec<Value> = chats.iter().map(|(login, room)| chat_ref(login, room)).collect();
    rule(
        id,
        true,
        mention(),
        json!({ "kind": "these", "list": list }),
        vec![json!({ "kind": "timeout", "id": "a-1", "seconds": 600, "reason": "" })],
    )
}

fn role_badges(roles: &[&str]) -> Vec<crate::models::chat_layout::Badge> {
    roles.iter().map(|r| serde_json::from_value(json!({ "name": r, "version": "1" })).expect("a badge")).collect()
}

impl FakeOutward {
    /// The first `verified_token` of `id` that happens after a claim call has gone out answers `how` (step 9's re-verify).
    pub fn script_verify_after_claim(&self, id: &str, how: FakeToken) {
        lock(&self.after_claim).insert(id.into(), how);
    }

    /// From the first claim call on, the registry's primary reads as `other` (the account changes
    /// between step 2 and step 9).
    pub fn script_primary_after_claim(&self, other: Option<&str>) {
        *lock(&self.primary_after_claim) = Some(other.map(str::to_string));
    }

    /// How many times `verified_token` was asked for `id` (a Kick id must stay 0, D277).
    pub fn verifies_of(&self, id: &str) -> usize {
        lock(&self.verifies).get(id).copied().unwrap_or(0)
    }
}

impl FakeServer {
    /// Move the server's clock on by `ms` (a windowed claim really expires).
    pub fn advance(&self, ms: i64) {
        lock(&self.state).skew_ms += ms;
    }
}

impl FakeLocal {
    /// A window lets go of `room`.
    pub fn unhold(&self, label: &str, room: &str) {
        if let Some(holders) = lock(&self.chat).holders.get_mut(room) {
            holders.remove(label);
        }
    }
}

impl Kit {
    /// The compiled rule as the engine holds it (after `normalize`).
    pub fn stored_rule(&self, id: &str) -> Value {
        self.engine.compiled().rule(id).map(|c| c.rule.to_value()).expect("a compiled rule")
    }

    /// Why the compiled rule cannot run, if it cannot.
    pub fn compiled_rule_held(&self, id: &str) -> Option<String> {
        self.engine.compiled().rule(id).and_then(|c| c.held.clone())
    }

    /// `validate` of this rule value, as `automations_save_rule` runs it (no other rules stored).
    pub fn save_errors(&self, rule: Value) -> Vec<crate::models::automations::FieldError> {
        let mut rule = crate::models::automations::Rule::from_value(&rule).expect("a readable rule");
        crate::models::automations::validate(&mut rule, &crate::models::automations::ValidateCtx::default())
    }

    /// Engine bookings made on this test thread (E25i).
    pub fn engine_bookings(&self) -> usize {
        crate::services::chat_send_limiter::engine_bookings_for_test()
    }

    /// This test's chat as the chat rules key it.
    pub fn chat_key(&self) -> String {
        crate::services::chat_rules::channel_filter_key("twitch", &chan())
    }

    /// One mention from a different viewer in each of these chats, all handed to the engine before any job has run.
    pub fn fire_rows_in_no_wait(&self, chats: &[(String, String)], roles: &[&str]) {
        for (i, (login, room)) in chats.iter().enumerate() {
            let mut row = chat_row(&format!("many-{i}"), &format!("viewer{i}"), "hey @me");
            row.channel = login.clone();
            row.tags.insert("room-id".into(), room.clone());
            row.badges = role_badges(roles);
            self.engine.on_chat(&row, false);
        }
    }

    /// A mention from `user` in the owner's own room (login "me", room id = the main account's id), no wait.
    pub fn fire_own_room_no_wait(&self, id: &str, user: &str) {
        let mut row = chat_row(id, user, "hey @me");
        row.channel = me();
        row.tags.insert("room-id".into(), main_id());
        self.engine.on_chat(&row, false);
    }

    /// A mention from `viewer{i}` in each chat, then wait for the jobs.
    pub async fn fire_rows_in(&self, chats: &[(String, String)], roles: &[&str]) {
        self.fire_rows_in_no_wait(chats, roles);
        self.idle().await;
    }

    /// A Kit with a Delay rule in this test's chat that sends "delay line" (varied, so the limiter's 30 s duplicate rule never refuses it).
    pub async fn new_delay_rule(seconds: u32) -> Kit {
        let kit = Kit::new();
        kit.prefs(|p| p.other_channels = true).await;
        let mut delay = rule("d1", true, json!({ "kind": "delay", "seconds": seconds }), this_chat(), vec![send("a-1", None, "delay line")]);
        delay["vary"] = json!(true);
        kit.settings(RunState::On, vec![delay]).await;
        kit
    }

    /// Two devices on one claim server, both running that Delay rule.
    pub async fn pair_with_delay(seconds: u32) -> (Kit, Kit) {
        let a = Kit::new_delay_rule(seconds).await;
        let b = a.second_device();
        b.prefs(|p| p.other_channels = true).await;
        let rules = lock(&a.local.settings).0.rules.clone();
        b.settings(RunState::On, rules).await;
        (a, b)
    }

    /// A window opened `chat`: it holds the chat under "main" and the engine notes the join.
    pub async fn open_window_chat(&self, chat: &str) {
        self.local.hold("main", chat);
        super::note_window_claim_on(&self.engine, chat);
    }

    /// The last window let go of `chat`: the hold goes and the engine forgets the join (`note_window_release_on`).
    pub async fn close_window_chat(&self, chat: &str) {
        self.local.unhold("main", chat);
        super::note_window_release_on(&self.engine, chat);
    }

    /// Step each kit's timed machine one second at a time, with its clock and the shared
    /// claim server's clock, waiting for the jobs each second, so a windowed claim really
    /// expires and jobs run in order. Every kit passed shares one server (a pair).
    pub async fn advance_all(kits: &[&Kit], seconds: u32) {
        for _ in 0..seconds {
            for kit in kits {
                kit.step_timers().await;
            }
            kits[0].server.advance(1000);
            for kit in kits {
                kit.idle().await;
            }
        }
    }

    /// A kit whose Kick actor is connected in the fake Kick door (P4's `connect_kick`).
    pub fn new_kick_actor() -> Kit {
        let kit = Kit::new();
        kit.outward.connect_kick(KICK_ME, "mykick");
        kit
    }

    /// P4's Kick message rule (a Kick `send_as`, "hello" fires it) whose actions are these.
    pub fn kick_rule_with(&self, id: &str, actions: Value) -> Value {
        let mut rule = kick_these_rule(id, &["alpha"]);
        rule["actions"] = actions;
        rule
    }

    /// A Kick row saying "hello" from viewer 7001 carrying these badges, then wait for the jobs.
    pub async fn fire_kick_row(&self, id: &str, roles: &[&str]) {
        let mut row = kick_row(id, "7001", "hello");
        row.badges = role_badges(roles);
        self.engine.on_chat(&row, false);
        self.idle().await;
    }
}

// ---- the kit ---------------------------------------------------------------------------------

/// The main account's send limiter is one per process, so engine tests that
/// send as the main account take turns on it: the first kit of a test waits
/// for the earlier test's last kit to drop, then empties the limiter.
static PRIMARY_TURN: Mutex<()> = Mutex::new(());

thread_local! {
    static TURN: std::cell::RefCell<(usize, Option<std::sync::MutexGuard<'static, ()>>)> =
        const { std::cell::RefCell::new((0, None)) };
}

struct PrimaryLease;

impl PrimaryLease {
    fn take() -> PrimaryLease {
        TURN.with(|t| {
            let mut t = t.borrow_mut();
            if t.0 == 0 {
                t.1 = Some(PRIMARY_TURN.lock().unwrap_or_else(|e| e.into_inner()));
                chat_send_limiter::reset_primary_for_test();
            }
            t.0 += 1;
        });
        PrimaryLease
    }
}

impl Drop for PrimaryLease {
    fn drop(&mut self) {
        TURN.with(|t| {
            let mut t = t.borrow_mut();
            t.0 -= 1;
            if t.0 == 0 {
                t.1 = None;
            }
        });
    }
}

/// One engine on fakes; `second_device` makes another on the same claim server.
pub struct Kit {
    pub engine: Arc<Engine>,
    pub outward: Arc<FakeOutward>,
    pub local: Arc<FakeLocal>,
    pub server: Arc<FakeServer>,
    /// The token verifier's clock.
    pub clock: TestClock,
    pub command_doors: Arc<FakeCommandDoors>,
    pub mod_doors: Arc<FakeModDoors>,
    pub whisper_door: Arc<FakeWhisperDoor>,
    /// The timed machine's injected wall time (ms); its steady time is the engine's own clock.
    timed_wall: AtomicI64,
    rx: Mutex<Option<watch::Receiver<u64>>>,
    /// Dropped last: the main limiter is this test's until the kit is gone.
    _turn: PrimaryLease,
}

impl Kit {
    pub fn new() -> Kit {
        Kit::on(Arc::new(FakeServer::default()))
    }

    /// For plain `#[test]`s: the same kit (nothing here needs a runtime).
    pub fn new_sync() -> Kit {
        Kit::new()
    }

    fn on(server: Arc<FakeServer>) -> Kit {
        let turn = PrimaryLease::take();
        let local = Arc::new(FakeLocal::new());
        let clock = TestClock::new();
        let outward = Arc::new(FakeOutward::new(server.clone(), clock.as_clock()));
        let (engine, rx) = Engine::new(Deps {
            outward: outward.clone(),
            local: local.clone(),
            app: None,
            files: None,
            device: Device::fresh(),
            timings: fast(),
            clock: clock.as_clock(),
            spawn: test_spawner(),
        });
        engine.presence.store(true, Ordering::SeqCst);
        Kit {
            engine,
            command_doors: outward.command_doors.clone(),
            mod_doors: outward.mod_doors.clone(),
            whisper_door: outward.whisper_door.clone(),
            outward,
            local,
            server,
            clock,
            timed_wall: AtomicI64::new(now_ms()),
            rx: Mutex::new(Some(rx)),
            _turn: turn,
        }
    }

    /// Another device signed in to the same accounts, on the same claim server.
    pub fn second_device(&self) -> Kit {
        Kit::on(self.server.clone())
    }

    pub async fn settings(&self, run: RunState, rules: Vec<Value>) {
        {
            let mut s = lock(&self.local.settings);
            s.0.run = run;
            if run != RunState::Off {
                s.0.last_active = run;
            }
            s.0.rules = rules;
        }
        self.engine.recompile().await;
    }

    pub async fn prefs(&self, change: impl FnOnce(&mut AutomationPrefs)) {
        change(&mut lock(&self.local.settings).1);
        self.engine.recompile().await;
    }

    pub async fn set_away(&self, message: Option<&str>) {
        let away = message.map(|m| Away { message: m.into(), since_ms: now_ms() });
        lock(&self.local.settings).0.away = away.clone();
        self.engine.recompile().await;
        self.engine.set_away_now(away);
    }

    pub fn fire_no_wait(&self, id: &str) {
        self.engine.on_chat(&chat_row(id, "viewer", "hey @me"), false);
    }

    pub fn fire_row_no_wait(&self, row: ChatMessage) {
        self.engine.on_chat(&row, false);
    }

    pub async fn fire(&self, id: &str) {
        self.fire_from(id, "viewer").await;
    }

    pub async fn fire_from(&self, id: &str, user: &str) {
        self.engine.on_chat(&chat_row(id, user, "hey @me"), false);
        self.idle().await;
    }

    pub async fn fire_in(&self, id: &str, login: &str) {
        let mut row = chat_row(id, "viewer", "hey @me");
        row.channel = login.into();
        row.tags.insert("room-id".into(), "777".into());
        self.engine.on_chat(&row, false);
        self.idle().await;
    }

    pub async fn fire_row(&self, row: ChatMessage) {
        self.engine.on_chat(&row, false);
        self.idle().await;
    }

    /// Wait until no job is queued or running (5 s at most).
    pub async fn idle(&self) {
        tokio::time::sleep(Duration::from_millis(5)).await;
        let started = Instant::now();
        while !self.engine.executor.is_idle() {
            assert!(started.elapsed() < Duration::from_secs(10), "jobs never finished");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    /// Wait for claim tasks and releases to finish.
    pub async fn settle_claims(&self) {
        tokio::time::sleep(Duration::from_millis(400)).await;
    }

    pub async fn wait_until(&self, done: impl Fn() -> bool) {
        let started = Instant::now();
        while !done() {
            assert!(started.elapsed() < Duration::from_secs(5), "never happened: {:?}", self.activity_texts());
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    /// A job for `msg_id` and rule `rule_id`, as admission would build it.
    pub fn job_for(&self, msg_id: &str, rule_id: &str) -> Job {
        let compiled = self.engine.compiled();
        let facts = facts::from_chat(&chat_row(msg_id, "viewer", "hey @me")).expect("facts");
        let index = compiled.rules.iter().position(|r| r.rule.id == rule_id).expect("rule");
        let rule = &compiled.rules[index].rule;
        let room = claims::event_room(rule, &facts).to_string();
        let (plans, unready) = claims::classes_for(rule, &room);
        let lanes = guards::lanes_for(rule, &plans);
        Job::new(
            &self.engine,
            compiled.clone(),
            index,
            Arc::new(facts),
            Vec::new(),
            HashMap::new(),
            true,
            plans,
            unready,
            lanes,
            room,
            self.engine.now(),
        )
    }

    /// A job for `rule_id` on these facts (an edge event's, say), as admission would build it.
    pub fn job_with_facts(&self, facts: Facts, rule_id: &str) -> Job {
        let compiled = self.engine.compiled();
        let index = compiled.rules.iter().position(|r| r.rule.id == rule_id).expect("rule");
        let rule = &compiled.rules[index].rule;
        let room = claims::event_room(rule, &facts).to_string();
        let (plans, unready) = claims::classes_for(rule, &room);
        let lanes = guards::lanes_for(rule, &plans);
        Job::new(
            &self.engine,
            compiled.clone(),
            index,
            Arc::new(facts),
            Vec::new(),
            HashMap::new(),
            true,
            plans,
            unready,
            lanes,
            room,
            self.engine.now(),
        )
    }

    /// Kick is connected as the test's account (the door's id and the chat
    /// identity), then the rules compile again.
    pub async fn connect_kick(&self) {
        self.outward.connect_kick(KICK_ME, "mykick");
        self.engine.recompile().await;
    }

    /// A job for a row of another platform, as admission would build it.
    pub fn job_for_row(&self, row: ChatMessage, rule_id: &str) -> Job {
        let compiled = self.engine.compiled();
        let facts = facts::from_chat(&row).expect("facts");
        let index = compiled.rules.iter().position(|r| r.rule.id == rule_id).expect("rule");
        let rule = &compiled.rules[index].rule;
        let room = claims::event_room(rule, &facts).to_string();
        let (plans, unready) = claims::classes_for(rule, &room);
        let lanes = guards::lanes_for(rule, &plans);
        Job::new(
            &self.engine,
            compiled.clone(),
            index,
            Arc::new(facts),
            Vec::new(),
            HashMap::new(),
            true,
            plans,
            unready,
            lanes,
            room,
            self.engine.now(),
        )
    }

    /// A message you typed in `login` just now (the typed path's record).
    pub fn typed_now(&self, account: &str, login: &str) {
        // The main account is the engine's primary here, but `record_sent_as` asks the
        // machine's own account registry, so the main account's typed line goes in directly.
        if account == main_id() {
            chat_send_limiter::record_sent(login, "typed by hand");
        } else {
            chat_send_limiter::record_sent_as(account, login, "typed by hand");
        }
    }

    pub fn activity_texts(&self) -> Vec<String> {
        self.engine.activity_list(Filter::All, 500, None).into_iter().map(|e| e.text).collect()
    }

    pub fn spawn_reconcile(&self) -> futures::future::AbortHandle {
        let rx = lock(&self.rx).take().expect("one reconcile per kit");
        super::reconcile::spawn(self.engine.clone(), rx)
    }

    /// Request a pass and wait for its compile.
    pub async fn settle(&self) {
        let mut compiled = self.engine.compiled_events();
        self.engine.request_refresh();
        let _ = tokio::time::timeout(Duration::from_secs(2), compiled.changed()).await;
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    /// Admit 2 x spm + 1 rows in each of two consecutive 2-minute windows
    /// (the second one dated 121 s later), which trips the breaker.
    pub async fn trip_breaker_with_rows(&self, row: impl Fn(usize) -> ChatMessage) {
        let compiled = self.engine.compiled();
        let per_window = 2 * compiled.prefs.sends_per_minute as usize + 1;
        // Two consecutive 2-minute windows on the steady clock.
        let t0 = (self.engine.now() / guards::BREAKER_WINDOW_MS + 1) * guards::BREAKER_WINDOW_MS;
        for window in 0..2i64 {
            let at = t0 + window * guards::BREAKER_WINDOW_MS;
            for i in 0..per_window {
                let msg = row(window as usize * 1000 + i);
                if let Some(f) = facts::from_chat(&msg) {
                    self.engine.admit(&compiled, Arc::new(f), at);
                }
            }
        }
        self.idle().await;
    }
}
