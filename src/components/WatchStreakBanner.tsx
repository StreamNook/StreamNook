import React from 'react';
import { CheckCircle2, X } from 'lucide-react';
import { Tooltip } from './ui/Tooltip';
import { ACCENT_BUTTON, ACCENT_FILL, BANNER_STYLE, GLAZE_BANNER } from './ui/glazeButtons';
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
  /** Twitch's answer to the last share. */
  feedback?: 'shared' | 'failed' | null;
}

// The phones' streak banner on the desktop: the chat header capsules' glaze
// (solid at Glassiness 0), Twitch's streak flame in the theme's orange, what
// it is in bold with the detail quieter beside it, then the one glazed accent
// action and a quieter dismiss. Share arms the composer: the next message
// sent carries the streak, and the banner says so until then or a cancel.
const surface = `${GLAZE_BANNER} mb-2 flex items-center gap-2.5 pl-3 pr-1.5 py-1.5`;
const quiet =
  'flex-shrink-0 w-7 h-7 flex items-center justify-center rounded-full text-textMuted hover:text-textSecondary hover:bg-glass transition-colors';

/** Twitch took the share: said once, then the banner goes. */
const SharedRow = ({ title }: { title: string }) => (
  <div className={surface} style={BANNER_STYLE} role="status">
    <CheckCircle2 size={15} className="flex-shrink-0 text-success" />
    <span className="min-w-0 flex-1 truncate py-1 text-[13px] font-semibold text-textPrimary">{title}</span>
  </div>
);

/** Twitch did not take it: one line under the banner's own words. */
const FailedLine = () => (
  <span role="alert" className="truncate text-[11.5px] font-medium text-error">
    Didn't go through. Try again.
  </span>
);

const WatchStreakBanner: React.FC<WatchStreakBannerProps> = ({
  milestone,
  isStreakMode,
  onActivateShare,
  onDismiss,
  onCancelShare,
  feedback = null,
}) => {
  const streakCount = milestone.streak_count;

  if (feedback === 'shared') return <SharedRow title="Your watch streak is shared" />;
  const copoBonus = milestone.copo_bonus;

  // Armed: the next message sent carries the streak.
  if (isStreakMode) {
    return (
      <div className={surface} style={BANNER_STYLE} role="status">
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
    <div className={surface} style={BANNER_STYLE}>
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
        {feedback === 'failed' ? (
          <FailedLine />
        ) : (
          copoBonus > 0 && (
            <span className="truncate text-[11.5px] text-textMuted">
              +{copoBonus.toLocaleString()} points for sharing
            </span>
          )
        )}
      </span>
      <button
        onClick={onActivateShare}
        className={`${ACCENT_BUTTON} flex-shrink-0 h-7 px-3.5 !text-[12px]`}
        style={ACCENT_FILL}
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
