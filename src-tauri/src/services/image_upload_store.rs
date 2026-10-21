//! What the image uploader remembers between runs, beside the settings and
//! owned by the backend alone: the hosts the viewer said yes to (the first
//! upload to a host asks once) and the images they uploaded, newest first,
//! for the emote menu's Images section. Kept in `image_uploads.json` in the
//! cache folder. A picture sent to a host whose links expire also keeps a
//! copy in `image_uploads/`, so it can be sent again after its link dies.

use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// How many uploads the section keeps.
pub const MAX_RECENTS: usize = 100;
/// The kept copies, all together, at most this big; the oldest go first.
const MAX_COPY_BYTES: u64 = 300 * 1024 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RecentUpload {
    pub url: String,
    pub width: u32,
    pub height: u32,
    pub at_ms: i64,
    /// The host key it went to (`image_upload::Target::key`).
    pub host: String,
    /// When the host deletes it, if it does (Litterbox, uguu, or what the
    /// host's reply said).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_ms: Option<i64>,
    /// The URL that deletes it on the host (POST), where the host has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delete_url: Option<String>,
    /// The kept copy's file name in `image_uploads/`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copy: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct Store {
    accepted_hosts: Vec<String>,
    recents: Vec<RecentUpload>,
}

static STORE: Mutex<Option<Store>> = Mutex::new(None);

fn file() -> Option<PathBuf> {
    crate::services::cache_service::get_cache_dir().ok().map(|d| d.join("image_uploads.json"))
}

fn copies_dir() -> Option<PathBuf> {
    crate::services::cache_service::get_cache_dir().ok().map(|d| d.join("image_uploads"))
}

/// Where a kept copy lives on disk.
pub fn copy_path(name: &str) -> Option<PathBuf> {
    copies_dir().map(|d| d.join(name))
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

/// The uploads to show at `now_ms`, newest first: an expired link stays only
/// while its copy does (it can be sent again); the rest are dropped.
pub fn recents(now_ms: i64) -> Vec<RecentUpload> {
    with_store(false, |s| shown(&s.recents, now_ms, |name| copy_path(name).is_some_and(|p| p.is_file())))
}

pub fn find(url: &str) -> Option<RecentUpload> {
    with_store(false, |s| s.recents.iter().find(|r| r.url == url).cloned())
}

/// Saves a copy of `bytes` for an upload whose link will expire; the file
/// name to store on its entry.
pub fn keep_copy(bytes: &[u8], ext: &str) -> Option<String> {
    let dir = copies_dir()?;
    std::fs::create_dir_all(&dir).ok()?;
    let name = format!("{:016x}.{ext}", rand::random::<u64>());
    std::fs::write(dir.join(&name), bytes).ok()?;
    Some(name)
}

/// Adds an upload (or replaces the one with `replacing`'s link, keeping its
/// place), then trims the list and the kept copies to their caps.
pub fn remember(upload: RecentUpload, replacing: Option<&str>) {
    let dropped = with_store(true, |s| {
        let mut dropped = Vec::new();
        if let Some(old) = replacing.and_then(|u| s.recents.iter().position(|r| r.url == u)) {
            let prev = std::mem::replace(&mut s.recents[old], upload);
            if prev.copy != s.recents[old].copy {
                dropped.extend(prev.copy);
            }
        } else {
            dropped.extend(push(&mut s.recents, upload).into_iter().filter_map(|r| r.copy));
        }
        dropped.extend(trim_copies(&mut s.recents));
        dropped
    });
    remove_copies(&dropped);
}

/// Takes an upload off the list, and its kept copy off the disk.
pub fn forget(url: &str) -> Option<RecentUpload> {
    let gone = with_store(true, |s| {
        let at = s.recents.iter().position(|r| r.url == url)?;
        Some(s.recents.remove(at))
    })?;
    remove_copies(gone.copy.as_slice());
    Some(gone)
}

fn remove_copies(names: &[String]) {
    for name in names {
        if let Some(path) = copy_path(name) {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn shown(list: &[RecentUpload], now_ms: i64, has_copy: impl Fn(&str) -> bool) -> Vec<RecentUpload> {
    list.iter()
        .filter(|r| r.expires_ms.is_none_or(|e| e > now_ms) || r.copy.as_deref().is_some_and(&has_copy))
        .cloned()
        .collect()
}

/// Puts `upload` first, one entry per link, at most `MAX_RECENTS`; returns
/// what fell off the end.
fn push(list: &mut Vec<RecentUpload>, upload: RecentUpload) -> Vec<RecentUpload> {
    let mut dropped: Vec<RecentUpload> = Vec::new();
    if let Some(at) = list.iter().position(|r| r.url == upload.url) {
        dropped.push(list.remove(at));
    }
    list.insert(0, upload);
    if list.len() > MAX_RECENTS {
        dropped.extend(list.split_off(MAX_RECENTS));
    }
    dropped
}

/// Drops the oldest kept copies past `MAX_COPY_BYTES` (their entries stay,
/// without a copy); returns the file names to delete.
fn trim_copies(list: &mut [RecentUpload]) -> Vec<String> {
    let size = |name: &str| copy_path(name).and_then(|p| std::fs::metadata(p).ok()).map_or(0, |m| m.len());
    let mut total: u64 = 0;
    let mut dropped = Vec::new();
    for r in list.iter_mut() {
        let Some(name) = r.copy.clone() else { continue };
        total += size(&name);
        if total > MAX_COPY_BYTES {
            dropped.push(name);
            r.copy = None;
        }
    }
    dropped
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
        for i in 0..(MAX_RECENTS + 5) {
            push(&mut list, up(&format!("https://i.nuuls.com/{i}.png"), None));
        }
        let dropped = push(&mut list, up("https://i.nuuls.com/5.png", None));
        assert_eq!(list.len(), MAX_RECENTS);
        assert_eq!(list[0].url, "https://i.nuuls.com/5.png");
        assert_eq!(list.iter().filter(|r| r.url.ends_with("/5.png")).count(), 1);
        assert_eq!(dropped.len(), 1, "the old copy of the same link");
    }

    #[test]
    fn expired_links_stay_only_with_a_copy() {
        let mut kept = up("c", Some(100));
        kept.copy = Some("c.png".into());
        let list = vec![up("a", Some(100)), up("b", None), kept, up("d", Some(300))];
        let urls: Vec<_> = shown(&list, 200, |name| name == "c.png").into_iter().map(|r| r.url).collect();
        assert_eq!(urls, ["b", "c", "d"]);
    }
}
