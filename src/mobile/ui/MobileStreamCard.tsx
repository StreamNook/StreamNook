// Touch-native stream card in StreamNook's own card language, matching the
// desktop Home cards: padded glass panel, rounded thumbnail, the canonical
// .live-dot, .drops-badge-glass, hype-train badge, watch-streak flame,
// .glass-badge viewer chip, partner verified mark, and Apple-style emoji
// titles. Only the sizing is phone-tuned.
import React from 'react';
import { Flame, Gift } from 'lucide-react';
import StreamTitleWithEmojis from '../../components/StreamTitleWithEmojis';
import { campaignEarnableOn } from '../dropsEligibility';
import type { DropsByGame } from '../dropsCampaigns';
import type { TwitchStream } from '../../types';

function thumbUrl(stream: TwitchStream): string {
  return stream.thumbnail_url.replace('{width}', '640').replace('{height}', '360');
}

export interface HypeTrainBadgeInfo {
  level: number;
  isGolden?: boolean;
}

const HypeTrainBadge: React.FC<{ info: HypeTrainBadgeInfo }> = ({ info }) => (
  <div className={info.isGolden ? 'hype-train-badge-glass-golden' : 'hype-train-badge-glass'}>
    <svg className="w-2.5 h-2.5" viewBox="0 0 15 13" fill="none">
      <path
        fillRule="evenodd"
        clipRule="evenodd"
        d="M4.10001 0.549988H2.40001V4.79999H0.700012V10.75H1.55001C1.55001 11.6889 2.31113 12.45 3.25001 12.45C4.1889 12.45 4.95001 11.6889 4.95001 10.75H5.80001C5.80001 11.6889 6.56113 12.45 7.50001 12.45C8.4389 12.45 9.20001 11.6889 9.20001 10.75H10.05C10.05 11.6889 10.8111 12.45 11.75 12.45C12.6889 12.45 13.45 11.6889 13.45 10.75H14.3V0.549988H6.65001V2.24999H7.50001V4.79999H4.10001V0.549988ZM12.6 9.04999V6.49999H2.40001V9.04999H12.6ZM9.20001 4.79999H12.6V2.24999H9.20001V4.79999Z"
        fill="currentColor"
      />
    </svg>
    <span>LVL {info.level}</span>
  </div>
);

const StreakBadge: React.FC<{ streak: number }> = ({ streak }) => (
  <div className="flex items-center gap-1 font-bold text-[10px] leading-tight px-1.5 py-0.5 rounded shadow-[0_0_10px_color-mix(in_srgb,var(--color-warning)_25%,transparent)] bg-amber-500/10 text-amber-400 border border-amber-500/30 backdrop-blur-md">
    <Flame size={10} className="stroke-[2.5]" />
    <span>{streak}</span>
  </div>
);

export const MobileStreamCard: React.FC<{
  stream: TwitchStream;
  dropsGameNames?: DropsByGame;
  hypeTrain?: HypeTrainBadgeInfo;
  watchStreak?: number;
  onPress: (stream: TwitchStream) => void;
  /** 'card' = big thumbnail stack; 'row' = compact list row (thumb left). */
  variant?: 'card' | 'row';
}> = ({ stream, dropsGameNames, hypeTrain, watchStreak, onPress, variant = 'card' }) => {
  // The icon means "you can earn drops HERE", not "this game has drops".
  //
  // It used to mean the latter, which put a gift on every channel in a
  // drops-enabled category including the ones a restricted campaign excludes.
  // Tapping through then showed no progress and nothing explained why. There is
  // deliberately no third state for "this category has drops but not on this
  // channel": that is a promise the channel cannot keep, and a card is the
  // wrong place to explain someone else's campaign rules.
  const hasDrops = !!(
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
            <div
              className="drops-badge-glass"
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
              <Gift size={10} />
            </div>
          )}
          {hypeTrain && (
            <div
              className={hypeTrain.isGolden ? 'hype-train-badge-glass-golden' : 'hype-train-badge-glass'}
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
            </div>
          )}
        </div>
        <div className="flex-1 min-w-0 flex flex-col justify-center gap-0.5">
          <h3 className="text-textPrimary font-medium text-[13px] leading-snug line-clamp-2">
            <StreamTitleWithEmojis title={stream.title} />
          </h3>
          <div className="flex items-center gap-1 text-textSecondary text-[12px]">
            <span className="truncate">{stream.user_name}</span>
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
              {hasDrops && <Gift size={10} className="text-accent flex-shrink-0" />}
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
          src={thumbUrl(stream)}
          alt=""
          className="w-full aspect-video object-cover"
          draggable={false}
        />
        <div className="absolute top-1.5 left-1.5 flex items-center gap-1">
          <div className="live-dot text-xs px-1.5 py-0.5">LIVE</div>
          {hasDrops && (
            <div className="drops-badge-glass">
              <Gift size={10} />
              <span>DROPS</span>
            </div>
          )}
          {hypeTrain && <HypeTrainBadge info={hypeTrain} />}
        </div>
        <div className="absolute bottom-1.5 left-1.5 px-2 py-0.5 glass-badge text-white text-xs font-medium rounded">
          {stream.viewer_count.toLocaleString()} viewers
        </div>
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
          <span className="truncate">{stream.user_name}</span>
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
        {stream.game_name && (
          <div className="flex items-center gap-1 text-textMuted text-[13px]">
            <span className="line-clamp-1">{stream.game_name}</span>
            {hasDrops && <Gift size={10} className="text-accent flex-shrink-0" />}
          </div>
        )}
      </div>
    </button>
  );
};
