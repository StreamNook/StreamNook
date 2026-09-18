// Universal cache service for badges, emotes, and other assets
import { invoke } from '@tauri-apps/api/core';

import { Logger } from '../utils/logger';

/**
 * Auto-sync universal cache if stale (>24 hours since last sync)
 * This is a fire-and-forget operation - runs in background on app startup
 * Returns true if sync was triggered, false if cache was fresh
 */
export async function autoSyncUniversalCacheIfStale(): Promise<boolean> {
  try {
    const synced = await invoke<boolean>('auto_sync_universal_cache_if_stale');
    if (synced) {
      Logger.debug('[UniversalCache] Auto-sync triggered (cache was stale)');
    }
    return synced;
  } catch (error) {
    Logger.error('[UniversalCache] Auto-sync check failed:', error);
    return false;
  }
}
