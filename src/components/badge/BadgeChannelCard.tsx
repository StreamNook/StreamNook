import { ArrowUpRight } from 'lucide-react';
import type { TwitchStream } from '../../types';
import type { PlaceChannel } from '../../services/watchPlaces';
import { DEFAULT_AVATAR } from '../multi-nook/channelSearch';
import { SevenTvAvatar } from '../SevenTvAvatar';

/** A themed card for a streamer a badge points at: avatar, name, and Rust's
 *  live line (category and viewers) or "Offline". Clicking opens the channel
 *  through the app's normal watch flow (`onWatch`), with the live stream so it
 *  opens with all the normal services (player, chat, metadata). */
export const BadgeChannelCard = ({
  place,
  onWatch,
}: {
  place: PlaceChannel;
  onWatch: (login: string, streamInfo?: TwitchStream) => void;
}) => {
  const isLive = !!place.live;
  return (
    <button
      onClick={() => onWatch(place.login, place.live ?? undefined)}
      className="group flex items-center gap-3 w-full text-left p-2.5 rounded-xl bg-white/[0.04] hover:bg-white/[0.07] border border-white/[0.06] transition-colors"
    >
      <div className="relative shrink-0">
        <SevenTvAvatar
          cosmeticsId={place.user_id ?? undefined}
          px={46}
          motion="hover"
          src={place.avatar_url || DEFAULT_AVATAR}
          alt=""
          onError={(e) => {
            const img = e.currentTarget as HTMLImageElement;
            if (img.src !== DEFAULT_AVATAR) img.src = DEFAULT_AVATAR;
          }}
          className="w-[46px] h-[46px] rounded-full object-cover"
        />
        {isLive && (
          <span className="absolute -bottom-0.5 -right-0.5 w-3 h-3 rounded-full bg-red-500 border-2 border-secondary" />
        )}
      </div>
      <div className="flex-1 min-w-0">
        <div className="text-[11px] text-textMuted uppercase tracking-wide">Channel</div>
        <div className="text-[15px] font-medium text-textPrimary truncate group-hover:text-accent transition-colors">
          {place.display_name}
        </div>
        <div className={`text-[12px] truncate ${isLive ? 'text-red-400' : 'text-textSecondary'}`}>
          {place.status_line}
        </div>
      </div>
      <ArrowUpRight
        size={18}
        className="text-textMuted group-hover:text-accent transition-colors shrink-0"
      />
    </button>
  );
};
