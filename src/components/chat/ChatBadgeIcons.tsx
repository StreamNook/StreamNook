import { Tooltip } from '../ui/Tooltip';
import { FallbackImage } from '../FallbackImage';
import { StreamNookBadge } from '../StreamNookBadge';
import { getBadgeImageUrl, getBadgeFallbackUrls } from '../../services/seventvService';
import { openBadgesWithBadgeInMain } from '../../utils/openBadgesInMain';
import { CHANNEL_SPECIFIC_TWITCH_BADGES } from '../../utils/badgeOrder';
import { queueBadgeForCaching, getCachedBadgeUrl } from '../../services/badgeImageCacheService';
import type { ThirdPartyBadge } from '../../services/badgeService';
import { Logger } from '../../utils/logger';

/** What a resolved Twitch badge carries (twitchBadges.parseBadges). */
export interface TwitchBadgeInfo {
  title?: string;
  localUrl?: string;
  url?: string;
  image_url_1x?: string;
  image_url_2x?: string;
  image_url_4x?: string;
}

export interface ResolvedTwitchBadge {
  key: string;
  info: TwitchBadgeInfo | null;
}

/** The selected 7TV badge, as chatUserStore holds it. */
export type SevenTvBadgeLike = Parameters<typeof getBadgeImageUrl>[0];

/**
 * Get the best URL for a Twitch badge, with reactive caching.
 * - Channel-specific badges (subscriber, bits, etc.) are never cached to avoid cross-channel pollution
 * - Global badges are cached locally for faster loading
 */
function getTwitchBadgeUrl(badgeKey: string, badgeInfo: TwitchBadgeInfo): string {
  if (badgeInfo.localUrl) {
    return badgeInfo.localUrl;
  }

  // Badge key format: "set/version"
  const setId = badgeKey.split('/')[0];

  // Channel-specific badges always use the remote URL, so one streamer's sub
  // badge never appears in another's chat.
  if (CHANNEL_SPECIFIC_TWITCH_BADGES.has(setId)) {
    return badgeInfo.image_url_4x || badgeInfo.image_url_1x || '';
  }

  const cacheId = `twitch-${badgeKey}`;
  const cachedUrl = getCachedBadgeUrl(cacheId);
  if (cachedUrl) {
    return cachedUrl;
  }

  const remoteUrl = badgeInfo.image_url_4x;
  if (remoteUrl) {
    queueBadgeForCaching(cacheId, remoteUrl);
  }

  return remoteUrl || badgeInfo.image_url_1x || '';
}

interface ChatBadgeIconsProps {
  /** Twitch badges, already in chat order (Rust ranks them). */
  twitchBadges: readonly ResolvedTwitchBadge[];
  seventvBadge: SevenTvBadgeLike | null | undefined;
  thirdPartyBadges: readonly ThirdPartyBadge[];
  /** Member id to draw the StreamNook badge for; null draws none. */
  streamNookMemberId: string | null;
  onBadgeClick?: (badgeKey: string, badgeInfo: TwitchBadgeInfo) => void;
  /** False inside another control (a button): no nested buttons, no clicks. */
  interactive?: boolean;
  keyPrefix?: string;
}

/**
 * A chatter's badge icons in chat order: Twitch (role, subscription, earned in
 * the channel, global), then 7TV, then the other third-party providers, then
 * the StreamNook badge rightmost, next to the name. Every chat row and the
 * pinned message draw their badges through this, inside their own strip.
 */
export function ChatBadgeIcons({
  twitchBadges,
  seventvBadge,
  thirdPartyBadges,
  streamNookMemberId,
  onBadgeClick,
  interactive = true,
  keyPrefix = 'badge',
}: ChatBadgeIconsProps) {
  return (
    <>
      {twitchBadges.map((badge, idx) => {
        const info = badge.info;
        if (!info) return null;
        return (
          <Tooltip key={`${keyPrefix}-${badge.key}-${idx}`} content={info.title} side="top">
            <img
              src={getTwitchBadgeUrl(badge.key, info)}
              alt={info.title}
              className={`sn-chat-badge${interactive ? ' cursor-pointer hover:scale-110 transition-transform' : ''}`}
              onClick={interactive ? () => onBadgeClick?.(badge.key, info) : undefined}
              onError={(e) => {
                Logger.warn('[Badge] Failed to load badge:', badge.key, info.image_url_1x);
                e.currentTarget.style.display = 'none';
              }}
            />
          </Tooltip>
        );
      })}
      {seventvBadge && (
        interactive ? (
          <Tooltip content={`Click for details: ${seventvBadge.description || seventvBadge.name}`} side="top">
            <button
              type="button"
              onClick={() => openBadgesWithBadgeInMain(seventvBadge.id)}
              // A flex box around a block image: an inline-block button put the
              // image on its text baseline, which sat it lower than the badges
              // beside it.
              className="inline-flex items-center cursor-pointer hover:scale-110 transition-transform"
            >
              <FallbackImage
                src={getBadgeImageUrl(seventvBadge)}
                fallbackUrls={getBadgeFallbackUrls(seventvBadge.id).slice(1)}
                alt={seventvBadge.description || seventvBadge.name}
                className="sn-chat-badge block"
              />
            </button>
          </Tooltip>
        ) : (
          <Tooltip content={seventvBadge.description || seventvBadge.name} side="top">
            <FallbackImage
              src={getBadgeImageUrl(seventvBadge)}
              fallbackUrls={getBadgeFallbackUrls(seventvBadge.id).slice(1)}
              alt={seventvBadge.description || seventvBadge.name}
              className="sn-chat-badge block"
            />
          </Tooltip>
        )
      )}
      {/* The row is a 20px box, so the 2x variant is the right one: `imageUrl`
          is the 4x file, which for Moltorino is a 72px, 120-frame, ~960KB
          animated webp per badge. Member-loadout badges carry one URL for
          every size, so the fallback keeps them unchanged. */}
      {thirdPartyBadges.filter((badge) => badge && badge.imageUrl).map((badge, idx) => (
        <Tooltip key={`${keyPrefix}-tp-${badge.id}-${idx}`} content={`${badge.title} (${badge.provider.toUpperCase()})`} side="top">
          <img
            src={badge.image2x || badge.imageUrl}
            alt={badge.title}
            className="sn-chat-badge block object-contain"
            onError={(e) => {
              e.currentTarget.style.display = 'none';
            }}
          />
        </Tooltip>
      ))}
      {streamNookMemberId && (
        <StreamNookBadge userId={streamNookMemberId} className="sn-chat-badge sn-chat-badge--sn block object-contain" />
      )}
    </>
  );
}
