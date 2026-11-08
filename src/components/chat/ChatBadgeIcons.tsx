import { Fragment, type ReactNode } from 'react';
import { Tooltip } from '../ui/Tooltip';
import { FallbackImage } from '../FallbackImage';
import { StreamNookBadge } from '../StreamNookBadge';
import { BadgeHoverCard } from './BadgeHoverCard';
import { BadgeFoldChip, BadgeFoldItem } from './BadgeFold';
import { getBadgeImageUrl, getBadgeFallbackUrls } from '../../services/seventvService';
import { openBadgeInfo } from '../../stores/badgeSpotlightStore';
import { sevenTvBadgeRef, thirdPartyBadgeRef, twitchRowBadgeRef } from '../../utils/badgeSpotlight';
import { IS_MOBILE } from '../../utils/platform';
import { useStillChat } from '../../utils/lowPower';
import { CHANNEL_SPECIFIC_TWITCH_BADGES } from '../../utils/badgeOrder';
import { queueBadgeForCaching, getCachedBadgeUrl } from '../../services/badgeImageCacheService';
import type { ThirdPartyBadge } from '../../services/badgeService';
import { Logger } from '../../utils/logger';
import { boundedAdd } from '../../utils/boundedMap';
import { foldBadges, foldKind, type FoldKind } from '../../utils/badgeFold';
import { markBadgeArtFailed, useFailedBadgeArt } from '../../utils/badgeArtFailures';

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
const REPORTED_BADGE_FAILURES_MAX = 1000;

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
  /** Badges to show before the rest fold into a "+N" chip, normalized by
   *  `badgeFoldLimit`. 0 (the default) folds nothing. */
  foldLimit?: number;
  keyPrefix?: string;
  /** Wraps each badge, given an id naming the cosmetic it draws (`tw-<set>/<v>`,
   *  `7tv-<id>`, `tp-<provider>-<id>`, `sn`). Live previews use it to play a
   *  swap on the badge that just changed; chat rows leave it out. */
  wrapSlot?: (id: string, node: ReactNode) => ReactNode;
}

// A flex box around a block image: an inline-block button put the image on its
// text baseline, which sat it lower than the badges beside it. No padding,
// border or background (preflight), so a button is exactly its badge's box and
// the row keeps its height; the focus ring is a box-shadow, outside the flow.
const BADGE_BUTTON_CLASS =
  'inline-flex items-center cursor-pointer rounded-[3px] hover:scale-110 transition-transform focus:outline-none focus-visible:ring-1 focus-visible:ring-accent/70';

/** One drawable badge: its fold kind, its art url ('' for the StreamNook mark),
 *  Twitch's own copy of that art when one exists, and its finished element. */
interface BadgeSlot {
  key: string;
  /** The cosmetic it draws, for `wrapSlot`. */
  id: string;
  kind: FoldKind;
  url: string;
  remote?: string;
  node: ReactNode;
}

/**
 * A chatter's badge icons in chat order: Twitch (role, subscription, earned in
 * the channel, global), then 7TV, then the other third-party providers, then
 * the StreamNook badge rightmost, next to the name. Every chat row and the
 * pinned message draw their badges through this, inside their own strip.
 * Past the row's limit the rest fold into a "+N" chip (`BadgeFold`; the rule is
 * `utils/badgeFold`, shared with Rust).
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
  foldLimit = 0,
  keyPrefix = 'badge',
  wrapSlot,
}: ChatBadgeIconsProps) {
  // Battery Saver or Freeze emotes: the 7TV badge's published still (utils/lowPower).
  const stillBadges = useStillChat();
  // A tap on the phone opens the badge sheet (openBadgeInfo); touch has no
  // hover, so the hover card never offers a click there.
  const clickable = interactive;
  const hoverClickable = interactive && !IS_MOBILE;
  const slots: BadgeSlot[] = [];

  twitchBadges.forEach((badge, idx) => {
    const info = badge.info;
    if (!info) return;
    const src = getTwitchBadgeUrl(badge.key, info);
    const ref = twitchRowBadgeRef(badge.key, info, src);
    const img = (
      <img
        src={src}
        alt={info.title}
        className="sn-chat-badge block"
        loading="lazy"
        decoding="async"
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
            boundedAdd(reportedBadgeFailures, failed, REPORTED_BADGE_FAILURES_MAX);
            Logger.warn('[Badge] Failed to load badge:', badge.key, failed);
          }
          el.style.display = 'none';
          markBadgeArtFailed(src, remote);
        }}
      />
    );
    slots.push({
      key: `${keyPrefix}-${badge.key}-${idx}`,
      id: `tw-${badge.key}`,
      kind: foldKind(info.provider ?? 'twitch', badge.key.split('/')[0]),
      url: src,
      remote: info.image_url_4x || info.image_url_1x,
      node: (
        <Tooltip content={<BadgeHoverCard badge={ref} clickable={hoverClickable} />} side="top" requirePointerMove>
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
      ),
    });
  });

  if (seventvBadge) {
    const ref = sevenTvBadgeRef(seventvBadge);
    const url = getBadgeImageUrl(seventvBadge, stillBadges);
    const img = (
      <FallbackImage
        src={url}
        fallbackUrls={getBadgeFallbackUrls(seventvBadge.id).slice(1)}
        alt={ref.title}
        className="sn-chat-badge block"
        onError={() => markBadgeArtFailed(url)}
      />
    );
    slots.push({
      key: `${keyPrefix}-7tv`,
      id: `7tv-${seventvBadge.id}`,
      kind: 'seventv',
      url,
      node: (
        <Tooltip content={<BadgeHoverCard badge={ref} clickable={hoverClickable} />} side="top" requirePointerMove>
          {clickable ? (
            <button type="button" aria-label={ref.title} className={BADGE_BUTTON_CLASS} onClick={() => openBadgeInfo(ref)}>
              {img}
            </button>
          ) : (
            img
          )}
        </Tooltip>
      ),
    });
  }

  // The row is a 20px box, so the 2x variant is the right one: `imageUrl`
  // is the 4x file, which for Moltorino is a 72px, 120-frame, ~960KB
  // animated webp per badge. Member-loadout badges carry one URL for
  // every size, so the fallback keeps them unchanged. The hover card
  // loads the big file, only while it shows.
  thirdPartyBadges
    .filter((badge) => badge && badge.imageUrl)
    .forEach((badge, idx) => {
      const ref = thirdPartyBadgeRef(badge);
      const url = badge.image2x || badge.imageUrl;
      const img = (
        <img
          src={url}
          alt={badge.title}
          className="sn-chat-badge block object-contain"
          loading="lazy"
          decoding="async"
          onError={(e) => {
            e.currentTarget.style.display = 'none';
            markBadgeArtFailed(url);
          }}
        />
      );
      slots.push({
        key: `${keyPrefix}-tp-${badge.id}-${idx}`,
        id: `tp-${badge.provider}-${badge.id}`,
        kind: 'addon',
        url,
        node: (
          <Tooltip content={<BadgeHoverCard badge={ref} clickable={hoverClickable} />} side="top" requirePointerMove>
            {clickable ? (
              <button type="button" aria-label={ref.title} className={BADGE_BUTTON_CLASS} onClick={() => openBadgeInfo(ref)}>
                {img}
              </button>
            ) : (
              img
            )}
          </Tooltip>
        ),
      });
    });

  if (streamNookMemberId) {
    slots.push({
      key: `${keyPrefix}-sn`,
      id: 'sn',
      kind: 'streamnook',
      url: '',
      node: (
        <StreamNookBadge
          userId={streamNookMemberId}
          className="sn-chat-badge sn-chat-badge--sn block object-contain"
          interactive={clickable}
          requirePointerMove
        />
      ),
    });
  }

  // Art that finally failed to load is dropped before folding, so a dead image
  // leaves no gap and no "+N" that opens onto nothing.
  const failedArt = useFailedBadgeArt(slots.map((s) => s.url));
  const drawable = slots.filter((s) => !s.url || !failedArt.includes(`${s.url}\n`));
  const fold = foldBadges(
    drawable.map((s) => s.kind),
    foldLimit,
  );

  return (
    <>
      {fold.shown.map((i) => (
        <Fragment key={drawable[i].key}>
          {wrapSlot ? wrapSlot(drawable[i].id, drawable[i].node) : drawable[i].node}
        </Fragment>
      ))}
      {fold.hidden.length > 0 && (
        <BadgeFoldChip
          count={fold.hidden.length}
          peeks={fold.hidden
            .map((i) => drawable[i])
            .filter((s) => s.url)
            .slice(0, 2)
            .map((s) => ({ url: s.url, remote: s.remote }))}
          tier={fold.tier}
          still={!interactive}
        >
          {() =>
            fold.hidden.map((i, n) => (
              <BadgeFoldItem key={drawable[i].key} index={n}>
                {drawable[i].node}
              </BadgeFoldItem>
            ))
          }
        </BadgeFoldChip>
      )}
    </>
  );
}
