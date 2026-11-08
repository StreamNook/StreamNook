//! The one source of the inbox's unread counts: mentions and whispers
//! together, plus what the desktop island lists. Every change to either
//! inbox calls `wake`. A short-lived coalescer waits 50 ms, reads both counts
//! fresh, and emits `inbox-changed` only when they differ from what it last
//! published. Only the publish bumps `rev`; a view returns the last published
//! snapshot without bumping, so a read taken before a change can never come
//! back looking newer than the change.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::Emitter;

pub const EVENT: &str = "inbox-changed";

/// How long a wake waits for more changes before reading the counts.
const COALESCE: Duration = Duration::from_millis(50);

/// Which app asks for the inbox view. The phones list everything. The desktop
/// island leaves out mentions that arrived in a chat on screen and the
/// conversations its Clear all put away. MultiChat's Mentions mode leaves out
/// only what Clear all put away (the whisper side applies that rule).
/// `mention_inbox` re-exports it.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Surface {
    #[default]
    Phone,
    Island,
    Multichat,
}

impl Surface {
    pub fn skips_seen_live(self) -> bool {
        self == Surface::Island
    }
}

#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct Counts {
    pub unread_total: u32,
    pub mentions: u32,
    pub whispers: u32,
    pub island: u32,
    /// MultiChat's Mentions mode: every unread mention (it lists those seen
    /// live too) and the whispers after its Clear.
    pub multichat: u32,
    pub rev: u64,
}

/// The counts as read, before a revision is put on them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
struct Fresh {
    mentions: u32,
    whispers: u32,
    island: u32,
    multichat: u32,
}

/// What was last published. Pure, so the rules are tested without an app.
#[derive(Debug, Default)]
struct Coalescer {
    published: Option<Counts>,
}

impl Coalescer {
    /// The counts to emit for a fresh read, or None when nothing changed.
    /// The first read always publishes, so a shell that seeded from a view
    /// gets its first event.
    fn publish(&mut self, fresh: Fresh) -> Option<Counts> {
        let same = self.published.as_ref().is_some_and(|p| {
            p.mentions == fresh.mentions
                && p.whispers == fresh.whispers
                && p.island == fresh.island
                && p.multichat == fresh.multichat
        });
        if same {
            return None;
        }
        let rev = self.published.as_ref().map_or(1, |p| p.rev + 1);
        let counts = Counts {
            unread_total: fresh.mentions + fresh.whispers,
            mentions: fresh.mentions,
            whispers: fresh.whispers,
            island: fresh.island,
            multichat: fresh.multichat,
            rev,
        };
        self.published = Some(counts.clone());
        Some(counts)
    }

    fn snapshot(&self) -> Counts {
        self.published.clone().unwrap_or_default()
    }
}

static COALESCER: Mutex<Coalescer> = Mutex::new(Coalescer { published: None });
static WAKE: tokio::sync::Notify = tokio::sync::Notify::const_new();
static STARTED: AtomicBool = AtomicBool::new(false);

fn read_fresh() -> Fresh {
    let (mentions, mention_island) = crate::services::mention_inbox::badge_counts();
    let (whispers, whisper_island) = crate::services::whisper_inbox::unread_counts();
    Fresh {
        mentions,
        whispers,
        island: mention_island + whisper_island,
        multichat: mentions + whisper_island,
    }
}

fn ensure_started() {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    tauri::async_runtime::spawn(async {
        loop {
            WAKE.notified().await;
            tokio::time::sleep(COALESCE).await;
            // Reading the mention inbox may load it, which wakes this loop
            // once more; the next read finds nothing new and stays quiet.
            let fresh = tauri::async_runtime::spawn_blocking(read_fresh).await.unwrap_or_default();
            let publish = COALESCER.lock().unwrap_or_else(|p| p.into_inner()).publish(fresh);
            if let Some(counts) = publish {
                if let Some(app) = crate::services::providers::app_handle() {
                    let _ = app.emit(EVENT, &counts);
                }
            }
        }
    });
}

/// Some inbox count may have changed. Never waits; many wakes in a burst
/// become one read.
pub fn wake() {
    ensure_started();
    WAKE.notify_one();
}

/// The last published counts (all zero with `rev` 0 before the first
/// publish, which this call then asks for).
pub fn snapshot() -> Counts {
    let counts = COALESCER.lock().unwrap_or_else(|p| p.into_inner()).snapshot();
    if counts.rev == 0 {
        wake();
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh(mentions: u32, whispers: u32, island: u32) -> Fresh {
        Fresh { mentions, whispers, island, multichat: island }
    }

    #[test]
    fn inbox_changed_is_coalesced_and_fresh() {
        let mut c = Coalescer::default();
        let first = c.publish(fresh(1, 2, 3)).expect("the first read always publishes");
        assert_eq!(first, Counts { unread_total: 3, mentions: 1, whispers: 2, island: 3, multichat: 3, rev: 1 });
        // The same counts again: no event.
        assert_eq!(c.publish(fresh(1, 2, 3)), None);
        // Any one count changing publishes the whole fresh set, one rev on.
        let next = c.publish(fresh(1, 2, 1)).expect("the island count changed");
        assert_eq!(next, Counts { unread_total: 3, mentions: 1, whispers: 2, island: 1, multichat: 1, rev: 2 });
        // MultiChat's count alone changing (a mention seen live read elsewhere) publishes too.
        let mut d = Coalescer::default();
        d.publish(Fresh { mentions: 2, whispers: 1, island: 2, multichat: 3 });
        assert!(d.publish(Fresh { mentions: 2, whispers: 1, island: 2, multichat: 2 }).is_some());
        let next = c.publish(fresh(0, 5, 1)).expect("both inboxes changed");
        assert_eq!((next.unread_total, next.rev), (5, 3));
    }

    #[test]
    fn view_read_before_change_never_lowers_count() {
        // A view reads the snapshot (3 unread, rev 1). A whisper lands and the
        // emitter publishes 4 at rev 2. A view answered after that returns the
        // snapshot as published: never the old 3 under a newer rev.
        let mut c = Coalescer::default();
        c.publish(fresh(1, 2, 0));
        let before = c.snapshot();
        c.publish(fresh(1, 3, 0));
        let after = c.snapshot();
        assert_eq!((before.unread_total, before.rev), (3, 1));
        assert_eq!((after.unread_total, after.rev), (4, 2));
        // Reading never bumps the revision.
        assert_eq!(c.snapshot().rev, 2);
        assert_eq!(c.snapshot().rev, 2);
    }

    #[test]
    fn nothing_is_published_before_the_first_read() {
        let c = Coalescer::default();
        assert_eq!(c.snapshot(), Counts::default());
    }

    #[test]
    fn the_event_name_is_the_registered_one() {
        assert_eq!(EVENT, "inbox-changed");
    }
}
