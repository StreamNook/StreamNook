import type { TwitchStream } from '../../types';
import type { PlaceChannel } from '../../services/watchPlaces';
import { Tooltip } from '../ui/Tooltip';
import { DEFAULT_AVATAR } from '../multi-nook/channelSearch';
import { SevenTvAvatar } from '../SevenTvAvatar';

/** Inline clickable chip for a channel named in badge earn text (e.g.
 *  "/studbudz"): the avatar and display name Rust resolved, a live dot, and a
 *  click that opens the channel. A plain "/login" span until (or if never) the
 *  channel resolves, so the text always reads correctly. */
export const BadgeChannelChip = ({
  login,
  place,
  onWatch,
}: {
  login: string;
  place?: PlaceChannel;
  onWatch: (login: string, streamInfo?: TwitchStream) => void;
}) => {
  if (!place) {
    return <span className="text-textSecondary">/{login}</span>;
  }

  return (
    <Tooltip content={`Watch ${place.display_name}`}>
      <button
        type="button"
        onClick={() => onWatch(place.login, place.live ?? undefined)}
        style={{ verticalAlign: '-5px' }}
        className="inline-flex items-center gap-1.5 pl-0.5 pr-2 py-0.5 rounded-full bg-white/[0.06] hover:bg-white/[0.10] transition-colors"
      >
        <SevenTvAvatar
          cosmeticsId={place.user_id ?? undefined}
          px={18}
          src={place.avatar_url || DEFAULT_AVATAR}
          alt=""
          onError={(e) => {
            const img = e.currentTarget as HTMLImageElement;
            if (img.src !== DEFAULT_AVATAR) img.src = DEFAULT_AVATAR;
          }}
          className="w-[18px] h-[18px] rounded-full object-cover"
        />
        <span className="text-[13px] font-medium text-textPrimary leading-none">{place.display_name}</span>
        {place.live && <span className="w-1.5 h-1.5 rounded-full bg-red-500 shrink-0" aria-label="live" />}
      </button>
    </Tooltip>
  );
};
