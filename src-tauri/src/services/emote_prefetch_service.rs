// AFK bulk emote prefetch: walk every followed channel, fetch
// each provider emote set, and download every emote image to the on-disk cache
// so the emote menu opens disk-first with nothing to fetch when you return.
//
// Two phases, both background tasks driven by events so the UI never blocks:
//   1. plan()  — drain follows, scan emote lists (bounded concurrency), dedup
//                by the same per-DPI cache key the frontend uses, diff against
//                what is already on disk, and stash the to-download list.
//   2. start() — download the to-download list with bounded concurrency, writing
//                the manifest in batches (the single change that keeps caching
//                10k+ files linear instead of O(N^2) — see
//                universal_cache_service::save_cached_items_batch).
//
// AFK is the ideal time to be aggressive: no video is playing, so there is
// nothing to be polite to. We still cap concurrency so the free provider CDNs
// are not hammered, and dedup means a global/shared emote downloads exactly once
// across all follows.

use log::{debug, warn};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::sync::RwLock;
use tokio::task::JoinSet;

use crate::services::emote_service::{
    seventv_circuit_open, Emote, EmoteProvider, EmoteService, EmoteSet,
};
use crate::services::twitch_service::TwitchService;
use crate::services::universal_cache_service::{
    download_file_to_disk, get_cached_files_list, save_cached_items_batch, CacheType,
    UniversalCacheEntry,
};

/// How many channels' emote lists to fetch at once during planning. Each fetch
/// internally fans out to 4 providers, so keep this modest.
const SCAN_CONCURRENCY: usize = 8;
/// How many emote image downloads to keep in flight. AFK, so aggressive, but
/// capped to stay a decent citizen to the provider CDNs.
const DOWNLOAD_CONCURRENCY: usize = 16;
/// Flush the manifest to memory once this many file entries have accumulated.
const MANIFEST_FLUSH_EVERY: usize = 150;
/// Emit a progress event at most every this many completed downloads.
const PROGRESS_EMIT_EVERY: usize = 25;
/// Rough average on-disk size of one cached emote (AVIF/webp at 1x-2x), used
/// only to show an estimated total in the UI before anything is downloaded.
const AVG_EMOTE_BYTES: u64 = 10 * 1024;
/// Disk-cache lifetime for prefetched files (matches the app's default).
const EXPIRY_DAYS: u32 = 7;

const EVENT_PROGRESS: &str = "emote-prefetch-progress";
const EVENT_COMPLETE: &str = "emote-prefetch-complete";

/// Snapshot of the prefetch job, sent to the UI on every progress event and
/// returned by the status command so a reopened panel re-syncs.
#[derive(Debug, Clone, Serialize)]
pub struct PrefetchProgress {
    /// "idle" | "scanning" | "planned" | "downloading" | "complete" | "cancelled"
    pub phase: String,
    pub channels_total: usize,
    pub channels_done: usize,
    pub current_channel: Option<String>,
    /// Unique emotes discovered across all follows (deduped by cache key).
    pub total_emotes: usize,
    pub already_cached: usize,
    pub to_download: usize,
    pub downloaded: usize,
    pub failed: usize,
    /// Rough byte estimate for the to-download set (count * AVG_EMOTE_BYTES).
    pub estimated_bytes: u64,
    /// True if 7TV's API was unavailable, so the counts omit most 7TV emotes.
    pub seventv_unavailable: bool,
    /// Human-readable heads-up shown in the UI when the scan is incomplete.
    pub warning: Option<String>,
}

impl Default for PrefetchProgress {
    fn default() -> Self {
        Self {
            phase: "idle".to_string(),
            channels_total: 0,
            channels_done: 0,
            current_channel: None,
            total_emotes: 0,
            already_cached: 0,
            to_download: 0,
            downloaded: 0,
            failed: 0,
            estimated_bytes: 0,
            seventv_unavailable: false,
            warning: None,
        }
    }
}

/// One emote to cache: `key` is the cache id (what we pass to the cache so the
/// manifest key matches the frontend's lookup) and `url` is where to fetch it.
#[derive(Debug, Clone)]
struct PrefetchItem {
    key: String,
    url: String,
}

pub struct EmotePrefetchService {
    emote_service: Arc<RwLock<EmoteService>>,
    progress: Arc<RwLock<PrefetchProgress>>,
    plan: Arc<RwLock<Vec<PrefetchItem>>>,
    cancel: Arc<RwLock<bool>>,
}

impl EmotePrefetchService {
    pub fn new(emote_service: Arc<RwLock<EmoteService>>) -> Self {
        Self {
            emote_service,
            progress: Arc::new(RwLock::new(PrefetchProgress::default())),
            plan: Arc::new(RwLock::new(Vec::new())),
            cancel: Arc::new(RwLock::new(false)),
        }
    }

    pub async fn get_progress(&self) -> PrefetchProgress {
        self.progress.read().await.clone()
    }

    /// Request cancellation of the current scan/download. The running loop exits
    /// at its next checkpoint and emits a final "cancelled" event.
    pub async fn stop(&self) {
        *self.cancel.write().await = true;
    }

    /// Phase 1: scan all follows and compute the to-download list. Spawns a
    /// background task and returns immediately; progress arrives via events.
    pub async fn plan(&self, app_handle: AppHandle, tier: String) {
        {
            let p = self.progress.read().await;
            if p.phase == "scanning" || p.phase == "downloading" {
                return; // already busy
            }
        }
        *self.cancel.write().await = false;
        {
            let mut p = self.progress.write().await;
            *p = PrefetchProgress::default();
            p.phase = "scanning".to_string();
        }
        self.plan.write().await.clear();

        let emote_service = self.emote_service.clone();
        let progress = self.progress.clone();
        let plan = self.plan.clone();
        let cancel = self.cancel.clone();

        tokio::spawn(async move {
            run_plan(emote_service, progress, plan, cancel, app_handle, tier).await;
        });
    }

    /// Phase 2: download the stashed to-download list. Spawns a background task
    /// and returns immediately; progress arrives via events.
    pub async fn start(&self, app_handle: AppHandle) {
        {
            let p = self.progress.read().await;
            if p.phase == "downloading" {
                return; // already running
            }
        }
        *self.cancel.write().await = false;

        let items = self.plan.read().await.clone();
        {
            let mut p = self.progress.write().await;
            p.phase = "downloading".to_string();
            p.downloaded = 0;
            p.failed = 0;
            p.current_channel = None;
        }

        let progress = self.progress.clone();
        let cancel = self.cancel.clone();

        tokio::spawn(async move {
            run_downloads(items, progress, cancel, app_handle).await;
        });
    }
}

/// Map an emote to the (cache-key, url) pair the frontend would use, so a
/// prefetched file lands under exactly the key the picker looks up. 7TV is
/// per-DPI-tiered (`id@tier`, tier url); every other provider keys by bare id at
/// its canonical url. Mirrors `emoteCacheKey` / `sevenTvTierUrl` in
/// services/emoteService.ts.
fn emote_cache_target(emote: &Emote, tier: &str) -> (String, String) {
    match emote.provider {
        EmoteProvider::SevenTV => (
            format!("{}@{}", emote.id, tier),
            format!("https://cdn.7tv.app/emote/{}/{}.avif", emote.id, tier),
        ),
        // Provider-namespaced so a Twitch and an FFZ emote that share a numeric
        // id can't collide. Must match emoteCacheKey() in services/emoteService.ts.
        EmoteProvider::Twitch => (format!("twitch-{}", emote.id), emote.url.clone()),
        EmoteProvider::BTTV => (format!("bttv-{}", emote.id), emote.url.clone()),
        EmoteProvider::FFZ => (format!("ffz-{}", emote.id), emote.url.clone()),
        EmoteProvider::Kick => (format!("kick-{}", emote.id), emote.url.clone()),
    }
}

fn set_emotes(set: &EmoteSet) -> impl Iterator<Item = &Emote> {
    set.twitch
        .iter()
        .chain(set.bttv.iter())
        .chain(set.seven_tv.iter())
        .chain(set.ffz.iter())
        .chain(set.kick.iter())
}

async fn emit_progress(progress: &Arc<RwLock<PrefetchProgress>>, app_handle: &AppHandle) {
    let snapshot = progress.read().await.clone();
    let _ = app_handle.emit(EVENT_PROGRESS, &snapshot);
}

async fn run_plan(
    emote_service: Arc<RwLock<EmoteService>>,
    progress: Arc<RwLock<PrefetchProgress>>,
    plan: Arc<RwLock<Vec<PrefetchItem>>>,
    cancel: Arc<RwLock<bool>>,
    app_handle: AppHandle,
    tier: String,
) {
    // Auth token for user-specific Twitch emotes (sub/follower/bits). Optional —
    // without it we still get globals + third-party sets.
    let token = TwitchService::get_token().await.ok();

    // Probe 7TV up front so the count can be honestly flagged as incomplete when
    // 7TV's API is down (it lists each channel's emotes; without it most of a
    // channel's set is invisible). Going through the probe also trips the circuit
    // breaker, so the scan that follows fails fast on 7TV instead of grinding.
    let seventv_ok_at_start = emote_service.read().await.seventv_api_healthy().await;
    if !seventv_ok_at_start {
        {
            let mut p = progress.write().await;
            p.seventv_unavailable = true;
            p.warning = Some(
                "7TV's API is unavailable right now, so 7TV emotes can't be scanned. This count will be incomplete — re-scan when 7TV recovers.".to_string(),
            );
        }
        emit_progress(&progress, &app_handle).await;
    }

    // Drain the full followed-channels list (live or offline).
    let mut channels: Vec<(String, String)> = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        match TwitchService::get_all_followed_channels(100, cursor.clone()).await {
            Ok((page, next)) => {
                for s in page {
                    if !s.user_login.is_empty() && !s.user_id.is_empty() {
                        channels.push((s.user_login, s.user_id));
                    }
                }
                match next {
                    Some(c) => cursor = Some(c),
                    None => break,
                }
            }
            Err(e) => {
                warn!("[EmotePrefetch] Failed to fetch followed channels: {}", e);
                break;
            }
        }
        if *cancel.read().await {
            break;
        }
    }

    {
        let mut p = progress.write().await;
        p.channels_total = channels.len();
    }
    emit_progress(&progress, &app_handle).await;

    // Discovered emotes deduped by cache key -> url.
    let mut discovered: HashMap<String, String> = HashMap::new();

    let mut iter = channels.into_iter();
    let mut join_set: JoinSet<(String, String, anyhow::Result<EmoteSet>)> = JoinSet::new();

    // Prime the scan pool.
    for _ in 0..SCAN_CONCURRENCY {
        if let Some((login, id)) = iter.next() {
            spawn_scan(&mut join_set, &emote_service, login, id, token.clone());
        } else {
            break;
        }
    }

    while let Some(joined) = join_set.join_next().await {
        if let Ok((login, id, result)) = joined {
            match result {
                Ok(set) => {
                    // Persist this channel's dictionary disk-first so chat can
                    // recognize its emotes with no live fetch on join. Skip when
                    // 7TV's circuit is open (the set is likely globals-only and
                    // would be a deficient canonical copy); save()'s don't-shrink
                    // guard is the backstop for everything else.
                    if !seventv_circuit_open() {
                        crate::services::emote_set_cache::save(&id, &set);
                    }
                    for emote in set_emotes(&set) {
                        let (key, url) = emote_cache_target(emote, &tier);
                        discovered.entry(key).or_insert(url);
                    }
                }
                Err(e) => {
                    warn!("[EmotePrefetch] emote fetch failed for {}: {}", login, e);
                }
            }
            {
                let mut p = progress.write().await;
                p.channels_done += 1;
                p.current_channel = Some(login);
                p.total_emotes = discovered.len();
            }
            emit_progress(&progress, &app_handle).await;
        }

        if *cancel.read().await {
            join_set.abort_all();
            let mut p = progress.write().await;
            p.phase = "cancelled".to_string();
            p.current_channel = None;
            drop(p);
            emit_progress(&progress, &app_handle).await;
            return;
        }

        // Refill.
        if let Some((login, id)) = iter.next() {
            spawn_scan(&mut join_set, &emote_service, login, id, token.clone());
        }
    }

    // Diff against what is already on disk (pure in-memory manifest read).
    let cached = get_cached_files_list(CacheType::Emote)
        .await
        .unwrap_or_default();

    let mut to_download: Vec<PrefetchItem> = Vec::new();
    let mut already_cached = 0usize;
    for (key, url) in discovered.iter() {
        if cached.contains_key(key) {
            already_cached += 1;
        } else {
            to_download.push(PrefetchItem {
                key: key.clone(),
                url: url.clone(),
            });
        }
    }

    let to_download_count = to_download.len();
    *plan.write().await = to_download;

    // 7TV counts as failed if the up-front probe failed OR the circuit opened
    // mid-scan (i.e. it died partway through). Either way the count is incomplete.
    let seventv_failed = !seventv_ok_at_start || seventv_circuit_open();
    {
        let mut p = progress.write().await;
        p.phase = "planned".to_string();
        p.current_channel = None;
        p.total_emotes = discovered.len();
        p.already_cached = already_cached;
        p.to_download = to_download_count;
        p.estimated_bytes = to_download_count as u64 * AVG_EMOTE_BYTES;
        p.seventv_unavailable = seventv_failed;
        if seventv_failed && p.warning.is_none() {
            p.warning = Some(
                "7TV's API errored during the scan, so some 7TV emotes may be missing. Re-scan when 7TV is stable.".to_string(),
            );
        }
    }
    debug!(
        "[EmotePrefetch] Plan ready: {} unique emotes, {} already cached, {} to download",
        discovered.len(),
        already_cached,
        to_download_count
    );
    emit_progress(&progress, &app_handle).await;
}

fn spawn_scan(
    join_set: &mut JoinSet<(String, String, anyhow::Result<EmoteSet>)>,
    emote_service: &Arc<RwLock<EmoteService>>,
    login: String,
    id: String,
    token: Option<String>,
) {
    let es = emote_service.clone();
    join_set.spawn(async move {
        let guard = es.read().await;
        let result = guard
            // Prefetch runs over Twitch logins only.
            .fetch_channel_emotes(Some(login.clone()), Some(id.clone()), token, None)
            .await;
        (login, id, result)
    });
}

async fn run_downloads(
    items: Vec<PrefetchItem>,
    progress: Arc<RwLock<PrefetchProgress>>,
    cancel: Arc<RwLock<bool>>,
    app_handle: AppHandle,
) {
    let mut iter = items.into_iter();
    let mut join_set: JoinSet<anyhow::Result<UniversalCacheEntry>> = JoinSet::new();
    let mut buffer: Vec<UniversalCacheEntry> = Vec::new();
    let mut since_emit = 0usize;

    // Prime the download pool.
    for _ in 0..DOWNLOAD_CONCURRENCY {
        if let Some(item) = iter.next() {
            join_set.spawn(download_file_to_disk(
                CacheType::Emote,
                item.key,
                item.url,
                EXPIRY_DAYS,
            ));
        } else {
            break;
        }
    }

    while let Some(joined) = join_set.join_next().await {
        match joined {
            Ok(Ok(entry)) => {
                buffer.push(entry);
                progress.write().await.downloaded += 1;
            }
            Ok(Err(_)) | Err(_) => {
                progress.write().await.failed += 1;
            }
        }

        if buffer.len() >= MANIFEST_FLUSH_EVERY {
            let batch = std::mem::take(&mut buffer);
            let _ = save_cached_items_batch(batch).await;
        }

        since_emit += 1;
        if since_emit >= PROGRESS_EMIT_EVERY {
            since_emit = 0;
            emit_progress(&progress, &app_handle).await;
        }

        if *cancel.read().await {
            join_set.abort_all();
            break;
        }

        if let Some(item) = iter.next() {
            join_set.spawn(download_file_to_disk(
                CacheType::Emote,
                item.key,
                item.url,
                EXPIRY_DAYS,
            ));
        }
    }

    // Persist whatever is left.
    if !buffer.is_empty() {
        let _ = save_cached_items_batch(buffer).await;
    }

    let cancelled = *cancel.read().await;
    {
        let mut p = progress.write().await;
        p.phase = if cancelled {
            "cancelled".to_string()
        } else {
            "complete".to_string()
        };
        p.current_channel = None;
    }
    emit_progress(&progress, &app_handle).await;
    let final_snapshot = progress.read().await.clone();
    let _ = app_handle.emit(EVENT_COMPLETE, &final_snapshot);
}
