// Channel avatars for a grid of stream cards, across every platform.
//
// A stream row does not reliably carry its channel's avatar, and where it comes
// from differs per platform:
//
//  - Twitch  : Helix `streams` omits it entirely; it needs a `users` lookup.
//  - YouTube : search results and the subscriptions feed DO ship one on the row
//              (`profile_image_url`), but a game's category grid ships none, so
//              those need the per-channel resolver.
//  - Others  : whatever the row carried, else nothing.
//
// Rust owns the lookups and the cache (`services/channel_avatars.rs`), shared by
// every window and kept across restarts. This asks for the rows it draws, gets
// whatever is cached at once, and picks up the rest from the `channel-avatars`
// broadcast as each platform answers. A card that scrolled away before its
// answer arrived loses nothing: the answer is cached for the next ask.
//
// Results are keyed by the row's COMPOSITE key so a caller can look a card up
// without knowing which platform produced it. Anything unresolved is simply
// absent, so the caller keeps its own placeholder rather than drawing a broken
// image.

import { useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { TwitchStream } from '../types';
import type { ProviderId } from '../types/providers';
import { streamKey, streamProvider } from '../utils/streamProvider';
import { Logger } from '../utils/logger';

/** Cache key, in each platform's own id space; matches Rust's. */
const avatarKey = (provider: ProviderId, channelId: string) =>
  `${provider}:${channelId.toLowerCase()}`;

// The avatar store this hook kept before Rust owned the cache. Handed over on
// the first request of a session and then removed, so the faces it already
// had are not fetched again.
const LEGACY_STORE_KEY = 'streamnook.avatars.v1';
let legacyHandedOver = false;

function takeLegacyStore(): Record<string, { url: string; t: number }> | null {
  if (legacyHandedOver) return null;
  legacyHandedOver = true;
  try {
    const raw = localStorage.getItem(LEGACY_STORE_KEY);
    if (!raw) return null;
    localStorage.removeItem(LEGACY_STORE_KEY);
    return JSON.parse(raw) as Record<string, { url: string; t: number }>;
  } catch {
    return null;
  }
}

export function useStreamAvatars(streams: TwitchStream[]): Record<string, string> {
  // Avatar per cache key, for the keys this grid asked about.
  const [avatars, setAvatars] = useState<Record<string, string>>({});

  // What to ask for, and which rows each answer feeds.
  const wanted = useMemo(() => {
    const rowsByKey = new Map<string, string[]>();
    const channels: { provider: ProviderId; id: string }[] = [];
    for (const s of streams) {
      if (s.profile_image_url) continue;
      const rowKey = streamKey(s);
      const provider = streamProvider(s);
      // Each platform's lookup takes the id IT addresses channels by: Twitch
      // and YouTube by numeric/UC id, Kick by SLUG.
      const channelId = provider === 'kick' ? s.user_login : s.user_id;
      if (!rowKey || !channelId) continue;
      const key = avatarKey(provider, channelId);
      const rows = rowsByKey.get(key);
      if (rows) {
        rows.push(rowKey);
      } else {
        rowsByKey.set(key, [rowKey]);
        channels.push({ provider, id: channelId });
      }
    }
    return { rowsByKey, channels };
  }, [streams]);

  // The listener outlives any one row list, so it reads the current list here.
  const wantedRef = useRef(wanted);
  useEffect(() => {
    wantedRef.current = wanted;
  }, [wanted]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let disposed = false;
    void listen<Record<string, string>>('channel-avatars', (e) => {
      const mine: Record<string, string> = {};
      for (const [key, url] of Object.entries(e.payload)) {
        if (wantedRef.current.rowsByKey.has(key)) mine[key] = url;
      }
      if (Object.keys(mine).length > 0) setAvatars((prev) => ({ ...prev, ...mine }));
    }).then((u) => {
      if (disposed) u();
      else unlisten = u;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    if (wanted.channels.length === 0) return;
    let current = true;
    invoke<Record<string, string>>('request_channel_avatars', {
      channels: wanted.channels,
      legacy: takeLegacyStore(),
    })
      .then((cached) => {
        if (current && Object.keys(cached).length > 0) setAvatars((prev) => ({ ...prev, ...cached }));
      })
      .catch((e) => Logger.debug('[avatars] request failed:', e));
    return () => {
      current = false;
    };
  }, [wanted]);

  return useMemo(() => {
    const out: Record<string, string> = {};
    for (const [key, rows] of wanted.rowsByKey) {
      const url = avatars[key];
      if (url) for (const row of rows) out[row] = url;
    }
    return out;
  }, [wanted, avatars]);
}

export default useStreamAvatars;
