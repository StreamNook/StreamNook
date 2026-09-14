// Display names for the channels that own your subscription emotes.
//
// The shared picker groups Twitch sub emotes by owner and titles each group
// `channelNameCache.get(ownerId) || "Channel <id>"`. Without the cache it shows
// raw numeric ids as headings, which is what mobile did until this existed.
// The desktop composer builds the same map inline while loading emotes.
import { useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { EmoteSet } from '../../services/emoteService';
import { Logger } from '../../utils/logger';

const EMPTY: Map<string, string> = new Map();

export function useEmoteOwnerNames(emotes: EmoteSet | null): Map<string, string> {
  const [names, setNames] = useState<Map<string, string>>(EMPTY);

  // Owner ids as a stable string so the effect only reruns when the actual set
  // of sub-emote owners changes, not on every emote-cache revision.
  //
  // Memoized because the composer re-renders on every arriving chat message,
  // and this walked the whole emote list each time. The dedupe used indexOf,
  // which is a scan per element, so someone subscribed to a lot of channels was
  // paying tens of thousands of comparisons per message just to rebuild a string
  // that almost never changes.
  const ownerKey = useMemo(
    () =>
      Array.from(
        new Set(
          (emotes?.twitch ?? [])
            .filter((e) => e.emote_type === 'subscriptions' && e.owner_id)
            .map((e) => e.owner_id as string),
        ),
      )
        .sort()
        .join(','),
    [emotes],
  );

  useEffect(() => {
    if (!ownerKey) return;
    const ids = ownerKey.split(',');
    let cancelled = false;
    (async () => {
      const results = await Promise.allSettled(
        ids.map(async (id) => {
          const user = await invoke<{ display_name: string }>('get_user_by_id', { userId: id });
          return { id, name: user.display_name };
        }),
      );
      if (cancelled) return;
      const next = new Map<string, string>();
      for (const r of results) {
        if (r.status === 'fulfilled' && r.value.name) next.set(r.value.id, r.value.name);
      }
      if (next.size > 0) setNames(next);
      else Logger.debug('[MobileEmotes] no owner names resolved');
    })();
    return () => {
      cancelled = true;
    };
  }, [ownerKey]);

  return names;
}
