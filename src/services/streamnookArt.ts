// StreamNook's own cosmetic art (badges, relics, frames and atmosphere images on
// cdn.streamnook.app) from Rust's disk cache. Rust fetches each file once, resuming a transfer the
// network cut short (universal_cache_service::cache_streamnook_art), and keeps
// it until the art ships under a new URL. This keeps the window's url -> file
// map and tells subscribers when a file lands, so a tile whose CDN load failed
// picks up the disk copy on its own.
import { convertFileSrc, invoke } from '@tauri-apps/api/core';

import { Logger } from '../utils/logger';
import { onAssetsCached, requestAssetCaching } from './assetCacheQueue';

const ART_ORIGIN = 'https://cdn.streamnook.app/';

/** CDN URL -> file on disk. */
const files = new Map<string, string>();
let version = 0;
const listeners = new Set<() => void>();
let listed = false;

function changed(): void {
  version++;
  for (const listener of listeners) listener();
}

onAssetsCached('streamnook', {
  arrived: (arrived) => {
    for (const [url, path] of Object.entries(arrived)) files.set(url, path);
    changed();
  },
  cleared: () => {
    files.clear();
    changed();
  },
});

/** Read what is already on disk, once per window, before the first request. */
function listCachedArt(): void {
  if (listed) return;
  listed = true;
  invoke<Record<string, string>>('get_cached_files', { cacheType: 'streamnook' })
    .then((found) => {
      let added = 0;
      for (const [url, path] of Object.entries(found ?? {})) {
        if (!files.has(url)) {
          files.set(url, path);
          added++;
        }
      }
      if (added > 0) changed();
    })
    .catch((e) => Logger.debug('[StreamNookArt] could not list cached art:', e));
}

export function isStreamnookArtUrl(url: string): boolean {
  return url.startsWith(ART_ORIGIN);
}

/**
 * The disk copy of a piece of art as a loadable URL, or null when there is none
 * yet. convertFileSrc leaves `(`, `)` and `'` as they are, and atmosphere CSS
 * puts the URL in an unquoted `url(...)`, so a profile folder like
 * "John (Work)" would end the rule early; those three are percent-encoded.
 */
function artCopy(url: string): string | null {
  listCachedArt();
  const path = files.get(url);
  return path
    ? convertFileSrc(path).replace(/[()']/g, (c) => `%${c.charCodeAt(0).toString(16).toUpperCase()}`)
    : null;
}

/** Ask Rust for a disk copy of a piece of art (repeats are dropped). */
export function requestStreamnookArt(url: string): void {
  if (isStreamnookArtUrl(url) && !artCopy(url)) requestAssetCaching('streamnook', url, url);
}

/**
 * Where to load a piece of art from: its disk copy when Rust has one, else
 * the CDN URL itself, with Rust asked to fetch a copy. Any other URL comes
 * back unchanged. Cheap to call on every render.
 */
export function streamnookArtSrc(url: string): string {
  if (!isStreamnookArtUrl(url)) return url;
  const copy = artCopy(url);
  if (copy) return copy;
  requestAssetCaching('streamnook', url, url);
  return url;
}

// A CDN URL on its own or inside CSS (`url(https://cdn.streamnook.app/x.webp) center / cover`).
const ART_URL_IN_TEXT = /https:\/\/cdn\.streamnook\.app\/[^\s"'()]+/g;

/** Every piece of StreamNook art a value names: a plain URL or URLs inside CSS. */
export function streamnookArtUrlsIn(text: string | undefined): string[] {
  return text ? (text.match(ART_URL_IN_TEXT) ?? []) : [];
}

/**
 * The same value with each piece of art that has a disk copy pointed at it.
 * Asks for nothing, so a catalog can be mapped without fetching art nobody
 * is looking at. Works on a plain URL and on CSS.
 */
export function withStreamnookArtCopies<T extends string | undefined>(text: T): T {
  if (!text) return text;
  return text.replace(ART_URL_IN_TEXT, (url) => artCopy(url) ?? url) as T;
}

/** For useSyncExternalStore: re-render when a piece of art lands on disk. */
export function subscribeStreamnookArt(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function getStreamnookArtVersion(): number {
  return version;
}
