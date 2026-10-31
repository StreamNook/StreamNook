import { convertFileSrc, invoke } from '@tauri-apps/api/core';
import { Logger } from './logger';

/** The universal cache folders the loopback server serves (cache_file_server.rs CACHE_DIRS). */
const CACHE_DIRS = new Set(['badges', 'emotes', 'third-party-badges', 'cosmetics', 'streamnook-art']);

/** Windows and Android serve cached art over loopback (cache_file_server.rs
 *  `serves_on`); every other platform keeps the asset protocol. Read from the
 *  user agent rather than from utils/platform, which many tests mock with only
 *  the names they use. */
export function usesLoopback(userAgent: string): boolean {
  return /Windows NT|Android/.test(userAgent);
}

const AGENT = typeof navigator !== 'undefined' ? navigator.userAgent : '';
const USES_LOOPBACK = usesLoopback(AGENT);
/** Android's asset replies are `no-store`, so its windows learn the server
 *  before their first paint and hold on to it harder (see awaitCacheFileServer). */
const ANDROID = /Android/.test(AGENT);

/** A loopback cache URL from any launch (the port can change between launches). */
const LOOPBACK_CACHE_URL = /^http:\/\/127\.0\.0\.1:\d+\/cache\//;

let base: string | null = null;
let ready: Promise<void> | null = null;
let probing = false;
/** Rust said the server gave up this launch (Android only): stop asking. */
let unavailable = false;

/** Learn the loopback server's address. Cheap to call often; retries while the
 *  server has not bound yet. */
export function ensureCacheFileServer(): Promise<void> {
  if (!USES_LOOPBACK) return Promise.resolve();
  return askCacheFileServer();
}

/** One ask of Rust for the server's address (shared while in flight). */
function askCacheFileServer(): Promise<void> {
  if (base || unavailable) return Promise.resolve();
  ready ??= invoke<{ port: number; token: string } | null>('cache_file_server')
    .then((s) => {
      if (s) base = `http://127.0.0.1:${s.port}/cache/${s.token}/`;
      else ready = null;
    })
    .catch((e) => {
      ready = null;
      if (e === 'unavailable') unavailable = true;
    });
  return ready;
}

/** Whether this window learns the server before its first paint: Android only.
 *  Every other window paints at once, as before. */
export function waitsForCacheFileServer(): boolean {
  return ANDROID;
}

const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, Math.max(0, ms)));

/** Android, before the first paint: ask until the server answers, gives up, or
 *  `capMs` pass (it binds just after the app starts), so no row starts on the
 *  asset protocol and switches. Each ask races the deadline, so a hung invoke
 *  still paints at the cap. Never rejects. Elsewhere it resolves at once. */
export async function awaitCacheFileServer(
  opts: { android?: boolean; capMs?: number; stepMs?: number } = {},
): Promise<void> {
  const { android = ANDROID, capMs = 3000, stepMs = 100 } = opts;
  if (!android) return;
  const started = Date.now();
  const until = started + capMs;
  for (;;) {
    await Promise.race([askCacheFileServer(), sleep(until - Date.now())]);
    if (base || unavailable || Date.now() >= until) break;
    await sleep(Math.min(stepMs, until - Date.now()));
  }
  const outcome = base ? 'serving' : unavailable ? 'unavailable' : 'no answer';
  Logger.info(`[CacheFiles] first paint after ${Date.now() - started} ms (${outcome})`);
}

/** Whether the server at `probe` is gone: `tries` pings `gapMs` apart all
 *  failed. One ping on Windows (as before); three on Android, where an app
 *  coming back from the background can fail one. */
export async function probeLost(
  probe: string,
  tries: number,
  gapMs: number,
  ping: (url: string) => Promise<{ status: number }> = (url) => fetch(url),
): Promise<boolean> {
  for (let i = 0; i < tries; i++) {
    if (i > 0) await sleep(gapMs);
    try {
      if ((await ping(`${probe}ping`)).status === 204) return false;
    } catch {
      /* counts as a failed ping */
    }
  }
  return true;
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
  const ping = ANDROID ? (url: string) => fetch(url, { signal: AbortSignal.timeout(1500) }) : undefined;
  void probeLost(probe, ANDROID ? 3 : 1, 700, ping)
    .then((lost) => {
      if (lost && base === probe) {
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
  unavailable = false;
}
