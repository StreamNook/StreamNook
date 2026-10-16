// Channels a search found that are not streaming. They are not stream cards:
// there is no preview, no viewer count and nothing to play, and drawn as cards
// they read as live channels with 0 viewers that opened onto nothing. A row
// opens the channel's offline room, where its chat is and where its stream
// starts when it goes live.
import React from 'react';
import type { TwitchStream } from '../../types';
import { SevenTvAvatar } from '../../components/SevenTvAvatar';
import { sevenTvIdForStream } from '../../services/sevenTvPictures';

export const OfflineChannelsGroup: React.FC<{
  channels: TwitchStream[];
  onOpen: (channel: TwitchStream) => void;
}> = ({ channels, onOpen }) => (
  <section aria-label="Offline channels">
    <h2 className="px-1 pb-2 text-[12px] font-semibold uppercase tracking-wide text-textMuted">
      Offline channels
    </h2>
    <div className="glass-panel overflow-hidden rounded-2xl">
      {channels.map((c, i) => {
        // Search hands back the channel's picture as its "thumbnail".
        const avatar = c.profile_image_url || c.thumbnail_url;
        return (
          <button
            key={c.user_id || c.user_login}
            onClick={() => onOpen(c)}
            className={`flex w-full items-center gap-3 px-3 py-2.5 text-left active:bg-white/5 ${
              i > 0 ? 'border-t border-borderSubtle' : ''
            }`}
          >
            {avatar ? (
              <SevenTvAvatar
                cosmeticsId={sevenTvIdForStream(c)}
                px={36}
                src={avatar}
                alt=""
                draggable={false}
                loading="lazy"
                decoding="async"
                className="h-9 w-9 shrink-0 rounded-full object-cover ring-1 ring-borderSubtle"
              />
            ) : (
              <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-surface text-[14px] font-semibold text-textSecondary">
                {(c.user_name || c.user_login).charAt(0).toUpperCase()}
              </div>
            )}
            <span className="min-w-0 flex-1">
              <span className="block truncate text-[14px] font-semibold text-textPrimary">{c.user_name}</span>
              <span className="block truncate text-[12.5px] text-textMuted">{c.game_name || 'Offline'}</span>
            </span>
          </button>
        );
      })}
    </div>
  </section>
);
