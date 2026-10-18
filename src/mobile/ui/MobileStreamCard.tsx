// Touch-native stream card in StreamNook's own card language, matching the
// desktop Home cards: padded glass panel, rounded thumbnail, the canonical
// CardChip (DROPS, hype train, watch-streak flame; live is the dot in the viewers chip),
// .glass-badge viewer chip, a "+2" beside the name while the channel streams
// with others, partner verified mark, and Apple-style emoji titles.
// Only the sizing is phone-tuned.
import React from 'react';
import { Flame } from 'lucide-react';
import { Package } from 'phosphor-react';
import StreamTitleWithEmojis from '../../components/StreamTitleWithEmojis';
import type { TwitchStream } from '../../types';
import { previewStamp } from '../followRefresh';
import { ProviderMark } from '../../components/ProviderLogo';
import { isTwitchStream, streamProvider } from '../../utils/streamProvider';
import { CardChip } from '../../components/ui/CardChip';
import { TogetherTag } from '../../components/SharedViewers';
import { useAppStore } from '../../stores/AppStore';
import { groupFor } from '../../utils/sharedViewers';
import { SevenTvAvatar } from '../../components/SevenTvAvatar';
import { sevenTvIdForStream } from '../../services/sevenTvPictures';
import { useChannelHold } from './ChannelHold';

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
  hypeTrain?: HypeTrainBadgeInfo;
  watchStreak?: number;
  onPress: (stream: TwitchStream) => void;
  /** Mark non-Twitch rows with their platform. Off where the screen already
   *  says which platform every row is on (Browse switched to Kick). */
  showPlatform?: boolean;
  /** 'card' = big thumbnail stack; 'row' = compact list row (thumb left). */
  variant?: 'card' | 'row';
  /** Off where the screen already names the category (a category's own list). */
  showCategory?: boolean;
}> = ({ stream, hypeTrain, watchStreak, onPress, variant = 'card', showPlatform = true, showCategory = true }) => {
  // The icon means "you can earn drops HERE", not "this game has drops".
  //
  // It used to mean the latter, which put a gift on every channel in a
  // drops-enabled category including the ones a restricted campaign excludes.
  // Tapping through then showed no progress and nothing explained why. There is
  // deliberately no third state for "this category has drops but not on this
  // channel": that is a promise the channel cannot keep, and a card is the
  // wrong place to explain someone else's campaign rules.
  const isTwitch = isTwitchStream(stream);
  // Twitch's Shared Viewership or Shared Chat, from the Rust home snapshot.
  // Read here rather than passed in, so every screen's cards carry it.
  const collab = useAppStore((s) => groupFor(s.collaborations, s.sharedChats, stream));
  // Rust stamps it on every Twitch row it serves; a Kick stream in the same
  // category earns nothing.
  const hasDrops = isTwitch && !!stream.drops_earnable;
  // A tap watches; a hold offers the chat on its own.
  const hold = useChannelHold(stream, onPress);

  if (variant === 'row') {
    return (
      <>
      <button
        {...hold.handlers}
        className="w-full text-left glass-panel media-card p-2 flex gap-2.5 active:opacity-80 transition-opacity select-none [-webkit-touch-callout:none]"
      >
        <div className="sn-card-thumb relative w-[156px] shrink-0 overflow-hidden self-center">
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
        {/* Line heights at about 1.2 times the size, as the iPhone sets them:
            five lines then stand as tall as the 156-wide preview, so the card's
            8px inset reads the same on every side. Looser leading made the
            column 14px taller than the preview, which sat 15px from the top and
            bottom but 8px from the left. */}
        <div className="flex-1 min-w-0 flex flex-col justify-center gap-0.5">
          <h3 className="text-textPrimary font-semibold text-[14px] leading-[17px] line-clamp-2 mb-px">
            <StreamTitleWithEmojis title={stream.title} />
          </h3>
          <div className="flex items-center gap-1 text-textSecondary text-[13px] leading-4">
            <span className="truncate">{stream.user_name}</span>
            {collab && <TogetherTag collab={collab} compact />}
            {/* At the name's cap height: the Kick K fills its box, so 12 stood
                taller than the letters beside it. */}
            {showPlatform && !isTwitch && <ProviderMark provider={streamProvider(stream)} size={10} />}
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
          {showCategory && stream.game_name && (
            <div className="flex items-center gap-1 text-textMuted text-[12.5px] leading-[15px]">
              <span className="truncate">{stream.game_name}</span>
              {/* The DROPS badge over the thumbnail already says this. */}
            </div>
          )}
          {/* The streak rides the viewers line as amber words: as a dark chip it
              was the heaviest thing in the text column. */}
          <div className="flex items-center gap-2 text-[12.5px] leading-[15px]">
            <span className="text-textMuted">{stream.viewer_count.toLocaleString()} viewers</span>
            {!!watchStreak && watchStreak > 0 && (
              <span
                className="flex items-center gap-0.5 tabular-nums text-[#fcd34d]"
                aria-label={`Watch streak ${watchStreak}`}
              >
                <Flame size={11} className="fill-current stroke-[2.5]" aria-hidden />
                {watchStreak}
              </span>
            )}
          </div>
        </div>
      </button>
      {hold.sheet}
      </>
    );
  }

  return (
    <>
    <button
      {...hold.handlers}
      className="w-full text-left glass-panel media-card p-2 active:opacity-80 transition-opacity select-none [-webkit-touch-callout:none]"
    >
      <div className="sn-card-thumb relative mb-2 overflow-hidden">
        <img
          loading="lazy"
          decoding="async"
          src={thumbUrl(stream)}
          alt=""
          className="w-full aspect-video object-cover"
          draggable={false}
        />
        {/* No LIVE chip: every stream in these lists is live, so live is said
            once, by the red dot in the viewers chip. */}
        <div className="absolute top-1.5 left-1.5 flex items-center gap-1">
          {hasDrops && (
            <CardChip kind="drops">
              <Package size={10} />
              <span>DROPS</span>
            </CardChip>
          )}
          {hypeTrain && <HypeTrainBadge info={hypeTrain} />}
        </div>
        <CardChip
          kind="neutral"
          className="absolute bottom-1.5 left-1.5 tabular-nums"
          aria-label={`Live, ${stream.viewer_count.toLocaleString()} viewers`}
        >
          <span className="blend-live-dot" aria-hidden />
          {stream.viewer_count.toLocaleString()}
        </CardChip>
        {!!watchStreak && watchStreak > 0 && (
          <div className="absolute bottom-1.5 right-1.5">
            <StreakBadge streak={watchStreak} />
          </div>
        )}
      </div>
      <div className="space-y-0.5 px-0.5 pb-0.5">
        <h3 className="text-textPrimary font-semibold text-[14px] leading-snug line-clamp-2 mb-px">
          <StreamTitleWithEmojis title={stream.title} />
        </h3>
        <div className="flex items-center gap-1 text-textSecondary text-[13px]">
          {stream.profile_image_url && (
            <SevenTvAvatar
              cosmeticsId={sevenTvIdForStream(stream)}
              px={16}
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
          {showPlatform && !isTwitch && <ProviderMark provider={streamProvider(stream)} size={10} />}
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
        {showCategory && (
          <div className="flex items-center gap-1 text-textMuted text-[12.5px] min-h-4">
            {stream.game_name && <span className="line-clamp-1">{stream.game_name}</span>}
          </div>
        )}
      </div>
    </button>
    {hold.sheet}
    </>
  );
};
