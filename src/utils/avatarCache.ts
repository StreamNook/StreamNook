import { invoke } from '@tauri-apps/api/core';
import { useEffect, useState } from 'react';
import { boundedSet } from './boundedMap';

// Tiny avatar resolver: maps a Twitch login -> profile_image_url, fetched once
// via the existing get_user_by_login command and memoized process-wide. Used by
// the mod log to show channel + moderator pictures without re-fetching per row.

const cache = new Map<string, string | null>();
const AVATAR_CACHE_MAX = 500;
const inflight = new Map<string, Promise<string | null>>();

export function getCachedAvatar(login?: string | null): string | null {
  if (!login) return null;
  return cache.get(login.toLowerCase()) ?? null;
}

export async function resolveAvatar(login?: string | null): Promise<string | null> {
  if (!login) return null;
  const key = login.toLowerCase();
  if (cache.has(key)) return cache.get(key) ?? null;
  const existing = inflight.get(key);
  if (existing) return existing;
  const p = (async () => {
    try {
      const info = await invoke<{ profile_image_url?: string | null }>('get_user_by_login', { login: key });
      const url = info?.profile_image_url ?? null;
      boundedSet(cache, key, url, AVATAR_CACHE_MAX);
      return url;
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

/** Resolve a user's avatar by login. Returns null until resolved (cached after). */
export function useAvatar(login?: string | null): string | null {
  const key = login ? login.toLowerCase() : null;
  // Only the async resolver writes state; the cached value is read at render
  // time, so a login change repaints from cache with no effect-time setState.
  const [resolved, setResolved] = useState<{ key: string; url: string | null } | null>(null);
  useEffect(() => {
    if (!key) return;
    let alive = true;
    void resolveAvatar(key).then((u) => {
      if (alive) setResolved({ key, url: u });
    });
    return () => {
      alive = false;
    };
  }, [key]);
  if (!key) return null;
  return getCachedAvatar(key) ?? (resolved?.key === key ? resolved.url : null);
}
