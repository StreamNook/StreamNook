import { Tooltip } from '../ui/Tooltip';
import { FallbackImage } from '../FallbackImage';
import { StreamNookBadge } from '../StreamNookBadge';
import { BadgeHoverCard } from './BadgeHoverCard';
import { getBadgeImageUrl, getBadgeFallbackUrls } from '../../services/seventvService';
import { openBadgeInfo } from '../../stores/badgeSpotlightStore';
import { sevenTvBadgeRef, thirdPartyBadgeRef, twitchRowBadgeRef } from '../../utils/badgeSpotlight';
import { IS_MOBILE } from '../../utils/platform';
import { useLowPower } from '../../utils/lowPower';
import { CHANNEL_SPECIFIC_TWITCH_BADGES } from '../../utils/badgeOrder';
import { queueBadgeForCaching, getCachedBadgeUrl } from '../../services/badgeImageCacheService';
import type { ThirdPartyBadge } from '../../services/badgeService';
import { Logger } from '../../utils/logger';

/** What a resolved Twitch badge carries (twitchBadges.parseBadges), or a
 *  Kick / YouTube / TikTok badge in the same slot (`provider` set). */
export interface TwitchBadgeInfo {
  /** The Twitch version id (`set/version`'s version). */
  id?: string;
  title?: string;
  description?: string;
  localUrl?: string;
  url?: string;
  image_url_1x?: string;
  image_url_2x?: string;
  image_url_4x?: string;
  click_action?: string | null;
  click_url?: string | null;
  /** The platform for a non-Twitch badge; absent on Twitch's own. */
  provider?: string;
}

export interface ResolvedTwitchBadge {
  key: string;
  info: TwitchBadgeInfo | null;
}

/** The selected 7TV badge, as chatUserStore holds it. */
export type SevenTvBadgeLike = Parameters<typeof getBadgeImageUrl>[0];

const isTwitchBadgeCdn = (url: string | undefined): boolean =>
  !url || url.startsWith('https://static-cdn.jtvnw.net/');

// A badge that fails fails on every row that shows it; one line per source.
const reportedBadgeFailures = new Set<string>();

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
  // badge never appears in another's chat. Art from anywhere but Twitch's CDN
  // is a channel's own (its FFZ moderator / VIP badge) and is never cached
  // under the shared global key either.
  if (CHANNEL_SPECIFIC_TWITCH_BADGES.has(setId) || !isTwitchBadgeCdn(badgeInfo.image_url_4x)) {
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
  /** Twitch badges, already in chat order (Rust ranks them). Kick, YouTube
   *  and TikTok rows put their own badges here too, marked by `provider`. */
  twitchBadges: readonly ResolvedTwitchBadge[];
  seventvBadge: SevenTvBadgeLike | null | undefined;
  thirdPartyBadges: readonly ThirdPartyBadge[];
  /** Member id to draw the StreamNook badge for; null draws none. */
  streamNookMemberId: string | null;
  /** Replaces the default click (the spotlight card) for badges in the Twitch
   *  slot. Hosts pass their own to route elsewhere or (a no-op) to opt out. */
  onBadgeClick?: (badgeKey: string, badgeInfo: TwitchBadgeInfo) => void;
  /** False inside another control (a button): no nested buttons, no clicks. */
  interactive?: boolean;
  keyPrefix?: string;
}

// A flex box around a block image: an inline-block button put the image on its
// text baseline, which sat it lower than the badges beside it. No padding,
// border or background (preflight), so a button is exactly its badge's box and
// the row keeps its height; the focus ring is a box-shadow, outside the flow.
const BADGE_BUTTON_CLASS =
  'inline-flex items-center cursor-pointer rounded-[3px] hover:scale-110 transition-transform focus:outline-none focus-visible:ring-1 focus-visible:ring-accent/70';

/**
 * A chatter's badge icons in chat order: Twitch (role, subscription, earned in
 * the channel, global), then 7TV, then the other third-party providers, then
 * the StreamNook badge rightmost, next to the name. Every chat row and the
 * pinned message draw their badges through this, inside their own strip.
 *
 * Hovering any badge shows its big-art card (only after the pointer really
 * moves over it, so cards never pop as chat scrolls under a parked cursor);
 * clicking one opens the spotlight card in this window. Every clickable badge
 * is a real button, so the keyboard reaches it too.
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
  // Battery Saver: the 7TV badge's published still (utils/lowPower).
  const stillBadges = useLowPower();
  // Touch has no hover and no spotlight host on the phone shell.
  const clickable = interactive && !IS_MOBILE;
  return (
    <>
      {twitchBadges.map((badge, idx) => {
        const info = badge.info;
        if (!info) return null;
        const src = getTwitchBadgeUrl(badge.key, info);
        const ref = twitchRowBadgeRef(badge.key, info, src);
        const img = (
          <img
            src={src}
            alt={info.title}
            className="sn-chat-badge block"
            onError={(e) => {
              // A disk copy that went missing: show Twitch's own image instead.
              const remote = info.image_url_4x || info.image_url_1x;
              const el = e.currentTarget;
              if (remote && src !== remote && !el.dataset.remote) {
                el.dataset.remote = '1';
                el.src = remote;
                return;
              }
              const failed = el.dataset.remote ? remote : src;
              if (failed && !reportedBadgeFailures.has(failed)) {
                reportedBadgeFailures.add(failed);
                Logger.warn('[Badge] Failed to load badge:', badge.key, failed);
              }
              el.style.display = 'none';
            }}
          />
        );
        return (
          <Tooltip
            key={`${keyPrefix}-${badge.key}-${idx}`}
            content={<BadgeHoverCard badge={ref} clickable={clickable} />}
            side="top"
            requirePointerMove
          >
            {clickable ? (
              <button
                type="button"
                aria-label={ref.title}
                className={BADGE_BUTTON_CLASS}
                onClick={() => (onBadgeClick ? onBadgeClick(badge.key, info) : openBadgeInfo(ref))}
              >
                {img}
              </button>
            ) : (
              img
            )}
          </Tooltip>
        );
      })}
      {seventvBadge && (() => {
        const ref = sevenTvBadgeRef(seventvBadge);
        const img = (
          <FallbackImage
            src={getBadgeImageUrl(seventvBadge, stillBadges)}
            fallbackUrls={getBadgeFallbackUrls(seventvBadge.id).slice(1)}
            alt={ref.title}
            className="sn-chat-badge block"
          />
        );
        return (
          <Tooltip content={<BadgeHoverCard badge={ref} clickable={clickable} />} side="top" requirePointerMove>
            {clickable ? (
              <button type="button" aria-label={ref.title} className={BADGE_BUTTON_CLASS} onClick={() => openBadgeInfo(ref)}>
                {img}
              </button>
            ) : (
              img
            )}
          </Tooltip>
        );
      })()}
      {/* The row is a 20px box, so the 2x variant is the right one: `imageUrl`
          is the 4x file, which for Moltorino is a 72px, 120-frame, ~960KB
          animated webp per badge. Member-loadout badges carry one URL for
          every size, so the fallback keeps them unchanged. The hover card
          loads the big file, only while it shows. */}
      {thirdPartyBadges.filter((badge) => badge && badge.imageUrl).map((badge, idx) => {
        const ref = thirdPartyBadgeRef(badge);
        const img = (
          <img
            src={badge.image2x || badge.imageUrl}
            alt={badge.title}
            className="sn-chat-badge block object-contain"
            onError={(e) => {
              e.currentTarget.style.display = 'none';
            }}
          />
        );
        return (
          <Tooltip
            key={`${keyPrefix}-tp-${badge.id}-${idx}`}
            content={<BadgeHoverCard badge={ref} clickable={clickable} />}
            side="top"
            requirePointerMove
          >
            {clickable ? (
              <button type="button" aria-label={ref.title} className={BADGE_BUTTON_CLASS} onClick={() => openBadgeInfo(ref)}>
                {img}
              </button>
            ) : (
              img
            )}
          </Tooltip>
        );
      })}
      {streamNookMemberId && (
        <StreamNookBadge
          userId={streamNookMemberId}
          className="sn-chat-badge sn-chat-badge--sn block object-contain"
          interactive={clickable}
          requirePointerMove
        />
      )}
    </>
  );
}
