// Shareable streaks above the composer: a watch-streak milestone, and a resub
// notification with its optional sub-streak.
//
// The desktop banners (WatchStreakBanner / ResubNotificationBanner) are reused
// for their DATA contract but not their chrome — both lean on `group-hover`,
// which does nothing under a thumb, and both are laid out for a wide dock. So
// the same four backend calls drive a compact touch strip instead:
//
//   get_watch_streak      -> milestone for this channel
//   share_watch_streak    -> posts it, consuming the milestone
//   get_resub_notification-> a pending resub token
//   use_resub_token       -> posts it, optionally including the sub streak
//
// Both are one-shot: sharing consumes the token, so each strip disappears
// afterwards rather than lingering as a dead control.
import React, { useCallback, useEffect, useState } from 'react';
// lucide Flame with stroke-[2.5], not the phosphor one: this is the same glyph
// the desktop WatchStreakBanner and the mobile StreakBadge on stream cards both
// use, and a watch streak should look like a watch streak everywhere.
import { Flame } from 'lucide-react';
import { Star, X } from 'phosphor-react';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../../stores/AppStore';
import { Logger } from '../../utils/logger';

interface WatchStreakMilestone {
  milestone_id: string;
  streak_count?: number;
  [k: string]: unknown;
}

interface ResubNotification {
  id: string;
  cumulative_tenure_months: number;
  streak_tenure_months: number;
  months: number;
  is_gift_subscription: boolean;
  gifter_display_name: string | null;
}

interface Props {
  channel: string | null;
  channelId: string | null;
  /** Text currently typed; shared alongside the streak like desktop does. */
  message: string;
  /** Clears the composer once a share consumes the message. */
  onShared: () => void;
}

export const StreakBanners: React.FC<Props> = ({ channel, channelId, message, onShared }) => {
  const addToast = useAppStore((s) => s.addToast);
  const [streak, setStreak] = useState<WatchStreakMilestone | null>(null);
  const [resub, setResub] = useState<ResubNotification | null>(null);
  // Dismissals are keyed by channel so leaving and returning does not resurrect
  // something you already waved away, while switching rooms starts clean.
  const [dismissed, setDismissed] = useState<{ channel: string; what: string }[]>([]);
  const [includeStreak, setIncludeStreak] = useState(true);
  const [busy, setBusy] = useState(false);

  const isDismissed = (what: string) =>
    !!channel && dismissed.some((d) => d.channel === channel && d.what === what);

  useEffect(() => {
    if (!channelId || !channel) {
      return;
    }
    let cancelled = false;
    (async () => {
      try {
        const m = await invoke<WatchStreakMilestone | null>('get_watch_streak', { channelId });
        if (!cancelled) setStreak(m ?? null);
      } catch (err) {
        Logger.debug('[Streaks] watch streak unavailable:', err);
      }
      try {
        const r = await invoke<ResubNotification | null>('get_resub_notification', { channelId });
        if (!cancelled) setResub(r ?? null);
      } catch (err) {
        Logger.debug('[Streaks] resub unavailable:', err);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [channel, channelId]);

  const dismiss = useCallback(
    (what: string) => {
      if (!channel) return;
      setDismissed((prev) => [...prev, { channel, what }]);
    },
    [channel],
  );

  const shareWatchStreak = async () => {
    if (!channelId || !streak || busy) return;
    setBusy(true);
    try {
      const ok = await invoke<boolean>('share_watch_streak', {
        channelId,
        milestoneId: streak.milestone_id,
        message: message.trim() || null,
      });
      if (ok) {
        setStreak(null);
        onShared();
      } else {
        addToast('Could not share that watch streak.', 'error');
      }
    } catch (err) {
      Logger.error('[Streaks] watch streak share failed:', err);
      addToast('Could not share that watch streak.', 'error');
    } finally {
      setBusy(false);
    }
  };

  const shareResub = async () => {
    if (!channel || !resub || busy) return;
    setBusy(true);
    try {
      const ok = await invoke<boolean>('use_resub_token', {
        channelLogin: channel,
        message: message.trim() || null,
        includeStreak,
        tokenId: resub.id,
      });
      if (ok) {
        setResub(null);
        onShared();
      } else {
        addToast('Could not share that resub.', 'error');
      }
    } catch (err) {
      Logger.error('[Streaks] resub share failed:', err);
      addToast('Could not share that resub.', 'error');
    } finally {
      setBusy(false);
    }
  };

  const showStreak = !!streak && !isDismissed('watch');
  const showResub = !!resub && !isDismissed('resub');
  if (!showStreak && !showResub) return null;

  const chip =
    'shrink-0 px-2.5 py-1 rounded-full text-[11.5px] font-semibold disabled:opacity-50';

  return (
    <div className="flex flex-col gap-1 px-2.5 pt-1.5">
      {/* Resub sits above the watch streak: it is the rarer event and expires,
          so it should be the one you see first. */}
      {showResub && resub && (
        <div className="sn-popover flex items-center gap-2 px-2.5 py-1.5">
          <Star size={13} weight="fill" className="text-accent shrink-0" />
          <div className="min-w-0 flex-1">
            <div className="text-[12px] text-textPrimary truncate">
              {resub.is_gift_subscription && resub.gifter_display_name
                ? `Gift sub from ${resub.gifter_display_name}`
                : `${resub.cumulative_tenure_months} month${resub.cumulative_tenure_months === 1 ? '' : 's'} subscribed`}
            </div>
            {resub.streak_tenure_months > 1 && (
              <button
                onClick={() => setIncludeStreak((v) => !v)}
                className="text-[10.5px] text-textMuted active:opacity-70"
              >
                {includeStreak ? '✓ ' : ''}
                Include {resub.streak_tenure_months}-month streak
              </button>
            )}
          </div>
          <button
            onClick={() => void shareResub()}
            disabled={busy}
            className={`${chip} bg-accent/20 text-accent`}
          >
            Share
          </button>
          <button
            onClick={() => dismiss('resub')}
            className="shrink-0 p-1 text-textMuted active:text-textPrimary"
            aria-label="Dismiss resub"
          >
            <X size={12} weight="bold" />
          </button>
        </div>
      )}

      {/* Amber, exactly as the desktop banner and the card StreakBadge do it:
          amber-500/5 ground, amber-500/20 border, the flame in its own amber
          chip with the warning-token glow, and a solid amber share button. */}
      {showStreak && streak && (
        <div className="flex items-center gap-2 px-2.5 py-1.5 rounded-lg bg-amber-500/5 border border-amber-500/20 backdrop-blur-md">
          <div className="flex items-center justify-center w-6 h-6 rounded bg-amber-500/10 shrink-0 shadow-[0_0_8px_color-mix(in_srgb,var(--color-warning)_20%,transparent)]">
            <Flame size={13} className="text-amber-400 stroke-[2.5]" />
          </div>
          <span className="text-[12px] text-textPrimary truncate flex-1 min-w-0">
            {streak.streak_count
              ? `${streak.streak_count} stream watch streak`
              : 'Watch streak milestone'}
          </span>
          <button
            onClick={() => void shareWatchStreak()}
            disabled={busy}
            className={`${chip} bg-amber-500 text-black shadow-[0_0_10px_color-mix(in_srgb,var(--color-warning)_20%,transparent)] active:scale-95 transition-transform`}
          >
            Share
          </button>
          <button
            onClick={() => dismiss('watch')}
            className="shrink-0 p-1 text-amber-400/60 active:text-amber-400"
            aria-label="Dismiss watch streak"
          >
            <X size={12} weight="bold" />
          </button>
        </div>
      )}
    </div>
  );
};
