// StreamNook Identity loadout — frontend client for the shared, cross-client
// badge selection served by our Identity API (/api/v1/identity), proxied
// through Rust commands (the write needs the Twitch token, which lives in Rust).
//
// The loadout is a member's chosen subset of badges to display as their
// StreamNook presence. v1 governs which third-party badges (BTTV/FFZ/Chatterino
// /Homies/Chatsen/Chatty/DankChat) are promoted into chat + the profile card.
// Keys are provider-agnostic: `<provider>:<id>` (and `7tv:<id>` / `twitch:<set>/<ver>`
// are reserved for later). `customized=false` ⇒ show everything (the default).

import { invoke } from '@tauri-apps/api/core';
import { boundedSet } from '../utils/boundedMap';
import { Logger } from '../utils/logger';

export interface IdentityLoadout {
  twitch_user_id: string;
  customized: boolean;
  badges: string[];
  paint: string | null;
  updated_at: string | null;
}

const TTL = 5 * 60 * 1000; // 5 minutes
const IDENTITY_CACHE_MAX = 1000;
const cache = new Map<string, { data: IdentityLoadout; ts: number }>();
const pending = new Map<string, Promise<IdentityLoadout>>();

// Persist OUR OWN accounts' loadout + resolved badge bundle to disk so their
// curated third-party badges paint in chat on frame one of the next launch,
// instead of after the resolve round-trip. Per account we've added (primary +
// linked) — the set is populated at boot via seedOwnIdentitiesFromCache; only
// those rows are persisted (every other chatter resolves from the network).
const OWN_IDENTITY_KEY = 'streamnook_own_identity_v1';
const ownIdentityAccounts = new Set<string>();

type PersistedIdentityStore = Record<
  string,
  { loadout?: IdentityLoadout; resolved?: ResolvedIdentity }
>;

function readPersistedIdentities(): PersistedIdentityStore {
  try {
    const raw = localStorage.getItem(OWN_IDENTITY_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw);
    return parsed && typeof parsed === 'object' ? (parsed as PersistedIdentityStore) : {};
  } catch {
    return {};
  }
}

function persistOwnIdentity(userId: string): void {
  try {
    const loadout = cache.get(userId)?.data;
    const resolved = resolvedCache.get(userId)?.data;
    const store = readPersistedIdentities();
    if (!loadout && !resolved) delete store[userId];
    else store[userId] = { loadout, resolved };
    localStorage.setItem(OWN_IDENTITY_KEY, JSON.stringify(store));
  } catch (e) {
    Logger.warn('[identityService] persistOwnIdentity failed:', e);
  }
}

/**
 * At launch, register all of OUR account ids (primary + linked) so their
 * loadout/resolved writes persist, and hydrate each from disk so chat paints every
 * account's curated third-party badges instantly. Seeded with a stale timestamp:
 * synchronous peeks (getResolvedIdentityFromCache, which chat uses) return it
 * immediately, while getResolvedIdentity / getIdentityWithCache still revalidate on
 * their next call and overwrite if anything changed. No-op for ids with nothing stored.
 */
export function seedOwnIdentitiesFromCache(userIds: string[]): void {
  const store = readPersistedIdentities();
  for (const userId of userIds) {
    if (!userId) continue;
    ownIdentityAccounts.add(userId);
    const entry = store[userId];
    if (!entry) continue;
    if (entry.loadout && !cache.has(userId)) {
      boundedSet(cache, userId, { data: entry.loadout, ts: 0 }, IDENTITY_CACHE_MAX);
      version++;
      listeners.forEach((l) => l());
    }
    if (entry.resolved && !resolvedCache.has(userId)) {
      boundedSet(resolvedCache, userId, { data: entry.resolved, ts: 0 }, IDENTITY_CACHE_MAX);
      notifyResolved(userId);
    }
  }
}

// Listeners notified when any loadout in the cache changes (own-row edits or
// background refreshes), so chat rows re-read synchronously.
const listeners = new Set<() => void>();
let version = 0;
const publish = (userId: string, data: IdentityLoadout) => {
  boundedSet(cache, userId, { data, ts: Date.now() }, IDENTITY_CACHE_MAX);
  version++;
  listeners.forEach((l) => l());
  if (ownIdentityAccounts.has(userId)) persistOwnIdentity(userId);
};

export const subscribeIdentityVersion = (cb: () => void): (() => void) => {
  listeners.add(cb);
  return () => listeners.delete(cb);
};
export const getIdentityVersion = (): number => version;

const defaultLoadout = (userId: string): IdentityLoadout => ({
  twitch_user_id: userId,
  customized: false,
  badges: [],
  paint: null,
  updated_at: null,
});

/** Synchronous cache peek — null if not loaded yet. */
export function getIdentityFromMemoryCache(userId: string): IdentityLoadout | null {
  const hit = cache.get(userId);
  return hit ? hit.data : null;
}

/**
 * Fetch a user's loadout, cached. Non-customized results are cached too, so a
 * member who hasn't curated isn't re-fetched on every message. De-dupes
 * in-flight requests for the same user.
 */
export async function getIdentityWithCache(userId: string): Promise<IdentityLoadout> {
  if (!userId) return defaultLoadout(userId);
  const hit = cache.get(userId);
  if (hit && Date.now() - hit.ts < TTL) return hit.data;

  const inflight = pending.get(userId);
  if (inflight) return inflight;

  const p = (async () => {
    try {
      const data = (await invoke('get_streamnook_identity', { userId })) as IdentityLoadout;
      publish(userId, data);
      return data;
    } catch (e) {
      Logger.warn('[identityService] get failed, defaulting to show-all:', e);
      const fallback = defaultLoadout(userId);
      boundedSet(cache, userId, { data: fallback, ts: Date.now() }, IDENTITY_CACHE_MAX);
      return fallback;
    } finally {
      pending.delete(userId);
    }
  })();
  pending.set(userId, p);
  return p;
}

/** Batch prefetch (chat backfill). Populates the cache; ignores failures. */
export async function prefetchIdentities(userIds: string[]): Promise<void> {
  const need = userIds.filter((id) => {
    const hit = cache.get(id);
    return id && (!hit || Date.now() - hit.ts >= TTL);
  });
  if (need.length === 0) return;
  try {
    const map = (await invoke('get_streamnook_identities', { userIds: need })) as Record<string, IdentityLoadout>;
    for (const [id, data] of Object.entries(map)) publish(id, data);
  } catch (e) {
    Logger.warn('[identityService] batch prefetch failed:', e);
  }
}

// ── Resolved (all-in-one) bundle ────────────────────────────────────────────
// One call returns the member's selected badges already resolved to images
// (server-side, ownership-checked) + their live 7TV badge/paint. Chat uses this
// for StreamNook users instead of resolving every provider locally.
export interface ResolvedBadge {
  key: string;
  provider: string;
  title: string;
  image_url: string;
}
export interface ResolvedIdentity {
  twitch_user_id: string;
  customized: boolean;
  badges: ResolvedBadge[];
  seventv: unknown;
  updated_at: string | null;
}

const resolvedCache = new Map<string, { data: ResolvedIdentity; ts: number }>();
const resolvedPending = new Map<string, Promise<ResolvedIdentity>>();
const defaultResolved = (userId: string): ResolvedIdentity => ({
  twitch_user_id: userId,
  customized: false,
  badges: [],
  seventv: null,
  updated_at: null,
});

// Listeners notified (with the affected userId) whenever a resolved bundle is
// fetched or cleared, so a surface holding a copy (chat) can re-resolve and
// repaint. The resolved analogue of the loadout listeners near the top.
const resolvedListeners = new Set<(userId: string) => void>();
const notifyResolved = (userId: string) => {
  resolvedListeners.forEach((l) => l(userId));
  // A resolved-bundle change for one of our accounts (a badge toggle re-resolves
  // here) is exactly when the persisted copy must update, so the next launch
  // reflects the edit — per account.
  if (ownIdentityAccounts.has(userId)) persistOwnIdentity(userId);
};
export const subscribeResolvedIdentity = (cb: (userId: string) => void): (() => void) => {
  resolvedListeners.add(cb);
  return () => resolvedListeners.delete(cb);
};

/** Synchronous peek at the resolved bundle — null if it hasn't been fetched yet. */
export function getResolvedIdentityFromCache(userId: string): ResolvedIdentity | null {
  const hit = resolvedCache.get(userId);
  return hit ? hit.data : null;
}

/** Drop a user's resolved-bundle cache (e.g. right after they edit their own loadout). */
export function clearResolvedIdentity(userId: string): void {
  resolvedCache.delete(userId);
  notifyResolved(userId);
}

/**
 * Seed the resolved-bundle cache directly from an authoritative write response.
 * The identity write now returns the server-resolved bundle, so the member's own
 * chat row can repaint with the just-saved selection WITHOUT re-reading the
 * `?resolve=1` endpoint (edge-cached ~60s, which would serve the pre-write value
 * and make the change look like it didn't take). notifyResolved fires so chat
 * re-reads synchronously from the now-fresh cache.
 */
export function setResolvedIdentityFromWrite(userId: string, data: ResolvedIdentity): void {
  boundedSet(resolvedCache, userId, { data, ts: Date.now() }, IDENTITY_CACHE_MAX);
  notifyResolved(userId);
}

export async function getResolvedIdentity(userId: string): Promise<ResolvedIdentity> {
  if (!userId) return defaultResolved(userId);
  const hit = resolvedCache.get(userId);
  if (hit && Date.now() - hit.ts < TTL) return hit.data;

  const inflight = resolvedPending.get(userId);
  if (inflight) return inflight;

  const p = (async () => {
    try {
      const data = (await invoke('get_streamnook_identity_resolved', { userId })) as ResolvedIdentity;
      boundedSet(resolvedCache, userId, { data, ts: Date.now() }, IDENTITY_CACHE_MAX);
      notifyResolved(userId);
      return data;
    } catch (e) {
      Logger.warn('[identityService] resolved get failed:', e);
      const fb = defaultResolved(userId);
      boundedSet(resolvedCache, userId, { data: fb, ts: Date.now() }, IDENTITY_CACHE_MAX);
      return fb;
    } finally {
      resolvedPending.delete(userId);
    }
  })();
  resolvedPending.set(userId, p);
  return p;
}

/**
 * Persist the current user's loadout. Updates the local cache synchronously
 * (so their own chat rows + the profile card reflect it immediately), then
 * writes through the API. `userId` is the caller's own id (for the cache key);
 * the server derives the authoritative id from the token.
 */
export async function setIdentity(
  userId: string,
  badges: string[],
  paint: string | null,
  customized: boolean,
  accountId?: string,
): Promise<void> {
  // Optimistic local loadout update. We deliberately do NOT clearResolvedIdentity
  // here: clearing it would make chat immediately re-read the edge-cached
  // resolve endpoint and re-cache the PRE-write bundle (the exact stale-read this
  // change avoids). The resolved bundle is updated below from the write itself.
  publish(userId, { twitch_user_id: userId, customized, badges, paint, updated_at: null });
  try {
    // `accountId` routes the write through that linked account's token (the
    // server authorizes per-account); omitted ⇒ the primary.
    const saved = (await invoke('set_streamnook_identity', {
      badges,
      paint,
      customized,
      accountId: accountId ?? null,
    })) as IdentityLoadout & { resolved?: ResolvedIdentity | null };
    const { resolved, ...loadout } = saved;
    const id = loadout.twitch_user_id || userId;
    publish(id, loadout);
    if (resolved) {
      // Authoritative resolved bundle from the write — seed the cache directly,
      // no racy re-read of the edge-cached endpoint.
      setResolvedIdentityFromWrite(id, resolved);
    } else {
      // Older backend without resolved-on-write: fall back to the clear+re-fetch
      // (still correct, just subject to the ~60s cache it used to be).
      clearResolvedIdentity(id);
    }
  } catch (e) {
    Logger.error('[identityService] set failed:', e);
    throw e;
  }
}
