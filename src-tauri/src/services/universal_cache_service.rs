use anyhow::{Context, Result};
use log::debug;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex as StdMutex;
use std::sync::RwLock as StdRwLock;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex as TokioMutex;

use crate::services::cache_service::get_cache_dir;

// Global mutex to prevent concurrent manifest writes (for sync operations)
static MANIFEST_LOCK: Lazy<StdMutex<()>> = Lazy::new(|| StdMutex::new(()));

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
                // `rt_stall` freezes. Serialize under the read lock rather than
                // cloning the whole entry map first (the clone doubled the
                // manifest's memory for the duration of every flush), then drop
                // the guard BEFORE touching the disk so writers only wait for
                // the encode, never for the file write.
                let flushed = tokio::task::spawn_blocking(|| {
                    let json = {
                        let guard = MANIFEST_MEMORY
                            .read()
                            .map_err(|_| anyhow::anyhow!("manifest memory lock poisoned"))?;
                        serde_json::to_vec(&*guard)?
                    };
                    write_manifest_bytes(&json)
                })
                .await;
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
    // If expiry_days is 0, never expire
    if metadata.expiry_days == 0 {
        return false;
    }

    let current_time = get_current_timestamp();
    let expiry_seconds = metadata.expiry_days as u64 * 24 * 60 * 60;
    current_time > metadata.timestamp + expiry_seconds
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

    match serde_json::from_str::<UniversalCacheManifest>(&json) {
        Ok(manifest) => Ok(manifest),
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
    // Compact, not pretty: nothing reads this file by eye, and the indentation
    // was ~22% of a 22 MB file rewritten many times per session.
    let json = serde_json::to_vec(manifest)?;
    write_manifest_bytes(&json)
}

/// Replace manifest.json with `json` atomically: write a sibling, then rename
/// over the original, so a crash mid-write can never leave a truncated manifest
/// behind (the load path treats parse failure as "start over", which would
/// silently forget every cached file). std::fs::rename replaces an existing
/// destination on Windows (MoveFileExW with replace) and on Unix.
fn write_manifest_bytes(json: &[u8]) -> Result<()> {
    use std::io::Write;
    let manifest_path = get_universal_cache_dir()?.join("manifest.json");
    let tmp_path = manifest_path.with_extension("json.tmp");
    {
        let mut file = fs::File::create(&tmp_path).context("Failed to create manifest temp file")?;
        file.write_all(json).context("Failed to write manifest file")?;
        // Flush the data before the rename lands in the journal: without it a
        // power loss can leave a correctly named, zero-filled manifest.
        file.sync_all().context("Failed to sync manifest file")?;
    }
    fs::rename(&tmp_path, &manifest_path).context("Failed to replace manifest file")?;
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
pub fn upsert_cached_entry(entry: UniversalCacheEntry) -> Result<()> {
    let _lock = MANIFEST_LOCK.lock().unwrap();
    {
        let mut guard = MANIFEST_MEMORY
            .write()
            .map_err(|_| anyhow::anyhow!("manifest memory lock poisoned"))?;
        guard.entries.insert(entry.id.clone(), entry);
    }
    MANIFEST_DIRTY.store(true, Ordering::Release);
    ensure_flush_task();
    Ok(())
}

/// Synchronous flush of the in-memory manifest to disk, if dirty. For shutdown
/// paths that skip or precede the debounced flush task.
pub fn flush_manifest_now() -> Result<()> {
    if !MANIFEST_DIRTY.swap(false, Ordering::AcqRel) {
        return Ok(());
    }
    let json = match MANIFEST_MEMORY.read() {
        Ok(g) => serde_json::to_vec(&*g),
        Err(_) => {
            MANIFEST_DIRTY.store(true, Ordering::Release);
            return Err(anyhow::anyhow!("manifest memory lock poisoned"));
        }
    };
    let json = match json {
        Ok(j) => j,
        Err(e) => {
            MANIFEST_DIRTY.store(true, Ordering::Release);
            return Err(e.into());
        }
    };
    if let Err(e) = write_manifest_bytes(&json) {
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

            let mut local_manifest = load_manifest()?;

            // Only add badgebase entries from remote if they are newer than local
            for (key, entry) in remote_manifest.entries {
                if entry.metadata.source == "badgebase" {
                    let should_replace = match local_manifest.entries.get(&key) {
                        // Relay enrichment is campaign-grounded and richer than
                        // the scrape, and the daily job always carries a newer
                        // timestamp, so it must never win on timestamp alone.
                        Some(local)
                            if local.metadata.source
                                == crate::commands::badge_metadata::ENRICHMENT_SOURCE =>
                        {
                            false
                        }
                        Some(local) => entry.metadata.timestamp > local.metadata.timestamp,
                        None => true, // New entry not present locally, always insert
                    };
                    if should_replace {
                        local_manifest.entries.insert(key, entry);
                    }
                }
            }

            // Update last_sync time
            local_manifest.last_sync = Some(get_current_timestamp());

            // Read what the caller still needs from the local copy BEFORE moving
            // it into the mirror, so the merge costs no second clone.
            let needs_positions = local_manifest
                .entries
                .iter()
                .filter(|(_, entry)| entry.metadata.source == "badgebase")
                .any(|(_, entry)| entry.position.is_none());

            save_manifest_owned(local_manifest)?;
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
pub async fn save_cached_items_batch(entries: Vec<UniversalCacheEntry>) -> Result<()> {
    if entries.is_empty() {
        return Ok(());
    }
    // Blocking pool for the same lock-hold reason as save_cached_item.
    tokio::task::spawn_blocking(move || -> Result<()> {
        let _lock = MANIFEST_LOCK.lock().unwrap();
        {
            let mut guard = MANIFEST_MEMORY
                .write()
                .map_err(|_| anyhow::anyhow!("manifest memory lock poisoned"))?;
            for entry in entries {
                guard.entries.insert(entry.id.clone(), entry);
            }
        }
        MANIFEST_DIRTY.store(true, Ordering::Release);
        ensure_flush_task();
        Ok(())
    })
    .await
    .context("save_cached_items_batch task panicked")?
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
    assign_badge_metadata_positions_impl()
}

/// Implementation of position assignment (assumes lock is held or not needed)
fn assign_badge_metadata_positions_impl() -> Result<usize> {
    let mut manifest = load_manifest()?;

    // Collect all badge entries from badge metadata source
    let mut metadata_entries: Vec<(String, UniversalCacheEntry)> = manifest
        .entries
        .iter()
        .filter(|(_, entry)| {
            entry.cache_type == CacheType::Badge && entry.metadata.source == "badgebase"
        })
        .map(|(id, entry)| (id.clone(), entry.clone()))
        .collect();

    // Sort by date (newest first), then by usage (highest first)
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
        b_date.cmp(&a_date).then(b_usage.cmp(&a_usage)).then(a.0.cmp(&b.0))
    });

    // Assign positions
    for (position, (id, mut entry)) in metadata_entries.into_iter().enumerate() {
        entry.position = Some(position as u32);
        manifest.entries.insert(id, entry);
    }

    let count = manifest
        .entries
        .iter()
        .filter(|(_, entry)| {
            entry.cache_type == CacheType::Badge
                && entry.metadata.source == "badgebase"
                && entry.position.is_some()
        })
        .count();

    save_manifest_owned(manifest)?;
    debug!(
        "[UniversalCache] Assigned positions to {} badge metadata entries",
        count
    );

    Ok(count)
}

/// Assign positions to badge metadata entries based on date and usage (public API)
pub async fn assign_badge_metadata_positions() -> Result<usize> {
    let _lock = ASYNC_MANIFEST_LOCK.lock().await;
    assign_badge_metadata_positions_impl()
}

/// Export manifest to a specific path for GitHub upload
pub fn export_manifest_for_github(output_path: PathBuf) -> Result<()> {
    let manifest = load_manifest()?;
    let json = serde_json::to_string_pretty(&manifest)?;
    fs::write(&output_path, json)?;
    debug!("[UniversalCache] Exported manifest to {:?}", output_path);
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

    // Update last sync time
    let mut manifest = load_manifest()?;
    manifest.last_sync = Some(get_current_timestamp());
    save_manifest_owned(manifest)?;

    debug!(
        "[UniversalCache] Sync complete. Synced {} items",
        synced_count
    );

    Ok(synced_count)
}

/// Clear expired entries from cache
pub fn cleanup_expired_entries() -> Result<usize> {
    let mut manifest = load_manifest()?;
    let initial_count = manifest.entries.len();

    manifest
        .entries
        .retain(|_, entry| !is_cache_expired(&entry.metadata));

    let removed_count = initial_count - manifest.entries.len();

    if removed_count > 0 {
        save_manifest_owned(manifest)?;
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

    if cache_dir.exists() {
        fs::remove_dir_all(&cache_dir).context("Failed to remove universal cache directory")?;
        fs::create_dir_all(&cache_dir).context("Failed to recreate universal cache directory")?;
    }

    // CRITICAL: drop the in-memory manifest mirror too. Without this, stale
    // `file:<id>` entries with paths to now-deleted files (or files with the
    // wrong extension after a code change) keep getting served back to the
    // frontend via get_cached_files_list. That's why a "clear cache" can
    // appear to do nothing — the disk was wiped but the in-memory paths
    // outlived it. Also clear the dirty flag so the next debounced flush
    // doesn't write the stale snapshot back to disk.
    if let Ok(mut guard) = MANIFEST_MEMORY.write() {
        *guard = UniversalCacheManifest {
            version: CACHE_VERSION,
            last_sync: None,
            entries: HashMap::new(),
        };
    }
    MANIFEST_DIRTY.store(false, Ordering::Release);

    Ok(())
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
    if let Some(ct) = content_type {
        let ct = ct.to_ascii_lowercase();
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
    let type_dir = file_type_dir(&cache_type)?;

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
    let safe_id = id.replace(['/', '\\'], "_");
    let file_name = format!("{}.{}", safe_id, extension);
    let (path_str, entry) =
        write_file_entry(&type_dir, cache_type, id, url, file_name, bytes, expiry_days).await?;
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

/// Write a downloaded file into its type folder and build its manifest entry,
/// without saving the manifest. Returns the file's path with the entry.
async fn write_file_entry<B>(
    type_dir: &std::path::Path,
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
    let file_path = type_dir.join(&file_name);

    // Write on the blocking pool: a channel join downloads many emotes/badges
    // and a sync write per file on the runtime saturates the worker threads
    // (the join-time freeze). The HTTP fetch is already async.
    let write_path = file_path.clone();
    tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        let mut file = fs::File::create(&write_path)?;
        std::io::copy(&mut Cursor::new(bytes), &mut file)?;
        Ok(())
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
    use sha2::{Digest, Sha256};
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let last = path.rsplit('/').next().unwrap_or("");
    let stem = last.rsplit_once('.').map_or(last, |(stem, _)| stem);
    let readable: String = stem
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(48)
        .collect();
    let digest = Sha256::digest(url.as_bytes());
    let hash: String = digest.iter().take(4).map(|b| format!("{b:02x}")).collect();
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
    let type_dir = file_type_dir(&CacheType::StreamNookArt)?;
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
        &type_dir,
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
    let type_dir = file_type_dir(&cache_type)?;

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
    let safe_id = id.replace(['/', '\\'], "_");
    let file_name = format!("{}.{}", safe_id, extension);
    let (_, entry) =
        write_file_entry(&type_dir, cache_type, id, url, file_name, bytes, expiry_days).await?;
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

/// The cached files among `ids` (id -> path), by the same rule as the listing.
/// Reads the in-memory manifest only.
pub fn cached_file_paths(cache_type: CacheType, ids: &[String]) -> HashMap<String, String> {
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

pub async fn get_cached_files_list(cache_type: CacheType) -> Result<HashMap<String, String>> {
    // debug!(
    //     "[UniversalCache] Getting cached files list for {:?}",
    //     cache_type
    // );
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
        badge_date_key, is_still_paint_entry, usable_file_path, webp_is_animated, CacheMetadata, CacheType,
        UniversalCacheEntry,
    };

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
