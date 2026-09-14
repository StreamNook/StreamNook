//! Bounded per-channel chat history ring for search (Ctrl+F) and the
//! mentions feed backfill. Rust-owned so every window reads one copy and the
//! corpus never crosses IPC: `search` returns at most `limit` compact hits.
//!
//! Entries are compact (no wire frame, no segments): roughly 300 bytes each.
//! The cap is `chat_query.history_cap` (default 1000 per joined channel,
//! clamped 200..5000), pushed here by `ChatRules::refresh`. A channel's ring
//! is created on its first message and dropped when the channel is parted.
//!
//! Search syntax follows Chatterino's search popup: free words are a
//! case-insensitive substring match on the text; `from:name`, `in:channel`,
//! `badge:name`, `subtier:N`, `has:link`, `regex:pattern`, and
//! `is:{sub,subscription,highlighted,system,first-msg,cheer-msg,redemption,
//! reply,shared,deleted,mention}` narrow it; a `!` prefix negates any of
//! them; commas separate alternatives; quotes allow spaces.

use crate::models::chat_layout::ChatMessage;
use crate::services::chat_rules::{
    MessageFacts, F_CHEER, F_DELETED, F_FIRST, F_HIGHLIGHTED, F_HIGHLIGHT_RULE, F_LINK,
    F_MENTION, F_REDEEMED, F_REPLY, F_SHARED, F_SUB, F_SYSTEM, MAX_HISTORY_CAP,
    MIN_HISTORY_CAP,
};
use regex::Regex;
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

pub struct HistEntry {
    pub id: String,
    pub ts_ms: i64,
    pub login: String,
    pub display: String,
    pub user_id: String,
    pub content: String,
    pub color: Option<String>,
    /// `name/version` lowercased.
    pub badge_keys: Vec<String>,
    pub sub_length: i64,
    pub flags: u32,
    pub msg_type: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct SearchHit {
    pub id: String,
    pub channel: String,
    pub ts_ms: i64,
    pub login: String,
    pub display_name: String,
    pub user_id: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    pub flags: u32,
}

#[derive(Serialize, Clone, Debug)]
pub struct HistoryStats {
    pub channels: usize,
    pub entries: usize,
    pub cap: usize,
}

static RINGS: OnceLock<Mutex<HashMap<String, VecDeque<HistEntry>>>> = OnceLock::new();
static CAP: AtomicUsize = AtomicUsize::new(1000);

fn rings() -> &'static Mutex<HashMap<String, VecDeque<HistEntry>>> {
    RINGS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub struct ChatHistory;

impl ChatHistory {
    pub fn set_cap(cap: usize) {
        let cap = cap.clamp(MIN_HISTORY_CAP, MAX_HISTORY_CAP);
        let old = CAP.swap(cap, Ordering::AcqRel);
        if cap < old {
            if let Ok(mut map) = rings().lock() {
                for ring in map.values_mut() {
                    while ring.len() > cap {
                        ring.pop_front();
                    }
                }
            }
        }
    }

    pub fn cap() -> usize {
        CAP.load(Ordering::Acquire)
    }

    /// O(1) push with eviction. Called on the IRC lane after evaluation; the
    /// lock is uncontended except during a search.
    pub fn push(msg: &ChatMessage, facts: &MessageFacts<'_>, highlighted: bool, mentioned: bool) {
        if msg.id.is_empty() || facts.channel_key.is_empty() {
            return;
        }
        let mut flags = facts.flags;
        if highlighted {
            flags |= F_HIGHLIGHT_RULE;
        }
        if mentioned {
            flags |= F_MENTION;
        }
        let entry = HistEntry {
            id: msg.id.clone(),
            ts_ms: facts.ts_ms,
            login: facts.login.clone(),
            display: msg.display_name.clone(),
            user_id: msg.user_id.clone(),
            content: msg.content.clone(),
            color: msg.color.clone(),
            badge_keys: facts.badge_keys.clone(),
            sub_length: facts.sub_length,
            flags,
            msg_type: msg.metadata.msg_type.clone(),
        };
        let cap = Self::cap();
        let Ok(mut map) = rings().lock() else { return };
        let ring = map
            .entry(facts.channel_key.clone())
            .or_insert_with(|| VecDeque::with_capacity(64));
        if ring.len() >= cap {
            ring.pop_front();
        }
        ring.push_back(entry);
    }

    /// Mark a message deleted (CLEARMSG) so `is:deleted` can find it.
    pub fn mark_deleted(channel: &str, msg_id: &str) {
        let key = channel.trim_start_matches('#').to_lowercase();
        let Ok(mut map) = rings().lock() else { return };
        if let Some(ring) = map.get_mut(&key) {
            if let Some(e) = ring.iter_mut().rev().find(|e| e.id == msg_id) {
                e.flags |= F_DELETED;
            }
        }
    }

    /// Mark every message from a user deleted (CLEARCHAT with a target).
    pub fn mark_user_cleared(channel: &str, user_id: &str) {
        let key = channel.trim_start_matches('#').to_lowercase();
        let Ok(mut map) = rings().lock() else { return };
        if let Some(ring) = map.get_mut(&key) {
            for e in ring.iter_mut().filter(|e| e.user_id == user_id) {
                e.flags |= F_DELETED;
            }
        }
    }

    /// Drop a channel's ring. Called when its last consumer parts.
    pub fn clear_channel(channel: &str) {
        let key = channel.trim_start_matches('#').to_lowercase();
        if let Ok(mut map) = rings().lock() {
            map.remove(&key);
        }
    }

    pub fn clear_all() {
        if let Ok(mut map) = rings().lock() {
            map.clear();
        }
    }

    pub fn stats() -> HistoryStats {
        let (channels, entries) = rings()
            .lock()
            .map(|m| (m.len(), m.values().map(|r| r.len()).sum()))
            .unwrap_or((0, 0));
        HistoryStats {
            channels,
            entries,
            cap: Self::cap(),
        }
    }

    /// Newest-first hits. `channel` None searches every joined channel.
    pub fn search(channel: Option<&str>, query: &str, limit: usize) -> Result<Vec<SearchHit>, String> {
        let q = SearchQuery::parse(query)?;
        let limit = limit.clamp(1, 500);
        let map = rings().lock().map_err(|e| e.to_string())?;
        let key = channel.map(|c| c.trim_start_matches('#').to_lowercase());
        let mut hits: Vec<SearchHit> = Vec::new();
        let mut scan = |ch: &str, ring: &VecDeque<HistEntry>| {
            for e in ring.iter().rev() {
                if q.matches(ch, e) {
                    hits.push(SearchHit {
                        id: e.id.clone(),
                        channel: ch.to_string(),
                        ts_ms: e.ts_ms,
                        login: e.login.clone(),
                        display_name: e.display.clone(),
                        user_id: e.user_id.clone(),
                        content: e.content.clone(),
                        color: e.color.clone(),
                        flags: e.flags,
                    });
                }
            }
        };
        match key {
            Some(k) => {
                if let Some(ring) = map.get(&k) {
                    scan(&k, ring);
                }
            }
            None => {
                for (ch, ring) in map.iter() {
                    scan(ch, ring);
                }
            }
        }
        hits.sort_by(|a, b| b.ts_ms.cmp(&a.ts_ms));
        hits.truncate(limit);
        Ok(hits)
    }
}

// ---------------------------------------------------------------------------
// Query language
// ---------------------------------------------------------------------------

#[derive(Debug)]
enum Pred {
    Text(String),
    From(Vec<String>),
    In(Vec<String>),
    Badge(Vec<String>),
    SubTier(Vec<i64>),
    Has(Vec<String>),
    Is(Vec<String>),
    Regex(Regex),
}

#[derive(Debug)]
struct Term {
    negate: bool,
    pred: Pred,
}

pub struct SearchQuery {
    terms: Vec<Term>,
}

fn split_terms(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quote = false;
    for c in input.chars() {
        match c {
            '"' => in_quote = !in_quote,
            ' ' | '\t' | '\n' if !in_quote => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn list(v: &str) -> Vec<String> {
    v.split(',')
        .map(|s| s.trim().trim_start_matches('@').trim_start_matches('#').to_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}

impl SearchQuery {
    pub fn parse(input: &str) -> Result<SearchQuery, String> {
        let mut terms = Vec::new();
        for raw in split_terms(input.trim()) {
            let (negate, body) = match raw.strip_prefix('!') {
                Some(rest) => (true, rest.to_string()),
                None => (false, raw),
            };
            if body.is_empty() {
                continue;
            }
            let pred = match body.split_once(':') {
                Some(("from", v)) => Pred::From(list(v)),
                Some(("in", v)) => Pred::In(list(v)),
                Some(("badge", v)) => Pred::Badge(list(v)),
                Some(("subtier", v)) => Pred::SubTier(
                    list(v).iter().filter_map(|s| s.parse::<i64>().ok()).collect(),
                ),
                Some(("has", v)) => Pred::Has(list(v)),
                Some(("is", v)) => Pred::Is(list(v)),
                Some(("regex", v)) => {
                    let re = Regex::new(&format!("(?i){}", v)).map_err(|e| e.to_string())?;
                    Pred::Regex(re)
                }
                _ => Pred::Text(body.to_lowercase()),
            };
            terms.push(Term { negate, pred });
        }
        Ok(SearchQuery { terms })
    }

    fn matches(&self, channel: &str, e: &HistEntry) -> bool {
        self.terms.iter().all(|t| {
            let hit = match &t.pred {
                Pred::Text(s) => e.content.to_lowercase().contains(s.as_str()),
                Pred::From(names) => names.iter().any(|n| *n == e.login || *n == e.display.to_lowercase()),
                Pred::In(chs) => chs.iter().any(|c| c == channel || channel.ends_with(&format!(":{}", c))),
                Pred::Badge(names) => names.iter().any(|n| {
                    e.badge_keys
                        .iter()
                        .any(|k| k == n || k.starts_with(&format!("{}/", n)))
                }),
                Pred::SubTier(tiers) => {
                    // subscriber/<version>: 0 = tier 1, 2000 = tier 2, 3000 = tier 3;
                    // versions above 1000 encode tier in the thousands digit.
                    let tier = e
                        .badge_keys
                        .iter()
                        .find_map(|k| k.strip_prefix("subscriber/"))
                        .and_then(|v| v.parse::<i64>().ok())
                        .map(|v| if v >= 1000 { v / 1000 } else { 1 });
                    tier.map(|t| tiers.contains(&t)).unwrap_or(false)
                }
                Pred::Has(what) => what.iter().any(|w| match w.as_str() {
                    "link" => e.flags & F_LINK != 0,
                    _ => false,
                }),
                Pred::Is(kinds) => kinds.iter().any(|k| match k.as_str() {
                    "sub" | "subscription" => e.flags & F_SUB != 0,
                    "highlighted" => e.flags & (F_HIGHLIGHTED | F_HIGHLIGHT_RULE) != 0,
                    "system" => e.flags & F_SYSTEM != 0,
                    "first-msg" => e.flags & F_FIRST != 0,
                    "cheer-msg" => e.flags & F_CHEER != 0,
                    "redemption" => e.flags & F_REDEEMED != 0,
                    "reply" => e.flags & F_REPLY != 0,
                    "shared" => e.flags & F_SHARED != 0,
                    "deleted" => e.flags & F_DELETED != 0,
                    "mention" => e.flags & F_MENTION != 0,
                    _ => false,
                }),
                Pred::Regex(re) => re.is_match(&e.content),
            };
            hit != t.negate
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(login: &str, content: &str, flags: u32) -> HistEntry {
        HistEntry {
            id: format!("{}-{}", login, content.len()),
            ts_ms: 0,
            login: login.into(),
            display: login.to_uppercase(),
            user_id: "1".into(),
            content: content.into(),
            color: None,
            badge_keys: vec!["subscriber/2012".into(), "moderator/1".into()],
            sub_length: 12,
            flags,
            msg_type: None,
        }
    }

    #[test]
    fn query_predicates() {
        let e = entry("bob", "check https://x.y out", F_LINK | F_REPLY);
        let ok = |q: &str| SearchQuery::parse(q).unwrap().matches("forsen", &e);
        assert!(ok("check"));
        assert!(ok("CHECK out"));
        assert!(!ok("missing"));
        assert!(ok("from:bob"));
        assert!(ok("from:alice,BOB"));
        assert!(ok("!from:alice"));
        assert!(!ok("!from:bob"));
        assert!(ok("in:forsen"));
        assert!(ok("badge:moderator"));
        assert!(ok("badge:subscriber"));
        assert!(!ok("badge:vip"));
        assert!(ok("subtier:2"));
        assert!(!ok("subtier:1,3"));
        assert!(ok("has:link"));
        assert!(!ok("!has:link"));
        assert!(ok("is:reply"));
        assert!(!ok("is:sub"));
        assert!(ok("regex:^che.k"));
        assert!(ok("\"check https\""));
        assert!(SearchQuery::parse("regex:(").is_err());
    }

    #[test]
    fn ring_caps_and_searches_newest_first() {
        ChatHistory::clear_all();
        ChatHistory::set_cap(200);
        let mut m = crate::models::chat_layout::ChatMessage {
            id: String::new(),
            user_id: "1".into(),
            username: "bob".into(),
            display_name: "Bob".into(),
            color: None,
            badges: vec![],
            timestamp: "0".into(),
            content: String::new(),
            provider: "twitch".into(),
            channel: "#ringtest".into(),
            emotes: vec![],
            tags: HashMap::new(),
            layout: Default::default(),
            segments: vec![],
            metadata: Default::default(),
        };
        for i in 0..250 {
            m.id = format!("id{}", i);
            m.content = format!("message number {}", i);
            m.tags.insert("tmi-sent-ts".into(), i.to_string());
            let facts = MessageFacts::from_message(&m);
            ChatHistory::push(&m, &facts, false, false);
        }
        let stats = ChatHistory::stats();
        assert_eq!(stats.entries, 200);
        let hits = ChatHistory::search(Some("ringtest"), "number 24", 10).unwrap();
        assert_eq!(hits.len(), 10);
        assert_eq!(hits[0].id, "id249");
        // Evicted rows are gone.
        assert!(ChatHistory::search(Some("ringtest"), "regex:\"number 3$\"", 5).unwrap().is_empty());
        assert_eq!(ChatHistory::search(Some("ringtest"), "regex:\"number 53$\"", 5).unwrap().len(), 1);
        ChatHistory::mark_deleted("#ringtest", "id249");
        let del = ChatHistory::search(Some("ringtest"), "is:deleted", 5).unwrap();
        assert_eq!(del.len(), 1);
        ChatHistory::clear_channel("ringtest");
        assert_eq!(ChatHistory::stats().entries, 0);
    }
}
