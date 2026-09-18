import type { CSSProperties, KeyboardEvent, ReactNode } from 'react';
import { Film, Play, Scissors, Sparkles, Upload, Users } from 'lucide-react';
import { TwitchVerifiedMark } from './ui/TwitchGlyph';
import { VodProgressBar, VodRecordingBadge } from './VodCardMarks';
import { VOD_FALLBACK_THUMB, type MediaKind } from '../utils/vodProgress';
import { glowThumbProps } from '../utils/mediaGlow';
import type { VodProgressSummary } from '../types';

/**
 * The one card every clip and VOD in the app is drawn with: Home's Continue
 * Watching rail and category Clips / Videos grids, and a channel profile's
 * Clips / Videos tabs.
 *
 * Shape: an edge-to-edge 16:9 thumbnail carries everything that is ABOUT the
 * media (what kind it is, how long, how many watched, how far you got) on a
 * bottom scrim; the body below is only what it is called and who made it.
 * No dividers, no pills in the body, and the hover affordance is a centred
 * play disc plus a gentle zoom, so a card reads as one pressable object.
 */

export type { MediaKind };

const KIND_LABEL: Record<MediaKind, { label: string; Icon: typeof Film }> = {
  clip: { label: 'Clip', Icon: Scissors },
  vod: { label: 'VOD', Icon: Film },
  highlight: { label: 'Highlight', Icon: Sparkles },
  upload: { label: 'Upload', Icon: Upload },
};

export interface MediaCardChannel {
  name: string;
  /** Absent means "not known yet": the row shows no placeholder, a grey
   *  monogram would only be noise next to cards that have the real one. */
  avatarUrl?: string;
  /** Twitch partner: draws the verified mark after the name. */
  partner?: boolean;
}

export interface MediaCardProps {
  kind: MediaKind;
  title: string;
  /** Already resolved: placeholders substituted, fallback applied. */
  thumbnailUrl: string;
  /** "5:41:27" for a VOD, "28.4s" for a clip. Bottom-right of the scrim. */
  durationLabel?: string;
  /** Bottom-left of the scrim, unless `stat` overrides it. */
  viewCount?: number;
  /** Replaces the view count (Continue Watching shows time left instead). */
  stat?: ReactNode;
  progress?: VodProgressSummary;
  lengthSeconds?: number;
  /** Helix/GQL video status; "recording" shows the Live now pill. */
  status?: string;
  /** Top-right of the thumbnail: a reaction count, a dismiss button. */
  topRight?: ReactNode;
  channel: MediaCardChannel;
  /** Right end of the channel row, typically the date. */
  trailing?: ReactNode;
  /** Category the media was made in. Always has a line reserved, so a grid
   *  with a few unknowns stays level. */
  category?: string;
  /** Trails the category on its line: "Clipped by X". Watch position is
   *  already on the thumbnail (bar + time left), so VODs pass nothing. */
  note?: string;
  onClick: () => void;
  className?: string;
  style?: CSSProperties;
  ariaLabel?: string;
}

export function MediaCard({
  kind,
  title,
  thumbnailUrl,
  durationLabel,
  viewCount,
  stat,
  progress,
  lengthSeconds,
  status,
  topRight,
  channel,
  trailing,
  category,
  note,
  onClick,
  className = '',
  style,
  ariaLabel,
}: MediaCardProps) {
  const { label, Icon } = KIND_LABEL[kind];
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      onClick();
    }
  };

  return (
    <div
      role="button"
      tabIndex={0}
      aria-label={ariaLabel ?? title}
      className={`glass-panel media-card group/card relative cursor-pointer overflow-hidden transition-all duration-200 hover:bg-glass-hover focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-accent/60 ${className}`}
      style={style}
      onClick={onClick}
      onKeyDown={onKeyDown}
    >
      <div className="relative aspect-video overflow-hidden bg-black/30">
        <img
          loading="lazy"
          src={thumbnailUrl}
          alt=""
          // The card takes its colour from its own thumbnail. Same helper the
          // hand-rolled live cards in Home use, so there is one path rather
          // than two that can drift.
          {...glowThumbProps(thumbnailUrl)}
          onError={(e) => {
            e.currentTarget.src = VOD_FALLBACK_THUMB;
          }}
          className="absolute inset-0 h-full w-full object-cover transition-transform duration-300 ease-out group-hover/card:scale-[1.04]"
        />

        {/* Scrim: keeps the stat row legible over any thumbnail and gives the
            progress bar a dark edge to sit on instead of a random frame. */}
        <div className="pointer-events-none absolute inset-x-0 bottom-0 h-3/5 bg-gradient-to-t from-black/80 via-black/35 to-transparent" />

        {/* What kind of media this is. One quiet pill, top-left, so a grid
            that mixes highlights into past broadcasts stays scannable. */}
        <div className="glass-badge pointer-events-none absolute left-2 top-2 flex items-center gap-1 rounded px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-white">
          <Icon size={10} strokeWidth={2.5} />
          {label}
        </div>

        {status === 'recording' ? <VodRecordingBadge status={status} /> : topRight}

        <div className="pointer-events-none absolute inset-0 flex items-center justify-center">
          <span className="glass-button flex h-11 w-11 items-center justify-center rounded-full text-white opacity-0 scale-90 group-hover/card:opacity-100 group-hover/card:scale-100 group-focus-visible/card:opacity-100 group-focus-visible/card:scale-100">
            <Play size={18} fill="currentColor" className="ml-0.5" />
          </span>
        </div>

        <div className="pointer-events-none absolute inset-x-0 bottom-[3px] flex items-end justify-between gap-3 px-2.5 pb-2 text-[11px] leading-none tabular-nums text-white [text-shadow:0_1px_2px_rgba(0,0,0,0.6)]">
          <span className="flex items-center gap-1 font-medium">
            {stat ??
              (viewCount !== undefined && (
                <>
                  <Users size={11} className="opacity-80" />
                  {viewCount.toLocaleString()}
                </>
              ))}
          </span>
          {durationLabel && <span className="text-white/65">{durationLabel}</span>}
        </div>

        <VodProgressBar progress={progress} lengthSeconds={lengthSeconds} />
      </div>

      <div className="px-3 pt-2.5 pb-3">
        {/* One line, always: the title is the least stable thing on the card,
            and letting it wrap is what used to push the category off. The
            full title is a hover away. */}
        <h3
          title={title}
          className="truncate text-[13px] font-medium leading-5 text-textPrimary transition-colors group-hover/card:text-accent"
        >
          {title}
        </h3>
        <div className="mt-1 flex items-center gap-1.5 text-[12px] text-textSecondary">
          {channel.avatarUrl && (
            <img
              loading="lazy"
              src={channel.avatarUrl}
              alt=""
              className="h-4 w-4 shrink-0 rounded-full object-cover ring-1 ring-borderSubtle"
            />
          )}
          {/* min-w-0 is load-bearing for `truncate` inside a flex row. */}
          <span className="min-w-0 truncate">{channel.name}</span>
          {channel.partner && <TwitchVerifiedMark size={12} className="text-[#9146FF]" />}
          {trailing && <span className="ml-auto shrink-0 tabular-nums text-textMuted">{trailing}</span>}
        </div>
        <p className="mt-0.5 min-h-4 truncate text-[11px] leading-4 text-textSecondary/70">
          {[category, note].filter(Boolean).join(' \u00b7 ')}
        </p>
      </div>
    </div>
  );
}

export default MediaCard;
