/**
 * Badge Image Cache Service
 *
 * Twitch badge images on disk, for faster loading and offline-capable display.
 * Rust fills the cache (services/asset_cache_queue.rs, shared by every window);
 * this keeps the window's id -> file map that renders read synchronously.
 *
 * Badges are cached at 4x resolution (72px) for maximum crispness on HiDPI displays.
 */
import { invoke } from '@tauri-apps/api/core';
import { convertFileSrc } from '@tauri-apps/api/core';

import { Logger } from '../utils/logger';
import { onAssetsCached, requestAssetCaching } from './assetCacheQueue';

// Module-level registry of cached badge files (id -> localPath)
const cachedBadgeFiles: Map<string, string> = new Map();

onAssetsCached('badge', {
  arrived: (files) => {
    for (const [id, path] of Object.entries(files)) cachedBadgeFiles.set(id, path);
  },
  cleared: () => cachedBadgeFiles.clear(),
});

let initializationPromise: Promise<void> | null = null;

/**
 * Queue a badge for caching. This is called reactively when badges are rendered.
 * Rust answers at once when the file is already on disk.
 */
export function queueBadgeForCaching(id: string, url: string) {
  if (cachedBadgeFiles.has(id)) return;
  requestAssetCaching('badge', id, url);
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
