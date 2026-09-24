// Touch-native stream card in StreamNook's own card language, matching the
// desktop Home cards: padded glass panel, rounded thumbnail, the canonical
// CardChip (LIVE, DROPS, hype train, watch-streak flame),
// .glass-badge viewer chip, a "+2" beside the name while the channel streams
// with others, partner verified mark, and Apple-style emoji titles.
// Only the sizing is phone-tuned.
import React from 'react';
import { Flame } from 'lucide-react';
import { Package, UsersThree } from 'phosphor-react';
import StreamTitleWithEmojis from '../../components/StreamTitleWithEmojis';
import { campaignEarnableOn } from '../dropsEligibility';
import type { DropsByGame } from '../dropsCampaigns';
import type { TwitchStream } from '../../types';
import { previewStamp } from '../followRefresh';
import { ProviderMark } from '../../components/ProviderLogo';
import { isTwitchStream, streamProvider } from '../../utils/streamProvider';
import { CardChip } from '../../components/ui/CardChip';
import { TogetherTag } from '../../components/SharedViewers';
import { useAppStore } from '../../stores/AppStore';
import { collabFor } from '../../utils/sharedViewers';

// The stamp is what makes a refreshed list show refreshed previews: Twitch's
// preview URL is fixed per channel and the WebView caches it, so without a
// changing query string a pull-to-refresh brought new titles over old frames.
function thumbUrl(stream: TwitchStream): string {
  const base = stream.thumbnail_url.replace('{width}', '640').replace('{height}', '360');
  if (!base) return base;
  return `${base}${base.includes('?') ? '&' : '?'}sn=${previewStamp()}`;
}

export interface HypeTrainBadgeInfo {
  level: number;
  isGolden?: boolean;
}

const HypeTrainBadge: React.FC<{ info: HypeTrainBadgeInfo }> = ({ info }) => (
  <CardChip kind={info.isGolden ? 'hype-golden' : 'hype'}>
    <svg className="w-2.5 h-2.5" viewBox="0 0 15 13" fill="none">
      <path
        fillRule="evenodd"
        clipRule="evenodd"
        d="M4.10001 0.549988H2.40001V4.79999H0.700012V10.75H1.55001C1.55001 11.6889 2.31113 12.45 3.25001 12.45C4.1889 12.45 4.95001 11.6889 4.95001 10.75H5.80001C5.80001 11.6889 6.56113 12.45 7.50001 12.45C8.4389 12.45 9.20001 11.6889 9.20001 10.75H10.05C10.05 11.6889 10.8111 12.45 11.75 12.45C12.6889 12.45 13.45 11.6889 13.45 10.75H14.3V0.549988H6.65001V2.24999H7.50001V4.79999H4.10001V0.549988ZM12.6 9.04999V6.49999H2.40001V9.04999H12.6ZM9.20001 4.79999H12.6V2.24999H9.20001V4.79999Z"
        fill="currentColor"
      />
    </svg>
    <span>LVL {info.level}</span>
  </CardChip>
);

const StreakBadge: React.FC<{ streak: number }> = ({ streak }) => (
  // The shared card chip, flat: an opaque-enough fill rather than a backdrop
  // blur, which would be a composited layer PER CARD on a device whose whole
  // cost is compositing, and over a thumbnail the two read the same.
  <CardChip kind="streak" flat>
    <Flame size={10} className="stroke-[2.5]" />
    <span>{streak}</span>
  </CardChip>
);

export const MobileStreamCard: React.FC<{
  stream: TwitchStream;
  dropsGameNames?: DropsByGame;
  hypeTrain?: HypeTrainBadgeInfo;
  watchStreak?: number;
  onPress: (stream: TwitchStream) => void;
  /** Mark non-Twitch rows with their platform. Off where the screen already
   *  says which platform every row is on (Browse switched to Kick). */
  showPlatform?: boolean;
  /** 'card' = big thumbnail stack; 'row' = compact list row (thumb left). */
  variant?: 'card' | 'row';
}> = ({ stream, dropsGameNames, hypeTrain, watchStreak, onPress, variant = 'card', showPlatform = true }) => {
  // The icon means "you can earn drops HERE", not "this game has drops".
  //
  // It used to mean the latter, which put a gift on every channel in a
  // drops-enabled category including the ones a restricted campaign excludes.
  // Tapping through then showed no progress and nothing explained why. There is
  // deliberately no third state for "this category has drops but not on this
  // channel": that is a promise the channel cannot keep, and a card is the
  // wrong place to explain someone else's campaign rules.
  const isTwitch = isTwitchStream(stream);
  // Twitch's Shared Viewership, from the Rust home snapshot. Read here rather
  // than passed in, so every screen's cards carry it.
  const collab = useAppStore((s) => collabFor(s.collaborations, stream));
  // Drops are Twitch campaigns matched by category name; a Kick stream in the
  // same category earns nothing.
  const hasDrops = isTwitch && !!(
    stream.game_name &&
    (dropsGameNames?.get(stream.game_name.toLowerCase()) ?? []).some((c) =>
      campaignEarnableOn(c, stream.user_login),
    )
  );

  if (variant === 'row') {
    return (
      <button
        onClick={() => onPress(stream)}
        className="w-full text-left glass-panel media-card p-2 flex gap-2.5 active:opacity-80 transition-opacity"
      >
        <div className="relative w-[156px] shrink-0 overflow-hidden rounded self-center">
          <img
            loading="lazy"
            decoding="async"
            src={thumbUrl(stream)}
            alt=""
            className="w-full aspect-video object-cover"
            draggable={false}
          />
          {/* Bare live dot instead of the pill: rows are too small for chrome.
              A ping halo keeps it visible without adding chrome. */}
          <span className="absolute top-1.5 left-1.5 flex w-2 h-2">
            <span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-live opacity-60" />
            <span className="relative inline-flex w-2 h-2 rounded-full bg-live ring-1 ring-black/40" />
          </span>
          {/* Drops matter as much in list mode as in card mode, and the row had
              no way to say so. Icon only: no space for the DROPS wordmark the
              card carries. */}
          {hasDrops && (
            <CardChip
              kind="drops"
              style={{
                position: 'absolute',
                top: 4,
                right: 4,
                fontSize: 8,
                padding: '2px 3px',
                gap: 0,
              }}
              aria-label="Drops enabled"
            >
              <Package size={10} />
            </CardChip>
          )}
          {hypeTrain && (
            <CardChip
              kind={hypeTrain.isGolden ? 'hype-golden' : 'hype'}
              style={{
                position: 'absolute',
                bottom: 4,
                left: 4,
                fontSize: 8,
                padding: '1px 4px',
                gap: 2,
              }}
            >
              <svg className="w-2 h-2" viewBox="0 0 15 13" fill="none">
                <path
                  fillRule="evenodd"
                  clipRule="evenodd"
                  d="M4.10001 0.549988H2.40001V4.79999H0.700012V10.75H1.55001C1.55001 11.6889 2.31113 12.45 3.25001 12.45C4.1889 12.45 4.95001 11.6889 4.95001 10.75H5.80001C5.80001 11.6889 6.56113 12.45 7.50001 12.45C8.4389 12.45 9.20001 11.6889 9.20001 10.75H10.05C10.05 11.6889 10.8111 12.45 11.75 12.45C12.6889 12.45 13.45 11.6889 13.45 10.75H14.3V0.549988H6.65001V2.24999H7.50001V4.79999H4.10001V0.549988ZM12.6 9.04999V6.49999H2.40001V9.04999H12.6ZM9.20001 4.79999H12.6V2.24999H9.20001V4.79999Z"
                  fill="currentColor"
                />
              </svg>
              <span>{hypeTrain.level}</span>
            </CardChip>
          )}
        </div>
        <div className="flex-1 min-w-0 flex flex-col justify-center gap-0.5">
          <h3 className="text-textPrimary font-medium text-[13px] leading-snug line-clamp-2">
            <StreamTitleWithEmojis title={stream.title} />
          </h3>
          <div className="flex items-center gap-1 text-textSecondary text-[12px]">
            <span className="truncate">{stream.user_name}</span>
            {collab && <TogetherTag collab={collab} />}
            {showPlatform && !isTwitch && <ProviderMark provider={streamProvider(stream)} size={12} />}
            {stream.broadcaster_type === 'partner' && (
              <svg className="w-2.5 h-2.5 flex-shrink-0" viewBox="0 0 16 16" fill="#9146FF">
                <path
                  fillRule="evenodd"
                  d="M12.5 3.5 8 2 3.5 3.5 2 8l1.5 4.5L8 14l4.5-1.5L14 8l-1.5-4.5ZM7 11l4.5-4.5L10 5 7 8 5.5 6.5 4 8l3 3Z"
                  clipRule="evenodd"
                ></path>
              </svg>
            )}
          </div>
          {stream.game_name && (
            <div className="flex items-center gap-1 text-textMuted text-[12px]">
              <span className="truncate">{stream.game_name}</span>
              {/* The DROPS badge over the thumbnail already says this. */}
            </div>
          )}
          <div className="flex items-center gap-1.5 flex-wrap">
            <span className="text-[11.5px] text-textMuted">
              {stream.viewer_count.toLocaleString()} viewers
            </span>
            {!!watchStreak && watchStreak > 0 && <StreakBadge streak={watchStreak} />}
          </div>
        </div>
      </button>
    );
  }

  return (
    <button
      onClick={() => onPress(stream)}
      className="w-full text-left glass-panel media-card p-2 active:opacity-80 transition-opacity"
    >
      <div className="relative mb-2 overflow-hidden rounded">
        <img
          loading="lazy"
          decoding="async"
          src={thumbUrl(stream)}
          alt=""
          className="w-full aspect-video object-cover"
          draggable={false}
        />
        <div className="absolute top-1.5 left-1.5 flex items-center gap-1">
          <CardChip kind="live">LIVE</CardChip>
          {hasDrops && (
            <CardChip kind="drops">
              <Package size={10} />
              <span>DROPS</span>
            </CardChip>
          )}
          {hypeTrain && <HypeTrainBadge info={hypeTrain} />}
        </div>
        <CardChip kind="neutral" className="absolute bottom-1.5 left-1.5">
          <UsersThree size={12} weight="bold" className="shrink-0 opacity-80" aria-label="viewers" />
          {stream.viewer_count.toLocaleString()}
        </CardChip>
        {!!watchStreak && watchStreak > 0 && (
          <div className="absolute bottom-1.5 right-1.5">
            <StreakBadge streak={watchStreak} />
          </div>
        )}
      </div>
      <div className="space-y-0.5 px-0.5 pb-0.5">
        <h3 className="text-textPrimary font-medium text-[13px] leading-snug line-clamp-2">
          <StreamTitleWithEmojis title={stream.title} />
        </h3>
        <div className="flex items-center gap-1 text-textSecondary text-[13px]">
          {stream.profile_image_url && (
            <img
              src={stream.profile_image_url}
              alt=""
              draggable={false}
              loading="lazy"
              decoding="async"
              className="w-4 h-4 rounded-full object-cover shrink-0 ring-1 ring-borderSubtle"
            />
          )}
          <span className="truncate">{stream.user_name}</span>
          {collab && <TogetherTag collab={collab} />}
          {showPlatform && !isTwitch && <ProviderMark provider={streamProvider(stream)} size={12} />}
          {stream.broadcaster_type === 'partner' && (
            <svg className="w-3 h-3 flex-shrink-0" viewBox="0 0 16 16" fill="#9146FF">
              <path
                fillRule="evenodd"
                d="M12.5 3.5 8 2 3.5 3.5 2 8l1.5 4.5L8 14l4.5-1.5L14 8l-1.5-4.5ZM7 11l4.5-4.5L10 5 7 8 5.5 6.5 4 8l3 3Z"
                clipRule="evenodd"
              ></path>
            </svg>
          )}
        </div>
        {/* Always rendered with a reserved height, so cards in a row share a
            baseline whether or not this one has a category. The DROPS badge
            over the thumbnail already says the rest. */}
        <div className="flex items-center gap-1 text-textMuted text-[13px] min-h-4">
          {stream.game_name && <span className="line-clamp-1">{stream.game_name}</span>}
        </div>
      </div>
    </button>
  );
};
