//! The chat's "emote of the moment": the emote the most different chatters
//! used in a room over the last minute, which the composer's emote button
//! wears as its face (the deck, `src/components/chat/EmoteDeckButton.tsx`).
//!
//! Every row a window receives is recorded by `ChatRules::evaluate_into_history`,
//! for every platform, and that is where [`observe`] is fed. The hot path stays
//! cheap: a row with no countable emote never takes the lock, a room ranks at
//! most once per [`RANK_EVERY_MS`], and only a change of leader is sent
//! (`emote-moment`, global; each window keeps its own room by the key
//! [`get`] gave it). A quiet room's moment ends through a sweep that runs only
//! while some room holds uses.
//!
//! Time is this process's own monotonic clock, taken as a row arrives: a
//! platform's timestamp is not a common clock (Kick, YouTube and TikTok rows
//! carry RFC 3339 text, Twitch's `tmi-sent-ts` is Twitch's clock, not this
//! PC's). History replayed into a chat (`from_backfill`, Twitch's
//! `historical=1`) says nothing about right now and is not counted.
//!
//! A chatter counts once per emote and a row counts each emote once, so one
//! person spamming never makes the moment; it takes [`MIN_CHATTERS`] people.
//! The face is meant to be calm (Brandon 2026-11-19: "it flips way too much
//! and is just too in your face"): an emote keeps the face at least
//! [`MIN_HOLD_MS`] once it takes it, whatever the room does meanwhile, and a
//! new emote takes over only with a clear lead ([`takeover_needs`]: at least
//! [`TAKEOVER_MARGIN`] more chatters, and half again as many in a busy room),
//! so two close emotes never swap the face back and forth.
//! The face is a picture of the room, not an insert shortcut (pressing the
//! button opens the menu), so an emote the viewer cannot send still counts.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use serde::Serialize;

use crate::models::chat_layout::{ChatMessage, MessageSegment};
use crate::services::chat_rules::F_SYSTEM;

/// Tauri event carrying a [`MomentAnswer`] whenever a room's moment changes.
pub const EVENT: &str = "emote-moment";

/// How far back "right now" reaches.
pub const WINDOW_MS: i64 = 60_000;
/// Distinct chatters an emote needs before it is the moment.
pub const MIN_CHATTERS: usize = 3;
/// The fewest more chatters a challenger needs than the emote shown.
pub const TAKEOVER_MARGIN: usize = 2;
/// How long an emote keeps the face once it takes it, before another emote
/// (or a quiet room's smiley) may replace it.
pub const MIN_HOLD_MS: i64 = 30_000;
/// A room ranks its uses at most this often while rows arrive.
pub const RANK_EVERY_MS: i64 = 1_500;
/// (emote, chatter) pairs a room keeps after a rank; past it the emotes with
/// the fewest chatters go first.
pub const MAX_PAIRS: usize = 6_000;
/// Distinct emotes read from one row.
const MAX_PER_ROW: usize = 8;
/// How often rooms are ranked while no rows arrive.
const SWEEP_EVERY: Duration = Duration::from_secs(5);

/// The emote a room's button wears.
#[derive(Serialize, Clone, PartialEq, Debug)]
pub struct MomentEmote {
    pub name: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

/// A room's moment as the page reads it. `seq` grows with every change of any
/// room, so a window applies an answer only when it is newer than the one it
/// holds (a `get` answer and an event can cross).
#[derive(Serialize, Clone, PartialEq, Debug)]
pub struct MomentAnswer {
    pub key: String,
    pub seq: u64,
    pub emote: Option<MomentEmote>,
}

#[derive(Default)]
struct Room {
    /// Emote name -> chatter -> when that chatter last used it.
    seen: HashMap<Arc<str>, HashMap<u64, i64>>,
    info: HashMap<Arc<str>, MomentEmote>,
    leader: Option<Arc<str>>,
    /// When the shown emote took the face.
    leader_since: i64,
    last_rank_ms: i64,
    seq: u64,
}

/// Chatters a challenger needs to take the face from an emote `held` chatters
/// use: [`TAKEOVER_MARGIN`] more, or half again as many when that is more, so
/// a busy room's leader is not unseated by a couple of chatters.
pub fn takeover_needs(held: usize) -> usize {
    held + TAKEOVER_MARGIN.max(held / 2)
}

static SEQ: AtomicU64 = AtomicU64::new(0);
static SWEEPING: AtomicBool = AtomicBool::new(false);

fn rooms() -> MutexGuard<'static, HashMap<String, Room>> {
    static ROOMS: OnceLock<Mutex<HashMap<String, Room>>> = OnceLock::new();
    ROOMS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// Milliseconds since this process started, never going back. The monotonic
/// clock stops while macOS, Linux and Android sleep, so the wall clock's
/// elapsed time is taken when it is ahead (a laptop or phone that slept for
/// an hour must not count the minute before it slept as "right now").
fn now_ms() -> i64 {
    static START: OnceLock<(Instant, SystemTime)> = OnceLock::new();
    static LAST: AtomicI64 = AtomicI64::new(0);
    let (mono, wall) = *START.get_or_init(|| (Instant::now(), SystemTime::now()));
    let mono_ms = mono.elapsed().as_millis() as i64;
    let wall_ms = SystemTime::now().duration_since(wall).map(|d| d.as_millis() as i64).unwrap_or(0);
    // Offset by a window so a fresh process never reads a negative cutoff.
    let now = mono_ms.max(wall_ms) + WINDOW_MS;
    LAST.fetch_max(now, Ordering::AcqRel).max(now)
}

/// The room key for a chat the page names by `(provider, channel)`, as its
/// emote matching does: the history ring's own key (a Twitch login bare, any
/// other platform composite), lowercased as rows are filed.
pub fn room_key(provider: &str, channel: &str) -> String {
    crate::services::emote_match::history_room(provider, channel)
        .trim_start_matches('#')
        .to_lowercase()
}

/// The countable emotes of one row: room and global emotes, not zero-width
/// overlays, FFZ modifiers or a sender's personal 7TV emotes. Each name once.
fn row_emotes(msg: &ChatMessage) -> Vec<(&str, &str, Option<&str>)> {
    let mut out: Vec<(&str, &str, Option<&str>)> = Vec::new();
    for seg in &msg.segments {
        if let MessageSegment::Emote { content, emote_id, emote_url, is_zero_width, modifier_flags, is_personal, .. } = seg {
            if *is_zero_width == Some(true) || modifier_flags.is_some() || *is_personal == Some(true) {
                continue;
            }
            if content.is_empty() || emote_url.is_empty() || out.iter().any(|(n, _, _)| *n == content.as_str()) {
                continue;
            }
            out.push((content.as_str(), emote_url.as_str(), emote_id.as_deref()));
            if out.len() >= MAX_PER_ROW {
                break;
            }
        }
    }
    out
}

fn chatter_of(msg: &ChatMessage) -> u64 {
    let mut h = DefaultHasher::new();
    if msg.user_id.is_empty() {
        msg.username.to_lowercase().hash(&mut h);
    } else {
        msg.user_id.hash(&mut h);
    }
    h.finish()
}

/// Whether a row is history replayed into the chat rather than said now.
fn is_history(msg: &ChatMessage) -> bool {
    msg.metadata.from_backfill || msg.tags.get("historical").is_some_and(|v| v == "1")
}

/// Record one row. Called for every recorded row; returns at once unless the
/// row holds a countable emote and was said just now.
pub fn observe(channel_key: &str, msg: &ChatMessage, flags: u32) {
    if let Some(update) = observe_at(channel_key, msg, flags, now_ms()) {
        emit(update);
    }
}

fn observe_at(channel_key: &str, msg: &ChatMessage, flags: u32, now: i64) -> Option<MomentAnswer> {
    // Shared Chat rows count: they are in the chat you see.
    if channel_key.is_empty() || flags & F_SYSTEM != 0 || is_history(msg) {
        return None;
    }
    let emotes = row_emotes(msg);
    if emotes.is_empty() {
        return None;
    }
    let chatter = chatter_of(msg);
    let mut map = rooms();
    // The key is copied only for a room's first row.
    if !map.contains_key(channel_key) {
        map.insert(channel_key.to_string(), Room::default());
    }
    let room = map.get_mut(channel_key)?;
    for (name, url, id) in emotes {
        let key: Arc<str> = match room.info.get_key_value(name) {
            Some((k, info)) => {
                let k = k.clone();
                if info.url != url {
                    room.info.insert(k.clone(), moment_emote(name, url, id));
                }
                k
            }
            None => {
                let k: Arc<str> = Arc::from(name);
                room.info.insert(k.clone(), moment_emote(name, url, id));
                k
            }
        };
        room.seen.entry(key).or_default().insert(chatter, now);
    }
    let update = if now - room.last_rank_ms >= RANK_EVERY_MS { rank(channel_key, room, now) } else { None };
    // Under the lock, which the sweep also holds when it ends.
    if !SWEEPING.swap(true, Ordering::AcqRel) && !start_sweep() {
        SWEEPING.store(false, Ordering::Release);
    }
    update
}

fn moment_emote(name: &str, url: &str, id: Option<&str>) -> MomentEmote {
    MomentEmote { name: name.to_string(), url: url.to_string(), id: id.map(str::to_string) }
}

/// Rank a room's recent uses; `Some` when its leader changed.
fn rank(key: &str, room: &mut Room, now: i64) -> Option<MomentAnswer> {
    room.last_rank_ms = now;
    let since = now - WINDOW_MS;
    room.seen.retain(|_, who| {
        who.retain(|_, at| *at >= since);
        !who.is_empty()
    });
    // Past the cap, the emotes with the fewest chatters go first.
    let pairs: usize = room.seen.values().map(HashMap::len).sum();
    if pairs > MAX_PAIRS {
        let mut sizes: Vec<(Arc<str>, usize)> = room.seen.iter().map(|(k, v)| (k.clone(), v.len())).collect();
        sizes.sort_by_key(|(_, n)| *n);
        let mut over = pairs - MAX_PAIRS;
        for (name, n) in sizes {
            if over == 0 {
                break;
            }
            room.seen.remove(&name);
            over = over.saturating_sub(n);
        }
    }
    let seen = &room.seen;
    // The emote shown keeps its picture while it holds the face, even if its
    // uses have aged out meanwhile.
    let shown = room.leader.as_ref();
    room.info.retain(|k, _| seen.contains_key(k) || shown == Some(k));
    // Within its hold the shown emote stays, whatever the room does.
    if room.leader.is_some() && now - room.leader_since < MIN_HOLD_MS {
        return None;
    }
    let last_use = |who: &HashMap<u64, i64>| who.values().copied().max().unwrap_or(0);
    let best = room
        .seen
        .iter()
        .filter(|(_, who)| who.len() >= MIN_CHATTERS)
        .max_by(|a, b| (a.1.len(), last_use(a.1)).cmp(&(b.1.len(), last_use(b.1))))
        .map(|(name, who)| (name.clone(), who.len()));
    let held = room.leader.as_ref().and_then(|l| room.seen.get(l)).map(HashMap::len).filter(|n| *n >= MIN_CHATTERS);
    let next = match (held, best) {
        // The emote shown keeps its place until another clearly leads.
        (Some(n), Some((_, m))) if m < takeover_needs(n) => room.leader.clone(),
        (Some(_), None) => room.leader.clone(),
        (_, best) => best.map(|(name, _)| name),
    };
    if next == room.leader {
        return None;
    }
    if next.is_some() {
        room.leader_since = now;
    }
    if let Some(old) = room.leader.as_ref().filter(|old| !room.seen.contains_key(*old)) {
        room.info.remove(old);
    }
    room.leader = next;
    room.seq = SEQ.fetch_add(1, Ordering::AcqRel) + 1;
    Some(answer(key, room))
}

fn answer(key: &str, room: &Room) -> MomentAnswer {
    MomentAnswer {
        key: key.to_string(),
        seq: room.seq,
        emote: room.leader.as_ref().and_then(|l| room.info.get(l).cloned()),
    }
}

/// The moment of the chat the page names by `(provider, channel)`, ranked now
/// (a room that went quiet answers none at once).
pub fn get(provider: &str, channel: &str) -> MomentAnswer {
    let key = room_key(provider, channel);
    let (out, changed) = get_at(&key, now_ms());
    if changed {
        emit(out.clone());
    }
    out
}

fn get_at(key: &str, now: i64) -> (MomentAnswer, bool) {
    let mut map = rooms();
    match map.get_mut(key) {
        Some(room) => match rank(key, room, now) {
            Some(update) => (update, true),
            None => (answer(key, room), false),
        },
        None => (MomentAnswer { key: key.to_string(), seq: 0, emote: None }, false),
    }
}

/// Rank every room; forget rooms with nothing left. `true` while rooms remain.
fn sweep_map(map: &mut HashMap<String, Room>, now: i64, out: &mut Vec<MomentAnswer>) -> bool {
    for (key, room) in map.iter_mut() {
        if let Some(update) = rank(key, room, now) {
            out.push(update);
        }
    }
    map.retain(|_, room| !room.seen.is_empty() || room.leader.is_some());
    !map.is_empty()
}

/// Lowers the sweep flag if the sweep task ends any other way than its own
/// decision (a panic or an abort), under the rooms lock, so the next row
/// starts a new sweep instead of trusting a dead one.
struct SweepGuard {
    armed: bool,
}

impl Drop for SweepGuard {
    fn drop(&mut self) {
        if self.armed {
            let _map = rooms();
            SWEEPING.store(false, Ordering::Release);
        }
    }
}

/// One sweep step: rank and forget under the lock, and lower the flag in the
/// same hold when nothing is left, so a row arriving right after sees it down
/// and starts the next sweep. `true` while rooms remain.
fn sweep_step(now: i64, out: &mut Vec<MomentAnswer>) -> bool {
    let mut map = rooms();
    let more = sweep_map(&mut map, now, out);
    if !more {
        SWEEPING.store(false, Ordering::Release);
    }
    more
}

/// Start the sweep; `false` where there is no app to run it on (unit tests).
fn start_sweep() -> bool {
    if crate::services::providers::app_handle().is_none() {
        return false;
    }
    tauri::async_runtime::spawn(async {
        let mut guard = SweepGuard { armed: true };
        loop {
            tokio::time::sleep(SWEEP_EVERY).await;
            let mut changes = Vec::new();
            let more = sweep_step(now_ms(), &mut changes);
            if !more {
                guard.armed = false;
            }
            for update in changes {
                emit(update);
            }
            if !more {
                break;
            }
        }
    });
    true
}

fn emit(update: MomentAnswer) {
    use tauri::Emitter;
    if let Some(app) = crate::services::providers::app_handle() {
        if let Err(e) = app.emit(EVENT, &update) {
            log::debug!("[EmoteMoment] emit failed: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(user: &str, emotes: &[&str]) -> ChatMessage {
        let mut m = ChatMessage {
            id: format!("{user}-{}", emotes.join("-")),
            user_id: user.into(),
            username: user.into(),
            display_name: user.into(),
            color: None,
            badges: vec![],
            timestamp: "2026-11-13T10:00:00Z".into(),
            content: emotes.join(" "),
            provider: "twitch".into(),
            channel: "#test".into(),
            received_at_ms: None,
            emotes: vec![],
            tags: HashMap::new(),
            layout: Default::default(),
            segments: vec![],
            metadata: Default::default(),
        };
        m.segments = emotes
            .iter()
            .map(|e| MessageSegment::Emote {
                content: e.to_string(),
                emote_id: Some(format!("id-{e}")),
                emote_url: format!("https://cdn/{e}.webp"),
                is_zero_width: None,
                modifier_flags: None,
                is_personal: None,
                local_path: None,
            })
            .collect();
        m
    }

    /// Feed rows to room `key` at `now`, ranking on every row (the throttle
    /// is a clock matter, tested on its own).
    fn feed(key: &str, rows: &[(&str, &[&str])], now: i64) -> Option<MomentAnswer> {
        let mut last = None;
        for (user, emotes) in rows {
            if let Some(room) = rooms().get_mut(key) {
                room.last_rank_ms = i64::MIN / 2;
            }
            if let Some(u) = observe_at(key, &row(user, emotes), 0, now) {
                last = Some(u);
            }
        }
        last
    }

    fn leader(key: &str, now: i64) -> Option<String> {
        get_at(key, now).0.emote.map(|e| e.name)
    }

    fn reset(key: &str) {
        rooms().remove(key);
    }

    #[test]
    fn three_chatters_make_the_moment() {
        let k = "test-three";
        reset(k);
        let now = 1_000_000;
        feed(k, &[("a", &["KEKW"]), ("b", &["KEKW"])], now);
        assert_eq!(leader(k, now), None, "two chatters are not a moment");
        let update = feed(k, &[("c", &["KEKW"])], now).expect("the third chatter makes it");
        assert_eq!(update.key, k);
        assert_eq!(update.emote.as_ref().map(|e| e.url.as_str()), Some("https://cdn/KEKW.webp"));
        assert_eq!(leader(k, now).as_deref(), Some("KEKW"));
    }

    #[test]
    fn one_spammer_never_wins() {
        let k = "test-spam";
        reset(k);
        let now = 2_000_000;
        let spam: Vec<(&str, &[&str])> = (0..40).map(|_| ("spammer", &["OMEGALUL"][..])).collect();
        feed(k, &spam, now);
        assert_eq!(leader(k, now), None);
    }

    #[test]
    fn moment_decays_after_the_window() {
        let k = "test-decay";
        reset(k);
        let now = 3_000_000;
        feed(k, &[("a", &["Pog"]), ("b", &["Pog"]), ("c", &["Pog"])], now);
        assert_eq!(leader(k, now).as_deref(), Some("Pog"));
        assert_eq!(leader(k, now + WINDOW_MS - 1).as_deref(), Some("Pog"));
        let (gone, changed) = get_at(k, now + WINDOW_MS + 1);
        assert!(changed, "the decay is a change the windows hear");
        assert_eq!(gone.emote, None);
    }

    #[test]
    fn the_shown_emote_holds_until_another_clearly_leads() {
        let k = "test-margin";
        reset(k);
        let now = 4_000_000;
        feed(k, &[("a", &["LUL"]), ("b", &["LUL"]), ("c", &["LUL"])], now);
        // Past the hold, so only the lead decides.
        let later = now + MIN_HOLD_MS;
        feed(k, &[("d", &["Kappa"]), ("e", &["Kappa"]), ("f", &["Kappa"]), ("g", &["Kappa"])], later);
        assert_eq!(leader(k, later).as_deref(), Some("LUL"), "one more chatter does not take over");
        feed(k, &[("h", &["Kappa"])], later + 10);
        assert_eq!(leader(k, later + 10).as_deref(), Some("Kappa"), "two more chatters do");
    }

    #[test]
    fn a_new_face_holds_a_while_whatever_the_room_does() {
        let k = "test-hold";
        reset(k);
        let now = 4_500_000;
        feed(k, &[("a", &["LUL"]), ("b", &["LUL"]), ("c", &["LUL"])], now);
        let crowd: Vec<String> = (0..20).map(|i| format!("p{i}")).collect();
        let kappa: Vec<(&str, &[&str])> = crowd.iter().map(|u| (u.as_str(), &["Kappa"][..])).collect();
        assert!(feed(k, &kappa, now + 1_000).is_none(), "no change inside the hold, however clear the lead");
        assert_eq!(leader(k, now + MIN_HOLD_MS - 1).as_deref(), Some("LUL"));
        let (next, changed) = get_at(k, now + MIN_HOLD_MS);
        assert!(changed, "the change comes as the hold ends");
        assert_eq!(next.emote.map(|e| e.name).as_deref(), Some("Kappa"));
        // The new face holds as long in turn.
        let flood: Vec<String> = (0..40).map(|i| format!("q{i}")).collect();
        let omega: Vec<(&str, &[&str])> = flood.iter().map(|u| (u.as_str(), &["OMEGALUL"][..])).collect();
        feed(k, &omega, now + MIN_HOLD_MS + 1_000);
        assert_eq!(leader(k, now + 2 * MIN_HOLD_MS - 1).as_deref(), Some("Kappa"));
        assert_eq!(leader(k, now + 2 * MIN_HOLD_MS).as_deref(), Some("OMEGALUL"));
    }

    #[test]
    fn a_busy_room_needs_a_bigger_lead() {
        assert_eq!(takeover_needs(3), 5, "a small room: two more chatters");
        assert_eq!(takeover_needs(4), 6);
        assert_eq!(takeover_needs(20), 30, "a busy room: half again as many");
        assert_eq!(takeover_needs(200), 300);
    }

    #[test]
    fn a_held_face_keeps_its_picture_after_its_uses_age_out() {
        let k = "test-hold-picture";
        reset(k);
        let now = 4_800_000;
        feed(k, &[("a", &["Clap"]), ("b", &["Clap"])], now);
        // The third chatter late in the window makes it, with the first two
        // about to age out.
        feed(k, &[("c", &["Clap"])], now + WINDOW_MS - 1_000);
        let taken = now + WINDOW_MS - 1_000;
        let (held, _) = get_at(k, now + WINDOW_MS + 1);
        assert_eq!(held.emote.map(|e| e.url).as_deref(), Some("https://cdn/Clap.webp"), "still drawn while it holds");
        let (gone, changed) = get_at(k, taken + MIN_HOLD_MS);
        assert!(changed);
        assert_eq!(gone.emote, None, "the quiet room's smiley comes after the hold");
    }

    #[test]
    fn a_chatter_counts_once_per_emote_however_often() {
        let k = "test-repeat";
        reset(k);
        let now = 4_500_000;
        feed(k, &[("a", &["Clap"]), ("a", &["Clap"]), ("b", &["Clap"]), ("b", &["Clap"])], now);
        assert_eq!(leader(k, now), None);
        feed(k, &[("c", &["Clap"])], now);
        assert_eq!(leader(k, now).as_deref(), Some("Clap"));
    }

    #[test]
    fn overlays_personal_and_system_rows_do_not_count() {
        let k = "test-overlay";
        reset(k);
        let now = 5_000_000;
        for user in ["a", "b", "c"] {
            let mut m = row(user, &["cvHazmat"]);
            if let MessageSegment::Emote { is_zero_width, .. } = &mut m.segments[0] {
                *is_zero_width = Some(true);
            }
            observe_at(k, &m, 0, now);
            let mut p = row(user, &["MyOwn"]);
            if let MessageSegment::Emote { is_personal, .. } = &mut p.segments[0] {
                *is_personal = Some(true);
            }
            observe_at(k, &p, 0, now);
            let mut f = row(user, &["ffzX"]);
            if let MessageSegment::Emote { modifier_flags, .. } = &mut f.segments[0] {
                *modifier_flags = Some(1);
            }
            observe_at(k, &f, 0, now);
            observe_at(k, &row(user, &["Sys"]), F_SYSTEM, now);
        }
        assert_eq!(leader(k, now), None);
    }

    #[test]
    fn shared_chat_rows_count() {
        let k = "test-shared";
        reset(k);
        let now = 5_500_000;
        for user in ["a", "b", "c"] {
            observe_at(k, &row(user, &["Partner"]), crate::services::chat_rules::F_SHARED, now);
        }
        assert_eq!(leader(k, now).as_deref(), Some("Partner"));
    }

    #[test]
    fn history_counts_nothing() {
        let k = "test-history";
        reset(k);
        let now = 6_000_000;
        for user in ["a", "b", "c"] {
            let mut m = row(user, &["PepeLaugh"]);
            m.metadata.from_backfill = true;
            observe_at(k, &m, 0, now);
            let mut h = row(user, &["PepeLaugh"]);
            h.tags.insert("historical".into(), "1".into());
            observe_at(k, &h, 0, now);
        }
        assert_eq!(leader(k, now), None);
    }

    #[test]
    fn rows_without_a_numeric_timestamp_count() {
        // Kick, YouTube and TikTok rows carry RFC 3339 text and no
        // `tmi-sent-ts`, so the rules' `ts_ms` is 0 for them: the count must
        // not depend on it.
        let k = "test-kick";
        reset(k);
        let now = 6_500_000;
        for user in ["a", "b", "c"] {
            let mut m = row(user, &["catJAM"]);
            m.provider = "kick".into();
            m.channel = "kick:xqc".into();
            assert_eq!(crate::services::chat_rules::MessageFacts::from_message(&m).ts_ms, 0);
            observe_at(k, &m, 0, now);
        }
        assert_eq!(leader(k, now).as_deref(), Some("catJAM"));
    }

    #[test]
    fn a_busy_room_keeps_a_whole_window_within_bounds() {
        let k = "test-bound";
        reset(k);
        let now = 7_000_000;
        // Early in the window: one emote from four chatters.
        feed(k, &[("a", &["Early"]), ("b", &["Early"]), ("c", &["Early"]), ("d", &["Early"])], now);
        // Then a flood of one-off emotes from thousands of chatters.
        for i in 0..(MAX_PAIRS * 2) {
            observe_at(k, &row(&format!("u{i}"), &[&format!("e{i}")]), 0, now + 1_000);
        }
        let (a, _) = get_at(k, now + 2_000);
        assert_eq!(a.emote.map(|e| e.name).as_deref(), Some("Early"), "the leader from early in the window is still counted");
        let map = rooms();
        let pairs: usize = map.get(k).map(|r| r.seen.values().map(HashMap::len).sum()).unwrap_or(0);
        assert!(pairs <= MAX_PAIRS, "pairs stay bounded: {pairs}");
    }

    #[test]
    fn ranking_waits_between_rows() {
        let k = "test-throttle";
        reset(k);
        let now = 7_500_000;
        observe_at(k, &row("a", &["Wait"]), 0, now);
        observe_at(k, &row("b", &["Wait"]), 0, now + 10);
        assert!(observe_at(k, &row("c", &["Wait"]), 0, now + 20).is_none(), "no rank inside the throttle");
        assert!(observe_at(k, &row("d", &["Wait"]), 0, now + RANK_EVERY_MS).is_some(), "the next rank names it");
    }

    #[test]
    fn key_matches_the_history_ring() {
        // Rows are filed under `msg.channel` without '#', lowercased
        // (`MessageFacts::channel_key`); the page names a chat by provider
        // and channel as its emote matching does.
        assert_eq!(room_key("twitch", "Forsen"), "forsen");
        assert_eq!(room_key("twitch", "#forsen"), "forsen");
        assert_eq!(room_key("kick", "XQC"), "kick:xqc");
        assert_eq!(room_key("kick", "kick:xqc"), "kick:xqc");
        assert_eq!(room_key("youtube", "UCabcDEF"), "youtube:ucabcdef");
        assert_eq!(room_key("tiktok", "SomeOne"), "tiktok:someone");
        let facts_key = |provider: &str, channel: &str| {
            let mut m = row("a", &["x"]);
            m.provider = provider.into();
            m.channel = channel.into();
            crate::services::chat_rules::MessageFacts::from_message(&m).channel_key
        };
        assert_eq!(facts_key("twitch", "#Forsen"), room_key("twitch", "forsen"));
        assert_eq!(facts_key("kick", "kick:xqc"), room_key("kick", "xqc"));
        assert_eq!(facts_key("youtube", "youtube:UCabcDEF"), room_key("youtube", "UCabcDEF"));
        assert_eq!(facts_key("tiktok", "tiktok:someone"), room_key("tiktok", "SomeOne"));
    }

    #[test]
    fn a_quiet_room_is_forgotten_by_the_sweep() {
        let k = "test-sweep";
        reset(k);
        let now = 8_000_000;
        feed(k, &[("a", &["EZ"]), ("b", &["EZ"]), ("c", &["EZ"])], now);
        // The sweep ranks every room, so it runs on this room alone: the
        // other tests' rooms share the process-wide map.
        let room = rooms().remove(k).expect("the room");
        let mut map = HashMap::from([(k.to_string(), room)]);
        let mut out = Vec::new();
        assert!(sweep_map(&mut map, now + 1_000, &mut out), "a room with a moment stays");
        assert!(out.is_empty());
        assert!(!sweep_map(&mut map, now + WINDOW_MS + 1, &mut out), "nothing left to sweep: the task ends");
        assert!(out.iter().any(|u| u.key == k && u.emote.is_none()), "the sweep tells the windows it ended");
        assert!(map.is_empty(), "and forgets the room");
    }

    #[test]
    fn the_answer_matches_the_shared_fixture() {
        // The page parses the same file (src/hooks/useEmoteMoment.test.ts).
        let answer = MomentAnswer {
            key: "kick:xqc".into(),
            seq: 7,
            emote: Some(MomentEmote {
                name: "catJAM".into(),
                url: "https://cdn.7tv.app/emote/60ae7316f7c927fad14e6ca2/2x.avif".into(),
                id: Some("60ae7316f7c927fad14e6ca2".into()),
            }),
        };
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("fixtures/emote_moment_answer.json")).expect("the fixture parses");
        assert_eq!(serde_json::to_value(&answer).unwrap(), fixture);
    }

    #[test]
    fn the_sweep_flag_comes_down_however_the_sweep_ends() {
        SWEEPING.store(true, Ordering::Release);
        drop(SweepGuard { armed: true });
        assert!(!SWEEPING.load(Ordering::Acquire));
    }

    #[test]
    fn the_clock_never_goes_back() {
        let a = now_ms();
        let b = now_ms();
        assert!(b >= a);
        assert!(a >= WINDOW_MS);
    }
}
