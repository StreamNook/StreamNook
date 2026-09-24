// Hands images to Rust's disk-cache fill (services/asset_cache_queue.rs) and
// hears back where they landed. Rust keeps one queue per kind for every window,
// dedupes across them, and paces the downloads; this only batches a window's
// requests into one call per tick and passes arrivals to the service that keeps
// that kind's lookup map (emotes, 7TV cosmetics, Twitch badges).
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

import { features } from '../features';
import { Logger } from '../utils/logger';

export type AssetKind = 'emote' | 'cosmetic' | 'badge';

type Arrivals = (files: Record<string, string>) => void;

interface Sink {
  arrived: Arrivals;
  /** The cache folder was wiped (in any window): drop every path. */
  cleared: () => void;
}

interface CachedEvent {
  kind: AssetKind;
  files: Record<string, string>;
}

/** A request that got no answer (a failed download) may be asked again after this. */
const RETRY_AFTER_MS = 10 * 60_000;

const sinks = new Map<AssetKind, Sink>();
const requestedAt = new Map<string, number>();
const batches = new Map<string, { kind: AssetKind; priority: boolean; items: Array<{ id: string; url: string }> }>();
let flushScheduled = false;
let listening: Promise<unknown> | null = null;

function ensureListening(): Promise<unknown> {
  listening ??= Promise.all([
    listen<CachedEvent>('asset-cache://cached', (event) => {
      const { kind, files } = event.payload;
      for (const id of Object.keys(files)) requestedAt.delete(`${kind}:${id}`);
      sinks.get(kind)?.arrived(files);
    }),
    listen('asset-cache://cleared', () => {
      requestedAt.clear();
      for (const sink of sinks.values()) sink.cleared();
    }),
  ]).catch((e) => {
    listening = null;
    Logger.warn('[AssetCache] could not listen for cached files:', e);
  });
  return listening;
}

/** Where a kind's arrivals go: the service's own id -> path map. */
export function onAssetsCached(kind: AssetKind, sink: Sink): void {
  sinks.set(kind, sink);
  void ensureListening();
}

async function flush(): Promise<void> {
  flushScheduled = false;
  const pending = [...batches.values()];
  batches.clear();
  // Files already on disk are answered inside the enqueue call, so the listener
  // must be up before it goes out.
  await ensureListening();
  for (const { kind, priority, items } of pending) {
    invoke('asset_cache_enqueue', { kind, items, priority }).catch((e) => {
      for (const { id } of items) requestedAt.delete(`${kind}:${id}`);
      Logger.debug(`[AssetCache] enqueue failed for ${items.length} ${kind} files:`, e);
    });
  }
}

/**
 * Ask for a file to be cached on disk. Cheap to call on every render: repeats
 * within a window are dropped here, and repeats across windows in Rust.
 * Priority requests (search results, what the user is looking at) go first.
 */
export function requestAssetCaching(kind: AssetKind, id: string, url: string, priority = false): void {
  if (!features.assetDiskCache || !id || !url) return;
  const key = `${kind}:${id}`;
  const now = Date.now();
  const at = requestedAt.get(key);
  if (at !== undefined && now - at < RETRY_AFTER_MS) return;
  requestedAt.set(key, now);

  const batchKey = `${kind}:${priority}`;
  let batch = batches.get(batchKey);
  if (!batch) {
    batch = { kind, priority, items: [] };
    batches.set(batchKey, batch);
  }
  batch.items.push({ id, url });
  if (!flushScheduled) {
    flushScheduled = true;
    queueMicrotask(() => void flush());
  }
}

/** An emote picker opened (true) or closed (false) in this window: emotes fill fast while one is open. */
export function setAssetCacheBurst(active: boolean): void {
  if (!features.assetDiskCache) return;
  invoke('asset_cache_set_burst', { active }).catch((e) => Logger.debug('[AssetCache] burst toggle failed:', e));
}

/** Forget what this window asked for, after a cache clear. */
export function forgetAssetRequests(kind: AssetKind): void {
  const prefix = `${kind}:`;
  for (const key of requestedAt.keys()) {
    if (key.startsWith(prefix)) requestedAt.delete(key);
  }
}
