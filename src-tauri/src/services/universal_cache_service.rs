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
// anything if the app never modifies the manifest (read-only sessions).
static FLUSH_TASK_STARTED: AtomicBool = AtomicBool::new(false);

fn ensure_flush_task() {
    if FLUSH_TASK_STARTED
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
    {
        // The flush loop needs a Tokio runtime. Some callers (e.g. the one-time
        // emote-cache migration invoked from main() before the async runtime is
        // built) would panic here with "no reactor running". When there's no
        // runtime yet, reset the flag and bail: MANIFEST_DIRTY stays set, so the
        // next save once the runtime is up starts the task and flushes nothing is
        // lost. Callers that must persist before then write through to disk
        // directly (see migrate_emote_cache_on_version_change).
        if tokio::runtime::Handle::try_current().is_err() {
            FLUSH_TASK_STARTED.store(false, Ordering::SeqCst);
            return;
        }
        tokio::spawn(async {
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
        });
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
const UNIVERSAL_CACHE_URL: &str =
    "https://raw.githubusercontent.com/winters27/StreamNook/refs/heads/main/universal-cache/main";

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
    let type_str = match cache_type {
        CacheType::Badge => "badges",
        CacheType::Emote => "emotes",
        CacheType::ThirdPartyBadge => "third-party-badges",
        CacheType::Cosmetic => "cosmetics",
    };

    // Try to fetch from universal cache repository
    let url = format!("{}/{}/{}.json", UNIVERSAL_CACHE_URL, type_str, id);

    debug!("[UniversalCache] Attempting to fetch from: {}", url);

    let client = &*META_CLIENT_5S;

    match client.get(&url).send().await {
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

    match client.get(&url).send().await {
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
        let type_str = match cache_type {
            CacheType::Badge => "badges",
            CacheType::Emote => "emotes",
            CacheType::ThirdPartyBadge => "third-party-badges",
            CacheType::Cosmetic => "cosmetics",
        };

        let index_url = format!("{}/{}/index.json", UNIVERSAL_CACHE_URL, type_str);

        debug!("[UniversalCache] Fetching index from: {}", index_url);

        let client = &*META_CLIENT_10S;

        match client.get(&index_url).send().await {
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
        let type_str = match entry.cache_type {
            CacheType::Badge => "badges",
            CacheType::Emote => "emotes",
            CacheType::ThirdPartyBadge => "third-party-badges",
            CacheType::Cosmetic => "cosmetics",
        };

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
    let cache_dir = get_universal_cache_dir()?;
    let type_str = match cache_type {
        CacheType::Badge => "badges",
        CacheType::Emote => "emotes",
        CacheType::ThirdPartyBadge => "third-party-badges",
        CacheType::Cosmetic => "cosmetics",
    };

    let type_dir = cache_dir.join(type_str);
    if !type_dir.exists() {
        fs::create_dir_all(&type_dir).context("Failed to create cache type directory")?;
    }

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
    let file_path = type_dir.join(&file_name);

    // Write on the blocking pool: a channel join downloads many emotes/badges
    // and a sync write per file on the runtime saturates the worker threads
    // (the join-time freeze). The HTTP fetch above is already async.
    let write_path = file_path.clone();
    tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        let mut file = fs::File::create(&write_path)?;
        std::io::copy(&mut Cursor::new(bytes), &mut file)?;
        Ok(())
    })
    .await
    .context("cache_file write task panicked")??;

    let path_str = file_path.to_string_lossy().to_string();
    let manifest_id = format!("file:{}", id);
    let entry = UniversalCacheEntry {
        id: manifest_id,
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
    save_cached_item(entry).await?;

    debug!("[UniversalCache] Successfully cached file: {}", path_str);
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
    let cache_dir = get_universal_cache_dir()?;
    let type_str = match cache_type {
        CacheType::Badge => "badges",
        CacheType::Emote => "emotes",
        CacheType::ThirdPartyBadge => "third-party-badges",
        CacheType::Cosmetic => "cosmetics",
    };

    let type_dir = cache_dir.join(type_str);
    if !type_dir.exists() {
        fs::create_dir_all(&type_dir).context("Failed to create cache type directory")?;
    }

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
    let file_path = type_dir.join(&file_name);

    // Write on the blocking pool: a channel join downloads many emotes/badges
    // and a sync write per file on the runtime saturates the worker threads
    // (the join-time freeze). The HTTP fetch above is already async.
    let write_path = file_path.clone();
    tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        let mut file = fs::File::create(&write_path)?;
        std::io::copy(&mut Cursor::new(bytes), &mut file)?;
        Ok(())
    })
    .await
    .context("cache_file write task panicked")??;

    let path_str = file_path.to_string_lossy().to_string();
    let manifest_id = format!("file:{}", id);
    Ok(UniversalCacheEntry {
        id: manifest_id,
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
    })
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
    let manifest = load_manifest()?;
    let mut files = HashMap::new();

    for (key, entry) in manifest.entries.iter() {
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

    match client.get(&url).send().await {
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
