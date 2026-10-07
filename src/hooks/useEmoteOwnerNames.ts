// Display names of the channels that own the Twitch subscription emotes in a
// set, for the emote menu's per-channel groups. Shared by the chat's menu and
// the popped-out one. Names are looked up once per window (module cache), not
// once per chat box, and only for owners not already known.
import { useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { EmoteSet } from '../services/emoteService';
import { Logger } from '../utils/logger';

const known = new Map<string, string>();

export function useEmoteOwnerNames(emotes: EmoteSet | null): Map<string, string> {
  const ownerIds = useMemo(() => {
    const ids = new Set<string>();
    for (const e of emotes?.twitch ?? []) {
      if (e.owner_id && e.emote_type === 'subscriptions') ids.add(e.owner_id);
    }
    return [...ids].sort().join(',');
  }, [emotes]);
  // Bumped when a lookup lands, so the returned map is a fresh object.
  const [version, setVersion] = useState(0);

  useEffect(() => {
    if (!ownerIds) return;
    const missing = ownerIds.split(',').filter((id) => !known.has(id));
    if (missing.length === 0) return;
    let cancelled = false;
    Logger.debug(`[EmoteOwnerNames] Fetching ${missing.length} channel names for emote groups`);
    void Promise.allSettled(
      missing.map(async (id) => {
        const user = await invoke<{ display_name: string }>('get_user_by_id', { userId: id });
        known.set(id, user.display_name);
      }),
    ).then(() => {
      if (!cancelled) setVersion((v) => v + 1);
    });
    return () => {
      cancelled = true;
    };
  }, [ownerIds]);

  // Every name this window knows (a superset of this set's owners), as a new
  // object each time a lookup lands.
  return useMemo(() => {
    void version;
    return new Map(known);
  }, [version]);
}
