//! What the image uploader remembers between runs, beside the settings and
//! owned by the backend alone: the hosts the viewer said yes to (the first
//! upload to a host asks once) and the images they uploaded most recently,
//! newest first, for the emote menu's Images section. Kept in
//! `image_uploads.json` in the cache folder.

use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// How many recent uploads the section shows.
pub const MAX_RECENTS: usize = 24;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RecentUpload {
    pub url: String,
    pub width: u32,
    pub height: u32,
    pub at_ms: i64,
    /// The host key it went to (`image_upload::Target::key`).
    pub host: String,
    /// When the host deletes it, if it does (Litterbox, uguu).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_ms: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct Store {
    accepted_hosts: Vec<String>,
    recents: Vec<RecentUpload>,
}

static STORE: Mutex<Option<Store>> = Mutex::new(None);

fn file() -> Option<std::path::PathBuf> {
    crate::services::cache_service::get_cache_dir().ok().map(|d| d.join("image_uploads.json"))
}

fn with_store<R>(write: bool, f: impl FnOnce(&mut Store) -> R) -> R {
    let mut guard = STORE.lock().unwrap_or_else(|e| e.into_inner());
    let store = guard.get_or_insert_with(|| {
        file()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|bytes| serde_json::from_slice::<Store>(&bytes).ok())
            .unwrap_or_default()
    });
    let out = f(store);
    if write {
        if let (Some(path), Ok(bytes)) = (file(), serde_json::to_vec(store)) {
            if let Err(e) = std::fs::write(&path, bytes) {
                log::warn!("[image_upload] could not save {}: {}", path.display(), e);
            }
        }
    }
    out
}

pub fn accepted(host: &str) -> bool {
    with_store(false, |s| s.accepted_hosts.iter().any(|h| h == host))
}

pub fn accept(host: &str) {
    with_store(true, |s| {
        if !s.accepted_hosts.iter().any(|h| h == host) {
            s.accepted_hosts.push(host.to_string());
        }
    })
}

/// The recent uploads still alive at `now_ms`, newest first.
pub fn recents(now_ms: i64) -> Vec<RecentUpload> {
    with_store(false, |s| alive(&s.recents, now_ms))
}

pub fn remember(upload: RecentUpload) {
    with_store(true, |s| push(&mut s.recents, upload))
}

fn alive(list: &[RecentUpload], now_ms: i64) -> Vec<RecentUpload> {
    list.iter().filter(|r| r.expires_ms.is_none_or(|e| e > now_ms)).cloned().collect()
}

fn push(list: &mut Vec<RecentUpload>, upload: RecentUpload) {
    list.retain(|r| r.url != upload.url);
    list.insert(0, upload);
    list.truncate(MAX_RECENTS);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn up(url: &str, expires_ms: Option<i64>) -> RecentUpload {
        RecentUpload { url: url.into(), expires_ms, ..Default::default() }
    }

    #[test]
    fn newest_first_one_copy_capped() {
        let mut list = Vec::new();
        for i in 0..30 {
            push(&mut list, up(&format!("https://i.nuuls.com/{i}.png"), None));
        }
        push(&mut list, up("https://i.nuuls.com/5.png", None));
        assert_eq!(list.len(), MAX_RECENTS);
        assert_eq!(list[0].url, "https://i.nuuls.com/5.png");
        assert_eq!(list.iter().filter(|r| r.url.ends_with("/5.png")).count(), 1);
    }

    #[test]
    fn expired_links_are_left_out() {
        let list = vec![up("a", Some(100)), up("b", None), up("c", Some(300))];
        let urls: Vec<_> = alive(&list, 200).into_iter().map(|r| r.url).collect();
        assert_eq!(urls, ["b", "c"]);
    }
}
