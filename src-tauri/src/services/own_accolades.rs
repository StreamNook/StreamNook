//! Your own accolades, earned in Rust on every platform.
//!
//! The phones already COUNT everything (watch minutes, messages, streams, chest
//! points); what they never did is SAVE an accolade, because every grant was
//! desktop TypeScript. This module works out the signed-in member's own wall from
//! their stats, saved set, IVR years, cosmetics, drops claimed and channel
//! points held (each a part with its own state), and writes the base and secret
//! accolades their numbers have lit to `user_accolades`, for the signed-in
//! member only.
//!
//! Drops and points are read with ONE captured token. The drops token is
//! device-wide and its account is not guaranteed to be the signed-in one, so
//! `own_drops_points` loads it once into a local, validates that value against
//! Twitch, requires the account it belongs to, the primary account and the
//! owner of the follow list to be the same member, and uses only that value. The
//! followed-channels walk that adds up the points is bound to the same account
//! (it is dropped if the account changed while it ran, or if any batch came back
//! wrong), and only a complete walk's total is kept, on disk.
//!
//! Triggers go through `poke`: sign-in, the stats and watch-minute flushes, a
//! confirmed drop claim, the end of a points walk, a 30-minute clock while signed
//! in, and the local date changing. A trailing 10-minute throttle runs a trigger
//! that lands inside the window once, at the window's end; a trigger that lands
//! during a run runs it once more. Reading the wall (`own_wall`) writes at once.

use crate::services::account_store::AccountStore;
use crate::services::cosmetics_registry as registry;
use crate::services::drops_auth_service::DropsAuthService;
use crate::services::drops_service::DropsService;
use crate::services::member_profile::{
    self, AccoladeWall, Catalog, Clock, Kind, MemberFacts, Part, RuleDef, Special, StatKind,
    WallMode, CATALOG,
};
use crate::services::streamnook_backend as backend;
use crate::services::twitch_service::TwitchService;
use chrono::{DateTime, Local, NaiveDate};
use log::{debug, warn};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

/// Emitted (payload `{ "user_id": "<id>" }`) after an insert or a completed points walk.
const EVENT: &str = "own-accolades-changed";
/// The trailing throttle: at most one run per window, the last trigger's run at its end.
const WINDOW: Duration = Duration::from_secs(10 * 60);
/// A validated token is trusted this long.
const VALIDATE_TTL: Duration = Duration::from_secs(30 * 60);
/// A walk that did not complete is tried again no sooner than this.
const WALK_RETRY: Duration = Duration::from_secs(10 * 60);
/// The clock that looks for a new local date, and every Nth look pokes a run.
const TICK: Duration = Duration::from_secs(5 * 60);
const PERIODIC_EVERY: u32 = 6;
/// The accolades only the client can attest (R5 E4): no rule lights them.
const ATTESTED: [&str; 4] = [
    "insomniac",
    "perfect_corner",
    "shiny_peepo",
    "worth_the_wait",
];

/// Why a run was asked for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Poke {
    SignIn,
    Stats,
    WatchMinutes,
    DropClaim,
    WalkEnded,
    Periodic,
    DateChanged,
}

/// Drops claimed and channel points held, each a part. Unknown unless the one
/// captured token belongs to the signed-in member.
#[derive(Clone, Debug, PartialEq)]
pub struct DropsPoints {
    pub drops: Part<u64>,
    pub points: Part<u64>,
}

impl DropsPoints {
    pub fn unknown() -> Self {
        Self {
            drops: Part::Failed,
            points: Part::Failed,
        }
    }
}

/// Everything an own wall reads, each with its state.
#[derive(Clone, Debug)]
pub struct OwnParts {
    pub member_id: String,
    /// Stats, saved set, IVR, 7TV counts and the member number.
    pub facts: MemberFacts,
    pub drops: Part<u64>,
    pub points: Part<u64>,
}

// ---- Which account ---------------------------------------------------------------

/// The signed-in account a read or write belongs to: who is primary and the
/// number that moves when the account changes.
#[derive(Clone, Debug, PartialEq)]
struct Bind {
    generation: u64,
    member: String,
}

fn bind_now() -> Option<Bind> {
    let member = AccountStore::primary()?.user_id;
    (!member.is_empty()).then(|| Bind {
        generation: TwitchService::account_generation(),
        member,
    })
}

/// Whether the account is still the one `bind` captured.
fn still(bind: &Bind) -> bool {
    bind_now().as_ref() == Some(bind)
}

fn announce(member_id: &str) {
    if let Some(app) = crate::services::providers::app_handle() {
        use tauri::Emitter;
        let _ = app.emit(EVENT, json!({ "user_id": member_id }));
    }
}

// ---- The drops token ---------------------------------------------------------------

/// A hash of a token string: the cache key, so a token never borrows another's
/// answer and the secret itself is not kept as a key.
fn token_key(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Which account a token belongs to, by the token's own hash.
#[derive(Default)]
struct TokenUsers {
    by_token: HashMap<String, (String, Instant)>,
}

impl TokenUsers {
    fn get(&self, token: &str, now: Instant) -> Option<String> {
        self.by_token
            .get(&token_key(token))
            .filter(|(_, at)| now.saturating_duration_since(*at) < VALIDATE_TTL)
            .map(|(user, _)| user.clone())
    }

    fn put(&mut self, token: &str, user: &str, now: Instant) {
        if self.by_token.len() >= 8 {
            self.by_token.clear();
        }
        self.by_token
            .insert(token_key(token), (user.to_string(), now));
    }
}

static TOKEN_USERS: LazyLock<Mutex<TokenUsers>> =
    LazyLock::new(|| Mutex::new(TokenUsers::default()));

/// The account a token belongs to, from `oauth2/validate`. Never touches the
/// stored token (a rejected one is reported, not deleted).
async fn validated_user(token: &str) -> Result<String, String> {
    if let Some(user) = TOKEN_USERS
        .lock()
        .ok()
        .and_then(|c| c.get(token, Instant::now()))
    {
        return Ok(user);
    }
    let res = crate::services::http::client()
        .get("https://id.twitch.tv/oauth2/validate")
        .header("Authorization", format!("OAuth {token}"))
        .send()
        .await
        .map_err(|e| format!("validate: {e}"))?;
    if !res.status().is_success() {
        return Err(format!("validate: {}", res.status()));
    }
    let body: Value = res.json().await.map_err(|e| format!("validate: {e}"))?;
    let user = body
        .get("user_id")
        .and_then(Value::as_str)
        .filter(|u| !u.is_empty())
        .ok_or("validate: no user id")?
        .to_string();
    if let Ok(mut cache) = TOKEN_USERS.lock() {
        cache.put(token, &user, Instant::now());
    }
    Ok(user)
}

/// The account a walk or a read is bound to: the signed-in account and the
/// owner of the follow list (`get_all_followed_channels` reads the MAIN token).
#[derive(Clone, Debug, PartialEq)]
struct WalkBind {
    bind: Bind,
    owner: String,
}

async fn walk_bind() -> Option<WalkBind> {
    let bind = bind_now()?;
    let owner = TwitchService::get_user_info().await.ok()?.id;
    Some(WalkBind { bind, owner })
}

/// Whether a validated token's account, the member asked about, the primary
/// account and the follow-list owner are one and the same.
fn account_matches(validated: &str, member_id: &str, bind: &WalkBind) -> bool {
    !member_id.is_empty()
        && validated == member_id
        && bind.bind.member == member_id
        && bind.owner == member_id
}

// ---- Drops claimed -------------------------------------------------------------------

/// A process-local device and session id for the Android-client inventory read.
static DEVICE: LazyLock<(String, String)> = LazyLock::new(|| {
    (
        uuid::Uuid::new_v4().simple().to_string(),
        uuid::Uuid::new_v4().simple().to_string(),
    )
});

/// Drops claimed from an `Inventory` answer: the desktop's rule, the sum over the
/// permanent inventory's `gameEventDrops` that parse as completed drops of each
/// one's `totalCount` (at least 1). None when the answer is an error.
fn drops_claimed(body: &Value) -> Option<u64> {
    if body.get("errors").is_some() || body.pointer("/data/currentUser").is_none_or(Value::is_null)
    {
        return None;
    }
    let events = body
        .pointer("/data/currentUser/inventory/gameEventDrops")
        .and_then(Value::as_array);
    Some(
        events
            .map(|events| {
                events
                    .iter()
                    .filter(|e| {
                        e["id"].is_string()
                            && e["name"].is_string()
                            && e["imageURL"].is_string()
                            && e["lastAwardedAt"]
                                .as_str()
                                .is_some_and(|t| DateTime::parse_from_rfc3339(t).is_ok())
                    })
                    .map(|e| e["totalCount"].as_i64().unwrap_or(1).max(1) as u64)
                    .sum()
            })
            .unwrap_or(0),
    )
}

/// Drops claimed with this token only. No `DropsService` lock is taken.
async fn fetch_drops(token: &str) -> Part<u64> {
    let sent = crate::services::http::client()
        .post("https://gql.twitch.tv/gql")
        .headers(DropsService::gql_headers(token, &DEVICE.0, &DEVICE.1))
        .json(&json!({
            "operationName": "Inventory",
            "variables": { "fetchRewardCampaigns": false },
            "extensions": { "persistedQuery": {
                "version": 1,
                "sha256Hash": crate::services::drops_service::INVENTORY_QUERY_HASH
            } }
        }))
        .send()
        .await;
    let body: Value = match sent {
        Ok(res) => match res.json().await {
            Ok(body) => body,
            Err(e) => {
                debug!("[OwnAccolades] inventory unreadable: {e}");
                return Part::Failed;
            }
        },
        Err(e) => {
            debug!("[OwnAccolades] inventory failed: {e}");
            return Part::Failed;
        }
    };
    match drops_claimed(&body) {
        Some(n) => Part::Answered(n),
        None => Part::Failed,
    }
}

// ---- Channel points held ---------------------------------------------------------------

const WEB_CLIENT_ID: &str = env!("TWITCH_WEB_CLIENT_ID");

const POINTS_QUERY: &str = r#"
    query ChannelPointsContext($channelLogin: String!) {
        user(login: $channelLogin) {
            channel {
                self {
                    communityPoints {
                        balance
                    }
                }
            }
        }
    }
    "#;

/// The positive balances in one batch answer, summed. An answer is whole or it
/// is an error: not an array, a length that does not match the batch, or any
/// item that came back with `errors` makes the batch a failed one.
fn sum_batch(parsed: &Value, expected: usize) -> Result<u64, String> {
    let items = parsed.as_array().ok_or("the batch answer is not a list")?;
    if items.len() != expected {
        return Err(format!("the batch answered {} of {expected}", items.len()));
    }
    let mut total = 0u64;
    for item in items {
        if item.get("errors").is_some() {
            return Err("a batch item came back with errors".to_string());
        }
        if let Some(balance) = item
            .pointer("/data/user/channel/self/communityPoints/balance")
            .and_then(Value::as_i64)
        {
            if balance > 0 {
                total += balance as u64;
            }
        }
    }
    Ok(total)
}

/// What a walk found.
#[derive(Debug, PartialEq)]
struct WalkOutcome {
    total: u64,
    failed_batches: usize,
}

/// Adds up the channel points of every followed channel with `token`. A failure
/// to list the followed channels is an error; a batch that failed is counted.
async fn walk_points(token: &str) -> Result<WalkOutcome, String> {
    let mut channels: Vec<(String, String)> = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let (page, next) = TwitchService::get_all_followed_channels(100, cursor.clone())
            .await
            .map_err(|e| format!("followed channels: {e}"))?;
        for s in page {
            if !s.user_login.is_empty() && !s.user_id.is_empty() {
                channels.push((s.user_login, s.user_id));
            }
        }
        match next {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }
    let client = crate::services::http::client();
    let (mut total, mut failed_batches) = (0u64, 0usize);
    for chunk in channels.chunks(crate::services::twitch_limits::GQL_MAX_BATCHED_OPERATIONS) {
        let body: Vec<Value> = chunk
            .iter()
            .map(|(login, _)| {
                json!({
                    "operationName": "ChannelPointsContext",
                    "query": POINTS_QUERY,
                    "variables": { "channelLogin": login.to_lowercase() }
                })
            })
            .collect();
        let sent = client
            .post("https://gql.twitch.tv/gql")
            .header("Client-Id", WEB_CLIENT_ID)
            .header("Authorization", format!("OAuth {token}"))
            .json(&body)
            .send()
            .await;
        let batch = match sent {
            Ok(res) => {
                let ok = res.status().is_success();
                match res.json::<Value>().await {
                    Ok(parsed) if ok => sum_batch(&parsed, chunk.len()),
                    Ok(_) => Err("the batch was refused".to_string()),
                    Err(e) => Err(format!("the batch was unreadable: {e}")),
                }
            }
            Err(e) => Err(format!("the batch failed: {e}")),
        };
        match batch {
            Ok(n) => total += n,
            Err(e) => {
                debug!("[OwnAccolades] points batch failed: {e}");
                failed_batches += 1;
            }
        }
    }
    Ok(WalkOutcome {
        total,
        failed_batches,
    })
}

/// Whether a finished walk may be kept: it ran under one account the whole way
/// (the signed-in account, the follow-list owner), that account is the one the
/// token was validated for, and not one batch failed.
fn walk_accepted(
    start: &WalkBind,
    end: Option<&WalkBind>,
    validated: &str,
    failed_batches: usize,
) -> bool {
    failed_batches == 0
        && end == Some(start)
        && account_matches(validated, &start.bind.member, start)
}

/// A complete walk's total, per validated drops account.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct Walked {
    total: u64,
    /// The local date it was walked on, `YYYY-MM-DD`.
    day: String,
    /// Unix seconds.
    at: i64,
}

/// What a walk saves: a complete one only.
fn walk_to_save(accepted: bool, total: u64, day: &str, at: i64) -> Option<Walked> {
    accepted.then(|| Walked {
        total,
        day: day.to_string(),
        at,
    })
}

/// Whether a walk should start: none saved for this account, or not walked today,
/// and an earlier try that did not complete is at least `WALK_RETRY` old.
fn walk_due(saved: Option<&Walked>, today: &str, retry_at: Option<Instant>, now: Instant) -> bool {
    retry_at.is_none_or(|t| now >= t) && saved.is_none_or(|w| w.day != today)
}

struct WalkState {
    running: bool,
    retry_at: Option<Instant>,
}

static WALK: Mutex<WalkState> = Mutex::new(WalkState {
    running: false,
    retry_at: None,
});

fn today_string() -> String {
    Local::now().date_naive().to_string()
}

fn app_file(name: &str) -> Option<PathBuf> {
    crate::services::cache_service::get_app_data_dir()
        .ok()
        .map(|d| d.join(name))
}

fn read_map<T: serde::de::DeserializeOwned>(path: &Path) -> HashMap<String, T> {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn write_map<T: Serialize>(path: &Path, map: &HashMap<String, T>) {
    if let Ok(body) = serde_json::to_vec(map) {
        let _ = std::fs::write(path, body);
    }
}

/// Serializes the read-modify-write of the two files.
static FILES: Mutex<()> = Mutex::new(());

fn saved_walk(user_id: &str) -> Option<Walked> {
    let _files = FILES.lock().unwrap_or_else(|e| e.into_inner());
    read_map::<Walked>(&app_file("points_walk.json")?).remove(user_id)
}

fn save_walk(user_id: &str, walked: Walked) {
    let _files = FILES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(path) = app_file("points_walk.json") {
        let mut map = read_map::<Walked>(&path);
        map.insert(user_id.to_string(), walked);
        write_map(&path, &map);
    }
}

/// Starts the background walk (one at a time) with the token value captured by
/// the caller. When it ends it is kept only if `walk_accepted`; the end is a
/// trigger and an event either way a total was saved.
fn start_walk(token: String, validated: String, start: WalkBind) {
    {
        let mut state = WALK.lock().unwrap_or_else(|e| e.into_inner());
        if state.running {
            return;
        }
        state.running = true;
    }
    tauri::async_runtime::spawn(async move {
        let outcome = walk_points(&token).await;
        let end = walk_bind().await;
        let (accepted, total) = match &outcome {
            Ok(o) => (
                walk_accepted(&start, end.as_ref(), &validated, o.failed_batches),
                o.total,
            ),
            Err(e) => {
                debug!("[OwnAccolades] points walk failed: {e}");
                (false, 0)
            }
        };
        {
            let mut state = WALK.lock().unwrap_or_else(|e| e.into_inner());
            state.running = false;
            state.retry_at = (!accepted).then(|| Instant::now() + WALK_RETRY);
        }
        if let Some(walked) = walk_to_save(
            accepted,
            total,
            &today_string(),
            chrono::Utc::now().timestamp(),
        ) {
            save_walk(&validated, walked);
            announce(&validated);
            poke(Poke::WalkEnded);
        }
    });
}

/// Drops claimed and channel points held for `member_id`, read with ONE captured
/// token and unknown unless that token belongs to the signed-in member. Points
/// between walks are the saved complete-walk total for that account; a walk is
/// started in the background when one is due.
pub async fn own_drops_points(member_id: &str) -> DropsPoints {
    let Ok(token) = DropsAuthService::get_token().await else {
        return DropsPoints::unknown();
    };
    let Some(start) = walk_bind().await else {
        return DropsPoints::unknown();
    };
    let Ok(validated) = validated_user(&token).await else {
        return DropsPoints::unknown();
    };
    if !account_matches(&validated, member_id, &start) {
        return DropsPoints::unknown();
    }
    let drops = fetch_drops(&token).await;
    if !still(&start.bind) {
        return DropsPoints::unknown();
    }
    let saved = saved_walk(&validated);
    let due = {
        let state = WALK.lock().unwrap_or_else(|e| e.into_inner());
        !state.running
            && walk_due(
                saved.as_ref(),
                &today_string(),
                state.retry_at,
                Instant::now(),
            )
    };
    if due {
        start_walk(token, validated, start);
    }
    DropsPoints {
        drops,
        points: saved.map_or(Part::Failed, |w| Part::Answered(w.total)),
    }
}

// ---- The own wall -----------------------------------------------------------------------

async fn read_parts(bind: &Bind, login: &str) -> OwnParts {
    let id: String = bind.member.chars().filter(char::is_ascii_digit).collect();
    let (stats, saved, ivr, inventory, dp) = tokio::join!(
        member_profile::first_row("user_stats", format!("select=*&user_id=eq.{id}")),
        crate::services::profile_view::accolades(&bind.member),
        async {
            if login.is_empty() {
                Err("no login to ask about".to_string())
            } else {
                crate::services::ivr::user_summary(login).await
            }
        },
        crate::services::seventv_cosmetics_resolver::inventory(&bind.member),
        own_drops_points(&bind.member),
    );
    let facts = MemberFacts {
        member_id: bind.member.clone(),
        number: registry::member_number(&bind.member),
        stats: Part::from_result(stats, "user_stats"),
        saved: Part::from_result(saved.map(|ids| ids.into_iter().collect()), "user_accolades"),
        ivr: Part::from_result(ivr, "ivr"),
        seventv: match inventory {
            Some(i) => Part::Answered((i.paints.len() as u64, i.badges.len() as u64)),
            None => Part::Failed,
        },
        ..Default::default()
    };
    OwnParts {
        member_id: bind.member.clone(),
        facts,
        drops: dp.drops,
        points: dp.points,
    }
}

/// The member's parts and own wall; None when they are not a member (or the
/// member list has not loaded), or when their stats or saved set did not read
/// (a wall built without them would show earned accolades as locked).
async fn build(bind: &Bind, login: &str, clock: &Clock) -> Option<(OwnParts, AccoladeWall)> {
    registry::ready(Duration::from_secs(3)).await;
    registry::member_number(&bind.member)?;
    let parts = read_parts(bind, login).await;
    if !parts.facts.stats.is_answered() || !parts.facts.saved.is_answered() {
        return None;
    }
    let (input, unknown) = member_profile::wall_input(
        &parts.facts,
        clock,
        parts.drops.as_answer().copied(),
        parts.points.as_answer().copied(),
    );
    let wall = member_profile::wall_core(&CATALOG, &input, &unknown, WallMode::Own);
    Some((parts, wall))
}

fn primary_login() -> String {
    AccountStore::primary().map(|a| a.login).unwrap_or_default()
}

/// The signed-in member's own wall, and the grants it lights, written at once.
/// None unless `member_id` is the signed-in primary member.
pub async fn own_wall(member_id: &str, tz_offset_min: Option<i32>) -> Option<AccoladeWall> {
    let bind = bind_now().filter(|b| b.member == member_id)?;
    let (parts, wall) = build(&bind, &primary_login(), &Clock::system(tz_offset_min)).await?;
    remember(&bind.member, &wall);
    grant_from(&bind, &parts, &wall).await;
    Some(wall)
}

/// The last own wall Rust built for this member, from disk. No network.
pub fn cached_own_wall(member_id: &str) -> Option<AccoladeWall> {
    let _files = FILES.lock().unwrap_or_else(|e| e.into_inner());
    cached_from(&app_file("own_accolades.json")?, member_id)
}

fn cached_from(path: &Path, member_id: &str) -> Option<AccoladeWall> {
    read_map::<AccoladeWall>(path).remove(member_id)
}

fn remember(member_id: &str, wall: &AccoladeWall) {
    let _files = FILES.lock().unwrap_or_else(|e| e.into_inner());
    let Some(path) = app_file("own_accolades.json") else {
        return;
    };
    let mut map = read_map::<AccoladeWall>(&path);
    if map.get(member_id) != Some(wall) {
        map.insert(member_id.to_string(), wall.clone());
        write_map(&path, &map);
    }
}

// ---- What gets written ------------------------------------------------------------------

/// Whether the inputs an accolade's rule reads all answered.
fn inputs_known(rule: &RuleDef, p: &OwnParts) -> bool {
    let known = |stat: StatKind| match stat {
        StatKind::Messages | StatKind::Hours | StatKind::Streams => p.facts.stats.is_answered(),
        StatKind::Drops => p.drops.is_answered(),
        StatKind::Points => p.points.is_answered(),
        StatKind::Cosmetics => p.facts.seventv.is_answered(),
        StatKind::Years => p.facts.ivr.is_answered(),
    };
    match rule {
        RuleDef::Stat(r) => known(r.stat),
        RuleDef::Special(s) => match s.special {
            Special::FirstHundred | Special::Palindrome | Special::Nice => true,
            Special::Triple => {
                known(StatKind::Messages) && known(StatKind::Hours) && known(StatKind::Drops)
            }
            Special::Completionist => [
                StatKind::Messages,
                StatKind::Streams,
                StatKind::Drops,
                StatKind::Points,
                StatKind::Cosmetics,
                StatKind::Years,
            ]
            .into_iter()
            .all(known),
            Special::Saved => false,
        },
    }
}

/// The accolade ids to write: lit by the own wall, not saved yet, of a kind the
/// numbers decide (base or secret), and with every input their rule reads
/// answered. Seasons, the cake day, events and the offline three never come
/// from here (the server and the client moments grant those), and nothing is
/// written while the saved set is unknown.
pub fn to_grant(parts: &OwnParts, wall: &AccoladeWall, saved: &HashSet<String>) -> Vec<String> {
    to_grant_with(&CATALOG, parts, wall, saved)
}

pub fn to_grant_with(
    cat: &Catalog,
    parts: &OwnParts,
    wall: &AccoladeWall,
    saved: &HashSet<String>,
) -> Vec<String> {
    if !parts.facts.saved.is_answered() || !parts.facts.stats.is_answered() {
        return Vec::new();
    }
    wall.items
        .iter()
        .filter(|m| m.earned && !saved.contains(&m.id))
        .filter(|m| {
            cat.accolades
                .iter()
                .find(|d| d.id == m.id)
                .is_some_and(|d| {
                    matches!(d.kind, Kind::Base | Kind::Secret) && inputs_known(&d.rule, parts)
                })
        })
        .map(|m| m.id.clone())
        .collect()
}

/// How a pass of writes ended.
#[derive(Debug, Default, PartialEq)]
struct Written {
    inserted: Vec<String>,
    failed: usize,
    /// The account changed before a write, so the rest were not made.
    aborted: bool,
}

/// One request per id, re-checking `still` right before each, so a refused id
/// never fails the others and an account that changed mid-pass gets no more writes.
async fn write_all<F, Fut>(ids: &[String], still: impl Fn() -> bool, mut insert: F) -> Written
where
    F: FnMut(String) -> Fut,
    Fut: Future<Output = Result<bool, String>>,
{
    let mut out = Written::default();
    for id in ids {
        if !still() {
            out.aborted = true;
            break;
        }
        match insert(id.clone()).await {
            Ok(true) => out.inserted.push(id.clone()),
            Ok(false) => {}
            Err(e) => {
                warn!("[OwnAccolades] {id} not saved: {e}");
                out.failed += 1;
            }
        }
    }
    out
}

fn accolade_row(member_id: &str, accolade_id: &str) -> Value {
    json!({
        "twitch_user_id": member_id,
        "accolade_id": accolade_id,
        "earned_at": chrono::Utc::now().to_rfc3339(),
    })
}

/// Writes what the wall has lit and the saved set lacks. True when nothing went
/// wrong (a pass that wrote nothing is a success).
async fn grant_from(bind: &Bind, parts: &OwnParts, wall: &AccoladeWall) -> bool {
    let Some(saved) = parts.facts.saved.as_answer() else {
        return false;
    };
    let ids = to_grant(parts, wall, saved);
    if ids.is_empty() {
        return true;
    }
    let member = bind.member.clone();
    let out = write_all(
        &ids,
        || still(bind),
        |id| {
            let row = accolade_row(&member, &id);
            async move {
                backend::insert_ignore("user_accolades", "twitch_user_id,accolade_id", row).await
            }
        },
    )
    .await;
    if !out.inserted.is_empty() {
        crate::services::profile_view::bump_member_epoch(&bind.member);
        announce(&bind.member);
    }
    !out.aborted && out.failed == 0
}

/// One pass: the wall from fresh reads, then the writes.
async fn run_once(member_id: &str) -> bool {
    let Some(bind) = bind_now().filter(|b| b.member == member_id) else {
        return false;
    };
    let Some((parts, wall)) = build(&bind, &primary_login(), &Clock::system(None)).await else {
        return false;
    };
    remember(&bind.member, &wall);
    grant_from(&bind, &parts, &wall).await
}

// ---- The throttle -------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
enum Step {
    Run,
    Wait(Duration),
    Nothing,
}

/// A trailing throttle: at most one run per window; a trigger inside the window
/// schedules one run at its end; a trigger during a run runs it once more.
#[derive(Default)]
struct Throttle {
    /// When the last run that succeeded ended.
    last_ok: Option<Instant>,
    running: bool,
    dirty: bool,
    scheduled: bool,
}

impl Throttle {
    fn trigger(&mut self, now: Instant, force: bool) -> Step {
        if self.running {
            self.dirty = true;
            return Step::Nothing;
        }
        let wait = self.last_ok.map_or(Duration::ZERO, |t| {
            (t + WINDOW).saturating_duration_since(now)
        });
        if force || wait.is_zero() {
            self.running = true;
            self.dirty = false;
            return Step::Run;
        }
        if self.scheduled {
            return Step::Nothing;
        }
        self.scheduled = true;
        Step::Wait(wait)
    }

    fn after_wait(&mut self) {
        self.scheduled = false;
    }

    /// A run ended. True when another is owed (a trigger arrived during it); the
    /// stamp moves only when the run succeeded.
    fn finished(&mut self, now: Instant, ok: bool) -> bool {
        if ok {
            self.last_ok = Some(now);
        }
        self.running = self.dirty;
        std::mem::take(&mut self.dirty)
    }
}

static THROTTLE: LazyLock<Mutex<Throttle>> = LazyLock::new(|| Mutex::new(Throttle::default()));

fn throttle<R>(f: impl FnOnce(&mut Throttle) -> R) -> R {
    f(&mut THROTTLE.lock().unwrap_or_else(|e| e.into_inner()))
}

/// Works out the wall and writes what it has lit, for the signed-in member, no
/// more often than the window allows (`force` skips the wait).
pub async fn grant_earned(member_id: &str, force: bool) {
    let mut step = throttle(|t| t.trigger(Instant::now(), force));
    loop {
        match step {
            Step::Nothing => return,
            Step::Wait(wait) => {
                tokio::time::sleep(wait).await;
                step = throttle(|t| {
                    t.after_wait();
                    t.trigger(Instant::now(), false)
                });
            }
            Step::Run => {
                let ok = run_once(member_id).await;
                if throttle(|t| t.finished(Instant::now(), ok)) {
                    step = Step::Run;
                } else {
                    return;
                }
            }
        }
    }
}

/// Something happened that may have lit an accolade for the signed-in member.
/// Nothing blocks on it.
pub fn poke(reason: Poke) {
    let Some(bind) = bind_now() else { return };
    debug!("[OwnAccolades] poked: {reason:?}");
    tauri::async_runtime::spawn(async move { grant_earned(&bind.member, false).await });
}

// ---- Client-attested accolades ----------------------------------------------------------

/// Whether an id is one only the client can attest.
pub fn is_attested(id: &str) -> bool {
    ATTESTED.contains(&id)
}

/// Saves one client-attested accolade for the signed-in member. True when it was
/// new. Refuses every id off the allowlist; the member is Rust's, never an argument.
pub async fn grant_attested(id: &str) -> Result<bool, String> {
    if !is_attested(id) {
        return Err(format!("{id} is not an accolade the app can attest"));
    }
    let member = AccountStore::primary()
        .map(|a| a.user_id)
        .filter(|m| !m.is_empty())
        .ok_or("not signed in")?;
    let inserted = backend::insert_ignore(
        "user_accolades",
        "twitch_user_id,accolade_id",
        accolade_row(&member, id),
    )
    .await?;
    if inserted {
        crate::services::profile_view::bump_member_epoch(&member);
        announce(&member);
    }
    Ok(inserted)
}

// ---- The clock --------------------------------------------------------------------------------

/// Whether the local date moved on since the last look.
fn date_changed(last: Option<NaiveDate>, today: NaiveDate) -> bool {
    last.is_some_and(|d| d != today)
}

/// Starts the clock: while signed in, every few minutes it looks at the local
/// date (a new day claims the day's login accolades again and pokes a run) and
/// every half hour pokes a run, which covers a plugin that claims drops over its
/// own connection and tells nobody.
pub fn start() {
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::SeqCst) || !backend::configured() {
        return;
    }
    tauri::async_runtime::spawn(async {
        let mut ticker = tokio::time::interval_at(tokio::time::Instant::now() + TICK, TICK);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let (mut last_day, mut looks) = (None::<NaiveDate>, 0u32);
        loop {
            ticker.tick().await;
            let Some(bind) = bind_now() else {
                last_day = None;
                continue;
            };
            looks += 1;
            let today = Local::now().date_naive();
            if date_changed(last_day, today) {
                match backend::rpc(
                    "claim_login_accolades",
                    json!({ "p_twitch_user_id": bind.member }),
                )
                .await
                {
                    Ok(v) => debug!("[OwnAccolades] a new day, accolades: {v}"),
                    Err(e) => warn!("[OwnAccolades] claim_login_accolades failed: {e}"),
                }
                poke(Poke::DateChanged);
            }
            last_day = Some(today);
            if looks % PERIODIC_EVERY == 0 {
                poke(Poke::Periodic);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::member_profile::{AccoladeMedal, Catalog, TEST_CATALOG_JSON};

    fn at(secs: u64) -> Instant {
        // A fixed origin far enough from the clock's zero to subtract from.
        static ORIGIN: LazyLock<Instant> = LazyLock::new(Instant::now);
        *ORIGIN + Duration::from_secs(secs)
    }

    // ---- The throttle (R5 F9) ----

    #[test]
    fn the_first_trigger_runs_and_one_during_a_run_runs_it_once_more() {
        let mut t = Throttle::default();
        assert_eq!(t.trigger(at(0), false), Step::Run);
        // Two triggers while it runs are one more run, not two.
        assert_eq!(t.trigger(at(1), false), Step::Nothing);
        assert_eq!(t.trigger(at(2), true), Step::Nothing);
        assert!(t.finished(at(3), true), "the dirty flag owes one more run");
        assert!(!t.finished(at(4), true), "and only one");
        // The stamp is the end of the run that succeeded.
        assert_eq!(t.last_ok, Some(at(4)));
    }

    #[test]
    fn a_trigger_inside_the_window_runs_once_at_its_end_and_a_forced_one_runs_now() {
        let mut t = Throttle::default();
        assert_eq!(t.trigger(at(0), false), Step::Run);
        assert!(!t.finished(at(1), true));
        assert_eq!(
            t.trigger(at(60), false),
            Step::Wait(WINDOW - Duration::from_secs(59))
        );
        // More triggers inside the window schedule nothing more.
        assert_eq!(t.trigger(at(120), false), Step::Nothing);
        assert_eq!(t.trigger(at(121), false), Step::Nothing);
        // The wait ends: the one scheduled trigger now runs.
        t.after_wait();
        assert_eq!(t.trigger(at(1) + WINDOW, false), Step::Run);
        assert!(!t.finished(at(2) + WINDOW, true));
        // A forced trigger (reading the wall) does not wait for the window.
        assert_eq!(t.trigger(at(3) + WINDOW, true), Step::Run);
    }

    #[test]
    fn a_run_that_failed_does_not_start_the_window() {
        let mut t = Throttle::default();
        assert_eq!(t.trigger(at(0), false), Step::Run);
        assert!(!t.finished(at(1), false));
        assert_eq!(t.last_ok, None);
        assert_eq!(
            t.trigger(at(2), false),
            Step::Run,
            "the next trigger tries again at once"
        );
    }

    // ---- The drops token (R5 H1) ----

    #[test]
    fn a_token_swapped_between_two_reads_never_borrows_the_other_tokens_account() {
        let mut cache = TokenUsers::default();
        cache.put("token-a", "111", at(0));
        assert_eq!(cache.get("token-a", at(1)).as_deref(), Some("111"));
        // The other token, read a moment later, is a different key: no answer, never "111".
        assert_eq!(cache.get("token-b", at(1)), None);
        cache.put("token-b", "222", at(2));
        assert_eq!(cache.get("token-a", at(3)).as_deref(), Some("111"));
        assert_eq!(cache.get("token-b", at(3)).as_deref(), Some("222"));
        // The key is a hash of the token, never the token.
        assert_ne!(token_key("token-a"), "token-a");
        assert_eq!(token_key("token-a").len(), 64);
        assert_ne!(token_key("token-a"), token_key("token-b"));
        // An answer is trusted for a while only.
        assert_eq!(
            cache.get("token-a", at(0) + VALIDATE_TTL + Duration::from_secs(1)),
            None
        );
    }

    #[test]
    fn a_drops_token_for_another_account_reads_nothing() {
        let member = "111";
        let bind = |primary: &str, owner: &str| WalkBind {
            bind: Bind {
                generation: 5,
                member: primary.to_string(),
            },
            owner: owner.to_string(),
        };
        assert!(account_matches("111", member, &bind("111", "111")));
        assert!(
            !account_matches("222", member, &bind("111", "111")),
            "the token is another account's"
        );
        assert!(
            !account_matches("111", member, &bind("333", "111")),
            "the primary account is someone else"
        );
        assert!(
            !account_matches("111", member, &bind("111", "333")),
            "the follow list is someone else's"
        );
        assert!(
            !account_matches("", "", &bind("", "")),
            "no member is no match"
        );
        assert_eq!(
            DropsPoints::unknown(),
            DropsPoints {
                drops: Part::Failed,
                points: Part::Failed
            }
        );
    }

    // ---- The walk (R5 G4, H2) ----

    fn bind5(member: &str, owner: &str, generation: u64) -> WalkBind {
        WalkBind {
            bind: Bind {
                generation,
                member: member.to_string(),
            },
            owner: owner.to_string(),
        }
    }

    #[test]
    fn a_walk_is_kept_only_when_the_account_held_and_every_batch_answered() {
        let start = bind5("111", "111", 5);
        assert!(walk_accepted(
            &start,
            Some(&bind5("111", "111", 5)),
            "111",
            0
        ));
        // The account changed while it ran.
        assert!(
            !walk_accepted(&start, Some(&bind5("111", "111", 6)), "111", 0),
            "a new account generation"
        );
        assert!(
            !walk_accepted(&start, Some(&bind5("222", "222", 5)), "111", 0),
            "a different primary"
        );
        assert!(
            !walk_accepted(&start, Some(&bind5("111", "222", 5)), "111", 0),
            "a different follow-list owner"
        );
        assert!(
            !walk_accepted(&start, None, "111", 0),
            "the account is gone"
        );
        // The token's account is not the walked account.
        assert!(!walk_accepted(&start, Some(&start), "999", 0));
        // A failed batch makes it incomplete.
        assert!(!walk_accepted(&start, Some(&start), "111", 1));
        // And an incomplete walk saves nothing.
        assert_eq!(walk_to_save(false, 5_000, "2026-11-05", 1), None);
        assert_eq!(
            walk_to_save(true, 5_000, "2026-11-05", 1),
            Some(Walked {
                total: 5_000,
                day: "2026-11-05".into(),
                at: 1
            })
        );
    }

    #[test]
    fn a_batch_with_an_item_that_came_back_with_errors_is_a_failed_batch() {
        let item = |balance: i64| json!({ "data": { "user": { "channel": { "self": { "communityPoints": { "balance": balance } } } } } });
        assert_eq!(
            sum_batch(&json!([item(100), item(0), item(-5), item(250)]), 4),
            Ok(350)
        );
        assert!(sum_batch(
            &json!([item(100), { "errors": [{ "message": "service timeout" }] }]),
            2
        )
        .is_err());
        assert!(
            sum_batch(&json!([item(100)]), 2).is_err(),
            "a short answer is not mapped by position"
        );
        assert!(sum_batch(&json!({ "error": "Bad Request" }), 1).is_err());
        // A channel that is gone answers no balance; that is not an error.
        assert_eq!(
            sum_batch(&json!([{ "data": { "user": null } }, item(7)]), 2),
            Ok(7)
        );
    }

    #[test]
    fn a_walk_is_due_once_a_day_and_an_incomplete_one_waits_before_trying_again() {
        let walked = |day: &str| Walked {
            total: 10,
            day: day.into(),
            at: 0,
        };
        assert!(walk_due(None, "2026-11-05", None, at(0)));
        assert!(
            !walk_due(Some(&walked("2026-11-05")), "2026-11-05", None, at(0)),
            "walked today"
        );
        assert!(
            walk_due(Some(&walked("2026-11-04")), "2026-11-05", None, at(0)),
            "a new day"
        );
        assert!(
            !walk_due(None, "2026-11-05", Some(at(100)), at(50)),
            "an incomplete walk waits"
        );
        assert!(walk_due(None, "2026-11-05", Some(at(100)), at(100)));
    }

    #[test]
    fn the_walked_total_and_the_last_wall_survive_a_restart() {
        let dir = std::env::temp_dir().join(format!("sn-own-accolades-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("points_walk.json");
        let mut map: HashMap<String, Walked> = HashMap::new();
        map.insert(
            "111".into(),
            Walked {
                total: 42,
                day: "2026-11-05".into(),
                at: 7,
            },
        );
        write_map(&path, &map);
        assert_eq!(read_map::<Walked>(&path), map);
        assert!(
            read_map::<Walked>(&dir.join("missing.json")).is_empty(),
            "no file reads as nothing walked"
        );
        std::fs::write(&path, b"{not json").unwrap();
        assert!(
            read_map::<Walked>(&path).is_empty(),
            "a damaged file reads as nothing walked"
        );
        let wall_path = dir.join("own_accolades.json");
        let wall = AccoladeWall {
            earned: 1,
            total: 1,
            items: vec![medal("og", true)],
        };
        let mut walls: HashMap<String, AccoladeWall> = HashMap::new();
        walls.insert("111".into(), wall.clone());
        write_map(&wall_path, &walls);
        assert_eq!(
            read_map::<AccoladeWall>(&wall_path).remove("111"),
            Some(wall)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_cached_wall_is_that_members_last_build_and_nobody_elses() {
        let dir = std::env::temp_dir().join(format!("sn-own-cached-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("own_accolades.json");
        assert_eq!(cached_from(&path, "111"), None, "no file, no wall");
        let first = AccoladeWall {
            earned: 1,
            total: 1,
            items: vec![medal("og", true)],
        };
        let second = AccoladeWall {
            earned: 0,
            total: 1,
            items: vec![medal("og", false)],
        };
        let mut walls: HashMap<String, AccoladeWall> = HashMap::new();
        walls.insert("111".into(), first.clone());
        walls.insert("222".into(), second.clone());
        write_map(&path, &walls);
        assert_eq!(cached_from(&path, "111"), Some(first));
        assert_eq!(cached_from(&path, "222"), Some(second));
        assert_eq!(cached_from(&path, "333"), None);
        std::fs::write(&path, b"{not json").unwrap();
        assert_eq!(
            cached_from(&path, "111"),
            None,
            "a damaged file reads as no wall"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- Drops claimed ----

    #[test]
    fn drops_claimed_is_the_desktops_sum_of_total_count() {
        let event = |id: Value, count: Value| json!({ "id": id, "name": "n", "imageURL": "u", "lastAwardedAt": "2026-01-01T00:00:00Z", "totalCount": count });
        let body = json!({ "data": { "currentUser": { "inventory": { "gameEventDrops": [
            event(json!("a"), json!(3)),
            event(json!("b"), json!(0)),
            event(json!("c"), Value::Null),
            event(Value::Null, json!(9)),
            { "id": "d", "name": "n", "imageURL": "u", "lastAwardedAt": "yesterday", "totalCount": 4 },
        ] } } } });
        // 3, then 0 and a missing count each count once, and the two malformed events not at all.
        assert_eq!(drops_claimed(&body), Some(5));
        assert_eq!(
            drops_claimed(&json!({ "data": { "currentUser": { "inventory": {} } } })),
            Some(0)
        );
        assert_eq!(
            drops_claimed(&json!({ "errors": [{ "message": "x" }] })),
            None
        );
        assert_eq!(
            drops_claimed(&json!({ "data": { "currentUser": null } })),
            None
        );
        assert_eq!(drops_claimed(&json!({})), None);
    }

    // ---- What is granted (R5 F9) ----

    fn medal(id: &str, earned: bool) -> AccoladeMedal {
        AccoladeMedal {
            id: id.into(),
            label: id.into(),
            icon: "x".into(),
            colors: ["#000000".into(), "#ffffff".into()],
            earned,
            secret: false,
            hint: String::new(),
        }
    }

    fn wall_of(earned: &[&str]) -> AccoladeWall {
        AccoladeWall {
            earned: earned.len() as u32,
            total: earned.len() as u32,
            items: earned.iter().map(|id| medal(id, true)).collect(),
        }
    }

    fn known_parts() -> OwnParts {
        OwnParts {
            member_id: "100".into(),
            facts: MemberFacts {
                member_id: "100".into(),
                number: Some(1000),
                stats: Part::Answered(None),
                saved: Part::Answered(HashSet::new()),
                ivr: Part::Answered(None),
                seventv: Part::Answered((0, 0)),
                ..Default::default()
            },
            drops: Part::Answered(30),
            points: Part::Answered(200_000),
        }
    }

    fn ids(v: Vec<String>) -> Vec<String> {
        v
    }

    #[test]
    fn what_the_numbers_have_lit_and_the_account_lacks_is_granted_once() {
        let cat = Catalog::parse(TEST_CATALOG_JSON).unwrap();
        let parts = known_parts();
        let none = HashSet::new();
        let wall = wall_of(&["chatterbox", "dropgoblin", "veteran", "triple"]);
        assert_eq!(
            ids(to_grant_with(&cat, &parts, &wall, &none)),
            vec!["chatterbox", "dropgoblin", "veteran", "triple"]
        );
        // What is saved is not written again.
        let saved: HashSet<String> = ["chatterbox".to_string(), "triple".to_string()]
            .into_iter()
            .collect();
        assert_eq!(
            ids(to_grant_with(&cat, &parts, &wall, &saved)),
            vec!["dropgoblin", "veteran"]
        );
        // A locked medal is not granted.
        let locked = AccoladeWall {
            earned: 0,
            total: 1,
            items: vec![medal("chatterbox", false)],
        };
        assert!(to_grant_with(&cat, &parts, &locked, &none).is_empty());
    }

    #[test]
    fn seasons_the_cake_day_events_the_offline_three_and_unknown_ids_never_reach_the_writer() {
        let cat = Catalog::parse(TEST_CATALOG_JSON).unwrap();
        // Lit on the wall (an own wall lights the season from today's date), yet not the numbers' to grant.
        let wall = wall_of(&[
            "halloween",
            "cs2_major_cologne_2026",
            "perfect_corner",
            "cake_day",
            "not-in-the-catalog",
        ]);
        assert!(to_grant_with(&cat, &known_parts(), &wall, &HashSet::new()).is_empty());
        // The shipped catalog agrees: every kind but base and secret is refused.
        for def in &CATALOG.accolades {
            let granted = !to_grant(
                &known_parts(),
                &wall_of(&[def.id.as_str()]),
                &HashSet::new(),
            )
            .is_empty();
            if !matches!(def.kind, Kind::Base | Kind::Secret) {
                assert!(!granted, "{} is a {:?} accolade", def.id, def.kind);
            }
        }
    }

    #[test]
    fn nothing_is_granted_from_an_input_that_never_answered() {
        let cat = Catalog::parse(TEST_CATALOG_JSON).unwrap();
        let wall = wall_of(&[
            "dropgoblin",
            "triple",
            "veteran",
            "collector",
            "chatterbox",
            "completionist",
        ]);
        // Drops unknown (no drops token for this account): not Drop Goblin, not Triple Threat.
        let mut parts = known_parts();
        parts.drops = Part::Failed;
        assert_eq!(
            ids(to_grant_with(&cat, &parts, &wall, &HashSet::new())),
            vec!["veteran", "collector", "chatterbox"]
        );
        // IVR down: not Veteran, and Completionist waits for every input.
        let mut parts = known_parts();
        parts.facts.ivr = Part::Failed;
        let granted = to_grant_with(&cat, &parts, &wall, &HashSet::new());
        assert!(
            !granted.contains(&"veteran".to_string())
                && !granted.contains(&"completionist".to_string())
        );
        // The 7TV inventory down: not Collector.
        let mut parts = known_parts();
        parts.facts.seventv = Part::Failed;
        assert!(
            !to_grant_with(&cat, &parts, &wall, &HashSet::new()).contains(&"collector".to_string())
        );
        // The stats or the saved set unread: nothing at all (a missing saved set could mean anything).
        let mut parts = known_parts();
        parts.facts.saved = Part::Failed;
        assert!(to_grant_with(&cat, &parts, &wall, &HashSet::new()).is_empty());
        let mut parts = known_parts();
        parts.facts.stats = Part::Failed;
        assert!(to_grant_with(&cat, &parts, &wall, &HashSet::new()).is_empty());
    }

    #[test]
    fn a_wall_built_from_unknown_drops_grants_nothing_a_drops_token_could_not_vouch_for() {
        // End to end through the wall: messages and hours are past their lines, drops are unknown.
        let cat = Catalog::parse(TEST_CATALOG_JSON).unwrap();
        let mut parts = known_parts();
        parts.drops = Part::Failed;
        parts.points = Part::Failed;
        let input = member_profile::WallInput {
            member_number: Some(1000),
            messages: 2000,
            hours: 150.0,
            today: "2026-11-05".into(),
            ..Default::default()
        };
        let wall = member_profile::wall_core(
            &cat,
            &input,
            &[StatKind::Drops, StatKind::Points],
            WallMode::Own,
        );
        assert!(wall.items.iter().all(|m| m.id != "triple" || !m.earned));
        let granted = to_grant_with(&cat, &parts, &wall, &HashSet::new());
        assert!(
            granted.contains(&"chatterbox".to_string())
                && granted.contains(&"marathoner".to_string())
        );
        assert!(
            !granted.contains(&"dropgoblin".to_string())
                && !granted.contains(&"triple".to_string())
        );
    }

    // ---- The writes ----

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[tokio::test]
    async fn the_account_changing_between_the_read_and_the_write_writes_nothing() {
        let writes = std::sync::Mutex::new(Vec::new());
        let out = write_all(
            &strings(&["a", "b"]),
            || false,
            |id| {
                writes.lock().unwrap().push(id);
                async { Ok(true) }
            },
        )
        .await;
        assert!(out.aborted && out.inserted.is_empty());
        assert!(
            writes.lock().unwrap().is_empty(),
            "not one request went out"
        );
    }

    #[tokio::test]
    async fn the_account_changing_mid_pass_stops_the_rest_and_a_refused_id_never_fails_the_others()
    {
        // The account holds for two checks, then changes.
        let checks = std::sync::atomic::AtomicUsize::new(0);
        let out = write_all(
            &strings(&["a", "b", "c"]),
            || checks.fetch_add(1, Ordering::SeqCst) < 2,
            |id| async move {
                if id == "a" {
                    Err("refused".to_string())
                } else {
                    Ok(true)
                }
            },
        )
        .await;
        assert_eq!(
            out,
            Written {
                inserted: strings(&["b"]),
                failed: 1,
                aborted: true
            }
        );
        // Everything accepted: one inserted, one already there.
        let out = write_all(
            &strings(&["a", "b"]),
            || true,
            |id| async move { Ok(id == "a") },
        )
        .await;
        assert_eq!(
            out,
            Written {
                inserted: strings(&["a"]),
                failed: 0,
                aborted: false
            }
        );
    }

    // ---- The client-attested ids ----

    #[tokio::test]
    async fn grant_own_accolade_refuses_every_id_off_the_allowlist() {
        for id in [
            "insomniac",
            "perfect_corner",
            "shiny_peepo",
            "worth_the_wait",
        ] {
            assert!(is_attested(id));
        }
        for id in [
            "",
            "dropgoblin",
            "dropdragon",
            "hoarder",
            "tycoon",
            "triple",
            "completionist",
            "og",
            "halloween",
            "cake_day",
            "INSOMNIAC",
            "insomniac ",
            "../x",
        ] {
            assert!(!is_attested(id), "{id:?}");
            let refused = grant_attested(id).await;
            assert!(
                refused.is_err(),
                "{id:?} must be refused before any request is made"
            );
        }
        // The allowlist is exactly the four (a change here is a review).
        assert_eq!(ATTESTED.len(), 4);
        // The shipped catalog names them, and none of them is something a number lights.
        for id in ATTESTED {
            let def = CATALOG
                .accolades
                .iter()
                .find(|d| d.id == id)
                .unwrap_or_else(|| panic!("{id} is in the catalog"));
            assert!(
                matches!(def.rule, RuleDef::Special(ref s) if s.special == Special::Saved),
                "{id}"
            );
        }
    }

    #[test]
    fn a_new_local_date_is_noticed_once() {
        let d = |m, day| NaiveDate::from_ymd_opt(2026, m, day).unwrap();
        assert!(
            !date_changed(None, d(11, 5)),
            "the first look only learns the date"
        );
        assert!(!date_changed(Some(d(11, 5)), d(11, 5)));
        assert!(date_changed(Some(d(11, 5)), d(11, 6)));
        assert!(date_changed(Some(d(12, 31)), d(1, 1)));
    }
}
