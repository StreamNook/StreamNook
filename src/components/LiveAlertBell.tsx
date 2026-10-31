// The live-alerts bell beside a Follow button.
//
// Shown only for a channel you follow or favorited, once Rust answers that
// the channel has an identity it can mute. It sits to the LEFT of Follow:
// the rows it joins are anchored on the right, so its arrival never moves
// Follow under the pointer. A step quieter than Follow, at the same size.

import { Bell, BellSlash } from 'phosphor-react';
import { Bell as LucideBell, BellOff as LucideBellOff } from 'lucide-react';
import { Tooltip } from './ui/Tooltip';
import { useIsFavorite } from '../utils/favoriteDoor';
import { useLiveAlert } from '../hooks/useLiveAlert';
import type { ProviderId } from '../types/providers';

interface LiveAlertBellProps {
  provider: ProviderId;
  userId?: string | null;
  userLogin?: string | null;
  userName?: string | null;
  /** The Follow button's own state; null while it is still checking. */
  isFollowing: boolean | null;
  /** The Follow button's class, so the bell matches its size and surface. */
  buttonClassName: string;
  /** Icon size classes, as the Follow button's icon. */
  iconClassName: string;
  tooltipSide?: 'top' | 'bottom';
  /** The icon family of the controls beside it: phosphor beside the player's
   *  and tiles' hearts, lucide in the About strip. */
  iconSet?: 'phosphor' | 'lucide';
}

export function LiveAlertBell(props: LiveAlertBellProps) {
  const favorited = useIsFavorite(
    props.userLogin ? { provider: props.provider, userId: props.userId, userLogin: props.userLogin } : null,
  );
  if (props.isFollowing !== true && !favorited) return null;
  return <LiveAlertBellButton {...props} />;
}

function LiveAlertBellButton({
  provider,
  userId,
  userLogin,
  userName,
  buttonClassName,
  iconClassName,
  tooltipSide = 'bottom',
  iconSet = 'phosphor',
}: LiveAlertBellProps) {
  const { on, busy, setOn } = useLiveAlert({ provider, userId, userLogin });
  if (on === null) return null;
  const name = userName || userLogin || '';
  return (
    <Tooltip content={on ? `Live alerts on for ${name}` : `Live alerts off for ${name}`} side={tooltipSide}>
      <button
        type="button"
        onClick={(e) => {
          e.stopPropagation();
          setOn(!on);
        }}
        disabled={busy}
        aria-pressed={on}
        aria-label={on ? `Turn off live alerts for ${name}` : `Turn on live alerts for ${name}`}
        className={`${buttonClassName} ${busy ? 'opacity-60 cursor-wait' : ''}`}
      >
        {iconSet === 'lucide' ? (
          on ? (
            <LucideBell className={`${iconClassName} text-textSecondary`} />
          ) : (
            <LucideBellOff className={`${iconClassName} text-textMuted`} />
          )
        ) : on ? (
          <Bell weight="fill" className={`${iconClassName} text-textSecondary`} />
        ) : (
          <BellSlash weight="bold" className={`${iconClassName} text-textMuted`} />
        )}
      </button>
    </Tooltip>
  );
}
