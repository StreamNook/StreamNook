//! What each rule remembers between fires on this device: how often it
//! acted, when it last did, where each action is in its lines (in order, or
//! its shuffle bag), and the badge drops it posted. Kept in
//! `automation_counts.json`; fields a newer build added are kept as they are.
//! Pure: the caller passes the time and the random number.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::badges::BadgePhase;
use crate::models::automations::Pick;

pub const COUNTS_VERSION: u32 = 1;
/// A posted badge drop is remembered this long...
pub const POSTED_KEEP_MS: i64 = 30 * 24 * 3_600_000;
/// ...and at most this many per rule.
pub const POSTED_MAX: usize = 500;

/// One badge drop a rule posted.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PostedBadge {
    pub feed_id: String,
    pub phase: BadgePhase,
    pub at_ms: i64,
}

/// One rule's memory.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct RuleCounts {
    /// Times it acted (Live).
    pub count: u64,
    pub last_fire_ms: i64,
    /// Per action: the next line for In order.
    pub next_line: HashMap<String, usize>,
    /// Per action: the lines left in this round of Shuffle.
    pub bags: HashMap<String, Vec<usize>>,
    /// Per action: the line picked last.
    pub last_line: HashMap<String, usize>,
    pub posted: Vec<PostedBadge>,
    /// Per chat key: what each timed trigger last fired for.
    pub timed: HashMap<String, TimedFired>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// What a timed rule last fired for in one chat.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(default)]
pub struct TimedFired {
    /// Timer: the wall ms of its last fire.
    pub last_fire_ms: i64,
    /// Clock: the last local date (`YYYY-MM-DD`) it fired for.
    pub last_date: String,
    /// Uptime and Delay: the `started_at` epoch seconds or the join stamp it fired for.
    pub fired_for: i64,
}

/// Channels remembered for live edges, at most (the least recent is not
/// tracked: a stamp dropped over this only risks one repeat edge, which the
/// claim then answers once across devices).
pub const LIVE_EDGES_MAX: usize = 200;

/// The last live edges this device answered for one channel, as the
/// broadcast's start in epoch seconds (0: none yet).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(default)]
pub struct LiveStamp {
    pub online_started_s: i64,
    pub offline_started_s: i64,
}

/// Every rule's memory on this device.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Counts {
    pub version: u32,
    pub rules: HashMap<String, RuleCounts>,
    /// The last whisper ids the engine saw (a ring of `WHISPER_IDS_MAX`), so a
    /// whisper the history sync filed first, or a re-delivery, fires once.
    pub whisper_ids: Vec<String>,
    /// P3: per channel, the broadcasts whose live and offline edges this
    /// device already answered (once per `started_at`, across restarts).
    pub live_edges: HashMap<String, LiveStamp>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// The whisper-id ring's size.
pub const WHISPER_IDS_MAX: usize = 200;

fn next_random(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
}

/// A new Shuffle round: every line once, never starting with the line the
/// last round ended on.
fn shuffled(lines: usize, avoid_first: Option<usize>, random: u64) -> Vec<usize> {
    let mut bag: Vec<usize> = (0..lines).collect();
    let mut state = random | 1;
    for i in (1..lines).rev() {
        let j = (next_random(&mut state) % (i as u64 + 1)) as usize;
        bag.swap(i, j);
    }
    if lines > 1 && avoid_first == Some(bag[0]) {
        bag.swap(0, lines - 1);
    }
    bag
}

impl Counts {
    pub fn from_json(text: &str) -> Result<Counts, String> {
        serde_json::from_str(text).map_err(|e| e.to_string())
    }

    pub fn to_json(&self) -> String {
        let mut out = self.clone();
        out.version = COUNTS_VERSION;
        serde_json::to_string(&out).unwrap_or_default()
    }

    pub fn rule(&self, rule_id: &str) -> Option<&RuleCounts> {
        self.rules.get(rule_id)
    }

    /// `{count}`: times the rule acted.
    pub fn count(&self, rule_id: &str) -> u64 {
        self.rules.get(rule_id).map_or(0, |r| r.count)
    }

    /// Note a live edge. False when this broadcast's edge of this kind was
    /// already answered here (so a restart or a second source never repeats it).
    pub fn note_live_edge(&mut self, login: &str, online: bool, started_s: i64) -> bool {
        let key = login.trim().to_ascii_lowercase();
        let stamp = self.live_edges.entry(key.clone()).or_default();
        let slot = if online { &mut stamp.online_started_s } else { &mut stamp.offline_started_s };
        if *slot == started_s {
            return false;
        }
        *slot = started_s;
        if self.live_edges.len() > LIVE_EDGES_MAX {
            if let Some(other) = self.live_edges.keys().find(|k| **k != key).cloned() {
                self.live_edges.remove(&other);
            }
        }
        true
    }

    fn rule_mut(&mut self, rule_id: &str) -> &mut RuleCounts {
        self.rules.entry(rule_id.to_string()).or_default()
    }

    /// A Live rule acted.
    pub fn note_fired(&mut self, rule_id: &str, now_ms: i64) {
        let rule = self.rule_mut(rule_id);
        rule.count += 1;
        rule.last_fire_ms = now_ms;
    }

    /// The line an action posts this time, out of `lines`.
    pub fn pick_line(&mut self, rule_id: &str, action_id: &str, lines: usize, pick: Pick, random: u64) -> usize {
        if lines <= 1 {
            return 0;
        }
        let rule = self.rule_mut(rule_id);
        let index = match pick {
            Pick::Random => (random % lines as u64) as usize,
            Pick::InOrder => {
                let next = rule.next_line.entry(action_id.to_string()).or_insert(0);
                let index = *next % lines;
                *next = index + 1;
                index
            }
            Pick::Shuffle => {
                let last = rule.last_line.get(action_id).copied();
                let bag = rule.bags.entry(action_id.to_string()).or_default();
                bag.retain(|&i| i < lines);
                if bag.is_empty() {
                    *bag = shuffled(lines, last, random);
                }
                bag.remove(0)
            }
        };
        rule.last_line.insert(action_id.to_string(), index);
        index
    }

    /// Remember a whisper id; false when it was already seen.
    pub fn note_whisper(&mut self, whisper_id: &str) -> bool {
        if self.whisper_ids.iter().any(|w| w == whisper_id) {
            return false;
        }
        self.whisper_ids.push(whisper_id.to_string());
        let extra = self.whisper_ids.len().saturating_sub(WHISPER_IDS_MAX);
        self.whisper_ids.drain(..extra);
        true
    }

    pub fn timed(&self, rule_id: &str, chat_key: &str) -> TimedFired {
        self.rules.get(rule_id).and_then(|r| r.timed.get(chat_key)).cloned().unwrap_or_default()
    }

    pub fn set_timed(&mut self, rule_id: &str, chat_key: &str, fired: TimedFired) {
        self.rule_mut(rule_id).timed.insert(chat_key.to_string(), fired);
    }

    /// This rule posted this drop's phase on this device before.
    pub fn has_posted(&self, rule_id: &str, feed_id: &str, phase: BadgePhase) -> bool {
        self.rules.get(rule_id).is_some_and(|r| r.posted.iter().any(|p| p.feed_id == feed_id && p.phase == phase))
    }

    pub fn mark_posted(&mut self, rule_id: &str, drops: &[(String, BadgePhase)], now_ms: i64) {
        let rule = self.rule_mut(rule_id);
        for (feed_id, phase) in drops {
            if !rule.posted.iter().any(|p| p.feed_id == *feed_id && p.phase == *phase) {
                rule.posted.push(PostedBadge { feed_id: feed_id.clone(), phase: *phase, at_ms: now_ms });
            }
        }
        let extra = rule.posted.len().saturating_sub(POSTED_MAX);
        rule.posted.drain(..extra);
    }

    /// Forget rules that no longer exist.
    pub fn retain_rules(&mut self, ids: &HashSet<String>) {
        self.rules.retain(|id, _| ids.contains(id));
    }

    /// Forget badge drops posted more than 30 days ago.
    pub fn prune(&mut self, now_ms: i64) {
        for rule in self.rules.values_mut() {
            rule.posted.retain(|p| now_ms - p.at_ms <= POSTED_KEEP_MS);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_order_cycles_through_the_lines() {
        let mut c = Counts::default();
        let picks: Vec<usize> = (0..7).map(|_| c.pick_line("r", "a", 3, Pick::InOrder, 0)).collect();
        assert_eq!(picks, vec![0, 1, 2, 0, 1, 2, 0]);
        // Fewer lines after an edit keep it in range.
        assert!(c.pick_line("r", "a", 2, Pick::InOrder, 0) < 2);
        // Each action keeps its own place.
        assert_eq!(c.pick_line("r", "b", 3, Pick::InOrder, 0), 0);
    }

    #[test]
    fn shuffle_says_every_line_once_a_round_and_never_twice_in_a_row() {
        for seed in 0..200u64 {
            let mut c = Counts::default();
            let mut previous = None;
            for round in 0..5u64 {
                let mut seen = HashSet::new();
                for i in 0..4u64 {
                    let pick = c.pick_line("r", "a", 4, Pick::Shuffle, seed.wrapping_mul(7919).wrapping_add(round * 4 + i));
                    assert!(seen.insert(pick), "seed {seed} round {round}");
                    assert_ne!(Some(pick), previous, "seed {seed}");
                    previous = Some(pick);
                }
            }
        }
    }

    #[test]
    fn random_stays_in_range_and_one_line_is_always_it() {
        let mut c = Counts::default();
        for r in 0..100u64 {
            assert!(c.pick_line("r", "a", 3, Pick::Random, r) < 3);
        }
        assert_eq!(c.pick_line("r", "a", 1, Pick::Shuffle, 9), 0);
        assert_eq!(c.pick_line("r", "a", 0, Pick::InOrder, 9), 0);
    }

    #[test]
    fn counts_posted_badges_and_forgotten_rules() {
        let mut c = Counts::default();
        c.note_fired("r", 5);
        c.note_fired("r", 9);
        assert_eq!(c.count("r"), 2);
        assert_eq!(c.rule("r").unwrap().last_fire_ms, 9);
        assert_eq!(c.count("other"), 0);
        c.mark_posted("r", &[("x-v1".into(), BadgePhase::Announced)], 0);
        c.mark_posted("r", &[("x-v1".into(), BadgePhase::Announced)], 1);
        assert_eq!(c.rule("r").unwrap().posted.len(), 1);
        assert!(c.has_posted("r", "x-v1", BadgePhase::Announced));
        assert!(!c.has_posted("r", "x-v1", BadgePhase::Earnable));
        assert!(!c.has_posted("q", "x-v1", BadgePhase::Announced));
        c.prune(POSTED_KEEP_MS + 2);
        assert!(!c.has_posted("r", "x-v1", BadgePhase::Announced));
        for i in 0..600 {
            c.mark_posted("r", &[(format!("b{i}"), BadgePhase::Announced)], POSTED_KEEP_MS);
        }
        assert_eq!(c.rule("r").unwrap().posted.len(), POSTED_MAX);
        assert!(c.has_posted("r", "b599", BadgePhase::Announced));
        assert!(!c.has_posted("r", "b0", BadgePhase::Announced));
        c.retain_rules(&HashSet::new());
        assert_eq!(c.count("r"), 0);
    }

    #[test]
    fn the_file_round_trips_and_keeps_what_a_newer_build_wrote() {
        let text = r#"{"version":1,"future":{"a":1},"rules":{"r":{"count":3,"timer_fired":true,"posted":[{"feed_id":"x","phase":"earnable","at_ms":5}]}}}"#;
        let c = Counts::from_json(text).unwrap();
        assert_eq!(c.count("r"), 3);
        assert!(c.has_posted("r", "x", BadgePhase::Earnable));
        let back: Value = serde_json::from_str(&c.to_json()).unwrap();
        assert_eq!(back["future"], serde_json::json!({"a": 1}));
        assert_eq!(back["rules"]["r"]["timer_fired"], serde_json::json!(true));
        assert_eq!(back["version"], serde_json::json!(COUNTS_VERSION));
        assert_eq!(Counts::from_json(&c.to_json()).unwrap(), c);
        assert!(Counts::from_json("[").is_err());
        assert_eq!(Counts::from_json("{}").unwrap(), Counts::default());
    }

    #[test]
    fn a_live_edge_is_answered_once_per_broadcast_and_survives_a_restart() {
        let mut counts = Counts::default();
        assert!(counts.note_live_edge("Friend", true, 100));
        assert!(!counts.note_live_edge("friend", true, 100), "the same broadcast again");
        assert!(counts.note_live_edge("friend", false, 100), "its end is its own edge");
        assert!(!counts.note_live_edge("friend", false, 100));
        assert!(counts.note_live_edge("friend", true, 200), "a new broadcast");
        let back = Counts::from_json(&counts.to_json()).unwrap();
        assert_eq!(back.live_edges["friend"], LiveStamp { online_started_s: 200, offline_started_s: 100 });
        let mut back = back;
        assert!(!back.note_live_edge("friend", true, 200), "still answered after the file was read back");
    }

    #[test]
    fn live_stamps_are_capped_and_an_older_file_reads() {
        let mut counts = Counts::default();
        for i in 0..(LIVE_EDGES_MAX + 20) {
            counts.note_live_edge(&format!("c{i}"), true, 1);
        }
        assert!(counts.live_edges.len() <= LIVE_EDGES_MAX + 1);
        let old = Counts::from_json(r#"{"version":1,"rules":{}}"#).unwrap();
        assert!(old.live_edges.is_empty(), "a P1 file has no live_edges");
        let newer = Counts::from_json(r#"{"version":1,"live_edges":{"a":{"online_started_s":5,"future":1}},"x":2}"#).unwrap();
        assert_eq!(newer.live_edges["a"].online_started_s, 5);
        assert_eq!(newer.extra["x"], 2);
    }
}
