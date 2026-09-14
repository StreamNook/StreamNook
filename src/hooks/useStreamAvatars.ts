// Channel avatars for a grid of stream cards, across every platform.
//
// A stream row does not reliably carry its channel's avatar, and where it comes
// from differs per platform:
//
//  - Twitch  : Helix `streams` omits it entirely; it needs a `users` lookup, which
//              batches 100 ids per request.
//  - YouTube : search results and the subscriptions feed DO ship one on the row
//              (`profile_image_url`), but a game's category grid ships none, so
//              those need the per-channel resolver.
//  - Others  : whatever the row carried, else nothing.
//
// Results are keyed by the row's COMPOSITE key so a caller can look a card up
// without knowing which platform produced it. Anything unresolved is simply
// absent, so the caller keeps its own placeholder rather than drawing a broken
// image.

import { useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { helixGet } from '../services/helix';
import type { TwitchStream } from '../types';
import type { ProviderId } from '../types/providers';
import { streamKey, streamProvider } from '../utils/streamProvider';
import { Logger } from '../utils/logger';

// Resolved avatars, shared by every instance of this hook and persisted across
// restarts.
//
// Both halves matter. The state used to live in the hook, so Home and the
// Sidebar each resolved the same faces separately, and every remount started
// from nothing — on a large subscription list that is hundreds of requests for
// pictures we already had. Avatars change rarely, so a stale one for a few days
// costs nothing next to re-fetching the lot on every launch.
const AVATAR_STORE_KEY = 'streamnook.avatars.v1';
const AVATAR_TTL_MS = 7 * 86_400_000;
type AvatarEntry = { url: string; t: number };
const avatarMemo = new Map<string, AvatarEntry>();
let avatarStoreLoaded = false;

/** Cache key: platform id space, so a Kick id can't shadow a Twitch one. */
const avatarKey = (provider: ProviderId, channelId: string) =>
  `${provider}:${channelId.toLowerCase()}`;

function loadAvatarStore(): void {
  if (avatarStoreLoaded) return;
  avatarStoreLoaded = true;
  try {
    const raw = localStorage.getItem(AVATAR_STORE_KEY);
    if (!raw) return;
    const now = Date.now();
    for (const [k, v] of Object.entries(JSON.parse(raw) as Record<string, AvatarEntry>)) {
      if (v?.url && typeof v.t === 'number' && now - v.t < AVATAR_TTL_MS) {
        avatarMemo.set(k, v);
      }
    }
  } catch {
    /* unreadable or blocked storage just means we resolve again */
  }
}

let saveTimer: ReturnType<typeof setTimeout> | null = null;
function saveAvatarStore(): void {
  if (saveTimer) return;
  // Debounced: a grid resolving in waves would otherwise serialize the whole
  // map on each wave.
  saveTimer = setTimeout(() => {
    saveTimer = null;
    try {
      localStorage.setItem(AVATAR_STORE_KEY, JSON.stringify(Object.fromEntries(avatarMemo)));
    } catch {
      /* storage full: the in-memory map still serves this session */
    }
  }, 1000);
}

function rememberAvatar(provider: ProviderId, channelId: string, url: string): void {
  avatarMemo.set(avatarKey(provider, channelId), { url, t: Date.now() });
  saveAvatarStore();
}

/** Twitch's `users` endpoint takes 100 ids per call. */
const HELIX_BATCH = 100;
/** Provider channels resolved per pass. Each costs a real request upstream, so
 *  this stays small and the rest are picked up as more cards render. */
const PROVIDER_PER_PASS = 24;

export function useStreamAvatars(streams: TwitchStream[]): Record<string, string> {
    const [avatars, setAvatars] = useState<Record<string, string>>({});
    // Already-known faces, read straight through during render. Deriving these
    // rather than seeding state keeps a cache hit from costing an extra render
    // (and from being a setState inside the effect).
    const cached = useMemo(() => {
        loadAvatarStore();
        const out: Record<string, string> = {};
        for (const s of streams) {
            if (s.profile_image_url) continue;
            const rowKey = streamKey(s);
            const prov = streamProvider(s);
            const channelId = prov === 'kick' ? s.user_login : s.user_id;
            if (!rowKey || !channelId) continue;
            const hit = avatarMemo.get(avatarKey(prov, channelId));
            if (hit) out[rowKey] = hit.url;
        }
        return out;
    }, [streams]);
    // Ids already requested, INCLUDING ones that resolved to nothing, so a channel
    // without an avatar is asked once rather than on every render.
    const askedRef = useRef<Set<string>>(new Set());

    useEffect(() => {
        loadAvatarStore();
        // Rows still missing an avatar, split by where the avatar has to come from.
        const twitchIds: string[] = [];
        const twitchKeys = new Map<string, string[]>();
        const providerIds = new Map<ProviderId, Map<string, string[]>>();
        for (const s of streams) {
            if (s.profile_image_url) continue;
            const rowKey = streamKey(s);
            const provider = streamProvider(s);
            // Each platform's avatar endpoint takes the id IT addresses channels
            // by: Twitch and YouTube by numeric/UC id, Kick by SLUG. Sending
            // Kick's numeric broadcaster id 404s on every request, because its
            // channel endpoint is slug-only.
            const channelId = provider === 'kick' ? s.user_login : s.user_id;
            if (!rowKey || !channelId) continue;
            if (avatarMemo.has(avatarKey(provider, channelId))) continue; // served by `cached` below
            if (provider === 'twitch') {
                const keys = twitchKeys.get(channelId);
                if (keys) {
                    keys.push(rowKey);
                } else {
                    twitchKeys.set(channelId, [rowKey]);
                    if (!askedRef.current.has(channelId)) twitchIds.push(channelId);
                }
                continue;
            }
            if (askedRef.current.has(channelId)) continue;
            let forProvider = providerIds.get(provider);
            if (!forProvider) {
                forProvider = new Map();
                providerIds.set(provider, forProvider);
            }
            const keys = forProvider.get(channelId);
            if (keys) keys.push(rowKey);
            else forProvider.set(channelId, [rowKey]);
        }

        if (twitchIds.length === 0 && providerIds.size === 0) return;

        let cancelled = false;
        const merge = (next: Record<string, string>) => {
            if (!cancelled && Object.keys(next).length > 0) {
                setAvatars((prev) => ({ ...prev, ...next }));
            }
        };

        void (async () => {
            if (twitchIds.length > 0) {
                twitchIds.forEach((id) => askedRef.current.add(id));
                try {
                    for (let i = 0; i < twitchIds.length; i += HELIX_BATCH) {
                        const batch = twitchIds.slice(i, i + HELIX_BATCH);
                        const query = batch.map((id) => `id=${id}`).join('&');
                        let data: { data?: { id: string; profile_image_url?: string }[] };
                        try {
                            data = await helixGet('users', query);
                        } catch {
                            continue;
                        }
                        const next: Record<string, string> = {};
                        for (const user of data?.data ?? []) {
                            if (!user?.profile_image_url) continue;
                            rememberAvatar('twitch', user.id, user.profile_image_url);
                            for (const rowKey of twitchKeys.get(user.id) ?? []) {
                                next[rowKey] = user.profile_image_url;
                            }
                        }
                        merge(next);
                        if (cancelled) return;
                    }
                } catch (e) {
                    Logger.debug('[avatars] Twitch lookup failed:', e);
                }
            }

            for (const [provider, channels] of providerIds) {
                const ids = Array.from(channels.keys()).slice(0, PROVIDER_PER_PASS);
                // Marked before awaiting so a re-render mid-flight can't re-ask.
                ids.forEach((id) => askedRef.current.add(id));
                try {
                    const resolved = await invoke<Record<string, string>>('provider_channel_avatars', {
                        provider,
                        channelIds: ids,
                    });
                    if (cancelled) return;
                    const next: Record<string, string> = {};
                    for (const [channelId, url] of Object.entries(resolved ?? {})) {
                        rememberAvatar(provider, channelId, url);
                        for (const rowKey of channels.get(channelId) ?? []) {
                            next[rowKey] = url;
                        }
                    }
                    merge(next);
                } catch (e) {
                    Logger.debug('[avatars] provider lookup failed:', e);
                }
            }
        })();

        return () => {
            cancelled = true;
        };
    }, [streams]);

    // Freshly resolved wins over the cached copy for the same row.
    return useMemo(() => ({ ...cached, ...avatars }), [cached, avatars]);
}

export default useStreamAvatars;
