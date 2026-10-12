import { invoke } from '@tauri-apps/api/core';
import { useEffect, useState } from 'react';
import { boundedSet } from './boundedMap';

// Tiny avatar resolver: maps a Twitch login -> profile_image_url and user id,
// fetched once via the existing get_user_by_login command and memoized
// process-wide. Used by the mod log to show channel + moderator pictures
// without re-fetching per row; the id is the person's 7TV cosmetics id.

interface TwitchIdentity {
  url: string | null;
  id: string | null;
}

const cache = new Map<string, TwitchIdentity | null>();
const AVATAR_CACHE_MAX = 500;
const inflight = new Map<string, Promise<TwitchIdentity | null>>();

export function getCachedAvatar(login?: string | null): string | null {
  if (!login) return null;
  return cache.get(login.toLowerCase())?.url ?? null;
}

async function resolveIdentity(login?: string | null): Promise<TwitchIdentity | null> {
  if (!login) return null;
  const key = login.toLowerCase();
  if (cache.has(key)) return cache.get(key) ?? null;
  const existing = inflight.get(key);
  if (existing) return existing;
  const p = (async () => {
    try {
      const info = await invoke<{ id?: string | null; profile_image_url?: string | null }>('get_user_by_login', { login: key });
      const identity = { url: info?.profile_image_url ?? null, id: info?.id ?? null };
      boundedSet(cache, key, identity, AVATAR_CACHE_MAX);
      return identity;
    } catch {
      boundedSet(cache, key, null, AVATAR_CACHE_MAX);
      return null;
    } finally {
      inflight.delete(key);
    }
  })();
  inflight.set(key, p);
  return p;
}

export async function resolveAvatar(login?: string | null): Promise<string | null> {
  return (await resolveIdentity(login))?.url ?? null;
}

/** Resolve a Twitch user's picture and id by login. Null until resolved
 *  (cached after). */
export function useTwitchIdentity(login?: string | null): TwitchIdentity | null {
  const key = login ? login.toLowerCase() : null;
  // Only the async resolver writes state; the cached value is read at render
  // time, so a login change repaints from cache with no effect-time setState.
  const [resolved, setResolved] = useState<{ key: string; identity: TwitchIdentity | null } | null>(null);
  useEffect(() => {
    if (!key) return;
    let alive = true;
    void resolveIdentity(key).then((identity) => {
      if (alive) setResolved({ key, identity });
    });
    return () => {
      alive = false;
    };
  }, [key]);
  if (!key) return null;
  return cache.get(key) ?? (resolved?.key === key ? resolved.identity : null);
}
