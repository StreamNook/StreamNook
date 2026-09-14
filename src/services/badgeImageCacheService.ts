/**
 * Badge Image Cache Service
 * 
 * Reactive caching for Twitch badge images - downloads and caches badge images locally
 * for faster loading and offline-capable display. Uses the same pattern as emoteService.
 * 
 * Badges are cached at 4x resolution (72px) for maximum crispness on HiDPI displays.
 */
import { invoke } from '@tauri-apps/api/core';
import { convertFileSrc } from '@tauri-apps/api/core';

import { Logger } from '../utils/logger';
import { features } from '../features';
// Module-level registry of cached badge files (id -> localPath)
const cachedBadgeFiles: Map<string, string> = new Map();

// Pending downloads to prevent duplicate requests
const pendingDownloads: Map<string, Promise<string | null>> = new Map();

// Queue system for downloads
const MAX_CONCURRENT_DOWNLOADS = 5;
const downloadQueue: Array<{ id: string, url: string }> = [];
let activeDownloads = 0;

// Settings cache
let cachedSettings: { enabled: boolean; expiryDays: number } | null = null;

let initializationPromise: Promise<void> | null = null;

async function getBadgeCacheSettings(): Promise<{ enabled: boolean; expiryDays: number }> {
  if (cachedSettings) return cachedSettings;
  try {
    const settings = await invoke('load_settings') as any;
    cachedSettings = {
      enabled: settings.cache?.enabled !== false,
      expiryDays: settings.cache?.expiry_days ?? 30 // Badges can cache longer since they rarely change
    };
    return cachedSettings;
  } catch (e) {
    Logger.warn('[BadgeImageCache] Failed to load settings:', e);
    return { enabled: true, expiryDays: 30 };
  }
}

async function processDownloadQueue() {
  if (activeDownloads >= MAX_CONCURRENT_DOWNLOADS || downloadQueue.length === 0) {
    return;
  }

  const next = downloadQueue.shift();
  if (!next) return;

  activeDownloads++;

  try {
    await downloadBadgeIfNeeded(next.id, next.url);
  } catch (e) {
    Logger.debug(`[BadgeImageCache] Error processing queue item ${next.id}:`, e);
  } finally {
    activeDownloads--;
    processDownloadQueue();
  }
}

async function downloadBadgeIfNeeded(id: string, url: string): Promise<string | null> {
  if (cachedBadgeFiles.has(id)) {
    return cachedBadgeFiles.get(id)!;
  }

  if (pendingDownloads.has(id)) {
    return pendingDownloads.get(id)!;
  }

  if (!features.assetDiskCache) return null;

  const settings = await getBadgeCacheSettings();
  if (!settings.enabled) return null;

  const downloadPromise = (async () => {
    try {
      const localPath = await invoke('download_and_cache_file', {
        cacheType: 'badge',
        id,
        url,
        expiryDays: settings.expiryDays
      }) as string;

      if (localPath) {
        cachedBadgeFiles.set(id, localPath);
        return localPath;
      }
      return null;
    } catch (e) {
      Logger.debug(`[BadgeImageCache] Failed to cache badge ${id}:`, e);
      return null;
    } finally {
      pendingDownloads.delete(id);
    }
  })();

  pendingDownloads.set(id, downloadPromise);
  return downloadPromise;
}
// Deferred queue for items that arrive during initialization
const deferredQueue: Array<{ id: string, url: string }> = [];
let cacheInitialized = false;

/**
 * Queue a badge for caching. This is called reactively when badges are rendered.
 * If the badge is not already cached or being downloaded, it will be queued.
 */
export function queueBadgeForCaching(id: string, url: string) {
  // If already in any queue/cache, skip
  if (cachedBadgeFiles.has(id) || pendingDownloads.has(id) || 
      downloadQueue.some(item => item.id === id) || 
      deferredQueue.some(item => item.id === id)) {
    return;
  }

  // If cache not yet initialized, defer the queue and start init
  if (!cacheInitialized) {
    deferredQueue.push({ id, url });
    
    // Start initialization if not already in progress
    if (!initializationPromise) {
      initializeBadgeImageCache().then(() => {
        cacheInitialized = true;
        // Process deferred queue - items may now be in cache
        while (deferredQueue.length > 0) {
          const item = deferredQueue.shift()!;
          // Re-check if it's now cached
          if (!cachedBadgeFiles.has(item.id) && !pendingDownloads.has(item.id) && 
              !downloadQueue.some(q => q.id === item.id)) {
            downloadQueue.push(item);
          }
        }
        processDownloadQueue();
      });
    }
    return;
  }

  downloadQueue.push({ id, url });
  processDownloadQueue();
}

/**
 * Get the cached local URL for a badge if it exists.
 * Returns undefined if the badge is not cached.
 */
export function getCachedBadgeUrl(id: string): string | undefined {
  const path = cachedBadgeFiles.get(id);
  return path ? convertFileSrc(path) : undefined;
}

/**
 * Check if a badge is cached locally.
 */
export function isBadgeCached(id: string): boolean {
  return cachedBadgeFiles.has(id);
}

/**
 * Initialize the badge file cache from disk.
 * This should be called once at app startup.
 */
export async function initializeBadgeImageCache(): Promise<void> {
  if (cachedBadgeFiles.size > 0) return;

  if (initializationPromise) {
    return initializationPromise;
  }

  initializationPromise = (async () => {
    try {
      Logger.debug('[BadgeImageCache] Initializing badge file cache...');
      const files = await invoke('get_cached_files', { cacheType: 'badge' }) as Record<string, string>;
      Object.entries(files).forEach(([id, path]) => cachedBadgeFiles.set(id, path));
      Logger.debug(`[BadgeImageCache] Badge file cache initialized with ${cachedBadgeFiles.size} entries`);
    } catch (e) {
      Logger.warn('[BadgeImageCache] Failed to init badge file cache:', e);
    } finally {
      initializationPromise = null;
    }
  })();

  return initializationPromise;
}

/**
 * Clear the badge image cache (both in-memory and disk).
 */
export async function clearBadgeImageCache(): Promise<void> {
  cachedBadgeFiles.clear();
  Logger.debug('[BadgeImageCache] Cleared in-memory badge cache');
}

/**
 * Get cache statistics.
 */
export function getBadgeCacheStats(): { cached: number; pending: number; queued: number } {
  return {
    cached: cachedBadgeFiles.size,
    pending: pendingDownloads.size,
    queued: downloadQueue.length
  };
}
