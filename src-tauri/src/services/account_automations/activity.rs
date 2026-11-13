//! The Activity log: what rules did, what they would do in Test, and what
//! they skipped.
//!
//! A ring of the last 500 entries; the newest 200 are kept on disk
//! (`automation_activity.json`). A skip that repeats for one rule, one chat
//! and one reason folds into one entry with a count, so a busy chat never
//! floods the log. Each watching window gets at most 4 updates a second
//! ([`Coalescer`]). Pure: the caller stamps every entry with its time.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

/// Entries kept in memory.
pub const RING: usize = 500;
/// Entries kept on disk.
pub const PERSISTED: usize = 200;
/// How far back a skip looks for its twin to fold into.
pub const FOLD_LOOKBACK: usize = 50;
/// A skip folds into a twin at most this old.
pub const FOLD_MS: i64 = 10 * 60_000;
/// The least time between two updates to one watcher.
pub const EMIT_GAP_MS: i64 = 250;
pub const ACTIVITY_VERSION: u32 = 1;
/// A reminder's rows carry `rule_id` = this prefix + the reminder's id: never
/// a rule id, so a rule's last outcome never reads one.
pub const REMINDER_ID_PREFIX: &str = "reminder:";

/// What an entry says happened.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    /// A Live rule acted.
    Acted,
    /// A Test rule would have acted.
    Test,
    Skipped,
    /// About automations as a whole (the breaker, the outage status).
    #[default]
    Notice,
}

/// One line of Activity.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(default)]
pub struct Entry {
    pub id: u64,
    /// The latest time (a folded skip's last repeat).
    pub at_ms: i64,
    /// The first time.
    pub first_at_ms: i64,
    pub kind: EntryKind,
    /// Empty for a notice about automations as a whole.
    pub rule_id: String,
    pub rule_name: String,
    /// `twitch:<login>`, or empty.
    pub channel: String,
    pub text: String,
    /// How many times it happened (more than 1 only for folded skips).
    pub count: u32,
    /// The id this entry had before it folded into another and moved to the
    /// top with a new id; 0 when it did not fold. A live window removes that
    /// older row (S7.6 `applyEntry`) so it never shows the entry twice.
    pub replaces: u64,
}

/// An entry to log.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct NewEntry {
    pub kind: EntryKind,
    pub rule_id: String,
    pub rule_name: String,
    pub channel: String,
    pub text: String,
}

/// The page's filter: All, Acted, Test, Skipped.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Filter {
    #[default]
    All,
    Acted,
    Test,
    Skipped,
}

impl Filter {
    pub fn admits(self, kind: EntryKind) -> bool {
        match self {
            Filter::All => true,
            Filter::Acted => kind == EntryKind::Acted,
            Filter::Test => kind == EntryKind::Test,
            Filter::Skipped => kind == EntryKind::Skipped,
        }
    }
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct ActivityFile {
    version: u32,
    entries: Vec<Entry>,
}

/// The log.
#[derive(Clone, Debug, Default)]
pub struct Log {
    entries: VecDeque<Entry>,
    next_id: u64,
}

impl Log {
    fn take_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    /// A reminder's row folds into its twin wherever it sits in the ring and
    /// however old it is, so a 10 s reminder never pushes rule entries out:
    /// its fires (`acted`) become one row with a count and the last time, and
    /// a repeated skip (same chat, same words) becomes one row too. The folded
    /// row moves to the top with a new id, as a folded skip does.
    fn fold_reminder(&mut self, new: &NewEntry, now_ms: i64) -> Option<Entry> {
        let twin = self.entries.iter().rposition(|e| {
            e.rule_id == new.rule_id
                && e.kind == new.kind
                && (new.kind == EntryKind::Acted || (e.channel == new.channel && e.text == new.text))
        })?;
        let mut entry = self.entries.remove(twin)?;
        entry.count = entry.count.saturating_add(1);
        entry.at_ms = now_ms;
        entry.replaces = entry.id;
        entry.id = self.take_id();
        entry.rule_name = new.rule_name.clone();
        entry.channel = new.channel.clone();
        entry.text = new.text.clone();
        self.entries.push_back(entry.clone());
        Some(entry)
    }

    /// Log an entry at `now_ms` and return it as stored (a folded skip comes
    /// back with its count). A folded entry moves to the top with a new id.
    pub fn push(&mut self, new: NewEntry, now_ms: i64) -> Entry {
        if new.rule_id.starts_with(REMINDER_ID_PREFIX) && matches!(new.kind, EntryKind::Acted | EntryKind::Skipped) {
            if let Some(entry) = self.fold_reminder(&new, now_ms) {
                return entry;
            }
        }
        if new.kind == EntryKind::Skipped {
            let start = self.entries.len().saturating_sub(FOLD_LOOKBACK);
            let twin = (start..self.entries.len()).rev().find(|&i| {
                let e = &self.entries[i];
                e.kind == EntryKind::Skipped
                    && e.rule_id == new.rule_id
                    && e.channel == new.channel
                    && e.text == new.text
                    && now_ms - e.at_ms < FOLD_MS
            });
            if let Some(i) = twin {
                if let Some(mut entry) = self.entries.remove(i) {
                    entry.count = entry.count.saturating_add(1);
                    entry.at_ms = now_ms;
                    entry.replaces = entry.id;
                    entry.id = self.take_id();
                    self.entries.push_back(entry.clone());
                    return entry;
                }
            }
        }
        let entry = Entry {
            id: self.take_id(),
            at_ms: now_ms,
            first_at_ms: now_ms,
            kind: new.kind,
            rule_id: new.rule_id,
            rule_name: new.rule_name,
            channel: new.channel,
            text: new.text,
            count: 1,
            replaces: 0,
        };
        self.entries.push_back(entry.clone());
        while self.entries.len() > RING {
            self.entries.pop_front();
        }
        entry
    }

    /// Newest first: at most `limit` entries the filter admits, older than
    /// the entry `before` when given.
    pub fn query(&self, filter: Filter, limit: usize, before: Option<u64>) -> Vec<Entry> {
        self.entries
            .iter()
            .rev()
            .filter(|e| before.map_or(true, |b| e.id < b))
            .filter(|e| filter.admits(e.kind))
            .take(limit)
            .cloned()
            .collect()
    }

    /// The rule's newest entry: its row's last outcome.
    pub fn last_for_rule(&self, rule_id: &str) -> Option<&Entry> {
        self.entries.iter().rev().find(|e| e.kind != EntryKind::Notice && e.rule_id == rule_id)
    }

    /// Clear the log; ids keep counting up.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The newest 200, oldest first, as the file keeps them.
    pub fn persisted(&self) -> Vec<Entry> {
        let skip = self.entries.len().saturating_sub(PERSISTED);
        self.entries.iter().skip(skip).cloned().collect()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(&ActivityFile { version: ACTIVITY_VERSION, entries: self.persisted() }).unwrap_or_default()
    }

    pub fn from_json(text: &str) -> Result<Log, String> {
        let file: ActivityFile = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let mut entries = file.entries;
        entries.sort_by_key(|e| e.id);
        let skip = entries.len().saturating_sub(RING);
        let next_id = entries.iter().map(|e| e.id).max().unwrap_or(0);
        Ok(Log { entries: entries.into_iter().skip(skip).collect(), next_id })
    }
}

/// What to do with a watcher after an entry was logged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Emit {
    /// Send it now.
    Now,
    /// Set one timer for this time, then call [`Coalescer::flushed`] and send.
    At(i64),
    /// A timer is already set; it will carry this entry too.
    Waiting,
}

/// One watcher's update pacing: at most one update per 250 ms.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Coalescer {
    last_ms: Option<i64>,
    flush_at: Option<i64>,
}

impl Coalescer {
    pub fn offer(&mut self, now_ms: i64) -> Emit {
        if self.flush_at.is_some() {
            return Emit::Waiting;
        }
        match self.last_ms {
            Some(last) if now_ms - last < EMIT_GAP_MS => {
                let at = last + EMIT_GAP_MS;
                self.flush_at = Some(at);
                Emit::At(at)
            }
            _ => {
                self.last_ms = Some(now_ms);
                Emit::Now
            }
        }
    }

    /// The timer fired and the update went out.
    pub fn flushed(&mut self, now_ms: i64) {
        self.flush_at = None;
        self.last_ms = Some(now_ms);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skip(rule: &str, text: &str) -> NewEntry {
        NewEntry {
            kind: EntryKind::Skipped,
            rule_id: rule.into(),
            rule_name: format!("Rule {rule}"),
            channel: "twitch:me".into(),
            text: text.into(),
        }
    }

    fn acted(rule: &str) -> NewEntry {
        NewEntry { kind: EntryKind::Acted, ..skip(rule, "Replied to @fan in #me") }
    }

    #[test]
    fn repeated_skips_fold_into_one_entry() {
        let mut log = Log::default();
        for i in 0..100 {
            log.push(skip("r", "Cooling down"), i * 1_000);
        }
        assert_eq!(log.len(), 1);
        let e = &log.query(Filter::All, 10, None)[0];
        assert_eq!((e.count, e.first_at_ms, e.at_ms), (100, 0, 99_000));
        // Another reason, rule or chat is its own entry; acts never fold.
        log.push(skip("r", "Not your channel"), 100_000);
        log.push(skip("q", "Cooling down"), 100_000);
        log.push(NewEntry { channel: "twitch:other".into(), ..skip("r", "Cooling down") }, 100_000);
        log.push(acted("r"), 100_000);
        log.push(acted("r"), 100_001);
        assert_eq!(log.len(), 6);
        // A twin 10 minutes old starts a new entry.
        log.push(skip("r", "Cooling down"), 99_000 + FOLD_MS);
        assert_eq!(log.len(), 7);
        // A folded entry moves to the top.
        let folded = log.push(skip("q", "Cooling down"), 699_500);
        assert_eq!(folded.count, 2);
        assert!(folded.replaces != 0 && folded.replaces < folded.id, "a folded entry names the id it replaced");
        let top = &log.query(Filter::All, 1, None)[0];
        assert_eq!((top.rule_id.as_str(), top.count, top.id), ("q", 2, folded.id));
        assert_eq!(log.len(), 7);
    }

    #[test]
    fn the_ring_keeps_500_and_the_file_keeps_the_newest_200() {
        let mut log = Log::default();
        for i in 0..600 {
            log.push(acted(&format!("r{i}")), i);
        }
        assert_eq!(log.len(), RING);
        let kept = log.persisted();
        assert_eq!(kept.len(), PERSISTED);
        assert_eq!(kept.first().unwrap().rule_id, "r400");
        assert_eq!(kept.last().unwrap().rule_id, "r599");
        let mut back = Log::from_json(&log.to_json()).unwrap();
        assert_eq!(back.len(), PERSISTED);
        assert_eq!(back.query(Filter::All, 1, None)[0].rule_id, "r599");
        // New entries continue after the restored ids.
        let next = back.push(acted("next"), 1_000);
        assert!(next.id > kept.last().unwrap().id);
        assert!(Log::from_json("not json").is_err());
        assert!(Log::from_json("{}").unwrap().is_empty());
    }

    #[test]
    fn queries_filter_by_kind_and_page_by_id() {
        let mut log = Log::default();
        log.push(acted("a"), 1);
        log.push(NewEntry { kind: EntryKind::Test, text: "Would reply: hi".into(), ..acted("b") }, 2);
        log.push(skip("c", "Cooling down"), 3);
        log.push(NewEntry { kind: EntryKind::Notice, text: "Automations turned off: rules were firing too fast".into(), ..acted("") }, 4);
        let ids = |log: &Log, filter: Filter| log.query(filter, 10, None).iter().map(|e| e.rule_id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&log, Filter::All), vec!["", "c", "b", "a"]);
        assert_eq!(ids(&log, Filter::Acted), vec!["a"]);
        assert_eq!(ids(&log, Filter::Test), vec!["b"]);
        assert_eq!(ids(&log, Filter::Skipped), vec!["c"]);
        let newest_two = log.query(Filter::All, 2, None);
        let older = log.query(Filter::All, 10, Some(newest_two[1].id));
        assert_eq!(older.iter().map(|e| e.rule_id.as_str()).collect::<Vec<_>>(), vec!["b", "a"]);
        assert_eq!(log.last_for_rule("b").unwrap().text, "Would reply: hi");
        assert!(log.last_for_rule("").is_none());
        log.clear();
        assert!(log.is_empty());
        assert!(log.push(acted("d"), 5).id > newest_two[0].id);
    }

    #[test]
    fn a_watcher_gets_at_most_four_updates_a_second() {
        let mut c = Coalescer::default();
        let mut emits: Vec<i64> = Vec::new();
        let mut flush_at: Option<i64> = None;
        for now in (0..2_000i64).step_by(10) {
            if let Some(at) = flush_at {
                if now >= at {
                    c.flushed(now);
                    emits.push(now);
                    flush_at = None;
                }
            }
            match c.offer(now) {
                Emit::Now => emits.push(now),
                Emit::At(at) => flush_at = Some(at),
                Emit::Waiting => {}
            }
        }
        for start in (0..1_000i64).step_by(10) {
            let n = emits.iter().filter(|t| **t >= start && **t < start + 1_000).count();
            assert!(n <= 4, "{n} updates from {start}");
        }
        assert!(emits.len() >= 7);
        // A quiet watcher gets the next entry at once.
        let mut quiet = Coalescer::default();
        assert_eq!(quiet.offer(0), Emit::Now);
        assert_eq!(quiet.offer(5_000), Emit::Now);
        assert_eq!(quiet.offer(5_100), Emit::At(5_250));
        assert_eq!(quiet.offer(5_200), Emit::Waiting);
    }
}
