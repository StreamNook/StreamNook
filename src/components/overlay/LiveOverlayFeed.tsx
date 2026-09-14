// Live feed for the builder preview. Connects one or more real sources across
// ANY provider through the app's own chat pipeline (ref-counted
// chatConnectionStore → Rust IRC / provider adapters) and MERGES them into one
// feed, exactly like MultiChat — inside the app all providers connect (the
// hosted OBS page is the only thing that needs the later backend). Cosmetics
// resolve the same way the real chat row does; everything renders through the
// SAME twin renderer the OBS overlay uses.

import { useEffect, useMemo, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { helixGet } from '../../services/helix';
import {
  acquireChannel,
  releaseChannel,
  useChatConnectionStore,
} from '../../stores/chatConnectionStore';
import { makeKey, parseKey } from '../../utils/providerKey';
import { parseBadges } from '../../services/twitchBadges';
import { useChatUserStore } from '../../stores/chatUserStore';
import { isStreamNookUser, getStreamNookUserNumber, getActiveCosmeticSlug } from '../../services/supabaseService';
import { getBadgeImageUrl } from '../../services/seventvService';
import { COSMETIC_ASSET_BY_SLUG } from '../cosmeticAssets';
import { getAtmosphere } from '../../services/atmospheres';
import type { Atmosphere } from '../../services/atmospheres';
import type { BackendChatMessage } from '../../services/twitchChat';
import type { ProviderId } from '../../types/providers';
import { OverlayChat } from './OverlayChat';
import type { OverlayMessage, OverlayAtmosphere } from './sampleMessages';
import type { OverlayStyle } from './overlayConfig';

// Map the app's full Atmosphere → the portable overlay subset. The overlay skips
// the rare CS2 'cologne-chrome' render kind (no MajorCologneChrome on a broadcast).
const toOverlayAtmosphere = (atm: Atmosphere | null | undefined): OverlayAtmosphere | null => {
  if (!atm || atm.kind === 'cologne-chrome') return null;
  return {
    baseColor: atm.baseColor,
    baseLayers: atm.baseLayers,
    image: atm.image,
    layers: atm.layers,
    layers2: atm.layers2,
    chatEdge: atm.chatEdge,
    chatFrost: atm.chatFrost,
    chatBlur: atm.chatBlur,
    chatRim: atm.chatRim,
  };
};

export interface LiveSource { provider: ProviderId; channel: string; }

type UsersMap = ReturnType<typeof useChatUserStore.getState>['users'];

// The chat store's slice key: bare login for Twitch, composite for the rest.
const sliceKeyFor = (provider: ProviderId, channel: string): string =>
  provider === 'twitch' ? channel.trim().toLowerCase() : makeKey(provider, channel.trim());

const twitchBadgeUrl = (info: { localUrl?: string; url?: string; image_url_4x?: string; image_url_2x?: string; image_url_1x?: string }): string | undefined =>
  info.localUrl || info.image_url_4x || info.image_url_2x || info.image_url_1x || info.url;

// Real send time (ms). Twitch stamps `tmi-sent-ts`; others carry an ISO timestamp.
const sentAt = (m: BackendChatMessage): number => {
  const tmi = m.tags?.['tmi-sent-ts'];
  if (tmi) { const n = Number(tmi); if (Number.isFinite(n) && n > 0) return n; }
  const p = Date.parse(m.timestamp);
  return Number.isFinite(p) ? p : 0;
};

// Resolve a Twitch login → broadcaster id so the store's preloadChannel can
// backfill recent chat history on connect (it bails without an id, leaving the
// overlay empty until live messages trickle in). Cached across the session.
const twitchIdCache = new Map<string, string | null>();
async function resolveTwitchId(login: string): Promise<string | null> {
  const key = login.trim().toLowerCase();
  const cached = twitchIdCache.get(key);
  if (cached !== undefined) return cached;
  try {
    {
      const data = await helixGet<{ data?: Array<{ id?: string }> }>('users', `login=${encodeURIComponent(key)}`);
      const id = (data?.data?.[0]?.id as string | undefined) ?? null;
      twitchIdCache.set(key, id);
      return id;
    }
  } catch {
    /* connect without backfill */
  }
  twitchIdCache.set(key, null);
  return null;
}

// One raw backend message → OverlayMessage. Twitch badge images resolve through
// the badge cache (keyed by the message's own room-id); non-Twitch providers
// already carry baked badge images + an avatar. Cosmetics (paint/7TV badge/
// third-party) come from the shared user store; StreamNook membership from the
// identity registry (Twitch-keyed, so non-Twitch chatters simply have none).
function adapt(m: BackendChatMessage, users: UsersMap): OverlayMessage {
  const provider = m.provider ?? 'twitch';
  const rawBadges = Array.isArray(m.badges) ? m.badges : [];
  let badges = rawBadges;
  if (provider === 'twitch' && rawBadges.length > 0) {
    const roomId = m.tags?.['room-id'];
    const resolved = parseBadges(rawBadges.map((b) => `${b.name}/${b.version}`).join(','), roomId)
      .filter((r) => r.info)
      .map((r) => {
        const [name, version] = r.key.split('/');
        return { name, version, image_url_4x: twitchBadgeUrl(r.info), title: r.info.title };
      });
    if (resolved.length > 0) badges = resolved;
  }

  const user = m.user_id ? users.get(m.user_id) : undefined;
  const seventvBadge = user?.seventvBadge;
  const extraBadges = (user?.thirdPartyBadges ?? [])
    .filter((b) => b && b.imageUrl)
    .map((b) => ({ url: b.imageUrl as string, title: b.title, source: b.provider }));
  const streamNookUserNumber =
    m.user_id && isStreamNookUser(m.user_id) ? (getStreamNookUserNumber(m.user_id) ?? null) : null;
  // The member's equipped StreamNook cosmetic badge (bundled asset URL); the twin
  // falls back to the default logo when there's none.
  const snSlug = m.user_id && streamNookUserNumber != null ? getActiveCosmeticSlug(m.user_id) : null;
  const streamNookBadgeUrl = snSlug ? (COSMETIC_ASSET_BY_SLUG[snSlug] ?? null) : null;
  const atmosphere = user?.atmosphereId ? toOverlayAtmosphere(getAtmosphere(user.atmosphereId)) : null;

  return {
    ...m,
    badges,
    paint: user?.paint ?? undefined,
    seventvBadgeUrl: seventvBadge ? getBadgeImageUrl(seventvBadge) : undefined,
    seventvBadgeTitle: seventvBadge?.description || seventvBadge?.name,
    extraBadges,
    streamNookUserNumber,
    streamNookBadgeUrl,
    atmosphere,
  };
}

export const LiveOverlayFeed = ({ sources, style, superSample = 1 }: { sources: LiveSource[]; style: OverlayStyle; superSample?: number }) => {
  const usersMap = useChatUserStore((s) => s.users);
  const revision = useChatConnectionStore((s) => s.revision);
  const registeredRef = useRef<Set<string>>(new Set());
  const badgeInitRef = useRef<Set<string>>(new Set());
  // Composite keys currently held (acquired) so the diffing effect can add/remove
  // without tearing down unchanged sources.
  const acquiredRef = useRef<Set<string>>(new Set());
  // First-seen arrival order per message id. Provider timestamps aren't
  // comparable across platforms (Twitch send-time vs YouTube batch-time), which
  // made YouTube messages land mid-list. Ordering by when THIS feed first saw a
  // message is stable and makes every source populate from the bottom.
  const orderRef = useRef({ live: 0, map: new Map<string, number>() });

  const valid = useMemo(
    () => sources.map((s) => ({ provider: s.provider, channel: s.channel.trim() })).filter((s) => s.channel),
    [sources],
  );

  // Diff acquires/releases like MultiChat: ACQUIRE newly-added sources BEFORE
  // releasing removed ones, so swapping a source never drops the shared bridge to
  // zero channels — which would tear down the WS/IRC and race the re-acquire (the
  // "messages stop after swapping a stream" bug). Unchanged sources are untouched.
  useEffect(() => {
    const current = new Set(valid.map((s) => makeKey(s.provider, s.channel)));
    for (const s of valid) {
      const k = makeKey(s.provider, s.channel);
      if (!acquiredRef.current.has(k)) {
        acquiredRef.current.add(k);
        if (s.provider === 'twitch') {
          // Resolve the broadcaster id first so the store backfills recent history
          // on connect (the overlay fills immediately instead of starting empty).
          void resolveTwitchId(s.channel).then((id) => acquireChannel(s.channel, id, 'twitch'));
        } else {
          void acquireChannel(s.channel, null, s.provider);
        }
      }
    }
    for (const k of Array.from(acquiredRef.current)) {
      if (!current.has(k)) {
        acquiredRef.current.delete(k);
        const parsed = parseKey(k);
        void releaseChannel(parsed.channel, parsed.provider);
      }
    }
  }, [valid]);

  // Release everything still held on unmount (leaving the builder, or switching
  // to Sample). The diffing effect above has no per-run cleanup by design.
  useEffect(() => {
    return () => {
      // Intentionally reads the CURRENT held set at unmount time (not a mount-time
      // snapshot) — we release whatever is still acquired.
      // eslint-disable-next-line react-hooks/exhaustive-deps
      const held = acquiredRef.current;
      for (const k of Array.from(held)) {
        const parsed = parseKey(k);
        void releaseChannel(parsed.channel, parsed.provider);
      }
      held.clear();
    };
  }, []);

  // Register Twitch chatters for 7TV cosmetics, and load channel badges once the
  // room-id is known. Non-Twitch chatters have no 7TV cosmetics (their badges +
  // avatar ride the message), so they need no registration.
  useEffect(() => {
    void revision;
    const addUser = useChatUserStore.getState().addUser;
    for (const s of valid) {
      const key = sliceKeyFor(s.provider, s.channel);
      const slice = useChatConnectionStore.getState().channels.get(key);
      if (!slice) continue;
      for (const raw of slice.messages) {
        if (!raw || typeof raw !== 'object') continue;
        const m = raw as BackendChatMessage;
        if ((m.provider ?? 'twitch') !== 'twitch') continue;
        const roomId = m.tags?.['room-id'];
        if (roomId && !badgeInitRef.current.has(key)) {
          badgeInitRef.current.add(key);
          void acquireChannel(s.channel, roomId, 'twitch').then(() => releaseChannel(s.channel, 'twitch'));
        }
        if (m.user_id && !registeredRef.current.has(m.user_id)) {
          registeredRef.current.add(m.user_id);
          addUser({ userId: m.user_id, username: m.username, displayName: m.display_name || m.username, color: m.color || '#9147ff' });
        }
      }
    }
  }, [revision, valid]);

  const overlayMessages = useMemo<OverlayMessage[]>(() => {
    void revision;
    const order = orderRef.current;
    const now = Date.now();
    const rows: { m: BackendChatMessage; seq: number }[] = [];
    for (const s of valid) {
      const slice = useChatConnectionStore.getState().channels.get(sliceKeyFor(s.provider, s.channel));
      if (!slice) continue;
      for (const raw of slice.messages) {
        if (!raw || typeof raw !== 'object') continue;
        const m = raw as BackendChatMessage;
        if (!m.id) continue;
        // Order key, assigned once per id (stable across renders):
        //  - Backfilled HISTORY (seen well after it was sent) sorts by its real
        //    send time, so it lands ABOVE live messages in chronological order.
        //  - LIVE messages sort by ARRIVAL (a monotonic "now" counter), so a
        //    poll-lagged YouTube message shows at the bottom when it arrives, not
        //    mid-list at its older timestamp.
        let seq = order.map.get(m.id);
        if (seq === undefined) {
          const ts = sentAt(m);
          const isHistory = ts > 0 && now - ts > 30_000 && now - ts < 6 * 3_600_000;
          if (isHistory) {
            seq = ts;
          } else {
            order.live = Math.max(order.live + 1, now);
            seq = order.live;
          }
          order.map.set(m.id, seq);
        }
        rows.push({ m, seq });
      }
    }
    rows.sort((a, b) => a.seq - b.seq);
    return rows.map((r) => adapt(r.m, usersMap));
  }, [revision, valid, usersMap]);

  const anyConnected = valid.some((s) => useChatConnectionStore.getState().channels.get(sliceKeyFor(s.provider, s.channel))?.isConnected);
  const status = valid.length === 0
    ? 'Add a source to see live chat.'
    : overlayMessages.length === 0
      ? (anyConnected ? 'Connected — waiting for messages…' : `Connecting to ${valid.map((s) => s.channel).join(', ')}…`)
      : null;

  return (
    <div style={{ position: 'relative', height: '100%', width: '100%' }}>
      <OverlayChat messages={overlayMessages} style={style} superSample={superSample} />
      {status && (
        <div style={{ position: 'absolute', inset: 0, display: 'flex', alignItems: 'center', justifyContent: 'center', pointerEvents: 'none', padding: '0 16px', textAlign: 'center' }}>
          <span style={{ fontSize: 12, color: 'rgba(255,255,255,0.7)', textShadow: '0 1px 2px rgba(0,0,0,0.85)' }}>
            {status}
          </span>
        </div>
      )}
    </div>
  );
};

export default LiveOverlayFeed;
