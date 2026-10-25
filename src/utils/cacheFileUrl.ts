import { convertFileSrc, invoke } from '@tauri-apps/api/core';
import { Logger } from './logger';

/** The universal cache folders the loopback server serves (cache_file_server.rs CACHE_DIRS). */
const CACHE_DIRS = new Set(['badges', 'emotes', 'third-party-badges', 'cosmetics', 'streamnook-art']);

/** Windows serves cached art over loopback (cache_file_server.rs); every other
 *  platform keeps the asset protocol. Read from the user agent here rather than
 *  from utils/platform, which many tests mock with only the names they use. */
const USES_LOOPBACK = typeof navigator !== 'undefined' && /Windows NT/.test(navigator.userAgent);

/** A loopback cache URL from any launch (the port and token change every launch). */
const LOOPBACK_CACHE_URL = /^http:\/\/127\.0\.0\.1:\d+\/cache\//;

let base: string | null = null;
let ready: Promise<void> | null = null;
let probing = false;

/** Learn the loopback server's address. Cheap to call often; retries while the
 *  server has not bound yet. */
export function ensureCacheFileServer(): Promise<void> {
  if (!USES_LOOPBACK) return Promise.resolve();
  if (base) return Promise.resolve();
  ready ??= invoke<{ port: number; token: string } | null>('cache_file_server')
    .then((s) => {
      if (s) base = `http://127.0.0.1:${s.port}/cache/${s.token}/`;
      else ready = null;
    })
    .catch(() => {
      ready = null;
    });
  return ready;
}

/** The page URL for a universal-cache file: the loopback server once known,
 *  else Tauri's asset URL (also for any path outside the cache folders). */
export function cacheFileUrl(path: string): string {
  if (base) {
    const parts = path.split(/[\\/]/);
    const file = parts.pop();
    const dir = parts.pop();
    if (file && dir && CACHE_DIRS.has(dir)) return `${base}${dir}/${encodeURIComponent(file)}`;
  }
  return convertFileSrc(path);
}

/** Whether a URL is a disk copy, in either form. */
export function isDiskCopyUrl(url: string | null | undefined): boolean {
  if (!url) return false;
  return url.startsWith('asset://') || url.includes('asset.localhost') || (base !== null && url.startsWith(base));
}

/** A `JSON.stringify` replacer for anything saved across launches: drops a
 *  `localUrl` that points at a loopback cache URL (the next launch has another
 *  port and token). Asset URLs stay; they name the file itself. */
export function withoutLoopbackUrls(key: string, value: unknown): unknown {
  return key === 'localUrl' && typeof value === 'string' && LOOPBACK_CACHE_URL.test(value) ? undefined : value;
}

/** An image on the loopback server failed: if the server itself is gone, later
 *  images use the asset protocol (the failed one falls back through its own
 *  onError ladder). */
function onImageError(e: Event): void {
  const img = e.target;
  if (!(img instanceof HTMLImageElement) || !base || !img.src.startsWith(base) || probing) return;
  probing = true;
  const probe = base;
  fetch(`${probe}ping`)
    .then((r) => {
      if (r.status !== 204) throw new Error(String(r.status));
    })
    .catch(() => {
      if (base === probe) {
        base = null;
        Logger.warn('[CacheFiles] the loopback server stopped answering; cached images use the asset protocol');
      }
    })
    .finally(() => {
      probing = false;
    });
}

if (USES_LOOPBACK && typeof window !== 'undefined') {
  window.addEventListener('error', onImageError, true);
}

/** Test seam. */
export function __setCacheFileBase(next: string | null): void {
  base = next;
  ready = null;
}
