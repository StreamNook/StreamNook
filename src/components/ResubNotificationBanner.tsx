import React from 'react';
import { Check, Star, X } from 'phosphor-react';
import { CheckCircle2 } from 'lucide-react';
import { Tooltip } from './ui/Tooltip';
import { ACCENT_BUTTON, ACCENT_FILL, BANNER_STYLE, GLAZE_BANNER } from './ui/glazeButtons';

// Types
export interface ResubNotification {
  id: string;
  token: string;
  cumulative_tenure_months: number;
  streak_tenure_months: number;
  months: number;
  is_gift_subscription: boolean;
  gifter_display_name: string | null;
}

interface ResubNotificationBannerProps {
  resubNotification: ResubNotification;
  channelLogin: string;
  isResubMode: boolean;
  includeStreak: boolean;
  onActivateShare: () => void;
  onDismiss: () => void;
  onToggleStreak: () => void;
  onCancelShare: () => void;
  /** Twitch's answer to the last share. */
  feedback?: 'shared' | 'failed' | null;
}

// The streak banners' family, like the watch streak above the composer: the
// chat header capsules' glaze (solid at Glassiness 0), the accent star,
// what it is in bold with the detail quieter beside it, the streak choice on
// its own line, one Share and a quieter dismiss. Share arms the composer: the
// next message sent carries the resub (with the streak when it is ticked).
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

const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? '' : 's'}`;

const ResubNotificationBanner: React.FC<ResubNotificationBannerProps> = ({
  resubNotification,
  // channelLogin is passed but reserved for future use (e.g., channel-specific styling)
  isResubMode,
  includeStreak,
  onActivateShare,
  onDismiss,
  onToggleStreak,
  onCancelShare,
  feedback = null,
}) => {
  const months = resubNotification.cumulative_tenure_months;
  const streakMonths = resubNotification.streak_tenure_months;
  const isGift = resubNotification.is_gift_subscription;
  const gifter = resubNotification.gifter_display_name;
  // A one-month streak says nothing the months do not.
  const hasStreak = streakMonths > 1;
  const detail = isGift ? (gifter ? `from ${gifter}` : null) : plural(months, 'month');
  const star = <Star size={14} weight="fill" className="flex-shrink-0 text-accent" />;

  if (feedback === 'shared') return <SharedRow title={`Your ${isGift ? 'gift sub' : 'resub'} is shared`} />;

  // Armed: the next message sent carries the resub.
  if (isResubMode) {
    return (
      <div className={surface} style={BANNER_STYLE} role="status">
        {star}
        <span className="min-w-0 flex-1 flex flex-col">
          <span className="truncate text-[13px] font-semibold text-textPrimary">
            Sharing your {isGift ? 'gift sub' : 'resub'}
          </span>
          <span className="truncate text-[11.5px] text-textMuted">
            {hasStreak && includeStreak
              ? `Next message, with the ${streakMonths}-month streak`
              : 'It sends with your next message'}
          </span>
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
      {star}
      <span className="min-w-0 flex-1 flex flex-col">
        <span className="truncate text-[13px]">
          <span className="font-semibold text-textPrimary">{isGift ? 'Gift sub' : 'Resub'}</span>
          {detail && <span className="text-textSecondary"> · {detail}</span>}
        </span>
        {hasStreak && (
          <button
            onClick={onToggleStreak}
            aria-pressed={includeStreak}
            className="mt-0.5 self-start max-w-full flex items-center gap-1.5 whitespace-nowrap text-[11.5px] text-textMuted hover:text-textSecondary transition-colors"
          >
            <span
              className={`shrink-0 w-3 h-3 rounded-[3px] border flex items-center justify-center ${
                includeStreak ? 'bg-accent border-accent' : 'border-textMuted'
              }`}
            >
              {includeStreak && <Check size={8} weight="bold" className="text-background" />}
            </span>
            <span className="truncate">Include the {streakMonths}-month streak</span>
          </button>
        )}
        {feedback === 'failed' && <FailedLine />}
      </span>
      <button
        onClick={onActivateShare}
        className={`${ACCENT_BUTTON} flex-shrink-0 h-7 px-3.5 !text-[12px]`}
        style={ACCENT_FILL}
      >
        Share
      </button>
      <Tooltip content="Dismiss" side="top">
        <button onClick={onDismiss} className={quiet} aria-label="Dismiss resub">
          <X size={13} />
        </button>
      </Tooltip>
    </div>
  );
};

export default ResubNotificationBanner;
