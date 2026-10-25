use anyhow::{Context, Result};
use log::debug;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex as StdMutex;
use std::sync::RwLock as StdRwLock;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex as TokioMutex;

use crate::services::cache_service::get_cache_dir;

/// Held for the whole of a manifest flush and of a clear, so manifest writes
/// never overlap and a clear waits for a flush in progress.
///
/// Lock order, outermost first: `FLUSH_LOCK`, `MANIFEST_LOCK`,
/// `MANIFEST_MEMORY`, `FOREIGN_ENTRIES`. A thread that holds one never takes an
/// earlier one.
static FLUSH_LOCK: StdMutex<()> = StdMutex::new(());

// Global mutex to prevent concurrent manifest writes (for sync operations)
static MANIFEST_LOCK: Lazy<StdMutex<()>> = Lazy::new(|| StdMutex::new(()));

/// Manifest entries this build could not read (a newer build's cache type),
/// kept as they were so writing the manifest back never drops them. Cleared
/// only with the cache.
static FOREIGN_ENTRIES: Lazy<StdMutex<Vec<(String, serde_json::Value)>>> = Lazy::new(|| StdMutex::new(Vec::new()));

/// A manifest's `entries`, read one at a time: each converts as it is read, and
/// one this build cannot read is set aside instead of failing the whole file.
#[derive(Default)]
struct LenientEntries {
    known: HashMap<String, UniversalCacheEntry>,
    foreign: Vec<(String, serde_json::Value)>,
}

impl<'de> Deserialize<'de> for LenientEntries {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct EntriesVisitor;
        impl<'de> serde::de::Visitor<'de> for EntriesVisitor {
            type Value = LenientEntries;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a map of cache entries")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> std::result::Result<LenientEntries, A::Error> {
                let mut out = LenientEntries {
                    known: HashMap::with_capacity(map.size_hint().unwrap_or(0)),
                    foreign: Vec::new(),
                };
                while let Some((key, value)) = map.next_entry::<String, serde_json::Value>()? {
                    match UniversalCacheEntry::deserialize(&value) {
                        Ok(entry) => {
                            out.known.insert(key, entry);
                        }
                        Err(_) => out.foreign.push((key, value)),
                    }
                }
                Ok(out)
            }
        }
        deserializer.deserialize_map(EntriesVisitor)
    }
}

#[derive(Deserialize)]
struct LenientManifest {
    version: u32,
    last_sync: Option<u64>,
    #[serde(default)]
    entries: LenientEntries,
}

/// Read a manifest, setting aside any entry this build cannot read.
fn parse_manifest(json: &str) -> serde_json::Result<(UniversalCacheManifest, Vec<(String, serde_json::Value)>)> {
    let raw: LenientManifest = serde_json::from_str(json)?;
    Ok((
        UniversalCacheManifest { version: raw.version, last_sync: raw.last_sync, entries: raw.entries.known },
        raw.entries.foreign,
    ))
}

/// `entries` as written: this build's entries, then each set-aside one it has
/// no entry for under the same key, without building a second map.
struct EntriesOut<'a> {
    known: &'a HashMap<String, UniversalCacheEntry>,
    foreign: &'a [(String, serde_json::Value)],
}

impl Serialize for EntriesOut<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let extra = self.foreign.iter().filter(|(k, _)| !self.known.contains_key(k)).count();
        let mut map = serializer.serialize_map(Some(self.known.len() + extra))?;
        for (k, v) in self.known {
            map.serialize_entry(k, v)?;
        }
        for (k, v) in self.foreign {
            if !self.known.contains_key(k) {
                map.serialize_entry(k, v)?;
            }
        }
        map.end()
    }
}

#[derive(Serialize)]
struct ManifestOut<'a> {
    version: u32,
    last_sync: Option<u64>,
    entries: EntriesOut<'a>,
}

/// The manifest as written to disk, with the set-aside entries back in.
fn encode_manifest(manifest: &UniversalCacheManifest) -> serde_json::Result<Vec<u8>> {
    let foreign = FOREIGN_ENTRIES.lock().unwrap_or_else(|e| e.into_inner());
    encode_with(manifest, foreign.as_slice())
}

/// `encode_manifest` with the set-aside entries given.
fn encode_with(manifest: &UniversalCacheManifest, foreign: &[(String, serde_json::Value)]) -> serde_json::Result<Vec<u8>> {
    if foreign.is_empty() {
        return serde_json::to_vec(manifest);
    }
    serde_json::to_vec(&ManifestOut {
        version: manifest.version,
        last_sync: manifest.last_sync,
        entries: EntriesOut { known: &manifest.entries, foreign },
    })
}

// Async mutex for async operations
static ASYNC_MANIFEST_LOCK: Lazy<TokioMutex<()>> = Lazy::new(|| TokioMutex::new(()));

// Flag to prevent concurrent downloads
static DOWNLOAD_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

// One shared HTTP client for ALL cache-file downloads (emotes + badges).
// Reusing a single client keeps its connection pool warm, so concurrent
// downloads to the same CDN host multiplex over one HTTP/2 connection instead
// of each paying a fresh TCP + TLS handshake. The timeouts are load-bearing:
// without them a hung/half-open CDN response would await forever, never freeing
// its download-queue slot, and a few of those would permanently wedge caching.
static DOWNLOAD_CLIENT: Lazy<reqwest::Client> = Lazy::new(|| {
    reqwest::Client::builder()
        .user_agent("StreamNook/1.0")
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .expect("failed to build shared cache-download HTTP client")
});

// The metadata/index fetches below used to build a fresh `Client` per call, each
// one carrying its own connection pool and TLS session for a single request.
// They are kept as three separate clients rather than folded into the shared
// `services::http` one because their timeouts are deliberately different from
// its 30s default, and one of them runs in a loop of up to 100 items where a
// longer timeout would change worst-case runtime by a lot.
fn cache_meta_client(secs: u64) -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent("StreamNook/1.0")
        .timeout(std::time::Duration::from_secs(secs))
        .build()
        .expect("failed to build universal-cache metadata HTTP client")
}
/// Single-entry lookups, in a loop of up to 100. Short on purpose.
static META_CLIENT_5S: Lazy<reqwest::Client> = Lazy::new(|| cache_meta_client(5));
/// Index files and remote-manifest probes.
static META_CLIENT_10S: Lazy<reqwest::Client> = Lazy::new(|| cache_meta_client(10));
/// The bulk index download, which is a much larger body.
static META_CLIENT_30S: Lazy<reqwest::Client> = Lazy::new(|| cache_meta_client(30));

// In-memory mirror of the on-disk manifest. Initialized from disk on first
// access; every subsequent read/write hits memory only. A background task
// flushes dirty state to disk on a MANIFEST_FLUSH_DEBOUNCE_SECS debounce (or on next tick if a
// flush failed). The public load_manifest/save_manifest functions keep their
// existing signatures so the dozens of read-modify-write call sites work
// unchanged. The pre-existing MANIFEST_LOCK / ASYNC_MANIFEST_LOCK continue to
// serialize RMW sequences at the call site, so concurrent writes still don't
// race here.
static MANIFEST_MEMORY: Lazy<StdRwLock<UniversalCacheManifest>> =
    Lazy::new(|| StdRwLock::new(load_manifest_from_disk().unwrap_or_default()));

// True when the in-memory manifest has unflushed changes. The background
// flush task clears this on a successful disk write; sets it back if write
// failed so the next tick retries.
static MANIFEST_DIRTY: AtomicBool = AtomicBool::new(false);
/// Seconds between debounced manifest flushes. See ensure_flush_task.
const MANIFEST_FLUSH_DEBOUNCE_SECS: u64 = 30;

// Background flush task is started lazily on first write so we don't spawn
// anything if the app never modifies the manifest (read-only sessions). The
// handle, not a flag: Android's notification worker writes from a short-lived
// runtime of its own, and a task on a runtime that has since been dropped is
// gone. A flag would still read "started", and the app reusing that process
// would never persist the manifest again.
static FLUSH_TASK: Lazy<StdMutex<Option<tokio::task::JoinHandle<()>>>> = Lazy::new(|| StdMutex::new(None));

fn ensure_flush_task() {
    let Ok(mut task) = FLUSH_TASK.lock() else {
        return;
    };
    if task.as_ref().is_some_and(|t| !t.is_finished()) {
        return;
    }
    {
        // The flush loop needs a Tokio runtime. Some callers (e.g. the one-time
        // emote-cache migration invoked from main() before the async runtime is
        // built) would panic here with "no reactor running". When there's no
        // runtime yet, bail: MANIFEST_DIRTY stays set, so the next save once
        // the runtime is up starts the task and nothing is lost. Callers that
        // must persist before then write through to disk directly (see
        // migrate_emote_cache_on_version_change).
        if tokio::runtime::Handle::try_current().is_err() {
            return;
        }
        *task = Some(tokio::spawn(async {
            let mut consecutive_failures: u32 = 0;
            loop {
                // Every flush rewrites the whole manifest (tens of thousands of
                // entries, ~20 MB on a long-lived install), and a busy chat dirties
                // it on every newly cached emote. Measured at 5 s this rewrote the
                // file every 15-30 s for the whole session; 30 s bounds the disk
                // and CPU cost at two writes a minute worst case. Controlled exits
                // still flush synchronously (flush_manifest_now); a crash loses at
                // most the last window of entries, whose files self-heal on the
                // next cache of the same id.
                tokio::time::sleep(std::time::Duration::from_secs(MANIFEST_FLUSH_DEBOUNCE_SECS)).await;
                if !MANIFEST_DIRTY.swap(false, Ordering::AcqRel) {
                    continue;
                }
                // On the blocking pool: the manifest can be large JSON, and a
                // sync write here (on this runtime task) stalls every async task
                // in the process — the same sync-fs-on-runtime class behind the
                // `rt_stall` freezes. The flush serializes under the read lock
                // rather than cloning the whole entry map first (the clone
                // doubled the manifest's memory for the duration of every
                // flush), and drops the guard BEFORE touching the disk so
                // writers only wait for the encode, never for the file write.
                let flushed = tokio::task::spawn_blocking(flush_manifest_to_disk).await;
                if !matches!(flushed, Ok(Ok(()))) {
                    consecutive_failures += 1;
                    if let Ok(Err(e)) = flushed {
                        // One failure is routine (a scanner holding the file);
                        // a run of them means nothing is being persisted, which
                        // must be visible in a normal log.
                        if consecutive_failures >= 2 {
                            log::warn!("[UniversalCache] Manifest flush has failed {consecutive_failures} times in a row (will keep retrying): {e}");
                        } else {
                            debug!("[UniversalCache] Debounced manifest flush failed (will retry): {e}");
                        }
                    }
                    MANIFEST_DIRTY.store(true, Ordering::Release);
                } else {
                    consecutive_failures = 0;
                }
            }
        }));
    }
}

/// Universal cache entry for badges, emotes, and other assets
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UniversalCacheEntry {
    pub id: String,
    pub cache_type: CacheType,
    pub data: serde_json::Value,
    pub metadata: CacheMetadata,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<u32>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum CacheType {
    Badge,
    Emote,
    #[serde(rename = "thirdpartybadge")]
    ThirdPartyBadge,
    Cosmetic,
    /// StreamNook's own cosmetic art (badges, relics, frames) from
    /// cdn.streamnook.app, keyed by its full URL. A new upload ships under a new
    /// URL, so these files never go stale by age.
    #[serde(rename = "streamnookart")]
    StreamNookArt,
}

impl CacheType {
    /// The type's folder, on disk and in the hosted universal cache.
    fn dir_name(&self) -> &'static str {
        match self {
            CacheType::Badge => "badges",
            CacheType::Emote => "emotes",
            CacheType::ThirdPartyBadge => "third-party-badges",
            CacheType::Cosmetic => "cosmetics",
            CacheType::StreamNookArt => "streamnook-art",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CacheMetadata {
    pub timestamp: u64,
    pub expiry_days: u32,
    pub source: String, // e.g., "twitch", "bttv", "7tv", "ffz", "badgebase", "universal"
    pub version: u32,   // Cache format version for future compatibility
}

/// Universal cache manifest - tracks all cached items
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct UniversalCacheManifest {
    pub version: u32,
    pub last_sync: Option<u64>,
    pub entries: HashMap<String, UniversalCacheEntry>,
}

const CACHE_VERSION: u32 = 1;
// The daily cache job commits to its own branch: releases force-push main,
// which erased every cache commit made there and froze main's copy.
const UNIVERSAL_CACHE_URL: &str =
    "https://raw.githubusercontent.com/StreamNook/StreamNook/refs/heads/badge-cache/universal-cache/main";
/// main's copy, which stands in when the branch has no such file: before the
/// cache job has first run there (the branch does not exist yet), and for
/// per-item files only main carries.
const UNIVERSAL_CACHE_FALLBACK_URL: &str =
    "https://raw.githubusercontent.com/StreamNook/StreamNook/refs/heads/main/universal-cache/main";

/// GETs `path` under the universal cache, from the badge-cache branch, or from
/// main when the branch answers 404.
async fn fetch_cache_file(client: &reqwest::Client, path: &str) -> reqwest::Result<reqwest::Response> {
    let primary = client.get(format!("{UNIVERSAL_CACHE_URL}/{path}")).send().await?;
    if primary.status() == reqwest::StatusCode::NOT_FOUND {
        return client.get(format!("{UNIVERSAL_CACHE_FALLBACK_URL}/{path}")).send().await;
    }
    Ok(primary)
}

/// Get the universal cache directory
pub fn get_universal_cache_dir() -> Result<PathBuf> {
    let cache_dir = get_cache_dir()?.join("universal");

    if !cache_dir.exists() {
        fs::create_dir_all(&cache_dir).context("Failed to create universal cache directory")?;
    }

    Ok(cache_dir)
}

/// Get current timestamp in seconds
fn get_current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// Check if cache entry is expired
/// Note: expiry_days of 0 means never expire (permanent cache)
fn is_cache_expired(metadata: &CacheMetadata) -> bool {
    is_expired_at(metadata, get_current_timestamp())
}

/// Whether the entry is expired at `now` (seconds since the epoch).
fn is_expired_at(metadata: &CacheMetadata, now: u64) -> bool {
    // If expiry_days is 0, never expire
    if metadata.expiry_days == 0 {
        return false;
    }

    let expiry_seconds = metadata.expiry_days as u64 * 24 * 60 * 60;
    now > metadata.timestamp + expiry_seconds
}

/// Read the manifest from disk. Private; callers should use `load_manifest()`
/// which serves the in-memory mirror. This is only invoked once per app
/// session (during MANIFEST_MEMORY's lazy init).
fn load_manifest_from_disk() -> Result<UniversalCacheManifest> {
    let manifest_path = get_universal_cache_dir()?.join("manifest.json");

    if !manifest_path.exists() {
        return Ok(UniversalCacheManifest {
            version: CACHE_VERSION,
            last_sync: None,
            entries: HashMap::new(),
        });
    }

    let json = match fs::read_to_string(&manifest_path) {
        Ok(json) => json,
        Err(e) => {
            debug!("[UniversalCache] Failed to read manifest file: {}", e);
            return Ok(UniversalCacheManifest {
                version: CACHE_VERSION,
                last_sync: None,
                entries: HashMap::new(),
            });
        }
    };

    match parse_manifest(&json) {
        Ok((manifest, foreign)) => {
            if !foreign.is_empty() {
                log::info!("[UniversalCache] kept {} manifest entries this build cannot read", foreign.len());
                *FOREIGN_ENTRIES.lock().unwrap_or_else(|e| e.into_inner()) = foreign;
            }
            Ok(manifest)
        }
        Err(e) => {
            debug!(
                "[UniversalCache] Failed to parse manifest ({}), creating new one",
                e
            );
            let backup_path = manifest_path.with_extension("json.backup");
            let _ = fs::rename(&manifest_path, &backup_path);
            debug!(
                "[UniversalCache] Backed up corrupted manifest to {:?}",
                backup_path
            );

            Ok(UniversalCacheManifest {
                version: CACHE_VERSION,
                last_sync: None,
                entries: HashMap::new(),
            })
        }
    }
}

/// Write the manifest to disk. Private; callers should use `save_manifest()`
/// which updates the in-memory mirror and schedules a debounced flush. This
/// is only invoked by the background flush task.
fn save_manifest_to_disk(manifest: &UniversalCacheManifest) -> Result<()> {
    let _flush = FLUSH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // Compact, not pretty: nothing reads this file by eye, and the indentation
    // was ~22% of a 22 MB file rewritten many times per session.
    let json = encode_manifest(manifest)?;
    write_manifest_bytes(&json)
}

/// Replace manifest.json with `json` atomically: write a sibling, then rename
/// over the original, so a crash mid-write can never leave a truncated manifest
/// behind (the load path treats parse failure as "start over", which would
/// silently forget every cached file). std::fs::rename replaces an existing
/// destination on Windows (MoveFileExW with replace) and on Unix. The caller
/// holds `FLUSH_LOCK`, since every write goes through the same sibling file.
fn write_manifest_bytes(json: &[u8]) -> Result<()> {
    let manifest_path = get_universal_cache_dir()?.join("manifest.json");
    write_manifest_bytes_to(&manifest_path, json)
}

/// `write_manifest_bytes` for the manifest at `manifest_path`.
fn write_manifest_bytes_to(manifest_path: &Path, json: &[u8]) -> Result<()> {
    use std::io::Write;
    let tmp_path = manifest_path.with_extension("json.tmp");
    {
        let mut file = fs::File::create(&tmp_path).context("Failed to create manifest temp file")?;
        file.write_all(json).context("Failed to write manifest file")?;
        // Flush the data before the rename lands in the journal: without it a
        // power loss can leave a correctly named, zero-filled manifest.
        file.sync_all().context("Failed to sync manifest file")?;
    }
    fs::rename(&tmp_path, manifest_path).context("Failed to replace manifest file")?;
    Ok(())
}

/// Return a clone of the in-memory manifest. The first call lazily reads from
/// disk; subsequent calls are pure memory reads. Existing read-modify-write
/// call sites work unchanged because the returned clone can be mutated
/// without affecting the mirror until `save_manifest` is called.
pub fn load_manifest() -> Result<UniversalCacheManifest> {
    let guard = MANIFEST_MEMORY
        .read()
        .map_err(|_| anyhow::anyhow!("manifest memory lock poisoned"))?;
    Ok(guard.clone())
}

/// Replace the in-memory manifest with `manifest`, then schedule a debounced
/// disk flush. Concurrent writers are still expected to serialize via the
/// pre-existing MANIFEST_LOCK / ASYNC_MANIFEST_LOCK at the call site — this
/// function does not protect against load-modify-save races.
pub fn save_manifest(manifest: &UniversalCacheManifest) -> Result<()> {
    // Clone BEFORE taking the lock. Cloning tens of thousands of entries takes
    // ~80 ms on a long-lived install, and std's RwLock is writer-preferring on
    // Windows: holding the write lock across that clone parked every reader,
    // including the per-message emote lookups on the chat path.
    save_manifest_owned(manifest.clone())
}

/// `save_manifest` for callers that own their copy and are done with it. Moves
/// the manifest into the mirror instead of cloning it, so a read-modify-write
/// batch costs one clone (the `load_manifest`) instead of two.
pub fn save_manifest_owned(manifest: UniversalCacheManifest) -> Result<()> {
    {
        let mut guard = MANIFEST_MEMORY
            .write()
            .map_err(|_| anyhow::anyhow!("manifest memory lock poisoned"))?;
        *guard = manifest;
    }
    MANIFEST_DIRTY.store(true, Ordering::Release);
    ensure_flush_task();
    Ok(())
}

/// O(1) upsert of a single entry into the in-memory manifest. Acquires
/// MANIFEST_LOCK because batch read-modify-write peers hold it across their
/// load -> mutate -> save sequence, and save_manifest fully REPLACES the memory
/// mirror; an unlocked upsert landing between a batch's load and save would be
/// silently erased.
///
/// An entry that names a file is stored only when that file exists, and the
/// file of the entry it replaces is deleted when it is another file, once the
/// manifest without it is on disk. Fails when the entry's file is not on disk.
pub fn upsert_cached_entry(entry: UniversalCacheEntry) -> Result<()> {
    let outcome = store_entries_locked(vec![entry])?;
    match outcome.missing.first() {
        Some(path) => Err(anyhow::anyhow!("cached file is not on disk: {path}")),
        None => Ok(()),
    }
}

/// The file an entry names, if it names one.
fn entry_file_path(entry: &UniversalCacheEntry) -> Option<&str> {
    entry.data.get("local_path").and_then(|p| p.as_str())
}

/// What `store_entries` did.
struct StoreOutcome {
    /// Entries put in the manifest.
    stored: usize,
    /// Files of refused entries, which were not on disk.
    missing: Vec<String>,
    /// Ids of the refused entries, in the order of `missing`.
    refused: Vec<String>,
}

/// A cached file waiting to be deleted: the folder of its type, then the file.
type PendingDelete = (PathBuf, PathBuf);

/// Files that replaced or expired entries named. They are deleted only after a
/// manifest write that no longer names them, so a crash in between never leaves
/// a manifest on disk that names a file already gone.
static PENDING_DELETES: StdMutex<Vec<PendingDelete>> = StdMutex::new(Vec::new());

/// Queues the files `entries` name for deletion from their type folders under
/// `cache_root`. Entries without a file are skipped.
fn queue_entry_files(pending: &StdMutex<Vec<PendingDelete>>, cache_root: &Path, entries: &[UniversalCacheEntry]) {
    let mut queue = pending.lock().unwrap_or_else(|e| e.into_inner());
    for entry in entries {
        if let Some(path) = entry_file_path(entry) {
            queue.push((cache_root.join(entry.cache_type.dir_name()), PathBuf::from(path)));
        }
    }
}

/// Takes the queued deletions. Called before the manifest is encoded, so the
/// encoded manifest already holds the change that queued each of them.
fn take_pending(pending: &StdMutex<Vec<PendingDelete>>) -> Vec<PendingDelete> {
    std::mem::take(&mut *pending.lock().unwrap_or_else(|e| e.into_inner()))
}

/// Puts deletions back in the queue after the manifest write that would have
/// allowed them failed.
fn requeue_pending(pending: &StdMutex<Vec<PendingDelete>>, files: Vec<PendingDelete>) {
    pending.lock().unwrap_or_else(|e| e.into_inner()).extend(files);
}

/// Files `delete_unnamed_files` checks and deletes under one hold of
/// MANIFEST_LOCK.
const DELETE_CHUNK: usize = 64;

/// Deletes the queued files that no entry of `memory` names. Each type folder is
/// resolved once. The files are taken in chunks of `DELETE_CHUNK`; the check and
/// the deletes of a chunk run under MANIFEST_LOCK, with the names read again
/// under each hold, so a store that names one of these files again is either
/// seen here or finds the file gone and is refused.
fn delete_unnamed_files(files: &[PendingDelete], memory: &StdRwLock<UniversalCacheManifest>) {
    if files.is_empty() {
        return;
    }
    let mut folders: HashMap<&Path, Option<PathBuf>> = HashMap::new();
    for (type_dir, _) in files {
        folders
            .entry(type_dir.as_path())
            .or_insert_with(|| resolve_path(type_dir));
    }
    for chunk in files.chunks(DELETE_CHUNK) {
        let _lock = MANIFEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let candidates: HashSet<&Path> = chunk.iter().map(|(_, file)| file.as_path()).collect();
        let named: HashSet<PathBuf> = match memory.read() {
            Ok(guard) => guard
                .entries
                .values()
                .filter_map(entry_file_path)
                .map(Path::new)
                .filter(|path| candidates.contains(path))
                .map(Path::to_path_buf)
                .collect(),
            Err(_) => return,
        };
        for (type_dir, file) in chunk {
            if named.contains(file) {
                continue;
            }
            if let Some(Some(dir)) = folders.get(type_dir.as_path()) {
                remove_cached_file(dir, file);
            }
        }
    }
}

/// Puts `entries` in `memory`, in order. An entry whose file is not on disk is
/// refused. When an entry replaces one with another file, the old file is
/// queued in `pending` for deletion from its type folder under `cache_root`,
/// after the manifest change and outside the manifest's write lock. The caller
/// holds MANIFEST_LOCK.
fn store_entries(
    memory: &StdRwLock<UniversalCacheManifest>,
    cache_root: &Path,
    pending: &StdMutex<Vec<PendingDelete>>,
    entries: Vec<UniversalCacheEntry>,
) -> Result<StoreOutcome> {
    let mut missing = Vec::new();
    let mut refused = Vec::new();
    let mut present = Vec::with_capacity(entries.len());
    for entry in entries {
        let gone = entry_file_path(&entry)
            .filter(|path| !Path::new(path).exists())
            .map(str::to_string);
        match gone {
            Some(path) => {
                refused.push(entry.id.clone());
                missing.push(path);
            }
            None => present.push(entry),
        }
    }
    let stored = present.len();
    let mut replaced = Vec::new();
    if stored > 0 {
        let mut guard = memory
            .write()
            .map_err(|_| anyhow::anyhow!("manifest memory lock poisoned"))?;
        for entry in present {
            let new_path = entry_file_path(&entry).map(str::to_owned);
            if let Some(old) = guard.entries.insert(entry.id.clone(), entry) {
                if entry_file_path(&old) != new_path.as_deref() {
                    replaced.push(old);
                }
            }
        }
    }
    queue_entry_files(pending, cache_root, &replaced);
    Ok(StoreOutcome { stored, missing, refused })
}

/// `store_entries` on the in-memory manifest, under MANIFEST_LOCK, then a
/// scheduled flush. An entry whose file was not on disk is refused and named in
/// the outcome; the other entries are stored.
fn store_entries_locked(entries: Vec<UniversalCacheEntry>) -> Result<StoreOutcome> {
    let cache_root = get_universal_cache_dir()?;
    let _lock = MANIFEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let outcome = store_entries(&MANIFEST_MEMORY, &cache_root, &PENDING_DELETES, entries)?;
    if outcome.stored > 0 {
        MANIFEST_DIRTY.store(true, Ordering::Release);
        ensure_flush_task();
    }
    Ok(outcome)
}

/// Writes the in-memory manifest to disk. The files that replaced and expired
/// entries named are taken before the manifest is encoded and deleted only
/// once it is written; a failed write puts them back in the queue.
fn flush_manifest_to_disk() -> Result<()> {
    let manifest_path = get_universal_cache_dir()?.join("manifest.json");
    flush_manifest_with(&MANIFEST_MEMORY, &PENDING_DELETES, &manifest_path, || {})
}

/// `flush_manifest_to_disk` of `memory` and `pending` to the manifest at
/// `manifest_path`, under `FLUSH_LOCK` for its whole length. `after_encode` runs
/// once the manifest is encoded and before it is written.
fn flush_manifest_with(
    memory: &StdRwLock<UniversalCacheManifest>,
    pending: &StdMutex<Vec<PendingDelete>>,
    manifest_path: &Path,
    after_encode: impl FnOnce(),
) -> Result<()> {
    let _flush = FLUSH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let doomed = take_pending(pending);
    let written = (|| -> Result<()> {
        let json = {
            let guard = memory
                .read()
                .map_err(|_| anyhow::anyhow!("manifest memory lock poisoned"))?;
            encode_manifest(&*guard)?
        };
        after_encode();
        write_manifest_bytes_to(manifest_path, &json)
    })();
    match written {
        Ok(()) => {
            delete_unnamed_files(&doomed, memory);
            Ok(())
        }
        Err(e) => {
            requeue_pending(pending, doomed);
            Err(e)
        }
    }
}

/// Synchronous flush of the in-memory manifest to disk, if dirty. For shutdown
/// paths that skip or precede the debounced flush task.
pub fn flush_manifest_now() -> Result<()> {
    if !MANIFEST_DIRTY.swap(false, Ordering::AcqRel) {
        return Ok(());
    }
    if let Err(e) = flush_manifest_to_disk() {
        MANIFEST_DIRTY.store(true, Ordering::Release);
        return Err(e);
    }
    Ok(())
}

/// Fetch universal cache data from hosted repository
pub async fn fetch_universal_cache_data(
    cache_type: &CacheType,
    id: &str,
) -> Result<Option<UniversalCacheEntry>> {
    let type_str = cache_type.dir_name();

    // Try to fetch from universal cache repository
    let url = format!("{}/{}/{}.json", UNIVERSAL_CACHE_URL, type_str, id);

    debug!("[UniversalCache] Attempting to fetch from: {}", url);

    let client = &*META_CLIENT_5S;

    match fetch_cache_file(client, &format!("{}/{}.json", type_str, id)).await {
        Ok(response) if response.status().is_success() => {
            let entry: UniversalCacheEntry = response.json().await?;
            debug!(
                "[UniversalCache] Successfully fetched {} from universal cache",
                id
            );
            Ok(Some(entry))
        }
        Ok(response) => {
            debug!(
                "[UniversalCache] Universal cache returned status: {}",
                response.status()
            );
            Ok(None)
        }
        Err(e) => {
            debug!(
                "[UniversalCache] Failed to fetch from universal cache: {}",
                e
            );
            Ok(None)
        }
    }
}

/// Merges the badgebase entries of `remote` into `memory` in place, under
/// MANIFEST_LOCK, and stamps `last_sync`. A remote entry replaces a local one
/// only when it is newer and the local one is not relay enrichment. A remote
/// entry that names a file not on disk is skipped. Returns whether a badgebase
/// entry still has no position.
fn merge_remote_into(memory: &StdRwLock<UniversalCacheManifest>, remote: UniversalCacheManifest) -> Result<bool> {
    merge_remote_into_with(memory, remote, || {})
}

/// `merge_remote_into` that calls `after_read` once the entries to replace are
/// chosen and before they are put in `memory`.
fn merge_remote_into_with(
    memory: &StdRwLock<UniversalCacheManifest>,
    remote: UniversalCacheManifest,
    after_read: impl FnOnce(),
) -> Result<bool> {
    let candidates: Vec<(String, UniversalCacheEntry)> = remote
        .entries
        .into_iter()
        .filter(|(_, entry)| entry.metadata.source == "badgebase")
        .filter(|(_, entry)| entry_file_path(entry).map_or(true, |path| Path::new(path).exists()))
        .collect();
    let _lock = MANIFEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let replacing: Vec<(String, UniversalCacheEntry)> = {
        let guard = memory
            .read()
            .map_err(|_| anyhow::anyhow!("manifest memory lock poisoned"))?;
        candidates
            .into_iter()
            .filter(|(key, entry)| match guard.entries.get(key) {
                // Relay enrichment is campaign-grounded and richer than the
                // scrape, and the daily job always carries a newer timestamp,
                // so it must never win on timestamp alone.
                Some(local) if local.metadata.source == crate::commands::badge_metadata::ENRICHMENT_SOURCE => false,
                Some(local) => entry.metadata.timestamp > local.metadata.timestamp,
                None => true, // New entry not present locally, always insert
            })
            .collect()
    };
    after_read();
    let mut guard = memory
        .write()
        .map_err(|_| anyhow::anyhow!("manifest memory lock poisoned"))?;
    for (key, entry) in replacing {
        guard.entries.insert(key, entry);
    }
    guard.last_sync = Some(get_current_timestamp());
    let needs_positions = guard
        .entries
        .values()
        .filter(|entry| entry.metadata.source == "badgebase")
        .any(|entry| entry.position.is_none());
    Ok(needs_positions)
}

/// `merge_remote_into` on the in-memory manifest, then a scheduled flush.
fn merge_remote_manifest(remote: UniversalCacheManifest) -> Result<bool> {
    let needs_positions = merge_remote_into(&MANIFEST_MEMORY, remote)?;
    MANIFEST_DIRTY.store(true, Ordering::Release);
    ensure_flush_task();
    Ok(needs_positions)
}

/// Download and merge the universal manifest from GitHub
/// Returns true if download was performed, false if skipped (already in progress)
async fn download_universal_manifest() -> Result<bool> {
    // Check if download is already in progress using compare_exchange
    // This atomically checks if false and sets to true
    if DOWNLOAD_IN_PROGRESS
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        // Another download is already in progress, skip
        return Ok(false);
    }

    // Ensure we reset the flag when done (even on error)
    let _guard = scopeguard::guard((), |_| {
        DOWNLOAD_IN_PROGRESS.store(false, Ordering::SeqCst);
    });

    debug!("[UniversalCache] Downloading universal manifest from GitHub...");

    let url = format!("{}/manifest.json", UNIVERSAL_CACHE_URL);

    let client = &*META_CLIENT_30S;

    match fetch_cache_file(client, "manifest.json").await {
        Ok(response) if response.status().is_success() => {
            let remote_manifest: UniversalCacheManifest = response.json().await?;
            debug!(
                "[UniversalCache] Downloaded manifest with {} entries",
                remote_manifest.entries.len()
            );

            // Acquire async lock for all manifest operations
            let _async_lock = ASYNC_MANIFEST_LOCK.lock().await;

            // The merge changes the in-memory manifest in place on the blocking
            // pool, under MANIFEST_LOCK, so a store that lands meanwhile is kept.
            let needs_positions = tokio::task::spawn_blocking(move || merge_remote_manifest(remote_manifest))
                .await
                .context("universal manifest merge task panicked")??;
            debug!("[UniversalCache] Merged universal manifest into local cache");

            if needs_positions {
                debug!("[UniversalCache] Assigning positions to badge metadata...");
                // Note: assign_badge_metadata_positions will acquire its own lock
                drop(_async_lock);
                let _ = assign_badge_metadata_positions_internal().await;
            }

            Ok(true)
        }
        Ok(response) => {
            debug!(
                "[UniversalCache] GitHub returned status: {}",
                response.status()
            );
            Ok(true)
        }
        Err(e) => {
            debug!(
                "[UniversalCache] Failed to download universal manifest: {}",
                e
            );
            Ok(true) // Don't fail if GitHub is unavailable
        }
    }
}

/// Read a SINGLE entry (cloned) plus `last_sync` from the in-memory manifest,
/// without cloning the whole manifest. `load_manifest()` clones every entry
/// (thousands), so doing it per-lookup is heavy CPU on the calling thread — a
/// busy channel / prefetch-plan issues thousands of `get_cached_item` calls and
/// the cumulative full-manifest clones blocked the async runtime (a mid-session
/// `rt_stall`). This reads only what a lookup needs.
fn peek_entry(id: &str) -> Result<(Option<UniversalCacheEntry>, Option<u64>)> {
    let guard = MANIFEST_MEMORY
        .read()
        .map_err(|_| anyhow::anyhow!("manifest memory lock poisoned"))?;
    Ok((guard.entries.get(id).cloned(), guard.last_sync))
}

/// Public single-entry read. Callers that want ONE entry must use this rather
/// than `load_manifest()`, which clones every entry.
pub fn peek_cached_entry(id: &str) -> Result<Option<UniversalCacheEntry>> {
    Ok(peek_entry(id)?.0)
}

/// Timestamp of the last universal-cache sync, without cloning the manifest.
pub fn manifest_last_sync() -> Result<Option<u64>> {
    let guard = MANIFEST_MEMORY
        .read()
        .map_err(|_| anyhow::anyhow!("manifest memory lock poisoned"))?;
    Ok(guard.last_sync)
}

/// Get an item from cache (checks local first, downloads manifest if needed).
/// Reads only the requested entry from memory — no full-manifest clone.
pub async fn get_cached_item(
    cache_type: CacheType,
    id: &str,
) -> Result<Option<UniversalCacheEntry>> {
    let (entry, last_sync) = peek_entry(id)?;

    // Check if we have it locally
    if let Some(entry) = entry {
        if entry.cache_type == cache_type && !is_cache_expired(&entry.metadata) {
            return Ok(Some(entry));
        } else if is_cache_expired(&entry.metadata) {
            debug!("[UniversalCache] Local cache entry for {} is expired", id);
        }
    }

    // Not in local cache — force a download if the manifest hasn't been synced
    // in the last day. (The primary daily refresh runs at app start via
    // auto_sync_if_stale, which compares remote vs local timestamp; this is
    // the fallback for lookup-misses during a single very long session.)
    let should_download = last_sync.is_none() || {
        let current_time = get_current_timestamp();
        current_time - last_sync.unwrap_or(0) > 24 * 60 * 60 // More than 1 day old
    };

    if should_download {
        // download_universal_manifest will handle concurrency protection and updating last_sync
        let downloaded = download_universal_manifest().await?;

        if downloaded {
            let (entry, _) = peek_entry(id)?;
            if let Some(entry) = entry {
                if entry.cache_type == cache_type && !is_cache_expired(&entry.metadata) {
                    debug!("[UniversalCache] Found {} in downloaded manifest", id);
                    return Ok(Some(entry));
                }
            }
        }
    }

    Ok(None)
}

/// Save an item to local cache.
///
/// Kept on the blocking pool: the upsert itself is O(1), but MANIFEST_LOCK can
/// be held for tens of ms by blocking-pool batch read-modify-writes, and
/// blocking on that from a runtime worker would stall async tasks.
pub async fn save_cached_item(entry: UniversalCacheEntry) -> Result<()> {
    tokio::task::spawn_blocking(move || upsert_cached_entry(entry))
        .await
        .context("save_cached_item task panicked")?
}

/// Insert many file entries under ONE MANIFEST_LOCK acquisition. Inserts are
/// idempotent upserts, so re-running with already-present ids is harmless.
/// Returns the ids of the entries that were refused because their file is not
/// on disk; every other entry is stored.
pub async fn save_cached_items_batch(entries: Vec<UniversalCacheEntry>) -> Result<Vec<String>> {
    if entries.is_empty() {
        return Ok(Vec::new());
    }
    // Blocking pool for the same lock-hold reason as save_cached_item.
    let outcome = tokio::task::spawn_blocking(move || store_entries_locked(entries))
        .await
        .context("save_cached_items_batch task panicked")??;
    if !outcome.refused.is_empty() {
        debug!(
            "[UniversalCache] {} entries were refused, their file is not on disk (first: {})",
            outcome.refused.len(),
            outcome.missing.first().map_or("", String::as_str)
        );
    }
    Ok(outcome.refused)
}

/// Save a new item to cache with metadata
/// Preserves existing position if the entry already exists (avoids losing sort order)
pub async fn cache_item(
    cache_type: CacheType,
    id: String,
    data: serde_json::Value,
    source: String,
    expiry_days: u32,
) -> Result<()> {
    // Preserve existing position if the entry already has one. peek_entry reads
    // just this entry — not a full-manifest clone (see get_cached_item).
    let existing_position = peek_entry(&id)
        .ok()
        .and_then(|(e, _)| e.and_then(|e| e.position));

    let entry = UniversalCacheEntry {
        id: id.clone(),
        cache_type,
        data,
        metadata: CacheMetadata {
            timestamp: get_current_timestamp(),
            expiry_days,
            source,
            version: CACHE_VERSION,
        },
        position: existing_position,
    };

    save_cached_item(entry).await
}

/// Badgebase's "Date of addition" as a sortable (year, month, day). It comes
/// as "30 March 2026", or as "March 2018" for older badges, and anything else
/// is (0, 0, 0) so it sorts as oldest. Compared as raw text these orderings
/// scramble: "4 February 2026" sorts above "30 March 2026" because '4' > '3'.
/// A month-only date takes day 0, so it sorts after every dated day of that
/// month.
fn badge_date_key(date_str: &str) -> (i32, u32, u32) {
    const MONTHS: [&str; 12] = [
        "january", "february", "march", "april", "may", "june", "july", "august",
        "september", "october", "november", "december",
    ];
    let month_of = |s: &str| {
        let s = s.to_ascii_lowercase();
        MONTHS.iter().position(|m| *m == s).map(|i| i as u32 + 1)
    };
    let year_of = |s: &str| s.parse::<i32>().ok().filter(|y| (1900..=3000).contains(y));

    let parts: Vec<&str> = date_str.split_whitespace().collect();
    let parsed = match parts.as_slice() {
        [day, month, year] => day
            .parse::<u32>()
            .ok()
            .filter(|d| (1..=31).contains(d))
            .zip(month_of(month))
            .zip(year_of(year))
            .map(|((d, m), y)| (y, m, d)),
        [month, year] => month_of(month).zip(year_of(year)).map(|(m, y)| (y, m, 0)),
        _ => None,
    };
    parsed.unwrap_or((0, 0, 0))
}

/// Internal function to assign positions (called from download_universal_manifest with lock already held)
async fn assign_badge_metadata_positions_internal() -> Result<usize> {
    let _lock = ASYNC_MANIFEST_LOCK.lock().await;
    tokio::task::spawn_blocking(assign_badge_metadata_positions_impl)
        .await
        .context("badge position task panicked")?
}

/// Implementation of position assignment on the in-memory manifest, followed by
/// a scheduled flush. Runs on the blocking pool.
fn assign_badge_metadata_positions_impl() -> Result<usize> {
    let count = assign_positions_in(&MANIFEST_MEMORY)?;
    MANIFEST_DIRTY.store(true, Ordering::Release);
    ensure_flush_task();
    Ok(count)
}

/// Orders the badgebase badge entries of `memory` by date added (newest first),
/// then usage, and sets each entry's position in place under MANIFEST_LOCK. The
/// order is worked out from a read snapshot before the lock is taken, and set
/// only on the entries still in `memory`. Returns how many badgebase badges have
/// a position.
fn assign_positions_in(memory: &StdRwLock<UniversalCacheManifest>) -> Result<usize> {
    assign_positions_in_with(memory, || {})
}

/// `assign_positions_in` that calls `after_read` once the order is worked out and
/// before the positions are set in `memory`.
fn assign_positions_in_with(memory: &StdRwLock<UniversalCacheManifest>, after_read: impl FnOnce()) -> Result<usize> {
    // The ids of the badge entries from the badge metadata source, in position order.
    let ordered: Vec<String> = {
        let guard = memory
            .read()
            .map_err(|_| anyhow::anyhow!("manifest memory lock poisoned"))?;
        let mut metadata_entries: Vec<(&String, &UniversalCacheEntry)> = guard
            .entries
            .iter()
            .filter(|(_, entry)| {
                entry.cache_type == CacheType::Badge && entry.metadata.source == "badgebase"
            })
            .collect();
        sort_badge_entries(&mut metadata_entries);
        metadata_entries.into_iter().map(|(id, _)| id.clone()).collect()
    };
    after_read();

    let _lock = MANIFEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut guard = memory
        .write()
        .map_err(|_| anyhow::anyhow!("manifest memory lock poisoned"))?;
    for (position, id) in ordered.into_iter().enumerate() {
        if let Some(entry) = guard.entries.get_mut(&id) {
            entry.position = Some(position as u32);
        }
    }
    let count = guard
        .entries
        .values()
        .filter(|entry| {
            entry.cache_type == CacheType::Badge
                && entry.metadata.source == "badgebase"
                && entry.position.is_some()
        })
        .count();
    debug!(
        "[UniversalCache] Assigned positions to {} badge metadata entries",
        count
    );
    Ok(count)
}

/// Sorts badge entries by date added (newest first), then by usage (highest
/// first), then by id.
fn sort_badge_entries(metadata_entries: &mut [(&String, &UniversalCacheEntry)]) {
    metadata_entries.sort_by(|a, b| {
        let a_data = &a.1.data;
        let b_data = &b.1.data;

        let date_of = |data: &serde_json::Value| {
            badge_date_key(data.get("date_added").and_then(|v| v.as_str()).unwrap_or(""))
        };
        let a_date = date_of(a_data);
        let b_date = date_of(b_data);

        // Parse usage stats
        let parse_usage = |stats: &str| -> u32 {
            stats
                .split_whitespace()
                .next()
                .and_then(|s| s.replace(",", "").parse::<u32>().ok())
                .unwrap_or(0)
        };

        let a_usage = a_data
            .get("usage_stats")
            .and_then(|v| v.as_str())
            .map(parse_usage)
            .unwrap_or(0);
        let b_usage = b_data
            .get("usage_stats")
            .and_then(|v| v.as_str())
            .map(parse_usage)
            .unwrap_or(0);

        // Sort by date (newest first), then usage (highest first)
        b_date.cmp(&a_date).then(b_usage.cmp(&a_usage)).then(a.0.cmp(b.0))
    });
}

/// Assign positions to badge metadata entries based on date and usage (public API)
pub async fn assign_badge_metadata_positions() -> Result<usize> {
    let _lock = ASYNC_MANIFEST_LOCK.lock().await;
    tokio::task::spawn_blocking(assign_badge_metadata_positions_impl)
        .await
        .context("badge position task panicked")?
}

/// Export manifest to a specific path for GitHub upload
pub fn export_manifest_for_github(output_path: PathBuf) -> Result<()> {
    let manifest = load_manifest()?;
    let json = serde_json::to_string_pretty(&manifest)?;
    fs::write(&output_path, json)?;
    debug!("[UniversalCache] Exported manifest to {:?}", output_path);
    Ok(())
}

/// Sets `last_sync` of `memory` to `now` in place, under MANIFEST_LOCK.
fn stamp_last_sync_in(memory: &StdRwLock<UniversalCacheManifest>, now: u64) -> Result<()> {
    let _lock = MANIFEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut guard = memory
        .write()
        .map_err(|_| anyhow::anyhow!("manifest memory lock poisoned"))?;
    guard.last_sync = Some(now);
    Ok(())
}

/// `stamp_last_sync_in` on the in-memory manifest, then a scheduled flush.
fn stamp_last_sync(now: u64) -> Result<()> {
    stamp_last_sync_in(&MANIFEST_MEMORY, now)?;
    MANIFEST_DIRTY.store(true, Ordering::Release);
    ensure_flush_task();
    Ok(())
}

/// Sync with universal cache - download commonly used items
pub async fn sync_universal_cache(item_types: Vec<CacheType>) -> Result<usize> {
    debug!("[UniversalCache] Starting sync with universal cache");

    let mut synced_count = 0;

    // For each type, fetch the index file which lists available items
    for cache_type in item_types {
        let type_str = cache_type.dir_name();

        let index_url = format!("{}/{}/index.json", UNIVERSAL_CACHE_URL, type_str);

        debug!("[UniversalCache] Fetching index from: {}", index_url);

        let client = &*META_CLIENT_10S;

        match fetch_cache_file(client, &format!("{}/index.json", type_str)).await {
            Ok(response) if response.status().is_success() => {
                let index: Vec<String> = response.json().await?;
                debug!(
                    "[UniversalCache] Found {} items in {} index",
                    index.len(),
                    type_str
                );

                // Fetch each item (limit to prevent overwhelming)
                let limit = 100.min(index.len());
                for id in index.iter().take(limit) {
                    if let Ok(Some(entry)) = fetch_universal_cache_data(&cache_type, id).await {
                        save_cached_item(entry).await?;
                        synced_count += 1;
                    }
                }
            }
            Ok(response) => {
                debug!(
                    "[UniversalCache] Index fetch returned status: {}",
                    response.status()
                );
            }
            Err(e) => {
                debug!("[UniversalCache] Failed to fetch index: {}", e);
            }
        }
    }

    // Update last sync time in place, on the blocking pool
    tokio::task::spawn_blocking(|| stamp_last_sync(get_current_timestamp()))
        .await
        .context("last sync task panicked")??;

    debug!(
        "[UniversalCache] Sync complete. Synced {} items",
        synced_count
    );

    Ok(synced_count)
}

/// Removes the entries expired at `now` from `manifest` and returns them.
fn drop_expired(manifest: &mut UniversalCacheManifest, now: u64) -> Vec<UniversalCacheEntry> {
    let mut dropped = Vec::new();
    manifest.entries.retain(|_, entry| {
        let expired = is_expired_at(&entry.metadata, now);
        if expired {
            dropped.push(entry.clone());
        }
        !expired
    });
    dropped
}

/// Clear expired entries from cache. The entries leave the in-memory manifest
/// under MANIFEST_LOCK, so a download that stores an entry meanwhile is not
/// undone, and their files are deleted once the manifest without them is on
/// disk.
pub fn cleanup_expired_entries() -> Result<usize> {
    let cache_root = get_universal_cache_dir()?;
    let _lock = MANIFEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dropped = {
        let mut guard = MANIFEST_MEMORY
            .write()
            .map_err(|_| anyhow::anyhow!("manifest memory lock poisoned"))?;
        drop_expired(&mut guard, get_current_timestamp())
    };

    let removed_count = dropped.len();
    if removed_count > 0 {
        queue_entry_files(&PENDING_DELETES, &cache_root, &dropped);
        MANIFEST_DIRTY.store(true, Ordering::Release);
        ensure_flush_task();
        debug!(
            "[UniversalCache] Cleaned up {} expired entries",
            removed_count
        );
    }

    Ok(removed_count)
}

/// Get cache statistics
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UniversalCacheStats {
    pub total_entries: usize,
    pub entries_by_type: HashMap<String, usize>,
    pub last_sync: Option<u64>,
    pub cache_dir: String,
}

pub fn get_universal_cache_stats() -> Result<UniversalCacheStats> {
    let manifest = load_manifest()?;
    let cache_dir = get_universal_cache_dir()?;

    let mut entries_by_type: HashMap<String, usize> = HashMap::new();

    for entry in manifest.entries.values() {
        let type_str = entry.cache_type.dir_name();

        *entries_by_type.entry(type_str.to_string()).or_insert(0) += 1;
    }

    Ok(UniversalCacheStats {
        total_entries: manifest.entries.len(),
        entries_by_type,
        last_sync: manifest.last_sync,
        cache_dir: cache_dir.to_string_lossy().to_string(),
    })
}

/// Clear all universal cache data
pub fn clear_universal_cache() -> Result<()> {
    let cache_dir = get_universal_cache_dir()?;
    clear_cache_in(&cache_dir, &MANIFEST_MEMORY, &PENDING_DELETES)
}

/// `clear_universal_cache` of `memory`, `pending` and the folder `cache_dir`.
fn clear_cache_in(
    cache_dir: &Path,
    memory: &StdRwLock<UniversalCacheManifest>,
    pending: &StdMutex<Vec<PendingDelete>>,
) -> Result<()> {
    // A flush in progress finishes first, so it cannot write its manifest back
    // into the folder once it is cleared.
    let _flush = FLUSH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // Held for the whole clear: a store runs entirely before it (its entry is
    // cleared with the rest) or entirely after it.
    let _lock = MANIFEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    // CRITICAL: drop the in-memory manifest mirror too. Without this, stale
    // `file:<id>` entries with paths to now-deleted files (or files with the
    // wrong extension after a code change) keep getting served back to the
    // frontend via get_cached_files_list. That's why a "clear cache" can
    // appear to do nothing — the disk was wiped but the in-memory paths
    // outlived it. Also clear the dirty flag so the next debounced flush
    // doesn't write the stale snapshot back to disk.
    //
    // Done BEFORE the delete: on Windows `remove_dir_all` stops at the first
    // file another process holds open, after deleting the ones before it, and
    // returning there used to keep every entry for the files already gone.
    if let Ok(mut guard) = memory.write() {
        *guard = UniversalCacheManifest {
            version: CACHE_VERSION,
            last_sync: None,
            entries: HashMap::new(),
        };
        FOREIGN_ENTRIES.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
    take_pending(pending);
    MANIFEST_DIRTY.store(false, Ordering::Release);
    clear_verified_types();

    let deleted = if cache_dir.exists() {
        fs::remove_dir_all(cache_dir)
            .context("Failed to remove universal cache directory")
            .and_then(|()| fs::create_dir_all(cache_dir).context("Failed to recreate universal cache directory"))
    } else {
        Ok(())
    };

    // A type folder listed while the delete ran, or before a delete that failed
    // part way, is not a record of what is on disk now.
    clear_verified_types();
    deleted
}

/// Forgets which cache types were checked against the disk this session.
fn clear_verified_types() {
    VERIFIED_TYPES.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

/// One-time emote cache migration token
/// This token triggers cache clearing exactly once when upgrading.
/// After migration completes, the token is written to disk and never triggers again.
/// UPDATE: Changed token to force re-migration with manifest clearing
const EMOTE_CACHE_MIGRATION_TOKEN: &str = "PERDPI_TIER_2026_V3";

/// Migrate emote cache if migration token not yet applied
/// This is a ONE-TIME migration - it clears the cache once, then never again.
pub fn migrate_emote_cache_on_version_change(_current_version: &str) -> Result<bool> {
    let cache_dir = get_universal_cache_dir()?;
    let version_file = cache_dir.join(".emote_cache_version");

    // Read stored token
    let stored_token = if version_file.exists() {
        fs::read_to_string(&version_file)
            .unwrap_or_default()
            .trim()
            .to_string()
    } else {
        String::new()
    };

    // If migration token not applied yet, clear cache and write token
    if stored_token != EMOTE_CACHE_MIGRATION_TOKEN {
        debug!(
            "[UniversalCache] One-time emote cache migration: '{}' -> '{}'",
            if stored_token.is_empty() {
                "<none>"
            } else {
                &stored_token
            },
            EMOTE_CACHE_MIGRATION_TOKEN
        );

        // Clear only the emotes directory
        let emotes_dir = cache_dir.join("emotes");
        if emotes_dir.exists() {
            fs::remove_dir_all(&emotes_dir)?;
            fs::create_dir_all(&emotes_dir)?;
            debug!("[UniversalCache] Emote cache directory cleared for one-time migration");
        }

        // Also clear emote entries from manifest to prevent stale references
        let manifest_path = cache_dir.join("manifest.json");
        if manifest_path.exists() {
            if let Ok(mut manifest) = load_manifest() {
                let initial_count = manifest.entries.len();

                // Remove all emote-type entries and file:emote entries
                manifest.entries.retain(|key, entry| {
                    // Keep non-emote entries
                    if entry.cache_type != CacheType::Emote {
                        // Also check for file: prefixed emote entries
                        if key.starts_with("file:") {
                            // Check if it's an emote file by path or type
                            if let Some(path) =
                                entry.data.get("local_path").and_then(|p| p.as_str())
                            {
                                if path.contains("/emotes/") || path.contains("\\emotes\\") {
                                    return false; // Remove emote files
                                }
                            }
                        }
                        return true;
                    }
                    false // Remove emote entries
                });

                let removed_count = initial_count - manifest.entries.len();
                if removed_count > 0 {
                    // Write through to disk now: this runs before the async
                    // runtime, so the debounced flush task can't run yet and a
                    // direct write guarantees the pruned manifest persists even
                    // if the app exits before the first post-runtime flush.
                    let _ = save_manifest_to_disk(&manifest);
                    // ...then hand the same manifest to the in-memory mirror.
                    let _ = save_manifest_owned(manifest);
                    debug!(
                        "[UniversalCache] Cleared {} emote entries from manifest",
                        removed_count
                    );
                }
            }
        }

        // Write migration token (prevents re-triggering on future updates)
        fs::write(&version_file, EMOTE_CACHE_MIGRATION_TOKEN)?;

        return Ok(true);
    }

    Ok(false)
}

/// One-time, FFZ-only emote cache purge.
///
/// FrankerFaceZ emotes are keyed on disk by bare emote id, and their CDN URL
/// carries no file extension, so a file lands at `{id}.bin` regardless of
/// whether it was the static PNG (`/emote/{id}/1`) or the animated WebP
/// (`/emote/{id}/animated/1`). An emote first cached as its static frame would
/// therefore keep being served even after the fetcher started preferring the
/// animated URL. This deletes ONLY FrankerFaceZ emote files (matched by the
/// `frankerfacez.com/emote/` source URL recorded in the manifest) so they
/// re-download as the animated variant on next display; every other provider's
/// cache — and FFZ room badges, which live under `frankerfacez.com/room-badge/`
/// — is left untouched. Gated by its own token file so it runs exactly once.
const FFZ_ANIMATED_MIGRATION_TOKEN: &str = "FFZ_ANIMATED_2026_V1";

pub fn migrate_ffz_animated_cache() -> Result<bool> {
    let cache_dir = get_universal_cache_dir()?;
    let token_file = cache_dir.join(".ffz_animated_migration");

    let stored_token = if token_file.exists() {
        fs::read_to_string(&token_file)
            .unwrap_or_default()
            .trim()
            .to_string()
    } else {
        String::new()
    };

    if stored_token == FFZ_ANIMATED_MIGRATION_TOKEN {
        return Ok(false);
    }

    debug!(
        "[UniversalCache] One-time FFZ emote cache purge: '{}' -> '{}'",
        if stored_token.is_empty() {
            "<none>"
        } else {
            &stored_token
        },
        FFZ_ANIMATED_MIGRATION_TOKEN
    );

    // Match on the emote-path URL specifically so FFZ room badges (same CDN host,
    // `/room-badge/` path) survive — only `/emote/` files are stale.
    let is_ffz_emote = |entry: &UniversalCacheEntry| {
        entry
            .data
            .get("url")
            .and_then(|u| u.as_str())
            .map(|u| u.contains("frankerfacez.com/emote/"))
            .unwrap_or(false)
    };

    let manifest_path = cache_dir.join("manifest.json");
    if manifest_path.exists() {
        if let Ok(mut manifest) = load_manifest() {
            // Delete the on-disk files for matching FFZ entries first.
            for entry in manifest.entries.values() {
                if is_ffz_emote(entry) {
                    if let Some(path) = entry.data.get("local_path").and_then(|p| p.as_str()) {
                        let _ = fs::remove_file(path);
                    }
                }
            }

            // Then drop those manifest entries so stale paths are never served.
            let initial_count = manifest.entries.len();
            manifest.entries.retain(|_key, entry| !is_ffz_emote(entry));
            let removed_count = initial_count - manifest.entries.len();

            if removed_count > 0 {
                // Mirror the existing migration: update the in-memory manifest and
                // write through to disk now. This runs before the async runtime, so
                // the debounced flush task can't run yet; a direct write guarantees
                // the pruned manifest survives even if the app exits early.
                let _ = save_manifest_to_disk(&manifest);
                let _ = save_manifest_owned(manifest);
                debug!(
                    "[UniversalCache] Purged {} FFZ emote entries from cache",
                    removed_count
                );
            }
        }
    }

    // Write token (prevents re-triggering on future launches).
    fs::write(&token_file, FFZ_ANIMATED_MIGRATION_TOKEN)?;

    Ok(true)
}

/// One-time purge of cached 7TV paint images that hold a still frame.
///
/// A paint image is cached under its layer id alone, and every surface draws
/// whatever file sits there. Stills got into that slot two ways: callers that
/// cached a layer's first scale-1 image, which 7TV lists as the `_static`
/// variant, and layers that 7TV animated after they were cached. This drops
/// every 7TV paint entry fetched from a `_static` URL or holding a still WebP, so
/// each re-downloads through the renderer's own pick. A paint that really is
/// still comes back as the same still. Gated by its own token file so it runs
/// exactly once.
const PAINT_ANIMATED_MIGRATION_TOKEN: &str = "PAINT_ANIMATED_2026_V1";

pub fn migrate_paint_animated_cache() -> Result<bool> {
    let cache_dir = get_universal_cache_dir()?;
    let token_file = cache_dir.join(".paint_animated_migration");

    let stored_token = fs::read_to_string(&token_file).unwrap_or_default();
    if stored_token.trim() == PAINT_ANIMATED_MIGRATION_TOKEN {
        return Ok(false);
    }

    let manifest_path = cache_dir.join("manifest.json");
    if manifest_path.exists() {
        if let Ok(mut manifest) = load_manifest() {
            let still: Vec<String> = manifest
                .entries
                .iter()
                .filter(|(_, entry)| is_still_paint_entry(entry))
                .map(|(key, _)| key.clone())
                .collect();

            for key in &still {
                if let Some(entry) = manifest.entries.remove(key) {
                    if let Some(path) = entry.data.get("local_path").and_then(|p| p.as_str()) {
                        let _ = fs::remove_file(path);
                    }
                }
            }

            if !still.is_empty() {
                // Written through like the FFZ purge: this runs before the async
                // runtime, so the debounced flush cannot persist it yet.
                let _ = save_manifest_to_disk(&manifest);
                let _ = save_manifest_owned(manifest);
                debug!(
                    "[UniversalCache] Purged {} still 7TV paint images for re-fetch",
                    still.len()
                );
            }
        }
    }

    fs::write(&token_file, PAINT_ANIMATED_MIGRATION_TOKEN)?;

    Ok(true)
}

/// A cached 7TV paint image that draws a still frame: fetched from a `_static`
/// URL, or a WebP whose header says it does not animate.
fn is_still_paint_entry(entry: &UniversalCacheEntry) -> bool {
    if entry.cache_type != CacheType::Cosmetic {
        return false;
    }
    let Some(url) = entry.data.get("url").and_then(|u| u.as_str()) else {
        return false;
    };
    if !url.contains("cdn.7tv.app/paint/") {
        return false;
    }
    if url.contains("_static.") {
        return true;
    }
    entry
        .data
        .get("local_path")
        .and_then(|p| p.as_str())
        .and_then(read_image_head)
        .and_then(|head| webp_is_animated(&head))
        == Some(false)
}

fn read_image_head(path: &str) -> Option<[u8; 21]> {
    use std::io::Read;
    let mut head = [0u8; 21];
    fs::File::open(path).ok()?.read_exact(&mut head).ok()?;
    Some(head)
}

/// Whether a WebP animates, from its first 21 bytes; `None` when they are not
/// WebP. A still WebP is a plain `VP8 ` or `VP8L` file, or `VP8X` with the
/// animation flag (0x02 in the flags byte) clear.
fn webp_is_animated(head: &[u8]) -> Option<bool> {
    if head.len() < 21 || &head[0..4] != b"RIFF" || &head[8..12] != b"WEBP" {
        return None;
    }
    Some(&head[12..16] == b"VP8X" && head[20] & 0x02 != 0)
}

/// One-time migration to provider-namespaced cache keys + content-typed
/// extensions. Purges ONLY the non-7TV emote files (Twitch/BTTV/FFZ): they were
/// keyed by bare id (so a Twitch and an FFZ emote sharing an integer id could
/// collide) and saved with URL-derived extensions (`.0`/`.bin`). They re-download
/// under `{provider}-{id}` keys with correct extensions on next display/prefetch.
/// 7TV files (already `{id}@{tier}.avif`, correctly keyed/typed, and costly to
/// re-enumerate while the 7TV API is flaky) are KEPT, identified by the source
/// URL recorded in the manifest. Gated by its own token so it runs exactly once.
const EMOTE_NAMESPACE_MIGRATION_TOKEN: &str = "NAMESPACE_CONTENT_EXT_2026_V1";

pub fn migrate_emote_namespace_cache() -> Result<bool> {
    let cache_dir = get_universal_cache_dir()?;
    let token_file = cache_dir.join(".emote_namespace_migration");

    let stored_token = if token_file.exists() {
        fs::read_to_string(&token_file)
            .unwrap_or_default()
            .trim()
            .to_string()
    } else {
        String::new()
    };

    if stored_token == EMOTE_NAMESPACE_MIGRATION_TOKEN {
        return Ok(false);
    }

    if let Ok(mut manifest) = load_manifest() {
        let initial_count = manifest.entries.len();
        let mut files_deleted = 0usize;
        manifest.entries.retain(|key, entry| {
            // Only touch cached emote FILES.
            if entry.cache_type != CacheType::Emote || !key.starts_with("file:") {
                return true;
            }
            // Keep 7TV (already correctly keyed/typed).
            let url = entry.data.get("url").and_then(|u| u.as_str()).unwrap_or("");
            if url.contains("7tv") {
                return true;
            }
            // Purge the rest: delete the on-disk file and drop the manifest entry.
            if let Some(path) = entry.data.get("local_path").and_then(|p| p.as_str()) {
                if fs::remove_file(path).is_ok() {
                    files_deleted += 1;
                }
            }
            false
        });

        let removed = initial_count - manifest.entries.len();
        if removed > 0 {
            let _ = save_manifest_to_disk(&manifest);
            let _ = save_manifest_owned(manifest);
            debug!(
                "[UniversalCache] Namespace migration: dropped {} non-7TV emote entries, deleted {} files",
                removed, files_deleted
            );
        }
    }

    fs::write(&token_file, EMOTE_NAMESPACE_MIGRATION_TOKEN)?;
    Ok(true)
}

/// The first 1024 bytes of `bytes` without a UTF-8 BOM or leading whitespace,
/// in lower case.
fn markup_head(bytes: &[u8]) -> Vec<u8> {
    let head = &bytes[..bytes.len().min(1024)];
    let head = head.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(head);
    let start = head.iter().position(|b| !b.is_ascii_whitespace()).unwrap_or(head.len());
    head[start..].to_ascii_lowercase()
}

/// Whether the first 1024 bytes read as an HTML page: after an optional UTF-8
/// BOM and whitespace they hold, in any letter case, an `<html` element, an html
/// doctype, a `<head` or a `<body`.
pub(crate) fn looks_like_html(bytes: &[u8]) -> bool {
    let head = markup_head(bytes);
    [&b"<html"[..], b"<!doctype html", b"<head", b"<body"]
        .iter()
        .any(|needle| head.windows(needle.len()).any(|w| w == *needle))
}

/// Refuses bytes that read as an HTML page, which a captive portal or a block
/// page answers in place of an image.
fn refuse_html(bytes: &[u8]) -> Result<()> {
    // A real image is never refused, whatever its comment or metadata holds.
    if matches!(detect_image_ext(bytes, None), "png" | "gif" | "jpg" | "webp" | "avif") {
        return Ok(());
    }
    if looks_like_html(bytes) {
        return Err(anyhow::anyhow!("answer was an HTML page, not an image"));
    }
    Ok(())
}

/// Whether the first 1024 bytes read as SVG markup: after an optional UTF-8 BOM
/// and whitespace, an `<svg` element, an svg doctype, or an XML declaration or
/// a comment with an `<svg` element after it. HTML is never SVG.
pub(crate) fn looks_like_svg(bytes: &[u8]) -> bool {
    if looks_like_html(bytes) {
        return false;
    }
    let head = markup_head(bytes);
    if head.starts_with(b"<svg") || head.starts_with(b"<!doctype svg") {
        return true;
    }
    (head.starts_with(b"<?xml") || head.starts_with(b"<!--")) && head.windows(4).any(|w| w == b"<svg")
}

/// Pick the correct file extension from the downloaded bytes (magic numbers,
/// most reliable) with the response Content-Type as a fallback. Emote/badge CDNs
/// frequently serve images from extension-less URLs (BTTV/FFZ) or misleading ones
/// (Twitch `.../3.0`), so deriving the extension from the URL mislabels the file
/// and makes the asset server hand back the wrong Content-Type. Typing it from
/// the real content keeps the cache self-describing and rendering robust.
fn detect_image_ext(bytes: &[u8], content_type: Option<&str>) -> &'static str {
    if bytes.len() >= 12 {
        if bytes.starts_with(b"\x89PNG") {
            return "png";
        }
        if bytes.starts_with(b"GIF8") {
            return "gif";
        }
        if &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
            return "webp";
        }
        if &bytes[4..8] == b"ftyp" && (&bytes[8..12] == b"avif" || &bytes[8..12] == b"avis") {
            return "avif";
        }
    }
    if bytes.starts_with(b"\xFF\xD8\xFF") {
        return "jpg";
    }
    // A page is never an image, whatever the Content-Type claims.
    if looks_like_html(bytes) {
        return "bin";
    }
    if looks_like_svg(bytes) {
        return "svg";
    }
    if let Some(ct) = content_type {
        let ct = ct.to_ascii_lowercase();
        if ct.contains("svg") {
            return "svg";
        }
        if ct.contains("avif") {
            return "avif";
        }
        if ct.contains("webp") {
            return "webp";
        }
        if ct.contains("gif") {
            return "gif";
        }
        if ct.contains("png") {
            return "png";
        }
        if ct.contains("jpeg") || ct.contains("jpg") {
            return "jpg";
        }
    }
    "bin"
}

/// Cache a file from a URL. The on-disk extension is derived from the ACTUAL
/// downloaded content (magic bytes, Content-Type fallback), not the URL, so the
/// file is correctly typed regardless of what the provider's URL looks like.
pub async fn cache_file(
    cache_type: CacheType,
    id: String,
    url: String,
    expiry_days: u32,
) -> Result<String> {
    debug!(
        "[UniversalCache] Downloading and caching file: {} (type: {:?})",
        id, cache_type
    );

    // Download via the shared pooled client (warm HTTP/2 connection, bounded by
    // the client timeouts). Callers dedup before reaching here, so we always
    // fetch rather than guessing a filename from the URL up front.
    let response = DOWNLOAD_CLIENT.get(&url).send().await?;
    if !response.status().is_success() {
        return Err(anyhow::anyhow!(
            "Failed to download file: {}",
            response.status()
        ));
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let bytes = response.bytes().await?;

    let extension = detect_image_ext(&bytes, content_type.as_deref());
    let file_name = content_addressed_file_name(&id, &url, extension);
    // The entry replaces any earlier one for this id, and the earlier file is
    // deleted with that change (see `store_entries`).
    let (path_str, entry) = write_file_entry(cache_type, id, url, file_name, bytes, expiry_days).await?;
    save_cached_item(entry).await?;

    debug!("[UniversalCache] Successfully cached file: {}", path_str);
    Ok(path_str)
}

/// The on-disk folder for a type's files, created on first use.
fn file_type_dir(cache_type: &CacheType) -> Result<PathBuf> {
    let type_dir = get_universal_cache_dir()?.join(cache_type.dir_name());
    if !type_dir.exists() {
        fs::create_dir_all(&type_dir).context("Failed to create cache type directory")?;
    }
    Ok(type_dir)
}

/// Waits between the retries of a rename that is refused.
const RENAME_RETRY_MS: [u64; 3] = [20, 40, 80];

/// File that every running instance holds a shared lock on while it may write
/// cache files. The temp sweep takes it exclusively, so it runs only when no
/// other instance can be mid-write. This is the name used inside the cache
/// root, for a root that has no parent folder.
const WRITERS_LOCK_FILE: &str = ".writers.lock";

/// Name of the writers lock file kept in a `locks` folder beside the cache
/// root, so clearing the cache never has to delete a file that running
/// instances hold locked.
const WRITERS_LOCK_BESIDE_ROOT: &str = "streamnook-cache-writers.lock";

/// Folder, beside the cache root, that holds the writers lock file. Clearing the
/// cache removes the cache root and the plain files beside it, never this folder.
const WRITERS_LOCK_DIR: &str = "locks";

/// Where the writers lock file lives for `cache_root`: in the `locks` folder of
/// the folder that holds the cache root, or inside the root when it has no
/// parent.
fn writers_lock_path(cache_root: &Path) -> PathBuf {
    match cache_root.parent() {
        Some(parent) => parent.join(WRITERS_LOCK_DIR).join(WRITERS_LOCK_BESIDE_ROOT),
        None => cache_root.join(WRITERS_LOCK_FILE),
    }
}

/// The writers lock file while this process holds it shared, kept open for the
/// life of the process. `None` until a join succeeds, so a failed join is tried
/// again by the next write.
static WRITERS: StdMutex<Option<fs::File>> = StdMutex::new(None);

/// Whether this process has written a cache file. Once it has, an unnamed file
/// in a type folder may be one whose manifest entry is still being stored.
static WROTE_THIS_RUN: AtomicBool = AtomicBool::new(false);

/// Whether a failure to join the writers has been logged at warn level.
static JOIN_WARNED: AtomicBool = AtomicBool::new(false);

/// Logs a failure to join the writers: at warn level the first time, at debug
/// level after that, since every write retries the join.
fn log_join_failure(message: &str) {
    if JOIN_WARNED.swap(true, Ordering::Relaxed) {
        debug!("[UniversalCache] {message}");
    } else {
        log::warn!("[UniversalCache] {message}");
    }
}

/// The type-folder and file name of every file a manifest entry names, or `None`
/// when no sweep of unnamed files may run: this process has written a file
/// already, or the manifest holds no entry at all (it may have failed to read).
fn manifest_named_files() -> Option<HashSet<(OsString, OsString)>> {
    if WROTE_THIS_RUN.load(Ordering::Acquire) {
        return None;
    }
    let guard = MANIFEST_MEMORY.read().ok()?;
    let foreign = FOREIGN_ENTRIES.lock().unwrap_or_else(|e| e.into_inner());
    if guard.entries.is_empty() && foreign.is_empty() {
        return None;
    }
    let mut named = HashSet::new();
    let paths = guard
        .entries
        .values()
        .filter_map(entry_file_path)
        .chain(foreign.iter().filter_map(|(_, value)| value.get("data")?.get("local_path")?.as_str()));
    for path in paths {
        let path = Path::new(path);
        let folder = path.parent().and_then(Path::file_name);
        if let (Some(folder), Some(name)) = (folder, path.file_name()) {
            named.insert((folder.to_os_string(), name.to_os_string()));
        }
    }
    Some(named)
}

/// Writes `bytes` to `dest` through a temp file of its own, so concurrent
/// writers of one name never mix bytes and a reader never sees a half-written
/// file. `dest` sits in a type folder of the cache root. The first write of the
/// process waits for the temp sweep, so call this from the blocking pool. Fails,
/// before any file is created, when the process does not hold the writers lock.
fn write_atomic<B: AsRef<[u8]>>(dest: &Path, bytes: B) -> std::io::Result<()> {
    let written = write_atomic_held(&WRITERS, dest, bytes, manifest_named_files);
    if written.is_ok() {
        WROTE_THIS_RUN.store(true, Ordering::Release);
    }
    written
}

/// `write_atomic` with the writers lock `holder` and the sweep's `named_files`
/// given.
fn write_atomic_held<B, N>(
    holder: &StdMutex<Option<fs::File>>,
    dest: &Path,
    bytes: B,
    named_files: N,
) -> std::io::Result<()>
where
    B: AsRef<[u8]>,
    N: FnOnce() -> Option<HashSet<(OsString, OsString)>>,
{
    if !sweep_before_write(holder, dest, named_files) {
        return Err(std::io::Error::other("cache writers lock not held"));
    }
    write_atomic_with(dest, bytes, |from, to| fs::rename(from, to))
}

/// Joins the writers for the cache root that holds `dest`'s type folder, unless
/// `holder` already holds the lock. Returns whether the lock is held; a `dest`
/// that sits in no cache root holds none.
fn sweep_before_write<N>(holder: &StdMutex<Option<fs::File>>, dest: &Path, named_files: N) -> bool
where
    N: FnOnce() -> Option<HashSet<(OsString, OsString)>>,
{
    let Some(cache_root) = dest.parent().and_then(Path::parent) else {
        return false;
    };
    join_writers_once(holder, cache_root, named_files)
}

/// Joins the writers into `holder` when it is empty. The mutex is held while
/// joining, so a write that starts first waits for the sweep. A join that fails
/// leaves `holder` empty and is tried again by the next call. Returns whether
/// `holder` holds the lock.
fn join_writers_once<N>(holder: &StdMutex<Option<fs::File>>, cache_root: &Path, named_files: N) -> bool
where
    N: FnOnce() -> Option<HashSet<(OsString, OsString)>>,
{
    let mut held = holder.lock().unwrap_or_else(|e| e.into_inner());
    if held.is_some() {
        return true;
    }
    match join_writers_with(cache_root, named_files) {
        Some((file, _)) => {
            *held = Some(file);
            true
        }
        None => false,
    }
}

/// Runs the temp sweep for the process on the calling thread, which should be a
/// blocking-pool thread, unless the process has already joined the writers. A
/// first write finds it done, or waits for it.
pub fn sweep_temp_files_once(cache_root: &Path) {
    let _ = join_writers_once(&WRITERS, cache_root, manifest_named_files);
}

/// Joins the instances that write to the cache: locks the writers file shared
/// and returns it, with the number of temp files swept. The sweep runs only when
/// the file could first be locked exclusively, which means no other instance
/// holds it; otherwise it is skipped (`None`). Returns `None` when the writers
/// file cannot be opened or locked.
#[cfg(test)]
fn join_writers(cache_root: &Path) -> Option<(fs::File, Option<usize>)> {
    join_writers_with(cache_root, || None)
}

/// `join_writers` that also deletes, in the same exclusive hold, the files of
/// the type folders that `named_files` (called once the lock is held) says no
/// manifest entry names. See `sweep_unnamed_files`.
fn join_writers_with<N>(cache_root: &Path, named_files: N) -> Option<(fs::File, Option<usize>)>
where
    N: FnOnce() -> Option<HashSet<(OsString, OsString)>>,
{
    let lock_path = writers_lock_path(cache_root);
    if let Some(folder) = lock_path.parent() {
        if let Err(e) = fs::create_dir_all(folder) {
            log_join_failure(&format!("Could not create the writers lock folder ({e}); temp files are not swept"));
            return None;
        }
    }
    let file = match fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
    {
        Ok(file) => file,
        Err(e) => {
            log_join_failure(&format!("Could not open the writers lock ({e}); temp files are not swept"));
            return None;
        }
    };
    let swept = if file.try_lock().is_ok() {
        let mut removed = sweep_orphan_temp_files(cache_root);
        if let Some(named) = named_files() {
            removed += sweep_unnamed_files(cache_root, &named);
        }
        if let Err(e) = file.unlock() {
            debug!("[UniversalCache] Could not release the sweep lock: {e}");
        }
        Some(removed)
    } else {
        debug!("[UniversalCache] Another instance is writing; temp files are not swept");
        None
    };
    if let Err(e) = file.lock_shared() {
        log_join_failure(&format!("Could not lock the writers file ({e})"));
        return None;
    }
    Some((file, swept))
}

/// Whether a rename was refused because something holds the file: a
/// `PermissionDenied` (antivirus, or a reader holding the target), or on Windows
/// a sharing violation (32) or lock violation (33).
fn rename_is_refused(e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::PermissionDenied || (cfg!(windows) && matches!(e.raw_os_error(), Some(32 | 33)))
}

/// `write_atomic` with the final rename passed in. A rename refused as
/// `rename_is_refused` says is retried after each wait in `RENAME_RETRY_MS`. Any
/// other failure, or the last refusal, deletes the temp file and returns the
/// error.
fn write_atomic_with<B, R>(dest: &Path, bytes: B, mut rename: R) -> std::io::Result<()>
where
    B: AsRef<[u8]>,
    R: FnMut(&Path, &Path) -> std::io::Result<()>,
{
    let mut tmp_name = dest.file_name().unwrap_or_default().to_os_string();
    tmp_name.push(format!(".{}.tmp", uuid::Uuid::new_v4().simple()));
    let tmp_path = dest.with_file_name(tmp_name);
    let result = (|| -> std::io::Result<()> {
        let mut file = fs::File::create(&tmp_path)?;
        std::io::copy(&mut Cursor::new(bytes), &mut file)?;
        file.sync_all()?;
        drop(file);
        let mut waits = RENAME_RETRY_MS.iter();
        loop {
            match rename(&tmp_path, dest) {
                Err(e) if rename_is_refused(&e) => match waits.next() {
                    Some(ms) => std::thread::sleep(std::time::Duration::from_millis(*ms)),
                    None => return Err(e),
                },
                other => return other,
            }
        }
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp_path);
    }
    result
}

/// The file name for a cached file: its id (path separators replaced), a short
/// hash of the id and the URL, and the extension. The same key fetched from
/// another URL gets another name, and so do two ids that differ only in
/// characters a file name cannot hold, so one name always holds the same bytes.
fn content_addressed_file_name(id: &str, url: &str, extension: &str) -> String {
    format!("{}-{}.{extension}", safe_id(id), url_hash8(&format!("{id}\n{url}")))
}

/// Most characters of an id kept in a file name; the hash that follows covers
/// the whole id.
const SAFE_ID_MAX_CHARS: usize = 64;

/// Most bytes of an id kept in a file name, for ids of multi-byte characters.
const SAFE_ID_MAX_BYTES: usize = 120;

/// `id` made usable in a file name on every platform: each character Windows
/// rejects in a name (`<>:"/\|?*` and the control characters) and each dot
/// becomes `_`, the result is cut to `SAFE_ID_MAX_CHARS` characters (and
/// `SAFE_ID_MAX_BYTES` bytes), trailing spaces are dropped, and a reserved
/// device name gets a leading `_`.
fn safe_id(id: &str) -> String {
    let mut replaced = String::new();
    for c in id.chars() {
        let c = match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' | '.' => '_',
            c if (c as u32) < 0x20 => '_',
            c => c,
        };
        if replaced.chars().count() >= SAFE_ID_MAX_CHARS || replaced.len() + c.len_utf8() > SAFE_ID_MAX_BYTES {
            break;
        }
        replaced.push(c);
    }
    let trimmed = replaced.trim_end_matches(' ');
    if is_reserved_device_name(trimmed) {
        format!("_{trimmed}")
    } else {
        trimmed.to_string()
    }
}

/// Whether Windows reserves `name` as a device (`CON`, `PRN`, `AUX`, `NUL`,
/// `COM1` to `COM9`, `LPT1` to `LPT9`), with or without an extension and in any
/// letter case.
fn is_reserved_device_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or("").trim_end_matches(' ');
    let upper = stem.to_ascii_uppercase();
    matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (upper.len() == 4
            && (upper.starts_with("COM") || upper.starts_with("LPT"))
            && matches!(upper.as_bytes()[3], b'1'..=b'9'))
}

/// Whether `name` is a temp file `write_atomic_with` left behind: the target
/// name, a dot, 32 lowercase hex digits and `.tmp`.
fn is_temp_leftover(name: &str) -> bool {
    let Some(stem) = name.strip_suffix(".tmp") else {
        return false;
    };
    let Some((_, token)) = stem.rsplit_once('.') else {
        return false;
    };
    token.len() == 32 && token.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Deletes the temp files a crash left in the cache's file folders and returns
/// how many were removed. Call it only while holding the writers lock
/// exclusively (see `join_writers`), so no live writer's temp file can be taken.
pub fn sweep_orphan_temp_files(cache_root: &Path) -> usize {
    let mut removed = 0;
    for cache_type in [
        CacheType::Badge,
        CacheType::Emote,
        CacheType::ThirdPartyBadge,
        CacheType::Cosmetic,
        CacheType::StreamNookArt,
    ] {
        let Ok(entries) = fs::read_dir(cache_root.join(cache_type.dir_name())) else {
            continue;
        };
        for entry in entries.flatten() {
            let is_file = entry.file_type().map_or(false, |t| t.is_file());
            if is_file && is_temp_leftover(&entry.file_name().to_string_lossy()) && fs::remove_file(entry.path()).is_ok() {
                removed += 1;
            }
        }
    }
    if removed > 0 {
        log::info!("[UniversalCache] Removed {removed} temp files left by an earlier run");
    }
    removed
}

/// Whether `name` is shaped like a cached file name (`content_addressed_file_name`
/// and `streamnook_art_file_name`): an id that may be empty, then a dash, 8
/// lowercase hex digits, a dot and an alphanumeric extension. With an empty id
/// the dash is left out as well, so the name is the 8 digits and the extension.
fn is_hashed_cache_name(name: &str) -> bool {
    let Some((stem, ext)) = name.rsplit_once('.') else {
        return false;
    };
    if ext.is_empty() || !ext.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return false;
    }
    let hash = stem.rsplit_once('-').map_or(stem, |(_, hash)| hash);
    hash.len() == 8 && hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Deletes the files of the cache's type folders that are shaped like cached
/// file names (`is_hashed_cache_name`) and that no manifest entry names, which a
/// crash between a file write and the manifest write leaves behind. `named` holds
/// the (type folder, file name) pairs the manifest names. Any other name in a
/// type folder is left alone. Call it only while holding the writers lock
/// exclusively (see `join_writers_with`) and with the manifest loaded; returns how
/// many files were removed.
fn sweep_unnamed_files(cache_root: &Path, named: &HashSet<(OsString, OsString)>) -> usize {
    let mut removed = 0;
    for cache_type in [
        CacheType::Badge,
        CacheType::Emote,
        CacheType::ThirdPartyBadge,
        CacheType::Cosmetic,
        CacheType::StreamNookArt,
    ] {
        let folder = cache_type.dir_name();
        let Ok(entries) = fs::read_dir(cache_root.join(folder)) else {
            continue;
        };
        for entry in entries.flatten() {
            let is_file = entry.file_type().map_or(false, |t| t.is_file());
            let file_name = entry.file_name();
            let key = (OsString::from(folder), file_name.clone());
            let unnamed = file_name.to_str().is_some_and(is_hashed_cache_name) && !named.contains(&key);
            if is_file && unnamed && fs::remove_file(entry.path()).is_ok() {
                removed += 1;
            }
        }
    }
    if removed > 0 {
        log::info!("[UniversalCache] Removed {removed} cached files that no manifest entry names");
    }
    removed
}

/// `p` with links and relative parts resolved. A path that is not there gives
/// `None`; any other failure is logged.
fn resolve_path(p: &Path) -> Option<PathBuf> {
    match p.canonicalize() {
        Ok(c) => Some(c),
        Err(e) => {
            if e.kind() != std::io::ErrorKind::NotFound {
                log::warn!("[UniversalCache] Could not resolve {}: {e}", p.display());
            }
            None
        }
    }
}

/// Deletes the cached file at `path` when it sits inside `type_dir`, which is
/// already resolved (see `resolve_path`). A path outside the folder, the folder
/// itself and a file already gone are left alone; any other failure is logged.
fn remove_cached_file(type_dir: &Path, path: &Path) {
    let Some(file) = resolve_path(path) else {
        return;
    };
    if file == type_dir || !file.starts_with(type_dir) {
        return;
    }
    match fs::remove_file(&file) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => log::warn!("[UniversalCache] Could not delete {}: {e}", file.display()),
    }
}

/// The first 4 bytes of the SHA-256 of `url`, as 8 hex digits.
fn url_hash8(url: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(url.as_bytes()).iter().take(4).map(|b| format!("{b:02x}")).collect()
}

/// Write a downloaded file into its type folder and build its manifest entry,
/// without saving the manifest. Returns the file's path with the entry. An HTML
/// page is refused before anything is written. The type folder is found, and
/// created if need be, on the blocking pool with the write.
async fn write_file_entry<B>(
    cache_type: CacheType,
    id: String,
    url: String,
    file_name: String,
    bytes: B,
    expiry_days: u32,
) -> Result<(String, UniversalCacheEntry)>
where
    B: AsRef<[u8]> + Send + 'static,
{
    let folder_type = cache_type.clone();
    write_file_entry_with(
        move || file_type_dir(&folder_type),
        cache_type,
        id,
        url,
        file_name,
        bytes,
        expiry_days,
    )
    .await
}

/// `write_file_entry` with the type folder given by `type_dir`, which runs on the
/// blocking pool.
async fn write_file_entry_with<B, D>(
    type_dir: D,
    cache_type: CacheType,
    id: String,
    url: String,
    file_name: String,
    bytes: B,
    expiry_days: u32,
) -> Result<(String, UniversalCacheEntry)>
where
    B: AsRef<[u8]> + Send + 'static,
    D: FnOnce() -> Result<PathBuf> + Send + 'static,
{
    refuse_html(bytes.as_ref())?;

    // Write on the blocking pool: a channel join downloads many emotes/badges
    // and a sync write per file on the runtime saturates the worker threads
    // (the join-time freeze). The HTTP fetch is already async.
    let write_name = file_name.clone();
    let file_path = tokio::task::spawn_blocking(move || -> Result<PathBuf> {
        let file_path = type_dir()?.join(write_name);
        write_atomic(&file_path, bytes)?;
        Ok(file_path)
    })
    .await
    .context("cache_file write task panicked")??;

    let path_str = file_path.to_string_lossy().to_string();
    let entry = UniversalCacheEntry {
        id: format!("file:{}", id),
        cache_type,
        data: serde_json::json!({
            "local_path": path_str,
            "url": url,
            "file_name": file_name
        }),
        metadata: CacheMetadata {
            timestamp: get_current_timestamp(),
            expiry_days,
            source: "universal_file".to_string(),
            version: CACHE_VERSION,
        },
        position: None,
    };
    Ok((path_str, entry))
}

/// The only host StreamNook art is fetched from.
const STREAMNOOK_ART_ORIGIN: &str = "https://cdn.streamnook.app/";
/// Largest StreamNook art file accepted. The biggest badge today is ~4 MB.
const STREAMNOOK_ART_MAX_BYTES: usize = 24 * 1024 * 1024;
/// Connections a resumable fetch may use for one file before it gives up.
const RESUMABLE_ATTEMPTS: u32 = 8;

// StreamNook's own art is a few large animated files, so it gets a client with
// no total cap: at a slow link's speed the 20 s cap above never finishes one.
// The read timeout still frees a stalled transfer, and the fetch resumes from
// the bytes it already has.
static LARGE_DOWNLOAD_CLIENT: Lazy<reqwest::Client> = Lazy::new(|| {
    reqwest::Client::builder()
        .user_agent("StreamNook/1.0")
        .connect_timeout(std::time::Duration::from_secs(10))
        .read_timeout(std::time::Duration::from_secs(20))
        .build()
        .expect("failed to build large-download HTTP client")
});

/// Whether `url` is StreamNook art this cache will fetch.
pub fn is_streamnook_art_url(url: &str) -> bool {
    url.starts_with(STREAMNOOK_ART_ORIGIN) && url.len() > STREAMNOOK_ART_ORIGIN.len()
}

/// The file name for a piece of StreamNook art: its readable name from the URL
/// plus a short hash of the whole URL, so a re-upload under `?v=3` gets its own
/// file beside the old one.
fn streamnook_art_file_name(url: &str, extension: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let last = path.rsplit('/').next().unwrap_or("");
    let stem = last.rsplit_once('.').map_or(last, |(stem, _)| stem);
    let readable: String = stem
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(48)
        .collect();
    let hash = url_hash8(url);
    if readable.is_empty() {
        format!("{hash}.{extension}")
    } else {
        format!("{readable}-{hash}.{extension}")
    }
}

/// The first byte and the total size in a `Content-Range: bytes a-b/total`.
fn content_range_start(value: &str) -> Option<(u64, Option<u64>)> {
    let rest = value.trim().strip_prefix("bytes ")?;
    let (range, total) = rest.split_once('/')?;
    let start = range.split_once('-')?.0.trim().parse().ok()?;
    Some((start, total.trim().parse().ok()))
}

/// GET `url` in full, resuming with a Range request after a dropped connection.
/// Some networks cut a long transfer to a host part way through, so a plain GET
/// of a multi-MB file can fail every time while a resumed one gets there.
/// Returns the body and its Content-Type.
async fn fetch_resumable(
    client: &reqwest::Client,
    url: &str,
    max_bytes: usize,
    attempts: u32,
    backoff: std::time::Duration,
) -> Result<(Vec<u8>, Option<String>)> {
    use reqwest::{header, StatusCode};

    let mut body: Vec<u8> = Vec::new();
    let mut content_type: Option<String> = None;
    let mut last_error = anyhow::anyhow!("no attempt made");
    for attempt in 0..attempts {
        if attempt > 0 {
            tokio::time::sleep(backoff * 2u32.pow((attempt - 1).min(5))).await;
        }
        let mut request = client.get(url);
        if !body.is_empty() {
            request = request.header(header::RANGE, format!("bytes={}-", body.len()));
        }
        let mut response = match request.send().await {
            Ok(response) => response,
            Err(e) => {
                last_error = e.into();
                continue;
            }
        };
        let status = response.status();
        let range = response
            .headers()
            .get(header::CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .and_then(content_range_start);
        if status == StatusCode::PARTIAL_CONTENT {
            // Only a resume that starts where this copy stopped can be appended.
            if range.map(|(start, _)| start) != Some(body.len() as u64) {
                body.clear();
                last_error = anyhow::anyhow!("resume answered from the wrong offset");
                continue;
            }
        } else if status.is_success() {
            // A server that ignores Range sends the whole file again.
            body.clear();
        } else if status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS {
            last_error = anyhow::anyhow!("HTTP {status}");
            continue;
        } else {
            return Err(anyhow::anyhow!("HTTP {status}"));
        }
        if content_type.is_none() {
            content_type = response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);
        }
        loop {
            match response.chunk().await {
                Ok(Some(chunk)) => {
                    body.extend_from_slice(&chunk);
                    if body.len() > max_bytes {
                        return Err(anyhow::anyhow!("larger than {max_bytes} bytes"));
                    }
                }
                Ok(None) => {
                    let total = range.and_then(|(_, total)| total);
                    if total.is_none_or(|total| total == body.len() as u64) {
                        return Ok((body, content_type));
                    }
                    last_error = anyhow::anyhow!("ended at {} of {:?} bytes", body.len(), total);
                    break;
                }
                Err(e) => {
                    last_error = e.into();
                    break;
                }
            }
        }
    }
    Err(last_error.context(format!("gave up after {attempts} connections")))
}

/// Cache one piece of StreamNook art from cdn.streamnook.app. The URL is the
/// manifest id. Only images are kept: a captive portal or a block page that
/// answers 200 with HTML is never written as art.
pub async fn cache_streamnook_art(url: String) -> Result<String> {
    if !is_streamnook_art_url(&url) {
        return Err(anyhow::anyhow!("not StreamNook art: {url}"));
    }
    let (bytes, content_type) = fetch_resumable(
        &LARGE_DOWNLOAD_CLIENT,
        &url,
        STREAMNOOK_ART_MAX_BYTES,
        RESUMABLE_ATTEMPTS,
        std::time::Duration::from_secs(1),
    )
    .await?;
    let extension = detect_image_ext(&bytes, content_type.as_deref());
    if extension == "bin" {
        return Err(anyhow::anyhow!(
            "answer was not an image ({})",
            content_type.as_deref().unwrap_or("no content type")
        ));
    }
    let file_name = streamnook_art_file_name(&url, extension);
    // Never expires: new art ships under a new URL.
    let (path_str, entry) = write_file_entry(
        CacheType::StreamNookArt,
        url.clone(),
        url,
        file_name,
        bytes,
        0,
    )
    .await?;
    save_cached_item(entry).await?;
    Ok(path_str)
}

/// Download a file to disk and RETURN its manifest entry WITHOUT writing the
/// manifest. Lets a bulk caller (the AFK emote prefetch) collect many entries
/// and persist them in one `save_cached_items_batch` call, sidestepping the
/// per-file manifest clone that makes `cache_file` O(N^2) in a tight loop. The
/// extension is derived from the actual content (see `detect_image_ext`), not
/// the URL. Callers (the prefetch plan) dedup upstream, so this always fetches.
pub async fn download_file_to_disk(
    cache_type: CacheType,
    id: String,
    url: String,
    expiry_days: u32,
) -> Result<UniversalCacheEntry> {
    let response = DOWNLOAD_CLIENT.get(&url).send().await?;
    if !response.status().is_success() {
        return Err(anyhow::anyhow!(
            "Failed to download file: {}",
            response.status()
        ));
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let bytes = response.bytes().await?;

    // Type the file by its real content; the manifest id keeps the raw (already
    // provider-namespaced) id, only path separators stripped for the filename.
    let extension = detect_image_ext(&bytes, content_type.as_deref());
    let file_name = content_addressed_file_name(&id, &url, extension);
    let (_, entry) = write_file_entry(cache_type, id, url, file_name, bytes, expiry_days).await?;
    Ok(entry)
}

/// Get cached file path if exists and valid
pub async fn get_cached_file_path(cache_type: CacheType, id: &str) -> Result<Option<String>> {
    let manifest_id = format!("file:{}", id);

    // Use existing get_cached_item which handles expiry
    let entry = get_cached_item(cache_type.clone(), &manifest_id).await?;

    if let Some(entry) = entry {
        if let Some(path) = entry.data.get("local_path").and_then(|p| p.as_str()) {
            let path_buf = PathBuf::from(path);
            if path_buf.exists() {
                return Ok(Some(path.to_string()));
            }
        }
    }

    Ok(None)
}

/// Get ALL cached items of a specific type - efficient batch lookup (single disk read)
pub fn get_all_cached_items_by_type(
    cache_type: CacheType,
) -> Result<HashMap<String, UniversalCacheEntry>> {
    // Reads under the lock and clones only the entries it keeps. It used to call
    // load_manifest(), which deep-clones EVERY entry, and then throw most of
    // them away. Mobile calls this on the Rewards screen, where the manifest is
    // almost entirely badges, so the discarded half of that clone was the
    // expensive half.
    drop_missing_files(&cache_type);
    let guard = MANIFEST_MEMORY
        .read()
        .map_err(|_| anyhow::anyhow!("manifest memory lock poisoned"))?;
    let mut items = HashMap::new();

    for (key, entry) in guard.entries.iter() {
        if entry.cache_type == cache_type && !is_cache_expired(&entry.metadata) {
            items.insert(key.clone(), entry.clone());
        }
    }

    Ok(items)
}

/// Get multiple cached items by their IDs - efficient batch lookup (single disk read)
pub fn get_cached_items_batch(
    cache_type: CacheType,
    ids: &[String],
) -> Result<HashMap<String, UniversalCacheEntry>> {
    // Same reasoning as get_all_cached_items_by_type: this asks for a handful of
    // ids, so cloning the whole manifest to look them up was the wrong shape.
    let guard = MANIFEST_MEMORY
        .read()
        .map_err(|_| anyhow::anyhow!("manifest memory lock poisoned"))?;
    let mut items = HashMap::new();

    for id in ids {
        if let Some(entry) = guard.entries.get(id) {
            if entry.cache_type == cache_type && !is_cache_expired(&entry.metadata) {
                items.insert(id.clone(), entry.clone());
            }
        }
    }

    Ok(items)
}

/// Get all cached files for a specific type
/// The on-disk path a file entry serves, when it may be served. Emote and badge
/// files never go stale by age; a cosmetic's art can change under the same id
/// (7TV can animate a paint after it was cached), so cosmetic files honor their
/// expiry and re-download the next time they are shown.
fn usable_file_path<'a>(cache_type: &CacheType, entry: &'a UniversalCacheEntry) -> Option<&'a str> {
    if entry.cache_type != *cache_type {
        return None;
    }
    if *cache_type == CacheType::Cosmetic && is_cache_expired(&entry.metadata) {
        return None;
    }
    entry.data.get("local_path").and_then(|p| p.as_str())
}

/// Cache types whose file entries were checked against the disk this session,
/// by folder name. See `drop_missing_files`.
static VERIFIED_TYPES: Lazy<StdMutex<HashSet<&'static str>>> = Lazy::new(|| StdMutex::new(HashSet::new()));

/// Whether `path` is on disk. `present` is a listing of `type_dir` (None when
/// it could not be read), so the common case costs no stat of its own.
fn file_present(path: &Path, type_dir: &Path, present: Option<&HashSet<OsString>>) -> bool {
    match (present, path.parent(), path.file_name()) {
        (Some(names), Some(parent), Some(name)) if parent == type_dir => names.contains(name),
        _ => path.exists(),
    }
}

/// The `file:` entries of `cache_type` whose file is not on disk.
fn missing_file_keys(
    entries: &HashMap<String, UniversalCacheEntry>,
    cache_type: &CacheType,
    type_dir: &Path,
    present: Option<&HashSet<OsString>>,
) -> Vec<String> {
    entries
        .iter()
        .filter(|(key, entry)| key.starts_with("file:") && entry.cache_type == *cache_type)
        .filter_map(|(key, entry)| {
            let path = entry.data.get("local_path")?.as_str()?;
            (!file_present(Path::new(path), type_dir, present)).then(|| key.clone())
        })
        .collect()
}

/// Drop this type's file entries whose file is gone, once per session.
///
/// The manifest can outlive its files: the folder cleaned while the app ran
/// (the in-memory copy is then flushed back over the cleaned one), a clear that
/// stopped partway, a data folder copied from another profile. Every such entry
/// was handed to the windows as cached, so they never asked for it again, drew
/// nothing, and every render asked the asset protocol for the missing file: one
/// error line each, thousands an hour, until the log rotated away everything
/// else. Dropping the entry lets the next request download the file again.
///
/// One directory listing per type rather than a stat per entry, because the
/// emote folder can hold tens of thousands of files.
fn drop_missing_files(cache_type: &CacheType) {
    let dir_name = cache_type.dir_name();
    let first_this_session = VERIFIED_TYPES.lock().map(|mut v| v.insert(dir_name)).unwrap_or(false);
    if !first_this_session {
        return;
    }
    let Ok(type_dir) = get_universal_cache_dir().map(|d| d.join(dir_name)) else {
        return;
    };
    let present: Option<HashSet<OsString>> = match fs::read_dir(&type_dir) {
        Ok(listing) => Some(listing.flatten().map(|e| e.file_name()).collect()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(HashSet::new()),
        Err(_) => None,
    };
    let missing = {
        let Ok(guard) = MANIFEST_MEMORY.read() else {
            return;
        };
        missing_file_keys(&guard.entries, cache_type, &type_dir, present.as_ref())
    };
    if missing.is_empty() {
        return;
    }

    // Same locking as `upsert_cached_entry`. Re-checked under the lock: a
    // download may have written one of these files since the listing.
    let _lock = MANIFEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut dropped = 0usize;
    if let Ok(mut guard) = MANIFEST_MEMORY.write() {
        for key in &missing {
            let gone = guard
                .entries
                .get(key)
                .and_then(|e| e.data.get("local_path"))
                .and_then(|p| p.as_str())
                .is_some_and(|p| !Path::new(p).exists());
            if gone && guard.entries.remove(key).is_some() {
                dropped += 1;
            }
        }
    }
    if dropped > 0 {
        MANIFEST_DIRTY.store(true, Ordering::Release);
        ensure_flush_task();
        log::info!(
            "[UniversalCache] {dropped} cached {dir_name} file(s) were missing on disk; dropped so they download again"
        );
    }
}

/// The cached files among `ids` (id -> path), by the same rule as the listing.
/// Reads the in-memory manifest only.
pub fn cached_file_paths(cache_type: CacheType, ids: &[String]) -> HashMap<String, String> {
    drop_missing_files(&cache_type);
    let Ok(guard) = MANIFEST_MEMORY.read() else {
        return HashMap::new();
    };
    ids.iter()
        .filter_map(|id| {
            let entry = guard.entries.get(&format!("file:{id}"))?;
            usable_file_path(&cache_type, entry).map(|p| (id.clone(), p.to_string()))
        })
        .collect()
}

/// `cached_files_list` for async callers. It lists a type's folder the first time
/// the type is asked for, so call it from the blocking pool.
pub async fn get_cached_files_list(cache_type: CacheType) -> Result<HashMap<String, String>> {
    cached_files_list(cache_type)
}

/// The cached files of a type (id -> path). The first call for a type lists its
/// folder, so call this from a blocking thread.
pub fn cached_files_list(cache_type: CacheType) -> Result<HashMap<String, String>> {
    // debug!(
    //     "[UniversalCache] Getting cached files list for {:?}",
    //     cache_type
    // );
    drop_missing_files(&cache_type);
    // Filtered under the read lock. `load_manifest()` clones every entry of
    // every kind first, which cost ~150 ms per call on a 70k-entry cache to
    // return a few hundred badge paths.
    let guard = MANIFEST_MEMORY
        .read()
        .map_err(|_| anyhow::anyhow!("manifest memory lock poisoned"))?;
    let mut files = HashMap::new();

    for (key, entry) in guard.entries.iter() {
        if let (Some(id), Some(path)) = (key.strip_prefix("file:"), usable_file_path(&cache_type, entry)) {
            files.insert(id.to_string(), path.to_string());
        }
    }

    // debug!(
    //     "[UniversalCache] Found {} cached files for {:?}",
    //     files.len(),
    //     cache_type
    // );
    Ok(files)
}

/// Fetch the remote manifest's last_sync timestamp (lightweight check)
/// Returns Some(timestamp) if remote manifest exists and has a last_sync, None otherwise
async fn fetch_remote_manifest_timestamp() -> Result<Option<u64>> {
    let url = format!("{}/manifest.json", UNIVERSAL_CACHE_URL);

    let client = &*META_CLIENT_10S;

    match fetch_cache_file(client, "manifest.json").await {
        Ok(response) if response.status().is_success() => {
            let remote_manifest: UniversalCacheManifest = response.json().await?;
            Ok(remote_manifest.last_sync)
        }
        Ok(response) => {
            debug!(
                "[UniversalCache] Remote manifest fetch returned status: {}",
                response.status()
            );
            Ok(None)
        }
        Err(e) => {
            debug!(
                "[UniversalCache] Failed to fetch remote manifest timestamp: {}",
                e
            );
            Ok(None)
        }
    }
}

/// Auto-sync universal cache if remote manifest is newer than local
/// Returns true if sync was triggered, false if cache was already current
pub async fn auto_sync_if_stale() -> Result<bool> {
    let manifest = load_manifest()?;
    let local_sync = manifest.last_sync.unwrap_or(0);

    // Fetch remote manifest timestamp to compare
    let remote_sync = match fetch_remote_manifest_timestamp().await {
        Ok(Some(ts)) => ts,
        Ok(None) => {
            debug!("[UniversalCache] Remote manifest unavailable, skipping sync");
            return Ok(false);
        }
        Err(e) => {
            debug!("[UniversalCache] Failed to check remote manifest: {}", e);
            return Ok(false);
        }
    };

    let should_sync = remote_sync > local_sync;

    if should_sync {
        debug!(
            "[UniversalCache] Remote manifest is newer (remote: {}, local: {}), triggering sync",
            remote_sync, local_sync
        );
        // Fire-and-forget: spawn download in background so we don't block startup
        tokio::spawn(async {
            match download_universal_manifest().await {
                Ok(true) => debug!("[UniversalCache] Auto-sync completed successfully"),
                Ok(false) => debug!("[UniversalCache] Auto-sync skipped (already in progress)"),
                Err(e) => debug!("[UniversalCache] Auto-sync failed: {}", e),
            }
        });
        Ok(true)
    } else {
        debug!(
            "[UniversalCache] Local cache is current (remote: {}, local: {})",
            remote_sync, local_sync
        );
        Ok(false)
    }
}

/// Entries in the in-memory universal cache manifest (try-read). Diagnostics
/// for the resource line.
pub fn manifest_len() -> Option<usize> {
    MANIFEST_MEMORY.try_read().ok().map(|m| m.entries.len())
}

#[cfg(test)]
mod tests {
    use super::{
        assign_positions_in_with, badge_date_key, clear_cache_in, content_addressed_file_name, delete_unnamed_files,
        detect_image_ext, drop_expired, encode_with, flush_manifest_with, is_hashed_cache_name, is_still_paint_entry,
        join_writers, looks_like_html, looks_like_svg, merge_remote_into_with, missing_file_keys, parse_manifest,
        queue_entry_files, refuse_html, remove_cached_file, resolve_path, safe_id, stamp_last_sync_in, store_entries,
        sweep_before_write, sweep_orphan_temp_files, sweep_unnamed_files, take_pending, usable_file_path,
        webp_is_animated, write_atomic_held, write_atomic_with, write_file_entry_with, writers_lock_path, CacheMetadata,
        CacheType, PendingDelete, UniversalCacheEntry, UniversalCacheManifest, DELETE_CHUNK, WRITERS_LOCK_FILE,
    };
    use std::collections::{HashMap, HashSet};
    use std::ffi::OsString;
    use std::fs;
    use std::io::{Error, ErrorKind};
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, RwLock};

    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sn-cache-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn temp_files_in(dir: &Path) -> Vec<String> {
        fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".tmp"))
            .collect()
    }

    #[test]
    fn concurrent_writers_of_one_file_leave_one_complete_payload_and_no_temp_files() {
        let dir = scratch_dir("concurrent");
        let dest = dir.join("a-12345678.webp");
        let payloads = [vec![0xAA_u8; 256 * 1024], vec![0xBB_u8; 256 * 1024]];
        let barrier = std::sync::Barrier::new(payloads.len());
        std::thread::scope(|scope| {
            for payload in &payloads {
                let (dest, barrier) = (&dest, &barrier);
                scope.spawn(move || {
                    barrier.wait();
                    for _ in 0..10 {
                        write_atomic_with(dest, payload.as_slice(), |from, to| fs::rename(from, to)).unwrap();
                    }
                });
            }
        });
        let written = fs::read(&dest).unwrap();
        assert!(payloads.iter().any(|p| *p == written), "the file is one whole payload");
        assert!(temp_files_in(&dir).is_empty(), "{:?}", temp_files_in(&dir));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_write_leaves_no_temp_file() {
        let dir = scratch_dir("failed-write");
        let dest = dir.join("b-12345678.webp");
        let err = write_atomic_with(&dest, b"bytes", |_, _| Err(Error::new(ErrorKind::Other, "injected"))).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Other);
        assert!(!dest.exists());
        assert!(temp_files_in(&dir).is_empty(), "{:?}", temp_files_in(&dir));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_refused_rename_is_retried_and_then_given_up_cleanly() {
        let dir = scratch_dir("refused-rename");
        let dest = dir.join("c-12345678.webp");
        let mut calls = 0;
        write_atomic_with(&dest, b"landed", |from, to| {
            calls += 1;
            if calls < 3 {
                Err(Error::new(ErrorKind::PermissionDenied, "held by a reader"))
            } else {
                fs::rename(from, to)
            }
        })
        .unwrap();
        assert_eq!(calls, 3, "two refusals, then the rename lands");
        assert_eq!(fs::read(&dest).unwrap(), b"landed");

        let never = dir.join("d-12345678.webp");
        let mut attempts = 0;
        let err = write_atomic_with(&never, b"x", |_, _| {
            attempts += 1;
            Err(Error::new(ErrorKind::PermissionDenied, "held by a reader"))
        })
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
        assert_eq!(attempts, 4, "the first try and three retries");
        assert!(!never.exists());
        assert!(temp_files_in(&dir).is_empty(), "{:?}", temp_files_in(&dir));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_cached_file_name_follows_its_url() {
        let a = content_addressed_file_name("emote/x@1x", "https://cdn.example/a.webp", "webp");
        let b = content_addressed_file_name("emote/x@1x", "https://cdn.example/b.webp", "webp");
        assert_ne!(a, b, "the same key from another URL is another file");
        assert_eq!(a, content_addressed_file_name("emote/x@1x", "https://cdn.example/a.webp", "webp"));
        assert!(a.starts_with("emote_x@1x-") && a.ends_with(".webp"), "{a}");
        assert_eq!(a.len(), "emote_x@1x-".len() + 8 + ".webp".len(), "{a}");
    }

    #[test]
    fn two_ids_never_share_a_file_name() {
        let slash = content_addressed_file_name("a/b", "https://cdn.example/a.webp", "webp");
        let colon = content_addressed_file_name("a:b", "https://cdn.example/a.webp", "webp");
        assert_ne!(slash, colon, "ids that differ only in a character a name cannot hold");
        assert!(slash.starts_with("a_b-") && colon.starts_with("a_b-"), "{slash} {colon}");
    }

    #[test]
    fn html_is_never_an_image() {
        let page: &[u8] = b"  <!DOCTYPE HTML><HTML><BODY>Sign in to the network</BODY></HTML>";
        assert!(looks_like_html(page));
        assert!(looks_like_html(b"\xEF\xBB\xBF\n<head><title>x</title></head>"));
        assert!(looks_like_html(b"<body onload='x'>"));
        assert!(!looks_like_html(b"<svg xmlns='http://www.w3.org/2000/svg'/>"));
        assert!(!looks_like_html(b"\x89PNG\r\n\x1a\nrest"));
        assert_eq!(detect_image_ext(page, Some("image/svg+xml")), "bin");
        assert_eq!(detect_image_ext(page, Some("image/png")), "bin");
        assert!(refuse_html(page).is_err());
        assert!(refuse_html(b"<svg/>").is_ok());
        assert!(refuse_html(b"\x89PNG\r\n\x1a\nrest").is_ok());
    }

    #[tokio::test]
    async fn an_html_answer_is_refused_before_anything_is_written() {
        let root = scratch_dir("html-refused");
        let page = b"<html><body>Sign in to the network</body></html>".to_vec();
        let type_dir = root.clone();
        let result = write_file_entry_with(
            move || Ok(type_dir),
            CacheType::Emote,
            "x".into(),
            "https://cdn.example/x.png".into(),
            "x-12345678.png".into(),
            page,
            7,
        )
        .await;
        assert!(result.is_err());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0, "no file was written");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_sweep_is_skipped_while_another_instance_may_be_writing() {
        let base = scratch_dir("writers");
        let root = base.join("cache");
        let emotes = root.join("emotes");
        fs::create_dir_all(&emotes).unwrap();
        let leftover = emotes.join(format!("a.png.{}.tmp", "0123456789abcdef0123456789abcdef"));
        fs::write(&leftover, b"half").unwrap();
        let open_lock_file = || {
            fs::create_dir_all(writers_lock_path(&root).parent().unwrap()).unwrap();
            fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(writers_lock_path(&root))
                .unwrap()
        };

        // Another instance holds the writers lock shared.
        let other = open_lock_file();
        other.lock_shared().unwrap();
        let (mine, swept) = join_writers(&root).expect("joins the writers");
        assert_eq!(swept, None, "the sweep is skipped");
        assert!(leftover.exists(), "the temp file stays");
        drop(mine);
        drop(other);

        // Nobody else writes: the sweep runs, then this process holds the lock shared.
        let (mine, swept) = join_writers(&root).expect("joins the writers");
        assert_eq!(swept, Some(1));
        assert!(!leftover.exists());
        let third = open_lock_file();
        assert!(third.try_lock().is_err(), "a sweeper cannot start while this process may write");
        drop(third);
        drop(mine);
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn the_writers_lock_sits_beside_the_cache_root_and_survives_removing_the_root() {
        let base = scratch_dir("lock-beside-root");
        let root = base.join("cache");
        fs::create_dir_all(root.join("emotes")).unwrap();
        fs::write(root.join("emotes").join("a-12345678.png"), b"png").unwrap();

        let lock_path = writers_lock_path(&root);
        assert_eq!(lock_path, base.join("locks").join("streamnook-cache-writers.lock"));
        assert_eq!(writers_lock_path(Path::new("")), Path::new(WRITERS_LOCK_FILE), "no parent: inside the root");

        let (mine, _) = join_writers(&root).expect("joins the writers");
        assert!(lock_path.exists(), "the lock file is outside the cache root");
        assert!(!root.join(WRITERS_LOCK_FILE).exists(), "nothing is created inside the cache root");

        // The removal and recreation `clear_universal_cache` performs.
        fs::remove_dir_all(&root).unwrap();
        fs::create_dir_all(&root).unwrap();

        assert!(lock_path.exists(), "the lock file outlives the clear");
        let other = fs::OpenOptions::new().read(true).write(true).open(&lock_path).unwrap();
        assert!(other.try_lock().is_err(), "the lock is still held after the clear");
        drop(other);
        drop(mine);
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn a_write_waits_for_the_one_sweep_of_the_process() {
        let base = scratch_dir("first-write");
        let root = base.join("cache");
        let emotes = root.join("emotes");
        fs::create_dir_all(&emotes).unwrap();
        let leftover = emotes.join(format!("a.png.{}.tmp", "0123456789abcdef0123456789abcdef"));
        fs::write(&leftover, b"half").unwrap();
        let dest = emotes.join("a-12345678.png");
        let holder = Mutex::new(None);

        sweep_before_write(&holder, &dest, || None);
        assert!(!leftover.exists(), "the sweep is done before the write starts");
        write_atomic_with(&dest, b"png", |from, to| fs::rename(from, to)).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"png");
        assert!(holder.lock().unwrap().is_some(), "the writers lock is kept");

        let later = emotes.join(format!("b.png.{}.tmp", "fedcba9876543210fedcba9876543210"));
        fs::write(&later, b"half").unwrap();
        sweep_before_write(&holder, &dest, || None);
        assert!(later.exists(), "the sweep runs once");
        drop(holder);
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn a_failed_join_is_tried_again_by_the_next_write() {
        let base = scratch_dir("join-retry");
        let root = base.join("cache");
        let emotes = root.join("emotes");
        fs::create_dir_all(&emotes).unwrap();
        let dest = emotes.join("a-12345678.png");
        let holder = Mutex::new(None);

        // A file where the locks folder belongs: the join cannot start.
        let blocker = base.join("locks");
        fs::write(&blocker, b"in the way").unwrap();
        assert!(!sweep_before_write(&holder, &dest, || None), "the lock is not held");
        assert!(holder.lock().unwrap().is_none(), "nothing is held after a failed join");

        fs::remove_file(&blocker).unwrap();
        assert!(sweep_before_write(&holder, &dest, || None), "the lock is held");
        assert!(holder.lock().unwrap().is_some(), "the next write joined");
        drop(holder);
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn a_write_without_the_writers_lock_fails_before_creating_any_file() {
        let base = scratch_dir("write-needs-lock");
        let emotes = base.join("cache").join("emotes");
        fs::create_dir_all(&emotes).unwrap();
        let dest = emotes.join("a-12345678.png");
        let holder = Mutex::new(None);

        // A file where the locks folder belongs: the join cannot start.
        let blocker = base.join("locks");
        fs::write(&blocker, b"in the way").unwrap();
        let err = write_atomic_held(&holder, &dest, b"png", || None).unwrap_err();
        assert!(err.to_string().contains("writers lock not held"), "{err}");
        assert!(!dest.exists(), "no file was written");
        assert_eq!(fs::read_dir(&emotes).unwrap().count(), 0, "no temp file was made");

        fs::remove_file(&blocker).unwrap();
        write_atomic_held(&holder, &dest, b"png", || None).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"png", "the next write lands");
        assert!(temp_files_in(&emotes).is_empty(), "{:?}", temp_files_in(&emotes));
        drop(holder);
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn clearing_the_cache_folders_leaves_the_writers_lock_held() {
        let base = scratch_dir("clear-proof");
        let root = base.join("cache");
        let emotes = root.join("emotes");
        fs::create_dir_all(&emotes).unwrap();
        fs::write(base.join("settings.json"), b"{}").unwrap();

        let (mine, _) = join_writers(&root).expect("joins the writers");
        let in_flight = emotes.join(format!("a.png.{}.tmp", "0123456789abcdef0123456789abcdef"));
        fs::write(&in_flight, b"half").unwrap();

        // What the app's clear-all does beside the cache root: delete the plain
        // files of the folder that holds it.
        for entry in fs::read_dir(&base).unwrap().flatten() {
            if entry.file_type().unwrap().is_file() {
                fs::remove_file(entry.path()).unwrap();
            }
        }
        assert!(writers_lock_path(&root).exists(), "the lock file is not a plain file of that folder");

        let (second, swept) = join_writers(&root).expect("joins the writers");
        assert_eq!(swept, None, "no exclusive lock while the first process may write");
        assert!(in_flight.exists(), "the in-flight temp file survives");
        drop(second);
        drop(mine);
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn a_file_no_entry_names_is_removed_by_the_sweep_and_other_names_are_kept() {
        let (root, files) = emote_files(
            "unnamed-sweep",
            &[
                "kept-aaaaaaaa.png",
                "orphan-bbbbbbbb.png",
                "note.txt",
                "plain-name.png",
                "x-ABCDEF12.png",
                "-cccccccc.png",
                "dddddddd.png",
            ],
        );
        let named: HashSet<(OsString, OsString)> =
            [(OsString::from("emotes"), OsString::from("kept-aaaaaaaa.png"))].into_iter().collect();

        assert_eq!(sweep_unnamed_files(&root, &named), 3);
        let left: Vec<bool> = files.iter().map(|f| f.exists()).collect();
        assert_eq!(
            left,
            [true, false, true, true, true, false, false],
            "only the unnamed hashed files go, with or without an id"
        );

        // The same name in another folder is not what the entry names.
        let other = root.join("badges");
        fs::create_dir_all(&other).unwrap();
        fs::write(other.join("kept-aaaaaaaa.png"), b"x").unwrap();
        assert_eq!(sweep_unnamed_files(&root, &named), 1);
        assert!(!other.join("kept-aaaaaaaa.png").exists());
        assert!(files[0].exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn cached_file_names_are_told_from_other_names() {
        assert!(is_hashed_cache_name("emote_x@1x-0123abcd.webp"));
        assert!(is_hashed_cache_name("streamnook-mobius-0123abcd.webp"));
        assert!(is_hashed_cache_name("0123abcd.webp"), "no id and no dash");
        assert!(is_hashed_cache_name("-0123abcd.png"), "an empty id");
        assert!(!is_hashed_cache_name("x-0123ABCD.webp"), "upper case hex");
        assert!(!is_hashed_cache_name("x-0123abc.webp"), "short hash");
        assert!(!is_hashed_cache_name("x-0123abcd"), "no extension");
        assert!(!is_hashed_cache_name(&format!("x-0123abcd.png.{}.tmp", "0123456789abcdef0123456789abcdef")));
    }

    #[test]
    fn a_comment_is_svg_only_when_an_svg_element_follows_it() {
        assert!(looks_like_svg(b"<!-- made by a tool -->\n<svg/>"));
        assert!(!looks_like_svg(b"<!-- made by a tool --><html><body></body></html>"));
        assert!(!looks_like_svg(b"<!-- only a comment -->"));
        assert!(!looks_like_svg(b"<?xml version='1.0'?><HTML></HTML>"));
        assert_eq!(detect_image_ext(b"<!-- page --><html></html>", None), "bin");
    }

    #[test]
    fn a_page_without_an_html_tag_is_never_svg() {
        assert!(!looks_like_svg(b"<!-- x --><!doctype html><title>Login</title><svg>"));
        assert!(!looks_like_svg(b"<?xml version='1.0'?><HEAD><title>x</title></HEAD><svg/>"));
        assert!(!looks_like_svg(b"<!-- x --><BODY><svg/></BODY>"));
        assert!(looks_like_svg(b"<?xml version='1.0'?><svg xmlns='http://www.w3.org/2000/svg'/>"));
    }

    #[test]
    fn crash_leftover_temp_files_are_swept_and_other_files_stay() {
        let root = scratch_dir("sweep");
        let emotes = root.join("emotes");
        let badges = root.join("badges");
        fs::create_dir_all(&emotes).unwrap();
        fs::create_dir_all(&badges).unwrap();
        let leftover = format!("a.png.{}.tmp", "0123456789abcdef0123456789abcdef");
        fs::write(emotes.join(&leftover), b"half").unwrap();
        fs::write(badges.join(&leftover), b"half").unwrap();
        fs::write(emotes.join("a.png"), b"png").unwrap();
        fs::write(emotes.join("b.tmp"), b"other").unwrap();
        fs::write(emotes.join(format!("c.png.{}.tmp", "0123456789ABCDEF0123456789ABCDEF")), b"other").unwrap();
        fs::write(emotes.join(format!("d.png.{}.tmp", "0123456789abcdef")), b"other").unwrap();

        assert_eq!(sweep_orphan_temp_files(&root), 2);

        let mut left: Vec<String> = temp_files_in(&emotes);
        left.extend(temp_files_in(&badges));
        left.sort();
        assert_eq!(left.len(), 3, "{left:?}");
        assert!(emotes.join("a.png").exists());
        assert!(emotes.join("b.tmp").exists());
        assert!(!emotes.join(&leftover).exists());
        assert!(!badges.join(&leftover).exists());
        assert_eq!(sweep_orphan_temp_files(&root), 0);
        let _ = fs::remove_dir_all(&root);
    }

    /// An entry for `id` that names `path`, in the emote cache.
    fn entry_with_file(id: &str, path: &Path) -> UniversalCacheEntry {
        let mut entry = file_entry(CacheType::Emote, 0);
        entry.id = id.into();
        entry.data = serde_json::json!({ "local_path": path.to_string_lossy() });
        entry
    }

    /// A cache root with an `emotes` folder that holds `names`, each with its
    /// own name as content. Returns the root and the paths of the files.
    fn emote_files(name: &str, names: &[&str]) -> (PathBuf, Vec<PathBuf>) {
        let root = scratch_dir(name);
        let type_dir = root.join("emotes");
        fs::create_dir_all(&type_dir).unwrap();
        let paths = names
            .iter()
            .map(|n| {
                let path = type_dir.join(n);
                fs::write(&path, n.as_bytes()).unwrap();
                path
            })
            .collect();
        (root, paths)
    }

    /// The deletion a manifest flush performs once its write has succeeded.
    fn flush_deletes(pending: &Mutex<Vec<PendingDelete>>, memory: &RwLock<UniversalCacheManifest>) {
        delete_unnamed_files(&take_pending(pending), memory);
    }

    #[test]
    fn an_entry_that_replaces_another_deletes_the_old_file_and_the_same_file_stays() {
        let (root, files) = emote_files("replaced", &["x-aaaaaaaa.png", "x-bbbbbbbb.png"]);
        let (old, new) = (&files[0], &files[1]);
        let memory = RwLock::new(UniversalCacheManifest::default());
        let pending = Mutex::new(Vec::new());

        let first = store_entries(&memory, &root, &pending, vec![entry_with_file("file:x", old)]).unwrap();
        assert_eq!((first.stored, first.missing.len()), (1, 0));
        store_entries(&memory, &root, &pending, vec![entry_with_file("file:x", old)]).unwrap();
        flush_deletes(&pending, &memory);
        assert!(old.exists(), "the same path twice is kept");

        store_entries(&memory, &root, &pending, vec![entry_with_file("file:x", new)]).unwrap();
        assert!(old.exists(), "the replaced file stays until the manifest without it is written");
        flush_deletes(&pending, &memory);
        assert!(!old.exists(), "the replaced file is deleted");
        assert!(new.exists());
        let guard = memory.read().unwrap();
        assert_eq!(guard.entries.len(), 1);
        assert_eq!(guard.entries["file:x"].data["local_path"].as_str(), new.to_str());
        drop(guard);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_batch_that_replaces_entries_deletes_their_old_files() {
        let (root, files) = emote_files(
            "batch-replaced",
            &["a-old.png", "a-new.png", "b-old.png", "b-new.png", "c-same.png", "d-one.png", "d-two.png"],
        );
        let memory = RwLock::new(UniversalCacheManifest::default());
        let pending = Mutex::new(Vec::new());
        let start = vec![
            entry_with_file("file:a", &files[0]),
            entry_with_file("file:b", &files[2]),
            entry_with_file("file:c", &files[4]),
        ];
        store_entries(&memory, &root, &pending, start).unwrap();

        let batch = vec![
            entry_with_file("file:a", &files[1]),
            entry_with_file("file:b", &files[3]),
            entry_with_file("file:c", &files[4]),
            // One id twice in a batch: the first file is replaced by the second.
            entry_with_file("file:d", &files[5]),
            entry_with_file("file:d", &files[6]),
        ];
        let out = store_entries(&memory, &root, &pending, batch).unwrap();
        assert_eq!(out.stored, 5);
        flush_deletes(&pending, &memory);
        let left: Vec<bool> = files.iter().map(|f| f.exists()).collect();
        assert_eq!(left, [false, true, false, true, true, false, true]);
        assert_eq!(memory.read().unwrap().entries.len(), 4);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn an_entry_whose_file_is_missing_is_refused() {
        let (root, files) = emote_files("missing-file", &["real-aaaaaaaa.png"]);
        let ghost = root.join("emotes").join("ghost-aaaaaaaa.png");
        let memory = RwLock::new(UniversalCacheManifest::default());
        let pending = Mutex::new(Vec::new());

        let out = store_entries(
            &memory,
            &root,
            &pending,
            vec![entry_with_file("file:ghost", &ghost), entry_with_file("file:real", &files[0])],
        )
        .unwrap();
        assert_eq!(out.stored, 1);
        assert_eq!(out.missing, vec![ghost.to_string_lossy().to_string()]);
        assert_eq!(out.refused, vec!["file:ghost".to_string()], "the refused ids are reported");
        {
            let guard = memory.read().unwrap();
            assert!(guard.entries.contains_key("file:real"));
            assert!(!guard.entries.contains_key("file:ghost"));
        }

        // A refused replacement leaves the stored entry and its file alone.
        let out = store_entries(&memory, &root, &pending, vec![entry_with_file("file:real", &ghost)]).unwrap();
        assert_eq!((out.stored, out.missing.len()), (0, 1));
        flush_deletes(&pending, &memory);
        assert!(files[0].exists());
        assert_eq!(
            memory.read().unwrap().entries["file:real"].data["local_path"].as_str(),
            files[0].to_str()
        );

        // An entry that names no file is not a file entry and is stored.
        let mut plain = file_entry(CacheType::Emote, 0);
        plain.id = "meta".into();
        plain.data = serde_json::json!({ "name": "x" });
        assert_eq!(store_entries(&memory, &root, &pending, vec![plain]).unwrap().stored, 1);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_path_outside_the_type_folder_is_never_deleted() {
        let root = scratch_dir("outside");
        let type_dir = root.join("emotes");
        let other_dir = root.join("elsewhere");
        fs::create_dir_all(&type_dir).unwrap();
        fs::create_dir_all(&other_dir).unwrap();
        let outside = other_dir.join("keep.png");
        fs::write(&outside, b"keep").unwrap();

        let resolved = resolve_path(&type_dir).expect("the type folder resolves");
        remove_cached_file(&resolved, &outside);
        assert!(outside.exists());
        remove_cached_file(&resolved, &type_dir.join("..").join("elsewhere").join("keep.png"));
        assert!(outside.exists());
        remove_cached_file(&resolved, &type_dir);
        assert!(type_dir.exists());
        let inside = type_dir.join("drop-aaaaaaaa.png");
        fs::write(&inside, b"drop").unwrap();
        remove_cached_file(&resolved, &inside);
        assert!(!inside.exists(), "a file inside the folder is deleted");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_long_queue_of_deletions_is_checked_in_chunks_and_keeps_what_an_entry_names() {
        let names: Vec<String> = (0..DELETE_CHUNK * 2 + 5).map(|i| format!("f{i}-aaaaaaaa.png")).collect();
        let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let (root, files) = emote_files("delete-chunks", &name_refs);
        let type_dir = root.join("emotes");
        let kept = [0, DELETE_CHUNK - 1, DELETE_CHUNK, DELETE_CHUNK * 2 + 4];
        let memory = RwLock::new(UniversalCacheManifest::default());
        {
            let mut guard = memory.write().unwrap();
            for &i in &kept {
                let id = format!("file:{i}");
                guard.entries.insert(id.clone(), entry_with_file(&id, &files[i]));
            }
        }
        let queue: Vec<PendingDelete> = files.iter().map(|f| (type_dir.clone(), f.clone())).collect();

        delete_unnamed_files(&queue, &memory);

        for (i, file) in files.iter().enumerate() {
            assert_eq!(file.exists(), kept.contains(&i), "file {i}");
        }
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn expiry_removes_only_the_expired_entrys_file() {
        let (root, files) = emote_files("expired", &["old-aaaaaaaa.png", "fresh-aaaaaaaa.png"]);
        let outside = root.join("keep.png");
        fs::write(&outside, b"keep").unwrap();

        let entry = |id: &str, age_days: u64, data: serde_json::Value| {
            let mut e = file_entry(CacheType::Emote, age_days);
            e.id = id.into();
            e.data = data;
            (id.to_string(), e)
        };
        let mut manifest = UniversalCacheManifest {
            version: 1,
            last_sync: None,
            entries: [
                entry("file:old", 30, serde_json::json!({ "local_path": files[0].to_string_lossy() })),
                entry("file:fresh", 1, serde_json::json!({ "local_path": files[1].to_string_lossy() })),
                entry("file:outside", 30, serde_json::json!({ "local_path": outside.to_string_lossy() })),
                entry("file:none", 30, serde_json::json!({ "name": "x" })),
            ]
            .into_iter()
            .collect(),
        };

        let dropped = drop_expired(&mut manifest, super::get_current_timestamp());
        let mut dropped_ids: Vec<&str> = dropped.iter().map(|e| e.id.as_str()).collect();
        dropped_ids.sort();
        assert_eq!(dropped_ids, ["file:none", "file:old", "file:outside"]);
        assert_eq!(manifest.entries.keys().collect::<Vec<_>>(), ["file:fresh"]);

        let pending = Mutex::new(Vec::new());
        queue_entry_files(&pending, &root, &dropped);
        assert!(files[0].exists(), "the expired entry's file stays until the manifest without it is written");
        flush_deletes(&pending, &RwLock::new(manifest));
        assert!(!files[0].exists(), "the expired entry's file is deleted");
        assert!(files[1].exists(), "a fresh entry's file stays");
        assert!(outside.exists(), "a path outside the type folder is never deleted");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn cached_file_names_are_valid_on_windows() {
        assert_eq!(safe_id("a:b?c."), "a_b_c_");
        assert_eq!(safe_id("con"), "_con");
        assert_eq!(safe_id("CON.txt"), "CON_txt");
        assert_eq!(safe_id("Lpt3"), "_Lpt3");
        assert_eq!(safe_id("com10"), "com10");
        assert_eq!(safe_id("a<b>c\"d\\e|f*g\u{1}h  "), "a_b_c_d_e_f_g_h");
        assert_eq!(safe_id("emote/x@1x"), "emote_x@1x");
        assert_eq!(safe_id("01H2XYZABC7TVEMOTEID"), "01H2XYZABC7TVEMOTEID");
    }

    #[test]
    fn a_long_or_dotted_id_gives_a_short_name_without_dots() {
        let long = "x".repeat(300);
        let name = content_addressed_file_name(&long, "https://cdn.example/a.webp", "webp");
        assert!(name.len() < 100, "{name}");
        assert_ne!(
            name,
            content_addressed_file_name(&format!("{long}y"), "https://cdn.example/a.webp", "webp"),
            "the hash covers the whole id"
        );
        assert!(!safe_id("a..b").contains(".."));
        assert_eq!(safe_id("a..b"), "a__b");
        let wide = "\u{1F600}".repeat(100);
        assert!(safe_id(&wide).len() <= 120 && safe_id(&wide).chars().count() <= 64);
    }

    #[test]
    fn a_sharing_violation_is_retried_like_a_refused_rename() {
        if !cfg!(windows) {
            return;
        }
        let dir = scratch_dir("sharing-violation");
        let dest = dir.join("e-12345678.webp");
        for code in [32, 33] {
            let mut calls = 0;
            write_atomic_with(&dest, b"landed", |from, to| {
                calls += 1;
                if calls < 3 {
                    Err(Error::from_raw_os_error(code))
                } else {
                    fs::rename(from, to)
                }
            })
            .unwrap();
            assert_eq!(calls, 3, "os error {code} is retried");
        }
        assert_eq!(fs::read(&dest).unwrap(), b"landed");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_gif_that_mentions_a_body_tag_is_still_an_image() {
        let mut gif = b"GIF89a\x01\x00\x01\x00\x00\x00\x00".to_vec();
        gif.extend_from_slice(b"!\xFE\x0d<body bgcolor>\x00;");
        assert!(looks_like_html(&gif), "the comment block reads as a page to the markup check");
        assert!(refuse_html(&gif).is_ok(), "the GIF magic wins");
        assert_eq!(detect_image_ext(&gif, None), "gif");
    }

    /// A badge entry from the badgebase source, added on `date`.
    fn badgebase_entry(id: &str, date: &str, position: Option<u32>) -> UniversalCacheEntry {
        let mut entry = file_entry(CacheType::Badge, 0);
        entry.id = id.into();
        entry.data = serde_json::json!({ "date_added": date });
        entry.metadata.source = "badgebase".into();
        entry.position = position;
        entry
    }

    /// A manifest that already holds a stored file entry, as a manifest change
    /// finds it after a store landed first.
    fn memory_with_stored_file(name: &str) -> (PathBuf, RwLock<UniversalCacheManifest>, PathBuf) {
        let (root, files) = emote_files(name, &["stored-aaaaaaaa.png"]);
        let memory = RwLock::new(UniversalCacheManifest::default());
        let pending = Mutex::new(Vec::new());
        store_entries(&memory, &root, &pending, vec![entry_with_file("file:stored", &files[0])]).unwrap();
        (root, memory, files[0].clone())
    }

    fn stored_path(memory: &RwLock<UniversalCacheManifest>) -> Option<String> {
        let guard = memory.read().unwrap();
        guard.entries.get("file:stored").and_then(|e| e.data["local_path"].as_str().map(str::to_string))
    }

    #[test]
    fn a_remote_merge_keeps_an_entry_stored_during_it_and_skips_an_entry_with_a_missing_file() {
        let (root, files) = emote_files("merge-in-place", &["stored-aaaaaaaa.png"]);
        let memory = RwLock::new(UniversalCacheManifest::default());
        let pending = Mutex::new(Vec::new());
        let mut remote = UniversalCacheManifest::default();
        remote.entries.insert("new".into(), badgebase_entry("new", "1 March 2026", None));
        let mut ghost = badgebase_entry("ghost", "1 March 2026", None);
        let ghost_path = root.join("emotes").join("ghost-aaaaaaaa.png");
        ghost.data = serde_json::json!({ "local_path": ghost_path.to_string_lossy() });
        remote.entries.insert("ghost".into(), ghost);

        // A store lands after the merge has read the manifest and before it writes.
        let needs_positions = merge_remote_into_with(&memory, remote, || {
            store_entries(&memory, &root, &pending, vec![entry_with_file("file:stored", &files[0])]).unwrap();
        })
        .unwrap();
        assert!(needs_positions, "a merged badge has no position yet");
        assert_eq!(stored_path(&memory).as_deref(), files[0].to_str(), "the stored entry is kept");
        let guard = memory.read().unwrap();
        assert!(guard.entries.contains_key("new"), "the merge went on past the skipped entry");
        assert!(!guard.entries.contains_key("ghost"));
        assert!(guard.last_sync.is_some());
        drop(guard);
        let _ = fs::remove_dir_all(&root);
    }

    /// A manifest with two unplaced badgebase badges, "recent" and "old".
    fn memory_with_two_badges() -> RwLock<UniversalCacheManifest> {
        let memory = RwLock::new(UniversalCacheManifest::default());
        {
            let mut guard = memory.write().unwrap();
            guard.entries.insert("old".into(), badgebase_entry("old", "March 2018", None));
            guard.entries.insert("recent".into(), badgebase_entry("recent", "2 September 2026", None));
        }
        memory
    }

    #[test]
    fn assigning_positions_keeps_an_entry_stored_during_it() {
        let (root, files) = emote_files("positions-in-place", &["stored-aaaaaaaa.png"]);
        let memory = memory_with_two_badges();
        let pending = Mutex::new(Vec::new());

        // A store lands after the order is worked out and before it is applied.
        let count = assign_positions_in_with(&memory, || {
            store_entries(&memory, &root, &pending, vec![entry_with_file("file:stored", &files[0])]).unwrap();
        })
        .unwrap();
        assert_eq!(count, 2);
        assert_eq!(stored_path(&memory).as_deref(), files[0].to_str(), "the stored entry is kept");
        let guard = memory.read().unwrap();
        assert_eq!(guard.entries["recent"].position, Some(0));
        assert_eq!(guard.entries["old"].position, Some(1));
        drop(guard);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn assigning_positions_skips_a_badge_removed_after_the_order_was_worked_out() {
        let memory = memory_with_two_badges();

        let count = assign_positions_in_with(&memory, || {
            memory.write().unwrap().entries.remove("recent");
        })
        .unwrap();
        assert_eq!(count, 1);
        let guard = memory.read().unwrap();
        assert!(!guard.entries.contains_key("recent"), "the removed badge is not brought back");
        assert_eq!(guard.entries["old"].position, Some(1));
    }

    #[test]
    fn stamping_the_last_sync_keeps_the_stored_file_entry() {
        let (root, memory, stored) = memory_with_stored_file("sync-in-place");
        stamp_last_sync_in(&memory, 1234).unwrap();
        assert_eq!(stored_path(&memory).as_deref(), stored.to_str(), "the stored entry is kept");
        assert_eq!(memory.read().unwrap().last_sync, Some(1234));
        let _ = fs::remove_dir_all(&root);
    }

    /// A manifest of `count` emote entries whose ids carry `version`.
    fn manifest_of(version: u32, count: usize) -> UniversalCacheManifest {
        let mut manifest = UniversalCacheManifest { version, last_sync: None, entries: HashMap::new() };
        for i in 0..count {
            let mut entry = file_entry(CacheType::Emote, 0);
            entry.id = format!("file:{version}-{i}");
            manifest.entries.insert(entry.id.clone(), entry);
        }
        manifest
    }

    #[test]
    fn flushes_take_turns_and_the_manifest_always_parses() {
        let dir = scratch_dir("flush-turns");
        let path = dir.join("manifest.json");
        let memories = [RwLock::new(manifest_of(1, 300)), RwLock::new(manifest_of(2, 500))];
        let pendings = [Mutex::new(Vec::new()), Mutex::new(Vec::new())];
        let barrier = std::sync::Barrier::new(memories.len());
        std::thread::scope(|scope| {
            for (memory, pending) in memories.iter().zip(&pendings) {
                let (path, barrier) = (&path, &barrier);
                scope.spawn(move || {
                    barrier.wait();
                    for _ in 0..40 {
                        // The other flush has finished its write: what is on disk is
                        // whole.
                        let on_disk_is_whole = || {
                            if path.exists() {
                                let json = fs::read_to_string(path).unwrap();
                                assert!(parse_manifest(&json).is_ok(), "the manifest on disk parses");
                            }
                        };
                        flush_manifest_with(memory, pending, path, on_disk_is_whole).expect("the flush writes");
                    }
                });
            }
        });
        let json = fs::read_to_string(&path).unwrap();
        let (manifest, _) = parse_manifest(&json).unwrap();
        assert!(manifest.entries.len() == 300 || manifest.entries.len() == 500);
        assert!(temp_files_in(&dir).is_empty(), "{:?}", temp_files_in(&dir));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_clear_waits_for_a_flush_in_progress_and_leaves_no_manifest() {
        let base = scratch_dir("clear-waits");
        let cache_dir = base.join("cache");
        fs::create_dir_all(&cache_dir).unwrap();
        let path = cache_dir.join("manifest.json");
        let memory = RwLock::new(manifest_of(1, 50));
        let pending = Mutex::new(Vec::new());
        let (paused_tx, paused_rx) = std::sync::mpsc::channel::<()>();
        let (resume_tx, resume_rx) = std::sync::mpsc::channel::<()>();
        let (clearing_tx, clearing_rx) = std::sync::mpsc::channel::<()>();
        std::thread::scope(|scope| {
            let (memory, pending, path, cache_dir) = (&memory, &pending, &path, &cache_dir);
            let flush = scope.spawn(move || {
                flush_manifest_with(memory, pending, path, || {
                    paused_tx.send(()).unwrap();
                    resume_rx.recv().unwrap();
                })
                .expect("the flush writes");
            });
            // The flush is paused between encoding and writing.
            paused_rx.recv().unwrap();
            let clear = scope.spawn(move || {
                clearing_tx.send(()).unwrap();
                clear_cache_in(cache_dir, memory, pending).expect("the clear works");
            });
            clearing_rx.recv().unwrap();
            std::thread::sleep(std::time::Duration::from_millis(200));
            resume_tx.send(()).unwrap();
            flush.join().unwrap();
            clear.join().unwrap();
        });
        let left = match fs::read_to_string(&path) {
            Ok(json) => parse_manifest(&json).unwrap().0.entries.len(),
            Err(_) => 0,
        };
        assert_eq!(left, 0, "the flush did not write its manifest back after the clear");
        assert!(memory.read().unwrap().entries.is_empty());
        assert!(cache_dir.exists(), "the folder is recreated");
        let _ = fs::remove_dir_all(&base);
    }

    fn file_entry(cache_type: CacheType, age_days: u64) -> UniversalCacheEntry {
        UniversalCacheEntry {
            id: "file:x".into(),
            cache_type,
            data: serde_json::json!({ "local_path": "C:/cache/x.png" }),
            metadata: CacheMetadata {
                timestamp: super::get_current_timestamp() - age_days * 24 * 60 * 60,
                expiry_days: 7,
                source: "universal_file".into(),
                version: 1,
            },
            position: None,
        }
    }

    #[test]
    fn entries_whose_file_is_gone_are_found() {
        let dir = std::env::temp_dir().join(format!("sn-cache-missing-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("twitch-moderator_1.png"), b"png").unwrap();
        let elsewhere = dir.join("elsewhere.png");
        fs::write(&elsewhere, b"png").unwrap();

        let entry = |id: &str, cache_type: CacheType, path: PathBuf| {
            let mut e = file_entry(cache_type, 1);
            e.id = id.into();
            e.data = serde_json::json!({ "local_path": path.to_string_lossy() });
            (id.to_string(), e)
        };
        let entries: HashMap<String, UniversalCacheEntry> = [
            entry("file:twitch-moderator/1", CacheType::Badge, dir.join("twitch-moderator_1.png")),
            entry("file:twitch-vip/1", CacheType::Badge, dir.join("twitch-vip_1.png")),
            // A copied profile: a path outside the folder, checked on its own.
            entry("file:twitch-old/1", CacheType::Badge, PathBuf::from("C:/Users/someone-else/x.png")),
            entry("file:twitch-moved/1", CacheType::Badge, elsewhere.clone()),
            // Another type's missing file is not this listing's business.
            entry("file:emote-gone", CacheType::Emote, dir.join("gone.webp")),
        ]
        .into_iter()
        .collect();

        let listing: HashSet<OsString> = fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name()).collect();
        let mut from_listing = missing_file_keys(&entries, &CacheType::Badge, &dir, Some(&listing));
        from_listing.sort();
        assert_eq!(from_listing, vec!["file:twitch-old/1".to_string(), "file:twitch-vip/1".to_string()]);

        // An unreadable folder falls back to a stat per file, same answer.
        let mut by_stat = missing_file_keys(&entries, &CacheType::Badge, &dir, None);
        by_stat.sort();
        assert_eq!(by_stat, from_listing);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_cosmetic_files_go_stale_by_age() {
        let old_badge = file_entry(CacheType::Badge, 30);
        assert_eq!(usable_file_path(&CacheType::Badge, &old_badge), Some("C:/cache/x.png"));
        let old_paint = file_entry(CacheType::Cosmetic, 30);
        assert_eq!(usable_file_path(&CacheType::Cosmetic, &old_paint), None);
        let new_paint = file_entry(CacheType::Cosmetic, 1);
        assert!(usable_file_path(&CacheType::Cosmetic, &new_paint).is_some());
        assert_eq!(usable_file_path(&CacheType::Emote, &old_badge), None, "the kind must match");
    }

    fn webp_head(fourcc: &[u8; 4], flags: u8) -> [u8; 21] {
        let mut head = [0u8; 21];
        head[0..4].copy_from_slice(b"RIFF");
        head[8..12].copy_from_slice(b"WEBP");
        head[12..16].copy_from_slice(fourcc);
        head[20] = flags;
        head
    }

    #[test]
    fn webp_animation_comes_from_the_vp8x_flag() {
        assert_eq!(webp_is_animated(&webp_head(b"VP8X", 0x12)), Some(true));
        assert_eq!(webp_is_animated(&webp_head(b"VP8X", 0x10)), Some(false));
        // A simple-format WebP cannot animate, whatever its bitstream bytes are.
        assert_eq!(webp_is_animated(&webp_head(b"VP8 ", 0x02)), Some(false));
        assert_eq!(webp_is_animated(&webp_head(b"VP8L", 0x02)), Some(false));
        assert_eq!(webp_is_animated(b"GIF89a"), None);
    }

    fn cached(cache_type: CacheType, url: &str) -> UniversalCacheEntry {
        UniversalCacheEntry {
            id: "01K6ZYS5X633HHK0K3JVDK4FT0".into(),
            cache_type,
            data: serde_json::json!({ "url": url }),
            metadata: CacheMetadata {
                timestamp: 0,
                expiry_days: 7,
                source: "universal_file".into(),
                version: 1,
            },
            position: None,
        }
    }

    #[test]
    fn only_7tv_paint_stills_are_purged() {
        let paint = "https://cdn.7tv.app/paint/01K6ZYS5X6TF6EGK85Y9ZZR8YC/layer/01K6ZYS5X633HHK0K3JVDK4FT0";
        assert!(is_still_paint_entry(&cached(CacheType::Cosmetic, &format!("{paint}/1x_static.webp"))));
        // The animated URL with no readable file is left alone.
        assert!(!is_still_paint_entry(&cached(CacheType::Cosmetic, &format!("{paint}/1x.webp"))));
        // A 7TV badge shares the cosmetic cache and is never touched.
        assert!(!is_still_paint_entry(&cached(
            CacheType::Cosmetic,
            "https://cdn.7tv.app/badge/01GFR6GNB80001V63R0CCB85YD/1x_static.webp",
        )));
        assert!(!is_still_paint_entry(&cached(CacheType::Emote, &format!("{paint}/1x_static.webp"))));
    }

    #[test]
    fn badge_dates_order_by_calendar_not_text() {
        let mut dates = vec![
            "4 February 2026",
            "30 March 2026",
            "March 2018",
            "2 September 2026",
            "16 September 2025",
            "garbage",
            "13 August 2026",
        ];
        dates.sort_by(|a, b| badge_date_key(b).cmp(&badge_date_key(a)));
        assert_eq!(
            dates,
            [
                "2 September 2026",
                "13 August 2026",
                "30 March 2026",
                "4 February 2026",
                "16 September 2025",
                "March 2018",
                "garbage",
            ]
        );
    }

    #[test]
    fn month_only_sorts_after_that_months_dated_days() {
        assert!(badge_date_key("1 March 2018") > badge_date_key("March 2018"));
        assert!(badge_date_key("March 2018") > badge_date_key("28 February 2018"));
    }

    #[test]
    fn an_unknown_cache_type_is_kept_aside_and_written_back() {
        let json = r#"{"version":1,"last_sync":null,"entries":{
            "a":{"id":"a","cache_type":"emote","data":{},"metadata":META},
            "b":{"id":"b","cache_type":"futuretype","data":{},"metadata":META}}}"#
            .replace("META", r#"{"timestamp":1,"expiry_days":7,"source":"universal_file","version":1}"#);
        let (manifest, foreign) = parse_manifest(&json).unwrap();
        assert_eq!(manifest.entries.len(), 1);
        assert!(manifest.entries.contains_key("a"));
        assert_eq!(foreign.len(), 1);
        assert_eq!(foreign[0].0, "b");
        let bytes = encode_with(&manifest, &foreign).unwrap();
        let out: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(out["entries"]["a"]["cache_type"], "emote");
        assert_eq!(out["entries"]["b"]["cache_type"], "futuretype");
        assert_eq!(out["entries"].as_object().unwrap().len(), 2);
        // Read back: the unknown entry is still set aside, the known one still known.
        let (again, foreign_again) = parse_manifest(std::str::from_utf8(&bytes).unwrap()).unwrap();
        assert_eq!(again.entries.len(), 1);
        assert_eq!(foreign_again.len(), 1);
    }

    #[test]
    fn this_builds_entry_wins_over_a_set_aside_one_with_its_key() {
        let manifest = UniversalCacheManifest {
            version: 1,
            last_sync: None,
            entries: [("a".to_string(), file_entry(CacheType::Emote, 0))].into_iter().collect(),
        };
        let foreign = vec![("a".to_string(), serde_json::json!({ "cache_type": "futuretype" }))];
        let out: serde_json::Value = serde_json::from_slice(&encode_with(&manifest, &foreign).unwrap()).unwrap();
        assert_eq!(out["entries"].as_object().unwrap().len(), 1);
        assert_eq!(out["entries"]["a"]["cache_type"], "emote");
    }

    #[test]
    fn svg_art_is_typed_svg_by_its_content_or_its_content_type() {
        assert_eq!(detect_image_ext(b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>", None), "svg");
        assert_eq!(detect_image_ext(b"\xEF\xBB\xBF  \n<svg width=\"1\"/>", None), "svg");
        assert_eq!(
            detect_image_ext(b"<?xml version=\"1.0\"?>\n<svg width=\"1\"/>", None),
            "svg"
        );
        assert_eq!(detect_image_ext(b"<!-- made by hand --><svg/>", None), "svg");
        assert_eq!(detect_image_ext(b"<!DOCTYPE svg PUBLIC \"x\"><svg/>", None), "svg");
        // An XML file that holds no svg element within the first 1024 bytes is not art.
        let mut far = b"<?xml version=\"1.0\"?>".to_vec();
        far.resize(far.len() + 1100, b' ');
        far.extend_from_slice(b"<svg/>");
        assert_eq!(detect_image_ext(&far, None), "bin");
        assert_eq!(detect_image_ext(b"opaque bytes", Some("image/svg+xml; charset=utf-8")), "svg");
        // A PNG stays a PNG, even when the server mislabels it.
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend_from_slice(&[0u8; 16]);
        assert_eq!(detect_image_ext(&png, None), "png");
        assert_eq!(detect_image_ext(&png, Some("image/svg+xml")), "png");
    }
}

#[cfg(test)]
mod streamnook_art_tests {
    use super::{content_range_start, fetch_resumable, is_streamnook_art_url, streamnook_art_file_name};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A one-file HTTP server that closes every connection after `cut_after`
    /// body bytes, the way a throttled link drops a long transfer.
    async fn serve(body: Vec<u8>, cut_after: usize, honor_range: bool, status: u16) -> (String, Arc<AtomicUsize>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/badges/x.webp", listener.local_addr().unwrap());
        let connections = Arc::new(AtomicUsize::new(0));
        let counter = connections.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else { return };
                counter.fetch_add(1, Ordering::SeqCst);
                let body = body.clone();
                tokio::spawn(async move {
                    let mut head = Vec::new();
                    let mut buf = [0u8; 1024];
                    while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                        match socket.read(&mut buf).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => head.extend_from_slice(&buf[..n]),
                        }
                    }
                    let head = String::from_utf8_lossy(&head).to_ascii_lowercase();
                    let start = head
                        .lines()
                        .find_map(|l| l.strip_prefix("range: bytes="))
                        .and_then(|r| r.trim_end_matches('-').trim().trim_end_matches('-').parse::<usize>().ok())
                        .filter(|_| honor_range);
                    let response_head = if status != 200 {
                        format!("HTTP/1.1 {status} Nope\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    } else if let Some(start) = start {
                        format!(
                            "HTTP/1.1 206 Partial Content\r\nContent-Type: image/webp\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nConnection: close\r\n\r\n",
                            body.len() - start,
                            start,
                            body.len() - 1,
                            body.len()
                        )
                    } else {
                        format!("HTTP/1.1 200 OK\r\nContent-Type: image/webp\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len())
                    };
                    let _ = socket.write_all(response_head.as_bytes()).await;
                    if status == 200 {
                        let from = start.unwrap_or(0);
                        let to = (from + cut_after).min(body.len());
                        let _ = socket.write_all(&body[from..to]).await;
                    }
                    let _ = socket.shutdown().await;
                });
            }
        });
        (url, connections)
    }

    fn client() -> reqwest::Client {
        reqwest::Client::builder().no_proxy().build().unwrap()
    }

    fn pattern(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i * 7 % 251) as u8).collect()
    }

    #[tokio::test]
    async fn a_cut_transfer_resumes_to_the_whole_file() {
        let body = pattern(100_000);
        let (url, connections) = serve(body.clone(), 16_384, true, 200).await;
        let (got, content_type) = fetch_resumable(&client(), &url, 1 << 20, 8, Duration::from_millis(1))
            .await
            .expect("resumed to the end");
        assert_eq!(got, body);
        assert_eq!(content_type.as_deref(), Some("image/webp"));
        assert_eq!(connections.load(Ordering::SeqCst), 7, "16 KB per connection for 100 KB");
    }

    #[tokio::test]
    async fn an_uncut_transfer_takes_one_connection() {
        let body = pattern(50_000);
        let (url, connections) = serve(body.clone(), usize::MAX, true, 200).await;
        let (got, _) = fetch_resumable(&client(), &url, 1 << 20, 8, Duration::from_millis(1)).await.unwrap();
        assert_eq!(got, body);
        assert_eq!(connections.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_server_that_ignores_range_gives_up_after_its_attempts() {
        let (url, connections) = serve(pattern(100_000), 16_384, false, 200).await;
        let result = fetch_resumable(&client(), &url, 1 << 20, 3, Duration::from_millis(1)).await;
        assert!(result.is_err());
        assert_eq!(connections.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn a_missing_file_is_not_retried() {
        let (url, connections) = serve(Vec::new(), 0, true, 404).await;
        let result = fetch_resumable(&client(), &url, 1 << 20, 8, Duration::from_millis(1)).await;
        assert!(result.unwrap_err().to_string().contains("404"));
        assert_eq!(connections.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn an_oversized_file_is_refused() {
        let (url, _) = serve(pattern(10_000), usize::MAX, true, 200).await;
        let result = fetch_resumable(&client(), &url, 4_096, 8, Duration::from_millis(1)).await;
        assert!(result.unwrap_err().to_string().contains("larger than"));
    }

    #[test]
    fn only_cdn_streamnook_art_is_fetched() {
        assert!(is_streamnook_art_url("https://cdn.streamnook.app/badges/kindred.webp?v=2"));
        assert!(!is_streamnook_art_url("https://cdn.streamnook.app/"));
        assert!(!is_streamnook_art_url("http://cdn.streamnook.app/badges/kindred.webp"));
        assert!(!is_streamnook_art_url("https://cdn.streamnook.app.evil.test/badges/kindred.webp"));
        assert!(!is_streamnook_art_url("https://static-cdn.jtvnw.net/badges/v1/x/3"));
    }

    #[test]
    fn art_file_names_are_readable_and_versioned() {
        let v2 = streamnook_art_file_name("https://cdn.streamnook.app/badges/streamnook-mobius.webp?v=2", "webp");
        let v3 = streamnook_art_file_name("https://cdn.streamnook.app/badges/streamnook-mobius.webp?v=3", "webp");
        assert!(v2.starts_with("streamnook-mobius-") && v2.ends_with(".webp"), "{v2}");
        assert_ne!(v2, v3, "a re-upload under a new version gets its own file");
        let odd = streamnook_art_file_name("https://cdn.streamnook.app/badges/M%C3%B6bius:%20x.webp", "webp");
        assert!(odd.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)), "{odd}");
    }

    #[test]
    fn content_range_reads_start_and_total() {
        assert_eq!(content_range_start("bytes 16384-99999/100000"), Some((16384, Some(100000))));
        assert_eq!(content_range_start("bytes 0-9/*"), Some((0, None)));
        assert_eq!(content_range_start("items 0-9/10"), None);
    }
}
