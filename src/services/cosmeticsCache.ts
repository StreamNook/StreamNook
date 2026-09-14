import { Logger } from '../utils/logger';
// Service for caching cosmetics - user lookups in memory, image files on disk
// User -> cosmetics mappings come from APIs fresh each time
// Image files are cached to disk by their respective services (seventvService, thirdPartyBadges)

interface CachedCosmetics {
  paints: any[];
  badges: any[];
  seventvUserId?: string;
}

// Full profile cache structure - includes all badge types
export interface CachedProfile {
  userId: string;
  username: string;
  channelId?: string;
  channelName?: string;
  twitchBadges: any[];
  seventvCosmetics: CachedCosmetics;
  thirdPartyBadges: any[];
  lastUpdated: number;
}

// Small bounded LRU. Touch-on-hit keeps recently-used entries warm; least-recently
// used falls off when size exceeds maxSize. Designed to match the Map subset this
// module needs (get / set / clear / size). Exported so other services (emojiService
// for one) can reuse the same shape rather than re-implementing it.
export class LruMap<K, V> {
  private map = new Map<K, V>();
  constructor(private maxSize: number) {}
  get(key: K): V | undefined {
    if (!this.map.has(key)) return undefined;
    const v = this.map.get(key)!;
    this.map.delete(key);
    this.map.set(key, v);
    return v;
  }
  set(key: K, value: V): void {
    if (this.map.has(key)) this.map.delete(key);
    this.map.set(key, value);
    if (this.map.size > this.maxSize) {
      const oldest = this.map.keys().next().value;
      if (oldest !== undefined) this.map.delete(oldest);
    }
  }
  has(key: K): boolean { return this.map.has(key); }
  delete(key: K): boolean { return this.map.delete(key); }
  clear(): void { this.map.clear(); }
  get size(): number { return this.map.size; }
}

// Self-healing on transient 7TV failures. A "hard fail" (network error /
// 5xx / retry-exhausted) used to live in cache for the whole session,
// stranding the user without a paint until restart. We now mark hard-fail
// results with a timestamp and re-fetch on the next read after the TTL.
// Successful responses (including legitimate "user has no 7TV" answers) do
// NOT get this mark — they ride the normal LRU indefinitely, since they're
// stable answers that don't need to be re-asked.
const HARD_FAIL_RETRY_MS = 30_000;
const hardFailTimestamps = new Map<string, number>();

const isHardFailExpired = (userId: string): boolean => {
  const t = hardFailTimestamps.get(userId);
  return t !== undefined && Date.now() - t > HARD_FAIL_RETRY_MS;
};

const hasHardFailMark = (userId: string): boolean => hardFailTimestamps.has(userId);

// Public read so callers can distinguish "we got a real answer from 7TV"
// (paint, or a genuine no-paint) from "the last fetch hard-failed." Chat's
// addUser uses it to avoid stamping a null paint after a transient failure —
// a null would trip the addUser fast-path and strand the user paint-less for
// the whole session, defeating the 30s self-heal above.
export const isUserCosmeticsHardFailed = (userId: string): boolean => hasHardFailMark(userId);

// Reactive subscription so anything that depends on a user's cosmetics
// (chatUserStore, future surfaces) gets a push when the cache is updated.
// Without this, a paint that arrived via a profile-card fetch was invisible
// to the chat row that had already cached a "no paint" answer earlier.
type CosmeticsListener = (userId: string, cosmetics: CachedCosmetics, hardFail: boolean) => void;
const cosmeticsListeners = new Set<CosmeticsListener>();

export function subscribeToCosmetics(listener: CosmeticsListener): () => void {
  cosmeticsListeners.add(listener);
  return () => { cosmeticsListeners.delete(listener); };
}

function publishCosmetics(userId: string, cosmetics: CachedCosmetics, hardFail = false): void {
  inMemoryCosmeticsCache.set(userId, cosmetics);
  // The short-TTL "retry me" mark only applies to hard failures (network /
  // 5xx / retry-exhausted). A successful response that says the user has
  // no 7TV cosmetics is a stable answer — full LRU TTL applies. Without
  // this distinction, every non-7TV user in chat would re-trigger fetches
  // every 30s from any caller that wasn't deduped by addUser's short-circuit.
  if (hardFail) {
    hardFailTimestamps.set(userId, Date.now());
  } else {
    hardFailTimestamps.delete(userId);
    // Persist OUR OWN accounts' cosmetics (never a hard-fail empty) so chat paints
    // the right paint/badge on frame one next launch, per account we've added.
    if (ownCosmeticAccounts.has(userId)) persistOwnCosmetics(userId);
  }
  for (const listener of cosmeticsListeners) {
    try { listener(userId, cosmetics, hardFail); } catch (e) { Logger.warn('[cosmeticsCache] listener threw:', e); }
  }
}

// Disk persistence of OUR OWN accounts' cosmetics (primary + every linked
// account). Keyed by account id so each account paints its own paint/badge
// instantly; other chatters are never persisted (they resolve from the network).
const OWN_COSMETICS_KEY = 'streamnook_own_cosmetics_v1';
const ownCosmeticAccounts = new Set<string>();

function readPersistedCosmetics(): Record<string, CachedCosmetics> {
  try {
    const raw = localStorage.getItem(OWN_COSMETICS_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw);
    return parsed && typeof parsed === 'object' ? (parsed as Record<string, CachedCosmetics>) : {};
  } catch {
    return {};
  }
}

function persistOwnCosmetics(userId: string): void {
  try {
    const cosmetics = inMemoryCosmeticsCache.get(userId);
    if (!cosmetics) return;
    const store = readPersistedCosmetics();
    store[userId] = cosmetics;
    localStorage.setItem(OWN_COSMETICS_KEY, JSON.stringify(store));
  } catch (e) {
    Logger.warn('[cosmeticsCache] persistOwnCosmetics failed:', e);
  }
}

/**
 * Register all of OUR account ids (primary + linked) and hydrate each one's
 * cosmetics from disk, so a cold launch paints every account's paint/badge on
 * frame one instead of after the fetch returns. Marks them as own so subsequent
 * publishes (the boot revalidate, a paint/badge change) write through. Seeds only
 * accounts not already resolved this session (an in-session value is fresher).
 */
export function registerOwnCosmeticAccounts(userIds: string[]): void {
  const store = readPersistedCosmetics();
  for (const userId of userIds) {
    if (!userId) continue;
    ownCosmeticAccounts.add(userId);
    const persisted = store[userId];
    if (persisted && !inMemoryCosmeticsCache.has(userId)) {
      publishCosmetics(userId, persisted, false);
    }
  }
}

/**
 * Drop a user's cached cosmetics so the next read fetches fresh.
 * Use when the user changes their own paint/badge — the LRU entry would
 * otherwise serve the stale selection until LRU eviction.
 */
export function invalidateUserCosmetics(userId: string): void {
  inMemoryCosmeticsCache.delete(userId);
  // The full-profile cache embeds the same paints/badges, so it must be cleared
  // too or the profile overlay keeps serving the old paint after a change.
  inMemoryProfileCache.delete(userId);
  hardFailTimestamps.delete(userId);
}

/**
 * Optimistically reflect the signed-in user's OWN paint/badge change without
 * waiting on 7TV's read-after-write propagation. The mutation lands on 7TV's
 * write path instantly, but its read API lags a few seconds, so an immediate
 * re-fetch pulls back the OLD selection and re-caches it (the user then sees no
 * change until a reload). Instead we flip the `selected` flag on the already
 * cached owned cosmetics (the chosen paint/badge is in the inventory we last
 * fetched) and republish, so chat rows + the profile card repaint at once.
 *
 * Pass only the field that changed: `{ paintId }` leaves badges untouched and
 * vice-versa. A null id means "unequipped" (nothing selected). Returns false if
 * nothing is cached for the user yet, so the caller can fall back to a fetch.
 */
export function applyLocalCosmeticSelection(
  userId: string,
  change: { paintId?: string | null; badgeId?: string | null },
): boolean {
  const current = inMemoryCosmeticsCache.get(userId);
  if (!current) return false;

  const next: CachedCosmetics = {
    ...current,
    paints:
      'paintId' in change
        ? current.paints.map((p) => ({ ...p, selected: p.id === change.paintId }))
        : current.paints,
    badges:
      'badgeId' in change
        ? current.badges.map((b) => ({ ...b, selected: b.id === change.badgeId }))
        : current.badges,
  };

  // The full-profile cache embeds the same cosmetics, so keep it in sync or the
  // profile overlay keeps serving the old selection after a change.
  const profile = inMemoryProfileCache.get(userId);
  if (profile) {
    inMemoryProfileCache.set(userId, {
      ...profile,
      seventvCosmetics: next,
      lastUpdated: Date.now(),
    });
  }

  publishCosmetics(userId, next);
  return true;
}

/**
 * Force a genuinely fresh cosmetics resolution: clear BOTH cache layers (this
 * module's LRU + the lower-level seventvService userCache) and re-fetch.
 *
 * Plain invalidateUserCosmetics only clears this layer, so a poisoned
 * success-empty in the seventvService cache (e.g. an early prefetch that raced
 * 7TV's warmup) survives its 5-minute TTL and gets re-served, defeating the
 * "force refresh" intent. Clearing both BEFORE the fetch, sequenced inside one
 * async fn (so there's no clear-then-read race), guarantees the next read
 * actually hits 7TV. The result publishes to listeners (chatUserStore et al.)
 * exactly like getCosmeticsWithFallback, so a repaint follows.
 */
export async function forceRefreshCosmetics(userId: string): Promise<CachedCosmetics> {
  inMemoryCosmeticsCache.delete(userId);
  inMemoryProfileCache.delete(userId);
  hardFailTimestamps.delete(userId);
  try {
    const { invalidateUserCosmeticsCache } = await import('./seventvService');
    invalidateUserCosmeticsCache(userId);
  } catch (e) {
    Logger.warn('[cosmeticsCache] forceRefresh: failed to clear 7TV cache:', e);
  }
  return getCosmeticsWithFallback(userId);
}

/**
 * Revalidate one of OUR accounts' cosmetics WITHOUT first blanking the cached
 * value. forceRefreshCosmetics clears the in-memory entry before re-fetching,
 * which opens a window where chat/profile read an empty cache and paint barebones.
 * For our own cosmetics at launch we keep the seeded (last-known) value on screen,
 * drop only the lower-level 7TV cache so the fetch is genuinely fresh, and publish
 * the result — overwriting in place — UNLESS it's a hard failure, so a transient
 * 7TV blip at launch can't strip our paint down to nothing.
 */
export async function revalidateOwnCosmetics(userId: string): Promise<void> {
  if (!userId) return;
  try {
    const { getUserCosmetics, invalidateUserCosmeticsCache } = await import('./seventvService');
    invalidateUserCosmeticsCache(userId);
    const { data, hardFail } = await getUserCosmetics(userId);
    if (!hardFail) publishCosmetics(userId, data, false);
  } catch (e) {
    Logger.warn('[cosmeticsCache] revalidateOwnCosmetics failed:', e);
  }
}

// Caps sized for a heavy viewing session. Profile entries are the largest
// (badges + paints + IVR), so cap them tighter. Cosmetic / badge entries are
// smaller — 512 covers ~all active chatters in a busy channel.
const inMemoryCosmeticsCache = new LruMap<string, CachedCosmetics>(512);
const inMemoryThirdPartyBadgesCache = new LruMap<string, any[]>(512);
const inMemoryTwitchBadgesCache = new LruMap<string, any[]>(512);
const inMemoryProfileCache = new LruMap<string, CachedProfile>(256);

// Drop fields that no cached-side consumer reads. BadgeDetailOverlay uses the
// full Twitch badge shape, but it receives `badge` from BadgesOverlay's own
// catalog, not from these per-user caches — so clickAction/clickUrl can be
// dropped here. `link` on third-party badges is defined upstream but never read.
const compactTwitchBadges = (badges: any[]): any[] =>
  badges.map(({ clickAction: _ca, clickUrl: _cu, ...rest }) => rest);
const compactThirdPartyBadges = (badges: any[]): any[] =>
  badges.map(({ link: _link, ...rest }) => rest);

// Track pending requests to prevent duplicate fetches
const pendingCosmeticsRequests = new Map<string, Promise<CachedCosmetics>>();
const pendingThirdPartyBadgesRequests = new Map<string, Promise<any[]>>();
const pendingTwitchBadgesRequests = new Map<string, Promise<any[]>>();
const pendingProfileRequests = new Map<string, Promise<CachedProfile>>();

/**
 * Get cosmetics from synchronous in-memory cache (instant, no async)
 * Returns null if not in memory cache - use this for initial state
 */
export function getCosmeticsFromMemoryCache(userId: string): CachedCosmetics | null {
  const cached = inMemoryCosmeticsCache.get(userId);
  if (!cached) return null;
  // Treat hard-failed entries as a miss once their retry TTL has elapsed,
  // so the caller (chat addUser, autocomplete, etc.) re-fetches on demand.
  if (hasHardFailMark(userId) && isHardFailExpired(userId)) {
    return null;
  }
  return cached;
}

/**
 * Get third-party badges from synchronous in-memory cache (instant, no async)
 * Returns null if not in memory cache - use this for initial state
 */
export function getThirdPartyBadgesFromMemoryCache(userId: string): any[] | null {
  return inMemoryThirdPartyBadgesCache.get(userId) || null;
}

/**
 * Get cosmetics for a user - memory cache -> API fetch with deduplication
 * Image caching is handled by the seventvService internally
 */
export async function getCosmeticsWithFallback(userId: string): Promise<CachedCosmetics> {
  // 1. Try in-memory cache first (instant, synchronous). A hard-failed entry
  //    past its retry TTL is treated as a miss so a transient 7TV failure
  //    earlier in the session doesn't strand the user without a paint.
  const memoryCached = inMemoryCosmeticsCache.get(userId);
  if (memoryCached && !(hasHardFailMark(userId) && isHardFailExpired(userId))) {
    return memoryCached;
  }

  // 2. Check if there's already a pending request for this user (dedupe)
  const pendingRequest = pendingCosmeticsRequests.get(userId);
  if (pendingRequest) {
    return pendingRequest;
  }

  // 3. Create a new request and track it
  const request = (async (): Promise<CachedCosmetics> => {
    try {
      // Fetch from API. seventvService returns a wrapper that flags hard
      // failures vs. successful responses (including legitimate empties).
      const { getUserCosmetics } = await import('./seventvService');
      const { data, hardFail } = await getUserCosmetics(userId);

      // Publish so listeners (chatUserStore et al.) react.
      publishCosmetics(userId, data, hardFail);

      return data;
    } finally {
      pendingCosmeticsRequests.delete(userId);
    }
  })();

  pendingCosmeticsRequests.set(userId, request);
  return request;
}

/**
 * Get third-party badges for a user - memory cache -> API fetch
 * Third-party badges (FFZ, Chatterino, Homies) come from the unified Rust badge service
 * Note: This requires channelId/channelName context, so we use a simpler approach
 */
export async function getThirdPartyBadgesWithFallback(userId: string): Promise<any[]> {
  // 1. Try in-memory cache first (instant, synchronous)
  const memoryCached = inMemoryThirdPartyBadgesCache.get(userId);
  if (memoryCached) {
    return memoryCached;
  }

  // Third-party badges are now fetched as part of the unified badge service
  // They'll be populated when getTwitchBadgesWithFallback is called
  // Return empty for now - the full profile fetch will populate this
  return [];
}

/**
 * Get Twitch badges from synchronous in-memory cache (instant, no async)
 * Returns null if not in memory cache - use this for initial state
 */
export function getTwitchBadgesFromMemoryCache(cacheKey: string): any[] | null {
  return inMemoryTwitchBadgesCache.get(cacheKey) || null;
}

/**
 * Get Twitch badges for a user - memory cache -> API fetch with deduplication
 */
export async function getTwitchBadgesWithFallback(
  userId: string,
  username: string,
  channelId: string,
  channelName: string
): Promise<any[]> {
  const cacheKey = `${userId}-${channelId}`;

  // 1. Try in-memory cache first (instant, synchronous)
  const memoryCached = inMemoryTwitchBadgesCache.get(cacheKey);
  if (memoryCached) {
    return memoryCached;
  }

  // 2. Check if there's already a pending request for this user (dedupe)
  const pendingRequest = pendingTwitchBadgesRequests.get(cacheKey);
  if (pendingRequest) {
    return pendingRequest;
  }

  // 3. Create a new request and track it
  const request = (async (): Promise<any[]> => {
    try {
      const { getAllUserBadges } = await import('./badgeService');
      const badgeData = await getAllUserBadges(userId, username, channelId, channelName);

      const uniqueBadges = new Map<string, any>();

      // Add display badges first
      badgeData.displayBadges.forEach((badge: any) => {
        uniqueBadges.set(badge.id, badge);
      });

      // Add earned badges that aren't already displayed
      badgeData.earnedBadges.forEach((badge: any) => {
        if (!uniqueBadges.has(badge.id)) {
          uniqueBadges.set(badge.id, badge);
        }
      });

      const result = compactTwitchBadges(Array.from(uniqueBadges.values()));

      Logger.debug(`[cosmeticsCache] Resolved ${result.length} total Twitch badges for ${username}`);

      // Store in memory cache for this session
      inMemoryTwitchBadgesCache.set(cacheKey, result);

      return result;
    } finally {
      pendingTwitchBadgesRequests.delete(cacheKey);
    }
  })();

  pendingTwitchBadgesRequests.set(cacheKey, request);
  return request;
}

/**
 * Get full profile from synchronous in-memory cache (instant, no async)
 * Returns null if not in memory cache
 */
export function getProfileFromMemoryCache(userId: string): CachedProfile | null {
  return inMemoryProfileCache.get(userId) || null;
}

/**
 * Get full profile data with cache-first strategy
 * Returns cached data immediately if available, then refreshes in background
 */
export async function getFullProfileWithFallback(
  userId: string,
  username: string,
  channelId?: string,
  channelName?: string
): Promise<CachedProfile> {
  // 1. Try in-memory cache first (instant, synchronous)
  const memoryCached = inMemoryProfileCache.get(userId);
  if (memoryCached) {
    return memoryCached;
  }

  // 2. Check if there's already a pending request for this user (dedupe)
  const pendingRequest = pendingProfileRequests.get(userId);
  if (pendingRequest) {
    return pendingRequest;
  }

  // 3. Create a new request and track it
  const request = (async (): Promise<CachedProfile> => {
    try {
      const effectiveChannelId = channelId || userId;
      const effectiveChannelName = channelName || username;

      // Fetch badge data from unified service and 7TV cosmetics in parallel
      // Use getAllUserBadgesWithEarned for profile overlays to get full earned badge collection
      const { getAllUserBadgesWithEarned } = await import('./badgeService');
      // Profile surfaces LIST what an account owns (the cosmetics picker, the
      // attainables overlay), so this one path pays the full inventory query.
      // The chat path deliberately fetches only the worn ids, which is why it
      // cannot be reused here. Falls back to the worn-only shape if the heavier
      // query fails, so a profile still paints rather than showing nothing.
      const { fetchUserInventory } = await import('./seventvService');
      const [badgeData, seventvCosmetics] = await Promise.all([
        getAllUserBadgesWithEarned(userId, username, effectiveChannelId, effectiveChannelName),
        fetchUserInventory(userId).then((inv) => inv ?? getCosmeticsWithFallback(userId)),
      ]);

      // Merge display and earned Twitch badges, then strip cache-dead fields.
      const uniqueTwitchBadges = new Map<string, any>();
      badgeData.displayBadges.forEach((badge: any) => uniqueTwitchBadges.set(badge.id, badge));
      badgeData.earnedBadges.forEach((badge: any) => {
        if (!uniqueTwitchBadges.has(badge.id)) uniqueTwitchBadges.set(badge.id, badge);
      });
      const twitchBadges = compactTwitchBadges(Array.from(uniqueTwitchBadges.values()));

      // Third-party badges come from the unified service (FFZ, Chatterino, Homies)
      // They're already in the correct format with imageUrl from badgeService.ts
      const thirdPartyBadges = compactThirdPartyBadges(badgeData.thirdPartyBadges || []);

      Logger.debug(`[cosmeticsCache] Fetched ${twitchBadges.length} Twitch badges and ${thirdPartyBadges.length} third-party badges for ${username}`);

      const profile: CachedProfile = {
        userId,
        username,
        channelId: effectiveChannelId,
        channelName: effectiveChannelName,
        twitchBadges,
        seventvCosmetics,
        thirdPartyBadges,
        lastUpdated: Date.now()
      };

      // Also cache individual components
      const twitchCacheKey = `${userId}-${effectiveChannelId}`;
      inMemoryTwitchBadgesCache.set(twitchCacheKey, twitchBadges);
      inMemoryThirdPartyBadgesCache.set(userId, thirdPartyBadges);

      // Store in memory cache for this session
      inMemoryProfileCache.set(userId, profile);

      return profile;
    } finally {
      pendingProfileRequests.delete(userId);
    }
  })();

  pendingProfileRequests.set(userId, request);
  return request;
}

/**
 * Refresh profile data in background and update cache
 * This fetches fresh data without blocking
 */
export async function refreshProfileInBackground(
  userId: string,
  username: string,
  channelId?: string,
  channelName?: string
): Promise<void> {
  const effectiveChannelId = channelId || userId;
  const effectiveChannelName = channelName || username;
  const twitchCacheKey = `${userId}-${effectiveChannelId}`;

  Logger.debug('[CosmeticsCache] Refreshing profile in background for:', username);

  try {
    // Fetch badge data from unified service and 7TV cosmetics in parallel
    // Use getAllUserBadgesWithEarned for profile overlays to get full earned badge collection
    const { getAllUserBadgesWithEarned } = await import('./badgeService');
    const [badgeDataResult, seventvCosmeticsResult] = await Promise.allSettled([
      getAllUserBadgesWithEarned(userId, username, effectiveChannelId, effectiveChannelName),
      (async () => {
        const { getUserCosmetics } = await import('./seventvService');
        return await getUserCosmetics(userId);
      })()
    ]);

    // Process badge data if successful
    let twitchBadges: any[] = [];
    let thirdPartyBadges: any[] = [];
    
    if (badgeDataResult.status === 'fulfilled') {
      const badgeData = badgeDataResult.value;
      
      // Merge display and earned badges
      const uniqueBadges = new Map<string, any>();
      badgeData.displayBadges.forEach((badge: any) => uniqueBadges.set(badge.id, badge));
      badgeData.earnedBadges.forEach((badge: any) => {
        if (!uniqueBadges.has(badge.id)) uniqueBadges.set(badge.id, badge);
      });
      twitchBadges = compactTwitchBadges(Array.from(uniqueBadges.values()));

      // Transform third-party badges. `link` intentionally omitted — no
      // consumer reads it; compactThirdPartyBadges would strip it anyway.
      thirdPartyBadges = (badgeData.thirdPartyBadges || []).map((b: any) => ({
        id: b.id,
        title: b.title,
        imageUrl: b.imageUrl,
        provider: b.provider,
      }));
    }

    // Update individual caches with fresh data
    if (twitchBadges.length > 0) {
      inMemoryTwitchBadgesCache.set(twitchCacheKey, twitchBadges);
    }
    if (seventvCosmeticsResult.status === 'fulfilled') {
      const { data, hardFail } = seventvCosmeticsResult.value;
      publishCosmetics(userId, data, hardFail);
    }
    if (thirdPartyBadges.length > 0) {
      inMemoryThirdPartyBadgesCache.set(userId, thirdPartyBadges);
    }

    // Update full profile cache
    const seventvCosmeticsForProfile: CachedCosmetics =
      seventvCosmeticsResult.status === 'fulfilled'
        ? seventvCosmeticsResult.value.data
        : inMemoryCosmeticsCache.get(userId) || { paints: [], badges: [] };
    const profile: CachedProfile = {
      userId,
      username,
      channelId: effectiveChannelId,
      channelName: effectiveChannelName,
      twitchBadges: twitchBadges.length > 0 ? twitchBadges : inMemoryTwitchBadgesCache.get(twitchCacheKey) || [],
      seventvCosmetics: seventvCosmeticsForProfile,
      thirdPartyBadges: thirdPartyBadges.length > 0 ? thirdPartyBadges : inMemoryThirdPartyBadgesCache.get(userId) || [],
      lastUpdated: Date.now()
    };

    inMemoryProfileCache.set(userId, profile);
    Logger.debug('[CosmeticsCache] Profile refreshed for:', username, `(${twitchBadges.length} Twitch, ${thirdPartyBadges.length} third-party badges)`);
  } catch (error) {
    Logger.error('[CosmeticsCache] Failed to refresh profile:', error);
  }
}

/**
 * Clear in-memory caches (useful for testing or channel switch)
 */
export function clearCosmeticsMemoryCache(): void {
  inMemoryCosmeticsCache.clear();
  hardFailTimestamps.clear();
  pendingCosmeticsRequests.clear();
  inMemoryThirdPartyBadgesCache.clear();
  pendingThirdPartyBadgesRequests.clear();
  inMemoryTwitchBadgesCache.clear();
  pendingTwitchBadgesRequests.clear();
  inMemoryProfileCache.clear();
  pendingProfileRequests.clear();
  Logger.debug('[CosmeticsCache] All memory caches cleared');
}
