// Channels a search found that are not streaming. They are not stream cards:
// there is no preview, no viewer count and nothing to play, and drawn as cards
// they read as live channels with 0 viewers that opened onto nothing. A row
// opens the channel's offline room, where its chat is and where its stream
// starts when it goes live. One group for Twitch and Kick alike.
import React from 'react';
import type { TwitchStream } from '../../types';
import { SevenTvAvatar } from '../../components/SevenTvAvatar';
import { sevenTvIdForStream } from '../../services/sevenTvPictures';
import { streamKey } from '../../utils/streamProvider';

/** Row padding plus the picture plus the gap: where a divider starts, so it
 *  runs under the names and leaves the pictures a clean column. */
const AVATAR = 36;
const DIVIDER_INSET = 14 + AVATAR + 12;

export const OfflineChannelsGroup: React.FC<{
  channels: TwitchStream[];
  onOpen: (channel: TwitchStream) => void;
  /** Spacing from what sits above it, decided by the screen. */
  className?: string;
}> = ({ channels, onOpen, className = '' }) => (
  <section aria-label="Offline channels" className={`px-4 ${className}`}>
    <h2 className="px-1 pb-2 text-[12px] font-semibold uppercase tracking-wide text-textMuted">
      Offline channels
    </h2>
    {/* The stream cards' own surface, so the group and the live cards above
        it read as one list. */}
    <div className="glass-panel media-card overflow-hidden">
      {channels.map((c, i) => {
        // Search hands back the channel's picture as its "thumbnail".
        const avatar = c.profile_image_url || c.thumbnail_url;
        const name = c.user_name || c.user_login;
        return (
          <button
            key={streamKey(c)}
            onClick={() => onOpen(c)}
            className="relative flex w-full items-center gap-3 px-3.5 py-2.5 text-left active:bg-white/5"
          >
            {i > 0 && (
              <span
                aria-hidden
                className="pointer-events-none absolute right-0 top-0 h-px origin-top scale-y-50 bg-borderSubtle"
                style={{ left: DIVIDER_INSET }}
              />
            )}
            {avatar ? (
              <SevenTvAvatar
                cosmeticsId={sevenTvIdForStream(c)}
                px={AVATAR}
                src={avatar}
                alt=""
                draggable={false}
                loading="lazy"
                decoding="async"
                className="h-9 w-9 shrink-0 rounded-full object-cover ring-1 ring-borderSubtle"
              />
            ) : (
              <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-surface text-[14px] font-semibold text-textSecondary">
                {name.charAt(0).toUpperCase()}
              </div>
            )}
            <span className="min-w-0 flex-1">
              <span className="block truncate text-[15px] font-semibold leading-5 text-textPrimary">{name}</span>
              <span className="mt-0.5 block truncate text-[12.5px] leading-4 text-textSecondary">
                {c.game_name || 'Offline'}
              </span>
            </span>
          </button>
        );
      })}
    </div>
  </section>
);
