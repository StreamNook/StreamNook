// Live feed for the builder preview. Connects one or more real sources across
// ANY provider through the app's own chat pipeline (ref-counted
// chatConnectionStore → Rust IRC / provider adapters) and MERGES them into one
// feed, exactly like MultiChat — inside the app all providers connect (the
// hosted OBS page is the only thing that needs the later backend). Cosmetics
// resolve the same way the real chat row does; everything renders through the
// SAME twin renderer the OBS overlay uses.

import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { helixGet } from '../../services/helix';
import {
  acquireChannel,
  releaseChannel,
  useChatConnectionStore,
} from '../../stores/chatConnectionStore';
import { makeKey, parseKey } from '../../utils/providerKey';
import { badgeSourceKey, HOMIES_CUSTOM_SOURCE } from '../../utils/chatterBadges';
import { initializeBadgeCache, parseBadges } from '../../services/twitchBadges';
import { useChatUserStore } from '../../stores/chatUserStore';
import {
  isStreamNookUser,
  getStreamNookUserNumber,
  getActiveCosmeticSlug,
  getCosmeticBySlug,
  getCosmeticsVersion,
  subscribeCosmeticsVersion,
} from '../../services/supabaseService';
import { getBadgeImageUrl } from '../../services/seventvService';
import { resolveCosmeticAsset } from '../cosmeticAssets';
import { getAtmosphere } from '../../services/atmospheres';
import type { Atmosphere } from '../../services/atmospheres';
import type { BackendChatMessage } from '../../services/twitchChat';
import type { ProviderId } from '../../types/providers';
import { OverlayChat } from './OverlayChat';
import { createSourceHolds } from './liveSourceHolds';
import { badgeRoomOf, isSystemRow, overlayTwitchBadges, systemRowForOverlay, type RoomRoleArt } from './liveOverlayRows';
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
function adapt(
  m: BackendChatMessage,
  users: UsersMap,
  roleArtFor: (roomId: string | undefined) => RoomRoleArt | null | undefined,
  sourceRoomId: string | null,
): OverlayMessage {
  const provider = m.provider ?? 'twitch';
  const rawBadges = Array.isArray(m.badges) ? m.badges : [];
  let badges = rawBadges;
  if (provider === 'twitch' && rawBadges.length > 0) {
    const roleArt = roleArtFor(m.tags?.['room-id'] || sourceRoomId || undefined);
    badges = overlayTwitchBadges(rawBadges, m.tags, parseBadges, roleArt, sourceRoomId) ?? rawBadges;
  }

  // Two different id spaces, and reading one as the other shows a stranger's
  // cosmetics under someone's name. The chat store is keyed the way chat keys
  // it: bare for Twitch, `provider:id` otherwise. A bare Kick id read as a chat
  // key returns whichever Twitch chatter shares that number.
  const cosmeticsKey =
    m.user_id ? (provider === 'twitch' ? m.user_id : `${provider}:${m.user_id}`) : undefined;
  const user = cosmeticsKey ? users.get(cosmeticsKey) : undefined;
  const seventvBadge = user?.seventvBadge;
  const extraBadges = (user?.thirdPartyBadges ?? [])
    .filter((b) => b && b.imageUrl)
    .map((b) => ({
      url: b.imageUrl as string,
      title: b.title,
      source: b.provider,
      // Lets the overlay's Homies custom entry hide this badge on its own.
      category: badgeSourceKey(b) === HOMIES_CUSTOM_SOURCE ? HOMIES_CUSTOM_SOURCE : b.category,
    }));
  // Membership is a Twitch id, so only a Twitch chatter can be looked up. Kick
  // ids are small integers from the same range as Twitch's, so passing one here
  // matches an unrelated member and paints their number and badge.
  const memberId = provider === 'twitch' ? m.user_id : undefined;
  const streamNookUserNumber =
    memberId && isStreamNookUser(memberId) ? (getStreamNookUserNumber(memberId) ?? null) : null;
  // The member's equipped StreamNook cosmetic badge: the bundled asset, else the
  // catalog's CDN file. The twin falls back to the default logo when there's none.
  const snSlug = memberId && streamNookUserNumber != null ? getActiveCosmeticSlug(memberId) : null;
  const streamNookBadgeUrl = snSlug ? resolveCosmeticAsset(getCosmeticBySlug(snSlug)) : null;
  const atmosphere =
    user?.atmosphereId && !user.themeHiddenInChat ? toOverlayAtmosphere(getAtmosphere(user.atmosphereId)) : null;

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
  // Members' StreamNook badges resolve from the cosmetics registry, which can
  // land after the rows it decorates; a quiet chat would keep the default mark.
  const cosmeticsVersion = useSyncExternalStore(subscribeCosmeticsVersion, getCosmeticsVersion, getCosmeticsVersion);
  // Each channel's FFZ moderator and VIP art, read from Rust while the overlay
  // shows custom role art (by room id; null = the room has none).
  const customRoleArt = style.ffzCustomRoleBadges === true;
  const [roleArt, setRoleArt] = useState<ReadonlyMap<string, RoomRoleArt | null>>(() => new Map());
  const roleArtAskedRef = useRef<Set<string>>(new Set());
  const registeredRef = useRef<Set<string>>(new Set());
  const badgeInitRef = useRef<Set<string>>(new Set());
  const sharedBadgeRoomsRef = useRef<Set<string>>(new Set());
  const [badgeRoomsLoaded, setBadgeRoomsLoaded] = useState(0);
  // The composite keys this feed wants and the ones it holds (see
  // liveSourceHolds: a Twitch acquire waits on the broadcaster id).
  const [holds] = useState(() =>
    createSourceHolds({
      acquire: (k, id) => {
        const parsed = parseKey(k);
        void acquireChannel(parsed.channel, id, parsed.provider);
      },
      release: (k) => {
        const parsed = parseKey(k);
        void releaseChannel(parsed.channel, parsed.provider);
      },
    }),
  );
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
    holds.want(valid.map((s) => makeKey(s.provider, s.channel)), (k) => {
      const parsed = parseKey(k);
      if (parsed.provider === 'twitch') {
        // Resolve the broadcaster id first so the store backfills recent history
        // on connect (the overlay fills immediately instead of starting empty).
        void resolveTwitchId(parsed.channel).then((id) => holds.take(k, id));
      } else {
        holds.take(k, null);
      }
    });
  }, [valid, holds]);

  // Release everything still held on unmount (leaving the builder, or switching
  // to Sample). The diffing effect above has no per-run cleanup by design.
  useEffect(() => {
    return () => holds.dropAll();
  }, [holds]);

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
        // A shared-chat row wears the badges of the channel it was sent in,
        // so that channel's set loads too, as the chat loads it.
        const badgeRoom = badgeRoomOf(m.tags);
        if (badgeRoom && badgeRoom !== roomId && !sharedBadgeRoomsRef.current.has(badgeRoom)) {
          sharedBadgeRoomsRef.current.add(badgeRoom);
          void initializeBadgeCache(badgeRoom).then(() => setBadgeRoomsLoaded((n) => n + 1));
        }
        if (roomId && customRoleArt && !roleArtAskedRef.current.has(roomId)) {
          roleArtAskedRef.current.add(roomId);
          void invoke<RoomRoleArt | null>('get_ffz_room_role_art', { channelId: roomId })
            .then((art) => setRoleArt((prev) => new Map(prev).set(roomId, art ?? null)))
            .catch(() => roleArtAskedRef.current.delete(roomId));
        }
        if (m.user_id && !registeredRef.current.has(m.user_id)) {
          registeredRef.current.add(m.user_id);
          addUser({ userId: m.user_id, username: m.username, displayName: m.display_name || m.username, color: m.color || '#9147ff' });
        }
      }
    }
  }, [revision, valid, customRoleArt]);

  const overlayMessages = useMemo<OverlayMessage[]>(() => {
    void revision;
    void cosmeticsVersion;
    void badgeRoomsLoaded;
    const roleArtFor = (roomId: string | undefined): RoomRoleArt | null | undefined =>
      customRoleArt ? (roomId ? roleArt.get(roomId) ?? null : null) : undefined;
    const order = orderRef.current;
    const now = Date.now();
    const rows: { m: BackendChatMessage; seq: number; provider: ProviderId; roomId: string | null }[] = [];
    for (const s of valid) {
      const slice = useChatConnectionStore.getState().channels.get(sliceKeyFor(s.provider, s.channel));
      if (!slice) continue;
      // The source's broadcaster id, for a row that arrives without its own.
      const roomId = s.provider === 'twitch' ? (twitchIdCache.get(s.channel.toLowerCase()) ?? null) : null;
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
        rows.push({ m, seq, provider: s.provider, roomId });
      }
    }
    rows.sort((a, b) => a.seq - b.seq);
    const out: OverlayMessage[] = [];
    for (const r of rows) {
      if (isSystemRow(r.m)) {
        const row = systemRowForOverlay(r.m, r.provider);
        if (row) out.push(row);
        continue;
      }
      out.push(adapt(r.m, usersMap, roleArtFor, r.roomId));
    }
    return out;
  }, [revision, valid, usersMap, cosmeticsVersion, customRoleArt, roleArt, badgeRoomsLoaded]);

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
