// 7TV badges and paints, as the webview renders them.
//
// Which cosmetics a chatter wears is resolved in Rust
// (services/seventv_cosmetics_resolver.rs): one batch queue, one cache and one
// definitions catalog for every window. What stays here is what only the
// webview can do: point cosmetics at their on-disk copies (convertFileSrc), ask
// for new ones to be cached, and pick badge image URLs.
import { invoke } from '@tauri-apps/api/core';
import { convertFileSrc } from '@tauri-apps/api/core';

import { Logger } from '../utils/logger';
import { forgetAssetRequests, onAssetsCached, requestAssetCaching } from './assetCacheQueue';
// The paint → CSS engine + its v4 paint types now live in the standalone,
// Tauri-free `paintStyle` module so the hosted overlay page can share the exact
// same rendering. Re-exported here so existing importers stay unchanged.
import type { PaintV4 } from './paintStyle';
export { computePaintStyle, pickPaintLayerImage } from './paintStyle';
export type { PaintShadowMode } from './paintStyle';

interface BadgeImageV4 {
  url: string;
  mime?: string;
  scale?: number;
  frameCount?: number;
}

interface BadgeV4 {
  id: string;
  name: string;
  description?: string;
  selected?: boolean;
  localUrl?: string;
  // Authoritative image URLs from the V4 API. A badge's id is NOT its image id
  // in V4, so these must be used rather than constructing a URL from `id`.
  images?: BadgeImageV4[];
}

interface UserCosmeticsResponse {
  paints: PaintV4[];
  badges: BadgeV4[];
  seventvUserId?: string; // The user's 7TV profile ID
}

// Public result shape for getUserCosmetics. hardFail distinguishes "we never
// got a real answer from 7TV" from "API said this user has no cosmetics."
// Callers (cosmeticsCache.ts) use this to decide whether to short-TTL the
// outer cache too.
export interface UserCosmeticsResult {
  data: UserCosmeticsResponse;
  hardFail: boolean;
}

// Cache for cosmetic file paths (id -> localPath) to avoid repeated IPC calls
let cachedCosmeticFiles: Record<string, string> | null = null;
// Whether the disk listing has been read successfully. Split out from
// `cachedCosmeticFiles === null` because that sentinel was doing two jobs at
// once: "we have not loaded from disk yet" and "we have nothing memoized". A
// download that finished before the listing arrived therefore had nowhere to
// record itself and got re-queued forever, at a manifest write each time.
let cosmeticFilesLoadedFromDisk = false;

let filesInitializationPromise: Promise<void> | null = null;

// Rust fills the disk cache (services/asset_cache_queue.rs, one queue for every
// window). Arrivals are recorded even before the disk listing has come back, so
// a download that wins that race is not reported as a miss and asked for again.
onAssetsCached('cosmetic', {
  arrived: (files) => {
    Object.assign((cachedCosmeticFiles ??= {}), files);
  },
  cleared: () => {
    cachedCosmeticFiles = null;
    cosmeticFilesLoadedFromDisk = false;
  },
});

// Queue a cosmetic for lazy caching - called when actually displayed
export function queueCosmeticForCaching(id: string, url: string) {
  if (cachedCosmeticFiles?.[id]) return;
  requestAssetCaching('cosmetic', id, url);
}

/** Point a paint's image layers at their on-disk copies, where we have them. */
function applyPaintLocalFiles(paintData: any, cachedFiles: Record<string, string>): void {
  if (!paintData?.data?.layers) return;
  for (const layer of paintData.data.layers) {
    if (layer.ty?.__typename === 'PaintLayerTypeImage' && layer.ty.images) {
      const localPath = cachedFiles[layer.id];
      if (localPath) {
        const localUrl = convertFileSrc(localPath);
        layer.ty.images.forEach((img: any) => {
          img.localUrl = localUrl;
        });
      }
    }
  }
}

/** Point a lookup's cosmetics at their on-disk copies. Results arrive fresh
 *  from Rust per call, so stamping them in place touches nothing shared. */
function applyLocalFiles(cosmetics: UserCosmeticsResponse, cachedFiles: Record<string, string>): UserCosmeticsResponse {
  for (const paint of cosmetics.paints) applyPaintLocalFiles(paint, cachedFiles);
  for (const badge of cosmetics.badges) {
    const localPath = cachedFiles[badge.id];
    if (localPath) badge.localUrl = convertFileSrc(localPath);
  }
  return cosmetics;
}

/** The disk listing of cached cosmetic files, read once per window. */
function ensureCosmeticFileListing(): Promise<void> | null {
  if (!cosmeticFilesLoadedFromDisk && !filesInitializationPromise) {
    filesInitializationPromise = (async () => {
      try {
        const fromDisk = await invoke<Record<string, string>>('get_cached_files', {
          cacheType: 'cosmetic',
        });
        // Merge rather than replace: anything downloaded while this was in
        // flight is already memoized above and would otherwise be dropped.
        cachedCosmeticFiles = { ...fromDisk, ...(cachedCosmeticFiles ?? {}) };
        cosmeticFilesLoadedFromDisk = true;
      } catch (e) {
        Logger.warn('Failed to get cached cosmetic files:', e);
        // Leave the flag false so the next cosmetic resolve retries. A transient
        // failure here (e.g. the shared Rust file cache contended while another
        // window is also hitting it) used to poison the cache as {} forever,
        // which left 7TV paints/badges broken until an app restart. Whatever is
        // already memoized stays: it came from real downloads, not from disk.
      } finally {
        filesInitializationPromise = null;
      }
    })();
  }
  return filesInitializationPromise;
}

// What a chatter wears on 7TV. Rust batches, caches and resolves it for every
// window; this only points the result at cosmetics already on disk.
export async function getUserCosmetics(twitchId: string): Promise<UserCosmeticsResult> {
  const listing = ensureCosmeticFileListing();
  const result = await invoke<UserCosmeticsResult>('seventv_user_cosmetics', { id: twitchId }).catch((error) => {
    Logger.error('[7TV] Failed to fetch user cosmetics:', error);
    return { data: { paints: [], badges: [] }, hardFail: true } as UserCosmeticsResult;
  });
  if (listing) await listing;
  applyLocalFiles(result.data, cachedCosmeticFiles || {});
  return result;
}

/**
 * Drop this user's entry from the 7TV cosmetics cache so the next
 * getUserCosmetics call genuinely re-hits the API. Await it before that call:
 * the cache lives in Rust, and an un-awaited invalidate can land after the
 * lookup it was meant to precede.
 */
export function invalidateUserCosmeticsCache(twitchId: string): Promise<void> {
  return invoke<void>('seventv_invalidate_cosmetics', { id: twitchId }).catch(() => {});
}

/**
 * Everything a user OWNS, not just what they are wearing.
 *
 * The chat path deliberately resolves only active ids, so it cannot answer
 * "what does this account own". The cosmetics picker and the attainables
 * overlay genuinely need that, and they are opened one account at a time by a
 * human, so the heavy query is affordable exactly there and nowhere else.
 */
export async function fetchUserInventory(
  twitchId: string,
): Promise<UserCosmeticsResponse | null> {
  try {
    const owned = await invoke<UserCosmeticsResponse | null>('seventv_user_inventory', { id: twitchId });
    if (!owned) return null;
    const listing = ensureCosmeticFileListing();
    if (listing) await listing;
    return applyLocalFiles(owned, cachedCosmeticFiles || {});
  } catch (e) {
    Logger.warn('[7TV] inventory fetch failed:', e);
    return null;
  }
}

// Compute paint style layers
// (paint → CSS engine moved to ./paintStyle — see the re-exports near the top)

// Get badge image URL (7TV v4 badges need to be fetched from CDN).
// The .webp suffix is REQUIRED — 7TV's CDN serves animated badges as
// animated WebP at that path. Without the extension the CDN returns a
// default/static representation, breaking animation on badges like the
// year-streak crowns. See https://cdn.7tv.app/badge/<id>/<res>.webp
// Pick the best image URL from a V4 badge's images[] for a target scale. A
// badge's id is NOT its image id in V4, so these API-provided URLs are the only
// reliable source. Prefers the requested scale, the animated (non-_static)
// form, and webp > avif > png > gif.
const pickBadgeImage = (images: BadgeImageV4[] | undefined, scale: number): string | undefined => {
  if (!images?.length) return undefined;
  const rank = (img: BadgeImageV4): number => {
    let s = Math.abs((img.scale ?? 1) - scale) * 10;
    if (img.url.includes('_static')) s += 3;
    const m = img.mime ?? '';
    s += m.includes('webp') ? 0 : m.includes('avif') ? 1 : m.includes('png') ? 2 : 4;
    return s;
  };
  return [...images].sort((a, b) => rank(a) - rank(b))[0]?.url;
};

export const getBadgeImageUrl = (badge: BadgeV4): string => {
  if (badge.localUrl) return badge.localUrl;
  return pickBadgeImage(badge.images, 4) ?? `https://cdn.7tv.app/badge/${badge.id}/4x.webp`;
};

// Get all resolution URLs for a 7TV badge (for srcSet)
export const getBadgeImageUrls = (badge: BadgeV4): { url1x: string; url2x: string; url3x: string; url4x: string } => {
  if (badge.localUrl) {
    // If we have a local URL, use it for all resolutions
    return { url1x: badge.localUrl, url2x: badge.localUrl, url3x: badge.localUrl, url4x: badge.localUrl };
  }
  const legacy = `https://cdn.7tv.app/badge/${badge.id}`;
  return {
    url1x: pickBadgeImage(badge.images, 1) ?? `${legacy}/1x.webp`,
    url2x: pickBadgeImage(badge.images, 2) ?? `${legacy}/2x.webp`,
    url3x: pickBadgeImage(badge.images, 3) ?? `${legacy}/3x.webp`,
    url4x: pickBadgeImage(badge.images, 4) ?? `${legacy}/4x.webp`,
  };
};

// Get badge URLs with fallback priority (highest to lowest resolution)
// Used when 4x may 404 - tries 3x, 2x, 1x as fallbacks
export const getBadgeFallbackUrls = (badgeId: string): string[] => {
  const baseUrl = `https://cdn.7tv.app/badge/${badgeId}`;
  return [
    `${baseUrl}/4x.webp`,
    `${baseUrl}/3x.webp`,
    `${baseUrl}/2x.webp`,
    `${baseUrl}/1x.webp`,
  ];
};

export function clearUserCache() {
  void invoke('seventv_clear_cosmetics').catch(() => {});
  // Also clear the file cache so it re-fetches. Both halves: the map itself and
  // the flag that says we have read the disk listing.
  cachedCosmeticFiles = null;
  cosmeticFilesLoadedFromDisk = false;
  forgetAssetRequests('cosmetic');
}
