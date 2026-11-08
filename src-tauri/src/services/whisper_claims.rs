//! Which conversation each window (or the phone app) shows, and whether that
//! counts as reading it. A whisper landing in a conversation being read is
//! read at once, here and on Twitch, and does not notify, so "being read" is
//! strict: on desktop the conversation's own window must be the focused one
//! and not minimized; on a phone the app must be interactive (in front and
//! not in picture-in-picture).
//!
//! A destroyed window runs no cleanup, and a reloaded page cannot clear what
//! the old page claimed, so Rust clears a window's claim itself on both, and
//! orders each page's calls by (page load, sequence).

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Mutex;

use once_cell::sync::Lazy;

use crate::services::whisper_sync::{self, Reason};

/// Retired page loads remembered per window, to refuse their late calls.
const RETIRED_KEPT: usize = 8;

#[derive(Clone, Debug)]
struct Claim {
    owner: String,
    key: Option<String>,
    page_load: String,
    seq: u64,
    generation: u64,
}

pub(crate) struct ClaimState {
    mobile: bool,
    claims: HashMap<String, Claim>,
    focused: HashMap<String, bool>,
    minimized: HashMap<String, bool>,
    interactive: bool,
    retired: HashMap<String, VecDeque<String>>,
    counting: HashSet<(String, String)>,
}

impl ClaimState {
    pub(crate) fn new(mobile: bool) -> Self {
        Self {
            mobile,
            claims: HashMap::new(),
            focused: HashMap::new(),
            minimized: HashMap::new(),
            interactive: true,
            retired: HashMap::new(),
            counting: HashSet::new(),
        }
    }

    fn label_counts(&self, label: &str) -> bool {
        if self.mobile {
            self.interactive
        } else {
            self.focused.get(label).copied().unwrap_or(false) && !self.minimized.get(label).copied().unwrap_or(false)
        }
    }

    pub(crate) fn reading(&self, owner: &str, key: &str) -> bool {
        self.claims
            .iter()
            .any(|(label, c)| c.owner == owner && c.key.as_deref() == Some(key) && self.label_counts(label))
    }

    fn now_counting(&self) -> HashSet<(String, String)> {
        self.claims
            .iter()
            .filter(|(label, _)| self.label_counts(label))
            .filter_map(|(_, c)| c.key.clone().map(|k| (c.owner.clone(), k)))
            .collect()
    }

    /// The (owner, key) pairs that started counting since the last call.
    pub(crate) fn newly_counting(&mut self) -> Vec<(String, String)> {
        let now = self.now_counting();
        let mut new: Vec<(String, String)> = now.difference(&self.counting).cloned().collect();
        new.sort();
        self.counting = now;
        new
    }

    fn retire(&mut self, label: &str, page_load: &str) {
        let list = self.retired.entry(label.to_string()).or_default();
        if !list.iter().any(|p| p == page_load) {
            list.push_back(page_load.to_string());
            while list.len() > RETIRED_KEPT {
                list.pop_front();
            }
        }
    }

    /// Apply a page's call when it is newer than the last one applied for the
    /// window. Returns whether it was applied.
    pub(crate) fn set_claim(
        &mut self,
        label: &str,
        owner: &str,
        key: Option<String>,
        page_load: &str,
        seq: u64,
        generation: u64,
    ) -> bool {
        if self.retired.get(label).is_some_and(|r| r.iter().any(|p| p == page_load)) {
            return false;
        }
        if let Some(c) = self.claims.get(label).cloned() {
            if c.page_load == page_load && seq <= c.seq {
                return false;
            }
            if c.page_load != page_load {
                self.retire(label, &c.page_load);
            }
        }
        self.claims.insert(
            label.to_string(),
            Claim { owner: owner.to_string(), key, page_load: page_load.to_string(), seq, generation },
        );
        true
    }

    pub(crate) fn clear_label(&mut self, label: &str) {
        if let Some(c) = self.claims.remove(label) {
            self.retire(label, &c.page_load);
        }
    }

    pub(crate) fn forget_window(&mut self, label: &str) {
        self.clear_label(label);
        self.focused.remove(label);
        self.minimized.remove(label);
    }

    pub(crate) fn note_focus(&mut self, label: &str, focused: bool) {
        self.focused.insert(label.to_string(), focused);
    }

    pub(crate) fn note_minimized(&mut self, label: &str, minimized: bool) -> bool {
        self.minimized.insert(label.to_string(), minimized) != Some(minimized)
    }

    pub(crate) fn set_interactive(&mut self, interactive: bool) {
        self.interactive = interactive;
    }

    pub(crate) fn clear_scope(&mut self, generation: u64) {
        self.claims.retain(|_, c| c.generation == generation);
    }

    fn claimed_key(&self, label: &str) -> Option<String> {
        self.claims.get(label).and_then(|c| c.key.clone())
    }
}

static STATE: Lazy<Mutex<ClaimState>> = Lazy::new(|| Mutex::new(ClaimState::new(cfg!(mobile))));

fn state() -> std::sync::MutexGuard<'static, ClaimState> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Read through whatever just started counting. Runs after the lock is gone:
/// the read takes the store lock and asks `reading` again.
fn after_change(new: Vec<(String, String)>) {
    for (owner, key) in new {
        crate::services::whisper_inbox::on_claim_counting(&owner, &key);
    }
}

/// A page says which conversation it shows (None: none). A newly opened one
/// is refreshed from Twitch, so replies sent from another device show.
pub fn set_claim(label: &str, owner: &str, key: Option<String>, page_load: &str, seq: u64) {
    let generation = whisper_sync::current_scope().map(|s| s.generation).unwrap_or(0);
    let (opened, new) = {
        let mut s = state();
        let before = s.claimed_key(label);
        let applied = s.set_claim(label, owner, key.clone(), page_load, seq, generation);
        let opened = if applied && key.is_some() && key != before { key } else { None };
        (opened, s.newly_counting())
    };
    if let Some(key) = opened {
        whisper_sync::trigger(Reason::ConversationOpened(key));
    }
    after_change(new);
}

/// The window was destroyed or started loading a new page.
pub fn clear_label(label: &str) {
    let mut s = state();
    s.clear_label(label);
    let _ = s.newly_counting();
}

/// A destroyed window: its claim and what was known about its focus go.
pub fn forget_window(label: &str) {
    let mut s = state();
    s.forget_window(label);
    let _ = s.newly_counting();
}

/// `WindowEvent::Focused` for a desktop window.
pub fn note_focus(label: &str, focused: bool) {
    let new = {
        let mut s = state();
        s.note_focus(label, focused);
        s.newly_counting()
    };
    after_change(new);
}

/// The visibility poll's verdict for a window (minimized or hidden).
pub fn note_minimized(label: &str, minimized: bool) {
    let new = {
        let mut s = state();
        if !s.note_minimized(label, minimized) {
            return;
        }
        s.newly_counting()
    };
    after_change(new);
}

/// The phone app became interactive (in front, not picture-in-picture) or not.
pub fn set_app_interactive(interactive: bool) {
    let new = {
        let mut s = state();
        s.set_interactive(interactive);
        s.newly_counting()
    };
    after_change(new);
}

/// Whether someone is reading this conversation right now.
pub fn reading(owner: &str, key: &str) -> bool {
    state().reading(owner, key)
}

/// A new account generation: claims made under another one go.
pub fn clear_scope(generation: u64) {
    let mut s = state();
    s.clear_scope(generation);
    let _ = s.newly_counting();
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWNER: &str = "99999999";
    const KEY: &str = "1000000000";

    fn desktop() -> ClaimState {
        ClaimState::new(false)
    }

    #[test]
    fn other_focused_window_does_not_read() {
        let mut s = desktop();
        s.set_claim("main", OWNER, Some(KEY.into()), "p1", 1, 1);
        s.note_focus("main", false);
        s.note_focus("multichat-1", true);
        assert!(!s.reading(OWNER, KEY));
        s.note_focus("multichat-1", false);
        s.note_focus("main", true);
        assert!(s.reading(OWNER, KEY));
    }

    #[test]
    fn hidden_window_claim_does_not_count() {
        let mut s = desktop();
        s.set_claim("main", OWNER, Some(KEY.into()), "p1", 1, 1);
        s.note_focus("main", true);
        s.note_minimized("main", true);
        assert!(!s.reading(OWNER, KEY));
    }

    #[test]
    fn claim_regaining_front_reads_through() {
        let mut s = desktop();
        s.set_claim("main", OWNER, Some(KEY.into()), "p1", 1, 1);
        assert!(s.newly_counting().is_empty());
        s.note_focus("main", true);
        assert_eq!(s.newly_counting(), vec![(OWNER.to_string(), KEY.to_string())]);
        // Still counting: nothing new to read through.
        assert!(s.newly_counting().is_empty());
        s.note_focus("main", false);
        assert!(s.newly_counting().is_empty());
        s.note_focus("main", true);
        assert_eq!(s.newly_counting().len(), 1);
    }

    #[test]
    fn pip_does_not_read() {
        let mut s = ClaimState::new(true);
        s.set_claim("main", OWNER, Some(KEY.into()), "p1", 1, 1);
        s.set_interactive(false);
        assert!(!s.reading(OWNER, KEY));
        s.set_interactive(true);
        assert!(s.reading(OWNER, KEY));
    }

    #[test]
    fn claim_ignored_for_other_owner() {
        let mut s = desktop();
        s.set_claim("main", OWNER, Some(KEY.into()), "p1", 1, 1);
        s.note_focus("main", true);
        assert!(!s.reading("12345678", KEY));
    }

    #[test]
    fn reload_clears_claim() {
        let mut s = desktop();
        s.note_focus("main", true);
        s.set_claim("main", OWNER, Some(KEY.into()), "p1", 1, 1);
        s.clear_label("main");
        assert!(!s.reading(OWNER, KEY));
        // A late call from the page that went away is refused.
        assert!(!s.set_claim("main", OWNER, Some(KEY.into()), "p1", 2, 1));
        assert!(!s.reading(OWNER, KEY));
        // The new page's calls apply, in order.
        assert!(s.set_claim("main", OWNER, None, "p2", 0, 1));
        assert!(s.set_claim("main", OWNER, Some(KEY.into()), "p2", 1, 1));
        assert!(!s.set_claim("main", OWNER, None, "p2", 0, 1));
        assert!(s.reading(OWNER, KEY));
    }

    #[test]
    fn destroyed_window_clears_claim() {
        let mut s = desktop();
        s.note_focus("multichat-2", true);
        s.set_claim("multichat-2", OWNER, Some(KEY.into()), "p1", 1, 1);
        s.forget_window("multichat-2");
        assert!(!s.reading(OWNER, KEY));
    }

    #[test]
    fn a_new_generation_drops_old_claims() {
        let mut s = desktop();
        s.note_focus("main", true);
        s.set_claim("main", OWNER, Some(KEY.into()), "p1", 1, 1);
        s.clear_scope(2);
        assert!(!s.reading(OWNER, KEY));
    }
}
