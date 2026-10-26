// How a chatter looks outside a chat row: the pinned message and its "pinned
// by" line. Reads the same chatUserStore entry a chat row reads (7TV paint and
// badge, third-party badges), applies the same badge settings, nickname and
// color override, and warms the store for someone who has not chatted yet.
import { useEffect, useMemo, useSyncExternalStore, type CSSProperties } from 'react';
import { useShallow } from 'zustand/react/shallow';
import { useChatUserStore } from '../stores/chatUserStore';
import { useAppStore } from '../stores/AppStore';
import { computePaintStyle } from '../services/seventvService';
import {
  isStreamNookUser,
  subscribeStreamNookRegistryVersion,
  getStreamNookRegistryVersion,
} from '../services/supabaseService';
import type { ThirdPartyBadge } from '../services/badgeService';
import type { ResolvedTwitchBadge, SevenTvBadgeLike } from '../components/chat/ChatBadgeIcons';
import { visibleChatterBadges } from '../utils/chatterBadges';
import { cosmeticsKey } from '../utils/chatterIdentity';
import { memberIdFor } from '../utils/memberIdentity';
import { getColorOverride, getDisplayedName } from '../utils/userChatOverrides';
import { useNameColorAdjust } from './useNameColor';
import { useLowPower } from '../utils/lowPower';
import type { ProviderId } from '../types/providers';

const NO_TWITCH_BADGES: ResolvedTwitchBadge[] = [];
const NO_THIRD_PARTY: ThirdPartyBadge[] = [];

export interface ChatterLookInput {
  provider: ProviderId;
  /** Raw platform user id; empty when the platform did not send one. */
  userId: string;
  name: string;
  /** The chatter's login, when known; keys a new chatter-store entry. */
  login?: string;
  /** The name chat rows show (Rust's shown name), when the source carries it. */
  shownName?: string;
  /** Platform name color, if the source carried one. */
  color?: string;
  /** Twitch badges in chat order, already resolved to images. */
  twitchBadges?: readonly ResolvedTwitchBadge[];
}

export interface ChatterLook {
  /** Nickname override, else the name passed in. */
  displayName: string;
  /** Paint style when the chatter wears one, else their (readable) color. */
  nameStyle: CSSProperties;
  painted: boolean;
  twitchBadges: readonly ResolvedTwitchBadge[];
  seventvBadge: SevenTvBadgeLike | null;
  thirdPartyBadges: readonly ThirdPartyBadge[];
  /** Member id for the StreamNook badge, or null when it should not show. */
  streamNookMemberId: string | null;
  hasBadges: boolean;
}

export function useChatterLook({ provider, userId, name, login, shownName, color, twitchBadges }: ChatterLookInput): ChatterLook {
  // Same key space as ChatMessage's cosmeticsKey: bare Twitch id, `provider:id`
  // for every other platform.
  const key = userId ? cosmeticsKey(userId, provider) : null;

  const { inStore, paint, seventvBadgeRaw, thirdPartyAll, storeColor } = useChatUserStore(
    useShallow((s) => {
      const u = key ? s.users.get(key) : undefined;
      return {
        inStore: u !== undefined,
        paint: u?.paint,
        seventvBadgeRaw: u?.seventvBadge,
        thirdPartyAll: u?.thirdPartyBadges ?? NO_THIRD_PARTY,
        storeColor: u?.color,
      };
    }),
  );

  // Someone who has not chatted this session has no entry, so nothing resolved
  // their cosmetics. addUser is the one resolver chat rows use; it only runs
  // for a missing entry, because for a known chatter it would overwrite their
  // login with this display name. Re-runs if the entry is evicted or cleared.
  useEffect(() => {
    if (!key || inStore) return;
    useChatUserStore.getState().addUser({
      userId: key,
      // The store is keyed by login; a localized display name lowercased is not one.
      username: login || (provider === 'twitch' ? name.toLowerCase() : name),
      displayName: name,
      color: color || '',
    });
  }, [key, inStore, provider, name, login, color]);

  // Membership loads after first paint; re-render when it lands.
  useSyncExternalStore(subscribeStreamNookRegistryVersion, getStreamNookRegistryVersion, getStreamNookRegistryVersion);

  const showBadges = useAppStore((s) => s.settings.chat_design?.show_badges);
  const showThirdParty = useAppStore((s) => s.settings.chat_design?.show_third_party_badges);
  const hiddenProviders = useAppStore((s) => s.settings.chat_design?.hidden_badge_providers);
  const overrides = useAppStore((s) => s.settings.chat_customization?.user_overrides);
  const paintShadowMode = useAppStore((s) => s.settings.cosmetics?.paint_shadows) ?? 'all';
  const adjustNameColor = useNameColorAdjust();

  const visible = useMemo(
    () =>
      visibleChatterBadges(
        { show_badges: showBadges, show_third_party_badges: showThirdParty, hidden_badge_providers: hiddenProviders },
        twitchBadges ?? NO_TWITCH_BADGES,
        seventvBadgeRaw as SevenTvBadgeLike | null | undefined,
        thirdPartyAll,
      ),
    [showBadges, showThirdParty, hiddenProviders, twitchBadges, seventvBadgeRaw, thirdPartyAll],
  );

  // Battery Saver draws the paint's first frame (utils/lowPower), as chat rows do.
  const stillPaint = useLowPower();
  const baseColor = adjustNameColor(getColorOverride(key, overrides) ?? (color || storeColor || null));
  const nameStyle = useMemo<CSSProperties>(() => {
    if (paint) return computePaintStyle(paint, baseColor, paintShadowMode, stillPaint);
    return baseColor ? { color: baseColor } : {};
  }, [paint, baseColor, paintShadowMode, stillPaint]);

  const memberId = memberIdFor(key);
  const streamNookMemberId = visible.streamNook && memberId && isStreamNookUser(memberId) ? memberId : null;

  return {
    displayName: getDisplayedName(key, shownName || name, overrides),
    nameStyle,
    painted: !!paint,
    twitchBadges: visible.twitch,
    seventvBadge: visible.seventv,
    thirdPartyBadges: visible.thirdParty,
    streamNookMemberId,
    hasBadges:
      visible.twitch.length > 0 || !!visible.seventv || visible.thirdParty.length > 0 || !!streamNookMemberId,
  };
}
