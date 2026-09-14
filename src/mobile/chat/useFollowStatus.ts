// Do I follow this channel?
//
// Needed to answer exactly one question: whether followers-only mode is actually
// stopping YOU from talking. `chatGating.ts` deliberately refused to treat
// followers-only as blocking until now, and its reasoning was sound - nothing in
// the chat payload says whether you follow, and guessing wrong either hides a
// working composer or promises one that silently drops messages. The answer was
// never in the chat payload; it is a Helix call. `check_following_status` is
// authoritative, so the guess is no longer necessary.
//
// Only fetched when it can change the answer (`enabled`), because it is a
// request per channel and the overwhelming majority of rooms are unrestricted.
import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Logger } from '../../utils/logger';

/** Survives channel hops and tab switches. A follow does not change on its own,
 *  and the follow/unfollow action announces itself (see below). */
const cache = new Map<string, boolean>();

/** Dispatched by whatever performs a follow/unfollow, so a composer blocked on
 *  "Follow to send a message" unblocks the moment you actually follow instead of
 *  waiting for a remount. Fired as a plain window event rather than an import so
 *  the shared desktop profile card does not have to depend on mobile code. */
export interface FollowChangedDetail {
  userId: string;
  following: boolean;
}

export function useFollowStatus(channelId: string | null, enabled: boolean): boolean | null {
  // The answer is READ FROM THE CACHE DURING RENDER, and this is only a
  // re-render trigger. Mirroring the cache into state meant setting state
  // synchronously inside the effect on every channel change, which the repo's
  // react-hooks rules reject (cascading renders) and which was pure bookkeeping
  // anyway - the cache is already the source of truth.
  const [, bump] = useState(0);

  useEffect(() => {
    if (!channelId || !enabled) return;
    let cancelled = false;

    if (cache.get(channelId) === undefined) {
      void (async () => {
        try {
          const following = await invoke<boolean>('check_following_status', {
            targetUserId: channelId,
          });
          if (cancelled) return;
          cache.set(channelId, following);
          bump((n) => n + 1);
        } catch (err) {
          // Stays unknown, and unknown never blocks. Failing open is the right
          // way round: a composer that works when it should not costs one
          // rejected message, while one locked for no reason is unusable and
          // gives the user nothing to act on.
          Logger.warn('[Chat] follow-status check failed:', err);
        }
      })();
    }

    const onChanged = (e: Event) => {
      const d = (e as CustomEvent<FollowChangedDetail>).detail;
      if (!d || d.userId !== channelId) return;
      cache.set(channelId, d.following);
      bump((n) => n + 1);
    };
    window.addEventListener('sn:follow-changed', onChanged);
    return () => {
      cancelled = true;
      window.removeEventListener('sn:follow-changed', onChanged);
    };
  }, [channelId, enabled]);

  if (!channelId || !enabled) return null;
  return cache.get(channelId) ?? null;
}
