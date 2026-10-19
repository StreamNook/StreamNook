import React from 'react';
import { X } from 'lucide-react';
import { Tooltip } from './ui/Tooltip';
import { WatchStreakFlame } from './icons/WatchStreakFlame';

// Types
export interface WatchStreakMilestone {
  milestone_id: string;
  streak_count: number;
  threshold: number;
  share_status: string;
  copo_bonus: number;
}

interface WatchStreakBannerProps {
  milestone: WatchStreakMilestone;
  isStreakMode: boolean;
  onActivateShare: () => void;
  onDismiss: () => void;
  onCancelShare: () => void;
}

// The phones' streak banner on the desktop: the static popover surface the
// pinned banners use (solid at Glassiness 0), Twitch's streak flame in the
// theme's orange, what it is in bold with the detail quieter beside it, then
// one action and a quieter dismiss. Share arms the composer: the next message
// sent carries the streak, and the banner says so until then or a cancel.
const surface = 'sn-popover sn-popover--static mb-2 flex items-center gap-2.5 pl-3 pr-1.5 py-1.5';
const quiet =
  'flex-shrink-0 w-7 h-7 flex items-center justify-center rounded-full text-textMuted hover:text-textSecondary hover:bg-glass transition-colors';

const WatchStreakBanner: React.FC<WatchStreakBannerProps> = ({
  milestone,
  isStreakMode,
  onActivateShare,
  onDismiss,
  onCancelShare,
}) => {
  const streakCount = milestone.streak_count;
  const copoBonus = milestone.copo_bonus;

  // Armed: the next message sent carries the streak.
  if (isStreakMode) {
    return (
      <div className={surface} role="status">
        <WatchStreakFlame size={15} className="flex-shrink-0 text-highlight-orange" />
        <span className="min-w-0 flex-1 flex flex-col">
          <span className="truncate text-[13px] font-semibold text-textPrimary">Sharing your watch streak</span>
          <span className="truncate text-[11.5px] text-textMuted">It sends with your next message</span>
        </span>
        <Tooltip content="Cancel share" side="top">
          <button onClick={onCancelShare} className={quiet} aria-label="Cancel share">
            <X size={13} />
          </button>
        </Tooltip>
      </div>
    );
  }

  return (
    <div className={surface}>
      <WatchStreakFlame size={15} className="flex-shrink-0 text-highlight-orange" />
      <span className="min-w-0 flex-1 flex flex-col">
        <span className="truncate text-[13px]">
          <span className="font-semibold text-textPrimary">Watch streak</span>
          {streakCount > 0 && (
            <span className="text-textSecondary">
              {' · '}
              {streakCount} stream{streakCount === 1 ? '' : 's'}
            </span>
          )}
        </span>
        {copoBonus > 0 && (
          <span className="truncate text-[11.5px] text-textMuted">
            +{copoBonus.toLocaleString()} points for sharing
          </span>
        )}
      </span>
      <button
        onClick={onActivateShare}
        className="flex-shrink-0 h-7 px-3 rounded-full bg-accent/15 hover:bg-accent/25 text-accent text-[12px] font-semibold transition-colors active:scale-95"
      >
        Share
      </button>
      <Tooltip content="Dismiss" side="top">
        <button onClick={onDismiss} className={quiet} aria-label="Dismiss watch streak">
          <X size={13} />
        </button>
      </Tooltip>
    </div>
  );
};

export default WatchStreakBanner;
