// Service for fetching 7TV badges and paints using the v4 GraphQL API
import { invoke } from '@tauri-apps/api/core';
import { convertFileSrc } from '@tauri-apps/api/core';

import { Logger } from '../utils/logger';
import { features } from '../features';
import { LruMap } from './cosmeticsCache';
import { IS_MOBILE } from '../utils/platform';
// The paint → CSS engine + its v4 paint types now live in the standalone,
// Tauri-free `paintStyle` module so the hosted overlay page can share the exact
// same rendering. Re-exported here so existing importers stay unchanged.
import type { PaintV4 } from './paintStyle';
export { computePaintStyle } from './paintStyle';
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

// Cache for 7TV user data.
//   - Successful lookups (on 7TV with or without inventory, or genuinely not
//     on 7TV) ride the full TTL: they are stable answers.
//   - Hard failures (network error, 5xx, retry-exhausted) get a much shorter
//     TTL so a transient 7TV blip can't strand a real user without a paint
//     for 5 minutes. The next request retries.
// Bounded, because this used to be a plain Map that only ever grew: the TTL
// below decides FRESHNESS, and an expired entry is overwritten rather than
// deleted, so a long session in a busy channel accumulated one entry per unique
// chatter forever. Each one holds that user's whole 7TV inventory.
//
// A size cap is safe precisely BECAUSE of the TTL: anything older than
// CACHE_DURATION is re-fetched on read anyway, so evicting a cold entry can
// only ever discard something the next read would have thrown away. The cap is
// set well above a realistic 5-minute working set so it never causes refetches
// that the TTL would not already have caused.
//
// Note for anyone tempted to shrink these entries instead: the full inventory
// is load-bearing. The pickers map over it to change a selection, and the
// public profile overlay reports a paint COUNT for other users, both fed from
// this same data via getFullProfileWithFallback.
const userCache = new LruMap<string, { data: UserCosmeticsResponse; hardFail: boolean; timestamp: number }>(
  IS_MOBILE ? 1500 : 4000,
);
const CACHE_DURATION = 5 * 60 * 1000;
const HARD_FAIL_CACHE_DURATION = 30 * 1000;

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

// Track pending requests to prevent duplicate fetches
const pendingRequests = new Map<string, Promise<UserCosmeticsResult>>();
let filesInitializationPromise: Promise<void> | null = null;

// GraphQL query fragments
const fullPaintQueryFields = /* GraphQL */ `
  {
    id
    name
    description
    data {
      layers {
        id
        ty {
          ... on PaintLayerTypeImage {
            __typename
            images {
              __typename
              url
              mime
              size
              scale
              width
              height
              frameCount
            }
          }
          ... on PaintLayerTypeRadialGradient {
            __typename
            repeating
            shape
            stops {
              at
              color {
                __typename
                hex
                r
                g
                b
                a
              }
            }
          }
          ... on PaintLayerTypeLinearGradient {
            __typename
            angle
            repeating
            stops {
              __typename
              at
              color {
                __typename
                hex
                r
                g
                b
                a
              }
            }
          }
          ... on PaintLayerTypeSingleColor {
            __typename
            color {
              __typename
              hex
              r
              g
              b
              a
            }
          }
        }
        opacity
      }
      shadows {
        __typename
        offsetX
        offsetY
        blur
        color {
          __typename
          hex
          r
          g
          b
          a
        }
      }
    }
  }
`;

const fullBadgeQueryFields = /* GraphQL */ `
  {
    id
    name
    description
    images {
      url
      mime
      scale
      frameCount
    }
  }
`;

// Field selection for `userByConnection`. Reused by the batched drain to build
// aliased multi-user queries.
const fullUserSelection = /* GraphQL */ `{
  id
  style {
    activePaint { id }
    activeBadge { id description }
  }
  inventory {
    paints {
      to {
        paint ${fullPaintQueryFields}
      }
    }
    badges {
      to {
        badge ${fullBadgeQueryFields}
      }
    }
  }
}`;

// What CHAT needs from a user: which cosmetics are they WEARING. The definitions
// themselves come from the shared catalog below, not from this query.
const activeOnlyUserSelection = /* GraphQL */ `{
  id
  style {
    activePaint { id }
    activeBadge { id description }
  }
}`;

// ---------------------------------------------------------------------------
// Shared cosmetic DEFINITIONS catalog.
//
// Paints and badges are a small, shared, PUBLIC set: measured 2026-08-29 at
// 1013 paints + 127 badges, ~1MB, one request, 1.9s. Nothing about it is
// per-user, so it is cached once and reused for every chatter.
//
// This exists because the per-user query used to carry a full definition for
// every cosmetic the user OWNED in order to render the single one they were
// wearing. A chatter with fifty paints shipped fifty definitions, of which
// forty-nine were discarded, and that payload is what pinned the batch size:
// each user scored ~71 against 7TV's ~400 query-complexity ceiling, so only
// five fitted in a request.
const CATALOG_KEY = 'sn-7tv-cosmetic-catalog-v1';
// Definitions change when 7TV ships new cosmetics, which is not often. A stale
// entry costs nothing: an id we do not recognise triggers a refresh below.
const CATALOG_TTL_MS = 24 * 60 * 60 * 1000;

const paintCatalog = new Map<string, PaintV4>();
const badgeCatalog = new Map<string, BadgeV4>();
let catalogPromise: Promise<void> | null = null;
let catalogFetchedAt = 0;
let catalogRefreshInFlight = false;

const catalogQuery = /* GraphQL */ `{
  paints { paints ${fullPaintQueryFields} }
  badges { badges ${fullBadgeQueryFields} }
}`;

function seedCatalogFromStorage(): boolean {
  try {
    const raw = localStorage.getItem(CATALOG_KEY);
    if (!raw) return false;
    const parsed = JSON.parse(raw) as { at?: number; paints?: PaintV4[]; badges?: BadgeV4[] };
    if (!parsed?.paints?.length) return false;
    parsed.paints.forEach((p) => paintCatalog.set(p.id, p));
    parsed.badges?.forEach((b) => badgeCatalog.set(b.id, b));
    catalogFetchedAt = parsed.at ?? 0;
    return true;
  } catch {
    return false;
  }
}

async function fetchCatalog(): Promise<void> {
  const response = await requestGql({ query: catalogQuery });
  const paints = response?.data?.paints?.paints;
  const badges = response?.data?.badges?.badges;
  // A degraded response must never empty a catalog we already have: a chatter
  // resolving against nothing renders with no paint at all, which looks like
  // their cosmetic was removed.
  if (!Array.isArray(paints) || paints.length === 0) return;
  paintCatalog.clear();
  paints.forEach((p: PaintV4) => paintCatalog.set(p.id, p));
  if (Array.isArray(badges) && badges.length > 0) {
    badgeCatalog.clear();
    badges.forEach((b: BadgeV4) => badgeCatalog.set(b.id, b));
  }
  catalogFetchedAt = Date.now();
  // Loud on purpose. This should fire ONCE per launch (or once a day). If it
  // repeats, an id we keep failing to find is driving a refetch loop, and each
  // pass parses ~1MB and rewrites the same to localStorage.
  Logger.info(`[7TV] cosmetic catalog fetched: ${paints.length} paints, ${badges?.length ?? 0} badges`);
  try {
    localStorage.setItem(
      CATALOG_KEY,
      JSON.stringify({ at: catalogFetchedAt, paints, badges: badges ?? [] }),
    );
  } catch {
    // Over quota, or storage disabled. The in-memory catalog still serves this
    // session; the next launch just pays the fetch again.
  }
}

/** Resolve once per session. Disk-seeded launches never wait on the network. */
function ensureCatalog(): Promise<void> {
  if (catalogPromise) return catalogPromise;
  const seeded = seedCatalogFromStorage();
  const fresh = seeded && Date.now() - catalogFetchedAt < CATALOG_TTL_MS;
  catalogPromise = fresh
    ? Promise.resolve()
    : fetchCatalog().catch((e) => {
        Logger.warn('[7TV] cosmetic catalog fetch failed:', e);
      });
  return catalogPromise;
}

/** An id we have never seen means 7TV shipped a cosmetic since our snapshot. */
function refreshCatalogForUnknownId(id: string): void {
  if (catalogRefreshInFlight) return;
  catalogRefreshInFlight = true;
  Logger.info(`[7TV] cosmetic id ${id} missing from catalog; refreshing`);
  void fetchCatalog()
    .catch(() => {})
    .finally(() => {
      catalogRefreshInFlight = false;
    });
}

// Remove newlines and extra spaces from GraphQL query
const cleanQuery = (query: string): string => {
  return query.replace(/\n/g, '').replace(/\s+/g, ' ');
};

// GraphQL response type from Rust backend
interface GraphQLResponse {
  data?: any;
  errors?: any[];
  message?: string;
}

// Request GraphQL API with retry logic via Tauri backend (bypasses CORS)
const requestGql = async ({ query }: { query: string }): Promise<any> => {
  let retryCount = 0;
  while (retryCount <= 5) {
    try {
      const response = await invoke('seventv_graphql', { query: cleanQuery(query) }) as GraphQLResponse;

      if (response.errors || response.message) {
        // [7TV-diag] Surface the real error on EVERY attempt (it was only logged
        // after 5 retries) to confirm whether MultiChat load makes 7TV reject or
        // rate-limit the core app's cosmetic queries. Temporary diagnostic.
        Logger.warn(`[7TV-diag] gql error (attempt ${retryCount}):`, response.message || response.errors);
        if (retryCount === 5) {
          Logger.error('[7TV] Error fetching user cosmetics:', response.errors || response.message);
          return undefined;
        }
        await new Promise((r) => setTimeout(r, 500));
        retryCount++;
        continue;
      }

      return response;
    } catch (error) {
      if (retryCount === 5) {
        Logger.error('[7TV] Network error:', error);
        return undefined;
      }
      await new Promise((r) => setTimeout(r, 500));
      retryCount++;
    }
  }
  return undefined;
};

// Track pending cosmetic downloads to prevent duplicates
const pendingCosmeticDownloads = new Map<string, Promise<string | null>>();

// Queue system for cosmetic downloads (matches emoteService/badgeImageCacheService pattern)
const MAX_CONCURRENT_COSMETIC_DOWNLOADS = 5;
const cosmeticDownloadQueue: Array<{ id: string, url: string }> = [];
let activeCosmeticDownloads = 0;

// Settings cache for cosmetics
let cosmeticCacheSettings: { enabled: boolean; expiryDays: number } | null = null;

async function getCosmeticCacheSettings(): Promise<{ enabled: boolean; expiryDays: number }> {
  if (cosmeticCacheSettings) return cosmeticCacheSettings;
  try {
    const settings = await invoke('load_settings') as any;
    cosmeticCacheSettings = {
      enabled: settings.cache?.enabled !== false,
      expiryDays: settings.cache?.expiry_days ?? 7
    };
    return cosmeticCacheSettings;
  } catch (e) {
    Logger.warn('[7TVService] Failed to load settings:', e);
    return { enabled: true, expiryDays: 7 };
  }
}

async function processCosmeticDownloadQueue() {
  if (activeCosmeticDownloads >= MAX_CONCURRENT_COSMETIC_DOWNLOADS || cosmeticDownloadQueue.length === 0) {
    return;
  }

  const next = cosmeticDownloadQueue.shift();
  if (!next) return;

  activeCosmeticDownloads++;

  try {
    await downloadCosmeticIfNeeded(next.id, next.url);
  } catch (e) {
    Logger.debug(`[7TVService] Error processing queue item ${next.id}:`, e);
  } finally {
    activeCosmeticDownloads--;
    processCosmeticDownloadQueue();
  }
}

// Lazy download a single cosmetic on-demand
async function downloadCosmeticIfNeeded(id: string, url: string): Promise<string | null> {
  // Already cached?
  if (cachedCosmeticFiles && cachedCosmeticFiles[id]) {
    return cachedCosmeticFiles[id];
  }

  // Already downloading?
  if (pendingCosmeticDownloads.has(id)) {
    return pendingCosmeticDownloads.get(id)!;
  }

  if (!features.assetDiskCache) return null;

  const settings = await getCosmeticCacheSettings();
  if (!settings.enabled) return null;

  // Start download
  const downloadPromise = (async () => {
    try {
      const localPath = await invoke('download_and_cache_file', {
        cacheType: 'cosmetic',
        id,
        url,
        expiryDays: settings.expiryDays
      }) as string;

      if (localPath) {
        // Record it even if the disk listing has not arrived yet. This used to
        // be gated on cachedCosmeticFiles being non-null, so a download that
        // won that race was cached on disk, reported as a miss, and queued
        // again on the next render.
        (cachedCosmeticFiles ??= {})[id] = localPath;
        return localPath;
      }
      return null;
    } catch (e) {
      Logger.debug(`[7TVService] Failed to cache cosmetic ${id}:`, e);
      return null;
    } finally {
      pendingCosmeticDownloads.delete(id);
    }
  })();

  pendingCosmeticDownloads.set(id, downloadPromise);
  return downloadPromise;
}

// Queue a cosmetic for lazy caching - called when actually displayed
export function queueCosmeticForCaching(id: string, url: string) {
  if ((cachedCosmeticFiles && cachedCosmeticFiles[id]) || pendingCosmeticDownloads.has(id)) {
    return;
  }
  if (cosmeticDownloadQueue.some(item => item.id === id)) {
    return;
  }
  cosmeticDownloadQueue.push({ id, url });
  processCosmeticDownloadQueue();
}

// Parse a single user's GraphQL payload (the `userByConnection` value) into
// the frontend-facing cosmetics shape. Extracted so both single-fetch and
// batched-fetch paths can share it.
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

/**
 * Build a chatter's cosmetics from their ACTIVE ids plus the shared catalog.
 *
 * Returns the same shape as `parseUserCosmetics`, carrying only the worn
 * cosmetic. Safe because every consumer selects with `.find(x => x.selected)`
 * and none iterates the inventory; the surfaces that genuinely list what a user
 * OWNS go through `fetchUserInventory` instead.
 *
 * Definitions are CLONED out of the catalog. It is shared across every chatter,
 * so stamping `selected` or a `localUrl` onto the stored object would leak one
 * user's render state onto everyone else wearing the same paint.
 */
function buildCosmeticsFromCatalog(
  data: any,
  cachedFiles: Record<string, string>,
): UserCosmeticsResponse {
  const seventvUserId = data.id;
  const activePaintId = data.style?.activePaint?.id;
  const activeBadge = data.style?.activeBadge;

  const paints: PaintV4[] = [];
  if (activePaintId) {
    const def = paintCatalog.get(activePaintId);
    if (def) {
      const paintData: any = structuredClone(def);
      paintData.selected = true;
      applyPaintLocalFiles(paintData, cachedFiles);
      paints.push(paintData);
    } else {
      refreshCatalogForUnknownId(activePaintId);
    }
  }

  const badges: BadgeV4[] = [];
  if (activeBadge?.id) {
    const def = badgeCatalog.get(activeBadge.id);
    if (!def) refreshCatalogForUnknownId(activeBadge.id);
    // With no definition the badge still renders: getBadgeImageUrl falls back to
    // the CDN path built from the id, so an unknown badge shows art rather than
    // disappearing while the catalog refreshes.
    const badgeData: any = def
      ? structuredClone(def)
      : { id: activeBadge.id, name: '', description: activeBadge.description ?? '', images: [] };
    badgeData.selected = true;
    const localPath = cachedFiles[badgeData.id];
    if (localPath) badgeData.localUrl = convertFileSrc(localPath);
    badges.push(badgeData);
  }

  return { paints, badges, seventvUserId };
}

function parseUserCosmetics(
  data: any,
  cachedFiles: Record<string, string>,
): UserCosmeticsResponse {
  const seventvUserId = data.id;
  const activePaintId = data.style?.activePaint?.id;
  const activeBadgeId = data.style?.activeBadge?.id;

  const paints: PaintV4[] = [];
  for (const paint of data.inventory?.paints ?? []) {
    if (paint.to?.paint) {
      const paintData = paint.to.paint;
      if (paintData.id === activePaintId) {
        paintData.selected = true;
      }
      if (paintData.data?.layers) {
        for (const layer of paintData.data.layers) {
          if (layer.ty.__typename === 'PaintLayerTypeImage' && layer.ty.images) {
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
      paints.push(paintData);
    }
  }

  const badges: BadgeV4[] = [];
  for (const badge of data.inventory?.badges ?? []) {
    const badgeData = badge.to?.badge;
    if (!badgeData) continue;
    if (badgeData.id === activeBadgeId) {
      badgeData.selected = true;
    }
    const localPath = cachedFiles[badgeData.id];
    if (localPath) {
      badgeData.localUrl = convertFileSrc(localPath);
    }
    badges.push(badgeData);
  }

  return {
    paints: paints.filter((p) => p !== null),
    badges: badges.filter((b) => b !== null),
    seventvUserId,
  };
}

// Batch coordinator: collects twitchIds requested inside a short window and
// fires a single aliased GraphQL query. A flood of 50 new chatters (hype train,
// channel switch, replay) collapses from 50 parallel HTTP round-trips into one.
//
// The window is a TIMER, not a microtask. A microtask drains at the end of the
// CURRENT task, and live chat delivers one message per task, so every live
// chatter landed in a batch of exactly one and paid a full round-trip of their
// own. On a large channel the diag line below read "resolved 1/1" for hundreds
// of consecutive chunks and never once read 5/5. Scrollback still drains at
// end-of-tick, because those rows render together and trip the size threshold.
//
// 7TV's `users.userByConnection` is a per-user field, so we use GraphQL
// aliasing (`u_<id>: users { userByConnection(...) { ... } }`) to multiplex N
// users into one request. Chunked at BATCH_MAX_SIZE to stay under 7TV's
// server-side query-complexity limit (about 400). Over the limit is rejected
// outright with "Query is too complex." and the ENTIRE batch returns null,
// stranding every user in it without cosmetics.
//
// This was 5, because each user then carried the full paint field selection and
// scored about 71. Asking only for the ACTIVE ids and resolving definitions from
// the shared catalog drops that to roughly a tenth. Ceiling re-measured against
// live 7TV on 2026-08-29 with the active-only selection: 40 passed, 45 was
// rejected. 30 keeps a quarter of that headroom.
//
// Do NOT raise this without re-measuring, and do NOT put the full definition
// selection back into the per-user query without lowering it again.
const BATCH_MAX_SIZE = 30;
// Cap parallel in-flight chunks so an extreme cold-start burst (e.g.
// scrollback dump from a 10k-viewer hype-train channel join → 40+ chunks)
// doesn't fire dozens of concurrent HTTP requests at 7TV. Five is the
// sweet spot: 125 users in flight at any moment is plenty for snappy
// resolution while staying polite to the upstream API. Bigger drains
// process the rest in subsequent waves, still way faster than the
// pre-fix sequential O(N/25).
const MAX_PARALLEL_CHUNKS = 5;
type CosmeticsResolver = (data: UserCosmeticsResponse | null) => void;
const batchQueue = new Map<string, CosmeticsResolver[]>();
let batchScheduled = false;

// Cosmetics ids are namespaced by platform for non-Twitch chatters (e.g.
// "kick:12345"); a 7TV account's cosmetics are user-level and resolve from any
// linked platform. Twitch stays a BARE numeric id, so its query, cache key, and
// alias are all byte-identical to before — the no-prefix path is unchanged.
const cosmeticPlatform = (id: string): { platform: string; platformId: string } => {
  if (id.startsWith('kick:')) return { platform: 'KICK', platformId: id.slice(5) };
  // YouTube was falling through to the TWITCH arm, so a `youtube:UC…` id was sent
  // to 7TV as a Twitch user id — never a match, and a wasted lookup on every
  // YouTube chatter.
  // 7TV names this platform GOOGLE, not YOUTUBE. Verified against the live v4
  // schema: the Platform enum is TWITCH, DISCORD, GOOGLE, KICK, and sending
  // YOUTUBE fails the whole query with "enumeration type Platform does not
  // contain the value YOUTUBE" — so YouTube chatters resolved NO cosmetics at all.
  if (id.startsWith('youtube:')) return { platform: 'GOOGLE', platformId: id.slice(8) };
  return { platform: 'TWITCH', platformId: id };
};

// GraphQL aliases must match /[_A-Za-z][_0-9A-Za-z]*/, so a "kick:123" id can't be
// used raw. Sanitize to a stable token used identically when building the query
// and when reading the response back. Bare numeric Twitch ids are unaffected.
const cosmeticAlias = (id: string): string => `u_${id.replace(/[^a-zA-Z0-9_]/g, '_')}`;

// How long ids accumulate before a drain. Cosmetics paint onto a row that has
// already rendered, so this is invisible to the reader, and it is the whole
// difference between one request per chatter and one request per chunk.
const BATCH_WINDOW_MS = 150;
// One full parallel wave. A queue this size is already worth sending, so waiting
// out the timer would only delay it.
const BATCH_DRAIN_THRESHOLD = MAX_PARALLEL_CHUNKS * BATCH_MAX_SIZE;
let batchTimer: ReturnType<typeof setTimeout> | null = null;

const scheduleDrain = (immediate: boolean) => {
  if (immediate) {
    if (batchTimer !== null) {
      // A pending timer is the only thing that scheduled this drain, so cancel
      // it and clear the flag before re-scheduling, or nothing would drain.
      clearTimeout(batchTimer);
      batchTimer = null;
      batchScheduled = false;
    }
    if (!batchScheduled) {
      batchScheduled = true;
      queueMicrotask(drainBatch);
    }
    return;
  }
  if (batchScheduled) return;
  batchScheduled = true;
  batchTimer = setTimeout(() => {
    batchTimer = null;
    void drainBatch();
  }, BATCH_WINDOW_MS);
};

const requestUserCosmeticsBatched = (
  twitchId: string,
): Promise<UserCosmeticsResponse | null> => {
  return new Promise((resolve) => {
    let resolvers = batchQueue.get(twitchId);
    if (!resolvers) {
      resolvers = [];
      batchQueue.set(twitchId, resolvers);
    }
    resolvers.push(resolve);
    scheduleDrain(batchQueue.size >= BATCH_DRAIN_THRESHOLD);
  });
};

const drainBatch = async () => {
  batchScheduled = false;
  if (batchQueue.size === 0) return;

  // Wait for the cosmetic file cache to finish loading before reading it below,
  // so a batch that drains mid-init doesn't render image paints/badges without
  // their local files.
  if (filesInitializationPromise) await filesInitializationPromise;
  // Definitions must be in hand before ids can be resolved into cosmetics. Costs
  // nothing on a disk-seeded launch, and one fetch on a cold one.
  await ensureCatalog();

  // Snapshot the queue and clear it so new requests during this drain start a
  // fresh batch (will get their own microtask).
  const snapshot = new Map(batchQueue);
  batchQueue.clear();

  const cachedFiles = cachedCosmeticFiles || {};
  const ids = Array.from(snapshot.keys());

  // Build the chunk list, then run them through a worker-pool so at most
  // MAX_PARALLEL_CHUNKS are in flight at once. Previously this awaited each
  // chunk in sequence, so a flood of new chatters at channel-join paid
  // O(N/25) sequential round-trips before the last user's paint resolved.
  // Unbounded parallelism would resolve fastest but risks 429s from 7TV on
  // extreme bursts; the worker-pool gets most of the win while staying
  // polite to the upstream API.
  const chunks: string[][] = [];
  for (let i = 0; i < ids.length; i += BATCH_MAX_SIZE) {
    chunks.push(ids.slice(i, i + BATCH_MAX_SIZE));
  }

  const runChunk = async (chunk: string[]) => {
    const query = `{ ${chunk
      .map((id) => {
        const { platform, platformId } = cosmeticPlatform(id);
        return `${cosmeticAlias(id)}: users { userByConnection(platform: ${platform}, platformId: "${platformId}") ${activeOnlyUserSelection} }`;
      })
      .join(' ')} }`;

    try {
      const response = await requestGql({ query });

      // requestGql swallows errors and returns undefined after exhausting its
      // retries (network error, 5xx, or 7TV rejecting the query outright, e.g.
      // "Query is too complex."). A missing data payload is a HARD FAILURE for
      // the whole chunk, not a confirmed "these users have no cosmetics."
      // Resolve null so getUserCosmetics marks the entry hardFail (short TTL)
      // and self-heals on the next read, instead of caching a bogus empty for
      // the full 5-minute TTL and stranding everyone in the chunk.
      if (!response?.data) {
        // [7TV-diag] 200 OK but no data payload = a degraded/empty response.
        Logger.warn(`[7TV-diag] chunk HARD FAIL (200, no data) for ${chunk.length} user(s)`);
        for (const id of chunk) {
          snapshot.get(id)?.forEach((r) => r(null));
        }
        return;
      }

      let resolved = 0;
      for (const id of chunk) {
        const userByConnection = response.data[cosmeticAlias(id)]?.userByConnection;
        if (userByConnection) resolved++;
        const result = userByConnection
          ? buildCosmeticsFromCatalog(userByConnection, cachedFiles)
          : { paints: [], badges: [], seventvUserId: undefined };
        snapshot.get(id)?.forEach((r) => r(result));
      }
      // [7TV-diag] If this drops to 0/N while MultiChat is open, 7TV is silently
      // returning empty user entries (soft throttling) rather than erroring.
      Logger.debug(`[7TV-diag] chunk resolved ${resolved}/${chunk.length} user(s) with a 7TV connection`);
    } catch (error) {
      Logger.error('[7TV] Batch cosmetics fetch failed:', error);
      for (const id of chunk) {
        snapshot.get(id)?.forEach((r) => r(null));
      }
    }
  };

  let chunkIdx = 0;
  const worker = async () => {
    while (chunkIdx < chunks.length) {
      const myIdx = chunkIdx++;
      await runChunk(chunks[myIdx]);
    }
  };
  const workerCount = Math.min(MAX_PARALLEL_CHUNKS, chunks.length);
  await Promise.all(Array.from({ length: workerCount }, worker));
};

// Fetch user cosmetics from 7TV v4 API.
// Content-first: lazy cache initialization in the background; cached URLs are
// used only if already in memory. Requests for distinct users in the same tick
// are batched into one HTTP round-trip via requestUserCosmeticsBatched above.
export async function getUserCosmetics(twitchId: string): Promise<UserCosmeticsResult> {
  const cached = userCache.get(twitchId);
  const now = Date.now();

  if (cached) {
    const ttl = cached.hardFail ? HARD_FAIL_CACHE_DURATION : CACHE_DURATION;
    if ((now - cached.timestamp) < ttl) {
      return { data: cached.data, hardFail: cached.hardFail };
    }
  }

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

  const pending = pendingRequests.get(twitchId);
  if (pending) {
    return pending;
  }

  const request: Promise<UserCosmeticsResult> = (async () => {
    try {
      // requestUserCosmeticsBatched returns null only for hard failures
      // (network error, retry-exhausted batch query). Successful responses
      // — including ones that legitimately say "this user has no 7TV
      // account / no inventory" — return a UserCosmeticsResponse with
      // empty arrays + seventvUserId undefined.
      const result = await requestUserCosmeticsBatched(twitchId);
      if (result === null) {
        const empty: UserCosmeticsResponse = { paints: [], badges: [], seventvUserId: undefined };
        userCache.set(twitchId, { data: empty, hardFail: true, timestamp: now });
        return { data: empty, hardFail: true };
      }
      userCache.set(twitchId, { data: result, hardFail: false, timestamp: now });
      return { data: result, hardFail: false };
    } catch (error) {
      Logger.error('[7TV] Failed to fetch user cosmetics:', error);
      const empty: UserCosmeticsResponse = { paints: [], badges: [], seventvUserId: undefined };
      userCache.set(twitchId, { data: empty, hardFail: true, timestamp: now });
      return { data: empty, hardFail: true };
    } finally {
      pendingRequests.delete(twitchId);
    }
  })();

  pendingRequests.set(twitchId, request);
  return request;
}

/**
 * Drop this user's entry from the low-level 7TV cosmetics cache so the next
 * getUserCosmetics call genuinely re-hits the API. The cosmeticsCache-layer
 * invalidate does NOT reach this map, so a poisoned success-empty (e.g. an
 * app-mount prefetch that raced 7TV's warmup, cached hardFail=false for the
 * full CACHE_DURATION) would otherwise keep being served even after a
 * "force refresh". Pairs with cosmeticsCache.forceRefreshCosmetics.
 */
export function invalidateUserCosmeticsCache(twitchId: string): void {
  userCache.delete(twitchId);
}

/**
 * Everything a user OWNS, not just what they are wearing.
 *
 * The batched chat path deliberately fetches only active ids, so it cannot
 * answer "what does this account own". The cosmetics picker and the attainables
 * overlay genuinely need that, and they are opened one account at a time by a
 * human, so the heavy query is affordable exactly there and nowhere else.
 */
export async function fetchUserInventory(
  twitchId: string,
): Promise<UserCosmeticsResponse | null> {
  const { platform, platformId } = cosmeticPlatform(twitchId);
  const query = `{ ${cosmeticAlias(twitchId)}: users { userByConnection(platform: ${platform}, platformId: "${platformId}") ${fullUserSelection} } }`;
  try {
    const response = await requestGql({ query });
    const user = response?.data?.[cosmeticAlias(twitchId)]?.userByConnection;
    if (!user) return null;
    if (filesInitializationPromise) await filesInitializationPromise;
    return parseUserCosmetics(user, cachedCosmeticFiles || {});
  } catch (e) {
    Logger.warn('[7TV] inventory fetch failed:', e);
    return null;
  }
}

// Legacy function for backwards compatibility — unwraps to the historical
// `UserCosmeticsResponse | null` shape (null = hard failure).
export async function fetch7TVUserData(twitchUserId: string): Promise<UserCosmeticsResponse | null> {
  const { data, hardFail } = await getUserCosmetics(twitchUserId);
  return hardFail ? null : data;
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

// Get badge image URL for any provider
export const getBadgeImageUrlForProvider = (badge: any, provider: '7tv' | 'ffz'): string => {
  if (provider === '7tv') {
    if (badge.localUrl) return badge.localUrl;
    return pickBadgeImage(badge.images, 3) ?? `https://cdn.7tv.app/badge/${badge.id}/3x.webp`;
  } else if (provider === 'ffz') {
    return badge.urls?.['4'] || badge.urls?.['2'] || badge.urls?.['1'] || badge.image;
  }
  return '';
};

export function clearUserCache() {
  userCache.clear();
  // Also clear the file cache so it re-fetches. Both halves: the map itself and
  // the flag that says we have read the disk listing.
  cachedCosmeticFiles = null;
  cosmeticFilesLoadedFromDisk = false;
}
