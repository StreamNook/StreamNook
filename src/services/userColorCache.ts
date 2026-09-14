import { invoke } from '@tauri-apps/api/core';
import { useEffect, useState } from 'react';
import { boundedSet } from '../utils/boundedMap';
import { Logger } from '../utils/logger';

/**
 * Batched cache of Twitch users' chosen name colors, keyed by user id.
 *
 * Twitch does not send a gift recipient's own color in the subgift tags (the
 * message `color` is the gifter's), so recipients otherwise fall back to a
 * default purple. This resolves each recipient's real color via the Helix
 * `chat/color` GET, coalescing all ids requested in the same tick into one
 * batched backend call (the command chunks to Helix's 100-id limit). Values
 * are cached; a `null` sentinel marks "fetched, no color set" so we never
 * refetch. The backend degrades to an empty map when unauthenticated, so
 * callers simply fall back to their default color.
 */

// user_id -> hex color, or null once fetched with no color (avoids refetch).
// Session-long; bounded so gift waves across many channels can't grow it forever.
const colorCache = new Map<string, string | null>();
const COLOR_CACHE_MAX = 2000;
const subscribers = new Map<string, Set<() => void>>();
const pending = new Set<string>();
let flushScheduled = false;

function notify(id: string): void {
  const subs = subscribers.get(id);
  if (subs) for (const cb of subs) cb();
}

async function flush(): Promise<void> {
  flushScheduled = false;
  if (pending.size === 0) return;
  const ids = Array.from(pending);
  pending.clear();

  for (let i = 0; i < ids.length; i += 100) {
    const chunk = ids.slice(i, i + 100);
    try {
      const result = await invoke<Record<string, string>>('get_user_chat_colors', { userIds: chunk });
      for (const id of chunk) {
        const color = result?.[id];
        boundedSet(colorCache, id, color && color.length > 0 ? color : null, COLOR_CACHE_MAX);
        notify(id);
      }
    } catch (e) {
      Logger.debug('[userColorCache] batch failed', e);
      // Cache a sentinel so a failing endpoint isn't hammered; caller falls back.
      for (const id of chunk) {
        if (!colorCache.has(id)) boundedSet(colorCache, id, null, COLOR_CACHE_MAX);
        notify(id);
      }
    }
  }
}

function enqueue(id: string): void {
  if (!id || colorCache.has(id) || pending.has(id)) return;
  pending.add(id);
  if (!flushScheduled) {
    flushScheduled = true;
    queueMicrotask(() => void flush());
  }
}

/** Synchronous cache read. Returns undefined when unknown or when the user has
 *  no color set (the `null` sentinel), so callers fall back to their default. */
export function getUserColor(id: string | null | undefined): string | undefined {
  if (!id) return undefined;
  return colorCache.get(id) ?? undefined;
}

/** Hook variant: returns the cached color and enqueues a batched fetch on first
 *  miss, re-rendering when it resolves. Ignores empty ids. */
export function useUserColor(id: string | null | undefined): string | undefined {
  const [, forceRender] = useState(0);
  useEffect(() => {
    if (!id) return;
    const cb = () => forceRender((n) => n + 1);
    let set = subscribers.get(id);
    if (!set) {
      set = new Set();
      subscribers.set(id, set);
    }
    set.add(cb);
    enqueue(id);
    return () => {
      set!.delete(cb);
      if (set!.size === 0) subscribers.delete(id);
    };
  }, [id]);
  return getUserColor(id);
}
