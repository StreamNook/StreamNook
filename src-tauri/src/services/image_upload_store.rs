//! What the image uploader keeps, owned by the backend alone, in
//! `image_uploads/` under the app's data folder: `uploads.json` (the hosts
//! the viewer said yes to, since the first upload to a host asks once, and
//! the uploads, newest first), `files/` (a copy of each picture sent, so it
//! can be sent again, also after its link dies) and `thumbs/` (a small
//! picture of each, which is all the Images section draws, so full-size
//! pictures never sit decoded in the webview's memory).

use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// How many uploads the section keeps.
pub const MAX_RECENTS: usize = 100;
/// The kept files, all together, at most this big; the oldest go first.
const MAX_FILE_BYTES: u64 = 1024 * 1024 * 1024;

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
    /// The kept file's name in `files/`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copy: Option<String>,
    /// The small picture's name in `thumbs/`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumb: Option<String>,
    /// The file sent, its size and kind ("png", "jpg"); 0 and empty on
    /// entries from before they were recorded.
    #[serde(default)]
    pub bytes: u64,
    #[serde(default)]
    pub ext: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct Store {
    accepted_hosts: Vec<String>,
    recents: Vec<RecentUpload>,
}

static STORE: Mutex<Option<Store>> = Mutex::new(None);

/// The folder everything here lives in.
pub fn folder() -> Option<PathBuf> {
    let dir = crate::services::cache_service::get_app_data_dir().ok()?.join("image_uploads");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

fn file() -> Option<PathBuf> {
    folder().map(|d| d.join("uploads.json"))
}

fn sub(name: &str) -> Option<PathBuf> {
    let dir = folder()?.join(name);
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// Where a kept file lives on disk.
pub fn copy_path(name: &str) -> Option<PathBuf> {
    sub("files").map(|d| d.join(name))
}

/// Where a small picture lives on disk.
pub fn thumb_path(name: &str) -> Option<PathBuf> {
    sub("thumbs").map(|d| d.join(name))
}

/// The store from before it moved out of the cache folder, with its kept
/// files moved along.
fn migrate_from_cache() -> Option<Store> {
    let cache = crate::services::cache_service::get_cache_dir().ok()?;
    let old = cache.join("image_uploads.json");
    let store = serde_json::from_slice::<Store>(&std::fs::read(&old).ok()?).ok()?;
    for name in store.recents.iter().filter_map(|r| r.copy.as_deref()) {
        if let Some(to) = copy_path(name) {
            let from = cache.join("image_uploads").join(name);
            if std::fs::rename(&from, &to).is_err() && std::fs::copy(&from, &to).is_ok() {
                let _ = std::fs::remove_file(&from);
            }
        }
    }
    let _ = std::fs::remove_file(&old);
    let _ = std::fs::remove_dir(cache.join("image_uploads"));
    Some(store)
}

fn with_store<R>(write: bool, f: impl FnOnce(&mut Store) -> R) -> R {
    let mut guard = STORE.lock().unwrap_or_else(|e| e.into_inner());
    let mut migrated = false;
    let store = guard.get_or_insert_with(|| {
        file()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|bytes| serde_json::from_slice::<Store>(&bytes).ok())
            .or_else(|| {
                migrated = true;
                migrate_from_cache()
            })
            .unwrap_or_default()
    });
    let out = f(store);
    if write || migrated {
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
/// while its file does (it can be sent again); the rest are dropped.
pub fn recents(now_ms: i64) -> Vec<RecentUpload> {
    with_store(false, |s| shown(&s.recents, now_ms, |name| copy_path(name).is_some_and(|p| p.is_file())))
}

pub fn find(url: &str) -> Option<RecentUpload> {
    with_store(false, |s| s.recents.iter().find(|r| r.url == url).cloned())
}

fn keep(dir: Option<PathBuf>, bytes: &[u8], ext: &str) -> Option<String> {
    let name = format!("{:016x}.{ext}", rand::random::<u64>());
    std::fs::write(dir?.join(&name), bytes).ok()?;
    Some(name)
}

/// Saves the picture sent; the file name to store on its entry.
pub fn keep_copy(bytes: &[u8], ext: &str) -> Option<String> {
    keep(sub("files"), bytes, ext)
}

/// Saves a small picture; the file name to store on its entry.
pub fn keep_thumb(bytes: &[u8], ext: &str) -> Option<String> {
    keep(sub("thumbs"), bytes, ext)
}

/// Gives an entry its small picture (made after the fact, for an upload from
/// before small pictures were kept).
pub fn set_thumb(url: &str, name: String) {
    let stale = with_store(true, |s| {
        let r = s.recents.iter_mut().find(|r| r.url == url)?;
        r.thumb.replace(name)
    });
    remove_files(&[], &stale.into_iter().collect::<Vec<_>>());
}

/// The entries still without a small picture: their link, and their kept
/// file when there is one.
pub fn missing_thumbs() -> Vec<(String, Option<String>)> {
    with_store(false, |s| {
        s.recents
            .iter()
            .filter(|r| r.thumb.as_deref().and_then(thumb_path).is_none_or(|p| !p.is_file()))
            .map(|r| (r.url.clone(), r.copy.clone()))
            .collect()
    })
}

/// Adds an upload (or replaces the one with `replacing`'s link, keeping its
/// place), then trims the list and the kept files to their caps.
pub fn remember(upload: RecentUpload, replacing: Option<&str>) {
    let (files, thumbs) = with_store(true, |s| {
        let mut gone: Vec<RecentUpload> = Vec::new();
        if let Some(old) = replacing.and_then(|u| s.recents.iter().position(|r| r.url == u)) {
            let prev = std::mem::replace(&mut s.recents[old], upload);
            let now = &s.recents[old];
            gone.push(RecentUpload {
                copy: prev.copy.filter(|c| now.copy.as_ref() != Some(c)),
                thumb: prev.thumb.filter(|t| now.thumb.as_ref() != Some(t)),
                ..Default::default()
            });
        } else {
            gone.extend(push(&mut s.recents, upload));
        }
        let mut files: Vec<String> = gone.iter().filter_map(|r| r.copy.clone()).collect();
        let thumbs: Vec<String> = gone.iter().filter_map(|r| r.thumb.clone()).collect();
        files.extend(trim_copies(&mut s.recents));
        (files, thumbs)
    });
    remove_files(&files, &thumbs);
}

/// Takes an upload off the list, and its files off the disk.
pub fn forget(url: &str) -> Option<RecentUpload> {
    let gone = with_store(true, |s| {
        let at = s.recents.iter().position(|r| r.url == url)?;
        Some(s.recents.remove(at))
    })?;
    remove_files(gone.copy.as_slice(), gone.thumb.as_slice());
    Some(gone)
}

/// What the kept files and small pictures take on disk.
pub fn disk_bytes() -> u64 {
    let size = |dir: Option<PathBuf>| -> u64 {
        dir.and_then(|d| std::fs::read_dir(d).ok())
            .map(|entries| entries.flatten().filter_map(|e| e.metadata().ok()).map(|m| m.len()).sum())
            .unwrap_or(0)
    };
    size(sub("files")) + size(sub("thumbs"))
}

fn remove_files(files: &[String], thumbs: &[String]) {
    for path in files.iter().filter_map(|n| copy_path(n)).chain(thumbs.iter().filter_map(|n| thumb_path(n))) {
        let _ = std::fs::remove_file(path);
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

/// Drops the oldest kept files past `MAX_FILE_BYTES` (their entries stay,
/// without a file); returns the file names to delete.
fn trim_copies(list: &mut [RecentUpload]) -> Vec<String> {
    let size = |name: &str| copy_path(name).and_then(|p| std::fs::metadata(p).ok()).map_or(0, |m| m.len());
    let mut total: u64 = 0;
    let mut dropped = Vec::new();
    for r in list.iter_mut() {
        let Some(name) = r.copy.clone() else { continue };
        total += size(&name);
        if total > MAX_FILE_BYTES {
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
