//! What the drops account can earn on the channel it is earning on right now:
//! the lead reward for chat's drops bar and the phones' card, and every
//! campaign that counts here with all its reward tiers, for the desktop
//! drawer. One model for every surface: the title-bar badge, the chat bar, the
//! Drops page's live minutes and the phones all read it.
//!
//! The place is the watch heartbeat's target (the main stream, or MultiNook's
//! focused Twitch tile) in the newest game seen for that channel. The
//! inventory is read here and nowhere else: the drops monitoring loop asks
//! `inventory_newer_than` for its auto-claim instead of reading it itself.
//! It reads once a minute while drops are being earned. `user-drop-events`
//! pushes, when the points socket subscribes to them, move minutes between
//! reads, and the next read reconciles them; the socket does not subscribe
//! today (Twitch sent none there), so the reads alone keep the bar moving.
//!
//! `watch-drops://changed` carries the whole model whenever it changes;
//! `get_watch_drops` answers the current one. Every value carries `rev`, so a
//! surface keeps only the newest it has seen.

use crate::models::drops::{DropCampaign, DropProgress, InventoryResponse, TimeBasedDrop};
use crate::models::settings::AppState;
use crate::rt::AppHandle;
use crate::services::drop_words;
use crate::services::drops_auth_service::DropsAuthService;
use crate::services::drops_service::{DropsService, UngrantedHandle};
use crate::services::watch_drop::WatchDrop;
use crate::services::watch_heartbeat_service::Earning;
use chrono::{DateTime, Utc};
use once_cell::sync::Lazy;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager};
use tokio::sync::{Notify, RwLock};

/// Sent with the whole model whenever it changes.
pub const WATCH_DROPS_CHANGED: &str = "watch-drops://changed";

/// Between reads while no push arrived lately.
const FAST: Duration = Duration::from_secs(60);
/// Between reads while pushes flow, and after three failures in a row.
const SLOW: Duration = Duration::from_secs(300);
/// A push this recent means Twitch is pushing.
const PUSH_FRESH: Duration = Duration::from_secs(150);
/// Twitch's second drops list is reused for a read when this fresh.
const SECOND_LIST_MAX_AGE: u64 = 60;
/// The cached campaign list is fetched again past this age.
const CAMPAIGNS_MAX_AGE_SECS: i64 = 900;
/// The campaign clock flat this long while playing is a stall.
const STALL_AFTER: Duration = Duration::from_secs(240);
/// A stall still flat this long after its episode began starts the next one.
const REARM_AFTER: Duration = Duration::from_secs(240);
/// A read settles a claim made at least this long before it started.
pub const CLAIM_SETTLE_SECS: i64 = 30;
/// How long `get_watch_drop` waits for the first answer at a new place.
const VERDICT_WAIT: Duration = Duration::from_secs(5);
/// Channels whose last seen game is kept.
const GAMES_CAP: usize = 8;

// ---- The contract ------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct WatchDropsSnapshot {
    pub rev: u64,
    pub view: WatchDrops,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct WatchDrops {
    /// Empty: no place (nothing earning, or signed out of Drops).
    pub channel_id: String,
    pub login: String,
    pub game_id: String,
    pub game_name: String,
    /// The reward the bar shows.
    pub lead: Option<WatchDrop>,
    /// Other rewards not yet claimed here besides the lead ("+N").
    pub more: u32,
    /// Pictures of the first of those others (at most two, campaign order),
    /// stacked on the bar in place of a bare "+N".
    pub more_art: Vec<String>,
    /// Started first (fewest minutes left), then not started, by name.
    pub campaigns: Vec<HereCampaign>,
    /// A campaign counts here and every watch-time reward here is earned.
    pub complete: bool,
    pub stalled: bool,
    pub stall_episode: u64,
    /// The viewer tucked the lip away: chat draws a thin progress line on the
    /// message box instead. Cleared on its own by anything worth showing (see
    /// `tuck_holds`).
    pub lip_hidden: bool,
}

/// The lip tucked away, and what it was showing then.
#[derive(Debug, Clone, PartialEq)]
struct Tucked {
    campaign_id: String,
    drop_id: String,
    /// The channel's id, else its login (lowercase).
    channel: String,
}

fn channel_key(view: &WatchDrops) -> String {
    if view.channel_id.is_empty() {
        view.login.to_lowercase()
    } else {
        view.channel_id.clone()
    }
}

/// The lip stays tucked only while nothing new is worth showing: the same
/// reward on the same channel, still just earning. A reward ready to claim,
/// one waiting on a game account link, the next reward starting, or another
/// channel's drops all bring it back; minutes ticking up never do.
fn tuck_holds(t: &Tucked, view: &WatchDrops) -> bool {
    let Some(lead) = &view.lead else {
        return false;
    };
    let waits_on_link = view
        .campaigns
        .iter()
        .find(|c| c.id == lead.campaign_id)
        .and_then(|c| c.drops.iter().find(|d| d.id == lead.drop_id))
        .is_some_and(|d| d.connect);
    lead.campaign_id == t.campaign_id
        && lead.drop_id == t.drop_id
        && channel_key(view) == t.channel
        && !lead.complete
        && !waits_on_link
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct HereCampaign {
    pub id: String,
    pub name: String,
    pub game_name: String,
    pub art: String,
    /// The game's box art, sized; empty for a campaign with no category.
    pub box_art: String,
    /// "3d left"; empty when unknown.
    pub ends: String,
    /// "2 of 4 claimed"; empty when not started.
    pub claimed_label: String,
    pub started: bool,
    /// The game's page to link the account, for a tier's Connect.
    pub link_url: Option<String>,
    /// The campaign clock (a per-reward campaign: its furthest reward).
    pub cumulative_minutes: i32,
    /// Watch-time rewards, fewest required minutes first.
    pub drops: Vec<HereDrop>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct HereDrop {
    pub id: String,
    pub instance_id: Option<String>,
    pub name: String,
    pub image: String,
    pub claimed: bool,
    pub ready: bool,
    pub pending: bool,
    /// Its minutes are in but the game account is not linked (Twitch's own
    /// Inventory offers Connect here too): Connect opens the campaign's
    /// `link_url` in place of a Claim Twitch would not grant.
    pub connect: bool,
    /// Its minutes are in but Twitch has not granted the claim and there is
    /// no page to link an account (the auto-claim tries again later).
    pub watched: bool,
    /// Watching cannot earn it: it needs an earlier reward first (an
    /// evolution stage whose earlier form is not held). Never leads.
    pub locked: bool,
    pub lead: bool,
    /// "Claimed", "Ready", "Claiming", "Connect", "Watched", "Locked",
    /// "12m to watch" or "Starts when you watch".
    pub status: String,
    pub detail: Option<String>,
    pub bar: Option<f32>,
    pub current_minutes: i32,
    pub required_minutes: i32,
    pub claim_label: String,
}

// ---- Place -------------------------------------------------------------------

/// Where the account earns: a channel, in a game.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Place {
    pub channel_id: String,
    pub login: String,
    pub game_id: String,
    pub game_name: String,
}

/// The same game: by id when both sides have one, else by name.
pub fn same_game(a_id: &str, a_name: &str, b_id: &str, b_name: &str) -> bool {
    if !a_id.is_empty() && !b_id.is_empty() {
        return a_id == b_id;
    }
    let a = a_name.trim();
    !a.is_empty() && a.eq_ignore_ascii_case(b_name.trim())
}

fn same_place(a: &Place, b: &Place) -> bool {
    let no_game = |p: &Place| p.game_id.is_empty() && p.game_name.is_empty();
    a.channel_id == b.channel_id
        && ((no_game(a) && no_game(b))
            || same_game(&a.game_id, &a.game_name, &b.game_id, &b.game_name))
}

/// A caller's channel and game are the model's place: the channel by id when
/// both have one, else by login; the game as `same_game`.
pub fn place_matches(
    place: &Place,
    channel_id: &str,
    login: &str,
    game_id: &str,
    game_name: &str,
) -> bool {
    let channel = if !channel_id.is_empty() && !place.channel_id.is_empty() {
        channel_id == place.channel_id
    } else {
        !login.is_empty() && login.eq_ignore_ascii_case(&place.login)
    };
    channel && same_game(&place.game_id, &place.game_name, game_id, game_name)
}

/// The last game seen for a channel, with when it was asked for or arrived.
#[derive(Debug, Clone)]
pub struct SeenGame {
    pub game_id: String,
    pub game_name: String,
    pub at: Instant,
}

static GAMES: Lazy<Mutex<HashMap<String, SeenGame>>> = Lazy::new(|| Mutex::new(HashMap::new()));

/// The earning place: the heartbeat's channel in the newest game seen for it.
pub fn place_from(earning: &Earning, seen: Option<&SeenGame>) -> Place {
    let (game_id, game_name) = match seen {
        Some(g) => (g.game_id.clone(), g.game_name.clone()),
        None => (earning.game_id.clone(), earning.game_name.clone()),
    };
    Place {
        channel_id: earning.channel_id.clone(),
        login: earning.login.clone(),
        game_id,
        game_name,
    }
}

/// A game from a row of unknown age (the row a stream was started from, which
/// can be a cached list minutes old): kept only while nothing is known for the
/// channel, and stamped old, so stream info asked for or an update that
/// arrives at any time replaces it.
pub fn seed_game(channel_id: &str, game_id: &str, game_name: &str) {
    let known = GAMES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains_key(channel_id);
    if known {
        return;
    }
    let old = Instant::now()
        .checked_sub(Duration::from_secs(3600))
        .unwrap_or_else(Instant::now);
    saw_game(channel_id, game_id, game_name, old);
}

/// A game seen for `channel_id` at `at` (a request's start, or an event's
/// arrival). Newer than what is kept, it replaces it; a different game wakes
/// the watcher and reaches the heartbeat's minute payload.
pub fn saw_game(channel_id: &str, game_id: &str, game_name: &str, at: Instant) {
    if channel_id.is_empty() || (game_id.is_empty() && game_name.is_empty()) {
        return;
    }
    let changed = {
        let mut games = GAMES.lock().unwrap_or_else(|e| e.into_inner());
        let changed = match games.get_mut(channel_id) {
            Some(kept) if kept.at >= at => {
                // Older news; it may still know the id of the same game.
                if kept.game_id.is_empty() && same_game("", &kept.game_name, game_id, game_name) {
                    kept.game_id = game_id.to_string();
                }
                false
            }
            Some(kept) => {
                let changed = !same_game(&kept.game_id, &kept.game_name, game_id, game_name);
                let keep_id = if game_id.is_empty() && !changed {
                    kept.game_id.clone()
                } else {
                    game_id.to_string()
                };
                *kept = SeenGame {
                    game_id: keep_id,
                    game_name: game_name.to_string(),
                    at,
                };
                changed
            }
            None => {
                games.insert(
                    channel_id.to_string(),
                    SeenGame {
                        game_id: game_id.to_string(),
                        game_name: game_name.to_string(),
                        at,
                    },
                );
                true
            }
        };
        if games.len() > GAMES_CAP {
            let oldest = games
                .iter()
                .min_by_key(|(_, g)| g.at)
                .map(|(k, _)| k.clone());
            if let Some(k) = oldest.filter(|k| k != channel_id) {
                games.remove(&k);
            }
        }
        changed
    };
    if !changed {
        return;
    }
    poke();
    if let Some(app) = APP.get() {
        let heartbeat = app.state::<AppState>().watch_heartbeat.clone();
        let (channel, id, name) = (
            channel_id.to_string(),
            game_id.to_string(),
            game_name.to_string(),
        );
        tauri::async_runtime::spawn(async move { heartbeat.set_game(&channel, id, name).await });
    }
}

/// The game a stream-info read asked for at `asked_at` should report: what it
/// fetched, unless a different game was seen after it was asked.
pub fn newest_game(
    channel_id: &str,
    asked_at: Instant,
    fetched_id: String,
    fetched_name: String,
) -> (String, String) {
    let games = GAMES.lock().unwrap_or_else(|e| e.into_inner());
    match games.get(channel_id) {
        Some(g)
            if g.at > asked_at
                && !same_game(&g.game_id, &g.game_name, &fetched_id, &fetched_name) =>
        {
            (g.game_id.clone(), g.game_name.clone())
        }
        _ => (fetched_id, fetched_name),
    }
}

fn game_for(channel_id: &str) -> Option<SeenGame> {
    GAMES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(channel_id)
        .cloned()
}

// ---- The picker --------------------------------------------------------------

/// How a claim stands while Twitch's inventory catches up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimState {
    Pending,
    Claimed,
}

/// Everything one build reads. Pure, so tests drive it with fixtures.
pub struct Inputs<'a> {
    pub place: &'a Place,
    /// The campaign list (Twitch's two lists joined).
    pub list: &'a [DropCampaign],
    pub inventory: Option<&'a InventoryResponse>,
    /// Pushed minutes by drop id, with when they arrived.
    pub overlay: &'a HashMap<String, (i32, DateTime<Utc>)>,
    /// When the inventory read started: pushes from before it yield to it.
    pub read_started_at: Option<DateTime<Utc>>,
    pub instances: &'a HashMap<String, String>,
    pub claims: &'a HashMap<String, ClaimState>,
    pub ungranted: &'a HashSet<String>,
    /// Rewards watching cannot earn yet, by drop id, with the badge each
    /// needs first (`evolution::locked_stages`).
    pub locked: &'a HashMap<String, String>,
    /// The reward the lip showed last, so a tie never trades it away.
    pub held_lead: Option<&'a str>,
    pub now_ms: i64,
}

/// Minutes a reward on the lip may trail the closest one and still keep it:
/// rewards finishing together have counters that reach the app a moment
/// apart, and the lip must not trade places each time one lands.
const LEAD_SLACK_MINUTES: i32 = 1;

static NO_OVERLAY: Lazy<HashMap<String, (i32, DateTime<Utc>)>> = Lazy::new(HashMap::new);
static NO_INSTANCES: Lazy<HashMap<String, String>> = Lazy::new(HashMap::new);
static NO_CLAIMS: Lazy<HashMap<String, ClaimState>> = Lazy::new(HashMap::new);
static NO_UNGRANTED: Lazy<HashSet<String>> = Lazy::new(HashSet::new);
static NO_LOCKED: Lazy<HashMap<String, String>> = Lazy::new(HashMap::new);

impl<'a> Inputs<'a> {
    /// A place, the list and an inventory only (the phones' own read).
    pub fn bare(
        place: &'a Place,
        list: &'a [DropCampaign],
        inventory: Option<&'a InventoryResponse>,
        now_ms: i64,
    ) -> Self {
        Inputs {
            place,
            list,
            inventory,
            overlay: &NO_OVERLAY,
            read_started_at: None,
            instances: &NO_INSTANCES,
            claims: &NO_CLAIMS,
            ungranted: &NO_UNGRANTED,
            locked: &NO_LOCKED,
            held_lead: None,
            now_ms,
        }
    }
}

fn norm(name: &str) -> String {
    name.trim().to_lowercase()
}

/// Rewards the account already holds: by drop id, benefit id, or exact name.
#[derive(Default)]
struct Owned {
    benefit_ids: HashSet<String>,
    names: HashSet<String>,
    drop_ids: HashSet<String>,
}

impl Owned {
    fn from(inventory: Option<&InventoryResponse>) -> Owned {
        let mut owned = Owned::default();
        let Some(inv) = inventory else { return owned };
        for d in &inv.completed_drops {
            owned.benefit_ids.insert(d.id.clone());
            let n = norm(&d.name);
            if !n.is_empty() {
                owned.names.insert(n);
            }
        }
        for item in &inv.items {
            for d in &item.campaign.time_based_drops {
                if !d.progress.as_ref().is_some_and(|p| p.is_claimed) {
                    continue;
                }
                owned.drop_ids.insert(d.id.clone());
                for b in &d.benefit_edges {
                    owned.benefit_ids.insert(b.id.clone());
                    let n = norm(&b.name);
                    if !n.is_empty() {
                        owned.names.insert(n);
                    }
                }
            }
        }
        owned
    }

    /// Claimed here; otherwise, with no live minutes, already in the
    /// collection (Twitch reissues a campaign under new ids while the reward
    /// stays owned). A reward with live minutes is a fresh one, never owned.
    fn owns(&self, d: &TimeBasedDrop, have: i32, claimed: bool) -> bool {
        if claimed {
            return true;
        }
        if have > 0 {
            return false;
        }
        self.drop_ids.contains(&d.id)
            || d.benefit_edges.iter().any(|b| {
                let n = norm(&b.name);
                self.benefit_ids.contains(&b.id) || (!n.is_empty() && self.names.contains(&n))
            })
    }
}

fn accruing(c: &DropCampaign) -> bool {
    c.time_based_drops.iter().any(|d| {
        d.progress
            .as_ref()
            .and_then(|p| p.twitch_progress.as_ref())
            .is_some_and(|t| t.accruing)
    })
}

/// Watching this channel credits the campaign: earnable on it, and in its
/// game or (a drop running across categories) earning right now.
fn counts_here(c: &DropCampaign, place: &Place) -> bool {
    c.earnable_on(&place.channel_id, &place.login)
        && (c.counts_in(&place.game_id, &place.game_name) || accruing(c))
}

/// Some campaign can be earned here, so the inventory is worth reading.
pub fn gate_open(place: &Place, list: &[DropCampaign], second: &[DropCampaign]) -> bool {
    if (place.channel_id.is_empty() && place.login.is_empty()) || (place.game_id.is_empty() && place.game_name.is_empty()) {
        return false;
    }
    list.iter().chain(second).any(|c| counts_here(c, place))
}

struct Row<'a> {
    d: &'a TimeBasedDrop,
    need: i32,
    shown: i32,
    owned: bool,
    ready: bool,
    pending: bool,
    connect: bool,
    watched: bool,
    /// The badge it needs first, when watching cannot earn it yet.
    locked: Option<String>,
}

/// A campaign's watch-time rewards, fewest minutes first, and its clock.
/// `link`: whether the campaign has a page to link the game account;
/// `needs_link`: Twitch says the account is not linked (and there is a page).
fn rows<'a>(
    c: &'a DropCampaign,
    started: bool,
    link: bool,
    needs_link: bool,
    i: &Inputs,
    owned: &Owned,
) -> (Vec<Row<'a>>, i32) {
    let mut tiers: Vec<&TimeBasedDrop> = c
        .time_based_drops
        .iter()
        .filter(|d| d.required_minutes_watched > 0)
        .collect();
    tiers.sort_by_key(|d| d.required_minutes_watched);
    let minutes = |d: &TimeBasedDrop| -> i32 {
        if !started {
            return 0;
        }
        let base = d
            .progress
            .as_ref()
            .map(|p| p.current_minutes_watched)
            .unwrap_or(0);
        let pushed = i
            .overlay
            .get(&d.id)
            .filter(|(_, at)| i.read_started_at.map_or(true, |read| *at > read))
            .map(|(m, _)| *m);
        pushed.unwrap_or(base).clamp(0, d.required_minutes_watched)
    };
    // Twitch zeroes a claimed reward's own counter and keeps one running count
    // per campaign, unless each reward keeps its own.
    let per_drop = c.separate_progress
        || tiers.iter().any(|d| {
            d.progress
                .as_ref()
                .is_some_and(|p| p.twitch_progress.is_some())
        });
    let base: Vec<(i32, bool)> = tiers
        .iter()
        .map(|d| {
            let have = minutes(d);
            let claimed = d.progress.as_ref().is_some_and(|p| p.is_claimed)
                || i.claims.get(&d.id) == Some(&ClaimState::Claimed);
            (have, owned.owns(d, have, claimed))
        })
        .collect();
    let clock = tiers
        .iter()
        .zip(&base)
        .map(|(d, (have, own))| {
            if *own && !per_drop {
                (*have).max(d.required_minutes_watched)
            } else {
                *have
            }
        })
        .max()
        .unwrap_or(0);
    let rows = tiers
        .iter()
        .zip(base)
        .map(|(d, (have, own))| {
            let need = d.required_minutes_watched;
            let shown = if own {
                need
            } else if per_drop {
                have
            } else {
                clock.min(need)
            };
            let twitch_ready = d
                .progress
                .as_ref()
                .and_then(|p| p.twitch_progress.as_ref())
                .is_some_and(|t| t.ready_to_claim);
            let pending = !own && i.claims.get(&d.id) == Some(&ClaimState::Pending);
            // One Twitch already finished is claimed whatever it needed.
            let locked = i
                .locked
                .get(&d.id)
                .filter(|_| !own && !pending && !twitch_ready)
                .cloned();
            let earned = locked.is_none() && started && (shown >= need || twitch_ready);
            let ungranted = i.ungranted.contains(&d.id);
            // Twitch will not grant it until the game account is linked: one
            // whose minutes are in while Twitch says the account is not
            // linked, or one it already took but did not grant.
            let connect = !own && !pending && locked.is_none() && link && (ungranted || (needs_link && earned));
            // Not granted, with no page to link anything; the auto-claim
            // tries it again later.
            let watched = !own && !pending && !connect && locked.is_none() && ungranted;
            // A claim already sent, or one Twitch will not grant, never
            // offers Claim.
            let ready = !own && !pending && !connect && !watched && earned;
            Row {
                d,
                need,
                shown,
                owned: own,
                ready,
                pending,
                connect,
                watched,
                locked,
            }
        })
        .collect();
    (rows, clock)
}

impl Row<'_> {
    /// Its minutes are in: ready to claim, being claimed, waiting on a link,
    /// or not granted.
    fn waits(&self) -> bool {
        self.ready || self.pending || self.connect || self.watched
    }

    /// The lip's order, then minutes left: one to claim, then one still
    /// earning, then one the lip only shows when nothing else is left (a
    /// claim in flight, one waiting on a link, or not granted), so a finished
    /// reward never parks the lip while another is making progress.
    fn rank(&self) -> (u8, i32) {
        let kind = if self.ready {
            0
        } else if self.pending || self.connect || self.watched {
            2
        } else {
            1
        };
        (kind, self.need - self.shown)
    }
}

fn instance_of(d: &TimeBasedDrop, i: &Inputs) -> Option<String> {
    i.instances
        .get(&d.id)
        .cloned()
        .or_else(|| d.progress.as_ref().and_then(|p| p.drop_instance_id.clone()))
}

fn here_drop(r: &Row, i: &Inputs, started: bool, lead: bool) -> HereDrop {
    let d = r.d;
    let twitch = d.progress.as_ref().and_then(|p| p.twitch_progress.as_ref());
    // What it turned into once Twitch says; an unopened draw's first reward is its container.
    let benefit = twitch
        .and_then(|t| t.earned.as_ref())
        .or(d.next_reward.as_ref())
        .or(d.benefit_edges.first());
    let status = if r.owned {
        "Claimed".to_string()
    } else if r.pending {
        "Claiming".to_string()
    } else if r.connect {
        "Connect".to_string()
    } else if r.watched {
        "Watched".to_string()
    } else if r.ready {
        "Ready".to_string()
    } else if r.locked.is_some() {
        "Locked".to_string()
    } else if !started {
        "Starts when you watch".to_string()
    } else {
        drop_words::to_watch(r.need - r.shown)
    };
    HereDrop {
        id: d.id.clone(),
        instance_id: instance_of(d, i),
        name: benefit
            .map(|b| b.name.clone())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| {
                if d.name.is_empty() {
                    "Drop".to_string()
                } else {
                    d.name.clone()
                }
            }),
        image: benefit.map(|b| b.image_url.clone()).unwrap_or_default(),
        claimed: r.owned,
        ready: r.ready,
        pending: r.pending,
        connect: r.connect,
        watched: r.watched,
        locked: r.locked.is_some(),
        lead,
        status,
        detail: if r.owned {
            None
        } else if let Some(needs) = &r.locked {
            Some(crate::services::evolution::locked_words(needs))
        } else {
            drop_words::progress_line(d, i.now_ms).or_else(|| drop_words::requirement_text(d))
        },
        // Minutes Twitch shows on a locked reward go nowhere, so no bar.
        bar: r.locked.is_none().then_some(if r.owned {
            100.0
        } else {
            (r.shown as f32 / r.need as f32 * 100.0).min(100.0)
        }),
        current_minutes: r.shown,
        required_minutes: r.need,
        claim_label: drop_words::claim_label(d).to_string(),
    }
}

/// The phones' and the bar's lead, worded as `get_watch_drop` always was.
fn lead_drop(c: &DropCampaign, rows: &[Row], at: usize, i: &Inputs) -> WatchDrop {
    let r = &rows[at];
    let d = r.d;
    let line = if rows.len() > 1 {
        format!("{} · {} of {}", c.name, at + 1, rows.len())
    } else {
        c.name.clone()
    };
    let benefit = d.benefit_edges.first();
    let name = benefit
        .map(|b| b.name.clone())
        .filter(|n| !n.is_empty())
        .or_else(|| Some(d.name.clone()).filter(|n| !n.is_empty()))
        .unwrap_or_else(|| "Drop".to_string());
    let left = r.need - r.shown;
    let status = if r.pending {
        "Claiming".to_string()
    } else if r.connect {
        "Connect".to_string()
    } else if r.watched {
        "Watched".to_string()
    } else if r.ready {
        "Ready".to_string()
    } else {
        drop_words::to_watch(left)
    };
    WatchDrop {
        campaign_id: c.id.clone(),
        drop_id: d.id.clone(),
        instance_id: instance_of(d, i),
        // A finished drop is said once, by its Claim button.
        trailing: if r.ready { String::new() } else { status.clone() },
        spoken: if r.pending {
            format!("{name}, claiming")
        } else if r.connect {
            format!("{name}, link your game account to claim")
        } else if r.watched {
            format!("{name}, watched, not granted yet")
        } else if r.ready {
            format!("{name}, ready to claim")
        } else {
            format!(
                "{name}, {left} minute{} left",
                if left == 1 { "" } else { "s" }
            )
        },
        status,
        name,
        image: benefit.map(|b| b.image_url.clone()).unwrap_or_default(),
        line,
        percent: if r.waits() {
            100.0
        } else {
            r.shown as f32 / r.need as f32 * 100.0
        },
        complete: r.ready,
        claim_label: drop_words::claim_label(d).into(),
    }
}

/// The model for one place. Campaigns are joined by id only: two campaigns of
/// one game never lend each other minutes.
pub fn build(i: &Inputs) -> WatchDrops {
    let p = i.place;
    let mut view = WatchDrops {
        channel_id: p.channel_id.clone(),
        login: p.login.clone(),
        game_id: p.game_id.clone(),
        game_name: p.game_name.clone(),
        ..Default::default()
    };
    // A channel is known by its id, else its login (a pane still resolving its id).
    if (p.channel_id.is_empty() && p.login.is_empty()) || (p.game_id.is_empty() && p.game_name.is_empty()) {
        return view;
    }
    let owned = Owned::from(i.inventory);
    let mut seen = HashSet::new();
    let mut found: Vec<(&DropCampaign, bool)> = Vec::new();
    for item in i.inventory.map(|inv| inv.items.as_slice()).unwrap_or(&[]) {
        if counts_here(&item.campaign, p) && seen.insert(item.campaign.id.clone()) {
            found.push((&item.campaign, true));
        }
    }
    for c in i.list {
        if c.earnable_on(&p.channel_id, &p.login)
            && c.counts_in(&p.game_id, &p.game_name)
            && seen.insert(c.id.clone())
        {
            found.push((c, false));
        }
    }
    struct Built<'a> {
        c: &'a DropCampaign,
        started: bool,
        rows: Vec<Row<'a>>,
        clock: i32,
        link: Option<String>,
    }
    let mut built: Vec<Built> = found
        .into_iter()
        .filter_map(|(c, started)| {
            // The Inventory's copy of a campaign carries no account link; the
            // campaign list's copy does, so Connect still has somewhere to go.
            let link = c
                .account_link
                .clone()
                .filter(|u| !u.is_empty())
                .or_else(|| {
                    i.list
                        .iter()
                        .find(|l| l.id == c.id)
                        .and_then(|l| l.account_link.clone())
                        .filter(|u| !u.is_empty())
                });
            // Twitch's own flag, the one its Inventory draws Connect from. A
            // badge campaign also reports no linked account, but has no link
            // to follow, so it never asks for one.
            let needs_link = !c.is_account_connected && link.is_some();
            let (rows, clock) = rows(c, started, link.is_some(), needs_link, i, &owned);
            if rows.is_empty() {
                return None;
            }
            Some(Built {
                c,
                started,
                rows,
                clock,
                link,
            })
        })
        .collect();
    // A locked reward is nothing to earn here: it never leads, counts or sorts.
    let work = |b: &Built| -> (u8, i32) {
        b.rows
            .iter()
            .filter(|r| !r.owned && r.locked.is_none())
            .map(Row::rank)
            .min()
            .unwrap_or((u8::MAX, i32::MAX))
    };
    built.sort_by(|a, b| {
        (!a.started, work(a), a.c.name.to_lowercase()).cmp(&(
            !b.started,
            work(b),
            b.c.name.to_lowercase(),
        ))
    });
    // The lip's reward, by `Row::rank`: one to claim, else the one still
    // earning closest to done, ties to the first in drawer order. The reward
    // already on the lip keeps it while it is the same kind and within the
    // slack of the closest, so rewards finishing together never trade places.
    let candidates: Vec<((usize, usize), (u8, i32))> = built
        .iter()
        .enumerate()
        .filter(|(_, b)| b.started)
        .flat_map(|(bi, b)| {
            b.rows
                .iter()
                .enumerate()
                .filter(|(_, r)| !r.owned && r.locked.is_none())
                .map(move |(ri, r)| ((bi, ri), r.rank()))
        })
        .collect();
    let lead_at: Option<(usize, usize)> = candidates
        .iter()
        .map(|(_, key)| *key)
        .min()
        .and_then(|(kind, left)| {
            let held = i.held_lead.and_then(|id| {
                candidates.iter().find(|((bi, ri), (k, l))| {
                    built[*bi].rows[*ri].d.id == id && *k == kind && *l <= left + LEAD_SLACK_MINUTES
                })
            });
            held.or_else(|| candidates.iter().find(|(_, key)| *key == (kind, left)))
                .map(|(at, _)| *at)
        });
    view.lead = lead_at.map(|(bi, ri)| lead_drop(built[bi].c, &built[bi].rows, ri, i));
    let unowned: u32 = built
        .iter()
        .map(|b| b.rows.iter().filter(|r| !r.owned && r.locked.is_none()).count() as u32)
        .sum();
    let locked = built.iter().any(|b| b.rows.iter().any(|r| !r.owned && r.locked.is_some()));
    view.more = unowned.saturating_sub(view.lead.is_some() as u32);
    // Only what is left locked is not everything claimed.
    view.complete = !built.is_empty() && unowned == 0 && !locked;
    view.campaigns = built
        .iter()
        .enumerate()
        .map(|(bi, b)| {
            let owned_count = b.rows.iter().filter(|r| r.owned).count();
            HereCampaign {
                id: b.c.id.clone(),
                name: b.c.name.clone(),
                game_name: b.c.game_name.clone(),
                art: b
                    .rows
                    .first()
                    .and_then(|r| r.d.benefit_edges.first())
                    .map(|x| x.image_url.clone())
                    .filter(|u| !u.is_empty())
                    .unwrap_or_else(|| b.c.image_url.clone()),
                box_art: if b.c.has_category {
                    sized_box_art(&b.c.image_url)
                } else {
                    String::new()
                },
                ends: crate::services::rewards_drops::ends_in(
                    b.c.end_at.timestamp_millis(),
                    i.now_ms,
                )
                .unwrap_or_default(),
                claimed_label: if b.started {
                    format!("{owned_count} of {} claimed", b.rows.len())
                } else {
                    String::new()
                },
                started: b.started,
                link_url: b.link.clone(),
                cumulative_minutes: b.clock,
                drops: b
                    .rows
                    .iter()
                    .enumerate()
                    .map(|(ri, r)| here_drop(r, i, b.started, lead_at == Some((bi, ri))))
                    .collect(),
            }
        })
        .collect();
    view.more_art = more_art(&view.campaigns);
    view
}

/// The bar's stacked pictures: the first two rewards still to earn here
/// besides the lead, each with the picture its own drawer row shows.
fn more_art(campaigns: &[HereCampaign]) -> Vec<String> {
    campaigns
        .iter()
        .flat_map(|c| &c.drops)
        .filter(|d| !d.claimed && !d.lead && !d.locked && !d.image.is_empty())
        .map(|d| d.image.clone())
        .take(2)
        .collect()
}

/// Box art at the drawer's size: Twitch sends some URLs with `{width}x{height}`
/// left for the caller (the Inventory's own 144x192, already cached).
fn sized_box_art(url: &str) -> String {
    url.replace("{width}", "144").replace("{height}", "192")
}

/// The phones' lead: never a campaign not started.
pub fn lead_for(view: &WatchDrops) -> Option<WatchDrop> {
    view.lead.clone()
}

// ---- Cadence and the stall clock --------------------------------------------

/// The wait before the next read: short while Twitch is not pushing, long
/// while it is and after three failures in a row.
/// Whether the watcher reads now. A wanted read (a place change, a claim, an
/// unknown drop in a push) goes at once; otherwise a read is due when the last
/// answer is older than the cadence AND the last attempt is too, so failures
/// back off from when they were tried rather than retrying every step.
pub fn read_due(
    gate: bool,
    wanted: bool,
    have_snapshot: bool,
    playing: bool,
    answer_age: Option<Duration>,
    since_attempt: Option<Duration>,
    cadence: Duration,
) -> bool {
    if !gate {
        return false;
    }
    if wanted {
        return true;
    }
    let stale = !have_snapshot || (playing && answer_age.map_or(true, |a| a >= cadence));
    stale && since_attempt.map_or(true, |t| t >= cadence)
}

pub fn next_read_after(last_push_age: Option<Duration>, failures: u32) -> Duration {
    if failures >= 3 {
        return SLOW;
    }
    match last_push_age {
        Some(age) if age < PUSH_FRESH => SLOW,
        _ => FAST,
    }
}

/// What the stall verdict reads, sampled on each rebuild.
pub struct StallInputs {
    /// The earning channel is the solo stream with no grid over it.
    pub solo: bool,
    pub playing: bool,
    pub lead_campaign: Option<String>,
    /// The lead still has minutes to earn.
    pub lead_below: bool,
    pub cumulative: i32,
}

/// The farmed stream's clock: flat for `STALL_AFTER` while it plays is a stall,
/// counted in episodes so a surface acts once per episode.
#[derive(Debug, Default, Clone)]
pub struct StallClock {
    campaign: Option<String>,
    last_cum: Option<i32>,
    advanced: bool,
    last_advance: Option<Instant>,
    stalled_since: Option<Instant>,
    pub episode: u64,
}

impl StallClock {
    /// A new place: watch it afresh. Episodes keep counting.
    pub fn reset(&mut self) {
        *self = StallClock {
            episode: self.episode,
            ..Default::default()
        };
    }

    /// Playback resumed: a pause is never a stall.
    pub fn play_edge(&mut self, now: Instant) {
        self.last_advance = Some(now);
        self.stalled_since = None;
    }

    /// The verdict at `now`; bumps `episode` when a stall starts, and again
    /// each `REARM_AFTER` it stays stalled.
    pub fn observe(&mut self, i: &StallInputs, now: Instant) -> bool {
        if i.lead_campaign != self.campaign {
            self.campaign = i.lead_campaign.clone();
            self.last_cum = None;
            self.advanced = false;
            self.last_advance = Some(now);
            self.stalled_since = None;
        }
        match self.last_cum {
            Some(prev) if i.cumulative > prev => {
                self.advanced = true;
                self.last_advance = Some(now);
                self.stalled_since = None;
            }
            None => self.last_advance = Some(now),
            _ => {}
        }
        self.last_cum = Some(self.last_cum.map_or(i.cumulative, |p| p.max(i.cumulative)));
        let armed = i.solo
            && i.playing
            && i.lead_campaign.is_some()
            && i.lead_below
            && i.cumulative > 0
            && self.advanced;
        let flat = self
            .last_advance
            .is_some_and(|t| now.duration_since(t) >= STALL_AFTER);
        if !armed || !flat {
            self.stalled_since = None;
            return false;
        }
        match self.stalled_since {
            None => {
                self.stalled_since = Some(now);
                self.episode += 1;
            }
            Some(t) if now.duration_since(t) >= REARM_AFTER => {
                self.stalled_since = Some(now);
                self.episode += 1;
            }
            _ => {}
        }
        true
    }
}

// ---- State -------------------------------------------------------------------

/// One inventory read, account-wide.
pub struct InvSnapshot {
    pub read_started_at: DateTime<Utc>,
    /// The drops token epoch it was read under.
    pub epoch: u64,
    pub inventory: InventoryResponse,
    /// Per-drop progress parsed from the same answer exactly as the monitoring
    /// loop's auto-claim always read it.
    pub progress_overlay: HashMap<String, DropProgress>,
}

#[derive(Debug, Clone, Copy)]
struct Claim {
    state: ClaimState,
    at: DateTime<Utc>,
    /// The monitoring loop made it and settles it.
    by_loop: bool,
}

/// What the watcher reads with, captured once from the drops service. It never
/// locks that service.
pub struct Handles {
    pub client: reqwest::Client,
    pub device_id: String,
    pub session_id: String,
    pub cached_campaigns: Arc<RwLock<Option<(Vec<DropCampaign>, DateTime<Utc>)>>>,
    pub ungranted: UngrantedHandle,
}

#[derive(Default)]
struct State {
    rev: u64,
    view: WatchDrops,
    /// The drops token epoch this state belongs to.
    epoch: Option<u64>,
    /// The drops account, for telling its pushes apart.
    user_id: Option<String>,
    place: Option<Place>,
    place_gen: u64,
    /// The place generation whose first answer is in.
    verdict_gen: u64,
    list: Vec<DropCampaign>,
    gate_open: bool,
    snapshot: Option<Arc<InvSnapshot>>,
    /// Campaigns kept from the read before because the newest read left them out.
    carried: HashSet<String>,
    overlay: HashMap<String, (i32, DateTime<Utc>)>,
    instances: HashMap<String, String>,
    seen_claims: HashSet<(String, String)>,
    claims: HashMap<String, Claim>,
    ungranted: HashSet<String>,
    /// Rewards watching cannot earn yet (`evolution::locked_stages`).
    locked: HashMap<String, String>,
    read_wanted: bool,
    /// When the wanted read was asked for: a read that started before then
    /// (a claim or a push arrived while it ran) does not answer it.
    wanted_at: Option<DateTime<Utc>>,
    asked_ids: HashSet<String>,
    last_push: Option<Instant>,
    failures: u32,
    /// When the watcher last asked for a read, answered or not: the cadence
    /// (and so the failure backoff) counts from here.
    last_attempt: Option<Instant>,
    playing: bool,
    solo: bool,
    stall: StallClock,
    acked: u64,
    ready_ids: HashSet<String>,
    /// The lip tucked away by the viewer, until `tuck_holds` lets go.
    tucked: Option<Tucked>,
}

static STATE: Lazy<Mutex<State>> = Lazy::new(|| Mutex::new(State::default()));
static WAKE: Lazy<Notify> = Lazy::new(Notify::new);
static READY: Lazy<Notify> = Lazy::new(Notify::new);
static VERDICT: Lazy<Notify> = Lazy::new(Notify::new);
static APP: OnceLock<AppHandle> = OnceLock::new();

/// The app, once the watcher has started (for work begun outside a command,
/// such as a finished start re-deciding MultiNook's exit).
pub fn app() -> Option<AppHandle> {
    APP.get().cloned()
}

/// One inventory read at a time, for the watcher and the monitoring loop alike.
static READ_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(&mut guard)
}

fn send(snapshot: Option<WatchDropsSnapshot>) {
    if let (Some(snapshot), Some(app)) = (snapshot, APP.get()) {
        let _ = app.emit(WATCH_DROPS_CHANGED, &snapshot);
    }
}

/// Something the model depends on may have changed: the watcher looks again.
pub fn poke() {
    WAKE.notify_one();
}

/// The drops token was stored or removed.
pub fn drops_auth_changed() {
    poke();
}

/// Resolves when a reward here turns ready to claim, for the monitoring loop.
pub async fn ready_changed() {
    READY.notified().await
}

/// Tuck the lip away (`true`) or bring it back (`false`). Tucking holds the
/// reward on screen now, and lets go on its own when that is no longer true.
pub fn set_lip_hidden(hidden: bool) {
    let out = with_state(|s| {
        s.tucked = if hidden {
            s.view.lead.as_ref().map(|l| Tucked {
                campaign_id: l.campaign_id.clone(),
                drop_id: l.drop_id.clone(),
                channel: channel_key(&s.view),
            })
        } else {
            None
        };
        rebuild(s)
    });
    send(out);
}

/// Rewards watching cannot earn yet, as of the last look, for reads made
/// outside the model (the phones' own bar read).
pub fn locked() -> HashMap<String, String> {
    with_state(|s| s.locked.clone())
}

/// The model now.
pub fn current() -> WatchDropsSnapshot {
    with_state(|s| WatchDropsSnapshot {
        rev: s.rev,
        view: s.view.clone(),
    })
}

/// Rebuild the view; `Some` (to emit, after the lock) only when it changed.
fn rebuild(s: &mut State) -> Option<WatchDropsSnapshot> {
    let now_ms = Utc::now().timestamp_millis();
    let mut view = match (&s.place, s.gate_open) {
        (Some(place), true) => {
            let claims: HashMap<String, ClaimState> =
                s.claims.iter().map(|(k, c)| (k.clone(), c.state)).collect();
            build(&Inputs {
                place,
                list: &s.list,
                inventory: s.snapshot.as_ref().map(|x| &x.inventory),
                overlay: &s.overlay,
                read_started_at: s.snapshot.as_ref().map(|x| x.read_started_at),
                instances: &s.instances,
                claims: &claims,
                ungranted: &s.ungranted,
                locked: &s.locked,
                held_lead: s.view.lead.as_ref().map(|l| l.drop_id.as_str()),
                now_ms,
            })
        }
        (Some(place), false) => WatchDrops {
            channel_id: place.channel_id.clone(),
            login: place.login.clone(),
            game_id: place.game_id.clone(),
            game_name: place.game_name.clone(),
            ..Default::default()
        },
        (None, _) => WatchDrops::default(),
    };
    let lead_campaign = view.lead.as_ref().map(|l| l.campaign_id.clone());
    let cumulative = lead_campaign
        .as_ref()
        .and_then(|id| view.campaigns.iter().find(|c| &c.id == id))
        .map(|c| c.cumulative_minutes)
        .unwrap_or(0);
    let inputs = StallInputs {
        solo: s.solo,
        playing: s.playing,
        lead_below: view.lead.as_ref().is_some_and(|l| !l.complete),
        lead_campaign,
        cumulative,
    };
    view.stalled = s.stall.observe(&inputs, Instant::now());
    view.stall_episode = s.stall.episode;
    let ready: HashSet<String> = view
        .campaigns
        .iter()
        .flat_map(|c| c.drops.iter())
        .filter(|d| d.ready && !d.pending)
        .map(|d| d.id.clone())
        .collect();
    if ready.iter().any(|id| !s.ready_ids.contains(id)) {
        READY.notify_one();
    }
    s.ready_ids = ready;
    if let Some(t) = &s.tucked {
        if tuck_holds(t, &view) {
            view.lip_hidden = true;
        } else {
            s.tucked = None;
        }
    }
    if view == s.view {
        return None;
    }
    s.view = view;
    s.rev += 1;
    Some(WatchDropsSnapshot {
        rev: s.rev,
        view: s.view.clone(),
    })
}

fn set_place(s: &mut State, place: Option<Place>) -> Option<WatchDropsSnapshot> {
    let same = match (&s.place, &place) {
        (Some(a), Some(b)) => same_place(a, b),
        (None, None) => true,
        _ => false,
    };
    if !same {
        s.place_gen += 1;
        want_read(s);
        s.stall.reset();
        s.carried.clear();
    }
    s.place = place;
    rebuild(s)
}

fn clear_account(s: &mut State) {
    s.user_id = None;
    s.snapshot = None;
    s.carried.clear();
    s.overlay.clear();
    s.instances.clear();
    s.seen_claims.clear();
    s.claims.clear();
    s.ungranted.clear();
    s.asked_ids.clear();
    s.ready_ids.clear();
    s.last_push = None;
    s.failures = 0;
    want_read(s);
    s.stall.reset();
    // Anyone waiting on an answer waits for the new account's.
    s.place_gen += 1;
}

/// A campaign the newest read left out while it still counts here keeps the
/// previous read's entry once; a second read without it drops it.
fn carry_missing(
    prev: &InvSnapshot,
    next: Arc<InvSnapshot>,
    place: Option<&Place>,
    carried: &mut HashSet<String>,
) -> Arc<InvSnapshot> {
    let Some(place) = place else {
        carried.clear();
        return next;
    };
    let present: HashSet<String> = next
        .inventory
        .items
        .iter()
        .map(|i| i.campaign.id.clone())
        .collect();
    carried.retain(|id| !present.contains(id));
    let now = Utc::now();
    let keep: Vec<_> = prev
        .inventory
        .items
        .iter()
        .filter(|item| {
            counts_here(&item.campaign, place)
                && !present.contains(&item.campaign.id)
                && item.campaign.end_at > now
                && !carried.contains(&item.campaign.id)
        })
        .cloned()
        .collect();
    if keep.is_empty() {
        return next;
    }
    let mut inventory = next.inventory.clone();
    for item in keep {
        carried.insert(item.campaign.id.clone());
        inventory.items.push(item);
    }
    Arc::new(InvSnapshot {
        read_started_at: next.read_started_at,
        epoch: next.epoch,
        inventory,
        progress_overlay: next.progress_overlay.clone(),
    })
}

/// The monitoring loop stopped, and with it its record of unconfirmed claims.
/// Claims it made become ordinary ones, cleared by the next read taken late
/// enough after them, so none stays "Claiming" for good.
pub fn loop_stopped() {
    with_state(|s| {
        for claim in s.claims.values_mut() {
            claim.by_loop = false;
        }
    });
}

/// A read is wanted from now on.
fn want_read(s: &mut State) {
    s.read_wanted = true;
    s.wanted_at = Some(Utc::now());
}

fn store_snapshot(s: &mut State, snap: Arc<InvSnapshot>) -> Option<WatchDropsSnapshot> {
    if s.epoch != Some(snap.epoch) {
        return None;
    }
    if s.snapshot
        .as_ref()
        .is_some_and(|old| old.read_started_at > snap.read_started_at)
    {
        return None;
    }
    let started = snap.read_started_at;
    let kept = match s.snapshot.clone() {
        Some(prev) => carry_missing(&prev, snap, s.place.as_ref(), &mut s.carried),
        None => snap,
    };
    s.snapshot = Some(kept);
    s.overlay.retain(|_, (_, pushed)| *pushed > started);
    let settle = chrono::Duration::seconds(CLAIM_SETTLE_SECS);
    s.claims
        .retain(|_, c| (c.by_loop && c.state == ClaimState::Pending) || started < c.at + settle);
    s.asked_ids.clear();
    s.failures = 0;
    if s.wanted_at.map_or(true, |w| started >= w) {
        s.read_wanted = false;
        s.wanted_at = None;
    }
    s.verdict_gen = s.place_gen;
    VERDICT.notify_waiters();
    rebuild(s)
}

fn fresh(t: DateTime<Utc>, epoch: u64) -> Option<Arc<InvSnapshot>> {
    with_state(|s| s.snapshot.clone()).filter(|x| x.epoch == epoch && x.read_started_at >= t)
}

/// An inventory read that started at or after `t`: the newest one when it
/// qualifies, else one read now (one at a time for the whole app). Account-wide:
/// discarded only when the drops account changed during the read.
pub async fn inventory_newer_than(
    t: DateTime<Utc>,
    client: &reqwest::Client,
    device_id: &str,
    session_id: &str,
) -> anyhow::Result<Arc<InvSnapshot>> {
    let epoch = DropsAuthService::token_epoch();
    if let Some(snap) = fresh(t, epoch) {
        return Ok(snap);
    }
    let _one = READ_LOCK.lock().await;
    if let Some(snap) = fresh(t, epoch) {
        return Ok(snap);
    }
    let read_started_at = Utc::now();
    let (inventory, progress_overlay) =
        DropsService::fetch_inventory_with(client, device_id, session_id, SECOND_LIST_MAX_AGE)
            .await?;
    if DropsAuthService::token_epoch() != epoch {
        anyhow::bail!("the drops account changed during the inventory read");
    }
    let snap = Arc::new(InvSnapshot {
        read_started_at,
        epoch,
        inventory,
        progress_overlay,
    });
    let out = with_state(|s| store_snapshot(s, snap.clone()));
    send(out);
    Ok(snap)
}

// ---- Claims, pushes, the stall acknowledgement, the phones --------------------

/// A claim's answer (a click, or the monitoring loop with `by_loop`).
pub fn claimed(drop_id: &str, state: ClaimState, at: DateTime<Utc>, by_loop: bool) {
    let out = with_state(|s| {
        s.claims
            .insert(drop_id.to_string(), Claim { state, at, by_loop });
        want_read(s);
        rebuild(s)
    });
    send(out);
    poke();
}

/// The loop found a claim not granted: the tier shows Connect (or Watched)
/// at once, so it never flashes back to Ready while the next read is due.
pub fn forget_claim(drop_id: &str) {
    let out = with_state(|s| {
        s.claims.remove(drop_id);
        s.ungranted.insert(drop_id.to_string());
        rebuild(s)
    });
    send(out);
    poke();
}

/// Fold one `user-drop-events` message in. Returns (rebuild, wake the watcher).
fn apply_push(
    s: &mut State,
    topic_user: &str,
    msg: &serde_json::Value,
    now: DateTime<Utc>,
) -> (bool, bool) {
    if topic_user.is_empty() || s.user_id.as_deref() != Some(topic_user) {
        return (false, false);
    }
    let data = &msg["data"];
    let Some(drop_id) = data["drop_id"].as_str().filter(|id| !id.is_empty()) else {
        return (false, false);
    };
    s.last_push = Some(Instant::now());
    match msg["type"].as_str().unwrap_or_default() {
        "drop-progress" => {
            let minutes = data["current_progress_min"]
                .as_i64()
                .unwrap_or(0)
                .clamp(0, i32::MAX as i64) as i32;
            s.overlay.insert(drop_id.to_string(), (minutes, now));
            let known = s.snapshot.as_ref().is_some_and(|x| {
                x.inventory
                    .items
                    .iter()
                    .any(|i| i.campaign.time_based_drops.iter().any(|d| d.id == drop_id))
            });
            let ask = !known && s.asked_ids.insert(drop_id.to_string());
            if ask {
                want_read(s);
            }
            (true, ask)
        }
        "drop-claim" => {
            if let Some(instance) = data["drop_instance_id"].as_str().filter(|x| !x.is_empty()) {
                if !s
                    .seen_claims
                    .insert((drop_id.to_string(), instance.to_string()))
                {
                    return (false, false);
                }
                s.instances
                    .insert(drop_id.to_string(), instance.to_string());
            }
            want_read(s);
            (true, true)
        }
        _ => (false, false),
    }
}

static LOGGED_PROGRESS: AtomicBool = AtomicBool::new(false);
static LOGGED_CLAIM: AtomicBool = AtomicBool::new(false);

/// A `user-drop-events.<user>` message from the points socket.
pub fn on_push(topic_user: &str, msg: &serde_json::Value) {
    let kind = msg["type"].as_str().unwrap_or_default().to_string();
    let first = match kind.as_str() {
        "drop-progress" => !LOGGED_PROGRESS.swap(true, Ordering::Relaxed),
        "drop-claim" => !LOGGED_CLAIM.swap(true, Ordering::Relaxed),
        _ => false,
    };
    let (out, wake, known) = with_state(|s| {
        let drop_id = msg["data"]["drop_id"].as_str().unwrap_or_default();
        let known = s.snapshot.as_ref().is_some_and(|x| {
            x.inventory
                .items
                .iter()
                .any(|i| i.campaign.time_based_drops.iter().any(|d| d.id == drop_id))
        });
        let (changed, wake) = apply_push(s, topic_user, msg, Utc::now());
        (if changed { rebuild(s) } else { None }, wake, known)
    });
    // Build step 0 reads these lines: the first of each kind, and whether its
    // drop id is one the inventory knows. Logged as a warning because release
    // logs keep only warnings, and this is the one proof the pushes arrive.
    if first {
        log::warn!("[DropEvents] first {kind}: known={known} message={msg}");
    }
    send(out);
    if wake {
        poke();
    }
}

fn ack_in(s: &mut State, episode: u64) -> bool {
    if episode == 0 || !s.view.stalled || s.view.stall_episode != episode || s.acked >= episode {
        return false;
    }
    s.acked = episode;
    true
}

/// A surface claims the current stall episode; true once per episode.
pub fn ack_stall(episode: u64) -> bool {
    with_state(|s| ack_in(s, episode))
}

/// The phones' `get_watch_drop` from the model, when it follows this very
/// place: `Some(lead)` once its first answer for the place is in (waiting up
/// to `VERDICT_WAIT`), `None` when it follows somewhere else or is too slow.
pub async fn answer_for(
    channel_id: &str,
    login: &str,
    game_id: &str,
    game_name: &str,
) -> Option<Option<WatchDrop>> {
    let deadline = Instant::now() + VERDICT_WAIT;
    loop {
        let wait = VERDICT.notified();
        let (matches, ready, lead) = with_state(|s| {
            let matches = s
                .place
                .as_ref()
                .is_some_and(|p| place_matches(p, channel_id, login, game_id, game_name));
            (matches, s.verdict_gen == s.place_gen, lead_for(&s.view))
        });
        if !matches {
            return None;
        }
        if ready {
            return Some(lead);
        }
        let now = Instant::now();
        if now >= deadline {
            return None;
        }
        let _ = tokio::time::timeout((deadline - now).min(Duration::from_millis(250)), wait).await;
    }
}

// ---- The watcher -------------------------------------------------------------

/// The cached campaign list, fetched again when missing or older than
/// `CAMPAIGNS_MAX_AGE_SECS`. Priming the cache never touches the live progress map.
async fn campaigns(h: &Handles) -> Vec<DropCampaign> {
    let cached = h.cached_campaigns.read().await.clone();
    let young = cached
        .as_ref()
        .is_some_and(|(_, at)| (Utc::now() - *at).num_seconds() < CAMPAIGNS_MAX_AGE_SECS);
    if young {
        return cached.map(|(c, _)| c).unwrap_or_default();
    }
    match DropsService::fetch_active_campaigns(&h.client, &h.device_id, &h.session_id).await {
        Ok(list) => {
            *h.cached_campaigns.write().await = Some((list.clone(), Utc::now()));
            list
        }
        Err(e) => {
            log::debug!("[WatchDrops] campaign list: {e}");
            cached.map(|(c, _)| c).unwrap_or_default()
        }
    }
}

/// One look at everything. The wait before the next one, or `None` to wait for a poke.
async fn step(app: &AppHandle, h: &Handles) -> Option<Duration> {
    let state = app.state::<AppState>();
    let epoch = DropsAuthService::token_epoch();
    let reaim = with_state(|s| {
        if s.epoch == Some(epoch) {
            return false;
        }
        let had = s.epoch.is_some();
        clear_account(s);
        s.epoch = Some(epoch);
        had
    });
    if reaim {
        let background = state.background_service.clone();
        tauri::async_runtime::spawn(
            async move { background.lock().await.reaim_for_account().await },
        );
    }
    if !DropsAuthService::is_authenticated().await {
        send(with_state(|s| set_place(s, None)));
        return None;
    }
    if with_state(|s| s.user_id.is_none()) {
        if let Ok(token) = DropsAuthService::get_token().await {
            if let Some(user) =
                crate::services::background_service::BackgroundService::fetch_user_id(&token).await
            {
                with_state(|s| {
                    if s.epoch == Some(epoch) {
                        s.user_id = Some(user);
                    }
                });
            }
        }
    }
    let Some(earning) = state.watch_heartbeat.earning().await else {
        send(with_state(|s| set_place(s, None)));
        return None;
    };
    let place = place_from(&earning, game_for(&earning.channel_id).as_ref());
    if place.game_id.is_empty() && place.game_name.is_empty() {
        let heartbeat = state.watch_heartbeat.clone();
        tauri::async_runtime::spawn(async move { heartbeat.resolve_now().await });
    }
    let playing =
        state.watch_heartbeat.is_playing() && !crate::services::app_presence::phone_quiet(app);
    let solo = crate::services::watch_session::solo_earning(&place.channel_id);
    let list = campaigns(h).await;
    let second = crate::services::reward_drops::cached();
    let gate = gate_open(&place, &list, &second);
    let locked = crate::services::evolution::locked_stages(app).await;
    let (out, due, locked_changed) = with_state(|s| {
        if playing && !s.playing {
            s.stall.play_edge(Instant::now());
        }
        s.playing = playing;
        s.solo = solo;
        s.list = list;
        let locked_changed = s.locked != locked;
        s.locked = locked;
        s.gate_open = gate;
        let out = set_place(s, Some(place.clone()));
        if !gate {
            s.verdict_gen = s.place_gen;
            VERDICT.notify_waiters();
        }
        let age = s.snapshot.as_ref().map(|x| {
            (Utc::now() - x.read_started_at)
                .to_std()
                .unwrap_or_default()
        });
        let cadence = next_read_after(s.last_push.map(|t| t.elapsed()), s.failures);
        let due = read_due(
            gate,
            s.read_wanted,
            s.snapshot.is_some(),
            playing,
            age,
            s.last_attempt.map(|t| t.elapsed()),
            cadence,
        );
        if due {
            s.last_attempt = Some(Instant::now());
        }
        (out, due, locked_changed)
    });
    send(out);
    // Autopilot works only lines that can move; it hears the new set at once.
    if locked_changed {
        state.plugin_host.push_drops_settings();
    }
    if due {
        if let Err(e) =
            inventory_newer_than(Utc::now(), &h.client, &h.device_id, &h.session_id).await
        {
            let first = with_state(|s| {
                s.failures += 1;
                // The phones get the view as it stands rather than wait on a retry.
                s.read_wanted = false;
                s.verdict_gen = s.place_gen;
                s.failures == 1
            });
            VERDICT.notify_waiters();
            if first {
                log::warn!("[WatchDrops] inventory read failed: {e}");
            } else {
                log::debug!("[WatchDrops] inventory read failed again: {e}");
            }
        }
    }
    let ungranted = h.ungranted.ids().await;
    let out = with_state(|s| {
        s.ungranted = ungranted;
        rebuild(s)
    });
    send(out);
    // While a stream plays here the stall clock needs a look each minute.
    Some(with_state(|s| {
        if s.gate_open && s.playing {
            FAST
        } else {
            SLOW
        }
    }))
}

/// Starts the watcher (once, at setup, after `AppState` is managed).
pub fn start(app: AppHandle) {
    if APP.set(app.clone()).is_err() {
        return;
    }
    tauri::async_runtime::spawn(async move {
        let handles = {
            let state = app.state::<AppState>();
            let drops = state.drops_service.lock().await;
            drops.read_handles()
        };
        loop {
            let wake = WAKE.notified();
            tokio::pin!(wake);
            wake.as_mut().enable();
            match step(&app, &handles).await {
                Some(wait) => {
                    tokio::select! {
                        _ = &mut wake => {}
                        _ = tokio::time::sleep(wait) => {}
                    }
                }
                None => wake.await,
            }
        }
    });
}

#[cfg(test)]
pub(crate) mod golden {
    use super::Place;
    use crate::models::drops::{CompletedDrop, DropCampaign, InventoryItem, InventoryResponse};
    use serde_json::{json, Value};

    pub const CASES: &str = include_str!("../../tests/fixtures/watch_drops_cases.json");

    pub struct Case {
        pub name: String,
        pub list: Vec<DropCampaign>,
        pub inventory: InventoryResponse,
        pub rust: Value,
        pub via_pick: bool,
    }

    fn drop_json(cid: &str, t: &Value, with_progress: bool, accruing: bool) -> Value {
        let id = t["id"].as_str().unwrap();
        let need = t["need"].as_i64().unwrap();
        let progress = if with_progress {
            json!({
                "campaign_id": cid, "drop_id": id,
                "current_minutes_watched": t["have"].as_i64().unwrap(), "required_minutes_watched": need,
                "is_claimed": t["claimed"].as_bool().unwrap_or(false),
                "last_updated": "2026-11-06T00:00:00Z", "drop_instance_id": format!("inst-{id}"),
                "twitch_progress": if accruing {
                    json!({ "days_done": 0, "minutes_today": 0, "window_expires_at": null, "accruing": true,
                            "subs_done": 0, "ready_to_claim": false, "earned": null })
                } else { Value::Null },
            })
        } else {
            Value::Null
        };
        json!({
            "id": id, "name": format!("drop {id}"), "required_minutes_watched": need,
            "benefit_edges": [{
                "id": t["benefit"].as_str().map(String::from).unwrap_or_else(|| format!("b{id}")),
                "name": t["bname"].as_str().map(String::from).unwrap_or_else(|| format!("Reward {id}")),
                "image_url": format!("img{id}"), "distribution_type": null,
            }],
            "progress": progress, "is_collectible": need > 0, "required_subs": 0, "required_days": 0,
            "random_of": null, "next_reward": null,
        })
    }

    fn campaign(c: &Value, with_progress: bool) -> DropCampaign {
        let id = c["id"].as_str().unwrap();
        let game = c["game"].as_str().unwrap();
        let allowed: Vec<Value> = c["allowed"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|n| json!({ "id": format!("id-{}", n.as_str().unwrap()), "name": n }))
                    .collect()
            })
            .unwrap_or_default();
        let accruing = c["accruing"].as_bool().unwrap_or(false);
        serde_json::from_value(json!({
            "id": id, "name": c["name"].as_str().map(String::from).unwrap_or_else(|| format!("Campaign {id}")),
            "game_id": c["game_id"].as_str().map(String::from).unwrap_or_else(|| format!("g-{game}")),
            "game_name": game, "description": "", "image_url": "",
            "start_at": "2026-01-01T00:00:00Z", "end_at": "2099-01-01T00:00:00Z",
            "time_based_drops": c["tiers"].as_array().unwrap().iter().map(|t| drop_json(id, t, with_progress, accruing)).collect::<Vec<_>>(),
            "is_account_connected": c["linked"].as_bool().unwrap_or(true),
            "allowed_channels": allowed.clone(), "is_acl_based": !allowed.is_empty(),
            "details_url": null, "account_link": c["link"].clone(),
            "separate_progress": c["separate"].as_bool().unwrap_or(false),
            "has_category": c["has_category"].as_bool().unwrap_or(true), "category_ids": [],
        }))
        .unwrap()
    }

    pub fn place() -> Place {
        Place {
            channel_id: "id-shroud".into(),
            login: "shroud".into(),
            game_id: String::new(),
            game_name: "Rust".into(),
        }
    }

    pub fn cases() -> Vec<Case> {
        let all: Vec<Value> = serde_json::from_str(CASES).unwrap();
        all.into_iter()
            .map(|k| {
                let campaigns = k["campaigns"].as_array().unwrap();
                let items: Vec<InventoryItem> = campaigns
                    .iter()
                    .filter(|c| c["in_inventory"].as_bool().unwrap_or(true))
                    .map(|c| {
                        serde_json::from_value(json!({
                            "campaign": serde_json::to_value(campaign(c, true)).unwrap(), "status": "Active",
                            "progress_percentage": 0.0, "total_drops": 0, "claimed_drops": 0, "drops_in_progress": 0,
                        }))
                        .unwrap()
                    })
                    .collect();
                let list = campaigns.iter().filter(|c| c["in_list"].as_bool().unwrap_or(true)).map(|c| campaign(c, false)).collect();
                let completed: Vec<CompletedDrop> = k["completed"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .map(|d| {
                                serde_json::from_value(json!({
                                    "id": d["id"], "name": d["name"], "image_url": "", "game_name": null,
                                    "is_connected": true, "required_account_link": null,
                                    "last_awarded_at": "2026-01-01T00:00:00Z", "total_count": 1,
                                }))
                                .unwrap()
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                Case {
                    name: k["name"].as_str().unwrap().to_string(),
                    list,
                    inventory: InventoryResponse {
                        items,
                        total_campaigns: 0,
                        active_campaigns: 0,
                        upcoming_campaigns: 0,
                        expired_campaigns: 0,
                        completed_drops: completed,
                    },
                    rust: k["rust"].clone(),
                    via_pick: k["via_pick"].as_bool().unwrap_or(false),
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn now_ms() -> i64 {
        Utc::now().timestamp_millis()
    }

    #[test]
    fn every_golden_case_gives_the_intended_answer() {
        let place = golden::place();
        for case in golden::cases() {
            let view = build(&Inputs::bare(
                &place,
                &case.list,
                Some(&case.inventory),
                now_ms(),
            ));
            let r = &case.rust;
            assert_eq!(
                view.lead.as_ref().map(|l| l.drop_id.as_str()),
                r["lead"].as_str(),
                "{}: lead",
                case.name
            );
            if let Some(lead) = &view.lead {
                let row = view
                    .campaigns
                    .iter()
                    .flat_map(|c| &c.drops)
                    .find(|d| d.lead)
                    .expect("the lead row");
                assert_eq!(row.id, lead.drop_id, "{}", case.name);
                assert_eq!(
                    row.current_minutes as i64,
                    r["lead_shown"].as_i64().unwrap(),
                    "{}: minutes",
                    case.name
                );
                assert_eq!(
                    lead.complete,
                    r["lead_ready"].as_bool().unwrap(),
                    "{}: ready",
                    case.name
                );
            }
            assert_eq!(
                view.complete,
                r["complete"].as_bool().unwrap(),
                "{}: complete",
                case.name
            );
            let listed: Vec<&str> = view.campaigns.iter().map(|c| c.id.as_str()).collect();
            let want: Vec<&str> = r["listed"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap())
                .collect();
            assert_eq!(listed, want, "{}: listed", case.name);
            assert_eq!(
                view.more as i64,
                r["more"].as_i64().unwrap(),
                "{}: more",
                case.name
            );
        }
    }

    #[test]
    fn the_bars_stacked_art_is_the_next_rewards_never_the_lead_or_a_claimed_one() {
        let place = golden::place();
        let mut stacked_any = false;
        for case in golden::cases() {
            let view = build(&Inputs::bare(
                &place,
                &case.list,
                Some(&case.inventory),
                now_ms(),
            ));
            assert!(view.more_art.len() <= 2, "{}", case.name);
            assert!(view.more_art.len() as u32 <= view.more, "{}", case.name);
            let rows: Vec<&HereDrop> = view.campaigns.iter().flat_map(|c| &c.drops).collect();
            if let Some(lead) = rows.iter().find(|d| d.lead) {
                assert!(!view.more_art.contains(&lead.image), "{}: lead stacked", case.name);
            }
            for art in &view.more_art {
                let row = rows.iter().find(|d| &d.image == art).expect("a row's picture");
                assert!(!row.claimed, "{}: claimed stacked", case.name);
            }
            let first: Vec<String> = rows
                .iter()
                .filter(|d| !d.claimed && !d.lead)
                .map(|d| d.image.clone())
                .take(2)
                .collect();
            assert_eq!(view.more_art, first, "{}: campaign order", case.name);
            stacked_any |= !view.more_art.is_empty();
        }
        assert!(stacked_any, "some golden case stacks art");
    }

    #[test]
    fn an_earned_tier_offers_connect_not_claim_while_twitch_says_the_account_is_not_linked() {
        // Twitch's own Inventory shows Connect in place of Claim for exactly
        // this: the minutes are in and the game account is not linked.
        let place = golden::place();
        let case = golden::cases()
            .into_iter()
            .find(|c| c.name == "a_campaign_reported_unlinked_still_leads")
            .unwrap();
        let mut inv = case.inventory.clone();
        for it in inv.items.iter_mut() {
            for d in it.campaign.time_based_drops.iter_mut() {
                if let Some(p) = d.progress.as_mut() {
                    p.current_minutes_watched = d.required_minutes_watched;
                }
            }
        }
        let view = build(&Inputs::bare(&place, &case.list, Some(&inv), now_ms()));
        let tier = &view.campaigns[0].drops[0];
        assert!(tier.connect && !tier.ready && !tier.watched, "Connect in place of Claim");
        assert_eq!(tier.status, "Connect");
        assert_eq!(view.campaigns[0].link_url.as_deref(), Some("https://example.test/link"));
        // Still earning: no Connect yet, just its time.
        let view = build(&Inputs::bare(&place, &case.list, Some(&case.inventory), now_ms()));
        let tier = &view.campaigns[0].drops[0];
        assert!(!tier.connect && !tier.ready);
        // Linked: the same earned tier offers Claim.
        let mut list = case.list.clone();
        for c in list.iter_mut() {
            c.is_account_connected = true;
        }
        for it in inv.items.iter_mut() {
            it.campaign.is_account_connected = true;
        }
        let view = build(&Inputs::bare(&place, &list, Some(&inv), now_ms()));
        let tier = &view.campaigns[0].drops[0];
        assert!(tier.ready && !tier.connect);
        assert_eq!(tier.status, "Ready");
    }

    #[test]
    fn the_inventory_copy_keeps_the_lists_account_link() {
        // The Inventory never carries accountLinkURL; Connect still needs the
        // list's link.
        let place = golden::place();
        let case = golden::cases()
            .into_iter()
            .find(|c| {
                let v = build(&Inputs::bare(&golden::place(), &c.list, Some(&c.inventory), now_ms()));
                v.campaigns.len() == 1 && c.inventory.items.iter().any(|it| it.campaign.id == v.campaigns[0].id)
            })
            .expect("a case listing one campaign that is in the inventory");
        let (mut list, mut inv) = (case.list.clone(), case.inventory.clone());
        let id = inv.items[0].campaign.id.clone();
        for c in list.iter_mut().filter(|c| c.id == id) {
            c.account_link = Some("https://game.example/link".into());
        }
        for it in inv.items.iter_mut() {
            it.campaign.account_link = None;
        }
        let view = build(&Inputs::bare(&place, &list, Some(&inv), now_ms()));
        let c = view.campaigns.iter().find(|c| c.id == id).expect("the campaign");
        assert_eq!(c.link_url.as_deref(), Some("https://game.example/link"));
        // Without the list's copy there is still no link to invent.
        let view = build(&Inputs::bare(&place, &[], Some(&inv), now_ms()));
        if let Some(c) = view.campaigns.iter().find(|c| c.id == id) {
            assert_eq!(c.link_url, None);
        }
    }

    #[test]
    fn box_art_is_sized_and_only_for_a_campaign_with_a_category() {
        assert_eq!(
            sized_box_art("https://static-cdn.jtvnw.net/ttv-boxart/1_IGDB-{width}x{height}.jpg"),
            "https://static-cdn.jtvnw.net/ttv-boxart/1_IGDB-144x192.jpg"
        );
        assert_eq!(sized_box_art("https://x/a-285x380.jpg"), "https://x/a-285x380.jpg");
        let place = golden::place();
        let case = golden::cases()
            .into_iter()
            .find(|c| {
                let v = build(&Inputs::bare(&golden::place(), &c.list, Some(&c.inventory), now_ms()));
                v.campaigns.len() == 1
            })
            .expect("a case listing one campaign");
        let art = |list: &mut Vec<DropCampaign>, inv: &mut crate::models::drops::InventoryResponse, url: &str| {
            for c in list.iter_mut() {
                c.image_url = url.into();
            }
            for it in inv.items.iter_mut() {
                it.campaign.image_url = url.into();
            }
        };
        let (mut list, mut inv) = (case.list.clone(), case.inventory.clone());
        art(&mut list, &mut inv, "https://box/{width}x{height}.jpg");
        let view = build(&Inputs::bare(&place, &list, Some(&inv), now_ms()));
        assert_eq!(view.campaigns[0].box_art, "https://box/144x192.jpg");
        for c in list.iter_mut() {
            c.has_category = false;
        }
        for it in inv.items.iter_mut() {
            it.campaign.has_category = false;
        }
        let view = build(&Inputs::bare(&place, &list, Some(&inv), now_ms()));
        if let Some(c) = view.campaigns.first() {
            assert_eq!(c.box_art, "", "no category, no box art");
        }
    }

    #[test]
    fn not_started_rows_say_so() {
        let place = golden::place();
        let cases = golden::cases();
        let not_started = cases
            .iter()
            .find(|c| c.name == "a_not_started_campaign_is_listed_but_never_leads")
            .unwrap();
        let view = build(&Inputs::bare(
            &place,
            &not_started.list,
            Some(&not_started.inventory),
            now_ms(),
        ));
        assert!(!view.campaigns[0].started);
        assert_eq!(view.campaigns[0].drops[0].status, "Starts when you watch");
        assert_eq!(view.campaigns[0].claimed_label, "");
    }

    fn reward_list() -> Vec<serde_json::Value> {
        vec![
            json!({ "id": "multi", "name": "Partners", "brand": "Partners", "summary": "Partners",
                     "startsAt": "2026-01-01T00:00:00Z", "endsAt": "2099-01-01T00:00:00Z",
                     "aboutURL": "https://www.twitch.tv/drops/campaigns", "game": null }),
        ]
    }

    fn reward_details(minutes: u32) -> serde_json::Value {
        json!({
            "c0": { "id": "multi", "imageURL": "https://art/c.png", "rewardGroups": [
                { "id": "g-multi", "name": "Badge",
                  "progressCriteria": { "requirementType": "WATCH", "requirements": { "minutesWatched": 60, "subs": null }, "repeatableConfig": null },
                  "self": null,
                  "rewards": [ { "id": "badge", "name": "Badge", "thumbnailURL": "https://art/badge.png", "pool": null } ] } ] },
            "currentUser": { "id": "u1", "inventory": { "viewerRewardDropCampaignsInProgress": [
                { "id": "x", "rewardGroups": [ { "id": "g-multi", "self": { "status": "IN_PROGRESS", "currentMinutesWatched": minutes, "grantCount": 0 } } ] } ] } }
        })
    }

    #[test]
    fn a_cross_category_drop_opens_the_gate_only_while_its_minutes_rise() {
        let mut tracker = HashMap::new();
        let t0 = Utc::now();
        let place = golden::place();
        let first = crate::services::reward_drops::build(
            &reward_list(),
            &reward_details(5),
            &mut tracker,
            t0,
        );
        assert!(
            !gate_open(&place, &[], &first),
            "a cold start has seen no rise yet"
        );
        let rising = crate::services::reward_drops::build(
            &reward_list(),
            &reward_details(7),
            &mut tracker,
            t0 + chrono::Duration::minutes(2),
        );
        assert!(gate_open(&place, &[], &rising));
        let flat = crate::services::reward_drops::build(
            &reward_list(),
            &reward_details(7),
            &mut tracker,
            t0 + chrono::Duration::minutes(9),
        );
        assert!(!gate_open(&place, &[], &flat));
    }

    fn one_tier(have: i32) -> (Vec<DropCampaign>, InventoryResponse) {
        let case = golden::cases()
            .into_iter()
            .find(|c| c.name == "a_campaign_in_list_and_inventory_is_listed_once")
            .unwrap();
        let mut inv = case.inventory;
        inv.items[0].campaign.time_based_drops[0]
            .progress
            .as_mut()
            .unwrap()
            .current_minutes_watched = have;
        (case.list, inv)
    }

    #[test]
    fn a_push_newer_than_the_read_wins_and_an_older_one_yields() {
        let place = golden::place();
        let (list, inv) = one_tier(20);
        let read = Utc::now();
        let mut overlay = HashMap::new();
        overlay.insert("t1".to_string(), (35, read + chrono::Duration::seconds(10)));
        let mut i = Inputs::bare(&place, &list, Some(&inv), now_ms());
        i.overlay = &overlay;
        i.read_started_at = Some(read);
        assert_eq!(build(&i).campaigns[0].drops[0].current_minutes, 35);
        let mut older = HashMap::new();
        older.insert("t1".to_string(), (35, read - chrono::Duration::seconds(10)));
        i.overlay = &older;
        assert_eq!(build(&i).campaigns[0].drops[0].current_minutes, 20);
    }

    #[test]
    fn a_reward_that_needs_an_earlier_one_is_locked_and_never_leads() {
        // Reported by Jonas: Gengar's line showed as in progress forever for
        // an account with no Gastly.
        let place = golden::place();
        let (list, inv) = one_tier(20);
        let mut i = Inputs::bare(&place, &list, Some(&inv), now_ms());
        assert!(build(&i).lead.is_some());
        let locked = HashMap::from([("t1".to_string(), "Gastly".to_string())]);
        i.locked = &locked;
        let view = build(&i);
        assert!(view.lead.is_none());
        assert_eq!((view.more, view.complete), (0, false));
        let row = &view.campaigns[0].drops[0];
        assert!(row.locked && !row.ready && !row.lead);
        assert_eq!(row.status, "Locked");
        assert_eq!(row.detail.as_deref(), Some("Needs the Gastly badge first"));
        assert_eq!(row.bar, None);
    }

    fn state_with(user: &str) -> State {
        let (_, inv) = one_tier(20);
        State {
            epoch: Some(1),
            user_id: Some(user.to_string()),
            snapshot: Some(Arc::new(InvSnapshot {
                read_started_at: Utc::now(),
                epoch: 1,
                inventory: inv,
                progress_overlay: HashMap::new(),
            })),
            ..Default::default()
        }
    }

    #[test]
    fn a_read_begun_before_a_newer_want_leaves_it_wanted() {
        let mut s = state_with("u1");
        // No answer held yet, so nothing newer turns the read away first.
        s.snapshot = None;
        let begun = Utc::now() - chrono::Duration::seconds(5);
        // A claim (or a push) asks for a read while an older one is still running.
        want_read(&mut s);
        let snap = |at| {
            Arc::new(InvSnapshot {
                read_started_at: at,
                epoch: 1,
                inventory: one_tier(20).1,
                progress_overlay: HashMap::new(),
            })
        };
        let _ = store_snapshot(&mut s, snap(begun));
        assert!(s.read_wanted, "the older read does not answer the newer want");
        let _ = store_snapshot(&mut s, snap(Utc::now() + chrono::Duration::seconds(1)));
        assert!(!s.read_wanted);
    }

    #[test]
    fn a_push_for_an_unknown_drop_asks_for_one_read() {
        let mut s = state_with("u1");
        let msg = json!({ "type": "drop-progress", "data": { "drop_id": "t9", "current_progress_min": 4, "required_progress_min": 60 } });
        assert_eq!(apply_push(&mut s, "u1", &msg, Utc::now()), (true, true));
        assert_eq!(
            apply_push(&mut s, "u1", &msg, Utc::now()),
            (true, false),
            "asked once"
        );
        let known = json!({ "type": "drop-progress", "data": { "drop_id": "t1", "current_progress_min": 21, "required_progress_min": 60 } });
        assert_eq!(apply_push(&mut s, "u1", &known, Utc::now()), (true, false));
        assert_eq!(s.overlay.get("t1").map(|(m, _)| *m), Some(21));
    }

    #[test]
    fn another_accounts_push_is_dropped_and_a_claim_push_counts_once() {
        let mut s = state_with("u1");
        let progress = json!({ "type": "drop-progress", "data": { "drop_id": "t1", "current_progress_min": 30 } });
        assert_eq!(
            apply_push(&mut s, "u2", &progress, Utc::now()),
            (false, false)
        );
        assert!(s.overlay.is_empty());
        let claim =
            json!({ "type": "drop-claim", "data": { "drop_id": "t1", "drop_instance_id": "i-1" } });
        assert_eq!(apply_push(&mut s, "u1", &claim, Utc::now()), (true, true));
        assert_eq!(apply_push(&mut s, "u1", &claim, Utc::now()), (false, false));
        assert_eq!(s.instances.get("t1").map(String::as_str), Some("i-1"));
    }

    #[test]
    fn an_account_change_clears_what_belonged_to_the_old_one() {
        let mut s = state_with("u1");
        s.claims.insert(
            "t1".into(),
            Claim {
                state: ClaimState::Pending,
                at: Utc::now(),
                by_loop: true,
            },
        );
        s.overlay.insert("t1".into(), (3, Utc::now()));
        let gen = s.place_gen;
        clear_account(&mut s);
        assert!(
            s.snapshot.is_none()
                && s.claims.is_empty()
                && s.overlay.is_empty()
                && s.user_id.is_none()
        );
        assert!(s.read_wanted);
        assert_eq!(s.place_gen, gen + 1);
    }

    #[test]
    fn reads_slow_down_while_pushes_flow_and_after_failures() {
        assert_eq!(next_read_after(None, 0), FAST);
        assert_eq!(next_read_after(Some(Duration::from_secs(30)), 0), SLOW);
        assert_eq!(next_read_after(Some(Duration::from_secs(200)), 0), FAST);
        assert_eq!(next_read_after(None, 3), SLOW);
    }

    #[test]
    fn a_failed_read_waits_its_cadence_from_when_it_was_tried() {
        let s = Duration::from_secs;
        // No answer yet and never tried: read.
        assert!(read_due(true, false, false, true, None, None, FAST));
        // No answer because the last try failed 10 s ago: wait, not every step.
        assert!(!read_due(true, false, false, true, None, Some(s(10)), FAST));
        assert!(read_due(true, false, false, true, None, Some(s(61)), FAST));
        // Three failures stretch it to the slow cadence, still from the try.
        assert!(!read_due(true, false, true, true, Some(s(400)), Some(s(120)), SLOW));
        assert!(read_due(true, false, true, true, Some(s(400)), Some(s(301)), SLOW));
        // A wanted read goes at once; nothing reads with the gate closed.
        assert!(read_due(true, true, true, true, Some(s(1)), Some(s(1)), SLOW));
        assert!(!read_due(false, true, false, true, None, None, FAST));
        // A fresh answer while playing waits for the cadence.
        assert!(!read_due(true, false, true, true, Some(s(20)), Some(s(20)), FAST));
    }

    #[test]
    fn a_new_place_never_shows_the_previous_channels_drops() {
        let mut s = state_with("u1");
        s.gate_open = true;
        s.list = one_tier(20).0;
        set_place(&mut s, Some(golden::place()));
        assert!(s.view.lead.is_some());
        let elsewhere = Place {
            channel_id: "id-other".into(),
            login: "other".into(),
            game_id: String::new(),
            game_name: "Apex".into(),
        };
        let gen = s.place_gen;
        set_place(&mut s, Some(elsewhere));
        assert_eq!(s.place_gen, gen + 1);
        assert!(s.read_wanted);
        assert!(s.view.lead.is_none() && s.view.campaigns.is_empty());
        assert_eq!(s.view.channel_id, "id-other");
    }

    #[test]
    fn a_read_missing_a_campaign_keeps_it_once() {
        let place = golden::place();
        let (_, inv) = one_tier(20);
        let prev = InvSnapshot {
            read_started_at: Utc::now(),
            epoch: 1,
            inventory: inv.clone(),
            progress_overlay: HashMap::new(),
        };
        let mut empty = inv;
        empty.items.clear();
        let next = Arc::new(InvSnapshot {
            read_started_at: Utc::now(),
            epoch: 1,
            inventory: empty.clone(),
            progress_overlay: HashMap::new(),
        });
        let mut carried = HashSet::new();
        let kept = carry_missing(&prev, next, Some(&place), &mut carried);
        assert_eq!(kept.inventory.items.len(), 1);
        let again = Arc::new(InvSnapshot {
            read_started_at: Utc::now(),
            epoch: 1,
            inventory: empty,
            progress_overlay: HashMap::new(),
        });
        assert!(carry_missing(&kept, again, Some(&place), &mut carried)
            .inventory
            .items
            .is_empty());
    }

    fn armed(cumulative: i32) -> StallInputs {
        StallInputs {
            solo: true,
            playing: true,
            lead_campaign: Some("c1".into()),
            lead_below: true,
            cumulative,
        }
    }

    #[test]
    fn a_stall_needs_minutes_that_advanced_then_went_flat() {
        let t0 = Instant::now();
        let mut clock = StallClock::default();
        assert!(!clock.observe(&armed(10), t0));
        assert!(
            !clock.observe(&armed(10), t0 + Duration::from_secs(300)),
            "never advanced in this place"
        );
        assert!(!clock.observe(&armed(11), t0 + Duration::from_secs(301)));
        assert!(!clock.observe(&armed(11), t0 + Duration::from_secs(500)));
        assert!(clock.observe(&armed(11), t0 + Duration::from_secs(542)));
        assert_eq!(clock.episode, 1);
        assert!(clock.observe(&armed(11), t0 + Duration::from_secs(600)));
        assert_eq!(clock.episode, 1, "one episode while it stays stalled");
        assert!(clock.observe(&armed(11), t0 + Duration::from_secs(790)));
        assert_eq!(clock.episode, 2, "re-armed after REARM_AFTER");
    }

    #[test]
    fn every_stall_guard_holds_it_off() {
        let t0 = Instant::now();
        let guards: Vec<Box<dyn Fn(&mut StallInputs)>> = vec![
            Box::new(|i| i.solo = false),
            Box::new(|i| i.playing = false),
            Box::new(|i| i.lead_below = false),
            Box::new(|i| i.lead_campaign = None),
        ];
        for guard in guards {
            let mut clock = StallClock::default();
            clock.observe(&armed(10), t0);
            clock.observe(&armed(11), t0 + Duration::from_secs(1));
            let mut i = armed(11);
            guard(&mut i);
            assert!(!clock.observe(&i, t0 + Duration::from_secs(600)));
        }
        let mut zero = StallClock::default();
        zero.observe(&armed(0), t0);
        assert!(
            !zero.observe(&armed(0), t0 + Duration::from_secs(600)),
            "a clock at 0 never stalls"
        );
    }

    #[test]
    fn a_resume_after_a_long_pause_is_not_a_stall() {
        let t0 = Instant::now();
        let mut clock = StallClock::default();
        clock.observe(&armed(10), t0);
        clock.observe(&armed(11), t0 + Duration::from_secs(1));
        let mut paused = armed(11);
        paused.playing = false;
        assert!(!clock.observe(&paused, t0 + Duration::from_secs(900)));
        clock.play_edge(t0 + Duration::from_secs(900));
        assert!(!clock.observe(&armed(11), t0 + Duration::from_secs(901)));
    }

    #[test]
    fn a_stall_episode_is_acted_on_once() {
        let mut s = State::default();
        s.view.stalled = true;
        s.view.stall_episode = 3;
        assert!(!ack_in(&mut s, 2), "an old episode");
        assert!(ack_in(&mut s, 3));
        assert!(!ack_in(&mut s, 3), "taken once");
        s.view.stalled = false;
        s.view.stall_episode = 4;
        assert!(!ack_in(&mut s, 4), "not stalled");
    }

    #[test]
    fn the_newest_game_seen_wins_by_when_it_was_asked() {
        let t0 = Instant::now();
        let ch = "watch-drops-test-recency";
        saw_game(ch, "1", "Rust", t0);
        saw_game(ch, "2", "Apex", t0 + Duration::from_secs(5));
        assert_eq!(
            newest_game(ch, t0 + Duration::from_secs(1), "1".into(), "Rust".into()),
            ("2".into(), "Apex".into())
        );
        assert_eq!(
            newest_game(
                ch,
                t0 + Duration::from_secs(9),
                "3".into(),
                "Valorant".into()
            ),
            ("3".into(), "Valorant".into())
        );
        saw_game(ch, "1", "Rust", t0 + Duration::from_secs(2));
        assert_eq!(
            game_for(ch).unwrap().game_name,
            "Apex",
            "an older answer never replaces a newer one"
        );
    }

    #[test]
    fn a_start_row_seeds_the_game_but_never_beats_fresher_news() {
        let ch = "watch-drops-test-seed";
        // Stream info asked for before the session's row arrives keeps its answer.
        saw_game(ch, "2", "Apex", Instant::now());
        seed_game(ch, "1", "Rust");
        assert_eq!(game_for(ch).unwrap().game_name, "Apex");
        // With nothing known, the row seeds it, and any later news replaces it.
        let fresh = "watch-drops-test-seed-fresh";
        seed_game(fresh, "1", "Rust");
        assert_eq!(game_for(fresh).unwrap().game_name, "Rust");
        saw_game(fresh, "2", "Apex", Instant::now());
        assert_eq!(game_for(fresh).unwrap().game_name, "Apex");
    }

    #[test]
    fn the_place_takes_the_seen_game_over_the_heartbeats() {
        let earning = Earning {
            channel_id: "1".into(),
            login: "a".into(),
            game_id: "9".into(),
            game_name: "Old".into(),
        };
        assert_eq!(place_from(&earning, None).game_name, "Old");
        let seen = SeenGame {
            game_id: "".into(),
            game_name: "New".into(),
            at: Instant::now(),
        };
        let p = place_from(&earning, Some(&seen));
        assert_eq!((p.game_id.as_str(), p.game_name.as_str()), ("", "New"));
    }

    #[test]
    fn a_place_matches_by_id_else_by_login_and_game_by_id_else_name() {
        let place = Place {
            channel_id: "42".into(),
            login: "Caster".into(),
            game_id: "7".into(),
            game_name: "Rust".into(),
        };
        assert!(place_matches(&place, "42", "", "7", ""));
        assert!(place_matches(&place, "", "caster", "", "RUST"));
        assert!(
            !place_matches(&place, "43", "caster", "7", "Rust"),
            "an id that differs wins over the login"
        );
        assert!(
            !place_matches(&place, "42", "", "8", "Rust"),
            "ids that differ win over the name"
        );
        assert!(!place_matches(&place, "42", "", "", ""));
    }

    #[test]
    fn claims_clear_only_with_a_late_enough_read() {
        let t = Utc::now();
        let mut s = state_with("u1");
        s.claims.insert(
            "click".into(),
            Claim {
                state: ClaimState::Pending,
                at: t,
                by_loop: false,
            },
        );
        s.claims.insert(
            "loop".into(),
            Claim {
                state: ClaimState::Pending,
                at: t,
                by_loop: true,
            },
        );
        s.claims.insert(
            "done".into(),
            Claim {
                state: ClaimState::Claimed,
                at: t,
                by_loop: true,
            },
        );
        let (_, inv) = one_tier(20);
        let early = Arc::new(InvSnapshot {
            read_started_at: t + chrono::Duration::seconds(5),
            epoch: 1,
            inventory: inv.clone(),
            progress_overlay: HashMap::new(),
        });
        store_snapshot(&mut s, early);
        assert_eq!(
            s.claims.len(),
            3,
            "a read 5 s after the claims settles none"
        );
        let late = Arc::new(InvSnapshot {
            read_started_at: t + chrono::Duration::seconds(31),
            epoch: 1,
            inventory: inv,
            progress_overlay: HashMap::new(),
        });
        store_snapshot(&mut s, late);
        assert_eq!(
            s.claims.keys().map(String::as_str).collect::<Vec<_>>(),
            vec!["loop"],
            "the loop settles its own"
        );
    }

    #[test]
    fn a_pending_or_ungranted_tier_says_so() {
        let place = golden::place();
        let (list, mut inv) = one_tier(60);
        inv.items[0].campaign.time_based_drops[0]
            .progress
            .as_mut()
            .unwrap()
            .current_minutes_watched = 60;
        let mut claims = HashMap::new();
        claims.insert("t1".to_string(), ClaimState::Pending);
        let mut i = Inputs::bare(&place, &list, Some(&inv), now_ms());
        i.claims = &claims;
        let pending = build(&i);
        assert_eq!(pending.campaigns[0].drops[0].status, "Claiming");
        // The UI branches on the booleans: a claim in flight never offers Claim again.
        assert!(!pending.campaigns[0].drops[0].ready && pending.campaigns[0].drops[0].pending);
        let lead = pending.lead.as_ref().expect("the tier still leads");
        assert!(!lead.complete);
        assert_eq!(lead.status, "Claiming");
        let mut ungranted = HashSet::new();
        ungranted.insert("t1".to_string());
        let mut j = Inputs::bare(&place, &list, Some(&inv), now_ms());
        j.ungranted = &ungranted;
        let watched = build(&j);
        assert_eq!(watched.campaigns[0].drops[0].status, "Watched");
        assert!(!watched.campaigns[0].drops[0].ready && watched.campaigns[0].drops[0].watched);
        // Nothing else is left, so the lip still shows it rather than vanish.
        assert!(watched.lead.as_ref().is_some_and(|l| !l.complete && l.status == "Watched"));
    }

    #[test]
    fn a_stuck_reward_never_parks_the_lip_while_another_is_earning() {
        let place = golden::place();
        let (list, inv) = tiers(true, &[("a", 60, 60), ("b", 120, 70), ("c", 240, 70)]);
        assert_eq!(lead_of(&list, &inv, None), "a", "Ready leads");
        let mut claims = HashMap::new();
        claims.insert("a".to_string(), ClaimState::Pending);
        let mut i = Inputs::bare(&place, &list, Some(&inv), now_ms());
        i.claims = &claims;
        i.held_lead = Some("a");
        let view = build(&i);
        assert_eq!(view.lead.as_ref().unwrap().drop_id, "b", "a claim in flight hands the lip on");
        assert_eq!(view.campaigns[0].drops[0].status, "Claiming", "the drawer still says so");
        let mut ungranted = HashSet::new();
        ungranted.insert("a".to_string());
        let mut j = Inputs::bare(&place, &list, Some(&inv), now_ms());
        j.ungranted = &ungranted;
        j.held_lead = Some("a");
        let view = build(&j);
        assert_eq!(view.lead.as_ref().unwrap().drop_id, "b", "one not granted hands it on too");
        assert_eq!(view.campaigns[0].drops[0].status, "Watched");
        // The game account not linked: the finished one offers Connect in the
        // drawer while the lip follows the one still earning.
        let (mut list, mut inv) = (list.clone(), inv.clone());
        for c in list.iter_mut() {
            c.account_link = Some("https://game.example/link".into());
            c.is_account_connected = false;
        }
        inv.items[0].campaign.is_account_connected = false;
        let mut k = Inputs::bare(&place, &list, Some(&inv), now_ms());
        k.held_lead = Some("a");
        let view = build(&k);
        assert_eq!(view.campaigns[0].drops[0].status, "Connect");
        assert!(view.campaigns[0].drops[0].connect);
        assert_eq!(view.lead.as_ref().unwrap().drop_id, "b", "Connect never parks the lip");
    }

    /// One started campaign with a tier per (id, minutes needed, minutes in).
    fn tiers(separate: bool, spec: &[(&str, i32, i32)]) -> (Vec<DropCampaign>, InventoryResponse) {
        let (list, mut inv) = one_tier(0);
        let c = &mut inv.items[0].campaign;
        c.separate_progress = separate;
        let base = c.time_based_drops[0].clone();
        c.time_based_drops = spec
            .iter()
            .map(|(id, need, have)| {
                let mut d = base.clone();
                d.id = id.to_string();
                d.required_minutes_watched = *need;
                for b in &mut d.benefit_edges {
                    b.id = format!("benefit-{id}");
                    b.name = format!("Reward {id}");
                }
                let p = d.progress.as_mut().expect("a started tier");
                p.current_minutes_watched = *have;
                p.is_claimed = false;
                p.twitch_progress = None;
                d
            })
            .collect();
        (list, inv)
    }

    fn lead_of(list: &[DropCampaign], inv: &InventoryResponse, held: Option<&str>) -> String {
        let place = golden::place();
        let mut i = Inputs::bare(&place, list, Some(inv), now_ms());
        i.held_lead = held;
        build(&i).lead.expect("a lead").drop_id
    }

    #[test]
    fn the_lip_moves_to_the_next_reward_once_the_closest_is_claimed() {
        let place = golden::place();
        let (list, inv) = tiers(false, &[("t1", 30, 40), ("t2", 60, 40), ("t3", 90, 40)]);
        assert_eq!(lead_of(&list, &inv, None), "t1", "the finished tier leads");
        let mut claims = HashMap::new();
        claims.insert("t1".to_string(), ClaimState::Claimed);
        let mut i = Inputs::bare(&place, &list, Some(&inv), now_ms());
        i.claims = &claims;
        i.held_lead = Some("t1");
        let view = build(&i);
        let lead = view.lead.expect("the next tier leads");
        assert_eq!(lead.drop_id, "t2", "the closest one still to earn");
        assert!(lead.line.ends_with("2 of 3"), "{}", lead.line);
    }

    #[test]
    fn a_tucked_lip_comes_back_only_for_something_worth_showing() {
        let place = golden::place();
        let view_of = |spec: &[(&str, i32, i32)], held: Option<&str>| {
            let (list, inv) = tiers(true, spec);
            let mut i = Inputs::bare(&place, &list, Some(&inv), now_ms());
            i.held_lead = held;
            build(&i)
        };
        let earning = view_of(&[("a", 60, 30), ("b", 120, 30)], None);
        let lead = earning.lead.as_ref().unwrap();
        let t = Tucked {
            campaign_id: lead.campaign_id.clone(),
            drop_id: lead.drop_id.clone(),
            channel: channel_key(&earning),
        };
        assert!(tuck_holds(&t, &earning));
        // Minutes ticking up keep it tucked.
        assert!(tuck_holds(&t, &view_of(&[("a", 60, 45), ("b", 120, 45)], Some("a"))));
        // Ready to claim brings it back.
        assert!(!tuck_holds(&t, &view_of(&[("a", 60, 60), ("b", 120, 60)], Some("a"))));
        // The next reward starting brings it back.
        let mut claims = HashMap::new();
        claims.insert("a".to_string(), ClaimState::Claimed);
        let (list, inv) = tiers(true, &[("a", 60, 60), ("b", 120, 60)]);
        let mut i = Inputs::bare(&place, &list, Some(&inv), now_ms());
        i.claims = &claims;
        assert!(!tuck_holds(&t, &build(&i)));
        // Another channel's drops bring it back.
        let mut elsewhere = earning.clone();
        elsewhere.channel_id = "someone-else".into();
        assert!(!tuck_holds(&t, &elsewhere));
        // Nothing left to show lets go too.
        let mut empty = earning.clone();
        empty.lead = None;
        assert!(!tuck_holds(&t, &empty));
    }

    #[test]
    fn rewards_finishing_together_keep_the_first_on_the_lip() {
        let (list, inv) = tiers(true, &[("a", 60, 50), ("b", 60, 50)]);
        assert_eq!(lead_of(&list, &inv, None), "a", "a tie goes to the first");
        assert_eq!(lead_of(&list, &inv, Some("b")), "b", "a tie keeps the one shown");
        // The second one's minute lands a moment before the first's.
        let (list, inv) = tiers(true, &[("a", 60, 50), ("b", 60, 51)]);
        assert_eq!(lead_of(&list, &inv, Some("a")), "a", "a minute apart never trades places");
        assert_eq!(lead_of(&list, &inv, None), "b", "with nothing shown, the closest leads");
        // Truly further along: the lip follows it.
        let (list, inv) = tiers(true, &[("a", 60, 50), ("b", 60, 52)]);
        assert_eq!(lead_of(&list, &inv, Some("a")), "b");
        // Both finish together: the first, or the one already shown.
        let (list, inv) = tiers(true, &[("a", 60, 60), ("b", 60, 60)]);
        assert_eq!(lead_of(&list, &inv, None), "a");
        assert_eq!(lead_of(&list, &inv, Some("b")), "b");
        // A finished one always takes the lip from one still earning.
        let (list, inv) = tiers(true, &[("a", 60, 59), ("b", 60, 60)]);
        assert_eq!(lead_of(&list, &inv, Some("a")), "b");
        // One running count: equal tiers finish together, the first leads.
        let (list, inv) = tiers(false, &[("a", 60, 45), ("b", 60, 45), ("c", 120, 45)]);
        assert_eq!(lead_of(&list, &inv, None), "a");
        assert_eq!(lead_of(&list, &inv, Some("c")), "a", "a held tier far behind lets go");
    }

    fn body_of<'a>(source: &'a str, signature: &str) -> &'a str {
        let start = source
            .find(signature)
            .unwrap_or_else(|| panic!("{signature} not found"));
        let rest = &source[start + signature.len()..];
        // The body ends where the next item starts, at either indentation
        // (methods in an impl, or free fns).
        let end = [
            "\n    pub ",
            "\n    async fn ",
            "\n    fn ",
            "\npub ",
            "\nasync fn ",
            "\nfn ",
            "\n#[",
        ]
        .iter()
        .filter_map(|marker| rest.find(marker))
        .min()
        .unwrap_or(rest.len());
        &rest[..end]
    }

    #[test]
    fn every_writer_the_model_follows_wakes_it() {
        // The sources may carry CRLF line endings; the needles below are LF.
        let heartbeat = include_str!("watch_heartbeat_service.rs").replace("\r\n", "\n");
        let heartbeat = heartbeat.as_str();
        for sig in [
            "pub async fn set_target(",
            "pub async fn clear_target(",
            "pub fn set_playing(",
        ] {
            assert!(
                body_of(heartbeat, sig).contains("watch_drops::poke()"),
                "heartbeat {sig}"
            );
        }
        assert!(
            heartbeat.contains("watch_drops::saw_game("),
            "the heartbeat's stream info"
        );
        let session = include_str!("watch_session.rs").replace("\r\n", "\n");
        let session = session.as_str();
        assert!(body_of(session, "async fn begin(").contains("watch_drops::poke()"));
        assert!(body_of(session, "async fn begin(").contains("watch_drops::seed_game("));
        assert!(body_of(session, "pub async fn stop(").contains("watch_drops::poke()"));
        assert!(
            session.contains("fn take_if(")
                && session.contains(
                    "if taken.is_some() {\n        crate::services::watch_drops::poke();"
                )
        );
        assert!(session.contains("pub fn on_channel_update(app: &AppHandle, update: &ChannelUpdateEvent) {\n    crate::services::watch_drops::saw_game("));
        let auth = include_str!("drops_auth_service.rs").replace("\r\n", "\n");
        let auth = auth.as_str();
        for sig in ["fn store_token_to_file(", "fn delete_token_file("] {
            assert!(
                body_of(auth, sig).contains("watch_drops::drops_auth_changed()"),
                "{sig}"
            );
        }
    }

    #[test]
    fn the_model_serializes_the_contracts_keys() {
        let keys = |v: serde_json::Value| -> Vec<String> {
            let mut k: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
            k.sort();
            k
        };
        let snap = serde_json::to_value(WatchDropsSnapshot::default()).unwrap();
        assert_eq!(keys(snap.clone()), vec!["rev", "view"]);
        assert_eq!(
            keys(snap["view"].clone()),
            vec![
                "campaigns",
                "channel_id",
                "complete",
                "game_id",
                "game_name",
                "lead",
                "lip_hidden",
                "login",
                "more",
                "more_art",
                "stall_episode",
                "stalled"
            ]
        );
        let place = golden::place();
        let case = golden::cases()
            .into_iter()
            .find(|c| c.name == "shuffled_order_and_a_zeroed_claimed_tier")
            .unwrap();
        let view = serde_json::to_value(build(&Inputs::bare(
            &place,
            &case.list,
            Some(&case.inventory),
            now_ms(),
        )))
        .unwrap();
        assert_eq!(
            keys(view["campaigns"][0].clone()),
            vec![
                "art",
                "box_art",
                "claimed_label",
                "cumulative_minutes",
                "drops",
                "ends",
                "game_name",
                "id",
                "link_url",
                "name",
                "started"
            ]
        );
        assert_eq!(
            keys(view["campaigns"][0]["drops"][0].clone()),
            vec![
                "bar",
                "claim_label",
                "claimed",
                "connect",
                "current_minutes",
                "detail",
                "id",
                "image",
                "instance_id",
                "lead",
                "locked",
                "name",
                "pending",
                "ready",
                "required_minutes",
                "status",
                "watched"
            ]
        );
    }
}
