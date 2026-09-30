import React from 'react';
import { Tooltip } from './ui/Tooltip';
import { Flame, X, Reply } from 'lucide-react';

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

const WatchStreakBanner: React.FC<WatchStreakBannerProps> = ({
  milestone,
  isStreakMode,
  onActivateShare,
  onDismiss,
  onCancelShare,
}) => {
  const streakCount = milestone.streak_count;
  const copoBonus = milestone.copo_bonus;

  // Active mode: user is composing their streak share message
  if (isStreakMode) {
    return (
      /* glass-exempt: a tint banner on the chat column's own ground. */
      <div className="mb-2 flex items-center gap-2 px-3 py-2 bg-amber-500/5 backdrop-blur-md rounded-lg border border-amber-500/20">
        <Reply size={14} className="text-amber-400 flex-shrink-0" />
        <span className="text-xs text-textSecondary flex-1 flex items-center gap-1.5">
          <span>Sharing</span>
          <span className="text-amber-400 font-semibold flex items-center gap-0.5">
            <Flame size={12} className="stroke-[2.5]" />
            {streakCount}-Stream Streak
          </span>
          {copoBonus > 0 && (
            <span className="text-textTertiary text-[10px] ml-0.5">(+{copoBonus.toLocaleString()} points)</span>
          )}
        </span>
        
        {/* Cancel button */}
        <Tooltip content="Cancel share" side="top">
          <button 
            onClick={onCancelShare} 
            className="text-textSecondary hover:text-textPrimary transition-colors p-1 rounded-md hover:bg-glass" 
          >
            <X size={14} />
          </button>
        </Tooltip>
      </div>
    );
  }

  // Collapsed mode: notification banner
  return (
    /* glass-exempt: a tint banner on the chat column's own ground. */
    <div className="mb-2 flex items-center gap-3 px-3 py-2 bg-amber-500/5 backdrop-blur-md rounded-lg border border-amber-500/20 hover:border-amber-500/40 hover:bg-amber-500/10 transition-colors duration-200">
      <div className="flex items-center justify-center w-6 h-6 rounded bg-amber-500/10 flex-shrink-0 shadow-[0_0_8px_color-mix(in_srgb,var(--color-warning)_20%,transparent)]">
        <Flame size={14} className="text-amber-400 stroke-[2.5]" />
      </div>
      
      <div className="flex-1 min-w-0 flex flex-col justify-center">
        <span className="text-[13px] text-textPrimary leading-tight">
          Current Watch Streak
        </span>
        <div className="flex items-center gap-1.5 mt-0.5">
          <span className="text-xs text-amber-400 font-semibold">
            {streakCount} Streams
          </span>
          {copoBonus > 0 && (
            <span className="text-[10px] text-textTertiary">• +{copoBonus.toLocaleString()} points</span>
          )}
        </div>
      </div>
      
      <div className="flex items-center gap-1.5 flex-shrink-0 ml-2">
        {/* Share button */}
        <button
          onClick={onActivateShare}
          className="px-3 py-1.5 text-xs font-semibold text-black bg-amber-500 hover:bg-amber-400 rounded-[4px] shadow-[0_0_10px_color-mix(in_srgb,var(--color-warning)_20%,transparent)] transition-all active:scale-95"
        >
          Share
        </button>
        
        {/* Dismiss button */}
        <Tooltip content="Dismiss" side="top">
          <button 
            onClick={onDismiss} 
            className="p-1.5 text-textTertiary hover:text-textPrimary transition-colors rounded-md hover:bg-glass" 
          >
            <X size={14} strokeWidth={2.5} />
          </button>
        </Tooltip>
      </div>
    </div>
  );
};

export default WatchStreakBanner;
