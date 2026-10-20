//! What the watcher is doing, for a person or a monitor to read: a JSON file
//! rewritten every 30 s.
//!
//! `{"connected": 3, "channels": 412, "wanted": 600, "wanted_live": 40, "users": 57,
//!   "last_targets_ok": 1792424183, "last_post_ok": 1792424170, "dropped": 0}`
//! (`channels` joined of `wanted`; times in Unix seconds, null until the first
//! success).

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;

use crate::sign::now_secs;

pub const EVERY: Duration = Duration::from_secs(30);

#[derive(Default)]
pub struct Health {
    pub connected: AtomicUsize,
    pub channels: AtomicUsize,
    /// Channels the last targets read wanted, and how many of them are live.
    pub wanted: AtomicUsize,
    pub wanted_live: AtomicUsize,
    pub users: AtomicUsize,
    pub last_targets_ok: AtomicU64,
    pub last_post_ok: AtomicU64,
    pub dropped: AtomicU64,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Snapshot {
    pub connected: usize,
    pub channels: usize,
    pub wanted: usize,
    pub wanted_live: usize,
    pub users: usize,
    pub last_targets_ok: Option<u64>,
    pub last_post_ok: Option<u64>,
    pub dropped: u64,
}

impl Health {
    pub fn targets_ok(&self) {
        self.last_targets_ok.store(now_secs(), Ordering::Relaxed);
    }

    pub fn post_ok(&self) {
        self.last_post_ok.store(now_secs(), Ordering::Relaxed);
    }

    pub fn drop_mentions(&self, n: u64) {
        self.dropped.fetch_add(n, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> Snapshot {
        let when = |a: &AtomicU64| Some(a.load(Ordering::Relaxed)).filter(|t| *t > 0);
        Snapshot {
            connected: self.connected.load(Ordering::Relaxed),
            channels: self.channels.load(Ordering::Relaxed),
            wanted: self.wanted.load(Ordering::Relaxed),
            wanted_live: self.wanted_live.load(Ordering::Relaxed),
            users: self.users.load(Ordering::Relaxed),
            last_targets_ok: when(&self.last_targets_ok),
            last_post_ok: when(&self.last_post_ok),
            dropped: self.dropped.load(Ordering::Relaxed),
        }
    }
}

/// Rewrite the file every `EVERY`, through a temporary file so a reader never
/// sees half of one.
pub async fn run(health: Arc<Health>, path: PathBuf) {
    let mut tick = tokio::time::interval(EVERY);
    loop {
        tick.tick().await;
        let json = match serde_json::to_vec_pretty(&health.snapshot()) {
            Ok(json) => json,
            Err(e) => {
                tracing::warn!("health: could not serialize: {e}");
                continue;
            }
        };
        let tmp = path.with_extension("tmp");
        let written = async {
            tokio::fs::write(&tmp, &json).await?;
            tokio::fs::rename(&tmp, &path).await
        }
        .await;
        if let Err(e) = written {
            tracing::warn!("health: could not write {}: {e}", path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_read_null_until_the_first_success() {
        let h = Health::default();
        h.channels.store(4, Ordering::Relaxed);
        h.drop_mentions(2);
        let s = h.snapshot();
        assert_eq!(s.last_targets_ok, None);
        assert_eq!(s.channels, 4);
        assert_eq!(s.dropped, 2);
        h.targets_ok();
        assert!(h.snapshot().last_targets_ok.is_some());
    }
}
